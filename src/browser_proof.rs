use axum::{
    body::Body,
    extract::State,
    http::{header::RETRY_AFTER, HeaderMap, HeaderValue, Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tracing::{error, warn};

use crate::{redis_store::RedisStore, static_api::AppState};

const BROWSER_PROOF_COOKIE: &str = "uma_browser_proof";
const BROWSER_PROOF_HEADER: &str = "X-Browser-Proof";
const API_KEY_HEADER: &str = "X-API-Key";
const STATIC_AUTH_TOKEN_ENV: &str = "STATIC_AUTH_TOKEN";
const BETA_STATIC_AUTH_TOKEN_ENV: &str = "BETA_STATIC_AUTH_TOKEN";

static RATE_LIMITS: OnceLock<Mutex<HashMap<String, RateWindow>>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct RateWindow {
    count: u32,
    reset_at: Instant,
}

#[derive(Debug, Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
    status: u16,
}

#[derive(Debug)]
enum BrowserProofError {
    Missing,
    Store(String),
}

pub async fn api_protection_middleware(
    State(state): State<AppState>,
    headers: HeaderMap,
    method: Method,
    request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();

    if should_skip_api_protection(&method, &path) || api_protection_bypassed() {
        return next.run(request).await;
    }

    if has_static_auth_token(&headers) {
        return next.run(request).await;
    }

    let Some(store) = state.redis_store.as_ref() else {
        error!("REDIS_URL is required for resources browser proof validation");
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "browser_proof_store_unavailable",
        );
    };

    let client_ip = extract_client_ip(&headers);
    let Some(proof) = extract_browser_proof(&headers) else {
        warn!("Browser proof required for ip {} on {}", client_ip, path);
        return json_error(StatusCode::FORBIDDEN, "browser_proof_required");
    };

    match verify_browser_proof(store, proof).await {
        Ok(rate_limit_key) => {
            let limit = browser_rate_limit(&method);
            if let Some(retry_after) =
                check_rate_limit(rate_limit_key, limit, Duration::from_secs(60))
            {
                warn!("Browser proof rate limited on {}", path);
                return rate_limited(retry_after);
            }

            next.run(request).await
        }
        Err(BrowserProofError::Missing) => {
            warn!(
                "Invalid browser proof from ip {} on {}: proof is not present in shared store",
                client_ip, path
            );
            json_error(StatusCode::FORBIDDEN, "browser_proof_required")
        }
        Err(BrowserProofError::Store(error)) => {
            error!("Browser proof store unavailable: {}", error);
            json_error(StatusCode::SERVICE_UNAVAILABLE, "browser_proof_unavailable")
        }
    }
}

async fn verify_browser_proof(
    store: &RedisStore,
    token: &str,
) -> Result<String, BrowserProofError> {
    let key = store.hashed_key("browser-proof", token);
    let exists = store
        .get_string(&key)
        .await
        .map_err(BrowserProofError::Store)?
        .is_some();

    if exists {
        Ok(format!("browser-proof:{}", key))
    } else {
        Err(BrowserProofError::Missing)
    }
}

fn extract_browser_proof(headers: &HeaderMap) -> Option<&str> {
    header_str(headers, BROWSER_PROOF_HEADER)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| cookie_value(headers, BROWSER_PROOF_COOKIE))
}

fn has_static_auth_token(headers: &HeaderMap) -> bool {
    configured_static_auth_token()
        .as_deref()
        .is_some_and(|configured_token| matches_static_auth_token(headers, configured_token))
}

fn configured_static_auth_token() -> Option<String> {
    env_string(STATIC_AUTH_TOKEN_ENV).or_else(|| env_string(BETA_STATIC_AUTH_TOKEN_ENV))
}

fn matches_static_auth_token(headers: &HeaderMap, configured_token: &str) -> bool {
    extract_static_auth_token(headers).is_some_and(|token| token == configured_token)
}

fn extract_static_auth_token<'a>(headers: &'a HeaderMap) -> Option<&'a str> {
    header_str(headers, API_KEY_HEADER)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| bearer_token(headers))
}

fn bearer_token<'a>(headers: &'a HeaderMap) -> Option<&'a str> {
    let authorization = header_str(headers, "Authorization")?;
    let (scheme, token) = authorization.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return None;
    }

    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let cookie = header_str(headers, "Cookie")?;
    cookie.split(';').find_map(|part| {
        let (cookie_name, value) = part.trim().split_once('=')?;
        (cookie_name == name).then_some(value)
    })
}

fn should_skip_api_protection(method: &Method, path: &str) -> bool {
    *method == Method::OPTIONS || path == "/healthz" || path == "/resources/healthz"
}

fn api_protection_bypassed() -> bool {
    env_bool("API_PROTECTION_BYPASS")
}

fn browser_rate_limit(method: &Method) -> u32 {
    if *method == Method::GET || *method == Method::HEAD {
        env_u32("API_BROWSER_READS_PER_MINUTE", 120)
    } else {
        env_u32("API_BROWSER_WRITES_PER_MINUTE", 30)
    }
}

fn check_rate_limit(key: String, limit: u32, window: Duration) -> Option<u64> {
    if limit == 0 {
        return None;
    }

    let limits = RATE_LIMITS.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut limits) = limits.lock() else {
        return Some(1);
    };
    let now = Instant::now();

    if limits.len() > 10_000 {
        limits.retain(|_, value| value.reset_at > now);
    }

    if let Some(entry) = limits.get_mut(&key) {
        if now >= entry.reset_at {
            entry.count = 1;
            entry.reset_at = now + window;
            return None;
        }

        if entry.count >= limit {
            return Some(
                entry
                    .reset_at
                    .saturating_duration_since(now)
                    .as_secs()
                    .max(1),
            );
        }

        entry.count += 1;
        return None;
    }

    limits.insert(
        key,
        RateWindow {
            count: 1,
            reset_at: now + window,
        },
    );
    None
}

fn extract_client_ip(headers: &HeaderMap) -> String {
    if let Some(cf_ip) = header_str(headers, "CF-Connecting-IP") {
        return cf_ip.to_string();
    }

    if let Some(forwarded_for) = header_str(headers, "X-Forwarded-For") {
        if let Some(first_ip) = forwarded_for.split(',').next() {
            return first_ip.trim().to_string();
        }
    }

    if let Some(real_ip) = header_str(headers, "X-Real-IP") {
        return real_ip.to_string();
    }

    "unknown".to_string()
}

fn env_bool(name: &str) -> bool {
    std::env::var(name)
        .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
        .unwrap_or(false)
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(default)
}

fn json_error(status: StatusCode, error: &'static str) -> Response {
    (
        status,
        Json(ErrorBody {
            error,
            status: status.as_u16(),
        }),
    )
        .into_response()
}

fn rate_limited(retry_after: u64) -> Response {
    let mut response = json_error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    if let Ok(value) = HeaderValue::from_str(&retry_after.to_string()) {
        response.headers_mut().insert(RETRY_AFTER, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::{extract_static_auth_token, matches_static_auth_token};
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn extracts_static_auth_from_api_key_header() {
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", HeaderValue::from_static("beta-secret"));

        assert_eq!(extract_static_auth_token(&headers), Some("beta-secret"));
    }

    #[test]
    fn extracts_static_auth_from_bearer_header() {
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", HeaderValue::from_static("Bearer beta-secret"));

        assert_eq!(extract_static_auth_token(&headers), Some("beta-secret"));
    }

    #[test]
    fn ignores_non_bearer_authorization_header() {
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", HeaderValue::from_static("Basic beta-secret"));

        assert_eq!(extract_static_auth_token(&headers), None);
    }

    #[test]
    fn matches_only_configured_static_token() {
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", HeaderValue::from_static("beta-secret"));

        assert!(matches_static_auth_token(&headers, "beta-secret"));
        assert!(!matches_static_auth_token(&headers, "different-secret"));
    }
}

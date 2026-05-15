use axum::{
    body::Body,
    extract::State,
    http::{
        header::{RETRY_AFTER, SET_COOKIE},
        HeaderMap, HeaderValue, Method, Request, StatusCode,
    },
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tracing::{error, warn};

use crate::{redis_store::RedisStore, static_api::AppState};

const BROWSER_PROOF_COOKIE: &str = "uma_browser_proof";
const BROWSER_PROOF_HEADER: &str = "X-Browser-Proof";
const BROWSER_PROOF_TTL_HEADER: &str = "X-Browser-Proof-TTL";
const BROWSER_PROOF_AUDIENCE: &str = "uma-api";
const BROWSER_PROOF_TYPE: &str = "browser_proof";
const DEFAULT_TURNSTILE_ACTION: &str = "api_request";

static RATE_LIMITS: OnceLock<Mutex<HashMap<String, RateWindow>>> = OnceLock::new();
static PROOF_ISSUER_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrowserProofClaims {
    typ: String,
    jti: String,
    sub: String,
    uid: Option<String>,
    iat: usize,
    exp: usize,
    aud: String,
    action: String,
    host: String,
}

#[derive(Debug, Clone)]
struct IssuedBrowserProof {
    proof_header: HeaderValue,
    ttl_header: HeaderValue,
    set_cookie: Option<HeaderValue>,
}

#[derive(Debug)]
enum BrowserProofError {
    Invalid(String),
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

    let Some(store) = state.redis_store.as_ref() else {
        error!("REDIS_URL is required for resources browser proof validation");
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "browser_proof_store_unavailable",
        );
    };

    let client_ip = extract_client_ip(&headers);

    if let Some(proof) = extract_browser_proof(&headers) {
        match verify_browser_proof(store, proof).await {
            Ok(claims) => {
                let limit = browser_rate_limit(&method);
                if let Some(retry_after) = check_rate_limit(
                    format!("browser-proof:{}", claims.sub),
                    limit,
                    Duration::from_secs(60),
                ) {
                    warn!(
                        "Browser proof subject {} rate limited on {}",
                        claims.sub, path
                    );
                    return rate_limited(retry_after);
                }

                return next.run(request).await;
            }
            Err(BrowserProofError::Invalid(error)) => {
                warn!(
                    "Invalid browser proof from ip {} on {}: {}",
                    client_ip, path, error
                );
            }
            Err(BrowserProofError::Store(error)) => {
                error!("Browser proof store unavailable: {}", error);
                return json_error(StatusCode::SERVICE_UNAVAILABLE, "browser_proof_unavailable");
            }
        }
    }

    if can_bootstrap_browser_read(&method, &headers) {
        let limit = env_u32("API_BROWSER_BOOTSTRAP_READS_PER_MINUTE", 6);
        if let Some(retry_after) = check_rate_limit(
            format!("browser-bootstrap:{}", client_ip),
            limit,
            Duration::from_secs(60),
        ) {
            warn!(
                "Browser bootstrap lane rate limited for ip {} on {}",
                client_ip, path
            );
            return rate_limited(retry_after);
        }

        let issued_proof = match request_browser_proof(&headers).await {
            Ok(proof) => proof,
            Err(error) => {
                error!("Failed to request resources browser proof: {}", error);
                return json_error(StatusCode::SERVICE_UNAVAILABLE, "browser_proof_unavailable");
            }
        };

        let mut response = next.run(request).await;
        attach_browser_proof(&mut response, &issued_proof);
        return response;
    }

    warn!("Browser proof required for ip {} on {}", client_ip, path);
    json_error(StatusCode::FORBIDDEN, "browser_proof_required")
}

async fn request_browser_proof(headers: &HeaderMap) -> Result<IssuedBrowserProof, String> {
    let issuer_url = std::env::var("BROWSER_PROOF_ISSUER_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "BROWSER_PROOF_ISSUER_URL is not configured".to_string())?;
    let client = PROOF_ISSUER_CLIENT.get_or_init(reqwest::Client::new);
    let mut request = client.post(issuer_url);

    for name in [
        "Origin",
        "Referer",
        "Authorization",
        "CF-Connecting-IP",
        "X-Forwarded-For",
        "X-Real-IP",
        "User-Agent",
    ] {
        if let Some(value) = header_str(headers, name) {
            request = request.header(name, value);
        }
    }

    let response = request.send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("proof issuer returned {}", status.as_u16()));
    }

    let headers = response.headers();
    Ok(IssuedBrowserProof {
        proof_header: copy_response_header(headers, BROWSER_PROOF_HEADER)?,
        ttl_header: copy_response_header(headers, BROWSER_PROOF_TTL_HEADER)?,
        set_cookie: copy_optional_response_header(headers, "set-cookie")?,
    })
}

async fn verify_browser_proof(
    store: &RedisStore,
    token: &str,
) -> Result<BrowserProofClaims, BrowserProofError> {
    let key = store.hashed_key("browser-proof", token);
    let Some(payload) = store
        .get_string(&key)
        .await
        .map_err(BrowserProofError::Store)?
    else {
        return Err(BrowserProofError::Invalid(
            "proof is not present in shared store".to_string(),
        ));
    };

    let claims = serde_json::from_str::<BrowserProofClaims>(&payload)
        .map_err(|error| BrowserProofError::Invalid(error.to_string()))?;
    validate_browser_proof_claims(claims).map_err(BrowserProofError::Invalid)
}

fn validate_browser_proof_claims(claims: BrowserProofClaims) -> Result<BrowserProofClaims, String> {
    if claims.typ != BROWSER_PROOF_TYPE {
        return Err("wrong proof type".to_string());
    }
    if claims.aud != BROWSER_PROOF_AUDIENCE {
        return Err("wrong proof audience".to_string());
    }
    if claims.action != expected_turnstile_action() {
        return Err("wrong proof action".to_string());
    }
    if !allowed_turnstile_host(&claims.host) {
        return Err("wrong proof host".to_string());
    }
    let now = chrono::Utc::now().timestamp() as usize;
    if claims.exp <= now {
        return Err("expired proof".to_string());
    }

    Ok(claims)
}

fn attach_browser_proof(response: &mut Response, proof: &IssuedBrowserProof) {
    let headers = response.headers_mut();
    if let Some(cookie) = proof.set_cookie.as_ref() {
        headers.insert(SET_COOKIE, cookie.clone());
    }
    headers.insert(BROWSER_PROOF_HEADER, proof.proof_header.clone());
    headers.insert(BROWSER_PROOF_TTL_HEADER, proof.ttl_header.clone());
}

fn copy_response_header(
    headers: &reqwest::header::HeaderMap,
    name: &'static str,
) -> Result<HeaderValue, String> {
    let value = headers
        .get(name)
        .ok_or_else(|| format!("proof issuer response missing {}", name))?;
    HeaderValue::from_bytes(value.as_bytes()).map_err(|error| error.to_string())
}

fn copy_optional_response_header(
    headers: &reqwest::header::HeaderMap,
    name: &'static str,
) -> Result<Option<HeaderValue>, String> {
    headers
        .get(name)
        .map(|value| HeaderValue::from_bytes(value.as_bytes()).map_err(|error| error.to_string()))
        .transpose()
}

fn extract_browser_proof(headers: &HeaderMap) -> Option<&str> {
    header_str(headers, BROWSER_PROOF_HEADER)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| cookie_value(headers, BROWSER_PROOF_COOKIE))
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

fn can_bootstrap_browser_read(method: &Method, headers: &HeaderMap) -> bool {
    if *method != Method::GET && *method != Method::HEAD {
        return false;
    }

    if let Some(origin) = header_str(headers, "Origin") {
        return allowed_request_origin(origin);
    }

    header_str(headers, "Referer")
        .map(allowed_request_referer)
        .unwrap_or(false)
}

fn should_skip_api_protection(method: &Method, path: &str) -> bool {
    *method == Method::OPTIONS || path != "/resources/current/sql"
}

fn api_protection_bypassed() -> bool {
    env_bool("API_PROTECTION_BYPASS") || env_bool("TURNSTILE_BYPASS")
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

fn allowed_turnstile_host(hostname: &str) -> bool {
    let hostname = hostname.trim().to_ascii_lowercase();
    if hostname.is_empty() {
        return false;
    }

    allowed_hosts()
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(&hostname))
}

fn allowed_request_origin(origin: &str) -> bool {
    if let Ok(uri) = origin.parse::<axum::http::Uri>() {
        if let Some(host) = uri.host() {
            return allowed_turnstile_host(host);
        }
    }

    false
}

fn allowed_request_referer(referer: &str) -> bool {
    if let Ok(uri) = referer.parse::<axum::http::Uri>() {
        if let Some(host) = uri.host() {
            return allowed_turnstile_host(host);
        }
    }

    false
}

fn allowed_hosts() -> Vec<String> {
    std::env::var("TURNSTILE_ALLOWED_HOSTS")
        .unwrap_or_else(|_| {
            if is_development() {
                "uma.moe,www.uma.moe,beta.uma.moe,honse.moe,www.honse.moe,localhost,127.0.0.1"
                    .to_string()
            } else {
                "uma.moe,www.uma.moe,beta.uma.moe,honse.moe,www.honse.moe".to_string()
            }
        })
        .split(',')
        .map(|host| host.trim().to_ascii_lowercase())
        .filter(|host| !host.is_empty())
        .collect()
}

fn expected_turnstile_action() -> String {
    std::env::var("TURNSTILE_ACTION").unwrap_or_else(|_| DEFAULT_TURNSTILE_ACTION.to_string())
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

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(default)
}

fn is_development() -> bool {
    env_bool("DEBUG_MODE")
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
use axum::{
    body::Body,
    extract::State,
    http::{
        header::{CONTENT_TYPE, HOST, REFERER, SET_COOKIE},
        HeaderMap, Method, Request, StatusCode,
    },
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::{sync::OnceLock, time::Duration};
use tracing::{error, warn};

use crate::static_api::AppState;

const BROWSER_PROOF_COOKIE: &str = "uma_browser_proof";
const BROWSER_PROOF_HEADER: &str = "X-Browser-Proof";
const BROWSER_PROOF_TTL_HEADER: &str = "X-Browser-Proof-TTL";
const API_KEY_HEADER: &str = "X-API-Key";
const API_TOKEN_HEADER: &str = "X-API-Token";
const AUTH_INTERNAL_BASE_URL_ENV: &str = "AUTH_INTERNAL_BASE_URL";
const AUTH_VERIFY_INTERNAL_URL_ENV: &str = "AUTH_VERIFY_INTERNAL_URL";
const AUTH_BROWSER_PROOF_INTERNAL_URL_ENV: &str = "AUTH_BROWSER_PROOF_INTERNAL_URL";
const AUTH_INTERNAL_TIMEOUT_SECONDS_ENV: &str = "AUTH_INTERNAL_TIMEOUT_SECONDS";
const DEFAULT_AUTH_INTERNAL_BASE_URL: &str = "http://umamoe-backend:3201";

static AUTH_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

#[derive(Debug, Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
    status: u16,
}

#[derive(Debug, Serialize)]
struct AuthContext<'a> {
    method: &'a str,
    path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    referer: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    record_usage: Option<bool>,
}

#[derive(Debug, Serialize)]
struct BrowserProofRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    referer: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct AuthVerifyResponse {
    valid: bool,
    credential: Option<String>,
    message: Option<String>,
    error: Option<String>,
}

#[derive(Debug)]
enum Credential<'a> {
    ApiCredential {
        header_name: &'static str,
        value: &'a str,
    },
    BrowserProof(&'a str),
}

#[derive(Debug)]
enum AuthError {
    Invalid {
        status: StatusCode,
        error: &'static str,
        message: String,
    },
    Unavailable(String),
}

#[derive(Debug, Clone, Copy)]
enum ExpectedCredential {
    Api,
    BrowserProof,
}

pub async fn api_protection_middleware(
    State(_state): State<AppState>,
    headers: HeaderMap,
    method: Method,
    request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();

    if should_skip_api_protection(&method, &path) || api_protection_bypassed() {
        return next.run(request).await;
    }

    let context = request_context(&headers, &method, &path);

    if let Some(credential) = extract_api_credential(&headers) {
        return match verify_with_backend(credential, &context).await {
            Ok(()) => next.run(request).await,
            Err(error) => auth_error_response(error, &path),
        };
    }

    if let Some(proof) = extract_browser_proof(&headers) {
        return match verify_with_backend(Credential::BrowserProof(proof), &context).await {
            Ok(()) => next.run(request).await,
            Err(error) => auth_error_response(error, &path),
        };
    }

    if method == Method::GET || method == Method::HEAD {
        return match request_browser_proof(&context).await {
            Ok(proof_headers) => {
                let mut response = next.run(request).await;
                forward_browser_proof_headers(&proof_headers, response.headers_mut());
                response
            }
            Err(error) => auth_error_response(error, &path),
        };
    }

    warn!("Missing API credential or browser proof on {}", path);
    json_error(StatusCode::FORBIDDEN, "browser_proof_required")
}

async fn verify_with_backend(
    credential: Credential<'_>,
    context: &AuthContext<'_>,
) -> Result<(), AuthError> {
    let mut request = auth_client()
        .post(auth_verify_internal_url())
        .header(CONTENT_TYPE, "application/json")
        .json(context);

    let expected_credential = match credential {
        Credential::ApiCredential { header_name, value } => {
            request = request.header(header_name, value);
            ExpectedCredential::Api
        }
        Credential::BrowserProof(value) => {
            request = request.header(BROWSER_PROOF_HEADER, value);
            ExpectedCredential::BrowserProof
        }
    };

    let response = request
        .send()
        .await
        .map_err(|error| AuthError::Unavailable(error.to_string()))?;

    let status = response.status();
    let body = response
        .json::<AuthVerifyResponse>()
        .await
        .map_err(|error| AuthError::Unavailable(error.to_string()))?;

    if status != StatusCode::OK || !body.valid || !credential_matches(&body, expected_credential) {
        return Err(AuthError::Invalid {
            status: auth_failure_status(expected_credential, status),
            error: auth_failure_error(expected_credential),
            message: body
                .error
                .or(body.message)
                .unwrap_or_else(|| auth_failure_error(expected_credential).to_string()),
        });
    }

    Ok(())
}

async fn request_browser_proof(context: &AuthContext<'_>) -> Result<HeaderMap, AuthError> {
    let response = auth_client()
        .post(auth_browser_proof_internal_url())
        .header(CONTENT_TYPE, "application/json")
        .json(&BrowserProofRequest {
            origin: context.origin,
            referer: context.referer,
            host: context.host,
        })
        .send()
        .await
        .map_err(|error| AuthError::Unavailable(error.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        return Err(AuthError::Invalid {
            status: StatusCode::FORBIDDEN,
            error: "browser_proof_required",
            message: format!("browser proof bootstrap failed with {}", status),
        });
    }

    Ok(response.headers().clone())
}

fn request_context<'a>(
    headers: &'a HeaderMap,
    method: &'a Method,
    path: &'a str,
) -> AuthContext<'a> {
    AuthContext {
        method: method.as_str(),
        path,
        origin: header_str(headers, "Origin"),
        referer: header_str(headers, REFERER.as_str()),
        host: header_str(headers, HOST.as_str()),
        record_usage: extract_api_credential(headers).map(|_| true),
    }
}

fn extract_api_credential(headers: &HeaderMap) -> Option<Credential<'_>> {
    header_str(headers, API_KEY_HEADER)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| Credential::ApiCredential {
            header_name: API_KEY_HEADER,
            value,
        })
        .or_else(|| {
            header_str(headers, API_TOKEN_HEADER)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Credential::ApiCredential {
                    header_name: API_TOKEN_HEADER,
                    value,
                })
        })
}

fn extract_browser_proof(headers: &HeaderMap) -> Option<&str> {
    header_str(headers, BROWSER_PROOF_HEADER)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| cookie_value(headers, BROWSER_PROOF_COOKIE))
}

fn forward_browser_proof_headers(source: &HeaderMap, target: &mut HeaderMap) {
    for value in source.get_all(SET_COOKIE).iter() {
        target.append(SET_COOKIE, value.clone());
    }

    for header_name in [BROWSER_PROOF_HEADER, BROWSER_PROOF_TTL_HEADER] {
        for value in source.get_all(header_name).iter() {
            target.append(header_name, value.clone());
        }
    }
}

fn auth_client() -> &'static reqwest::Client {
    AUTH_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(env_u64(
                AUTH_INTERNAL_TIMEOUT_SECONDS_ENV,
                5,
            )))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

fn auth_verify_internal_url() -> String {
    env_string(AUTH_VERIFY_INTERNAL_URL_ENV)
        .unwrap_or_else(|| format!("{}/api/auth/verify/internal", auth_internal_base_url()))
}

fn auth_browser_proof_internal_url() -> String {
    env_string(AUTH_BROWSER_PROOF_INTERNAL_URL_ENV).unwrap_or_else(|| {
        format!(
            "{}/api/auth/browser-proof/internal",
            auth_internal_base_url()
        )
    })
}

fn auth_internal_base_url() -> String {
    env_string(AUTH_INTERNAL_BASE_URL_ENV)
        .unwrap_or_else(|| DEFAULT_AUTH_INTERNAL_BASE_URL.to_string())
        .trim_end_matches('/')
        .to_string()
}

fn credential_matches(body: &AuthVerifyResponse, expected_credential: ExpectedCredential) -> bool {
    match expected_credential {
        ExpectedCredential::Api => true,
        ExpectedCredential::BrowserProof => body.credential.as_deref() == Some("browser_proof"),
    }
}

fn auth_failure_status(
    expected_credential: ExpectedCredential,
    backend_status: StatusCode,
) -> StatusCode {
    if backend_status == StatusCode::UNAUTHORIZED
        || matches!(expected_credential, ExpectedCredential::Api)
    {
        StatusCode::UNAUTHORIZED
    } else {
        StatusCode::FORBIDDEN
    }
}

fn auth_failure_error(expected_credential: ExpectedCredential) -> &'static str {
    match expected_credential {
        ExpectedCredential::Api => "invalid_api_key",
        ExpectedCredential::BrowserProof => "browser_proof_required",
    }
}

fn auth_error_response(error: AuthError, path: &str) -> Response {
    match error {
        AuthError::Invalid {
            status,
            error,
            message,
        } => {
            warn!("Auth rejected on {}: {}", path, message);
            json_error(status, error)
        }
        AuthError::Unavailable(message) => {
            error!("Backend auth unavailable on {}: {}", path, message);
            json_error(StatusCode::SERVICE_UNAVAILABLE, "auth_unavailable")
        }
    }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let cookie = header_str(headers, "Cookie")?;
    cookie.split(';').find_map(|part| {
        let (cookie_name, value) = part.trim().split_once('=')?;
        (cookie_name == name && !value.trim().is_empty()).then_some(value)
    })
}

fn should_skip_api_protection(method: &Method, path: &str) -> bool {
    *method == Method::OPTIONS || path == "/healthz" || path == "/resources/healthz"
}

fn api_protection_bypassed() -> bool {
    env_bool("API_PROTECTION_BYPASS")
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

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
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

#[cfg(test)]
mod tests {
    use super::{
        extract_api_credential, extract_browser_proof, forward_browser_proof_headers,
        request_context, should_skip_api_protection, Credential,
    };
    use axum::http::{HeaderMap, HeaderValue, Method};

    #[test]
    fn extracts_api_key_header() {
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", HeaderValue::from_static("uma_k_test"));

        assert!(matches!(
            extract_api_credential(&headers),
            Some(Credential::ApiCredential {
                header_name: "X-API-Key",
                value: "uma_k_test"
            })
        ));
    }

    #[test]
    fn extracts_api_token_header() {
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Token", HeaderValue::from_static("uma_t_test"));

        assert!(matches!(
            extract_api_credential(&headers),
            Some(Credential::ApiCredential {
                header_name: "X-API-Token",
                value: "uma_t_test"
            })
        ));
    }

    #[test]
    fn extracts_browser_proof_header() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Browser-Proof", HeaderValue::from_static("proof"));

        assert_eq!(extract_browser_proof(&headers), Some("proof"));
    }

    #[test]
    fn extracts_browser_proof_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Cookie",
            HeaderValue::from_static("other=1; uma_browser_proof=proof-cookie"),
        );

        assert_eq!(extract_browser_proof(&headers), Some("proof-cookie"));
    }

    #[test]
    fn includes_usage_tracking_only_for_api_credentials() {
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", HeaderValue::from_static("uma_k_test"));
        headers.insert("Origin", HeaderValue::from_static("https://uma.moe"));
        headers.insert(
            "Referer",
            HeaderValue::from_static("https://uma.moe/resources"),
        );
        headers.insert("Host", HeaderValue::from_static("uma.moe"));

        let context = request_context(&headers, &Method::GET, "/resources/some-file.json");

        assert_eq!(context.method, "GET");
        assert_eq!(context.path, "/resources/some-file.json");
        assert_eq!(context.origin, Some("https://uma.moe"));
        assert_eq!(context.referer, Some("https://uma.moe/resources"));
        assert_eq!(context.host, Some("uma.moe"));
        assert_eq!(context.record_usage, Some(true));
    }

    #[test]
    fn forwards_browser_proof_bootstrap_headers() {
        let mut source = HeaderMap::new();
        let mut target = HeaderMap::new();
        source.insert(
            "Set-Cookie",
            HeaderValue::from_static("uma_browser_proof=proof; Path=/"),
        );
        source.append(
            "Set-Cookie",
            HeaderValue::from_static("uma_browser_proof_sig=sig; Path=/"),
        );
        source.insert("X-Browser-Proof", HeaderValue::from_static("proof"));
        source.insert("X-Browser-Proof-TTL", HeaderValue::from_static("300"));

        forward_browser_proof_headers(&source, &mut target);

        let cookies = target
            .get_all("Set-Cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect::<Vec<_>>();
        assert_eq!(
            cookies,
            vec![
                "uma_browser_proof=proof; Path=/",
                "uma_browser_proof_sig=sig; Path=/"
            ]
        );
        assert_eq!(
            target
                .get("X-Browser-Proof")
                .and_then(|value| value.to_str().ok()),
            Some("proof")
        );
        assert_eq!(
            target
                .get("X-Browser-Proof-TTL")
                .and_then(|value| value.to_str().ok()),
            Some("300")
        );
    }

    #[test]
    fn skips_api_protection_for_health_endpoints() {
        assert!(should_skip_api_protection(&Method::GET, "/healthz"));
        assert!(should_skip_api_protection(&Method::HEAD, "/healthz"));
        assert!(should_skip_api_protection(
            &Method::GET,
            "/resources/healthz"
        ));
        assert!(should_skip_api_protection(
            &Method::HEAD,
            "/resources/healthz"
        ));
    }
}

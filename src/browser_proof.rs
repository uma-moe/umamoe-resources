use axum::{
    body::Body,
    extract::State,
    http::{header::CONTENT_TYPE, HeaderMap, Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::{sync::OnceLock, time::Duration};
use tracing::{error, info, warn};

use crate::auth_common::{
    self, AuthRequestContext as AuthContext, BrowserProofRequest, Credential, BROWSER_PROOF_HEADER,
};
use crate::static_api::AppState;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct AuthVerifyResponse {
    valid: bool,
    credential: Option<String>,
    message: Option<String>,
    error: Option<String>,
    usage_recorded: Option<bool>,
    user_id: Option<String>,
    api_key: Option<AuthVerifyApiKey>,
    browser_proof: Option<AuthVerifyBrowserProof>,
}

#[derive(Debug, Deserialize)]
struct AuthVerifyApiKey {
    user_id: Option<String>,
    usage_recorded: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct AuthVerifyBrowserProof {
    subject: Option<String>,
    user_id: Option<String>,
    host: Option<String>,
    source: Option<String>,
    context_matches_proof: Option<bool>,
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

    if should_skip_api_protection(&method, &path) {
        return next.run(request).await;
    }

    let context = request_context(&headers, &method, &path);

    if api_protection_bypassed() {
        dry_run_api_protection(&headers, &method, &path, &context).await;
        return next.run(request).await;
    }

    if let Some(credential) = extract_api_credential(&headers) {
        return match verify_with_backend(credential, &context).await {
            Ok(_) => next.run(request).await,
            Err(error) => auth_error_response(error, &path),
        };
    }

    if let Some(proof) = extract_browser_proof(&headers) {
        return match verify_with_backend(Credential::BrowserProof(proof), &context).await {
            Ok(_) => next.run(request).await,
            Err(error) => auth_error_response(error, &path),
        };
    }

    if can_bootstrap_browser_read(&method, &path) {
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

pub async fn planner_protection_middleware(
    State(_state): State<AppState>,
    headers: HeaderMap,
    method: Method,
    request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    if !planner_source_allowed(&headers) {
        warn!(method = %method, path, "Rejected planner resource request from non-uma source");
        return json_error(StatusCode::FORBIDDEN, "planner_source_required");
    }
    if api_protection_bypassed() {
        return next.run(request).await;
    }

    let context = request_context(&headers, &method, &path);
    if let Some(proof) = extract_browser_proof(&headers) {
        return match verify_with_backend(Credential::BrowserProof(proof), &context).await {
            Ok(_) => next.run(request).await,
            Err(error) => auth_error_response(error, &path),
        };
    }

    if can_bootstrap_planner_manifest(&method, &path) {
        return match request_browser_proof(&context).await {
            Ok(proof_headers) => {
                let mut response = next.run(request).await;
                forward_browser_proof_headers(&proof_headers, response.headers_mut());
                response
            }
            Err(error) => auth_error_response(error, &path),
        };
    }

    json_error(StatusCode::FORBIDDEN, "planner_browser_proof_required")
}

fn can_bootstrap_planner_manifest(method: &Method, path: &str) -> bool {
    (*method == Method::GET || *method == Method::HEAD)
        && path == "/resources/planner/manifest.json"
}

fn planner_source_allowed(headers: &HeaderMap) -> bool {
    ["Origin", "Referer"]
        .iter()
        .filter_map(|name| auth_common::header_str(headers, name))
        .any(planner_source_url_allowed)
}

fn planner_source_url_allowed(value: &str) -> bool {
    let value = value.trim().trim_end_matches('/');
    for origin in [
        "https://uma.moe",
        "https://www.uma.moe",
        "https://beta.uma.moe",
    ] {
        if value == origin || value.starts_with(&format!("{origin}/")) {
            return true;
        }
    }
    if !env_bool("DEBUG_MODE") {
        return false;
    }
    let Some(rest) = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let host = authority.split(':').next().unwrap_or_default();
    matches!(host, "localhost" | "127.0.0.1")
}

fn can_bootstrap_browser_read(method: &Method, path: &str) -> bool {
    (*method == Method::GET || *method == Method::HEAD) && path == "/resources/manifest.json"
}

async fn dry_run_api_protection(
    headers: &HeaderMap,
    method: &Method,
    path: &str,
    context: &AuthContext,
) {
    log_auth_dry_run_request(context, headers);

    if let Some(credential) = extract_api_credential(headers) {
        let (header_name, token_len) = match &credential {
            Credential::ApiCredential { header_name, value } => (*header_name, value.len()),
            Credential::BrowserProof(_) => unreachable!("API extractor cannot return proof"),
        };

        info!(
            method = %context.method,
            path = %context.path,
            host = context.host.as_deref().unwrap_or("<none>"),
            header_name,
            token_len,
            "Resources auth dry-run verifying API credential"
        );

        match verify_with_backend(credential, context).await {
            Ok(response) => log_auth_dry_run_resolved("api", context, &response),
            Err(error) => log_auth_dry_run_error("api", path, error),
        }
        return;
    }

    if let Some(proof) = extract_browser_proof(headers) {
        info!(
            method = %context.method,
            path = %context.path,
            host = context.host.as_deref().unwrap_or("<none>"),
            proof_len = proof.len(),
            "Resources auth dry-run verifying browser proof"
        );

        match verify_with_backend(Credential::BrowserProof(proof), context).await {
            Ok(response) => log_auth_dry_run_resolved("browser_proof", context, &response),
            Err(error) => log_auth_dry_run_error("browser_proof", path, error),
        }
        return;
    }

    if *method == Method::GET || *method == Method::HEAD {
        if api_protection_dry_run_bootstrap() {
            info!(
                method = %context.method,
                path = %context.path,
                host = context.host.as_deref().unwrap_or("<none>"),
                "Resources auth dry-run requesting browser proof bootstrap"
            );
            match request_browser_proof(context).await {
                Ok(headers) => info!(
                    method = %context.method,
                    path = %context.path,
                    set_cookie_count = headers.get_all("Set-Cookie").iter().count(),
                    has_proof_header = headers.get(BROWSER_PROOF_HEADER).is_some(),
                    proof_source = headers
                        .get(auth_common::BROWSER_PROOF_SOURCE_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("<none>"),
                    "Resources auth dry-run browser proof bootstrap would succeed"
                ),
                Err(error) => log_auth_dry_run_error("browser_proof_bootstrap", path, error),
            }
        } else {
            info!(
                method = %context.method,
                path = %context.path,
                host = context.host.as_deref().unwrap_or("<none>"),
                "Resources auth dry-run would bootstrap browser proof; backend bootstrap call skipped"
            );
        }
        return;
    }

    warn!(
        method = %method,
        path,
        "Resources auth dry-run would reject request: missing API credential or browser proof"
    );
}

async fn verify_with_backend(
    credential: Credential<'_>,
    context: &AuthContext,
) -> Result<AuthVerifyResponse, AuthError> {
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

    Ok(body)
}

async fn request_browser_proof(context: &AuthContext) -> Result<HeaderMap, AuthError> {
    let response = auth_client()
        .post(auth_browser_proof_internal_url())
        .header(CONTENT_TYPE, "application/json")
        .json(&BrowserProofRequest {
            origin: context.origin.as_deref(),
            referer: context.referer.as_deref(),
            host: context.host.as_deref(),
            client_ip: context.client_ip.as_deref(),
            user_agent: context.user_agent.as_deref(),
            warmup_marker: context.warmup_marker.as_deref(),
        })
        .send()
        .await
        .map_err(|error| AuthError::Unavailable(error.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        let is_rate_limited = status == StatusCode::TOO_MANY_REQUESTS;
        return Err(AuthError::Invalid {
            status: if is_rate_limited {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::FORBIDDEN
            },
            error: if is_rate_limited {
                "rate_limited"
            } else {
                "browser_proof_required"
            },
            message: format!("browser proof bootstrap failed with {}", status),
        });
    }

    Ok(response.headers().clone())
}

fn request_context(headers: &HeaderMap, method: &Method, path: &str) -> AuthContext {
    auth_common::request_context(headers, method, path)
}

fn extract_api_credential(headers: &HeaderMap) -> Option<Credential<'_>> {
    auth_common::extract_api_credential(headers)
}

fn extract_browser_proof(headers: &HeaderMap) -> Option<&str> {
    auth_common::extract_browser_proof(headers)
}

fn forward_browser_proof_headers(source: &HeaderMap, target: &mut HeaderMap) {
    auth_common::forward_browser_proof_headers(source, target);
}

#[cfg(test)]
fn browser_context_host(headers: &HeaderMap) -> Option<String> {
    auth_common::browser_context_host(headers)
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

fn log_auth_dry_run_request(context: &AuthContext, headers: &HeaderMap) {
    info!(
        method = %context.method,
        path = %context.path,
        origin = context.origin.as_deref().unwrap_or("<none>"),
        referer = context.referer.as_deref().unwrap_or("<none>"),
        host = context.host.as_deref().unwrap_or("<none>"),
        has_authorization = headers.contains_key(auth_common::AUTHORIZATION_HEADER),
        has_bearer = auth_common::extract_bearer_token(headers).is_some(),
        has_api_credential = auth_common::extract_api_credential(headers).is_some(),
        has_browser_proof = auth_common::extract_browser_proof(headers).is_some(),
        "Resources auth dry-run request"
    );
}

fn log_auth_dry_run_resolved(
    expected_credential: &'static str,
    context: &AuthContext,
    response: &AuthVerifyResponse,
) {
    info!(
        method = %context.method,
        path = %context.path,
        expected_credential,
        valid = response.valid,
        credential = response.credential.as_deref().unwrap_or("<none>"),
        message = response.message.as_deref().unwrap_or("<none>"),
        error = response.error.as_deref().unwrap_or("<none>"),
        user_id = response.resolved_user_id().unwrap_or("<none>"),
        usage_recorded = response.resolved_usage_recorded(),
        proof_source = response.proof_source().unwrap_or("<none>"),
        proof_subject = response.proof_subject().unwrap_or("<none>"),
        proof_host = response.proof_host().unwrap_or("<none>"),
        context_matches_proof = response.context_matches_proof(),
        "Resources auth dry-run resolved backend credential"
    );
}

fn log_auth_dry_run_error(credential: &'static str, path: &str, error: AuthError) {
    match error {
        AuthError::Invalid {
            status,
            error,
            message,
        } => warn!(
            path,
            credential,
            status = status.as_u16(),
            auth_error = error,
            message,
            "Resources auth dry-run would reject request"
        ),
        AuthError::Unavailable(message) => error!(
            path,
            credential, message, "Resources auth dry-run backend unavailable"
        ),
    }
}

impl AuthVerifyResponse {
    fn resolved_user_id(&self) -> Option<&str> {
        self.api_key
            .as_ref()
            .and_then(|api_key| api_key.user_id.as_deref())
            .or_else(|| {
                self.browser_proof
                    .as_ref()
                    .and_then(|proof| proof.user_id.as_deref())
            })
            .or(self.user_id.as_deref())
    }

    fn resolved_usage_recorded(&self) -> bool {
        self.api_key
            .as_ref()
            .and_then(|api_key| api_key.usage_recorded)
            .or(self.usage_recorded)
            .unwrap_or(false)
    }

    fn proof_source(&self) -> Option<&str> {
        self.browser_proof
            .as_ref()
            .and_then(|proof| proof.source.as_deref())
    }

    fn proof_subject(&self) -> Option<&str> {
        self.browser_proof
            .as_ref()
            .and_then(|proof| proof.subject.as_deref())
    }

    fn proof_host(&self) -> Option<&str> {
        self.browser_proof
            .as_ref()
            .and_then(|proof| proof.host.as_deref())
    }

    fn context_matches_proof(&self) -> Option<bool> {
        self.browser_proof
            .as_ref()
            .and_then(|proof| proof.context_matches_proof)
    }
}

fn should_skip_api_protection(method: &Method, path: &str) -> bool {
    auth_common::should_skip_api_protection(method, path, &["/resources/healthz"])
}

fn api_protection_bypassed() -> bool {
    env_bool("API_PROTECTION_BYPASS")
}

fn api_protection_dry_run_bootstrap() -> bool {
    env_bool_default("API_PROTECTION_DRY_RUN_BOOTSTRAP", true)
}

fn env_bool(name: &str) -> bool {
    env_bool_default(name, false)
}

fn env_bool_default(name: &str, default: bool) -> bool {
    std::env::var(name)
        .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
        .unwrap_or(default)
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
            message: error_message(error),
        }),
    )
        .into_response()
}

fn error_message(error: &'static str) -> Option<&'static str> {
    match error {
        "browser_proof_required" => Some(
            "This endpoint requires a browser proof. Browser clients should wait for the Turnstile/browser-proof exchange and retry. Bots, scripts, and integrations should use an API key instead; API keys can be generated from your Uma account at any time.",
        ),
        "planner_source_required" => Some(
            "Planner resources are available only to requests originating from the uma.moe frontend.",
        ),
        "planner_browser_proof_required" => Some(
            "Planner resources require a valid browser proof from the uma.moe frontend.",
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        browser_context_host, can_bootstrap_browser_read, can_bootstrap_planner_manifest,
        extract_api_credential, extract_browser_proof, forward_browser_proof_headers,
        planner_source_url_allowed, request_context, should_skip_api_protection, Credential,
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
        assert_eq!(context.origin.as_deref(), Some("https://uma.moe"));
        assert_eq!(
            context.referer.as_deref(),
            Some("https://uma.moe/resources")
        );
        assert_eq!(context.host.as_deref(), Some("uma.moe"));
        assert_eq!(context.record_usage, Some(true));
    }

    #[test]
    fn derives_browser_context_host_from_origin_before_service_host() {
        let mut headers = HeaderMap::new();
        headers.insert("Origin", HeaderValue::from_static("https://uma.moe"));
        headers.insert("Host", HeaderValue::from_static("umamoe-resources"));

        assert_eq!(browser_context_host(&headers).as_deref(), Some("uma.moe"));
    }

    #[test]
    fn derives_browser_context_host_from_original_host_without_origin() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Original-Host", HeaderValue::from_static("beta.uma.moe"));
        headers.insert("Host", HeaderValue::from_static("umamoe-resources"));

        assert_eq!(
            browser_context_host(&headers).as_deref(),
            Some("beta.uma.moe")
        );
    }

    #[test]
    fn derives_browser_context_host_from_forwarded_host_without_origin() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-Host", HeaderValue::from_static("uma.moe"));
        headers.insert("Host", HeaderValue::from_static("umamoe-resources"));

        assert_eq!(browser_context_host(&headers).as_deref(), Some("uma.moe"));
    }

    #[test]
    fn omits_internal_host_without_browser_context() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-Host", HeaderValue::from_static("umamoe-proxy"));
        headers.insert("Host", HeaderValue::from_static("umamoe-resources"));

        assert_eq!(browser_context_host(&headers), None);
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
        source.insert(
            "X-Browser-Proof-Source",
            HeaderValue::from_static("turnstile"),
        );

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
        assert_eq!(
            target
                .get("X-Browser-Proof-Source")
                .and_then(|value| value.to_str().ok()),
            Some("turnstile")
        );
    }

    #[test]
    fn skips_api_protection_for_health_endpoints() {
        assert!(should_skip_api_protection(&Method::GET, "/healthz"));
        assert!(should_skip_api_protection(&Method::HEAD, "/healthz"));
        assert!(should_skip_api_protection(&Method::GET, "/health"));
        assert!(should_skip_api_protection(&Method::HEAD, "/health"));
        assert!(should_skip_api_protection(
            &Method::GET,
            "/resources/healthz"
        ));
        assert!(should_skip_api_protection(
            &Method::HEAD,
            "/resources/healthz"
        ));
    }

    #[test]
    fn bootstraps_only_resource_manifest() {
        assert!(can_bootstrap_browser_read(
            &Method::GET,
            "/resources/manifest.json"
        ));
        assert!(can_bootstrap_browser_read(
            &Method::HEAD,
            "/resources/manifest.json"
        ));
        assert!(!can_bootstrap_browser_read(
            &Method::GET,
            "/resources/current/factors.json.gz"
        ));
        assert!(!can_bootstrap_browser_read(
            &Method::POST,
            "/resources/manifest.json"
        ));
    }

    #[test]
    fn planner_source_allows_only_uma_production_hosts() {
        assert!(planner_source_url_allowed("https://uma.moe"));
        assert!(planner_source_url_allowed("https://www.uma.moe/timeline"));
        assert!(planner_source_url_allowed("https://beta.uma.moe/timeline"));
        assert!(!planner_source_url_allowed("https://honse.moe/timeline"));
        assert!(!planner_source_url_allowed("https://uma.moe.attacker.test"));
    }

    #[test]
    fn planner_bootstrap_is_limited_to_its_manifest() {
        assert!(can_bootstrap_planner_manifest(
            &Method::GET,
            "/resources/planner/manifest.json"
        ));
        assert!(!can_bootstrap_planner_manifest(
            &Method::GET,
            "/resources/planner/current/planner_core.json.gz"
        ));
    }
}

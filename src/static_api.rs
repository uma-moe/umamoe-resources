use crate::pipeline::{read_manifest, read_version_manifest, ResourceManifest};
use anyhow::Result;
use axum::extract::{Path, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, ETAG, VARY,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use tokio::fs;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

#[derive(Clone)]
struct AppState {
    data_dir: PathBuf,
}

pub async fn serve(data_dir: PathBuf, bind: SocketAddr) -> Result<()> {
    let state = AppState { data_dir };
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/resources", get(get_manifest))
        .route("/resources/", get(get_manifest))
        .route("/resources/healthz", get(healthz))
        .route("/resources/manifest.json", get(get_manifest))
        .route("/resources/current/:file_name", get(get_current_resource))
        .route(
            "/resources/:version/:file_name",
            get(get_versioned_resource),
        )
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    info!(address = %bind, "serving generated resources");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> &'static str {
    "ok"
}

async fn get_manifest(State(state): State<AppState>) -> Result<Response, ApiError> {
    let manifest_path = state.data_dir.join("manifest.json");
    let manifest_bytes = fs::read(&manifest_path).await.map_err(|error| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            format!("failed to read {}: {}", manifest_path.display(), error),
        )
    })?;

    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60, s-maxage=300, stale-while-revalidate=86400"),
    );
    headers.insert(CONTENT_LENGTH, header_value(manifest_bytes.len())?);
    Ok((headers, manifest_bytes).into_response())
}

async fn get_current_resource(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
) -> Result<Response, ApiError> {
    let manifest = read_manifest(&state.data_dir).map_err(internal_error)?;
    resource_response(
        &state.data_dir,
        &manifest,
        &manifest.version,
        &file_name,
        false,
    )
    .await
}

async fn get_versioned_resource(
    State(state): State<AppState>,
    Path((version, file_name)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let current_manifest = read_manifest(&state.data_dir).map_err(internal_error)?;
    let manifest = if version == current_manifest.version {
        current_manifest
    } else {
        read_version_manifest(&state.data_dir, &version).map_err(internal_error)?
    };
    resource_response(&state.data_dir, &manifest, &version, &file_name, true).await
}

async fn resource_response(
    data_dir: &FsPath,
    manifest: &ResourceManifest,
    version: &str,
    file_name: &str,
    immutable: bool,
) -> Result<Response, ApiError> {
    let logical_file_name = normalize_file_name(file_name)?;
    let artifact = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.name == logical_file_name)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("unknown resource {}", file_name),
            )
        })?;

    let gzip_path = data_dir
        .join(version)
        .join(format!("{}.gz", logical_file_name));
    let gzip_bytes = fs::read(&gzip_path).await.map_err(|error| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            format!("failed to read {}: {}", gzip_path.display(), error),
        )
    })?;

    let cache_control = if immutable {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=60, s-maxage=300, stale-while-revalidate=86400"
    };

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, header_value(&artifact.content_type)?);
    headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache_control));
    headers.insert(ETAG, header_value(&artifact.etag)?);
    headers.insert(VARY, HeaderValue::from_static("Accept-Encoding"));
    headers.insert(CONTENT_LENGTH, header_value(gzip_bytes.len())?);
    Ok((headers, gzip_bytes).into_response())
}

fn normalize_file_name(file_name: &str) -> Result<&str, ApiError> {
    let valid = file_name.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
    });
    if !valid {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid resource file name",
        ));
    }

    if let Some(logical_file_name) = file_name.strip_suffix(".json.gz") {
        return Ok(&file_name[..logical_file_name.len() + ".json".len()]);
    }

    if file_name.ends_with(".json") {
        return Ok(file_name);
    }

    Err(ApiError::new(
        StatusCode::BAD_REQUEST,
        "invalid resource file name",
    ))
}

fn header_value(value: impl ToString) -> Result<HeaderValue, ApiError> {
    HeaderValue::from_str(&value.to_string()).map_err(internal_error)
}

fn internal_error(error: impl std::fmt::Display) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

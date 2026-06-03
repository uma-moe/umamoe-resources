use crate::pipeline::{read_manifest, read_version_manifest, ResourceManifest};
use anyhow::{Context, Result};
use axum::extract::{Path, Query, State};
use axum::http::header::{
    ACCEPT, AUTHORIZATION, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, ETAG,
    ORIGIN, REFERER, VARY,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use tokio::fs;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

const MAX_SQL_ROWS: usize = 1_000;
const MAX_SQL_LENGTH: usize = 16_384;

#[derive(Clone)]
pub(crate) struct AppState {
    data_dir: PathBuf,
    master_path: PathBuf,
}

pub async fn serve(data_dir: PathBuf, master_path: PathBuf, bind: SocketAddr) -> Result<()> {
    let state = AppState {
        data_dir,
        master_path,
    };

    if let Some(internal_port) = internal_resources_port() {
        let internal_host =
            std::env::var("RESOURCES_INTERNAL_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let internal_bind = format!("{}:{}", internal_host, internal_port)
            .parse::<SocketAddr>()
            .context("failed to parse RESOURCES_INTERNAL_HOST/RESOURCES_INTERNAL_PORT")?;
        let internal_app = resource_router(state.clone(), false);
        tokio::spawn(async move {
            match tokio::net::TcpListener::bind(internal_bind).await {
                Ok(listener) => {
                    info!(address = %internal_bind, "serving internal generated resources");
                    if let Err(error) = axum::serve(listener, internal_app).await {
                        tracing::error!("internal resources server stopped: {}", error);
                    }
                }
                Err(error) => {
                    tracing::error!("failed to bind internal resources server: {}", error)
                }
            }
        });
    }

    let app = resource_router(state, true);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    info!(address = %bind, "serving generated resources");
    axum::serve(listener, app).await?;
    Ok(())
}

fn resource_router(state: AppState, protected: bool) -> Router {
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/resources", get(get_manifest))
        .route("/resources/", get(get_manifest))
        .route("/resources/healthz", get(healthz))
        .route("/resources/manifest.json", get(get_manifest))
        .route("/resources/current/sql", get(get_current_sql))
        .route("/resources/current/:file_name", get(get_current_resource))
        .route(
            "/resources/:version/:file_name",
            get(get_versioned_resource),
        );

    let app = if protected {
        app.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::browser_proof::api_protection_middleware,
        ))
    } else {
        app
    };

    app.layer(cors_layer())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn internal_resources_port() -> Option<u16> {
    std::env::var("RESOURCES_INTERNAL_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
}

fn cors_layer() -> CorsLayer {
    let origins: Vec<HeaderValue> = allowed_origins()
        .into_iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();

    CorsLayer::new()
        .allow_origin(origins)
        .allow_credentials(true)
        .allow_methods([Method::GET, Method::HEAD, Method::OPTIONS])
        .allow_headers([
            CONTENT_TYPE,
            AUTHORIZATION,
            ACCEPT,
            REFERER,
            ORIGIN,
            "X-Browser-Proof".parse().unwrap(),
            "X-API-Key".parse().unwrap(),
            "X-API-Token".parse().unwrap(),
        ])
        .expose_headers([
            "Set-Cookie".parse().unwrap(),
            "X-Browser-Proof".parse().unwrap(),
            "X-Browser-Proof-TTL".parse().unwrap(),
        ])
}

fn allowed_origins() -> Vec<String> {
    std::env::var("ALLOWED_ORIGINS")
        .unwrap_or_else(|_| {
            if std::env::var("DEBUG_MODE").unwrap_or_default() == "true" {
                "http://localhost:4200,http://localhost:3000,http://127.0.0.1:4200".to_string()
            } else {
                "https://uma.moe,https://www.uma.moe,https://beta.uma.moe,https://honse.moe,https://www.honse.moe".to_string()
            }
        })
        .split(',')
        .map(|origin| origin.trim().to_string())
        .filter(|origin| !origin.is_empty())
        .collect()
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

#[derive(Deserialize)]
struct SqlQueryRequest {
    sql: String,
}

#[derive(Serialize)]
struct SqlQueryResponse {
    columns: Vec<String>,
    rows: Vec<Vec<serde_json::Value>>,
    truncated: bool,
}

async fn get_current_sql(
    State(state): State<AppState>,
    Query(query): Query<SqlQueryRequest>,
) -> Result<Response, ApiError> {
    validate_read_only_sql(&query.sql)?;

    let master_path = state.master_path.clone();
    let sql = query.sql;
    let response = tokio::task::spawn_blocking(move || execute_sql_query(&master_path, &sql))
        .await
        .map_err(|error| internal_error(format!("failed to join SQL task: {error}")))?
        .map_err(internal_error)?;

    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );

    Ok((headers, Json(response)).into_response())
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
    let valid = file_name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'));
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

fn validate_read_only_sql(sql: &str) -> Result<(), ApiError> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "missing sql query"));
    }

    if trimmed.len() > MAX_SQL_LENGTH {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("sql query exceeds {} characters", MAX_SQL_LENGTH),
        ));
    }

    let without_trailing_semicolon = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
    if without_trailing_semicolon.contains(';') {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "only a single SQL statement is allowed",
        ));
    }

    let first_keyword = without_trailing_semicolon
        .split(|character: char| !character.is_ascii_alphabetic())
        .find(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "missing sql query"))?;

    if !matches!(first_keyword.as_str(), "select" | "with") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "only SELECT or WITH queries are allowed",
        ));
    }

    Ok(())
}

fn execute_sql_query(master_path: &FsPath, sql: &str) -> Result<SqlQueryResponse> {
    let connection = Connection::open_with_flags(master_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement = connection.prepare(sql)?;

    if statement.column_count() == 0 {
        anyhow::bail!("SQL query must return rows");
    }

    let columns = statement
        .column_names()
        .into_iter()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut rows = statement.query([])?;
    let mut values = Vec::new();
    let mut truncated = false;

    while let Some(row) = rows.next()? {
        if values.len() >= MAX_SQL_ROWS {
            truncated = true;
            break;
        }

        let mut output_row = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            output_row.push(sqlite_value_to_json(row.get_ref(index)?));
        }
        values.push(output_row);
    }

    Ok(SqlQueryResponse {
        columns,
        rows: values,
        truncated,
    })
}

fn sqlite_value_to_json(value: ValueRef<'_>) -> serde_json::Value {
    match value {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(value) => serde_json::Value::from(value),
        ValueRef::Real(value) => serde_json::Value::from(value),
        ValueRef::Text(value) => serde_json::Value::String(String::from_utf8_lossy(value).into()),
        ValueRef::Blob(value) => serde_json::json!({ "blob_hex": hex::encode(value) }),
    }
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

#[cfg(test)]
mod tests {
    use super::validate_read_only_sql;

    #[test]
    fn allows_select_queries() {
        assert!(validate_read_only_sql("SELECT 1").is_ok());
        assert!(validate_read_only_sql("WITH x AS (SELECT 1) SELECT * FROM x").is_ok());
    }

    #[test]
    fn rejects_non_select_queries() {
        assert!(validate_read_only_sql("INSERT INTO card_data VALUES (1)").is_err());
        assert!(validate_read_only_sql("DELETE FROM card_data").is_err());
    }

    #[test]
    fn rejects_multiple_statements() {
        assert!(validate_read_only_sql("SELECT 1; SELECT 2").is_err());
    }
}

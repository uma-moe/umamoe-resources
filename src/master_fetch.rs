#![allow(non_snake_case)]

use crate::game_cdn::{self, GLOBAL_CDN};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::info;

#[path = "types/master_fetch.rs"]
mod types;
use types::{GameVersion, VERSION_URL};

pub(crate) async fn fetchMasterMdb(resource_version: &str, output_path: &Path) -> Result<()> {
    let client = reqwest::Client::new();

    info!("Fetching manifests for version {}...", resource_version);
    let categories =
        game_cdn::fetchCategories(&client, &GLOBAL_CDN, game_cdn::WINDOWS, resource_version)
            .await?;
    let master_entries =
        game_cdn::fetchCategoryFiles(&client, &GLOBAL_CDN, &categories, &["master"]).await?;
    let mdb_entry = master_entries
        .iter()
        .find(|e| e.name.to_lowercase().contains("master.mdb"))
        .context("master.mdb entry not found in master manifest")?;

    info!("Downloading master.mdb...");
    let mdb_compressed = game_cdn::downloadGeneric(&client, &GLOBAL_CDN, mdb_entry).await?;
    let mdb_data = game_cdn::decompressLz4(&mdb_compressed)?;

    anyhow::ensure!(
        mdb_data.starts_with(b"SQLite format 3\0"),
        "downloaded master is not a SQLite database"
    );
    if let Some(parent) = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create parent directory for {}",
                output_path.display()
            )
        })?;
    }
    let pending_path = output_path.with_extension("mdb.download");
    std::fs::write(&pending_path, &mdb_data)
        .with_context(|| format!("failed to write master.mdb to {}", pending_path.display()))?;
    {
        let connection = rusqlite::Connection::open_with_flags(
            &pending_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let course_count: u64 =
            connection.query_row("SELECT COUNT(*) FROM race_course_set", [], |row| row.get(0))?;
        anyhow::ensure!(course_count > 0, "downloaded master has no race courses");
    }
    std::fs::rename(&pending_path, output_path)
        .with_context(|| format!("failed to replace master at {}", output_path.display()))?;
    info!(
        "Saved master.mdb: {} bytes ({:.2} MB)",
        mdb_data.len(),
        mdb_data.len() as f64 / (1024.0 * 1024.0)
    );

    Ok(())
}

/// Check the current Global game version on every call. Failed checks are
/// errors, so an old local marker cannot be mistaken for the current version.
pub async fn maybeUpdateMasterMdb(mdb_path: &Path) -> Result<bool> {
    let version = fetchGameVersion(VERSION_URL).await?;
    let combined = versionMarker(&version)?;

    let marker_path = markerPath(mdb_path);
    let current = std::fs::read_to_string(&marker_path).unwrap_or_default();
    if current.trim() == combined && mdb_path.is_file() {
        info!(
            "master.mdb is up to date (app_version: {}, resource_version: {})",
            version.app_version, version.resource_version
        );
        return Ok(false);
    }

    info!(
        "Updating master.mdb: '{}' -> '{}'",
        current.trim(),
        combined
    );
    fetchMasterMdb(version.resource_version.trim(), mdb_path).await?;
    std::fs::write(&marker_path, &combined).with_context(|| {
        format!(
            "failed to write master version marker to {}",
            marker_path.display()
        )
    })?;
    Ok(true)
}

/// The resource version of the local master.mdb, from the marker written after it was saved.
pub(crate) fn localResourceVersion(mdb_path: &Path) -> Option<String> {
    let marker = std::fs::read_to_string(markerPath(mdb_path)).ok()?;
    let (_, resource_version) = marker.trim().split_once(':')?;
    Some(resource_version.to_string())
}

/// The version marker next to master.mdb.
fn markerPath(mdb_path: &Path) -> PathBuf {
    let mut marker_name = mdb_path.as_os_str().to_os_string();
    marker_name.push(".version");
    PathBuf::from(marker_name)
}

async fn fetchGameVersion(url: &str) -> Result<GameVersion> {
    let version: GameVersion = reqwest::Client::new()
        .get(url)
        .header("Cache-Control", "no-cache")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .context("failed to check current Global game version")?
        .error_for_status()
        .context("Global game version endpoint returned an error")?
        .json()
        .await
        .context("invalid Global game version response")?;
    versionMarker(&version)?;
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn checksVersionEachTimeAndRejectsFailedOrInvalidResponses() {
        use axum::{routing::get, Json, Router};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let app = Router::new()
            .route("/version", get(move || {
                let observed = observed.clone();
                async move {
                    let call = observed.fetch_add(1, Ordering::SeqCst);
                    Json(serde_json::json!({"app_version": "1.35.2", "resource_version": (10008010 + call).to_string()}))
                }
            }))
            .route("/invalid", get(|| async { Json(serde_json::json!({"app_version": "", "resource_version": "../old"})) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let url = format!("http://{address}/version");
        assert_eq!(
            versionMarker(&fetchGameVersion(&url).await.unwrap()).unwrap(),
            "1.35.2:10008010"
        );
        assert_eq!(
            versionMarker(&fetchGameVersion(&url).await.unwrap()).unwrap(),
            "1.35.2:10008011"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(fetchGameVersion(&format!("http://{address}/invalid"))
            .await
            .is_err());
        assert!(fetchGameVersion(&format!("http://{address}/missing"))
            .await
            .is_err());
        server.abort();
    }
}

fn versionMarker(version: &GameVersion) -> Result<String> {
    let app_version = version.app_version.trim();
    let resource_version = version.resource_version.trim();
    anyhow::ensure!(
        app_version.split('.').count() == 3
            && app_version
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            && !resource_version.is_empty()
            && resource_version.bytes().all(|byte| byte.is_ascii_digit()),
        "invalid current Global game version"
    );
    Ok(format!("{app_version}:{resource_version}"))
}

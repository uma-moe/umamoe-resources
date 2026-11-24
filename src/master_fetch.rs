#![allow(non_snake_case)]

use crate::game_cdn::{self, CdnSource};
use anyhow::{Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};
use tokio::task::JoinSet;
use tracing::{info, warn};

#[path = "types/master_fetch.rs"]
mod types;
pub use types::{EarlyMaster, EarlyMasterMeta};
use types::{
    GameVersion, EARLY_PROBE_CONCURRENCY, EARLY_PROBE_MAX_STEPS, EARLY_PROBE_STEP, VERSION_URL,
    VERSION_URL_VARIABLE,
};

pub(crate) async fn fetchMasterMdb(
    source: &CdnSource,
    resource_version: &str,
    output_path: &Path,
) -> Result<()> {
    let client = reqwest::Client::new();

    info!(
        region = source.region,
        "Fetching manifests for version {}...", resource_version
    );
    let categories =
        game_cdn::fetchCategories(&client, source, game_cdn::WINDOWS, resource_version).await?;
    let master_entries =
        game_cdn::fetchCategoryFiles(&client, source, &categories, &["master"]).await?;
    let mdb_entry = master_entries
        .iter()
        .find(|e| e.name.to_lowercase().contains("master.mdb"))
        .context("master.mdb entry not found in master manifest")?;

    info!(region = source.region, "Downloading master.mdb...");
    let mdb_compressed = game_cdn::downloadGeneric(&client, source, mdb_entry).await?;
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
        region = source.region,
        "Saved master.mdb: {} bytes ({:.2} MB)",
        mdb_data.len(),
        mdb_data.len() as f64 / (1024.0 * 1024.0)
    );

    Ok(())
}

/// Check the region's current game version on every call and download its master when it is
/// newer than the local one. Failed checks are errors, so an old local marker cannot be mistaken
/// for the current version.
pub async fn maybeUpdateMasterMdb(source: &CdnSource, mdb_path: &Path) -> Result<bool> {
    let version = fetchGameVersion(source, &versionUrl(source)).await?;
    let combined = versionMarker(source, &version)?;

    let marker_path = markerPath(mdb_path);
    let current = std::fs::read_to_string(&marker_path).unwrap_or_default();
    if current.trim() == combined && mdb_path.is_file() {
        info!(
            region = source.region,
            "master.mdb is up to date (app_version: {}, resource_version: {})",
            version.app_version,
            version.resource_version
        );
        return Ok(false);
    }

    info!(
        region = source.region,
        "Updating master.mdb: '{}' -> '{}'",
        current.trim(),
        combined
    );
    fetchMasterMdb(source, version.resource_version.trim(), mdb_path).await?;
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

/// Where the JP master is kept: beside the Global one, as `jp-master.mdb` next to `master.mdb`.
pub(crate) fn jpMasterPath(master: &Path) -> PathBuf {
    let name = master.file_name().unwrap_or_default().to_string_lossy();
    master.with_file_name(format!("jp-{name}"))
}

/// Where an unreleased Global master staged ahead of the live app version is kept: beside
/// `master.mdb` as `early-master.mdb`.
pub(crate) fn earlyMasterPath(master: &Path) -> PathBuf {
    let name = master.file_name().unwrap_or_default().to_string_lossy();
    master.with_file_name(format!("early-{name}"))
}

/// The staged early Global master when present on disk and strictly ahead of the live master.
pub(crate) fn localEarlyMaster(live_master: &Path) -> Option<EarlyMaster> {
    let path = earlyMasterPath(live_master);
    if !path.is_file() {
        return None;
    }
    let raw = std::fs::read_to_string(markerPath(&path)).ok()?;
    let meta: EarlyMasterMeta = serde_json::from_str(&raw).ok()?;
    let early_version: u64 = meta.resource_version.trim().parse().ok()?;
    if let Some(live_version) =
        localResourceVersion(live_master).and_then(|version| version.trim().parse::<u64>().ok())
    {
        if early_version <= live_version {
            return None;
        }
    }
    Some(EarlyMaster { path, meta })
}

/// The newest Global CDN resource version known locally: the early staged version when one is
/// ahead of live, else the live master's version.
pub(crate) fn latestGlobalResourceVersion(live_master: &Path) -> Option<String> {
    localEarlyMaster(live_master)
        .map(|early| early.meta.resource_version)
        .or_else(|| localResourceVersion(live_master))
}

/// Probes the Global CDN ahead of the live master's resource version for an unreleased upload.
/// Downloads it to `early-master.mdb` when found, or removes stale early files once live catches
/// up. Returns `true` when the local early master changed (created, updated, or cleared).
pub async fn maybeUpdateEarlyMasterMdb(source: &CdnSource, live_master: &Path) -> Result<bool> {
    if source.hashed_root {
        return Ok(false);
    }
    let Some(live_version_str) = localResourceVersion(live_master) else {
        return Ok(false);
    };
    let Ok(live_version) = live_version_str.trim().parse::<u64>() else {
        return Ok(false);
    };

    let client = reqwest::Client::new();
    let found = probeHighestAhead(
        &client,
        source.base_url,
        live_version,
        EARLY_PROBE_STEP,
        EARLY_PROBE_MAX_STEPS,
    )
    .await?;

    let early_path = earlyMasterPath(live_master);
    let early_marker = markerPath(&early_path);

    let Some((candidate, last_modified)) = found else {
        return Ok(clearEarlyMasterFiles(&early_path, &early_marker));
    };

    let candidate_version = candidate.to_string();
    let existing = std::fs::read_to_string(&early_marker)
        .ok()
        .and_then(|raw| serde_json::from_str::<EarlyMasterMeta>(&raw).ok());
    if early_path.is_file()
        && existing.as_ref().is_some_and(|meta| {
            meta.resource_version == candidate_version && meta.last_modified == last_modified
        })
    {
        info!(
            region = source.region,
            early_resource_version = candidate_version,
            live_resource_version = live_version_str,
            "early master.mdb is up to date"
        );
        return Ok(false);
    }

    info!(
        region = source.region,
        early_resource_version = candidate_version,
        live_resource_version = live_version_str,
        last_modified = last_modified.as_deref().unwrap_or("unknown"),
        "Found unreleased CDN resource version ahead of live; downloading early master.mdb"
    );
    fetchMasterMdb(source, &candidate_version, &early_path).await?;
    let meta = EarlyMasterMeta {
        resource_version: candidate_version,
        live_resource_version: live_version_str,
        detected_at: Utc::now().to_rfc3339(),
        last_modified,
    };
    let body = serde_json::to_vec_pretty(&meta)?;
    std::fs::write(&early_marker, body).with_context(|| {
        format!(
            "failed to write early master marker to {}",
            early_marker.display()
        )
    })?;
    Ok(true)
}

fn clearEarlyMasterFiles(early_path: &Path, early_marker: &Path) -> bool {
    let mut removed = false;
    for path in [early_path, early_marker] {
        if path.exists() {
            match std::fs::remove_file(path) {
                Ok(()) => removed = true,
                Err(error) => warn!(
                    path = %path.display(),
                    error = %error,
                    "failed to remove stale early master file"
                ),
            }
        }
    }
    if removed {
        info!("Cleared stale early master.mdb after live version caught up");
    }
    removed
}

async fn probeHighestAhead(
    client: &reqwest::Client,
    base_url: &str,
    live_version: u64,
    step: u64,
    max_steps: u64,
) -> Result<Option<(u64, Option<String>)>> {
    let base = base_url.trim_end_matches('/').to_string();
    let mut pending = (1..=max_steps).map(|index| live_version + index * step);
    let mut tasks = JoinSet::new();
    let mut highest: Option<(u64, Option<String>)> = None;

    loop {
        while tasks.len() < EARLY_PROBE_CONCURRENCY {
            let Some(candidate) = pending.next() else {
                break;
            };
            let client = client.clone();
            let url = format!("{base}/{candidate}/manifests/manifestdat/root.manifest.bsv.lz4");
            tasks.spawn(async move {
                let outcome = game_cdn::probeRootManifestUrl(&client, &url).await;
                (candidate, outcome)
            });
        }
        let Some(joined) = tasks.join_next().await else {
            break;
        };
        let (candidate, outcome) = joined.context("early CDN probe task failed")?;
        match outcome {
            Ok(Some(last_modified)) => {
                if highest.as_ref().is_none_or(|(best, _)| candidate > *best) {
                    highest = Some((candidate, last_modified));
                }
            }
            Ok(None) => {}
            Err(error) => {
                warn!(
                    candidate,
                    error = format!("{error:#}"),
                    "early CDN version probe failed"
                );
            }
        }
    }

    Ok(highest)
}

/// The version marker next to master.mdb.
fn markerPath(mdb_path: &Path) -> PathBuf {
    let mut marker_name = mdb_path.as_os_str().to_os_string();
    marker_name.push(".version");
    PathBuf::from(marker_name)
}

/// The version endpoint for a region; Global is the endpoint's default.
fn versionUrl(source: &CdnSource) -> String {
    let endpoint = std::env::var(VERSION_URL_VARIABLE)
        .ok()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| VERSION_URL.to_string());
    regionVersionUrl(endpoint.trim(), source)
}

fn regionVersionUrl(endpoint: &str, source: &CdnSource) -> String {
    if source.region == game_cdn::GLOBAL_CDN.region {
        return endpoint.to_string();
    }
    format!("{endpoint}?region={}", source.region)
}

async fn fetchGameVersion(source: &CdnSource, url: &str) -> Result<GameVersion> {
    let region = source.region;
    let version: GameVersion = reqwest::Client::new()
        .get(url)
        .header("Cache-Control", "no-cache")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .with_context(|| format!("failed to check current {region} game version"))?
        .error_for_status()
        .with_context(|| format!("{region} game version endpoint returned an error"))?
        .json()
        .await
        .with_context(|| format!("invalid {region} game version response"))?;
    versionMarker(source, &version)?;
    Ok(version)
}

fn versionMarker(source: &CdnSource, version: &GameVersion) -> Result<String> {
    let app_version = version.app_version.trim();
    let resource_version = version.resource_version.trim();
    anyhow::ensure!(
        app_version.split('.').count() == 3
            && app_version
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            && isResourceVersion(source, resource_version),
        "invalid current {} game version",
        source.region
    );
    Ok(format!("{app_version}:{resource_version}"))
}

/// A resource version in the form its region uses: a number, followed on a CDN with a hashed
/// root manifest by `:` and the base64 token that names it. An endpoint that answers every
/// region with the Global version is caught here.
fn isResourceVersion(source: &CdnSource, resource_version: &str) -> bool {
    let (number, token) = match resource_version.split_once(':') {
        Some((number, token)) => (number, Some(token)),
        None => (resource_version, None),
    };
    let is_number = !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit());
    let token_fits = match token {
        Some(token) => {
            source.hashed_root
                && !token.is_empty()
                && token
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
        }
        None => !source.hashed_root,
    };
    is_number && token_fits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_cdn::{GLOBAL_CDN, JP_CDN};

    fn version(app_version: &str, resource_version: &str) -> GameVersion {
        GameVersion {
            app_version: app_version.to_string(),
            resource_version: resource_version.to_string(),
        }
    }

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
            versionMarker(
                &GLOBAL_CDN,
                &fetchGameVersion(&GLOBAL_CDN, &url).await.unwrap()
            )
            .unwrap(),
            "1.35.2:10008010"
        );
        assert_eq!(
            versionMarker(
                &GLOBAL_CDN,
                &fetchGameVersion(&GLOBAL_CDN, &url).await.unwrap()
            )
            .unwrap(),
            "1.35.2:10008011"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        // The same answer is no JP version: it lacks the token JP's root manifest is found by.
        assert!(fetchGameVersion(&JP_CDN, &url).await.is_err());
        assert!(
            fetchGameVersion(&GLOBAL_CDN, &format!("http://{address}/invalid"))
                .await
                .is_err()
        );
        assert!(
            fetchGameVersion(&GLOBAL_CDN, &format!("http://{address}/missing"))
                .await
                .is_err()
        );
        server.abort();
    }

    #[test]
    fn eachRegionAcceptsOnlyItsOwnVersionForm() {
        assert_eq!(
            versionMarker(&JP_CDN, &version("2.32.5", "10021450:TZH99sxhsgYr")).unwrap(),
            "2.32.5:10021450:TZH99sxhsgYr"
        );
        assert!(versionMarker(&JP_CDN, &version("2.32.5", "10021450:TUUhuK/ZZV+f")).is_ok());
        assert!(versionMarker(&JP_CDN, &version("1.35.5", "10008010")).is_err());
        assert!(versionMarker(&JP_CDN, &version("2.32.5", "10021450:")).is_err());
        assert!(versionMarker(&JP_CDN, &version("2.32.5", "10021450:../x")).is_err());
        assert!(versionMarker(&GLOBAL_CDN, &version("1.35.5", "10021450:TZH99sxhsgYr")).is_err());
    }

    #[test]
    fn otherRegionsAskTheEndpointByName() {
        let endpoint = "http://localhost:3001/api/ver";
        assert_eq!(regionVersionUrl(endpoint, &GLOBAL_CDN), endpoint);
        assert_eq!(
            regionVersionUrl(endpoint, &JP_CDN),
            "http://localhost:3001/api/ver?region=jp"
        );
    }

    #[test]
    fn jpMasterSitsBesideTheGlobalOneAndKeepsItsWholeVersion() {
        assert_eq!(
            jpMasterPath(Path::new("data/master.mdb")),
            Path::new("data/jp-master.mdb")
        );

        let folder = std::env::temp_dir().join(format!("jp-master-marker-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let master = folder.join("jp-master.mdb");
        std::fs::write(markerPath(&master), "2.32.5:10021450:TZH99sxhsgYr\n").unwrap();
        assert_eq!(
            localResourceVersion(&master).as_deref(),
            Some("10021450:TZH99sxhsgYr")
        );
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn earlyMasterIsIgnoredOnceLiveCatchesUp() {
        assert_eq!(
            earlyMasterPath(Path::new("data/master.mdb")),
            Path::new("data/early-master.mdb")
        );

        let folder = std::env::temp_dir().join(format!(
            "early-master-marker-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let live = folder.join("master.mdb");
        let early = earlyMasterPath(&live);
        std::fs::write(&live, b"live").unwrap();
        std::fs::write(&early, b"early").unwrap();
        std::fs::write(markerPath(&live), "1.35.5:10008790\n").unwrap();
        let meta = EarlyMasterMeta {
            resource_version: "10008800".to_string(),
            live_resource_version: "10008790".to_string(),
            detected_at: "2026-11-24T12:00:00Z".to_string(),
            last_modified: Some("Tue, 24 Nov 2026 11:14:05 GMT".to_string()),
        };
        std::fs::write(markerPath(&early), serde_json::to_vec(&meta).unwrap()).unwrap();

        let loaded = localEarlyMaster(&live).expect("early master ahead of live should load");
        assert_eq!(loaded.meta.resource_version, "10008800");
        assert_eq!(
            latestGlobalResourceVersion(&live).as_deref(),
            Some("10008800")
        );

        // Once live reaches 10008800, the early master must no longer apply.
        std::fs::write(markerPath(&live), "1.36.0:10008800\n").unwrap();
        assert!(localEarlyMaster(&live).is_none());
        assert_eq!(
            latestGlobalResourceVersion(&live).as_deref(),
            Some("10008800")
        );

        assert!(clearEarlyMasterFiles(&early, &markerPath(&early)));
        assert!(!early.exists());
        assert!(!clearEarlyMasterFiles(&early, &markerPath(&early)));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[tokio::test]
    async fn probeHighestAheadPicksNewestUploadedManifest() {
        use axum::http::{header, HeaderMap, StatusCode};
        use axum::{routing::head, Router};

        let app = Router::new()
            .route(
                "/10008800/manifests/manifestdat/root.manifest.bsv.lz4",
                head(|| async {
                    let mut headers = HeaderMap::new();
                    headers.insert(
                        header::LAST_MODIFIED,
                        "Tue, 24 Nov 2026 11:14:05 GMT".parse().unwrap(),
                    );
                    (StatusCode::OK, headers)
                }),
            )
            .route(
                "/10008830/manifests/manifestdat/root.manifest.bsv.lz4",
                head(|| async {
                    let mut headers = HeaderMap::new();
                    headers.insert(
                        header::LAST_MODIFIED,
                        "Wed, 25 Nov 2026 09:00:00 GMT".parse().unwrap(),
                    );
                    (StatusCode::OK, headers)
                }),
            )
            .fallback(|| async { StatusCode::FORBIDDEN });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let client = reqwest::Client::new();
        let base = format!("http://{address}");
        let found = probeHighestAhead(&client, &base, 10008790, 10, 5)
            .await
            .unwrap();
        assert_eq!(
            found,
            Some((10008830, Some("Wed, 25 Nov 2026 09:00:00 GMT".to_string())))
        );

        let none = probeHighestAhead(&client, &base, 10008830, 10, 5)
            .await
            .unwrap();
        assert_eq!(none, None);
        server.abort();
    }
}

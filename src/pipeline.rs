use crate::generators;
use anyhow::{ensure, Context, Result};
use chrono::Utc;
use flate2::write::GzEncoder;
use flate2::Compression;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const PUBLIC_TIMELINE_GZIP_BUDGET: u64 = 300 * 1024;
const PLANNER_MANIFEST_GZIP_BUDGET: usize = 10 * 1024;
const PLANNER_INITIAL_GZIP_BUDGET: u64 = 150 * 1024;
const PLANNER_GACHA_SHARD_GZIP_BUDGET: u64 = 100 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceManifest {
    pub version: String,
    pub generated_at: String,
    pub base_path: String,
    pub master: MasterInfo,
    pub artifacts: Vec<ResourceArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterInfo {
    pub marker: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceArtifact {
    pub name: String,
    pub path: String,
    pub current_path: String,
    pub content_type: String,
    pub content_encoding: String,
    pub etag: String,
    pub sha256: String,
    pub json_bytes: u64,
    pub gzip_bytes: u64,
}

pub fn generate_resources(
    master_path: &Path,
    out_dir: &Path,
    write_json: bool,
) -> Result<ResourceManifest> {
    let connection = Connection::open(master_path)
        .with_context(|| format!("failed to open SQLite master at {}", master_path.display()))?;
    let jp_master_path = std::env::var_os("JP_MASTER_PATH").map(PathBuf::from);
    if let Some(path) = &jp_master_path {
        ensure!(
            path.is_file(),
            "JP_MASTER_PATH points to a missing file: {}",
            path.display(),
        );
    }
    let jp_connection = jp_master_path
        .as_ref()
        .filter(|path| path.is_file())
        .map(|path| {
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .with_context(|| format!("failed to open JP SQLite master at {}", path.display()))
        })
        .transpose()?;
    let jp_master_hash = jp_master_path
        .as_ref()
        .filter(|path| path.is_file())
        .map(|path| fs::read(path).map(|bytes| sha256_hex(&bytes)))
        .transpose()?;
    let master_bytes = fs::read(master_path)
        .with_context(|| format!("failed to read master at {}", master_path.display()))?;
    let master_sha256 = sha256_hex(&master_bytes);
    let marker = read_master_marker(master_path)
        .unwrap_or_else(|| format!("master-{}", &master_sha256[..12]));
    let confirmed_dates_hash = generators::timeline::confirmed_dates_version_hash()?;
    let jp_events_hash = generators::jp_events::version_hash();
    let course_hash = generators::simulator_courses::version_hash();
    let planner_hash = generators::planner::version_hash();
    let geometry_hash = generators::simulator_course_geometry::version_hash()?;
    let jp_reward_source_version = match jp_master_hash.as_deref() {
        Some(hash) => format!("-jp-master-{}", &hash[..12]),
        None => {
            let hash = generators::planner::jp_master_reward_catalog_version_hash();
            format!("-jp-catalog-{}", &hash[..12])
        }
    };
    let version = sanitize_version(&format!(
        "{}-timeline-{}-jp-{}-courses-{}-planner-{}-geometry-{}{}",
        marker,
        &confirmed_dates_hash[..12],
        &jp_events_hash[..12],
        &course_hash[..12],
        &planner_hash[..12],
        &geometry_hash[..12],
        jp_reward_source_version,
    ));
    let version_dir = out_dir.join(&version);
    fs::create_dir_all(&version_dir)
        .with_context(|| format!("failed to create {}", version_dir.display()))?;

    let generated = generators::generate_all(&connection, jp_connection.as_ref(), &marker)?;
    let artifacts = write_outputs(
        generated.public,
        &version_dir,
        &version,
        "/resources",
        write_json,
    )?;
    enforce_named_budget(
        &artifacts,
        "banner_timeline.json",
        PUBLIC_TIMELINE_GZIP_BUDGET,
    )?;

    let manifest = ResourceManifest {
        version: version.clone(),
        generated_at: Utc::now().to_rfc3339(),
        base_path: format!("/resources/{}", version),
        master: MasterInfo {
            marker,
            sha256: master_sha256,
        },
        artifacts,
    };

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    fs::write(version_dir.join("manifest.json"), &manifest_json)
        .with_context(|| format!("failed to write manifest in {}", version_dir.display()))?;
    fs::write(out_dir.join("manifest.json"), manifest_json)
        .with_context(|| format!("failed to write manifest in {}", out_dir.display()))?;

    let planner_root = out_dir.join("planner");
    let planner_version_dir = planner_root.join(&version);
    fs::create_dir_all(&planner_version_dir)
        .with_context(|| format!("failed to create {}", planner_version_dir.display()))?;
    let planner_artifacts = write_outputs(
        generated.planner,
        &planner_version_dir,
        &version,
        "/resources/planner",
        write_json,
    )?;
    enforce_planner_budgets(&planner_artifacts)?;
    let planner_manifest = ResourceManifest {
        version: version.clone(),
        generated_at: manifest.generated_at.clone(),
        base_path: format!("/resources/planner/{}", version),
        master: manifest.master.clone(),
        artifacts: planner_artifacts,
    };
    let planner_manifest_json = serde_json::to_vec_pretty(&planner_manifest)?;
    let planner_manifest_gzip = gzip(&planner_manifest_json)?;
    if planner_manifest_gzip.len() > PLANNER_MANIFEST_GZIP_BUDGET {
        anyhow::bail!(
            "planner manifest exceeds gzip budget: {} > {} bytes",
            planner_manifest_gzip.len(),
            PLANNER_MANIFEST_GZIP_BUDGET
        );
    }
    fs::write(
        planner_version_dir.join("manifest.json"),
        &planner_manifest_json,
    )
    .with_context(|| {
        format!(
            "failed to write planner manifest in {}",
            planner_version_dir.display()
        )
    })?;
    fs::write(planner_root.join("manifest.json"), planner_manifest_json).with_context(|| {
        format!(
            "failed to write planner manifest in {}",
            planner_root.display()
        )
    })?;

    Ok(manifest)
}

fn write_outputs(
    outputs: Vec<generators::ResourceOutput>,
    version_dir: &Path,
    version: &str,
    route_prefix: &str,
    write_json: bool,
) -> Result<Vec<ResourceArtifact>> {
    let mut artifacts = Vec::with_capacity(outputs.len());
    for output in outputs {
        let json_bytes = serde_json::to_vec(&output.value)?;
        let json_sha256 = sha256_hex(&json_bytes);
        let gzip_bytes = gzip(&json_bytes)?;
        let gzip_path = version_dir.join(format!("{}.gz", output.file_name));
        fs::write(&gzip_path, &gzip_bytes)
            .with_context(|| format!("failed to write {}", gzip_path.display()))?;
        if write_json {
            let json_path = version_dir.join(&output.file_name);
            fs::write(&json_path, &json_bytes)
                .with_context(|| format!("failed to write {}", json_path.display()))?;
        }
        artifacts.push(ResourceArtifact {
            name: output.file_name.clone(),
            path: format!("{route_prefix}/{version}/{}.gz", output.file_name),
            current_path: format!("{route_prefix}/current/{}.gz", output.file_name),
            content_type: "application/json; charset=utf-8".to_string(),
            content_encoding: "gzip".to_string(),
            etag: format!("\"sha256-{}\"", json_sha256),
            sha256: json_sha256,
            json_bytes: json_bytes.len() as u64,
            gzip_bytes: gzip_bytes.len() as u64,
        });
    }
    Ok(artifacts)
}

fn enforce_named_budget(artifacts: &[ResourceArtifact], name: &str, budget: u64) -> Result<()> {
    let artifact = artifacts
        .iter()
        .find(|artifact| artifact.name == name)
        .with_context(|| format!("required generated resource {name} is missing"))?;
    if artifact.gzip_bytes > budget {
        anyhow::bail!(
            "{} exceeds gzip budget: {} > {} bytes",
            name,
            artifact.gzip_bytes,
            budget
        );
    }
    Ok(())
}

fn enforce_planner_budgets(artifacts: &[ResourceArtifact]) -> Result<()> {
    let initial_bytes = artifacts
        .iter()
        .filter(|artifact| {
            matches!(
                artifact.name.as_str(),
                "planner_core.json" | "planner_income.json" | "planner_rewards.json"
            )
        })
        .map(|artifact| artifact.gzip_bytes)
        .sum::<u64>();
    if initial_bytes > PLANNER_INITIAL_GZIP_BUDGET {
        anyhow::bail!(
            "initial planner payload exceeds gzip budget: {} > {} bytes",
            initial_bytes,
            PLANNER_INITIAL_GZIP_BUDGET
        );
    }
    for artifact in artifacts
        .iter()
        .filter(|artifact| artifact.name.starts_with("planner_gacha_"))
    {
        if artifact.gzip_bytes > PLANNER_GACHA_SHARD_GZIP_BUDGET {
            anyhow::bail!(
                "{} exceeds gzip budget: {} > {} bytes",
                artifact.name,
                artifact.gzip_bytes,
                PLANNER_GACHA_SHARD_GZIP_BUDGET
            );
        }
    }
    Ok(())
}

pub fn read_manifest(data_dir: &Path) -> Result<ResourceManifest> {
    let manifest_path = data_dir.join("manifest.json");
    read_manifest_path(&manifest_path)
}

pub fn read_version_manifest(data_dir: &Path, version: &str) -> Result<ResourceManifest> {
    let manifest_path = data_dir.join(version).join("manifest.json");
    read_manifest_path(&manifest_path)
}

fn read_manifest_path(manifest_path: &Path) -> Result<ResourceManifest> {
    let manifest_json = fs::read(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    Ok(serde_json::from_slice(&manifest_json)?)
}

fn read_master_marker(master_path: &Path) -> Option<String> {
    let marker_path = marker_path(master_path);
    fs::read_to_string(marker_path)
        .ok()
        .map(|marker| marker.trim().to_string())
        .filter(|marker| !marker.is_empty())
}

fn marker_path(master_path: &Path) -> PathBuf {
    let mut marker_path = OsString::from(master_path.as_os_str());
    marker_path.push(".version");
    PathBuf::from(marker_path)
}

fn sanitize_version(version: &str) -> String {
    version
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-' => character,
            _ => '-',
        })
        .collect()
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::{
        enforce_named_budget, enforce_planner_budgets, ResourceArtifact,
        PLANNER_GACHA_SHARD_GZIP_BUDGET, PLANNER_INITIAL_GZIP_BUDGET, PUBLIC_TIMELINE_GZIP_BUDGET,
    };

    fn artifact(name: &str, gzip_bytes: u64) -> ResourceArtifact {
        ResourceArtifact {
            name: name.to_string(),
            path: String::new(),
            current_path: String::new(),
            content_type: "application/json".to_string(),
            content_encoding: "gzip".to_string(),
            etag: String::new(),
            sha256: String::new(),
            json_bytes: gzip_bytes,
            gzip_bytes,
        }
    }

    #[test]
    fn rejects_timeline_over_public_budget() {
        let artifacts = vec![artifact(
            "banner_timeline.json",
            PUBLIC_TIMELINE_GZIP_BUDGET + 1,
        )];
        assert!(enforce_named_budget(
            &artifacts,
            "banner_timeline.json",
            PUBLIC_TIMELINE_GZIP_BUDGET
        )
        .is_err());
    }

    #[test]
    fn rejects_oversized_initial_planner_payload() {
        let artifacts = vec![artifact(
            "planner_core.json",
            PLANNER_INITIAL_GZIP_BUDGET + 1,
        )];
        assert!(enforce_planner_budgets(&artifacts).is_err());
    }

    #[test]
    fn rejects_oversized_gacha_shard() {
        let artifacts = vec![artifact(
            "planner_gacha_2026.json",
            PLANNER_GACHA_SHARD_GZIP_BUDGET + 1,
        )];
        assert!(enforce_planner_budgets(&artifacts).is_err());
    }
}

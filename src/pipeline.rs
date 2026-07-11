use crate::generators;
use anyhow::{Context, Result};
use chrono::Utc;
use flate2::write::GzEncoder;
use flate2::Compression;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

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
    let master_bytes = fs::read(master_path)
        .with_context(|| format!("failed to read master at {}", master_path.display()))?;
    let master_sha256 = sha256_hex(&master_bytes);
    let marker = read_master_marker(master_path)
        .unwrap_or_else(|| format!("master-{}", &master_sha256[..12]));
    let confirmed_dates_hash = generators::timeline::confirmed_dates_version_hash()?;
    let jp_events_hash = generators::jp_events::version_hash();
    let version = sanitize_version(&format!(
        "{}-timeline-{}-jp-{}",
        marker,
        &confirmed_dates_hash[..12],
        &jp_events_hash[..12]
    ));
    let version_dir = out_dir.join(&version);
    fs::create_dir_all(&version_dir)
        .with_context(|| format!("failed to create {}", version_dir.display()))?;

    let generated_outputs = generators::generate_all(&connection, &marker)?;
    let mut artifacts = Vec::with_capacity(generated_outputs.len());

    for generated_output in generated_outputs {
        let json_bytes = serde_json::to_vec(&generated_output.value)?;
        let json_sha256 = sha256_hex(&json_bytes);
        let gzip_bytes = gzip(&json_bytes)?;
        let gzip_path = version_dir.join(format!("{}.gz", generated_output.file_name));
        fs::write(&gzip_path, &gzip_bytes)
            .with_context(|| format!("failed to write {}", gzip_path.display()))?;

        if write_json {
            let json_path = version_dir.join(generated_output.file_name);
            fs::write(&json_path, &json_bytes)
                .with_context(|| format!("failed to write {}", json_path.display()))?;
        }

        artifacts.push(ResourceArtifact {
            name: generated_output.file_name.to_string(),
            path: format!("/resources/{}/{}.gz", version, generated_output.file_name),
            current_path: format!("/resources/current/{}.gz", generated_output.file_name),
            content_type: "application/json; charset=utf-8".to_string(),
            content_encoding: "gzip".to_string(),
            etag: format!("\"sha256-{}\"", json_sha256),
            sha256: json_sha256,
            json_bytes: json_bytes.len() as u64,
            gzip_bytes: gzip_bytes.len() as u64,
        });
    }

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

    Ok(manifest)
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

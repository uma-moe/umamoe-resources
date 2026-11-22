use crate::generators;
use crate::master_history::{self, MasterSnapshot};
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
use tracing::warn;

/// Where the Global set is served. Each region has a path of its own; the paths directly below
/// `/resources` from before the split keep serving Global for older clients.
pub const GLOBAL_RESOURCE_PATH: &str = "/resources/en";

/// Where the JP set is served, beside Global's.
pub const JP_RESOURCE_PATH: &str = "/resources/jp";

/// The folder of the data directory that holds the JP set.
const JP_DATA_FOLDER: &str = "jp";

const PUBLIC_TIMELINE_GZIP_BUDGET: u64 = 300 * 1024;
const PLANNER_MANIFEST_GZIP_BUDGET: usize = 10 * 1024;
const PLANNER_INITIAL_GZIP_BUDGET: u64 = 150 * 1024;
const PLANNER_GACHA_SHARD_GZIP_BUDGET: u64 = 100 * 1024;
static GENERATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
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

/// The JP master the planner reads its rewards from. It is set only to refresh the bundled
/// reward catalogue; without it the planner reads the catalogue.
fn planner_jp_master() -> Result<Option<PathBuf>> {
    let Some(path) = std::env::var_os("JP_MASTER_PATH").map(PathBuf::from) else {
        return Ok(None);
    };
    ensure!(
        path.is_file(),
        "JP_MASTER_PATH points to a missing file: {}",
        path.display(),
    );
    Ok(Some(path))
}

/// The JP master the Global set reads Japanese names and JP's newest banners from: the one the
/// server keeps beside the Global master, or JP_MASTER_PATH when that is set.
fn names_jp_master(master_path: &Path) -> Option<PathBuf> {
    std::env::var_os("JP_MASTER_PATH")
        .map(PathBuf::from)
        .or_else(|| Some(crate::master_fetch::jpMasterPath(master_path)))
        .filter(|path| path.is_file())
}

fn open_jp_master(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open JP SQLite master at {}", path.display()))
}

fn file_sha256(path: &Path) -> Result<String> {
    Ok(sha256_hex(&fs::read(path).with_context(|| {
        format!("failed to read {}", path.display())
    })?))
}

/// The version the Global set would get from the masters and bundled data on disk now, so a
/// refresh can tell whether anything it reads changed.
pub fn resources_version(master_path: &Path) -> Result<String> {
    let master_sha256 = file_sha256(master_path)?;
    let marker = read_master_marker(master_path)
        .unwrap_or_else(|| format!("master-{}", &master_sha256[..12]));
    global_version(master_path, &marker, &master_sha256)
}

fn global_version(master_path: &Path, marker: &str, master_sha256: &str) -> Result<String> {
    let confirmed_dates_hash = generators::timeline::confirmed_dates_version_hash()?;
    // The JP data the timeline reads: the news archive, the bundled names, and the JP master the
    // server keeps. One hash keeps the version short enough for Windows paths.
    let mut jp_digest = Sha256::new();
    jp_digest.update(generators::jp_events::version_hash());
    if let Some(path) = names_jp_master(master_path) {
        jp_digest.update(file_sha256(&path)?);
    }
    let jp_events_hash = hex::encode(jp_digest.finalize());
    let course_hash = generators::simulator_courses::version_hash();
    let planner_hash = generators::planner::version_hash();
    let geometry_hash = generators::simulator_course_geometry::version_hash()?;
    let jp_source_version = match planner_jp_master()? {
        Some(path) => format!("-jp-master-{}", &file_sha256(&path)?[..12]),
        None => {
            let hash = generators::planner::jp_master_reward_catalog_version_hash();
            format!("-jp-catalog-{}", &hash[..12])
        }
    };
    Ok(sanitize_version(&format!(
        "{}-timeline-{}-jp-{}-courses-{}-planner-{}-geometry-{}{}-master-{}",
        marker,
        &confirmed_dates_hash[..12],
        &jp_events_hash[..12],
        &course_hash[..12],
        &planner_hash[..12],
        &geometry_hash[..12],
        jp_source_version,
        &master_sha256[..12],
    )))
}

pub fn generate_resources(
    master_path: &Path,
    out_dir: &Path,
    write_json: bool,
) -> Result<ResourceManifest> {
    // Background refreshers publish complete exports in order.
    let _generation = lockGeneration()?;
    let jp_master_path = planner_jp_master()?;
    let jp_connection = jp_master_path.as_deref().map(open_jp_master).transpose()?;
    let jp_names_connection = names_jp_master(master_path)
        .as_deref()
        .map(open_jp_master)
        .transpose()?;
    let master_bytes = fs::read(master_path)
        .with_context(|| format!("failed to read master at {}", master_path.display()))?;
    let master_sha256 = sha256_hex(&master_bytes);
    let marker = read_master_marker(master_path)
        .unwrap_or_else(|| format!("master-{}", &master_sha256[..12]));
    let version = global_version(master_path, &marker, &master_sha256)?;
    // Read before this export replaces it, so the release history can diff against its master.
    let previous_manifest = read_manifest(out_dir).ok();
    let version_dir = out_dir.join(&version);
    fs::create_dir_all(&version_dir)
        .with_context(|| format!("failed to create {}", version_dir.display()))?;

    // Generate against these exact bytes even if the live master is refreshed meanwhile.
    let snapshot = version_dir.join("master.mdb");
    if !snapshot.exists() {
        writeAtomic(&snapshot, &master_bytes)?;
    } else {
        ensure!(
            sha256_hex(&fs::read(&snapshot)?) == master_sha256,
            "immutable master snapshot changed"
        );
    }
    let connection = Connection::open_with_flags(&snapshot, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let compressed_master = version_dir.join("master.mdb.gz");
    if !compressed_master.exists() {
        writeAtomic(&compressed_master, &gzip(&master_bytes)?)?;
    }

    let generated = generators::generate_all(
        &connection,
        jp_connection.as_ref(),
        jp_names_connection.as_ref(),
        &marker,
    )?;
    let artifacts = write_outputs(
        generated.public,
        &version_dir,
        &version,
        GLOBAL_RESOURCE_PATH,
        write_json,
    )?;
    enforce_named_budget(
        &artifacts,
        "banner_timeline.json",
        PUBLIC_TIMELINE_GZIP_BUDGET,
    )?;

    let generated_at = Utc::now();
    let manifest = ResourceManifest {
        version: version.clone(),
        generated_at: generated_at.to_rfc3339(),
        base_path: format!("{GLOBAL_RESOURCE_PATH}/{version}"),
        master: MasterInfo {
            marker,
            sha256: master_sha256,
            path: Some(format!("/resources/simulator/{version}/master.mdb.gz")),
            bytes: Some(master_bytes.len() as u64),
        },
        artifacts,
    };

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    writeAtomic(&version_dir.join("manifest.json"), &manifest_json)
        .with_context(|| format!("failed to write manifest in {}", version_dir.display()))?;
    writeAtomic(&out_dir.join("manifest.json"), &manifest_json)
        .with_context(|| format!("failed to write manifest in {}", out_dir.display()))?;
    let current_master = MasterSnapshot {
        sha256: &manifest.master.sha256,
        path: &snapshot,
    };
    recordMasterRelease(
        out_dir,
        &current_master,
        previous_manifest.as_ref(),
        &generated_at.format("%Y-%m-%d").to_string(),
    );

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

/// Where the JP set is written: a data directory of its own inside the Global one.
pub fn jp_data_dir(out_dir: &Path) -> PathBuf {
    out_dir.join(JP_DATA_FOLDER)
}

/// Generates the JP set from the JP master into `jp/` of the data directory, apart from the
/// Global export and with a manifest of its own. The Global master only says which JP cards
/// Global has released.
pub fn generate_jp_resources(
    jp_master_path: &Path,
    global_master_path: &Path,
    out_dir: &Path,
    write_json: bool,
) -> Result<ResourceManifest> {
    let _generation = lockGeneration()?;
    let master_bytes = fs::read(jp_master_path)
        .with_context(|| format!("failed to read JP master at {}", jp_master_path.display()))?;
    let master_sha256 = sha256_hex(&master_bytes);
    let marker = read_master_marker(jp_master_path)
        .unwrap_or_else(|| format!("master-{}", &master_sha256[..12]));
    let version = jp_set_version(&marker, &master_sha256, global_master_path)?;

    let jp = Connection::open_with_flags(jp_master_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open JP master at {}", jp_master_path.display()))?;
    let global = Connection::open_with_flags(global_master_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| {
            format!(
                "failed to open Global master at {}",
                global_master_path.display()
            )
        })?;
    let outputs = generators::generate_jp(&jp, &global)?;

    let jp_dir = jp_data_dir(out_dir);
    let version_dir = jp_dir.join(&version);
    fs::create_dir_all(&version_dir)
        .with_context(|| format!("failed to create {}", version_dir.display()))?;
    let artifacts = write_outputs(
        outputs,
        &version_dir,
        &version,
        JP_RESOURCE_PATH,
        write_json,
    )?;

    let manifest = ResourceManifest {
        version: version.clone(),
        generated_at: Utc::now().to_rfc3339(),
        base_path: format!("{JP_RESOURCE_PATH}/{version}"),
        master: MasterInfo {
            marker,
            sha256: master_sha256,
            path: None,
            bytes: Some(master_bytes.len() as u64),
        },
        artifacts,
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    writeAtomic(&version_dir.join("manifest.json"), &manifest_json)
        .with_context(|| format!("failed to write manifest in {}", version_dir.display()))?;
    writeAtomic(&jp_dir.join("manifest.json"), &manifest_json)
        .with_context(|| format!("failed to write manifest in {}", jp_dir.display()))?;

    Ok(manifest)
}

/// The version the JP set would have for these two masters, to tell whether the set on disk is
/// current without generating it.
pub fn jp_resources_version(jp_master_path: &Path, global_master_path: &Path) -> Result<String> {
    let master_sha256 =
        sha256_hex(&fs::read(jp_master_path).with_context(|| {
            format!("failed to read JP master at {}", jp_master_path.display())
        })?);
    let marker = read_master_marker(jp_master_path)
        .unwrap_or_else(|| format!("master-{}", &master_sha256[..12]));
    jp_set_version(&marker, &master_sha256, global_master_path)
}

/// Release flags and English names come from the Global master and the bundled names, so an
/// update to either changes the JP set too.
fn jp_set_version(marker: &str, master_sha256: &str, global_master_path: &Path) -> Result<String> {
    let global_sha256 = sha256_hex(&fs::read(global_master_path).with_context(|| {
        format!(
            "failed to read Global master at {}",
            global_master_path.display()
        )
    })?);
    let mut names = Sha256::new();
    crate::jp_names::hashNames(&mut names);
    let names_sha256 = hex::encode(names.finalize());
    Ok(sanitize_version(&format!(
        "{}-master-{}-global-{}-names-{}",
        marker,
        &master_sha256[..12],
        &global_sha256[..12],
        &names_sha256[..12],
    )))
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
        writeAtomic(&gzip_path, &gzip_bytes)
            .with_context(|| format!("failed to write {}", gzip_path.display()))?;
        if write_json {
            let json_path = version_dir.join(&output.file_name);
            writeAtomic(&json_path, &json_bytes)
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

/// The history is supplementary, so a failure is logged instead of blocking the export.
#[allow(non_snake_case)]
fn recordMasterRelease(
    out_dir: &Path,
    current: &MasterSnapshot,
    previous_manifest: Option<&ResourceManifest>,
    date: &str,
) {
    let previous_path =
        previous_manifest.map(|manifest| out_dir.join(&manifest.version).join("master.mdb"));
    let previous = previous_manifest
        .zip(previous_path.as_deref())
        .map(|(manifest, path)| MasterSnapshot {
            sha256: &manifest.master.sha256,
            path,
        });
    if let Err(error) = master_history::recordRelease(out_dir, current, previous.as_ref(), date) {
        warn!(error = %error, "failed to record the master release history");
    }
}

/// Held while a resource set is generated, and by whatever replaces data generation reads, so
/// one export never mixes two versions of it.
#[allow(non_snake_case)]
pub(crate) fn lockGeneration() -> Result<std::sync::MutexGuard<'static, ()>> {
    GENERATION_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("resource generation lock poisoned"))
}

#[allow(non_snake_case)]
pub(crate) fn writeAtomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension("next");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
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

pub(crate) fn gzip(bytes: &[u8]) -> Result<Vec<u8>> {
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

    /// The bundled Global master stands in for both masters: the queries and the layout are
    /// what is checked, and every card of a master is released where that master is from.
    #[test]
    fn jp_set_is_written_apart_with_its_own_manifest() {
        use super::{generate_jp_resources, jp_data_dir, jp_resources_version, read_manifest};

        let master = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("master.mdb");
        let out = std::env::temp_dir().join(format!("jp-resource-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);

        let manifest = generate_jp_resources(&master, &master, &out, false).unwrap();

        assert_eq!(
            manifest.base_path,
            format!("/resources/jp/{}", manifest.version)
        );
        assert_eq!(
            manifest.version,
            jp_resources_version(&master, &master).unwrap()
        );
        assert!(!out.join("manifest.json").exists());
        let jp_dir = jp_data_dir(&out);
        assert_eq!(read_manifest(&jp_dir).unwrap().version, manifest.version);

        let names: Vec<&str> = manifest
            .artifacts
            .iter()
            .map(|artifact| artifact.name.as_str())
            .collect();
        assert!(names.contains(&"character.json") && names.contains(&"skills.json"));
        // Built around Global's schedule; a JP set has neither.
        assert!(!names.contains(&"banner_timeline.json"));
        assert!(!names.iter().any(|name| name.starts_with("planner_")));
        for artifact in &manifest.artifacts {
            assert_eq!(
                artifact.current_path,
                format!("/resources/jp/current/{}.gz", artifact.name)
            );
            assert!(jp_dir
                .join(&manifest.version)
                .join(format!("{}.gz", artifact.name))
                .is_file());
        }

        let gzipped = std::fs::read(jp_dir.join(&manifest.version).join("character.json.gz"));
        let mut json = Vec::new();
        std::io::Read::read_to_end(
            &mut flate2::read::GzDecoder::new(gzipped.unwrap().as_slice()),
            &mut json,
        )
        .unwrap();
        let characters: serde_json::Value = serde_json::from_slice(&json).unwrap();
        let characters = characters.as_array().unwrap();
        assert!(!characters.is_empty());
        assert!(characters
            .iter()
            .all(|card| card["isReleased_jp"] == true && card["isReleased_en"] == true));

        std::fs::remove_dir_all(&out).unwrap();
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

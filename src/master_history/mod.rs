//! Release history of the published master database. `masterdata/versions.json` lists every
//! release, and `masterdata/diffs/<short_hash>.json.gz` holds its row-level changes.
#![allow(non_snake_case)]

mod diff;
#[path = "../types/master_history.rs"]
mod types;

pub(crate) use diff::sqliteValueToJson;
pub use types::MasterSnapshot;
use types::{DiffSummary, MasterDiff, MasterRelease, SHORT_HASH_LENGTH};

use crate::pipeline::{gzip, writeAtomic};
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub fn versionsPath(data_dir: &Path) -> PathBuf {
    data_dir.join("masterdata").join("versions.json")
}

pub fn diffPath(data_dir: &Path, short_hash: &str) -> PathBuf {
    data_dir
        .join("masterdata")
        .join("diffs")
        .join(format!("{short_hash}.json.gz"))
}

pub fn isShortHash(value: &str) -> bool {
    value.len() == SHORT_HASH_LENGTH
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Appends `current` to the release history unless it is already listed. `previous` is the
/// master of the export being replaced; the diff is written when it is the latest release.
pub fn recordRelease(
    data_dir: &Path,
    current: &MasterSnapshot,
    previous: Option<&MasterSnapshot>,
    date: &str,
) -> Result<()> {
    let versions_path = versionsPath(data_dir);
    let mut releases = readReleases(&versions_path)?;
    // Each short hash names one immutable diff, so a master that returns is not listed again.
    if releases
        .iter()
        .any(|release| release.hash == current.sha256)
    {
        return Ok(());
    }

    let previous_hash = releases.last().map(|release| release.hash.clone());
    let summary = match previous_hash.as_deref() {
        Some(previous_hash) => writeDiff(data_dir, previous_hash, previous, current, date)?,
        None => None,
    };
    let short_hash = &current.sha256[..SHORT_HASH_LENGTH];
    releases.push(MasterRelease {
        hash: current.sha256.to_string(),
        short_hash: short_hash.to_string(),
        date: date.to_string(),
        previous_hash,
        summary,
    });

    // The diff is written first, so the history never lists a diff that is not there yet.
    if let Some(parent) = versions_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    writeAtomic(&versions_path, &serde_json::to_vec_pretty(&releases)?)
        .with_context(|| format!("failed to write {}", versions_path.display()))?;
    info!(short_hash, ?summary, "recorded master release");
    Ok(())
}

/// Writes the diff from the latest listed release when its database is the one being replaced.
fn writeDiff(
    data_dir: &Path,
    previous_hash: &str,
    previous: Option<&MasterSnapshot>,
    current: &MasterSnapshot,
    date: &str,
) -> Result<Option<DiffSummary>> {
    let previous =
        previous.filter(|snapshot| snapshot.sha256 == previous_hash && snapshot.path.is_file());
    let Some(previous) = previous else {
        warn!(
            previous_hash,
            "previous master release is unavailable; recording the new release without a diff"
        );
        return Ok(None);
    };

    let diff = diffMasterFiles(previous, current, date)?;
    let path = diffPath(data_dir, &current.sha256[..SHORT_HASH_LENGTH]);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    writeAtomic(&path, &gzip(&serde_json::to_vec(&diff)?)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(Some(diff.summary))
}

fn diffMasterFiles(
    previous: &MasterSnapshot,
    current: &MasterSnapshot,
    date: &str,
) -> Result<MasterDiff> {
    let previous_connection = openReadOnly(previous.path)?;
    let current_connection = openReadOnly(current.path)?;
    let tables = diff::diffMasters(&previous_connection, &current_connection)?;
    Ok(MasterDiff {
        from_hash: previous.sha256.to_string(),
        to_hash: current.sha256.to_string(),
        date: date.to_string(),
        summary: diff::summarize(&tables),
        tables,
    })
}

fn openReadOnly(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open master snapshot {}", path.display()))
}

fn readReleases(path: &Path) -> Result<Vec<MasterRelease>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()))
        }
    };
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::types::{DiffSummary, MasterDiff, MasterRelease};
    use super::{diffPath, recordRelease, versionsPath, MasterSnapshot};
    use flate2::read::GzDecoder;
    use rusqlite::Connection;
    use std::path::{Path, PathBuf};

    const FIRST_HASH: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const SECOND_HASH: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    fn temporaryDirectory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "master-history-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn writeMaster(path: &Path, sql: &str) {
        Connection::open(path).unwrap().execute_batch(sql).unwrap();
    }

    fn readReleases(data_dir: &Path) -> Vec<MasterRelease> {
        serde_json::from_slice(&std::fs::read(versionsPath(data_dir)).unwrap()).unwrap()
    }

    #[test]
    fn recordsTheInitialReleaseAndDiffsEachLaterRelease() {
        let data_dir = temporaryDirectory("record");
        let first_path = data_dir.join("first.mdb");
        let second_path = data_dir.join("second.mdb");
        writeMaster(
            &first_path,
            "CREATE TABLE item (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO item VALUES (1, 'old');",
        );
        writeMaster(
            &second_path,
            "CREATE TABLE item (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO item VALUES (1, 'new'), (2, 'added');",
        );
        let first = MasterSnapshot {
            sha256: FIRST_HASH,
            path: &first_path,
        };
        let second = MasterSnapshot {
            sha256: SECOND_HASH,
            path: &second_path,
        };

        recordRelease(&data_dir, &first, None, "2026-10-01").unwrap();
        recordRelease(&data_dir, &second, Some(&first), "2026-10-08").unwrap();
        recordRelease(&data_dir, &second, Some(&second), "2026-10-09").unwrap();

        let releases = readReleases(&data_dir);
        let expected_summary = DiffSummary {
            tables_changed: 1,
            added: 1,
            removed: 0,
            modified: 1,
        };
        assert_eq!(
            releases,
            [
                MasterRelease {
                    hash: FIRST_HASH.into(),
                    short_hash: "111111111111".into(),
                    date: "2026-10-01".into(),
                    previous_hash: None,
                    summary: None,
                },
                MasterRelease {
                    hash: SECOND_HASH.into(),
                    short_hash: "222222222222".into(),
                    date: "2026-10-08".into(),
                    previous_hash: Some(FIRST_HASH.into()),
                    summary: Some(expected_summary),
                },
            ]
        );
        let compressed = std::fs::read(diffPath(&data_dir, "222222222222")).unwrap();
        let diff: MasterDiff = serde_json::from_reader(GzDecoder::new(&compressed[..])).unwrap();
        assert_eq!(diff.from_hash, FIRST_HASH);
        assert_eq!(diff.to_hash, SECOND_HASH);
        assert_eq!(diff.summary, expected_summary);
        assert_eq!(diff.tables["item"].modified[0].after[1], "new");
        std::fs::remove_dir_all(data_dir).unwrap();
    }

    #[test]
    fn recordsReleaseWithoutDiffWhenThePreviousMasterIsUnavailable() {
        let data_dir = temporaryDirectory("unavailable");
        let first_path = data_dir.join("first.mdb");
        writeMaster(&first_path, "CREATE TABLE item (id INTEGER PRIMARY KEY);");
        let first = MasterSnapshot {
            sha256: FIRST_HASH,
            path: &first_path,
        };
        let second = MasterSnapshot {
            sha256: SECOND_HASH,
            path: &first_path,
        };
        let missing_path = data_dir.join("missing.mdb");
        let missing_first = MasterSnapshot {
            sha256: FIRST_HASH,
            path: &missing_path,
        };

        recordRelease(&data_dir, &first, None, "2026-10-01").unwrap();
        recordRelease(&data_dir, &second, Some(&missing_first), "2026-10-08").unwrap();

        let latest = readReleases(&data_dir).pop().unwrap();
        assert_eq!(latest.previous_hash.as_deref(), Some(FIRST_HASH));
        assert_eq!(latest.summary, None);
        assert!(!diffPath(&data_dir, "222222222222").exists());
        std::fs::remove_dir_all(data_dir).unwrap();
    }
}

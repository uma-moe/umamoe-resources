//! Release history of the published master database. `masterdata/versions.json` lists every
//! release, and `masterdata/diffs/<short_hash>.json.gz` holds its row-level changes.
#![allow(non_snake_case)]

mod backfill;
mod diff;
#[path = "../types/master_history.rs"]
mod types;

pub use backfill::backfillMasterHistory;
pub(crate) use diff::sqliteValueToJson;
use types::{DiffSummary, MasterDiff, MasterRelease};
pub use types::{MasterSnapshot, PreliminaryResourceVersion, SHORT_HASH_LENGTH};

use crate::master_fetch::EarlyMasterMeta;
use crate::pipeline::{gzip, writeAtomic};
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub fn versionsPath(data_dir: &Path) -> PathBuf {
    data_dir.join("masterdata").join("versions.json")
}

pub fn preliminaryVersionsPath(data_dir: &Path) -> PathBuf {
    data_dir
        .join("masterdata")
        .join("preliminary_versions.json")
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

/// Appends `current` to the release history unless it is already listed as a live release. If it
/// was previously recorded as preliminary, promotes it in place to live. `previous` is the live
/// master being replaced; the diff is written against the latest live release.
pub fn recordRelease(
    data_dir: &Path,
    current: &MasterSnapshot,
    previous: Option<&MasterSnapshot>,
    date: &str,
    resource_version: Option<&str>,
) -> Result<()> {
    let versions_path = versionsPath(data_dir);
    let mut releases = readReleases(&versions_path)?;
    let mut reordered = sortReleasesByResourceVersion(&mut releases);
    let last_live_hash = releases
        .iter()
        .rev()
        .find(|release| !release.is_preliminary)
        .map(|release| release.hash.clone());

    if let Some(existing) = releases
        .iter_mut()
        .find(|release| release.hash == current.sha256)
    {
        if !existing.is_preliminary {
            let mut updated = false;
            if existing.resource_version.is_none() && resource_version.is_some() {
                existing.resource_version = resource_version.map(str::to_string);
                updated = true;
            }
            reordered |= sortReleasesByResourceVersion(&mut releases);
            if updated || reordered {
                writeReleases(&versions_path, &releases)?;
            }
            return Ok(());
        }

        // Promote a previously preliminary entry to an official live release.
        let needs_rediff = existing.previous_hash != last_live_hash
            || (last_live_hash.is_some() && existing.summary.is_none());
        if needs_rediff {
            existing.previous_hash = last_live_hash.clone();
            existing.summary = match last_live_hash.as_deref() {
                Some(previous_hash) => writeDiff(data_dir, previous_hash, previous, current, date)?,
                None => None,
            };
        }
        existing.is_preliminary = false;
        existing.date = date.to_string();
        if resource_version.is_some() {
            existing.resource_version = resource_version.map(str::to_string);
        }
        let short_hash = existing.short_hash.clone();
        let summary = existing.summary;
        sortReleasesByResourceVersion(&mut releases);
        writeReleases(&versions_path, &releases)?;
        info!(
            short_hash,
            ?summary,
            "promoted preliminary master release to live"
        );
        return Ok(());
    }

    let summary = match last_live_hash.as_deref() {
        Some(previous_hash) => writeDiff(data_dir, previous_hash, previous, current, date)?,
        None => None,
    };
    let short_hash = &current.sha256[..SHORT_HASH_LENGTH];
    releases.push(MasterRelease {
        hash: current.sha256.to_string(),
        short_hash: short_hash.to_string(),
        date: date.to_string(),
        previous_hash: last_live_hash,
        summary,
        resource_version: resource_version.map(str::to_string),
        is_preliminary: false,
        detected_at: None,
        cdn_last_modified: None,
    });
    sortReleasesByResourceVersion(&mut releases);

    // The diff is written first, so the history never lists a diff that is not there yet.
    writeReleases(&versions_path, &releases)?;
    info!(short_hash, ?summary, "recorded master release");
    Ok(())
}

/// Records an unreleased early CDN master in `masterdata/versions.json` with `is_preliminary: true`
/// and writes its diff relative to the live master. Skips when the early master has identical bytes
/// to live or is already recorded as a live release.
pub fn recordPreliminaryRelease(
    data_dir: &Path,
    early: &MasterSnapshot,
    live: &MasterSnapshot,
    meta: &EarlyMasterMeta,
    date: &str,
) -> Result<()> {
    if early.sha256 == live.sha256 {
        return Ok(());
    }

    let versions_path = versionsPath(data_dir);
    let mut releases = readReleases(&versions_path)?;
    let mut changed = sortReleasesByResourceVersion(&mut releases);
    if let Some(existing) = releases
        .iter_mut()
        .find(|release| release.hash == early.sha256)
    {
        if !existing.is_preliminary {
            if changed {
                writeReleases(&versions_path, &releases)?;
            }
            return Ok(());
        }
        if existing.resource_version.as_deref() != Some(meta.resource_version.as_str()) {
            existing.resource_version = Some(meta.resource_version.clone());
            changed = true;
        }
        if existing.cdn_last_modified != meta.last_modified {
            existing.cdn_last_modified = meta.last_modified.clone();
            changed = true;
        }
        if existing.previous_hash.as_deref() != Some(live.sha256) || existing.summary.is_none() {
            let diff_date = existing.date.clone();
            existing.previous_hash = Some(live.sha256.to_string());
            existing.summary = writeDiff(data_dir, live.sha256, Some(live), early, &diff_date)?;
            changed = true;
        }
        changed |= sortReleasesByResourceVersion(&mut releases);
        if changed {
            writeReleases(&versions_path, &releases)?;
        }
        return Ok(());
    }

    let summary = writeDiff(data_dir, live.sha256, Some(live), early, date)?;
    let short_hash = &early.sha256[..SHORT_HASH_LENGTH];
    releases.push(MasterRelease {
        hash: early.sha256.to_string(),
        short_hash: short_hash.to_string(),
        date: date.to_string(),
        previous_hash: Some(live.sha256.to_string()),
        summary,
        resource_version: Some(meta.resource_version.clone()),
        is_preliminary: true,
        detected_at: Some(meta.detected_at.clone()),
        cdn_last_modified: meta.last_modified.clone(),
    });
    sortReleasesByResourceVersion(&mut releases);
    writeReleases(&versions_path, &releases)?;
    info!(
        short_hash,
        resource_version = meta.resource_version,
        ?summary,
        "recorded preliminary master release"
    );
    Ok(())
}

/// Appends or updates an entry in `masterdata/preliminary_versions.json` tracking every unreleased
/// Global CDN resource version observed ahead of live (even if `master.mdb` bytes were unchanged).
pub fn recordPreliminaryVersion(
    data_dir: &Path,
    meta: &EarlyMasterMeta,
    master_sha256: &str,
    master_changed_from_live: bool,
) -> Result<()> {
    let path = preliminaryVersionsPath(data_dir);
    let mut versions = readPreliminaryVersions(&path)?;
    let mut changed = sortPreliminaryVersions(&mut versions);
    if let Some(existing) = versions.iter_mut().find(|entry| {
        entry.resource_version == meta.resource_version && entry.master_sha256 == master_sha256
    }) {
        if existing.cdn_last_modified != meta.last_modified {
            existing.cdn_last_modified = meta.last_modified.clone();
            changed = true;
        }
        changed |= sortPreliminaryVersions(&mut versions);
        if changed {
            writePreliminaryVersions(&path, &versions)?;
        }
        return Ok(());
    }

    versions.push(PreliminaryResourceVersion {
        resource_version: meta.resource_version.clone(),
        live_resource_version: meta.live_resource_version.clone(),
        master_sha256: master_sha256.to_string(),
        master_short_hash: master_sha256[..SHORT_HASH_LENGTH].to_string(),
        master_changed_from_live,
        detected_at: meta.detected_at.clone(),
        cdn_last_modified: meta.last_modified.clone(),
        promoted_at: None,
    });
    sortPreliminaryVersions(&mut versions);
    writePreliminaryVersions(&path, &versions)?;
    Ok(())
}

/// Marks tracked preliminary CDN resource versions as promoted once the live version reaches or
/// passes them.
pub fn markPromotedPreliminaryVersions(
    data_dir: &Path,
    live_resource_version: &str,
    promoted_at: &str,
) -> Result<()> {
    let Ok(live_number) = live_resource_version.trim().parse::<u64>() else {
        return Ok(());
    };
    let path = preliminaryVersionsPath(data_dir);
    if !path.is_file() {
        return Ok(());
    }
    let mut versions = readPreliminaryVersions(&path)?;
    let mut changed = sortPreliminaryVersions(&mut versions);
    for entry in &mut versions {
        if entry.promoted_at.is_some() {
            continue;
        }
        if entry
            .resource_version
            .trim()
            .parse::<u64>()
            .is_ok_and(|version| version <= live_number)
        {
            entry.promoted_at = Some(promoted_at.to_string());
            changed = true;
        }
    }
    if changed {
        writePreliminaryVersions(&path, &versions)?;
    }
    Ok(())
}

fn parseResourceVersionNumber(value: Option<&str>) -> Option<u64> {
    value.and_then(|version| version.trim().parse::<u64>().ok())
}

/// Orders releases with a numeric `resource_version` ascending (oldest/lowest first) while keeping
/// legacy unversioned entries in their recorded slots. When a preliminary upload and a live release
/// share the same `resource_version`, the preliminary entry sorts earlier so live appears newest.
pub(super) fn sortReleasesByResourceVersion(releases: &mut [MasterRelease]) -> bool {
    let versioned_indices: Vec<usize> = releases
        .iter()
        .enumerate()
        .filter_map(|(index, release)| {
            parseResourceVersionNumber(release.resource_version.as_deref()).map(|_| index)
        })
        .collect();
    if versioned_indices.len() <= 1 {
        return false;
    }

    let mut versioned: Vec<(usize, MasterRelease)> = versioned_indices
        .iter()
        .map(|&index| (index, releases[index].clone()))
        .collect();
    versioned.sort_by(|(left_index, left), (right_index, right)| {
        let left_ver = parseResourceVersionNumber(left.resource_version.as_deref()).unwrap_or(0);
        let right_ver = parseResourceVersionNumber(right.resource_version.as_deref()).unwrap_or(0);
        left_ver
            .cmp(&right_ver)
            .then_with(|| (!left.is_preliminary).cmp(&(!right.is_preliminary)))
            .then_with(|| left.date.cmp(&right.date))
            .then_with(|| left_index.cmp(right_index))
    });

    let mut changed = false;
    for (slot_index, (_, sorted_release)) in versioned_indices.into_iter().zip(versioned) {
        if releases[slot_index] != sorted_release {
            releases[slot_index] = sorted_release;
            changed = true;
        }
    }
    changed
}

fn sortPreliminaryVersions(versions: &mut [PreliminaryResourceVersion]) -> bool {
    let mut indexed: Vec<(usize, PreliminaryResourceVersion)> =
        versions.iter().cloned().enumerate().collect();
    indexed.sort_by(|(left_index, left), (right_index, right)| {
        let left_ver = parseResourceVersionNumber(Some(&left.resource_version));
        let right_ver = parseResourceVersionNumber(Some(&right.resource_version));
        left_ver
            .cmp(&right_ver)
            .then_with(|| left.detected_at.cmp(&right.detected_at))
            .then_with(|| left_index.cmp(right_index))
    });

    let mut changed = false;
    for (slot_index, (_, sorted_entry)) in indexed.into_iter().enumerate() {
        if versions[slot_index] != sorted_entry {
            versions[slot_index] = sorted_entry;
            changed = true;
        }
    }
    changed
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

    writeDiffFile(data_dir, previous, current, date).map(Some)
}

/// Diffs two masters and writes the result under the current release's short hash.
fn writeDiffFile(
    data_dir: &Path,
    previous: &MasterSnapshot,
    current: &MasterSnapshot,
    date: &str,
) -> Result<DiffSummary> {
    let diff = diffMasterFiles(previous, current, date)?;
    let path = diffPath(data_dir, &current.sha256[..SHORT_HASH_LENGTH]);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    writeAtomic(&path, &gzip(&serde_json::to_vec(&diff)?)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(diff.summary)
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

fn writeReleases(path: &Path, releases: &[MasterRelease]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    writeAtomic(path, &serde_json::to_vec_pretty(releases)?)
        .with_context(|| format!("failed to write {}", path.display()))
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

fn writePreliminaryVersions(path: &Path, versions: &[PreliminaryResourceVersion]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    writeAtomic(path, &serde_json::to_vec_pretty(versions)?)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn readPreliminaryVersions(path: &Path) -> Result<Vec<PreliminaryResourceVersion>> {
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
    use super::types::{DiffSummary, MasterDiff, MasterRelease, PreliminaryResourceVersion};
    use super::{
        diffPath, markPromotedPreliminaryVersions, preliminaryVersionsPath,
        recordPreliminaryRelease, recordPreliminaryVersion, recordRelease, versionsPath,
        MasterSnapshot,
    };
    use crate::master_fetch::EarlyMasterMeta;
    use flate2::read::GzDecoder;
    use rusqlite::Connection;
    use std::path::{Path, PathBuf};

    const FIRST_HASH: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const SECOND_HASH: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const THIRD_HASH: &str = "3333333333333333333333333333333333333333333333333333333333333333";

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

        recordRelease(&data_dir, &first, None, "2026-10-01", Some("10008780")).unwrap();
        recordRelease(
            &data_dir,
            &second,
            Some(&first),
            "2026-10-08",
            Some("10008790"),
        )
        .unwrap();
        recordRelease(
            &data_dir,
            &second,
            Some(&second),
            "2026-10-09",
            Some("10008790"),
        )
        .unwrap();

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
                    resource_version: Some("10008780".into()),
                    is_preliminary: false,
                    detected_at: None,
                    cdn_last_modified: None,
                },
                MasterRelease {
                    hash: SECOND_HASH.into(),
                    short_hash: "222222222222".into(),
                    date: "2026-10-08".into(),
                    previous_hash: Some(FIRST_HASH.into()),
                    summary: Some(expected_summary),
                    resource_version: Some("10008790".into()),
                    is_preliminary: false,
                    detected_at: None,
                    cdn_last_modified: None,
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

        recordRelease(&data_dir, &first, None, "2026-10-01", None).unwrap();
        recordRelease(&data_dir, &second, Some(&missing_first), "2026-10-08", None).unwrap();

        let latest = readReleases(&data_dir).pop().unwrap();
        assert_eq!(latest.previous_hash.as_deref(), Some(FIRST_HASH));
        assert_eq!(latest.summary, None);
        assert!(!diffPath(&data_dir, "222222222222").exists());
        std::fs::remove_dir_all(data_dir).unwrap();
    }

    #[test]
    fn preliminaryReleaseDiffsAgainstLiveAndPromotesCleanlyWhenLiveCatchesUp() {
        let data_dir = temporaryDirectory("preliminary");
        let live_path = data_dir.join("live.mdb");
        let early_path = data_dir.join("early.mdb");
        let next_live_path = data_dir.join("next_live.mdb");
        writeMaster(
            &live_path,
            "CREATE TABLE item (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO item VALUES (1, 'live');",
        );
        writeMaster(
            &early_path,
            "CREATE TABLE item (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO item VALUES (1, 'live'), (2, 'early');",
        );
        writeMaster(
            &next_live_path,
            "CREATE TABLE item (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO item VALUES (1, 'live'), (2, 'final');",
        );
        let live = MasterSnapshot {
            sha256: FIRST_HASH,
            path: &live_path,
        };
        let early = MasterSnapshot {
            sha256: SECOND_HASH,
            path: &early_path,
        };
        let next_live = MasterSnapshot {
            sha256: THIRD_HASH,
            path: &next_live_path,
        };
        let meta = EarlyMasterMeta {
            resource_version: "10008800".into(),
            live_resource_version: "10008790".into(),
            detected_at: "2026-11-24T12:00:00Z".into(),
            last_modified: Some("Tue, 24 Nov 2026 11:14:05 GMT".into()),
        };

        recordRelease(&data_dir, &live, None, "2026-11-20", Some("10008790")).unwrap();
        recordPreliminaryRelease(&data_dir, &early, &live, &meta, "2026-11-24").unwrap();
        recordPreliminaryVersion(&data_dir, &meta, SECOND_HASH, true).unwrap();

        let releases = readReleases(&data_dir);
        assert_eq!(releases.len(), 2);
        assert!(releases[1].is_preliminary);
        assert_eq!(releases[1].resource_version.as_deref(), Some("10008800"));
        assert_eq!(releases[1].previous_hash.as_deref(), Some(FIRST_HASH));
        assert_eq!(releases[1].summary.unwrap().added, 1);
        assert!(diffPath(&data_dir, "222222222222").is_file());

        // If a different final master goes live, live chains from live (FIRST_HASH -> THIRD_HASH)
        // while keeping the preliminary release entry intact.
        recordRelease(
            &data_dir,
            &next_live,
            Some(&live),
            "2026-12-01",
            Some("10008800"),
        )
        .unwrap();
        markPromotedPreliminaryVersions(&data_dir, "10008800", "2026-12-01T22:00:00Z").unwrap();

        let releases = readReleases(&data_dir);
        assert_eq!(releases.len(), 3);
        assert!(releases[1].is_preliminary);
        assert!(!releases[2].is_preliminary);
        assert_eq!(releases[2].previous_hash.as_deref(), Some(FIRST_HASH));

        // If the exact same preliminary master goes live instead, it promotes in place.
        recordRelease(
            &data_dir,
            &early,
            Some(&live),
            "2026-12-02",
            Some("10008800"),
        )
        .unwrap();
        let promoted = readReleases(&data_dir)
            .into_iter()
            .find(|r| r.hash == SECOND_HASH)
            .unwrap();
        assert!(!promoted.is_preliminary);
        assert_eq!(promoted.date, "2026-12-02");

        let prelim_versions: Vec<PreliminaryResourceVersion> =
            serde_json::from_slice(&std::fs::read(preliminaryVersionsPath(&data_dir)).unwrap())
                .unwrap();
        assert_eq!(prelim_versions.len(), 1);
        assert_eq!(prelim_versions[0].resource_version, "10008800");
        assert_eq!(
            prelim_versions[0].promoted_at.as_deref(),
            Some("2026-12-01T22:00:00Z")
        );
        std::fs::remove_dir_all(data_dir).unwrap();
    }

    #[test]
    fn inBetweenLiveHotfixSortsBeforeHigherPreliminaryAndRefreshesPreliminaryDiff() {
        let data_dir = temporaryDirectory("hotfix-order");
        let legacy_path = data_dir.join("legacy.mdb");
        let live_path = data_dir.join("live.mdb");
        let hotfix_path = data_dir.join("hotfix.mdb");
        let early_path = data_dir.join("early.mdb");
        writeMaster(
            &legacy_path,
            "CREATE TABLE item (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO item VALUES (1, 'legacy');",
        );
        writeMaster(
            &live_path,
            "CREATE TABLE item (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO item VALUES (1, 'live');",
        );
        writeMaster(
            &hotfix_path,
            "CREATE TABLE item (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO item VALUES (1, 'hotfix');",
        );
        writeMaster(
            &early_path,
            "CREATE TABLE item (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO item VALUES (1, 'live'), (2, 'early');",
        );

        let legacy_hash = "0000000000000000000000000000000000000000000000000000000000000000";
        let legacy = MasterSnapshot {
            sha256: legacy_hash,
            path: &legacy_path,
        };
        let live = MasterSnapshot {
            sha256: FIRST_HASH,
            path: &live_path,
        };
        let hotfix = MasterSnapshot {
            sha256: THIRD_HASH,
            path: &hotfix_path,
        };
        let early = MasterSnapshot {
            sha256: SECOND_HASH,
            path: &early_path,
        };
        let meta = EarlyMasterMeta {
            resource_version: "10008800".into(),
            live_resource_version: "10008790".into(),
            detected_at: "2026-11-24T12:00:00Z".into(),
            last_modified: Some("Tue, 24 Nov 2026 11:14:05 GMT".into()),
        };

        recordRelease(&data_dir, &legacy, None, "2026-11-17", None).unwrap();
        recordRelease(
            &data_dir,
            &live,
            Some(&legacy),
            "2026-11-20",
            Some("10008790"),
        )
        .unwrap();
        recordPreliminaryRelease(&data_dir, &early, &live, &meta, "2026-11-24").unwrap();

        // An in-between live hotfix (10008795) arrives after preliminary 10008800 was recorded.
        recordRelease(
            &data_dir,
            &hotfix,
            Some(&live),
            "2026-11-26",
            Some("10008795"),
        )
        .unwrap();

        let releases = readReleases(&data_dir);
        let ordered: Vec<(Option<&str>, bool)> = releases
            .iter()
            .map(|r| (r.resource_version.as_deref(), r.is_preliminary))
            .collect();
        assert_eq!(
            ordered,
            vec![
                (None, false),
                (Some("10008790"), false),
                (Some("10008795"), false),
                (Some("10008800"), true),
            ]
        );
        assert_eq!(releases[2].previous_hash.as_deref(), Some(FIRST_HASH));

        // Re-recording the preliminary release against the new live hotfix refreshes its diff base.
        let updated_meta = EarlyMasterMeta {
            live_resource_version: "10008795".into(),
            ..meta
        };
        recordPreliminaryRelease(&data_dir, &early, &hotfix, &updated_meta, "2026-11-26").unwrap();

        let releases = readReleases(&data_dir);
        assert_eq!(releases[3].hash, SECOND_HASH);
        assert_eq!(releases[3].previous_hash.as_deref(), Some(THIRD_HASH));
        assert_eq!(releases[3].date, "2026-11-24");
        assert_eq!(releases[3].summary.unwrap().added, 1);
        assert_eq!(releases[3].summary.unwrap().modified, 1);

        let compressed = std::fs::read(diffPath(&data_dir, "222222222222")).unwrap();
        let diff: MasterDiff = serde_json::from_reader(GzDecoder::new(&compressed[..])).unwrap();
        assert_eq!(diff.from_hash, THIRD_HASH);
        assert_eq!(diff.to_hash, SECOND_HASH);

        std::fs::remove_dir_all(data_dir).unwrap();
    }
}

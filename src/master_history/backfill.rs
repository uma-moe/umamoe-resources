//! Rebuilds the release history from the game versions uma.moe has seen. Each master is
//! downloaded to a scratch file, diffed against the master before it and deleted again, so only
//! `versions.json` and the diffs remain.

use super::types::{GameVersionHistory, MasterRelease, MasterSnapshot, SHORT_HASH_LENGTH};
use super::{readReleases, versionsPath, writeDiffFile, writeReleases};
use crate::master_fetch::fetchMasterMdb;
use anyhow::{bail, Context, Result};
use chrono::NaiveDateTime;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const VERSION_HISTORY_URL: &str = "https://uma.moe/api/ver/history";

/// A resource version and the UTC day uma.moe first saw it.
#[derive(Debug, PartialEq, Eq)]
struct HistoricVersion {
    resource_version: String,
    date: String,
}

pub async fn backfillMasterHistory(data_dir: &Path) -> Result<()> {
    let versions = historicVersions(fetchVersionHistory().await?)?;
    info!(
        versions = versions.len(),
        "backfilling the master release history"
    );

    let scratch_dir = data_dir.join("masterdata").join("backfill");
    fs::create_dir_all(&scratch_dir)
        .with_context(|| format!("failed to create {}", scratch_dir.display()))?;
    let result = backfillFromVersions(data_dir, &scratch_dir, &versions).await;

    // Masters are only needed for diffing, so none outlive the run, even a failed one.
    if let Err(error) = fs::remove_dir_all(&scratch_dir) {
        warn!(error = %error, path = %scratch_dir.display(), "failed to remove backfill masters");
    }

    let releases = result?;
    info!(releases, "backfilled the master release history");
    Ok(())
}

async fn backfillFromVersions(
    data_dir: &Path,
    scratch_dir: &Path,
    versions: &[HistoricVersion],
) -> Result<usize> {
    let mut backfill = HistoryBackfill::new(data_dir);
    for version in versions {
        let master_path = scratch_dir.join(format!("{}.mdb", version.resource_version));
        // A missing version only widens the next diff, which still names the master it starts from.
        if let Err(error) = fetchMasterMdb(&version.resource_version, &master_path).await {
            warn!(
                resource_version = version.resource_version,
                error = %error,
                "master is unavailable; the next release is diffed against the last available one"
            );
            continue;
        }

        backfill.add(master_path, &version.date)?;
    }

    backfill.finish()
}

async fn fetchVersionHistory() -> Result<GameVersionHistory> {
    reqwest::Client::new()
        .get(VERSION_HISTORY_URL)
        .header("Cache-Control", "no-cache")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .context("failed to load the game version history")?
        .error_for_status()
        .context("the game version history endpoint returned an error")?
        .json()
        .await
        .context("invalid game version history response")
}

/// Distinct resource versions, oldest first, each dated by its first appearance.
fn historicVersions(history: GameVersionHistory) -> Result<Vec<HistoricVersion>> {
    let mut seen = HashSet::new();
    let mut versions = Vec::new();
    let oldest_first = history
        .history
        .into_iter()
        .rev()
        .chain(std::iter::once(history.current));
    for record in oldest_first {
        let resource_version = record.resource_version.trim().to_string();
        // The version becomes part of a CDN path.
        let is_valid_version =
            !resource_version.is_empty() && resource_version.bytes().all(|b| b.is_ascii_digit());
        if !is_valid_version {
            bail!("invalid resource version {resource_version:?} in the game version history");
        }

        let seen_at = NaiveDateTime::parse_from_str(&record.updated_at, "%Y-%m-%dT%H:%M:%S")
            .with_context(|| format!("invalid version date {:?}", record.updated_at))?;
        if seen.insert(resource_version.clone()) {
            versions.push(HistoricVersion {
                resource_version,
                date: seen_at.format("%Y-%m-%d").to_string(),
            });
        }
    }

    Ok(versions)
}

/// Builds the history from masters added oldest first, holding on to only the latest one.
struct HistoryBackfill<'a> {
    data_dir: &'a Path,
    releases: Vec<MasterRelease>,
    /// The master of `releases.last()`, deleted once the next release is diffed against it.
    latest_master: Option<PathBuf>,
}

impl<'a> HistoryBackfill<'a> {
    fn new(data_dir: &'a Path) -> Self {
        Self {
            data_dir,
            releases: Vec::new(),
            latest_master: None,
        }
    }

    /// Takes ownership of the master file: it is deleted as soon as it is no longer needed.
    fn add(&mut self, master_path: PathBuf, date: &str) -> Result<()> {
        let bytes = fs::read(&master_path)
            .with_context(|| format!("failed to read {}", master_path.display()))?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        drop(bytes);

        // Resource updates often leave the master unchanged; like live releases, a master is listed once.
        if self.releases.iter().any(|release| release.hash == sha256) {
            return removeMaster(&master_path);
        }

        let current = MasterSnapshot {
            sha256: &sha256,
            path: &master_path,
        };
        let previous_release = self.releases.last();
        let summary = match previous_release.zip(self.latest_master.as_deref()) {
            Some((release, path)) => {
                let previous = MasterSnapshot {
                    sha256: &release.hash,
                    path,
                };
                Some(writeDiffFile(self.data_dir, &previous, &current, date)?)
            }
            None => None,
        };

        self.releases.push(MasterRelease {
            short_hash: sha256[..SHORT_HASH_LENGTH].to_string(),
            previous_hash: previous_release.map(|release| release.hash.clone()),
            hash: sha256,
            date: date.to_string(),
            summary,
        });
        if let Some(previous_path) = self.latest_master.replace(master_path) {
            removeMaster(&previous_path)?;
        }

        Ok(())
    }

    /// Replaces the history with the backfilled releases followed by any recorded after them.
    fn finish(mut self) -> Result<usize> {
        if let Some(path) = self.latest_master.take() {
            removeMaster(&path)?;
        }

        let Some(latest) = self.releases.last() else {
            bail!("no master of the game version history could be downloaded");
        };
        let versions_path = versionsPath(self.data_dir);
        let recorded = readReleases(&versions_path)?;
        // Releases before the backfill's latest are superseded; later ones continue its chain.
        let later_releases = match recorded
            .iter()
            .position(|release| release.hash == latest.hash)
        {
            Some(index) => &recorded[index + 1..],
            None if recorded.is_empty() => &[],
            None => bail!(
                "the backfill ends at {} but the recorded history does not list it; \
                 keeping the history unchanged",
                latest.short_hash
            ),
        };

        let mut releases = self.releases;
        releases.extend_from_slice(later_releases);
        writeReleases(&versions_path, &releases)?;
        Ok(releases.len())
    }
}

fn removeMaster(path: &Path) -> Result<()> {
    fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::super::types::{DiffSummary, GameVersionRecord};
    use super::super::{diffPath, recordRelease};
    use super::*;
    use rusqlite::Connection;

    fn temporaryDirectory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "master-backfill-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn writeMaster(path: &Path, rows: &str) -> String {
        Connection::open(path)
            .unwrap()
            .execute_batch(&format!(
                "CREATE TABLE item (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO item VALUES {rows};"
            ))
            .unwrap();
        hex::encode(Sha256::digest(fs::read(path).unwrap()))
    }

    fn record(resource_version: &str, updated_at: &str) -> GameVersionRecord {
        GameVersionRecord {
            resource_version: resource_version.into(),
            updated_at: updated_at.into(),
        }
    }

    #[test]
    fn listsEachResourceVersionOnceOldestFirstByItsFirstDay() {
        let history = GameVersionHistory {
            current: record("10000300", "2026-10-30T16:15:22"),
            history: vec![
                record("10000300", "2026-10-27T09:06:38"),
                record("10000200", "2026-10-22T10:50:42"),
                record("10000200", "2026-10-20T08:05:08"),
                record("10000100", "2026-10-14T08:00:15"),
            ],
        };

        let versions = historicVersions(history).unwrap();

        let listed: Vec<(&str, &str)> = versions
            .iter()
            .map(|version| (version.resource_version.as_str(), version.date.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("10000100", "2026-10-14"),
                ("10000200", "2026-10-20"),
                ("10000300", "2026-10-27"),
            ]
        );
    }

    #[test]
    fn rejectsAResourceVersionThatIsNotANumber() {
        let history = GameVersionHistory {
            current: record("../old", "2026-10-30T16:15:22"),
            history: Vec::new(),
        };

        assert!(historicVersions(history).is_err());
    }

    #[test]
    fn keepsOnlyDiffsAndContinuesIntoReleasesRecordedLater() {
        let data_dir = temporaryDirectory("rebuild");
        let first_path = data_dir.join("first.mdb");
        let unchanged_path = data_dir.join("unchanged.mdb");
        let second_path = data_dir.join("second.mdb");
        let later_path = data_dir.join("later.mdb");
        let first_hash = writeMaster(&first_path, "(1, 'old')");
        writeMaster(&unchanged_path, "(1, 'old')");
        let second_hash = writeMaster(&second_path, "(1, 'new'), (2, 'added')");
        let later_hash = writeMaster(&later_path, "(1, 'new')");
        // The live history began at the second master, with no earlier master to diff against.
        let second_copy = data_dir.join("second-copy.mdb");
        fs::copy(&second_path, &second_copy).unwrap();
        let recorded_second = MasterSnapshot {
            sha256: &second_hash,
            path: &second_copy,
        };
        let later = MasterSnapshot {
            sha256: &later_hash,
            path: &later_path,
        };
        recordRelease(&data_dir, &recorded_second, None, "2026-10-30").unwrap();
        recordRelease(&data_dir, &later, Some(&recorded_second), "2026-10-31").unwrap();

        let mut backfill = HistoryBackfill::new(&data_dir);
        backfill.add(first_path.clone(), "2026-10-01").unwrap();
        backfill.add(unchanged_path.clone(), "2026-10-08").unwrap();
        backfill.add(second_path.clone(), "2026-10-15").unwrap();
        let release_count = backfill.finish().unwrap();

        let releases = readReleases(&versionsPath(&data_dir)).unwrap();
        let chain: Vec<(&str, Option<&str>, &str)> = releases
            .iter()
            .map(|release| {
                (
                    release.hash.as_str(),
                    release.previous_hash.as_deref(),
                    release.date.as_str(),
                )
            })
            .collect();
        assert_eq!(release_count, 3);
        assert_eq!(
            chain,
            [
                (first_hash.as_str(), None, "2026-10-01"),
                (
                    second_hash.as_str(),
                    Some(first_hash.as_str()),
                    "2026-10-15"
                ),
                (
                    later_hash.as_str(),
                    Some(second_hash.as_str()),
                    "2026-10-31"
                ),
            ]
        );
        assert_eq!(
            releases[1].summary,
            Some(DiffSummary {
                tables_changed: 1,
                added: 1,
                removed: 0,
                modified: 1,
            })
        );
        assert!(diffPath(&data_dir, &second_hash[..SHORT_HASH_LENGTH]).is_file());
        assert!(!first_path.exists());
        assert!(!unchanged_path.exists());
        assert!(!second_path.exists());
        fs::remove_dir_all(data_dir).unwrap();
    }

    #[test]
    fn leavesTheHistoryAloneWhenTheBackfillDoesNotReachItsLatestRelease() {
        let data_dir = temporaryDirectory("unreached");
        let recorded_path = data_dir.join("recorded.mdb");
        let backfilled_path = data_dir.join("backfilled.mdb");
        let recorded_hash = writeMaster(&recorded_path, "(1, 'live')");
        writeMaster(&backfilled_path, "(1, 'old')");
        let recorded = MasterSnapshot {
            sha256: &recorded_hash,
            path: &recorded_path,
        };
        recordRelease(&data_dir, &recorded, None, "2026-10-30").unwrap();

        let mut backfill = HistoryBackfill::new(&data_dir);
        backfill.add(backfilled_path, "2026-10-01").unwrap();
        let result = backfill.finish();

        let releases = readReleases(&versionsPath(&data_dir)).unwrap();
        assert!(result.is_err());
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].hash, recorded_hash);
        fs::remove_dir_all(data_dir).unwrap();
    }
}

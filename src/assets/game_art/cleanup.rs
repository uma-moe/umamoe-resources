//! Deletes what the publisher put in the bucket once no record lists it: outputs whose game file
//! moved to another key, and outputs of files the catalog no longer builds. Objects it never
//! recorded, such as the copied web root and the timeline banners, are never touched.

use super::plan::publishedRecord;
use super::types::{ArtJob, ArtState, PublishedArt, ORPHAN_ALERT_MINIMUM, ORPHAN_ALERT_PERCENT};
use crate::assets::Bucket;
use anyhow::Result;
use std::collections::{BTreeSet, HashSet};

/// Every key a record put in the bucket.
pub(super) fn recordKeys(record: &PublishedArt) -> impl Iterator<Item = &String> {
    record
        .outputs
        .iter()
        .chain(&record.content_outputs)
        .chain(&record.textures)
}

/// The keys a file's record lists as named after its contents, which are only known once it is
/// converted again.
pub(super) fn keptContentOutputs(state: &ArtState, name: &str) -> Vec<String> {
    state
        .files
        .get(name)
        .map(|record| record.content_outputs.clone())
        .unwrap_or_default()
}

/// The candidates no record in `state` lists any more, so they can be deleted.
pub(super) fn unlistedKeys(state: &ArtState, candidates: BTreeSet<String>) -> Vec<String> {
    let listed: HashSet<&str> = state
        .files
        .values()
        .flat_map(recordKeys)
        .map(String::as_str)
        .collect();
    candidates
        .into_iter()
        .filter(|key| !listed.contains(key.as_str()))
        .collect()
}

/// Whether so many records look orphaned that the file list is more likely incomplete than the
/// game or the catalog changed that much.
pub(super) fn isSuspiciousOrphanCount(orphans: usize, records: usize) -> bool {
    orphans > ORPHAN_ALERT_MINIMUM && orphans * 100 > records * ORPHAN_ALERT_PERCENT
}

/// Removes the records of `names` and returns the keys they listed.
pub(super) fn removeRecords(state: &mut ArtState, names: &[String]) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for name in names {
        if let Some(record) = state.files.remove(name) {
            keys.extend(recordKeys(&record).cloned());
        }
    }
    keys
}

/// What publishing `jobs` and dropping `orphans` would delete, for a dry run.
pub(super) fn plannedDeletions(
    state: &ArtState,
    jobs: &[ArtJob],
    orphans: &[String],
) -> Vec<String> {
    let mut after = state.clone();
    let mut stale_keys = removeRecords(&mut after, orphans);
    for job in jobs {
        // A dry run does not convert, so it cannot tell which content outputs would go.
        let record = publishedRecord(job, None, keptContentOutputs(&after, &job.name));
        if let Some(replaced) = after.files.insert(job.name.clone(), record) {
            stale_keys.extend(recordKeys(&replaced).cloned());
        }
    }
    unlistedKeys(&after, stale_keys)
}

pub(super) async fn deleteKeys(
    client: &reqwest::Client,
    bucket: &Bucket,
    keys: &[String],
) -> Result<()> {
    for key in keys {
        bucket.deleteObject(client, key).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::game_art::types::{Fit, PlannedOutput};
    use crate::game_cdn::{ManifestEntry, GLOBAL_CDN};

    fn record(outputs: &[&str], textures: &[&str]) -> PublishedArt {
        PublishedArt {
            region: "global".to_string(),
            checksum: 1,
            converter: 1,
            outputs: outputs.iter().map(|key| key.to_string()).collect(),
            dependency_checksums: Vec::new(),
            content_outputs: Vec::new(),
            textures: textures.iter().map(|key| key.to_string()).collect(),
            error: None,
        }
    }

    fn job(name: &str, keys: &[&str]) -> ArtJob {
        ArtJob {
            source: &GLOBAL_CDN,
            name: name.to_string(),
            entry: ManifestEntry::new(name, 1, 2, 1),
            dependencies: Vec::new(),
            lookups: Vec::new(),
            model: None,
            outputs: keys
                .iter()
                .map(|key| PlannedOutput {
                    key: key.to_string(),
                    object: name.to_string(),
                    fit: Fit::Native,
                })
                .collect(),
        }
    }

    fn shaderRecord(files: &[&str]) -> PublishedArt {
        PublishedArt {
            content_outputs: files
                .iter()
                .map(|file| format!("game-assets/shaders/{file}"))
                .collect(),
            ..record(&["game-assets/shaders/index.json"], &[])
        }
    }

    #[test]
    fn aShaderTheBundleNoLongerHoldsIsDeleted() {
        let mut state = ArtState::default();
        state.files.insert(
            "android/shader".to_string(),
            shaderRecord(&["kept.json", "gone.json"]),
        );

        let replaced = state
            .files
            .insert("android/shader".to_string(), shaderRecord(&["kept.json"]))
            .unwrap();
        let deletable = unlistedKeys(&state, recordKeys(&replaced).cloned().collect());

        assert_eq!(deletable, vec!["game-assets/shaders/gone.json"]);
    }

    #[test]
    fn aDryRunKeepsTheShadersItCannotKnowAbout() {
        let mut state = ArtState::default();
        state
            .files
            .insert("android/shader".to_string(), shaderRecord(&["kept.json"]));
        let rebuilt = job("android/shader", &["game-assets/shaders/index.json"]);

        let deletions = plannedDeletions(&state, &[rebuilt], &[]);

        assert!(deletions.is_empty());
    }

    #[test]
    fn keysAnotherRecordStillListsAreKept() {
        let mut state = ArtState::default();
        state
            .files
            .insert("new".to_string(), record(&["shared.webp"], &["loose.webp"]));
        let candidates =
            BTreeSet::from(["shared.webp", "loose.webp", "gone.webp"].map(str::to_string));

        let deletable = unlistedKeys(&state, candidates);

        assert_eq!(deletable, vec!["gone.webp"]);
    }

    #[test]
    fn aRebuildThatMovesItsOutputDeletesTheOldKey() {
        let mut state = ArtState::default();
        state.files.insert(
            "3d/chara/body/bdy1001_00".to_string(),
            record(&["game-assets/umas/special-week/old-name.glb"], &[]),
        );
        let renamed = job(
            "3d/chara/body/bdy1001_00",
            &["game-assets/umas/special-week/new-name.glb"],
        );

        let deletions = plannedDeletions(&state, &[renamed], &[]);

        assert_eq!(
            deletions,
            vec!["game-assets/umas/special-week/old-name.glb"]
        );
    }

    #[test]
    fn anOrphanDeletesOnlyWhatNoOtherRecordServes() {
        let mut state = ArtState::default();
        state.files.insert(
            "chara/chr1007/chara_stand_1007_101".to_string(),
            record(&["thumbs/100701.webp", "thumbs/old.webp"], &[]),
        );
        state.files.insert(
            "chara/chr1007/chara_stand_1007_100701".to_string(),
            record(&["thumbs/100701.webp"], &[]),
        );
        let orphans = vec!["chara/chr1007/chara_stand_1007_101".to_string()];

        let deletions = plannedDeletions(&state, &[], &orphans);

        assert_eq!(deletions, vec!["thumbs/old.webp"]);
    }

    #[test]
    fn aBigShareOfOrphansLooksLikeABadFileList() {
        assert!(!isSuspiciousOrphanCount(3, 10));
        assert!(!isSuspiciousOrphanCount(100, 1000));
        assert!(isSuspiciousOrphanCount(300, 1000));
    }
}

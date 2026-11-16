//! Deletes what the publisher put in the bucket once no record lists it: outputs whose game file
//! moved to another key, and outputs of files the catalog no longer builds. Objects it never
//! recorded, such as the copied web root and the timeline banners, are never touched.

use super::plan::publishedRecord;
use super::types::{
    ArtJob, ArtState, ConvertedArt, PublishedArt, ORPHAN_ALERT_MINIMUM, ORPHAN_ALERT_PERCENT,
};
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

/// What a file's record says its last conversion reported: the keys named after its contents and
/// the shaders it uses, which are only known once it is converted again.
pub(super) fn keptConversion(state: &ArtState, name: &str) -> ConvertedArt {
    state
        .files
        .get(name)
        .map(|record| ConvertedArt {
            content_outputs: record.content_outputs.clone(),
            shaders: record.shaders.clone(),
        })
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

/// What publishing `jobs` and dropping `orphans` would leave, for a dry run, if every job
/// converted.
pub(super) struct PlannedRun {
    pub(super) state: ArtState,
    pub(super) deletions: Vec<String>,
}

pub(super) fn plannedRun(state: &ArtState, jobs: &[ArtJob], orphans: &[String]) -> PlannedRun {
    let mut after = state.clone();
    let mut stale_keys = removeRecords(&mut after, orphans);
    for job in jobs {
        // A dry run does not convert, so it cannot tell which content outputs would go.
        let record = publishedRecord(job, None, keptConversion(&after, &job.name));
        if let Some(replaced) = after.files.insert(job.name.clone(), record) {
            stale_keys.extend(recordKeys(&replaced).cloned());
        }
    }
    let deletions = unlistedKeys(&after, stale_keys);
    PlannedRun {
        state: after,
        deletions,
    }
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
            shaders: Vec::new(),
            links: None,
        }
    }

    fn job(name: &str, keys: &[&str]) -> ArtJob {
        ArtJob {
            source: &GLOBAL_CDN,
            name: name.to_string(),
            entry: ManifestEntry::new(name, 1, 2, 1),
            dependencies: Vec::new(),
            lookups: Vec::new(),
            links: Default::default(),
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
                .map(|file| format!("art/shaders/{file}"))
                .collect(),
            ..record(&["art/shaders/index.json"], &[])
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

        assert_eq!(deletable, vec!["art/shaders/gone.json"]);
    }

    #[test]
    fn aDryRunKeepsTheShadersItCannotKnowAbout() {
        let mut state = ArtState::default();
        state
            .files
            .insert("android/shader".to_string(), shaderRecord(&["kept.json"]));
        let rebuilt = job("android/shader", &["art/shaders/index.json"]);

        let planned = plannedRun(&state, &[rebuilt], &[]);

        assert!(planned.deletions.is_empty());
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
            record(&["art/umas/100101/model-old.glb"], &[]),
        );
        let renamed = job("3d/chara/body/bdy1001_00", &["art/umas/100101/model.glb"]);

        let planned = plannedRun(&state, &[renamed], &[]);

        assert_eq!(planned.deletions, vec!["art/umas/100101/model-old.glb"]);
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

        let planned = plannedRun(&state, &[], &orphans);

        assert_eq!(planned.deletions, vec!["thumbs/old.webp"]);
    }

    #[test]
    fn aBigShareOfOrphansLooksLikeABadFileList() {
        assert!(!isSuspiciousOrphanCount(3, 10));
        assert!(!isSuspiciousOrphanCount(100, 1000));
        assert!(isSuspiciousOrphanCount(300, 1000));
    }
}

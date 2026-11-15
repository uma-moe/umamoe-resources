//! The publisher's record of what is in the bucket, kept as one JSON shard per kind of game
//! file and an index linking them.

use super::catalog::trackGroup;
use super::race_motions::raceMotion;
use super::types::{
    ArtState, PublishedArt, ShardLink, StateIndex, StateLayout, StateShard, StoredIndex,
    StoredShards, STATE_FOLDER, UPLOAD_CONCURRENCY,
};
use super::umas::isUmaRecord;
use crate::assets::Bucket;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use tokio::task::JoinSet;
use tracing::warn;

/// Track scenery by its kind, `racetracks` or `skyboxes`, umas and race motions apart; every
/// other file by its top folder, `chara` for `chara/chr1001/chr_icon_1001`.
pub(super) fn shardName(name: &str) -> &str {
    if let Some(group) = trackGroup(name) {
        return group;
    }
    if isUmaRecord(name) {
        return "umas";
    }
    if raceMotion(name).is_some() {
        return "uma-motions";
    }
    name.split_once('/').map_or(name, |(top, _)| top)
}

pub(super) async fn loadState(
    client: &reqwest::Client,
    bucket: &Bucket,
) -> Result<(ArtState, StateLayout)> {
    let Some(bytes) = bucket.getObject(client, &indexKey()).await? else {
        return Ok((ArtState::default(), StateLayout::Current));
    };

    let index: StoredIndex = serde_json::from_slice(&bytes)
        .context("the game art index in the bucket is not valid JSON")?;
    let (stored_shards, linked): (Vec<String>, bool) = match index.shards {
        StoredShards::Links(links) => (links.into_iter().map(|link| link.name).collect(), true),
        StoredShards::Counts(counts) => (counts.into_keys().collect(), false),
    };
    let shards = loadShards(client, bucket, stored_shards.clone()).await?;
    let has_moved_records = hasMovedRecords(&shards);
    let state = ArtState {
        resource_version: index.resource_version,
        files: shards.into_values().flatten().collect(),
    };

    let obsolete_keys = obsoleteShardKeys(&state, &stored_shards);
    let layout = if linked && obsolete_keys.is_empty() && !has_moved_records {
        StateLayout::Current
    } else {
        StateLayout::Outdated { obsolete_keys }
    };
    Ok((state, layout))
}

/// Writes the shards that hold a `changed` file, then the index, so the index never links a
/// shard that was not written.
pub(super) async fn saveState(
    client: &reqwest::Client,
    bucket: &Bucket,
    state: &ArtState,
    changed: &BTreeSet<String>,
) -> Result<()> {
    let (shards, index) = shardRecords(state, changed);
    for (shard, records) in &shards {
        putJson(client, bucket, &shardKey(shard), records).await?;
    }
    putJson(client, bucket, &indexKey(), &index).await
}

/// Writes every shard in the current layout, then removes the files of the earlier one.
pub(super) async fn rewriteState(
    client: &reqwest::Client,
    bucket: &Bucket,
    state: &ArtState,
    obsolete_keys: &[String],
) -> Result<()> {
    let every_shard = state
        .files
        .keys()
        .map(|name| shardName(name).to_string())
        .collect();
    saveState(client, bucket, state, &every_shard).await?;
    for key in obsolete_keys {
        bucket.deleteObject(client, key).await?;
    }
    Ok(())
}

/// The keys of stored shards that the current layout no longer writes.
fn obsoleteShardKeys(state: &ArtState, stored_shards: &[String]) -> Vec<String> {
    let current: BTreeSet<&str> = state.files.keys().map(|name| shardName(name)).collect();
    stored_shards
        .iter()
        .filter(|shard| !current.contains(shard.as_str()))
        .map(|shard| shardKey(shard))
        .collect()
}

/// Whether a stored shard holds a record that belongs to another shard now.
fn hasMovedRecords(shards: &BTreeMap<String, BTreeMap<String, PublishedArt>>) -> bool {
    shards
        .iter()
        .any(|(shard, files)| files.keys().any(|name| shardName(name) != shard))
}

/// The records of every `changed` shard, and the index linking all shards.
fn shardRecords<'a>(
    state: &'a ArtState,
    changed: &BTreeSet<String>,
) -> (BTreeMap<&'a str, StateShard<&'a PublishedArt>>, StateIndex) {
    let mut shards: BTreeMap<&str, StateShard<&PublishedArt>> = BTreeMap::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();

    for (name, record) in &state.files {
        let shard = shardName(name);
        if changed.contains(shard) {
            shards
                .entry(shard)
                .or_insert_with(|| StateShard {
                    files: BTreeMap::new(),
                })
                .files
                .insert(name.clone(), record);
        }
        *counts.entry(shard).or_default() += 1;
    }

    let index = StateIndex {
        resource_version: state.resource_version.clone(),
        generated_at: Utc::now().to_rfc3339(),
        shards: counts
            .into_iter()
            .map(|(shard, files)| ShardLink {
                name: shard.to_string(),
                path: format!("/{}", shardKey(shard)),
                files,
            })
            .collect(),
    };
    (shards, index)
}

/// Reads the listed shards' records by shard. A shard that has gone missing reads as empty, so
/// its files are rebuilt instead of stopping every run.
async fn loadShards(
    client: &reqwest::Client,
    bucket: &Bucket,
    shards: Vec<String>,
) -> Result<BTreeMap<String, BTreeMap<String, PublishedArt>>> {
    let mut files = BTreeMap::new();
    let mut pending = shards.into_iter();
    let mut tasks = JoinSet::new();

    loop {
        while tasks.len() < UPLOAD_CONCURRENCY {
            let Some(shard) = pending.next() else {
                break;
            };
            let client = client.clone();
            let bucket = bucket.clone();
            tasks.spawn(async move {
                let key = shardKey(&shard);
                let Some(bytes) = bucket.getObject(&client, &key).await? else {
                    warn!(key, "a game art shard named in the index is missing");
                    return anyhow::Ok((shard, BTreeMap::new()));
                };
                let records: StateShard<PublishedArt> = serde_json::from_slice(&bytes)
                    .with_context(|| format!("{key} is not valid JSON"))?;
                anyhow::Ok((shard, records.files))
            });
        }

        let Some(joined) = tasks.join_next().await else {
            break;
        };
        let (shard, records) = joined??;
        files.insert(shard, records);
    }

    Ok(files)
}

async fn putJson(
    client: &reqwest::Client,
    bucket: &Bucket,
    key: &str,
    value: &impl Serialize,
) -> Result<()> {
    let body = serde_json::to_vec_pretty(value)?;
    bucket
        .putObject(client, key, body, "application/json")
        .await
}

fn indexKey() -> String {
    format!("{STATE_FOLDER}/index.json")
}

fn shardKey(shard: &str) -> String {
    format!("{STATE_FOLDER}/{shard}.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(checksum: u64) -> PublishedArt {
        PublishedArt {
            region: "global".to_string(),
            checksum,
            converter: 1,
            outputs: Vec::new(),
            dependency_checksums: Vec::new(),
            textures: Vec::new(),
            error: None,
        }
    }

    fn stateWith(names: &[&str]) -> ArtState {
        let mut state = ArtState::default();
        for (checksum, name) in names.iter().enumerate() {
            state
                .files
                .insert(name.to_string(), record(checksum as u64));
        }
        state
    }

    #[test]
    fn filesShareTheShardOfTheirTopFolderOrTrackScenery() {
        assert_eq!(
            shardName("chara/chr1001/petit/petit_chr_1001_100101_0020"),
            "chara"
        );
        assert_eq!(
            shardName("supportcard/support10001/tex_support_card_10001"),
            "supportcard"
        );
        assert_eq!(shardName("3d/env/race/race10001"), "racetracks");
        assert_eq!(
            shardName("3d/env/race/race10001/race10001_0000/pfb_env_race10001_0000_002"),
            "racetrack-parts"
        );
        assert_eq!(
            shardName("3d/env/race/race00000/race00000_9000/pfb_env_race00000_9000_112"),
            "skyboxes"
        );
        assert_eq!(shardName("shader"), "shader");
    }

    #[test]
    fn umasAndRaceMotionsHaveShardsOfTheirOwn() {
        let uma = shardName("3d/chara/body/bdy1001_00");
        let motion = shardName("3d/motion/racemain/body/type01/anm_rac_type01_run03_base");

        assert_eq!(uma, "umas");
        assert_eq!(motion, "uma-motions");
    }

    #[test]
    fn onlyChangedShardsAreWrittenButTheIndexLinksEveryShard() {
        let state = stateWith(&[
            "chara/chr1001/chr_icon_1001",
            "chara/chr1002/petit/petit_chr_1002_100201_0020",
            "item/item_icon_00041",
        ]);
        let changed = BTreeSet::from(["chara".to_string()]);

        let (shards, index) = shardRecords(&state, &changed);

        let written: Vec<&str> = shards.keys().copied().collect();
        assert_eq!(written, vec!["chara"]);
        assert_eq!(shards["chara"].files.len(), 2);
        let expected_links = vec![
            ShardLink {
                name: "chara".to_string(),
                path: "/_state/game-art/chara.json".to_string(),
                files: 2,
            },
            ShardLink {
                name: "item".to_string(),
                path: "/_state/game-art/item.json".to_string(),
                files: 1,
            },
        ];
        assert_eq!(index.shards, expected_links);
    }

    #[test]
    fn shardsOfTheEarlierLayoutAreObsolete() {
        let state = stateWith(&[
            "chara/chr1001/chr_icon_1001",
            "supportcard/support10001/tex_support_card_10001",
        ]);
        let stored_shards = vec!["chara/chr1001".to_string(), "supportcard".to_string()];

        let obsolete_keys = obsoleteShardKeys(&state, &stored_shards);

        assert_eq!(obsolete_keys, vec!["_state/game-art/chara/chr1001.json"]);
    }

    #[test]
    fn recordsKeptInAnotherShardHaveMoved() {
        let part = "3d/env/race/race10001/race10001_0000/pfb_env_race10001_0000_002";
        let in_place = BTreeMap::from([(
            "racetrack-parts".to_string(),
            BTreeMap::from([(part.to_string(), record(1))]),
        )]);
        let in_earlier_shard = BTreeMap::from([(
            "racetracks".to_string(),
            BTreeMap::from([(part.to_string(), record(1))]),
        )]);

        assert!(!hasMovedRecords(&in_place));
        assert!(hasMovedRecords(&in_earlier_shard));
    }

    #[test]
    fn earlierIndexesReadAsShardCounts() {
        let earlier = r#"{"resource_version": "10008600", "shards": {"chara/chr1001": 4}}"#;

        let index: StoredIndex = serde_json::from_str(earlier).unwrap();

        let StoredShards::Counts(counts) = index.shards else {
            panic!("an earlier index should read as shard counts");
        };
        assert_eq!(counts["chara/chr1001"], 4);
    }
}

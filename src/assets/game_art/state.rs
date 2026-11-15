//! The publisher's record of what is in the bucket, kept as a folder of JSON shards with an index,
//! so no single file grows to thousands of records.

use super::types::{
    ArtState, PublishedArt, StateIndex, StateOrigin, StateShard, LEGACY_STATE_KEY, STATE_FOLDER,
    UPLOAD_CONCURRENCY,
};
use crate::assets::Bucket;
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use tokio::task::JoinSet;
use tracing::warn;

/// Characters and race tracks get a shard each, since chibi sprites and prefabs make them the
/// largest groups; every other game file is grouped by its top folder.
pub(super) fn shardName(name: &str) -> String {
    let segments: Vec<&str> = name.split('/').collect();
    match segments.as_slice() {
        ["chara", character, _, ..] => format!("chara/{character}"),
        ["3d", "env", "race", track, _, ..] => format!("3d/env/race/{track}"),
        [top, _, ..] => top.to_string(),
        _ => name.to_string(),
    }
}

pub(super) async fn loadState(
    client: &reqwest::Client,
    bucket: &Bucket,
) -> Result<(ArtState, StateOrigin)> {
    if let Some(bytes) = bucket.getObject(client, &indexKey()).await? {
        let index: StateIndex = serde_json::from_slice(&bytes)
            .context("the game art index in the bucket is not valid JSON")?;
        let files = loadShards(client, bucket, index.shards.into_keys().collect()).await?;
        let state = ArtState {
            resource_version: index.resource_version,
            files,
        };
        return Ok((state, StateOrigin::Shards));
    }

    if let Some(bytes) = bucket.getObject(client, LEGACY_STATE_KEY).await? {
        let state: ArtState = serde_json::from_slice(&bytes)
            .context("the legacy game art record in the bucket is not valid JSON")?;
        return Ok((state, StateOrigin::Legacy));
    }

    Ok((ArtState::default(), StateOrigin::Empty))
}

/// Writes the shards that hold a `changed` file, then the index, so the index never names a
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

/// Splits the single-file record earlier versions kept into shards, then removes that file.
pub(super) async fn migrateLegacyState(
    client: &reqwest::Client,
    bucket: &Bucket,
    state: &ArtState,
) -> Result<()> {
    let every_shard = state.files.keys().map(|name| shardName(name)).collect();
    saveState(client, bucket, state, &every_shard).await?;
    bucket.deleteObject(client, LEGACY_STATE_KEY).await
}

/// The records of every `changed` shard, and the index counting the records of all shards.
fn shardRecords<'a>(
    state: &'a ArtState,
    changed: &BTreeSet<String>,
) -> (BTreeMap<String, StateShard<&'a PublishedArt>>, StateIndex) {
    let mut shards: BTreeMap<String, StateShard<&PublishedArt>> = BTreeMap::new();
    let mut index = StateIndex {
        resource_version: state.resource_version.clone(),
        shards: BTreeMap::new(),
    };

    for (name, record) in &state.files {
        let shard = shardName(name);
        if changed.contains(&shard) {
            shards
                .entry(shard.clone())
                .or_insert_with(|| StateShard {
                    files: BTreeMap::new(),
                })
                .files
                .insert(name.clone(), record);
        }
        *index.shards.entry(shard).or_default() += 1;
    }

    (shards, index)
}

/// Reads the listed shards. A shard that has gone missing reads as empty, so its files are
/// rebuilt instead of stopping every run.
async fn loadShards(
    client: &reqwest::Client,
    bucket: &Bucket,
    shards: Vec<String>,
) -> Result<BTreeMap<String, PublishedArt>> {
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
                    return anyhow::Ok(BTreeMap::new());
                };
                let shard: StateShard<PublishedArt> = serde_json::from_slice(&bytes)
                    .with_context(|| format!("{key} is not valid JSON"))?;
                anyhow::Ok(shard.files)
            });
        }

        let Some(joined) = tasks.join_next().await else {
            break;
        };
        files.extend(joined??);
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

    #[test]
    fn charactersGetAShardAndOtherFilesShareTheirTopFolder() {
        assert_eq!(
            shardName("chara/chr1001/petit/petit_chr_1001_100101_0020"),
            "chara/chr1001"
        );
        assert_eq!(shardName("chara/chr1001/chr_icon_1001"), "chara/chr1001");
        assert_eq!(
            shardName("supportcard/support10001/tex_support_card_10001"),
            "supportcard"
        );
        assert_eq!(shardName("atlas/common/common"), "atlas");
        assert_eq!(
            shardName("3d/env/race/race10001/race10001_0000/pfb_env_race10001_0000_002"),
            "3d/env/race/race10001"
        );
    }

    #[test]
    fn onlyChangedShardsAreWrittenButTheIndexCountsEveryShard() {
        let mut state = ArtState::default();
        state
            .files
            .insert("chara/chr1001/chr_icon_1001".to_string(), record(1));
        state.files.insert(
            "chara/chr1001/petit/petit_chr_1001_100101_0020".to_string(),
            record(2),
        );
        state
            .files
            .insert("item/item_icon_00041".to_string(), record(3));
        let changed = BTreeSet::from(["chara/chr1001".to_string()]);

        let (shards, index) = shardRecords(&state, &changed);

        let written: Vec<&String> = shards.keys().collect();
        assert_eq!(written, vec!["chara/chr1001"]);
        assert_eq!(shards["chara/chr1001"].files.len(), 2);
        assert_eq!(
            index.shards,
            BTreeMap::from([("chara/chr1001".to_string(), 2), ("item".to_string(), 1)])
        );
    }
}

//! Lists, moves and empties the asset bucket's trees, for reorganising or starting the published
//! art over.

use super::types::{Bucket, FolderTotals, ListedObject};
use anyhow::{bail, Result};
use std::collections::{BTreeMap, HashSet};
use tokio::task::JoinSet;
use tracing::info;

const DELETE_CONCURRENCY: usize = 16;
const COPY_CONCURRENCY: usize = 16;

/// A published tree's files, and the records its publishers keep of them.
const TREE_ART_FOLDER: &str = "art/";
const TREE_RECORDS_FOLDER: &str = "_state/";

/// Objects and bytes by top folder, `art` or `_state`; keys without a folder count under "".
fn folderTotals(objects: &[ListedObject]) -> BTreeMap<String, FolderTotals> {
    let mut totals: BTreeMap<String, FolderTotals> = BTreeMap::new();
    for object in objects {
        let folder = object.key.split_once('/').map_or("", |(folder, _)| folder);
        let entry = totals.entry(folder.to_string()).or_default();
        entry.objects += 1;
        entry.bytes += object.size;
    }
    totals
}

/// Logs how many objects and bytes each top folder holds, and returns the number of objects.
pub async fn bucketInventory(bucket: &Bucket) -> Result<usize> {
    let client = reqwest::Client::new();
    let objects = bucket.listObjects(&client, "").await?;
    for (folder, totals) in folderTotals(&objects) {
        info!(
            folder,
            objects = totals.objects,
            bytes = totals.bytes,
            "asset bucket folder"
        );
    }
    info!(objects = objects.len(), "asset bucket");
    Ok(objects.len())
}

/// Deletes every object, only when the bucket holds exactly `expected` of them, so nothing
/// written since the inventory is removed unseen.
pub async fn clearBucket(bucket: &Bucket, expected: usize) -> Result<usize> {
    let client = reqwest::Client::new();
    let objects = bucket.listObjects(&client, "").await?;
    if objects.len() != expected {
        bail!(
            "the bucket holds {} objects, not the {expected} expected; nothing was deleted",
            objects.len()
        );
    }

    let mut deletions = JoinSet::new();
    let mut pending = objects.into_iter();
    let mut deleted = 0;
    loop {
        while deletions.len() < DELETE_CONCURRENCY {
            let Some(object) = pending.next() else {
                break;
            };
            let (client, bucket) = (client.clone(), bucket.clone());
            deletions.spawn(async move { bucket.deleteObject(&client, &object.key).await });
        }
        let Some(deletion) = deletions.join_next().await else {
            break;
        };
        deletion??;
        deleted += 1;
        if deleted % 1000 == 0 {
            info!(deleted, expected, "clearing the asset bucket");
        }
    }

    info!(deleted, "cleared the asset bucket");
    Ok(deleted)
}

/// Copies the tree published at the bucket's root into `tree`'s folder, unless that folder already
/// holds publishing records. The root's copy stays until it is cleared on purpose.
pub async fn moveRootTree(root: &Bucket, tree: &Bucket) -> Result<usize> {
    let client = reqwest::Client::new();
    if !tree
        .listObjects(&client, TREE_RECORDS_FOLDER)
        .await?
        .is_empty()
    {
        return Ok(0);
    }

    let root_art = root.listObjects(&client, TREE_ART_FOLDER).await?;
    let root_records = root.listObjects(&client, TREE_RECORDS_FOLDER).await?;
    let tree_art = tree.listObjects(&client, TREE_ART_FOLDER).await?;
    let keys = treeMoveKeys(&root_art, &root_records, &tree_art);
    info!(
        objects = keys.len(),
        already_copied = root_art.len() + root_records.len() - keys.len(),
        "moving the root art tree into its folder"
    );

    let mut copies = JoinSet::new();
    let mut pending = keys.iter().cloned();
    let mut copied = 0;
    loop {
        while copies.len() < COPY_CONCURRENCY {
            let Some(key) = pending.next() else {
                break;
            };
            let (client, root, tree) = (client.clone(), root.clone(), tree.clone());
            copies.spawn(async move { tree.copyObjectFrom(&client, &root, &key).await });
        }
        let Some(copy) = copies.join_next().await else {
            break;
        };
        copy??;
        copied += 1;
        if copied % 1000 == 0 {
            info!(copied, objects = keys.len(), "moving the root art tree");
        }
    }

    info!(copied, "moved the root art tree into its folder");
    Ok(copied)
}

/// The keys to copy, records last: publishers trust a tree's records, so they may only appear once
/// every file they list is there. Art an interrupted move already copied is not copied again.
fn treeMoveKeys(
    root_art: &[ListedObject],
    root_records: &[ListedObject],
    tree_art: &[ListedObject],
) -> Vec<String> {
    let copied: HashSet<&str> = tree_art.iter().map(|object| object.key.as_str()).collect();
    let missing_art = root_art
        .iter()
        .filter(|object| !copied.contains(object.key.as_str()));
    missing_art
        .chain(root_records)
        .map(|object| object.key.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aTreeMoveCopiesMissingArtBeforeTheRecords() {
        let object = |key: &str| ListedObject {
            key: key.to_string(),
            size: 1,
        };
        let root_art = [object("art/a.webp"), object("art/b.webp")];
        let root_records = [object("_state/game-art/index.json")];
        let tree_art = [object("art/a.webp")];

        let keys = treeMoveKeys(&root_art, &root_records, &tree_art);

        assert_eq!(keys, ["art/b.webp", "_state/game-art/index.json"]);
    }

    #[test]
    fn objectsAreCountedByTheirTopFolder() {
        let object = |key: &str, size| ListedObject {
            key: key.to_string(),
            size,
        };
        let objects = [
            object("art/umas/100101/model.glb", 300),
            object("art/index.json", 20),
            object("_state/game-art/index.json", 5),
            object("robots.txt", 1),
        ];

        let totals = folderTotals(&objects);

        assert_eq!(
            totals,
            BTreeMap::from([
                (
                    "".to_string(),
                    FolderTotals {
                        objects: 1,
                        bytes: 1
                    }
                ),
                (
                    "_state".to_string(),
                    FolderTotals {
                        objects: 1,
                        bytes: 5
                    }
                ),
                (
                    "art".to_string(),
                    FolderTotals {
                        objects: 2,
                        bytes: 320
                    }
                ),
            ])
        );
    }
}

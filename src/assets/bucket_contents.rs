//! Lists and empties the asset bucket, for starting the published art over.

use super::types::{Bucket, FolderTotals, ListedObject};
use anyhow::{bail, Result};
use std::collections::BTreeMap;
use tokio::task::JoinSet;
use tracing::info;

const DELETE_CONCURRENCY: usize = 16;

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

#[cfg(test)]
mod tests {
    use super::*;

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

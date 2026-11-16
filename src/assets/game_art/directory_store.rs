//! Writes the directory documents where the art goes: only those that changed since the last
//! write, and removes those no longer written. The bucket keeps the MD5 of every document it was
//! given at `DIRECTORY_STATE_KEY`, so a run compares without reading the documents back.

use super::directory::directoryDocuments;
use super::layout::Layout;
use super::types::{
    ArtDestination, ArtState, Catalog, DIRECTORY_STATE_KEY, DOCUMENT_SIZE_LIMIT, UPLOAD_CONCURRENCY,
};
use crate::assets::types::DOCUMENT_CACHE_CONTROL;
use crate::assets::{purgeOverwritten, Bucket};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tracing::{info, warn};

const LARGEST_REPORTED: usize = 12;

/// During a run documents are rewritten at most this often, which keeps purges and the document
/// list's uploads few over a rebuild of several hours; the end of a run always writes them.
const WRITE_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Each written document's MD5, by key.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct WrittenDocuments {
    documents: BTreeMap<String, String>,
}

/// What one write changed.
#[derive(Debug, Default, PartialEq)]
struct DocumentChanges {
    written: Vec<String>,
    /// Written keys that held an earlier version, whose cached copies must be purged.
    replaced: Vec<String>,
    removed: Vec<String>,
}

/// Builds the documents from the publisher's records and writes them to one destination.
pub(super) struct DirectoryWriter<'a> {
    catalog: &'a Catalog,
    layout: &'a Layout,
    written: WrittenDocuments,
    last_write: Option<Instant>,
}

impl<'a> DirectoryWriter<'a> {
    /// A writer for a folder, which starts empty.
    pub(super) fn new(catalog: &'a Catalog, layout: &'a Layout) -> Self {
        Self {
            catalog,
            layout,
            written: WrittenDocuments::default(),
            last_write: None,
        }
    }

    /// A writer for the bucket, which knows the documents the last run wrote.
    pub(super) async fn forBucket(
        client: &reqwest::Client,
        bucket: &Bucket,
        catalog: &'a Catalog,
        layout: &'a Layout,
    ) -> Result<Self> {
        let written = match bucket.getObject(client, DIRECTORY_STATE_KEY).await? {
            Some(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("{DIRECTORY_STATE_KEY} is not a document list"))?,
            None => WrittenDocuments::default(),
        };
        Ok(Self {
            catalog,
            layout,
            written,
            last_write: None,
        })
    }

    /// Writes the documents unless they were written less than `WRITE_INTERVAL` ago.
    pub(super) async fn writeWhenDue(
        &mut self,
        client: &reqwest::Client,
        destination: &ArtDestination<'_>,
        state: &ArtState,
        purge_origins: &[String],
    ) -> Result<()> {
        let is_due = self
            .last_write
            .is_none_or(|written_at| written_at.elapsed() >= WRITE_INTERVAL);
        if !is_due {
            return Ok(());
        }
        self.write(client, destination, state, purge_origins).await
    }

    pub(super) async fn write(
        &mut self,
        client: &reqwest::Client,
        destination: &ArtDestination<'_>,
        state: &ArtState,
        purge_origins: &[String],
    ) -> Result<()> {
        self.last_write = Some(Instant::now());
        let documents = directoryDocuments(self.catalog, self.layout, state)?;
        reportOversized(&documents);
        let changes = plannedChanges(&self.written, &documents);
        if changes == DocumentChanges::default() {
            return Ok(());
        }

        match destination {
            ArtDestination::Folder(folder) => {
                for key in &changes.written {
                    let target = folder.join(key);
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&target, &documents[key])
                        .with_context(|| format!("failed to write {}", target.display()))?;
                }
                for key in &changes.removed {
                    std::fs::remove_file(folder.join(key)).ok();
                }
            }
            ArtDestination::Bucket(bucket) => {
                uploadDocuments(client, bucket, &changes.written, &documents).await?;
                for key in &changes.removed {
                    bucket.deleteObject(client, key).await?;
                }
            }
        }

        for key in &changes.removed {
            self.written.documents.remove(key);
        }
        for key in &changes.written {
            self.written
                .documents
                .insert(key.clone(), documentHash(&documents[key]));
        }
        if let ArtDestination::Bucket(bucket) = destination {
            let body = serde_json::to_vec(&self.written)?;
            bucket
                .putObject(client, DIRECTORY_STATE_KEY, body, "application/json")
                .await?;
            let stale: Vec<String> = changes
                .replaced
                .iter()
                .chain(&changes.removed)
                .cloned()
                .collect();
            purgeOverwritten(&stale, purge_origins).await?;
        }

        info!(
            documents = documents.len(),
            written = changes.written.len(),
            removed = changes.removed.len(),
            "wrote the game art directory"
        );
        Ok(())
    }
}

/// The documents whose bytes differ from what was last written, and the written ones that are
/// gone.
fn plannedChanges(
    written: &WrittenDocuments,
    documents: &BTreeMap<String, Vec<u8>>,
) -> DocumentChanges {
    let mut changes = DocumentChanges::default();
    for (key, bytes) in documents {
        let previous = written.documents.get(key);
        if previous == Some(&documentHash(bytes)) {
            continue;
        }
        if previous.is_some() {
            changes.replaced.push(key.clone());
        }
        changes.written.push(key.clone());
    }
    changes.removed = written
        .documents
        .keys()
        .filter(|key| !documents.contains_key(*key))
        .cloned()
        .collect();
    changes
}

async fn uploadDocuments(
    client: &reqwest::Client,
    bucket: &Bucket,
    keys: &[String],
    documents: &BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let mut uploads = JoinSet::new();
    let mut pending = keys.iter();
    loop {
        while uploads.len() < UPLOAD_CONCURRENCY {
            let Some(key) = pending.next() else {
                break;
            };
            let (client, bucket, key, body) = (
                client.clone(),
                bucket.clone(),
                key.clone(),
                documents[key].clone(),
            );
            uploads.spawn(async move {
                bucket
                    .putObjectWithCache(
                        &client,
                        &key,
                        body,
                        "application/json",
                        DOCUMENT_CACHE_CONTROL,
                    )
                    .await
            });
        }
        let Some(upload) = uploads.join_next().await else {
            break;
        };
        upload??;
    }
    Ok(())
}

fn reportOversized(documents: &BTreeMap<String, Vec<u8>>) {
    for (key, bytes) in documents {
        if bytes.len() > DOCUMENT_SIZE_LIMIT {
            warn!(
                key,
                bytes = bytes.len(),
                "a game art document is over its size limit"
            );
        }
    }
}

/// Logs how many documents there are, their total size and the largest ones.
pub(super) fn reportDocumentSizes(documents: &BTreeMap<String, Vec<u8>>) {
    let total: usize = documents.values().map(Vec::len).sum();
    info!(
        documents = documents.len(),
        bytes = total,
        "game art directory"
    );

    let mut by_size: Vec<(&String, usize)> = documents
        .iter()
        .map(|(key, bytes)| (key, bytes.len()))
        .collect();
    by_size.sort_unstable_by(|left, right| right.1.cmp(&left.1));
    for (key, bytes) in by_size.into_iter().take(LARGEST_REPORTED) {
        info!(key, bytes, "large game art document");
    }
}

fn documentHash(bytes: &[u8]) -> String {
    format!("{:x}", md5::compute(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn documents(entries: &[(&str, &str)]) -> BTreeMap<String, Vec<u8>> {
        entries
            .iter()
            .map(|(key, body)| (key.to_string(), body.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn onlyChangedDocumentsAreWrittenAndGoneOnesRemoved() {
        let mut written = WrittenDocuments::default();
        for (key, body) in [
            ("art/kept.json", "1"),
            ("art/changed.json", "1"),
            ("art/gone.json", "1"),
        ] {
            written
                .documents
                .insert(key.to_string(), documentHash(body.as_bytes()));
        }
        let next = documents(&[
            ("art/kept.json", "1"),
            ("art/changed.json", "2"),
            ("art/new.json", "1"),
        ]);

        let changes = plannedChanges(&written, &next);

        assert_eq!(
            changes,
            DocumentChanges {
                written: vec!["art/changed.json".to_string(), "art/new.json".to_string()],
                replaced: vec!["art/changed.json".to_string()],
                removed: vec!["art/gone.json".to_string()],
            }
        );
    }
}

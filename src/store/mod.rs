//! The shared data store: news archives and name translations. One server, the publisher,
//! collects them from their sources and writes them to a bucket. Every server keeps a copy in
//! its data directory and generates from the copy it last loaded, so the servers agree without
//! the data passing through git.

#![allow(non_snake_case)]

#[path = "../types/store.rs"]
mod types;

mod sync;

pub use sync::{auditGlobalNews, spawnSyncLoop};
pub use types::{StoreBytes, StoreFile, LOCAL_FOLDER};

use crate::assets::Bucket;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};
use tracing::{info, warn};
use types::{BUCKET_FOLDER, CACHE_CONTROL};

/// What generation reads. A file missing here is read from the bundled copy.
static LOADED: RwLock<BTreeMap<StoreFile, Arc<[u8]>>> = RwLock::new(BTreeMap::new());

impl StoreFile {
    pub const ALL: [Self; 4] = [
        Self::UmapyoiArchive,
        Self::GlobalNewsArchive,
        Self::GlobalSocialArchive,
        Self::NameTranslations,
    ];

    /// The file's name in the bucket and in a server's copy.
    pub fn fileName(self) -> &'static str {
        match self {
            Self::UmapyoiArchive => "umapyoi_archive.json",
            Self::GlobalNewsArchive => "official_news_archive.json",
            Self::GlobalSocialArchive => "official_social_archive.json",
            Self::NameTranslations => "name_translations.json",
        }
    }

    /// The copy built into the binary. It seeds a store that has no such file yet, and stands in
    /// on a server that cannot reach the store.
    fn bundled(self) -> &'static [u8] {
        match self {
            Self::UmapyoiArchive => include_bytes!("../jp_data/umapyoi_archive.json"),
            Self::GlobalNewsArchive => {
                include_bytes!("../global_data/official_news_archive.json")
            }
            Self::GlobalSocialArchive => {
                include_bytes!("../global_data/official_social_archive.json")
            }
            Self::NameTranslations => include_bytes!("../jp_data/name_translations.json"),
        }
    }
}

impl std::ops::Deref for StoreBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Self::Loaded(bytes) => bytes,
            Self::Bundled(bytes) => bytes,
        }
    }
}

impl AsRef<[u8]> for StoreBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

/// The file's content as generation reads it now.
pub fn bytes(file: StoreFile) -> StoreBytes {
    let loaded = LOADED.read().unwrap_or_else(PoisonError::into_inner);
    match loaded.get(&file) {
        Some(bytes) => StoreBytes::Loaded(bytes.clone()),
        None => StoreBytes::Bundled(file.bundled()),
    }
}

/// The bucket the store lives in, when this server has the asset bucket's credentials: the
/// `store/` folder of the bucket STORE_S3_BUCKET names, or of the asset bucket. It is reached with
/// the asset key unless STORE_S3_ACCESS_KEY_ID and STORE_S3_SECRET_ACCESS_KEY name a key of its
/// own; the endpoint is always the asset bucket's.
pub fn bucketFromEnv() -> Option<Bucket> {
    let assets = Bucket::fromEnv().ok()?;
    let own_bucket = std::env::var("STORE_S3_BUCKET").unwrap_or_default();
    let bucket = match own_bucket.trim() {
        "" => assets.inFolder(BUCKET_FOLDER),
        name => assets.named(name).inFolder(BUCKET_FOLDER),
    };

    let access_key = std::env::var("STORE_S3_ACCESS_KEY_ID").unwrap_or_default();
    let secret_key = std::env::var("STORE_S3_SECRET_ACCESS_KEY").unwrap_or_default();
    Some(match (access_key.trim(), secret_key.trim()) {
        ("", "") => bucket,
        ("", _) | (_, "") => {
            warn!("the store's own key needs both STORE_S3_ACCESS_KEY_ID and STORE_S3_SECRET_ACCESS_KEY; using the asset key");
            bucket
        }
        (access_key, secret_key) => bucket.withKey(access_key, secret_key),
    })
}

/// Readies this server's copy before its first generation. A reader takes what the bucket holds.
/// The publisher writes the bucket, so it keeps its own files and only fills in those it lacks.
/// A file that cannot be readied is read from the bundled copy until a later attempt succeeds.
pub async fn open(dir: &Path, bucket: Option<&Bucket>, publishes: bool) {
    let client = reqwest::Client::new();
    for file in StoreFile::ALL {
        let readied = match (publishes, bucket) {
            (true, _) => ensureLocal(bucket, &client, dir, file).await,
            (false, Some(bucket)) => pullFile(bucket, &client, dir, file).await.map(|_| ()),
            (false, None) => Ok(()),
        };
        if let Err(error) = readied.and_then(|()| install(dir, file).map(|_| ())) {
            warn!(
                ?file,
                error = format!("{error:#}"),
                "store file could not be readied"
            );
        }
    }
}

/// Fetches every file the bucket holds a different version of than this server. Returns whether
/// what generation reads changed.
pub async fn pull(bucket: &Bucket, dir: &Path) -> Result<bool> {
    let client = reqwest::Client::new();
    let mut changed = false;
    for file in StoreFile::ALL {
        changed |= pullFile(bucket, &client, dir, file).await?;
    }
    Ok(changed)
}

async fn pullFile(
    bucket: &Bucket,
    client: &reqwest::Client,
    dir: &Path,
    file: StoreFile,
) -> Result<bool> {
    let key = file.fileName();
    let Some(remote) = bucket.objectEtag(client, key).await? else {
        return Ok(false);
    };
    let path = localPath(dir, file);
    if readLocal(&path)?.is_some_and(|bytes| etag(&bytes) == remote) {
        return Ok(false);
    }
    let Some(bytes) = bucket.getObject(client, key).await? else {
        return Ok(false);
    };
    writeLocal(&path, &bytes)?;
    let changed = install(dir, file)?;
    info!(?file, "store file fetched");
    Ok(changed)
}

/// Uploads this server's copy of the file when the bucket's differs.
async fn push(
    bucket: &Bucket,
    client: &reqwest::Client,
    dir: &Path,
    file: StoreFile,
) -> Result<bool> {
    let Some(bytes) = readLocal(&localPath(dir, file))? else {
        return Ok(false);
    };
    let key = file.fileName();
    if bucket.objectEtag(client, key).await?.as_deref() == Some(etag(&bytes).as_str()) {
        return Ok(false);
    }
    bucket
        .putObjectWithCache(client, key, bytes, "application/json", CACHE_CONTROL)
        .await?;
    Ok(true)
}

/// Gives the publisher a copy of the file to continue from when it has none: the bucket's, or
/// the bundled one when the bucket holds none. A bucket that cannot be asked is an error, so a
/// sync never restarts from the bundled copy and overwrites newer data with it.
async fn ensureLocal(
    bucket: Option<&Bucket>,
    client: &reqwest::Client,
    dir: &Path,
    file: StoreFile,
) -> Result<()> {
    let path = localPath(dir, file);
    if path.exists() {
        return Ok(());
    }
    let stored = match bucket {
        Some(bucket) => bucket.getObject(client, file.fileName()).await?,
        None => None,
    };
    match stored {
        Some(bytes) => writeLocal(&path, &bytes),
        None => writeLocal(&path, file.bundled()),
    }
}

/// Makes this server's copy of the file what generation reads, when it has one that differs.
/// Waits for a running generation, so no export mixes two versions of the store.
fn install(dir: &Path, file: StoreFile) -> Result<bool> {
    let Some(local) = readLocal(&localPath(dir, file))? else {
        return Ok(false);
    };
    if *local == *bytes(file) {
        return Ok(false);
    }
    let _generation = crate::pipeline::lockGeneration()?;
    LOADED
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(file, Arc::from(local));
    Ok(true)
}

fn localPath(dir: &Path, file: StoreFile) -> PathBuf {
    dir.join(file.fileName())
}

fn readLocal(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn writeLocal(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    crate::pipeline::writeAtomic(path, bytes)
        .with_context(|| format!("failed to write {}", path.display()))
}

/// The ETag a bucket reports for these bytes: their MD5, as each file is written in one PUT.
fn etag(bytes: &[u8]) -> String {
    format!("{:x}", md5::compute(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everyFileHasAParseableBundledCopy() {
        for file in StoreFile::ALL {
            serde_json::from_slice::<serde_json::Value>(&bytes(file))
                .unwrap_or_else(|error| panic!("{file:?}: {error}"));
        }
    }

    #[tokio::test]
    async fn thePublisherSeedsAFileItLacksAndKeepsOneItHas() {
        let dir = std::env::temp_dir().join(format!("umamoe-store-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let client = reqwest::Client::new();
        let file = StoreFile::NameTranslations;

        ensureLocal(None, &client, &dir, file).await.unwrap();
        assert_eq!(
            readLocal(&localPath(&dir, file)).unwrap().unwrap(),
            file.bundled()
        );

        writeLocal(&localPath(&dir, file), b"{}").unwrap();
        ensureLocal(None, &client, &dir, file).await.unwrap();
        assert_eq!(readLocal(&localPath(&dir, file)).unwrap().unwrap(), b"{}");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}

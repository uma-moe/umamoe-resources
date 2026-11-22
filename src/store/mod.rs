//! The shared data store: the data every server generates from beside the masters. One server,
//! the publisher, collects the news archives and name translations from their sources and writes
//! them to a bucket; maintainers upload the static data files there. Every server keeps a copy in
//! its data directory and generates from the copy it last loaded, so the servers agree without
//! the data passing through git.

#![allow(non_snake_case)]

#[path = "../types/store.rs"]
mod types;

mod commands;
mod sync;

pub use commands::{pullCommand, putCommand};
pub use sync::{auditGlobalNews, spawnSyncLoop};
pub use types::{StoreBytes, StoreFile, LOCAL_FOLDER};

use crate::assets::Bucket;
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};
use tracing::{info, warn};
use types::{BUCKET_FOLDER, CACHE_CONTROL};

/// What generation reads: the files this server has loaded.
static LOADED: RwLock<BTreeMap<StoreFile, Arc<[u8]>>> = RwLock::new(BTreeMap::new());

impl StoreFile {
    pub const ALL: [Self; 23] = [
        Self::UmapyoiArchive,
        Self::GlobalNewsArchive,
        Self::GlobalSocialArchive,
        Self::NameTranslations,
        Self::CharacterNames,
        Self::SupportCardsDb,
        Self::TimelineCharacterBanners,
        Self::TimelineSupportBanners,
        Self::TimelinePaidBanners,
        Self::TimelineStoryEvents,
        Self::TimelineChampionsMeeting,
        Self::TimelineLegendRaces,
        Self::TimelineCampaigns,
        Self::PlannerGachaRates,
        Self::PlannerMasterRewards,
        Self::PlannerMissionRewards,
        Self::EnglishMissionCampaignAssets,
        Self::SimulatorCourseGeometry,
        Self::SimulatorCourseLanes,
        Self::CourseEventParamsJp,
        Self::CourseEventParamsGlobal,
        Self::RaceParamsGlobal,
        Self::NameOverrides,
    ];

    /// The file's name in the bucket and in a server's copy.
    pub fn fileName(self) -> &'static str {
        match self {
            Self::UmapyoiArchive => "umapyoi_archive.json",
            Self::GlobalNewsArchive => "official_news_archive.json",
            Self::GlobalSocialArchive => "official_social_archive.json",
            Self::NameTranslations => "name_translations.json",
            Self::CharacterNames => "character_names.json",
            Self::SupportCardsDb => "support-cards-db.json",
            Self::TimelineCharacterBanners => "timeline_character_banners.json",
            Self::TimelineSupportBanners => "timeline_support_banners.json",
            Self::TimelinePaidBanners => "timeline_paid_banners.json",
            Self::TimelineStoryEvents => "timeline_story_events.json",
            Self::TimelineChampionsMeeting => "timeline_champions_meeting.json",
            Self::TimelineLegendRaces => "timeline_legend_races.json",
            Self::TimelineCampaigns => "timeline_campaigns.json",
            Self::PlannerGachaRates => "planner_gacha_rates.json",
            Self::PlannerMasterRewards => "planner_master_rewards.json",
            Self::PlannerMissionRewards => "planner_mission_rewards.json",
            Self::EnglishMissionCampaignAssets => "english_mission_campaign_assets.json",
            Self::SimulatorCourseGeometry => "simulator_course_geometry.json.gz",
            Self::SimulatorCourseLanes => "simulator_course_lanes.json.gz",
            Self::CourseEventParamsJp => "course_event_params_jp.json",
            Self::CourseEventParamsGlobal => "course_event_params_global.json",
            Self::RaceParamsGlobal => "race_params_global.json",
            Self::NameOverrides => "name_overrides.json",
        }
    }

    /// The file a store file name names.
    pub fn fromFileName(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|file| file.fileName() == name)
    }

    /// Whether a maintainer provides the file, so the bucket's copy is the source of truth, rather
    /// than the publisher collecting it.
    pub fn isProvided(self) -> bool {
        !matches!(
            self,
            Self::UmapyoiArchive
                | Self::GlobalNewsArchive
                | Self::GlobalSocialArchive
                | Self::NameTranslations
        )
    }

    /// Whether nothing can be generated without the file. Only tests read the race parameters,
    /// the name translations exist only once something was translated (a publisher without the
    /// Anthropic key never writes them), and without name overrides nothing is overridden.
    fn isRequired(self) -> bool {
        !matches!(
            self,
            Self::RaceParamsGlobal | Self::NameTranslations | Self::NameOverrides
        )
    }

    /// Gzipped files are stored as they are.
    fn contentType(self) -> &'static str {
        match self.fileName().ends_with(".gz") {
            true => "application/gzip",
            false => "application/json",
        }
    }
}

/// The file's content as generation reads it now; empty when this server never loaded it.
/// Whatever generates checks `requireLoaded` first.
pub fn bytes(file: StoreFile) -> StoreBytes {
    if let Some(bytes) = LOADED
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&file)
    {
        return bytes.clone();
    }
    #[cfg(test)]
    if let Some(bytes) = loadTestCopy(file) {
        return bytes;
    }
    StoreBytes::from([])
}

/// Fails, naming them, while any file generation reads is not loaded.
pub fn requireLoaded() -> Result<()> {
    let missing = StoreFile::ALL
        .into_iter()
        .filter(|file| file.isRequired() && bytes(*file).is_empty())
        .map(StoreFile::fileName)
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    bail!(
        "the store copy lacks {}; fetch the store with `umamoe-resources store pull`, which needs the store bucket's credentials",
        missing.join(", ")
    )
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

/// Readies this server's copy before its first generation. A reader takes what the bucket holds,
/// and so does the publisher for provided files. For collected files, and for every file when it
/// has no bucket, the publisher keeps its own files and only fills in those it lacks. A file that
/// cannot be readied is read from this server's copy, when it has one, until a later attempt
/// succeeds.
pub async fn open(dir: &Path, bucket: Option<&Bucket>, publishes: bool) {
    let client = reqwest::Client::new();
    for file in StoreFile::ALL {
        let readied = match (file.isProvided(), publishes, bucket) {
            (true, _, Some(bucket)) => pullProvided(bucket, &client, dir, file).await.map(|_| ()),
            (_, true, _) => ensureLocal(bucket, &client, dir, file).await,
            (false, false, Some(bucket)) => pullFile(bucket, &client, dir, file).await.map(|_| ()),
            (_, _, None) => Ok(()),
        };
        if let Err(error) = readied.and_then(|()| install(dir, file).map(|_| ())) {
            warn!(
                ?file,
                error = format!("{error:#}"),
                "store file could not be readied"
            );
            // Generation keeps reading this server's copy while the bucket cannot give one.
            let _ = install(dir, file);
        }
    }
}

/// Fetches every file the bucket holds a different version of than this server: every file for
/// a reader, and for the publisher the provided files, as its own copy of a collected file is the
/// source of truth. Returns whether what generation reads changed.
pub async fn pull(bucket: &Bucket, dir: &Path, publishes: bool) -> Result<bool> {
    let client = reqwest::Client::new();
    let mut changed = false;
    for file in StoreFile::ALL {
        if publishes && !file.isProvided() {
            continue;
        }
        changed |= pullFile(bucket, &client, dir, file).await?;
    }
    Ok(changed)
}

/// Fetches the bucket's copy of the file when it differs from this server's, and makes it what
/// generation reads. Returns whether what generation reads changed.
async fn pullFile(
    bucket: &Bucket,
    client: &reqwest::Client,
    dir: &Path,
    file: StoreFile,
) -> Result<bool> {
    if !fetchFile(bucket, client, dir, file).await? {
        return Ok(false);
    }
    let changed = install(dir, file)?;
    info!(?file, "store file fetched");
    Ok(changed)
}

/// Writes the bucket's copy of the file over this server's when they differ. Returns whether it
/// did.
async fn fetchFile(
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
    Ok(true)
}

/// Takes the bucket's copy of a provided file. A bucket that holds none of a required file is an
/// error: only a maintainer provides the file.
async fn pullProvided(
    bucket: &Bucket,
    client: &reqwest::Client,
    dir: &Path,
    file: StoreFile,
) -> Result<bool> {
    if bucket.objectEtag(client, file.fileName()).await?.is_none() {
        if !file.isRequired() {
            return Ok(false);
        }
        bail!(
            "the store's bucket has no {name}; a maintainer uploads it with `store put {name} <path>`",
            name = file.fileName()
        );
    }
    pullFile(bucket, client, dir, file).await
}

async fn upload(
    bucket: &Bucket,
    client: &reqwest::Client,
    file: StoreFile,
    bytes: Vec<u8>,
) -> Result<()> {
    bucket
        .putObjectWithCache(
            client,
            file.fileName(),
            bytes,
            file.contentType(),
            CACHE_CONTROL,
        )
        .await
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
    upload(bucket, client, file, bytes).await?;
    Ok(true)
}

/// Gives the publisher the bucket's copy of the file to continue from when it has none. A
/// collected file the bucket holds none of starts from nothing, and its sync builds it; a
/// required provided one is an error, as only a maintainer provides it. A bucket that cannot be
/// asked is an error, so a sync never restarts from nothing and overwrites newer data.
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
        None if file.isProvided() && file.isRequired() => bail!(
            "neither this server nor the store's bucket has {name}; a maintainer uploads it with `store put {name} <path>`",
            name = file.fileName()
        ),
        None => Ok(()),
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

/// Loads a file of the repository's local store copy for tests, which have no server to ready
/// one. It takes no generation lock, as tests read the store while a generation holds it.
#[cfg(test)]
fn loadTestCopy(file: StoreFile) -> Option<StoreBytes> {
    let bytes = StoreBytes::from(std::fs::read(localPath(&testCopyDir(), file)).ok()?);
    let mut loaded = LOADED.write().unwrap_or_else(PoisonError::into_inner);
    Some(loaded.entry(file).or_insert(bytes).clone())
}

#[cfg(test)]
fn testCopyDir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(types::TEST_COPY_DIR)
}

/// The files the repository's local store copy lacks for the tests that read it. The name
/// overrides may be absent, as anywhere: none means no overrides.
#[cfg(test)]
pub fn missingTestFiles() -> Vec<&'static str> {
    let dir = testCopyDir();
    StoreFile::ALL
        .into_iter()
        .filter(|file| *file != StoreFile::NameOverrides && !localPath(&dir, *file).is_file())
        .map(StoreFile::fileName)
        .collect()
}

/// Returns from a test that reads store data when the repository's local store copy is
/// incomplete, naming the files it lacks.
#[cfg(test)]
macro_rules! requireStoreData {
    () => {
        let missing = crate::store::missingTestFiles();
        if !missing.is_empty() {
            println!(
                "skipped: the local store copy lacks {} (run `umamoe-resources store pull`)",
                missing.join(", ")
            );
            return;
        }
    };
}
#[cfg(test)]
pub(crate) use requireStoreData;

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
    fn everyFileOfTheLocalCopyParses() {
        requireStoreData!();
        for file in StoreFile::ALL {
            let bytes = bytes(file);
            if bytes.is_empty() && !file.isRequired() {
                continue;
            }
            commands::checkParses(file, &bytes)
                .unwrap_or_else(|error| panic!("{file:?}: {error:#}"));
        }
    }

    #[tokio::test]
    async fn thePublisherStartsACollectedFileFromNothingAndKeepsOneItHas() {
        let dir = std::env::temp_dir().join(format!("umamoe-store-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let client = reqwest::Client::new();
        let file = StoreFile::NameTranslations;

        ensureLocal(None, &client, &dir, file).await.unwrap();
        assert!(readLocal(&localPath(&dir, file)).unwrap().is_none());

        writeLocal(&localPath(&dir, file), b"{}").unwrap();
        ensureLocal(None, &client, &dir, file).await.unwrap();
        assert_eq!(readLocal(&localPath(&dir, file)).unwrap().unwrap(), b"{}");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn aProvidedFileNobodyHasIsAnError() {
        let dir =
            std::env::temp_dir().join(format!("umamoe-store-test-provided-{}", std::process::id()));
        let client = reqwest::Client::new();

        let error = ensureLocal(None, &client, &dir, StoreFile::TimelineCampaigns)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("store put timeline_campaigns.json"));
        assert!(!dir.join("timeline_campaigns.json").exists());

        // Nobody having the optional name overrides means there are none.
        ensureLocal(None, &client, &dir, StoreFile::NameOverrides)
            .await
            .unwrap();
        assert!(!dir.join("name_overrides.json").exists());
    }
}

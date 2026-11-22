//! Publishes the picture of every timeline banner below `art/timeline/`, one file per banner:
//! the official English art once it exists, else the Japanese art, else the site's own file.
//! `art/timeline/index.json` links each published banner by the timeline's `image_path`.

mod convert;
mod fetch;
mod layout;
mod records;
mod site;

use super::types::{Bucket, ObjectChange, DOCUMENT_CACHE_CONTROL};
use super::{originsInFolder, publishFile, purgeOverwritten};
use crate::generators::timeline_images::{
    compare_with_frontend_sources, plan_banner_images, BannerImagePlan, EnglishSource,
    FrontendSources, ParityReport, PlannedBanner,
};
use crate::pipeline;
use anyhow::{Context, Result};
use convert::{convertPictures, ConvertJob, PictureFit};
use fetch::isPastRetryWindow;
pub(super) use fetch::SourceFetcher;
use flate2::read::GzDecoder;
use layout::{bannerKeys, rootLink, INDEX_KEY};
use records::{
    attempts, Attempt, BannerRecord, BannerState, TimelineArtRecords, Variant, FIT_REVISION,
};
use serde::Serialize;
pub(super) use site::isWebp;
use site::SiteManifests;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const TIMELINE_FILE: &str = "banner_timeline.json";

/// Pictures converted and published together; records are saved after each batch.
const BATCH_SIZE: usize = 32;

const INDEX_SCHEMA: u32 = 1;

const DEFAULT_SITE_ORIGIN: &str = "https://uma.moe";

pub struct TimelineArtOptions {
    pub purge_origins: Vec<String>,
    /// The site whose asset folder holds the banners synced so far.
    pub site_origin: String,
}

impl TimelineArtOptions {
    pub fn fromEnv() -> Self {
        let purge_origins = std::env::var("ASSETS_PURGE_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(str::to_string)
            .collect();
        let site_origin = std::env::var("TIMELINE_ART_SITE_ORIGIN")
            .ok()
            .map(|origin| origin.trim().trim_end_matches('/').to_string())
            .filter(|origin| !origin.is_empty())
            .unwrap_or_else(|| DEFAULT_SITE_ORIGIN.to_string());
        Self {
            purge_origins,
            site_origin,
        }
    }

    /// The options for a tree published below `folder`, where the site serves its files.
    pub fn inFolder(self, folder: &str) -> Self {
        Self {
            purge_origins: originsInFolder(&self.purge_origins, folder),
            ..self
        }
    }
}

#[derive(Debug, Default)]
pub struct TimelineArtSummary {
    pub banners: usize,
    pub current: usize,
    pub published: usize,
    /// English art announced for a release that is not uploaded yet.
    pub english_missing: usize,
    pub english_retired: usize,
    /// Downloads or conversions that failed; retried on the next run.
    pub failures: usize,
    /// Banners with no picture anywhere yet.
    pub unavailable: usize,
    pub unplaced: usize,
}

/// The current timeline in the generated data folder.
pub fn currentTimelinePath(data_dir: &Path) -> Result<PathBuf> {
    let manifest = pipeline::read_manifest(data_dir)?;
    let artifact = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.name == TIMELINE_FILE)
        .with_context(|| format!("the current manifest lists no {TIMELINE_FILE}"))?;
    Ok(crate::resource_storage::artifactPath(
        data_dir,
        data_dir,
        &manifest.version,
        artifact,
    ))
}

/// The timeline's JSON, from a `.json` or `.json.gz` file.
pub fn readTimeline(path: &Path) -> Result<Vec<u8>> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if !path.extension().is_some_and(|extension| extension == "gz") {
        return Ok(bytes);
    }

    let mut json = Vec::new();
    GzDecoder::new(bytes.as_slice())
        .read_to_end(&mut json)
        .with_context(|| format!("{} is not gzip", path.display()))?;
    Ok(json)
}

/// Logs how many banners have an English, a Japanese or no official source.
pub fn reportTimelineArtPlan(timeline_json: &[u8]) -> Result<()> {
    let plan = plan_banner_images(timeline_json)?;
    let (_, unplaced) = bannerKeys(plan.banners.keys().map(String::as_str));
    let count = |has: fn(&PlannedBanner) -> bool| {
        plan.banners.values().filter(|banner| has(banner)).count()
    };

    info!(
        banners = plan.banners.len(),
        english = count(|banner| banner.english.is_some()),
        japanese = count(|banner| banner.japanese.is_some()),
        without_source = count(|banner| banner.english.is_none() && banner.japanese.is_none()),
        unplaced = unplaced.len(),
        "timeline art plan"
    );
    for image_path in unplaced {
        warn!(image_path, "timeline banner has no key below art/timeline/");
    }
    Ok(())
}

/// Compares the plan with the sources recorded in the frontend's `timeline-image-sync/`.
pub fn compareTimelineArtWithFrontend(
    timeline_json: &[u8],
    sync_dir: &Path,
) -> Result<ParityReport> {
    let plan = plan_banner_images(timeline_json)?;
    let japanese = readFrontendSources(&sync_dir.join("japanese-image-sources.json"))?;
    let english = readFrontendSources(&sync_dir.join("english-image-sources.json"))?;
    Ok(compare_with_frontend_sources(&plan, &japanese, &english))
}

fn readFrontendSources(path: &Path) -> Result<FrontendSources> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("{} is not valid JSON", path.display()))
}

/// Publishes the banners whose best picture is new or changed, then the index.
pub async fn publishTimelineArt(
    bucket: &Bucket,
    options: &TimelineArtOptions,
    timeline_json: &[u8],
) -> Result<TimelineArtSummary> {
    let plan = plan_banner_images(timeline_json)?;
    let client = reqwest::Client::new();
    let mut records = records::loadRecords(&client, bucket).await?;
    let destination = Destination::Bucket {
        bucket,
        client: &client,
    };

    let summary = runPass(&destination, options, &plan, &mut records, None).await?;
    writeIndex(&destination, options, &records).await?;
    info!(
        banners = summary.banners,
        current = summary.current,
        published = summary.published,
        english_missing = summary.english_missing,
        english_retired = summary.english_retired,
        failures = summary.failures,
        unavailable = summary.unavailable,
        unplaced = summary.unplaced,
        "published timeline art"
    );
    Ok(summary)
}

/// Builds a few banners of every folder into `out_dir`, with their index, to look at.
pub async fn buildTimelineArtSample(
    options: &TimelineArtOptions,
    timeline_json: &[u8],
    out_dir: &Path,
    per_folder: usize,
) -> Result<()> {
    let plan = plan_banner_images(timeline_json)?;
    let mut records = TimelineArtRecords::default();
    let destination = Destination::Folder(out_dir);

    let summary = runPass(&destination, options, &plan, &mut records, Some(per_folder)).await?;
    writeIndex(&destination, options, &records).await?;
    info!(
        out_dir = %out_dir.display(),
        published = summary.published,
        failures = summary.failures,
        unavailable = summary.unavailable,
        "built timeline art sample"
    );
    Ok(())
}

/// Where a run writes: the bucket, or a local folder to look at.
enum Destination<'a> {
    Bucket {
        bucket: &'a Bucket,
        client: &'a reqwest::Client,
    },
    Folder(&'a Path),
}

impl Destination<'_> {
    async fn write(&self, key: &str, file: &Path) -> Result<ObjectChange> {
        match self {
            Destination::Bucket { bucket, client } => {
                let published = publishFile(
                    (*client).clone(),
                    (*bucket).clone(),
                    file.to_path_buf(),
                    key.to_string(),
                    false,
                )
                .await?;
                Ok(published.change)
            }
            Destination::Folder(folder) => {
                let target = folder.join(key);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(file, &target)
                    .with_context(|| format!("failed to write {}", target.display()))?;
                Ok(ObjectChange::Created)
            }
        }
    }

    async fn saveRecords(&self, records: &TimelineArtRecords) -> Result<()> {
        match self {
            Destination::Bucket { bucket, client } => {
                records::saveRecords(client, bucket, records).await
            }
            Destination::Folder(_) => Ok(()),
        }
    }
}

/// A picture downloaded for a banner, waiting to be converted and published.
struct PendingPicture {
    image_path: String,
    key: String,
    variant: Variant,
    planned: String,
    source: String,
    downloaded: PathBuf,
    /// `None` publishes the downloaded file as it is.
    fit: Option<PictureFit>,
}

struct Pass<'a> {
    destination: &'a Destination<'a>,
    options: &'a TimelineArtOptions,
    fetcher: SourceFetcher,
    site: SiteManifests,
    work: WorkDir,
    pending: Vec<PendingPicture>,
    overwritten: Vec<String>,
    summary: TimelineArtSummary,
}

async fn runPass(
    destination: &Destination<'_>,
    options: &TimelineArtOptions,
    plan: &BannerImagePlan,
    records: &mut TimelineArtRecords,
    per_folder: Option<usize>,
) -> Result<TimelineArtSummary> {
    let (keys, unplaced) = bannerKeys(plan.banners.keys().map(String::as_str));
    for image_path in &unplaced {
        warn!(image_path, "timeline banner has no key below art/timeline/");
    }

    let mut fetcher = SourceFetcher::new()?;
    let has_new_banners = keys
        .keys()
        .any(|image_path| !records.banners.contains_key(*image_path));
    let site = if has_new_banners {
        SiteManifests::load(&mut fetcher, &options.site_origin).await?
    } else {
        SiteManifests::default()
    };

    let mut pass = Pass {
        destination,
        options,
        fetcher,
        site,
        work: WorkDir::create()?,
        pending: Vec::new(),
        overwritten: Vec::new(),
        summary: TimelineArtSummary {
            banners: plan.banners.len(),
            unplaced: unplaced.len(),
            ..TimelineArtSummary::default()
        },
    };
    let mut built_per_folder: BTreeMap<String, usize> = BTreeMap::new();

    for (image_path, key) in keys {
        if let Some(limit) = per_folder {
            let folder = key
                .rsplit_once('/')
                .map_or("", |(folder, _)| folder)
                .to_string();
            let built = built_per_folder.entry(folder).or_default();
            if *built >= limit {
                continue;
            }
            *built += 1;
        }

        let banner = &plan.banners[image_path];
        let state = BannerState {
            image_path,
            planned: banner,
            record: records.banners.get(image_path),
            site: pass.site.overlays(image_path),
        };
        let tried = attempts(&state, &records.retired);
        if tried.is_empty() {
            pass.summary.current += 1;
            continue;
        }

        let picture = pass.obtain(image_path, &key, banner, tried, records).await;
        match picture {
            Some(picture) => pass.pending.push(picture),
            None if records.banners.contains_key(image_path) => {}
            None => pass.summary.unavailable += 1,
        }
        if pass.pending.len() >= BATCH_SIZE {
            pass.publishPending(records).await?;
        }
    }

    pass.publishPending(records).await?;
    Ok(pass.summary)
}

impl Pass<'_> {
    /// The first attempt that yields a picture. A missing English header is retired once its
    /// release window has passed.
    async fn obtain(
        &mut self,
        image_path: &str,
        key: &str,
        banner: &PlannedBanner,
        tried: Vec<Attempt>,
        records: &mut TimelineArtRecords,
    ) -> Option<PendingPicture> {
        for attempt in tried {
            let outcome = match &attempt {
                Attempt::English(english) => {
                    self.englishPicture(image_path, key, banner, english, records)
                        .await
                }
                Attempt::Japanese(url) => self.japanesePicture(image_path, key, url).await,
                Attempt::SiteCopy {
                    path,
                    variant,
                    planned,
                } => {
                    self.siteCopy(image_path, key, path, *variant, planned)
                        .await
                }
            };

            match outcome {
                Ok(Some(picture)) => return Some(picture),
                Ok(None) => {}
                Err(error) => {
                    self.summary.failures += 1;
                    warn!(
                        image_path,
                        error = format!("{error:#}"),
                        "timeline banner source failed"
                    );
                }
            }
        }
        None
    }

    async fn englishPicture(
        &mut self,
        image_path: &str,
        key: &str,
        banner: &PlannedBanner,
        english: &EnglishSource,
        records: &mut TimelineArtRecords,
    ) -> Result<Option<PendingPicture>> {
        let url = if english.campaign {
            self.fetcher.campaignPicture(&english.url).await?
        } else {
            english.url.clone()
        };

        let Some(bytes) = self.fetcher.download(&url).await? else {
            if isPastRetryWindow(&english.url, chrono::Utc::now().timestamp()) {
                records.retired.insert(english.url.clone());
                self.summary.english_retired += 1;
            } else {
                self.summary.english_missing += 1;
            }
            return Ok(None);
        };

        let downloaded = self.work.save(&bytes)?;
        let fit = self
            .englishFit(image_path, banner, records.banners.get(image_path))
            .await;
        Ok(Some(PendingPicture {
            image_path: image_path.to_string(),
            key: key.to_string(),
            variant: Variant::English,
            planned: english.url.clone(),
            source: url,
            downloaded,
            fit: Some(fit),
        }))
    }

    /// English art takes the size of the banner's Japanese art: the size published before, else
    /// the Japanese source's, else the size of the site's original.
    async fn englishFit(
        &mut self,
        image_path: &str,
        banner: &PlannedBanner,
        record: Option<&BannerRecord>,
    ) -> PictureFit {
        let japanese_size = record
            .filter(|record| record.variant == Variant::Japanese)
            .and_then(|record| record.size);
        if japanese_size.is_some() {
            return PictureFit::English {
                target: japanese_size,
                reference: None,
                reference_is_japanese: false,
            };
        }

        let japanese_reference = match &banner.japanese {
            Some(url) => self.downloadReference(url).await,
            None => None,
        };
        if japanese_reference.is_some() {
            return PictureFit::English {
                target: None,
                reference: japanese_reference,
                reference_is_japanese: true,
            };
        }

        let original = self.siteUrl(image_path);
        PictureFit::English {
            target: None,
            reference: self.downloadReference(&original).await,
            reference_is_japanese: false,
        }
    }

    async fn downloadReference(&mut self, url: &str) -> Option<PathBuf> {
        let bytes = self.fetcher.download(url).await.ok().flatten()?;
        self.work.save(&bytes).ok()
    }

    async fn japanesePicture(
        &mut self,
        image_path: &str,
        key: &str,
        url: &str,
    ) -> Result<Option<PendingPicture>> {
        let Some(bytes) = self.fetcher.download(url).await? else {
            anyhow::bail!("{url} is gone");
        };
        Ok(Some(PendingPicture {
            image_path: image_path.to_string(),
            key: key.to_string(),
            variant: Variant::Japanese,
            planned: url.to_string(),
            source: url.to_string(),
            downloaded: self.work.save(&bytes)?,
            fit: Some(PictureFit::Japanese),
        }))
    }

    async fn siteCopy(
        &mut self,
        image_path: &str,
        key: &str,
        path: &str,
        variant: Variant,
        planned: &str,
    ) -> Result<Option<PendingPicture>> {
        let url = self.siteUrl(path);
        let Some(bytes) = self.fetcher.download(&url).await? else {
            return Ok(None);
        };
        if !isWebp(&bytes) {
            anyhow::bail!("{url} is not a WebP picture");
        }
        Ok(Some(PendingPicture {
            image_path: image_path.to_string(),
            key: key.to_string(),
            variant,
            planned: planned.to_string(),
            source: url,
            downloaded: self.work.save(&bytes)?,
            fit: None,
        }))
    }

    fn siteUrl(&self, path: &str) -> String {
        format!("{}/{path}", self.options.site_origin)
    }

    /// Converts the pending pictures, publishes those that converted, records them and saves
    /// the records. A picture that does not convert keeps the banner's earlier one.
    async fn publishPending(&mut self, records: &mut TimelineArtRecords) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }

        let pending = std::mem::take(&mut self.pending);
        let jobs: Vec<ConvertJob> = pending
            .iter()
            .filter_map(|picture| {
                Some(ConvertJob {
                    source: picture.downloaded.clone(),
                    output: picture.downloaded.with_extension("webp"),
                    fit: picture.fit.clone()?,
                })
            })
            .collect();
        let converted: BTreeMap<PathBuf, convert::ConvertResult> = convertPictures(&jobs)
            .await?
            .into_iter()
            .map(|result| (result.output.clone(), result))
            .collect();

        for picture in pending {
            let (file, size) = match picture.fit {
                None => (picture.downloaded.clone(), None),
                Some(_) => {
                    let output = picture.downloaded.with_extension("webp");
                    match converted.get(&output) {
                        Some(result) if result.error.is_none() => (output, result.size),
                        result => {
                            self.summary.failures += 1;
                            let error = result.and_then(|result| result.error.clone());
                            warn!(
                                image_path = picture.image_path,
                                error, "timeline banner did not convert"
                            );
                            continue;
                        }
                    }
                }
            };

            if self.destination.write(&picture.key, &file).await? == ObjectChange::Overwritten {
                self.overwritten.push(picture.key.clone());
            }
            records.banners.insert(
                picture.image_path,
                BannerRecord {
                    variant: picture.variant,
                    planned: picture.planned,
                    source: picture.source,
                    revision: FIT_REVISION,
                    size,
                },
            );
            self.summary.published += 1;
        }

        self.destination.saveRecords(records).await?;
        purgeOverwritten(
            &std::mem::take(&mut self.overwritten),
            &self.options.purge_origins,
        )
        .await?;
        self.work.clear()?;
        Ok(())
    }
}

#[derive(Serialize)]
struct TimelineIndex {
    schema: u32,
    /// Links by the timeline's `image_path`.
    images: BTreeMap<String, String>,
}

/// `art/timeline/index.json`, written only when its bytes change.
async fn writeIndex(
    destination: &Destination<'_>,
    options: &TimelineArtOptions,
    records: &TimelineArtRecords,
) -> Result<()> {
    let images = records
        .banners
        .keys()
        .filter_map(|image_path| {
            let key = layout::timelineKey(image_path)?;
            Some((image_path.clone(), rootLink(&key)))
        })
        .collect();
    let body = serde_json::to_vec(&TimelineIndex {
        schema: INDEX_SCHEMA,
        images,
    })?;

    match destination {
        Destination::Bucket { bucket, client } => {
            let digest = format!("{:x}", md5::compute(&body));
            let previous = bucket.objectEtag(client, INDEX_KEY).await?;
            if previous.as_deref() == Some(digest.as_str()) {
                return Ok(());
            }
            bucket
                .putObjectWithCache(
                    client,
                    INDEX_KEY,
                    body,
                    "application/json",
                    DOCUMENT_CACHE_CONTROL,
                )
                .await?;
            if previous.is_some() {
                purgeOverwritten(&[INDEX_KEY.to_string()], &options.purge_origins).await?;
            }
            Ok(())
        }
        Destination::Folder(folder) => {
            let target = folder.join(INDEX_KEY);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, body)
                .with_context(|| format!("failed to write {}", target.display()))
        }
    }
}

/// A temporary folder for the pictures of the current batch, removed afterwards.
struct WorkDir {
    root: PathBuf,
    saved: usize,
}

impl WorkDir {
    fn create() -> Result<Self> {
        let root = std::env::temp_dir().join("umamoe-timeline-art");
        // A run that crashed may have left files behind.
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)
            .with_context(|| format!("failed to create {}", root.display()))?;
        Ok(Self { root, saved: 0 })
    }

    fn save(&mut self, bytes: &[u8]) -> Result<PathBuf> {
        self.saved += 1;
        let path = self.root.join(format!("{}.download", self.saved));
        std::fs::write(&path, bytes)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(path)
    }

    fn clear(&self) -> Result<()> {
        std::fs::remove_dir_all(&self.root)?;
        std::fs::create_dir_all(&self.root)?;
        Ok(())
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

//! Builds the site's game images from the game's CDN and publishes them to the asset bucket.

mod build;
mod catalog;
mod chibis;
mod cleanup;
mod clips;
mod directory;
mod directory_store;
mod layout;
mod plan;
mod race_motions;
mod state;
#[path = "../../types/game_art.rs"]
mod types;
mod umas;

use super::types::ObjectChange;
use super::{originsInFolder, publishFile, purgeOverwritten, Bucket};
use crate::game_cdn::{self, CdnSource, ManifestEntry, ANDROID, GLOBAL_CDN, WINDOWS};
use anyhow::{ensure, Context, Result};
use build::{convertBatch, converterProcesses, downloadBatch, BuiltBatch, JobOutcome, WorkDir};
use catalog::catalogCategories;
use cleanup::{
    deleteKeys, isSuspiciousOrphanCount, keptConversion, plannedRun, recordKeys, removeRecords,
    unlistedKeys,
};
use directory::directoryDocuments;
use directory_store::{reportDocumentSizes, DirectoryWriter};
use layout::Layout;
use plan::{orphanedRecords, planJobs, publishedRecord};
use state::{loadState, rewriteState, saveState, shardName};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tracing::{info, warn};
use types::{
    ArtDestination, ArtJob, ArtState, Catalog, StateLayout, JOBS_PER_CONVERTER, SHADER_BUNDLE,
    UPLOAD_CONCURRENCY,
};
pub use types::{GameArtOptions, GameArtSummary};

impl GameArtOptions {
    /// Reads the asset key from `GAME_ASSET_KEY` and purge origins from `ASSETS_PURGE_ORIGINS`.
    pub fn fromEnv() -> Result<Self> {
        let key = std::env::var("GAME_ASSET_KEY").context("set GAME_ASSET_KEY")?;
        let base_key = hex::decode(key.trim()).context("GAME_ASSET_KEY is not hexadecimal")?;
        ensure!(!base_key.is_empty(), "GAME_ASSET_KEY is empty");

        let purge_origins = std::env::var("ASSETS_PURGE_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(str::to_string)
            .collect();

        Ok(Self {
            base_key,
            purge_origins,
        })
    }

    /// The options for a tree published below `folder`, where the site serves its files.
    pub fn inFolder(self, folder: &str) -> Self {
        Self {
            purge_origins: originsInFolder(&self.purge_origins, folder),
            ..self
        }
    }
}

impl GameArtSummary {
    /// Every planned file was built or recorded as unconvertible; none waits for a retry.
    pub fn isComplete(&self) -> bool {
        self.download_failures == 0
    }
}

/// Builds and uploads every catalogued image that is new or changed for this resource version,
/// whose cards are read from `master`, then deletes what it published that no record lists any
/// more. Plans again even for a version already handled, so a catalog change in a deploy takes
/// effect without waiting for a game update.
pub async fn publishGameArt(
    bucket: &Bucket,
    options: &GameArtOptions,
    master: &Path,
    resource_version: &str,
    dry_run: bool,
) -> Result<GameArtSummary> {
    let client = reqwest::Client::new();
    let (mut state, layout) = loadState(&client, bucket).await?;
    if let StateLayout::Outdated { obsolete_keys } = &layout {
        if !dry_run {
            rewriteState(&client, bucket, &state, obsolete_keys).await?;
        }
    }

    let catalog = Catalog::load(master)?;
    let sources = listSources(&client, resource_version).await?;
    let layout = Layout::new(&catalog, &sources);
    reportClashes(&layout);
    let jobs = planJobs(&sources, &catalog, &layout, &state);
    let orphans = removableOrphans(orphanedRecords(&sources, &catalog, &state), &state);
    let mut summary = GameArtSummary::default();
    for (shard, count) in countByShard(&jobs) {
        info!(shard, count, dry_run, "game art to build");
    }
    if dry_run {
        let planned = plannedRun(&state, &jobs, &orphans);
        for key in &planned.deletions {
            info!(key, "game art to delete");
        }
        info!(orphans = orphans.len(), "game art records to remove");
        reportDocumentSizes(&directoryDocuments(&catalog, &layout, &planned.state)?);
        return Ok(summary);
    }

    let destination = ArtDestination::Bucket(bucket);
    let mut directory = DirectoryWriter::forBucket(&client, bucket, &catalog, &layout).await?;
    runJobs(
        &client,
        &jobs,
        &destination,
        options,
        &mut state,
        &mut directory,
        &mut summary,
    )
    .await?;
    removeOrphans(&client, bucket, options, &mut state, &orphans, &mut summary).await?;
    directory
        .write(&client, &destination, &state, &options.purge_origins)
        .await?;

    if summary.isComplete() {
        state.resource_version = Some(resource_version.to_string());
        saveState(&client, bucket, &state, &BTreeSet::new()).await?;
    }
    info!(
        resource_version,
        planned = jobs.len(),
        ?summary,
        "published game art"
    );
    Ok(summary)
}

/// Builds up to `per_family` files of each family into `folder`, without the bucket, so
/// conversions can be checked by eye, with the directory documents of what was built. `only`
/// keeps the files served below any of these key prefixes, such as `art/umas/100101/`.
pub async fn buildGameArtSample(
    options: &GameArtOptions,
    master: &Path,
    resource_version: &str,
    folder: &Path,
    per_family: usize,
    only: &[String],
) -> Result<GameArtSummary> {
    let client = reqwest::Client::new();
    let catalog = Catalog::load(master)?;
    let sources = listSources(&client, resource_version).await?;
    let layout = Layout::new(&catalog, &sources);
    reportClashes(&layout);

    let mut taken: BTreeMap<String, usize> = BTreeMap::new();
    let jobs: Vec<ArtJob> = planJobs(&sources, &catalog, &layout, &ArtState::default())
        .into_iter()
        .filter(|job| {
            only.is_empty()
                || job.outputs.iter().any(|output| {
                    only.iter()
                        .any(|prefix| output.key.starts_with(prefix.as_str()))
                })
        })
        .filter(|job| {
            let count = taken.entry(shardName(&job.name).to_string()).or_default();
            *count += 1;
            *count <= per_family
        })
        .collect();

    let mut summary = GameArtSummary::default();
    let mut state = ArtState::default();
    let destination = ArtDestination::Folder(folder);
    let mut directory = DirectoryWriter::new(&catalog, &layout);
    runJobs(
        &client,
        &jobs,
        &destination,
        options,
        &mut state,
        &mut directory,
        &mut summary,
    )
    .await?;
    directory.write(&client, &destination, &state, &[]).await?;
    info!(folder = %folder.display(), planned = jobs.len(), ?summary, "built game art sample");
    Ok(summary)
}

/// Writes the directory documents into `folder` as if every planned file converted, without
/// downloading anything, to check the documents against the whole file list.
pub async fn previewGameArtDirectory(
    master: &Path,
    resource_version: &str,
    folder: &Path,
) -> Result<()> {
    let client = reqwest::Client::new();
    let catalog = Catalog::load(master)?;
    let sources = listSources(&client, resource_version).await?;
    let layout = Layout::new(&catalog, &sources);
    reportClashes(&layout);

    let jobs = planJobs(&sources, &catalog, &layout, &ArtState::default());
    let planned = plannedRun(&ArtState::default(), &jobs, &[]);
    reportDocumentSizes(&directoryDocuments(&catalog, &layout, &planned.state)?);
    DirectoryWriter::new(&catalog, &layout)
        .write(
            &client,
            &ArtDestination::Folder(folder),
            &planned.state,
            &[],
        )
        .await?;
    info!(folder = %folder.display(), planned = jobs.len(), "previewed the game art directory");
    Ok(())
}

/// Every source's file list, in precedence order: its Windows files and its Android shader
/// bundle, whose programs are GLSL ES. JP joins after Global once it has a version check of its
/// own; Global then wins every file both list.
async fn listSources(
    client: &reqwest::Client,
    resource_version: &str,
) -> Result<Vec<(&'static CdnSource, Vec<ManifestEntry>)>> {
    let categories =
        game_cdn::fetchCategories(client, &GLOBAL_CDN, WINDOWS, resource_version).await?;
    let mut files =
        game_cdn::fetchCategoryFiles(client, &GLOBAL_CDN, &categories, &catalogCategories())
            .await?;

    let android_categories =
        game_cdn::fetchCategories(client, &GLOBAL_CDN, ANDROID, resource_version).await?;
    let shaders =
        game_cdn::fetchCategoryFiles(client, &GLOBAL_CDN, &android_categories, &[SHADER_BUNDLE])
            .await?;
    files.extend(shaders);
    Ok(vec![(&GLOBAL_CDN, files)])
}

/// A shard holds thousands of records and is rewritten whole, so it is saved this often, not after
/// every batch.
const RECORD_SAVE_INTERVAL: Duration = Duration::from_secs(30);

/// Work folder slots a run rotates through: one batch downloads, one converts and one publishes.
const BATCH_SLOTS: usize = 3;

/// Builds the jobs batch by batch as a pipeline: while one batch converts, the next downloads and
/// the previous one is published, recorded and listed in the directory documents.
async fn runJobs(
    client: &reqwest::Client,
    jobs: &[ArtJob],
    destination: &ArtDestination<'_>,
    options: &GameArtOptions,
    state: &mut ArtState,
    directory: &mut DirectoryWriter<'_>,
    summary: &mut GameArtSummary,
) -> Result<()> {
    if jobs.is_empty() {
        return Ok(());
    }

    let work = WorkDir::create()?;
    let base_key = Arc::new(options.base_key.clone());
    let batch_size = converterProcesses() * JOBS_PER_CONVERTER;
    let batches: Vec<&[ArtJob]> = jobs.chunks(batch_size).collect();
    // Files with a record that this run does not rebuild are already published.
    let new_files = jobs
        .iter()
        .filter(|job| !state.files.contains_key(&job.name))
        .count();
    let progress = Progress::new(state.files.len() + new_files, jobs.len());
    info!(
        files = jobs.len(),
        published = progress.done,
        batches = batches.len(),
        converters = converterProcesses(),
        "building game art"
    );
    let mut publisher = BatchPublisher {
        client,
        destination,
        options,
        state,
        directory,
        summary,
        unsaved_shards: BTreeSet::new(),
        last_saved: None,
        progress,
    };

    let mut downloaded = Some(downloadBatch(client, &work, 0, batches[0], &base_key).await?);
    let mut built: Option<BuiltBatch> = None;
    for step in 0..=batches.len() {
        let next = step + 1;
        let download = async {
            match batches.get(next) {
                Some(batch) => downloadBatch(client, &work, next % BATCH_SLOTS, batch, &base_key)
                    .await
                    .map(Some),
                None => Ok(None),
            }
        };
        let convert = async {
            match downloaded.take() {
                Some(batch) => convertBatch(batches[step], batch).await.map(Some),
                None => Ok(None),
            }
        };
        let publish = async {
            match built.take() {
                Some(batch) => publisher.publish(batches[step - 1], batch).await,
                None => Ok(()),
            }
        };

        let (next_downloaded, converted, published) = tokio::join!(download, convert, publish);
        published?;
        downloaded = next_downloaded?;
        built = converted?;
    }

    publisher.saveRecords().await
}

/// Takes a converted batch the rest of the way: uploads its files, records them and rewrites the
/// directory documents when they are due.
struct BatchPublisher<'a, 'd, 'w> {
    client: &'a reqwest::Client,
    destination: &'a ArtDestination<'d>,
    options: &'a GameArtOptions,
    state: &'a mut ArtState,
    directory: &'a mut DirectoryWriter<'w>,
    summary: &'a mut GameArtSummary,
    /// Record shards changed since they were last saved.
    unsaved_shards: BTreeSet<String>,
    last_saved: Option<Instant>,
    progress: Progress,
}

/// How much of all game art is published, for the batch log; it counts the files earlier runs
/// published, so a restarted run carries on from where the last one stopped.
struct Progress {
    total: usize,
    done: usize,
    built_this_run: usize,
    started: Instant,
}

impl Progress {
    fn new(total: usize, to_build: usize) -> Self {
        Self {
            total,
            done: total.saturating_sub(to_build),
            built_this_run: 0,
            started: Instant::now(),
        }
    }

    fn advance(&mut self, files: usize) {
        self.done += files;
        self.built_this_run += files;
    }

    fn percent(&self) -> String {
        format!(
            "{:.1}%",
            self.done as f64 * 100.0 / self.total.max(1) as f64
        )
    }

    /// Time left at the pace of this run so far, to the nearest minute.
    fn minutesLeft(&self) -> u64 {
        if self.built_this_run == 0 {
            return 0;
        }
        let remaining = self.total.saturating_sub(self.done) as f64;
        let seconds_per_file = self.started.elapsed().as_secs_f64() / self.built_this_run as f64;
        (remaining * seconds_per_file / 60.0).round() as u64
    }
}

impl BatchPublisher<'_, '_, '_> {
    async fn publish(&mut self, batch: &[ArtJob], built: BuiltBatch) -> Result<()> {
        let mut files = BTreeMap::new();
        let mut stale_keys = BTreeSet::new();

        for (job, outcome) in batch.iter().zip(built.outcomes) {
            let (error, converted) = match outcome {
                JobOutcome::NotDownloaded(error) => {
                    self.summary.download_failures += 1;
                    warn!(file = job.name, error, "game art download failed");
                    continue;
                }
                JobOutcome::Unconvertible(error) => {
                    self.summary.conversion_failures += 1;
                    warn!(file = job.name, error, "game art does not convert");
                    (Some(error), None)
                }
                JobOutcome::Built { outputs, converted } => {
                    self.summary.built += 1;
                    files.extend(outputs);
                    (None, Some(converted))
                }
            };
            // A file that stopped converting keeps the files it published before.
            let converted = converted.unwrap_or_else(|| keptConversion(self.state, &job.name));
            let record = publishedRecord(job, error, converted);
            if let Some(replaced) = self.state.files.insert(job.name.clone(), record) {
                stale_keys.extend(recordKeys(&replaced).cloned());
            }
        }

        self.summary.uploaded += files.len();
        let upload_started = Instant::now();
        let mut changed_urls = deliver(self.client, self.destination, files).await?;
        let upload = upload_started.elapsed();

        let records_started = Instant::now();
        if let ArtDestination::Bucket(bucket) = self.destination {
            // Deleted before the state is saved, so a failed run deletes them again next time.
            let deleted = unlistedKeys(self.state, stale_keys);
            deleteKeys(self.client, bucket, &deleted).await?;
            self.summary.deleted += deleted.len();

            self.unsaved_shards
                .extend(batch.iter().map(|job| shardName(&job.name).to_string()));
            if self
                .last_saved
                .is_none_or(|saved| saved.elapsed() >= RECORD_SAVE_INTERVAL)
            {
                self.saveRecords().await?;
            }
            changed_urls.extend(deleted);
            purgeOverwritten(&changed_urls, &self.options.purge_origins).await?;
        }
        let records = records_started.elapsed();

        let directory_started = Instant::now();
        self.directory
            .writeWhenDue(
                self.client,
                self.destination,
                self.state,
                &self.options.purge_origins,
            )
            .await?;
        self.progress.advance(batch.len());
        info!(
            done = self.progress.done,
            total = self.progress.total,
            progress = self.progress.percent(),
            minutes_left = self.progress.minutesLeft(),
            jobs = batch.len(),
            download_ms = built.download.as_millis(),
            convert_ms = built.convert.as_millis(),
            upload_ms = upload.as_millis(),
            records_ms = records.as_millis(),
            directory_ms = directory_started.elapsed().as_millis(),
            "published a game art batch"
        );
        Ok(())
    }

    /// Saves the record shards changed since the last save. A run that stops before the next save
    /// rebuilds those files, whose unchanged uploads are then skipped.
    async fn saveRecords(&mut self) -> Result<()> {
        let ArtDestination::Bucket(bucket) = self.destination else {
            return Ok(());
        };
        if self.unsaved_shards.is_empty() {
            return Ok(());
        }

        saveState(self.client, bucket, self.state, &self.unsaved_shards).await?;
        self.unsaved_shards.clear();
        self.last_saved = Some(Instant::now());
        Ok(())
    }
}

/// Copies or uploads the files; a file the bucket already holds byte for byte is skipped. Returns
/// the keys that replaced a different object, such as one copied from the old web root, whose
/// cached copies must be purged.
async fn deliver(
    client: &reqwest::Client,
    destination: &ArtDestination<'_>,
    files: BTreeMap<String, PathBuf>,
) -> Result<Vec<String>> {
    let mut replaced = Vec::new();
    match destination {
        ArtDestination::Folder(folder) => {
            for (key, file) in files {
                let target = folder.join(&key);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&file, &target)
                    .with_context(|| format!("failed to copy {key} to {}", target.display()))?;
            }
        }
        ArtDestination::Bucket(bucket) => {
            let mut uploads = JoinSet::new();
            let mut pending = files.into_iter();
            loop {
                while uploads.len() < UPLOAD_CONCURRENCY {
                    let Some((key, file)) = pending.next() else {
                        break;
                    };
                    uploads.spawn(publishFile(
                        client.clone(),
                        (*bucket).clone(),
                        file,
                        key,
                        false,
                    ));
                }
                let Some(upload) = uploads.join_next().await else {
                    break;
                };
                let published = upload??;
                if published.change == ObjectChange::Overwritten {
                    replaced.push(published.key);
                }
            }
        }
    }
    Ok(replaced)
}

/// The orphans to remove, or none when there are so many that the file list is more likely
/// incomplete.
fn removableOrphans(orphans: Vec<String>, state: &ArtState) -> Vec<String> {
    if isSuspiciousOrphanCount(orphans.len(), state.files.len()) {
        warn!(
            orphans = orphans.len(),
            records = state.files.len(),
            "too many game art records look orphaned; none are removed"
        );
        return Vec::new();
    }
    orphans
}

/// Drops the records of files the catalog no longer builds and deletes what only they listed.
async fn removeOrphans(
    client: &reqwest::Client,
    bucket: &Bucket,
    options: &GameArtOptions,
    state: &mut ArtState,
    orphans: &[String],
    summary: &mut GameArtSummary,
) -> Result<()> {
    if orphans.is_empty() {
        return Ok(());
    }

    let stale_keys = removeRecords(state, orphans);
    let deleted = unlistedKeys(state, stale_keys);
    deleteKeys(client, bucket, &deleted).await?;
    summary.deleted += deleted.len();

    let changed = orphans
        .iter()
        .map(|name| shardName(name).to_string())
        .collect();
    saveState(client, bucket, state, &changed).await?;
    purgeOverwritten(&deleted, &options.purge_origins).await?;
    info!(
        records = orphans.len(),
        deleted = deleted.len(),
        "removed game art the catalog no longer builds"
    );
    Ok(())
}

/// Logs files whose natural keys collide; each of them keeps its full game name instead.
fn reportClashes(layout: &Layout) {
    for clash in layout.clashes() {
        warn!(key = clash.key, files = ?clash.names, "game art keys collide");
    }
}

fn countByShard(jobs: &[ArtJob]) -> BTreeMap<&str, usize> {
    let mut counts = BTreeMap::new();
    for job in jobs {
        *counts.entry(shardName(&job.name)).or_default() += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progressCountsEarlierRunsAndEstimatesAtThisRunsPace() {
        let mut progress = Progress::new(1000, 400);
        progress.started = Instant::now() - Duration::from_secs(120);
        assert_eq!(progress.percent(), "60.0%");
        assert_eq!(progress.minutesLeft(), 0);

        progress.advance(100);

        assert_eq!(progress.percent(), "70.0%");
        assert_eq!(progress.minutesLeft(), 6);
    }
}

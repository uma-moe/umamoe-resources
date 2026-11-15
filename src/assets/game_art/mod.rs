//! Builds the site's game images from the game's CDN and publishes them to the asset bucket.

mod build;
mod catalog;
mod plan;
mod race_motions;
mod state;
#[path = "../../types/game_art.rs"]
mod types;
mod umas;

use super::types::ObjectChange;
use super::{publishFile, purgeOverwritten, Bucket};
use crate::game_cdn::{self, CdnSource, ManifestEntry, GLOBAL_CDN};
use anyhow::{ensure, Context, Result};
use build::{buildBatch, JobOutcome, WorkDir};
use catalog::catalogCategories;
use plan::{planJobs, publishedRecord};
use state::{loadState, rewriteState, saveState, shardName};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::task::JoinSet;
use tracing::{info, warn};
use types::{
    ArtDestination, ArtJob, ArtState, Catalog, StateLayout, CONVERT_BATCH_SIZE, UPLOAD_CONCURRENCY,
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
}

impl GameArtSummary {
    /// Every planned file was built or recorded as unconvertible; none waits for a retry.
    pub fn isComplete(&self) -> bool {
        self.download_failures == 0
    }
}

/// Builds and uploads every catalogued image that is new or changed for this resource version,
/// whose cards are read from `master`. Plans again even for a version already handled, so a
/// catalog change in a deploy takes effect without waiting for a game update.
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
    let jobs = planJobs(&sources, &catalog, &state);
    let mut summary = GameArtSummary::default();
    for (folder, count) in countByFolder(&jobs) {
        info!(folder, count, dry_run, "game art to build");
    }
    if dry_run {
        return Ok(summary);
    }

    runJobs(
        &client,
        &jobs,
        ArtDestination::Bucket(bucket),
        options,
        Some(&mut state),
        &mut summary,
    )
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

/// Builds up to `per_family` images of each family into `folder`, without the bucket, so
/// conversions can be checked by eye. `only` keeps the files served below a key prefix, such as
/// `game-assets/umas/special-week/`.
pub async fn buildGameArtSample(
    options: &GameArtOptions,
    master: &Path,
    resource_version: &str,
    folder: &Path,
    per_family: usize,
    only: Option<&str>,
) -> Result<GameArtSummary> {
    let client = reqwest::Client::new();
    let catalog = Catalog::load(master)?;
    let sources = listSources(&client, resource_version).await?;

    let mut taken: BTreeMap<String, usize> = BTreeMap::new();
    let jobs: Vec<ArtJob> = planJobs(&sources, &catalog, &ArtState::default())
        .into_iter()
        .filter(|job| {
            only.is_none_or(|prefix| {
                job.outputs
                    .iter()
                    .any(|output| output.key.starts_with(prefix))
            })
        })
        .filter(|job| {
            let count = taken.entry(outputFolder(job)).or_default();
            *count += 1;
            *count <= per_family
        })
        .collect();

    let mut summary = GameArtSummary::default();
    runJobs(
        &client,
        &jobs,
        ArtDestination::Folder(folder),
        options,
        None,
        &mut summary,
    )
    .await?;
    info!(folder = %folder.display(), planned = jobs.len(), ?summary, "built game art sample");
    Ok(summary)
}

/// Every source's file list, in precedence order. JP joins after Global once it has a version
/// check of its own; Global then wins every file both list.
async fn listSources(
    client: &reqwest::Client,
    resource_version: &str,
) -> Result<Vec<(&'static CdnSource, Vec<ManifestEntry>)>> {
    let categories = game_cdn::fetchCategories(client, &GLOBAL_CDN, resource_version).await?;
    let files =
        game_cdn::fetchCategoryFiles(client, &GLOBAL_CDN, &categories, &catalogCategories())
            .await?;
    Ok(vec![(&GLOBAL_CDN, files)])
}

async fn runJobs(
    client: &reqwest::Client,
    jobs: &[ArtJob],
    destination: ArtDestination<'_>,
    options: &GameArtOptions,
    mut state: Option<&mut ArtState>,
    summary: &mut GameArtSummary,
) -> Result<()> {
    if jobs.is_empty() {
        return Ok(());
    }

    let work = WorkDir::create()?;
    let base_key = Arc::new(options.base_key.clone());

    for batch in jobs.chunks(CONVERT_BATCH_SIZE) {
        let outcomes = buildBatch(client, &work, batch, &base_key).await?;
        let mut files = BTreeMap::new();
        let mut dropped_textures = BTreeSet::new();

        for (job, outcome) in batch.iter().zip(outcomes) {
            let error = match outcome {
                JobOutcome::NotDownloaded(error) => {
                    summary.download_failures += 1;
                    warn!(file = job.name, error, "game art download failed");
                    continue;
                }
                JobOutcome::Unconvertible(error) => {
                    summary.conversion_failures += 1;
                    warn!(file = job.name, error, "game art does not convert");
                    Some(error)
                }
                JobOutcome::Built { outputs } => {
                    summary.built += 1;
                    files.extend(outputs);
                    None
                }
            };
            if let Some(state) = state.as_deref_mut() {
                let record = publishedRecord(job, error);
                if let Some(replaced) = state.files.insert(job.name.clone(), record) {
                    dropped_textures.extend(replaced.textures);
                }
            }
        }

        summary.uploaded += files.len();
        let replaced = deliver(client, &destination, files).await?;
        if let (ArtDestination::Bucket(bucket), Some(state)) = (&destination, state.as_deref()) {
            let changed = batch
                .iter()
                .map(|job| shardName(&job.name).to_string())
                .collect();
            saveState(client, bucket, state, &changed).await?;
            removeUnlistedTextures(client, bucket, state, &dropped_textures).await?;
            purgeOverwritten(&replaced, &options.purge_origins).await?;
        }
    }

    Ok(())
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

/// Deletes textures earlier converters wrote beside a model once no record lists them, unless
/// they are another file's output.
async fn removeUnlistedTextures(
    client: &reqwest::Client,
    bucket: &Bucket,
    state: &ArtState,
    dropped: &BTreeSet<String>,
) -> Result<()> {
    if dropped.is_empty() {
        return Ok(());
    }

    let listed: HashSet<&str> = state
        .files
        .values()
        .flat_map(|record| record.textures.iter().chain(&record.outputs))
        .map(String::as_str)
        .collect();
    for key in dropped.iter().filter(|key| !listed.contains(key.as_str())) {
        bucket.deleteObject(client, key).await?;
    }
    Ok(())
}

fn outputFolder(job: &ArtJob) -> String {
    let key = job.outputs.first().map_or("", |output| output.key.as_str());
    key.rsplit_once('/')
        .map_or(key, |(folder, _)| folder)
        .to_string()
}

fn countByFolder(jobs: &[ArtJob]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for job in jobs {
        *counts.entry(outputFolder(job)).or_default() += 1;
    }
    counts
}

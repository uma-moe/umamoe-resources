//! The publisher's side of the store: collects each collected file from its source, then makes
//! it what this server generates from and uploads it for the others. Provided files it takes
//! from the bucket on every refresh check, like every other server.

use super::types::{StoreFile, TRANSLATION_RETRY, UMAPYOI_REQUEST_INTERVAL_MS};
use super::{ensureLocal, install, localPath, push};
use crate::assets::Bucket;
use crate::generators::planner::GlobalNewsRewardAudit;
use crate::{global_news, global_social, jp_names, master_fetch, umapyoi};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use tracing::{info, warn};

/// When the names were last translated, and from which masters.
#[derive(Default)]
struct TranslationRun {
    at: Option<Instant>,
    masters: Option<[MasterMark; 2]>,
}

/// A master file's modification time and size: enough to tell that a refresh replaced it.
type MasterMark = Option<(SystemTime, u64)>;

/// Synchronizes every collected file at start and then on every interval. A source that fails is
/// tried again on the next round; the other files do not wait for it.
pub fn spawnSyncLoop(dir: PathBuf, bucket: Option<Bucket>, master: PathBuf, interval_seconds: u64) {
    if bucket.is_none() {
        warn!("the store has no bucket; this server's news and translations stay on this server");
    }
    if jp_names::translationKey().is_none() {
        warn!("ANTHROPIC_API_KEY is not set; names only JP has released stay untranslated");
    }
    info!(interval_seconds, "store synchronization enabled");

    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut translation = TranslationRun::default();
        let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            interval.tick().await;
            for file in StoreFile::ALL.into_iter().filter(|file| !file.isProvided()) {
                let synced = syncFile(
                    file,
                    &dir,
                    bucket.as_ref(),
                    &client,
                    &master,
                    &mut translation,
                )
                .await;
                if let Err(error) = synced {
                    warn!(
                        ?file,
                        error = format!("{error:#}"),
                        "store file could not be synchronized"
                    );
                }
            }
        }
    });
}

async fn syncFile(
    file: StoreFile,
    dir: &Path,
    bucket: Option<&Bucket>,
    client: &reqwest::Client,
    master: &Path,
    translation: &mut TranslationRun,
) -> Result<()> {
    ensureLocal(bucket, client, dir, file).await?;
    let path = localPath(dir, file);
    let collected = match file {
        StoreFile::UmapyoiArchive => collectUmapyoi(&path).await,
        StoreFile::GlobalNewsArchive => collectGlobalNews(&path).await,
        StoreFile::GlobalSocialArchive => collectGlobalSocial(&path).await,
        StoreFile::NameTranslations => translation.run(master, &path).await,
        // Provided files are not synchronized here.
        _ => Ok(()),
    };

    // Also after a failed collection: an upload that failed last round is still owed.
    if install(dir, file)? {
        info!(?file, "store file updated");
    }
    if let Some(bucket) = bucket {
        if push(bucket, client, dir, file).await? {
            info!(?file, "store file uploaded");
        }
    }
    collected
}

async fn collectUmapyoi(path: &Path) -> Result<()> {
    let summary = umapyoi::sync(
        umapyoi::BASE_URL,
        path,
        UMAPYOI_REQUEST_INTERVAL_MS,
        false,
        false,
    )
    .await?;
    if summary.changed {
        info!(
            news_posts = summary.news_posts,
            new_news_posts = summary.new_news_posts,
            gacha_banners = summary.gacha_banners,
            support_cards = summary.support_cards,
            "umapyoi JP archive synchronized"
        );
    }
    for error in summary.source_errors {
        warn!(error, "umapyoi source could not be fully synchronized");
    }
    Ok(())
}

/// Synchronizes a copy and takes it only when the planner can read every reward post in it, so
/// a post it cannot read never reaches generation.
async fn collectGlobalNews(path: &Path) -> Result<()> {
    let pending = path.with_file_name("official_news_archive.pending.json");
    // Without a copy, the sync builds the pending file from its source.
    if path.exists() {
        std::fs::copy(path, &pending)
            .with_context(|| format!("failed to copy {}", path.display()))?;
    }
    let checked = async {
        let summary = global_news::sync(global_news::ENDPOINT, &pending).await?;
        auditGlobalNews(&pending)?;
        Ok::<_, anyhow::Error>(summary)
    }
    .await;

    let summary = match checked {
        Ok(summary) => summary,
        Err(error) => {
            let _ = std::fs::remove_file(&pending);
            return Err(error);
        }
    };
    std::fs::rename(&pending, path)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    if summary.changed {
        info!(
            posts = summary.posts,
            new_posts = summary.new_posts,
            updated_posts = summary.updated_posts,
            "official Global news archive synchronized"
        );
    }
    Ok(())
}

/// The planner's reading of the archive at `path`. Fails when it holds a reward post the
/// planner cannot read.
pub fn auditGlobalNews(path: &Path) -> Result<GlobalNewsRewardAudit> {
    let audit = crate::generators::planner::audit_global_news_reward_archive(path)?;
    if audit.issues.is_empty() {
        return Ok(audit);
    }
    let issues = audit
        .issues
        .iter()
        .map(|issue| {
            format!(
                "EN post {} ({}): {}",
                issue.announce_id, issue.page_url, issue.reason
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    bail!("official Global news reward audit failed:\n{issues}")
}

async fn collectGlobalSocial(path: &Path) -> Result<()> {
    let summary = global_social::sync(
        global_social::PROFILE_URL,
        global_social::SYNDICATION_URL,
        global_social::ARCHIVE_INDEX_ENDPOINT,
        global_social::OEMBED_ENDPOINT,
        path,
    )
    .await?;
    if summary.changed {
        info!(
            posts = summary.posts,
            new_posts = summary.new_posts,
            updated_posts = summary.updated_posts,
            "official Global social archive synchronized"
        );
    }
    for error in summary.source_errors {
        warn!(
            error,
            "official Global social source could not be fully synchronized"
        );
    }
    Ok(())
}

impl TranslationRun {
    /// Translates the names only JP has released, when a master changed since the last run or
    /// the retry time has passed. It needs the Anthropic key and the JP master the JP refresh
    /// keeps; without either there is nothing to do.
    async fn run(&mut self, master: &Path, path: &Path) -> Result<()> {
        let jp_master = master_fetch::jpMasterPath(master);
        if jp_names::translationKey().is_none() || !jp_master.is_file() {
            return Ok(());
        }
        let masters = [masterMark(master), masterMark(&jp_master)];
        let retry_due = self.at.is_none_or(|at| at.elapsed() >= TRANSLATION_RETRY);
        if self.masters.as_ref() == Some(&masters) && !retry_due {
            return Ok(());
        }

        self.at = Some(Instant::now());
        self.masters = Some(masters);
        jp_names::translateNames(master, &jp_master, path).await
    }
}

fn masterMark(path: &Path) -> MasterMark {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

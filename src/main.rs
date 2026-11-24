use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{info, warn};

mod assets;
mod auth_common;
mod browser_proof;
mod cache;
mod game_cdn;
mod generators;
mod global_news;
mod global_social;
mod jp_names;
mod master_fetch;
mod master_history;
mod pipeline;
mod resource_storage;
mod static_api;
mod store;
mod umapyoi;

use game_cdn::{CdnSource, GLOBAL_CDN, JP_CDN};

#[derive(Parser)]
#[command(author, version, about = "Serve Umamusume resource JSON streams")]
struct Cli {
    #[command(flatten)]
    server: ServeArgs,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Serve resources over HTTP, generating them from master.mdb first by default.
    Serve(ServeArgs),
    /// Check a region's current game version and download its master if needed.
    FetchMaster {
        /// The Global master; the JP one is kept beside it as `jp-<name>`.
        #[arg(long, default_value = "master.mdb")]
        master: PathBuf,
        #[arg(long, value_enum, default_value_t = Region::Global)]
        region: Region,
    },
    /// Generate versioned .json.gz resources without starting the server.
    Generate {
        /// The Global master; the JP one is kept beside it as `jp-<name>`.
        #[arg(long, default_value = "master.mdb")]
        master: PathBuf,
        #[arg(long, default_value = "generated-data")]
        out: PathBuf,
        #[arg(long)]
        write_json: bool,
        /// The set to generate: Global's into the data directory, JP's into `jp/` inside it.
        #[arg(long, value_enum, default_value_t = Region::Global)]
        region: Region,
    },
    /// List the asset bucket by top folder, or empty it.
    AssetBucket {
        #[command(subcommand)]
        action: AssetBucketAction,
    },
    /// Fetch the shared store's files, or upload a provided one.
    Store {
        #[command(subcommand)]
        action: StoreAction,
    },
    /// Upload a folder of images to the asset bucket, skipping files whose bytes are already there.
    AssetsPublish {
        /// Folder whose files are uploaded, keeping their paths below it.
        #[arg(long)]
        dir: PathBuf,
        /// Key prefix in the bucket, which is also the URL path, such as game-assets.
        #[arg(long)]
        prefix: String,
        /// Compare against the bucket and report what would change, without uploading.
        #[arg(long)]
        dry_run: bool,
        #[arg(long, default_value_t = 16)]
        concurrency: usize,
        /// Site origin whose copies of replaced files are purged, such as https://uma.moe.
        #[arg(
            long = "purge-origin",
            env = "ASSETS_PURGE_ORIGINS",
            value_delimiter = ','
        )]
        purge_origins: Vec<String>,
    },
    /// Build the site's game images from the game CDN and upload the new or changed ones.
    GameArt {
        /// The Global master; the JP one is kept beside it as `jp-<name>`. The region's master
        /// is refreshed first; its cards decide which portraits are built.
        #[arg(long, default_value = "master.mdb")]
        master: PathBuf,
        /// The region to build. Each has a tree of its own in the bucket: Global's in `en/`,
        /// JP's in `jp/`.
        #[arg(long, value_enum, default_value_t = Region::Global)]
        region: Region,
        /// Report what would be built without downloading game files. With --out-dir, write the
        /// directory documents there as if every file converted.
        #[arg(long)]
        dry_run: bool,
        /// Build a few files per family into this folder instead of the bucket, to check them.
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Files per family with --out-dir.
        #[arg(long, default_value_t = 3)]
        sample: usize,
        /// With --out-dir, only files served below this key prefix, such as art/umas/100101/.
        /// Repeat it for several prefixes.
        #[arg(long)]
        only: Vec<String>,
    },
    /// Pick the source of every timeline banner and upload the new or better pictures.
    TimelineArt {
        /// The timeline to picture, `.json` or `.json.gz`; the current one in --data-dir if unset.
        #[arg(long)]
        timeline: Option<PathBuf>,
        #[arg(long, default_value = "generated-data")]
        data_dir: PathBuf,
        /// Report how many banners have an English, a Japanese or no source, and download nothing.
        #[arg(long)]
        dry_run: bool,
        /// Build a few banners per folder into this folder instead of the bucket, to check them.
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Banners per folder with --out-dir.
        #[arg(long, default_value_t = 3)]
        sample: usize,
        /// Compare the picked sources with those the frontend's image sync recorded in this
        /// folder (its timeline-image-sync/).
        #[arg(long)]
        compare_frontend: Option<PathBuf>,
    },
    /// Check the current game version, refresh master.mdb, regenerate, then purge current CDN URLs.
    Refresh {
        #[arg(long, env = "DATABASE_URL")]
        database_url: Option<String>,
        #[arg(long, default_value = "master.mdb")]
        master: PathBuf,
        #[arg(long, default_value = "generated-data")]
        out: PathBuf,
        #[arg(long)]
        write_json: bool,
        #[arg(long)]
        purge: bool,
    },
    /// Rebuild the master release history from every game version on uma.moe/api/ver/history.
    /// Masters are downloaded only to be diffed; the data directory keeps just the diffs.
    BackfillMasterHistory {
        #[arg(long, default_value = "generated-data")]
        data_dir: PathBuf,
    },
    /// Purge Cloudflare cache for manifest/current resource URLs from the latest manifest.
    Purge {
        #[arg(long, default_value = "generated-data")]
        data_dir: PathBuf,
    },
    /// Incrementally archive JP news events and gacha details from umapyoi.net.
    SyncUmapyoi {
        #[arg(long, default_value = umapyoi::BASE_URL)]
        base_url: String,
        #[arg(long, default_value = "generated-data/store/umapyoi_archive.json")]
        out: PathBuf,
        /// Minimum delay between requests. The default stays below the 7,200/hour limit.
        #[arg(long, default_value_t = 550, value_parser = clap::value_parser!(u64).range(500..))]
        request_interval_ms: u64,
        /// Re-fetch news posts already present in the archive.
        #[arg(long)]
        full: bool,
        /// Re-normalize the saved raw responses without making network requests.
        #[arg(long)]
        offline: bool,
    },
    /// Archive official English news posts from Umapyoi for Global planner rewards.
    SyncGlobalNews {
        #[arg(long, default_value = global_news::ENDPOINT)]
        endpoint: String,
        #[arg(
            long,
            default_value = "generated-data/store/official_news_archive.json"
        )]
        out: PathBuf,
    },
    /// Archive official English X posts for Global planner rewards.
    SyncGlobalSocial {
        #[arg(long, default_value = global_social::PROFILE_URL)]
        profile_url: String,
        #[arg(long, default_value = global_social::SYNDICATION_URL)]
        syndication_url: String,
        #[arg(long, default_value = global_social::ARCHIVE_INDEX_ENDPOINT)]
        archive_index_endpoint: String,
        #[arg(long, default_value = global_social::OEMBED_ENDPOINT)]
        oembed_endpoint: String,
        #[arg(
            long,
            default_value = "generated-data/store/official_social_archive.json"
        )]
        out: PathBuf,
    },
    /// Refresh both masters and translate the names only JP has released with Claude (needs
    /// ANTHROPIC_API_KEY). The publishing server does this on its own; this is for a local copy.
    TranslateNames {
        /// The Global master; the JP one is kept beside it as `jp-<name>`.
        #[arg(long, default_value = "master.mdb")]
        master: PathBuf,
        /// The translations to extend.
        #[arg(long, default_value = "generated-data/store/name_translations.json")]
        out: PathBuf,
    },
}

/// The asset bucket folder Global's art tree is published in; each region has a folder of its own.
const GLOBAL_ART_FOLDER: &str = "en";

/// The asset bucket folder JP's art tree is published in.
const JP_ART_FOLDER: &str = "jp";

/// A game region, with its own CDN, version and master.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Region {
    Global,
    Jp,
}

impl Region {
    fn cdn(self) -> &'static CdnSource {
        match self {
            Self::Global => &GLOBAL_CDN,
            Self::Jp => &JP_CDN,
        }
    }

    /// Where the region's master is kept, given the Global master's path.
    fn master(self, global_master: &Path) -> PathBuf {
        match self {
            Self::Global => global_master.to_path_buf(),
            Self::Jp => master_fetch::jpMasterPath(global_master),
        }
    }

    /// The asset bucket folder the region's art tree is published in. Every tree has the same
    /// layout inside its folder.
    fn folder(self) -> &'static str {
        match self {
            Self::Global => GLOBAL_ART_FOLDER,
            Self::Jp => JP_ART_FOLDER,
        }
    }
}

#[derive(Subcommand)]
enum AssetBucketAction {
    /// Log the objects and bytes below each top folder, and their total.
    Inventory,
    /// Delete every object, only when the bucket holds exactly as many as an inventory reported.
    Clear {
        #[arg(long)]
        expect: usize,
    },
}

#[derive(Subcommand)]
enum StoreAction {
    /// Fetch every file the store's bucket holds into the data directory's store/ folder.
    Pull {
        #[arg(long, default_value = "generated-data")]
        data_dir: PathBuf,
    },
    /// Upload a local file as a provided store file, such as a regenerated catalogue.
    Put {
        /// The store file name, such as timeline_campaigns.json.
        name: String,
        path: PathBuf,
    },
}

#[derive(Args, Clone)]
struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1:3000")]
    bind: SocketAddr,
    #[arg(long, default_value = "generated-data")]
    data_dir: PathBuf,
    #[arg(long, default_value = "master.mdb")]
    master: PathBuf,
    /// Legacy Postgres URL accepted for compatibility; master checks use the live game version.
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,
    /// How often the server checks the current Global game version.
    #[arg(long, env = "MASTER_REFRESH_INTERVAL_SECONDS", value_parser = clap::value_parser!(u64).range(1..), default_value_t = 300)]
    refresh_interval_seconds: u64,
    /// How often the server probes the Global CDN ahead of live for unreleased preliminary patches.
    #[arg(long, env = "EARLY_PROBE_INTERVAL_SECONDS", value_parser = clap::value_parser!(u64).range(60..), default_value_t = 3600)]
    early_probe_interval_seconds: u64,
    /// Purge manifest/current CDN URLs after an automatic refresh.
    #[arg(long, env = "PURGE_ON_REFRESH")]
    purge_on_refresh: bool,
    /// Makes this the publishing server: it builds and uploads new game images whenever the game
    /// version changes, and collects news and name translations into the shared store. Set on
    /// one server only: the bucket keeps a single record of what was published.
    #[arg(long, env = "GAME_ART_PUBLISH")]
    publish_game_art: bool,
    /// With GAME_ART_PUBLISH, also publish JP's game images as a tree of their own below `jp/`.
    /// On unless set to false.
    #[arg(long, env = "JP_GAME_ART_PUBLISH", default_value_t = true, action = clap::ArgAction::Set, value_parser = clap::builder::BoolishValueParser::new())]
    publish_jp_game_art: bool,
    /// Also generate the JP resource set from the JP master and serve it below `/resources/jp/`.
    /// On unless set to false.
    #[arg(long, env = "JP_RESOURCES", default_value_t = true, action = clap::ArgAction::Set, value_parser = clap::builder::BoolishValueParser::new())]
    jp_resources: bool,
    /// How often the publishing server collects news and new names into the shared store.
    #[arg(long, env = "STORE_SYNC_INTERVAL_SECONDS", value_parser = clap::value_parser!(u64).range(300..), default_value_t = 1800)]
    store_sync_interval_seconds: u64,
    /// How often the game art publisher also refreshes the timeline banners.
    #[arg(long, env = "TIMELINE_ART_INTERVAL_SECONDS", value_parser = clap::value_parser!(u64).range(60..), default_value_t = 1800)]
    timeline_art_interval_seconds: u64,
    /// Also write plain .json files beside the .json.gz artifacts during startup generation.
    #[arg(long)]
    write_json: bool,
    /// Serve the existing generated-data directory without regenerating from master.mdb first.
    #[arg(long)]
    no_generate: bool,
}

impl Default for ServeArgs {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:3000"
                .parse()
                .expect("default bind address should be valid"),
            data_dir: PathBuf::from("generated-data"),
            master: PathBuf::from("master.mdb"),
            database_url: None,
            refresh_interval_seconds: 300,
            early_probe_interval_seconds: 3600,
            purge_on_refresh: false,
            publish_game_art: false,
            publish_jp_game_art: true,
            jp_resources: true,
            store_sync_interval_seconds: 1800,
            timeline_art_interval_seconds: 1800,
            write_json: false,
            no_generate: false,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "umamoe_resources=info,info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Serve(cli.server)) {
        Command::Serve(args) => serve(args).await?,
        Command::FetchMaster { master, region } => {
            master_fetch::maybeUpdateMasterMdb(region.cdn(), &region.master(&master)).await?;
            if region == Region::Global {
                if let Err(error) =
                    master_fetch::maybeUpdateEarlyMasterMdb(&GLOBAL_CDN, &master).await
                {
                    warn!(
                        error = format!("{error:#}"),
                        "early Global CDN master probe failed"
                    );
                }
            }
        }
        Command::Generate {
            master,
            out,
            write_json,
            region,
        } => {
            openLocalStore(&out).await;
            master_fetch::maybeUpdateMasterMdb(&GLOBAL_CDN, &master).await?;
            if region == Region::Global {
                if let Err(error) =
                    master_fetch::maybeUpdateEarlyMasterMdb(&GLOBAL_CDN, &master).await
                {
                    warn!(
                        error = format!("{error:#}"),
                        "early Global CDN master probe failed"
                    );
                }
            }
            let manifest = match region {
                Region::Global => pipeline::generate_resources(&master, &out, write_json)?,
                Region::Jp => {
                    let jp_master = region.master(&master);
                    master_fetch::maybeUpdateMasterMdb(&JP_CDN, &jp_master).await?;
                    pipeline::generate_jp_resources(&jp_master, &master, &out, write_json)?
                }
            };
            info!(
                version = manifest.version,
                artifacts = manifest.artifacts.len(),
                "generated resources"
            );
        }
        Command::AssetBucket { action } => {
            let bucket = assets::Bucket::fromEnv()?;
            match action {
                AssetBucketAction::Inventory => {
                    assets::bucketInventory(&bucket).await?;
                }
                AssetBucketAction::Clear { expect } => {
                    assets::clearBucket(&bucket, expect).await?;
                }
            }
        }
        Command::Store { action } => match action {
            StoreAction::Pull { data_dir } => store::pullCommand(&data_dir).await?,
            StoreAction::Put { name, path } => store::putCommand(&name, &path).await?,
        },
        Command::AssetsPublish {
            dir,
            prefix,
            dry_run,
            concurrency,
            purge_origins,
        } => {
            let bucket = assets::Bucket::fromEnv()?;
            let options = assets::PublishOptions {
                source_dir: dir,
                key_prefix: prefix,
                dry_run,
                concurrency,
            };
            let summary = assets::publishAssets(&bucket, options).await?;
            if !dry_run {
                assets::purgeOverwritten(&summary.overwritten, &purge_origins).await?;
            }
        }
        Command::GameArt {
            master,
            region,
            dry_run,
            out_dir,
            sample,
            only,
        } => {
            let options = assets::GameArtOptions::fromEnv()?.inFolder(region.folder());
            let master = region.master(&master);
            master_fetch::maybeUpdateMasterMdb(region.cdn(), &master).await?;
            let version = master_fetch::localResourceVersion(&master)
                .context("master.mdb has no version marker")?;
            let source = assets::GameArtSource {
                cdn: region.cdn(),
                master: &master,
                resource_version: &version,
            };
            match out_dir {
                Some(folder) if dry_run => {
                    assets::previewGameArtDirectory(&source, &folder).await?;
                }
                Some(folder) => {
                    assets::buildGameArtSample(&options, &source, &folder, sample, &only).await?;
                }
                None => {
                    let tree = assets::Bucket::fromEnv()?.inFolder(region.folder());
                    assets::publishGameArt(&tree, &options, &source, dry_run).await?;
                }
            }
        }
        Command::TimelineArt {
            timeline,
            data_dir,
            dry_run,
            out_dir,
            sample,
            compare_frontend,
        } => {
            openLocalStore(&data_dir).await;
            let timeline_path = match timeline {
                Some(path) => path,
                None => assets::currentTimelinePath(&data_dir)?,
            };
            let timeline_json = assets::readTimeline(&timeline_path)?;
            let options = assets::TimelineArtOptions::fromEnv().inFolder(GLOBAL_ART_FOLDER);

            if let Some(sync_dir) = compare_frontend {
                let report = assets::compareTimelineArtWithFrontend(&timeline_json, &sync_dir)?;
                printParity("Japanese", &report.japanese);
                printParity("English", &report.english);
            } else if dry_run {
                assets::reportTimelineArtPlan(&timeline_json)?;
            } else if let Some(folder) = out_dir {
                assets::buildTimelineArtSample(&options, &timeline_json, &folder, sample).await?;
            } else {
                let tree = assets::Bucket::fromEnv()?.inFolder(GLOBAL_ART_FOLDER);
                assets::publishTimelineArt(&tree, &options, &timeline_json).await?;
            }
        }
        Command::Refresh {
            database_url: _,
            master,
            out,
            write_json,
            purge,
        } => {
            openLocalStore(&out).await;
            if refreshFromGameVersion(&master, &out, write_json, purge, true).await? {
                info!("master.mdb refresh completed");
            } else {
                info!("master.mdb did not change; skipping regeneration");
            }
        }
        Command::BackfillMasterHistory { data_dir } => {
            master_history::backfillMasterHistory(&data_dir).await?;
        }
        Command::Purge { data_dir } => {
            let manifest = pipeline::read_manifest(&data_dir)?;
            cache::purge_manifest_current_urls(&manifest).await?;
        }
        Command::SyncUmapyoi {
            base_url,
            out,
            request_interval_ms,
            full,
            offline,
        } => {
            let summary =
                umapyoi::sync(&base_url, &out, request_interval_ms, full, offline).await?;
            info!(
                news_posts = summary.news_posts,
                gacha_banners = summary.gacha_banners,
                support_cards = summary.support_cards,
                new_news_posts = summary.new_news_posts,
                changed = summary.changed,
                "umapyoi JP archive synchronized"
            );
            for error in summary.source_errors {
                warn!(error, "umapyoi source could not be fully synchronized");
            }
        }
        Command::TranslateNames { master, out } => {
            let jp_master = master_fetch::jpMasterPath(&master);
            master_fetch::maybeUpdateMasterMdb(&GLOBAL_CDN, &master).await?;
            master_fetch::maybeUpdateMasterMdb(&JP_CDN, &jp_master).await?;
            // The translations live in a store copy, whose name overrides they leave alone.
            if let Some(dir) = out.parent() {
                store::open(dir, None, false).await;
            }
            jp_names::translateNames(&master, &jp_master, &out).await?;
        }
        Command::SyncGlobalNews { endpoint, out } => {
            let summary = global_news::sync(&endpoint, &out).await?;
            let audit = store::auditGlobalNews(&out)?;
            info!(
                posts = summary.posts,
                new_posts = summary.new_posts,
                updated_posts = summary.updated_posts,
                changed = summary.changed,
                "official Global news archive synchronized"
            );
            info!(
                checked_posts = audit.checked_posts,
                reward_posts = audit.reward_posts,
                parsed_rewards = audit.parsed_rewards,
                "official Global news reward audit passed"
            );
        }
        Command::SyncGlobalSocial {
            profile_url,
            syndication_url,
            archive_index_endpoint,
            oembed_endpoint,
            out,
        } => {
            let summary = global_social::sync(
                &profile_url,
                &syndication_url,
                &archive_index_endpoint,
                &oembed_endpoint,
                &out,
            )
            .await?;
            info!(
                posts = summary.posts,
                discovered_posts = summary.discovered_posts,
                new_posts = summary.new_posts,
                updated_posts = summary.updated_posts,
                changed = summary.changed,
                "official Global social archive synchronized"
            );
            for error in summary.source_errors {
                if std::env::var_os("GITHUB_ACTIONS").is_some() {
                    // Shown on the run summary, so a source X starts blocking doesn't go unnoticed.
                    println!(
                        "::warning title=Official X sync::{}",
                        error
                            .replace('%', "%25")
                            .replace('\r', "%0D")
                            .replace('\n', "%0A")
                    );
                }
                warn!(
                    error,
                    "official Global social source could not be fully synchronized"
                );
            }
        }
    }

    Ok(())
}

/// Readies the store copy in `data_dir` for a one-shot command, so it generates from the same
/// data a server there would.
#[allow(non_snake_case)]
async fn openLocalStore(data_dir: &Path) {
    let dir = data_dir.join(store::LOCAL_FOLDER);
    store::open(&dir, store::bucketFromEnv().as_ref(), false).await;
}

async fn serve(args: ServeArgs) -> Result<()> {
    let store_dir = args.data_dir.join(store::LOCAL_FOLDER);
    let store_bucket = store::bucketFromEnv();
    store::open(&store_dir, store_bucket.as_ref(), args.publish_game_art).await;

    let startup = async {
        let refreshed_before_serve = refreshFromGameVersion(
            &args.master,
            &args.data_dir,
            args.write_json,
            args.purge_on_refresh,
            true,
        )
        .await?;

        if !args.no_generate && !refreshed_before_serve {
            let manifest =
                pipeline::generate_resources(&args.master, &args.data_dir, args.write_json)?;
            info!(
                version = manifest.version,
                artifacts = manifest.artifacts.len(),
                "generated resources before serving"
            );
        }
        anyhow::Ok(())
    }
    .await;
    // A server that already has resources serves them while the refresh loop retries, so a store
    // that is unavailable for a while does not keep it restarting. The publisher runs without
    // any: its sync loop collects the files the refresh loop then generates from.
    if let Err(error) = startup {
        if pipeline::read_manifest(&args.data_dir).is_ok() {
            warn!(
                error = format!("{error:#}"),
                "startup generation failed; serving the existing resources until a refresh succeeds"
            );
        } else if args.publish_game_art {
            warn!(
                error = format!("{error:#}"),
                "startup generation failed; serving nothing until the store is collected and a refresh succeeds"
            );
        } else {
            return Err(error);
        }
    }

    spawnRefreshLoop(args.clone(), store_bucket.clone());
    if args.publish_game_art {
        store::spawnSyncLoop(
            store_dir,
            store_bucket,
            args.master.clone(),
            args.store_sync_interval_seconds,
        );
        spawnArtPublishing(&args);
    }

    static_api::serve(args.data_dir, args.master, args.bind).await
}

#[allow(non_snake_case)]
async fn refreshFromGameVersion(
    master: &PathBuf,
    out: &PathBuf,
    write_json: bool,
    purge: bool,
    probe_early: bool,
) -> Result<bool> {
    let master_updated = master_fetch::maybeUpdateMasterMdb(&GLOBAL_CDN, master).await?;
    let early_updated = if probe_early || master_updated {
        match master_fetch::maybeUpdateEarlyMasterMdb(&GLOBAL_CDN, master).await {
            Ok(updated) => updated,
            Err(error) => {
                warn!(
                    error = format!("{error:#}"),
                    "early Global CDN master probe failed; continuing with live master"
                );
                false
            }
        }
    } else {
        false
    };
    // The version covers everything the set reads, including the JP master and any early CDN master.
    let version = pipeline::resources_version(master)?;
    let resources_match = pipeline::read_manifest(out).is_ok_and(|manifest| {
        let served_from_global_path = manifest
            .base_path
            .starts_with(&format!("{}/", pipeline::GLOBAL_RESOURCE_PATH));
        manifest.version == version && served_from_global_path
    });
    if !master_updated && !early_updated && resources_match {
        return Ok(false);
    }

    let manifest = pipeline::generate_resources(master, out, write_json)?;
    info!(
        version = manifest.version,
        artifacts = manifest.artifacts.len(),
        "regenerated resources after master update"
    );
    if purge {
        cache::purge_manifest_current_urls(&manifest).await?;
    } else {
        warn!("master changed; run `purge --data-dir {}` or pass --purge/--purge-on-refresh to clear current CDN URLs", out.display());
    }

    Ok(true)
}

#[allow(non_snake_case)]
/// Keeps the JP master current for whatever uses it, and regenerates the JP set when either
/// master changed. It does nothing when both JP switches are off. The refresh loop is the only
/// writer of the JP master, as it is of the Global one.
async fn refreshJpFromGameVersion(args: &ServeArgs) -> Result<bool> {
    if !args.jp_resources && !args.publish_jp_game_art {
        return Ok(false);
    }

    let jp_master = master_fetch::jpMasterPath(&args.master);
    let master_updated = master_fetch::maybeUpdateMasterMdb(&JP_CDN, &jp_master).await?;
    if !args.jp_resources {
        return Ok(master_updated);
    }

    let version = pipeline::jp_resources_version(&jp_master, &args.master)?;
    let jp_dir = pipeline::jp_data_dir(&args.data_dir);
    if pipeline::read_manifest(&jp_dir).is_ok_and(|manifest| manifest.version == version) {
        return Ok(false);
    }

    let manifest =
        pipeline::generate_jp_resources(&jp_master, &args.master, &args.data_dir, args.write_json)?;
    info!(
        version = manifest.version,
        artifacts = manifest.artifacts.len(),
        "regenerated JP resources"
    );
    if args.purge_on_refresh {
        cache::purge_manifest_current_urls(&manifest).await?;
    }
    Ok(true)
}

#[allow(non_snake_case)]
/// `store_bucket` is the shared store. Every server takes its provided files from it on each
/// check; a reading server also takes the collected files, which the publisher writes itself.
fn spawnRefreshLoop(args: ServeArgs, store_bucket: Option<assets::Bucket>) {
    let interval_seconds = args.refresh_interval_seconds;
    let early_probe_interval = Duration::from_secs(args.early_probe_interval_seconds);
    let store_dir = args.data_dir.join(store::LOCAL_FOLDER);
    let publishes = args.publish_game_art;
    let jp_args = args.clone();
    let master = args.master;
    let data_dir = args.data_dir;
    let write_json = args.write_json;
    let purge_on_refresh = args.purge_on_refresh;

    info!(
        interval_seconds,
        early_probe_interval_seconds = args.early_probe_interval_seconds,
        purge_on_refresh,
        "automatic master refresh enabled"
    );

    tokio::spawn(async move {
        // Startup already ran an initial early CDN probe; space out subsequent 100-step scans.
        let mut last_early_probe = tokio::time::Instant::now();

        // JP is an addition: the Global set is served without waiting for JP's first check.
        if let Err(error) = refreshJpFromGameVersion(&jp_args).await {
            warn!(error = format!("{error:#}"), "first JP refresh failed");
        }

        let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
        interval.tick().await;

        loop {
            interval.tick().await;

            // The store first: both sets regenerate when one of its files changed.
            if let Some(bucket) = &store_bucket {
                if let Err(error) = store::pull(bucket, &store_dir, publishes).await {
                    warn!(
                        error = format!("{error:#}"),
                        "store could not be fetched; keeping this server's copy"
                    );
                }
            }
            // JP first: the Global set reads the JP master, so it picks up a new one at once.
            if let Err(error) = refreshJpFromGameVersion(&jp_args).await {
                warn!(error = format!("{error:#}"), "automatic JP refresh failed");
            }
            let probe_early = last_early_probe.elapsed() >= early_probe_interval;
            if probe_early {
                last_early_probe = tokio::time::Instant::now();
            }
            if let Err(error) = refreshFromGameVersion(
                &master,
                &data_dir,
                write_json,
                purge_on_refresh,
                probe_early,
            )
            .await
            {
                warn!(error = %error, "automatic master refresh failed");
            }
        }
    });
}

#[allow(non_snake_case)]
/// Starts the art publishers once the tree that was published at the bucket's root has been
/// copied into Global's folder; started before, they would rebuild every file. The timeline
/// banners belong to Global's tree.
fn spawnArtPublishing(args: &ServeArgs) {
    let root = match assets::Bucket::fromEnv() {
        Ok(bucket) => bucket,
        Err(error) => {
            warn!(error = %error, "art publishing is on but the bucket is not configured");
            return;
        }
    };
    let tree = root.inFolder(GLOBAL_ART_FOLDER);
    let retry_seconds = args.refresh_interval_seconds;
    let timeline_interval_seconds = args.timeline_art_interval_seconds;
    let (master, data_dir) = (args.master.clone(), args.data_dir.clone());
    let mut regions = vec![Region::Global];
    if args.publish_jp_game_art {
        regions.push(Region::Jp);
    }

    tokio::spawn(async move {
        while let Err(error) = assets::moveRootTree(&root, &tree).await {
            warn!(
                error = format!("{error:#}"),
                "moving the root art tree failed; art publishing waits"
            );
            tokio::time::sleep(Duration::from_secs(retry_seconds)).await;
        }

        spawnGameArtLoop(retry_seconds, master, root, regions);
        spawnTimelineArtLoop(timeline_interval_seconds, data_dir, tree);
    });
}

#[allow(non_snake_case)]
/// Publishes each region's game art in its folder of the bucket, for the version of its local
/// master, so the cards it reads always match the files it builds. The refresh loop moves each
/// master to its region's new game versions. Regions take turns, Global first, since they share
/// the converter and its work folder.
fn spawnGameArtLoop(
    interval_seconds: u64,
    master: PathBuf,
    root: assets::Bucket,
    regions: Vec<Region>,
) {
    let options = match assets::GameArtOptions::fromEnv() {
        Ok(options) => options,
        Err(error) => {
            warn!(error = %error, "game art publishing is on but not configured");
            return;
        }
    };

    info!(
        interval_seconds,
        regions = regions.len(),
        "automatic game art publishing enabled"
    );
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut published_versions: Vec<Option<String>> = vec![None; regions.len()];

        loop {
            interval.tick().await;

            for (region, published_version) in regions.iter().zip(&mut published_versions) {
                let tree = root.inFolder(region.folder());
                let options = options.clone().inFolder(region.folder());
                publishRegionArt(*region, &master, &tree, &options, published_version).await;
            }
        }
    });
}

#[allow(non_snake_case)]
/// Publishes one region's tree when its master's version is not the one last published.
async fn publishRegionArt(
    region: Region,
    global_master: &Path,
    bucket: &assets::Bucket,
    options: &assets::GameArtOptions,
    published_version: &mut Option<String>,
) {
    let cdn = region.cdn();
    let master = region.master(global_master);
    let Some(version) = master_fetch::localResourceVersion(&master) else {
        warn!(master = %master.display(), "game art waits for a master.mdb version marker");
        return;
    };
    if published_version.as_ref() == Some(&version) {
        return;
    }

    let source = assets::GameArtSource {
        cdn,
        master: &master,
        resource_version: &version,
    };
    match assets::publishGameArt(bucket, options, &source, false).await {
        Ok(summary) if summary.isComplete() => *published_version = Some(version),
        Ok(_) => {}
        Err(error) => warn!(
            region = cdn.region,
            error = %error,
            "game art publishing failed"
        ),
    }
}

#[allow(non_snake_case)]
/// Publishes the current timeline's banners at start and then on every interval, which catches
/// new timelines, new news archives and English art uploaded around a release.
fn spawnTimelineArtLoop(interval_seconds: u64, data_dir: PathBuf, bucket: assets::Bucket) {
    let options = assets::TimelineArtOptions::fromEnv().inFolder(GLOBAL_ART_FOLDER);

    info!(
        interval_seconds,
        "automatic timeline art publishing enabled"
    );
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            interval.tick().await;

            let timeline =
                assets::currentTimelinePath(&data_dir).and_then(|path| assets::readTimeline(&path));
            let timeline_json = match timeline {
                Ok(timeline_json) => timeline_json,
                Err(error) => {
                    warn!(
                        error = format!("{error:#}"),
                        "timeline art waits for a generated timeline"
                    );
                    continue;
                }
            };
            if let Err(error) = assets::publishTimelineArt(&bucket, &options, &timeline_json).await
            {
                warn!(
                    error = format!("{error:#}"),
                    "timeline art publishing failed"
                );
            }
        }
    });
}

#[allow(non_snake_case)]
fn printParity(locale: &str, parity: &generators::timeline_images::LocaleParity) {
    println!(
        "{locale}: {} matching, {} different, {} recorded but not picked",
        parity.matching,
        parity.mismatches.len(),
        parity.unplanned.len()
    );
    for mismatch in &parity.mismatches {
        println!("  {}", mismatch.image_path);
        println!(
            "    picked   {}",
            mismatch.planned.as_deref().unwrap_or("nothing")
        );
        println!("    recorded {}", mismatch.recorded);
    }
    for state_key in &parity.unplanned {
        println!("  not picked: {state_key}");
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn jp_is_on_unless_switched_off() {
        let by_default = Cli::try_parse_from(["umamoe-resources"]).unwrap().server;
        let switched_off = Cli::try_parse_from([
            "umamoe-resources",
            "--jp-resources",
            "false",
            "--publish-jp-game-art",
            "0",
        ])
        .unwrap()
        .server;

        assert!(by_default.jp_resources && by_default.publish_jp_game_art);
        assert!(!switched_off.jp_resources && !switched_off.publish_jp_game_art);
    }
}

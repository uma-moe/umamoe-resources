use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{info, warn};

mod auth_common;
mod browser_proof;
mod cache;
mod generators;
#[path = "../master_fetch.rs"]
mod master_fetch;
mod pipeline;
mod static_api;
mod umapyoi;

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
    /// Generate versioned .json.gz resources without starting the server.
    Generate {
        #[arg(long, default_value = "master.mdb")]
        master: PathBuf,
        #[arg(long, default_value = "generated-data")]
        out: PathBuf,
        #[arg(long)]
        write_json: bool,
    },
    /// Check master_versions, refresh master.mdb if needed, regenerate, then purge current CDN URLs.
    Refresh {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        #[arg(long, default_value = "master.mdb")]
        master: PathBuf,
        #[arg(long, default_value = "generated-data")]
        out: PathBuf,
        #[arg(long)]
        write_json: bool,
        #[arg(long)]
        purge: bool,
    },
    /// Purge Cloudflare cache for manifest/current resource URLs from the latest manifest.
    Purge {
        #[arg(long, default_value = "generated-data")]
        data_dir: PathBuf,
    },
    /// Incrementally archive JP news events and gacha details from umapyoi.net.
    SyncUmapyoi {
        #[arg(long, default_value = "https://umapyoi.net/api/v1")]
        base_url: String,
        #[arg(long, default_value = "src/jp_data/umapyoi_archive.json")]
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
}

#[derive(Args, Clone)]
struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1:3000")]
    bind: SocketAddr,
    #[arg(long, default_value = "generated-data")]
    data_dir: PathBuf,
    #[arg(long, default_value = "master.mdb")]
    master: PathBuf,
    /// Optional Postgres URL used to auto-refresh master.mdb from master_versions while serving.
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,
    /// How often the server checks master_versions for a newer resource version.
    #[arg(long, env = "MASTER_REFRESH_INTERVAL_SECONDS", value_parser = clap::value_parser!(u64).range(1..), default_value_t = 300)]
    refresh_interval_seconds: u64,
    /// Purge manifest/current CDN URLs after an automatic refresh.
    #[arg(long, env = "PURGE_ON_REFRESH")]
    purge_on_refresh: bool,
    /// Optional CSV URL for phone-friendly confirmed banner date updates.
    #[arg(long, env = "CONFIRMED_BANNER_DATES_URL")]
    confirmed_banner_dates_url: Option<String>,
    /// Local path used to cache confirmed banner dates fetched from the URL.
    #[arg(long, env = "CONFIRMED_BANNER_DATES_PATH")]
    confirmed_banner_dates_path: Option<PathBuf>,
    /// How often the server checks CONFIRMED_BANNER_DATES_URL for edits.
    #[arg(long, env = "CONFIRMED_BANNER_DATES_REFRESH_INTERVAL_SECONDS", value_parser = clap::value_parser!(u64).range(1..), default_value_t = 60)]
    confirmed_banner_dates_refresh_interval_seconds: u64,
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
            purge_on_refresh: false,
            confirmed_banner_dates_url: None,
            confirmed_banner_dates_path: None,
            confirmed_banner_dates_refresh_interval_seconds: 60,
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
        Command::Generate {
            master,
            out,
            write_json,
        } => {
            let manifest = pipeline::generate_resources(&master, &out, write_json)?;
            info!(
                version = manifest.version,
                artifacts = manifest.artifacts.len(),
                "generated resources"
            );
        }
        Command::Refresh {
            database_url,
            master,
            out,
            write_json,
            purge,
        } => {
            let pool = PgPool::connect(&database_url)
                .await
                .context("failed to connect to DATABASE_URL")?;
            if refresh_from_db(&pool, &master, &out, write_json, purge).await? {
                info!("master.mdb refresh completed");
            } else {
                info!("master.mdb did not change; skipping regeneration");
            }
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
    }

    Ok(())
}

async fn serve(args: ServeArgs) -> Result<()> {
    let confirmed_banner_dates_url = confirmed_banner_dates_url(&args).map(ToOwned::to_owned);
    let confirmed_banner_dates_path = confirmed_banner_dates_path(&args);
    if let Some(path) = confirmed_banner_dates_path.as_ref() {
        std::env::set_var("CONFIRMED_BANNER_DATES_PATH", path);
    }

    let confirmed_dates_refreshed_before_serve = if let (Some(url), Some(path)) = (
        confirmed_banner_dates_url.as_deref(),
        confirmed_banner_dates_path.as_ref(),
    ) {
        match refresh_confirmed_banner_dates_from_url(url, path).await {
            Ok(true) => {
                info!(
                    path = %path.display(),
                    "loaded updated confirmed banner dates before serving"
                );
                true
            }
            Ok(false) => {
                info!(
                    path = %path.display(),
                    "confirmed banner dates already up to date before serving"
                );
                false
            }
            Err(error) => {
                warn!(
                    error = %error,
                    "failed to fetch confirmed banner dates before serving; using cached or bundled dates"
                );
                false
            }
        }
    } else {
        false
    };

    let refresh_pool = match database_url(&args) {
        Some(database_url) => Some(
            PgPool::connect(database_url)
                .await
                .context("failed to connect to DATABASE_URL")?,
        ),
        None => None,
    };

    let refreshed_before_serve = if let Some(pool) = refresh_pool.as_ref() {
        let refreshed = refresh_from_db(
            pool,
            &args.master,
            &args.data_dir,
            args.write_json,
            args.purge_on_refresh,
        )
        .await?;
        if refreshed {
            info!("refreshed master.mdb before serving");
        } else {
            info!("master.mdb did not change before serving");
        }
        refreshed
    } else {
        info!("DATABASE_URL not set; automatic master refresh disabled");
        false
    };

    if !args.no_generate && !refreshed_before_serve {
        let manifest = pipeline::generate_resources(&args.master, &args.data_dir, args.write_json)?;
        info!(
            version = manifest.version,
            artifacts = manifest.artifacts.len(),
            "generated resources before serving"
        );
    }

    if confirmed_dates_refreshed_before_serve {
        let manifest = pipeline::read_manifest(&args.data_dir)?;
        cache::purge_manifest_current_urls(&manifest).await?;
    }

    if let Some(pool) = refresh_pool {
        spawn_refresh_loop(pool, args.clone());
    }

    match (confirmed_banner_dates_url, confirmed_banner_dates_path) {
        (Some(url), Some(path)) => {
            spawn_confirmed_banner_dates_url_refresh_loop(args.clone(), path, url);
        }
        (None, Some(path)) => {
            spawn_confirmed_banner_dates_file_refresh_loop(args.clone(), path);
        }
        _ => {}
    }

    static_api::serve(args.data_dir, args.master, args.bind).await
}

fn confirmed_banner_dates_url(args: &ServeArgs) -> Option<&str> {
    args.confirmed_banner_dates_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
}

fn database_url(args: &ServeArgs) -> Option<&str> {
    args.database_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
}

fn confirmed_banner_dates_path(args: &ServeArgs) -> Option<PathBuf> {
    args.confirmed_banner_dates_path
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty())
        .cloned()
        .or_else(|| {
            confirmed_banner_dates_url(args)
                .map(|_| args.data_dir.join("confirmed_global_banner_dates.csv"))
        })
}

async fn refresh_from_db(
    pool: &PgPool,
    master: &PathBuf,
    out: &PathBuf,
    write_json: bool,
    purge: bool,
) -> Result<bool> {
    let master_path = master
        .to_str()
        .context("master path must be valid UTF-8 for the fetcher")?;
    let master_updated = master_fetch::maybe_update_master_mdb(pool, master_path).await?;

    if !master_updated {
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

fn spawn_refresh_loop(pool: PgPool, args: ServeArgs) {
    let interval_seconds = args.refresh_interval_seconds;
    let master = args.master;
    let data_dir = args.data_dir;
    let write_json = args.write_json;
    let purge_on_refresh = args.purge_on_refresh;

    info!(
        interval_seconds,
        purge_on_refresh, "automatic master refresh enabled"
    );

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
        interval.tick().await;

        loop {
            interval.tick().await;

            if let Err(error) =
                refresh_from_db(&pool, &master, &data_dir, write_json, purge_on_refresh).await
            {
                warn!(error = %error, "automatic master refresh failed");
            }
        }
    });
}

fn spawn_confirmed_banner_dates_url_refresh_loop(args: ServeArgs, path: PathBuf, url: String) {
    let interval_seconds = args.confirmed_banner_dates_refresh_interval_seconds;
    let master = args.master;
    let data_dir = args.data_dir;
    let write_json = args.write_json;

    info!(
        interval_seconds,
        url,
        path = %path.display(),
        "automatic confirmed banner dates refresh enabled"
    );

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
        interval.tick().await;

        loop {
            interval.tick().await;

            if let Err(error) =
                refresh_from_confirmed_banner_dates_url(&url, &path, &master, &data_dir, write_json)
                    .await
            {
                warn!(error = %error, "confirmed banner dates refresh failed");
            }
        }
    });
}

fn spawn_confirmed_banner_dates_file_refresh_loop(args: ServeArgs, path: PathBuf) {
    let interval_seconds = args.confirmed_banner_dates_refresh_interval_seconds;
    let master = args.master;
    let data_dir = args.data_dir;
    let write_json = args.write_json;

    info!(
        interval_seconds,
        path = %path.display(),
        "automatic mounted confirmed banner dates refresh enabled"
    );

    tokio::spawn(async move {
        let mut last_hash = match confirmed_banner_dates_file_hash(&path).await {
            Ok(hash) => hash,
            Err(error) => {
                warn!(error = %error, "failed to read mounted confirmed banner dates before polling");
                None
            }
        };
        let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
        interval.tick().await;

        loop {
            interval.tick().await;

            match confirmed_banner_dates_file_hash(&path).await {
                Ok(current_hash) if current_hash != last_hash => {
                    last_hash = current_hash;
                    if last_hash.is_none() {
                        warn!(
                            path = %path.display(),
                            "mounted confirmed banner dates file disappeared; keeping current generated resources"
                        );
                        continue;
                    }

                    if let Err(error) = regenerate_after_confirmed_banner_dates_change(
                        &master, &data_dir, write_json,
                    )
                    .await
                    {
                        warn!(error = %error, "failed to regenerate after mounted confirmed banner dates changed");
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    warn!(error = %error, "mounted confirmed banner dates refresh failed");
                }
            }
        }
    });
}

async fn refresh_from_confirmed_banner_dates_url(
    url: &str,
    path: &Path,
    master: &PathBuf,
    out: &PathBuf,
    write_json: bool,
) -> Result<bool> {
    if !refresh_confirmed_banner_dates_from_url(url, path).await? {
        return Ok(false);
    }

    regenerate_after_confirmed_banner_dates_change(master, out, write_json).await?;
    Ok(true)
}

async fn regenerate_after_confirmed_banner_dates_change(
    master: &PathBuf,
    out: &PathBuf,
    write_json: bool,
) -> Result<()> {
    let manifest = pipeline::generate_resources(master, out, write_json)?;
    info!(
        version = manifest.version,
        artifacts = manifest.artifacts.len(),
        "regenerated resources after confirmed banner date update"
    );

    cache::purge_manifest_current_urls(&manifest).await?;

    Ok(())
}

async fn refresh_confirmed_banner_dates_from_url(url: &str, path: &Path) -> Result<bool> {
    let response = reqwest::get(url)
        .await
        .with_context(|| format!("failed to fetch confirmed banner dates from {url}"))?
        .error_for_status()
        .with_context(|| format!("confirmed banner dates URL returned an error: {url}"))?;
    let body = response
        .text()
        .await
        .with_context(|| format!("failed to read confirmed banner dates response from {url}"))?;
    let body = normalize_text_file(&body);

    generators::timeline::validate_confirmed_dates_csv(&body)
        .context("remote confirmed banner dates CSV is invalid")?;

    let existing = tokio::fs::read_to_string(path).await.ok();
    if existing.as_deref() == Some(body.as_str()) {
        return Ok(false);
    }

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    tokio::fs::write(path, body)
        .await
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(true)
}

async fn confirmed_banner_dates_file_hash(path: &Path) -> Result<Option<String>> {
    let body = match tokio::fs::read_to_string(path).await {
        Ok(body) => normalize_text_file(&body),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };

    generators::timeline::validate_confirmed_dates_csv(&body).with_context(|| {
        format!(
            "mounted confirmed banner dates CSV is invalid: {}",
            path.display()
        )
    })?;

    Ok(Some(hex::encode(Sha256::digest(body.as_bytes()))))
}

fn normalize_text_file(value: &str) -> String {
    let mut normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    if !normalized.ends_with('\n') {
        normalized.push('\n');
    }
    normalized
}

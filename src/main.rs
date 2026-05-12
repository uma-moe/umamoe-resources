use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use sqlx::PgPool;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{info, warn};

mod cache;
mod generators;
#[path = "../master_fetch.rs"]
mod master_fetch;
mod pipeline;
mod static_api;

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
    #[arg(long)]
    purge_on_refresh: bool,
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
    }

    Ok(())
}

async fn serve(args: ServeArgs) -> Result<()> {
    let refresh_pool = match args.database_url.as_deref() {
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

    if let Some(pool) = refresh_pool {
        spawn_refresh_loop(pool, args.clone());
    }

    static_api::serve(args.data_dir, args.bind).await
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
        purge_on_refresh,
        "automatic master refresh enabled"
    );

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
        interval.tick().await;

        loop {
            interval.tick().await;

            if let Err(error) = refresh_from_db(
                &pool,
                &master,
                &data_dir,
                write_json,
                purge_on_refresh,
            )
            .await
            {
                warn!(error = %error, "automatic master refresh failed");
            }
        }
    });
}

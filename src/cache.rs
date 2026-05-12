use crate::pipeline::ResourceManifest;
use anyhow::{Context, Result};
use reqwest::StatusCode;
use serde::Serialize;
use tracing::{info, warn};

#[derive(Debug)]
struct CloudflareConfig {
    zone_id: String,
    api_token: String,
    public_base_url: String,
}

#[derive(Serialize)]
struct PurgeRequest {
    files: Vec<String>,
}

pub async fn purge_manifest_current_urls(manifest: &ResourceManifest) -> Result<()> {
    let Some(config) = CloudflareConfig::from_env() else {
        warn!(
            "Cloudflare purge skipped; set CLOUDFLARE_ZONE_ID, CLOUDFLARE_API_TOKEN, and PUBLIC_BASE_URL"
        );
        return Ok(());
    };

    let mut paths = vec!["/resources/manifest.json".to_string()];
    paths.extend(
        manifest
            .artifacts
            .iter()
            .map(|artifact| artifact.current_path.clone()),
    );
    purge_paths(&config, paths).await
}

async fn purge_paths(config: &CloudflareConfig, paths: Vec<String>) -> Result<()> {
    let public_base_url = config.public_base_url.trim_end_matches('/');
    let files = paths
        .into_iter()
        .map(|path| format!("{}{}", public_base_url, path))
        .collect::<Vec<_>>();

    let url = format!(
        "https://api.cloudflare.com/client/v4/zones/{}/purge_cache",
        config.zone_id
    );
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(&config.api_token)
        .json(&PurgeRequest {
            files: files.clone(),
        })
        .send()
        .await
        .context("failed to send Cloudflare purge request")?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status != StatusCode::OK {
        anyhow::bail!("Cloudflare purge failed with HTTP {}: {}", status, body);
    }

    info!(count = files.len(), "purged Cloudflare resource URLs");
    Ok(())
}

impl CloudflareConfig {
    fn from_env() -> Option<Self> {
        Some(Self {
            zone_id: std::env::var("CLOUDFLARE_ZONE_ID").ok()?,
            api_token: std::env::var("CLOUDFLARE_API_TOKEN").ok()?,
            public_base_url: std::env::var("PUBLIC_BASE_URL").ok()?,
        })
    }
}

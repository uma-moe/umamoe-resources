use crate::pipeline::{ResourceManifest, GLOBAL_RESOURCE_PATH};
use anyhow::{Context, Result};
use reqwest::StatusCode;
use serde::Serialize;
use tracing::{info, warn};

/// Cloudflare accepts up to 100 URLs per purge request on every plan below Enterprise.
const PURGE_BATCH_SIZE: usize = 100;

#[derive(Debug)]
struct CloudflareConfig {
    zone_id: String,
    api_token: String,
}

#[derive(Serialize)]
struct PurgeRequest<'a> {
    files: &'a [String],
}

pub async fn purge_manifest_current_urls(manifest: &ResourceManifest) -> Result<()> {
    let (Some(config), Ok(public_base_url)) = (
        CloudflareConfig::from_env(),
        std::env::var("PUBLIC_BASE_URL"),
    ) else {
        warn!(
            "Cloudflare purge skipped; set CLOUDFLARE_ZONE_ID, CLOUDFLARE_API_TOKEN, and PUBLIC_BASE_URL"
        );
        return Ok(());
    };

    let mut paths = vec![
        format!("{GLOBAL_RESOURCE_PATH}/manifest.json"),
        "/resources/manifest.json".to_string(),
        "/resources/masterdata/versions.json".to_string(),
    ];
    for artifact in &manifest.artifacts {
        paths.push(artifact.current_path.clone());
        paths.push(format!("/resources/current/{}.gz", artifact.name));
    }

    let public_base_url = public_base_url.trim_end_matches('/');
    let urls = paths
        .into_iter()
        .map(|path| format!("{}{}", public_base_url, path))
        .collect::<Vec<_>>();
    purge_urls(&config, &urls).await
}

/// Purges full URLs, such as replaced images on each site origin.
pub async fn purge_asset_urls(urls: Vec<String>) -> Result<()> {
    let Some(config) = CloudflareConfig::from_env() else {
        warn!(
            count = urls.len(),
            "Cloudflare purge skipped; set CLOUDFLARE_ZONE_ID and CLOUDFLARE_API_TOKEN"
        );
        return Ok(());
    };

    purge_urls(&config, &urls).await
}

async fn purge_urls(config: &CloudflareConfig, urls: &[String]) -> Result<()> {
    let endpoint = format!(
        "https://api.cloudflare.com/client/v4/zones/{}/purge_cache",
        config.zone_id
    );
    let client = reqwest::Client::new();
    for files in urls.chunks(PURGE_BATCH_SIZE) {
        let response = client
            .post(&endpoint)
            .bearer_auth(&config.api_token)
            .json(&PurgeRequest { files })
            .send()
            .await
            .context("failed to send Cloudflare purge request")?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status != StatusCode::OK {
            anyhow::bail!("Cloudflare purge failed with HTTP {}: {}", status, body);
        }
    }

    info!(count = urls.len(), "purged Cloudflare URLs");
    Ok(())
}

impl CloudflareConfig {
    fn from_env() -> Option<Self> {
        Some(Self {
            zone_id: std::env::var("CLOUDFLARE_ZONE_ID").ok()?,
            api_token: std::env::var("CLOUDFLARE_API_TOKEN").ok()?,
        })
    }
}

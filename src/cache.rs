use crate::pipeline::{ResourceManifest, GLOBAL_RESOURCE_PATH, JP_RESOURCE_PATH};
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

    let public_base_url = public_base_url.trim_end_matches('/');
    let urls = current_paths(manifest)
        .into_iter()
        .map(|path| format!("{}{}", public_base_url, path))
        .collect::<Vec<_>>();
    purge_urls(&config, &urls).await
}

/// The paths that change when a set is regenerated: its manifest and every `current` artifact.
/// The Global set is also purged at the paths from before the region split, and only it has a
/// master history.
fn current_paths(manifest: &ResourceManifest) -> Vec<String> {
    let is_jp = manifest
        .base_path
        .starts_with(&format!("{JP_RESOURCE_PATH}/"));
    if is_jp {
        let mut paths = vec![format!("{JP_RESOURCE_PATH}/manifest.json")];
        paths.extend(
            manifest
                .artifacts
                .iter()
                .map(|artifact| artifact.current_path.clone()),
        );
        return paths;
    }

    let mut paths = vec![
        format!("{GLOBAL_RESOURCE_PATH}/manifest.json"),
        "/resources/manifest.json".to_string(),
        "/resources/masterdata/versions.json".to_string(),
    ];
    for artifact in &manifest.artifacts {
        paths.push(artifact.current_path.clone());
        paths.push(format!("/resources/current/{}.gz", artifact.name));
    }
    paths
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

#[cfg(test)]
mod tests {
    use super::current_paths;
    use crate::pipeline::{MasterInfo, ResourceArtifact, ResourceManifest};

    /// A set served at `set_path` with one artifact, `skills.json`.
    fn manifest(set_path: &str) -> ResourceManifest {
        ResourceManifest {
            version: "v1".to_string(),
            generated_at: String::new(),
            base_path: format!("{set_path}/v1"),
            master: MasterInfo {
                marker: String::new(),
                sha256: String::new(),
                path: None,
                bytes: None,
            },
            artifacts: vec![ResourceArtifact {
                name: "skills.json".to_string(),
                path: format!("{set_path}/v1/skills.json.gz"),
                current_path: format!("{set_path}/current/skills.json.gz"),
                content_type: String::new(),
                content_encoding: String::new(),
                etag: String::new(),
                sha256: String::new(),
                json_bytes: 0,
                gzip_bytes: 0,
            }],
        }
    }

    #[test]
    fn the_global_set_is_also_purged_at_its_old_paths() {
        assert_eq!(
            current_paths(&manifest("/resources/en")),
            [
                "/resources/en/manifest.json",
                "/resources/manifest.json",
                "/resources/masterdata/versions.json",
                "/resources/en/current/skills.json.gz",
                "/resources/current/skills.json.gz",
            ]
        );
    }

    #[test]
    fn the_jp_set_purges_only_its_own_paths() {
        assert_eq!(
            current_paths(&manifest("/resources/jp")),
            [
                "/resources/jp/manifest.json",
                "/resources/jp/current/skills.json.gz",
            ]
        );
    }
}

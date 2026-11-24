//! Early/fallback English banner art extracted directly from the Global game CDN, mirroring
//! `game_art`'s bundle download, XOR decryption, and `game_art_convert.py` texture extraction.

use crate::game_cdn::{self, ManifestEntry, GLOBAL_CDN, WINDOWS};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tracing::warn;

const CONVERTER_PYTHON: &str = "/opt/game-art/bin/python";
const CONVERTER_SCRIPT: &str = "/app/scripts/game_art_convert.py";

/// Categories on the Global Windows CDN that hold gacha banner textures.
const BANNER_CATEGORIES: &[&str] = &["outgame", "gachaselect"];

/// Lazily loads Global CDN banner manifests and extracts decrypted textures to PNGs so
/// `timeline_art_convert.py` can fit them to the timeline's Japanese banner dimensions.
pub(super) struct CdnBannerFetcher {
    client: reqwest::Client,
    resource_version: Option<String>,
    base_key: Option<Vec<u8>>,
    /// Manifest entries by game asset path, loaded once on first need; `Some(empty)` if unavailable.
    entries: Option<HashMap<String, ManifestEntry>>,
}

impl CdnBannerFetcher {
    pub(super) fn new(resource_version: Option<String>, base_key: Option<Vec<u8>>) -> Self {
        Self {
            client: reqwest::Client::new(),
            resource_version,
            base_key,
            entries: None,
        }
    }

    pub(super) fn isConfigured(&self) -> bool {
        self.resource_version
            .as_deref()
            .is_some_and(|version| !version.trim().is_empty())
            && self.base_key.as_ref().is_some_and(|key| !key.is_empty())
    }

    /// Resolves the best Global CDN banner bundle for one of `gacha_ids`:
    /// prefers `gachaselect/{id}/img_bnr_gacha_{id}` (1024x256, 4:1 match for timeline cards),
    /// then falls back to `outgame/banner/img_bnr_home_1{id}` (512x256).
    pub(super) async fn findBanner(&mut self, gacha_ids: &[i64]) -> Result<Option<ManifestEntry>> {
        if !self.isConfigured() || gacha_ids.is_empty() {
            return Ok(None);
        }
        self.ensureManifestLoaded().await?;
        let Some(entries) = self.entries.as_ref() else {
            return Ok(None);
        };
        Ok(pickBannerEntry(entries, gacha_ids).cloned())
    }

    /// Stable planned identifier for a CDN bundle; changes if Cygames updates the asset checksum.
    pub(super) fn plannedKey(entry: &ManifestEntry) -> String {
        format!("cdn:{}@{}", entry.name, entry.checksum)
    }

    /// Source string recorded in `BannerRecord` for diagnostics.
    pub(super) fn sourceLabel(&self, entry: &ManifestEntry) -> String {
        let version = self.resource_version.as_deref().unwrap_or("unknown");
        format!("cdn:{version}/{}", entry.name)
    }

    /// Downloads, decrypts, and extracts `entry`'s texture to a PNG inside `work_dir`.
    pub(super) async fn downloadAndExtract(
        &self,
        entry: &ManifestEntry,
        work_dir: &Path,
        sequence: usize,
    ) -> Result<PathBuf> {
        let base_key = self
            .base_key
            .as_deref()
            .context("GAME_ASSET_KEY is required to decrypt CDN banner bundles")?;
        let mut bundle = game_cdn::downloadAssetBundle(&self.client, &GLOBAL_CDN, entry).await?;
        game_cdn::decryptAssetBundle(&mut bundle, base_key, entry.key);

        let bundle_path = work_dir.join(format!("cdn-{sequence}.bundle"));
        let png_path = work_dir.join(format!("cdn-{sequence}.png"));
        tokio::fs::write(&bundle_path, &bundle)
            .await
            .with_context(|| format!("failed to write {}", bundle_path.display()))?;

        let object = entry
            .name
            .rsplit('/')
            .next()
            .unwrap_or(entry.name.as_str())
            .to_string();
        extractBundleTexture(&bundle_path, &object, &png_path).await?;
        Ok(png_path)
    }

    async fn ensureManifestLoaded(&mut self) -> Result<()> {
        if self.entries.is_some() {
            return Ok(());
        }
        let Some(version) = self.resource_version.as_deref() else {
            self.entries = Some(HashMap::new());
            return Ok(());
        };
        let categories =
            game_cdn::fetchCategories(&self.client, &GLOBAL_CDN, WINDOWS, version).await?;
        let mut map = HashMap::new();
        for category in BANNER_CATEGORIES {
            match game_cdn::fetchCategoryFiles(&self.client, &GLOBAL_CDN, &categories, &[category])
                .await
            {
                Ok(files) => {
                    for entry in files {
                        if isGachaBannerPath(&entry.name) {
                            map.insert(entry.name.clone(), entry);
                        }
                    }
                }
                Err(error) => {
                    warn!(
                        category,
                        version,
                        error = format!("{error:#}"),
                        "failed to load Global CDN banner category"
                    );
                }
            }
        }
        self.entries = Some(map);
        Ok(())
    }
}

fn isGachaBannerPath(name: &str) -> bool {
    name.starts_with("outgame/banner/img_bnr_home_1")
        || (name.starts_with("gachaselect/") && name.contains("/img_bnr_gacha_"))
}

fn candidatePaths(gacha_id: i64) -> [String; 2] {
    [
        format!("gachaselect/{gacha_id}/img_bnr_gacha_{gacha_id}"),
        format!("outgame/banner/img_bnr_home_1{gacha_id}"),
    ]
}

fn pickBannerEntry<'a>(
    entries: &'a HashMap<String, ManifestEntry>,
    gacha_ids: &[i64],
) -> Option<&'a ManifestEntry> {
    for &gacha_id in gacha_ids {
        for path in candidatePaths(gacha_id) {
            if let Some(entry) = entries.get(&path) {
                return Some(entry);
            }
        }
    }
    None
}

#[derive(Serialize)]
struct ExtractRequest<'a> {
    jobs: &'a [ExtractJob],
}

#[derive(Serialize)]
struct ExtractJob {
    bundle: PathBuf,
    dependencies: Vec<PathBuf>,
    outputs: Vec<ExtractOutput>,
}

#[derive(Serialize)]
struct ExtractOutput {
    object: String,
    key: String,
    path: PathBuf,
    format: &'static str,
    size: Option<[u32; 2]>,
    art_size: Option<[u32; 2]>,
}

#[derive(Deserialize)]
struct ExtractResponse {
    results: Vec<ExtractResult>,
}

#[derive(Deserialize)]
struct ExtractResult {
    #[serde(default)]
    error: Option<String>,
}

async fn extractBundleTexture(bundle: &Path, object: &str, output: &Path) -> Result<()> {
    let job = ExtractJob {
        bundle: bundle.to_path_buf(),
        dependencies: Vec::new(),
        outputs: vec![ExtractOutput {
            object: object.to_string(),
            key: object.to_string(),
            path: output.to_path_buf(),
            format: "png",
            size: None,
            art_size: None,
        }],
    };
    let request = serde_json::to_vec(&ExtractRequest { jobs: &[job] })?;

    let mut child = tokio::process::Command::new("nice")
        .args(["-n", "19", CONVERTER_PYTHON, CONVERTER_SCRIPT])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start game_art_convert.py for CDN timeline banner")?;
    let mut stdin = child.stdin.take().context("the converter has no stdin")?;
    stdin.write_all(&request).await?;
    drop(stdin);

    let output_result = child.wait_with_output().await?;
    if !output_result.status.success() {
        bail!(
            "game_art_convert.py failed: {}",
            String::from_utf8_lossy(&output_result.stderr).trim()
        );
    }
    let response: ExtractResponse = serde_json::from_slice(&output_result.stdout)
        .context("game_art_convert.py printed no valid JSON")?;
    if let Some(error) = response
        .results
        .into_iter()
        .next()
        .and_then(|result| result.error)
    {
        bail!("failed to extract CDN banner texture: {error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefersGachaSelectBannerAndFallsBackToHomeBanner() {
        let home = ManifestEntry::new("outgame/banner/img_bnr_home_130151", 100, 111, 1);
        let wide = ManifestEntry::new("gachaselect/30151/img_bnr_gacha_30151", 200, 222, 2);
        let home_only = ManifestEntry::new("outgame/banner/img_bnr_home_130150", 300, 333, 3);

        let entries = HashMap::from([
            (home.name.clone(), home),
            (wide.name.clone(), wide.clone()),
            (home_only.name.clone(), home_only.clone()),
        ]);

        assert_eq!(
            pickBannerEntry(&entries, &[30151]).map(|e| e.name.as_str()),
            Some("gachaselect/30151/img_bnr_gacha_30151")
        );
        assert_eq!(
            pickBannerEntry(&entries, &[30150]).map(|e| e.name.as_str()),
            Some("outgame/banner/img_bnr_home_130150")
        );
        assert_eq!(
            CdnBannerFetcher::plannedKey(&wide),
            "cdn:gachaselect/30151/img_bnr_gacha_30151@222"
        );
    }
}

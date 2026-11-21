//! The synced banners the site serves today. A banner's first run copies the one the site shows
//! when its own sources fail, so nothing visible is lost when the bucket takes over.

use super::fetch::SourceFetcher;
use anyhow::{Context, Result};
use std::collections::BTreeMap;

/// The site's overlay manifests: synced copy by the timeline's `image_path`.
#[derive(Debug, Default)]
pub(super) struct SiteManifests {
    english: BTreeMap<String, String>,
    japanese: BTreeMap<String, String>,
}

/// The synced copies the site shows for one banner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct SiteOverlays<'a> {
    pub(super) english: Option<&'a str>,
    pub(super) japanese: Option<&'a str>,
}

impl SiteManifests {
    pub(super) async fn load(fetcher: &mut SourceFetcher, site_origin: &str) -> Result<Self> {
        Ok(Self {
            english: readManifest(fetcher, site_origin, "en").await?,
            japanese: readManifest(fetcher, site_origin, "jp").await?,
        })
    }

    pub(super) fn overlays(&self, image_path: &str) -> SiteOverlays<'_> {
        SiteOverlays {
            english: self.english.get(image_path).map(String::as_str),
            japanese: self.japanese.get(image_path).map(String::as_str),
        }
    }
}

/// A site that no longer has the manifest has no synced copies; any other failure stops the run,
/// since a first picture taken without them would never be corrected.
async fn readManifest(
    fetcher: &mut SourceFetcher,
    site_origin: &str,
    locale: &str,
) -> Result<BTreeMap<String, String>> {
    let url = format!("{site_origin}/assets/timeline-images/{locale}/manifest.json");
    let Some(bytes) = fetcher.download(&url).await? else {
        return Ok(BTreeMap::new());
    };
    serde_json::from_slice(&bytes).with_context(|| format!("{url} is not a manifest"))
}

/// Whether the bytes are a WebP file, as every banner on the site is.
pub(in crate::assets) fn isWebp(bytes: &[u8]) -> bool {
    bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onlyWebpFilesPassAsSitePictures() {
        let webp = b"RIFF\x24\x00\x00\x00WEBPVP8 ";

        assert!(isWebp(webp));
        assert!(!isWebp(b"<!doctype html><html>"));
        assert!(!isWebp(b"RIFF"));
    }
}

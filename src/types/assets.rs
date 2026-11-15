use std::path::PathBuf;

/// Browsers revalidate artwork daily; the CDN keeps it longer and is purged when a file changes.
pub(super) const ASSET_CACHE_CONTROL: &str = "public, max-age=86400";

/// The public R2 bucket that the site's image paths are routed to.
pub(super) const ASSET_BUCKET: &str = "umamoe-assets";

/// An S3-compatible bucket (Cloudflare R2 in production) and the key that may write to it.
#[derive(Clone)]
pub struct Bucket {
    pub(super) endpoint: String,
    pub(super) region: String,
    pub(super) name: String,
    pub(super) access_key: String,
    pub(super) secret_key: String,
}

pub struct PublishOptions {
    /// Folder whose files are uploaded, keeping their paths below it.
    pub source_dir: PathBuf,
    /// Key prefix, which is also the URL path the site serves the files under (`game-assets`).
    pub key_prefix: String,
    /// Compare against the bucket without uploading anything.
    pub dry_run: bool,
    pub concurrency: usize,
}

#[derive(Debug, Default)]
pub struct PublishSummary {
    pub files: usize,
    pub bytes: u64,
    pub created: usize,
    pub unchanged: usize,
    /// Keys whose bytes were replaced, so cached copies of their URLs are stale.
    pub overwritten: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ObjectChange {
    Created,
    Overwritten,
    Unchanged,
}

pub(super) struct PublishedFile {
    pub(super) key: String,
    pub(super) bytes: u64,
    pub(super) change: ObjectChange,
}

pub(super) struct SignedRequest {
    pub(super) url: String,
    pub(super) headers: reqwest::header::HeaderMap,
}

//! Uploads image folders to the asset bucket that the site's image paths are routed to.
#![allow(non_snake_case)]

mod bucket;
mod bucket_contents;
mod game_art;
mod timeline_art;
#[path = "../types/assets.rs"]
mod types;

use crate::cache;
use anyhow::{bail, Context, Result};
pub use bucket_contents::{bucketInventory, clearBucket};
pub use game_art::{buildGameArtSample, previewGameArtDirectory, publishGameArt, GameArtOptions};
use std::path::{Path, PathBuf};
pub use timeline_art::{
    buildTimelineArtSample, compareTimelineArtWithFrontend, currentTimelinePath,
    publishTimelineArt, readTimeline, reportTimelineArtPlan, TimelineArtOptions,
};
use tokio::task::JoinSet;
use tracing::{debug, info, warn};
pub use types::{Bucket, PublishOptions, PublishSummary};
use types::{ObjectChange, PublishedFile};

/// Uploads every file below `source_dir` to `{key_prefix}/{relative path}`. A file is sent only
/// when the bucket has no object at its key or holds different bytes; nothing is ever deleted.
pub async fn publishAssets(bucket: &Bucket, options: PublishOptions) -> Result<PublishSummary> {
    let key_prefix = normalizedPrefix(&options.key_prefix)?;
    let paths = collectFiles(&options.source_dir)?;
    if paths.is_empty() {
        bail!("no files below {}", options.source_dir.display());
    }

    let client = reqwest::Client::new();
    let concurrency = options.concurrency.max(1);
    let mut queue = paths.into_iter();
    let mut pending = JoinSet::new();
    let mut summary = PublishSummary::default();

    loop {
        while pending.len() < concurrency {
            let Some(relative) = queue.next() else {
                break;
            };
            pending.spawn(publishFile(
                client.clone(),
                bucket.clone(),
                options.source_dir.join(&relative),
                format!("{key_prefix}/{relative}"),
                options.dry_run,
            ));
        }

        let Some(joined) = pending.join_next().await else {
            break;
        };
        summary.record(joined.context("asset upload task failed")??);
    }

    summary.overwritten.sort();
    info!(
        prefix = key_prefix,
        dry_run = options.dry_run,
        files = summary.files,
        bytes = summary.bytes,
        created = summary.created,
        overwritten = summary.overwritten.len(),
        unchanged = summary.unchanged,
        "published assets"
    );
    Ok(summary)
}

/// Purges the site URLs of replaced files on every origin, logging a failure instead of failing
/// the publish. New files need no purge: nothing served them before.
pub async fn purgeOverwritten(keys: &[String], origins: &[String]) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    if origins.is_empty() {
        warn!(
            count = keys.len(),
            "replaced assets were not purged; pass --purge-origin or set ASSETS_PURGE_ORIGINS"
        );
        return Ok(());
    }

    // Best effort: the files are already published, and cached copies expire on their own.
    if let Err(error) = cache::purge_asset_urls(siteUrls(keys, origins)).await {
        warn!(
            count = keys.len(),
            error = format!("{error:#}"),
            "replaced assets were not purged"
        );
    }
    Ok(())
}

async fn publishFile(
    client: reqwest::Client,
    bucket: Bucket,
    source: PathBuf,
    key: String,
    dry_run: bool,
) -> Result<PublishedFile> {
    let bytes = tokio::fs::read(&source)
        .await
        .with_context(|| format!("failed to read {}", source.display()))?;
    let size = bytes.len() as u64;
    let change = match bucket.objectEtag(&client, &key).await? {
        None => ObjectChange::Created,
        Some(etag) if etag == format!("{:x}", md5::compute(&bytes)) => ObjectChange::Unchanged,
        Some(_) => ObjectChange::Overwritten,
    };

    match change {
        ObjectChange::Unchanged => {}
        ObjectChange::Created => debug!(key, dry_run, "new asset"),
        ObjectChange::Overwritten => info!(key, dry_run, "changed asset"),
    }
    if change != ObjectChange::Unchanged && !dry_run {
        bucket
            .putObject(&client, &key, bytes, contentType(&key))
            .await?;
    }

    Ok(PublishedFile {
        key,
        bytes: size,
        change,
    })
}

impl PublishSummary {
    fn record(&mut self, file: PublishedFile) {
        self.files += 1;
        self.bytes += file.bytes;
        match file.change {
            ObjectChange::Created => self.created += 1,
            ObjectChange::Unchanged => self.unchanged += 1,
            ObjectChange::Overwritten => self.overwritten.push(file.key),
        }
    }
}

/// The prefix without surrounding slashes. It doubles as the URL path, so it must be a plain path.
fn normalizedPrefix(prefix: &str) -> Result<String> {
    let prefix = prefix.trim_matches('/');
    let invalid = prefix.is_empty()
        || prefix
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..");
    if invalid {
        bail!("invalid key prefix: {prefix:?}");
    }
    Ok(prefix.to_string())
}

/// Every file below `root` as a sorted relative path with `/` separators. Dotfiles are tool state,
/// never site content, so they stay out.
fn collectFiles(root: &Path) -> Result<Vec<String>> {
    if !root.is_dir() {
        bail!("asset folder not found: {}", root.display());
    }

    let mut paths = Vec::new();
    collectInto(root, root, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collectInto(directory: &Path, root: &Path, paths: &mut Vec<String>) -> Result<()> {
    let entries = std::fs::read_dir(directory)
        .with_context(|| format!("failed to read {}", directory.display()))?;
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }

        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collectInto(&path, root, paths)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("{} is outside {}", path.display(), root.display()))?
            .to_str()
            .with_context(|| format!("{} is not a UTF-8 path", path.display()))?;
        paths.push(relative.replace('\\', "/"));
    }
    Ok(())
}

fn siteUrls(keys: &[String], origins: &[String]) -> Vec<String> {
    origins
        .iter()
        .flat_map(|origin| {
            let origin = origin.trim_end_matches('/');
            keys.iter()
                .map(move |key| format!("{origin}/{}", bucket::encodeKey(key)))
        })
        .collect()
}

fn contentType(path: &str) -> &'static str {
    let extension = path.rsplit('.').next().unwrap_or_default();
    match extension.to_ascii_lowercase().as_str() {
        "webp" => "image/webp",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "json" => "application/json",
        "glb" => "model/gltf-binary",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;
    use axum::extract::{Path as UrlPath, State};
    use axum::http::{header, HeaderMap, StatusCode};
    use axum::routing::put;
    use axum::Router;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Answers HEAD and PUT like a bucket, without checking signatures.
    #[derive(Clone, Default)]
    struct FakeBucket {
        objects: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        puts: Arc<Mutex<usize>>,
        /// Writes still to be refused with 429, as R2 refuses quick writes to one object.
        refused_puts: Arc<Mutex<usize>>,
    }

    impl FakeBucket {
        fn putCount(&self) -> usize {
            *self.puts.lock().unwrap()
        }

        fn objectKeys(&self) -> Vec<String> {
            let mut keys = self
                .objects
                .lock()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            keys.sort();
            keys
        }
    }

    async fn headObject(
        State(fake): State<FakeBucket>,
        UrlPath((_, key)): UrlPath<(String, String)>,
    ) -> (StatusCode, HeaderMap) {
        let mut headers = HeaderMap::new();
        let Some(bytes) = fake.objects.lock().unwrap().get(&key).cloned() else {
            return (StatusCode::NOT_FOUND, headers);
        };

        let etag = format!("\"{:x}\"", md5::compute(bytes));
        headers.insert(header::ETAG, etag.parse().unwrap());
        (StatusCode::OK, headers)
    }

    async fn putObject(
        State(fake): State<FakeBucket>,
        UrlPath((_, key)): UrlPath<(String, String)>,
        body: Bytes,
    ) -> StatusCode {
        let mut refused_puts = fake.refused_puts.lock().unwrap();
        if *refused_puts > 0 {
            *refused_puts -= 1;
            return StatusCode::TOO_MANY_REQUESTS;
        }
        drop(refused_puts);

        fake.objects.lock().unwrap().insert(key, body.to_vec());
        *fake.puts.lock().unwrap() += 1;
        StatusCode::OK
    }

    async fn startFakeBucket() -> (Bucket, FakeBucket) {
        let fake = FakeBucket::default();
        let app = Router::new()
            .route("/:bucket/*key", put(putObject).head(headObject))
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let bucket = Bucket {
            endpoint: format!("http://{address}"),
            region: "auto".into(),
            name: "assets".into(),
            access_key: "test".into(),
            secret_key: "test".into(),
        };
        (bucket, fake)
    }

    fn artworkFolder(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "assets-publish-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("textures")).unwrap();
        std::fs::write(root.join("textures/hint.webp"), b"hint").unwrap();
        std::fs::write(root.join("support-rarity.png"), b"rarity").unwrap();
        root
    }

    fn options(source_dir: &Path, dry_run: bool) -> PublishOptions {
        PublishOptions {
            source_dir: source_dir.to_path_buf(),
            key_prefix: "/game-assets/".into(),
            dry_run,
            concurrency: 4,
        }
    }

    #[tokio::test]
    async fn republishingUnchangedFilesUploadsNothing() {
        let (bucket, fake) = startFakeBucket().await;
        let folder = artworkFolder("republish");

        let first = publishAssets(&bucket, options(&folder, false))
            .await
            .unwrap();
        let second = publishAssets(&bucket, options(&folder, false))
            .await
            .unwrap();

        assert_eq!(first.created, 2);
        assert_eq!(second.unchanged, 2);
        assert_eq!(second.created + second.overwritten.len(), 0);
        assert_eq!(fake.putCount(), 2);
        assert_eq!(
            fake.objectKeys(),
            [
                "game-assets/support-rarity.png",
                "game-assets/textures/hint.webp"
            ]
        );
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[tokio::test]
    async fn changedFileIsOverwrittenAndReported() {
        let (bucket, fake) = startFakeBucket().await;
        let folder = artworkFolder("changed");
        publishAssets(&bucket, options(&folder, false))
            .await
            .unwrap();

        std::fs::write(folder.join("textures/hint.webp"), b"new hint").unwrap();
        let summary = publishAssets(&bucket, options(&folder, false))
            .await
            .unwrap();

        assert_eq!(summary.overwritten, ["game-assets/textures/hint.webp"]);
        assert_eq!(summary.unchanged, 1);
        assert_eq!(fake.putCount(), 3);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[tokio::test]
    async fn aWriteRefusedForLoadIsRetried() {
        let (bucket, fake) = startFakeBucket().await;
        *fake.refused_puts.lock().unwrap() = 1;
        let client = reqwest::Client::new();

        let written = bucket
            .putObject(&client, "game-assets/a.webp", b"a".to_vec(), "image/webp")
            .await;

        assert!(written.is_ok(), "{written:?}");
        assert_eq!(fake.objectKeys(), ["game-assets/a.webp"]);
    }

    #[tokio::test]
    async fn dryRunReportsWithoutUploading() {
        let (bucket, fake) = startFakeBucket().await;
        let folder = artworkFolder("dry-run");

        let summary = publishAssets(&bucket, options(&folder, true))
            .await
            .unwrap();

        assert_eq!(summary.created, 2);
        assert_eq!(fake.putCount(), 0);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn dotfilesStayOutOfTheUpload() {
        let folder = artworkFolder("dotfiles");
        std::fs::write(folder.join(".rsync-write-test"), b"").unwrap();
        std::fs::create_dir_all(folder.join(".cache")).unwrap();
        std::fs::write(folder.join(".cache/state"), b"").unwrap();

        let paths = collectFiles(&folder).unwrap();

        assert_eq!(paths, ["support-rarity.png", "textures/hint.webp"]);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn prefixMustBeAPlainPath() {
        assert_eq!(
            normalizedPrefix("/assets/images/").unwrap(),
            "assets/images"
        );
        for prefix in ["", "/", "a//b", "../game-assets", "game-assets/./x"] {
            assert!(normalizedPrefix(prefix).is_err(), "{prefix:?}");
        }
    }

    #[test]
    fn replacedFilesArePurgedOnEveryOrigin() {
        let keys = ["game-assets/textures/a b.webp".to_string()];
        let origins = [
            "https://uma.moe/".to_string(),
            "https://beta.uma.moe".to_string(),
        ];

        let urls = siteUrls(&keys, &origins);

        assert_eq!(
            urls,
            [
                "https://uma.moe/game-assets/textures/a%20b.webp",
                "https://beta.uma.moe/game-assets/textures/a%20b.webp"
            ]
        );
    }
}

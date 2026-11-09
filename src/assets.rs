//! Publishes the extracted game artwork.
//!
//! This mirrors the JSON pipeline: the manifest is content-addressed, one version directory holds
//! every object for that content, and a small pointer file says which version is current. The
//! difference is where the bytes go. JSON artifacts stay on disk and are served by this process;
//! artwork can be uploaded to an S3-compatible bucket (Cloudflare R2 in production) and served by
//! the CDN instead.
//!
//! Layout written under the data directory:
//!
//! ```text
//! generated-data/assets/{version}/assets.json   immutable, one per content version
//! generated-data/assets.json                    mutable pointer to the current version
//! ```
//!
//! and, when a bucket is configured, the same objects under `assets/{version}/{path}` so the URL
//! changes whenever a file's bytes do.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::task::JoinSet;
use tracing::info;

/// Object keys carry the content version, so a year-long cache never goes stale.
const IMMUTABLE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

/// Directory below the data directory that holds every published manifest version.
pub(crate) const ASSETS_PREFIX: &str = "assets";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssetFile {
    /// Path the frontend requests, relative to the artwork root (e.g. `character_thumbs/x.webp`).
    pub path: String,
    /// Object key in the bucket, already versioned.
    pub key: String,
    pub sha256: String,
    pub bytes: u64,
    pub content_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AssetManifest {
    pub version: String,
    pub generated_at: String,
    /// Prefix the frontend puts in front of `key` (a CDN custom domain), may be empty.
    pub base: String,
    pub count: usize,
    pub bytes: u64,
    pub files: Vec<AssetFile>,
}

impl AssetManifest {
    pub fn by_path(&self) -> impl Iterator<Item = (&str, &AssetFile)> {
        self.files.iter().map(|file| (file.path.as_str(), file))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AssetPointer {
    pub version: String,
    pub generated_at: String,
    pub base: String,
    pub count: usize,
    pub bytes: u64,
    /// Path of the immutable manifest below the data directory, served as a versioned resource.
    pub manifest_path: String,
}

pub struct PublishOptions {
    /// Directory holding the extracted artwork, laid out the way the frontend requests it.
    pub source_dir: PathBuf,
    pub data_dir: PathBuf,
    /// Public base URL prefixed to every key, such as `https://assets.uma.moe`.
    pub base: String,
    /// Upload the objects. Without it only the manifest is written.
    pub upload: bool,
    pub concurrency: usize,
}

#[derive(Debug)]
pub struct PublishSummary {
    pub version: String,
    pub count: usize,
    pub bytes: u64,
    pub uploaded: usize,
    pub skipped: usize,
}

#[derive(Clone)]
struct Bucket {
    endpoint: String,
    region: String,
    name: String,
    access_key: String,
    secret_key: String,
}

pub async fn publish(options: PublishOptions) -> Result<PublishSummary> {
    if !options.source_dir.is_dir() {
        bail!(
            "artwork directory not found: {}",
            options.source_dir.display()
        );
    }
    let concurrency = options.concurrency.max(1);

    let mut paths = Vec::new();
    collect_files(&options.source_dir, &options.source_dir, &mut paths)
        .with_context(|| format!("failed to walk {}", options.source_dir.display()))?;
    paths.sort();
    if paths.is_empty() {
        bail!(
            "no files below {} (nothing would be published)",
            options.source_dir.display()
        );
    }

    let mut files = Vec::with_capacity(paths.len());
    for relative in paths {
        let source = options.source_dir.join(&relative);
        let bytes = tokio::fs::read(&source)
            .await
            .with_context(|| format!("failed to read {}", source.display()))?;
        files.push(AssetFile {
            path: relative.clone(),
            key: String::new(),
            sha256: hex::encode(Sha256::digest(&bytes)),
            bytes: bytes.len() as u64,
            content_type: content_type(&relative).to_string(),
        });
    }

    let version = content_version(&files);
    for file in &mut files {
        file.key = format!("{ASSETS_PREFIX}/{version}/{}", file.path);
    }

    let manifest = AssetManifest {
        version: version.clone(),
        generated_at: Utc::now().to_rfc3339(),
        base: options.base.trim_end_matches('/').to_string(),
        count: files.len(),
        bytes: files.iter().map(|file| file.bytes).sum(),
        files,
    };

    let manifest_dir = options.data_dir.join(ASSETS_PREFIX).join(&version);
    tokio::fs::create_dir_all(&manifest_dir)
        .await
        .with_context(|| format!("failed to create {}", manifest_dir.display()))?;
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    tokio::fs::write(manifest_dir.join("assets.json"), &manifest_bytes)
        .await
        .context("failed to write the asset manifest")?;

    let pointer = AssetPointer {
        version: version.clone(),
        generated_at: manifest.generated_at.clone(),
        base: manifest.base.clone(),
        count: manifest.count,
        bytes: manifest.bytes,
        manifest_path: format!("{ASSETS_PREFIX}/{version}/assets.json"),
    };
    tokio::fs::create_dir_all(&options.data_dir).await.ok();
    tokio::fs::write(
        options.data_dir.join("assets.json"),
        serde_json::to_vec_pretty(&pointer)?,
    )
    .await
    .context("failed to write the asset pointer")?;

    info!(
        version = %version,
        files = manifest.count,
        bytes = manifest.bytes,
        manifest = %manifest_dir.join("assets.json").display(),
        "wrote the asset manifest"
    );

    let mut uploaded = 0;
    let mut skipped = 0;
    if options.upload {
        let bucket = bucket_from_env()?;
        let client = reqwest::Client::new();

        // The manifest travels with the objects so the CDN can serve it without this process.
        put_object(
            &client,
            &bucket,
            &format!("{ASSETS_PREFIX}/{version}/assets.json"),
            &manifest_bytes,
            "application/json; charset=utf-8",
        )
        .await
        .context("failed to upload the asset manifest")?;

        for chunk in manifest.files.chunks(concurrency) {
            let mut pending = JoinSet::new();
            for file in chunk {
                let client = client.clone();
                let bucket = bucket.clone();
                let source = options.source_dir.join(&file.path);
                let key = file.key.clone();
                let content_type = file.content_type.clone();
                pending.spawn(async move {
                    upload_one(&client, &bucket, &source, &key, &content_type).await
                });
            }
            while let Some(joined) = pending.join_next().await {
                match joined {
                    Ok(Ok(true)) => uploaded += 1,
                    Ok(Ok(false)) => skipped += 1,
                    Ok(Err(error)) => return Err(error),
                    Err(error) => bail!("asset upload task failed: {error}"),
                }
            }
        }
    }

    Ok(PublishSummary {
        version,
        count: manifest.count,
        bytes: manifest.bytes,
        uploaded,
        skipped,
    })
}

/// Sorts every file below `directory`, relative to `root`, so the version hash is stable.
fn collect_files(directory: &Path, root: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            collect_files(&path, root, out)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .with_context(|| format!("{} is outside {}", path.display(), root.display()))?;
            out.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

/// Hashes the sorted `path`+`sha256` pairs: identical content produces an identical version, so a
/// republish with unchanged files reuses the same keys and uploads nothing.
fn content_version(files: &[AssetFile]) -> String {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.path.as_bytes());
        hasher.update([0]);
        hasher.update(file.sha256.as_bytes());
        hasher.update(b"\n");
    }
    format!("assets-{}", &hex::encode(hasher.finalize())[..12])
}

fn content_type(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "webp" => "image/webp",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "json" => "application/json",
        "gz" => "application/gzip",
        "md" => "text/markdown; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn bucket_from_env() -> Result<Bucket> {
    let endpoint = std::env::var("ASSETS_S3_ENDPOINT").context(
        "set ASSETS_S3_ENDPOINT to upload (or pass --dry-run to only write the manifest)",
    )?;
    let name = std::env::var("ASSETS_S3_BUCKET").context("set ASSETS_S3_BUCKET to upload")?;
    Ok(Bucket {
        endpoint: endpoint.trim_end_matches('/').to_string(),
        // R2 wants "auto"; anything else is passed through.
        region: std::env::var("ASSETS_S3_REGION").unwrap_or_else(|_| "auto".to_string()),
        name,
        access_key: std::env::var("AWS_ACCESS_KEY_ID").context("set AWS_ACCESS_KEY_ID")?,
        secret_key: std::env::var("AWS_SECRET_ACCESS_KEY").context("set AWS_SECRET_ACCESS_KEY")?,
    })
}

/// Returns whether the object was written. Keys are content-addressed, so an object that already
/// exists already holds these exact bytes.
async fn upload_one(
    client: &reqwest::Client,
    bucket: &Bucket,
    source: &Path,
    key: &str,
    content_type: &str,
) -> Result<bool> {
    let request = signed_request(bucket, "HEAD", key, &[], &[])?;
    let response = client
        .request(reqwest::Method::HEAD, &request.url)
        .headers(request.headers)
        .send()
        .await
        .with_context(|| format!("HEAD {key} failed"))?;
    match response.status().as_u16() {
        200..=299 => return Ok(false),
        404 => {}
        status => bail!("HEAD {key} returned HTTP {status}"),
    }

    let bytes = tokio::fs::read(source)
        .await
        .with_context(|| format!("failed to read {}", source.display()))?;
    put_object(client, bucket, key, &bytes, content_type).await?;
    Ok(true)
}

async fn put_object(
    client: &reqwest::Client,
    bucket: &Bucket,
    key: &str,
    body: &[u8],
    content_type: &str,
) -> Result<()> {
    let request = signed_request(bucket, "PUT", key, body, &[])?;
    let response = client
        .request(reqwest::Method::PUT, &request.url)
        .headers(request.headers)
        .header("content-type", content_type)
        .header("cache-control", IMMUTABLE_CACHE_CONTROL)
        .body(body.to_vec())
        .send()
        .await
        .with_context(|| format!("PUT {key} failed"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        bail!("PUT {key} returned HTTP {status}: {detail}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// SigV4
// ---------------------------------------------------------------------------

struct SignedRequest {
    url: String,
    headers: reqwest::header::HeaderMap,
}

/// Signs the minimum set S3 requires: `host`, `x-amz-content-sha256` and `x-amz-date`. Content type
/// and cache control are not `x-amz-` headers, so they stay unsigned and are added by the caller.
fn signed_request(
    bucket: &Bucket,
    method: &str,
    key: &str,
    body: &[u8],
    extra: &[(&str, String)],
) -> Result<SignedRequest> {
    let endpoint = reqwest::Url::parse(&bucket.endpoint)
        .with_context(|| format!("invalid ASSETS_S3_ENDPOINT: {}", bucket.endpoint))?;
    let host = endpoint
        .host_str()
        .context("ASSETS_S3_ENDPOINT has no host")?
        .to_string();
    let host = match endpoint.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };

    let path = format!("/{}/{}", bucket.name, encode_key(key));
    let payload_hash = hex::encode(Sha256::digest(body));
    let amz_date = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let date = &amz_date[..8];

    let mut headers = vec![
        ("host".to_string(), host),
        ("x-amz-content-sha256".to_string(), payload_hash.clone()),
        ("x-amz-date".to_string(), amz_date.clone()),
    ];
    for (name, value) in extra {
        headers.push((name.to_lowercase(), value.clone()));
    }
    headers.sort();

    let canonical_headers = headers
        .iter()
        .map(|(name, value)| format!("{name}:{}\n", value.trim()))
        .collect::<String>();
    let signed_headers = headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical_request =
        format!("{method}\n{path}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}");

    let scope = format!("{date}/{}/s3/aws4_request", bucket.region);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );

    let mut key = hmac_sha256(
        format!("AWS4{}", bucket.secret_key).as_bytes(),
        date.as_bytes(),
    );
    key = hmac_sha256(&key, bucket.region.as_bytes());
    key = hmac_sha256(&key, b"s3");
    key = hmac_sha256(&key, b"aws4_request");
    let signature = hex::encode(hmac_sha256(&key, string_to_sign.as_bytes()));

    let mut header_map = reqwest::header::HeaderMap::new();
    for (name, value) in &headers {
        header_map.insert(
            reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .context("invalid signed header name")?,
            reqwest::header::HeaderValue::from_str(value).context("invalid signed header value")?,
        );
    }
    header_map.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(&format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
            bucket.access_key
        ))
        .context("invalid authorization header")?,
    );

    Ok(SignedRequest {
        url: format!("{}{}", bucket.endpoint, path),
        headers: header_map,
    })
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Percent-encodes everything but the unreserved set, keeping `/` so keys stay path-shaped.
fn encode_key(key: &str) -> String {
    let mut encoded = String::with_capacity(key.len());
    for byte in key.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

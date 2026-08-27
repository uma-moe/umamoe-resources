use anyhow::{bail, Context, Result};
use chrono::Utc;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
use tokio::time::sleep;

const ARCHIVE_VERSION: u8 = 1;
const PAGE_SIZE: usize = 32;
const MAX_PAGES: usize = 100;
const REQUEST_INTERVAL: Duration = Duration::from_millis(150);
const UMAPYOI_EN_NEWS_ROOT: &str = "https://umapyoi.net/news/en";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalNewsArchive {
    pub version: u8,
    pub source: String,
    #[serde(default)]
    pub posts: Vec<GlobalNewsPost>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalNewsPost {
    pub announce_id: i64,
    pub page_url: String,
    pub first_seen_at: String,
    #[serde(default)]
    pub snapshots: Vec<GlobalNewsSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalNewsSnapshot {
    pub captured_at: String,
    pub content_hash: String,
    pub raw: Value,
}

#[derive(Debug)]
pub struct SyncSummary {
    pub posts: usize,
    pub new_posts: usize,
    pub updated_posts: usize,
    pub changed: bool,
}

pub async fn sync(endpoint: &str, output_path: &Path) -> Result<SyncSummary> {
    let endpoint = endpoint.trim();
    let client = Client::builder()
        .user_agent("umamoe-resources/0.1 (+https://uma.moe)")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()?;
    let previous = load_archive(output_path)?;
    let mut posts = previous
        .posts
        .into_iter()
        .map(|mut post| {
            post.page_url = umapyoi_en_page_url(post.announce_id);
            (post.announce_id, post)
        })
        .collect::<BTreeMap<_, _>>();
    let captured_at = Utc::now().to_rfc3339();
    let mut new_posts = 0;
    let mut updated_posts = 0;

    for raw in fetch_all_posts(&client, endpoint).await? {
        let announce_id =
            announce_id(&raw).context("official EN news post is missing announce_id")?;
        let content_hash = hash_value(&raw)?;
        match posts.get_mut(&announce_id) {
            Some(post)
                if post
                    .snapshots
                    .last()
                    .is_some_and(|snapshot| snapshot.content_hash == content_hash) => {}
            Some(post) => {
                post.snapshots.push(GlobalNewsSnapshot {
                    captured_at: captured_at.clone(),
                    content_hash,
                    raw,
                });
                updated_posts += 1;
            }
            None => {
                posts.insert(
                    announce_id,
                    GlobalNewsPost {
                        announce_id,
                        page_url: umapyoi_en_page_url(announce_id),
                        first_seen_at: captured_at.clone(),
                        snapshots: vec![GlobalNewsSnapshot {
                            captured_at: captured_at.clone(),
                            content_hash,
                            raw,
                        }],
                    },
                );
                new_posts += 1;
            }
        }
    }

    let archive = GlobalNewsArchive {
        version: ARCHIVE_VERSION,
        source: endpoint.to_string(),
        posts: posts.into_values().collect(),
    };
    let changed = archive != load_archive(output_path)?;
    if changed {
        write_archive(output_path, &archive)?;
    }

    Ok(SyncSummary {
        posts: archive.posts.len(),
        new_posts,
        updated_posts,
        changed,
    })
}

fn umapyoi_en_page_url(announce_id: i64) -> String {
    format!("{UMAPYOI_EN_NEWS_ROOT}/{announce_id}")
}

async fn fetch_all_posts(client: &Client, endpoint: &str) -> Result<Vec<Value>> {
    let mut posts = Vec::new();
    let endpoint = endpoint.trim_end_matches('/');
    for page in 0..MAX_PAGES {
        let offset = page * PAGE_SIZE;
        let page_url = format!("{endpoint}/latest/{PAGE_SIZE}/{offset}");
        let payload = get_json_with_retry(client, &page_url).await?;
        let batch = payload
            .as_array()
            .with_context(|| format!("Umapyoi EN news response from {page_url} is not an array"))?;
        let batch_len = batch.len();
        posts.extend(batch.iter().cloned());
        if batch_len < PAGE_SIZE {
            return Ok(posts);
        }
        sleep(REQUEST_INTERVAL).await;
    }
    bail!("Umapyoi EN news pagination exceeded {MAX_PAGES} pages")
}

async fn get_json_with_retry(client: &Client, url: &str) -> Result<Value> {
    const MAX_ATTEMPTS: usize = 4;
    let mut delay = Duration::from_secs(2);
    for attempt in 1..=MAX_ATTEMPTS {
        let response = client.get(url).send().await;
        match response {
            Ok(response) if response.status().is_success() => {
                return response
                    .json()
                    .await
                    .with_context(|| format!("invalid JSON returned by {url}"));
            }
            Ok(response)
                if response.status() == StatusCode::TOO_MANY_REQUESTS
                    || response.status().is_server_error() =>
            {
                let status = response.status();
                if attempt == MAX_ATTEMPTS {
                    bail!("{url} returned {status} after {attempt} attempts");
                }
            }
            Ok(response) => bail!("{url} returned {}", response.status()),
            Err(error) if attempt == MAX_ATTEMPTS => {
                return Err(error).context(format!("request failed: {url}"));
            }
            Err(_) => {}
        }
        sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(30));
    }
    unreachable!()
}

fn announce_id(value: &Value) -> Option<i64> {
    value
        .get("announce_id")?
        .as_i64()
        .or_else(|| value.get("announce_id")?.as_str()?.parse().ok())
}

fn hash_value(value: &Value) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}

fn load_archive(path: &Path) -> Result<GlobalNewsArchive> {
    if !path.exists() {
        return Ok(GlobalNewsArchive {
            version: ARCHIVE_VERSION,
            source: String::new(),
            posts: Vec::new(),
        });
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read Global news archive at {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse Global news archive at {}", path.display()))
}

fn write_archive(path: &Path, archive: &GlobalNewsArchive) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(archive)?;
    bytes.push(b'\n');
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        hash_value, umapyoi_en_page_url, GlobalNewsArchive, GlobalNewsPost, GlobalNewsSnapshot,
    };
    use serde_json::json;

    #[test]
    fn snapshots_can_preserve_multiple_versions_of_a_deleted_or_corrected_post() {
        let original = json!({"announce_id": 10, "message": "Carats x 1,350"});
        let corrected = json!({"announce_id": 10, "message": "Correct Amount: 1,500 carats"});
        let archive = GlobalNewsArchive {
            version: 1,
            source: "official".to_string(),
            posts: vec![GlobalNewsPost {
                announce_id: 10,
                page_url: "https://umamusume.com/news/10/".to_string(),
                first_seen_at: "2026-01-01T00:00:00Z".to_string(),
                snapshots: vec![
                    GlobalNewsSnapshot {
                        captured_at: "2026-01-01T00:00:00Z".to_string(),
                        content_hash: hash_value(&original).unwrap(),
                        raw: original,
                    },
                    GlobalNewsSnapshot {
                        captured_at: "2026-01-02T00:00:00Z".to_string(),
                        content_hash: hash_value(&corrected).unwrap(),
                        raw: corrected,
                    },
                ],
            }],
        };

        assert_eq!(archive.posts[0].snapshots.len(), 2);
        assert_ne!(
            archive.posts[0].snapshots[0].content_hash,
            archive.posts[0].snapshots[1].content_hash
        );
    }

    #[test]
    fn english_news_pages_link_to_umapyoi_frontend() {
        assert_eq!(umapyoi_en_page_url(108), "https://umapyoi.net/news/en/108");
    }
}

use anyhow::{bail, Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;
use tokio::time::sleep;

const ARCHIVE_VERSION: u8 = 1;
const ACCOUNT: &str = "umamusume_eng";
const ARCHIVE_INDEX_LOOKBACK_DAYS: i64 = 90;
const HISTORICAL_SEED_STATUS_IDS: &[&str] = &[
    // Official posts supplied after they had already fallen out of the live
    // profile response, plus X URLs recovered from a public archival URL
    // index. Every ID is fetched and author-verified through X's own oEmbed
    // endpoint; third-party text is never stored as canonical evidence.
    "1938044706874880272",
    "1993592662512091236",
    "1994164440900751843",
    "1995621600927121752",
    "2000687486520136126",
    "2008659985736339598",
    "2024247665203302725",
    "2024248893773664540",
    "2031857857499709445",
    "2040917484069597339",
    "2040918728796516622",
    "2056500126261375333",
    "2065197488609562777",
    "2072446465872691644",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalSocialArchive {
    pub version: u8,
    pub profile_source: String,
    pub syndication_source: String,
    #[serde(default)]
    pub archive_index_source: String,
    pub oembed_source: String,
    #[serde(default)]
    pub posts: Vec<GlobalSocialPost>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalSocialPost {
    pub status_id: String,
    pub status_url: String,
    pub first_seen_at: String,
    #[serde(default)]
    pub snapshots: Vec<GlobalSocialSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalSocialSnapshot {
    pub captured_at: String,
    pub content_hash: String,
    pub raw: Value,
}

#[derive(Debug)]
pub struct SyncSummary {
    pub posts: usize,
    pub discovered_posts: usize,
    pub new_posts: usize,
    pub updated_posts: usize,
    pub changed: bool,
    pub source_errors: Vec<String>,
}

pub async fn sync(
    profile_url: &str,
    syndication_url: &str,
    archive_index_endpoint: &str,
    oembed_endpoint: &str,
    output_path: &Path,
) -> Result<SyncSummary> {
    let client = Client::builder()
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/127.0 Safari/537.36",
        )
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()?;
    let previous = load_archive(output_path)?;
    let mut posts = previous
        .posts
        .clone()
        .into_iter()
        .map(|post| (post.status_id.clone(), post))
        .collect::<BTreeMap<_, _>>();
    let mut status_ids = BTreeSet::new();
    let mut source_errors = Vec::new();

    // X refuses its logged-out profile page to datacenter IPs such as GitHub's runners and rate
    // limits the syndication history, so every discovery source is best effort. The public web
    // archive index still lists posts that have left the profile. Whichever source lists an ID,
    // the post is only stored once X's own oEmbed confirms @umamusume_eng wrote it.
    match get_text_with_retry(&client, profile_url).await {
        Ok(profile_html) => status_ids.extend(extract_status_ids(&profile_html)),
        Err(error) => {
            source_errors.push(format!("official X profile could not be read: {error:#}"))
        }
    }

    match get_text_with_retry(&client, syndication_url).await {
        Ok(timeline_html) => {
            status_ids.extend(
                extract_status_ids(&timeline_html)
                    .into_iter()
                    .filter(|status_id| !posts.contains_key(status_id)),
            );
        }
        Err(error) => source_errors.push(format!(
            "official X syndication history could not be read: {error:#}"
        )),
    }

    let index_url = archive_index_url(archive_index_endpoint, Utc::now());
    match get_text_with_retry(&client, &index_url).await {
        Ok(index) => {
            status_ids.extend(
                extract_status_ids(&index)
                    .into_iter()
                    .filter(|status_id| !posts.contains_key(status_id)),
            );
        }
        Err(error) => source_errors.push(format!(
            "web archive index of official X posts could not be read: {error:#}"
        )),
    }

    for status_id in HISTORICAL_SEED_STATUS_IDS {
        if !posts.contains_key(*status_id) {
            status_ids.insert((*status_id).to_string());
        }
    }

    let captured_at = Utc::now().to_rfc3339();
    let discovered_posts = status_ids.len();
    let mut new_posts = 0;
    let mut updated_posts = 0;
    for status_id in status_ids {
        let (status_url, raw) = match fetch_post(&client, oembed_endpoint, &status_id).await {
            Ok(post) => post,
            Err(error) => {
                // One unreadable post must not hold back the news archives committed in the same
                // job. A new post is tried again whenever a source lists it.
                let action = if posts.contains_key(&status_id) {
                    "refreshed"
                } else {
                    "archived"
                };
                source_errors.push(format!(
                    "status {status_id} could not be {action}: {error:#}"
                ));
                continue;
            }
        };
        let content_hash = hash_value(&raw)?;
        match posts.get_mut(&status_id) {
            Some(post)
                if post
                    .snapshots
                    .last()
                    .is_some_and(|snapshot| snapshot.content_hash == content_hash) => {}
            Some(post) => {
                post.snapshots.push(GlobalSocialSnapshot {
                    captured_at: captured_at.clone(),
                    content_hash,
                    raw,
                });
                updated_posts += 1;
            }
            None => {
                posts.insert(
                    status_id.clone(),
                    GlobalSocialPost {
                        status_id,
                        status_url,
                        first_seen_at: captured_at.clone(),
                        snapshots: vec![GlobalSocialSnapshot {
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

    let archive = GlobalSocialArchive {
        version: ARCHIVE_VERSION,
        profile_source: profile_url.to_string(),
        syndication_source: syndication_url.to_string(),
        archive_index_source: archive_index_endpoint.to_string(),
        oembed_source: oembed_endpoint.to_string(),
        posts: posts.into_values().collect(),
    };
    let changed = archive != previous;
    if changed {
        write_archive(output_path, &archive)?;
    }

    Ok(SyncSummary {
        posts: archive.posts.len(),
        discovered_posts,
        new_posts,
        updated_posts,
        changed,
        source_errors,
    })
}

/// Fetches one post through X's oEmbed endpoint and returns its canonical URL and raw record.
async fn fetch_post(
    client: &Client,
    oembed_endpoint: &str,
    status_id: &str,
) -> Result<(String, Value)> {
    let oembed_url = format!(
        "{oembed_endpoint}?url=https%3A%2F%2Fx.com%2F{ACCOUNT}%2Fstatus%2F{status_id}&omit_script=true"
    );
    let oembed = get_json_with_retry(client, &oembed_url).await?;
    let author_url = oembed
        .get("author_url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !is_official_author(author_url) {
        bail!("status {status_id} was not authored by @{ACCOUNT}");
    }
    let html = oembed
        .get("html")
        .and_then(Value::as_str)
        .context("official X oEmbed response is missing html")?;
    let text = extract_post_text(html)
        .with_context(|| format!("status {status_id} is missing embedded post text"))?;
    let created_at = snowflake_timestamp(status_id)
        .with_context(|| format!("invalid X status ID {status_id}"))?;
    let status_url = format!("https://x.com/{ACCOUNT}/status/{status_id}");
    let raw = json!({
        "status_id": status_id,
        "status_url": status_url,
        "created_at": created_at,
        "text": text,
        "oembed": oembed,
    });
    Ok((status_url, raw))
}

/// The oEmbed author must be the official account exactly. Anyone can get a URL into the web
/// archive index, so a look-alike such as @fake_umamusume_eng must not pass a suffix match.
fn is_official_author(author_url: &str) -> bool {
    author_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .is_some_and(|account| account.eq_ignore_ascii_case(ACCOUNT))
}

/// Internet Archive CDX query for official status URLs captured in the last
/// `ARCHIVE_INDEX_LOOKBACK_DAYS`.
fn archive_index_url(endpoint: &str, now: DateTime<Utc>) -> String {
    let since = (now - chrono::Duration::days(ARCHIVE_INDEX_LOOKBACK_DAYS)).format("%Y%m%d");
    format!(
        "{endpoint}?url=x.com/{ACCOUNT}/status/&matchType=prefix&from={since}&fl=original&collapse=urlkey"
    )
}

async fn get_text_with_retry(client: &Client, url: &str) -> Result<String> {
    Ok(get_with_retry(client, url).await?.text().await?)
}

async fn get_json_with_retry(client: &Client, url: &str) -> Result<Value> {
    get_with_retry(client, url)
        .await?
        .json()
        .await
        .with_context(|| format!("invalid JSON returned by {url}"))
}

async fn get_with_retry(client: &Client, url: &str) -> Result<reqwest::Response> {
    const MAX_ATTEMPTS: usize = 4;
    let mut delay = Duration::from_secs(2);
    for attempt in 1..=MAX_ATTEMPTS {
        match client.get(url).send().await {
            Ok(response) if response.status().is_success() => return Ok(response),
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

fn extract_status_ids(value: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for marker in ["/umamusume_eng/status/", r"\/umamusume_eng\/status\/"] {
        let mut remainder = value;
        while let Some(index) = remainder.find(marker) {
            remainder = &remainder[index + marker.len()..];
            let id = remainder
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            if (18..=20).contains(&id.len()) {
                ids.insert(id);
            }
        }
    }
    ids
}

fn extract_post_text(html: &str) -> Option<String> {
    let paragraph_start = html.find("<p")?;
    let body_start = paragraph_start + html[paragraph_start..].find('>')? + 1;
    let body_end = body_start + html[body_start..].find("</p>")?;
    let fragment = &html[body_start..body_end];
    let mut output = String::with_capacity(fragment.len());
    let mut tag = String::new();
    let mut in_tag = false;
    for character in fragment.chars() {
        if character == '<' {
            in_tag = true;
            tag.clear();
        } else if in_tag {
            if character == '>' {
                if tag.trim_start().to_ascii_lowercase().starts_with("br") {
                    output.push('\n');
                }
                in_tag = false;
            } else {
                tag.push(character);
            }
        } else {
            output.push(character);
        }
    }
    let decoded = decode_html_entities(&output);
    let normalized = decoded
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!normalized.is_empty()).then_some(normalized)
}

fn decode_html_entities(value: &str) -> String {
    value
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

fn snowflake_timestamp(status_id: &str) -> Option<String> {
    const X_EPOCH_MS: i64 = 1_288_834_974_657;
    let id = status_id.parse::<u64>().ok()?;
    let milliseconds = i64::try_from(id >> 22).ok()?.checked_add(X_EPOCH_MS)?;
    Utc.timestamp_millis_opt(milliseconds)
        .single()
        .map(|date| date.to_rfc3339())
}

fn hash_value(value: &Value) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}

fn load_archive(path: &Path) -> Result<GlobalSocialArchive> {
    if !path.exists() {
        return Ok(GlobalSocialArchive {
            version: ARCHIVE_VERSION,
            profile_source: String::new(),
            syndication_source: String::new(),
            archive_index_source: String::new(),
            oembed_source: String::new(),
            posts: Vec::new(),
        });
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read Global social archive at {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "failed to parse Global social archive at {}",
            path.display()
        )
    })
}

fn write_archive(path: &Path, archive: &GlobalSocialArchive) -> Result<()> {
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
        archive_index_url, extract_post_text, extract_status_ids, is_official_author,
        snowflake_timestamp,
    };
    use chrono::{TimeZone, Utc};

    #[test]
    fn accepts_only_the_exact_official_author() {
        assert!(is_official_author("https://twitter.com/umamusume_eng"));
        assert!(is_official_author("https://x.com/Umamusume_ENG/"));
        assert!(!is_official_author(
            "https://twitter.com/fake_umamusume_eng"
        ));
        assert!(!is_official_author("https://twitter.com/umamusume_eng_fan"));
        assert!(!is_official_author(""));
    }

    #[test]
    fn queries_the_archive_index_for_recent_official_status_urls() {
        let now = Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap();
        assert_eq!(
            archive_index_url("https://web.archive.org/cdx/search/cdx", now),
            "https://web.archive.org/cdx/search/cdx?url=x.com/umamusume_eng/status/\
             &matchType=prefix&from=20260803&fl=original&collapse=urlkey"
        );
    }

    #[test]
    fn discovers_status_ids_in_archive_index_lines() {
        let index = "https://x.com/umamusume_eng/status/2112401161940529303\n\
                     https://x.com/umamusume_eng/status/2105840693138575450/photo/1\n";
        assert_eq!(
            extract_status_ids(index).into_iter().collect::<Vec<_>>(),
            vec![
                "2105840693138575450".to_string(),
                "2112401161940529303".to_string()
            ]
        );
    }

    #[test]
    fn discovers_plain_and_json_escaped_official_status_ids() {
        let html = r#"/umamusume_eng/status/2072446465872691644
            \/umamusume_eng\/status\/2065197488609562777"#;
        assert_eq!(
            extract_status_ids(html).into_iter().collect::<Vec<_>>(),
            vec![
                "2065197488609562777".to_string(),
                "2072446465872691644".to_string()
            ]
        );
    }

    #[test]
    fn extracts_canonical_text_from_oembed_html() {
        let html = r#"<blockquote><p lang="en">We&#39;ve sent a gift!<br><br>🎁 Gift Contents:<br>- Carats ×600<a href="https://x.com/hashtag/Umamusume">#Umamusume</a></p></blockquote>"#;
        assert_eq!(
            extract_post_text(html).as_deref(),
            Some("We've sent a gift!\n🎁 Gift Contents:\n- Carats ×600#Umamusume")
        );
    }

    #[test]
    fn derives_immutable_post_time_from_status_id() {
        assert_eq!(
            snowflake_timestamp("2072446465872691644").as_deref(),
            Some("2026-07-01T22:25:16.314+00:00")
        );
    }
}

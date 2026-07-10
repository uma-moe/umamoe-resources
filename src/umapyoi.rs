use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;
use tokio::time::{sleep, Instant};

const ARCHIVE_VERSION: u8 = 1;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct UmapyoiArchive {
    pub version: u8,
    pub source: String,
    pub fetched_at: String,
    #[serde(default)]
    pub analysis: ArchiveAnalysis,
    #[serde(default)]
    pub news: Vec<NewsPost>,
    #[serde(default)]
    pub gachas: Vec<GachaBanner>,
    #[serde(default)]
    pub supports: Vec<SupportCard>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArchiveAnalysis {
    pub news_posts: usize,
    pub classified_news_posts: usize,
    pub news_posts_with_images: usize,
    pub news_posts_with_banner_candidates: usize,
    pub event_type_counts: BTreeMap<String, usize>,
    pub discovered_gacha_banner_images: usize,
    pub unique_discovered_gacha_ids: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub earliest_news_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_news_at: Option<String>,
    pub api_gacha_banners: usize,
    pub support_cards: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewsPost {
    pub post_id: i64,
    pub page_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_jp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posted_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<Value>,
    pub event_types: Vec<String>,
    pub images: Vec<SourceImage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gacha_banners: Vec<DiscoveredGachaBanner>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DiscoveredGachaBanner {
    pub gacha_id: i64,
    pub image_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GachaBanner {
    pub gacha_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_at: Option<String>,
    pub images: Vec<SourceImage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub support_card_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_support_cards: Vec<RelatedSupportCard>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SupportCard {
    pub support_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chara_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gametora_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_jp: Option<String>,
    pub images: Vec<SourceImage>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelatedSupportCard {
    pub support_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chara_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gametora_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_jp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rarity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gametora: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceImage {
    pub url: String,
    pub field_path: String,
    pub likely_banner: bool,
}

#[derive(Debug)]
pub struct SyncSummary {
    pub news_posts: usize,
    pub gacha_banners: usize,
    pub new_news_posts: usize,
    pub support_cards: usize,
    pub changed: bool,
    pub source_errors: Vec<String>,
}

pub async fn sync(
    base_url: &str,
    output_path: &Path,
    request_interval_ms: u64,
    full: bool,
    offline: bool,
) -> Result<SyncSummary> {
    let base_url = base_url.trim_end_matches('/');
    let client = Client::builder()
        .user_agent("umamoe-resources/0.1 (+https://uma.moe)")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .pool_max_idle_per_host(0)
        .build()?;
    let mut limiter = RateLimiter::new(Duration::from_millis(request_interval_ms));
    let existing = load_archive(output_path)?;
    let previous_analysis = existing.analysis.clone();
    let previous_news = existing.news.clone();
    let previous_gachas = existing.gachas.clone();
    let previous_supports = existing.supports.clone();
    let existing_news = existing
        .news
        .into_iter()
        .map(|post| (post.post_id, normalize_news(post.post_id, post.raw)))
        .collect::<BTreeMap<_, _>>();
    let existing_gachas = existing
        .gachas
        .into_iter()
        .map(|gacha| (gacha.gacha_id, normalize_gacha(gacha.gacha_id, gacha.raw)))
        .collect::<BTreeMap<_, _>>();
    let existing_supports = existing
        .supports
        .into_iter()
        .map(|support| {
            (
                support.support_id,
                normalize_support(support.support_id, support.raw),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut news = existing_news;
    let mut gachas = existing_gachas;
    let mut supports = existing_supports;
    let mut source_errors = Vec::new();

    if offline {
        merge_embedded_supports(&gachas, &mut supports);
        enrich_gachas_with_supports(&mut gachas, &supports);
        let mut news = news.into_values().collect::<Vec<_>>();
        news.sort_by_key(|post| std::cmp::Reverse(post.post_id));
        let mut gachas = gachas.into_values().collect::<Vec<_>>();
        gachas.sort_by_key(|gacha| gacha.gacha_id);
        let mut supports = supports.into_values().collect::<Vec<_>>();
        supports.sort_by_key(|support| support.support_id);
        let analysis = analyze_archive(&news, &gachas, &supports);
        let changed = news != previous_news
            || gachas != previous_gachas
            || supports != previous_supports
            || analysis != previous_analysis;
        let archive = UmapyoiArchive {
            version: ARCHIVE_VERSION,
            source: base_url.to_string(),
            fetched_at: Utc::now().to_rfc3339(),
            analysis,
            news,
            gachas,
            supports,
        };
        if changed {
            write_archive(output_path, &archive)?;
        }
        return Ok(SyncSummary {
            news_posts: archive.news.len(),
            gacha_banners: archive.gachas.len(),
            new_news_posts: 0,
            support_cards: archive.supports.len(),
            changed,
            source_errors,
        });
    }

    let mut new_news_posts = 0;
    match get_json(&client, &mut limiter, &format!("{base_url}/news")).await {
        Ok(news_index) => {
            let news_ids = collect_ids(&news_index, &["post_id", "id", "row_number"]);
            if news_ids.is_empty() {
                source_errors.push("news index returned no recognizable post IDs".to_string());
            }
            for post_id in news_ids {
                if !full && news.contains_key(&post_id) {
                    continue;
                }
                match get_json(&client, &mut limiter, &format!("{base_url}/news/{post_id}")).await {
                    Ok(raw) => {
                        news.insert(post_id, normalize_news(post_id, raw));
                        new_news_posts += 1;
                        if new_news_posts % 25 == 0 {
                            write_checkpoint(output_path, base_url, &news, &gachas, &supports)?;
                            tracing::info!(new_news_posts, "checkpointed umapyoi news crawl");
                        }
                    }
                    Err(error) => {
                        source_errors.push(format!("news {post_id}: {error:#}"));
                        tracing::warn!(post_id, error = %error, "skipping unavailable news post");
                    }
                }
            }
        }
        Err(error) => source_errors.push(format!("news: {error:#}")),
    }

    match get_json(&client, &mut limiter, &format!("{base_url}/support")).await {
        Ok(support_index) => {
            let support_ids = collect_ids(&support_index, &["support_id", "id"]);
            if support_ids.is_empty() {
                source_errors.push("support index returned no recognizable IDs".to_string());
            }
            for support_id in support_ids {
                if supports.contains_key(&support_id) && !full {
                    continue;
                }
                match get_json(
                    &client,
                    &mut limiter,
                    &format!("{base_url}/support/{support_id}"),
                )
                .await
                {
                    Ok(raw) => {
                        supports.insert(support_id, normalize_support(support_id, raw));
                    }
                    Err(error) if supports.contains_key(&support_id) => {
                        tracing::warn!(support_id, error = %error, "keeping cached support detail");
                    }
                    Err(error) => {
                        source_errors.push(format!("support {support_id}: {error:#}"));
                        break;
                    }
                }
            }
        }
        Err(error) => source_errors.push(format!("support: {error:#}")),
    }

    match get_json(&client, &mut limiter, &format!("{base_url}/gacha")).await {
        Ok(gacha_index) => {
            let gacha_ids = collect_ids(&gacha_index, &["gacha_id", "id"]);
            if gacha_ids.is_empty() {
                source_errors.push("gacha index returned no recognizable IDs".to_string());
            }
            for gacha_id in gacha_ids {
                match get_json(
                    &client,
                    &mut limiter,
                    &format!("{base_url}/gacha/{gacha_id}"),
                )
                .await
                {
                    Ok(raw) => {
                        gachas.insert(gacha_id, normalize_gacha(gacha_id, raw));
                    }
                    Err(error) if gachas.contains_key(&gacha_id) => {
                        tracing::warn!(gacha_id, error = %error, "keeping cached gacha detail");
                    }
                    Err(error) => {
                        source_errors.push(format!("gacha {gacha_id}: {error:#}"));
                        break;
                    }
                }
            }
        }
        Err(error) => source_errors.push(format!("gacha: {error:#}")),
    }

    merge_embedded_supports(&gachas, &mut supports);
    enrich_gachas_with_supports(&mut gachas, &supports);

    let mut news = news.into_values().collect::<Vec<_>>();
    news.sort_by_key(|post| std::cmp::Reverse(post.post_id));
    let mut gachas = gachas.into_values().collect::<Vec<_>>();
    gachas.sort_by_key(|gacha| gacha.gacha_id);
    let mut supports = supports.into_values().collect::<Vec<_>>();
    supports.sort_by_key(|support| support.support_id);
    let analysis = analyze_archive(&news, &gachas, &supports);
    let changed = news != previous_news
        || gachas != previous_gachas
        || supports != previous_supports
        || analysis != previous_analysis;
    let archive = UmapyoiArchive {
        version: ARCHIVE_VERSION,
        source: base_url.to_string(),
        fetched_at: Utc::now().to_rfc3339(),
        analysis,
        news,
        gachas,
        supports,
    };
    if changed {
        write_archive(output_path, &archive)?;
    }

    Ok(SyncSummary {
        news_posts: archive.news.len(),
        gacha_banners: archive.gachas.len(),
        new_news_posts,
        support_cards: archive.supports.len(),
        changed,
        source_errors,
    })
}

fn load_archive(path: &Path) -> Result<UmapyoiArchive> {
    if !path.exists() {
        return Ok(UmapyoiArchive {
            version: ARCHIVE_VERSION,
            source: String::new(),
            fetched_at: String::new(),
            analysis: ArchiveAnalysis::default(),
            news: Vec::new(),
            gachas: Vec::new(),
            supports: Vec::new(),
        });
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read umapyoi archive at {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse umapyoi archive at {}", path.display()))
}

fn write_archive(path: &Path, archive: &UmapyoiArchive) -> Result<()> {
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

fn write_checkpoint(
    path: &Path,
    base_url: &str,
    news: &BTreeMap<i64, NewsPost>,
    gachas: &BTreeMap<i64, GachaBanner>,
    supports: &BTreeMap<i64, SupportCard>,
) -> Result<()> {
    let mut news = news.values().cloned().collect::<Vec<_>>();
    news.sort_by_key(|post| std::cmp::Reverse(post.post_id));
    let mut gachas = gachas.values().cloned().collect::<Vec<_>>();
    gachas.sort_by_key(|gacha| gacha.gacha_id);
    let mut supports = supports.values().cloned().collect::<Vec<_>>();
    supports.sort_by_key(|support| support.support_id);
    write_archive(
        path,
        &UmapyoiArchive {
            version: ARCHIVE_VERSION,
            source: base_url.to_string(),
            fetched_at: Utc::now().to_rfc3339(),
            analysis: analyze_archive(&news, &gachas, &supports),
            news,
            gachas,
            supports,
        },
    )
}

fn analyze_archive(
    news: &[NewsPost],
    gachas: &[GachaBanner],
    supports: &[SupportCard],
) -> ArchiveAnalysis {
    let mut event_type_counts = BTreeMap::new();
    let mut discovered_ids = BTreeSet::new();
    let mut discovered_images = 0;
    for post in news {
        for event_type in &post.event_types {
            *event_type_counts.entry(event_type.clone()).or_default() += 1;
        }
        for banner in &post.gacha_banners {
            discovered_ids.insert(banner.gacha_id);
            discovered_images += 1;
        }
    }
    let mut dates = news
        .iter()
        .filter_map(|post| post.posted_at.clone())
        .collect::<Vec<_>>();
    dates.sort();
    ArchiveAnalysis {
        news_posts: news.len(),
        classified_news_posts: news
            .iter()
            .filter(|post| !post.event_types.is_empty())
            .count(),
        news_posts_with_images: news.iter().filter(|post| !post.images.is_empty()).count(),
        news_posts_with_banner_candidates: news
            .iter()
            .filter(|post| post.images.iter().any(|image| image.likely_banner))
            .count(),
        event_type_counts,
        discovered_gacha_banner_images: discovered_images,
        unique_discovered_gacha_ids: discovered_ids.len(),
        earliest_news_at: dates.first().cloned(),
        latest_news_at: dates.last().cloned(),
        api_gacha_banners: gachas.len(),
        support_cards: supports.len(),
    }
}

async fn get_json(client: &Client, limiter: &mut RateLimiter, url: &str) -> Result<Value> {
    const MAX_ATTEMPTS: usize = 3;
    let mut delay = Duration::from_secs(2);
    for attempt in 1..=MAX_ATTEMPTS {
        limiter.wait().await;
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
                return Err(error).context(format!("request failed: {url}"))
            }
            Err(_) => {}
        }
        sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(30));
    }
    unreachable!()
}

struct RateLimiter {
    interval: Duration,
    last_request: Option<Instant>,
}

impl RateLimiter {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_request: None,
        }
    }

    async fn wait(&mut self) {
        if let Some(last_request) = self.last_request {
            let next = last_request + self.interval;
            if next > Instant::now() {
                sleep(next - Instant::now()).await;
            }
        }
        self.last_request = Some(Instant::now());
    }
}

fn collect_ids(value: &Value, aliases: &[&str]) -> BTreeSet<i64> {
    let mut ids = BTreeSet::new();
    collect_ids_inner(value, aliases, &mut ids, true);
    ids
}

fn collect_ids_inner(value: &Value, aliases: &[&str], ids: &mut BTreeSet<i64>, root: bool) {
    match value {
        Value::Array(values) => {
            for value in values {
                if let Some(id) = value_as_i64(value) {
                    ids.insert(id);
                    continue;
                }
                collect_ids_inner(value, aliases, ids, false);
            }
        }
        Value::Object(object) => {
            for alias in aliases {
                if let Some(id) = object.get(*alias).and_then(value_as_i64) {
                    ids.insert(id);
                    break;
                }
            }
            for (key, child) in object {
                if root
                    || matches!(
                        key.as_str(),
                        "data" | "results" | "items" | "news" | "gacha"
                    )
                {
                    collect_ids_inner(child, aliases, ids, false);
                }
            }
        }
        Value::Number(_) | Value::String(_) if root => {
            if let Some(id) = value_as_i64(value) {
                ids.insert(id);
            }
        }
        _ => {}
    }
}

fn value_as_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
}

fn normalize_news(post_id: i64, raw: Value) -> NewsPost {
    let text = searchable_text(&raw);
    let mut images = collect_images(&raw);
    mark_primary_images(&mut images, &["$.article_image", "$.image"]);
    let gacha_banners = discover_gacha_banners(&images);
    NewsPost {
        post_id,
        page_url: format!("https://umapyoi.net/news/{post_id}?lang=jp"),
        title: string_field(
            &raw,
            &["title_english", "title_en", "subject_en", "subject"],
        ),
        title_jp: string_field(
            &raw,
            &["title", "title_jp", "title_ja", "subject_jp", "subject_ja"],
        ),
        posted_at: timestamp_field(&raw, &["posted_at", "post_at", "published_at", "date"]),
        updated_at: timestamp_field(&raw, &["updated_at", "update_at", "modified_at"]),
        label: field(
            &raw,
            &[
                "label_name_en",
                "announce_label",
                "label",
                "label_id",
                "category",
            ],
        )
        .cloned(),
        event_types: classify_event(&text),
        images,
        gacha_banners,
        raw,
    }
}

fn normalize_gacha(gacha_id: i64, raw: Value) -> GachaBanner {
    let card_type = string_field(&raw, &["card_type", "type"]);
    let related_support_cards = if card_type
        .as_deref()
        .is_some_and(|card_type| card_type.eq_ignore_ascii_case("support card"))
    {
        embedded_supports(&raw)
    } else {
        Vec::new()
    };
    let mut images = collect_images(&raw);
    mark_primary_images(&mut images, &["$.image_url", "$.image"]);
    GachaBanner {
        gacha_id,
        title: string_field(&raw, &["title", "name", "gacha_name"]),
        card_type,
        start_at: timestamp_field(&raw, &["start_at", "start_date", "start_time"]),
        end_at: timestamp_field(&raw, &["end_at", "end_date", "end_time"]),
        images,
        support_card_ids: related_support_cards
            .iter()
            .map(|support| support.support_id)
            .collect(),
        related_support_cards,
        raw,
    }
}

fn normalize_support(support_id: i64, raw: Value) -> SupportCard {
    SupportCard {
        support_id,
        chara_id: integer_field(&raw, &["chara_id", "character_id", "game_id"]),
        gametora_id: integer_field(&raw, &["gametora_id"])
            .or_else(|| leading_integer_field(&raw, &["gametora"])),
        title: string_field(&raw, &["title_en", "name_en", "name"]),
        title_jp: string_field(
            &raw,
            &["title", "title_jp", "title_ja", "name_jp", "name_ja"],
        ),
        images: collect_images(&raw),
        raw,
    }
}

fn enrich_gachas_with_supports(
    gachas: &mut BTreeMap<i64, GachaBanner>,
    supports: &BTreeMap<i64, SupportCard>,
) {
    for gacha in gachas.values_mut() {
        gacha.related_support_cards = gacha
            .support_card_ids
            .iter()
            .filter_map(|support_id| supports.get(support_id))
            .map(|support| RelatedSupportCard {
                support_id: support.support_id,
                chara_id: support.chara_id,
                gametora_id: support.gametora_id,
                title: support.title.clone(),
                title_jp: support.title_jp.clone(),
                rarity: string_field(&support.raw, &["rarity_string", "rarity"]),
                support_type: string_field(&support.raw, &["type"]),
                gametora: string_field(&support.raw, &["gametora"]),
            })
            .collect();
    }
}

fn merge_embedded_supports(
    gachas: &BTreeMap<i64, GachaBanner>,
    supports: &mut BTreeMap<i64, SupportCard>,
) {
    for gacha in gachas.values() {
        if !gacha
            .card_type
            .as_deref()
            .is_some_and(|card_type| card_type.eq_ignore_ascii_case("support card"))
        {
            continue;
        }
        if let Some(pickups) = field(&gacha.raw, &["pickups"]).and_then(Value::as_array) {
            for pickup in pickups {
                if let Some(support_id) =
                    field(pickup, &["support_id", "id"]).and_then(value_as_i64)
                {
                    supports
                        .entry(support_id)
                        .or_insert_with(|| normalize_support(support_id, pickup.clone()));
                }
            }
        }
    }
}

fn embedded_supports(raw: &Value) -> Vec<RelatedSupportCard> {
    field(raw, &["pickups"])
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|pickup| {
            let support_id = field(pickup, &["support_id", "id"]).and_then(value_as_i64)?;
            Some(RelatedSupportCard {
                support_id,
                chara_id: integer_field(pickup, &["chara_id", "character_id", "game_id"]),
                gametora_id: integer_field(pickup, &["gametora_id"])
                    .or_else(|| leading_integer_field(pickup, &["gametora"])),
                title: string_field(pickup, &["title_en", "name_en", "name"]),
                title_jp: string_field(pickup, &["title", "title_jp", "name_jp"]),
                rarity: string_field(pickup, &["rarity_string", "rarity"]),
                support_type: string_field(pickup, &["type"]),
                gametora: string_field(pickup, &["gametora"]),
            })
        })
        .collect()
}

fn field<'a>(value: &'a Value, aliases: &[&str]) -> Option<&'a Value> {
    let object = value.as_object()?;
    aliases.iter().find_map(|key| object.get(*key)).or_else(|| {
        ["data", "result", "item", "news", "gacha"]
            .iter()
            .find_map(|key| object.get(*key).and_then(|value| field(value, aliases)))
    })
}

fn string_field(value: &Value, aliases: &[&str]) -> Option<String> {
    field(value, aliases).and_then(|value| match value {
        Value::String(value) if !value.trim().is_empty() => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

fn integer_field(value: &Value, aliases: &[&str]) -> Option<i64> {
    field(value, aliases).and_then(value_as_i64)
}

fn leading_integer_field(value: &Value, aliases: &[&str]) -> Option<i64> {
    string_field(value, aliases).and_then(|value| {
        let digits = value
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>();
        digits.parse().ok()
    })
}

fn timestamp_field(value: &Value, aliases: &[&str]) -> Option<String> {
    let value = field(value, aliases)?;
    if let Some(timestamp) = value_as_i64(value) {
        return DateTime::from_timestamp(timestamp, 0).map(|date| date.to_rfc3339());
    }
    value.as_str().map(ToOwned::to_owned)
}

fn searchable_text(value: &Value) -> String {
    let mut strings = Vec::new();
    collect_strings(value, &mut strings);
    strings.join(" ").to_lowercase()
}

fn collect_strings<'a>(value: &'a Value, strings: &mut Vec<&'a str>) {
    match value {
        Value::String(value) => strings.push(value),
        Value::Array(values) => values
            .iter()
            .for_each(|value| collect_strings(value, strings)),
        Value::Object(object) => object
            .values()
            .for_each(|value| collect_strings(value, strings)),
        _ => {}
    }
}

fn classify_event(text: &str) -> Vec<String> {
    let families: [(&str, &[&str]); 12] = [
        ("gacha", &["gacha", "ガチャ"]),
        ("story_event", &["story event", "ストーリーイベント"]),
        (
            "champions_meeting",
            &["champions meeting", "チャンピオンズミーティング"],
        ),
        (
            "league_of_heroes",
            &["league of heroes", "リーグオブヒーローズ"],
        ),
        ("legend_race", &["legend race", "レジェンドレース"]),
        ("campaign", &["campaign", "キャンペーン"]),
        (
            "training_scenario",
            &["training scenario", "育成シナリオ", "新シナリオ"],
        ),
        ("career_event", &["育成イベント", "training event"]),
        ("race_event", &["レースイベント", "race event"]),
        ("login_bonus", &["login bonus", "ログインボーナス"]),
        ("update", &["update", "アップデート"]),
        ("maintenance", &["maintenance", "メンテナンス"]),
    ];
    families
        .iter()
        .filter(|(_, keywords)| keywords.iter().any(|keyword| text.contains(keyword)))
        .map(|(family, _)| (*family).to_string())
        .collect()
}

fn collect_images(value: &Value) -> Vec<SourceImage> {
    let mut images = BTreeSet::new();
    collect_images_inner(value, "$", &mut images);
    images.into_iter().collect()
}

fn mark_primary_images(images: &mut [SourceImage], field_paths: &[&str]) {
    for image in images {
        if field_paths.contains(&image.field_path.as_str()) {
            image.likely_banner = true;
        }
    }
}

fn discover_gacha_banners(images: &[SourceImage]) -> Vec<DiscoveredGachaBanner> {
    images
        .iter()
        .filter_map(|image| {
            let marker = "gacha_banner_";
            let start = image.url.to_lowercase().find(marker)? + marker.len();
            let digits = image.url[start..]
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>();
            let gacha_id = digits.parse().ok()?;
            Some(DiscoveredGachaBanner {
                gacha_id,
                image_url: image.url.clone(),
            })
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn collect_images_inner(value: &Value, path: &str, images: &mut BTreeSet<SourceImage>) {
    match value {
        Value::String(value) => {
            let lower_path = path.to_lowercase();
            for url in image_references(value, &lower_path) {
                let lower_url = url.to_lowercase();
                images.insert(SourceImage {
                    url,
                    field_path: path.to_string(),
                    likely_banner: lower_path.contains("banner")
                        || lower_path.contains("header")
                        || lower_path.contains("thumbnail")
                        || lower_url.contains("banner"),
                });
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                collect_images_inner(value, &format!("{path}[{index}]"), images);
            }
        }
        Value::Object(object) => collect_object_images(object, path, images),
        _ => {}
    }
}

fn collect_object_images(
    object: &Map<String, Value>,
    path: &str,
    images: &mut BTreeSet<SourceImage>,
) {
    for (key, value) in object {
        collect_images_inner(value, &format!("{path}.{key}"), images);
    }
}

fn looks_like_image(value: &str) -> bool {
    let lower = value.to_lowercase();
    [".png", ".jpg", ".jpeg", ".webp", ".gif", ".avif"]
        .iter()
        .any(|extension| lower.contains(extension))
}

fn image_references(value: &str, field_path: &str) -> BTreeSet<String> {
    let decoded = value
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    let image_field = field_path.contains("image")
        || field_path.contains("banner")
        || field_path.contains("thumb")
        || field_path.contains("header");
    let embedded_markup = decoded.contains('<')
        || decoded.contains("src=")
        || decoded.chars().any(char::is_whitespace);
    decoded
        .split(|character: char| {
            character.is_whitespace()
                || matches!(character, '\"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']')
        })
        .filter_map(|candidate| {
            let candidate = candidate
                .trim()
                .trim_start_matches("src=")
                .trim_start_matches("href=")
                .trim_end_matches([',', ';']);
            let asset_path = candidate.starts_with("http://")
                || candidate.starts_with("https://")
                || candidate.starts_with('/');
            ((looks_like_image(candidate) && (!embedded_markup || asset_path))
                || (image_field && asset_path))
                .then(|| candidate.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{classify_event, collect_ids, collect_images, discover_gacha_banners};
    use serde_json::json;

    #[test]
    fn extracts_ids_from_scalar_and_object_indexes() {
        assert_eq!(
            collect_ids(&json!([1, "2", {"post_id": 3}]), &["post_id", "id"])
                .into_iter()
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(
            collect_ids(&json!({"post_ids": [4, "5"]}), &["post_id", "id"])
                .into_iter()
                .collect::<Vec<_>>(),
            [4, 5]
        );
    }

    #[test]
    fn classifies_japanese_event_families() {
        let families = classify_event("新ストーリーイベントとピックアップガチャを開催します");
        assert_eq!(families, ["gacha", "story_event"]);
    }

    #[test]
    fn keeps_image_provenance_and_banner_hint() {
        let images = collect_images(&json!({
            "header_image": "https://cdn/a.webp",
            "body": "<img src=\"https://cdn/body.png\">"
        }));
        assert_eq!(images.len(), 2);
        assert!(images[0].likely_banner);
        assert_eq!(images[0].field_path, "$.header_image");
        assert_eq!(images[1].url, "https://cdn/body.png");
        assert!(!images[1].likely_banner);
    }

    #[test]
    fn discovers_gacha_ids_from_news_banner_urls() {
        let images = collect_images(&json!({
            "message": "<img src=\"https://cdn/Header/gacha_banner_30184.png?x=1\">"
        }));
        let banners = discover_gacha_banners(&images);
        assert_eq!(banners[0].gacha_id, 30184);
        assert_eq!(
            banners[0].image_url,
            "https://cdn/Header/gacha_banner_30184.png?x=1"
        );
    }
}

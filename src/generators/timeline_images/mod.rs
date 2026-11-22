//! Picks the source picture of every timeline banner: the official English art once Global has
//! announced it, the official Japanese art before that. It ports the selection of
//! `scripts/sync_umapyoi_timeline_images.mjs`, so the bucket shows the pictures the site showed.

mod campaigns;
mod english;
mod japanese;
mod news_images;
mod parity;

use crate::global_news::GlobalNewsArchive;
use crate::store::{self, StoreFile};
use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, NaiveDateTime};
use serde::Deserialize;
use std::collections::BTreeMap;

pub use parity::{compare_with_frontend_sources, FrontendSources, LocaleParity, ParityReport};

const RECOVERED_CAMPAIGNS_JSON: &[u8] =
    include_bytes!("../../jp_data/english_mission_campaign_assets.json");

const DAY_MS: i64 = 86_400_000;

/// The frontend's folder for each event type's own pictures, used when an event names none.
const EVENT_IMAGE_CATEGORIES: &[(&str, &str)] = &[
    ("campaign", "campaign"),
    ("champions_meeting", "champions-meeting"),
    ("factor_research", "factor-research"),
    ("league_of_heroes", "league-of-heroes"),
    ("legend_race", "legend-race"),
    ("masters_challenge", "masters-challenge"),
    ("racing_carnival", "racing-carnival"),
    ("scenario_release", "training-scenario"),
    ("strongest_team", "strongest-team"),
    ("trainer_skills_test", "trainer-skills-test"),
];

/// The fields of a `banner_timeline.json` event that decide its picture.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct TimelineEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub source: String,
    pub gacha_id: Option<i64>,
    pub gacha_ids: Vec<i64>,
    pub image: String,
    pub image_path: Option<String>,
    pub title: String,
    pub jp_release_date: String,
    pub global_release_date: String,
    pub is_confirmed: bool,
    pub umapyoi_url: Option<String>,
}

impl TimelineEvent {
    fn gacha_ids(&self) -> Vec<i64> {
        let mut ids = Vec::new();
        for id in self.gacha_id.iter().chain(&self.gacha_ids) {
            if !ids.contains(id) {
                ids.push(*id);
            }
        }
        ids
    }
}

#[derive(Debug, Default, Deserialize)]
struct TimelineDocument {
    #[serde(default)]
    events: Vec<TimelineEvent>,
}

/// A post of the Japanese news archive (`src/jp_data/umapyoi_archive.json`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct JapanesePost {
    pub title: Option<String>,
    pub posted_at: Option<String>,
    pub event_types: Vec<String>,
    pub images: Vec<PostImage>,
    pub gacha_banners: Vec<GachaBannerImage>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct PostImage {
    pub url: String,
    pub field_path: String,
    pub likely_banner: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct GachaBannerImage {
    pub gacha_id: i64,
    pub image_url: String,
}

#[derive(Debug, Default, Deserialize)]
struct JapaneseArchive {
    #[serde(default)]
    news: Vec<JapanesePost>,
}

/// An official English announcement as Umapyoi's feed gives it.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct EnglishPost {
    pub announce_id: i64,
    pub title: Option<String>,
    pub message: Option<String>,
    pub image: Option<String>,
    /// Unix seconds.
    pub post_at: Option<i64>,
}

/// An English mission campaign whose post left the feed (`english_mission_campaign_assets.json`).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RecoveredCampaign {
    pub title: String,
    pub post_at: String,
    #[serde(default)]
    pub available_until: Option<String>,
    pub image: String,
}

/// The news a plan is picked from.
#[derive(Debug, Default)]
pub struct BannerNews {
    pub japanese: Vec<JapanesePost>,
    /// Newest first, as the feed lists them.
    pub english: Vec<EnglishPost>,
    pub recovered_campaigns: Vec<RecoveredCampaign>,
}

impl BannerNews {
    /// The news archives this server last loaded from the store.
    pub fn stored() -> Result<Self> {
        let japanese: JapaneseArchive =
            serde_json::from_slice(&store::bytes(StoreFile::UmapyoiArchive))
                .context("the Umapyoi archive is not valid JSON")?;
        let global: GlobalNewsArchive =
            serde_json::from_slice(&store::bytes(StoreFile::GlobalNewsArchive))
                .context("the English news archive is not valid JSON")?;
        let recovered_campaigns = serde_json::from_slice(RECOVERED_CAMPAIGNS_JSON)
            .context("the bundled recovered campaigns are not valid JSON")?;

        Ok(Self {
            japanese: japanese.news,
            english: english_feed(&global),
            recovered_campaigns,
        })
    }
}

/// The latest snapshot of every archived post, newest first like the live feed.
fn english_feed(archive: &GlobalNewsArchive) -> Vec<EnglishPost> {
    let mut posts: Vec<EnglishPost> = archive
        .posts
        .iter()
        .filter_map(|post| post.snapshots.last())
        .filter_map(|snapshot| serde_json::from_value(snapshot.raw.clone()).ok())
        .collect();
    posts.sort_by(|left, right| {
        right
            .post_at
            .cmp(&left.post_at)
            .then(right.announce_id.cmp(&left.announce_id))
    });
    posts
}

/// Where each banner's picture comes from, by the timeline's `image_path`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BannerImagePlan {
    pub banners: BTreeMap<String, PlannedBanner>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlannedBanner {
    pub japanese: Option<String>,
    pub english: Option<EnglishSource>,
    /// Whether the site shows English art for the banner when it has any. A Champions Meeting
    /// keeps its Japanese art until it is confirmed.
    pub prefers_english: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnglishSource {
    pub url: String,
    /// Campaign posts also carry a thumbnail banner, which is preferred over their header.
    pub campaign: bool,
}

/// The plan for a `banner_timeline.json`, picked from the stored news.
pub fn plan_banner_images(timeline_json: &[u8]) -> Result<BannerImagePlan> {
    let timeline: TimelineDocument =
        serde_json::from_slice(timeline_json).context("the banner timeline is not valid JSON")?;
    Ok(plan_from_news(&timeline.events, &BannerNews::stored()?))
}

pub fn plan_from_news(events: &[TimelineEvent], news: &BannerNews) -> BannerImagePlan {
    let mut plan = BannerImagePlan::default();
    for event in events {
        if let Some(image_path) = banner_image_path(event) {
            let banner = plan.banners.entry(image_path).or_default();
            banner.prefers_english |= site_prefers_english(event);
        }
    }

    for (image_path, url) in japanese::japanese_sources(events, &news.japanese) {
        plan.banners.entry(image_path).or_default().japanese = Some(url);
    }
    for (image_path, source) in english::english_sources(events, news) {
        plan.banners.entry(image_path).or_default().english = Some(source);
    }
    plan
}

fn site_prefers_english(event: &TimelineEvent) -> bool {
    event.event_type != "champions_meeting" || event.is_confirmed
}

/// The picture an event shows. An event without one shows its type's picture for its master
/// id, as the frontend resolves it.
fn banner_image_path(event: &TimelineEvent) -> Option<String> {
    match event.image_path.as_deref() {
        Some(path) if !path.is_empty() && !path.ends_with('/') => Some(path.to_string()),
        _ => event_category_image_path(event),
    }
}

fn event_category_image_path(event: &TimelineEvent) -> Option<String> {
    let category = EVENT_IMAGE_CATEGORIES
        .iter()
        .find(|(event_type, _)| *event_type == event.event_type)
        .map(|(_, category)| *category)?;
    let master_id = last_number(&event.id).filter(|id| *id > 0)?;
    Some(format!(
        "assets/timeline-images/events/{category}/{master_id}.webp"
    ))
}

fn last_number(text: &str) -> Option<u64> {
    let end = text.rfind(|character: char| character.is_ascii_digit())? + 1;
    let start = text[..end]
        .rfind(|character: char| !character.is_ascii_digit())
        .map_or(0, |index| index + 1);
    text[start..end].parse().ok()
}

/// The two locales the site kept synced official art for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SyncedLocale {
    Japanese,
    English,
}

/// Where the site keeps a banner's synced art: `assets/images/campaign/100.webp` became
/// `assets/timeline-images/en/images/campaign/100.webp`.
pub fn frontend_overlay_path(image_path: &str, locale: SyncedLocale) -> String {
    let folder = match locale {
        SyncedLocale::Japanese => "jp",
        SyncedLocale::English => "en",
    };
    let relative = image_path.strip_prefix("assets/").unwrap_or(image_path);
    let relative = relative
        .strip_prefix("timeline-images/")
        .unwrap_or(relative);
    format!("assets/timeline-images/{folder}/{relative}")
}

/// Milliseconds since the epoch of a timestamp in any form the timeline and archives use. One
/// without an offset is UTC, as on the runner that synced the site's images.
fn instant_ms(value: &str) -> Option<i64> {
    if let Ok(instant) = DateTime::parse_from_rfc3339(value) {
        return Some(instant.timestamp_millis());
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(naive.and_utc().timestamp_millis());
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|midnight| midnight.and_utc().timestamp_millis())
}

fn days_between(left: i64, right: i64) -> f64 {
    (left - right).abs() as f64 / DAY_MS as f64
}

/// The first ten characters of a timestamp, its calendar date as written.
fn date_prefix(value: &str) -> &str {
    value.get(..10).unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(event_type: &str, id: &str, image_path: Option<&str>) -> TimelineEvent {
        TimelineEvent {
            id: id.to_string(),
            event_type: event_type.to_string(),
            image_path: image_path.map(str::to_string),
            jp_release_date: "2023-01-10T03:00:00Z".to_string(),
            global_release_date: "2025-01-10T22:00:00Z".to_string(),
            ..TimelineEvent::default()
        }
    }

    #[test]
    fn every_event_picture_is_planned_even_without_asource() {
        let events = [
            event(
                "story_event",
                "story-1",
                Some("assets/images/story/01_a.webp"),
            ),
            event("campaign", "campaign-1234", None),
            event(
                "legend_race",
                "legend-race-7",
                Some("assets/images/legend/"),
            ),
        ];

        let plan = plan_from_news(&events, &BannerNews::default());

        let paths: Vec<&str> = plan.banners.keys().map(String::as_str).collect();
        assert_eq!(
            paths,
            vec![
                "assets/images/story/01_a.webp",
                "assets/timeline-images/events/campaign/1234.webp",
                "assets/timeline-images/events/legend-race/7.webp",
            ]
        );
    }

    #[test]
    fn champions_meetings_prefer_english_art_once_confirmed() {
        let mut confirmed = event("champions_meeting", "champions-meeting-21", None);
        confirmed.is_confirmed = true;
        let predicted = event("champions_meeting", "champions-meeting-22", None);
        let campaign = event("campaign", "campaign-1234", None);

        let plan = plan_from_news(&[confirmed, predicted, campaign], &BannerNews::default());

        let prefers_english = |image_path: &str| plan.banners[image_path].prefers_english;
        assert!(prefers_english(
            "assets/timeline-images/events/champions-meeting/21.webp"
        ));
        assert!(!prefers_english(
            "assets/timeline-images/events/champions-meeting/22.webp"
        ));
        assert!(prefers_english(
            "assets/timeline-images/events/campaign/1234.webp"
        ));
    }

    #[test]
    fn synced_art_sits_in_the_locale_folder_of_the_site() {
        let legacy =
            frontend_overlay_path("assets/images/campaign/100.webp", SyncedLocale::English);
        let event = frontend_overlay_path(
            "assets/timeline-images/events/campaign/1001.webp",
            SyncedLocale::Japanese,
        );

        assert_eq!(legacy, "assets/timeline-images/en/images/campaign/100.webp");
        assert_eq!(event, "assets/timeline-images/jp/events/campaign/1001.webp");
    }

    #[test]
    fn timestamps_without_an_offset_are_utc() {
        assert_eq!(instant_ms("1970-01-02T00:00:00"), Some(DAY_MS));
        assert_eq!(instant_ms("1970-01-02T09:00:00+09:00"), Some(DAY_MS));
        assert_eq!(instant_ms("1970-01-02"), Some(DAY_MS));
        assert_eq!(instant_ms("soon"), None);
    }
}

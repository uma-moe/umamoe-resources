use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const UMAPYOI_ARCHIVE: &[u8] = include_bytes!("../jp_data/umapyoi_archive.json");
const CHARACTER_BANNERS: &[u8] = include_bytes!("../jp_data/timeline_character_banners.json");
const SUPPORT_BANNERS: &[u8] = include_bytes!("../jp_data/timeline_support_banners.json");
const PAID_BANNERS: &[u8] = include_bytes!("../jp_data/timeline_paid_banners.json");
const ALGORITHM_VERSION: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NewsTimelineKind {
    ChampionsMeeting,
    TrainingScenario,
    LeagueOfHeroes,
    MastersChallenge,
    TrainerSkillsTest,
    FactorResearch,
    StrongestTeam,
    RacingCarnival,
    UmaSanpo,
    HolidayCelebration,
}

#[derive(Debug, Clone)]
pub struct NewsTimelineEvent {
    pub key: String,
    pub kind: NewsTimelineKind,
    pub title: String,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub image_url: Option<String>,
    pub description: Option<String>,
    pub source_post_id: i64,
}

#[derive(Debug, Clone)]
pub struct CampaignTimelineMetadata {
    pub start_at: DateTime<Utc>,
    pub title: String,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub source_post_id: i64,
}

#[derive(Debug, Clone)]
pub struct LegendRaceTimelineMetadata {
    pub start_at: DateTime<Utc>,
    pub image_url: String,
    pub source_post_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdditionalGachaKind {
    Character,
    Support,
    Paid,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct AdditionalGachaBanner {
    pub gacha_id: i64,
    pub kind: AdditionalGachaKind,
    pub start_at: DateTime<Utc>,
    pub image_url: String,
    pub title: String,
    pub description: Option<String>,
    pub card_type: Option<String>,
    pub gacha_type: Option<i64>,
    pub pickup_card_ids: Vec<i64>,
    pub related_character_names: Vec<String>,
    pub related_support_names: Vec<String>,
    pub is_rerun: bool,
    pub is_scenario: bool,
}

pub fn generate() -> Result<Value> {
    let mut archive: Value = serde_json::from_slice(UMAPYOI_ARCHIVE)
        .context("failed to parse bundled umapyoi archive")?;
    let known_ids = known_timeline_gacha_ids()?;
    let candidates = additional_gacha_banners(&archive, &known_ids);
    archive
        .as_object_mut()
        .context("bundled umapyoi archive must be a JSON object")?
        .insert("additional_gacha_banners".to_string(), candidates);
    Ok(archive)
}

pub fn timeline_events() -> Result<Vec<NewsTimelineEvent>> {
    let archive: Value = serde_json::from_slice(UMAPYOI_ARCHIVE)
        .context("failed to parse bundled umapyoi archive")?;
    let mut posts = archive
        .get("news")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(news_post)
        .collect::<Vec<_>>();
    posts.sort_by_key(|post| (post.posted_at, post.post_id));

    let mut events = Vec::new();
    for kind in [
        NewsTimelineKind::ChampionsMeeting,
        NewsTimelineKind::TrainingScenario,
        NewsTimelineKind::LeagueOfHeroes,
        NewsTimelineKind::MastersChallenge,
        NewsTimelineKind::TrainerSkillsTest,
        NewsTimelineKind::FactorResearch,
        NewsTimelineKind::StrongestTeam,
        NewsTimelineKind::RacingCarnival,
        NewsTimelineKind::UmaSanpo,
        NewsTimelineKind::HolidayCelebration,
    ] {
        let family_posts = posts
            .iter()
            .filter(|post| title_matches_kind(kind, &post.title))
            .collect::<Vec<_>>();
        let mut starts = family_posts
            .iter()
            .copied()
            .filter(|post| is_start_post(kind, &post.title))
            .collect::<Vec<_>>();
        deduplicate_adjacent_starts(&mut starts);

        for (index, start) in starts.iter().enumerate() {
            let next_start = starts.get(index + 1).map(|post| post.posted_at);
            let maximum_end = start.posted_at + Duration::days(maximum_duration_days(kind));
            let end_at = family_posts
                .iter()
                .copied()
                .filter(|post| post.posted_at > start.posted_at)
                .filter(|post| next_start.is_none_or(|next| post.posted_at < next))
                .filter(|post| post.posted_at <= maximum_end)
                .filter(|post| is_end_post(kind, &post.title))
                .map(|post| post.posted_at)
                .max()
                .unwrap_or(start.posted_at + Duration::days(default_duration_days(kind)));
            events.push(NewsTimelineEvent {
                key: format!("{}-{}", kind_key(kind), start.posted_at.format("%Y-%m-%d")),
                kind,
                title: timeline_event_title(kind, &start.display_title),
                start_at: start.posted_at,
                end_at,
                image_url: start.image_url.clone(),
                description: start.description.clone(),
                source_post_id: start.post_id,
            });
        }
    }
    events.sort_by_key(|event| (event.start_at, event.key.clone()));
    Ok(events)
}

pub fn campaign_timeline_metadata() -> Result<Vec<CampaignTimelineMetadata>> {
    let archive: Value = serde_json::from_slice(UMAPYOI_ARCHIVE)
        .context("failed to parse bundled umapyoi archive")?;
    let mut campaigns = archive
        .get("news")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|post| has_event_type(post, "campaign"))
        .filter_map(news_post)
        .filter(|post| is_campaign_post(&post.title))
        .map(|post| CampaignTimelineMetadata {
            start_at: post.posted_at,
            title: standardized_campaign_title(&post.display_title),
            description: post.description,
            image_url: post.image_url,
            source_post_id: post.post_id,
        })
        .collect::<Vec<_>>();
    campaigns.sort_by_key(|campaign| (campaign.start_at, campaign.source_post_id));
    Ok(campaigns)
}

pub fn legend_race_timeline_metadata() -> Result<Vec<LegendRaceTimelineMetadata>> {
    let archive: Value = serde_json::from_slice(UMAPYOI_ARCHIVE)
        .context("failed to parse bundled umapyoi archive")?;
    let mut races = archive
        .get("news")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|post| has_event_type(post, "legend_race"))
        .filter_map(news_post)
        .filter(|post| is_legend_race_live_post(&post.title))
        .filter_map(|post| {
            Some(LegendRaceTimelineMetadata {
                start_at: post.posted_at,
                image_url: post.image_url?,
                source_post_id: post.post_id,
            })
        })
        .collect::<Vec<_>>();
    races.sort_by_key(|race| (race.start_at, race.source_post_id));
    Ok(races)
}

pub fn additional_gacha_banner_events() -> Result<Vec<AdditionalGachaBanner>> {
    let archive: Value = serde_json::from_slice(UMAPYOI_ARCHIVE)
        .context("failed to parse bundled umapyoi archive")?;
    let known_ids = known_timeline_gacha_ids()?;
    let candidates = additional_gacha_banners(&archive, &known_ids);
    Ok(candidates
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|candidate| {
            let gacha_id = candidate.get("gacha_id")?.as_i64()?;
            let start_at = DateTime::parse_from_rfc3339(candidate.get("jp_start_at")?.as_str()?)
                .ok()?
                .with_timezone(&Utc);
            Some(AdditionalGachaBanner {
                gacha_id,
                kind: match candidate.get("type")?.as_str()? {
                    "character_banner" => AdditionalGachaKind::Character,
                    "support_card_banner" => AdditionalGachaKind::Support,
                    "paid_banner" => AdditionalGachaKind::Paid,
                    _ => AdditionalGachaKind::Unknown,
                },
                start_at,
                image_url: candidate.get("image_url")?.as_str()?.to_string(),
                title: candidate
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Additional JP Gacha")
                    .to_string(),
                description: candidate
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                card_type: candidate
                    .get("card_type")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                gacha_type: candidate.get("gacha_type").and_then(Value::as_i64),
                pickup_card_ids: candidate
                    .get("pickup_card_ids")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_i64)
                    .collect(),
                related_character_names: string_array(candidate.get("related_character_names")),
                related_support_names: string_array(candidate.get("related_support_names")),
                is_rerun: candidate
                    .get("is_rerun")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                is_scenario: candidate
                    .get("is_scenario")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect())
}

pub fn version_hash() -> String {
    let mut digest = Sha256::new();
    digest.update(UMAPYOI_ARCHIVE);
    digest.update([ALGORITHM_VERSION]);
    hex::encode(digest.finalize())
}

#[derive(Debug)]
struct NewsPost {
    post_id: i64,
    posted_at: DateTime<Utc>,
    title: String,
    display_title: String,
    image_url: Option<String>,
    description: Option<String>,
}

fn news_post(value: &Value) -> Option<NewsPost> {
    let posted_at = DateTime::parse_from_rfc3339(value.get("posted_at")?.as_str()?)
        .ok()?
        .with_timezone(&Utc);
    let image_url = value
        .pointer("/raw/image")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("images")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|image| image.get("field_path").and_then(Value::as_str) == Some("$.image"))
                .filter_map(|image| image.get("url").and_then(Value::as_str))
                .find(|url| !url.contains("gacha_banner_"))
                .map(str::to_string)
        })
        .or_else(|| {
            value
                .get("images")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|image| {
                    image
                        .get("likely_banner")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
                .filter_map(|image| image.get("url").and_then(Value::as_str))
                .find(|url| !url.contains("gacha_banner_"))
                .map(str::to_string)
        });
    let description = value
        .pointer("/raw/message_english")
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
        .or_else(|| value.pointer("/raw/message").and_then(Value::as_str))
        .and_then(summarize_html);
    let display_title = value.get("title")?.as_str()?.to_string();
    Some(NewsPost {
        post_id: value.get("post_id")?.as_i64()?,
        posted_at,
        title: display_title.to_lowercase(),
        display_title,
        image_url,
        description,
    })
}

fn title_matches_kind(kind: NewsTimelineKind, title: &str) -> bool {
    match kind {
        NewsTimelineKind::ChampionsMeeting => {
            title.contains("champions meeting") || title.contains("meeting of champions")
        }
        NewsTimelineKind::TrainingScenario => title.contains("training scenario"),
        NewsTimelineKind::LeagueOfHeroes => title.contains("league of heroes"),
        NewsTimelineKind::MastersChallenge => title.contains("masters challenge"),
        NewsTimelineKind::TrainerSkillsTest => {
            title.contains("trainer skills test") || title.contains("trainer proficiency test")
        }
        NewsTimelineKind::FactorResearch => {
            title.contains("factor research")
                || title.contains("factor study")
                || title.contains("factor research of agnes")
        }
        NewsTimelineKind::StrongestTeam => title.contains("strongest team"),
        NewsTimelineKind::RacingCarnival => title.contains("racing carnival"),
        NewsTimelineKind::UmaSanpo => {
            title.contains("uma-sanpo campaign") || title.contains("uma sanpo campaign")
        }
        NewsTimelineKind::HolidayCelebration => title.contains("christmas campaign"),
    }
}

fn has_event_type(value: &Value, expected: &str) -> bool {
    value
        .get("event_types")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|event_type| event_type.as_str() == Some(expected))
}

fn is_legend_race_live_post(title: &str) -> bool {
    let names_race = title.contains("legend race")
        || title.contains("legendary race")
        || title.contains("legends race");
    let is_preview = title.contains("soon")
        || title.contains("starting")
        || title.contains("will be held")
        || title.contains("to be held");
    let is_live =
        title.contains("held") || title.contains("has begun") || title.contains("is here");
    names_race && is_live && !is_preview
}

fn is_campaign_post(title: &str) -> bool {
    title.contains("campaign")
        && !title.contains("gacha")
        && !title.contains("bug")
        && !title.contains("issue")
        && !title.contains("product")
        && !title.contains("sale")
        && !title.contains("store")
        && !is_campaign_end_notice(title)
        && !is_promotional_campaign_post(title)
}

fn is_campaign_end_notice(title: &str) -> bool {
    title.contains("end notice") || title.contains("notice of the end")
}

fn is_promotional_campaign_post(title: &str) -> bool {
    title.contains("twitter")
        || title.contains("x-follow")
        || title.contains("amazon")
        || title.contains("gift certificate")
        || title.contains("autographed")
}

fn is_start_post(kind: NewsTimelineKind, title: &str) -> bool {
    match kind {
        NewsTimelineKind::ChampionsMeeting => {
            (title.contains("champions meeting") || title.contains("meeting of champions"))
                && (title.contains("race event") || title.contains("racing event"))
                && (title.contains("has begun")
                    || title.contains("was held")
                    || title.starts_with("held ")
                    || title.contains("is here")
                    || title.ends_with('!'))
                && !title.contains("league selection")
                && !title.contains("participat")
                && !title.contains("registration")
        }
        NewsTimelineKind::TrainingScenario => {
            title.contains("training scenario")
                && (title.contains("released") || title.contains("now available"))
                && !title.contains("gacha")
        }
        NewsTimelineKind::LeagueOfHeroes => {
            title.starts_with("the \"league of heroes\"")
                && (title.contains("was held") || title.contains("has begun"))
        }
        NewsTimelineKind::MastersChallenge => {
            title.contains("event \"masters challenge\"")
                && (title.contains("underway") || title.ends_with("has begun!"))
                && !title.starts_with("notice")
        }
        NewsTimelineKind::TrainerSkillsTest => {
            title.starts_with("event \"trainer")
                && (title.contains("held!") || title.ends_with("has begun!"))
                && !title.contains("starting soon")
        }
        NewsTimelineKind::FactorResearch => {
            title.starts_with("the event \"factor")
                && (title.contains("underway")
                    || title.contains("has begun")
                    || title.contains("now being held"))
        }
        NewsTimelineKind::StrongestTeam => {
            title.contains("strongest team")
                && (title.contains("underway") || title.ends_with("has begun!"))
        }
        NewsTimelineKind::RacingCarnival => {
            title.contains("racing carnival") && title.contains("underway")
        }
        NewsTimelineKind::UmaSanpo => {
            (title.contains("uma-sanpo campaign") || title.contains("uma sanpo campaign"))
                && (title.contains("underway")
                    || title.contains("was held")
                    || title.contains("has begun"))
        }
        NewsTimelineKind::HolidayCelebration => {
            title.starts_with("christmas campaign vol.")
                || (title.starts_with("christmas campaign")
                    && (title.contains("was held") || title.contains("has begun")))
        }
    }
}

fn is_end_post(kind: NewsTimelineKind, title: &str) -> bool {
    let ended = title.contains("completed")
        || title.contains("is over")
        || title.contains("ends!")
        || title.contains("ends")
        || title.contains("end notice")
        || title.contains("notice of the end");
    match kind {
        NewsTimelineKind::ChampionsMeeting | NewsTimelineKind::TrainingScenario => false,
        NewsTimelineKind::LeagueOfHeroes => false,
        NewsTimelineKind::MastersChallenge => {
            title.contains("masters challenge")
                && (title.contains("end notice") || title.contains("notice of the end"))
        }
        _ => ended,
    }
}

fn deduplicate_adjacent_starts(starts: &mut Vec<&NewsPost>) {
    let mut deduplicated = Vec::<&NewsPost>::new();
    for start in starts.drain(..) {
        if let Some(previous) = deduplicated.last_mut() {
            if start.posted_at - previous.posted_at <= Duration::days(2) {
                *previous = start;
                continue;
            }
        }
        deduplicated.push(start);
    }
    *starts = deduplicated;
}

fn kind_key(kind: NewsTimelineKind) -> &'static str {
    match kind {
        NewsTimelineKind::ChampionsMeeting => "champions-meeting",
        NewsTimelineKind::TrainingScenario => "training-scenario",
        NewsTimelineKind::LeagueOfHeroes => "league-of-heroes",
        NewsTimelineKind::MastersChallenge => "masters-challenge",
        NewsTimelineKind::TrainerSkillsTest => "trainer-skills-test",
        NewsTimelineKind::FactorResearch => "factor-research",
        NewsTimelineKind::StrongestTeam => "strongest-team",
        NewsTimelineKind::RacingCarnival => "racing-carnival",
        NewsTimelineKind::UmaSanpo => "uma-sanpo",
        NewsTimelineKind::HolidayCelebration => "holiday-celebration",
    }
}

fn kind_title(kind: NewsTimelineKind) -> &'static str {
    match kind {
        NewsTimelineKind::ChampionsMeeting => "Champions Meeting",
        NewsTimelineKind::TrainingScenario => "Training Scenario",
        NewsTimelineKind::LeagueOfHeroes => "League of Heroes",
        NewsTimelineKind::MastersChallenge => "Masters Challenge",
        NewsTimelineKind::TrainerSkillsTest => "Trainer Skills Test",
        NewsTimelineKind::FactorResearch => "Agnes Tachyon's Factor Research",
        NewsTimelineKind::StrongestTeam => "Aim! The Strongest Team",
        NewsTimelineKind::RacingCarnival => "Racing Carnival",
        NewsTimelineKind::UmaSanpo => "Let's Go! Uma Outing!",
        NewsTimelineKind::HolidayCelebration => "Holiday Celebration",
    }
}

fn timeline_event_title(kind: NewsTimelineKind, source_title: &str) -> String {
    match kind {
        NewsTimelineKind::ChampionsMeeting => quoted_event_name(source_title)
            .map(|name| {
                strip_case_insensitive_prefix(&name, "Champions Meeting")
                    .or_else(|| strip_case_insensitive_prefix(&name, "Meeting of Champions"))
                    .unwrap_or(name.as_str())
                    .trim_matches(|character: char| {
                        character.is_whitespace() || character == ':' || character == '-'
                    })
                    .to_string()
            })
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "Champions Meeting".to_string()),
        NewsTimelineKind::TrainingScenario => quoted_event_name(source_title)
            .filter(|title| !title.to_lowercase().contains("training scenario"))
            .unwrap_or_else(|| {
                source_title
                    .trim()
                    .trim_matches('!')
                    .trim_start_matches("The ")
                    .to_string()
            }),
        NewsTimelineKind::HolidayCelebration => {
            let lower = source_title.to_ascii_lowercase();
            if lower.contains("vol. 1") || lower.contains("vol.1") {
                "Holiday Celebration Part 1".to_string()
            } else if lower.contains("vol. 2") || lower.contains("vol.2") {
                "Holiday Celebration Part 2".to_string()
            } else {
                kind_title(kind).to_string()
            }
        }
        _ => kind_title(kind).to_string(),
    }
}

fn quoted_event_name(value: &str) -> Option<String> {
    for (open, close) in [('"', '"'), ('“', '”')] {
        let Some(start) = value.find(open).map(|index| index + open.len_utf8()) else {
            continue;
        };
        let Some(end) = value[start..].find(close).map(|index| index + start) else {
            continue;
        };
        let name = value[start..end].trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

fn strip_case_insensitive_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}

fn standardized_campaign_title(value: &str) -> String {
    let mut title = value
        .trim()
        .trim_matches(|character: char| character == '"' || character == '“' || character == '”')
        .trim_end_matches('!')
        .trim()
        .to_string();
    for suffix in [" is now underway", " is underway", " now underway", " held"] {
        if title.to_lowercase().ends_with(suffix) {
            title.truncate(title.len() - suffix.len());
            title = title
                .trim()
                .trim_matches(|character: char| {
                    character == '"' || character == '“' || character == '”'
                })
                .trim_end_matches('!')
                .trim()
                .to_string();
            break;
        }
    }
    title
}

fn summarize_html(value: &str) -> Option<String> {
    let end = ["<h2", "<h3", "<figure"]
        .into_iter()
        .filter_map(|marker| value.find(marker))
        .min()
        .unwrap_or(value.len());
    let plain = html_to_text(&value[..end], " ");
    let mut summary = plain.trim().to_string();
    if summary.chars().count() > 360 {
        summary = summary.chars().take(357).collect::<String>();
        summary.push_str("...");
    }
    (!summary.is_empty()).then_some(summary)
}

fn html_to_text(value: &str, break_replacement: &str) -> String {
    let mut output = String::new();
    let mut in_tag = false;
    let mut index = 0;
    while index < value.len() {
        let rest = &value[index..];
        if rest.starts_with("<br>") || rest.starts_with("<br/>") || rest.starts_with("<br />") {
            output.push_str(break_replacement);
            index += if rest.starts_with("<br />") {
                6
            } else if rest.starts_with("<br/>") {
                5
            } else {
                4
            };
            continue;
        }
        let character = rest.chars().next().expect("non-empty remainder");
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(character),
            _ => {}
        }
        index += character.len_utf8();
    }
    let decoded = output
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    if break_replacement.contains('\n') {
        decoded
            .lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        decoded.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

fn default_duration_days(kind: NewsTimelineKind) -> i64 {
    match kind {
        NewsTimelineKind::ChampionsMeeting => 6,
        NewsTimelineKind::TrainingScenario => 1,
        NewsTimelineKind::LeagueOfHeroes => 6,
        NewsTimelineKind::MastersChallenge => 90,
        NewsTimelineKind::TrainerSkillsTest => 10,
        NewsTimelineKind::FactorResearch => 10,
        NewsTimelineKind::StrongestTeam => 8,
        NewsTimelineKind::RacingCarnival => 7,
        NewsTimelineKind::UmaSanpo => 9,
        NewsTimelineKind::HolidayCelebration => 14,
    }
}

fn maximum_duration_days(kind: NewsTimelineKind) -> i64 {
    match kind {
        NewsTimelineKind::ChampionsMeeting => 8,
        NewsTimelineKind::TrainingScenario => 2,
        NewsTimelineKind::MastersChallenge => 120,
        NewsTimelineKind::UmaSanpo => 12,
        NewsTimelineKind::HolidayCelebration => 31,
        _ => 20,
    }
}

fn known_timeline_gacha_ids() -> Result<BTreeSet<i64>> {
    let mut ids = BTreeSet::new();
    for (name, bytes) in [
        ("character", CHARACTER_BANNERS),
        ("support", SUPPORT_BANNERS),
        ("paid", PAID_BANNERS),
    ] {
        let banners: Vec<Value> = serde_json::from_slice(bytes)
            .with_context(|| format!("failed to parse bundled {name} timeline banners"))?;
        ids.extend(
            banners
                .iter()
                .filter_map(|banner| banner.get("gacha_id").and_then(Value::as_i64)),
        );
    }
    Ok(ids)
}

fn additional_gacha_banners(archive: &Value, known_ids: &BTreeSet<i64>) -> Value {
    let mut candidates = BTreeMap::<i64, Value>::new();
    let supports = archive
        .get("supports")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for post in archive
        .get("news")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let posted_at = post
            .get("posted_at")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let post_id = post.get("post_id").and_then(Value::as_i64);
        let title = post.get("title").and_then(Value::as_str);
        let title_lower = title.unwrap_or_default().to_lowercase();
        let description = post
            .pointer("/raw/message_english")
            .and_then(Value::as_str)
            .filter(|message| !message.trim().is_empty())
            .or_else(|| post.pointer("/raw/message").and_then(Value::as_str))
            .and_then(summarize_html);
        for banner in post
            .get("gacha_banners")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(gacha_id) = banner.get("gacha_id").and_then(Value::as_i64) else {
                continue;
            };
            if known_ids.contains(&gacha_id) {
                continue;
            }
            let image_url = banner
                .get("image_url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let section = gacha_section(post, gacha_id);
            let kind = inferred_gacha_kind(gacha_id, &title_lower, section.as_ref());
            let card_type = inferred_card_type(kind, section.as_ref());
            let gacha_type = inferred_gacha_type(&title_lower);
            let pickup_card_ids = if kind == "support_card_banner" {
                support_card_ids_in_post(post, supports)
            } else {
                Vec::new()
            };
            let section_names = section
                .as_ref()
                .map(|section| names_from_gacha_section(&section.body, &section.card_type))
                .unwrap_or_default();
            let related_character_names = if card_type == Some("character") {
                section_names.clone()
            } else {
                Vec::new()
            };
            let related_support_names = if card_type == Some("support") {
                section_names
            } else {
                Vec::new()
            };
            let is_scenario = title_lower.contains("training scenario")
                || title_lower.contains("scenario opening")
                || title_lower.contains("scenario kickoff");
            let candidate = json!({
                "gacha_id": gacha_id,
                "type": kind,
                "gacha_type": gacha_type,
                "jp_start_at": posted_at,
                "image_url": image_url,
                "source_post_id": post_id,
                "title": title,
                "description": description,
                "card_type": card_type,
                "pickup_card_ids": pickup_card_ids,
                "related_character_names": related_character_names,
                "related_support_names": related_support_names,
                "is_rerun": gacha_type == Some(12),
                "is_scenario": is_scenario,
                "source": "umapyoi_news"
            });
            let should_replace = candidates
                .get(&gacha_id)
                .and_then(|candidate| candidate.get("jp_start_at"))
                .and_then(Value::as_str)
                .is_none_or(|existing| posted_at > existing);
            if should_replace {
                candidates.insert(gacha_id, candidate);
            }
        }
    }
    Value::Array(candidates.into_values().collect())
}

#[derive(Debug)]
struct GachaSection {
    card_type: String,
    body: String,
}

fn gacha_section(post: &Value, gacha_id: i64) -> Option<GachaSection> {
    let message = post
        .pointer("/raw/message_english")
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
        .or_else(|| post.pointer("/raw/message").and_then(Value::as_str))?;
    let marker = format!("gacha_banner_{gacha_id}");
    let image_index = message.find(&marker)?;
    let heading_start = message[..image_index].rfind("<h2")?;
    let heading_open_end = message[heading_start..].find('>')? + heading_start + 1;
    let heading_end = message[heading_open_end..].find("</h2>")? + heading_open_end;
    let card_type =
        card_type_from_heading(&html_to_text(&message[heading_open_end..heading_end], " "))?;
    let body_start = message[image_index..]
        .find("</figure>")
        .map(|offset| image_index + offset + "</figure>".len())
        .unwrap_or(image_index + marker.len());
    let body_end = message[body_start..]
        .find("<h2")
        .map(|offset| body_start + offset)
        .unwrap_or(message.len());
    Some(GachaSection {
        card_type: card_type.to_string(),
        body: message[body_start..body_end].to_string(),
    })
}

fn card_type_from_heading(heading: &str) -> Option<&'static str> {
    let heading = heading.to_lowercase();
    if heading.contains("support") {
        Some("support")
    } else if heading.contains("umamusume")
        || heading.contains("cultivat")
        || heading.contains("trainable")
        || heading.contains("nurturing")
    {
        Some("character")
    } else {
        None
    }
}

fn inferred_gacha_kind(gacha_id: i64, title: &str, section: Option<&GachaSection>) -> &'static str {
    if let Some(section) = section {
        return match section.card_type.as_str() {
            "character" if gacha_id < 50_000 => "character_banner",
            "support" if gacha_id < 50_000 => "support_card_banner",
            _ if gacha_id >= 50_000 => "paid_banner",
            _ => "unknown",
        };
    }
    match gacha_id {
        _ if title.contains("twinkle collection") => "character_banner",
        30_000..=39_999 if gacha_id % 2 == 0 => "character_banner",
        30_000..=39_999 => "support_card_banner",
        50_000..=59_999 => "paid_banner",
        _ if title.contains("support card") => "support_card_banner",
        _ if title.contains("twinkle collection") || title.contains("pretty derby gacha") => {
            "character_banner"
        }
        _ => "unknown",
    }
}

fn inferred_card_type(kind: &str, section: Option<&GachaSection>) -> Option<&'static str> {
    match kind {
        "character_banner" => Some("character"),
        "support_card_banner" => Some("support"),
        "paid_banner" => section.and_then(|section| match section.card_type.as_str() {
            "character" => Some("character"),
            "support" => Some("support"),
            _ => None,
        }),
        _ => None,
    }
}

fn names_from_gacha_section(body: &str, card_type: &str) -> Vec<String> {
    let text = html_to_text(body, "\n");
    let mut names = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches(['・', '■', '●', ' ']).trim();
        let is_candidate = if card_type == "character" {
            line.starts_with('★') || line.starts_with('☆')
        } else {
            ["SSR ", "SR ", "R "]
                .into_iter()
                .any(|rarity| line.starts_with(rarity))
        };
        if !is_candidate {
            continue;
        }
        let without_rarity = line
            .trim_start_matches(['★', '☆'])
            .trim_start_matches("SSR")
            .trim_start_matches("SR")
            .trim_start_matches('R')
            .trim();
        let name = without_rarity
            .rfind(']')
            .map(|index| without_rarity[index + 1..].trim())
            .filter(|name| !name.is_empty())
            .unwrap_or(without_rarity)
            .trim_matches(|character: char| character == '・' || character.is_whitespace());
        if !name.is_empty() && !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
        }
    }
    names
}

fn inferred_gacha_type(title: &str) -> Option<i64> {
    if title.contains("select pickup stamp sheet") {
        Some(15)
    } else if title.contains("select step-up") || title.contains("select step up") {
        Some(14)
    } else if title.contains("select pickup") {
        Some(12)
    } else if title.contains("twinkle collection") {
        Some(11)
    } else if title.contains("group select") {
        Some(10)
    } else if title.contains("guaranteed") || title.contains("fixed") {
        Some(5)
    } else {
        None
    }
}

fn support_card_ids_in_post(post: &Value, supports: &[Value]) -> Vec<i64> {
    let text = searchable_post_text(post);
    let mut ids = supports
        .iter()
        .filter_map(|support| {
            let support_id = support.get("support_id")?.as_i64()?;
            let matched = [
                support.get("title").and_then(Value::as_str),
                support.get("title_jp").and_then(Value::as_str),
                support.pointer("/raw/title").and_then(Value::as_str),
                support.pointer("/raw/title_en").and_then(Value::as_str),
            ]
            .into_iter()
            .flatten()
            .map(str::trim)
            .filter(|title| title.chars().count() >= 4)
            .any(|title| text.contains(&title.to_lowercase()));
            matched.then_some(support_id)
        })
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn searchable_post_text(value: &Value) -> String {
    fn collect(value: &Value, output: &mut String) {
        match value {
            Value::String(value) => {
                output.push(' ');
                output.push_str(value);
            }
            Value::Array(values) => values.iter().for_each(|value| collect(value, output)),
            Value::Object(object) => object.values().for_each(|value| collect(value, output)),
            _ => {}
        }
    }
    let mut text = String::new();
    collect(value, &mut text);
    text.to_lowercase()
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{AdditionalGachaKind, NewsTimelineKind};

    #[test]
    fn bundled_archive_is_valid_json() {
        let archive = super::generate().expect("bundled umapyoi archive should parse");
        assert!(archive
            .get("news")
            .and_then(|value| value.as_array())
            .is_some());
        assert!(archive
            .get("additional_gacha_banners")
            .and_then(|value| value.as_array())
            .is_some_and(|banners| !banners.is_empty()));
        assert!(archive
            .get("gachas")
            .and_then(|value| value.as_array())
            .is_some());
        assert!(archive
            .get("supports")
            .and_then(|value| value.as_array())
            .is_some());
    }

    #[test]
    fn extracts_only_reliable_recurring_timeline_families() {
        let events = super::timeline_events().expect("news timeline events should extract");
        assert!(
            events
                .iter()
                .filter(|event| event.kind == NewsTimelineKind::LeagueOfHeroes)
                .count()
                >= 10
        );
        assert!(
            events
                .iter()
                .filter(|event| event.kind == NewsTimelineKind::MastersChallenge)
                .count()
                >= 5
        );
        assert!(events.iter().all(|event| event.end_at > event.start_at));
        assert!(
            events
                .iter()
                .filter(|event| event.kind == NewsTimelineKind::ChampionsMeeting)
                .count()
                >= 25
        );
        assert!(
            events
                .iter()
                .filter(|event| event.kind == NewsTimelineKind::TrainingScenario)
                .count()
                >= 10
        );
    }

    #[test]
    fn extracts_player_facing_campaign_families() {
        let events = super::timeline_events().expect("news timeline events should extract");
        let uma_sanpo = events
            .iter()
            .find(|event| event.key == "uma-sanpo-2022-10-19")
            .expect("the first Uma Sanpo campaign should be present");
        assert_eq!(uma_sanpo.kind, NewsTimelineKind::UmaSanpo);
        assert_eq!(uma_sanpo.title, "Let's Go! Uma Outing!");
        assert_eq!(uma_sanpo.source_post_id, 1010);
        assert_eq!((uma_sanpo.end_at - uma_sanpo.start_at).num_days(), 9);

        let holiday = events
            .iter()
            .find(|event| event.key == "holiday-celebration-2022-11-28")
            .expect("Holiday Celebration Part 1 should be present");
        assert_eq!(holiday.kind, NewsTimelineKind::HolidayCelebration);
        assert_eq!(holiday.title, "Holiday Celebration Part 1");
        assert_eq!(holiday.source_post_id, 1079);
        assert_eq!((holiday.end_at - holiday.start_at).num_days(), 14);
    }

    #[test]
    fn exposes_additional_banner_candidates_for_timeline_merge() {
        let banners = super::additional_gacha_banner_events()
            .expect("additional gacha candidates should extract");
        assert!(banners.len() >= 40);
    }

    #[test]
    fn uses_primary_event_thumbnails_and_concise_titles() {
        let events = super::timeline_events().expect("news timeline events should extract");
        let champions = events
            .iter()
            .find(|event| event.key == "champions-meeting-2026-06-23")
            .expect("latest CLASSIC champions meeting should be present");
        assert_eq!(champions.title, "CLASSIC");
        assert!(champions
            .image_url
            .as_deref()
            .is_some_and(|url| url.contains("/Thumbnail/banner_30310626.png")));

        let scenario = events
            .iter()
            .find(|event| event.key == "training-scenario-2026-06-29")
            .expect("latest training scenario should be present");
        assert_eq!(
            scenario.title,
            "Welcome to the Training Center! ~Paying It Forward Has Begun~"
        );
        assert!(scenario.description.is_some());
    }

    #[test]
    fn splits_combined_character_and_support_gacha_content() {
        let banners = super::additional_gacha_banner_events()
            .expect("additional gacha candidates should extract");
        let character = banners
            .iter()
            .find(|banner| banner.gacha_id == 30_448)
            .expect("scenario character banner should be extracted");
        assert_eq!(character.kind, AdditionalGachaKind::Character);
        assert_eq!(character.related_character_names, ["Narita Top Road"]);

        let support = banners
            .iter()
            .find(|banner| banner.gacha_id == 30_449)
            .expect("scenario support banner should be extracted");
        assert_eq!(support.kind, AdditionalGachaKind::Support);
        assert_eq!(support.pickup_card_ids, [30_304, 30_305]);
        assert_eq!(support.related_support_names.len(), 2);
    }

    #[test]
    fn extracts_campaign_banner_content() {
        let campaigns = super::campaign_timeline_metadata()
            .expect("campaign metadata should extract from news");
        let gi_campaign = campaigns
            .iter()
            .find(|campaign| campaign.source_post_id == 823)
            .expect("2022 GI campaign should be present");
        assert_eq!(gi_campaign.title, "GI Campaign");
        assert!(gi_campaign
            .image_url
            .as_deref()
            .is_some_and(|url| url.contains("/Thumbnail/banner_25000001.png")));
        assert!(gi_campaign.description.is_some());

        for promotional_post_id in [622, 1202, 1470] {
            assert!(
                campaigns
                    .iter()
                    .all(|campaign| campaign.source_post_id != promotional_post_id),
                "promotional news post {promotional_post_id} must stay out of mission metadata"
            );
        }
        assert!(campaigns
            .iter()
            .any(|campaign| campaign.source_post_id == 1194));
        assert!(campaigns
            .iter()
            .any(|campaign| campaign.source_post_id == 1474));
        for in_game_reward_post_id in [1018, 1133, 1378, 1692, 1712] {
            assert!(
                campaigns
                    .iter()
                    .any(|campaign| campaign.source_post_id == in_game_reward_post_id),
                "in-game reward post {in_game_reward_post_id} should remain mission metadata"
            );
        }
    }

    #[test]
    fn legend_race_titles_distinguish_live_posts_from_previews() {
        for title in [
            "Legend Race held!",
            "The Legendary Race has begun!",
            "The Legends Race is here!",
        ] {
            assert!(super::is_legend_race_live_post(&title.to_lowercase()));
        }
        for title in [
            "Legend Race coming soon!",
            "Legend Race starting tomorrow",
            "Legend Race will be held!",
            "Legend Race to be held next week",
            "PakaLive information on the next Legend Race",
        ] {
            assert!(!super::is_legend_race_live_post(&title.to_lowercase()));
        }
    }

    #[test]
    fn extracts_only_event_specific_legend_race_banners() {
        let races = super::legend_race_timeline_metadata()
            .expect("legend race metadata should extract from live news posts");
        assert!(races.len() >= 43);
        assert!(races
            .iter()
            .all(|race| race.image_url.contains("/Thumbnail/")));
        assert!(races
            .windows(2)
            .all(|pair| pair[0].start_at.date_naive() != pair[1].start_at.date_naive()));
    }

    #[test]
    fn extracts_type_12_select_pickup_rerun_metadata() {
        let banners = super::additional_gacha_banner_events()
            .expect("additional gacha candidates should extract");
        let banner = banners
            .iter()
            .find(|banner| banner.gacha_id == 30_197)
            .expect("gacha 30197 should be extracted from news 1558/1559");
        assert_eq!(banner.kind, AdditionalGachaKind::Support);
        assert_eq!(banner.gacha_type, Some(12));
        assert!(banner.is_rerun);
        assert_eq!(banner.pickup_card_ids.len(), 10);
    }

    #[test]
    fn classifies_twinkle_collection_as_character_gacha() {
        assert_eq!(
            super::inferred_gacha_kind(50_049, "twinkle collection pretty derby gacha", None),
            "character_banner"
        );
    }
}

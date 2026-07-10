use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const UMAPYOI_ARCHIVE: &[u8] = include_bytes!("../jp_data/umapyoi_archive.json");
const CHARACTER_BANNERS: &[u8] = include_bytes!("../jp_data/timeline_character_banners.json");
const SUPPORT_BANNERS: &[u8] = include_bytes!("../jp_data/timeline_support_banners.json");
const PAID_BANNERS: &[u8] = include_bytes!("../jp_data/timeline_paid_banners.json");
const ALGORITHM_VERSION: u8 = 3;

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
}

#[derive(Debug, Clone)]
pub struct NewsTimelineEvent {
    pub key: String,
    pub kind: NewsTimelineKind,
    pub title: String,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub image_url: Option<String>,
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
    pub source_post_id: i64,
    pub title: String,
    pub gacha_type: Option<i64>,
    pub pickup_card_ids: Vec<i64>,
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
                source_post_id: start.post_id,
            });
        }
    }
    events.sort_by_key(|event| (event.start_at, event.key.clone()));
    Ok(events)
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
                source_post_id: candidate.get("source_post_id")?.as_i64()?,
                title: candidate
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Additional JP Gacha")
                    .to_string(),
                gacha_type: candidate.get("gacha_type").and_then(Value::as_i64),
                pickup_card_ids: candidate
                    .get("pickup_card_ids")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_i64)
                    .collect(),
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
}

fn news_post(value: &Value) -> Option<NewsPost> {
    let posted_at = DateTime::parse_from_rfc3339(value.get("posted_at")?.as_str()?)
        .ok()?
        .with_timezone(&Utc);
    let image_url = value
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
        .map(str::to_string);
    let display_title = value.get("title")?.as_str()?.to_string();
    Some(NewsPost {
        post_id: value.get("post_id")?.as_i64()?,
        posted_at,
        title: display_title.to_lowercase(),
        display_title,
        image_url,
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
    }
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
    }
}

fn timeline_event_title(kind: NewsTimelineKind, source_title: &str) -> String {
    match kind {
        NewsTimelineKind::ChampionsMeeting | NewsTimelineKind::TrainingScenario => {
            source_title.trim().trim_matches('!').to_string()
        }
        _ => kind_title(kind).to_string(),
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
    }
}

fn maximum_duration_days(kind: NewsTimelineKind) -> i64 {
    match kind {
        NewsTimelineKind::ChampionsMeeting => 8,
        NewsTimelineKind::TrainingScenario => 2,
        NewsTimelineKind::MastersChallenge => 120,
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
            let kind = inferred_gacha_kind(gacha_id, &title_lower);
            let gacha_type = inferred_gacha_type(&title_lower);
            let pickup_card_ids = if kind == "support_card_banner" {
                support_card_ids_in_post(post, supports)
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
                "pickup_card_ids": pickup_card_ids,
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

fn inferred_gacha_kind(gacha_id: i64, title: &str) -> &'static str {
    if title.contains("support card") {
        return "support_card_banner";
    }
    if title.contains("twinkle collection") || title.contains("pretty derby gacha") {
        return "character_banner";
    }
    match gacha_id {
        30_000..=39_999 if gacha_id % 2 == 0 => "character_banner",
        30_000..=39_999 => "support_card_banner",
        50_000..=59_999 => "paid_banner",
        _ => "unknown",
    }
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
    fn exposes_additional_banner_candidates_for_timeline_merge() {
        let banners = super::additional_gacha_banner_events()
            .expect("additional gacha candidates should extract");
        assert!(banners.len() >= 40);
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
        assert_eq!(banner.source_post_id, 1559);
        assert_eq!(banner.pickup_card_ids.len(), 10);
    }

    #[test]
    fn classifies_twinkle_collection_as_character_gacha() {
        assert_eq!(
            super::inferred_gacha_kind(50_049, "twinkle collection pretty derby gacha"),
            "character_banner"
        );
    }
}

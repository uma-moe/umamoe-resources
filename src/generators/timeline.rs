use crate::generators::banners::{CharacterBanner, PaidBanner, SupportBanner};
use crate::generators::common;
use crate::generators::jp_events::{
    AdditionalGachaBanner, AdditionalGachaKind, CampaignTimelineMetadata,
    LegendRaceTimelineMetadata, NewsTimelineEvent, NewsTimelineKind,
};
use anyhow::{Context, Result};
use chrono::{
    DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, Offset, TimeZone, Utc,
};
use chrono_tz::Europe::Berlin;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::{env, fs, path::Path};

const BUNDLED_TIMELINE_CHARACTER_BANNERS_JSON: &[u8] =
    include_bytes!("../jp_data/timeline_character_banners.json");
const BUNDLED_TIMELINE_SUPPORT_BANNERS_JSON: &[u8] =
    include_bytes!("../jp_data/timeline_support_banners.json");
const BUNDLED_TIMELINE_PAID_BANNERS_JSON: &[u8] =
    include_bytes!("../jp_data/timeline_paid_banners.json");
const BUNDLED_TIMELINE_STORY_EVENTS_JSON: &[u8] =
    include_bytes!("../jp_data/timeline_story_events.json");
const BUNDLED_TIMELINE_CHAMPIONS_MEETING_JSON: &[u8] =
    include_bytes!("../jp_data/timeline_champions_meeting.json");
const BUNDLED_TIMELINE_LEGEND_RACES_JSON: &[u8] =
    include_bytes!("../jp_data/timeline_legend_races.json");
const BUNDLED_TIMELINE_CAMPAIGNS_JSON: &[u8] = include_bytes!("../jp_data/timeline_campaigns.json");
const BUNDLED_JP_SUPPORT_CARDS_DB_JSON: &[u8] = include_bytes!("../jp_data/support-cards-db.json");
const BUNDLED_UMAPYOI_ARCHIVE_JSON: &[u8] = include_bytes!("../jp_data/umapyoi_archive.json");
const CONFIRMED_GLOBAL_BANNER_DATES_CSV: &str =
    include_str!("../jp_data/confirmed_global_banner_dates.csv");
const SUPPORT_CARD_NAME_CATEGORY: i64 = 77;

const JP_LAUNCH_YEAR: i32 = 2021;
const JP_LAUNCH_MONTH: u32 = 2;
const JP_LAUNCH_DAY: u32 = 24;
const GLOBAL_LAUNCH_YEAR: i32 = 2025;
const GLOBAL_LAUNCH_MONTH: u32 = 6;
const GLOBAL_LAUNCH_DAY: u32 = 26;
const FALLBACK_ACCELERATION_RATE: f64 = 1.6;
const MIN_ACCELERATION_RATE: f64 = 1.2;
const MAX_ACCELERATION_RATE: f64 = 2.5;
const RECENT_ANCHOR_WINDOW_DAYS: i64 = 120;
const FALLBACK_RECENT_ANCHORS: usize = 18;
const GROUPING_JP_WINDOW_DAYS: i64 = 3;
const FAMILY_ADJUSTMENT_SAMPLE_LIMIT: usize = 6;
const TIMELINE_ALGORITHM_VERSION: u8 = 28;
const LEGEND_RACE_FALLBACK_IMAGE_URL: &str =
    "https://gametora.com/images/umamusume/events/2022/03_legend_race.png";
const LEGEND_RACE_FALLBACK_IMAGE_PATH: &str =
    "assets/timeline-images/events/legend-race/fallback.webp";

#[derive(Debug, Clone, Serialize)]
pub struct BannerTimeline {
    pub version: u8,
    pub calculation: TimelineCalculation,
    pub anniversaries: Vec<TimelineAnniversary>,
    pub events: Vec<BannerTimelineEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineCalculation {
    pub jp_launch_date: DateTime<Utc>,
    pub global_launch_date: DateTime<Utc>,
    pub fallback_acceleration_rate: f64,
    pub observed_acceleration_rate: f64,
    pub confirmed_anchor_count: usize,
    pub character_banner_month_count_likelihoods: Vec<CountLikelihood>,
    pub character_banner_gap_likelihoods: Vec<CountLikelihood>,
    pub character_banner_weekday_likelihoods: Vec<NamedLikelihood>,
    pub character_banner_month_day_likelihoods: Vec<CountLikelihood>,
    pub event_type_calendar_likelihoods: Vec<EventTypeCalendarLikelihood>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_closed_global_month: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unconfirmed_schedule_floor: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_confirmed_jp_date: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_confirmed_global_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BannerTimelineEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: BannerTimelineEventType,
    pub source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_id: Option<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gacha_ids: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_type: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_type_name: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    pub image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_path: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub jp_release_date: DateTime<Utc>,
    pub global_release_date: DateTime<Utc>,
    pub estimated_end_date: DateTime<Utc>,
    pub is_confirmed: bool,
    pub banner_duration_days: i64,
    pub tags: Vec<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pickup_card_ids: Vec<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related_characters: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related_support_cards: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related_support_card_names: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gametora_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub umapyoi_url: Option<String>,
    pub prediction: PredictionInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BannerTimelineEventType {
    CharacterBanner,
    SupportCardBanner,
    PaidBanner,
    StoryEvent,
    ChampionsMeeting,
    LegendRace,
    Campaign,
    LeagueOfHeroes,
    MastersChallenge,
    TrainerSkillsTest,
    FactorResearch,
    StrongestTeam,
    RacingCarnival,
    ScenarioRelease,
}

fn timeline_event_types() -> [BannerTimelineEventType; 14] {
    [
        BannerTimelineEventType::CharacterBanner,
        BannerTimelineEventType::SupportCardBanner,
        BannerTimelineEventType::PaidBanner,
        BannerTimelineEventType::StoryEvent,
        BannerTimelineEventType::ChampionsMeeting,
        BannerTimelineEventType::LegendRace,
        BannerTimelineEventType::Campaign,
        BannerTimelineEventType::LeagueOfHeroes,
        BannerTimelineEventType::MastersChallenge,
        BannerTimelineEventType::TrainerSkillsTest,
        BannerTimelineEventType::FactorResearch,
        BannerTimelineEventType::StrongestTeam,
        BannerTimelineEventType::RacingCarnival,
        BannerTimelineEventType::ScenarioRelease,
    ]
}

#[derive(Debug, Clone, Serialize)]
pub struct PredictionInfo {
    pub kind: PredictionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acceleration_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_adjustment_days: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_likelihood: Option<CalendarLikelihood>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_jp_date: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_global_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PredictionKind {
    Confirmed,
    Interpolated,
    Extrapolated,
    Fallback,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalendarLikelihood {
    pub month_character_banner_count: usize,
    pub month_character_banner_count_probability: f64,
    pub weekday: String,
    pub weekday_probability: f64,
    pub day_of_month: u32,
    pub day_of_month_probability: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_character_gap_days: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_character_gap_probability: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_character_gap_days: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_character_gap_probability: Option<f64>,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CountLikelihood {
    pub value: usize,
    pub samples: usize,
    pub probability: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamedLikelihood {
    pub value: String,
    pub samples: usize,
    pub probability: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventTypeCalendarLikelihood {
    #[serde(rename = "type")]
    pub event_type: BannerTimelineEventType,
    pub samples: usize,
    pub weekday_likelihoods: Vec<NamedLikelihood>,
    pub month_day_likelihoods: Vec<CountLikelihood>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineAnniversary {
    pub index: u32,
    pub label: String,
    pub jp_date: DateTime<Utc>,
    pub global_date: DateTime<Utc>,
    pub is_confirmed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_adjustment_days: Option<i64>,
}

#[derive(Debug, Clone)]
struct CalibrationAnchor {
    jp: DateTime<Utc>,
    global: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct ConfirmedDateLookup {
    character: BTreeMap<i64, DateTime<Utc>>,
    support: BTreeMap<i64, DateTime<Utc>>,
    paid: BTreeMap<i64, DateTime<Utc>>,
    story: BTreeMap<String, DateTime<Utc>>,
    champions: BTreeMap<String, DateTime<Utc>>,
    legend: BTreeMap<String, DateTime<Utc>>,
    campaign: BTreeMap<String, DateTime<Utc>>,
    news_events: BTreeMap<String, DateTime<Utc>>,
    anniversary: BTreeMap<u32, DateTime<Utc>>,
    closed_global_months: BTreeSet<(i32, u32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfirmedTimelineKind {
    Character,
    Support,
    Paid,
    Story,
    Champions,
    Legend,
    Campaign,
    LeagueOfHeroes,
    MastersChallenge,
    TrainerSkillsTest,
    FactorResearch,
    StrongestTeam,
    RacingCarnival,
    TrainingScenario,
    Anniversary,
}

#[derive(Debug, Clone)]
struct ConfirmedTimelineDate {
    kind: ConfirmedTimelineKind,
    key: String,
    global_date: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct TimelineCharacterBanner {
    gacha_id: i64,
    year: i32,
    image: String,
    image_path: String,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    pickup_card_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
struct TimelineSupportBanner {
    gacha_id: i64,
    year: i32,
    image: String,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    pickup_card_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
struct TimelinePaidBanner {
    gacha_id: i64,
    gacha_type: i64,
    card_type: String,
    year: i32,
    image: String,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    pickup_card_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
struct TimelineStoryEvent {
    master_event_id: Option<i64>,
    event_name: String,
    image: String,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct TimelineChampionsMeeting {
    index: usize,
    name: String,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    track: Option<String>,
    distance: Option<String>,
    conditions: Option<String>,
    image_url: Option<String>,
    image_path: Option<String>,
}

#[derive(Debug, Clone)]
struct TimelineLegendRace {
    index: usize,
    race_name: String,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    course: Option<String>,
    bosses: Vec<TimelineLegendBoss>,
    image_url: Option<String>,
    image_path: Option<String>,
    source_post_id: Option<i64>,
}

#[derive(Debug, Clone)]
struct TimelineLegendBoss {
    name: String,
    image: String,
    card_id: Option<i64>,
}

#[derive(Debug, Clone)]
struct TimelineCampaign {
    campaign_id: i64,
    jp_mission_event_id: Option<i64>,
    jp_title: Option<String>,
    mission_fingerprint: Option<String>,
    image: String,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    title: Option<String>,
    description: Option<String>,
    image_url: Option<String>,
    image_path: Option<String>,
    confirmed_global_start: Option<DateTime<Utc>>,
    confirmed_global_end: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct GlobalMissionCampaign {
    event_id: i64,
    title: String,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    mission_count: usize,
    fingerprint: String,
}

#[derive(Debug, Deserialize)]
struct RawTimelineCharacterBanner {
    gacha_id: i64,
    year: i32,
    image: String,
    start_date: String,
    end_date: String,
    pickup_card_ids: Vec<i64>,
    image_path: String,
}

#[derive(Debug, Deserialize)]
struct RawTimelineSupportBanner {
    gacha_id: i64,
    year: i32,
    image: String,
    start_date: String,
    end_date: String,
    pickup_card_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
struct RawTimelinePaidBanner {
    gacha_id: i64,
    gacha_type: i64,
    card_type: String,
    year: i32,
    image: String,
    start_date: String,
    end_date: String,
    pickup_card_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
struct RawTimelineStoryEvent {
    event_name: String,
    image: String,
    start_date: String,
    end_date: String,
}

#[derive(Debug, Deserialize)]
struct RawTimelineChampionsMeeting {
    name: String,
    start_date: String,
    end_date: String,
    track: Option<String>,
    distance: Option<String>,
    conditions: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawTimelineLegendRace {
    race_name: String,
    start_date: String,
    end_date: String,
    course: Option<String>,
    bosses: Option<Vec<RawTimelineLegendBoss>>,
}

#[derive(Debug, Deserialize)]
struct RawTimelineLegendBoss {
    name: Option<String>,
    image: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawTimelineCampaign {
    campaign_id: i64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    jp_title: Option<String>,
    #[serde(default)]
    jp_mission_event_id: Option<i64>,
    #[serde(default)]
    mission_fingerprint: Option<String>,
    image: String,
    start_date: String,
    end_date: String,
}

#[derive(Debug, Deserialize)]
struct RawSupportCardName {
    id: String,
    name: String,
    #[serde(default)]
    card_name: Option<String>,
    #[serde(default)]
    support_card_name: Option<String>,
}

#[derive(Debug, Clone)]
struct DatePrediction {
    global_date: DateTime<Utc>,
    kind: PredictionKind,
    acceleration_rate: Option<f64>,
    schedule_adjustment_days: Option<i64>,
    anchor_jp_date: Option<DateTime<Utc>>,
    anchor_global_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct FamilyAdjustmentModels {
    models: BTreeMap<BannerTimelineEventType, FamilyAdjustmentModel>,
}

#[derive(Debug, Clone)]
struct FamilyAdjustmentModel {
    samples: Vec<FamilyAdjustmentSample>,
}

#[derive(Debug, Clone)]
struct FamilyAdjustmentSample {
    jp: DateTime<Utc>,
    adjustment_days: i64,
}

#[derive(Debug, Clone)]
struct CalendarLikelihoodModel {
    month_count_likelihoods: Vec<CountLikelihood>,
    character_gap_likelihoods: Vec<CountLikelihood>,
    weekday_likelihoods: Vec<NamedLikelihood>,
    month_day_likelihoods: Vec<CountLikelihood>,
    event_type_likelihoods: Vec<EventTypeCalendarLikelihood>,
    month_count_probabilities: BTreeMap<usize, f64>,
    character_gap_probabilities: BTreeMap<usize, f64>,
    weekday_probabilities: BTreeMap<u32, f64>,
    month_day_probabilities: BTreeMap<u32, f64>,
    event_type_weekday_probabilities: BTreeMap<BannerTimelineEventType, BTreeMap<u32, f64>>,
    event_type_month_day_probabilities: BTreeMap<BannerTimelineEventType, BTreeMap<u32, f64>>,
    default_event_type_weekday_probabilities: BTreeMap<BannerTimelineEventType, f64>,
    default_event_type_month_day_probabilities: BTreeMap<BannerTimelineEventType, f64>,
    default_month_count_probability: f64,
    default_character_gap_probability: f64,
    default_weekday_probability: f64,
    default_month_day_probability: f64,
}

pub fn generate(
    connection: &Connection,
    jp_connection: Option<&Connection>,
    character_banners: &[CharacterBanner],
    support_banners: &[SupportBanner],
    paid_banners: &[PaidBanner],
) -> Result<BannerTimeline> {
    let timeline_character_banners = load_timeline_character_banners()?;
    let timeline_support_banners = load_timeline_support_banners()?;
    let timeline_paid_banners = load_timeline_paid_banners()?;
    let timeline_story_events = load_timeline_story_events(jp_connection)?;
    let mut timeline_champions_meetings = load_timeline_champions_meetings()?;
    let mut timeline_legend_races = load_timeline_legend_races(jp_connection)?;
    let mut timeline_campaigns = load_timeline_campaigns(connection)?;
    let mut news_timeline_events = crate::generators::jp_events::timeline_events()?;
    merge_champions_meeting_news(&mut timeline_champions_meetings, &mut news_timeline_events);
    reuse_champions_meeting_image_paths(&mut timeline_champions_meetings);
    merge_campaign_news(
        &mut timeline_campaigns,
        &crate::generators::jp_events::campaign_timeline_metadata()?,
    );
    merge_legend_race_news(
        &mut timeline_legend_races,
        &crate::generators::jp_events::legend_race_timeline_metadata()?,
    );
    let additional_gacha_banners = crate::generators::jp_events::additional_gacha_banner_events()?;
    let character_names = common::load_character_name_map(connection)?;
    let support_names = load_support_card_names(connection, &character_names)?;
    let support_card_names = load_support_card_specific_names(connection)?;
    let confirmed_dates = build_confirmed_date_lookup(
        character_banners,
        support_banners,
        paid_banners,
        &timeline_character_banners,
        &timeline_support_banners,
        &timeline_paid_banners,
        &additional_gacha_banners,
    )?;
    let anchors = build_banner_confirmed_anchors(
        &timeline_character_banners,
        &timeline_support_banners,
        &confirmed_dates,
    );
    let unique_anchors = monotonic_schedule_anchors(&anchors);
    let observed_rate = calculate_recent_acceleration_rate(&unique_anchors);
    let anniversary_anchors = build_anniversary_schedule_anchors(&unique_anchors, &confirmed_dates);
    let anniversary_rate = calculate_recent_acceleration_rate(&anniversary_anchors);
    let family_adjustments = build_family_adjustment_models(
        &timeline_paid_banners,
        &timeline_story_events,
        &timeline_champions_meetings,
        &timeline_legend_races,
        &timeline_campaigns,
        &news_timeline_events,
        &confirmed_dates,
        &unique_anchors,
        observed_rate,
    );
    let latest_anchor = unique_anchors
        .iter()
        .max_by_key(|anchor| anchor.global)
        .cloned();
    let latest_closed_global_month = latest_closed_global_month(&confirmed_dates);
    let unconfirmed_schedule_floor = latest_closed_global_month
        .map(|(year, month)| first_release_after_global_month(year, month));

    let mut events = Vec::new();
    events.extend(timeline_character_banners.iter().map(|banner| {
        character_event(
            banner,
            &character_names,
            &confirmed_dates,
            &unique_anchors,
            observed_rate,
        )
    }));
    events.extend(timeline_support_banners.iter().map(|banner| {
        support_event(
            banner,
            &support_names,
            &support_card_names,
            &confirmed_dates,
            &unique_anchors,
            observed_rate,
        )
    }));
    events.extend(additional_gacha_banners.iter().filter_map(|banner| {
        additional_gacha_event(
            banner,
            &support_names,
            &support_card_names,
            &confirmed_dates,
            &unique_anchors,
            observed_rate,
        )
    }));
    events.extend(paid_events(
        &timeline_paid_banners,
        &character_names,
        &support_names,
        &support_card_names,
        &confirmed_dates,
        &unique_anchors,
        &family_adjustments,
        observed_rate,
    ));
    events.extend(timeline_story_events.iter().map(|event| {
        story_event(
            event,
            &confirmed_dates,
            &unique_anchors,
            &family_adjustments,
            observed_rate,
        )
    }));
    events.extend(timeline_champions_meetings.iter().map(|event| {
        champions_meeting_event(
            event,
            &confirmed_dates,
            &unique_anchors,
            &family_adjustments,
            observed_rate,
        )
    }));
    events.extend(timeline_legend_races.iter().map(|event| {
        legend_race_event(
            event,
            &confirmed_dates,
            &unique_anchors,
            &family_adjustments,
            observed_rate,
        )
    }));
    events.extend(timeline_campaigns.iter().map(|event| {
        campaign_event(
            event,
            &confirmed_dates,
            &unique_anchors,
            &family_adjustments,
            observed_rate,
        )
    }));
    events.extend(news_timeline_events.iter().map(|event| {
        news_timeline_event(
            event,
            &confirmed_dates,
            &unique_anchors,
            &family_adjustments,
            observed_rate,
        )
    }));

    attach_umapyoi_urls(&mut events)?;
    annotate_rerun_banners(&mut events);
    apply_closed_schedule_adjustment(&mut events, &confirmed_dates);
    apply_grouped_event_adjustment(&mut events);
    apply_closed_schedule_adjustment(&mut events, &confirmed_dates);
    let calendar_likelihood_model = CalendarLikelihoodModel::from_events(&events);
    annotate_calendar_likelihoods(&mut events, &calendar_likelihood_model);
    let anniversaries = timeline_anniversaries(
        &events,
        &confirmed_dates,
        &anniversary_anchors,
        anniversary_rate,
    );

    events.sort_by(|a, b| {
        a.global_release_date
            .cmp(&b.global_release_date)
            .then_with(|| a.jp_release_date.cmp(&b.jp_release_date))
            .then_with(|| a.id.cmp(&b.id))
    });

    Ok(BannerTimeline {
        version: TIMELINE_ALGORITHM_VERSION,
        calculation: TimelineCalculation {
            jp_launch_date: jp_launch_date(),
            global_launch_date: global_launch_date(),
            fallback_acceleration_rate: FALLBACK_ACCELERATION_RATE,
            observed_acceleration_rate: round_rate(observed_rate),
            confirmed_anchor_count: unique_anchors.len(),
            character_banner_month_count_likelihoods: calendar_likelihood_model
                .month_count_likelihoods
                .clone(),
            character_banner_gap_likelihoods: calendar_likelihood_model
                .character_gap_likelihoods
                .clone(),
            character_banner_weekday_likelihoods: calendar_likelihood_model
                .weekday_likelihoods
                .clone(),
            character_banner_month_day_likelihoods: calendar_likelihood_model
                .month_day_likelihoods
                .clone(),
            event_type_calendar_likelihoods: calendar_likelihood_model
                .event_type_likelihoods
                .clone(),
            latest_closed_global_month: latest_closed_global_month
                .map(|(year, month)| format!("{year:04}-{month:02}")),
            unconfirmed_schedule_floor,
            latest_confirmed_jp_date: latest_anchor.as_ref().map(|anchor| anchor.jp),
            latest_confirmed_global_date: latest_anchor.map(|anchor| anchor.global),
        },
        anniversaries,
        events,
    })
}

fn character_event(
    banner: &TimelineCharacterBanner,
    character_names: &BTreeMap<i64, String>,
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    observed_rate: f64,
) -> BannerTimelineEvent {
    let names = banner
        .pickup_card_ids
        .iter()
        .map(|card_id| character_name_for_card(*card_id, character_names))
        .collect::<Vec<_>>();
    let confirmed_global_date = confirmed_dates.character.get(&banner.gacha_id).copied();
    let prediction = calculate_global_date(
        banner.start_at,
        confirmed_global_date,
        anchors,
        observed_rate,
    );
    let duration = banner_duration_days(banner.start_at, banner.end_at);
    let adjustment = if is_berlin_dst(prediction.global_date) {
        0
    } else {
        1
    };
    let banner_id = banner.gacha_id.to_string();

    BannerTimelineEvent {
        id: format!("banner-{}", image_stem(&banner.image)),
        event_type: BannerTimelineEventType::CharacterBanner,
        source: "character",
        gacha_id: Some(banner.gacha_id),
        gacha_ids: Vec::new(),
        gacha_type: Some(3),
        gacha_type_name: Some(gacha_type_name(3)),
        card_type: Some("character".to_string()),
        year: Some(banner.year),
        image: banner.image.clone(),
        image_path: Some(banner.image_path.clone()),
        title: title_from_names(&names, "Character Banner"),
        description: None,
        jp_release_date: banner.start_at,
        global_release_date: prediction.global_date,
        estimated_end_date: calculate_end_date(prediction.global_date, duration + adjustment),
        is_confirmed: confirmed_global_date.is_some(),
        banner_duration_days: duration,
        tags: vec!["character-banner"],
        pickup_card_ids: banner.pickup_card_ids.clone(),
        related_characters: names,
        related_support_cards: Vec::new(),
        related_support_card_names: Vec::new(),
        gametora_url: Some(format!(
            "https://gametora.com/umamusume/gacha/history?server=ja&year={}&type=char#{}",
            banner.year, banner_id
        )),
        umapyoi_url: None,
        prediction: prediction.into_info(),
    }
}

fn support_event(
    banner: &TimelineSupportBanner,
    support_names: &BTreeMap<i64, String>,
    support_card_names: &BTreeMap<i64, String>,
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    observed_rate: f64,
) -> BannerTimelineEvent {
    let names = banner
        .pickup_card_ids
        .iter()
        .map(|card_id| support_name_for_card(*card_id, support_names))
        .collect::<Vec<_>>();
    let card_names = banner
        .pickup_card_ids
        .iter()
        .filter_map(|card_id| support_card_specific_name_for_card(*card_id, support_card_names))
        .collect::<Vec<_>>();
    let confirmed_global_date = confirmed_dates.support.get(&banner.gacha_id).copied();
    let prediction = calculate_global_date(
        banner.start_at,
        confirmed_global_date,
        anchors,
        observed_rate,
    );
    let duration = banner_duration_days(banner.start_at, banner.end_at);
    let adjustment = if is_berlin_dst(prediction.global_date) {
        -1
    } else {
        0
    };
    let banner_id = banner.gacha_id.to_string();

    BannerTimelineEvent {
        id: format!("support-banner-{}", image_stem(&banner.image)),
        event_type: BannerTimelineEventType::SupportCardBanner,
        source: "support",
        gacha_id: Some(banner.gacha_id),
        gacha_ids: Vec::new(),
        gacha_type: Some(3),
        gacha_type_name: Some(gacha_type_name(3)),
        card_type: Some("support".to_string()),
        year: Some(banner.year),
        image: banner.image.clone(),
        image_path: Some(format!("assets/images/support/banner/{}", banner.image)),
        title: title_from_names(&names, "Support Card Banner"),
        description: None,
        jp_release_date: banner.start_at,
        global_release_date: prediction.global_date,
        estimated_end_date: calculate_end_date(prediction.global_date, duration + adjustment),
        is_confirmed: confirmed_global_date.is_some(),
        banner_duration_days: duration,
        tags: vec!["support-banner"],
        pickup_card_ids: banner.pickup_card_ids.clone(),
        related_characters: Vec::new(),
        related_support_cards: names,
        related_support_card_names: card_names,
        gametora_url: Some(format!(
            "https://gametora.com/umamusume/gacha/history?server=ja&year={}&type=sup#{}",
            banner.year, banner_id
        )),
        umapyoi_url: None,
        prediction: prediction.into_info(),
    }
}

fn additional_gacha_event(
    banner: &AdditionalGachaBanner,
    support_names: &BTreeMap<i64, String>,
    support_card_names: &BTreeMap<i64, String>,
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    observed_rate: f64,
) -> Option<BannerTimelineEvent> {
    let (event_type, default_card_type, source, tag, confirmed_global_date) = match banner.kind {
        AdditionalGachaKind::Character => (
            BannerTimelineEventType::CharacterBanner,
            "character",
            "umapyoi_news_character",
            "character-banner",
            confirmed_dates.character.get(&banner.gacha_id).copied(),
        ),
        AdditionalGachaKind::Support => (
            BannerTimelineEventType::SupportCardBanner,
            "support",
            "umapyoi_news_support",
            "support-banner",
            confirmed_dates.support.get(&banner.gacha_id).copied(),
        ),
        AdditionalGachaKind::Paid => (
            BannerTimelineEventType::PaidBanner,
            "paid",
            "umapyoi_news_paid",
            "paid-banner",
            confirmed_dates.paid.get(&banner.gacha_id).copied(),
        ),
        AdditionalGachaKind::Unknown => return None,
    };
    let card_type = banner.card_type.as_deref().unwrap_or(default_card_type);
    let prediction = calculate_global_date(
        banner.start_at,
        confirmed_global_date,
        anchors,
        observed_rate,
    );
    let duration = 10;
    let gacha_type = banner.gacha_type.or_else(|| {
        matches!(
            banner.kind,
            AdditionalGachaKind::Character | AdditionalGachaKind::Support
        )
        .then_some(3)
    });
    let mut related_support_cards = banner
        .pickup_card_ids
        .iter()
        .map(|card_id| support_name_for_card(*card_id, support_names))
        .collect::<Vec<_>>();
    if related_support_cards.is_empty() {
        related_support_cards = banner.related_support_names.clone();
    }
    let related_support_card_names = banner
        .pickup_card_ids
        .iter()
        .filter_map(|card_id| support_card_specific_name_for_card(*card_id, support_card_names))
        .collect::<Vec<_>>();
    let mut tags = vec![tag, "umapyoi-news"];
    if banner.is_rerun {
        tags.push("rerun-banner");
    }
    if banner.is_scenario {
        tags.push("scenario-banner");
    }
    let related_characters = banner.related_character_names.clone();
    let display_names = match card_type {
        "character" => related_characters.clone(),
        "support" => related_support_cards.clone(),
        _ => Vec::new(),
    };
    let title = if display_names.is_empty() {
        additional_gacha_title(banner, card_type)
    } else {
        title_from_names(&display_names, &additional_gacha_title(banner, card_type))
    };
    Some(BannerTimelineEvent {
        id: format!("umapyoi-gacha-{}", banner.gacha_id),
        event_type,
        source,
        gacha_id: Some(banner.gacha_id),
        gacha_ids: Vec::new(),
        gacha_type,
        gacha_type_name: gacha_type.map(gacha_type_name),
        card_type: Some(card_type.to_string()),
        year: Some(banner.start_at.year()),
        image: banner.image_url.clone(),
        image_path: (!banner.image_url.is_empty()).then(|| additional_gacha_asset_path(banner)),
        title,
        description: banner.description.clone(),
        jp_release_date: banner.start_at,
        global_release_date: prediction.global_date,
        estimated_end_date: calculate_end_date(prediction.global_date, duration),
        is_confirmed: confirmed_global_date.is_some(),
        banner_duration_days: duration,
        tags,
        pickup_card_ids: banner.pickup_card_ids.clone(),
        related_characters,
        related_support_cards,
        related_support_card_names,
        gametora_url: None,
        umapyoi_url: None,
        prediction: prediction.into_info(),
    })
}

fn additional_gacha_title(banner: &AdditionalGachaBanner, card_type: &str) -> String {
    match banner.gacha_type {
        Some(11) => "Twinkle Collection".to_string(),
        Some(15) => "Select Pickup Stamp Sheet".to_string(),
        Some(14) => "Select Step-Up".to_string(),
        Some(12) => "Pick 2 Support Card Gacha".to_string(),
        Some(5) if banner.is_scenario && card_type == "support" => {
            "SSR Guaranteed New Training Scenario".to_string()
        }
        Some(5) if banner.is_scenario && card_type == "character" => {
            "★3 Guaranteed New Training Scenario".to_string()
        }
        Some(5) if card_type == "support" => "SSR Guaranteed".to_string(),
        Some(5) if card_type == "character" => "★3 Guaranteed".to_string(),
        _ if banner.is_scenario && card_type == "support" => {
            "New Training Scenario Support Gacha".to_string()
        }
        _ if card_type == "character" => "Character Pickup".to_string(),
        _ if card_type == "support" => "Support Card Pickup".to_string(),
        _ => banner.title.trim().trim_matches('!').to_string(),
    }
}

fn gacha_type_name(gacha_type: i64) -> &'static str {
    match gacha_type {
        1 => "standard_pool",
        2 => "makeup_debut",
        3 => "standard_pickup",
        5 => "guaranteed",
        10 => "group_select",
        11 => "twinkle_collection",
        12 => "pick_2",
        13 => "special_guaranteed",
        14 => "select_step_up",
        15 => "stamp_sheet",
        _ => "unknown",
    }
}

fn annotate_rerun_banners(events: &mut [BannerTimelineEvent]) {
    let mut first_release_by_card: BTreeMap<(String, i64), DateTime<Utc>> = BTreeMap::new();

    for event in events.iter().filter(|event| rerun_candidate(event)) {
        let Some(card_type) = event.card_type.as_ref() else {
            continue;
        };
        for card_id in &event.pickup_card_ids {
            first_release_by_card
                .entry((card_type.clone(), *card_id))
                .and_modify(|date| *date = (*date).min(event.jp_release_date))
                .or_insert(event.jp_release_date);
        }
    }

    for event in events.iter_mut().filter(|event| rerun_candidate(event)) {
        let explicitly_rerun = event.gacha_type == Some(12) || event.tags.contains(&"rerun-banner");
        let historical_rerun = event.card_type.as_ref().is_some_and(|card_type| {
            !event.pickup_card_ids.is_empty()
                && event.pickup_card_ids.iter().all(|card_id| {
                    first_release_by_card
                        .get(&(card_type.clone(), *card_id))
                        .is_some_and(|first_release| *first_release < event.jp_release_date)
                })
        });

        if (explicitly_rerun || historical_rerun) && !event.tags.contains(&"rerun-banner") {
            event.tags.push("rerun-banner");
        }
    }
}

fn rerun_candidate(event: &BannerTimelineEvent) -> bool {
    matches!(
        event.event_type,
        BannerTimelineEventType::CharacterBanner | BannerTimelineEventType::SupportCardBanner
    ) && matches!(event.gacha_type, Some(3 | 11 | 12))
}

fn attach_umapyoi_urls(events: &mut [BannerTimelineEvent]) -> Result<()> {
    let archive: serde_json::Value = serde_json::from_slice(BUNDLED_UMAPYOI_ARCHIVE_JSON)
        .context("failed to parse bundled umapyoi archive for timeline source links")?;
    let posts = archive
        .get("news")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let by_id = posts
        .iter()
        .filter_map(|post| Some((post.get("post_id")?.as_i64()?, post)))
        .collect::<BTreeMap<_, _>>();
    let mut by_gacha_id = BTreeMap::new();
    for post in &posts {
        for banner in post
            .get("gacha_banners")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(gacha_id) = banner.get("gacha_id").and_then(serde_json::Value::as_i64) {
                by_gacha_id.insert(gacha_id, post);
            }
        }
    }

    for event in events {
        let direct = announcement_post_id(&event.image)
            .and_then(|post_id| by_id.get(&post_id).copied())
            .or_else(|| {
                event
                    .gacha_id
                    .and_then(|gacha_id| by_gacha_id.get(&gacha_id).copied())
            });
        let matched = direct.or_else(|| best_umapyoi_post_for_event(event, &posts));
        event.umapyoi_url = matched
            .and_then(|post| post.get("page_url"))
            .and_then(serde_json::Value::as_str)
            .filter(|url| url.starts_with("https://umapyoi.net/news/"))
            .map(str::to_string);
    }
    Ok(())
}

fn announcement_post_id(url: &str) -> Option<i64> {
    url.split("/announce/")
        .nth(1)?
        .split('/')
        .next()?
        .parse()
        .ok()
}

fn best_umapyoi_post_for_event<'a>(
    event: &BannerTimelineEvent,
    posts: &'a [serde_json::Value],
) -> Option<&'a serde_json::Value> {
    let expected_type = match event.event_type {
        BannerTimelineEventType::StoryEvent => "story_event",
        BannerTimelineEventType::ChampionsMeeting => "champions_meeting",
        BannerTimelineEventType::LegendRace => "legend_race",
        BannerTimelineEventType::Campaign => "campaign",
        BannerTimelineEventType::LeagueOfHeroes => "league_of_heroes",
        BannerTimelineEventType::MastersChallenge => "masters_challenge",
        BannerTimelineEventType::TrainerSkillsTest => "trainer_skills_test",
        BannerTimelineEventType::FactorResearch => "factor_research",
        BannerTimelineEventType::StrongestTeam => "strongest_team",
        BannerTimelineEventType::RacingCarnival => "racing_carnival",
        BannerTimelineEventType::ScenarioRelease => "training_scenario",
        _ => return None,
    };
    let event_family = campaign_link_family(&event.title);
    let mut candidates = Vec::new();
    for post in posts {
        let Some(event_types) = post
            .get("event_types")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        if !event_types
            .iter()
            .any(|value| value.as_str() == Some(expected_type))
        {
            continue;
        }
        let Some(posted_at) = post
            .get("posted_at")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| {
                DateTime::parse_from_rfc3339(value)
                    .ok()
                    .map(|date| date.with_timezone(&Utc))
            })
        else {
            continue;
        };
        let delta_days = (posted_at - event.jp_release_date).num_seconds().abs() / 86_400;
        let post_title = post
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();

        let score = if event.event_type == BannerTimelineEventType::Campaign {
            let Some(family) = event_family else { continue };
            if campaign_link_family(post_title) != Some(family)
                || posted_at > event.jp_release_date + Duration::days(2)
                || event.jp_release_date > posted_at + Duration::days(56)
            {
                continue;
            }
            200 - delta_days
        } else {
            if delta_days > 2 {
                continue;
            }
            let similarity = timeline_title_similarity(&event.title, post_title) as i64;
            let dedicated_story = event.event_type == BannerTimelineEventType::StoryEvent
                && event_types.len() == 1
                && event_types[0].as_str() == Some("story_event")
                && post
                    .get("images")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|image| image.get("url").and_then(serde_json::Value::as_str))
                    .any(|url| url.contains("banner_301") || url.contains("header_301"));
            if similarity == 0 && !dedicated_story {
                continue;
            }
            similarity * 10 + if dedicated_story { 30 } else { 0 } - delta_days
        };
        candidates.push((
            score,
            post.get("post_id")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
            post,
        ));
    }
    candidates
        .into_iter()
        .max_by_key(|(score, post_id, _)| (*score, *post_id))
        .map(|(_, _, post)| post)
}

fn normalized_timeline_title(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn timeline_title_similarity(left: &str, right: &str) -> usize {
    const IGNORED: &[&str] = &[
        "the", "a", "an", "is", "are", "now", "here", "held", "event", "campaign", "missions",
        "mission", "race", "live", "underway",
    ];
    let left = normalized_timeline_title(left)
        .split_whitespace()
        .filter(|token| !IGNORED.contains(token))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let right = normalized_timeline_title(right)
        .split_whitespace()
        .filter(|token| !IGNORED.contains(token))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    left.intersection(&right).count()
}

fn campaign_link_family(value: &str) -> Option<&'static str> {
    let title = normalized_timeline_title(value);
    if title.contains("anniversary") {
        Some("anniversary")
    } else if title
        .split_whitespace()
        .any(|token| token == "g1" || token == "gi")
    {
        Some("g1")
    } else if title.contains("golshi") || title.contains("gw special") {
        Some("golshi")
    } else if title.contains("training the trainer") || title.contains("tracen special") {
        Some("training-the-trainer")
    } else if title.contains("release celebration") {
        Some("release-celebration")
    } else {
        None
    }
}

fn additional_gacha_asset_path(banner: &AdditionalGachaBanner) -> String {
    let family = match banner.kind {
        AdditionalGachaKind::Character => "character",
        AdditionalGachaKind::Support => "support",
        AdditionalGachaKind::Paid => "paid",
        AdditionalGachaKind::Unknown => "unknown",
    };
    format!(
        "assets/timeline-images/gacha/{family}/{}.webp",
        banner.gacha_id
    )
}

fn news_event_asset_path(event: &NewsTimelineEvent) -> String {
    let family = match event.kind {
        NewsTimelineKind::ChampionsMeeting => "champions-meeting",
        NewsTimelineKind::TrainingScenario => "training-scenario",
        NewsTimelineKind::LeagueOfHeroes => "league-of-heroes",
        NewsTimelineKind::MastersChallenge => "masters-challenge",
        NewsTimelineKind::TrainerSkillsTest => "trainer-skills-test",
        NewsTimelineKind::FactorResearch => "factor-research",
        NewsTimelineKind::StrongestTeam => "strongest-team",
        NewsTimelineKind::RacingCarnival => "racing-carnival",
    };
    format!(
        "assets/timeline-images/events/{family}/{}.webp",
        event.source_post_id
    )
}

fn news_timeline_event(
    event: &NewsTimelineEvent,
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    family_adjustments: &FamilyAdjustmentModels,
    observed_rate: f64,
) -> BannerTimelineEvent {
    let (event_type, source, tag) = match event.kind {
        NewsTimelineKind::ChampionsMeeting => (
            BannerTimelineEventType::ChampionsMeeting,
            "champions_meeting_news",
            "champions-meeting",
        ),
        NewsTimelineKind::TrainingScenario => (
            BannerTimelineEventType::ScenarioRelease,
            "training_scenario",
            "training-scenario",
        ),
        NewsTimelineKind::LeagueOfHeroes => (
            BannerTimelineEventType::LeagueOfHeroes,
            "league_of_heroes",
            "league-of-heroes",
        ),
        NewsTimelineKind::MastersChallenge => (
            BannerTimelineEventType::MastersChallenge,
            "masters_challenge",
            "masters-challenge",
        ),
        NewsTimelineKind::TrainerSkillsTest => (
            BannerTimelineEventType::TrainerSkillsTest,
            "trainer_skills_test",
            "trainer-skills-test",
        ),
        NewsTimelineKind::FactorResearch => (
            BannerTimelineEventType::FactorResearch,
            "factor_research",
            "factor-research",
        ),
        NewsTimelineKind::StrongestTeam => (
            BannerTimelineEventType::StrongestTeam,
            "strongest_team",
            "strongest-team",
        ),
        NewsTimelineKind::RacingCarnival => (
            BannerTimelineEventType::RacingCarnival,
            "racing_carnival",
            "racing-carnival",
        ),
    };
    let confirmed_global_date = confirmed_dates.news_events.get(&event.key).copied();
    let prediction = apply_family_adjustment(
        calculate_global_date(
            event.start_at,
            confirmed_global_date,
            anchors,
            observed_rate,
        ),
        event_type,
        event.start_at,
        family_adjustments,
    );
    let duration = banner_duration_days(event.start_at, event.end_at).max(1);
    let image = event.image_url.clone().unwrap_or_default();
    BannerTimelineEvent {
        id: format!("news-event-{}", event.key),
        event_type,
        source,
        gacha_id: None,
        gacha_ids: Vec::new(),
        gacha_type: None,
        gacha_type_name: None,
        card_type: None,
        year: None,
        image: image.clone(),
        image_path: (!image.is_empty()).then(|| news_event_asset_path(event)),
        title: event.title.clone(),
        description: event.description.clone(),
        jp_release_date: event.start_at,
        global_release_date: prediction.global_date,
        estimated_end_date: calculate_end_date(prediction.global_date, duration),
        is_confirmed: confirmed_global_date.is_some(),
        banner_duration_days: duration,
        tags: vec!["event", tag, "umapyoi-news"],
        pickup_card_ids: Vec::new(),
        related_characters: Vec::new(),
        related_support_cards: Vec::new(),
        related_support_card_names: Vec::new(),
        gametora_url: None,
        umapyoi_url: None,
        prediction: prediction.into_info(),
    }
}

fn paid_events(
    banners: &[TimelinePaidBanner],
    character_names: &BTreeMap<i64, String>,
    support_names: &BTreeMap<i64, String>,
    support_card_names: &BTreeMap<i64, String>,
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    family_adjustments: &FamilyAdjustmentModels,
    observed_rate: f64,
) -> Vec<BannerTimelineEvent> {
    let mut events = Vec::new();
    let mut bundled_by_month: BTreeMap<String, Vec<&TimelinePaidBanner>> = BTreeMap::new();

    for banner in banners {
        if !banner.pickup_card_ids.is_empty() || banner.gacha_type != 14 {
            events.push(paid_event(
                banner,
                character_names,
                support_names,
                support_card_names,
                confirmed_dates,
                anchors,
                family_adjustments,
                observed_rate,
            ));
        } else {
            let key = format!("{}-{:02}", banner.start_at.year(), banner.start_at.month());
            bundled_by_month.entry(key).or_default().push(banner);
        }
    }

    for (month_key, mut group) in bundled_by_month {
        group.sort_by_key(|banner| (banner.start_at, banner.gacha_id));
        if let Some(representative) = group.first() {
            let confirmed_global_date = confirmed_dates.paid.get(&representative.gacha_id).copied();
            let prediction = apply_family_adjustment(
                calculate_global_date(
                    representative.start_at,
                    confirmed_global_date,
                    anchors,
                    observed_rate,
                ),
                BannerTimelineEventType::PaidBanner,
                representative.start_at,
                family_adjustments,
            );
            let count = group.len();
            events.push(BannerTimelineEvent {
                id: format!("paid-banner-bundle-{}", month_key),
                event_type: BannerTimelineEventType::PaidBanner,
                source: "paid_bundle",
                gacha_id: None,
                gacha_ids: group.iter().map(|banner| banner.gacha_id).collect(),
                gacha_type: Some(representative.gacha_type),
                gacha_type_name: Some(gacha_type_name(representative.gacha_type)),
                card_type: Some(representative.card_type.to_string()),
                year: Some(representative.year),
                image: representative.image.clone(),
                image_path: Some(format!(
                    "assets/images/paid/banner/{}",
                    representative.image
                )),
                title: format!(
                    "{} Other Paid Banner{}",
                    count,
                    if count > 1 { "s" } else { "" }
                ),
                description: None,
                jp_release_date: representative.start_at,
                global_release_date: prediction.global_date,
                estimated_end_date: calculate_end_date(prediction.global_date, 14),
                is_confirmed: confirmed_global_date.is_some(),
                banner_duration_days: 14,
                tags: vec!["paid-banner"],
                pickup_card_ids: Vec::new(),
                related_characters: Vec::new(),
                related_support_cards: Vec::new(),
                related_support_card_names: Vec::new(),
                gametora_url: None,
                umapyoi_url: None,
                prediction: prediction.into_info(),
            });
        }
    }

    events
}

fn paid_event(
    banner: &TimelinePaidBanner,
    character_names: &BTreeMap<i64, String>,
    support_names: &BTreeMap<i64, String>,
    support_card_names: &BTreeMap<i64, String>,
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    family_adjustments: &FamilyAdjustmentModels,
    observed_rate: f64,
) -> BannerTimelineEvent {
    let names = banner
        .pickup_card_ids
        .iter()
        .map(|card_id| {
            if banner.card_type == "character" {
                character_name_for_card(*card_id, character_names)
            } else {
                support_name_for_card(*card_id, support_names)
            }
        })
        .collect::<Vec<_>>();
    let support_card_names = if banner.card_type == "support" {
        banner
            .pickup_card_ids
            .iter()
            .filter_map(|card_id| support_card_specific_name_for_card(*card_id, support_card_names))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let confirmed_global_date = confirmed_dates.paid.get(&banner.gacha_id).copied();
    let prediction = apply_family_adjustment(
        calculate_global_date(
            banner.start_at,
            confirmed_global_date,
            anchors,
            observed_rate,
        ),
        BannerTimelineEventType::PaidBanner,
        banner.start_at,
        family_adjustments,
    );
    let duration = banner_duration_days(banner.start_at, banner.end_at).max(14);
    let title = if names.is_empty() {
        if banner.card_type == "character" {
            "Premium Character Banner".to_string()
        } else {
            "Premium Support Banner".to_string()
        }
    } else {
        title_from_names(&names, "Paid Banner")
    };

    BannerTimelineEvent {
        id: format!("paid-banner-{}", banner.gacha_id),
        event_type: BannerTimelineEventType::PaidBanner,
        source: "paid",
        gacha_id: Some(banner.gacha_id),
        gacha_ids: Vec::new(),
        gacha_type: Some(banner.gacha_type),
        gacha_type_name: Some(gacha_type_name(banner.gacha_type)),
        card_type: Some(banner.card_type.to_string()),
        year: Some(banner.year),
        image: banner.image.clone(),
        image_path: Some(format!("assets/images/paid/banner/{}", banner.image)),
        title,
        description: None,
        jp_release_date: banner.start_at,
        global_release_date: prediction.global_date,
        estimated_end_date: calculate_end_date(prediction.global_date, duration),
        is_confirmed: confirmed_global_date.is_some(),
        banner_duration_days: duration,
        tags: vec!["paid-banner"],
        pickup_card_ids: banner.pickup_card_ids.clone(),
        related_characters: if banner.card_type == "character" {
            names.clone()
        } else {
            Vec::new()
        },
        related_support_cards: if banner.card_type == "support" {
            names
        } else {
            Vec::new()
        },
        related_support_card_names: support_card_names,
        gametora_url: None,
        umapyoi_url: None,
        prediction: prediction.into_info(),
    }
}

fn story_event(
    event: &TimelineStoryEvent,
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    family_adjustments: &FamilyAdjustmentModels,
    observed_rate: f64,
) -> BannerTimelineEvent {
    let key = image_key(&event.image);
    let confirmed_global_date = event
        .master_event_id
        .is_none()
        .then(|| confirmed_dates.story.get(&key).copied())
        .flatten();
    let prediction = apply_family_adjustment(
        calculate_global_date(
            event.start_at,
            confirmed_global_date,
            anchors,
            observed_rate,
        ),
        BannerTimelineEventType::StoryEvent,
        event.start_at,
        family_adjustments,
    );
    let duration = banner_duration_days(event.start_at, event.end_at);

    BannerTimelineEvent {
        id: event
            .master_event_id
            .map(|event_id| format!("story-event-master-{event_id}"))
            .unwrap_or_else(|| format!("story-event-{}", image_stem(&event.image))),
        event_type: BannerTimelineEventType::StoryEvent,
        source: "story",
        gacha_id: None,
        gacha_ids: Vec::new(),
        gacha_type: None,
        gacha_type_name: None,
        card_type: None,
        year: None,
        image: event.image.clone(),
        image_path: (!event.image.is_empty())
            .then(|| format!("assets/images/story/{}", event.image)),
        title: event.event_name.clone(),
        description: None,
        jp_release_date: event.start_at,
        global_release_date: prediction.global_date,
        estimated_end_date: calculate_end_date(prediction.global_date, duration),
        is_confirmed: confirmed_global_date.is_some(),
        banner_duration_days: duration,
        tags: vec!["event", "story-event"],
        pickup_card_ids: Vec::new(),
        related_characters: Vec::new(),
        related_support_cards: Vec::new(),
        related_support_card_names: Vec::new(),
        gametora_url: None,
        umapyoi_url: None,
        prediction: prediction.into_info(),
    }
}

fn champions_meeting_event(
    event: &TimelineChampionsMeeting,
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    family_adjustments: &FamilyAdjustmentModels,
    observed_rate: f64,
) -> BannerTimelineEvent {
    let key = champions_key(event.index);
    let confirmed_global_date = confirmed_dates.champions.get(&key).copied();
    let prediction = apply_family_adjustment(
        calculate_global_date(
            event.start_at,
            confirmed_global_date,
            anchors,
            observed_rate,
        ),
        BannerTimelineEventType::ChampionsMeeting,
        event.start_at,
        family_adjustments,
    );
    let duration = banner_duration_days(event.start_at, event.end_at);
    let signup_period_adjustment = if is_berlin_dst(prediction.global_date) {
        2
    } else {
        4
    };
    let image_path = event.image_path.clone().or_else(|| {
        confirmed_global_date.map(|_| {
            format!(
                "assets/timeline-images/events/champions-meeting/confirmed-{}.webp",
                event.index
            )
        })
    });

    BannerTimelineEvent {
        id: format!("champions-meeting-{}", event.index),
        event_type: BannerTimelineEventType::ChampionsMeeting,
        source: "champions",
        gacha_id: None,
        gacha_ids: Vec::new(),
        gacha_type: None,
        gacha_type_name: None,
        card_type: None,
        year: None,
        image: event.image_url.clone().unwrap_or_default(),
        image_path,
        title: event.name.clone(),
        description: champions_description(event),
        jp_release_date: event.start_at,
        global_release_date: prediction.global_date,
        estimated_end_date: calculate_end_date(
            prediction.global_date,
            duration + signup_period_adjustment,
        ),
        is_confirmed: confirmed_global_date.is_some(),
        banner_duration_days: duration,
        tags: vec!["champions-meeting"],
        pickup_card_ids: Vec::new(),
        related_characters: Vec::new(),
        related_support_cards: Vec::new(),
        related_support_card_names: Vec::new(),
        gametora_url: None,
        umapyoi_url: None,
        prediction: prediction.into_info(),
    }
}

fn legend_race_event(
    event: &TimelineLegendRace,
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    family_adjustments: &FamilyAdjustmentModels,
    observed_rate: f64,
) -> BannerTimelineEvent {
    let key = legend_key(event.index);
    let confirmed_global_date = confirmed_dates.legend.get(&key).copied();
    let prediction = apply_family_adjustment(
        calculate_global_date(
            event.start_at,
            confirmed_global_date,
            anchors,
            observed_rate,
        ),
        BannerTimelineEventType::LegendRace,
        event.start_at,
        family_adjustments,
    );
    let duration = banner_duration_days(event.start_at, event.end_at);
    let adjustment = if is_berlin_dst(prediction.global_date) {
        0
    } else {
        1
    };
    let (boss_names, boss_card_ids, _) = legend_boss_metadata(event);

    BannerTimelineEvent {
        id: format!("legend-race-{}", event.index),
        event_type: BannerTimelineEventType::LegendRace,
        source: "legend",
        gacha_id: None,
        gacha_ids: Vec::new(),
        gacha_type: None,
        gacha_type_name: None,
        card_type: None,
        year: None,
        image: event.image_url.clone().unwrap_or_default(),
        image_path: event.image_path.clone(),
        title: event.race_name.clone(),
        description: event.course.clone(),
        jp_release_date: event.start_at,
        global_release_date: prediction.global_date,
        estimated_end_date: calculate_end_date(prediction.global_date, duration + adjustment),
        is_confirmed: confirmed_global_date.is_some(),
        banner_duration_days: duration,
        tags: vec!["event", "legend-race"],
        pickup_card_ids: boss_card_ids,
        related_characters: boss_names,
        related_support_cards: Vec::new(),
        related_support_card_names: Vec::new(),
        gametora_url: None,
        umapyoi_url: None,
        prediction: prediction.into_info(),
    }
}

fn legend_boss_metadata(event: &TimelineLegendRace) -> (Vec<String>, Vec<i64>, Vec<String>) {
    let names = event.bosses.iter().map(|boss| boss.name.clone()).collect();
    let card_ids = event
        .bosses
        .iter()
        .filter_map(|boss| boss.card_id)
        .collect();
    let images = event
        .bosses
        .iter()
        .map(|boss| format!("assets/images/legend/boss/{}", boss.image))
        .collect();
    (names, card_ids, images)
}

fn legend_boss_card_id(image: &str) -> Option<i64> {
    image
        .rsplit(['/', '\\'])
        .next()?
        .split('.')
        .next()?
        .strip_prefix("chara_stand_")?
        .parse()
        .ok()
}

fn campaign_event(
    event: &TimelineCampaign,
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    family_adjustments: &FamilyAdjustmentModels,
    observed_rate: f64,
) -> BannerTimelineEvent {
    let key = image_key(&event.image);
    let confirmed_global_date = event.confirmed_global_start.or_else(|| {
        confirmed_dates
            .campaign
            .get(&key)
            .or_else(|| confirmed_dates.campaign.get(&event.campaign_id.to_string()))
            .copied()
    });
    let prediction = apply_family_adjustment(
        calculate_global_date(
            event.start_at,
            confirmed_global_date,
            anchors,
            observed_rate,
        ),
        BannerTimelineEventType::Campaign,
        event.start_at,
        family_adjustments,
    );
    let duration = match (event.confirmed_global_start, event.confirmed_global_end) {
        (Some(start), Some(end)) => banner_duration_days(start, end),
        _ => banner_duration_days(event.start_at, event.end_at),
    };
    let adjustment = if is_berlin_dst(prediction.global_date) {
        0
    } else {
        1
    };

    BannerTimelineEvent {
        id: format!("campaign-{}", event.campaign_id),
        event_type: BannerTimelineEventType::Campaign,
        source: "campaign",
        gacha_id: None,
        gacha_ids: Vec::new(),
        gacha_type: None,
        gacha_type_name: None,
        card_type: None,
        year: None,
        image: event
            .image_url
            .clone()
            .unwrap_or_else(|| event.image.clone()),
        image_path: event
            .image_path
            .clone()
            .or_else(|| Some(format!("assets/images/campaign/{}", event.image))),
        title: event
            .title
            .clone()
            .unwrap_or_else(|| "Limited-Time Missions".to_string()),
        description: event.description.clone(),
        jp_release_date: event.start_at,
        global_release_date: prediction.global_date,
        estimated_end_date: event
            .confirmed_global_end
            .unwrap_or_else(|| calculate_end_date(prediction.global_date, duration + adjustment)),
        is_confirmed: confirmed_global_date.is_some(),
        banner_duration_days: duration,
        tags: vec!["mission campaign"],
        pickup_card_ids: Vec::new(),
        related_characters: Vec::new(),
        related_support_cards: Vec::new(),
        related_support_card_names: Vec::new(),
        gametora_url: None,
        umapyoi_url: None,
        prediction: prediction.into_info(),
    }
}

fn champions_description(event: &TimelineChampionsMeeting) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(track) = non_empty_string(&event.track) {
        parts.push(track);
    }
    if let Some(distance) = non_empty_string(&event.distance) {
        parts.push(distance);
    }
    if let Some(conditions) = non_empty_string(&event.conditions) {
        parts.push(conditions);
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("<br>"))
    }
}

fn non_empty_string(value: &Option<String>) -> Option<String> {
    value
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn build_family_adjustment_models(
    paid_banners: &[TimelinePaidBanner],
    story_events: &[TimelineStoryEvent],
    champions_meetings: &[TimelineChampionsMeeting],
    legend_races: &[TimelineLegendRace],
    campaigns: &[TimelineCampaign],
    news_events: &[NewsTimelineEvent],
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    observed_rate: f64,
) -> FamilyAdjustmentModels {
    let mut samples = BTreeMap::<BannerTimelineEventType, Vec<FamilyAdjustmentSample>>::new();

    for banner in paid_banners {
        if let Some(global) = confirmed_dates.paid.get(&banner.gacha_id).copied() {
            push_family_adjustment_sample(
                &mut samples,
                BannerTimelineEventType::PaidBanner,
                banner.start_at,
                global,
                anchors,
                observed_rate,
            );
        }
    }

    for event in story_events {
        if let Some(global) = confirmed_dates.story.get(&image_key(&event.image)).copied() {
            push_family_adjustment_sample(
                &mut samples,
                BannerTimelineEventType::StoryEvent,
                event.start_at,
                global,
                anchors,
                observed_rate,
            );
        }
    }

    for event in champions_meetings {
        if let Some(global) = confirmed_dates
            .champions
            .get(&champions_key(event.index))
            .copied()
        {
            push_family_adjustment_sample(
                &mut samples,
                BannerTimelineEventType::ChampionsMeeting,
                event.start_at,
                global,
                anchors,
                observed_rate,
            );
        }
    }

    for event in legend_races {
        if let Some(global) = confirmed_dates
            .legend
            .get(&legend_key(event.index))
            .copied()
        {
            push_family_adjustment_sample(
                &mut samples,
                BannerTimelineEventType::LegendRace,
                event.start_at,
                global,
                anchors,
                observed_rate,
            );
        }
    }

    for event in campaigns {
        if let Some(global) = event.confirmed_global_start.or_else(|| {
            confirmed_dates
                .campaign
                .get(&image_key(&event.image))
                .or_else(|| confirmed_dates.campaign.get(&event.campaign_id.to_string()))
                .copied()
        }) {
            push_family_adjustment_sample(
                &mut samples,
                BannerTimelineEventType::Campaign,
                event.start_at,
                global,
                anchors,
                observed_rate,
            );
        }
    }

    for event in news_events {
        if let Some(global) = confirmed_dates.news_events.get(&event.key).copied() {
            let event_type = match event.kind {
                NewsTimelineKind::LeagueOfHeroes => BannerTimelineEventType::LeagueOfHeroes,
                NewsTimelineKind::ChampionsMeeting => BannerTimelineEventType::ChampionsMeeting,
                NewsTimelineKind::TrainingScenario => BannerTimelineEventType::ScenarioRelease,
                NewsTimelineKind::MastersChallenge => BannerTimelineEventType::MastersChallenge,
                NewsTimelineKind::TrainerSkillsTest => BannerTimelineEventType::TrainerSkillsTest,
                NewsTimelineKind::FactorResearch => BannerTimelineEventType::FactorResearch,
                NewsTimelineKind::StrongestTeam => BannerTimelineEventType::StrongestTeam,
                NewsTimelineKind::RacingCarnival => BannerTimelineEventType::RacingCarnival,
            };
            push_family_adjustment_sample(
                &mut samples,
                event_type,
                event.start_at,
                global,
                anchors,
                observed_rate,
            );
        }
    }

    FamilyAdjustmentModels {
        models: samples
            .into_iter()
            .map(|(event_type, mut samples)| {
                samples.sort_by_key(|sample| sample.jp);
                (event_type, FamilyAdjustmentModel { samples })
            })
            .collect(),
    }
}

fn push_family_adjustment_sample(
    samples: &mut BTreeMap<BannerTimelineEventType, Vec<FamilyAdjustmentSample>>,
    event_type: BannerTimelineEventType,
    jp_date: DateTime<Utc>,
    confirmed_global_date: DateTime<Utc>,
    anchors: &[CalibrationAnchor],
    observed_rate: f64,
) {
    let baseline = calculate_global_date(jp_date, None, anchors, observed_rate).global_date;
    samples
        .entry(event_type)
        .or_default()
        .push(FamilyAdjustmentSample {
            jp: normalize_to_midnight_utc(jp_date),
            adjustment_days: days_between(baseline, confirmed_global_date),
        });
}

fn apply_family_adjustment(
    prediction: DatePrediction,
    event_type: BannerTimelineEventType,
    jp_date: DateTime<Utc>,
    family_adjustments: &FamilyAdjustmentModels,
) -> DatePrediction {
    if prediction.kind == PredictionKind::Confirmed {
        return prediction;
    }

    let adjustment_days = family_adjustments.adjustment_days(event_type, jp_date);
    adjust_prediction(prediction, adjustment_days)
}

fn adjust_prediction(mut prediction: DatePrediction, adjustment_days: i64) -> DatePrediction {
    if adjustment_days == 0 {
        return prediction;
    }

    prediction.global_date += Duration::days(adjustment_days);
    prediction.schedule_adjustment_days =
        Some(prediction.schedule_adjustment_days.unwrap_or_default() + adjustment_days);
    prediction
}

impl FamilyAdjustmentModels {
    fn adjustment_days(&self, event_type: BannerTimelineEventType, jp_date: DateTime<Utc>) -> i64 {
        self.models
            .get(&event_type)
            .map(|model| model.adjustment_days(jp_date))
            .unwrap_or_default()
    }
}

impl FamilyAdjustmentModel {
    fn adjustment_days(&self, jp_date: DateTime<Utc>) -> i64 {
        if self.samples.is_empty() {
            return 0;
        }

        let jp_date = normalize_to_midnight_utc(jp_date);
        let before = self
            .samples
            .iter()
            .rev()
            .find(|sample| sample.jp <= jp_date);
        let after = self.samples.iter().find(|sample| sample.jp > jp_date);

        if let (Some(before), Some(after)) = (before, after) {
            let jp_range = days_between(before.jp, after.jp);
            if jp_range > 0 {
                let jp_progress = days_between(before.jp, jp_date);
                let adjustment = before.adjustment_days as f64
                    + (after.adjustment_days - before.adjustment_days) as f64 * jp_progress as f64
                        / jp_range as f64;
                return adjustment.round() as i64;
            }
        }

        let nearby_samples = if let Some(before) = before {
            self.samples
                .iter()
                .filter(|sample| sample.jp <= before.jp)
                .rev()
                .take(FAMILY_ADJUSTMENT_SAMPLE_LIMIT)
                .collect::<Vec<_>>()
        } else {
            self.samples
                .iter()
                .take(FAMILY_ADJUSTMENT_SAMPLE_LIMIT)
                .collect::<Vec<_>>()
        };

        weighted_adjustment(&nearby_samples, jp_date)
    }
}

fn weighted_adjustment(samples: &[&FamilyAdjustmentSample], jp_date: DateTime<Utc>) -> i64 {
    if samples.is_empty() {
        return 0;
    }

    let mut weighted_adjustment = 0.0;
    let mut total_weight = 0.0;
    for sample in samples {
        let distance = days_between(sample.jp, jp_date).abs() as f64;
        let weight = 1.0 / (distance + 1.0);
        weighted_adjustment += sample.adjustment_days as f64 * weight;
        total_weight += weight;
    }

    if total_weight == 0.0 {
        0
    } else {
        (weighted_adjustment / total_weight).round() as i64
    }
}

fn apply_closed_schedule_adjustment(
    events: &mut [BannerTimelineEvent],
    confirmed_dates: &ConfirmedDateLookup,
) {
    let Some(schedule_floor) = latest_closed_global_month(confirmed_dates)
        .map(|(year, month)| first_release_after_global_month(year, month))
    else {
        return;
    };

    let Some(latest_confirmed_event) = events
        .iter()
        .filter(|event| event.is_confirmed)
        .max_by_key(|event| (event.global_release_date, event.jp_release_date))
    else {
        return;
    };

    let latest_confirmed_jp = normalize_to_midnight_utc(latest_confirmed_event.jp_release_date);
    let first_unconfirmed = events
        .iter()
        .filter(|event| {
            !event.is_confirmed
                && normalize_to_midnight_utc(event.jp_release_date) >= latest_confirmed_jp
                && event.global_release_date < schedule_floor
        })
        .min_by_key(|event| (event.global_release_date, event.jp_release_date));

    let Some(first_unconfirmed) = first_unconfirmed else {
        return;
    };

    let first_unconfirmed_jp = normalize_to_midnight_utc(first_unconfirmed.jp_release_date);
    let shift = schedule_floor - first_unconfirmed.global_release_date;
    if shift <= Duration::zero() {
        return;
    }

    let adjustment_days = shift.num_days();
    for event in events.iter_mut().filter(|event| {
        !event.is_confirmed
            && normalize_to_midnight_utc(event.jp_release_date) >= first_unconfirmed_jp
    }) {
        event.global_release_date += shift;
        event.estimated_end_date += shift;
        event.prediction.schedule_adjustment_days = Some(
            event
                .prediction
                .schedule_adjustment_days
                .unwrap_or_default()
                + adjustment_days,
        );
    }
}

fn apply_grouped_event_adjustment(events: &mut [BannerTimelineEvent]) {
    let patterns = grouped_event_patterns(events);
    if patterns.is_empty() {
        return;
    }

    let character_events = events
        .iter()
        .filter(|event| event.event_type == BannerTimelineEventType::CharacterBanner)
        .map(|event| {
            (
                normalize_to_midnight_utc(event.jp_release_date),
                event.global_release_date,
            )
        })
        .collect::<Vec<_>>();

    for event in events
        .iter_mut()
        .filter(|event| !event.is_confirmed && is_groupable_event_type(event.event_type))
    {
        let event_jp = normalize_to_midnight_utc(event.jp_release_date);
        let Some((character_jp, character_global)) =
            nearest_character_release(event_jp, &character_events)
        else {
            continue;
        };

        let jp_offset = days_between(character_jp, event_jp);
        let Some(global_offset) = patterns.get(&(event.event_type, jp_offset)).copied() else {
            continue;
        };

        let grouped_global = character_global + Duration::days(global_offset);
        let shift = grouped_global - event.global_release_date;
        if shift == Duration::zero() {
            continue;
        }

        event.global_release_date += shift;
        event.estimated_end_date += shift;
        event.prediction.schedule_adjustment_days = Some(
            event
                .prediction
                .schedule_adjustment_days
                .unwrap_or_default()
                + shift.num_days(),
        );
    }
}

fn grouped_event_patterns(
    events: &[BannerTimelineEvent],
) -> BTreeMap<(BannerTimelineEventType, i64), i64> {
    let confirmed_character_events = events
        .iter()
        .filter(|event| {
            event.event_type == BannerTimelineEventType::CharacterBanner && event.is_confirmed
        })
        .map(|event| {
            (
                normalize_to_midnight_utc(event.jp_release_date),
                event.global_release_date,
            )
        })
        .collect::<Vec<_>>();

    let mut samples = BTreeMap::<(BannerTimelineEventType, i64, i64), usize>::new();
    for event in events
        .iter()
        .filter(|event| event.is_confirmed && is_groupable_event_type(event.event_type))
    {
        let event_jp = normalize_to_midnight_utc(event.jp_release_date);
        let Some((character_jp, character_global)) =
            nearest_character_release(event_jp, &confirmed_character_events)
        else {
            continue;
        };
        let jp_offset = days_between(character_jp, event_jp);
        let global_offset = days_between(character_global, event.global_release_date);
        *samples
            .entry((event.event_type, jp_offset, global_offset))
            .or_default() += 1;
    }

    let mut best = BTreeMap::<(BannerTimelineEventType, i64), (i64, usize)>::new();
    for ((event_type, jp_offset, global_offset), count) in samples {
        best.entry((event_type, jp_offset))
            .and_modify(|(best_offset, best_count)| {
                if count > *best_count {
                    *best_offset = global_offset;
                    *best_count = count;
                }
            })
            .or_insert((global_offset, count));
    }

    best.into_iter()
        .map(|(key, (global_offset, _))| (key, global_offset))
        .collect()
}

fn nearest_character_release(
    event_jp: DateTime<Utc>,
    character_events: &[(DateTime<Utc>, DateTime<Utc>)],
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    character_events
        .iter()
        .filter_map(|(character_jp, character_global)| {
            let offset = days_between(*character_jp, event_jp);
            if offset.abs() <= GROUPING_JP_WINDOW_DAYS {
                Some((offset.abs(), *character_jp, *character_global))
            } else {
                None
            }
        })
        .min_by_key(|(distance, character_jp, _)| (*distance, *character_jp))
        .map(|(_, character_jp, character_global)| (character_jp, character_global))
}

fn is_groupable_event_type(event_type: BannerTimelineEventType) -> bool {
    matches!(
        event_type,
        BannerTimelineEventType::StoryEvent
            | BannerTimelineEventType::ChampionsMeeting
            | BannerTimelineEventType::LegendRace
            | BannerTimelineEventType::Campaign
            | BannerTimelineEventType::ScenarioRelease
    )
}

fn annotate_calendar_likelihoods(
    events: &mut [BannerTimelineEvent],
    model: &CalendarLikelihoodModel,
) {
    let character_gap_by_id = character_neighbor_gaps(events);
    let character_counts_by_month = events
        .iter()
        .filter(|event| event.event_type == BannerTimelineEventType::CharacterBanner)
        .fold(BTreeMap::<(i32, u32), usize>::new(), |mut counts, event| {
            let key = (
                event.global_release_date.year(),
                event.global_release_date.month(),
            );
            *counts.entry(key).or_default() += 1;
            counts
        });

    for event in events.iter_mut().filter(|event| !event.is_confirmed) {
        let month_key = (
            event.global_release_date.year(),
            event.global_release_date.month(),
        );
        let month_count = character_counts_by_month
            .get(&month_key)
            .copied()
            .unwrap_or_default();
        let weekday = event.global_release_date.weekday().num_days_from_monday();
        let day_of_month = event.global_release_date.day();
        let month_probability = model.month_count_probability(month_count);
        let weekday_probability = model.weekday_probability(event.event_type, weekday);
        let day_probability = model.month_day_probability(event.event_type, day_of_month);
        let (previous_gap_days, next_gap_days) = character_gap_by_id
            .get(&event.id)
            .copied()
            .unwrap_or((None, None));
        let previous_gap_probability =
            previous_gap_days.map(|gap| model.character_gap_probability(gap));
        let next_gap_probability = next_gap_days.map(|gap| model.character_gap_probability(gap));
        let mut score_factors = vec![month_probability, weekday_probability, day_probability];
        if let Some(probability) = previous_gap_probability {
            score_factors.push(probability);
        }
        if let Some(probability) = next_gap_probability {
            score_factors.push(probability);
        }
        let score = geometric_mean(&score_factors);

        event.prediction.calendar_likelihood = Some(CalendarLikelihood {
            month_character_banner_count: month_count,
            month_character_banner_count_probability: round_probability(month_probability),
            weekday: weekday_name(weekday).to_string(),
            weekday_probability: round_probability(weekday_probability),
            day_of_month,
            day_of_month_probability: round_probability(day_probability),
            previous_character_gap_days: previous_gap_days,
            previous_character_gap_probability: previous_gap_probability.map(round_probability),
            next_character_gap_days: next_gap_days,
            next_character_gap_probability: next_gap_probability.map(round_probability),
            score: round_probability(score),
        });
    }
}

fn character_neighbor_gaps(
    events: &[BannerTimelineEvent],
) -> BTreeMap<String, (Option<i64>, Option<i64>)> {
    let mut character_events = events
        .iter()
        .filter(|event| event.event_type == BannerTimelineEventType::CharacterBanner)
        .collect::<Vec<_>>();
    character_events.sort_by_key(|event| (event.global_release_date, event.jp_release_date));

    let mut gaps = BTreeMap::new();
    for (index, event) in character_events.iter().enumerate() {
        let previous_gap = index.checked_sub(1).map(|previous_index| {
            days_between(
                character_events[previous_index].global_release_date,
                event.global_release_date,
            )
        });
        let next_gap = character_events.get(index + 1).map(|next_event| {
            days_between(event.global_release_date, next_event.global_release_date)
        });
        gaps.insert(event.id.clone(), (previous_gap, next_gap));
    }

    gaps
}

impl CalendarLikelihoodModel {
    fn from_events(events: &[BannerTimelineEvent]) -> Self {
        let confirmed_character_dates = events
            .iter()
            .filter(|event| {
                event.event_type == BannerTimelineEventType::CharacterBanner && event.is_confirmed
            })
            .map(|event| event.global_release_date)
            .collect::<Vec<_>>();

        let mut character_counts_by_month = BTreeMap::<(i32, u32), usize>::new();
        let mut character_gap_counts = BTreeMap::<usize, usize>::new();
        let mut weekday_counts = BTreeMap::<u32, usize>::new();
        let mut month_day_counts = BTreeMap::<u32, usize>::new();
        let mut event_type_weekday_counts =
            BTreeMap::<BannerTimelineEventType, BTreeMap<u32, usize>>::new();
        let mut event_type_month_day_counts =
            BTreeMap::<BannerTimelineEventType, BTreeMap<u32, usize>>::new();
        let mut event_type_totals = BTreeMap::<BannerTimelineEventType, usize>::new();

        for event in events.iter().filter(|event| event.is_confirmed) {
            let weekday = event.global_release_date.weekday().num_days_from_monday();
            let day = event.global_release_date.day();
            *event_type_totals.entry(event.event_type).or_default() += 1;
            *event_type_weekday_counts
                .entry(event.event_type)
                .or_default()
                .entry(weekday)
                .or_default() += 1;
            *event_type_month_day_counts
                .entry(event.event_type)
                .or_default()
                .entry(day)
                .or_default() += 1;
        }

        for date in &confirmed_character_dates {
            if !is_global_launch_month(*date) {
                *character_counts_by_month
                    .entry((date.year(), date.month()))
                    .or_default() += 1;
            }
            *weekday_counts
                .entry(date.weekday().num_days_from_monday())
                .or_default() += 1;
            *month_day_counts.entry(date.day()).or_default() += 1;
        }

        let mut sorted_confirmed_dates = confirmed_character_dates.clone();
        sorted_confirmed_dates.sort();
        for window in sorted_confirmed_dates.windows(2) {
            if window.iter().any(|date| is_global_launch_month(*date)) {
                continue;
            }
            let gap = days_between(window[0], window[1]);
            if gap >= 0 {
                *character_gap_counts.entry(gap as usize).or_default() += 1;
            }
        }

        let month_count_frequency = character_counts_by_month.values().fold(
            BTreeMap::<usize, usize>::new(),
            |mut frequency, count| {
                *frequency.entry(*count).or_default() += 1;
                frequency
            },
        );

        let month_total = character_counts_by_month.len();
        let character_gap_total = character_gap_counts.values().sum::<usize>();
        let weekday_total = confirmed_character_dates.len();
        let month_day_total = confirmed_character_dates.len();

        let month_count_probabilities = probability_map(&month_count_frequency, month_total);
        let character_gap_probabilities =
            probability_map(&character_gap_counts, character_gap_total);
        let weekday_probabilities = probability_map(&weekday_counts, weekday_total);
        let month_day_probabilities = probability_map(&month_day_counts, month_day_total);
        let event_type_likelihoods = event_type_calendar_likelihoods(
            &event_type_weekday_counts,
            &event_type_month_day_counts,
            &event_type_totals,
        );
        let mut event_type_weekday_probabilities = BTreeMap::new();
        let mut event_type_month_day_probabilities = BTreeMap::new();
        let mut default_event_type_weekday_probabilities = BTreeMap::new();
        let mut default_event_type_month_day_probabilities = BTreeMap::new();

        for event_type in timeline_event_types() {
            let total = event_type_totals
                .get(&event_type)
                .copied()
                .unwrap_or_default();
            if let Some(counts) = event_type_weekday_counts.get(&event_type) {
                event_type_weekday_probabilities.insert(event_type, probability_map(counts, total));
            }
            if let Some(counts) = event_type_month_day_counts.get(&event_type) {
                event_type_month_day_probabilities
                    .insert(event_type, probability_map(counts, total));
            }
            default_event_type_weekday_probabilities
                .insert(event_type, default_probability(total, 7));
            default_event_type_month_day_probabilities
                .insert(event_type, default_probability(total, 31));
        }

        CalendarLikelihoodModel {
            month_count_likelihoods: count_likelihoods(&month_count_frequency, month_total),
            character_gap_likelihoods: count_likelihoods(
                &character_gap_counts,
                character_gap_total,
            ),
            weekday_likelihoods: weekday_likelihoods(&weekday_counts, weekday_total),
            month_day_likelihoods: month_day_likelihoods(&month_day_counts, month_day_total),
            event_type_likelihoods,
            default_month_count_probability: default_probability(
                month_total,
                month_count_frequency.len(),
            ),
            default_character_gap_probability: default_probability(
                character_gap_total,
                character_gap_counts.len(),
            ),
            default_weekday_probability: default_probability(weekday_total, 7),
            default_month_day_probability: default_probability(month_day_total, 31),
            month_count_probabilities,
            character_gap_probabilities,
            weekday_probabilities,
            month_day_probabilities,
            event_type_weekday_probabilities,
            event_type_month_day_probabilities,
            default_event_type_weekday_probabilities,
            default_event_type_month_day_probabilities,
        }
    }

    fn month_count_probability(&self, count: usize) -> f64 {
        self.month_count_probabilities
            .get(&count)
            .copied()
            .unwrap_or(self.default_month_count_probability)
    }

    fn character_gap_probability(&self, gap_days: i64) -> f64 {
        usize::try_from(gap_days)
            .ok()
            .and_then(|gap| self.character_gap_probabilities.get(&gap).copied())
            .unwrap_or(self.default_character_gap_probability)
    }

    fn weekday_probability(&self, event_type: BannerTimelineEventType, weekday: u32) -> f64 {
        match self.event_type_weekday_probabilities.get(&event_type) {
            Some(probabilities) if !probabilities.is_empty() => {
                probabilities.get(&weekday).copied().unwrap_or_else(|| {
                    self.default_event_type_weekday_probabilities
                        .get(&event_type)
                        .copied()
                        .unwrap_or(self.default_weekday_probability)
                })
            }
            _ => self
                .weekday_probabilities
                .get(&weekday)
                .copied()
                .unwrap_or(self.default_weekday_probability),
        }
    }

    fn month_day_probability(&self, event_type: BannerTimelineEventType, day: u32) -> f64 {
        match self.event_type_month_day_probabilities.get(&event_type) {
            Some(probabilities) if !probabilities.is_empty() => {
                probabilities.get(&day).copied().unwrap_or_else(|| {
                    self.default_event_type_month_day_probabilities
                        .get(&event_type)
                        .copied()
                        .unwrap_or(self.default_month_day_probability)
                })
            }
            _ => self
                .month_day_probabilities
                .get(&day)
                .copied()
                .unwrap_or(self.default_month_day_probability),
        }
    }
}

fn event_type_calendar_likelihoods(
    weekday_counts: &BTreeMap<BannerTimelineEventType, BTreeMap<u32, usize>>,
    month_day_counts: &BTreeMap<BannerTimelineEventType, BTreeMap<u32, usize>>,
    totals: &BTreeMap<BannerTimelineEventType, usize>,
) -> Vec<EventTypeCalendarLikelihood> {
    timeline_event_types()
        .iter()
        .copied()
        .map(|event_type| {
            let samples = totals.get(&event_type).copied().unwrap_or_default();
            let weekday_likelihoods = weekday_counts
                .get(&event_type)
                .map(|counts| weekday_likelihoods(counts, samples))
                .unwrap_or_default();
            let month_day_likelihoods = month_day_counts
                .get(&event_type)
                .map(|counts| month_day_likelihoods(counts, samples))
                .unwrap_or_default();

            EventTypeCalendarLikelihood {
                event_type,
                samples,
                weekday_likelihoods,
                month_day_likelihoods,
            }
        })
        .collect()
}

fn probability_map<K>(counts: &BTreeMap<K, usize>, total: usize) -> BTreeMap<K, f64>
where
    K: Copy + Ord,
{
    if total == 0 {
        return BTreeMap::new();
    }

    counts
        .iter()
        .map(|(value, samples)| (*value, *samples as f64 / total as f64))
        .collect()
}

fn geometric_mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    values
        .iter()
        .product::<f64>()
        .powf(1.0 / values.len() as f64)
}

fn count_likelihoods(counts: &BTreeMap<usize, usize>, total: usize) -> Vec<CountLikelihood> {
    counts
        .iter()
        .map(|(value, samples)| CountLikelihood {
            value: *value,
            samples: *samples,
            probability: probability(*samples, total),
        })
        .collect()
}

fn weekday_likelihoods(counts: &BTreeMap<u32, usize>, total: usize) -> Vec<NamedLikelihood> {
    (0..7)
        .filter_map(|weekday| {
            counts.get(&weekday).map(|samples| NamedLikelihood {
                value: weekday_name(weekday).to_string(),
                samples: *samples,
                probability: probability(*samples, total),
            })
        })
        .collect()
}

fn month_day_likelihoods(counts: &BTreeMap<u32, usize>, total: usize) -> Vec<CountLikelihood> {
    counts
        .iter()
        .map(|(value, samples)| CountLikelihood {
            value: *value as usize,
            samples: *samples,
            probability: probability(*samples, total),
        })
        .collect()
}

fn probability(samples: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        round_probability(samples as f64 / total as f64)
    }
}

fn default_probability(total: usize, observed_buckets: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        1.0 / (total + observed_buckets + 1) as f64
    }
}

fn round_probability(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn weekday_name(weekday: u32) -> &'static str {
    match weekday {
        0 => "monday",
        1 => "tuesday",
        2 => "wednesday",
        3 => "thursday",
        4 => "friday",
        5 => "saturday",
        6 => "sunday",
        _ => "unknown",
    }
}

fn is_global_launch_month(date: DateTime<Utc>) -> bool {
    date.year() == GLOBAL_LAUNCH_YEAR && date.month() == GLOBAL_LAUNCH_MONTH
}

fn build_banner_confirmed_anchors(
    character_banners: &[TimelineCharacterBanner],
    support_banners: &[TimelineSupportBanner],
    confirmed_dates: &ConfirmedDateLookup,
) -> Vec<CalibrationAnchor> {
    let mut anchors = Vec::new();

    for banner in character_banners {
        if let Some(global) = confirmed_dates.character.get(&banner.gacha_id).copied() {
            anchors.push(CalibrationAnchor {
                jp: normalize_to_midnight_utc(banner.start_at),
                global,
            });
        }
    }

    for banner in support_banners {
        if let Some(global) = confirmed_dates.support.get(&banner.gacha_id).copied() {
            anchors.push(CalibrationAnchor {
                jp: normalize_to_midnight_utc(banner.start_at),
                global,
            });
        }
    }

    anchors
}

fn build_anniversary_schedule_anchors(
    base_anchors: &[CalibrationAnchor],
    confirmed_dates: &ConfirmedDateLookup,
) -> Vec<CalibrationAnchor> {
    let mut anchors = base_anchors.to_vec();

    for (&index, &global) in &confirmed_dates.anniversary {
        anchors.push(CalibrationAnchor {
            jp: projected_jp_anniversary_date(index),
            global,
        });
    }

    monotonic_schedule_anchors(&anchors)
}

fn build_confirmed_date_lookup(
    character_banners: &[CharacterBanner],
    support_banners: &[SupportBanner],
    paid_banners: &[PaidBanner],
    timeline_character_banners: &[TimelineCharacterBanner],
    timeline_support_banners: &[TimelineSupportBanner],
    timeline_paid_banners: &[TimelinePaidBanner],
    additional_gacha_banners: &[AdditionalGachaBanner],
) -> Result<ConfirmedDateLookup> {
    let mut lookup = ConfirmedDateLookup {
        character: BTreeMap::new(),
        support: BTreeMap::new(),
        paid: BTreeMap::new(),
        story: BTreeMap::new(),
        champions: BTreeMap::new(),
        legend: BTreeMap::new(),
        campaign: BTreeMap::new(),
        news_events: BTreeMap::new(),
        anniversary: BTreeMap::new(),
        closed_global_months: BTreeSet::new(),
    };

    for banner in character_banners {
        if timeline_character_banners
            .iter()
            .any(|timeline_banner| character_banner_matches_timeline(banner, timeline_banner))
            || additional_gacha_banners.iter().any(|additional| {
                additional.kind == AdditionalGachaKind::Character
                    && additional.gacha_id == banner.gacha_id
                    && matching_pickup_candidates(
                        &banner.pickup_card_ids,
                        &additional.pickup_card_ids,
                    )
            })
        {
            lookup.character.insert(banner.gacha_id, banner.start_at);
        }
    }
    for banner in support_banners {
        if timeline_support_banners
            .iter()
            .any(|timeline_banner| support_banner_matches_timeline(banner, timeline_banner))
            || additional_gacha_banners.iter().any(|additional| {
                additional.kind == AdditionalGachaKind::Support
                    && additional.gacha_id == banner.gacha_id
                    && matching_pickup_candidates(
                        &banner.pickup_card_ids,
                        &additional.pickup_card_ids,
                    )
            })
        {
            lookup.support.insert(banner.gacha_id, banner.start_at);
        }
    }
    for banner in paid_banners {
        if timeline_paid_banners
            .iter()
            .any(|timeline_banner| paid_banner_matches_timeline(banner, timeline_banner))
        {
            lookup.paid.insert(banner.gacha_id, banner.start_at);
        }
    }

    for confirmed_dates_csv in confirmed_banner_dates_csv_sources()? {
        for confirmed_date in parse_confirmed_banner_dates(&confirmed_dates_csv)? {
            lookup.closed_global_months.insert((
                confirmed_date.global_date.year(),
                confirmed_date.global_date.month(),
            ));
            match confirmed_date.kind {
                ConfirmedTimelineKind::Character => {
                    if let Some(gacha_id) = parse_confirmed_banner_id(&confirmed_date.key) {
                        lookup
                            .character
                            .insert(gacha_id, confirmed_date.global_date);
                    }
                }
                ConfirmedTimelineKind::Support => {
                    if let Some(gacha_id) = parse_confirmed_banner_id(&confirmed_date.key) {
                        lookup.support.insert(gacha_id, confirmed_date.global_date);
                    }
                }
                ConfirmedTimelineKind::Paid => {
                    if let Some(gacha_id) = parse_confirmed_banner_id(&confirmed_date.key) {
                        lookup.paid.insert(gacha_id, confirmed_date.global_date);
                    }
                }
                ConfirmedTimelineKind::Story => {
                    lookup
                        .story
                        .insert(image_key(&confirmed_date.key), confirmed_date.global_date);
                }
                ConfirmedTimelineKind::Champions => {
                    lookup.champions.insert(
                        indexed_event_key("champions_meeting", &confirmed_date.key),
                        confirmed_date.global_date,
                    );
                }
                ConfirmedTimelineKind::Legend => {
                    lookup.legend.insert(
                        indexed_event_key("legend_race", &confirmed_date.key),
                        confirmed_date.global_date,
                    );
                }
                ConfirmedTimelineKind::Campaign => {
                    lookup
                        .campaign
                        .insert(image_key(&confirmed_date.key), confirmed_date.global_date);
                }
                ConfirmedTimelineKind::LeagueOfHeroes
                | ConfirmedTimelineKind::MastersChallenge
                | ConfirmedTimelineKind::TrainerSkillsTest
                | ConfirmedTimelineKind::FactorResearch
                | ConfirmedTimelineKind::StrongestTeam
                | ConfirmedTimelineKind::RacingCarnival
                | ConfirmedTimelineKind::TrainingScenario => {
                    lookup
                        .news_events
                        .insert(confirmed_date.key, confirmed_date.global_date);
                }
                ConfirmedTimelineKind::Anniversary => {
                    if let Some(index) = parse_confirmed_anniversary_index(&confirmed_date.key) {
                        lookup.anniversary.insert(index, confirmed_date.global_date);
                    }
                }
            }
        }
    }

    Ok(lookup)
}

fn matching_pickup_candidates(actual: &[i64], candidates: &[i64]) -> bool {
    !actual.is_empty()
        && !candidates.is_empty()
        && actual.iter().all(|card_id| candidates.contains(card_id))
}

fn character_banner_matches_timeline(
    banner: &CharacterBanner,
    timeline_banner: &TimelineCharacterBanner,
) -> bool {
    banner.gacha_id == timeline_banner.gacha_id
        && same_pickup_card_ids(&banner.pickup_card_ids, &timeline_banner.pickup_card_ids)
}

fn support_banner_matches_timeline(
    banner: &SupportBanner,
    timeline_banner: &TimelineSupportBanner,
) -> bool {
    banner.gacha_id == timeline_banner.gacha_id
        && same_pickup_card_ids(&banner.pickup_card_ids, &timeline_banner.pickup_card_ids)
}

fn paid_banner_matches_timeline(banner: &PaidBanner, timeline_banner: &TimelinePaidBanner) -> bool {
    banner.gacha_id == timeline_banner.gacha_id
        && banner.gacha_type == timeline_banner.gacha_type
        && banner.card_type == timeline_banner.card_type
        && same_pickup_card_ids(&banner.pickup_card_ids, &timeline_banner.pickup_card_ids)
}

fn same_pickup_card_ids(left: &[i64], right: &[i64]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut left = left.to_vec();
    left.sort_unstable();
    let mut right = right.to_vec();
    right.sort_unstable();
    left == right
}

pub(crate) fn confirmed_dates_version_hash() -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "timeline_algorithm_version={}\n",
        TIMELINE_ALGORITHM_VERSION
    ));
    hasher.update(
        confirmed_banner_dates_csv_sources()?
            .join("\n# external confirmed dates\n")
            .as_bytes(),
    );
    hasher.update(b"\n# bundled mission campaigns\n");
    hasher.update(BUNDLED_TIMELINE_CAMPAIGNS_JSON);
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) fn validate_confirmed_dates_csv(input: &str) -> Result<()> {
    parse_confirmed_banner_dates(input).map(|_| ())
}

fn confirmed_banner_dates_csv_sources() -> Result<Vec<String>> {
    let mut sources = vec![CONFIRMED_GLOBAL_BANNER_DATES_CSV.to_string()];

    if let Ok(path) = env::var("CONFIRMED_BANNER_DATES_PATH") {
        let path = path.trim();
        if !path.is_empty() && Path::new(path).exists() {
            sources.push(
                fs::read_to_string(path).with_context(|| {
                    format!("failed to read confirmed banner dates from {path}")
                })?,
            );
        }
    }

    Ok(sources)
}

fn parse_confirmed_banner_dates(input: &str) -> Result<Vec<ConfirmedTimelineDate>> {
    let mut dates = Vec::new();

    for (line_index, line) in input.lines().enumerate() {
        let line_number = line_index + 1;
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }

        let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 3 {
            anyhow::bail!(
                "confirmed_global_banner_dates.csv line {line_number} must be type,banner,date"
            );
        }

        let kind = parse_confirmed_timeline_kind(fields[0]).with_context(|| {
            format!(
                "invalid confirmed timeline type on line {line_number}: {}",
                fields[0]
            )
        })?;
        let key = parse_confirmed_timeline_key(kind, fields[1]).with_context(|| {
            format!(
                "invalid confirmed timeline key on line {line_number}: {}",
                fields[1]
            )
        })?;

        dates.push(ConfirmedTimelineDate {
            kind,
            key,
            global_date: parse_confirmed_global_date(fields[2]).with_context(|| {
                format!(
                    "invalid confirmed timeline date on line {line_number}: {}",
                    fields[2]
                )
            })?,
        });
    }

    Ok(dates)
}

fn parse_confirmed_timeline_kind(value: &str) -> Option<ConfirmedTimelineKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "character" | "char" => Some(ConfirmedTimelineKind::Character),
        "support" | "support_card" | "support-card" => Some(ConfirmedTimelineKind::Support),
        "paid" | "paid_gacha" | "paid-gacha" => Some(ConfirmedTimelineKind::Paid),
        "story" | "story_event" | "story-event" => Some(ConfirmedTimelineKind::Story),
        "champions" | "champions_meeting" | "champions-meeting" | "champion" => {
            Some(ConfirmedTimelineKind::Champions)
        }
        "legend" | "legend_race" | "legend-race" => Some(ConfirmedTimelineKind::Legend),
        "campaign" | "mission_campaign" | "mission-campaign" => {
            Some(ConfirmedTimelineKind::Campaign)
        }
        "league_of_heroes" | "league-of-heroes" | "loh" => {
            Some(ConfirmedTimelineKind::LeagueOfHeroes)
        }
        "masters_challenge" | "masters-challenge" | "masters" => {
            Some(ConfirmedTimelineKind::MastersChallenge)
        }
        "trainer_skills_test" | "trainer-skills-test" | "skills_test" | "skills-test" => {
            Some(ConfirmedTimelineKind::TrainerSkillsTest)
        }
        "factor_research" | "factor-research" | "tachyon" => {
            Some(ConfirmedTimelineKind::FactorResearch)
        }
        "strongest_team" | "strongest-team" => Some(ConfirmedTimelineKind::StrongestTeam),
        "racing_carnival" | "racing-carnival" => Some(ConfirmedTimelineKind::RacingCarnival),
        "training_scenario" | "training-scenario" | "scenario" => {
            Some(ConfirmedTimelineKind::TrainingScenario)
        }
        "anniversary" | "anniv" | "half_anniversary" | "half-anniversary" => {
            Some(ConfirmedTimelineKind::Anniversary)
        }
        _ => None,
    }
}

fn parse_confirmed_timeline_key(kind: ConfirmedTimelineKind, value: &str) -> Option<String> {
    match kind {
        ConfirmedTimelineKind::Character
        | ConfirmedTimelineKind::Support
        | ConfirmedTimelineKind::Paid => parse_confirmed_banner_id(value).map(|id| id.to_string()),
        ConfirmedTimelineKind::Story | ConfirmedTimelineKind::Campaign => Some(image_key(value)),
        ConfirmedTimelineKind::Champions => Some(indexed_event_key("champions_meeting", value)),
        ConfirmedTimelineKind::Legend => Some(indexed_event_key("legend_race", value)),
        ConfirmedTimelineKind::LeagueOfHeroes => {
            Some(news_event_confirmation_key("league-of-heroes", value))
        }
        ConfirmedTimelineKind::MastersChallenge => {
            Some(news_event_confirmation_key("masters-challenge", value))
        }
        ConfirmedTimelineKind::TrainerSkillsTest => {
            Some(news_event_confirmation_key("trainer-skills-test", value))
        }
        ConfirmedTimelineKind::FactorResearch => {
            Some(news_event_confirmation_key("factor-research", value))
        }
        ConfirmedTimelineKind::StrongestTeam => {
            Some(news_event_confirmation_key("strongest-team", value))
        }
        ConfirmedTimelineKind::RacingCarnival => {
            Some(news_event_confirmation_key("racing-carnival", value))
        }
        ConfirmedTimelineKind::TrainingScenario => {
            Some(news_event_confirmation_key("training-scenario", value))
        }
        ConfirmedTimelineKind::Anniversary => {
            parse_confirmed_anniversary_index(value).map(|index| index.to_string())
        }
    }
}

fn news_event_confirmation_key(prefix: &str, value: &str) -> String {
    let value = value.trim().to_ascii_lowercase().replace('_', "-");
    if value.starts_with(prefix) {
        value
    } else {
        format!("{prefix}-{value}")
    }
}

fn parse_confirmed_anniversary_index(value: &str) -> Option<u32> {
    let key = image_key(value)
        .replace("half_anniversary_", "")
        .replace("anniversary_", "");
    let index = key.parse::<u32>().ok()?;
    (index > 0).then_some(index)
}

fn parse_confirmed_banner_id(value: &str) -> Option<i64> {
    gacha_id_from_image_stem(image_stem(value.trim()))
}

fn image_key(value: &str) -> String {
    image_stem(value.trim()).to_ascii_lowercase()
}

fn indexed_event_key(prefix: &str, value: &str) -> String {
    let value = image_key(value);
    if value.starts_with(prefix) {
        value
    } else {
        format!("{prefix}_{value}")
    }
}

fn champions_key(index: usize) -> String {
    format!("champions_meeting_{index}")
}

fn legend_key(index: usize) -> String {
    format!("legend_race_{index}")
}

fn parse_confirmed_global_date(value: &str) -> Result<DateTime<Utc>> {
    if value.contains('T') {
        return Ok(DateTime::parse_from_rfc3339(value)
            .with_context(|| format!("failed to parse confirmed ISO date {value}"))?
            .with_timezone(&Utc));
    }

    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .with_context(|| format!("confirmed date must be YYYY-MM-DD or RFC3339: {value}"))?;
    Ok(utc_date(date.year(), date.month(), date.day(), 22))
}

fn gacha_id_from_image_stem(stem: &str) -> Option<i64> {
    stem.rsplit('_').next().unwrap_or(stem).parse().ok()
}

fn calculate_global_date(
    jp_date: DateTime<Utc>,
    confirmed_global_date: Option<DateTime<Utc>>,
    anchors: &[CalibrationAnchor],
    observed_rate: f64,
) -> DatePrediction {
    if let Some(global_date) = confirmed_global_date {
        return DatePrediction {
            global_date,
            kind: PredictionKind::Confirmed,
            acceleration_rate: None,
            schedule_adjustment_days: None,
            anchor_jp_date: None,
            anchor_global_date: None,
        };
    }

    let normalized_jp_date = normalize_to_midnight_utc(jp_date);
    if anchors.is_empty() {
        return fallback_prediction(normalized_jp_date);
    }

    let mut sorted_by_jp = anchors.to_vec();
    sorted_by_jp.sort_by_key(|anchor| (anchor.jp, anchor.global));
    let most_recent_anchor = anchors
        .iter()
        .max_by_key(|anchor| anchor.global)
        .expect("anchors should not be empty");

    let mut before = None;
    let mut after = None;
    for anchor in &sorted_by_jp {
        if anchor.jp <= normalized_jp_date {
            before = Some(anchor);
        } else {
            after = Some(anchor);
            break;
        }
    }

    if let (Some(before), Some(after)) = (before, after) {
        let jp_range = (after.jp - before.jp).num_milliseconds();
        if jp_range > 0 {
            let global_range = (after.global - before.global).num_milliseconds();
            let jp_progress = (normalized_jp_date - before.jp).num_milliseconds();
            let global_progress =
                ((global_range as f64) * (jp_progress as f64) / (jp_range as f64)).round() as i64;
            let calculated = before.global + Duration::milliseconds(global_progress);
            return DatePrediction {
                global_date: normalize_to_release_hour(calculated),
                kind: PredictionKind::Interpolated,
                acceleration_rate: Some(round_rate(observed_rate)),
                schedule_adjustment_days: None,
                anchor_jp_date: Some(before.jp),
                anchor_global_date: Some(before.global),
            };
        }
    }

    if let Some(before) = before {
        let anchor = if normalized_jp_date >= most_recent_anchor.jp {
            most_recent_anchor
        } else {
            before
        };
        let jp_days_after = days_between(anchor.jp, normalized_jp_date);
        let global_days_after = (jp_days_after as f64 / observed_rate).round() as i64;
        let calculated = anchor.global + Duration::days(global_days_after);
        return DatePrediction {
            global_date: normalize_to_release_hour(calculated),
            kind: PredictionKind::Extrapolated,
            acceleration_rate: Some(round_rate(observed_rate)),
            schedule_adjustment_days: None,
            anchor_jp_date: Some(anchor.jp),
            anchor_global_date: Some(anchor.global),
        };
    }

    if let Some(after) = after {
        let jp_days_before = days_between(normalized_jp_date, after.jp);
        let global_days_before = (jp_days_before as f64 / observed_rate).round() as i64;
        let calculated = after.global - Duration::days(global_days_before);
        return DatePrediction {
            global_date: normalize_to_release_hour(calculated),
            kind: PredictionKind::Extrapolated,
            acceleration_rate: Some(round_rate(observed_rate)),
            schedule_adjustment_days: None,
            anchor_jp_date: Some(after.jp),
            anchor_global_date: Some(after.global),
        };
    }

    fallback_prediction(normalized_jp_date)
}

fn fallback_prediction(jp_date: DateTime<Utc>) -> DatePrediction {
    let days_since_jp_launch = days_between(jp_launch_date(), jp_date);
    let adjusted_days = (days_since_jp_launch as f64 / FALLBACK_ACCELERATION_RATE).floor() as i64;
    let calculated = global_launch_date() + Duration::days(adjusted_days);
    DatePrediction {
        global_date: normalize_to_release_hour(calculated),
        kind: PredictionKind::Fallback,
        acceleration_rate: Some(FALLBACK_ACCELERATION_RATE),
        schedule_adjustment_days: None,
        anchor_jp_date: None,
        anchor_global_date: None,
    }
}

fn calculate_recent_acceleration_rate(anchors: &[CalibrationAnchor]) -> f64 {
    if anchors.len() < 2 {
        return FALLBACK_ACCELERATION_RATE;
    }

    let mut sorted = anchors.to_vec();
    sorted.sort_by_key(|anchor| (anchor.global, anchor.jp));
    let most_recent_global = sorted
        .last()
        .expect("anchors should have at least two values")
        .global;
    let cutoff = most_recent_global - Duration::days(RECENT_ANCHOR_WINDOW_DAYS);
    let mut dates_to_use = sorted
        .iter()
        .filter(|anchor| anchor.global >= cutoff)
        .cloned()
        .collect::<Vec<_>>();

    if dates_to_use.len() < 4 {
        dates_to_use = sorted
            .into_iter()
            .rev()
            .take(FALLBACK_RECENT_ANCHORS)
            .collect::<Vec<_>>();
        dates_to_use.reverse();
    }

    calculate_rate_from_dates(&dates_to_use).unwrap_or(FALLBACK_ACCELERATION_RATE)
}

fn calculate_rate_from_dates(dates_to_use: &[CalibrationAnchor]) -> Option<f64> {
    if dates_to_use.len() < 2 {
        return None;
    }

    let mut intervals = Vec::new();
    let mut weighted_jp_days = 0.0;
    let mut weighted_global_days = 0.0;

    for index in 1..dates_to_use.len() {
        let jp_days = days_between(dates_to_use[index - 1].jp, dates_to_use[index].jp);
        let global_days = days_between(dates_to_use[index - 1].global, dates_to_use[index].global);
        if jp_days <= 0 || global_days <= 0 {
            continue;
        }

        let weight = ((index as f64 - dates_to_use.len() as f64 + 1.0) / 8.0).exp();
        let rate = jp_days as f64 / global_days as f64;
        intervals.push((rate, weight));
        weighted_jp_days += jp_days as f64 * weight;
        weighted_global_days += global_days as f64 * weight;
    }

    if intervals.is_empty() || weighted_global_days == 0.0 {
        return None;
    }

    let aggregate_rate = weighted_jp_days / weighted_global_days;
    let median_rate = weighted_median_rate(intervals);
    let blended_rate = median_rate * 0.65 + aggregate_rate * 0.35;
    Some(clamp_rate(blended_rate))
}

fn weighted_median_rate(mut intervals: Vec<(f64, f64)>) -> f64 {
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total_weight = intervals.iter().map(|(_, weight)| *weight).sum::<f64>();
    let mut cumulative_weight = 0.0;

    for (rate, weight) in intervals {
        cumulative_weight += weight;
        if cumulative_weight >= total_weight / 2.0 {
            return rate;
        }
    }

    FALLBACK_ACCELERATION_RATE
}

fn unique_sorted_anchors(anchors: &[CalibrationAnchor]) -> Vec<CalibrationAnchor> {
    let mut seen = BTreeSet::new();
    let mut unique = Vec::new();

    for anchor in anchors {
        let key = (anchor.jp.timestamp(), anchor.global.timestamp());
        if seen.insert(key) {
            unique.push(anchor.clone());
        }
    }

    unique.sort_by_key(|anchor| (anchor.jp, anchor.global));
    unique
}

fn monotonic_schedule_anchors(anchors: &[CalibrationAnchor]) -> Vec<CalibrationAnchor> {
    let mut latest_global = None;
    let mut monotonic = Vec::new();

    for anchor in unique_sorted_anchors(anchors) {
        if latest_global.map_or(true, |global| anchor.global >= global) {
            latest_global = Some(anchor.global);
            monotonic.push(anchor);
        }
    }

    monotonic
}

fn timeline_anniversaries(
    events: &[BannerTimelineEvent],
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    observed_rate: f64,
) -> Vec<TimelineAnniversary> {
    let end_date = events
        .iter()
        .map(|event| event.global_release_date)
        .max()
        .map(|latest| latest + Duration::days(14))
        .unwrap_or_else(global_timeline_start_date);
    let latest_confirmed_jp = events
        .iter()
        .filter(|event| event.is_confirmed)
        .map(|event| normalize_to_midnight_utc(event.jp_release_date))
        .max();

    timeline_anniversaries_through(
        end_date,
        confirmed_dates,
        anchors,
        observed_rate,
        latest_confirmed_jp,
    )
}

fn timeline_anniversaries_through(
    end_date: DateTime<Utc>,
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    observed_rate: f64,
    latest_confirmed_jp: Option<DateTime<Utc>>,
) -> Vec<TimelineAnniversary> {
    let mut anniversaries = Vec::new();
    let mut index = 1;

    loop {
        let jp_date = projected_jp_anniversary_date(index);
        let confirmed_global_date = confirmed_dates.anniversary.get(&index).copied();
        let prediction =
            calculate_global_date(jp_date, confirmed_global_date, anchors, observed_rate);
        let global_date = prediction.global_date;

        if global_date > end_date {
            break;
        }

        anniversaries.push(TimelineAnniversary {
            index,
            label: anniversary_label(index),
            jp_date,
            global_date,
            is_confirmed: confirmed_global_date.is_some(),
            schedule_adjustment_days: None,
        });

        index += 1;
    }

    apply_closed_anniversary_adjustment(&mut anniversaries, confirmed_dates, latest_confirmed_jp);
    anniversaries
}

fn apply_closed_anniversary_adjustment(
    anniversaries: &mut [TimelineAnniversary],
    confirmed_dates: &ConfirmedDateLookup,
    latest_confirmed_jp: Option<DateTime<Utc>>,
) {
    let Some(schedule_floor) = latest_closed_global_month(confirmed_dates)
        .map(|(year, month)| first_release_after_global_month(year, month))
    else {
        return;
    };

    let Some(first_unconfirmed) = anniversaries
        .iter()
        .filter(|anniversary| {
            !anniversary.is_confirmed
                && latest_confirmed_jp.map_or(true, |latest_jp| {
                    normalize_to_midnight_utc(anniversary.jp_date) >= latest_jp
                })
                && anniversary.global_date < schedule_floor
        })
        .min_by_key(|anniversary| (anniversary.global_date, anniversary.index))
    else {
        return;
    };

    let first_unconfirmed_index = first_unconfirmed.index;
    let shift = schedule_floor - first_unconfirmed.global_date;
    if shift <= Duration::zero() {
        return;
    }

    let adjustment_days = shift.num_days();
    for anniversary in anniversaries.iter_mut().filter(|anniversary| {
        !anniversary.is_confirmed && anniversary.index >= first_unconfirmed_index
    }) {
        anniversary.global_date += shift;
        anniversary.schedule_adjustment_days =
            Some(anniversary.schedule_adjustment_days.unwrap_or_default() + adjustment_days);
    }
}

fn anniversary_label(index: u32) -> String {
    if index % 2 == 0 {
        format!("{} Year Anniversary", index / 2)
    } else {
        format!("{}.5 Year Anniversary", index / 2)
    }
}

fn projected_jp_anniversary_date(index: u32) -> DateTime<Utc> {
    add_calendar_months(jp_launch_date(), index * 6, 0)
}

fn add_calendar_months(date: DateTime<Utc>, months: u32, hour: u32) -> DateTime<Utc> {
    let total_months = date.year() * 12 + date.month0() as i32 + months as i32;
    let year = total_months.div_euclid(12);
    let month = total_months.rem_euclid(12) as u32 + 1;
    let day = date.day().min(days_in_month(year, month));

    utc_date(year, month, day, hour)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_of_next_month = NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .expect("next month from valid year/month should be valid");

    (first_of_next_month - Duration::days(1)).day()
}

fn image_stem(image: &str) -> &str {
    if let Some((stem, extension)) = image.rsplit_once('.') {
        if extension.eq_ignore_ascii_case("png") || extension.eq_ignore_ascii_case("webp") {
            return stem;
        }
    }

    image
}

fn webp_image_reference(image: &str) -> String {
    let stem = image_stem(image);
    if stem.is_empty() {
        String::new()
    } else {
        format!("{stem}.webp")
    }
}

impl DatePrediction {
    fn into_info(self) -> PredictionInfo {
        PredictionInfo {
            kind: self.kind,
            acceleration_rate: self.acceleration_rate,
            schedule_adjustment_days: self.schedule_adjustment_days,
            calendar_likelihood: None,
            anchor_jp_date: self.anchor_jp_date,
            anchor_global_date: self.anchor_global_date,
        }
    }
}

fn load_timeline_character_banners() -> Result<Vec<TimelineCharacterBanner>> {
    let raw_banners: Vec<RawTimelineCharacterBanner> =
        serde_json::from_slice(BUNDLED_TIMELINE_CHARACTER_BANNERS_JSON)
            .context("failed to parse bundled timeline_character_banners.json")?;

    raw_banners
        .into_iter()
        .map(|banner| {
            Ok(TimelineCharacterBanner {
                gacha_id: banner.gacha_id,
                year: banner.year,
                image: webp_image_reference(&banner.image),
                image_path: webp_image_reference(&banner.image_path),
                start_at: parse_timeline_date(&banner.start_date)?,
                end_at: parse_timeline_date(&banner.end_date)?,
                pickup_card_ids: banner.pickup_card_ids,
            })
        })
        .collect()
}

fn load_timeline_support_banners() -> Result<Vec<TimelineSupportBanner>> {
    let raw_banners: Vec<RawTimelineSupportBanner> =
        serde_json::from_slice(BUNDLED_TIMELINE_SUPPORT_BANNERS_JSON)
            .context("failed to parse bundled timeline_support_banners.json")?;

    raw_banners
        .into_iter()
        .map(|banner| {
            Ok(TimelineSupportBanner {
                gacha_id: banner.gacha_id,
                year: banner.year,
                image: webp_image_reference(&banner.image),
                start_at: parse_timeline_date(&banner.start_date)?,
                end_at: parse_timeline_date(&banner.end_date)?,
                pickup_card_ids: banner.pickup_card_ids,
            })
        })
        .collect()
}

fn load_timeline_paid_banners() -> Result<Vec<TimelinePaidBanner>> {
    let raw_banners: Vec<RawTimelinePaidBanner> =
        serde_json::from_slice(BUNDLED_TIMELINE_PAID_BANNERS_JSON)
            .context("failed to parse bundled timeline_paid_banners.json")?;

    raw_banners
        .into_iter()
        .map(|banner| {
            Ok(TimelinePaidBanner {
                gacha_id: banner.gacha_id,
                gacha_type: banner.gacha_type,
                card_type: banner.card_type,
                year: banner.year,
                image: webp_image_reference(&banner.image),
                start_at: parse_timeline_date(&banner.start_date)?,
                end_at: parse_timeline_date(&banner.end_date)?,
                pickup_card_ids: banner.pickup_card_ids,
            })
        })
        .collect()
}

fn load_timeline_story_events(
    jp_connection: Option<&Connection>,
) -> Result<Vec<TimelineStoryEvent>> {
    let raw_events: Vec<RawTimelineStoryEvent> =
        serde_json::from_slice(BUNDLED_TIMELINE_STORY_EVENTS_JSON)
            .context("failed to parse bundled timeline_story_events.json")?;

    let mut events = raw_events
        .into_iter()
        .map(|event| {
            Ok(TimelineStoryEvent {
                master_event_id: None,
                event_name: event.event_name,
                image: webp_image_reference(&event.image),
                start_at: parse_timeline_date(&event.start_date)?,
                end_at: parse_timeline_date(&event.end_date)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    append_master_story_events(jp_connection, &mut events)?;
    events.sort_by_key(|event| (event.start_at, event.master_event_id));
    Ok(events)
}

fn append_master_story_events(
    jp_connection: Option<&Connection>,
    events: &mut Vec<TimelineStoryEvent>,
) -> Result<()> {
    let Some(connection) = jp_connection else {
        return Ok(());
    };
    if !timeline_table_exists(connection, "story_event_data")? {
        return Ok(());
    }
    let latest_bundled_start = events
        .iter()
        .map(|event| event.start_at)
        .max()
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    let mut statement = connection.prepare(
        r#"
        SELECT story_event_id,
               CASE typeof(start_date)
                 WHEN 'integer' THEN start_date
                 ELSE CAST(strftime('%s', replace(start_date, '/', '-')) AS INTEGER)
               END,
               CASE typeof(end_date)
                 WHEN 'integer' THEN end_date
                 ELSE CAST(strftime('%s', replace(end_date, '/', '-')) AS INTEGER)
               END
        FROM story_event_data
        ORDER BY start_date, story_event_id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (event_id, start, end) = row?;
        let start_at = Utc
            .timestamp_opt(start, 0)
            .single()
            .with_context(|| format!("invalid story_event start timestamp {start}"))?;
        if start_at <= latest_bundled_start {
            continue;
        }
        let end_at = Utc
            .timestamp_opt(end, 0)
            .single()
            .with_context(|| format!("invalid story_event end timestamp {end}"))?;
        events.push(TimelineStoryEvent {
            master_event_id: Some(event_id),
            event_name: format!("Story Event #{event_id}"),
            image: String::new(),
            start_at,
            end_at,
        });
    }
    Ok(())
}

fn load_timeline_champions_meetings() -> Result<Vec<TimelineChampionsMeeting>> {
    let raw_events: Vec<RawTimelineChampionsMeeting> =
        serde_json::from_slice(BUNDLED_TIMELINE_CHAMPIONS_MEETING_JSON)
            .context("failed to parse bundled timeline_champions_meeting.json")?;
    let mut events = raw_events
        .into_iter()
        .map(|event| {
            Ok(TimelineChampionsMeeting {
                index: 0,
                name: event.name,
                start_at: parse_timeline_date(&event.start_date)?,
                end_at: parse_timeline_date(&event.end_date)?,
                track: event.track,
                distance: event.distance,
                conditions: event.conditions,
                image_url: None,
                image_path: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    events.sort_by_key(|event| (event.start_at, event.name.clone()));
    for (index, event) in events.iter_mut().enumerate() {
        event.index = index;
    }

    Ok(events)
}

fn merge_champions_meeting_news(
    bundled: &mut [TimelineChampionsMeeting],
    news_events: &mut Vec<NewsTimelineEvent>,
) {
    news_events.retain(|news| {
        if news.kind != NewsTimelineKind::ChampionsMeeting {
            return true;
        }
        let Some(existing) = bundled
            .iter_mut()
            .min_by_key(|event| (event.start_at - news.start_at).num_seconds().abs())
        else {
            return true;
        };
        if (existing.start_at - news.start_at).num_days().abs() > 2 {
            return true;
        }
        if existing.image_url.is_none() {
            existing.image_url = news.image_url.clone();
            existing.image_path = news.image_url.as_ref().map(|_| news_event_asset_path(news));
        }
        false
    });
}

fn reuse_champions_meeting_image_paths(events: &mut [TimelineChampionsMeeting]) {
    let reusable = events
        .iter()
        .filter_map(|event| {
            event
                .image_path
                .as_ref()
                .map(|path| (event.name.to_lowercase(), path.clone()))
        })
        .collect::<BTreeMap<_, _>>();

    for event in events {
        if event.image_path.is_none() {
            event.image_path = reusable.get(&event.name.to_lowercase()).cloned();
        }
    }
}

fn merge_campaign_news(
    campaigns: &mut [TimelineCampaign],
    news_campaigns: &[CampaignTimelineMetadata],
) {
    for campaign in campaigns {
        // Curated titles disambiguate campaign rows that share a release day. A
        // date-only news match must never replace that semantic identity.
        if campaign.title.is_some() {
            continue;
        }
        let jp_day = normalize_to_midnight_utc(campaign.start_at);
        let Some(news) = news_campaigns
            .iter()
            .filter(|news| normalize_to_midnight_utc(news.start_at) == jp_day)
            .max_by_key(|news| campaign_news_priority(&news.title))
        else {
            continue;
        };
        campaign.title = Some(news.title.clone());
        campaign.description = news.description.clone();
        if let Some(image_url) = news.image_url.clone() {
            campaign.image_url = Some(image_url);
            campaign.image_path = Some(format!(
                "assets/timeline-images/events/campaign/{}.webp",
                news.source_post_id
            ));
        }
    }
}

fn campaign_news_priority(title: &str) -> (u8, usize) {
    let title = title.to_lowercase();
    let priority = if title.contains("anniversary") {
        4
    } else if title.contains("gi campaign") || title.contains("g1 campaign") {
        3
    } else if title.contains("scenario") {
        2
    } else {
        1
    };
    (priority, title.len())
}

fn merge_legend_race_news(
    races: &mut [TimelineLegendRace],
    news_races: &[LegendRaceTimelineMetadata],
) {
    for race in races {
        let jp_day = normalize_to_midnight_utc(race.start_at);
        if let Some(news) = news_races
            .iter()
            .filter(|news| normalize_to_midnight_utc(news.start_at) == jp_day)
            .max_by_key(|news| news.source_post_id)
        {
            race.image_url = Some(news.image_url.clone());
            race.image_path = Some(format!(
                "assets/timeline-images/events/legend-race/{}.webp",
                news.source_post_id
            ));
            race.source_post_id = Some(news.source_post_id);
        } else {
            race.image_url = Some(LEGEND_RACE_FALLBACK_IMAGE_URL.to_string());
            race.image_path = Some(LEGEND_RACE_FALLBACK_IMAGE_PATH.to_string());
        }
    }
}

fn load_timeline_legend_races(
    jp_connection: Option<&Connection>,
) -> Result<Vec<TimelineLegendRace>> {
    let raw_events: Vec<RawTimelineLegendRace> =
        serde_json::from_slice(BUNDLED_TIMELINE_LEGEND_RACES_JSON)
            .context("failed to parse bundled timeline_legend_races.json")?;
    let mut events = raw_events
        .into_iter()
        .map(|event| {
            let bosses = event
                .bosses
                .unwrap_or_default()
                .into_iter()
                .filter_map(|boss| {
                    let image = boss.image?;
                    let card_id = legend_boss_card_id(&image);
                    Some(TimelineLegendBoss {
                        name: boss.name.unwrap_or_else(|| {
                            card_id
                                .map(|id| format!("Character {id}"))
                                .unwrap_or_else(|| "Unknown character".to_string())
                        }),
                        image: webp_image_reference(&image),
                        card_id,
                    })
                })
                .collect::<Vec<_>>();

            Ok(TimelineLegendRace {
                index: 0,
                race_name: event.race_name,
                start_at: parse_timeline_date(&event.start_date)?,
                end_at: parse_timeline_date(&event.end_date)?,
                course: event.course,
                bosses,
                image_url: None,
                image_path: None,
                source_post_id: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    if let Some(jp_connection) = jp_connection {
        append_master_legend_races(jp_connection, &mut events)?;
    }

    events.sort_by_key(|event| (event.start_at, event.race_name.clone()));
    for (index, event) in events.iter_mut().enumerate() {
        event.index = index;
    }

    Ok(events)
}

fn append_master_legend_races(
    connection: &Connection,
    events: &mut Vec<TimelineLegendRace>,
) -> Result<()> {
    for table in [
        "legend_race",
        "race_instance",
        "race",
        "race_course_set",
        "text_data",
    ] {
        if !timeline_table_exists(connection, table)? {
            return Ok(());
        }
    }

    let Some(latest_bundled_day) = events
        .iter()
        .map(|event| normalize_to_midnight_utc(event.start_at))
        .max()
    else {
        return Ok(());
    };
    let character_names = common::read_character_names()?;
    let mut statement = connection.prepare(
        r#"
        SELECT legend.race_instance_id,
               legend.image_id,
               legend.start_date,
               legend.end_date,
               course.distance,
               course.ground,
               race_name.text
        FROM legend_race AS legend
        JOIN race_instance AS instance
          ON instance.id = legend.race_instance_id
        JOIN race
          ON race.id = instance.race_id
        JOIN race_course_set AS course
          ON course.id = race.course_set
        LEFT JOIN text_data AS race_name
          ON race_name.category = 28
         AND race_name."index" = legend.race_instance_id
        ORDER BY legend.start_date, legend.id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, Option<String>>(6)?,
        ))
    })?;

    let mut master_events = BTreeMap::<i64, TimelineLegendRace>::new();
    for row in rows {
        let (race_instance_id, image_id, start, end, distance, ground, race_name) = row?;
        let start_at = master_timestamp(start)?;
        let end_at = master_timestamp(end)?;
        let event_key = race_instance_id / 10;
        let card_id = normalize_master_legend_card_id(image_id);
        let boss = TimelineLegendBoss {
            name: bundled_legend_boss_name(&character_names, card_id),
            image: format!("chara_stand_{card_id}.webp"),
            card_id: Some(card_id),
        };

        match master_events.entry(event_key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(TimelineLegendRace {
                    index: 0,
                    race_name: master_legend_race_title(race_name.as_deref(), event_key),
                    start_at,
                    end_at,
                    course: Some(master_legend_course(distance, ground)),
                    bosses: vec![boss],
                    image_url: None,
                    image_path: None,
                    source_post_id: None,
                });
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let event = entry.get_mut();
                event.start_at = event.start_at.min(start_at);
                event.end_at = event.end_at.max(end_at);
                if !event
                    .bosses
                    .iter()
                    .any(|existing| existing.card_id == boss.card_id)
                {
                    event.bosses.push(boss);
                }
            }
        }
    }

    events.extend(
        master_events
            .into_values()
            .filter(|event| normalize_to_midnight_utc(event.start_at) > latest_bundled_day),
    );
    Ok(())
}

fn timeline_table_exists(connection: &Connection, table: &str) -> Result<bool> {
    let count = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(count > 0)
}

fn master_timestamp(timestamp: i64) -> Result<DateTime<Utc>> {
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .with_context(|| format!("invalid legend_race master timestamp {timestamp}"))
}

fn normalize_master_legend_card_id(image_id: i64) -> i64 {
    if image_id >= 10_000_000 {
        image_id / 100
    } else {
        image_id
    }
}

fn bundled_legend_boss_name(character_names: &serde_json::Value, card_id: i64) -> String {
    let chara_id = card_id / 100;
    let skin_id = format!("{:02}", card_id % 100);
    let Some(entry) = character_names.get(chara_id.to_string()) else {
        return format!("Character {card_id}");
    };
    let Some(name) = entry.get("name").and_then(serde_json::Value::as_str) else {
        return format!("Character {card_id}");
    };
    entry
        .get("skins")
        .and_then(|skins| skins.get(&skin_id))
        .and_then(serde_json::Value::as_str)
        .map(|skin| format!("{name} ({skin})"))
        .unwrap_or_else(|| name.to_string())
}

fn master_legend_race_title(jp_title: Option<&str>, event_key: i64) -> String {
    let raw_title = jp_title.unwrap_or_default().trim();
    let jp_title = raw_title
        .strip_suffix(" HARD")
        .or_else(|| raw_title.strip_suffix("（HARD）"))
        .or_else(|| raw_title.strip_suffix("(HARD)"))
        .unwrap_or(raw_title);
    let race_name = match jp_title {
        "東京優駿（日本ダービー）" | "日本ダービー" => {
            "Tokyo Yushun (Japan Derby)"
        }
        "安田記念" => "Yasuda Kinen",
        "宝塚記念" => "Takarazuka Kinen",
        "帝王賞" => "Teio Sho",
        "スプリンターズステークス" | "スプリンターズS" => "Sprinters Stakes",
        "菊花賞" => "Kikka Sho",
        "ジャパンカップ" | "ジャパンC" => "Japan Cup",
        "有馬記念" => "Arima Kinen",
        "アメリカJCC" => "American JCC",
        "大阪杯" => "Osaka Hai",
        "桜花賞" => "Oka Sho",
        "皐月賞" => "Satsuki Sho",
        "天皇賞（春）" => "Tenno Sho (Spring)",
        "秋華賞" => "Shuuka Sho",
        "朝日杯フューチュリティステークス" | "朝日杯FS" => {
            "Asahi Hai Futurity Stakes"
        }
        "フェブラリーステークス" | "フェブラリーS" => "February Stakes",
        title if !title.is_empty() => title,
        _ => return format!("Legend Race {event_key}"),
    };
    format!("{race_name} Legend Race")
}

fn master_legend_course(distance: i64, ground: i64) -> String {
    let distance_type = match distance {
        ..=1_400 => "Short",
        1_401..=1_800 => "Mile",
        1_801..=2_400 => "Medium",
        _ => "Long",
    };
    let ground = match ground {
        1 => "Turf",
        2 => "Dirt",
        _ => "Unknown",
    };
    format!("{distance}m - {distance_type} - {ground}")
}

fn load_timeline_campaigns(connection: &Connection) -> Result<Vec<TimelineCampaign>> {
    let raw_campaigns: Vec<RawTimelineCampaign> =
        serde_json::from_slice(BUNDLED_TIMELINE_CAMPAIGNS_JSON)
            .context("failed to parse bundled timeline_campaigns.json")?;

    let mut campaigns = raw_campaigns
        .into_iter()
        .map(|campaign| {
            let standardized_jp_title = campaign
                .jp_title
                .as_deref()
                .and_then(standardized_jp_mission_title);
            Ok(TimelineCampaign {
                campaign_id: campaign.campaign_id,
                jp_mission_event_id: campaign.jp_mission_event_id,
                jp_title: campaign.jp_title,
                mission_fingerprint: campaign.mission_fingerprint,
                image: webp_image_reference(&campaign.image),
                start_at: parse_timeline_date(&campaign.start_date)?,
                end_at: parse_timeline_date(&campaign.end_date)?,
                title: campaign.title.or(standardized_jp_title),
                description: None,
                image_url: None,
                image_path: None,
                confirmed_global_start: None,
                confirmed_global_end: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    merge_global_mission_campaigns(connection, &mut campaigns)?;
    Ok(campaigns)
}

fn standardized_jp_mission_title(title: &str) -> Option<String> {
    let (season, remainder) = if let Some(remainder) = title.strip_prefix("春のGⅠ記念ミッション ")
    {
        (Some("Spring"), remainder)
    } else if let Some(remainder) = title.strip_prefix("秋のGⅠ記念ミッション ") {
        (Some("Fall"), remainder)
    } else if let Some(remainder) = title.strip_prefix("GⅠ記念ミッション ") {
        (None, remainder)
    } else {
        return None;
    };

    let (part, race) = if let Some(race) = remainder.strip_prefix("第1弾 ") {
        (Some(1), race)
    } else if let Some(race) = remainder.strip_prefix("第2弾 ") {
        (Some(2), race)
    } else if let Some(race) = remainder.strip_prefix("第3弾 ") {
        (Some(3), race)
    } else {
        (None, remainder)
    };
    let race = match race {
        "フェブラリーS" | "フェブラリーステークス" => "February Stakes",
        "川崎記念" => "Kawasaki Kinen",
        "帝王賞" => "Teio Sho",
        "ジャパンダートダービー" | "JDダービー" => "Japan Dirt Derby",
        "高松宮記念" => "Takamatsunomiya Kinen",
        "大阪杯" => "Osaka Hai",
        "桜花賞" => "Oka Sho",
        "皐月賞" => "Satsuki Sho",
        "天皇賞（春）" => "Tenno Sho (Spring)",
        "NHKマイルC" | "NHKマイルカップ" => "NHK Mile Cup",
        "かしわ記念" => "Kashiwa Kinen",
        "ヴィクトリアマイル" => "Victoria Mile",
        "オークス" => "Japanese Oaks",
        "日本ダービー" => "Japanese Derby",
        "安田記念" => "Yasuda Kinen",
        "宝塚記念" => "Takarazuka Kinen",
        "スプリンターズS" => "Sprinters Stakes",
        "マイルCS南部杯" => "M.C. Nambu Hai",
        "秋華賞" => "Shuka Sho",
        "菊花賞" => "Kikuka Sho",
        "JBC2022" | "JBC2023" | "JBC2024" | "JBC2025" => "JBC Series",
        "天皇賞（秋）" => "Tenno Sho (Autumn)",
        "エリザベス女王杯" => "Queen Elizabeth II Cup",
        "マイルCS" => "Mile Championship",
        "ジャパンC" | "ジャパンカップ" => "Japan Cup",
        "チャンピオンズC" => "Champions Cup",
        "阪神JF" | "阪神ジュベナイルF" => "Hanshin Juvenile Fillies",
        "朝日杯FS" => "Asahi Hai Futurity Stakes",
        "有馬記念" => "Arima Kinen",
        "ホープフルS" => "Hopeful Stakes",
        "全日本ジュニア優駿" => "Zen-Nippon Junior Yushun",
        "東京大賞典" => "Tokyo Daishoten",
        _ => return None,
    };

    match (season, part) {
        (Some(season), Some(part)) => Some(format!(
            "{season} G1 Celebration Missions, Part {part}: {race}"
        )),
        _ => Some(format!("G1 Celebration Missions: {race}")),
    }
}

fn merge_global_mission_campaigns(
    connection: &Connection,
    campaigns: &mut [TimelineCampaign],
) -> Result<()> {
    let global_campaigns = load_global_mission_campaigns(connection)?;
    let mut by_fingerprint: BTreeMap<&str, Vec<&GlobalMissionCampaign>> = BTreeMap::new();
    let mut by_event_id = BTreeMap::new();
    for campaign in &global_campaigns {
        by_fingerprint
            .entry(&campaign.fingerprint)
            .or_default()
            .push(campaign);
        by_event_id.insert(campaign.event_id, campaign);
    }

    for campaign in campaigns {
        let exact = campaign
            .mission_fingerprint
            .as_deref()
            .and_then(|fingerprint| by_fingerprint.get(fingerprint))
            .filter(|matches| matches.len() == 1)
            .and_then(|matches| matches.first().copied());
        let matched = exact.or_else(|| {
            campaign
                .jp_mission_event_id
                .and_then(|event_id| by_event_id.get(&event_id).copied())
        });
        let Some(global) = matched else {
            continue;
        };
        campaign.title = campaign
            .jp_title
            .as_deref()
            .and_then(standardized_jp_mission_title)
            .or_else(|| Some(global.title.clone()));
        if global.start_at >= global_timeline_start_date() {
            campaign.confirmed_global_start = Some(global.start_at);
            campaign.confirmed_global_end = Some(global.end_at);
        }
        campaign.description = Some(format!(
            "{} limited-time mission{}",
            global.mission_count,
            if global.mission_count == 1 { "" } else { "s" }
        ));
    }
    Ok(())
}

fn load_global_mission_campaigns(connection: &Connection) -> Result<Vec<GlobalMissionCampaign>> {
    type MissionSignature = [i64; 9];
    #[derive(Default)]
    struct Accumulator {
        title: String,
        start_at: Option<DateTime<Utc>>,
        end_at: Option<DateTime<Utc>>,
        signatures: BTreeMap<MissionSignature, usize>,
    }

    let mut statement = connection.prepare(
        r#"
        SELECT mission.event_id,
               mission.condition_type,
               mission.condition_value_1,
               mission.condition_value_2,
               mission.condition_value_3,
               mission.condition_value_4,
               mission.condition_num,
               mission.item_category,
               mission.item_id,
               mission.item_num,
               mission.start_date,
               mission.end_date,
               COALESCE(title.text, '')
        FROM mission_data AS mission
        LEFT JOIN text_data AS title
          ON title.category = 187
         AND title."index" = mission.event_id
        WHERE mission.mission_type = 4
          AND mission.event_id > 0
        ORDER BY mission.event_id, mission.id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            [
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
            ],
            row.get::<_, String>(10)?,
            row.get::<_, String>(11)?,
            row.get::<_, String>(12)?,
        ))
    })?;

    let mut grouped: BTreeMap<i64, Accumulator> = BTreeMap::new();
    for row in rows {
        let (event_id, signature, start_date, end_date, title) = row?;
        let group = grouped.entry(event_id).or_default();
        group.title = title.replace("\\n", " ");
        let start_at = parse_master_utc_date(&start_date)?;
        let end_at = parse_master_utc_date(&end_date)?;
        group.start_at = Some(
            group
                .start_at
                .map_or(start_at, |current| current.min(start_at)),
        );
        group.end_at = Some(group.end_at.map_or(end_at, |current| current.max(end_at)));
        *group.signatures.entry(signature).or_default() += 1;
    }

    grouped
        .into_iter()
        .map(|(event_id, group)| {
            let mission_count = group.signatures.values().sum();
            Ok(GlobalMissionCampaign {
                event_id,
                title: if group.title.trim().is_empty() {
                    "Limited-Time Missions".to_string()
                } else {
                    group.title.trim().to_string()
                },
                start_at: group
                    .start_at
                    .context("global mission campaign is missing a start date")?,
                end_at: group
                    .end_at
                    .context("global mission campaign is missing an end date")?,
                mission_count,
                fingerprint: mission_signature_fingerprint(&group.signatures),
            })
        })
        .collect()
}

fn mission_signature_fingerprint(signatures: &BTreeMap<[i64; 9], usize>) -> String {
    let mut hasher = Sha256::new();
    let mut first = true;
    for (signature, count) in signatures {
        let line = signature
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        for _ in 0..*count {
            if !first {
                hasher.update(b"\n");
            }
            first = false;
            hasher.update(line.as_bytes());
        }
    }
    hex::encode(hasher.finalize())
}

fn parse_master_utc_date(value: &str) -> Result<DateTime<Utc>> {
    for pattern in ["%Y/%m/%d %H:%M:%S", "%Y/%m/%d %H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(value, pattern) {
            return Ok(Utc.from_utc_datetime(&naive));
        }
    }
    anyhow::bail!("failed to parse master.mdb date {value}")
}

fn parse_timeline_date(value: &str) -> Result<DateTime<Utc>> {
    if value.contains('T') {
        if let Ok(date_time) = DateTime::parse_from_rfc3339(value) {
            return Ok(date_time.with_timezone(&Utc));
        }

        let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
            .with_context(|| format!("failed to parse ISO timeline date {value}"))?;
        return Ok(Utc.from_utc_datetime(&naive));
    }

    parse_display_date(value)
}

fn parse_display_date(value: &str) -> Result<DateTime<Utc>> {
    let (date_part, time_part) = value
        .split_once(',')
        .with_context(|| format!("timeline date must contain comma: {value}"))?;
    let mut date_tokens = date_part.split_whitespace();
    let day = date_tokens
        .next()
        .with_context(|| format!("timeline date missing day: {value}"))?
        .parse::<u32>()
        .with_context(|| format!("timeline date has invalid day: {value}"))?;
    let month = month_number(
        date_tokens
            .next()
            .with_context(|| format!("timeline date missing month: {value}"))?,
    )
    .with_context(|| format!("timeline date has invalid month: {value}"))?;
    let year = date_tokens
        .next()
        .with_context(|| format!("timeline date missing year: {value}"))?
        .parse::<i32>()
        .with_context(|| format!("timeline date has invalid year: {value}"))?;
    let (hour, minute) = time_part
        .trim()
        .split_once(':')
        .with_context(|| format!("timeline date has invalid time: {value}"))?;
    let hour = hour
        .parse::<u32>()
        .with_context(|| format!("timeline date has invalid hour: {value}"))?;
    let minute = minute
        .parse::<u32>()
        .with_context(|| format!("timeline date has invalid minute: {value}"))?;
    let naive = NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|date| date.and_hms_opt(hour, minute, 0))
        .with_context(|| format!("timeline date is out of range: {value}"))?;

    match Berlin.from_local_datetime(&naive) {
        LocalResult::Single(date_time) => Ok(date_time.with_timezone(&Utc)),
        LocalResult::Ambiguous(earliest, _) => Ok(earliest.with_timezone(&Utc)),
        LocalResult::None => {
            let adjusted = naive + Duration::hours(1);
            match Berlin.from_local_datetime(&adjusted) {
                LocalResult::Single(date_time) | LocalResult::Ambiguous(date_time, _) => {
                    Ok(date_time.with_timezone(&Utc))
                }
                LocalResult::None => {
                    anyhow::bail!("timeline date falls outside Berlin local time: {value}")
                }
            }
        }
    }
}

fn month_number(month: &str) -> Option<u32> {
    match month {
        "Jan" => Some(1),
        "Feb" => Some(2),
        "Mar" => Some(3),
        "Apr" => Some(4),
        "May" => Some(5),
        "Jun" => Some(6),
        "Jul" => Some(7),
        "Aug" => Some(8),
        "Sep" | "Sept" => Some(9),
        "Oct" => Some(10),
        "Nov" => Some(11),
        "Dec" => Some(12),
        _ => None,
    }
}

fn load_support_card_names(
    connection: &Connection,
    character_names: &BTreeMap<i64, String>,
) -> Result<BTreeMap<i64, String>> {
    let mut names = load_bundled_support_card_names()?;
    names.extend(load_umapyoi_support_character_names(character_names)?);
    let mut statement = connection.prepare(
        r#"
        SELECT id, chara_id
        FROM support_card_data
        ORDER BY id
        "#,
    )?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;

    for row in rows {
        let (card_id, chara_id) = row?;
        if let Some(name) = character_names.get(&chara_id) {
            names.insert(card_id, name.clone());
        }
    }

    Ok(names)
}

fn load_support_card_specific_names(connection: &Connection) -> Result<BTreeMap<i64, String>> {
    let mut names = load_bundled_support_card_specific_names()?;
    names.extend(load_umapyoi_support_card_names()?);
    names.extend(load_text_data_by_category(
        connection,
        SUPPORT_CARD_NAME_CATEGORY,
    )?);
    Ok(names)
}

fn load_umapyoi_support_character_names(
    character_names: &BTreeMap<i64, String>,
) -> Result<BTreeMap<i64, String>> {
    let archive: serde_json::Value = serde_json::from_slice(BUNDLED_UMAPYOI_ARCHIVE_JSON)
        .context("failed to parse bundled umapyoi archive for support character names")?;
    Ok(archive
        .get("supports")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|support| {
            let support_id = support.get("support_id")?.as_i64()?;
            let chara_id = support.get("chara_id")?.as_i64()?;
            character_names
                .get(&chara_id)
                .cloned()
                .map(|name| (support_id, name))
        })
        .collect())
}

fn load_umapyoi_support_card_names() -> Result<BTreeMap<i64, String>> {
    let archive: serde_json::Value = serde_json::from_slice(BUNDLED_UMAPYOI_ARCHIVE_JSON)
        .context("failed to parse bundled umapyoi archive for support card names")?;
    Ok(archive
        .get("supports")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|support| {
            let support_id = support.get("support_id")?.as_i64()?;
            let name = support
                .pointer("/raw/title_en")
                .and_then(serde_json::Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .or_else(|| support.get("title_jp").and_then(serde_json::Value::as_str))?;
            Some((support_id, name.trim().trim_matches(['[', ']']).to_string()))
        })
        .collect())
}

fn load_bundled_support_card_names() -> Result<BTreeMap<i64, String>> {
    let raw_entries: Vec<RawSupportCardName> =
        serde_json::from_slice(BUNDLED_JP_SUPPORT_CARDS_DB_JSON)
            .context("failed to parse bundled support-cards-db.json for timeline support names")?;

    Ok(raw_entries
        .into_iter()
        .filter_map(|entry| {
            entry
                .id
                .parse::<i64>()
                .ok()
                .map(|card_id| (card_id, entry.name))
        })
        .collect::<BTreeMap<_, _>>())
}

fn load_bundled_support_card_specific_names() -> Result<BTreeMap<i64, String>> {
    let raw_entries: Vec<RawSupportCardName> = serde_json::from_slice(
        BUNDLED_JP_SUPPORT_CARDS_DB_JSON,
    )
    .context("failed to parse bundled support-cards-db.json for timeline support card names")?;

    Ok(raw_entries
        .into_iter()
        .filter_map(|entry| {
            let name = entry.support_card_name.or(entry.card_name)?;
            entry.id.parse::<i64>().ok().map(|card_id| (card_id, name))
        })
        .collect::<BTreeMap<_, _>>())
}

fn load_text_data_by_category(
    connection: &Connection,
    category: i64,
) -> Result<BTreeMap<i64, String>> {
    let mut statement = connection.prepare(
        r#"
        SELECT "index", text
        FROM text_data
        WHERE category = ?
        ORDER BY "index"
        "#,
    )?;
    let rows = statement.query_map([category], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;

    rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()
        .map_err(Into::into)
}

fn character_name_for_card(card_id: i64, character_names: &BTreeMap<i64, String>) -> String {
    let chara_id = card_id / 100;
    character_names
        .get(&chara_id)
        .cloned()
        .unwrap_or_else(|| format!("Unknown_{}", card_id))
}

fn support_name_for_card(card_id: i64, support_names: &BTreeMap<i64, String>) -> String {
    support_names
        .get(&card_id)
        .cloned()
        .unwrap_or_else(|| format!("Unknown_{}", card_id))
}

fn support_card_specific_name_for_card(
    card_id: i64,
    support_card_names: &BTreeMap<i64, String>,
) -> Option<String> {
    support_card_names.get(&card_id).cloned()
}

fn title_from_names(names: &[String], fallback: &str) -> String {
    match names {
        [] => fallback.to_string(),
        [only] => only.clone(),
        [first, rest @ ..] => format!("{} + {} more", first, rest.len()),
    }
}

fn banner_duration_days(start: DateTime<Utc>, end: DateTime<Utc>) -> i64 {
    days_between(start, end)
}

fn calculate_end_date(global_date: DateTime<Utc>, duration_in_days: i64) -> DateTime<Utc> {
    normalize_to_release_hour(global_date + Duration::days(duration_in_days))
}

fn normalize_to_midnight_utc(date: DateTime<Utc>) -> DateTime<Utc> {
    utc_date(date.year(), date.month(), date.day(), 0)
}

fn normalize_to_release_hour(date: DateTime<Utc>) -> DateTime<Utc> {
    utc_date(date.year(), date.month(), date.day(), 22)
}

fn latest_closed_global_month(confirmed_dates: &ConfirmedDateLookup) -> Option<(i32, u32)> {
    confirmed_dates
        .closed_global_months
        .iter()
        .next_back()
        .copied()
}

fn first_release_after_global_month(year: i32, month: u32) -> DateTime<Utc> {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };

    utc_date(next_year, next_month, 1, 22)
}

fn is_berlin_dst(date: DateTime<Utc>) -> bool {
    date.with_timezone(&Berlin).offset().fix().local_minus_utc() == 7200
}

fn days_between(start: DateTime<Utc>, end: DateTime<Utc>) -> i64 {
    (end - start).num_days()
}

fn jp_launch_date() -> DateTime<Utc> {
    utc_date(JP_LAUNCH_YEAR, JP_LAUNCH_MONTH, JP_LAUNCH_DAY, 0)
}

fn global_launch_date() -> DateTime<Utc> {
    utc_date(
        GLOBAL_LAUNCH_YEAR,
        GLOBAL_LAUNCH_MONTH,
        GLOBAL_LAUNCH_DAY,
        0,
    )
}

fn global_timeline_start_date() -> DateTime<Utc> {
    utc_date(
        GLOBAL_LAUNCH_YEAR,
        GLOBAL_LAUNCH_MONTH,
        GLOBAL_LAUNCH_DAY,
        22,
    )
}

fn utc_date(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, 0, 0)
        .single()
        .expect("hard-coded UTC date should be valid")
}

fn clamp_rate(rate: f64) -> f64 {
    rate.clamp(MIN_ACCELERATION_RATE, MAX_ACCELERATION_RATE)
}

fn round_rate(rate: f64) -> f64 {
    (rate * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::{
        annotate_calendar_likelihoods, annotate_rerun_banners, apply_closed_schedule_adjustment,
        apply_family_adjustment, build_anniversary_schedule_anchors, build_confirmed_date_lookup,
        calculate_global_date, calculate_recent_acceleration_rate,
        first_release_after_global_month, gacha_type_name, latest_closed_global_month,
        legend_boss_metadata, load_bundled_support_card_names, load_timeline_campaigns,
        load_timeline_character_banners, load_timeline_legend_races, load_timeline_paid_banners,
        load_timeline_story_events, load_timeline_support_banners, load_umapyoi_support_card_names,
        load_umapyoi_support_character_names, merge_global_mission_campaigns,
        merge_legend_race_news, mission_signature_fingerprint, monotonic_schedule_anchors,
        parse_confirmed_banner_dates, standardized_jp_mission_title,
        timeline_anniversaries_through, utc_date, BannerTimelineEvent, BannerTimelineEventType,
        CalendarLikelihoodModel, CalibrationAnchor, ConfirmedDateLookup, ConfirmedTimelineKind,
        DatePrediction, FamilyAdjustmentModel, FamilyAdjustmentModels, FamilyAdjustmentSample,
        PredictionInfo, PredictionKind, TimelineCampaign, TimelineCharacterBanner,
        TimelineSupportBanner, FALLBACK_ACCELERATION_RATE,
    };
    use crate::generators::banners::{CharacterBanner, SupportBanner};
    use crate::generators::jp_events::LegendRaceTimelineMetadata;
    use chrono::Duration;
    use rusqlite::Connection;
    use std::collections::{BTreeMap, BTreeSet};

    fn mission_connection() -> Connection {
        let connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch(
                r#"
                CREATE TABLE mission_data (
                    id INTEGER, mission_type INTEGER, event_id INTEGER,
                    condition_type INTEGER, condition_value_1 INTEGER,
                    condition_value_2 INTEGER, condition_value_3 INTEGER,
                    condition_value_4 INTEGER, condition_num INTEGER,
                    item_category INTEGER, item_id INTEGER, item_num INTEGER,
                    start_date TEXT, end_date TEXT
                );
                CREATE TABLE text_data (category INTEGER, "index" INTEGER, text TEXT);
                "#,
            )
            .expect("mission test schema should build");
        connection
    }

    #[test]
    fn confirmed_dates_csv_accepts_easy_banner_formats() {
        let dates = parse_confirmed_banner_dates(
            r#"
            # type,banner,date
            character,2022_30100.png,2026-06-18
            support,2022_30101.webp,2026-06-18
            paid,50003,2025-11-03T22:00:00Z
            story,03_brand_new_friend_banner.png,2025-07-16
            champions,14,2026-06-21
            legend,legend_race_12,2026-06-07
            league_of_heroes,2023-05-12,2026-08-15
            anniversary,1,2025-10-26
            "#,
        )
        .expect("test confirmed date CSV should parse");

        assert_eq!(dates.len(), 8);
        assert_eq!(dates[0].kind, ConfirmedTimelineKind::Character);
        assert_eq!(dates[0].key, "30100");
        assert_eq!(dates[0].global_date, utc_date(2026, 6, 18, 22));
        assert_eq!(dates[1].kind, ConfirmedTimelineKind::Support);
        assert_eq!(dates[1].key, "30101");
        assert_eq!(dates[2].kind, ConfirmedTimelineKind::Paid);
        assert_eq!(dates[2].key, "50003");
        assert_eq!(dates[3].kind, ConfirmedTimelineKind::Story);
        assert_eq!(dates[3].key, "03_brand_new_friend_banner");
        assert_eq!(dates[4].kind, ConfirmedTimelineKind::Champions);
        assert_eq!(dates[4].key, "champions_meeting_14");
        assert_eq!(dates[5].kind, ConfirmedTimelineKind::Legend);
        assert_eq!(dates[5].key, "legend_race_12");
        assert_eq!(dates[6].kind, ConfirmedTimelineKind::LeagueOfHeroes);
        assert_eq!(dates[6].key, "league-of-heroes-2023-05-12");
        assert_eq!(dates[7].kind, ConfirmedTimelineKind::Anniversary);
        assert_eq!(dates[7].key, "1");
    }

    #[test]
    fn umapyoi_support_fallback_covers_new_jp_ids() {
        let character_names = BTreeMap::from([
            (1058, "Meisho Doto".to_string()),
            (9001, "Tazuna Hayakawa".to_string()),
        ]);
        let names = load_umapyoi_support_character_names(&character_names)
            .expect("umapyoi support character names should parse");
        let card_names =
            load_umapyoi_support_card_names().expect("umapyoi support card names should parse");
        assert_eq!(names.get(&30_304).map(String::as_str), Some("Meisho Doto"));
        assert_eq!(
            names.get(&30_305).map(String::as_str),
            Some("Tazuna Hayakawa")
        );
        assert!(card_names.get(&30_304).is_some_and(|name| !name.is_empty()));
    }

    #[test]
    fn db_banner_confirmations_require_matching_pickup_ids() {
        let matching_character_date = utc_date(2025, 6, 26, 0);
        let matching_support_date = utc_date(2025, 6, 27, 0);

        let lookup = build_confirmed_date_lookup(
            &[
                test_character_banner(90002, matching_character_date, vec![10, 20]),
                test_character_banner(90130, utc_date(2025, 7, 13, 22), vec![100101, 100201]),
            ],
            &[
                test_support_banner(90003, matching_support_date, vec![30, 40]),
                test_support_banner(90131, utc_date(2025, 7, 13, 22), vec![30001, 30002]),
            ],
            &[],
            &[
                test_timeline_character_banner(90002, vec![20, 10]),
                test_timeline_character_banner(90130, vec![104201]),
            ],
            &[
                test_timeline_support_banner(90003, vec![40, 30]),
                test_timeline_support_banner(90131, vec![30119, 30121]),
            ],
            &[],
            &[],
        )
        .expect("test confirmation lookup should build");

        assert_eq!(
            lookup.character.get(&90002).copied(),
            Some(matching_character_date)
        );
        assert_eq!(
            lookup.support.get(&90003).copied(),
            Some(matching_support_date)
        );
        assert!(!lookup.character.contains_key(&90130));
        assert!(!lookup.support.contains_key(&90131));
    }

    #[test]
    fn bundled_timeline_image_references_emit_webp() {
        for banner in
            load_timeline_character_banners().expect("character timeline data should parse")
        {
            assert_webp_reference(&banner.image);
            assert_webp_reference(&banner.image_path);
        }

        for banner in load_timeline_support_banners().expect("support timeline data should parse") {
            assert_webp_reference(&banner.image);
        }

        for banner in load_timeline_paid_banners().expect("paid timeline data should parse") {
            assert_webp_reference(&banner.image);
        }

        for event in load_timeline_story_events(None).expect("story timeline data should parse") {
            assert_webp_reference(&event.image);
        }

        let legend_events =
            load_timeline_legend_races(None).expect("legend timeline data should parse");
        let (first_names, first_card_ids, _) = legend_boss_metadata(&legend_events[0]);
        assert_eq!(
            first_names,
            [
                "El Condor Pasa (Original)",
                "Special Week (Original)",
                "Symboli Rudolf (Original)"
            ]
        );
        assert_eq!(first_card_ids, [101401, 100101, 101701]);
        for event in legend_events {
            let (names, card_ids, images) = legend_boss_metadata(&event);
            assert_eq!(names.len(), event.bosses.len());
            assert_eq!(card_ids.len(), event.bosses.len());
            assert_eq!(images.len(), event.bosses.len());
            assert!(names.iter().all(|name| !name.trim().is_empty()));
            for boss in event.bosses {
                assert_webp_reference(&boss.image);
            }
        }

        let connection = mission_connection();
        for campaign in
            load_timeline_campaigns(&connection).expect("campaign timeline data should parse")
        {
            if !campaign.image.is_empty() {
                assert_webp_reference(&campaign.image);
            }
        }
    }

    #[test]
    fn legend_race_news_adds_event_media_without_reusing_boss_portraits() {
        let mut races =
            load_timeline_legend_races(None).expect("legend timeline data should parse");
        let first_start = races[0].start_at;
        let boss_images = races[0]
            .bosses
            .iter()
            .map(|boss| boss.image.clone())
            .collect::<Vec<_>>();
        merge_legend_race_news(
            &mut races,
            &[LegendRaceTimelineMetadata {
                start_at: first_start + Duration::hours(2),
                image_url: "https://example.invalid/Thumbnail/banner_30200295.png".to_string(),
                source_post_id: 2_295,
            }],
        );

        assert_eq!(
            races[0].image_url.as_deref(),
            Some("https://example.invalid/Thumbnail/banner_30200295.png")
        );
        assert_eq!(
            races[0].image_path.as_deref(),
            Some("assets/timeline-images/events/legend-race/2295.webp")
        );
        assert_eq!(races[0].source_post_id, Some(2_295));
        assert_eq!(
            races[0]
                .bosses
                .iter()
                .map(|boss| boss.image.clone())
                .collect::<Vec<_>>(),
            boss_images
        );
        assert!(races.iter().skip(1).all(|race| {
            race.image_url.as_deref() == Some(super::LEGEND_RACE_FALLBACK_IMAGE_URL)
                && race.image_path.as_deref() == Some(super::LEGEND_RACE_FALLBACK_IMAGE_PATH)
        }));
    }

    #[test]
    fn jp_master_extends_story_events_after_bundled_horizon() {
        let connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch(
                r#"
                CREATE TABLE story_event_data (
                    story_event_id INTEGER,
                    start_date INTEGER,
                    end_date INTEGER
                );
                INSERT INTO story_event_data VALUES
                    (999, 1577836800, 1579046400),
                    (1055, 1785380400, 1786935599);
                "#,
            )
            .expect("story event test schema should build");

        let events = load_timeline_story_events(Some(&connection))
            .expect("JP master story events should load");
        let projected = events
            .iter()
            .find(|event| event.master_event_id == Some(1055))
            .expect("new JP master event should extend the bundled timeline");
        assert_eq!(projected.event_name, "Story Event #1055");
        assert!(projected.image.is_empty());
        assert!(!events
            .iter()
            .any(|event| event.master_event_id == Some(999)));
    }

    #[test]
    fn jp_master_extends_legend_races_after_bundled_horizon() {
        let connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch(
                r#"
                CREATE TABLE legend_race (
                    id INTEGER, image_id INTEGER, race_instance_id INTEGER,
                    start_date INTEGER, end_date INTEGER
                );
                CREATE TABLE race_instance (id INTEGER, race_id INTEGER);
                CREATE TABLE race (id INTEGER, course_set INTEGER);
                CREATE TABLE race_course_set (id INTEGER, distance INTEGER, ground INTEGER);
                CREATE TABLE text_data (category INTEGER, "index" INTEGER, text TEXT);
                INSERT INTO race VALUES (12017, 10808), (12046, 10501);
                INSERT INTO race_course_set VALUES (10808, 2400, 1), (10501, 1200, 1);
                INSERT INTO race_instance VALUES (1204501, 12046), (1204502, 12046);
                INSERT INTO race_instance VALUES (1201701, 12017), (1201702, 12017);
                INSERT INTO text_data VALUES
                    (28, 1201701, '東京優駿（日本ダービー） HARD'),
                    (28, 1201702, '東京優駿（日本ダービー） HARD'),
                    (28, 1204501, 'スプリンターズステークス HARD'),
                    (28, 1204502, 'スプリンターズステークス HARD');
                "#,
            )
            .expect("legend race test schema should build");
        connection
            .execute(
                "INSERT INTO legend_race VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    136,
                    10_690_102,
                    1_204_501,
                    utc_date(2026, 8, 9, 3).timestamp(),
                    utc_date(2026, 8, 11, 19).timestamp()
                ],
            )
            .expect("first legend opponent should insert");
        connection
            .execute(
                "INSERT INTO legend_race VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    137,
                    112_001,
                    1_204_502,
                    utc_date(2026, 8, 11, 20).timestamp(),
                    utc_date(2026, 8, 14, 19).timestamp()
                ],
            )
            .expect("second legend opponent should insert");
        connection
            .execute(
                "INSERT INTO legend_race VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    80,
                    103_101,
                    1_201_701,
                    utc_date(2024, 5, 5, 3).timestamp(),
                    utc_date(2024, 5, 7, 19).timestamp()
                ],
            )
            .expect("bundled-boundary first opponent should insert");
        connection
            .execute(
                "INSERT INTO legend_race VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    81,
                    103_301,
                    1_201_702,
                    utc_date(2024, 5, 7, 20).timestamp(),
                    utc_date(2024, 5, 10, 19).timestamp()
                ],
            )
            .expect("bundled-boundary second opponent should insert");

        let bundled_count = load_timeline_legend_races(None)
            .expect("bundled legend data should parse")
            .len();
        let races = load_timeline_legend_races(Some(&connection))
            .expect("JP master legend data should merge");
        let added = races.last().expect("master should append one race cycle");

        assert_eq!(races.len(), bundled_count + 1);
        assert_eq!(added.index, bundled_count);
        assert_eq!(added.race_name, "Sprinters Stakes Legend Race");
        assert_eq!(added.course.as_deref(), Some("1200m - Short - Turf"));
        assert_eq!(
            added
                .bosses
                .iter()
                .filter_map(|boss| boss.card_id)
                .collect::<Vec<_>>(),
            [106_901, 112_001]
        );
        assert_eq!(
            added
                .bosses
                .iter()
                .map(|boss| boss.name.as_str())
                .collect::<Vec<_>>(),
            ["Sakura Chiyono O (Original)", "Calstone Light O (Original)"]
        );
    }

    #[test]
    fn bundled_legend_races_match_exact_live_news_days() {
        let mut races =
            load_timeline_legend_races(None).expect("legend timeline data should parse");
        let news = crate::generators::jp_events::legend_race_timeline_metadata()
            .expect("legend news metadata should parse");
        merge_legend_race_news(&mut races, &news);

        assert!(races.iter().all(|race| race.image_path.is_some()));
        assert_eq!(
            races
                .iter()
                .filter(|race| race.source_post_id.is_some())
                .count(),
            17
        );
        assert!(races
            .iter()
            .filter_map(|race| race.image_path.as_deref())
            .all(
                |path| path.starts_with("assets/timeline-images/events/legend-race/")
                    && path.ends_with(".webp")
            ));
        assert!(races.iter().any(|race| {
            race.image_url.as_deref() == Some(super::LEGEND_RACE_FALLBACK_IMAGE_URL)
                && race.image_path.as_deref() == Some(super::LEGEND_RACE_FALLBACK_IMAGE_PATH)
        }));
    }

    #[test]
    fn mission_campaigns_match_different_region_ids_by_content() {
        let connection = mission_connection();
        connection
            .execute_batch(
                r#"
                INSERT INTO mission_data VALUES
                    (1, 4, 17, 10, 11, 12, 13, 14, 15, 16, 17, 18,
                     '2025/08/07 22:00:00', '2025/08/21 14:59:59');
                INSERT INTO text_data VALUES (187, 17, 'Golshi Week Special Missions');
                "#,
            )
            .expect("mission fixture should insert");
        let signature = [10, 11, 12, 13, 14, 15, 16, 17, 18];
        let mut signatures = BTreeMap::new();
        signatures.insert(signature, 1);
        let mut campaigns = vec![TimelineCampaign {
            campaign_id: 17,
            jp_mission_event_id: Some(46),
            jp_title: None,
            mission_fingerprint: Some(mission_signature_fingerprint(&signatures)),
            image: "17.webp".to_string(),
            start_at: utc_date(2021, 4, 30, 3),
            end_at: utc_date(2021, 5, 14, 19),
            title: None,
            description: None,
            image_url: None,
            image_path: None,
            confirmed_global_start: None,
            confirmed_global_end: None,
        }];

        merge_global_mission_campaigns(&connection, &mut campaigns)
            .expect("global mission campaign should merge");

        assert_eq!(
            campaigns[0].title.as_deref(),
            Some("Golshi Week Special Missions")
        );
        assert_eq!(
            campaigns[0].confirmed_global_start,
            Some(utc_date(2025, 8, 7, 22))
        );
        assert_eq!(
            campaigns[0].description.as_deref(),
            Some("1 limited-time mission")
        );
    }

    #[test]
    fn jp_g1_mission_titles_are_standardized_without_a_global_mission_match() {
        assert_eq!(
            standardized_jp_mission_title("GⅠ記念ミッション フェブラリーS").as_deref(),
            Some("G1 Celebration Missions: February Stakes")
        );
        assert_eq!(
            standardized_jp_mission_title("春のGⅠ記念ミッション 第2弾 NHKマイルカップ").as_deref(),
            Some("Spring G1 Celebration Missions, Part 2: NHK Mile Cup")
        );
        assert_eq!(
            standardized_jp_mission_title("秋のGⅠ記念ミッション 第3弾 東京大賞典").as_deref(),
            Some("Fall G1 Celebration Missions, Part 3: Tokyo Daishoten")
        );
    }

    #[test]
    fn timeline_anniversaries_include_confirmed_and_projected_markers() {
        let mut lookup = empty_confirmed_date_lookup();
        lookup.anniversary.insert(1, utc_date(2025, 10, 26, 22));
        let anchors = anniversary_schedule_anchors(&lookup);
        let rate = calculate_recent_acceleration_rate(&anchors);
        let anniversaries =
            timeline_anniversaries_through(utc_date(2026, 7, 1, 22), &lookup, &anchors, rate, None);

        assert_eq!(anniversaries.len(), 2);
        assert_eq!(anniversaries[0].index, 1);
        assert_eq!(anniversaries[0].label, "0.5 Year Anniversary");
        assert_eq!(anniversaries[0].global_date, utc_date(2025, 10, 26, 22));
        assert_eq!(anniversaries[0].jp_date, utc_date(2021, 8, 24, 0));
        assert!(anniversaries[0].is_confirmed);
        assert_eq!(anniversaries[1].index, 2);
        assert_eq!(anniversaries[1].label, "1 Year Anniversary");
        assert_eq!(anniversaries[1].global_date, utc_date(2026, 3, 12, 22));
        assert_eq!(anniversaries[1].jp_date, utc_date(2022, 2, 24, 0));
        assert!(!anniversaries[1].is_confirmed);
    }

    #[test]
    fn closed_schedule_month_shifts_future_unconfirmed_anniversary_markers() {
        let mut lookup = empty_confirmed_date_lookup();
        lookup.anniversary.insert(1, utc_date(2025, 10, 26, 22));
        lookup.closed_global_months.insert((2026, 6));
        let anchors = vec![CalibrationAnchor {
            jp: utc_date(2022, 2, 24, 0),
            global: utc_date(2026, 6, 26, 22),
        }];

        let anniversaries = timeline_anniversaries_through(
            utc_date(2026, 7, 1, 22),
            &lookup,
            &anchors,
            FALLBACK_ACCELERATION_RATE,
            Some(utc_date(2022, 1, 1, 0)),
        );

        assert_eq!(anniversaries.len(), 2);
        assert_eq!(anniversaries[0].global_date, utc_date(2025, 10, 26, 22));
        assert!(anniversaries[0].is_confirmed);
        assert_eq!(anniversaries[0].schedule_adjustment_days, None);
        assert_eq!(anniversaries[1].global_date, utc_date(2026, 7, 1, 22));
        assert!(!anniversaries[1].is_confirmed);
        assert_eq!(anniversaries[1].schedule_adjustment_days, Some(5));
    }

    #[test]
    fn closed_schedule_month_does_not_drag_past_anniversary_markers_forward() {
        let mut lookup = empty_confirmed_date_lookup();
        lookup.anniversary.insert(1, utc_date(2025, 10, 26, 22));
        lookup.closed_global_months.insert((2026, 6));
        let anchors = anniversary_schedule_anchors(&lookup);
        let rate = calculate_recent_acceleration_rate(&anchors);

        let anniversaries = timeline_anniversaries_through(
            utc_date(2026, 7, 1, 22),
            &lookup,
            &anchors,
            rate,
            Some(utc_date(2022, 7, 20, 0)),
        );

        assert_eq!(anniversaries.len(), 2);
        assert_eq!(anniversaries[1].global_date, utc_date(2026, 3, 12, 22));
        assert_eq!(anniversaries[1].schedule_adjustment_days, None);
    }

    #[test]
    fn future_unconfirmed_dates_are_extrapolated_from_latest_anchor() {
        let anchors = vec![
            CalibrationAnchor {
                jp: utc_date(2022, 4, 1, 0),
                global: utc_date(2026, 5, 1, 22),
            },
            CalibrationAnchor {
                jp: utc_date(2022, 4, 11, 0),
                global: utc_date(2026, 5, 8, 22),
            },
            CalibrationAnchor {
                jp: utc_date(2022, 4, 21, 0),
                global: utc_date(2026, 5, 15, 22),
            },
        ];
        let rate = calculate_recent_acceleration_rate(&anchors);
        let prediction = calculate_global_date(utc_date(2022, 5, 1, 3), None, &anchors, rate);

        assert_eq!(prediction.kind, PredictionKind::Extrapolated);
        assert!(prediction.global_date > utc_date(2026, 5, 15, 22));
    }

    #[test]
    fn out_of_order_confirmed_banners_do_not_anchor_future_predictions() {
        let anchors = vec![
            CalibrationAnchor {
                jp: utc_date(2022, 7, 20, 0),
                global: utc_date(2026, 6, 25, 22),
            },
            CalibrationAnchor {
                jp: utc_date(2022, 10, 17, 0),
                global: utc_date(2025, 7, 13, 22),
            },
        ];
        let schedule_anchors = monotonic_schedule_anchors(&anchors);
        let rate = calculate_recent_acceleration_rate(&schedule_anchors);
        let prediction =
            calculate_global_date(utc_date(2022, 7, 29, 3), None, &schedule_anchors, rate);

        assert_eq!(schedule_anchors.len(), 1);
        assert_eq!(prediction.kind, PredictionKind::Extrapolated);
        assert!(prediction.global_date > utc_date(2026, 6, 25, 22));
    }

    #[test]
    fn closed_schedule_month_shifts_unknown_future_sequence() {
        let mut events = vec![
            test_timeline_event(
                "confirmed",
                utc_date(2022, 7, 20, 0),
                utc_date(2026, 6, 25, 22),
                true,
            ),
            test_timeline_event(
                "first-unconfirmed",
                utc_date(2022, 7, 29, 0),
                utc_date(2026, 6, 29, 22),
                false,
            ),
            test_timeline_event(
                "second-unconfirmed",
                utc_date(2022, 8, 8, 0),
                utc_date(2026, 7, 6, 22),
                false,
            ),
        ];
        let mut closed_global_months = BTreeSet::new();
        closed_global_months.insert((2026, 6));
        let lookup = ConfirmedDateLookup {
            character: BTreeMap::new(),
            support: BTreeMap::new(),
            paid: BTreeMap::new(),
            story: BTreeMap::new(),
            champions: BTreeMap::new(),
            legend: BTreeMap::new(),
            campaign: BTreeMap::new(),
            news_events: BTreeMap::new(),
            anniversary: BTreeMap::new(),
            closed_global_months,
        };

        assert_eq!(latest_closed_global_month(&lookup), Some((2026, 6)));
        assert_eq!(
            first_release_after_global_month(2026, 6),
            utc_date(2026, 7, 1, 22)
        );

        apply_closed_schedule_adjustment(&mut events, &lookup);

        assert_eq!(events[1].global_release_date, utc_date(2026, 7, 1, 22));
        assert_eq!(events[2].global_release_date, utc_date(2026, 7, 8, 22));
        assert_eq!(events[1].prediction.schedule_adjustment_days, Some(2));
        assert_eq!(events[2].prediction.schedule_adjustment_days, Some(2));
    }

    #[test]
    fn bundled_support_card_names_resolve_future_jp_cards() {
        let names =
            load_bundled_support_card_names().expect("bundled support card name data should parse");

        assert_eq!(names.get(&30187).map(String::as_str), Some("Orfevre"));
        assert_eq!(names.get(&30214).map(String::as_str), Some("Fine Motion"));
    }

    #[test]
    fn support_event_includes_card_names_separately() {
        let banner = test_timeline_support_banner(30067, vec![30067]);
        let support_names = BTreeMap::from([(30067, "Symboli Rudolf".to_string())]);
        let support_card_names = BTreeMap::from([(30067, "Heirs to the Throne".to_string())]);
        let lookup = empty_confirmed_date_lookup();

        let event = super::support_event(
            &banner,
            &support_names,
            &support_card_names,
            &lookup,
            &[],
            FALLBACK_ACCELERATION_RATE,
        );

        assert_eq!(event.related_support_cards, ["Symboli Rudolf"]);
        assert_eq!(event.related_support_card_names, ["Heirs to the Throne"]);
    }

    #[test]
    fn rerun_banners_require_an_earlier_release_for_every_pickup() {
        let first_date = utc_date(2022, 1, 1, 3);
        let later_date = utc_date(2022, 2, 1, 3);
        let mut original = test_timeline_event_with_type(
            "original",
            BannerTimelineEventType::CharacterBanner,
            first_date,
            first_date,
            false,
        );
        original.gacha_type = Some(3);
        original.gacha_type_name = Some(gacha_type_name(3));
        original.card_type = Some("character".to_string());
        original.pickup_card_ids = vec![100_101];

        let mut same_day_duplicate = original.clone();
        same_day_duplicate.id = "same-day".to_string();

        let mut rerun = original.clone();
        rerun.id = "rerun".to_string();
        rerun.jp_release_date = later_date;

        let mut mixed_new_banner = rerun.clone();
        mixed_new_banner.id = "mixed-new".to_string();
        mixed_new_banner.jp_release_date = utc_date(2022, 3, 1, 3);
        mixed_new_banner.pickup_card_ids = vec![100_101, 100_202];

        let mut pick_two = test_timeline_event_with_type(
            "pick-two",
            BannerTimelineEventType::SupportCardBanner,
            later_date,
            later_date,
            false,
        );
        pick_two.gacha_type = Some(12);
        pick_two.gacha_type_name = Some(gacha_type_name(12));
        pick_two.card_type = Some("support".to_string());
        pick_two.pickup_card_ids = vec![30_001, 30_002];

        let mut events = vec![
            original,
            same_day_duplicate,
            rerun,
            mixed_new_banner,
            pick_two,
        ];
        annotate_rerun_banners(&mut events);

        assert!(!events[0].tags.contains(&"rerun-banner"));
        assert!(!events[1].tags.contains(&"rerun-banner"));
        assert!(events[2].tags.contains(&"rerun-banner"));
        assert!(!events[3].tags.contains(&"rerun-banner"));
        assert!(events[4].tags.contains(&"rerun-banner"));
        assert_eq!(gacha_type_name(12), "pick_2");
        assert_eq!(gacha_type_name(13), "special_guaranteed");
    }

    #[test]
    fn calendar_likelihood_marks_predicted_month_count_and_date_shape() {
        let mut events = vec![
            test_timeline_event_with_type(
                "confirmed-a",
                BannerTimelineEventType::CharacterBanner,
                utc_date(2022, 6, 1, 0),
                utc_date(2026, 5, 1, 22),
                true,
            ),
            test_timeline_event_with_type(
                "confirmed-b",
                BannerTimelineEventType::CharacterBanner,
                utc_date(2022, 6, 11, 0),
                utc_date(2026, 5, 8, 22),
                true,
            ),
            test_timeline_event_with_type(
                "confirmed-c",
                BannerTimelineEventType::CharacterBanner,
                utc_date(2022, 6, 21, 0),
                utc_date(2026, 5, 15, 22),
                true,
            ),
            test_timeline_event_with_type(
                "confirmed-d",
                BannerTimelineEventType::CharacterBanner,
                utc_date(2022, 7, 1, 0),
                utc_date(2026, 5, 22, 22),
                true,
            ),
            test_timeline_event_with_type(
                "predicted-a",
                BannerTimelineEventType::CharacterBanner,
                utc_date(2022, 7, 11, 0),
                utc_date(2026, 7, 2, 22),
                false,
            ),
            test_timeline_event_with_type(
                "predicted-b",
                BannerTimelineEventType::CharacterBanner,
                utc_date(2022, 7, 21, 0),
                utc_date(2026, 7, 9, 22),
                false,
            ),
            test_timeline_event_with_type(
                "predicted-c",
                BannerTimelineEventType::CharacterBanner,
                utc_date(2022, 8, 1, 0),
                utc_date(2026, 7, 16, 22),
                false,
            ),
            test_timeline_event_with_type(
                "predicted-d",
                BannerTimelineEventType::CharacterBanner,
                utc_date(2022, 8, 11, 0),
                utc_date(2026, 7, 23, 22),
                false,
            ),
            test_timeline_event_with_type(
                "predicted-e",
                BannerTimelineEventType::CharacterBanner,
                utc_date(2022, 8, 21, 0),
                utc_date(2026, 7, 30, 22),
                false,
            ),
        ];
        let model = CalendarLikelihoodModel::from_events(&events);

        annotate_calendar_likelihoods(&mut events, &model);

        let likelihood = events[4]
            .prediction
            .calendar_likelihood
            .as_ref()
            .expect("unconfirmed event should have calendar likelihood");
        assert_eq!(likelihood.month_character_banner_count, 5);
        assert_eq!(likelihood.weekday, "thursday");
        assert_eq!(likelihood.next_character_gap_days, Some(7));
        assert_eq!(likelihood.next_character_gap_probability, Some(1.0));
        assert!(likelihood.score > 0.0);
    }

    #[test]
    fn calendar_likelihood_uses_matching_event_type_date_shape() {
        let mut events = vec![
            test_timeline_event_with_type(
                "confirmed-character-a",
                BannerTimelineEventType::CharacterBanner,
                utc_date(2022, 6, 1, 0),
                utc_date(2026, 5, 4, 22),
                true,
            ),
            test_timeline_event_with_type(
                "confirmed-character-b",
                BannerTimelineEventType::CharacterBanner,
                utc_date(2022, 6, 11, 0),
                utc_date(2026, 5, 11, 22),
                true,
            ),
            test_timeline_event_with_type(
                "confirmed-story",
                BannerTimelineEventType::StoryEvent,
                utc_date(2022, 6, 2, 0),
                utc_date(2026, 5, 7, 22),
                true,
            ),
            test_timeline_event_with_type(
                "predicted-story",
                BannerTimelineEventType::StoryEvent,
                utc_date(2022, 6, 12, 0),
                utc_date(2026, 5, 14, 22),
                false,
            ),
        ];
        let model = CalendarLikelihoodModel::from_events(&events);

        annotate_calendar_likelihoods(&mut events, &model);

        let likelihood = events[3]
            .prediction
            .calendar_likelihood
            .as_ref()
            .expect("unconfirmed story event should have calendar likelihood");
        assert_eq!(likelihood.weekday, "thursday");
        assert_eq!(likelihood.weekday_probability, 1.0);
        assert!(model.event_type_likelihoods.iter().any(|likelihood| {
            likelihood.event_type == BannerTimelineEventType::StoryEvent
                && likelihood.samples == 1
                && likelihood
                    .weekday_likelihoods
                    .iter()
                    .any(|weekday| weekday.value == "thursday" && weekday.probability == 1.0)
        }));
        assert!(model.event_type_likelihoods.iter().any(|likelihood| {
            likelihood.event_type == BannerTimelineEventType::Campaign && likelihood.samples == 0
        }));
    }

    #[test]
    fn family_adjustments_do_not_leak_between_event_types() {
        let mut models = BTreeMap::new();
        models.insert(
            BannerTimelineEventType::ChampionsMeeting,
            FamilyAdjustmentModel {
                samples: vec![FamilyAdjustmentSample {
                    jp: utc_date(2022, 1, 1, 0),
                    adjustment_days: 3,
                }],
            },
        );
        let models = FamilyAdjustmentModels { models };
        let prediction = DatePrediction {
            global_date: utc_date(2026, 1, 1, 22),
            kind: PredictionKind::Extrapolated,
            acceleration_rate: Some(1.5),
            schedule_adjustment_days: None,
            anchor_jp_date: None,
            anchor_global_date: None,
        };

        let champions_prediction = apply_family_adjustment(
            prediction.clone(),
            BannerTimelineEventType::ChampionsMeeting,
            utc_date(2022, 1, 1, 0),
            &models,
        );
        let story_prediction = apply_family_adjustment(
            prediction,
            BannerTimelineEventType::StoryEvent,
            utc_date(2022, 1, 1, 0),
            &models,
        );

        assert_eq!(champions_prediction.global_date, utc_date(2026, 1, 4, 22));
        assert_eq!(champions_prediction.schedule_adjustment_days, Some(3));
        assert_eq!(story_prediction.global_date, utc_date(2026, 1, 1, 22));
        assert_eq!(story_prediction.schedule_adjustment_days, None);
    }

    fn test_timeline_event(
        id: &str,
        jp_release_date: chrono::DateTime<chrono::Utc>,
        global_release_date: chrono::DateTime<chrono::Utc>,
        is_confirmed: bool,
    ) -> BannerTimelineEvent {
        test_timeline_event_with_type(
            id,
            BannerTimelineEventType::CharacterBanner,
            jp_release_date,
            global_release_date,
            is_confirmed,
        )
    }

    fn empty_confirmed_date_lookup() -> ConfirmedDateLookup {
        ConfirmedDateLookup {
            character: BTreeMap::new(),
            support: BTreeMap::new(),
            paid: BTreeMap::new(),
            story: BTreeMap::new(),
            champions: BTreeMap::new(),
            legend: BTreeMap::new(),
            campaign: BTreeMap::new(),
            news_events: BTreeMap::new(),
            anniversary: BTreeMap::new(),
            closed_global_months: BTreeSet::new(),
        }
    }

    fn test_character_banner(
        gacha_id: i64,
        start_at: chrono::DateTime<chrono::Utc>,
        pickup_card_ids: Vec<i64>,
    ) -> CharacterBanner {
        CharacterBanner {
            gacha_id,
            gacha_type: None,
            year: 2025,
            image: format!("2025_{gacha_id}.webp"),
            start_date: String::new(),
            end_date: String::new(),
            pickup_card_ids,
            image_path: String::new(),
            start_date_string: String::new(),
            end_date_string: String::new(),
            start_at,
        }
    }

    fn test_support_banner(
        gacha_id: i64,
        start_at: chrono::DateTime<chrono::Utc>,
        pickup_card_ids: Vec<i64>,
    ) -> SupportBanner {
        SupportBanner {
            gacha_id,
            gacha_type: None,
            year: 2025,
            image: format!("2025_{gacha_id}.webp"),
            start_date: String::new(),
            end_date: String::new(),
            pickup_card_ids,
            start_at,
        }
    }

    fn test_timeline_character_banner(
        gacha_id: i64,
        pickup_card_ids: Vec<i64>,
    ) -> TimelineCharacterBanner {
        TimelineCharacterBanner {
            gacha_id,
            year: 2022,
            image: format!("2022_{gacha_id}.webp"),
            image_path: format!("assets/images/character/banner/2022_{gacha_id}.webp"),
            start_at: utc_date(2022, 10, 17, 3),
            end_at: utc_date(2022, 10, 24, 2),
            pickup_card_ids,
        }
    }

    fn test_timeline_support_banner(
        gacha_id: i64,
        pickup_card_ids: Vec<i64>,
    ) -> TimelineSupportBanner {
        TimelineSupportBanner {
            gacha_id,
            year: 2022,
            image: format!("2022_{gacha_id}.webp"),
            start_at: utc_date(2022, 10, 17, 3),
            end_at: utc_date(2022, 10, 24, 2),
            pickup_card_ids,
        }
    }

    fn anniversary_schedule_anchors(lookup: &ConfirmedDateLookup) -> Vec<CalibrationAnchor> {
        let banner_anchors = vec![
            CalibrationAnchor {
                jp: utc_date(2022, 2, 16, 0),
                global: utc_date(2026, 3, 5, 22),
            },
            CalibrationAnchor {
                jp: utc_date(2022, 2, 24, 0),
                global: utc_date(2026, 3, 12, 22),
            },
            CalibrationAnchor {
                jp: utc_date(2022, 3, 7, 0),
                global: utc_date(2026, 3, 22, 22),
            },
        ];

        build_anniversary_schedule_anchors(&banner_anchors, lookup)
    }

    fn test_timeline_event_with_type(
        id: &str,
        event_type: BannerTimelineEventType,
        jp_release_date: chrono::DateTime<chrono::Utc>,
        global_release_date: chrono::DateTime<chrono::Utc>,
        is_confirmed: bool,
    ) -> BannerTimelineEvent {
        BannerTimelineEvent {
            id: id.to_string(),
            event_type,
            source: "test",
            gacha_id: None,
            gacha_ids: Vec::new(),
            gacha_type: None,
            gacha_type_name: None,
            card_type: None,
            year: None,
            image: String::new(),
            image_path: None,
            title: id.to_string(),
            description: None,
            jp_release_date,
            global_release_date,
            estimated_end_date: global_release_date,
            is_confirmed,
            banner_duration_days: 7,
            tags: Vec::new(),
            pickup_card_ids: Vec::new(),
            related_characters: Vec::new(),
            related_support_cards: Vec::new(),
            related_support_card_names: Vec::new(),
            gametora_url: None,
            umapyoi_url: None,
            prediction: PredictionInfo {
                kind: if is_confirmed {
                    PredictionKind::Confirmed
                } else {
                    PredictionKind::Extrapolated
                },
                acceleration_rate: None,
                schedule_adjustment_days: None,
                calendar_likelihood: None,
                anchor_jp_date: None,
                anchor_global_date: None,
            },
        }
    }

    fn assert_webp_reference(image: &str) {
        assert!(
            image.ends_with(".webp"),
            "expected timeline image reference to use .webp: {image}"
        );
        assert!(
            !image.to_ascii_lowercase().ends_with(".png"),
            "timeline image reference still uses .png: {image}"
        );
    }
}

use crate::generators::banners::{CharacterBanner, PaidBanner, SupportBanner};
use crate::generators::common;
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
const CONFIRMED_GLOBAL_BANNER_DATES_CSV: &str =
    include_str!("../jp_data/confirmed_global_banner_dates.csv");

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
const TIMELINE_ALGORITHM_VERSION: u8 = 9;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gametora_url: Option<String>,
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
}

fn timeline_event_types() -> [BannerTimelineEventType; 7] {
    [
        BannerTimelineEventType::CharacterBanner,
        BannerTimelineEventType::SupportCardBanner,
        BannerTimelineEventType::PaidBanner,
        BannerTimelineEventType::StoryEvent,
        BannerTimelineEventType::ChampionsMeeting,
        BannerTimelineEventType::LegendRace,
        BannerTimelineEventType::Campaign,
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
}

#[derive(Debug, Clone)]
struct TimelineLegendRace {
    index: usize,
    race_name: String,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    course: Option<String>,
    bosses: Vec<TimelineLegendBoss>,
}

#[derive(Debug, Clone)]
struct TimelineLegendBoss {
    image: String,
}

#[derive(Debug, Clone)]
struct TimelineCampaign {
    campaign_id: i64,
    image: String,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
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
    image: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawTimelineCampaign {
    campaign_id: i64,
    image: String,
    start_date: String,
    end_date: String,
}

#[derive(Debug, Deserialize)]
struct RawSupportCardName {
    id: String,
    name: String,
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
    character_banners: &[CharacterBanner],
    support_banners: &[SupportBanner],
    paid_banners: &[PaidBanner],
) -> Result<BannerTimeline> {
    let timeline_character_banners = load_timeline_character_banners()?;
    let timeline_support_banners = load_timeline_support_banners()?;
    let timeline_paid_banners = load_timeline_paid_banners()?;
    let timeline_story_events = load_timeline_story_events()?;
    let timeline_champions_meetings = load_timeline_champions_meetings()?;
    let timeline_legend_races = load_timeline_legend_races()?;
    let timeline_campaigns = load_timeline_campaigns()?;
    let character_names = common::load_character_name_map(connection)?;
    let support_names = load_support_card_names(connection, &character_names)?;
    let confirmed_dates =
        build_confirmed_date_lookup(character_banners, support_banners, paid_banners)?;
    let anchors = build_banner_confirmed_anchors(
        &timeline_character_banners,
        &timeline_support_banners,
        &confirmed_dates,
    );
    let unique_anchors = monotonic_schedule_anchors(&anchors);
    let observed_rate = calculate_recent_acceleration_rate(&unique_anchors);
    let family_adjustments = build_family_adjustment_models(
        &timeline_paid_banners,
        &timeline_story_events,
        &timeline_champions_meetings,
        &timeline_legend_races,
        &timeline_campaigns,
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
            &confirmed_dates,
            &unique_anchors,
            observed_rate,
        )
    }));
    events.extend(paid_events(
        &timeline_paid_banners,
        &character_names,
        &support_names,
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

    apply_closed_schedule_adjustment(&mut events, &confirmed_dates);
    apply_grouped_event_adjustment(&mut events);
    apply_closed_schedule_adjustment(&mut events, &confirmed_dates);
    let calendar_likelihood_model = CalendarLikelihoodModel::from_events(&events);
    annotate_calendar_likelihoods(&mut events, &calendar_likelihood_model);
    let anniversaries = timeline_anniversaries(&events, &confirmed_dates);

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
        gacha_type: None,
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
        gametora_url: Some(format!(
            "https://gametora.com/umamusume/gacha/history?server=ja&year={}&type=char#{}",
            banner.year, banner_id
        )),
        prediction: prediction.into_info(),
    }
}

fn support_event(
    banner: &TimelineSupportBanner,
    support_names: &BTreeMap<i64, String>,
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    observed_rate: f64,
) -> BannerTimelineEvent {
    let names = banner
        .pickup_card_ids
        .iter()
        .map(|card_id| support_name_for_card(*card_id, support_names))
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
        gacha_type: None,
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
        gametora_url: Some(format!(
            "https://gametora.com/umamusume/gacha/history?server=ja&year={}&type=sup#{}",
            banner.year, banner_id
        )),
        prediction: prediction.into_info(),
    }
}

fn paid_events(
    banners: &[TimelinePaidBanner],
    character_names: &BTreeMap<i64, String>,
    support_names: &BTreeMap<i64, String>,
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
                card_type: Some(representative.card_type.to_string()),
                year: Some(representative.year),
                image: String::new(),
                image_path: None,
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
                gametora_url: None,
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
        gametora_url: None,
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
    let confirmed_global_date = confirmed_dates.story.get(&key).copied();
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
        id: format!("story-event-{}", image_stem(&event.image)),
        event_type: BannerTimelineEventType::StoryEvent,
        source: "story",
        gacha_id: None,
        gacha_ids: Vec::new(),
        gacha_type: None,
        card_type: None,
        year: None,
        image: event.image.clone(),
        image_path: Some(format!("assets/images/story/{}", event.image)),
        title: event.event_name.clone(),
        description: Some(format!("Story Event: {}", event.event_name)),
        jp_release_date: event.start_at,
        global_release_date: prediction.global_date,
        estimated_end_date: calculate_end_date(prediction.global_date, duration),
        is_confirmed: confirmed_global_date.is_some(),
        banner_duration_days: duration,
        tags: vec!["event", "story-event"],
        pickup_card_ids: Vec::new(),
        related_characters: Vec::new(),
        related_support_cards: Vec::new(),
        gametora_url: None,
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

    BannerTimelineEvent {
        id: format!("champions-meeting-{}", event.index),
        event_type: BannerTimelineEventType::ChampionsMeeting,
        source: "champions",
        gacha_id: None,
        gacha_ids: Vec::new(),
        gacha_type: None,
        card_type: None,
        year: None,
        image: String::new(),
        image_path: None,
        title: format!("Champions Meeting: {}", event.name),
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
        gametora_url: None,
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
    let boss_images = event
        .bosses
        .iter()
        .map(|boss| format!("assets/images/legend/boss/{}", boss.image))
        .collect::<Vec<_>>();

    BannerTimelineEvent {
        id: format!("legend-race-{}", event.index),
        event_type: BannerTimelineEventType::LegendRace,
        source: "legend",
        gacha_id: None,
        gacha_ids: Vec::new(),
        gacha_type: None,
        card_type: None,
        year: None,
        image: event
            .bosses
            .first()
            .map(|boss| boss.image.clone())
            .unwrap_or_default(),
        image_path: boss_images.first().cloned(),
        title: event.race_name.clone(),
        description: event.course.clone(),
        jp_release_date: event.start_at,
        global_release_date: prediction.global_date,
        estimated_end_date: calculate_end_date(prediction.global_date, duration + adjustment),
        is_confirmed: confirmed_global_date.is_some(),
        banner_duration_days: duration,
        tags: vec!["event", "legend-race"],
        pickup_card_ids: Vec::new(),
        related_characters: boss_images,
        related_support_cards: Vec::new(),
        gametora_url: None,
        prediction: prediction.into_info(),
    }
}

fn campaign_event(
    event: &TimelineCampaign,
    confirmed_dates: &ConfirmedDateLookup,
    anchors: &[CalibrationAnchor],
    family_adjustments: &FamilyAdjustmentModels,
    observed_rate: f64,
) -> BannerTimelineEvent {
    let key = image_key(&event.image);
    let confirmed_global_date = confirmed_dates
        .campaign
        .get(&key)
        .or_else(|| confirmed_dates.campaign.get(&event.campaign_id.to_string()))
        .copied();
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
    let duration = banner_duration_days(event.start_at, event.end_at);
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
        card_type: None,
        year: None,
        image: event.image.clone(),
        image_path: Some(format!("assets/images/campaign/{}", event.image)),
        title: "Mission Campaign".to_string(),
        description: None,
        jp_release_date: event.start_at,
        global_release_date: prediction.global_date,
        estimated_end_date: calculate_end_date(prediction.global_date, duration + adjustment),
        is_confirmed: confirmed_global_date.is_some(),
        banner_duration_days: duration,
        tags: vec!["mission campaign"],
        pickup_card_ids: Vec::new(),
        related_characters: Vec::new(),
        related_support_cards: Vec::new(),
        gametora_url: None,
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
        if let Some(global) = confirmed_dates
            .campaign
            .get(&image_key(&event.image))
            .or_else(|| confirmed_dates.campaign.get(&event.campaign_id.to_string()))
            .copied()
        {
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

fn build_confirmed_date_lookup(
    character_banners: &[CharacterBanner],
    support_banners: &[SupportBanner],
    paid_banners: &[PaidBanner],
) -> Result<ConfirmedDateLookup> {
    let mut lookup = ConfirmedDateLookup {
        character: BTreeMap::new(),
        support: BTreeMap::new(),
        paid: BTreeMap::new(),
        story: BTreeMap::new(),
        champions: BTreeMap::new(),
        legend: BTreeMap::new(),
        campaign: BTreeMap::new(),
        anniversary: BTreeMap::new(),
        closed_global_months: BTreeSet::new(),
    };

    for banner in character_banners {
        lookup.character.insert(banner.gacha_id, banner.start_at);
    }
    for banner in support_banners {
        lookup.support.insert(banner.gacha_id, banner.start_at);
    }
    for banner in paid_banners {
        lookup.paid.insert(banner.gacha_id, banner.start_at);
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

pub(crate) fn confirmed_dates_version_hash() -> Result<String> {
    let hash_input = format!(
        "timeline_algorithm_version={}\n{}",
        TIMELINE_ALGORITHM_VERSION,
        confirmed_banner_dates_csv_sources()?.join("\n# external confirmed dates\n")
    );
    Ok(hex::encode(Sha256::digest(hash_input.as_bytes())))
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
        ConfirmedTimelineKind::Anniversary => {
            parse_confirmed_anniversary_index(value).map(|index| index.to_string())
        }
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
) -> Vec<TimelineAnniversary> {
    let end_date = events
        .iter()
        .map(|event| event.global_release_date)
        .max()
        .map(|latest| latest + Duration::days(14))
        .unwrap_or_else(global_timeline_start_date);

    timeline_anniversaries_through(end_date, confirmed_dates)
}

fn timeline_anniversaries_through(
    end_date: DateTime<Utc>,
    confirmed_dates: &ConfirmedDateLookup,
) -> Vec<TimelineAnniversary> {
    let mut anniversaries = Vec::new();
    let mut index = 1;

    loop {
        let global_date = confirmed_dates
            .anniversary
            .get(&index)
            .copied()
            .unwrap_or_else(|| projected_global_anniversary_date(index));

        if global_date > end_date {
            break;
        }

        anniversaries.push(TimelineAnniversary {
            index,
            label: anniversary_label(index),
            jp_date: projected_jp_anniversary_date(index),
            global_date,
            is_confirmed: confirmed_dates.anniversary.contains_key(&index),
            schedule_adjustment_days: None,
        });

        index += 1;
    }

    apply_closed_anniversary_adjustment(&mut anniversaries, confirmed_dates);
    anniversaries
}

fn apply_closed_anniversary_adjustment(
    anniversaries: &mut [TimelineAnniversary],
    confirmed_dates: &ConfirmedDateLookup,
) {
    let Some(schedule_floor) = latest_closed_global_month(confirmed_dates)
        .map(|(year, month)| first_release_after_global_month(year, month))
    else {
        return;
    };

    let Some(first_unconfirmed) = anniversaries
        .iter()
        .filter(|anniversary| !anniversary.is_confirmed && anniversary.global_date < schedule_floor)
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

fn projected_global_anniversary_date(index: u32) -> DateTime<Utc> {
    add_calendar_months(global_timeline_start_date(), index * 6, 22)
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

fn load_timeline_story_events() -> Result<Vec<TimelineStoryEvent>> {
    let raw_events: Vec<RawTimelineStoryEvent> =
        serde_json::from_slice(BUNDLED_TIMELINE_STORY_EVENTS_JSON)
            .context("failed to parse bundled timeline_story_events.json")?;

    raw_events
        .into_iter()
        .map(|event| {
            Ok(TimelineStoryEvent {
                event_name: event.event_name,
                image: webp_image_reference(&event.image),
                start_at: parse_timeline_date(&event.start_date)?,
                end_at: parse_timeline_date(&event.end_date)?,
            })
        })
        .collect()
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
            })
        })
        .collect::<Result<Vec<_>>>()?;

    events.sort_by_key(|event| (event.start_at, event.name.clone()));
    for (index, event) in events.iter_mut().enumerate() {
        event.index = index;
    }

    Ok(events)
}

fn load_timeline_legend_races() -> Result<Vec<TimelineLegendRace>> {
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
                .filter_map(|boss| boss.image)
                .map(|image| TimelineLegendBoss {
                    image: webp_image_reference(&image),
                })
                .collect::<Vec<_>>();

            Ok(TimelineLegendRace {
                index: 0,
                race_name: event.race_name,
                start_at: parse_timeline_date(&event.start_date)?,
                end_at: parse_timeline_date(&event.end_date)?,
                course: event.course,
                bosses,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    events.sort_by_key(|event| (event.start_at, event.race_name.clone()));
    for (index, event) in events.iter_mut().enumerate() {
        event.index = index;
    }

    Ok(events)
}

fn load_timeline_campaigns() -> Result<Vec<TimelineCampaign>> {
    let raw_campaigns: Vec<RawTimelineCampaign> =
        serde_json::from_slice(BUNDLED_TIMELINE_CAMPAIGNS_JSON)
            .context("failed to parse bundled timeline_campaigns.json")?;

    raw_campaigns
        .into_iter()
        .map(|campaign| {
            Ok(TimelineCampaign {
                campaign_id: campaign.campaign_id,
                image: webp_image_reference(&campaign.image),
                start_at: parse_timeline_date(&campaign.start_date)?,
                end_at: parse_timeline_date(&campaign.end_date)?,
            })
        })
        .collect()
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
        annotate_calendar_likelihoods, apply_closed_schedule_adjustment, apply_family_adjustment,
        calculate_global_date, calculate_recent_acceleration_rate,
        first_release_after_global_month, latest_closed_global_month,
        load_bundled_support_card_names, load_timeline_campaigns, load_timeline_character_banners,
        load_timeline_legend_races, load_timeline_paid_banners, load_timeline_story_events,
        load_timeline_support_banners, monotonic_schedule_anchors, parse_confirmed_banner_dates,
        timeline_anniversaries_through, utc_date, BannerTimelineEvent, BannerTimelineEventType,
        CalendarLikelihoodModel, CalibrationAnchor, ConfirmedDateLookup, ConfirmedTimelineKind,
        DatePrediction, FamilyAdjustmentModel, FamilyAdjustmentModels, FamilyAdjustmentSample,
        PredictionInfo, PredictionKind,
    };
    use std::collections::{BTreeMap, BTreeSet};

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
            anniversary,1,2025-10-26
            "#,
        )
        .expect("test confirmed date CSV should parse");

        assert_eq!(dates.len(), 7);
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
        assert_eq!(dates[6].kind, ConfirmedTimelineKind::Anniversary);
        assert_eq!(dates[6].key, "1");
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

        for event in load_timeline_story_events().expect("story timeline data should parse") {
            assert_webp_reference(&event.image);
        }

        for event in load_timeline_legend_races().expect("legend timeline data should parse") {
            for boss in event.bosses {
                assert_webp_reference(&boss.image);
            }
        }

        for campaign in load_timeline_campaigns().expect("campaign timeline data should parse") {
            assert_webp_reference(&campaign.image);
        }
    }

    #[test]
    fn timeline_anniversaries_include_confirmed_and_projected_markers() {
        let mut lookup = empty_confirmed_date_lookup();
        lookup.anniversary.insert(1, utc_date(2025, 10, 26, 22));
        let anniversaries = timeline_anniversaries_through(utc_date(2026, 7, 1, 22), &lookup);

        assert_eq!(anniversaries.len(), 2);
        assert_eq!(anniversaries[0].index, 1);
        assert_eq!(anniversaries[0].label, "0.5 Year Anniversary");
        assert_eq!(anniversaries[0].global_date, utc_date(2025, 10, 26, 22));
        assert_eq!(anniversaries[0].jp_date, utc_date(2021, 8, 24, 0));
        assert!(anniversaries[0].is_confirmed);
        assert_eq!(anniversaries[1].index, 2);
        assert_eq!(anniversaries[1].label, "1 Year Anniversary");
        assert_eq!(anniversaries[1].global_date, utc_date(2026, 6, 26, 22));
        assert_eq!(anniversaries[1].jp_date, utc_date(2022, 2, 24, 0));
        assert!(!anniversaries[1].is_confirmed);
    }

    #[test]
    fn closed_schedule_month_shifts_unconfirmed_anniversary_markers() {
        let mut lookup = empty_confirmed_date_lookup();
        lookup.anniversary.insert(1, utc_date(2025, 10, 26, 22));
        lookup.closed_global_months.insert((2026, 6));

        let anniversaries = timeline_anniversaries_through(utc_date(2026, 7, 1, 22), &lookup);

        assert_eq!(anniversaries.len(), 2);
        assert_eq!(anniversaries[0].global_date, utc_date(2025, 10, 26, 22));
        assert!(anniversaries[0].is_confirmed);
        assert_eq!(anniversaries[0].schedule_adjustment_days, None);
        assert_eq!(anniversaries[1].global_date, utc_date(2026, 7, 1, 22));
        assert!(!anniversaries[1].is_confirmed);
        assert_eq!(anniversaries[1].schedule_adjustment_days, Some(5));
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
            anniversary: BTreeMap::new(),
            closed_global_months: BTreeSet::new(),
        }
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
            gametora_url: None,
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

use crate::generators::banners::{CharacterBanner, PaidBanner, SupportBanner};
use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Months, NaiveDate, NaiveDateTime, TimeZone, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const ALGORITHM_VERSION: u8 = 24;
const STANDARD_PICKUP_RATE: f64 = 0.0075;
const STANDARD_RARITY_RATES: [(i64, f64); 3] = [(3, 0.03), (2, 0.18), (1, 0.79)];
const JEWEL_CATEGORY: i64 = 90;
const JEWEL_ITEM_ID: i64 = 43;
const GACHA_TICKET_CATEGORY: i64 = 40;
const UMA_TICKET_ITEM_ID: i64 = 41;
const SUPPORT_TICKET_ITEM_ID: i64 = 111;
const LIMIT_BREAK_ITEM_CATEGORY: i64 = 164;
const RAINBOW_CRYSTAL_ITEM_ID: i64 = 149;
const GOLD_CRYSTAL_ITEM_ID: i64 = 150;
const TRAINEE_SELECTOR_CATEGORY: i64 = 41;
const SUPPORT_SELECTOR_CATEGORY: i64 = 42;
const ITEM_NAME_TEXT_CATEGORY: i64 = 23;
const DEFAULT_JEWEL_COST_PER_PULL: i64 = 150;
const DEFAULT_SPARK_PULLS: i64 = 200;
const GLOBAL_LAUNCH_TIMESTAMP: i64 = 1_750_896_000;
const FRIEND_POINT_EXCHANGE_ID: i64 = 2_000;
const CLOVER_EXCHANGE_ID: i64 = 4_000;
const TRAINER_SKILLS_TEST_CURRENCY_CATEGORY: i64 = 165;
const RACING_CARNIVAL_CURRENCY_CATEGORY: i64 = 166;
const UMAPYOI_ARCHIVE: &[u8] = include_bytes!("../jp_data/umapyoi_archive.json");
const GLOBAL_NEWS_ARCHIVE: &[u8] = include_bytes!("../global_data/official_news_archive.json");
const GLOBAL_SOCIAL_ARCHIVE: &[u8] = include_bytes!("../global_data/official_social_archive.json");
const TIMELINE_CAMPAIGNS: &[u8] = include_bytes!("../jp_data/timeline_campaigns.json");
const JP_MISSION_REWARDS: &[u8] = include_bytes!("../jp_data/planner_mission_rewards.json");

#[derive(Debug)]
pub struct GeneratedPlanner {
    pub core: PlannerCore,
    pub income: PlannerIncome,
    pub rewards: PlannerRewards,
    pub gacha_shards: Vec<PlannerGachaShard>,
    linked_event_ids: BTreeSet<String>,
    reward_event_ids: BTreeSet<String>,
    linked_gacha_ids: BTreeSet<i64>,
}

#[derive(Debug, Serialize)]
pub struct PlannerCore {
    pub version: u8,
    pub jewel_cost_per_pull: i64,
    pub default_spark_pulls: i64,
    pub gacha_shard_by_event: BTreeMap<String, String>,
    pub gacha_shard_by_id: BTreeMap<String, String>,
    pub gacha_shards: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PlannerIncome {
    pub version: u8,
    pub rules: Vec<PlannerIncomeRule>,
}

#[derive(Debug, Serialize)]
pub struct PlannerIncomeRule {
    pub id: String,
    pub label: String,
    pub currency: &'static str,
    pub amount: i64,
    pub cadence: &'static str,
    pub start_date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day_of_month: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    pub default_enabled: bool,
    pub provenance: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scenario_group: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scenario_option: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PlannerRewards {
    pub version: u8,
    pub rewards: Vec<PlannerReward>,
    pub global_reward_comparison: PlannerGlobalRewardComparison,
    pub event_benefits: Vec<PlannerEventBenefit>,
    pub free_pull_campaigns: Vec<PlannerFreePullCampaign>,
    pub competitive_variants: Vec<PlannerCompetitiveVariant>,
    pub news_details: Vec<PlannerNewsDetail>,
}

#[derive(Debug, Serialize)]
pub struct PlannerGlobalRewardComparison {
    pub news_match_method: &'static str,
    pub speculative_method: &'static str,
    pub archive_as_of: String,
    pub observation_start: String,
    pub observation_end: String,
    pub observation_days: i64,
    pub observed_months: f64,
    pub matched_news_global_carats: i64,
    pub matched_news_jp_carats: i64,
    pub matched_news_extra_carats: i64,
    pub en_only_news_carats: i64,
    pub social_carats: i64,
    pub social_reward_posts: usize,
    pub social_news_duplicate_reward_items_removed: usize,
    pub social_news_duplicate_carats_removed: i64,
    pub speculative_observed_carats: i64,
    pub speculative_mean_monthly_carats: i64,
    pub speculative_recent_median_monthly_carats: i64,
    pub speculative_recent_median_window_start: String,
    pub speculative_recent_median_window_end: String,
    pub speculative_monthly_carats: i64,
    pub speculative_window_start: String,
    pub speculative_window_end: String,
    pub speculative_months: Vec<PlannerGlobalRewardMonth>,
    pub matched_news: Vec<PlannerGlobalNewsComparison>,
    pub en_only_news: Vec<PlannerGlobalNewsComparison>,
}

#[derive(Debug, Serialize)]
pub struct PlannerGlobalRewardMonth {
    pub month: String,
    pub matched_news_extra_carats: i64,
    pub en_only_news_carats: i64,
    pub social_carats: i64,
    pub total_carats: i64,
}

#[derive(Debug, Serialize)]
pub struct PlannerGlobalNewsComparison {
    pub announce_id: i64,
    pub title: String,
    pub global_carats: i64,
    pub jp_carats: i64,
    pub extra_carats: i64,
    pub global_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jp_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannerFreePullCampaign {
    pub id: String,
    pub label: String,
    pub total_pulls: i64,
    pub pulls_per_day: i64,
    pub entitlement_days: i64,
    pub allocation_mode: &'static str,
    pub eligible_gacha_ids: Vec<i64>,
    pub default_allocations: Vec<PlannerFreePullAllocation>,
    pub source_url: String,
    pub provenance: &'static str,
    pub confidence: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PlannerFreePullAllocation {
    pub event_id: String,
    pub gacha_id: i64,
    pub pulls: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannerEventBenefit {
    pub id: String,
    pub event_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<String>,
    pub kind: &'static str,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_category: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<i64>,
    pub amount: i64,
    pub available_at: String,
    pub planner_effect: &'static str,
    pub provenance: &'static str,
    pub confidence: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannerReward {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_id: Option<i64>,
    pub currency: &'static str,
    pub amount: Option<i64>,
    pub available_at: String,
    pub provenance: &'static str,
    pub assumption: &'static str,
    pub default_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source_items: Vec<PlannerSourceItem>,
    pub confidence: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannerSourceItem {
    pub item_category: i64,
    pub item_id: i64,
    pub amount: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub odds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_max: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bonus: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannerCompetitiveVariant {
    pub id: String,
    pub competition: &'static str,
    pub event_id: String,
    pub master_event_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_at: Option<String>,
    pub label: String,
    pub source_items: Vec<PlannerSourceItem>,
    pub provenance: &'static str,
    pub confidence: &'static str,
    pub default_enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct PlannerNewsDetail {
    pub id: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    pub title: String,
    pub available_at: String,
    pub summary: String,
    pub source_url: String,
    pub provenance: &'static str,
    pub confidence: &'static str,
}

#[derive(Debug, Serialize)]
pub struct PlannerGachaShard {
    pub shard: String,
    pub gachas: Vec<PlannerGacha>,
}

#[derive(Debug, Serialize)]
pub struct PlannerGacha {
    pub event_id: String,
    pub gacha_id: i64,
    pub gacha_type: i64,
    pub banner_kind: String,
    pub start_date: String,
    pub end_date: String,
    pub jewel_cost_per_pull: i64,
    pub spark_pulls: i64,
    pub free_pulls: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_pulls_provenance: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_pulls_source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_pulls_confidence: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket_currency: Option<&'static str>,
    pub provenance: &'static str,
    pub confidence: &'static str,
    pub pickups: Vec<PlannerPickup>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub featured_pickups: Vec<PlannerPickup>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rarity_rates: Vec<PlannerRarityRate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannerPickup {
    pub pickup_id: i64,
    pub label: String,
    pub rate: f64,
    pub exchangeable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannerRarityRate {
    pub rarity: i64,
    pub rate: f64,
}

#[derive(Debug, Clone)]
struct TimelineLink {
    event_id: String,
    banner_kind: String,
    start_date: String,
    end_date: String,
    jp_start_date: String,
    pickup_card_ids: Vec<i64>,
    gacha_type: i64,
    is_paid: bool,
}

#[derive(Debug, Default)]
struct GachaAccumulator {
    card_type: i64,
    gacha_type: i64,
    cost: i64,
    spark_pulls: i64,
    free_pulls: i64,
    campaign_id: Option<String>,
    free_pulls_provenance: Option<&'static str>,
    free_pulls_source_url: Option<String>,
    free_pulls_confidence: Option<&'static str>,
    provenance: Option<&'static str>,
    confidence: Option<&'static str>,
    pickups: Vec<PlannerPickup>,
    featured_pickups: Vec<PlannerPickup>,
    rarity_rates: Vec<PlannerRarityRate>,
}

#[derive(Debug, Deserialize)]
struct CampaignLink {
    campaign_id: i64,
    #[serde(default)]
    jp_mission_event_id: Option<i64>,
    #[serde(default)]
    mission_fingerprint: Option<String>,
    #[serde(default)]
    mission_count: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct JpMissionRewardGroup {
    jp_mission_event_id: i64,
    #[serde(default)]
    jp_title: Option<String>,
    start_date: String,
    end_date: String,
    mission_count: i64,
    mission_fingerprint: String,
    #[serde(default)]
    rewards: Vec<PlannerSourceItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NewsFreePullClaim {
    banner_kind: Option<&'static str>,
    pulls_per_day: i64,
    days: Option<i64>,
    explicit_total: Option<i64>,
    campaign_start: Option<(u32, u32, u32, u32)>,
    evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NewsFreePullCampaignClaim {
    banner_kinds: BTreeSet<&'static str>,
    pulls_per_day: i64,
    entitlement_days: i64,
    total_pulls: i64,
    campaign_start: Option<(u32, u32, u32, u32)>,
    has_explicit_total: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NewsFreePullTargetWindow {
    banner_kind: &'static str,
    pin_to_window_start: bool,
    start: (u32, u32, u32, u32),
    end: (u32, u32, u32, u32),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NewsFreePullCampaignKey {
    // News summaries sometimes round or omit the exact campaign start time.
    // Group by the JP calendar day so the same campaign cannot be emitted
    // twice merely because two posts resolved a few hours apart.
    campaign_jp_day: i64,
    banner_kinds: Vec<&'static str>,
    pulls_per_day: i64,
    entitlement_days: i64,
    total_pulls: i64,
}

#[derive(Debug, Clone)]
struct NewsFreePullCampaignCandidate {
    post_id: i64,
    label: String,
    source_url: String,
    campaign: NewsFreePullCampaignClaim,
    assignments: BTreeMap<i64, i64>,
    allocation_mode: &'static str,
    source_specificity: (bool, bool, bool, usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NewsEventLink {
    event_id: String,
    start_date: String,
    end_date: String,
}

#[derive(Debug, Deserialize)]
struct Archive {
    #[serde(default)]
    news: Vec<ArchiveNews>,
}

#[derive(Debug, Deserialize)]
struct ArchiveNews {
    post_id: i64,
    page_url: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    posted_at: Option<String>,
    #[serde(default)]
    event_types: Vec<String>,
    raw: Value,
}

#[derive(Debug, Deserialize)]
struct GlobalNewsArchive {
    #[serde(default)]
    posts: Vec<GlobalNewsPost>,
}

#[derive(Debug, Deserialize)]
struct GlobalNewsPost {
    announce_id: i64,
    page_url: String,
    first_seen_at: String,
    #[serde(default)]
    snapshots: Vec<GlobalNewsSnapshot>,
}

#[derive(Debug, Deserialize)]
struct GlobalNewsSnapshot {
    raw: Value,
}

#[derive(Debug, Deserialize)]
struct GlobalSocialArchive {
    #[serde(default)]
    posts: Vec<GlobalSocialPost>,
}

#[derive(Debug, Deserialize)]
struct GlobalSocialPost {
    status_id: String,
    status_url: String,
    first_seen_at: String,
    #[serde(default)]
    snapshots: Vec<GlobalSocialSnapshot>,
}

#[derive(Debug, Deserialize)]
struct GlobalSocialSnapshot {
    raw: Value,
}

pub fn version_hash() -> String {
    let mut digest = Sha256::new();
    digest.update([ALGORITHM_VERSION]);
    digest.update(UMAPYOI_ARCHIVE);
    digest.update(GLOBAL_NEWS_ARCHIVE);
    digest.update(GLOBAL_SOCIAL_ARCHIVE);
    digest.update(TIMELINE_CAMPAIGNS);
    digest.update(JP_MISSION_REWARDS);
    hex::encode(digest.finalize())
}

pub fn generate(
    connection: &Connection,
    jp_connection: Option<&Connection>,
    _character_banners: &[CharacterBanner],
    _support_banners: &[SupportBanner],
    _paid_banners: &[PaidBanner],
    timeline: &Value,
) -> Result<GeneratedPlanner> {
    let archive = load_archive()?;
    let global_news_archive = load_global_news_archive()?;
    let global_social_archive = load_global_social_archive()?;
    let timeline_links = timeline_gacha_links(timeline);
    let mut gachas = load_gachas_with_jp(connection, jp_connection, &timeline_links)?;
    apply_free_pulls(connection, &mut gachas)?;
    let free_pull_campaigns = apply_news_free_pulls(&archive, &timeline_links, &mut gachas);

    let default_cost = mode(
        gachas
            .values()
            .map(|gacha| gacha.cost)
            .filter(|cost| *cost > 0),
    )
    .unwrap_or(DEFAULT_JEWEL_COST_PER_PULL);
    let default_spark = mode(
        gachas
            .values()
            .map(|gacha| gacha.spark_pulls)
            .filter(|pulls| *pulls > 0),
    )
    .unwrap_or(DEFAULT_SPARK_PULLS);

    let mut sharded: BTreeMap<String, Vec<PlannerGacha>> = BTreeMap::new();
    let mut gacha_shard_by_event = BTreeMap::new();
    let mut gacha_shard_by_id = BTreeMap::new();
    let mut linked_event_ids = BTreeSet::new();
    let mut linked_gacha_ids = BTreeSet::new();

    for (gacha_id, accumulator) in gachas {
        let Some(link) = timeline_links.get(&gacha_id) else {
            continue;
        };
        let shard = link
            .start_date
            .get(..4)
            .filter(|year| year.chars().all(|character| character.is_ascii_digit()))
            .unwrap_or("other")
            .to_string();
        let ticket_currency = match (accumulator.card_type, accumulator.gacha_type) {
            (1, 3 | 11 | 12) => Some("uma_ticket"),
            (2, 3 | 11 | 12) => Some("support_ticket"),
            _ => None,
        };
        gacha_shard_by_event.insert(link.event_id.clone(), shard.clone());
        gacha_shard_by_id.insert(gacha_id.to_string(), shard.clone());
        linked_event_ids.insert(link.event_id.clone());
        linked_gacha_ids.insert(gacha_id);
        sharded.entry(shard).or_default().push(PlannerGacha {
            event_id: link.event_id.clone(),
            gacha_id,
            gacha_type: accumulator.gacha_type,
            banner_kind: link.banner_kind.clone(),
            start_date: link.start_date.clone(),
            end_date: link.end_date.clone(),
            jewel_cost_per_pull: accumulator.cost.max(0),
            spark_pulls: accumulator.spark_pulls.max(0),
            free_pulls: accumulator.free_pulls.max(0),
            campaign_id: accumulator.campaign_id,
            free_pulls_provenance: accumulator.free_pulls_provenance,
            free_pulls_source_url: accumulator.free_pulls_source_url,
            free_pulls_confidence: accumulator.free_pulls_confidence,
            ticket_currency,
            provenance: accumulator.provenance.unwrap_or("global_master"),
            confidence: accumulator.confidence.unwrap_or("exact"),
            pickups: accumulator.pickups,
            featured_pickups: accumulator.featured_pickups,
            rarity_rates: accumulator.rarity_rates,
        });
    }

    let mut gacha_shards = sharded
        .into_iter()
        .map(|(shard, mut gachas)| {
            gachas.sort_by_key(|gacha| (gacha.start_date.clone(), gacha.gacha_id));
            PlannerGachaShard { shard, gachas }
        })
        .collect::<Vec<_>>();
    gacha_shards.sort_by(|left, right| left.shard.cmp(&right.shard));
    let shard_names = gacha_shards
        .iter()
        .map(|shard| shard.shard.clone())
        .collect::<Vec<_>>();

    let mut income_rules = load_income_rules(connection)?;
    income_rules.extend(load_paid_news_income_rules(&archive));
    let income = PlannerIncome {
        version: ALGORITHM_VERSION,
        rules: income_rules,
    };
    let mut global_rewards = load_global_news_rewards(&global_news_archive, &archive, timeline);
    global_rewards.extend(load_global_social_rewards(&global_social_archive));
    let global_social_deduplication =
        remove_global_social_rewards_covered_by_news(&mut global_rewards);
    let jp_news_rewards = load_news_rewards(&archive, timeline);
    let global_archive_as_of = global_archive_as_of(&global_news_archive, &global_social_archive);
    let global_reward_comparison = build_global_reward_comparison(
        &global_rewards,
        &jp_news_rewards,
        &archive,
        global_social_deduplication,
        global_archive_as_of,
    );

    let mut rewards = load_master_rewards(connection, jp_connection, timeline)?;
    rewards.extend(global_rewards);
    rewards.extend(jp_news_rewards);
    remove_global_news_login_bonuses_covered_by_master(&mut rewards);
    prefer_global_news_over_jp_news(&mut rewards);
    project_missing_story_rewards(timeline, &mut rewards);
    project_missing_event_exchange_rewards(
        timeline,
        &mut rewards,
        "trainer_skills_test",
        "Trainer Skills Test exchange rewards",
        "Trainer Skills Test exchange rewards (projected Global parity)",
    );
    project_missing_event_exchange_rewards(
        timeline,
        &mut rewards,
        "racing_carnival",
        "Racing Carnival exchange rewards",
        "Racing Carnival exchange rewards (projected Global parity)",
    );
    deduplicate_rewards(&mut rewards);
    let event_benefits = build_event_benefits(connection, &rewards, &gacha_shards)?;
    let competitive_variants =
        load_competitive_variants_with_jp(connection, jp_connection, timeline)?;
    let news_details = load_news_details(&archive, timeline);
    for reward in &rewards {
        if let Some(event_id) = &reward.event_id {
            linked_event_ids.insert(event_id.clone());
        }
        if let Some(gacha_id) = reward.gacha_id {
            linked_gacha_ids.insert(gacha_id);
        }
    }
    for variant in &competitive_variants {
        linked_event_ids.insert(variant.event_id.clone());
    }
    for detail in &news_details {
        if let Some(event_id) = &detail.event_id {
            linked_event_ids.insert(event_id.clone());
        }
    }
    let mut reward_event_ids = planner_reward_event_ids(&rewards);
    reward_event_ids.extend(
        competitive_variants
            .iter()
            .map(|variant| variant.event_id.clone()),
    );

    Ok(GeneratedPlanner {
        core: PlannerCore {
            version: ALGORITHM_VERSION,
            jewel_cost_per_pull: default_cost,
            default_spark_pulls: default_spark,
            gacha_shard_by_event,
            gacha_shard_by_id,
            gacha_shards: shard_names,
        },
        income,
        rewards: PlannerRewards {
            version: ALGORITHM_VERSION,
            rewards,
            global_reward_comparison,
            event_benefits,
            free_pull_campaigns,
            competitive_variants,
            news_details,
        },
        gacha_shards,
        linked_event_ids,
        reward_event_ids,
        linked_gacha_ids,
    })
}

/// Future story events are a stable recurring reward source. Global master data
/// only has released story tables, so carry the latest confirmed Global table
/// into unconfirmed timeline entries until exact data is available.
fn project_missing_story_rewards(timeline: &Value, rewards: &mut Vec<PlannerReward>) {
    let mut dates = BTreeMap::<String, String>::new();
    let mut confirmed_event_ids = BTreeSet::<String>::new();
    let mut missing_event_ids = Vec::new();
    let existing_event_currencies = rewards
        .iter()
        .filter_map(|reward| Some((reward.event_id.clone()?, reward.currency.to_string())))
        .collect::<BTreeSet<_>>();

    for event in timeline
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if event.get("type").and_then(Value::as_str) != Some("story_event") {
            continue;
        }
        let (Some(event_id), Some(date)) = (
            event.get("id").and_then(Value::as_str),
            event.get("global_release_date").and_then(Value::as_str),
        ) else {
            continue;
        };
        dates.insert(event_id.to_string(), date.to_string());
        if event.get("is_confirmed").and_then(Value::as_bool) == Some(true) {
            confirmed_event_ids.insert(event_id.to_string());
        } else {
            missing_event_ids.push(event_id.to_string());
        }
    }

    let mut templates = BTreeMap::<&'static str, PlannerReward>::new();
    for reward in rewards.iter().filter(|reward| {
        reward.event_id.as_ref().is_some_and(|event_id| {
            confirmed_event_ids.contains(event_id)
                && matches!(
                    reward.currency,
                    "free_jewels" | "uma_ticket" | "support_ticket"
                )
                && reward.amount.is_some_and(|amount| amount > 0)
        })
    }) {
        let replace = templates.get(reward.currency).is_none_or(|existing| {
            let reward_date = reward
                .event_id
                .as_ref()
                .and_then(|event_id| dates.get(event_id));
            let existing_date = existing
                .event_id
                .as_ref()
                .and_then(|event_id| dates.get(event_id));
            reward_date > existing_date
        });
        if replace {
            templates.insert(reward.currency, reward.clone());
        }
    }
    if templates.is_empty() {
        return;
    }

    missing_event_ids.sort_by_key(|event_id| dates.get(event_id).cloned());
    for event_id in missing_event_ids {
        let Some(available_at) = dates.get(&event_id) else {
            continue;
        };
        for template in templates.values() {
            if existing_event_currencies
                .contains(&(event_id.clone(), template.currency.to_string()))
            {
                continue;
            }
            rewards.push(PlannerReward {
                id: format!("projected-story-{event_id}-{}", template.id),
                label: "Story event rewards (projected Global parity)".to_string(),
                event_id: Some(event_id.clone()),
                gacha_id: None,
                currency: template.currency,
                amount: template.amount,
                available_at: available_at.clone(),
                provenance: "global_story_reward_parity",
                assumption: template.assumption,
                default_enabled: true,
                source_url: None,
                source_items: template.source_items.clone(),
                confidence: "projected_global_parity",
                evidence: Some(
                    "Latest confirmed Global story-event reward table; replaced automatically when exact event data is available"
                        .to_string(),
                ),
            });
        }
    }
}

fn project_missing_event_exchange_rewards(
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
    event_type: &str,
    exact_label: &str,
    projected_label: &str,
) {
    let templates = ["uma_ticket", "support_ticket"]
        .into_iter()
        .filter_map(|currency| {
            rewards
                .iter()
                .filter(|reward| {
                    reward.label == exact_label
                        && reward.provenance == "global_master"
                        && reward.currency == currency
                        && reward.confidence == "exact_source"
                })
                .max_by_key(|reward| reward.available_at.as_str())
                .cloned()
                .map(|reward| (currency, reward))
        })
        .collect::<BTreeMap<_, _>>();
    if templates.is_empty() {
        return;
    }

    let existing_event_currencies = rewards
        .iter()
        .filter_map(|reward| Some((reward.event_id.clone()?, reward.currency.to_string())))
        .collect::<BTreeSet<_>>();
    for event in timeline
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some(event_type))
        .filter(|event| event.get("is_confirmed").and_then(Value::as_bool) != Some(true))
    {
        let (Some(event_id), Some(available_at)) = (
            event.get("id").and_then(Value::as_str),
            event
                .get("estimated_end_date")
                .and_then(Value::as_str)
                .or_else(|| event.get("global_release_date").and_then(Value::as_str)),
        ) else {
            continue;
        };
        for template in templates.values() {
            if existing_event_currencies
                .contains(&(event_id.to_string(), template.currency.to_string()))
            {
                continue;
            }
            rewards.push(PlannerReward {
                id: format!(
                    "projected-exchange-{event_type}-{event_id}-{}",
                    template.currency
                ),
                label: projected_label.to_string(),
                event_id: Some(event_id.to_string()),
                gacha_id: None,
                currency: template.currency,
                amount: template.amount,
                available_at: available_at.to_string(),
                provenance: "global_exchange_reward_parity",
                assumption: "full_exchange",
                default_enabled: true,
                source_url: None,
                source_items: template.source_items.clone(),
                confidence: "projected_global_parity",
                evidence: Some(
                    "Projected from exact recurring ticket stock in the Global master exchange tables"
                        .to_string(),
                ),
            });
        }
    }
}
fn planner_reward_event_ids(rewards: &[PlannerReward]) -> BTreeSet<String> {
    rewards
        .iter()
        .filter(|reward| {
            (matches!(
                reward.currency,
                "free_jewels"
                    | "paid_jewels"
                    | "uma_ticket"
                    | "support_ticket"
                    | "rainbow_crystal"
                    | "gold_crystal"
            ) && reward.amount.is_some_and(|amount| amount > 0))
                || reward.source_items.iter().any(|item| {
                    matches!(
                        item.item_category,
                        TRAINEE_SELECTOR_CATEGORY | SUPPORT_SELECTOR_CATEGORY
                    ) && item.amount > 0
                })
        })
        .filter_map(|reward| reward.event_id.clone())
        .collect()
}

fn build_event_benefits(
    connection: &Connection,
    rewards: &[PlannerReward],
    gacha_shards: &[PlannerGachaShard],
) -> Result<Vec<PlannerEventBenefit>> {
    let item_labels = load_planner_item_labels(connection)?;
    let mut benefits = Vec::new();
    let mut seen_selectors = BTreeSet::new();

    for reward in rewards {
        if reward.confidence != "exact_source" {
            continue;
        }
        let Some(event_id) = reward.event_id.as_ref() else {
            continue;
        };
        for item in &reward.source_items {
            if item.amount <= 0 {
                continue;
            }
            let (kind, fallback_label) = match item.item_category {
                TRAINEE_SELECTOR_CATEGORY => ("trainee_selector", "3-star Trainee selector"),
                SUPPORT_SELECTOR_CATEGORY => ("support_selector", "SSR Support Card selector"),
                _ => continue,
            };
            let deduplication_key = (
                event_id.clone(),
                item.item_category,
                item.item_id,
                item.amount,
                reward.available_at.clone(),
            );
            if !seen_selectors.insert(deduplication_key) {
                continue;
            }
            benefits.push(PlannerEventBenefit {
                id: format!(
                    "event-benefit-{event_id}-{}-{}",
                    item.item_category, item.item_id
                ),
                event_id: event_id.clone(),
                gacha_id: None,
                campaign_id: None,
                kind,
                label: item_labels
                    .get(&item.item_id)
                    .filter(|label| !label.trim().is_empty())
                    .cloned()
                    .unwrap_or_else(|| fallback_label.to_string()),
                item_category: Some(item.item_category),
                item_id: Some(item.item_id),
                amount: item.amount,
                available_at: reward.available_at.clone(),
                planner_effect: "informational_only",
                provenance: reward.provenance,
                confidence: reward.confidence,
                source_url: reward.source_url.clone(),
            });
        }
    }

    for shard in gacha_shards {
        for gacha in &shard.gachas {
            if gacha.free_pulls <= 0 {
                continue;
            }
            let label = match gacha.banner_kind.to_ascii_lowercase().as_str() {
                "character" => "Free Trainee pulls",
                "support" => "Free Support Card pulls",
                _ => "Free banner pulls",
            };
            benefits.push(PlannerEventBenefit {
                id: format!(
                    "event-benefit-{}-free-pulls-{}",
                    gacha.event_id, gacha.gacha_id
                ),
                event_id: gacha.event_id.clone(),
                gacha_id: Some(gacha.gacha_id),
                campaign_id: gacha.campaign_id.clone(),
                kind: "free_pulls",
                label: label.to_string(),
                item_category: None,
                item_id: None,
                amount: gacha.free_pulls,
                // The aggregate is only fully earned after the campaign. The
                // source schema does not yet retain the daily entitlement
                // window separately from the linked banner.
                available_at: gacha.end_date.clone(),
                planner_effect: "included_in_banner_free_pulls",
                provenance: gacha.free_pulls_provenance.unwrap_or(gacha.provenance),
                confidence: gacha.free_pulls_confidence.unwrap_or(gacha.confidence),
                source_url: gacha.free_pulls_source_url.clone(),
            });
        }
    }

    benefits.sort_by(|left, right| {
        left.available_at
            .cmp(&right.available_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(benefits)
}

fn load_planner_item_labels(connection: &Connection) -> Result<BTreeMap<i64, String>> {
    let mut statement = connection.prepare(
        r#"
        SELECT "index", text
        FROM text_data
        WHERE category = ?1
        ORDER BY "index"
        "#,
    )?;
    let rows = statement.query_map([ITEM_NAME_TEXT_CATEGORY], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()
        .map_err(Into::into)
}

impl GeneratedPlanner {
    pub fn annotate_timeline(&self, timeline: &mut Value) {
        let Some(events) = timeline.get_mut("events").and_then(Value::as_array_mut) else {
            return;
        };
        for event in events {
            let event_id = event.get("id").and_then(Value::as_str);
            let direct_gacha = event.get("gacha_id").and_then(Value::as_i64);
            let grouped_gacha = event
                .get("gacha_ids")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_i64);
            let available = event_id.is_some_and(|id| self.linked_event_ids.contains(id))
                || direct_gacha.is_some_and(|id| self.linked_gacha_ids.contains(&id))
                || grouped_gacha
                    .into_iter()
                    .any(|id| self.linked_gacha_ids.contains(&id));
            let reward_available = event_id.is_some_and(|id| self.reward_event_ids.contains(id));
            if let Some(object) = event.as_object_mut() {
                object.insert("planner_data_available".to_string(), Value::Bool(available));
                if reward_available {
                    object.insert("planner_reward_available".to_string(), Value::Bool(true));
                } else {
                    object.remove("planner_reward_available");
                }
            }
        }
    }
}

fn timeline_gacha_links(timeline: &Value) -> BTreeMap<i64, TimelineLink> {
    let mut links = BTreeMap::new();
    for event in timeline
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(event_id) = event.get("id").and_then(Value::as_str) else {
            continue;
        };
        let start_date = event
            .get("global_release_date")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let end_date = event
            .get("estimated_end_date")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let jp_start_date = event
            .get("jp_release_date")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let banner_kind = event
            .get("card_type")
            .and_then(Value::as_str)
            .or_else(|| event.get("type").and_then(Value::as_str))
            .unwrap_or("unknown");
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let source = event
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let gacha_type = event
            .get("gacha_type")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let pickup_card_ids = event
            .get("pickup_card_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_i64)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let ids = event
            .get("gacha_id")
            .and_then(Value::as_i64)
            .into_iter()
            .chain(
                event
                    .get("gacha_ids")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_i64),
            );
        for gacha_id in ids {
            links.insert(
                gacha_id,
                TimelineLink {
                    event_id: event_id.to_string(),
                    banner_kind: banner_kind.to_string(),
                    start_date: start_date.to_string(),
                    end_date: end_date.to_string(),
                    jp_start_date: jp_start_date.to_string(),
                    pickup_card_ids: pickup_card_ids.clone(),
                    gacha_type,
                    is_paid: event_type.eq_ignore_ascii_case("paid_banner")
                        || source
                            .split(['_', '-'])
                            .any(|part| part.eq_ignore_ascii_case("paid"))
                        || event_id.starts_with("paid-banner-")
                        // JP paid/guaranteed gachas use the 50xxx namespace.
                        // Keep this fallback for news-derived events whose
                        // type metadata was inferred incorrectly.
                        || (50_000..60_000).contains(&gacha_id),
                },
            );
        }
    }
    links
}

fn timeline_top_rarity_pickup_ids(link: &TimelineLink) -> Vec<i64> {
    link.pickup_card_ids
        .iter()
        .copied()
        .filter(
            |pickup_id| match link.banner_kind.to_ascii_lowercase().as_str() {
                "character" => *pickup_id >= 100_000,
                // Support-card IDs encode rarity in the ten-thousands digit.
                "support" => (30_000..40_000).contains(pickup_id),
                _ => false,
            },
        )
        .collect()
}

fn discard_mismatched_master_gachas(
    timeline_links: &BTreeMap<i64, TimelineLink>,
    gachas: &mut BTreeMap<i64, GachaAccumulator>,
) {
    gachas.retain(|gacha_id, gacha| {
        let Some(link) = timeline_links.get(gacha_id) else {
            return false;
        };
        if link.pickup_card_ids.is_empty() {
            return true;
        }
        // Global and JP gacha IDs are reused on different schedules. A bare
        // ID match is not sufficient evidence that the Global master row
        // belongs to this timeline banner (for example JP 30130/30131).
        gacha
            .featured_pickups
            .iter()
            .chain(gacha.pickups.iter())
            .any(|pickup| {
                link.pickup_card_ids
                    .binary_search(&pickup.pickup_id)
                    .is_ok()
            })
    });
}

fn load_gachas(
    connection: &Connection,
    timeline_links: &BTreeMap<i64, TimelineLink>,
) -> Result<BTreeMap<i64, GachaAccumulator>> {
    let mut statement = connection.prepare(
        r#"
        SELECT data.id,
               data.card_type,
               data.type,
               data.cost_single,
               available.card_id,
               available.rarity,
               available.odds,
               COALESCE((
                   SELECT MIN(exchange.pay_item_num)
                   FROM gacha_exchange AS exchange
                   WHERE exchange.gacha_id = data.id
                     AND exchange.card_id = available.card_id
               ), 0)
        FROM gacha_data AS data
        JOIN gacha_available AS available
          ON available.gacha_id = data.id
         AND available.is_pickup = 1
        ORDER BY data.id, available.card_id
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
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
        ))
    })?;
    let mut gachas = BTreeMap::new();
    for row in rows {
        let (gacha_id, card_type, gacha_type, cost, card_id, rarity, odds, spark) = row?;
        if !timeline_links.contains_key(&gacha_id) {
            continue;
        }
        let gacha = gachas
            .entry(gacha_id)
            .or_insert_with(GachaAccumulator::default);
        gacha.card_type = card_type;
        gacha.gacha_type = gacha_type;
        gacha.cost = cost;
        gacha.spark_pulls = gacha.spark_pulls.max(spark);
        gacha.provenance = Some("global_master");
        gacha.confidence = Some("exact");
        let pickup = PlannerPickup {
            pickup_id: card_id,
            label: if card_type == 1 {
                format!("Umamusume {card_id}")
            } else {
                format!("Support Card {card_id}")
            },
            rate: odds as f64 / 1_000_000.0,
            exchangeable: spark > 0,
        };
        gacha.featured_pickups.push(pickup.clone());
        if rarity == 3 {
            gacha.pickups.push(pickup);
        }
    }

    drop(statement);
    let mut rarity_statement = connection.prepare(
        r#"
        SELECT gacha_id, rarity, SUM(odds)
        FROM gacha_available
        GROUP BY gacha_id, rarity
        ORDER BY gacha_id, rarity DESC
        "#,
    )?;
    let rarity_rows = rarity_statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rarity_rows {
        let (gacha_id, rarity, odds) = row?;
        let Some(gacha) = gachas.get_mut(&gacha_id) else {
            continue;
        };
        gacha.rarity_rates.push(PlannerRarityRate {
            rarity,
            rate: odds as f64 / 1_000_000.0,
        });
    }
    discard_mismatched_master_gachas(timeline_links, &mut gachas);
    seed_timeline_gacha_fallbacks(timeline_links, &mut gachas);
    Ok(gachas)
}

fn load_gachas_with_jp(
    connection: &Connection,
    jp_connection: Option<&Connection>,
    timeline_links: &BTreeMap<i64, TimelineLink>,
) -> Result<BTreeMap<i64, GachaAccumulator>> {
    let mut gachas = load_gachas(connection, timeline_links)?;
    let Some(jp_connection) = jp_connection else {
        return Ok(gachas);
    };
    for (gacha_id, mut jp_gacha) in load_gachas(jp_connection, timeline_links)? {
        if jp_gacha.provenance != Some("global_master") {
            continue;
        }
        jp_gacha.provenance = Some("jp_master");
        let keep_global = gachas.get(&gacha_id).is_some_and(|gacha| {
            gacha.provenance == Some("global_master") && gacha.confidence == Some("exact")
        });
        if !keep_global {
            gachas.insert(gacha_id, jp_gacha);
        }
    }
    Ok(gachas)
}

fn seed_timeline_gacha_fallbacks(
    timeline_links: &BTreeMap<i64, TimelineLink>,
    gachas: &mut BTreeMap<i64, GachaAccumulator>,
) {
    for (gacha_id, link) in timeline_links {
        if gachas.contains_key(gacha_id) || link.is_paid || !matches!(link.gacha_type, 3 | 11 | 12)
        {
            continue;
        }
        let card_type = match link.banner_kind.to_ascii_lowercase().as_str() {
            "character" => 1,
            "support" => 2,
            _ => continue,
        };
        let pickup_ids = timeline_top_rarity_pickup_ids(link);
        let infer_standard_rates =
            link.gacha_type == 3 && *gacha_id < 50_000 && matches!(pickup_ids.len(), 1 | 2);
        let pickups = if infer_standard_rates {
            pickup_ids
                .into_iter()
                .map(|pickup_id| PlannerPickup {
                    pickup_id,
                    label: if card_type == 1 {
                        format!("Umamusume {pickup_id}")
                    } else {
                        format!("Support Card {pickup_id}")
                    },
                    rate: STANDARD_PICKUP_RATE,
                    exchangeable: true,
                })
                .collect()
        } else {
            Vec::new()
        };
        let rarity_rates = infer_standard_rates
            .then(|| {
                STANDARD_RARITY_RATES
                    .into_iter()
                    .map(|(rarity, rate)| PlannerRarityRate { rarity, rate })
                    .collect()
            })
            .unwrap_or_default();
        gachas.insert(
            *gacha_id,
            GachaAccumulator {
                card_type,
                gacha_type: link.gacha_type,
                cost: DEFAULT_JEWEL_COST_PER_PULL,
                spark_pulls: DEFAULT_SPARK_PULLS,
                provenance: Some("jp_fallback"),
                confidence: Some(if infer_standard_rates {
                    "inferred_standard_rate"
                } else {
                    "timeline_schedule_defaults"
                }),
                pickups,
                rarity_rates,
                ..GachaAccumulator::default()
            },
        );
    }
}

fn apply_free_pulls(
    connection: &Connection,
    gachas: &mut BTreeMap<i64, GachaAccumulator>,
) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT gacha_id, target_draw_type, start_date, end_date FROM gacha_free_campaign",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    for row in rows {
        let (gacha_id, draws_per_day, start, end) = row?;
        let Some(gacha) = gachas.get_mut(&gacha_id) else {
            continue;
        };
        let days = ((end - start).max(0) + 86_399) / 86_400;
        gacha.free_pulls += days.max(1) * draws_per_day.max(0);
        gacha.free_pulls_provenance = Some("global_master");
        gacha.free_pulls_confidence = Some("exact");
    }
    Ok(())
}

fn load_archive() -> Result<Archive> {
    serde_json::from_slice(UMAPYOI_ARCHIVE).context("failed to parse Umapyoi archive")
}

fn load_global_news_archive() -> Result<GlobalNewsArchive> {
    serde_json::from_slice(GLOBAL_NEWS_ARCHIVE)
        .context("failed to parse official Global news archive")
}

fn load_global_social_archive() -> Result<GlobalSocialArchive> {
    serde_json::from_slice(GLOBAL_SOCIAL_ARCHIVE)
        .context("failed to parse official Global social archive")
}

fn global_archive_as_of(
    news: &GlobalNewsArchive,
    social: &GlobalSocialArchive,
) -> Option<NaiveDate> {
    news.posts
        .iter()
        .map(|post| post.first_seen_at.as_str())
        .chain(social.posts.iter().map(|post| post.first_seen_at.as_str()))
        .filter_map(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.date_naive())
        .max()
}

fn apply_news_free_pulls(
    archive: &Archive,
    timeline_links: &BTreeMap<i64, TimelineLink>,
    gachas: &mut BTreeMap<i64, GachaAccumulator>,
) -> Vec<PlannerFreePullCampaign> {
    let mut candidates = BTreeMap::<NewsFreePullCampaignKey, NewsFreePullCampaignCandidate>::new();
    for post in &archive.news {
        let combined = archive_combined_text(post);
        let target_windows = extract_news_free_pull_target_windows(&combined);
        let pinned_banner_kinds =
            extract_news_free_pull_pinned_banner_kinds(&combined, &target_windows);
        let campaigns = merge_news_free_pull_claims(extract_news_free_pull_claims(&combined));
        if campaigns.is_empty() {
            continue;
        }
        let Some(posted_at) = post
            .posted_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        else {
            continue;
        };

        for campaign in campaigns {
            // Without a named target gacha kind, banner schedule inference is
            // too broad to publish as planner metadata.
            if campaign.banner_kinds.is_empty() {
                continue;
            }
            let campaign_start = campaign
                .campaign_start
                .and_then(|parts| resolve_jp_campaign_start(posted_at.timestamp(), parts))
                .unwrap_or_else(|| posted_at.timestamp());
            let Some(assignments) = partition_news_free_pull_campaign_days_with_targets(
                campaign_start,
                &campaign,
                &target_windows,
                &pinned_banner_kinds,
                timeline_links,
                gachas,
            ) else {
                // A stated campaign duration must not be published when its
                // named JP target gacha cannot be resolved unambiguously.
                continue;
            };
            let key = NewsFreePullCampaignKey {
                campaign_jp_day: (campaign_start + 9 * 60 * 60).div_euclid(86_400),
                banner_kinds: campaign.banner_kinds.iter().copied().collect(),
                pulls_per_day: campaign.pulls_per_day,
                entitlement_days: campaign.entitlement_days,
                total_pulls: campaign.total_pulls,
            };
            let source_specificity = free_pull_source_specificity(post, &combined, &campaign);
            let allocation_mode = if free_pull_news_has_stock(&combined) {
                "daily_with_one_time_stock"
            } else {
                "fixed_daily_schedule"
            };
            let replace = candidates.get(&key).is_none_or(|current| {
                source_specificity > current.source_specificity
                    || (source_specificity == current.source_specificity
                        && post.post_id < current.post_id)
            });
            if replace {
                candidates.insert(
                    key,
                    NewsFreePullCampaignCandidate {
                        post_id: post.post_id,
                        label: post
                            .title
                            .clone()
                            .unwrap_or_else(|| "Free pull campaign".to_string()),
                        source_url: post.page_url.clone(),
                        campaign,
                        assignments,
                        allocation_mode,
                        source_specificity,
                    },
                );
            }
        }
    }

    // Preview and summary posts can describe the same campaign with an
    // ambiguous "10 free gachas" phrase, producing both 10x1 and 1x10
    // interpretations. For one JP day and target scope, keep only the unique
    // strongest source. A dedicated campaign notice outranks its preview.
    let mut preferred_scopes =
        BTreeMap::<(i64, Vec<&'static str>), (((bool, bool, bool, usize), i64, i64), usize)>::new();
    for (key, candidate) in &candidates {
        let scope = (key.campaign_jp_day, key.banner_kinds.clone());
        let rank = (
            candidate.source_specificity,
            candidate.campaign.total_pulls,
            -candidate.post_id,
        );
        preferred_scopes
            .entry(scope)
            .and_modify(|(current_rank, count)| match rank.cmp(current_rank) {
                std::cmp::Ordering::Greater => {
                    *current_rank = rank;
                    *count = 1;
                }
                std::cmp::Ordering::Equal => *count += 1,
                std::cmp::Ordering::Less => {}
            })
            .or_insert((rank, 1));
    }
    candidates.retain(|key, candidate| {
        let scope = (key.campaign_jp_day, key.banner_kinds.clone());
        let rank = (
            candidate.source_specificity,
            candidate.campaign.total_pulls,
            -candidate.post_id,
        );
        preferred_scopes
            .get(&scope)
            .is_some_and(|(preferred, count)| *count == 1 && rank == *preferred)
    });

    let mut result = Vec::new();
    for (key, candidate) in candidates {
        // Exact master data may already populate a banner. Retain it when it
        // agrees with the news schedule, but refuse to publish a pool whose
        // compatibility allocations conflict with the existing source.
        if candidate.assignments.iter().any(|(gacha_id, pulls)| {
            gachas
                .get(gacha_id)
                .is_some_and(|gacha| gacha.free_pulls > 0 && gacha.free_pulls != *pulls)
        }) {
            continue;
        }
        let campaign_id = stable_free_pull_campaign_id(&key);
        for (gacha_id, pulls) in &candidate.assignments {
            let Some(gacha) = gachas.get_mut(gacha_id) else {
                continue;
            };
            gacha.campaign_id = Some(campaign_id.clone());
            if gacha.free_pulls == 0 {
                gacha.free_pulls = *pulls;
                gacha.free_pulls_provenance = Some("jp_news");
                gacha.free_pulls_confidence = Some("schedule_partitioned");
            }
            if gacha.free_pulls_source_url.is_none() {
                gacha.free_pulls_source_url = Some(candidate.source_url.clone());
            }
        }
        let mut default_allocations = candidate
            .assignments
            .iter()
            .filter_map(|(gacha_id, pulls)| {
                timeline_links
                    .get(gacha_id)
                    .map(|link| PlannerFreePullAllocation {
                        event_id: link.event_id.clone(),
                        gacha_id: *gacha_id,
                        pulls: *pulls,
                    })
            })
            .collect::<Vec<_>>();
        // The frontend deliberately permits stock allocation only to the last
        // default allocation. Keep that contract semantic by ordering by the
        // real JP banner start, never by gacha ID.
        default_allocations.sort_by(|left, right| {
            let start = |gacha_id: i64| {
                timeline_links
                    .get(&gacha_id)
                    .and_then(|link| DateTime::parse_from_rfc3339(&link.jp_start_date).ok())
                    .map(|date| date.timestamp())
            };
            start(left.gacha_id)
                .cmp(&start(right.gacha_id))
                .then_with(|| left.gacha_id.cmp(&right.gacha_id))
        });
        result.push(PlannerFreePullCampaign {
            id: campaign_id,
            label: candidate.label,
            total_pulls: candidate.campaign.total_pulls,
            pulls_per_day: candidate.campaign.pulls_per_day,
            entitlement_days: candidate.campaign.entitlement_days,
            allocation_mode: candidate.allocation_mode,
            eligible_gacha_ids: default_allocations
                .iter()
                .map(|allocation| allocation.gacha_id)
                .collect(),
            default_allocations,
            source_url: candidate.source_url,
            provenance: "jp_news",
            // The notice proves the JP campaign total. Applying its banner
            // schedule to the predicted Global timeline remains a prediction.
            confidence: "jp_schedule_prediction",
        });
    }
    result.sort_by(|left, right| left.id.cmp(&right.id));
    result
}

fn stable_free_pull_campaign_id(key: &NewsFreePullCampaignKey) -> String {
    let campaign_date = Utc
        .timestamp_opt(key.campaign_jp_day.saturating_mul(86_400), 0)
        .single()
        .map(|date| date.format("%Y%m%d").to_string())
        .unwrap_or_else(|| key.campaign_jp_day.to_string());
    let banner_kinds = key.banner_kinds.join("-");
    format!(
        "jp-free-pulls-{campaign_date}-{banner_kinds}-{}x{}",
        key.pulls_per_day, key.entitlement_days
    )
}

fn partition_news_free_pull_days(
    campaign_start: i64,
    claim: &NewsFreePullClaim,
    timeline_links: &BTreeMap<i64, TimelineLink>,
    gachas: &BTreeMap<i64, GachaAccumulator>,
) -> Option<BTreeMap<i64, i64>> {
    let campaign = merge_news_free_pull_claims(vec![claim.clone()])
        .into_iter()
        .next()?;
    partition_news_free_pull_campaign_days(campaign_start, &campaign, timeline_links, gachas)
}

fn partition_news_free_pull_campaign_days(
    campaign_start: i64,
    campaign: &NewsFreePullCampaignClaim,
    timeline_links: &BTreeMap<i64, TimelineLink>,
    gachas: &BTreeMap<i64, GachaAccumulator>,
) -> Option<BTreeMap<i64, i64>> {
    partition_news_free_pull_campaign_days_with_targets(
        campaign_start,
        campaign,
        &[],
        &BTreeSet::new(),
        timeline_links,
        gachas,
    )
}

fn partition_news_free_pull_campaign_days_with_targets(
    campaign_start: i64,
    campaign: &NewsFreePullCampaignClaim,
    target_windows: &[NewsFreePullTargetWindow],
    pinned_banner_kinds: &BTreeSet<&'static str>,
    timeline_links: &BTreeMap<i64, TimelineLink>,
    gachas: &BTreeMap<i64, GachaAccumulator>,
) -> Option<BTreeMap<i64, i64>> {
    let entitlement_times = free_pull_entitlement_times(campaign_start, campaign.entitlement_days)?;
    let raw_resolved_windows = target_windows
        .iter()
        .filter(|window| campaign.banner_kinds.contains(window.banner_kind))
        .filter_map(|window| {
            let start = resolve_jp_campaign_start(campaign_start, window.start)?;
            let end = resolve_jp_campaign_start(campaign_start, window.end)?;
            Some((window.banner_kind, window.pin_to_window_start, start, end))
        })
        .collect::<Vec<_>>();
    let resolved_windows = raw_resolved_windows
        .iter()
        .filter_map(|(kind, pin_to_window_start, stated_start, end)| {
            let start = if stated_start <= end {
                *stated_start
            } else {
                // The archived English translation of post 1124 says 1/29
                // where the JP notice says 1/2. Recover a malformed later
                // window from the previous target's adjacent end boundary.
                raw_resolved_windows
                    .iter()
                    .filter_map(|(_, _, _, other_end)| (*other_end < *end).then_some(*other_end))
                    .max()?
                    .checked_add(60)?
            };
            (end >= &start && *end - start <= 60 * 86_400).then_some((
                *kind,
                *pin_to_window_start,
                start,
                *end,
            ))
        })
        .collect::<BTreeSet<_>>();
    let resolved_banner_kinds = resolved_windows
        .iter()
        .map(|(kind, _, _, _)| *kind)
        .collect::<BTreeSet<_>>();
    if !resolved_windows.is_empty() && resolved_banner_kinds != campaign.banner_kinds {
        return None;
    }

    // Detailed older notices explicitly list every target gacha window. Map
    // each entitlement through those windows, using the window start to pick
    // the banner. This preserves real mid-campaign switches, including cases
    // where character and support gachas share one master-data start time.
    if !resolved_windows.is_empty() {
        let mut assignments = BTreeMap::new();
        for entitlement_at in entitlement_times {
            let matching_gachas = resolved_windows
                .iter()
                .filter(|(_, _, start, end)| entitlement_at >= *start && entitlement_at <= *end)
                .filter_map(|(kind, pin_to_window_start, start, _)| {
                    if *pin_to_window_start {
                        free_pull_target_gacha(*kind, *start, timeline_links, gachas)
                    } else {
                        free_pull_schedule_gacha_at(*kind, entitlement_at, timeline_links, gachas)
                    }
                })
                .collect::<BTreeSet<_>>();
            if matching_gachas.len() != 1 {
                return None;
            }
            let gacha_id = matching_gachas.into_iter().next()?;
            *assignments.entry(gacha_id).or_insert(0) += campaign.pulls_per_day;
        }
        return (assignments.values().sum::<i64>() == campaign.total_pulls).then_some(assignments);
    }

    if campaign.banner_kinds.len() != 1 {
        return None;
    }
    let banner_kind = *campaign.banner_kinds.iter().next()?;
    if pinned_banner_kinds.contains(banner_kind) {
        // Newer notices name one specific target gacha for the complete
        // campaign. Later overlapping banners are separate products; their
        // start dates do not transfer the remaining free pulls.
        let gacha_id = free_pull_target_gacha(banner_kind, campaign_start, timeline_links, gachas)?;
        return Some(BTreeMap::from([(gacha_id, campaign.total_pulls)]));
    }

    // Generic targets such as "Pickup Pretty Derby Gacha" follow the active
    // standard banner when that banner is actually replaced mid-campaign.
    let mut assignments = BTreeMap::new();
    for entitlement_at in entitlement_times {
        let gacha_id =
            free_pull_schedule_gacha_at(banner_kind, entitlement_at, timeline_links, gachas)?;
        *assignments.entry(gacha_id).or_insert(0) += campaign.pulls_per_day;
    }
    (assignments.values().sum::<i64>() == campaign.total_pulls).then_some(assignments)
}

fn free_pull_entitlement_times(campaign_start: i64, days: i64) -> Option<Vec<i64>> {
    let mut entitlements = Vec::with_capacity(usize::try_from(days).ok()?);
    let campaign_start_jp = campaign_start + 9 * 60 * 60;
    let current_jp_day = campaign_start_jp.div_euclid(86_400) * 86_400;
    let mut next_reset_jp = current_jp_day + 5 * 60 * 60;
    if next_reset_jp <= campaign_start_jp {
        next_reset_jp += 86_400;
    }
    for day in 0..days {
        let entitlement_at = if day == 0 {
            campaign_start
        } else {
            next_reset_jp
                .checked_add((day - 1).checked_mul(86_400)?)?
                .checked_sub(9 * 60 * 60)?
        };
        entitlements.push(entitlement_at);
    }
    Some(entitlements)
}

fn free_pull_target_gacha(
    banner_kind: &str,
    target_start: i64,
    timeline_links: &BTreeMap<i64, TimelineLink>,
    gachas: &BTreeMap<i64, GachaAccumulator>,
) -> Option<i64> {
    let mut candidates = timeline_links
        .iter()
        .filter_map(|(gacha_id, link)| {
            if link.is_paid
                || !is_free_pull_schedule_gacha(*gacha_id, link, gachas)
                || !link.banner_kind.to_ascii_lowercase().contains(banner_kind)
            {
                return None;
            }
            let start = DateTime::parse_from_rfc3339(&link.jp_start_date)
                .ok()?
                .timestamp();
            (start <= target_start && target_start - start <= 21 * 86_400)
                .then_some((start, *gacha_id))
        })
        .collect::<Vec<_>>();
    candidates
        .sort_unstable_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    let (latest_start, gacha_id) = candidates.first().copied()?;
    if candidates
        .get(1)
        .is_some_and(|candidate| candidate.0 == latest_start)
    {
        return None;
    }
    Some(gacha_id)
}

fn free_pull_schedule_gacha_at(
    banner_kind: &str,
    entitlement_at: i64,
    timeline_links: &BTreeMap<i64, TimelineLink>,
    gachas: &BTreeMap<i64, GachaAccumulator>,
) -> Option<i64> {
    let mut candidates = timeline_links
        .iter()
        .filter_map(|(gacha_id, link)| {
            if !is_free_pull_schedule_gacha(*gacha_id, link, gachas)
                || !link.banner_kind.to_ascii_lowercase().contains(banner_kind)
            {
                return None;
            }
            let start = DateTime::parse_from_rfc3339(&link.jp_start_date)
                .ok()?
                .timestamp();
            (start <= entitlement_at && entitlement_at - start < 21 * 86_400)
                .then_some((start, *gacha_id))
        })
        .collect::<Vec<_>>();
    candidates
        .sort_unstable_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    let (latest_start, gacha_id) = candidates.first().copied()?;
    if candidates
        .get(1)
        .is_some_and(|candidate| candidate.0 == latest_start)
        || timeline_links
            .get(&gacha_id)
            .is_none_or(|link| link.is_paid)
    {
        return None;
    }
    Some(gacha_id)
}

fn is_free_pull_schedule_gacha(
    gacha_id: i64,
    link: &TimelineLink,
    gachas: &BTreeMap<i64, GachaAccumulator>,
) -> bool {
    if link.is_paid {
        // Normal paid-only types (for example guaranteed gacha type 5) run
        // alongside standard banners and are irrelevant to free-pull routing.
        // Only an eligible-looking paid/news gacha can steal a schedule day,
        // so retain those as invalidating boundaries.
        return matches!(link.gacha_type, 3 | 11 | 12);
    }
    matches!(link.gacha_type, 3 | 11 | 12)
        && gachas
            .get(&gacha_id)
            .is_some_and(|gacha| matches!(gacha.gacha_type, 3 | 11 | 12))
}

fn merge_news_free_pull_claims(claims: Vec<NewsFreePullClaim>) -> Vec<NewsFreePullCampaignClaim> {
    let mut campaigns =
        BTreeMap::<(i64, i64, i64, Option<(u32, u32, u32, u32)>), NewsFreePullCampaignClaim>::new();
    for claim in claims {
        // A banner's own duration is not evidence for a free-pull campaign
        // duration. Preview posts without a day count or total stay qualitative.
        let Some(entitlement_days) = claim.days.map(|days| days.clamp(1, 60)) else {
            continue;
        };
        let Some(total_pulls) = entitlement_days.checked_mul(claim.pulls_per_day) else {
            continue;
        };
        if total_pulls <= 0
            || total_pulls > 300
            || claim
                .explicit_total
                .is_some_and(|explicit| explicit != total_pulls)
        {
            continue;
        }
        let key = (
            claim.pulls_per_day,
            entitlement_days,
            total_pulls,
            claim.campaign_start,
        );
        let campaign = campaigns
            .entry(key)
            .or_insert_with(|| NewsFreePullCampaignClaim {
                banner_kinds: BTreeSet::new(),
                pulls_per_day: claim.pulls_per_day,
                entitlement_days,
                total_pulls,
                campaign_start: claim.campaign_start,
                has_explicit_total: false,
            });
        if let Some(kind) = claim.banner_kind {
            campaign.banner_kinds.insert(kind);
        }
        campaign.has_explicit_total |= claim.explicit_total.is_some();
    }
    campaigns.into_values().collect()
}

fn free_pull_news_has_stock(value: &str) -> bool {
    value.to_ascii_lowercase().contains("gacha stock") || value.contains("ガチャストック")
}

fn free_pull_source_specificity(
    post: &ArchiveNews,
    combined: &str,
    campaign: &NewsFreePullCampaignClaim,
) -> (bool, bool, bool, usize) {
    let title = post
        .title
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let direct_campaign_title = title.contains("free")
        && (title.contains("gacha") || title.contains("pull") || title.contains("draw"));
    (
        direct_campaign_title,
        campaign.has_explicit_total,
        free_pull_news_has_stock(combined),
        combined.len(),
    )
}

fn resolve_jp_campaign_start(
    posted_at: i64,
    (month, day, hour, minute): (u32, u32, u32, u32),
) -> Option<i64> {
    let posted_year = Utc
        .timestamp_opt(posted_at, 0)
        .single()?
        .format("%Y")
        .to_string();
    let posted_year = posted_year.parse::<i32>().ok()?;
    [posted_year - 1, posted_year, posted_year + 1]
        .into_iter()
        .filter_map(|year| {
            Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
                .single()
                .map(|date| date.timestamp() - 9 * 60 * 60)
        })
        .min_by_key(|candidate| (*candidate - posted_at).abs())
        .filter(|candidate| (*candidate - posted_at).abs() <= 60 * 86_400)
}

fn extract_free_pull_total(value: &str) -> Option<i64> {
    let lower = value.to_ascii_lowercase();
    if !(lower.contains("free gacha")
        || lower.contains("free pull")
        || lower.contains("free draw")
        || value.contains("無料ガチャ")
        || value.contains("回分無料"))
    {
        return None;
    }
    value
        .lines()
        .flat_map(|line| amounts_before_word(line, "free"))
        .chain(amounts_before_word(value, "回分無料"))
        .filter(|amount| (10..=300).contains(amount))
        .max()
}

fn extract_news_free_pull_claims(value: &str) -> Vec<NewsFreePullClaim> {
    let lower = value.to_ascii_lowercase();
    if !(lower.contains("free gacha")
        || lower.contains("free pull")
        || lower.contains("free draw")
        || value.contains("無料キャンペーン")
        || value.contains("無料ガチャ")
        || value.contains("無料で引け"))
    {
        return Vec::new();
    }
    let lines = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let anchor_indices = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| is_free_pull_anchor(line).then_some(index))
        .collect::<Vec<_>>();
    let mut claims = Vec::new();
    let windows = if anchor_indices.is_empty() {
        vec![value.to_string()]
    } else {
        anchor_indices
            .iter()
            .map(|index| {
                let start = index.saturating_sub(1);
                let end = (index + 12).min(lines.len());
                lines[start..end].join("\n")
            })
            .collect::<Vec<_>>()
    };
    for context in windows {
        let context_lower = context.to_ascii_lowercase();
        let pulls_per_day = if context_lower.contains("10-pull")
            || context_lower.contains("10 pull")
            || context_lower.contains("10-round")
            || context_lower.contains("10 round")
            || context_lower.contains("10 free draw")
            || context.contains("10連ガチャ")
            || context.contains("[10回引く!]")
            || context.contains("［10回引く！］")
        {
            10
        } else {
            1
        };
        let explicit_total = extract_free_pull_total(&context);
        let stated_days = context
            .lines()
            .filter(|line| {
                line.contains(':')
                    && (line.contains('-') || line.contains('～') || line.contains('~'))
            })
            .flat_map(|line| {
                amounts_before_word(line, "日分")
                    .into_iter()
                    .chain(amounts_before_word(line, "days"))
            })
            .filter(|days| (1..=60).contains(days))
            .max();
        let inferred_days = explicit_total
            .filter(|total| total % pulls_per_day == 0)
            .map(|total| total / pulls_per_day)
            .filter(|days| (1..=60).contains(days));
        let days = stated_days.or(inferred_days);
        let campaign_start = extract_month_day_time(&context);
        let kinds = [
            (
                "support",
                context_lower.contains("support card gacha")
                    || context.contains("サポートカードガチャ"),
            ),
            (
                "character",
                context_lower.contains("pretty derby gacha")
                    || context_lower.contains("training umamusume gacha")
                    || context.contains("プリティーダービーガチャ")
                    || context.contains("育成ウマ娘ガチャ"),
            ),
        ];
        let mut emitted_kind = false;
        for (kind, present) in kinds {
            if present {
                emitted_kind = true;
                claims.push(NewsFreePullClaim {
                    banner_kind: Some(kind),
                    pulls_per_day,
                    days,
                    explicit_total,
                    campaign_start,
                    evidence: context.chars().take(240).collect(),
                });
            }
        }
        if !emitted_kind && (days.is_some() || explicit_total.is_some()) {
            claims.push(NewsFreePullClaim {
                banner_kind: None,
                pulls_per_day,
                days,
                explicit_total,
                campaign_start,
                evidence: context.chars().take(240).collect(),
            });
        }
    }
    let has_specific = claims
        .iter()
        .any(|claim| claim.banner_kind.is_some() && claim.campaign_start.is_some());
    if has_specific {
        claims.retain(|claim| claim.banner_kind.is_some() && claim.campaign_start.is_some());
    }
    let scheduled_claims = claims
        .iter()
        .filter(|claim| claim.days.is_some())
        .map(|claim| (claim.banner_kind, claim.pulls_per_day, claim.campaign_start))
        .collect::<BTreeSet<_>>();
    claims.retain(|claim| {
        claim.days.is_some()
            || !scheduled_claims.contains(&(
                claim.banner_kind,
                claim.pulls_per_day,
                claim.campaign_start,
            ))
    });
    claims.sort_by_key(|claim| {
        (
            claim.banner_kind,
            claim.pulls_per_day,
            claim.days,
            claim.explicit_total,
            claim.campaign_start,
        )
    });
    claims.dedup_by(|left, right| {
        left.banner_kind == right.banner_kind
            && left.pulls_per_day == right.pulls_per_day
            && left.days == right.days
            && left.explicit_total == right.explicit_total
            && left.campaign_start == right.campaign_start
    });
    // Dedicated campaign posts repeat one entitlement in stock examples and
    // caution notes. Smaller counts in those fragments are explanatory, not
    // additional campaigns. Prefer the substantial explicit total for each
    // banner kind when the post provides one.
    let dominant_totals: BTreeMap<Option<&'static str>, i64> = claims
        .iter()
        .filter_map(|claim| claim.explicit_total.map(|total| (claim.banner_kind, total)))
        .fold(BTreeMap::new(), |mut totals, (kind, total)| {
            totals
                .entry(kind)
                .and_modify(|current| *current = (*current).max(total))
                .or_insert(total);
            totals
        });
    claims.retain(|claim| {
        dominant_totals
            .get(&claim.banner_kind)
            .is_none_or(|total| *total < 50 || claim.explicit_total == Some(*total))
    });
    claims
}

fn is_free_pull_anchor(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    ((lower.contains("free")
        && (lower.contains("gacha") || lower.contains("pull") || lower.contains("draw")))
        || (line.contains("無料") && line.contains("ガチャ")))
        && !has_sales_context(&lower)
}

fn extract_news_free_pull_target_windows(value: &str) -> Vec<NewsFreePullTargetWindow> {
    let lines = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    lines
        .windows(2)
        .filter_map(|pair| {
            let (banner_kind, pin_to_window_start) = free_pull_target_banner_kind(pair[0])?;
            if pair[1].chars().count() > 100 {
                return None;
            }
            let range = extract_numeric_month_day_times(pair[1]);
            (range.len() >= 2).then(|| NewsFreePullTargetWindow {
                banner_kind,
                pin_to_window_start,
                start: range[0],
                end: range[1],
            })
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn extract_news_free_pull_pinned_banner_kinds(
    value: &str,
    target_windows: &[NewsFreePullTargetWindow],
) -> BTreeSet<&'static str> {
    let mut result = target_windows
        .iter()
        .filter_map(|window| window.pin_to_window_start.then_some(window.banner_kind))
        .collect::<BTreeSet<_>>();
    let lines = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    for pair in lines.windows(2) {
        let heading = pair[0].to_ascii_lowercase();
        if !(heading.contains("target gacha") || heading.contains("eligible gacha")) {
            continue;
        }
        if let Some((kind, true)) = free_pull_target_banner_kind(pair[1]) {
            result.insert(kind);
        }
    }
    for line in lines {
        if line.chars().count() <= 120 {
            if let Some((kind, true)) = free_pull_target_banner_kind(line) {
                result.insert(kind);
            }
        }
    }
    result
}

fn free_pull_target_banner_kind(value: &str) -> Option<(&'static str, bool)> {
    let lower = value.to_ascii_lowercase();
    if lower.contains("example")
        || lower.contains("percentage")
        || lower.contains("offer ratio")
        || lower.contains("please check")
    {
        return None;
    }
    let support =
        lower.contains("support card gacha") || value.contains("ã‚µãƒãƒ¼ãƒˆã‚«ãƒ¼ãƒ‰ã‚¬ãƒãƒ£");
    let character = lower.contains("pretty derby gacha")
        || lower.contains("training umamusume gacha")
        || value.contains("ãƒ—ãƒªãƒ†ã‚£ãƒ¼ãƒ€ãƒ¼ãƒ“ãƒ¼ã‚¬ãƒãƒ£")
        || value.contains("è‚²æˆã‚¦ãƒžå¨˜ã‚¬ãƒãƒ£");
    let pins_one_banner =
        lower.contains("anniv") || lower.contains("scenario") || lower.contains("commemorat");
    match (support, character) {
        (true, false) => Some(("support", pins_one_banner)),
        (false, true) => Some(("character", pins_one_banner)),
        _ => None,
    }
}

fn extract_month_day_time(value: &str) -> Option<(u32, u32, u32, u32)> {
    extract_numeric_month_day_times(value)
        .into_iter()
        .next()
        .or_else(|| extract_english_month_day_time(value))
}

fn extract_numeric_month_day_times(value: &str) -> Vec<(u32, u32, u32, u32)> {
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    tokens
        .windows(2)
        .filter_map(|pair| {
            let date = pair[0]
                .trim_matches(|character: char| !character.is_ascii_digit() && character != '/');
            let time = pair[1]
                .trim_matches(|character: char| !character.is_ascii_digit() && character != ':');
            let (month, day) = date.split_once('/')?;
            let (hour, minute) = time.split_once(':')?;
            let (Ok(month), Ok(day), Ok(hour), Ok(minute)) = (
                month.parse::<u32>(),
                day.parse::<u32>(),
                hour.parse::<u32>(),
                minute.parse::<u32>(),
            ) else {
                return None;
            };
            let parsed = (month, day, hour, minute);
            if (1..=12).contains(&parsed.0)
                && (1..=31).contains(&parsed.1)
                && parsed.2 <= 23
                && parsed.3 <= 59
            {
                Some(parsed)
            } else {
                None
            }
        })
        .collect()
}

fn extract_english_month_day_time(value: &str) -> Option<(u32, u32, u32, u32)> {
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    for (index, token) in tokens.iter().enumerate() {
        let month = match token
            .trim_matches(|character: char| !character.is_ascii_alphabetic())
            .to_ascii_lowercase()
            .as_str()
        {
            "january" | "jan" => 1,
            "february" | "feb" => 2,
            "march" | "mar" => 3,
            "april" | "apr" => 4,
            "may" => 5,
            "june" | "jun" => 6,
            "july" | "jul" => 7,
            "august" | "aug" => 8,
            "september" | "sep" | "sept" => 9,
            "october" | "oct" => 10,
            "november" | "nov" => 11,
            "december" | "dec" => 12,
            _ => continue,
        };
        let day = tokens
            .get(index + 1)?
            .trim_matches(|character: char| !character.is_ascii_digit())
            .parse::<u32>()
            .ok()?;
        if !(1..=31).contains(&day) {
            continue;
        }
        let nearby_time = tokens[index.saturating_sub(3)..index]
            .iter()
            .rev()
            .find_map(|token| parse_clock_time(token))
            .or_else(|| {
                tokens[index + 2..tokens.len().min(index + 9)]
                    .iter()
                    .find_map(|token| parse_clock_time(token))
            });
        if let Some((hour, minute)) = nearby_time {
            return Some((month, day, hour, minute));
        }
    }
    None
}

fn parse_clock_time(value: &str) -> Option<(u32, u32)> {
    let token =
        value.trim_matches(|character: char| !character.is_ascii_digit() && character != ':');
    let (hour, minute) = token.split_once(':')?;
    let (hour, minute) = (hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?);
    (hour <= 23 && minute <= 59).then_some((hour, minute))
}

fn archive_combined_text(post: &ArchiveNews) -> String {
    let japanese = post
        .raw
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let english = post
        .raw
        .get("message_english")
        .and_then(Value::as_str)
        .unwrap_or_default();
    html_to_text(&format!(
        "{}\n{}\n{}",
        post.title.as_deref().unwrap_or_default(),
        japanese,
        english
    ))
}

fn archive_message(post: &ArchiveNews) -> &str {
    post.raw
        .get("message_english")
        .and_then(Value::as_str)
        .or_else(|| post.raw.get("message").and_then(Value::as_str))
        .unwrap_or_default()
}

fn load_income_rules(connection: &Connection) -> Result<Vec<PlannerIncomeRule>> {
    let mut rules = Vec::new();

    let daily: Option<(String, String, i64)> = connection
        .query_row(
            r#"
            SELECT start_date, MAX(end_date), SUM(item_num)
            FROM mission_data
            WHERE mission_type = 1
              AND item_category = ?1
              AND item_id = ?2
              AND start_date = (
                  SELECT MAX(start_date)
                  FROM mission_data
                  WHERE mission_type = 1
                    AND item_category = ?1
                    AND item_id = ?2
              )
            GROUP BY start_date
            "#,
            [JEWEL_CATEGORY, JEWEL_ITEM_ID],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();
    if let Some((start, end, amount)) = daily {
        rules.push(income_rule(
            "daily-missions",
            "Daily missions",
            amount,
            "daily",
            &start,
            Some(&end),
            true,
        )?);
    }

    let regular_login: Option<(String, String, i64)> = connection
        .query_row(
            r#"
            SELECT bonus.start_date, bonus.end_date, SUM(detail.item_num)
            FROM login_bonus_data AS bonus
            JOIN login_bonus_detail AS detail ON detail.login_bonus_id = bonus.id
            WHERE bonus.type = 1
              AND detail.item_category = ?1
              AND detail.item_id = ?2
              AND bonus.start_date = (
                  SELECT MAX(start_date) FROM login_bonus_data WHERE type = 1
              )
            GROUP BY bonus.id, bonus.start_date, bonus.end_date
            "#,
            [JEWEL_CATEGORY, JEWEL_ITEM_ID],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();
    if let Some((start, end, amount)) = regular_login {
        rules.push(income_rule(
            "regular-login-cycle",
            "Regular 7-day login cycle",
            amount,
            "weekly",
            &start,
            Some(&end),
            true,
        )?);
    }

    let latest_stadium: Option<i64> = connection
        .query_row("SELECT MAX(id) FROM team_stadium", [], |row| row.get(0))
        .ok();
    if let Some(team_stadium_id) = latest_stadium {
        let mut statement = connection.prepare(
            r#"
            SELECT team_class, item_num_1
            FROM team_stadium_class_reward
            WHERE team_stadium_id = ?1
              AND class_reward_type = 3
              AND item_category_1 = ?2
              AND item_id_1 = ?3
            ORDER BY team_class
            "#,
        )?;
        let rows = statement
            .query_map([team_stadium_id, JEWEL_CATEGORY, JEWEL_ITEM_ID], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })?;
        for row in rows {
            let (class, amount) = row?;
            rules.push(PlannerIncomeRule {
                id: format!("team-trials-class-{class}"),
                label: format!("Team Trials Class {class} maintenance"),
                currency: "free_jewels",
                amount,
                cadence: "weekly",
                start_date: "2025-06-26T00:00:00Z".to_string(),
                day_of_month: None,
                end_date: None,
                default_enabled: false,
                provenance: "global_master",
                description: Some(
                    "Weekly maintenance reward for the selected Team Trials class".to_string(),
                ),
                category: Some("competitive"),
                scenario_group: Some("team_trials_class"),
                scenario_option: Some(format!("class_{class}")),
            });
        }
    }

    let mut statement = connection.prepare(
        r#"
        SELECT id, need_ranking_min, need_ranking_max, reward_num_1
        FROM circle_rank_data
        WHERE reward_item_category_1 = ?1
          AND reward_item_id_1 = ?2
          AND reward_num_1 > 0
        ORDER BY id
        "#,
    )?;
    let rows = statement.query_map([JEWEL_CATEGORY, JEWEL_ITEM_ID], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    for row in rows {
        let (rank_id, min_rank, max_rank, amount) = row?;
        let label = if min_rank <= 1 && max_rank > 0 {
            format!("Club rank 1-{max_rank}")
        } else if max_rank == 0 {
            format!("Club rank {min_rank}+")
        } else {
            format!("Club rank {min_rank}-{max_rank}")
        };
        rules.push(PlannerIncomeRule {
            id: format!("club-rank-{rank_id}"),
            label,
            currency: "free_jewels",
            amount,
            cadence: "monthly",
            start_date: "2025-06-26T00:00:00Z".to_string(),
            day_of_month: None,
            end_date: None,
            default_enabled: false,
            provenance: "global_master",
            description: Some("Monthly reward for the selected club ranking band".to_string()),
            category: Some("club"),
            scenario_group: Some("club_rank"),
            scenario_option: Some(format!("rank_{rank_id}")),
        });
    }

    load_daily_pack_rules(connection, &mut rules)?;
    load_monthly_ticket_exchange_rules(connection, &mut rules)?;

    Ok(rules)
}

fn load_monthly_ticket_exchange_rules(
    connection: &Connection,
    rules: &mut Vec<PlannerIncomeRule>,
) -> Result<()> {
    if !sqlite_table_exists(connection, "item_exchange")? {
        return Ok(());
    }
    let mut statement = connection.prepare(
        r#"
        SELECT item_exchange_top_id,
               change_item_id,
               SUM(change_item_num * change_item_limit_num),
               MIN(start_date)
        FROM item_exchange
        WHERE item_exchange_top_id IN (?1, ?2)
          AND change_item_limit_type = 4
          AND change_item_category = ?3
          AND change_item_id IN (?4, ?5)
          AND change_item_num > 0
          AND change_item_limit_num > 0
        GROUP BY item_exchange_top_id, change_item_id
        ORDER BY item_exchange_top_id, change_item_id
        "#,
    )?;
    let rows = statement.query_map(
        [
            FRIEND_POINT_EXCHANGE_ID,
            CLOVER_EXCHANGE_ID,
            GACHA_TICKET_CATEGORY,
            UMA_TICKET_ITEM_ID,
            SUPPORT_TICKET_ITEM_ID,
        ],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        },
    )?;
    for row in rows {
        let (exchange_id, item_id, amount, start) = row?;
        let (exchange_slug, exchange_label) = match exchange_id {
            FRIEND_POINT_EXCHANGE_ID => ("friend-points", "Friend Point Exchange"),
            CLOVER_EXCHANGE_ID => ("clovers", "Clover Exchange"),
            _ => continue,
        };
        let currency = match item_id {
            UMA_TICKET_ITEM_ID => "uma_ticket",
            SUPPORT_TICKET_ITEM_ID => "support_ticket",
            _ => continue,
        };
        rules.push(PlannerIncomeRule {
            id: format!("monthly-shop-{exchange_slug}-{currency}"),
            label: format!("{exchange_label} tickets"),
            currency,
            amount,
            cadence: "monthly",
            start_date: timestamp_to_rfc3339(start.max(GLOBAL_LAUNCH_TIMESTAMP)),
            day_of_month: Some(1),
            end_date: None,
            default_enabled: false,
            provenance: "global_master",
            description: Some(format!(
                "Monthly ticket stock from the {exchange_label}; requires enough exchange currency"
            )),
            category: Some("shop"),
            scenario_group: Some("monthly_shop_tickets"),
            scenario_option: Some("include".to_string()),
        });
    }
    Ok(())
}

fn load_daily_pack_rules(
    connection: &Connection,
    rules: &mut Vec<PlannerIncomeRule>,
) -> Result<()> {
    let mut statement = connection.prepare(
        r#"
        SELECT shop_data_id,
               term,
               daily_free_num,
               MIN(start_date),
               MAX(end_date)
        FROM daily_pack
        WHERE daily_free_num > 0
        GROUP BY shop_data_id, term, daily_free_num
        ORDER BY shop_data_id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (shop_data_id, term, amount, start, end) = row?;
        rules.push(PlannerIncomeRule {
            id: format!("daily-jewel-pack-{shop_data_id}"),
            label: "Daily Jewel Pack (continuous)".to_string(),
            currency: "free_jewels",
            amount,
            cadence: "daily",
            start_date: master_date_to_rfc3339(&start)?,
            day_of_month: None,
            end_date: Some(master_date_to_rfc3339(&end)?),
            default_enabled: false,
            provenance: "global_master",
            description: Some(format!(
                "{amount} free jewels per day while the renewable {term}-day pack is active"
            )),
            category: Some("paid"),
            scenario_group: None,
            scenario_option: None,
        });
    }
    Ok(())
}

fn load_paid_news_income_rules(archive: &Archive) -> Vec<PlannerIncomeRule> {
    let mut candidates = Vec::new();
    for post in &archive.news {
        let title = post.title.as_deref().unwrap_or_default();
        if !title.to_ascii_lowercase().contains("training pass") {
            continue;
        }
        let message = html_to_text(archive_message(post));
        let lower = message.to_ascii_lowercase();
        if !lower.contains("premium pass") {
            continue;
        }
        let amount = amounts_before_word(&message, "paid jewel")
            .into_iter()
            .chain(amounts_before_word(&message, "paid gem"))
            .filter(|amount| (1..=10_000).contains(amount))
            .max();
        if let (Some(amount), Some(posted_at)) = (amount, post.posted_at.as_deref()) {
            candidates.push((posted_at.to_string(), amount));
        }
    }
    let Some((latest_start, amount)) = candidates
        .into_iter()
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)))
    else {
        return Vec::new();
    };
    vec![PlannerIncomeRule {
        id: "premium-training-pass".to_string(),
        label: "Premium Training Pass (continuous)".to_string(),
        currency: "paid_jewels",
        amount,
        cadence: "monthly",
        start_date: latest_start,
        day_of_month: None,
        end_date: None,
        default_enabled: false,
        provenance: "jp_fallback",
        description: Some(format!(
            "Includes {amount} paid jewels with each Premium Pass purchase; exclusive track rewards are not counted"
        )),
        category: Some("paid"),
        scenario_group: None,
        scenario_option: None,
    }]
}

#[allow(clippy::too_many_arguments)]
fn income_rule(
    id: &str,
    label: &str,
    amount: i64,
    cadence: &'static str,
    start: &str,
    end: Option<&str>,
    default_enabled: bool,
) -> Result<PlannerIncomeRule> {
    Ok(PlannerIncomeRule {
        id: id.to_string(),
        label: label.to_string(),
        currency: "free_jewels",
        amount,
        cadence,
        start_date: master_date_to_rfc3339(start)?,
        day_of_month: None,
        end_date: end.map(master_date_to_rfc3339).transpose()?,
        default_enabled,
        provenance: "global_master",
        description: None,
        category: None,
        scenario_group: None,
        scenario_option: None,
    })
}

fn load_master_rewards(
    connection: &Connection,
    jp_connection: Option<&Connection>,
    timeline: &Value,
) -> Result<Vec<PlannerReward>> {
    let mut rewards = Vec::new();
    load_mission_campaign_rewards(connection, timeline, &mut rewards)?;
    load_story_rewards(
        connection,
        timeline,
        &mut rewards,
        "global_release_date",
        "global_master",
        "full_completion",
        "exact_source",
    )?;
    load_event_exchange_rewards(
        connection,
        timeline,
        &mut rewards,
        "global_release_date",
        "trainer_skills_test",
        TRAINER_SKILLS_TEST_CURRENCY_CATEGORY,
        "Trainer Skills Test exchange rewards",
        "global_master",
        "full_exchange",
        "exact_source",
    )?;
    load_event_exchange_rewards(
        connection,
        timeline,
        &mut rewards,
        "global_release_date",
        "racing_carnival",
        RACING_CARNIVAL_CURRENCY_CATEGORY,
        "Racing Carnival exchange rewards",
        "global_master",
        "full_exchange",
        "exact_source",
    )?;
    if let Some(jp_connection) = jp_connection {
        load_story_rewards(
            jp_connection,
            timeline,
            &mut rewards,
            "jp_release_date",
            "jp_master",
            "jp_reward_parity_full_completion",
            "projected_parity",
        )?;
        load_event_exchange_rewards(
            jp_connection,
            timeline,
            &mut rewards,
            "jp_release_date",
            "trainer_skills_test",
            TRAINER_SKILLS_TEST_CURRENCY_CATEGORY,
            "Trainer Skills Test exchange rewards (projected JP parity)",
            "jp_master",
            "jp_reward_parity_full_exchange",
            "projected_parity",
        )?;
        load_event_exchange_rewards(
            jp_connection,
            timeline,
            &mut rewards,
            "jp_release_date",
            "racing_carnival",
            RACING_CARNIVAL_CURRENCY_CATEGORY,
            "Racing Carnival exchange rewards (projected JP parity)",
            "jp_master",
            "jp_reward_parity_full_exchange",
            "projected_parity",
        )?;
    }
    load_temporary_character_story_rewards(connection, jp_connection, timeline, &mut rewards)?;
    load_scenario_record_rewards(connection, jp_connection, timeline, &mut rewards)?;
    load_training_challenge_rewards(connection, jp_connection, timeline, &mut rewards)?;
    load_racing_carnival_rewards(connection, jp_connection, timeline, &mut rewards)?;
    load_factor_research_rewards(connection, jp_connection, timeline, &mut rewards)?;
    load_masters_challenge_rewards(connection, jp_connection, timeline, &mut rewards)?;
    load_main_story_rewards(connection, jp_connection, timeline, &mut rewards)?;
    load_login_bonus_rewards(connection, &mut rewards)?;
    load_competitive_reward_metadata(connection, timeline, &mut rewards)?;
    Ok(rewards)
}

fn sqlite_table_exists(connection: &Connection, table: &str) -> Result<bool> {
    let count = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(count > 0)
}
fn load_competitive_variants(
    connection: &Connection,
    timeline: &Value,
) -> Result<Vec<PlannerCompetitiveVariant>> {
    load_competitive_variants_for(
        connection,
        timeline,
        "global_release_date",
        "global_master",
        "exact_variant",
    )
}

fn load_competitive_variants_with_jp(
    connection: &Connection,
    jp_connection: Option<&Connection>,
    timeline: &Value,
) -> Result<Vec<PlannerCompetitiveVariant>> {
    let mut variants = load_competitive_variants(connection, timeline)?;
    if let Some(jp_connection) = jp_connection {
        variants.extend(load_competitive_variants_for(
            jp_connection,
            timeline,
            "jp_release_date",
            "jp_master",
            "projected_parity_variant",
        )?);
    }
    let mut seen = BTreeSet::new();
    let mut seen_legend_ids = BTreeSet::new();
    variants.retain(|variant| {
        if variant.competition == "legend_race" {
            return seen_legend_ids.insert(variant.id.clone());
        }
        seen.insert((
            variant.event_id.clone(),
            variant.competition,
            variant.label.clone(),
        ))
    });
    project_missing_competitive_variants(timeline, &mut variants);
    Ok(variants)
}

/// The Global master only contains already-released competitions. When the JP
/// master is unavailable to the deployment, carry the latest confirmed Global
/// reward table forward until an exact Global or JP row replaces it. League of
/// Heroes and Strongest Team have no released Global template yet, so their
/// projected planner currencies use compact baselines extracted from the latest
/// local JP master snapshot.
fn project_missing_competitive_variants(
    timeline: &Value,
    variants: &mut Vec<PlannerCompetitiveVariant>,
) {
    const COMPETITIONS: [&str; 4] = [
        "champions_meeting",
        "league_of_heroes",
        "strongest_team",
        "legend_race",
    ];

    let mut dates = BTreeMap::<String, String>::new();
    let mut confirmed_event_ids = BTreeSet::<String>::new();
    let mut missing_by_competition = BTreeMap::<&str, Vec<String>>::new();
    let existing_event_ids = variants
        .iter()
        .map(|variant| variant.event_id.clone())
        .collect::<BTreeSet<_>>();

    for event in timeline
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(event_id) = event.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(event_type) = event.get("type").and_then(Value::as_str) else {
            continue;
        };
        let Some(date) = event.get("global_release_date").and_then(Value::as_str) else {
            continue;
        };
        dates.insert(event_id.to_string(), date.to_string());
        if event.get("is_confirmed").and_then(Value::as_bool) == Some(true) {
            confirmed_event_ids.insert(event_id.to_string());
            continue;
        }
        if COMPETITIONS.contains(&event_type) && !existing_event_ids.contains(event_id) {
            missing_by_competition
                .entry(event_type)
                .or_default()
                .push(event_id.to_string());
        }
    }

    for competition in COMPETITIONS {
        let Some(missing_event_ids) = missing_by_competition.get_mut(competition) else {
            continue;
        };
        missing_event_ids.sort_by_key(|event_id| dates.get(event_id).cloned());

        let template_event_id = variants
            .iter()
            .filter(|variant| {
                variant.competition == competition
                    && confirmed_event_ids.contains(&variant.event_id)
            })
            .max_by_key(|variant| dates.get(&variant.event_id).cloned())
            .map(|variant| variant.event_id.clone());

        if let Some(template_event_id) = template_event_id {
            let templates = variants
                .iter()
                .filter(|variant| {
                    variant.competition == competition && variant.event_id == template_event_id
                })
                .cloned()
                .collect::<Vec<_>>();

            for event_id in missing_event_ids {
                for template in &templates {
                    variants.push(PlannerCompetitiveVariant {
                        id: format!("projected-{competition}-{event_id}-{}", template.id),
                        competition,
                        event_id: event_id.clone(),
                        master_event_id: template.master_event_id,
                        available_at: None,
                        label: template.label.clone(),
                        source_items: template.source_items.clone(),
                        provenance: "global_reward_parity",
                        confidence: "projected_global_parity",
                        default_enabled: false,
                    });
                }
            }
            continue;
        }

        for event_id in missing_event_ids {
            for template in projected_competition_templates(competition) {
                variants.push(PlannerCompetitiveVariant {
                    id: format!("projected-{competition}-{event_id}-{}", template.id),
                    competition,
                    event_id: event_id.clone(),
                    master_event_id: template.master_event_id,
                    available_at: None,
                    label: template.label.to_string(),
                    source_items: projected_source_items(template.items),
                    provenance: "jp_reward_parity_template",
                    confidence: "projected_jp_parity",
                    default_enabled: false,
                });
            }
        }
    }
}

#[derive(Clone, Copy)]
struct ProjectedCompetitionTemplate {
    id: &'static str,
    master_event_id: i64,
    label: &'static str,
    items: &'static [(i64, i64, i64)],
}

fn projected_competition_templates(competition: &str) -> &'static [ProjectedCompetitionTemplate] {
    const CHAMPIONS_MEETING: &[ProjectedCompetitionTemplate] = &[ProjectedCompetitionTemplate {
        id: "placement-outcomes",
        master_event_id: 0,
        label: "Projected final placement rewards",
        items: &[],
    }];
    const LEGEND_RACE: &[ProjectedCompetitionTemplate] = &[ProjectedCompetitionTemplate {
        id: "first-clear",
        master_event_id: 83,
        label: "First clear (projected Global parity)",
        items: &[(90, 43, 150)],
    }];
    const LEAGUE_OF_HEROES: &[ProjectedCompetitionTemplate] = &[
        ProjectedCompetitionTemplate {
            id: "rank-2",
            master_event_id: 15,
            label: "League rank type 1, rank 12 (1000-1999)",
            items: &[(90, 43, 50)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-3",
            master_event_id: 15,
            label: "League rank type 1, rank 13 (2000-2999)",
            items: &[(90, 43, 50)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-4",
            master_event_id: 15,
            label: "League rank type 1, rank 14 (3000-3999)",
            items: &[(90, 43, 50)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-5",
            master_event_id: 15,
            label: "League rank type 2, rank 21 (4000-5499)",
            items: &[(90, 43, 100)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-6",
            master_event_id: 15,
            label: "League rank type 2, rank 22 (5500-6999)",
            items: &[(90, 43, 100)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-7",
            master_event_id: 15,
            label: "League rank type 2, rank 23 (7000-8499)",
            items: &[(90, 43, 100)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-8",
            master_event_id: 15,
            label: "League rank type 2, rank 24 (8500-9999)",
            items: &[(90, 43, 100)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-9",
            master_event_id: 15,
            label: "League rank type 3, rank 31 (10000-12499)",
            items: &[(90, 43, 300), (40, 111, 1)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-10",
            master_event_id: 15,
            label: "League rank type 3, rank 32 (12500-14999)",
            items: &[(90, 43, 300), (40, 41, 1), (164, 150, 1)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-11",
            master_event_id: 15,
            label: "League rank type 3, rank 33 (15000-17499)",
            items: &[(90, 43, 300), (40, 111, 1), (164, 150, 1)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-12",
            master_event_id: 15,
            label: "League rank type 3, rank 34 (17500-19999)",
            items: &[(90, 43, 300), (40, 41, 1), (164, 149, 1)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-13",
            master_event_id: 15,
            label: "League rank type 4, rank 41 (20000-22999)",
            items: &[(90, 43, 500), (164, 149, 1)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-14",
            master_event_id: 15,
            label: "League rank type 4, rank 42 (23000-25999)",
            items: &[(90, 43, 500)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-15",
            master_event_id: 15,
            label: "League rank type 4, rank 43 (26000-29999)",
            items: &[(90, 43, 500)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-16",
            master_event_id: 15,
            label: "League rank type 4, rank 44 (30000+)",
            items: &[(90, 43, 500)],
        },
    ];
    const STRONGEST_TEAM: &[ProjectedCompetitionTemplate] = &[
        ProjectedCompetitionTemplate {
            id: "rank-5",
            master_event_id: 1008,
            label: "Team rank 5 (130000-139999 evaluation points)",
            items: &[(90, 43, 100)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-9",
            master_event_id: 1008,
            label: "Team rank 9 (170000-174999 evaluation points)",
            items: &[(90, 43, 100)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-13",
            master_event_id: 1008,
            label: "Team rank 13 (190000-194999 evaluation points)",
            items: &[(90, 43, 100)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-17",
            master_event_id: 1008,
            label: "Team rank 17 (210000-214999 evaluation points)",
            items: &[(90, 43, 200)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-18",
            master_event_id: 1008,
            label: "Team rank 18 (215000-219999 evaluation points)",
            items: &[(164, 150, 1)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-19",
            master_event_id: 1008,
            label: "Team rank 19 (220000-224999 evaluation points)",
            items: &[(40, 41, 1)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-21",
            master_event_id: 1008,
            label: "Team rank 21 (230000-234999 evaluation points)",
            items: &[(40, 111, 1)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-25",
            master_event_id: 1008,
            label: "Team rank 25 (250000-254999 evaluation points)",
            items: &[(90, 43, 300)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-27",
            master_event_id: 1008,
            label: "Team rank 27 (260000-264999 evaluation points)",
            items: &[(90, 43, 300)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-28",
            master_event_id: 1008,
            label: "Team rank 28 (265000-269999 evaluation points)",
            items: &[(164, 149, 1)],
        },
        ProjectedCompetitionTemplate {
            id: "rank-30",
            master_event_id: 1008,
            label: "Team rank 30 (275000+ evaluation points)",
            items: &[(90, 43, 500)],
        },
        ProjectedCompetitionTemplate {
            id: "missions",
            master_event_id: 1008,
            label: "Event missions (full completion)",
            items: &[(40, 41, 1), (40, 111, 1), (90, 43, 500), (164, 150, 1)],
        },
    ];

    match competition {
        "champions_meeting" => CHAMPIONS_MEETING,
        "league_of_heroes" => LEAGUE_OF_HEROES,
        "strongest_team" => STRONGEST_TEAM,
        "legend_race" => LEGEND_RACE,
        _ => &[],
    }
}

fn projected_source_items(items: &[(i64, i64, i64)]) -> Vec<PlannerSourceItem> {
    items
        .iter()
        .map(|&(item_category, item_id, amount)| PlannerSourceItem {
            item_category,
            item_id,
            amount,
            mission_count: None,
            odds: None,
            order_min: None,
            order_max: None,
            bonus: None,
        })
        .collect()
}

fn load_competitive_variants_for(
    connection: &Connection,
    timeline: &Value,
    timeline_date_field: &str,
    provenance: &'static str,
    confidence: &'static str,
) -> Result<Vec<PlannerCompetitiveVariant>> {
    let links = timeline_competitive_links(timeline, "champions_meeting", timeline_date_field);
    let mut grouped: BTreeMap<
        i64,
        (
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
            Vec<PlannerSourceItem>,
        ),
    > = BTreeMap::new();
    let mut statement = connection.prepare(
        r#"
        SELECT rate.id, rate.champions_id, schedule.start_date,
               rate.league_type, rate.round_id, rate.win_count, rate.ranking,
               rate.rate, rate.reward_set_id,
               reward.item_category, reward.item_id, reward.item_num,
               reward.odds, reward.order_min, reward.order_max, reward.bonus
        FROM champions_reward_rate AS rate
        JOIN champions_schedule AS schedule ON schedule.id = rate.champions_id
        LEFT JOIN single_mode_reward_set AS reward ON reward.reward_set_id = rate.reward_set_id
        ORDER BY rate.id, reward.id
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
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)?,
            match (
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, Option<i64>>(10)?,
                row.get::<_, Option<i64>>(11)?,
            ) {
                (Some(item_category), Some(item_id), Some(amount)) => Some(PlannerSourceItem {
                    item_category,
                    item_id,
                    amount,
                    mission_count: None,
                    odds: row.get(12)?,
                    order_min: row.get(13)?,
                    order_max: row.get(14)?,
                    bonus: row.get(15)?,
                }),
                _ => None,
            },
        ))
    })?;
    for row in rows {
        let (rate_id, champions_id, start, league, round, wins, ranking, rate, set_id, item) = row?;
        grouped
            .entry(rate_id)
            .or_insert_with(|| {
                (
                    champions_id,
                    start,
                    league,
                    round,
                    wins,
                    ranking,
                    rate,
                    set_id,
                    Vec::new(),
                )
            })
            .8
            .extend(item);
    }
    let mut variants = Vec::new();
    for (rate_id, (champions_id, start, league, round, wins, ranking, rate, set_id, items)) in
        grouped
    {
        let Some((event_id, _)) = timeline_link_near_start(&links, start) else {
            continue;
        };
        let variant_confidence = if items.is_empty() {
            "outcome_only"
        } else {
            confidence
        };
        variants.push(PlannerCompetitiveVariant {
            id: format!("champions-{champions_id}-rate-{rate_id}"),
            competition: "champions_meeting",
            event_id: event_id.clone(),
            master_event_id: champions_id,
            available_at: None,
            label: format!("League {league}, round {round}, {wins} wins, rank {ranking} (rate {rate}, reward set {set_id})"),
            source_items: items,
            provenance,
            confidence: variant_confidence,
            default_enabled: false,
        });
    }

    let heroes_links =
        timeline_competitive_links(timeline, "league_of_heroes", timeline_date_field);
    let mut statement = connection.prepare(
        r#"
        SELECT data.heroes_id, data.start_date, rank.id, rank.league_rank_type,
               rank.league_rank, rank.league_min_value, rank.league_max_value,
               reward.item_category, reward.item_id, reward.item_num
        FROM heroes_data AS data
        JOIN heroes_league_rank AS rank
        JOIN heroes_league_rank_reward_group AS reward
          ON reward.reward_group_id = rank.reward_group_id
        ORDER BY data.heroes_id, rank.id, reward.id
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
            row.get::<_, i64>(6)?,
            PlannerSourceItem {
                item_category: row.get(7)?,
                item_id: row.get(8)?,
                amount: row.get(9)?,
                mission_count: None,
                odds: None,
                order_min: None,
                order_max: None,
                bonus: None,
            },
        ))
    })?;
    let mut heroes: BTreeMap<(i64, i64), (i64, i64, i64, i64, i64, Vec<PlannerSourceItem>)> =
        BTreeMap::new();
    for row in rows {
        let (heroes_id, start, rank_id, rank_type, rank, min, max, item) = row?;
        heroes
            .entry((heroes_id, rank_id))
            .or_insert_with(|| (start, rank_type, rank, min, max, Vec::new()))
            .5
            .push(item);
    }
    for ((heroes_id, rank_id), (start, rank_type, rank, min, max, items)) in heroes {
        let Some((event_id, _)) = timeline_link_near_start(&heroes_links, start) else {
            continue;
        };
        variants.push(PlannerCompetitiveVariant {
            id: format!("heroes-{heroes_id}-rank-{rank_id}"),
            competition: "league_of_heroes",
            event_id: event_id.clone(),
            master_event_id: heroes_id,
            available_at: None,
            label: format!("League rank type {rank_type}, rank {rank} ({min}-{max})"),
            source_items: items,
            provenance,
            confidence,
            default_enabled: false,
        });
    }
    if sqlite_table_exists(connection, "team_building_data")?
        && sqlite_table_exists(connection, "team_building_rank")?
        && sqlite_table_exists(connection, "team_building_rank_reward_group")?
        && sqlite_table_exists(connection, "mission_data")?
    {
        let team_links =
            timeline_competitive_links(timeline, "strongest_team", timeline_date_field);
        let mut statement = connection.prepare(
            r#"
        SELECT data.team_building_event_id, data.start_date, rank.id,
               rank.team_rank, rank.team_min_value, rank.team_max_value,
               reward.item_category, reward.item_id, reward.item_num
        FROM team_building_data AS data
        JOIN team_building_rank AS rank
        JOIN team_building_rank_reward_group AS reward
          ON reward.reward_group_id = rank.reward_group_id
        ORDER BY data.team_building_event_id, rank.id, reward.id
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
                PlannerSourceItem {
                    item_category: row.get(6)?,
                    item_id: row.get(7)?,
                    amount: row.get(8)?,
                    mission_count: None,
                    odds: None,
                    order_min: None,
                    order_max: None,
                    bonus: None,
                },
            ))
        })?;
        let mut team_ranks: BTreeMap<(i64, i64), (i64, i64, i64, i64, Vec<PlannerSourceItem>)> =
            BTreeMap::new();
        for row in rows {
            let (team_id, start, rank_id, rank, min, max, item) = row?;
            team_ranks
                .entry((team_id, rank_id))
                .or_insert_with(|| (start, rank, min, max, Vec::new()))
                .4
                .push(item);
        }
        for ((team_id, rank_id), (start, rank, min, max, items)) in team_ranks {
            let Some((event_id, _)) = timeline_link_near_start(&team_links, start) else {
                continue;
            };
            variants.push(PlannerCompetitiveVariant {
                id: format!("strongest-team-{team_id}-rank-{rank_id}"),
                competition: "strongest_team",
                event_id: event_id.clone(),
                master_event_id: team_id,
                available_at: None,
                label: format!("Team rank {rank} ({min}-{max} evaluation points)"),
                source_items: items,
                provenance,
                confidence,
                default_enabled: false,
            });
        }

        let mut statement = connection.prepare(
        r#"
        SELECT data.team_building_event_id, data.start_date,
               mission.item_category, mission.item_id, SUM(mission.item_num), COUNT(mission.id)
        FROM team_building_data AS data
        JOIN mission_data AS mission
          ON mission.event_id = data.team_building_event_id AND mission.mission_type = 4
        GROUP BY data.team_building_event_id, data.start_date, mission.item_category, mission.item_id
        ORDER BY data.team_building_event_id, mission.item_category, mission.item_id
        "#,
    )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                PlannerSourceItem {
                    item_category: row.get(2)?,
                    item_id: row.get(3)?,
                    amount: row.get(4)?,
                    mission_count: Some(row.get(5)?),
                    odds: None,
                    order_min: None,
                    order_max: None,
                    bonus: None,
                },
            ))
        })?;
        let mut team_missions: BTreeMap<i64, (i64, Vec<PlannerSourceItem>)> = BTreeMap::new();
        for row in rows {
            let (team_id, start, item) = row?;
            team_missions
                .entry(team_id)
                .or_insert_with(|| (start, Vec::new()))
                .1
                .push(item);
        }
        for (team_id, (start, items)) in team_missions {
            let Some((event_id, _)) = timeline_link_near_start(&team_links, start) else {
                continue;
            };
            variants.push(PlannerCompetitiveVariant {
                id: format!("strongest-team-{team_id}-missions"),
                competition: "strongest_team",
                event_id: event_id.clone(),
                master_event_id: team_id,
                available_at: None,
                label: "Event missions (full completion)".to_string(),
                source_items: items,
                provenance,
                confidence,
                default_enabled: false,
            });
        }
    }

    if sqlite_table_exists(connection, "legend_race")? {
        let legend_links = timeline_competitive_links(timeline, "legend_race", timeline_date_field);
        let fallback_links =
            timeline_all_links(timeline, timeline_date_field, "estimated_end_date");
        let mut statement = connection.prepare(
            r#"
            SELECT id, start_date, image_id, race_instance_id,
                   first_clear_item_category_1, first_clear_item_id_1, first_clear_item_num_1,
                   first_clear_item_category_2, first_clear_item_id_2, first_clear_item_num_2,
                   first_clear_item_category_3, first_clear_item_id_3, first_clear_item_num_3
            FROM legend_race
            ORDER BY start_date, id
            "#,
        )?;
        let rows = statement.query_map([], |row| {
            let mut items = Vec::new();
            for offset in [4usize, 7, 10] {
                let item_category = row.get::<_, i64>(offset)?;
                let item_id = row.get::<_, i64>(offset + 1)?;
                let amount = row.get::<_, i64>(offset + 2)?;
                if item_category > 0 && item_id > 0 && amount > 0 {
                    items.push(PlannerSourceItem {
                        item_category,
                        item_id,
                        amount,
                        mission_count: None,
                        odds: None,
                        order_min: None,
                        order_max: None,
                        bonus: None,
                    });
                }
            }
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                items,
            ))
        })?;
        let mut legend_events =
            BTreeMap::<i64, (i64, Vec<(i64, i64, Vec<PlannerSourceItem>)>)>::new();
        for row in rows {
            let (legend_id, start, image_id, race_instance_id, items) = row?;
            let event_key = race_instance_id / 10;
            let entry = legend_events
                .entry(event_key)
                .or_insert_with(|| (start, Vec::new()));
            entry.1.push((legend_id, image_id, items));
        }
        for (event_key, (start, mut opponents)) in legend_events {
            let exact_link = timeline_link_near_start(&legend_links, start);
            if exact_link.is_none() && provenance == "global_master" {
                // Global masters retain unreleased JP rows. Only exact Global
                // timeline matches are evidence that a stored row went live.
                continue;
            }
            let projected_link =
                exact_link.or_else(|| timeline_closest_link(&fallback_links, start));
            let event_id = exact_link
                .map(|link| link.0.clone())
                .unwrap_or_else(|| format!("master-legend-race-{event_key}"));
            let available_at = exact_link
                .is_none()
                .then(|| projected_link.map(|link| link.1.clone()))
                .flatten();
            opponents.sort_by_key(|(legend_id, _, _)| *legend_id);
            for (legend_id, image_id, items) in opponents {
                variants.push(PlannerCompetitiveVariant {
                    id: format!("legend-race-{legend_id}-first-clear"),
                    competition: "legend_race",
                    event_id: event_id.clone(),
                    master_event_id: legend_id,
                    available_at: available_at.clone(),
                    label: format!("First clear vs Character {image_id}"),
                    source_items: items,
                    provenance,
                    confidence,
                    default_enabled: false,
                });
            }
        }
    }
    Ok(variants)
}

fn load_competitive_reward_metadata(
    connection: &Connection,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
) -> Result<()> {
    let timeline_links =
        timeline_competitive_links(timeline, "champions_meeting", "global_release_date");
    let mut statement = connection.prepare(
        r#"
        SELECT schedule.id,
               schedule.start_date,
               COUNT(rate.id)
        FROM champions_schedule AS schedule
        LEFT JOIN champions_reward_rate AS rate
          ON rate.champions_id = schedule.id
        GROUP BY schedule.id, schedule.start_date
        HAVING COUNT(rate.id) > 0
        ORDER BY schedule.id
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
        let (champions_id, start, reward_variants) = row?;
        let Some((event_id, available_at)) = timeline_link_near_start(&timeline_links, start)
        else {
            continue;
        };
        rewards.push(PlannerReward {
            id: format!("champions-meeting-{champions_id}-reward-metadata"),
            label: format!(
                "Champions Meeting rewards ({reward_variants} league/round outcomes; exact contents unavailable)"
            ),
            event_id: Some(event_id.clone()),
            gacha_id: None,
            currency: "free_jewels",
            amount: None,
            available_at: available_at.clone(),
            provenance: "global_master",
            assumption: "qualitative_only",
            default_enabled: false,
            source_url: None,
            source_items: Vec::new(),
            confidence: "qualitative",
            evidence: None,
        });
    }
    Ok(())
}

fn load_mission_campaign_rewards(
    connection: &Connection,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
) -> Result<()> {
    let campaign_links: Vec<CampaignLink> = serde_json::from_slice(TIMELINE_CAMPAIGNS)
        .context("failed to parse timeline campaign links for planner")?;
    let jp_reward_groups: Vec<JpMissionRewardGroup> = serde_json::from_slice(JP_MISSION_REWARDS)
        .context("failed to parse planner JP mission reward catalogue")?;
    let campaign_by_mission = campaign_links
        .iter()
        .filter_map(|campaign| {
            campaign
                .jp_mission_event_id
                .map(|event_id| (event_id, campaign.campaign_id))
        })
        .collect::<BTreeMap<_, _>>();
    let catalog_mission_counts = campaign_links
        .iter()
        .filter_map(|campaign| {
            campaign
                .mission_count
                .map(|mission_count| (campaign.campaign_id, mission_count))
        })
        .collect::<BTreeMap<_, _>>();
    let timeline_events = timeline_event_dates(timeline);
    let timeline_campaigns = timeline
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|event| {
            let event_type = event.get("type").and_then(Value::as_str);
            event_type == Some("mission_campaign")
                || (event_type == Some("campaign")
                    && event
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| id.starts_with("campaign-"))
                    && (event.get("source").and_then(Value::as_str) == Some("campaign")
                        || event
                            .get("tags")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .any(|tag| tag.eq_ignore_ascii_case("mission campaign"))))
        })
        .filter_map(|event| {
            Some((
                event.get("id")?.as_str()?.to_string(),
                event
                    .get("global_release_date")?
                    .as_str()?
                    .get(..19)?
                    .to_string(),
            ))
        })
        .collect::<Vec<_>>();
    let timeline_campaign_ids = timeline_campaigns
        .iter()
        .map(|(event_id, _)| event_id.clone())
        .collect::<BTreeSet<_>>();
    let mut timeline_campaigns_by_start = BTreeMap::<String, Vec<String>>::new();
    for (event_id, start) in &timeline_campaigns {
        timeline_campaigns_by_start
            .entry(start.clone())
            .or_default()
            .push(event_id.clone());
    }
    let mut statement = connection.prepare(
        r#"
        SELECT event_id, MIN(start_date), MAX(end_date), item_category, item_id,
               SUM(item_num), COUNT(*)
        FROM mission_data
        WHERE mission_type = 4
          AND event_id > 0
        GROUP BY event_id, item_category, item_id
        ORDER BY event_id, item_category, item_id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            PlannerSourceItem {
                item_category: row.get(3)?,
                item_id: row.get(4)?,
                amount: row.get(5)?,
                mission_count: Some(row.get(6)?),
                odds: None,
                order_min: None,
                order_max: None,
                bonus: None,
            },
        ))
    })?;
    let mut groups: BTreeMap<i64, (String, String, Vec<PlannerSourceItem>)> = BTreeMap::new();
    for row in rows {
        let (mission_event_id, start, end, item) = row?;
        groups
            .entry(mission_event_id)
            .or_insert_with(|| (start, end, Vec::new()))
            .2
            .push(item);
    }
    let mut global_campaign_ids = BTreeSet::new();
    for (mission_event_id, (start, end, items)) in groups {
        let normalized_start = master_date_to_rfc3339(&start)?
            .get(..19)
            .unwrap_or_default()
            .to_string();
        let exact_event_id = format!("campaign-{mission_event_id}");
        let event_id = timeline_campaign_ids
            .contains(&exact_event_id)
            .then_some(exact_event_id)
            .or_else(|| {
                timeline_campaigns_by_start
                    .get(&normalized_start)
                    .filter(|event_ids| event_ids.len() == 1)
                    .and_then(|event_ids| event_ids.first().cloned())
            });
        if let Some(campaign_id) = event_id
            .as_deref()
            .and_then(|id| id.strip_prefix("campaign-"))
            .and_then(|id| id.parse::<i64>().ok())
        {
            global_campaign_ids.insert(campaign_id);
        }
        let available_at = event_id
            .as_ref()
            .and_then(|event_id| timeline_events.get(event_id))
            .map(|dates| dates.1.clone())
            .unwrap_or(master_date_to_rfc3339(&end)?);
        push_structured_rewards(
            rewards,
            &format!("mission-campaign-{mission_event_id}"),
            "Limited-time mission rewards",
            event_id,
            &available_at,
            "global_master",
            "full_completion",
            true,
            items,
            None,
        );
    }

    let campaign_by_fingerprint = campaign_links
        .iter()
        .filter_map(|campaign| {
            campaign
                .mission_fingerprint
                .as_ref()
                .map(|fingerprint| (fingerprint.clone(), campaign.campaign_id))
        })
        .collect::<BTreeMap<_, _>>();
    for group in jp_reward_groups {
        if group.rewards.is_empty() {
            continue;
        }
        let campaign_id = campaign_by_mission
            .get(&group.jp_mission_event_id)
            .copied()
            .or_else(|| {
                campaign_by_fingerprint
                    .get(&group.mission_fingerprint)
                    .copied()
            });
        if campaign_id
            .and_then(|id| catalog_mission_counts.get(&id).copied())
            .is_some_and(|catalog_count| group.mission_count < catalog_count)
        {
            // Historical JP snapshots can be incomplete even when their group
            // ID still maps to a newer public campaign row. Never expose a
            // partial reward sum as exact planner data.
            continue;
        }
        if campaign_id.is_some_and(|id| global_campaign_ids.contains(&id)) {
            continue;
        }
        let event_id = campaign_id.map(|id| format!("campaign-{id}"));
        let available_at = event_id
            .as_ref()
            .and_then(|id| timeline_events.get(id).map(|dates| dates.1.clone()))
            .unwrap_or_else(|| planner_catalog_date(&group.end_date));
        let label = group
            .jp_title
            .as_deref()
            .unwrap_or("JP limited-time mission rewards");
        push_structured_rewards(
            rewards,
            &format!("jp-master-mission-{}", group.jp_mission_event_id),
            label,
            event_id,
            &available_at,
            "jp_master_snapshot",
            "jp_reward_parity",
            false,
            group.rewards,
            Some(format!(
                "{} JP master mission rows; JP period {} to {}",
                group.mission_count, group.start_date, group.end_date
            )),
        );
    }
    Ok(())
}

fn planner_catalog_date(value: &str) -> String {
    if value.ends_with('Z') || value.contains('+') {
        value.to_string()
    } else {
        format!("{value}Z")
    }
}

#[allow(clippy::too_many_arguments)]
fn push_structured_rewards(
    rewards: &mut Vec<PlannerReward>,
    id: &str,
    label: &str,
    event_id: Option<String>,
    available_at: &str,
    provenance: &'static str,
    assumption: &'static str,
    default_enabled: bool,
    source_items: Vec<PlannerSourceItem>,
    evidence: Option<String>,
) {
    let mut emitted = false;
    for (currency, amount) in planner_equivalents(&source_items) {
        emitted = true;
        let currency_source_items = source_items
            .iter()
            .filter(|item| planner_currency_for_item(item) == Some(currency))
            .cloned()
            .collect();
        rewards.push(PlannerReward {
            id: format!("{id}-{currency}"),
            label: label.to_string(),
            event_id: event_id.clone(),
            gacha_id: None,
            currency,
            amount: Some(amount),
            available_at: available_at.to_string(),
            provenance,
            assumption,
            default_enabled,
            source_url: None,
            source_items: currency_source_items,
            confidence: "exact_source",
            evidence: evidence.clone(),
        });
    }
    if !emitted
        || source_items
            .iter()
            .any(|item| planner_currency_for_item(item).is_none())
    {
        rewards.push(PlannerReward {
            id: format!("{id}-items"),
            label: format!("{label} item details"),
            event_id,
            gacha_id: None,
            currency: "free_jewels",
            amount: None,
            available_at: available_at.to_string(),
            provenance,
            assumption: "qualitative_only",
            default_enabled: false,
            source_url: None,
            source_items: source_items
                .into_iter()
                .filter(|item| planner_currency_for_item(item).is_none())
                .collect(),
            confidence: "exact_source",
            evidence,
        });
    }
}

fn planner_equivalents(items: &[PlannerSourceItem]) -> Vec<(&'static str, i64)> {
    let mut totals = BTreeMap::new();
    for item in items {
        if let Some(currency) = planner_currency_for_item(item) {
            *totals.entry(currency).or_insert(0) += item.amount;
        }
    }
    totals
        .into_iter()
        .filter(|(_, amount)| *amount > 0)
        .collect()
}

fn planner_currency_for_item(item: &PlannerSourceItem) -> Option<&'static str> {
    match (item.item_category, item.item_id) {
        (JEWEL_CATEGORY, JEWEL_ITEM_ID) => Some("free_jewels"),
        (GACHA_TICKET_CATEGORY, UMA_TICKET_ITEM_ID) => Some("uma_ticket"),
        (GACHA_TICKET_CATEGORY, SUPPORT_TICKET_ITEM_ID) => Some("support_ticket"),
        (LIMIT_BREAK_ITEM_CATEGORY, RAINBOW_CRYSTAL_ITEM_ID) => Some("rainbow_crystal"),
        (LIMIT_BREAK_ITEM_CATEGORY, GOLD_CRYSTAL_ITEM_ID) => Some("gold_crystal"),
        _ => None,
    }
}

fn load_story_rewards(
    connection: &Connection,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
    timeline_date_field: &str,
    provenance: &'static str,
    assumption: &'static str,
    confidence: &'static str,
) -> Result<()> {
    let story_links = timeline_story_links(timeline, timeline_date_field);
    let tracked_items = [
        (JEWEL_CATEGORY, JEWEL_ITEM_ID),
        (GACHA_TICKET_CATEGORY, UMA_TICKET_ITEM_ID),
        (GACHA_TICKET_CATEGORY, SUPPORT_TICKET_ITEM_ID),
        (LIMIT_BREAK_ITEM_CATEGORY, RAINBOW_CRYSTAL_ITEM_ID),
        (LIMIT_BREAK_ITEM_CATEGORY, GOLD_CRYSTAL_ITEM_ID),
    ];
    let mut totals: BTreeMap<(i64, i64, i64), i64> = BTreeMap::new();
    for (item_category, item_id) in tracked_items {
        for query in [
            "SELECT story_event_id, SUM(item_num) FROM story_event_point_reward WHERE item_category = ?1 AND item_id = ?2 GROUP BY story_event_id",
            "SELECT story_event_id, SUM(item_num) FROM story_event_mission WHERE item_category = ?1 AND item_id = ?2 GROUP BY story_event_id",
            "SELECT story_event_id, SUM(CASE WHEN add_reward_category_1 = ?1 AND add_reward_id_1 = ?2 THEN add_reward_num_1 ELSE 0 END + CASE WHEN add_reward_category_2 = ?1 AND add_reward_id_2 = ?2 THEN add_reward_num_2 ELSE 0 END) FROM story_event_story_data GROUP BY story_event_id",
        ] {
            let mut statement = connection.prepare(query)?;
            let rows = statement.query_map([item_category, item_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (event_id, amount) = row?;
                *totals
                    .entry((event_id, item_category, item_id))
                    .or_default() += amount;
            }
        }

        let mut statement = connection.prepare(
            r#"
            SELECT finite.story_event_id, SUM(reward.item_num)
            FROM story_event_roulette_bingo AS finite
            JOIN story_event_bingo_reward AS reward
              ON reward.reward_set_id = finite.reward_set_id
            WHERE finite.can_loop = 0
              AND reward.item_category = ?1
              AND reward.item_id = ?2
            GROUP BY finite.story_event_id
            "#,
        )?;
        let rows = statement.query_map([item_category, item_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (event_id, amount) = row?;
            *totals
                .entry((event_id, item_category, item_id))
                .or_default() += amount;
        }
    }

    let mut schedule =
        connection.prepare("SELECT story_event_id, start_date, end_date FROM story_event_data")?;
    let rows = schedule.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (story_event_id, start, end) = row?;
        let source_items = tracked_items
            .into_iter()
            .filter_map(|(item_category, item_id)| {
                let amount = totals
                    .remove(&(story_event_id, item_category, item_id))
                    .unwrap_or_default();
                (amount > 0).then_some(PlannerSourceItem {
                    item_category,
                    item_id,
                    amount,
                    mission_count: None,
                    odds: None,
                    order_min: None,
                    order_max: None,
                    bonus: None,
                })
            })
            .collect::<Vec<_>>();
        if source_items.is_empty() {
            continue;
        }
        let link = timeline_link_near_start(&story_links, start);
        let event_id = link.map(|link| link.0.clone());
        let available_at = link
            .map(|link| link.1.clone())
            .unwrap_or_else(|| timestamp_to_rfc3339(end));
        for (currency, amount) in planner_equivalents(&source_items) {
            let id = if currency == "free_jewels" {
                format!("{provenance}-story-event-{story_event_id}")
            } else {
                format!("{provenance}-story-event-{story_event_id}-{currency}")
            };
            rewards.push(PlannerReward {
                id,
                label: "Story event rewards".to_string(),
                event_id: event_id.clone(),
                gacha_id: None,
                currency,
                amount: Some(amount),
                available_at: available_at.clone(),
                provenance,
                assumption,
                default_enabled: true,
                source_url: None,
                source_items: source_items
                    .iter()
                    .filter(|item| planner_currency_for_item(item) == Some(currency))
                    .cloned()
                    .collect(),
                confidence,
                evidence: Some(
                    "Exact point, mission, story, and finite bingo reward tables from master data"
                        .to_string(),
                ),
            });
        }
    }
    Ok(())
}

fn timeline_typed_links(
    timeline: &Value,
    event_type: &str,
    date_field: &str,
    target_field: &str,
) -> BTreeMap<i64, (String, String)> {
    timeline
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some(event_type))
        .filter_map(|event| {
            let timestamp = DateTime::parse_from_rfc3339(event.get(date_field)?.as_str()?)
                .ok()?
                .timestamp();
            Some((
                timestamp,
                (
                    event.get("id")?.as_str()?.to_string(),
                    event.get(target_field)?.as_str()?.to_string(),
                ),
            ))
        })
        .collect()
}

fn timeline_all_links(
    timeline: &Value,
    date_field: &str,
    target_field: &str,
) -> BTreeMap<i64, (String, String)> {
    timeline
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            let timestamp = DateTime::parse_from_rfc3339(event.get(date_field)?.as_str()?)
                .ok()?
                .timestamp();
            Some((
                timestamp,
                (
                    event.get("id")?.as_str()?.to_string(),
                    event.get(target_field)?.as_str()?.to_string(),
                ),
            ))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn push_master_event_component(
    rewards: &mut Vec<PlannerReward>,
    links: &BTreeMap<i64, (String, String)>,
    fallback_links: &BTreeMap<i64, (String, String)>,
    master_start: i64,
    master_end: i64,
    synthetic_event_id: String,
    reward_id: String,
    label: &str,
    provenance: &'static str,
    assumption: &'static str,
    confidence: &'static str,
    default_enabled: bool,
    items: Vec<PlannerSourceItem>,
) {
    if items.is_empty() {
        return;
    }
    let exact_link = timeline_link_near_start(links, master_start);
    let projected_link =
        exact_link.or_else(|| timeline_link_near_start(fallback_links, master_start));
    let event_id = exact_link
        .map(|link| link.0.clone())
        .or(Some(synthetic_event_id));
    let available_at = projected_link
        .map(|link| link.1.clone())
        .unwrap_or_else(|| timestamp_to_rfc3339(master_end));
    let reward_start = rewards.len();
    push_structured_rewards(
        rewards,
        &reward_id,
        label,
        event_id,
        &available_at,
        provenance,
        assumption,
        default_enabled,
        items,
        None,
    );
    for reward in &mut rewards[reward_start..] {
        reward.confidence = confidence;
    }
}

fn load_scenario_record_rewards(
    connection: &Connection,
    jp_connection: Option<&Connection>,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
) -> Result<()> {
    load_scenario_record_rewards_for(
        connection,
        timeline,
        rewards,
        "global_release_date",
        "global_master",
        "scenario_evaluation_thresholds",
        "exact_source",
    )?;
    if let Some(jp_connection) = jp_connection {
        load_scenario_record_rewards_for(
            jp_connection,
            timeline,
            rewards,
            "jp_release_date",
            "jp_master",
            "jp_reward_parity_scenario_evaluation_thresholds",
            "projected_parity",
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn load_scenario_record_rewards_for(
    connection: &Connection,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
    timeline_date_field: &str,
    provenance: &'static str,
    assumption: &'static str,
    confidence: &'static str,
) -> Result<()> {
    if !sqlite_table_exists(connection, "single_mode_scenario_record")? {
        return Ok(());
    }
    let links = timeline_typed_links(
        timeline,
        "scenario_release",
        timeline_date_field,
        "global_release_date",
    );
    let ordered_links = links.values().cloned().collect::<Vec<_>>();
    let mut grouped = BTreeMap::<i64, Vec<PlannerSourceItem>>::new();
    let mut statement = connection.prepare(
        r#"
        SELECT scenario_id, reward_item_category, reward_item_id, reward_num
        FROM single_mode_scenario_record
        ORDER BY scenario_id, id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    for row in rows {
        let (scenario_id, category, item_id, amount) = row?;
        add_source_item(
            grouped.entry(scenario_id).or_default(),
            category,
            item_id,
            amount,
        );
    }
    for (scenario_id, items) in grouped {
        if scenario_id < 4 {
            continue;
        }
        let Some((event_id, available_at)) = ordered_links.get((scenario_id - 4) as usize) else {
            continue;
        };
        let reward_start = rewards.len();
        push_structured_rewards(
            rewards,
            &format!("{provenance}-scenario-record-{scenario_id}"),
            "Training scenario evaluation rewards",
            Some(event_id.clone()),
            available_at,
            provenance,
            assumption,
            true,
            items,
            None,
        );
        for reward in &mut rewards[reward_start..] {
            reward.confidence = confidence;
        }
    }
    Ok(())
}

fn load_temporary_character_story_rewards(
    connection: &Connection,
    jp_connection: Option<&Connection>,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
) -> Result<()> {
    let mut seen_chara_ids = BTreeSet::new();
    load_temporary_character_story_rewards_for(
        connection,
        timeline,
        rewards,
        "global_release_date",
        "global_master",
        "exact_source",
        &mut seen_chara_ids,
    )?;
    if let Some(jp_connection) = jp_connection {
        load_temporary_character_story_rewards_for(
            jp_connection,
            timeline,
            rewards,
            "jp_release_date",
            "jp_master",
            "projected_parity",
            &mut seen_chara_ids,
        )?;
    }
    Ok(())
}

fn load_temporary_character_story_rewards_for(
    connection: &Connection,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
    timeline_date_field: &str,
    provenance: &'static str,
    confidence: &'static str,
    seen_chara_ids: &mut BTreeSet<i64>,
) -> Result<()> {
    if !sqlite_table_exists(connection, "campaign_chara_story_schedule")?
        || !sqlite_table_exists(connection, "campaign_data")?
        || !sqlite_table_exists(connection, "chara_story_data")?
    {
        return Ok(());
    }

    // Anniversary campaigns re-open many old stories. Only the earliest
    // temporary unlock for each trainee is new claimable income.
    let mut statement = connection.prepare(
        r#"
        WITH campaign_characters AS (
            SELECT DISTINCT schedule.campaign_id, schedule.chara_id,
                            campaign.start_time, campaign.end_time
            FROM campaign_chara_story_schedule AS schedule
            JOIN campaign_data AS campaign
              ON campaign.campaign_id = schedule.campaign_id
        ), ranked AS (
            SELECT campaign_id, chara_id, start_time, end_time,
                   ROW_NUMBER() OVER (
                       PARTITION BY chara_id
                       ORDER BY start_time, campaign_id
                   ) AS occurrence
            FROM campaign_characters
        )
        SELECT ranked.campaign_id, ranked.chara_id,
               ranked.start_time, ranked.end_time,
               story.add_reward_category_1, story.add_reward_id_1,
               SUM(story.add_reward_num_1)
        FROM ranked
        JOIN campaign_chara_story_schedule AS schedule
          ON schedule.campaign_id = ranked.campaign_id
         AND schedule.chara_id = ranked.chara_id
        JOIN chara_story_data AS story
          ON story.story_id = schedule.story_id
        WHERE ranked.occurrence = 1
          AND story.episode_index BETWEEN 1 AND 4
          AND story.add_reward_category_1 > 0
          AND story.add_reward_id_1 > 0
          AND story.add_reward_num_1 > 0
        GROUP BY ranked.campaign_id, ranked.chara_id,
                 ranked.start_time, ranked.end_time,
                 story.add_reward_category_1, story.add_reward_id_1
        ORDER BY ranked.start_time, ranked.campaign_id, ranked.chara_id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            PlannerSourceItem {
                item_category: row.get(4)?,
                item_id: row.get(5)?,
                amount: row.get(6)?,
                mission_count: Some(4),
                odds: None,
                order_min: None,
                order_max: None,
                bonus: None,
            },
        ))
    })?;
    let mut grouped = BTreeMap::<(i64, i64), (i64, i64, Vec<PlannerSourceItem>)>::new();
    for row in rows {
        let (campaign_id, chara_id, start, end, item) = row?;
        grouped
            .entry((campaign_id, chara_id))
            .or_insert_with(|| (start, end, Vec::new()))
            .2
            .push(item);
    }

    let links = timeline_all_links(timeline, timeline_date_field, "global_release_date");
    for ((campaign_id, chara_id), (start, end, items)) in grouped {
        let projected = timeline_link_near_start(&links, start);
        if projected.is_none() && provenance == "global_master" {
            // The Global database also carries dormant JP campaign rows.
            // A nearby Global timeline date is required before treating one
            // as released; JP rows below provide the future projection.
            continue;
        }
        if !seen_chara_ids.insert(chara_id) {
            continue;
        }
        let projected = projected.or_else(|| timeline_closest_link(&links, start));
        let event_id = projected
            .map(|link| link.0.clone())
            .unwrap_or_else(|| format!("temporary-character-stories-{campaign_id}"));
        let available_at = projected
            .map(|link| link.1.clone())
            .unwrap_or_else(|| timestamp_to_rfc3339(end));
        let reward_start = rewards.len();
        push_structured_rewards(
            rewards,
            &format!("{provenance}-temporary-character-stories-{campaign_id}-{chara_id}"),
            &format!("Temporary trainee stories: Character {chara_id} chapters 1-4"),
            Some(event_id),
            &available_at,
            provenance,
            "temporary_character_story_read",
            true,
            items,
            Some(
                "First temporary unlock for this trainee; repeat anniversary unlocks excluded"
                    .to_string(),
            ),
        );
        for reward in &mut rewards[reward_start..] {
            reward.confidence = confidence;
        }
    }
    Ok(())
}

fn add_source_item(
    items: &mut Vec<PlannerSourceItem>,
    item_category: i64,
    item_id: i64,
    amount: i64,
) {
    if item_category <= 0 || item_id <= 0 || amount <= 0 {
        return;
    }
    if let Some(existing) = items
        .iter_mut()
        .find(|item| item.item_category == item_category && item.item_id == item_id)
    {
        existing.amount += amount;
        return;
    }
    items.push(PlannerSourceItem {
        item_category,
        item_id,
        amount,
        mission_count: None,
        odds: None,
        order_min: None,
        order_max: None,
        bonus: None,
    });
}

fn load_slotted_items(
    connection: &Connection,
    table: &str,
    event_column: &str,
    event_id: i64,
    prefix: &str,
    slot_count: usize,
) -> Result<Vec<PlannerSourceItem>> {
    let mut items = Vec::new();
    for slot in 1..=slot_count {
        let query = format!(
            "SELECT {prefix}category_{slot}, {prefix}id_{slot}, \
             {prefix}num_{slot} FROM {table} WHERE {event_column} = ?1"
        );
        let mut statement = connection.prepare(&query)?;
        let rows = statement.query_map([event_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (category, item_id, amount) = row?;
            add_source_item(&mut items, category, item_id, amount);
        }
    }
    Ok(items)
}

fn load_exchange_items(
    connection: &Connection,
    item_exchange_top_id: i64,
) -> Result<Vec<PlannerSourceItem>> {
    let mut items = Vec::new();
    let mut statement = connection.prepare(
        r#"
        SELECT change_item_category, change_item_id,
               change_item_num * change_item_limit_num
        FROM item_exchange
        WHERE item_exchange_top_id = ?1
          AND change_item_limit_num > 0
          AND change_item_num > 0
        "#,
    )?;
    let rows = statement.query_map([item_exchange_top_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (category, item_id, amount) = row?;
        add_source_item(&mut items, category, item_id, amount);
    }
    Ok(items)
}

#[allow(clippy::too_many_arguments)]
fn load_event_exchange_rewards(
    connection: &Connection,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
    timeline_date_field: &str,
    event_type: &str,
    payment_item_category: i64,
    label: &str,
    provenance: &'static str,
    assumption: &'static str,
    confidence: &'static str,
) -> Result<()> {
    if !sqlite_table_exists(connection, "item_exchange")? {
        return Ok(());
    }
    let links = timeline_competitive_links(timeline, event_type, timeline_date_field);
    let mut statement = connection.prepare(
        r#"
        SELECT item_exchange_top_id,
               MIN(start_date),
               MAX(end_date),
               change_item_category,
               change_item_id,
               SUM(change_item_num * change_item_limit_num)
        FROM item_exchange
        WHERE pay_item_category = ?1
          AND change_item_category = ?2
          AND change_item_id IN (?3, ?4)
          AND change_item_num > 0
          AND change_item_limit_num > 0
        GROUP BY item_exchange_top_id, change_item_category, change_item_id
        ORDER BY item_exchange_top_id, change_item_id
        "#,
    )?;
    let rows = statement.query_map(
        [
            payment_item_category,
            GACHA_TICKET_CATEGORY,
            UMA_TICKET_ITEM_ID,
            SUPPORT_TICKET_ITEM_ID,
        ],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                PlannerSourceItem {
                    item_category: row.get(3)?,
                    item_id: row.get(4)?,
                    amount: row.get(5)?,
                    mission_count: None,
                    odds: None,
                    order_min: None,
                    order_max: None,
                    bonus: None,
                },
            ))
        },
    )?;
    let mut groups = BTreeMap::<i64, (i64, i64, Vec<PlannerSourceItem>)>::new();
    for row in rows {
        let (exchange_id, start, end, item) = row?;
        groups
            .entry(exchange_id)
            .or_insert_with(|| (start, end, Vec::new()))
            .2
            .push(item);
    }
    for (exchange_id, (start, end, source_items)) in groups {
        let link = timeline_link_near_start(&links, start);
        let event_id = link.map(|link| link.0.clone());
        let available_at = link
            .map(|link| link.1.clone())
            .unwrap_or_else(|| timestamp_to_rfc3339(end));
        for (currency, amount) in planner_equivalents(&source_items) {
            rewards.push(PlannerReward {
                id: format!("{provenance}-{event_type}-{exchange_id}-{currency}"),
                label: label.to_string(),
                event_id: event_id.clone(),
                gacha_id: None,
                currency,
                amount: Some(amount),
                available_at: available_at.clone(),
                provenance,
                assumption,
                default_enabled: true,
                source_url: None,
                source_items: source_items
                    .iter()
                    .filter(|item| planner_currency_for_item(item) == Some(currency))
                    .cloned()
                    .collect(),
                confidence,
                evidence: Some(
                    "Exact ticket stock and exchange limits from master data".to_string(),
                ),
            });
        }
    }
    Ok(())
}

fn load_training_challenge_rewards(
    connection: &Connection,
    jp_connection: Option<&Connection>,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
) -> Result<()> {
    load_training_challenge_rewards_for(
        connection,
        timeline,
        rewards,
        "global_release_date",
        "global_master",
        "exact_source",
    )?;
    if let Some(jp_connection) = jp_connection {
        load_training_challenge_rewards_for(
            jp_connection,
            timeline,
            rewards,
            "jp_release_date",
            "jp_master",
            "projected_parity",
        )?;
    }
    Ok(())
}

fn load_training_challenge_rewards_for(
    connection: &Connection,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
    timeline_date_field: &str,
    provenance: &'static str,
    confidence: &'static str,
) -> Result<()> {
    if !sqlite_table_exists(connection, "training_challenge_master")? {
        return Ok(());
    }
    let links = timeline_competitive_links(timeline, "trainer_skills_test", timeline_date_field);
    let fallback_links = timeline_all_links(timeline, timeline_date_field, "estimated_end_date");
    let mut statement = connection.prepare(
        r#"
        SELECT id, start_date, end_date, shop_id,
               exam_id_1, exam_id_2, exam_id_3, exam_id_4, exam_id_5,
               ex_exam_id, free_exam_id
        FROM training_challenge_master
        ORDER BY start_date, id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            (4..=10)
                .map(|column| row.get::<_, i64>(column))
                .collect::<rusqlite::Result<Vec<_>>>()?,
        ))
    })?;
    for row in rows {
        let (event_id, start, end, shop_id, exam_ids) = row?;
        let mut score_items = Vec::new();
        for exam_id in exam_ids.into_iter().filter(|id| *id > 0) {
            for item in load_slotted_items(
                connection,
                "training_challenge_exam",
                "id",
                exam_id,
                "item_",
                5,
            )? {
                add_source_item(
                    &mut score_items,
                    item.item_category,
                    item.item_id,
                    item.amount,
                );
            }
        }
        push_master_event_component(
            rewards,
            &links,
            &fallback_links,
            start,
            end,
            format!("trainer-skills-test-master-{event_id}"),
            format!("{provenance}-trainer-skills-test-{event_id}-scores"),
            "Trainer Skills Test score rewards",
            provenance,
            "full_score_completion",
            confidence,
            true,
            score_items,
        );
        push_master_event_component(
            rewards,
            &links,
            &fallback_links,
            start,
            end,
            format!("trainer-skills-test-master-{event_id}"),
            format!("{provenance}-trainer-skills-test-{event_id}-shop"),
            "Trainer Skills Test shop exchanges",
            provenance,
            "all_limited_shop_exchanges",
            confidence,
            true,
            load_exchange_items(connection, shop_id)?,
        );
    }
    Ok(())
}

fn load_racing_carnival_rewards(
    connection: &Connection,
    jp_connection: Option<&Connection>,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
) -> Result<()> {
    let mut seen_event_ids = BTreeSet::new();
    load_racing_carnival_rewards_for(
        connection,
        timeline,
        rewards,
        "global_release_date",
        "global_master",
        "exact_source",
        &mut seen_event_ids,
    )?;
    if let Some(jp_connection) = jp_connection {
        load_racing_carnival_rewards_for(
            jp_connection,
            timeline,
            rewards,
            "jp_release_date",
            "jp_master",
            "projected_parity",
            &mut seen_event_ids,
        )?;
    }
    Ok(())
}

fn load_racing_carnival_rewards_for(
    connection: &Connection,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
    timeline_date_field: &str,
    provenance: &'static str,
    confidence: &'static str,
    seen_event_ids: &mut BTreeSet<i64>,
) -> Result<()> {
    if !sqlite_table_exists(connection, "challenge_match_data")? {
        return Ok(());
    }
    let links = timeline_competitive_links(timeline, "racing_carnival", timeline_date_field);
    let fallback_links = timeline_all_links(timeline, timeline_date_field, "estimated_end_date");
    let mut statement = connection.prepare(
        "SELECT challenge_match_id, start_date, end_date, item_exchange_top_id FROM challenge_match_data ORDER BY start_date",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    for row in rows {
        let (event_id, start, end, shop_id) = row?;
        if !seen_event_ids.insert(event_id) {
            continue;
        }
        push_master_event_component(
            rewards,
            &links,
            &fallback_links,
            start,
            end,
            format!("racing-carnival-master-{event_id}"),
            format!("{provenance}-racing-carnival-{event_id}-first-clear"),
            "Racing Carnival first-clear rewards",
            provenance,
            "all_first_clears",
            confidence,
            true,
            load_slotted_items(
                connection,
                "challenge_match_race",
                "challenge_match_id",
                event_id,
                "first_clear_item_",
                3,
            )?,
        );
        push_master_event_component(
            rewards,
            &links,
            &fallback_links,
            start,
            end,
            format!("racing-carnival-master-{event_id}"),
            format!("{provenance}-racing-carnival-{event_id}-shop"),
            "Racing Carnival shop exchanges",
            provenance,
            "all_limited_shop_exchanges",
            confidence,
            true,
            load_exchange_items(connection, shop_id)?,
        );
        push_master_event_component(
            rewards,
            &links,
            &fallback_links,
            start,
            end,
            format!("racing-carnival-master-{event_id}"),
            format!("{provenance}-racing-carnival-{event_id}-bonus-skill-mission"),
            "Racing Carnival bonus-skill Career mission",
            provenance,
            "racing_carnival_bonus_skill_mission",
            confidence,
            true,
            vec![PlannerSourceItem {
                item_category: JEWEL_CATEGORY,
                item_id: JEWEL_ITEM_ID,
                amount: 100,
                mission_count: Some(1),
                odds: None,
                order_min: None,
                order_max: None,
                bonus: None,
            }],
        );
    }
    Ok(())
}

fn load_factor_research_rewards(
    connection: &Connection,
    jp_connection: Option<&Connection>,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
) -> Result<()> {
    load_factor_research_rewards_for(
        connection,
        timeline,
        rewards,
        "global_release_date",
        "global_master",
        "exact_source",
    )?;
    if let Some(jp_connection) = jp_connection {
        load_factor_research_rewards_for(
            jp_connection,
            timeline,
            rewards,
            "jp_release_date",
            "jp_master",
            "projected_parity",
        )?;
    }
    Ok(())
}

fn load_factor_research_rewards_for(
    connection: &Connection,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
    timeline_date_field: &str,
    provenance: &'static str,
    confidence: &'static str,
) -> Result<()> {
    if !sqlite_table_exists(connection, "factor_research_data")?
        || !sqlite_table_exists(connection, "factor_research_box")?
    {
        return Ok(());
    }
    let links = timeline_competitive_links(timeline, "factor_research", timeline_date_field);
    let fallback_links = timeline_all_links(timeline, timeline_date_field, "estimated_end_date");
    let mut statement = connection.prepare(
        r#"
        SELECT data.factor_research_event_id, data.start_date, data.end_date,
               reward.item_category, reward.item_id,
               SUM(reward.item_num * reward.box_num)
        FROM factor_research_data AS data
        JOIN factor_research_box AS box
          ON box.factor_research_event_id = data.factor_research_event_id
        JOIN factor_research_box_reward AS reward
          ON reward.box_id = box.box_id
        GROUP BY data.factor_research_event_id, data.start_date, data.end_date,
                 reward.item_category, reward.item_id
        ORDER BY data.start_date, data.factor_research_event_id
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
        ))
    })?;
    let mut grouped = BTreeMap::<i64, (i64, i64, Vec<PlannerSourceItem>)>::new();
    for row in rows {
        let (event_id, start, end, category, item_id, amount) = row?;
        let entry = grouped
            .entry(event_id)
            .or_insert_with(|| (start, end, Vec::new()));
        add_source_item(&mut entry.2, category, item_id, amount);
    }
    for (event_id, (start, end, items)) in grouped {
        push_master_event_component(
            rewards,
            &links,
            &fallback_links,
            start,
            end,
            format!("factor-research-master-{event_id}"),
            format!("{provenance}-factor-research-{event_id}"),
            "Agnes Tachyon's Factor Research box rewards",
            provenance,
            "all_reward_boxes",
            confidence,
            true,
            items,
        );
    }
    Ok(())
}

fn load_masters_challenge_rewards(
    connection: &Connection,
    jp_connection: Option<&Connection>,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
) -> Result<()> {
    load_masters_challenge_rewards_for(
        connection,
        timeline,
        rewards,
        "global_release_date",
        "global_master",
        "exact_source",
    )?;
    if let Some(jp_connection) = jp_connection {
        load_masters_challenge_rewards_for(
            jp_connection,
            timeline,
            rewards,
            "jp_release_date",
            "jp_master",
            "projected_parity",
        )?;
    }
    Ok(())
}

fn load_masters_challenge_rewards_for(
    connection: &Connection,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
    timeline_date_field: &str,
    provenance: &'static str,
    confidence: &'static str,
) -> Result<()> {
    if !sqlite_table_exists(connection, "ultimate_race_data")?
        || !sqlite_table_exists(connection, "ultimate_race_contents")?
    {
        return Ok(());
    }
    let links = timeline_competitive_links(timeline, "masters_challenge", timeline_date_field);
    let fallback_links = timeline_all_links(timeline, timeline_date_field, "estimated_end_date");
    let mut statement = connection
        .prepare("SELECT id, start_date, end_date FROM ultimate_race_data ORDER BY start_date")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (event_id, start, end) = row?;
        push_master_event_component(
            rewards,
            &links,
            &fallback_links,
            start,
            end,
            format!("masters-challenge-master-{event_id}"),
            format!("{provenance}-masters-challenge-{event_id}"),
            "Masters Challenge first-clear rewards",
            provenance,
            "all_first_clears_high_difficulty",
            confidence,
            false,
            load_slotted_items(
                connection,
                "ultimate_race_contents",
                "event_id",
                event_id,
                "first_clear_item_",
                5,
            )?,
        );
    }
    Ok(())
}

fn load_main_story_rewards(
    connection: &Connection,
    jp_connection: Option<&Connection>,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
) -> Result<()> {
    load_main_story_rewards_for(
        connection,
        timeline,
        rewards,
        "global_release_date",
        "global_master",
        "exact_source",
    )?;
    if let Some(jp_connection) = jp_connection {
        load_main_story_rewards_for(
            jp_connection,
            timeline,
            rewards,
            "jp_release_date",
            "jp_master",
            "projected_parity",
        )?;
    }
    Ok(())
}

fn load_main_story_rewards_for(
    connection: &Connection,
    timeline: &Value,
    rewards: &mut Vec<PlannerReward>,
    timeline_date_field: &str,
    provenance: &'static str,
    confidence: &'static str,
) -> Result<()> {
    if !sqlite_table_exists(connection, "main_story_part")?
        || !sqlite_table_exists(connection, "main_story_data")?
    {
        return Ok(());
    }
    let projection_links = timeline_all_links(timeline, timeline_date_field, "global_release_date");
    let has_second_reward =
        sqlite_column_exists(connection, "main_story_data", "add_reward_category_2")?;
    let mut statement =
        connection.prepare("SELECT id, start_date FROM main_story_part ORDER BY id")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
    for row in rows {
        let (part_id, start) = row?;
        let mut items = load_slotted_items(
            connection,
            "main_story_data",
            "part_id",
            part_id,
            "add_reward_",
            if has_second_reward { 2 } else { 1 },
        )?;
        items.retain(|item| item.amount > 0);
        if items.is_empty() {
            continue;
        }
        let available_at = if timeline_date_field == "global_release_date" {
            timestamp_to_rfc3339(start)
        } else {
            timeline_link_near_start(&projection_links, start)
                .map(|link| link.1.clone())
                .unwrap_or_else(|| timestamp_to_rfc3339(start))
        };
        let reward_start = rewards.len();
        push_structured_rewards(
            rewards,
            &format!("{provenance}-main-story-part-{part_id}"),
            &format!("Main Story part {part_id} episode rewards"),
            Some(format!("main-story-part-{part_id}")),
            &available_at,
            provenance,
            "all_story_episodes_viewed",
            true,
            items,
            None,
        );
        for reward in &mut rewards[reward_start..] {
            reward.confidence = confidence;
        }
    }
    Ok(())
}

fn sqlite_column_exists(connection: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for item in columns {
        if item? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn load_login_bonus_rewards(
    connection: &Connection,
    rewards: &mut Vec<PlannerReward>,
) -> Result<()> {
    let mut statement = connection.prepare(
        r#"
        SELECT bonus.id, bonus.start_date, bonus.end_date,
               rewards.item_category, rewards.item_id, SUM(rewards.item_num)
        FROM login_bonus_data AS bonus
        JOIN (
            SELECT login_bonus_id, item_category, item_id, item_num FROM login_bonus_detail
            UNION ALL SELECT login_bonus_id, item_category_2, item_id_2, item_num_2 FROM login_bonus_detail
            UNION ALL SELECT login_bonus_id, item_category_3, item_id_3, item_num_3 FROM login_bonus_detail
            UNION ALL SELECT login_bonus_id, item_category_4, item_id_4, item_num_4 FROM login_bonus_detail
            UNION ALL SELECT login_bonus_id, item_category_5, item_id_5, item_num_5 FROM login_bonus_detail
        ) AS rewards ON rewards.login_bonus_id = bonus.id
        WHERE bonus.type = 3
          AND rewards.item_category > 0 AND rewards.item_id > 0 AND rewards.item_num > 0
        GROUP BY bonus.id, bonus.start_date, bonus.end_date, rewards.item_category, rewards.item_id
        ORDER BY bonus.id, rewards.item_category, rewards.item_id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            PlannerSourceItem {
                item_category: row.get(3)?,
                item_id: row.get(4)?,
                amount: row.get(5)?,
                mission_count: None,
                odds: None,
                order_min: None,
                order_max: None,
                bonus: None,
            },
        ))
    })?;
    let mut groups: BTreeMap<i64, (String, String, Vec<PlannerSourceItem>)> = BTreeMap::new();
    for row in rows {
        let (login_id, start, end, item) = row?;
        groups
            .entry(login_id)
            .or_insert_with(|| (start, end, Vec::new()))
            .2
            .push(item);
    }
    for (login_id, (_start, end, items)) in groups {
        push_structured_rewards(
            rewards,
            &format!("login-bonus-{login_id}"),
            "Limited login bonus",
            None,
            &master_date_to_rfc3339(&end)?,
            "global_master",
            "all_login_days",
            true,
            items,
            None,
        );
    }
    Ok(())
}

fn load_global_news_rewards(
    archive: &GlobalNewsArchive,
    jp_archive: &Archive,
    timeline: &Value,
) -> Vec<PlannerReward> {
    let mut rewards = Vec::new();
    for post in &archive.posts {
        let Some(raw) = post.snapshots.last().map(|snapshot| &snapshot.raw) else {
            continue;
        };
        let Some(posted_at) = raw
            .get("post_at")
            .and_then(Value::as_str)
            .and_then(normalize_global_news_timestamp)
        else {
            continue;
        };
        let title = raw
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Official Global reward");
        let message = raw.get("message").and_then(Value::as_str).unwrap_or("");
        let text = html_to_text(message);
        let event_id = jp_archive
            .news
            .iter()
            .find(|candidate| candidate.post_id == post.announce_id)
            .and_then(|candidate| {
                let candidate_title = candidate.title.as_deref().unwrap_or(title);
                match_news_event(candidate, candidate_title, timeline)
            })
            .map(|link| link.event_id);

        if is_global_correction_notice(&text) {
            continue;
        }

        if let Some((daily, total, evidence)) = extract_global_login_bonus_total(&text) {
            rewards.push(global_news_reward(
                format!("global-news-{}-login-bonus", post.announce_id),
                title,
                event_id.clone(),
                total,
                planner_date_after_days(&posted_at, (total / daily).saturating_sub(1)),
                "all_login_days_global",
                &post.page_url,
                format!("{daily} Carats per login day; {evidence}"),
            ));
        }

        for (occurrence, (amount, evidence)) in
            extract_global_direct_gifts(&text).into_iter().enumerate()
        {
            rewards.push(global_news_reward(
                format!(
                    "global-news-{}-gift-{amount}-{occurrence}",
                    post.announce_id
                ),
                title,
                event_id.clone(),
                amount,
                posted_at.clone(),
                "official_global_carat_gift",
                &post.page_url,
                evidence,
            ));
        }
    }
    rewards
}

fn load_global_social_rewards(archive: &GlobalSocialArchive) -> Vec<PlannerReward> {
    let mut rewards = Vec::new();
    for post in &archive.posts {
        let Some(raw) = post.snapshots.last().map(|snapshot| &snapshot.raw) else {
            continue;
        };
        let Some(posted_at) = raw
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(normalize_global_news_timestamp)
        else {
            continue;
        };
        let text = raw.get("text").and_then(Value::as_str).unwrap_or("");
        if !is_confirmed_social_distribution(text) {
            continue;
        }
        let label = social_reward_label(text);
        for (occurrence, (amount, evidence)) in
            extract_global_direct_gifts(text).into_iter().enumerate()
        {
            rewards.push(global_social_reward(
                format!(
                    "global-social-{}-gift-{amount}-{occurrence}",
                    post.status_id
                ),
                &label,
                amount,
                posted_at.clone(),
                &post.status_url,
                evidence,
            ));
        }
    }
    rewards
}

fn is_confirmed_social_distribution(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.contains("chance to win")
        || lower.contains("follow & repost")
        || lower.contains("follow and repost")
        || lower.contains("winner")
    {
        return false;
    }
    [
        "we've just sent out",
        "we have gifted",
        "we've gifted",
        "we're giving everyone",
        "we are giving everyone",
        "we've sent everyone",
        "we have sent everyone",
        "we're sending a gift",
        "we are sending a gift",
        "sent a gift to all trainers",
        "check your presents to claim it now",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn social_reward_label(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or("Official Global social gift");
    if let Some(start) = first_line.find('"') {
        if let Some(end) = first_line[start + 1..].find('"') {
            let quoted = first_line[start + 1..start + 1 + end].trim();
            if !quoted.is_empty() {
                return quoted.to_string();
            }
        }
    }
    first_line.chars().take(140).collect()
}

#[allow(clippy::too_many_arguments)]
fn global_news_reward(
    id: String,
    title: &str,
    event_id: Option<String>,
    amount: i64,
    available_at: String,
    assumption: &'static str,
    source_url: &str,
    evidence: String,
) -> PlannerReward {
    PlannerReward {
        id,
        label: title.to_string(),
        event_id,
        gacha_id: None,
        currency: "free_jewels",
        amount: Some(amount),
        available_at,
        provenance: "global_news",
        assumption,
        default_enabled: true,
        source_url: Some(source_url.to_string()),
        source_items: Vec::new(),
        confidence: "exact_source_text",
        evidence: Some(evidence),
    }
}

fn global_social_reward(
    id: String,
    title: &str,
    amount: i64,
    available_at: String,
    source_url: &str,
    evidence: String,
) -> PlannerReward {
    PlannerReward {
        id,
        label: title.to_string(),
        event_id: None,
        gacha_id: None,
        currency: "free_jewels",
        amount: Some(amount),
        available_at,
        provenance: "global_social",
        assumption: "official_global_carat_gift",
        default_enabled: true,
        source_url: Some(source_url.to_string()),
        source_items: Vec::new(),
        confidence: "exact_source_text",
        evidence: Some(evidence),
    }
}

fn normalize_global_news_timestamp(value: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(value)
        .map(|date| date.with_timezone(&Utc).to_rfc3339())
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|date| date.and_utc().to_rfc3339())
        })
}

fn is_global_correction_notice(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("correct amount")
        && lower.contains("actual amount")
        && (lower.contains("incorrect") || lower.contains("difference"))
}

fn extract_global_login_bonus_total(text: &str) -> Option<(i64, i64, String)> {
    let lower = text.to_ascii_lowercase();
    if !lower.contains("login bonus") || !lower.contains("carat") {
        return None;
    }
    text.lines().find_map(|line| {
        let lower_line = line.to_ascii_lowercase();
        if !lower_line.contains("carat")
            || !(lower_line.contains("each day") || lower_line.contains("per day"))
            || !lower_line.contains("up to")
        {
            return None;
        }
        let up_to = lower_line.find("up to")?;
        let daily = carat_amounts_from_line(&lower_line[..up_to])
            .into_iter()
            .find(|amount| (1..=10_000).contains(amount))?;
        let total = amount_after_phrase(&lower_line, "up to")?;
        (total >= daily && total <= daily.saturating_mul(60) && total % daily == 0)
            .then(|| (daily, total, line.chars().take(320).collect()))
    })
}

fn extract_global_direct_gifts(text: &str) -> Vec<(i64, String)> {
    let lines = text.lines().map(str::trim).collect::<Vec<_>>();
    let mut amounts = BTreeMap::<i64, String>::new();

    for (index, line) in lines.iter().enumerate() {
        if !is_global_gift_heading(line) {
            continue;
        }
        for candidate in lines.iter().skip(index + 1).take(10) {
            if is_global_gift_section_end(candidate) {
                break;
            }
            collect_global_gift_line(candidate, true, &mut amounts);
        }
    }

    for line in &lines {
        let lower = line.to_ascii_lowercase();
        let direct_sentence = lower.contains("gifted eligible trainers with")
            || lower.contains("we've sent everyone")
            || lower.contains("we have sent everyone")
            || lower.contains("we're sending a gift")
            || lower.contains("we are sending a gift")
            || ((lower.contains("we've sent") || lower.contains("we have sent"))
                && lower.contains("gift")
                && lower.contains("trainer"));
        if direct_sentence {
            collect_global_gift_line(line, true, &mut amounts);
        }
    }

    amounts.into_iter().collect()
}

fn collect_global_gift_line(
    line: &str,
    explicit_section: bool,
    amounts: &mut BTreeMap<i64, String>,
) {
    let lower = line.to_ascii_lowercase();
    if !lower.contains("carat")
        || lower.contains("up to")
        || lower.contains("winner")
        || lower.contains("paid carat")
        || has_sales_context(&lower)
        || has_cost_context(&lower)
        || (!explicit_section && !(lower.contains("gift") || lower.contains("gifted")))
    {
        return;
    }
    for amount in carat_amounts_from_line(line) {
        if (1..=100_000).contains(&amount) {
            amounts
                .entry(amount)
                .or_insert_with(|| line.chars().take(320).collect());
        }
    }
}

fn carat_amounts_from_line(line: &str) -> Vec<i64> {
    let lower = line.to_ascii_lowercase();
    let mut amounts = amounts_immediately_before_word(&lower, "carat");
    let mut offset = 0;
    while let Some(relative) = lower[offset..].find("carat") {
        let index = offset + relative;
        if let Some(amount) = amount_after_phrase(&lower[index..], "carat") {
            amounts.push(amount);
        }
        offset = index + "carat".len();
    }
    amounts.sort_unstable();
    amounts.dedup();
    amounts
}

fn is_global_gift_heading(value: &str) -> bool {
    let lower = normalized_global_heading(value);
    lower == "gift contents"
        || lower == "contents of the gift"
        || lower == "present contents"
        || lower == "contents"
}

fn is_global_gift_section_end(value: &str) -> bool {
    let lower = normalized_global_heading(value);
    [
        "eligible trainers",
        "how to receive",
        "gift expiry",
        "availability period",
        "important information",
        "sale period",
        "price",
    ]
    .iter()
    .any(|heading| lower.starts_with(heading))
}

fn normalized_global_heading(value: &str) -> String {
    let value = value
        .find(|character: char| character.is_ascii_alphanumeric())
        .map(|index| &value[index..])
        .unwrap_or("");
    value
        .trim()
        .trim_end_matches(|character: char| character.is_ascii_whitespace() || character == ':')
        .to_ascii_lowercase()
}

fn remove_global_news_login_bonuses_covered_by_master(rewards: &mut Vec<PlannerReward>) {
    let master_logins = rewards
        .iter()
        .enumerate()
        .filter(|(_, reward)| {
            reward.provenance == "global_master"
                && reward.label.to_ascii_lowercase().contains("login bonus")
                && reward.amount.is_some_and(|amount| amount > 0)
        })
        .filter_map(|(index, reward)| {
            Some((
                index,
                reward.amount?,
                planner_timestamp(&reward.available_at)?,
            ))
        })
        .collect::<Vec<_>>();
    let mut used_master_rows = BTreeSet::new();
    let mut suppressed_ids = BTreeSet::new();
    for reward in rewards.iter().filter(|reward| {
        reward.provenance == "global_news" && reward.assumption == "all_login_days_global"
    }) {
        let (Some(amount), Some(timestamp)) =
            (reward.amount, planner_timestamp(&reward.available_at))
        else {
            continue;
        };
        if let Some((master_index, _, _)) = master_logins
            .iter()
            .filter(|(index, master_amount, master_timestamp)| {
                !used_master_rows.contains(index)
                    && *master_amount == amount
                    && (timestamp - *master_timestamp).abs() <= 45 * 86_400
            })
            .min_by_key(|(_, _, master_timestamp)| (timestamp - *master_timestamp).abs())
        {
            used_master_rows.insert(*master_index);
            suppressed_ids.insert(reward.id.clone());
        }
    }
    rewards.retain(|reward| !suppressed_ids.contains(&reward.id));
}

#[derive(Clone, Copy, Debug, Default)]
struct GlobalSocialDeduplication {
    reward_items_removed: usize,
    carats_removed: i64,
}

#[derive(Clone, Copy, Debug, Default)]
struct GlobalRewardMonthAccumulator {
    matched_news_extra_carats: i64,
    en_only_news_carats: i64,
    social_carats: i64,
}

fn remove_global_social_rewards_covered_by_news(
    rewards: &mut Vec<PlannerReward>,
) -> GlobalSocialDeduplication {
    let news_claims = rewards
        .iter()
        .filter(|reward| reward.provenance == "global_news")
        .filter_map(|reward| {
            Some((
                reward.currency,
                reward.amount?,
                planner_timestamp(&reward.available_at)?,
                reward.available_at.get(..10)?.to_string(),
                reward.label.to_ascii_lowercase(),
            ))
        })
        .collect::<Vec<_>>();
    let mut result = GlobalSocialDeduplication::default();
    rewards.retain(|reward| {
        if reward.provenance != "global_social" {
            return true;
        }
        let (Some(amount), Some(timestamp), Some(day)) = (
            reward.amount,
            planner_timestamp(&reward.available_at),
            reward.available_at.get(..10),
        ) else {
            return true;
        };
        let label = reward.label.to_ascii_lowercase();
        let covered_by_news = news_claims.iter().any(
            |(news_currency, news_amount, news_timestamp, news_day, news_label)| {
                *news_currency == reward.currency
                    && *news_amount == amount
                    && (news_day == day
                        || ((news_timestamp - timestamp).abs() <= 2 * 86_400
                            && (news_label.contains(&label) || label.contains(news_label))))
            },
        );
        if covered_by_news {
            result.reward_items_removed += 1;
            result.carats_removed += amount.max(0);
        }
        !covered_by_news
    });
    result
}

fn build_global_reward_comparison(
    global_rewards: &[PlannerReward],
    jp_news_rewards: &[PlannerReward],
    jp_archive: &Archive,
    social_deduplication: GlobalSocialDeduplication,
    archive_as_of: Option<NaiveDate>,
) -> PlannerGlobalRewardComparison {
    let mut global_by_post = BTreeMap::<i64, Vec<&PlannerReward>>::new();
    for reward in global_rewards.iter().filter(|reward| {
        reward.provenance == "global_news"
            && reward.currency == "free_jewels"
            && reward.amount.is_some_and(|amount| amount > 0)
    }) {
        if let Some(post_id) = reward_post_id(&reward.id, "global-news-") {
            global_by_post.entry(post_id).or_default().push(reward);
        }
    }

    let mut jp_by_post = BTreeMap::<i64, i64>::new();
    for reward in jp_news_rewards.iter().filter(|reward| {
        reward.provenance == "jp_news"
            && reward.currency == "free_jewels"
            && reward.amount.is_some_and(|amount| amount > 0)
    }) {
        if let (Some(post_id), Some(amount)) = (reward_post_id(&reward.id, "news-"), reward.amount)
        {
            *jp_by_post.entry(post_id).or_default() += amount;
        }
    }
    let jp_posts = jp_archive
        .news
        .iter()
        .map(|post| (post.post_id, post))
        .collect::<BTreeMap<_, _>>();

    let mut matched_news = Vec::new();
    let mut en_only_news = Vec::new();
    let mut monthly_uplift = BTreeMap::<String, GlobalRewardMonthAccumulator>::new();
    for (announce_id, rewards) in global_by_post {
        let global_carats = rewards.iter().filter_map(|reward| reward.amount).sum();
        let first = rewards[0];
        let title = first.label.clone();
        let global_url = first.source_url.clone().unwrap_or_default();
        let month = first.available_at.get(..7).map(str::to_string);
        if let Some(jp_post) = jp_posts.get(&announce_id) {
            let jp_carats = jp_by_post.get(&announce_id).copied().unwrap_or_default();
            let extra_carats = global_carats - jp_carats;
            if let Some(month) = month {
                monthly_uplift
                    .entry(month)
                    .or_default()
                    .matched_news_extra_carats += extra_carats;
            }
            matched_news.push(PlannerGlobalNewsComparison {
                announce_id,
                title,
                global_carats,
                jp_carats,
                extra_carats,
                global_url,
                jp_url: Some(jp_post.page_url.clone()),
            });
        } else {
            if let Some(month) = month {
                monthly_uplift.entry(month).or_default().en_only_news_carats += global_carats;
            }
            en_only_news.push(PlannerGlobalNewsComparison {
                announce_id,
                title,
                global_carats,
                jp_carats: 0,
                extra_carats: global_carats,
                global_url,
                jp_url: None,
            });
        }
    }

    let matched_news_global_carats = matched_news
        .iter()
        .map(|comparison| comparison.global_carats)
        .sum::<i64>();
    let matched_news_jp_carats = matched_news
        .iter()
        .map(|comparison| comparison.jp_carats)
        .sum::<i64>();
    let matched_news_extra_carats = matched_news
        .iter()
        .map(|comparison| comparison.extra_carats)
        .sum::<i64>();
    let en_only_news_carats = en_only_news
        .iter()
        .map(|comparison| comparison.global_carats)
        .sum::<i64>();
    let social_carats = global_rewards
        .iter()
        .filter(|reward| reward.provenance == "global_social")
        .filter(|reward| reward.currency == "free_jewels")
        .filter_map(|reward| reward.amount)
        .filter(|amount| *amount > 0)
        .sum::<i64>();
    for reward in global_rewards.iter().filter(|reward| {
        reward.provenance == "global_social"
            && reward.currency == "free_jewels"
            && reward.amount.is_some_and(|amount| amount > 0)
    }) {
        if let (Some(month), Some(amount)) = (reward.available_at.get(..7), reward.amount) {
            monthly_uplift
                .entry(month.to_string())
                .or_default()
                .social_carats += amount;
        }
    }
    let social_reward_posts = global_rewards
        .iter()
        .filter(|reward| {
            reward.provenance == "global_social"
                && reward.currency == "free_jewels"
                && reward.amount.is_some_and(|amount| amount > 0)
        })
        .filter_map(|reward| reward.source_url.as_deref())
        .collect::<BTreeSet<_>>()
        .len();
    let speculative_observed_carats =
        (matched_news_extra_carats + en_only_news_carats + social_carats).max(0);

    let mut observed_dates = global_rewards
        .iter()
        .filter(|reward| {
            matches!(reward.provenance, "global_news" | "global_social")
                && reward.currency == "free_jewels"
                && reward.amount.is_some_and(|amount| amount > 0)
        })
        .filter_map(|reward| reward.available_at.get(..10))
        .filter_map(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        .collect::<Vec<_>>();
    observed_dates.sort_unstable();
    let observation_start_date = observed_dates
        .first()
        .copied()
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH.date_naive());
    let observation_end_date = observed_dates
        .last()
        .copied()
        .unwrap_or(observation_start_date);
    let observation_days = (observation_end_date - observation_start_date)
        .num_days()
        .saturating_add(1)
        .max(1);
    let observed_months = observation_days as f64 / (365.2425 / 12.0);
    let observed_months = (observed_months * 1000.0).round() / 1000.0;
    let speculative_mean_monthly_carats =
        (speculative_observed_carats as f64 / observed_months).round() as i64;
    let archive_as_of_date = archive_as_of
        .unwrap_or(observation_end_date)
        .max(observation_end_date);
    let archive_month_start =
        NaiveDate::from_ymd_opt(archive_as_of_date.year(), archive_as_of_date.month(), 1)
            .unwrap_or(archive_as_of_date);
    let speculative_months = (1..=6)
        .rev()
        .filter_map(|months_ago| archive_month_start.checked_sub_months(Months::new(months_ago)))
        .map(|month_start| {
            let month = month_start.format("%Y-%m").to_string();
            let values = monthly_uplift.get(&month).copied().unwrap_or_default();
            PlannerGlobalRewardMonth {
                month,
                matched_news_extra_carats: values.matched_news_extra_carats,
                en_only_news_carats: values.en_only_news_carats,
                social_carats: values.social_carats,
                total_carats: values.matched_news_extra_carats
                    + values.en_only_news_carats
                    + values.social_carats,
            }
        })
        .collect::<Vec<_>>();
    let speculative_monthly_carats = mean_monthly_carats(
        speculative_months
            .iter()
            .map(|month| month.total_carats)
            .collect(),
    )
    .max(0);
    let recent_median_months = speculative_months
        .iter()
        .rev()
        .take(6)
        .rev()
        .collect::<Vec<_>>();
    let speculative_recent_median_monthly_carats = median_monthly_carats(
        recent_median_months
            .iter()
            .map(|month| month.total_carats)
            .collect(),
    )
    .max(0);
    let speculative_recent_median_window_start = recent_median_months
        .first()
        .map(|month| month.month.clone())
        .unwrap_or_default();
    let speculative_recent_median_window_end = recent_median_months
        .last()
        .map(|month| month.month.clone())
        .unwrap_or_default();
    let speculative_window_start = speculative_months
        .first()
        .map(|month| month.month.clone())
        .unwrap_or_default();
    let speculative_window_end = speculative_months
        .last()
        .map(|month| month.month.clone())
        .unwrap_or_default();

    PlannerGlobalRewardComparison {
        news_match_method: "same_announce_id",
        speculative_method: "mean_last_6_complete_calendar_months",
        archive_as_of: archive_as_of_date.to_string(),
        observation_start: observation_start_date.to_string(),
        observation_end: observation_end_date.to_string(),
        observation_days,
        observed_months,
        matched_news_global_carats,
        matched_news_jp_carats,
        matched_news_extra_carats,
        en_only_news_carats,
        social_carats,
        social_reward_posts,
        social_news_duplicate_reward_items_removed: social_deduplication.reward_items_removed,
        social_news_duplicate_carats_removed: social_deduplication.carats_removed,
        speculative_observed_carats,
        speculative_mean_monthly_carats,
        speculative_recent_median_monthly_carats,
        speculative_recent_median_window_start,
        speculative_recent_median_window_end,
        speculative_monthly_carats,
        speculative_window_start,
        speculative_window_end,
        speculative_months,
        matched_news,
        en_only_news,
    }
}

fn mean_monthly_carats(values: Vec<i64>) -> i64 {
    if values.is_empty() {
        return 0;
    }
    (values.iter().map(|value| *value as i128).sum::<i128>() as f64 / values.len() as f64).round()
        as i64
}

fn median_monthly_carats(mut values: Vec<i64>) -> i64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        ((values[middle - 1] as i128 + values[middle] as i128) / 2) as i64
    } else {
        values[middle]
    }
}

fn reward_post_id(value: &str, prefix: &str) -> Option<i64> {
    value.strip_prefix(prefix)?.split('-').next()?.parse().ok()
}

fn prefer_global_news_over_jp_news(rewards: &mut Vec<PlannerReward>) {
    let mut global_claims = BTreeMap::<(String, &'static str, i64, &'static str), usize>::new();
    for reward in rewards
        .iter()
        .filter(|reward| reward.provenance == "global_news")
    {
        let (Some(event_id), Some(amount)) = (reward.event_id.as_ref(), reward.amount) else {
            continue;
        };
        *global_claims
            .entry((
                event_id.clone(),
                reward.currency,
                amount,
                reward_semantic_kind(reward),
            ))
            .or_default() += 1;
    }
    rewards.retain(|reward| {
        if reward.provenance != "jp_news" {
            return true;
        }
        let (Some(event_id), Some(amount)) = (reward.event_id.as_ref(), reward.amount) else {
            return true;
        };
        let key = (
            event_id.clone(),
            reward.currency,
            amount,
            reward_semantic_kind(reward),
        );
        let Some(remaining) = global_claims.get_mut(&key) else {
            return true;
        };
        if *remaining == 0 {
            return true;
        }
        *remaining -= 1;
        false
    });
}

fn reward_semantic_kind(reward: &PlannerReward) -> &'static str {
    if reward.assumption.contains("login") || reward.id.contains("login-bonus") {
        "login_bonus"
    } else {
        "gift"
    }
}

fn planner_timestamp(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .map(|date| date.timestamp())
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(value.get(..10)?, "%Y-%m-%d")
                .ok()?
                .and_hms_opt(0, 0, 0)
                .map(|date| date.and_utc().timestamp())
        })
}

fn load_news_rewards(archive: &Archive, timeline: &Value) -> Vec<PlannerReward> {
    let mut rewards = Vec::new();
    for post in &archive.news {
        let post_reward_start = rewards.len();
        let Some(posted_at) = post.posted_at.as_deref() else {
            continue;
        };
        let title = post.title.as_deref().unwrap_or("News reward");
        let text = archive_primary_text(post);
        let lower = format!("{}\n{}", title, text).to_ascii_lowercase();
        if !has_reward_context(&lower) {
            continue;
        }
        let event_link = match_news_event(post, title, timeline);
        let linked_event_id = event_link.as_ref().map(|link| link.event_id.clone());
        let linked_available_at = event_link
            .as_ref()
            .map(|link| link.end_date.clone())
            .unwrap_or_else(|| posted_at.to_string());
        if let Some((daily, total, evidence)) = extract_login_bonus_total(&text) {
            let login_available_at = event_link
                .as_ref()
                .map(|link| {
                    planner_date_after_days(&link.start_date, (total / daily).saturating_sub(1))
                })
                .unwrap_or_else(|| posted_at.to_string());
            rewards.push(PlannerReward {
                id: format!("news-{}-login-bonus", post.post_id),
                label: title.to_string(),
                event_id: linked_event_id.clone(),
                gacha_id: None,
                currency: "free_jewels",
                amount: Some(total),
                available_at: login_available_at,
                provenance: "jp_news",
                assumption: "all_login_days_jp_parity",
                default_enabled: false,
                source_url: Some(post.page_url.clone()),
                source_items: Vec::new(),
                confidence: "exact_source_text",
                evidence: Some(format!("{daily} jewels per login day; {evidence}")),
            });
        }
        for (section_index, section) in reward_sections(&text).into_iter().enumerate() {
            let lower_section = section.to_ascii_lowercase();
            if (!lower_section.contains("jewel") && !section.contains("ジュエル"))
                || lower_section.contains("login bonus")
                || section.contains("ログインボーナス")
            {
                continue;
            }
            let mut amounts = Vec::new();
            let explicit_gift_section = is_explicit_gift_section(&section);
            let qualitative_mission = lower_section.contains("limited-time mission")
                || lower_section.contains("mission reward")
                || section.contains("期間限定ミッション");
            for line in section.lines() {
                let lower_line = line.to_ascii_lowercase();
                if (!lower_line.contains("jewel") && !line.contains("ジュエル"))
                    || lower_line.contains("paid jewel")
                    || has_sales_context(&lower_line)
                    || has_cost_context(&lower_line)
                {
                    continue;
                }
                let line_amounts = jewel_amounts_from_line(line);
                if line_amounts.is_empty() || !explicit_gift_section {
                    continue;
                }
                for amount in line_amounts {
                    if amount > 0 && amount <= 100_000 {
                        amounts.push(amount);
                    }
                }
            }
            if amounts.is_empty() && !qualitative_mission {
                continue;
            }
            let section_label = section.lines().next().unwrap_or(title).trim();
            let event_id = linked_event_id.clone();
            let available_at = linked_available_at.clone();
            if amounts.is_empty() {
                rewards.push(PlannerReward {
                    id: format!(
                        "news-{}-section-{section_index}-jewels-unknown",
                        post.post_id
                    ),
                    label: section_label.to_string(),
                    event_id,
                    gacha_id: None,
                    currency: "free_jewels",
                    amount: None,
                    available_at,
                    provenance: "jp_fallback",
                    assumption: "qualitative_only",
                    default_enabled: false,
                    source_url: Some(post.page_url.clone()),
                    source_items: Vec::new(),
                    confidence: "qualitative",
                    evidence: Some(section.chars().take(320).collect()),
                });
            } else {
                for (occurrence, amount) in amounts.into_iter().enumerate() {
                    rewards.push(PlannerReward {
                        id: format!(
                            "news-{}-section-{section_index}-jewels-{amount}-{occurrence}",
                            post.post_id
                        ),
                        label: section_label.to_string(),
                        event_id: event_id.clone(),
                        gacha_id: None,
                        currency: "free_jewels",
                        amount: Some(amount),
                        available_at: available_at.clone(),
                        provenance: "jp_news",
                        assumption: "jp_reward_parity",
                        default_enabled: false,
                        source_url: Some(post.page_url.clone()),
                        source_items: Vec::new(),
                        confidence: "exact_source_text",
                        evidence: Some(section.chars().take(320).collect()),
                    });
                }
            }
        }
        prefer_detailed_gift_sections(&mut rewards, post_reward_start);
    }
    rewards
}

fn prefer_detailed_gift_sections(rewards: &mut Vec<PlannerReward>, post_reward_start: usize) {
    let mut post_rewards = rewards.split_off(post_reward_start);
    let detailed_keys = post_rewards
        .iter()
        .filter(|reward| reward.assumption == "jp_reward_parity")
        .filter(|reward| is_detailed_gift_label(&reward.label))
        .filter_map(|reward| reward.amount.map(|amount| (reward.currency, amount)))
        .collect::<BTreeSet<_>>();
    post_rewards.retain(|reward| {
        reward.amount.is_none_or(|amount| {
            !detailed_keys.contains(&(reward.currency, amount))
                || reward.assumption != "jp_reward_parity"
                || is_detailed_gift_label(&reward.label)
        })
    });
    rewards.extend(post_rewards);
}

fn is_detailed_gift_label(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.contains("プレゼントの詳細")
        || lower.contains("gift details")
        || lower.contains("details of the gift")
}

fn load_news_details(archive: &Archive, timeline: &Value) -> Vec<PlannerNewsDetail> {
    let mut details = Vec::new();
    for post in &archive.news {
        let Some(posted_at) = post.posted_at.as_deref() else {
            continue;
        };
        let text = archive_primary_text(post);
        let login = extract_login_bonus_total(&text).is_some();
        let free_pull_claims = extract_news_free_pull_claims(&archive_combined_text(post));
        let free_pull = !free_pull_claims.is_empty();
        let mission = text.contains("期間限定ミッション")
            || text.to_ascii_lowercase().contains("limited-time mission");
        let kinds = [
            login.then_some("login_bonus"),
            free_pull.then_some("free_pulls"),
            mission.then_some("limited_missions"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if kinds.is_empty() {
            continue;
        }
        let title = post.title.as_deref().unwrap_or("Special campaign");
        let event_link = match_news_event(post, title, timeline);
        for kind in kinds {
            details.push(PlannerNewsDetail {
                id: format!("news-{}-{kind}", post.post_id),
                kind,
                event_id: event_link.as_ref().map(|link| link.event_id.clone()),
                title: title.to_string(),
                available_at: event_link
                    .as_ref()
                    .map(|link| match kind {
                        "login_bonus" => extract_login_bonus_total(&text)
                            .map(|(daily, total, _)| {
                                planner_date_after_days(
                                    &link.start_date,
                                    (total / daily).saturating_sub(1),
                                )
                            })
                            .unwrap_or_else(|| link.start_date.clone()),
                        "free_pulls" => free_pull_claims
                            .iter()
                            .filter_map(|claim| claim.days)
                            .max()
                            .map(|days| {
                                planner_date_after_days(&link.start_date, days.saturating_sub(1))
                            })
                            .unwrap_or_else(|| link.start_date.clone()),
                        _ => link.end_date.clone(),
                    })
                    .unwrap_or_else(|| posted_at.to_string()),
                summary: news_detail_summary(&text, kind, title),
                source_url: post.page_url.clone(),
                provenance: "jp_news",
                confidence: "source_text",
            });
        }
    }
    details
}

fn news_detail_summary(text: &str, kind: &str, fallback: &str) -> String {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            let score = match kind {
                "login_bonus" => {
                    4 * i32::from(
                        (line.contains("1日") || lower.contains("per day"))
                            && (line.contains("最大") || lower.contains("up to")),
                    ) + 2 * i32::from(line.contains("ジュエル") || lower.contains("jewel"))
                        + i32::from(
                            line.contains("ログインボーナス") || lower.contains("login bonus"),
                        )
                }
                "free_pulls" => {
                    4 * i32::from(
                        (line.contains("無料") || lower.contains("free"))
                            && (line.contains("1日")
                                || lower.contains("per day")
                                || lower.contains("once a day")),
                    ) + 2 * i32::from(line.contains("日分") || lower.contains("days"))
                        + i32::from(line.contains("無料") || lower.contains("free"))
                }
                _ => {
                    4 * i32::from(
                        (line.contains("ミッション") || lower.contains("mission"))
                            && (line.contains("報酬")
                                || line.contains("ジュエル")
                                || lower.contains("reward")
                                || lower.contains("jewel")),
                    ) + i32::from(line.contains("ミッション") || lower.contains("mission"))
                }
            };
            (score, line)
        })
        .filter(|(score, _)| *score > 0)
        .max_by_key(|(score, _)| *score)
        .map(|(_, line)| line)
        .unwrap_or(fallback)
        .chars()
        .take(360)
        .collect()
}

fn archive_primary_text(post: &ArchiveNews) -> String {
    let message = post
        .raw
        .get("message")
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty() && message.chars().any(is_japanese_character))
        .unwrap_or_else(|| archive_message(post));
    html_to_text(message)
}

fn is_japanese_character(character: char) -> bool {
    matches!(character, '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{9fff}')
}

fn extract_login_bonus_total(value: &str) -> Option<(i64, i64, String)> {
    let lower = value.to_ascii_lowercase();
    if !(lower.contains("login bonus") || value.contains("ログインボーナス"))
        || !(lower.contains("jewel") || value.contains("ジュエル"))
    {
        return None;
    }
    value.lines().find_map(|line| {
        let lower_line = line.to_ascii_lowercase();
        if !(line.contains("ジュエル") || lower_line.contains("jewel"))
            || !(line.contains("1日") || lower_line.contains("per day"))
            || !(line.contains("最大") || lower_line.contains("up to"))
        {
            return None;
        }
        let daily = amount_after_phrase(line, "1日ジュエルを")
            .or_else(|| amount_after_phrase(line, "1日ジュエル"))
            .or_else(|| {
                amounts_before_word(&lower_line, "jewel")
                    .into_iter()
                    .find(|amount| (1..=10_000).contains(amount))
            })?;
        let total = amount_after_phrase(line, "最大で")
            .or_else(|| amount_after_phrase(line, "最大"))
            .or_else(|| amount_after_phrase(&lower_line, "up to"))?;
        (total >= daily && total <= daily.saturating_mul(60))
            .then(|| (daily, total, line.chars().take(320).collect::<String>()))
    })
}

fn amount_after_phrase(value: &str, phrase: &str) -> Option<i64> {
    let index = value.find(phrase)? + phrase.len();
    let mut token = String::new();
    let mut started = false;
    for character in value[index..].chars() {
        if character.is_ascii_digit() || (started && character == ',') {
            started = true;
            token.push(character);
        } else if started {
            break;
        }
    }
    token.replace(',', "").parse().ok()
}

fn jewel_amounts_from_line(line: &str) -> Vec<i64> {
    let lower = line.to_ascii_lowercase();
    let mut amounts = amounts_immediately_before_word(&lower, "jewel");
    let mut english_offset = 0;
    while let Some(relative) = lower[english_offset..].find("jewel") {
        let index = english_offset + relative;
        if let Some(amount) = amount_after_phrase(&lower[index..], "jewel") {
            amounts.push(amount);
        }
        english_offset = index + "jewel".len();
    }
    if line.contains("ジュエル") {
        let mut offset = 0;
        while let Some(relative) = line[offset..].find("ジュエル") {
            let index = offset + relative;
            if let Some(amount) = amount_after_phrase(&line[index..], "ジュエル") {
                amounts.push(amount);
            }
            offset = index + "ジュエル".len();
        }
    }
    amounts.sort_unstable();
    amounts.dedup();
    amounts
}

fn amounts_immediately_before_word(line: &str, word: &str) -> Vec<i64> {
    let mut amounts = Vec::new();
    let mut offset = 0;
    while let Some(relative) = line[offset..].find(word) {
        let index = offset + relative;
        let prefix = &line[..index];
        let trimmed = prefix.trim_end_matches(|character: char| {
            character.is_ascii_whitespace() || matches!(character, ':' | '-' | '・' | '×')
        });
        let token = trimmed
            .split(|character: char| !character.is_ascii_digit() && character != ',')
            .next_back()
            .filter(|token| !token.is_empty());
        if let Some(token) = token {
            if let Ok(amount) = token.replace(',', "").parse::<i64>() {
                amounts.push(amount);
            }
        }
        offset = index + word.len();
    }
    amounts
}

fn is_explicit_gift_section(section: &str) -> bool {
    section.lines().take(4).any(|line| {
        let lower = line.to_ascii_lowercase();
        [
            "gift contents",
            "contents of the gift",
            "present contents",
            "contents of present",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
            || line.contains("プレゼントの内容")
            || line.contains("プレゼント内容")
            || line.contains("配布内容")
            || ((line.contains("プレゼント")
                || lower.contains("gift")
                || lower.contains("present"))
                && !line.contains("ジュエル")
                && !lower.contains("jewel")
                && !line.chars().any(|character| character.is_ascii_digit()))
            || (line.trim_start().starts_with('■')
                && line.contains("プレゼント")
                && !line.contains("ジュエル"))
    })
}

fn reward_sections(text: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let lower = line.to_ascii_lowercase();
        if has_sales_context(&lower) {
            if !current.is_empty() {
                sections.push(current.join("\n"));
                current.clear();
            }
        } else if has_reward_context(&lower) {
            if current.len() == 1 && !current[0].to_ascii_lowercase().contains("jewel") {
                current.push(line);
                continue;
            }
            if !current.is_empty() {
                sections.push(current.join("\n"));
                current.clear();
            }
            current.push(line);
        } else if !current.is_empty() {
            current.push(line);
        }
    }
    if !current.is_empty() {
        sections.push(current.join("\n"));
    }
    sections
}

fn match_news_event(post: &ArchiveNews, title: &str, timeline: &Value) -> Option<NewsEventLink> {
    let lower_title = title.to_ascii_lowercase();
    let (event_type, campaign_match) = if lower_title.contains("champions meeting")
        || lower_title.contains("meeting of champions")
    {
        ("champions_meeting", false)
    } else if lower_title.contains("league of heroes") {
        ("league_of_heroes", false)
    } else if post.event_types.iter().any(|value| value == "campaign")
        && (lower_title.contains("campaign")
            || lower_title.contains("anniversary")
            || archive_primary_text(post).contains("期間限定ミッション")
            || archive_primary_text(post)
                .to_ascii_lowercase()
                .contains("limited-time mission"))
    {
        ("campaign", true)
    } else {
        return None;
    };
    if !post.event_types.iter().any(|value| value == event_type) {
        return None;
    }
    let posted_at = DateTime::parse_from_rfc3339(post.posted_at.as_deref()?).ok()?;
    let mut candidates = timeline
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|event| {
            let actual = event.get("type").and_then(Value::as_str);
            actual == Some(event_type) || (campaign_match && actual == Some("mission_campaign"))
        })
        .filter_map(|event| {
            let start =
                DateTime::parse_from_rfc3339(event.get("jp_release_date")?.as_str()?).ok()?;
            let distance = (start.timestamp() - posted_at.timestamp()).abs();
            let exact_source = event
                .get("umapyoi_url")
                .and_then(Value::as_str)
                .is_some_and(|url| same_news_page(url, &post.page_url));
            let title_match = event
                .get("title")
                .and_then(Value::as_str)
                .is_some_and(|event_title| titles_match(event_title, title));
            let accepted = if campaign_match {
                (exact_source && distance <= 10 * 86_400)
                    || (title_match && distance <= 36 * 60 * 60)
            } else {
                distance <= 10 * 86_400
            };
            accepted.then_some((
                if exact_source {
                    0
                } else if title_match {
                    1
                } else {
                    2
                },
                distance,
                event.get("id")?.as_str()?.to_string(),
                event.get("global_release_date")?.as_str()?.to_string(),
                event.get("estimated_end_date")?.as_str()?.to_string(),
            ))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.cmp(right));
    let best = candidates.first()?;
    if candidates
        .get(1)
        .is_some_and(|candidate| (candidate.0, candidate.1) == (best.0, best.1))
    {
        return None;
    }
    Some(NewsEventLink {
        event_id: best.2.clone(),
        start_date: best.3.clone(),
        end_date: best.4.clone(),
    })
}

fn planner_date_after_days(value: &str, days: i64) -> String {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .and_then(|date| date.timestamp().checked_add(days.clamp(0, 60) * 86_400))
        .and_then(|timestamp| Utc.timestamp_opt(timestamp, 0).single())
        .map(|date| date.to_rfc3339())
        .unwrap_or_else(|| value.to_string())
}

fn same_news_page(left: &str, right: &str) -> bool {
    left.split('?').next().unwrap_or(left).trim_end_matches('/')
        == right
            .split('?')
            .next()
            .unwrap_or(right)
            .trim_end_matches('/')
}

fn titles_match(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        value
            .chars()
            .flat_map(char::to_lowercase)
            .filter(|character| character.is_alphanumeric())
            .collect::<String>()
    };
    let left = normalize(left);
    let right = normalize(right);
    left.len() >= 8
        && right.len() >= 8
        && (left == right || left.contains(&right) || right.contains(&left))
}

fn html_to_text(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut tag = String::new();
    let mut in_tag = false;
    for character in html.chars() {
        if character == '<' {
            in_tag = true;
            tag.clear();
            continue;
        }
        if in_tag {
            if character == '>' {
                let tag_name = tag.trim().to_ascii_lowercase();
                if tag_name.starts_with("br")
                    || tag_name.starts_with("/h")
                    || tag_name.starts_with("h2")
                    || tag_name.starts_with("h3")
                    || tag_name.starts_with("/strong")
                {
                    output.push('\n');
                }
                in_tag = false;
            } else {
                tag.push(character);
            }
            continue;
        }
        output.push(character);
    }
    output
        .replace("&nbsp;", " ")
        .replace("&times;", "x")
        .replace("&amp;", "&")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_reward_context(value: &str) -> bool {
    [
        "gift contents",
        "contents of the gift",
        "login bonus",
        "what you get",
        "mission reward",
        "rewards",
        "free gacha",
        "free draw",
        "free pull",
        "ログインボーナス",
        "報酬",
        "プレゼント",
        "期間限定ミッション",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn has_sales_context(value: &str) -> bool {
    [
        "sale",
        "product",
        "purchase",
        "price",
        "webstore",
        "shop pack",
        "販売",
        "購入",
        "有償",
        "価格",
        "商品",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn has_cost_context(value: &str) -> bool {
    [
        "require",
        "requires",
        "required",
        "costs",
        "cost ",
        "spend",
        "consume",
        "exchange for",
        "消費",
        "必要",
        "交換",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn amounts_before_word(line: &str, word: &str) -> Vec<i64> {
    let lower = line.to_ascii_lowercase();
    let mut amounts = Vec::new();
    let mut offset = 0;
    while let Some(relative) = lower[offset..].find(word) {
        let index = offset + relative;
        let prefix = &line[..index];
        let token = prefix
            .split(|character: char| !character.is_ascii_digit() && character != ',')
            .filter(|token| !token.is_empty())
            .next_back();
        if let Some(amount) = token
            .map(|token| token.replace(',', ""))
            .and_then(|token| token.parse::<i64>().ok())
        {
            amounts.push(amount);
        }
        offset = index + word.len();
    }
    amounts
}

fn deduplicate_rewards(rewards: &mut Vec<PlannerReward>) {
    rewards.sort_by_key(|reward| {
        let priority = match reward.provenance {
            "global_master" => 0,
            "global_news" => 1,
            "global_social" => 2,
            "jp_master_snapshot" | "jp_master_catalog" => 3,
            "jp_news" | "jp_fallback" => 4,
            _ => 4,
        };
        (priority, reward.available_at.clone(), reward.id.clone())
    });
    let mut seen = BTreeSet::new();
    rewards.retain(|reward| {
        let day = reward
            .available_at
            .get(..10)
            .unwrap_or(&reward.available_at);
        seen.insert((
            reward.event_id.clone(),
            reward.gacha_id,
            reward.currency,
            reward.amount,
            day.to_string(),
            reward.label.clone(),
        ))
    });
}

fn timeline_event_dates(timeline: &Value) -> BTreeMap<String, (String, String)> {
    timeline
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            Some((
                event.get("id")?.as_str()?.to_string(),
                (
                    event.get("global_release_date")?.as_str()?.to_string(),
                    event.get("estimated_end_date")?.as_str()?.to_string(),
                ),
            ))
        })
        .collect()
}

fn timeline_story_links(timeline: &Value, date_field: &str) -> BTreeMap<i64, (String, String)> {
    timeline
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("story_event"))
        .filter_map(|event| {
            let timestamp = DateTime::parse_from_rfc3339(event.get(date_field)?.as_str()?)
                .ok()?
                .timestamp();
            Some((
                timestamp,
                (
                    event.get("id")?.as_str()?.to_string(),
                    event.get("estimated_end_date")?.as_str()?.to_string(),
                ),
            ))
        })
        .collect()
}

fn timeline_competitive_links(
    timeline: &Value,
    event_type: &str,
    date_field: &str,
) -> BTreeMap<i64, (String, String)> {
    timeline
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some(event_type))
        .filter_map(|event| {
            let timestamp = DateTime::parse_from_rfc3339(event.get(date_field)?.as_str()?)
                .ok()?
                .timestamp();
            Some((
                timestamp,
                (
                    event.get("id")?.as_str()?.to_string(),
                    event.get("estimated_end_date")?.as_str()?.to_string(),
                ),
            ))
        })
        .collect()
}

fn timeline_link_near_start(
    links: &BTreeMap<i64, (String, String)>,
    master_start: i64,
) -> Option<&(String, String)> {
    const MAX_EVENT_START_OFFSET_SECONDS: i64 = 7 * 24 * 60 * 60;

    links
        .range(
            (master_start - MAX_EVENT_START_OFFSET_SECONDS)
                ..=(master_start + MAX_EVENT_START_OFFSET_SECONDS),
        )
        .min_by_key(|(timeline_start, _)| (*timeline_start - master_start).abs())
        .map(|(_, link)| link)
}

fn timeline_closest_link(
    links: &BTreeMap<i64, (String, String)>,
    master_start: i64,
) -> Option<&(String, String)> {
    links
        .iter()
        .min_by_key(|(timeline_start, _)| (*timeline_start - master_start).abs())
        .map(|(_, link)| link)
}

fn master_date_to_rfc3339(value: &str) -> Result<String> {
    for pattern in [
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d %H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(date) = NaiveDateTime::parse_from_str(value, pattern) {
            return Ok(Utc.from_utc_datetime(&date).to_rfc3339());
        }
    }
    anyhow::bail!("failed to parse planner master date {value}")
}

fn timestamp_to_rfc3339(timestamp: i64) -> String {
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .to_rfc3339()
}

fn mode(values: impl Iterator<Item = i64>) -> Option<i64> {
    let mut counts = BTreeMap::new();
    for value in values {
        *counts.entry(value).or_insert(0usize) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(value, count)| (*count, *value))
        .map(|(value, _)| value)
}

#[cfg(test)]
mod tests {
    use super::{
        amounts_before_word, apply_news_free_pulls, archive_combined_text, build_event_benefits,
        build_global_reward_comparison, extract_free_pull_total, extract_global_direct_gifts,
        extract_global_login_bonus_total, extract_login_bonus_total, extract_month_day_time,
        extract_news_free_pull_claims, extract_news_free_pull_pinned_banner_kinds,
        extract_news_free_pull_target_windows, has_cost_context, has_sales_context, html_to_text,
        is_global_correction_notice, jewel_amounts_from_line, load_archive,
        load_competitive_reward_metadata, load_competitive_variants, load_competitive_variants_for,
        load_daily_pack_rules, load_event_exchange_rewards, load_factor_research_rewards_for,
        load_gachas, load_global_news_rewards, load_global_social_rewards,
        load_mission_campaign_rewards, load_monthly_ticket_exchange_rules, load_news_details,
        load_news_rewards, load_paid_news_income_rules, load_story_rewards,
        load_temporary_character_story_rewards_for, match_news_event,
        partition_news_free_pull_campaign_days, partition_news_free_pull_days,
        planner_reward_event_ids, remove_global_social_rewards_covered_by_news, reward_sections,
        seed_timeline_gacha_fallbacks, timeline_gacha_links, timeline_link_near_start, Archive,
        ArchiveNews, GachaAccumulator, GlobalNewsArchive, GlobalNewsPost, GlobalNewsSnapshot,
        GlobalSocialArchive, GlobalSocialPost, GlobalSocialSnapshot, NewsFreePullCampaignClaim,
        PlannerFreePullAllocation, PlannerGacha, PlannerGachaShard, PlannerReward,
        PlannerSourceItem, TimelineLink, TRAINER_SKILLS_TEST_CURRENCY_CATEGORY,
    };
    use chrono::NaiveDate;
    use rusqlite::Connection;
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn links_competitive_master_starts_only_to_the_nearest_event_in_one_week() {
        let mut links = BTreeMap::new();
        links.insert(1_000_000, ("older".to_string(), "older-end".to_string()));
        links.insert(
            1_500_000,
            ("nearest".to_string(), "nearest-end".to_string()),
        );

        assert_eq!(
            timeline_link_near_start(&links, 1_240_000).map(|link| link.0.as_str()),
            Some("older")
        );
        assert_eq!(
            timeline_link_near_start(&links, 2_200_000).map(|link| link.0.as_str()),
            None
        );
    }

    #[test]
    fn extracts_exact_jewel_amounts_from_news_lines() {
        assert_eq!(
            amounts_before_word("Gift Contents: 1,500 Jewels", "jewel"),
            vec![1500]
        );
        assert_eq!(
            amounts_before_word("Jewels are included", "jewel"),
            Vec::<i64>::new()
        );
    }

    #[test]
    fn extracts_global_direct_gifts_without_repeating_summary_sentences() {
        let text = "In appreciation of your support, we've sent a 1,500-carat gift to all trainers!\nGift Contents\nCarats x 1,500\nEligible Trainers\nAll trainers";
        assert_eq!(
            extract_global_direct_gifts(text)
                .into_iter()
                .map(|(amount, _)| amount)
                .collect::<Vec<_>>(),
            vec![1500]
        );
    }

    #[test]
    fn extracts_detailed_global_login_bonus_but_not_its_preview() {
        let detailed = "1.5-Year Anniversary Celebration Login Bonus has begun!\nIn addition, each day you log in during the event period, you will receive a gift of 300 carats, up to a maximum of 3,000!";
        let preview = "1.5-Year Anniversary Celebration Login Bonus coming soon!\nBy simply logging in every day during the event period, you can receive up to 3,000 carats in presents!";

        assert_eq!(
            extract_global_login_bonus_total(detailed).map(|(daily, total, _)| (daily, total)),
            Some((300, 3000))
        );
        assert_eq!(extract_global_login_bonus_total(preview), None);
    }

    #[test]
    fn identifies_correction_notices_that_must_not_create_new_income() {
        let text = "The amount of carats distributed was incorrect.\n■ Correct Amount\n1,500 carats\n■ Actual Amount\n1,350 carats\n■ Contents\nCarats x 150\nThis value is the difference.";
        assert!(is_global_correction_notice(text));
    }

    #[test]
    fn global_news_rewards_are_enabled_and_exclude_previews_sales_and_contests() {
        let archive = GlobalNewsArchive {
            posts: vec![
                global_post(
                    902,
                    "1.5-Year Anniversary Celebration Part 2 now available!",
                    "To celebrate, we've sent a gift of 3,000 carats to all trainers!<br>Gift Contents<br>Carats x 3,000<br>Eligible Trainers<br>All trainers<br>1.5-Year Anniversary Celebration Login Bonus has begun!<br>Each day you log in, you will receive 300 carats, up to a maximum of 3,000!",
                ),
                global_post(
                    904,
                    "Check out all the latest updates!",
                    "1.5-Year Anniversary Celebration Login Bonus coming soon!<br>By simply logging in every day, you can receive up to 3,000 carats in presents!",
                ),
                global_post(
                    917,
                    "Celebration items now available!",
                    "Sale Details<br>1,500 paid carats<br>Purchase Carats",
                ),
                global_post(
                    200001,
                    "English Launch Celebration Event",
                    "Digital Prize<br>5,000 Carats: 100 winners",
                ),
            ],
        };
        let rewards = load_global_news_rewards(
            &archive,
            &Archive { news: Vec::new() },
            &json!({"events": []}),
        );

        assert_eq!(
            rewards
                .iter()
                .map(|reward| reward.amount.unwrap())
                .collect::<Vec<_>>(),
            vec![3000, 3000]
        );
        assert!(rewards
            .iter()
            .all(|reward| reward.default_enabled && reward.provenance == "global_news"));
    }

    fn global_post(announce_id: i64, title: &str, message: &str) -> GlobalNewsPost {
        GlobalNewsPost {
            announce_id,
            page_url: format!("https://umamusume.com/news/{announce_id}/"),
            first_seen_at: "2026-07-23T00:00:00Z".to_string(),
            snapshots: vec![GlobalNewsSnapshot {
                raw: json!({
                    "announce_id": announce_id,
                    "post_at": "2026-07-22 22:00:00",
                    "title": title,
                    "message": message,
                }),
            }],
        }
    }

    #[test]
    fn global_social_rewards_include_distributed_gifts_but_not_contests() {
        let archive = GlobalSocialArchive {
            posts: vec![
                global_social_post(
                    "2072446465872691644",
                    "Heads up, Trainers! We've just sent out the \"Three Cheers for Trainer! July Giveaway\" gift, so keep an eye out!\n🎁 Gift Contents:\n- Carats ×600\n- Monies ×20,000",
                ),
                global_social_post(
                    "2080000000000000000",
                    "Follow & repost for a chance to win a $50 Gift Card, or 5,000 carats!",
                ),
            ],
        };

        let rewards = load_global_social_rewards(&archive);
        assert_eq!(rewards.len(), 1);
        assert_eq!(rewards[0].amount, Some(600));
        assert_eq!(rewards[0].label, "Three Cheers for Trainer! July Giveaway");
        assert_eq!(rewards[0].provenance, "global_social");
        assert!(rewards[0].default_enabled);
    }

    #[test]
    fn global_social_rewards_include_direct_sent_everyone_carat_lists() {
        let archive = GlobalSocialArchive {
            posts: vec![global_social_post(
                "1938044706874880272",
                "We've sent everyone a Head Start 3-star Voucher, 3,750 Carats, and other items to celebrate launch! Check your presents to claim them.",
            )],
        };

        let rewards = load_global_social_rewards(&archive);
        assert_eq!(rewards.len(), 1);
        assert_eq!(rewards[0].amount, Some(3750));
        assert_eq!(rewards[0].provenance, "global_social");
        assert!(rewards[0].default_enabled);
    }

    #[test]
    fn global_social_rewards_include_sending_gift_to_all_trainers() {
        let archive = GlobalSocialArchive {
            posts: vec![global_social_post(
                "2040917484069597339",
                "Giving Thanks to All Our Trainers! As a token of our gratitude, we're sending a gift of 1,500 carats to all trainers.",
            )],
        };

        let rewards = load_global_social_rewards(&archive);
        assert_eq!(rewards.len(), 1);
        assert_eq!(rewards[0].amount, Some(1500));
        assert_eq!(rewards[0].provenance, "global_social");
        assert!(rewards[0].default_enabled);
    }

    #[test]
    fn correction_news_is_ignored_without_hiding_the_original_social_gift() {
        let news = GlobalNewsArchive {
            posts: vec![global_post(
                100_301,
                "Regarding the Three Cheers for Trainer! August Giveaway Gift Contents",
                "The amount of carats distributed was incorrect.<br>■ Correct Amount<br>1,500 carats<br>■ Actual Amount<br>1,350 carats<br>■ Contents<br>Carats x 150<br>This value is the difference.",
            )],
        };
        let social = GlobalSocialArchive {
            posts: vec![global_social_post(
                "2085127495951753255",
                "Heads up, Trainers! We've just sent out the \"Three Cheers for Trainer! August Giveaway\" gift, so keep an eye out!\n🎁 Gift Contents:\n- Carats ×1,500",
            )],
        };
        let mut rewards =
            load_global_news_rewards(&news, &Archive { news: Vec::new() }, &json!({"events": []}));
        assert!(rewards.is_empty());

        rewards.extend(load_global_social_rewards(&social));
        let deduplication = remove_global_social_rewards_covered_by_news(&mut rewards);

        assert_eq!(rewards.len(), 1);
        assert_eq!(rewards[0].amount, Some(1500));
        assert_eq!(rewards[0].provenance, "global_social");
        assert_eq!(deduplication.reward_items_removed, 0);
        assert_eq!(deduplication.carats_removed, 0);
    }

    #[test]
    fn global_comparison_separates_jp_delta_en_only_and_deduplicated_social() {
        let reward = |id: &str,
                      label: &str,
                      available_at: &str,
                      provenance: &'static str,
                      amount: i64,
                      source_url: &str| PlannerReward {
            id: id.to_string(),
            label: label.to_string(),
            event_id: None,
            gacha_id: None,
            currency: "free_jewels",
            amount: Some(amount),
            available_at: available_at.to_string(),
            provenance,
            assumption: "official_global_carat_gift",
            default_enabled: provenance != "jp_news",
            source_url: Some(source_url.to_string()),
            source_items: Vec::new(),
            confidence: "exact_source_text",
            evidence: None,
        };
        let mut global_rewards = vec![
            reward(
                "global-news-902-gift-1500-0",
                "Matched campaign",
                "2026-01-01T00:00:00Z",
                "global_news",
                1500,
                "https://umamusume.com/news/902/",
            ),
            reward(
                "global-news-100001-gift-300-0",
                "EN maintenance",
                "2026-02-01T00:00:00Z",
                "global_news",
                300,
                "https://umamusume.com/news/100001/",
            ),
            reward(
                "global-social-duplicate-gift-300-0",
                "EN maintenance",
                "2026-02-01T01:00:00Z",
                "global_social",
                300,
                "https://x.com/umamusume_eng/status/1",
            ),
            reward(
                "global-social-unique-gift-600-0",
                "Social-only giveaway",
                "2026-03-01T00:00:00Z",
                "global_social",
                600,
                "https://x.com/umamusume_eng/status/2",
            ),
        ];
        let social_deduplication =
            remove_global_social_rewards_covered_by_news(&mut global_rewards);
        let jp_rewards = vec![reward(
            "news-902-section-0-jewels-1000-0",
            "Matched campaign",
            "2022-01-01T00:00:00Z",
            "jp_news",
            1000,
            "https://umapyoi.net/news/902?lang=jp",
        )];
        let jp_archive = Archive {
            news: vec![ArchiveNews {
                post_id: 902,
                page_url: "https://umapyoi.net/news/902?lang=jp".to_string(),
                title: Some("Matched campaign".to_string()),
                posted_at: Some("2022-01-01T00:00:00Z".to_string()),
                event_types: Vec::new(),
                raw: json!({}),
            }],
        };

        let comparison = build_global_reward_comparison(
            &global_rewards,
            &jp_rewards,
            &jp_archive,
            social_deduplication,
            Some(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()),
        );

        assert_eq!(comparison.matched_news_global_carats, 1500);
        assert_eq!(comparison.matched_news_jp_carats, 1000);
        assert_eq!(comparison.matched_news_extra_carats, 500);
        assert_eq!(comparison.en_only_news_carats, 300);
        assert_eq!(comparison.social_carats, 600);
        assert_eq!(comparison.social_reward_posts, 1);
        assert_eq!(comparison.social_news_duplicate_reward_items_removed, 1);
        assert_eq!(comparison.social_news_duplicate_carats_removed, 300);
        assert_eq!(comparison.speculative_observed_carats, 1400);
        assert_eq!(comparison.matched_news.len(), 1);
        assert_eq!(comparison.en_only_news.len(), 1);
        assert_eq!(comparison.observation_start, "2026-01-01");
        assert_eq!(comparison.observation_end, "2026-03-01");
        assert_eq!(comparison.archive_as_of, "2026-04-01");
        assert_eq!(comparison.speculative_window_start, "2025-10");
        assert_eq!(comparison.speculative_window_end, "2026-03");
        assert_eq!(
            comparison
                .speculative_months
                .iter()
                .map(|month| month.total_carats)
                .collect::<Vec<_>>(),
            vec![0, 0, 0, 500, 300, 600]
        );
        assert_eq!(comparison.speculative_monthly_carats, 233);
        assert_eq!(comparison.speculative_recent_median_monthly_carats, 150);
        assert_eq!(comparison.speculative_recent_median_window_start, "2025-10");
        assert_eq!(comparison.speculative_recent_median_window_end, "2026-03");
    }

    fn global_social_post(status_id: &str, text: &str) -> GlobalSocialPost {
        GlobalSocialPost {
            status_id: status_id.to_string(),
            status_url: format!("https://x.com/umamusume_eng/status/{status_id}"),
            first_seen_at: "2026-07-02T00:00:00Z".to_string(),
            snapshots: vec![GlobalSocialSnapshot {
                raw: json!({
                    "created_at": "2026-07-01T22:25:16.749+00:00",
                    "text": text,
                }),
            }],
        }
    }

    #[test]
    fn timeline_reward_flag_exposes_positive_planner_rewards_and_selectors() {
        let reward = |id: &str, event_id: &str, currency, amount| PlannerReward {
            id: id.to_string(),
            label: id.to_string(),
            event_id: Some(event_id.to_string()),
            gacha_id: None,
            currency,
            amount,
            available_at: "2026-01-01".to_string(),
            provenance: "global_news",
            assumption: "exact",
            default_enabled: false,
            source_url: None,
            source_items: Vec::new(),
            confidence: "exact",
            evidence: None,
        };
        let mut selector = reward("selector", "campaign-selector", "free_jewels", None);
        selector.source_items.push(PlannerSourceItem {
            item_category: 42,
            item_id: 141,
            amount: 1,
            mission_count: Some(1),
            odds: None,
            order_min: None,
            order_max: None,
            bonus: None,
        });
        let ids = planner_reward_event_ids(&[
            reward("jewels", "campaign-jewels", "free_jewels", Some(150)),
            reward("paid", "campaign-paid", "paid_jewels", Some(50)),
            reward("ticket", "campaign-ticket", "uma_ticket", Some(1)),
            reward("unknown", "campaign-unknown", "free_jewels", None),
            reward("zero", "campaign-zero", "free_jewels", Some(0)),
            selector,
        ]);

        assert_eq!(
            ids.into_iter().collect::<Vec<_>>(),
            vec![
                "campaign-jewels",
                "campaign-paid",
                "campaign-selector",
                "campaign-ticket"
            ]
        );
    }

    #[test]
    fn event_benefits_resolve_selectors_and_do_not_duplicate_cloned_source_items() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE text_data (category INTEGER, "index" INTEGER, text TEXT);
                INSERT INTO text_data VALUES (23, 141, 'Half Anniversary SSR Voucher');
                "#,
            )
            .unwrap();
        let selector_reward = PlannerReward {
            id: "mission-campaign-102-items".to_string(),
            label: "Limited-time mission rewards item details".to_string(),
            event_id: Some("campaign-102".to_string()),
            gacha_id: None,
            currency: "free_jewels",
            amount: None,
            available_at: "2025-12-10T14:59:59Z".to_string(),
            provenance: "global_master",
            assumption: "qualitative_only",
            default_enabled: false,
            source_url: None,
            source_items: vec![
                PlannerSourceItem {
                    item_category: 42,
                    item_id: 141,
                    amount: 1,
                    mission_count: Some(1),
                    odds: None,
                    order_min: None,
                    order_max: None,
                    bonus: None,
                },
                PlannerSourceItem {
                    item_category: 41,
                    item_id: 999,
                    amount: 1,
                    mission_count: Some(1),
                    odds: None,
                    order_min: None,
                    order_max: None,
                    bonus: None,
                },
            ],
            confidence: "exact_source",
            evidence: None,
        };
        let gacha_shards = vec![PlannerGachaShard {
            shard: "2025".to_string(),
            gachas: vec![PlannerGacha {
                event_id: "support-banner-2021_30039".to_string(),
                gacha_id: 30039,
                gacha_type: 3,
                banner_kind: "support".to_string(),
                start_date: "2025-11-06T22:00:00Z".to_string(),
                end_date: "2025-11-16T22:00:00Z".to_string(),
                jewel_cost_per_pull: 150,
                spark_pulls: 200,
                free_pulls: 100,
                campaign_id: None,
                free_pulls_provenance: Some("global_master"),
                free_pulls_source_url: None,
                free_pulls_confidence: Some("exact"),
                ticket_currency: Some("support_ticket"),
                provenance: "global_master",
                confidence: "exact",
                pickups: Vec::new(),
                featured_pickups: Vec::new(),
                rarity_rates: Vec::new(),
            }],
        }];

        let benefits = build_event_benefits(
            &connection,
            &[selector_reward.clone(), selector_reward],
            &gacha_shards,
        )
        .unwrap();

        assert_eq!(benefits.len(), 3);
        let support_selector = benefits
            .iter()
            .find(|benefit| benefit.kind == "support_selector")
            .unwrap();
        assert_eq!(support_selector.label, "Half Anniversary SSR Voucher");
        assert_eq!(support_selector.item_category, Some(42));
        assert_eq!(support_selector.item_id, Some(141));
        assert_eq!(support_selector.planner_effect, "informational_only");
        let trainee_selector = benefits
            .iter()
            .find(|benefit| benefit.kind == "trainee_selector")
            .unwrap();
        assert_eq!(trainee_selector.label, "3-star Trainee selector");
        let free_pulls = benefits
            .iter()
            .find(|benefit| benefit.kind == "free_pulls")
            .unwrap();
        assert_eq!(free_pulls.amount, 100);
        assert_eq!(free_pulls.item_category, None);
        assert_eq!(free_pulls.item_id, None);
        assert_eq!(free_pulls.gacha_id, Some(30039));
        assert_eq!(free_pulls.planner_effect, "included_in_banner_free_pulls");
        assert_eq!(free_pulls.provenance, "global_master");
        assert_eq!(free_pulls.confidence, "exact");
    }

    #[test]
    fn preserves_reward_sections_while_stripping_html() {
        let text = html_to_text("<h2>Gift Contents</h2><br>300 Jewels");
        assert!(text.contains("Gift Contents"));
        assert!(text.contains("300 Jewels"));
    }

    #[test]
    fn rejects_paid_sales_lines() {
        assert!(has_sales_context("webstore product sale: 1500 paid jewels"));
    }

    #[test]
    fn rejects_jewel_costs_as_income() {
        assert!(has_cost_context("entry requires 30 jewels"));
        assert!(has_cost_context(
            "entries require either 1 entry ticket or 30 jewels"
        ));
    }

    #[test]
    fn splits_reward_sections_at_paid_sales() {
        let sections = reward_sections(
            "Gift Contents\n300 Jewels\nLimited Jewel Sale\n7500 Jewels\nMission Rewards\nRewards include Jewels",
        );
        assert_eq!(sections.len(), 2);
        assert!(sections[0].contains("300 Jewels"));
        assert!(!sections[0].contains("7500 Jewels"));
    }

    #[test]
    fn extracts_news_free_pull_total() {
        assert_eq!(
            extract_free_pull_total("Up to 80 Free Gacha Pulls! One free 10-pull gacha per day"),
            Some(80)
        );
    }

    #[test]
    fn extracts_japanese_countdown_login_total_without_double_counting_daily_amount() {
        let text = "1.5th Anniversaryカウントダウンログインボーナスが開催中です！\n開催期間中にログインすると、1日ジュエルを150個、ログイン日数に応じて最大で1200個のジュエルをプレゼントいたします。";
        let (daily, total, evidence) = extract_login_bonus_total(text).unwrap();
        assert_eq!(daily, 150);
        assert_eq!(total, 1200);
        assert!(evidence.contains("1200"));
    }

    #[test]
    fn extracts_japanese_daily_single_pull_days_per_banner_kind() {
        let claims = extract_news_free_pull_claims(
            "毎日ガチャ1回無料キャンペーン\n・ピックアップ サポートカードガチャ\n8/14 12:00 ～ 8/24 11:59（11日分）\n開催中のガチャの[1回引く!]を1日1回無料で引けます。",
        );
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].banner_kind, Some("support"));
        assert_eq!(claims[0].pulls_per_day, 1);
        assert_eq!(claims[0].days, Some(11));
    }

    #[test]
    fn news_free_pulls_fill_only_the_unique_matching_banner() {
        let archive = Archive {
            news: vec![ArchiveNews {
                post_id: 1,
                page_url: "https://umapyoi.net/news/1".to_string(),
                title: Some("Up to 80 Free Gacha Pulls!".to_string()),
                posted_at: Some("2026-01-01T03:00:00+00:00".to_string()),
                event_types: Vec::new(),
                raw: json!({
                    "message_english": "One free 10-pull gacha per day.\nTarget Gacha\n2.5th Anniversary Support Card Gacha"
                }),
            }],
        };
        let mut links = BTreeMap::new();
        for (gacha_id, banner_kind, jp_start_date) in [
            (10, "support", "2026-01-01T03:00:00+00:00"),
            (11, "support", "2026-01-05T03:00:00+00:00"),
            (20, "character", "2026-01-01T03:00:00+00:00"),
        ] {
            links.insert(
                gacha_id,
                TimelineLink {
                    event_id: format!("gacha-{gacha_id}"),
                    banner_kind: banner_kind.to_string(),
                    start_date: "2026-07-01T03:00:00+00:00".to_string(),
                    end_date: "2026-07-10T03:00:00+00:00".to_string(),
                    jp_start_date: jp_start_date.to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            );
        }
        let mut gachas = BTreeMap::from([
            (
                10,
                GachaAccumulator {
                    gacha_type: 3,
                    ..GachaAccumulator::default()
                },
            ),
            (
                11,
                GachaAccumulator {
                    gacha_type: 3,
                    ..GachaAccumulator::default()
                },
            ),
            (
                20,
                GachaAccumulator {
                    gacha_type: 3,
                    ..GachaAccumulator::default()
                },
            ),
        ]);

        apply_news_free_pulls(&archive, &links, &mut gachas);

        assert_eq!(gachas[&10].free_pulls, 80);
        assert_eq!(gachas[&11].free_pulls, 0);
        assert_eq!(gachas[&20].free_pulls, 0);
        assert_eq!(gachas[&10].free_pulls_provenance, Some("jp_news"));
    }

    #[test]
    fn daily_free_pull_without_total_or_days_stays_qualitative() {
        let archive = Archive {
            news: vec![ArchiveNews {
                post_id: 2,
                page_url: "https://umapyoi.net/news/2".to_string(),
                title: Some("Anniversary campaign".to_string()),
                posted_at: Some("2022-08-16T03:00:00+00:00".to_string()),
                event_types: Vec::new(),
                raw: json!({
                    "message": "8/16 12:00から毎日ガチャ1回無料キャンペーンが開催中です！開催中のピックアップ プリティーダービーガチャの[1回引く!]を1日1回無料で引けます。"
                }),
            }],
        };
        let links = BTreeMap::from([(
            30,
            TimelineLink {
                event_id: "gacha-30".to_string(),
                banner_kind: "character".to_string(),
                start_date: "2025-08-16T03:00:00+00:00".to_string(),
                end_date: "2025-08-24T02:59:59+00:00".to_string(),
                jp_start_date: "2022-08-16T03:00:00+00:00".to_string(),
                pickup_card_ids: Vec::new(),
                gacha_type: 3,
                is_paid: false,
            },
        )]);
        let mut gachas = BTreeMap::from([(
            30,
            GachaAccumulator {
                gacha_type: 3,
                ..GachaAccumulator::default()
            },
        )]);
        apply_news_free_pulls(&archive, &links, &mut gachas);
        assert_eq!(gachas[&30].free_pulls, 0);
        assert_eq!(gachas[&30].free_pulls_confidence, None);
    }

    #[test]
    fn timeline_fallback_keeps_rates_unknown_and_partitions_daily_pulls_at_banner_change() {
        let links = BTreeMap::from([
            (
                30106,
                TimelineLink {
                    event_id: "banner-2022_30106".to_string(),
                    banner_kind: "character".to_string(),
                    start_date: "2026-07-07T22:00:00Z".to_string(),
                    end_date: "2026-07-15T22:00:00Z".to_string(),
                    jp_start_date: "2022-08-10T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
            (
                30108,
                TimelineLink {
                    event_id: "banner-2022_30108".to_string(),
                    banner_kind: "character".to_string(),
                    start_date: "2026-08-31T22:00:00Z".to_string(),
                    end_date: "2026-09-09T22:00:00Z".to_string(),
                    jp_start_date: "2022-08-19T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
            (
                30109,
                TimelineLink {
                    event_id: "support-banner-2022_30109".to_string(),
                    banner_kind: "support".to_string(),
                    start_date: "2026-08-31T22:00:00Z".to_string(),
                    end_date: "2026-09-08T22:00:00Z".to_string(),
                    jp_start_date: "2022-08-19T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
            (
                30110,
                TimelineLink {
                    event_id: "banner-2022_30110".to_string(),
                    banner_kind: "character".to_string(),
                    start_date: "2026-09-10T22:00:00Z".to_string(),
                    end_date: "2026-09-19T22:00:00Z".to_string(),
                    jp_start_date: "2022-08-24T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
            (
                50001,
                TimelineLink {
                    event_id: "paid-banner-50001".to_string(),
                    banner_kind: "character".to_string(),
                    start_date: "2026-08-01T22:00:00Z".to_string(),
                    end_date: "2026-08-20T22:00:00Z".to_string(),
                    jp_start_date: "2022-08-17T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 5,
                    is_paid: true,
                },
            ),
            (
                30400,
                TimelineLink {
                    event_id: "banner-2026_30400".to_string(),
                    banner_kind: "character".to_string(),
                    start_date: "2030-01-01T22:00:00Z".to_string(),
                    end_date: "2030-01-04T22:00:00Z".to_string(),
                    jp_start_date: "2026-01-05T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 11,
                    is_paid: false,
                },
            ),
        ]);
        let mut gachas = BTreeMap::new();
        seed_timeline_gacha_fallbacks(&links, &mut gachas);
        gachas.insert(
            50001,
            GachaAccumulator {
                gacha_type: 5,
                ..GachaAccumulator::default()
            },
        );
        assert!(gachas[&30108].pickups.is_empty());
        assert_eq!(gachas[&30400].gacha_type, 11);
        assert!(gachas[&30400].pickups.is_empty());
        assert_eq!(gachas[&30108].provenance, Some("jp_fallback"));
        assert_eq!(
            gachas[&30108].confidence,
            Some("timeline_schedule_defaults")
        );

        let archive = Archive {
            news: vec![ArchiveNews {
                post_id: 887,
                page_url: "https://umapyoi.net/news/887?lang=jp".to_string(),
                title: Some("1.5th Anniversary Campaign Vol.1".to_string()),
                posted_at: Some("2022-08-16T03:00:00Z".to_string()),
                event_types: vec!["campaign".to_string(), "gacha".to_string()],
                raw: json!({
                    "message_english": "One Free Gacha Everyday Campaign\nPickup Pretty Derby Gacha\n8/16 12:00 - 8/24 11:59 (for 9 days)\nOne pull of the ongoing gacha is available once a day for free."
                }),
            }],
        };
        apply_news_free_pulls(&archive, &links, &mut gachas);
        assert_eq!(gachas[&30106].free_pulls, 4);
        assert_eq!(gachas[&30108].free_pulls, 5);
        assert_eq!(gachas[&30110].free_pulls, 0);
        assert_eq!(gachas[&30109].free_pulls, 0);
        assert_eq!(
            [30106, 30108, 30110]
                .iter()
                .map(|gacha_id| gachas[gacha_id].free_pulls)
                .sum::<i64>(),
            9
        );
        assert_eq!(
            gachas[&30108].free_pulls_confidence,
            Some("schedule_partitioned")
        );
    }

    #[test]
    fn gacha_resources_keep_exact_featured_and_full_rarity_rates_separate() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE gacha_data (
                    id INTEGER PRIMARY KEY,
                    card_type INTEGER NOT NULL,
                    type INTEGER NOT NULL,
                    cost_single INTEGER NOT NULL
                );
                CREATE TABLE gacha_available (
                    gacha_id INTEGER NOT NULL,
                    card_id INTEGER NOT NULL,
                    is_pickup INTEGER NOT NULL,
                    rarity INTEGER NOT NULL,
                    odds INTEGER NOT NULL
                );
                CREATE TABLE gacha_exchange (
                    gacha_id INTEGER NOT NULL,
                    card_id INTEGER NOT NULL,
                    pay_item_num INTEGER NOT NULL
                );
                INSERT INTO gacha_data VALUES (30104, 1, 3, 150);
                INSERT INTO gacha_available VALUES
                    (30104, 100702, 1, 3, 7500),
                    (30104, 101303, 1, 3, 7500),
                    (30104, 100001, 0, 3, 15000),
                    (30104, 100101, 0, 2, 180000),
                    (30104, 100201, 0, 1, 790000);
                INSERT INTO gacha_exchange VALUES
                    (30104, 100702, 200),
                    (30104, 101303, 200);
                "#,
            )
            .unwrap();
        let links = BTreeMap::from([(
            30104,
            TimelineLink {
                event_id: "banner-30104".to_string(),
                banner_kind: "character".to_string(),
                start_date: "2026-07-01T00:00:00Z".to_string(),
                end_date: "2026-07-10T00:00:00Z".to_string(),
                jp_start_date: "2022-07-01T00:00:00Z".to_string(),
                pickup_card_ids: vec![100702, 101303],
                gacha_type: 3,
                is_paid: false,
            },
        )]);

        let gachas = load_gachas(&connection, &links).unwrap();
        let gacha = &gachas[&30104];
        assert_eq!(gacha.pickups.len(), 2);
        assert!(gacha
            .pickups
            .iter()
            .all(|pickup| (pickup.rate - 0.0075).abs() < 1e-12));
        assert_eq!(gacha.rarity_rates.len(), 3);
        assert!(
            (gacha
                .rarity_rates
                .iter()
                .find(|rate| rate.rarity == 3)
                .unwrap()
                .rate
                - 0.03)
                .abs()
                < 1e-12
        );
        assert!(
            (gacha
                .rarity_rates
                .iter()
                .find(|rate| rate.rarity == 2)
                .unwrap()
                .rate
                - 0.18)
                .abs()
                < 1e-12
        );
        assert!(
            (gacha
                .rarity_rates
                .iter()
                .find(|rate| rate.rarity == 1)
                .unwrap()
                .rate
                - 0.79)
                .abs()
                < 1e-12
        );
    }

    #[test]
    fn timeline_fallback_infers_only_ordinary_one_or_two_pickup_banners() {
        let links = BTreeMap::from([
            (
                100,
                TimelineLink {
                    event_id: "character-one".to_string(),
                    banner_kind: "character".to_string(),
                    start_date: "2027-01-01T00:00:00Z".to_string(),
                    end_date: "2027-01-10T00:00:00Z".to_string(),
                    jp_start_date: "2023-01-01T00:00:00Z".to_string(),
                    pickup_card_ids: vec![100701],
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
            (
                101,
                TimelineLink {
                    event_id: "support-two".to_string(),
                    banner_kind: "support".to_string(),
                    start_date: "2027-01-01T00:00:00Z".to_string(),
                    end_date: "2027-01-10T00:00:00Z".to_string(),
                    jp_start_date: "2023-01-01T00:00:00Z".to_string(),
                    // SR/R pickups are timeline display data, not SSR rate-ups.
                    pickup_card_ids: vec![10011, 20011, 30011, 30012],
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
            (
                102,
                TimelineLink {
                    event_id: "character-three".to_string(),
                    banner_kind: "character".to_string(),
                    start_date: "2027-01-01T00:00:00Z".to_string(),
                    end_date: "2027-01-10T00:00:00Z".to_string(),
                    jp_start_date: "2023-01-01T00:00:00Z".to_string(),
                    pickup_card_ids: vec![100701, 100801, 100901],
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
            (
                103,
                TimelineLink {
                    event_id: "twinkle".to_string(),
                    banner_kind: "character".to_string(),
                    start_date: "2027-01-01T00:00:00Z".to_string(),
                    end_date: "2027-01-10T00:00:00Z".to_string(),
                    jp_start_date: "2023-01-01T00:00:00Z".to_string(),
                    pickup_card_ids: vec![100701],
                    gacha_type: 11,
                    is_paid: false,
                },
            ),
            (
                104,
                TimelineLink {
                    event_id: "pick-two".to_string(),
                    banner_kind: "support".to_string(),
                    start_date: "2027-01-01T00:00:00Z".to_string(),
                    end_date: "2027-01-10T00:00:00Z".to_string(),
                    jp_start_date: "2023-01-01T00:00:00Z".to_string(),
                    pickup_card_ids: (30001..=30010).collect(),
                    gacha_type: 12,
                    is_paid: false,
                },
            ),
            (
                105,
                TimelineLink {
                    event_id: "no-pickups".to_string(),
                    banner_kind: "character".to_string(),
                    start_date: "2027-01-01T00:00:00Z".to_string(),
                    end_date: "2027-01-10T00:00:00Z".to_string(),
                    jp_start_date: "2023-01-01T00:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
            (
                50_001,
                TimelineLink {
                    event_id: "paid-banner-50001".to_string(),
                    banner_kind: "character".to_string(),
                    start_date: "2027-01-01T00:00:00Z".to_string(),
                    end_date: "2027-01-10T00:00:00Z".to_string(),
                    jp_start_date: "2023-01-01T00:00:00Z".to_string(),
                    pickup_card_ids: vec![100701],
                    gacha_type: 3,
                    is_paid: true,
                },
            ),
        ]);
        let mut gachas = BTreeMap::new();

        seed_timeline_gacha_fallbacks(&links, &mut gachas);

        let character = &gachas[&100];
        assert_eq!(character.pickups.len(), 1);
        assert_eq!(character.pickups[0].pickup_id, 100701);
        assert!((character.pickups[0].rate - 0.0075).abs() < 1e-12);
        assert!(character.pickups[0].exchangeable);
        assert_eq!(character.confidence, Some("inferred_standard_rate"));
        assert_eq!(character.rarity_rates.len(), 3);
        assert!((character.rarity_rates[0].rate - 0.03).abs() < 1e-12);
        assert!((character.rarity_rates[1].rate - 0.18).abs() < 1e-12);
        assert!((character.rarity_rates[2].rate - 0.79).abs() < 1e-12);

        let support = &gachas[&101];
        assert_eq!(
            support
                .pickups
                .iter()
                .map(|pickup| pickup.pickup_id)
                .collect::<Vec<_>>(),
            vec![30011, 30012]
        );
        assert!(support
            .pickups
            .iter()
            .all(|pickup| (pickup.rate - 0.0075).abs() < 1e-12));

        for gacha_id in [102, 103, 104, 105] {
            assert!(gachas[&gacha_id].pickups.is_empty());
            assert!(gachas[&gacha_id].rarity_rates.is_empty());
            assert_eq!(
                gachas[&gacha_id].confidence,
                Some("timeline_schedule_defaults")
            );
        }
        assert!(!gachas.contains_key(&50_001));
    }

    #[test]
    fn load_gachas_rejects_zero_overlap_global_master_id_collision() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE gacha_data (
                    id INTEGER PRIMARY KEY,
                    card_type INTEGER NOT NULL,
                    type INTEGER NOT NULL,
                    cost_single INTEGER NOT NULL
                );
                CREATE TABLE gacha_available (
                    gacha_id INTEGER NOT NULL,
                    card_id INTEGER NOT NULL,
                    is_pickup INTEGER NOT NULL,
                    rarity INTEGER NOT NULL,
                    odds INTEGER NOT NULL
                );
                CREATE TABLE gacha_exchange (
                    gacha_id INTEGER NOT NULL,
                    card_id INTEGER NOT NULL,
                    pay_item_num INTEGER NOT NULL
                );
                INSERT INTO gacha_data VALUES (30130, 1, 3, 150);
                INSERT INTO gacha_available VALUES
                    (30130, 100101, 1, 3, 7500),
                    (30130, 100201, 0, 3, 22500),
                    (30130, 100301, 0, 2, 180000),
                    (30130, 100401, 0, 1, 790000);
                INSERT INTO gacha_exchange VALUES (30130, 100101, 200);
                "#,
            )
            .unwrap();
        let links = BTreeMap::from([(
            30130,
            TimelineLink {
                event_id: "banner-2022_30130".to_string(),
                banner_kind: "character".to_string(),
                start_date: "2026-10-11T22:00:00Z".to_string(),
                end_date: "2026-10-20T22:00:00Z".to_string(),
                jp_start_date: "2022-09-20T03:00:00Z".to_string(),
                pickup_card_ids: vec![104201],
                gacha_type: 3,
                is_paid: false,
            },
        )]);

        let gachas = load_gachas(&connection, &links).unwrap();
        let gacha = &gachas[&30130];
        assert_eq!(gacha.provenance, Some("jp_fallback"));
        assert_eq!(gacha.confidence, Some("inferred_standard_rate"));
        assert_eq!(gacha.pickups.len(), 1);
        assert_eq!(gacha.pickups[0].pickup_id, 104201);
        assert!((gacha.pickups[0].rate - 0.0075).abs() < 1e-12);
        assert!(gacha.pickups[0].exchangeable);
        assert_eq!(gacha.rarity_rates.len(), 3);
        assert!((gacha.rarity_rates[0].rate - 0.03).abs() < 1e-12);
    }

    #[test]
    fn partition_refuses_two_same_kind_banners_with_the_same_jp_start() {
        let links = BTreeMap::from([
            (
                1,
                TimelineLink {
                    event_id: "banner-1".to_string(),
                    banner_kind: "character".to_string(),
                    start_date: "2026-01-01T00:00:00Z".to_string(),
                    end_date: "2026-01-10T00:00:00Z".to_string(),
                    jp_start_date: "2022-01-01T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
            (
                2,
                TimelineLink {
                    event_id: "banner-2".to_string(),
                    banner_kind: "character".to_string(),
                    start_date: "2026-01-01T00:00:00Z".to_string(),
                    end_date: "2026-01-10T00:00:00Z".to_string(),
                    jp_start_date: "2022-01-01T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
        ]);
        let gachas = BTreeMap::from([
            (
                1,
                GachaAccumulator {
                    gacha_type: 3,
                    ..GachaAccumulator::default()
                },
            ),
            (
                2,
                GachaAccumulator {
                    gacha_type: 3,
                    ..GachaAccumulator::default()
                },
            ),
        ]);
        let claim = extract_news_free_pull_claims(
            "One Free Gacha Everyday Campaign\nPickup Pretty Derby Gacha\n1/1 12:00 - 1/1 23:59 (for 1 days)",
        )
        .pop()
        .unwrap();
        assert!(partition_news_free_pull_days(1_640_995_200, &claim, &links, &gachas).is_none());
    }

    #[test]
    fn paid_news_gachas_make_an_incomplete_campaign_omit_instead_of_reassign() {
        let timeline = json!({"events": [
            {
                "id": "support-banner-early",
                "type": "support_card_banner",
                "source": "support",
                "card_type": "support",
                "gacha_id": 30100,
                "gacha_type": 3,
                "global_release_date": "2026-01-01T03:00:00Z",
                "estimated_end_date": "2026-01-10T03:00:00Z",
                "jp_release_date": "2022-01-01T03:00:00Z"
            },
            {
                "id": "umapyoi-gacha-50228",
                "type": "paid_banner",
                "source": "umapyoi_news_paid",
                "card_type": "support",
                "gacha_id": 50228,
                "gacha_type": 11,
                "global_release_date": "2026-01-03T03:00:00Z",
                "estimated_end_date": "2026-01-10T03:00:00Z",
                "jp_release_date": "2022-01-03T03:00:00Z"
            },
            {
                "id": "umapyoi-gacha-50234",
                "type": "support_card_banner",
                "source": "umapyoi_news_support",
                "card_type": "support",
                "gacha_id": 50234,
                "gacha_type": 11,
                "global_release_date": "2026-01-05T03:00:00Z",
                "estimated_end_date": "2026-01-10T03:00:00Z",
                "jp_release_date": "2022-01-05T03:00:00Z"
            }
        ]});
        let links = timeline_gacha_links(&timeline);
        assert!(links[&50228].is_paid);
        // The 50xxx namespace remains a guard when news metadata itself was
        // classified as an ordinary support banner.
        assert!(links[&50234].is_paid);

        let gachas = BTreeMap::from([
            (
                30100,
                GachaAccumulator {
                    gacha_type: 3,
                    ..GachaAccumulator::default()
                },
            ),
            (
                50228,
                GachaAccumulator {
                    gacha_type: 11,
                    ..GachaAccumulator::default()
                },
            ),
            (
                50234,
                GachaAccumulator {
                    gacha_type: 11,
                    ..GachaAccumulator::default()
                },
            ),
        ]);
        let campaign = NewsFreePullCampaignClaim {
            banner_kinds: BTreeSet::from(["support"]),
            pulls_per_day: 10,
            entitlement_days: 4,
            total_pulls: 40,
            campaign_start: Some((1, 1, 12, 0)),
            has_explicit_total: true,
        };
        let campaign_start = 1_641_006_000;

        assert!(
            partition_news_free_pull_campaign_days(campaign_start, &campaign, &links, &gachas,)
                .is_none()
        );

        let mut without_paid_boundaries = links;
        without_paid_boundaries.remove(&50228);
        without_paid_boundaries.remove(&50234);
        assert_eq!(
            partition_news_free_pull_campaign_days(
                campaign_start,
                &campaign,
                &without_paid_boundaries,
                &gachas,
            ),
            Some(BTreeMap::from([(30100, 40)]))
        );
    }

    #[test]
    fn free_pull_parser_keeps_unrelated_character_gacha_out_of_support_campaign() {
        let mut text = String::from(
            "Up to 100 Free Gacha Pulls!\nFree 10-pull gacha once a day campaign\n8/24 12:00 - 9/3 4:59\nTarget Support Card Gacha\nSupport Card Gacha to commemorate the new scenario\n",
        );
        for _ in 0..14 {
            text.push_str("campaign precaution\n");
        }
        text.push_str("Paid Pretty Derby Gacha is also on sale");
        let claims = extract_news_free_pull_claims(&text);
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].banner_kind, Some("support"));
        assert_eq!(claims[0].pulls_per_day, 10);
        assert_eq!(claims[0].days, Some(10));
        assert_eq!(claims[0].campaign_start, Some((8, 24, 12, 0)));
    }

    #[test]
    fn parses_written_english_campaign_start_in_preview_posts() {
        assert_eq!(
            extract_month_day_time("The campaign begins at 12:00 on October 29, 2025 (Wednesday)."),
            Some((10, 29, 12, 0))
        );
        assert_eq!(
            extract_month_day_time("The campaign begins on June 27, 2025 (Friday) at 12:00."),
            Some((6, 27, 12, 0))
        );
    }

    #[test]
    fn archived_notices_expose_named_target_gacha_windows() {
        let archive = load_archive().unwrap();
        let windows = |post_id| {
            let post = archive
                .news
                .iter()
                .find(|post| post.post_id == post_id)
                .unwrap();
            extract_news_free_pull_target_windows(&archive_combined_text(post))
        };

        let anniversary = windows(901);
        assert!(anniversary.iter().any(|window| {
            window.banner_kind == "support"
                && window.start == (8, 24, 12, 0)
                && window.end == (8, 29, 11, 59)
        }));
        assert!(anniversary.iter().any(|window| {
            window.banner_kind == "support"
                && window.start == (8, 29, 12, 0)
                && window.end == (9, 3, 4, 59)
        }));

        let year_end = windows(1124);
        assert!(year_end.iter().any(|window| {
            window.banner_kind == "support"
                && window.start == (12, 29, 12, 0)
                && window.end == (1, 2, 11, 59)
        }));
        assert!(
            year_end
                .iter()
                .any(|window| window.banner_kind == "character"),
            "{year_end:#?}"
        );
    }

    #[test]
    fn cited_single_target_notices_pin_the_named_support_gacha() {
        let archive = load_archive().unwrap();
        for post_id in [1462, 1994, 2079, 2197, 2606, 2869] {
            let post = archive
                .news
                .iter()
                .find(|post| post.post_id == post_id)
                .unwrap();
            let combined = archive_combined_text(post);
            let windows = extract_news_free_pull_target_windows(&combined);
            let pinned = extract_news_free_pull_pinned_banner_kinds(&combined, &windows);
            assert!(pinned.contains("support"), "post {post_id}: {pinned:#?}");
        }
    }

    #[test]
    fn archived_anniversary_support_post_is_one_hundred_pulls_not_note_fragments() {
        let archive = load_archive().unwrap();
        let post = archive
            .news
            .iter()
            .find(|post| post.post_id == 901)
            .expect("archived 1.5 anniversary support campaign");
        let claims = extract_news_free_pull_claims(&archive_combined_text(post));

        assert_eq!(claims.len(), 1, "{claims:#?}");
        assert_eq!(claims[0].banner_kind, Some("support"));
        assert_eq!(claims[0].pulls_per_day, 10);
        assert_eq!(claims[0].days, Some(10));
        assert_eq!(claims[0].explicit_total, Some(100));
        assert_eq!(claims[0].campaign_start, Some((8, 24, 12, 0)));
    }

    #[test]
    fn archived_anniversary_campaign_emits_one_stock_aware_pool_with_detailed_source() {
        let mut archive = load_archive().unwrap();
        archive
            .news
            .retain(|post| matches!(post.post_id, 901 | 902));
        let links = BTreeMap::from([
            (
                30111,
                TimelineLink {
                    event_id: "support-banner-2022_30111".to_string(),
                    banner_kind: "support".to_string(),
                    start_date: "2026-08-01T00:00:00Z".to_string(),
                    end_date: "2026-08-10T00:00:00Z".to_string(),
                    jp_start_date: "2022-08-24T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
            (
                30113,
                TimelineLink {
                    event_id: "support-banner-2022_30113".to_string(),
                    banner_kind: "support".to_string(),
                    start_date: "2026-08-10T00:00:00Z".to_string(),
                    end_date: "2026-08-15T00:00:00Z".to_string(),
                    jp_start_date: "2022-08-29T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
        ]);
        let mut gachas = BTreeMap::from([
            (
                30111,
                GachaAccumulator {
                    gacha_type: 3,
                    ..GachaAccumulator::default()
                },
            ),
            (
                30113,
                GachaAccumulator {
                    gacha_type: 3,
                    ..GachaAccumulator::default()
                },
            ),
        ]);

        let campaigns = apply_news_free_pulls(&archive, &links, &mut gachas);

        assert_eq!(campaigns.len(), 1, "{campaigns:#?}");
        let campaign = &campaigns[0];
        assert_eq!(campaign.id, "jp-free-pulls-20220824-support-10x10");
        assert_eq!(campaign.total_pulls, 100);
        assert_eq!(campaign.pulls_per_day, 10);
        assert_eq!(campaign.entitlement_days, 10);
        assert_eq!(campaign.allocation_mode, "daily_with_one_time_stock");
        assert_eq!(campaign.eligible_gacha_ids, vec![30111, 30113]);
        assert_eq!(
            campaign.default_allocations,
            vec![
                PlannerFreePullAllocation {
                    event_id: "support-banner-2022_30111".to_string(),
                    gacha_id: 30111,
                    pulls: 60,
                },
                PlannerFreePullAllocation {
                    event_id: "support-banner-2022_30113".to_string(),
                    gacha_id: 30113,
                    pulls: 40,
                },
            ]
        );
        assert_eq!(campaign.source_url, "https://umapyoi.net/news/901?lang=jp");
        assert_eq!(gachas[&30111].free_pulls, 60);
        assert_eq!(gachas[&30113].free_pulls, 40);
        assert_eq!(
            gachas[&30111].campaign_id.as_deref(),
            Some("jp-free-pulls-20220824-support-10x10")
        );
    }

    #[test]
    fn archived_single_target_campaign_is_not_split_by_a_later_banner() {
        let mut archive = load_archive().unwrap();
        archive.news.retain(|post| post.post_id == 1462);
        let links = BTreeMap::from([
            (
                30187,
                TimelineLink {
                    event_id: "support-banner-2023_30187".to_string(),
                    banner_kind: "support".to_string(),
                    start_date: "2027-04-11T22:00:00Z".to_string(),
                    end_date: "2027-04-21T22:00:00Z".to_string(),
                    jp_start_date: "2023-08-24T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
            (
                30189,
                TimelineLink {
                    event_id: "support-banner-2023_30189".to_string(),
                    banner_kind: "support".to_string(),
                    start_date: "2027-04-18T22:00:00Z".to_string(),
                    end_date: "2027-04-28T22:00:00Z".to_string(),
                    jp_start_date: "2023-08-31T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
        ]);
        let mut gachas = BTreeMap::from([
            (
                30187,
                GachaAccumulator {
                    gacha_type: 3,
                    ..GachaAccumulator::default()
                },
            ),
            (
                30189,
                GachaAccumulator {
                    gacha_type: 3,
                    ..GachaAccumulator::default()
                },
            ),
        ]);

        let campaigns = apply_news_free_pulls(&archive, &links, &mut gachas);

        assert_eq!(campaigns.len(), 1, "{campaigns:#?}");
        assert_eq!(campaigns[0].total_pulls, 100);
        assert_eq!(
            campaigns[0].default_allocations,
            vec![PlannerFreePullAllocation {
                event_id: "support-banner-2023_30187".to_string(),
                gacha_id: 30187,
                pulls: 100,
            }]
        );
        assert_eq!(gachas[&30187].free_pulls, 100);
        assert_eq!(gachas[&30189].free_pulls, 0);
    }

    #[test]
    fn dedicated_notice_outranks_ambiguous_preview_claims() {
        let mut archive = load_archive().unwrap();
        archive
            .news
            .retain(|post| matches!(post.post_id, 2712 | 2727));
        let links = BTreeMap::from([
            (
                30363,
                TimelineLink {
                    event_id: "support-banner-2025_30363".to_string(),
                    banner_kind: "support".to_string(),
                    start_date: "2028-08-30T22:00:00Z".to_string(),
                    end_date: "2028-09-09T22:00:00Z".to_string(),
                    jp_start_date: "2025-08-24T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
            (
                30365,
                TimelineLink {
                    event_id: "support-banner-2025_30365".to_string(),
                    banner_kind: "support".to_string(),
                    start_date: "2028-09-04T22:00:00Z".to_string(),
                    end_date: "2028-09-14T22:00:00Z".to_string(),
                    jp_start_date: "2025-08-29T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
        ]);
        let mut gachas = BTreeMap::from([
            (
                30363,
                GachaAccumulator {
                    gacha_type: 3,
                    ..GachaAccumulator::default()
                },
            ),
            (
                30365,
                GachaAccumulator {
                    gacha_type: 3,
                    ..GachaAccumulator::default()
                },
            ),
        ]);

        let campaigns = apply_news_free_pulls(&archive, &links, &mut gachas);

        assert_eq!(campaigns.len(), 1, "{campaigns:#?}");
        assert_eq!(campaigns[0].total_pulls, 100);
        assert_eq!(campaigns[0].pulls_per_day, 10);
        assert_eq!(campaigns[0].entitlement_days, 10);
        assert_eq!(
            campaigns[0].source_url,
            "https://umapyoi.net/news/2712?lang=jp"
        );
        assert_eq!(gachas[&30363].free_pulls, 100);
        assert_eq!(gachas[&30365].free_pulls, 0);
    }

    #[test]
    fn archived_cross_kind_campaign_uses_notice_target_ranges() {
        let mut archive = load_archive().unwrap();
        archive.news.retain(|post| post.post_id == 1124);
        let links = BTreeMap::from([
            (
                30138,
                TimelineLink {
                    event_id: "banner-2022_30138".to_string(),
                    banner_kind: "character".to_string(),
                    start_date: "2026-12-29T03:00:00Z".to_string(),
                    end_date: "2027-01-08T03:00:00Z".to_string(),
                    jp_start_date: "2022-12-29T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
            (
                30139,
                TimelineLink {
                    event_id: "support-banner-2022_30139".to_string(),
                    banner_kind: "support".to_string(),
                    start_date: "2026-12-29T03:00:00Z".to_string(),
                    end_date: "2027-01-08T03:00:00Z".to_string(),
                    // Both gachas share the master-data start. Their distinct
                    // eligibility windows exist only in the detailed notice.
                    jp_start_date: "2022-12-29T03:00:00Z".to_string(),
                    pickup_card_ids: Vec::new(),
                    gacha_type: 3,
                    is_paid: false,
                },
            ),
        ]);
        let mut gachas = BTreeMap::from([
            (
                30138,
                GachaAccumulator {
                    gacha_type: 3,
                    ..GachaAccumulator::default()
                },
            ),
            (
                30139,
                GachaAccumulator {
                    gacha_type: 3,
                    ..GachaAccumulator::default()
                },
            ),
        ]);

        let campaigns = apply_news_free_pulls(&archive, &links, &mut gachas);

        assert_eq!(campaigns.len(), 1, "{campaigns:#?}");
        assert_eq!(campaigns[0].total_pulls, 100);
        assert_eq!(
            campaigns[0].default_allocations,
            vec![
                PlannerFreePullAllocation {
                    event_id: "banner-2022_30138".to_string(),
                    gacha_id: 30138,
                    pulls: 50,
                },
                PlannerFreePullAllocation {
                    event_id: "support-banner-2022_30139".to_string(),
                    gacha_id: 30139,
                    pulls: 50,
                },
            ]
        );
        assert_eq!(gachas[&30138].free_pulls, 50);
        assert_eq!(gachas[&30139].free_pulls, 50);
    }

    #[test]
    fn broad_year_end_summary_does_not_replace_the_dedicated_free_pull_source() {
        let mut archive = load_archive().unwrap();
        archive
            .news
            .retain(|post| matches!(post.post_id, 2297 | 2314));
        let links = BTreeMap::from([(
            1,
            TimelineLink {
                event_id: "support-banner-year-end-2024".to_string(),
                banner_kind: "support".to_string(),
                start_date: "2028-12-27T03:00:00Z".to_string(),
                end_date: "2029-01-06T03:00:00Z".to_string(),
                jp_start_date: "2024-12-27T03:00:00Z".to_string(),
                pickup_card_ids: Vec::new(),
                gacha_type: 3,
                is_paid: false,
            },
        )]);
        let mut gachas = BTreeMap::from([(
            1,
            GachaAccumulator {
                gacha_type: 3,
                ..GachaAccumulator::default()
            },
        )]);

        let campaigns = apply_news_free_pulls(&archive, &links, &mut gachas);

        assert_eq!(campaigns.len(), 1, "{campaigns:#?}");
        assert_eq!(campaigns[0].id, "jp-free-pulls-20241227-support-10x10");
        assert_eq!(
            campaigns[0].source_url,
            "https://umapyoi.net/news/2297?lang=jp"
        );
    }

    #[test]
    fn explicit_login_total_wins_over_unrelated_campaign_numbers() {
        let text = "Up to 100 free gacha pulls and up to 6000 jewels!\nThe gift expires after 30 days.\n1.5th Anniversary Login Bonus\nYou will receive 300 jewels per day and up to 3000 jewels depending on login days.\nRewards can be earned for up to 10 days.";
        let (daily, total, _) = extract_login_bonus_total(text).unwrap();
        assert_eq!((daily, total), (300, 3000));
    }

    #[test]
    fn adjacent_story_and_mission_numbers_are_not_jewel_amounts() {
        assert!(jewel_amounts_from_line(
            "Uma Musume story episodes 1-4 grant the first Jewel Reward"
        )
        .is_empty());
        assert!(jewel_amounts_from_line(
            "1.5th Anniversary Mission Part 4 grants rewards such as Jewels"
        )
        .is_empty());
        assert_eq!(jewel_amounts_from_line("Jewel 3000"), vec![3000]);
    }

    #[test]
    fn anniversary_news_links_every_detail_kind_to_global_planner_dates() {
        let archive = Archive {
            news: vec![ArchiveNews {
                post_id: 887,
                page_url: "https://umapyoi.net/news/887?lang=jp".to_string(),
                title: Some("1.5th Anniversary Campaign Vol.1".to_string()),
                posted_at: Some("2022-08-16T03:00:00Z".to_string()),
                event_types: vec!["campaign".to_string(), "gacha".to_string()],
                raw: json!({
                    "message_english": "1.5th Anniversary Countdown Login Bonus\nYou will receive 150 jewels per day and up to 1200 jewels depending on login days.\nOne Free Gacha Everyday Campaign\nPickup Pretty Derby Gacha\n8/16 12:00 - 8/24 11:59 (for 9 days)\nOne pull of the ongoing gacha is available once a day for free.\n1.5th Anniversary Commemorative Limited-time Mission Part 1\nClear the limited-time missions to earn rewards such as Jewels and an Umamusume gacha ticket."
                }),
            }],
        };
        let timeline = json!({"events":[{
            "id":"campaign-217", "type":"campaign",
            "title":"1.5th Anniversary Campaign Vol.1",
            "jp_release_date":"2022-08-16T03:00:00Z",
            "global_release_date":"2026-08-26T22:00:00Z",
            "estimated_end_date":"2026-10-09T22:00:00Z",
            "umapyoi_url":"https://umapyoi.net/news/887?lang=jp"
        }]});

        let link = match_news_event(
            &archive.news[0],
            archive.news[0].title.as_deref().unwrap(),
            &timeline,
        )
        .unwrap();
        assert_eq!(link.event_id, "campaign-217");

        let rewards = load_news_rewards(&archive, &timeline);
        let login = rewards
            .iter()
            .find(|reward| reward.id == "news-887-login-bonus")
            .unwrap();
        assert_eq!(login.event_id.as_deref(), Some("campaign-217"));
        assert_eq!(login.amount, Some(1200));
        assert_eq!(login.available_at, "2026-09-02T22:00:00+00:00");
        let mission = rewards
            .iter()
            .find(|reward| reward.id.starts_with("news-887-section-") && reward.amount.is_none())
            .unwrap();
        assert_eq!(mission.event_id.as_deref(), Some("campaign-217"));
        assert_eq!(mission.available_at, "2026-10-09T22:00:00Z");

        let details = load_news_details(&archive, &timeline);
        assert_eq!(details.len(), 3);
        for detail in &details {
            assert_eq!(detail.event_id.as_deref(), Some("campaign-217"));
        }
        assert_eq!(
            details
                .iter()
                .find(|detail| detail.kind == "login_bonus")
                .unwrap()
                .available_at,
            "2026-09-02T22:00:00+00:00"
        );
        assert_eq!(
            details
                .iter()
                .find(|detail| detail.kind == "free_pulls")
                .unwrap()
                .available_at,
            "2026-09-03T22:00:00+00:00"
        );
        assert_eq!(
            details
                .iter()
                .find(|detail| detail.kind == "limited_missions")
                .unwrap()
                .available_at,
            "2026-10-09T22:00:00Z"
        );
        assert!(details
            .iter()
            .find(|detail| detail.kind == "login_bonus")
            .unwrap()
            .summary
            .contains("1200"));
    }

    #[test]
    fn ambiguous_campaign_news_match_stays_unlinked() {
        let post = ArchiveNews {
            post_id: 1,
            page_url: "https://umapyoi.net/news/1".to_string(),
            title: Some("Anniversary Campaign".to_string()),
            posted_at: Some("2022-08-16T03:00:00Z".to_string()),
            event_types: vec!["campaign".to_string()],
            raw: json!({"message_english":"Limited-time mission rewards"}),
        };
        let timeline = json!({"events":[
            {
                "id":"campaign-a", "type":"campaign", "title":"Anniversary Campaign",
                "jp_release_date":"2022-08-16T03:00:00Z",
                "global_release_date":"2026-08-01T00:00:00Z",
                "estimated_end_date":"2026-09-01T00:00:00Z"
            },
            {
                "id":"campaign-b", "type":"campaign", "title":"Anniversary Campaign",
                "jp_release_date":"2022-08-16T03:00:00Z",
                "global_release_date":"2026-08-02T00:00:00Z",
                "estimated_end_date":"2026-09-02T00:00:00Z"
            }
        ]});
        assert!(match_news_event(&post, "Anniversary Campaign", &timeline).is_none());
    }

    #[test]
    fn bundled_news_regressions_do_not_emit_adjacent_numbers_as_jewels() {
        let archive = load_archive().unwrap();
        let rewards = load_news_rewards(&archive, &json!({"events":[]}));
        let amounts_for = |post_id: i64| {
            rewards
                .iter()
                .filter(|reward| reward.id.starts_with(&format!("news-{post_id}-")))
                .filter_map(|reward| reward.amount)
                .collect::<Vec<_>>()
        };
        assert!(amounts_for(877).is_empty());
        assert!(amounts_for(917).is_empty());
        assert_eq!(amounts_for(230), vec![150, 150]);
        assert_eq!(amounts_for(319), vec![150]);
        let post_902 = amounts_for(902);
        assert!(
            !post_902
                .iter()
                .any(|amount| [1, 300, 9000].contains(amount)),
            "unexpected post 902 amounts: {post_902:?}"
        );
        assert_eq!(
            rewards
                .iter()
                .find(|reward| reward.id == "news-902-login-bonus")
                .and_then(|reward| reward.amount),
            Some(3000)
        );
    }

    #[test]
    fn bundled_post_887_has_one_exact_character_free_pull_claim() {
        let archive = load_archive().unwrap();
        let post = archive
            .news
            .iter()
            .find(|post| post.post_id == 887)
            .unwrap();
        let claims = extract_news_free_pull_claims(&archive_combined_text(post));
        assert_eq!(claims.len(), 1, "claims: {claims:#?}");
        assert_eq!(claims[0].banner_kind, Some("character"));
        assert_eq!(claims[0].pulls_per_day, 1);
        assert_eq!(claims[0].days, Some(9));
        assert_eq!(claims[0].campaign_start, Some((8, 16, 12, 0)));
    }

    #[test]
    fn bundled_post_1203_preview_has_no_quantitative_free_pull_claim() {
        let archive = load_archive().unwrap();
        let post = archive
            .news
            .iter()
            .find(|post| post.post_id == 1203)
            .unwrap();
        let claims = extract_news_free_pull_claims(&archive_combined_text(post));
        assert!(
            claims.iter().all(|claim| claim.days.is_none()),
            "preview claims must remain qualitative: {claims:#?}"
        );
    }

    #[test]
    fn bundled_post_2998_preview_does_not_invent_support_pull_days() {
        let archive = load_archive().unwrap();
        let post = archive
            .news
            .iter()
            .find(|post| post.post_id == 2998)
            .unwrap();
        let claims = extract_news_free_pull_claims(&archive_combined_text(post));
        assert!(
            claims.iter().all(|claim| claim.days.is_none()),
            "preview ranges without an explicit entitlement count stay qualitative: {claims:#?}"
        );
    }

    #[test]
    fn preserves_all_limited_mission_items_and_derives_planner_currencies() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
            CREATE TABLE mission_data (
                id INTEGER, mission_type INTEGER, event_id INTEGER,
                start_date TEXT, end_date TEXT, item_category INTEGER,
                item_id INTEGER, item_num INTEGER
            );
            INSERT INTO mission_data VALUES
                (1,4,4,'2025/08/01 00:00:00','2025/08/10 00:00:00',90,43,300),
                (2,4,4,'2025/08/01 00:00:00','2025/08/10 00:00:00',40,41,1),
                (3,4,4,'2025/08/01 00:00:00','2025/08/10 00:00:00',91,59,100),
                (4,4,4,'2025/08/01 00:00:00','2025/08/10 00:00:00',164,149,1),
                (5,4,4,'2025/08/01 00:00:00','2025/08/10 00:00:00',164,150,1);
        "#,
            )
            .unwrap();
        let timeline = json!({"events":[{
            "id":"campaign-4", "type":"mission_campaign",
            "global_release_date":"2025-08-01T00:00:00+00:00",
            "estimated_end_date":"2025-08-10T00:00:00+00:00"
        }]});
        let mut rewards = Vec::new();
        load_mission_campaign_rewards(&connection, &timeline, &mut rewards).unwrap();
        assert!(rewards
            .iter()
            .any(|reward| reward.currency == "free_jewels" && reward.amount == Some(300)));
        assert!(rewards
            .iter()
            .any(|reward| reward.currency == "uma_ticket" && reward.amount == Some(1)));
        assert!(rewards
            .iter()
            .any(|reward| { reward.currency == "rainbow_crystal" && reward.amount == Some(1) }));
        assert!(rewards
            .iter()
            .any(|reward| reward.currency == "gold_crystal" && reward.amount == Some(1)));
        let details = rewards
            .iter()
            .find(|reward| reward.id.ends_with("-items"))
            .unwrap();
        assert_eq!(details.source_items.len(), 1);
        assert_eq!(details.source_items[0].item_id, 59);
        assert!(!details.default_enabled);
    }

    #[test]
    fn bundled_jp_snapshot_maps_anniversary_missions_to_timeline_campaigns() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
            CREATE TABLE mission_data (
                id INTEGER, mission_type INTEGER, event_id INTEGER,
                start_date TEXT, end_date TEXT, item_category INTEGER,
                item_id INTEGER, item_num INTEGER
            );
        "#,
            )
            .unwrap();
        let timeline = json!({"events":[
            {"id":"campaign-217","type":"mission_campaign","global_release_date":"2026-08-01T00:00:00Z","estimated_end_date":"2026-10-09T22:00:00Z"},
            {"id":"campaign-218","type":"mission_campaign","global_release_date":"2026-08-10T00:00:00Z","estimated_end_date":"2026-10-10T22:00:00Z"}
        ]});
        let mut rewards = Vec::new();
        load_mission_campaign_rewards(&connection, &timeline, &mut rewards).unwrap();
        for campaign in [217, 218] {
            let expected_event_id = format!("campaign-{campaign}");
            let reward = rewards
                .iter()
                .find(|reward| reward.id == format!("jp-master-mission-{campaign}-free_jewels"))
                .unwrap();
            assert_eq!(reward.event_id.as_deref(), Some(expected_event_id.as_str()));
            assert_eq!(reward.amount, Some(500));
            assert!(reward
                .evidence
                .as_deref()
                .unwrap()
                .contains("15 JP master mission rows"));
            assert_eq!(reward.provenance, "jp_master_snapshot");
            assert!(!reward.default_enabled);
        }
    }

    #[test]
    fn incomplete_historical_snapshot_is_not_emitted_as_exact_campaign_rewards() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
            CREATE TABLE mission_data (
                id INTEGER, mission_type INTEGER, event_id INTEGER,
                start_date TEXT, end_date TEXT, item_category INTEGER,
                item_id INTEGER, item_num INTEGER
            );
        "#,
            )
            .unwrap();
        let timeline = json!({"events":[{
            "id":"campaign-28", "type":"campaign", "source":"campaign",
            "global_release_date":"2027-01-01T00:00:00Z",
            "estimated_end_date":"2049-12-31T19:59:59Z"
        }]});
        let mut rewards = Vec::new();
        load_mission_campaign_rewards(&connection, &timeline, &mut rewards).unwrap();
        assert!(!rewards
            .iter()
            .any(|reward| reward.id.starts_with("jp-master-mission-28-")));
    }

    #[test]
    fn global_campaign_uses_exact_id_and_suppresses_same_jp_snapshot() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
            CREATE TABLE mission_data (
                id INTEGER, mission_type INTEGER, event_id INTEGER,
                start_date TEXT, end_date TEXT, item_category INTEGER,
                item_id INTEGER, item_num INTEGER
            );
            INSERT INTO mission_data VALUES
                (1,4,217,'2026/08/26 22:00:00','2026/10/09 22:00:00',90,43,500);
        "#,
            )
            .unwrap();
        let timeline = json!({"events":[
            {
                "id":"campaign-217", "type":"campaign", "source":"campaign",
                "tags":["mission campaign"],
                "global_release_date":"2026-08-26T22:00:00Z",
                "estimated_end_date":"2026-10-09T22:00:00Z"
            },
            {
                "id":"campaign-999", "type":"campaign", "source":"campaign",
                "tags":["mission campaign"],
                "global_release_date":"2026-08-26T22:00:00Z",
                "estimated_end_date":"2026-10-10T22:00:00Z"
            }
        ]});
        let mut rewards = Vec::new();
        load_mission_campaign_rewards(&connection, &timeline, &mut rewards).unwrap();
        let global = rewards
            .iter()
            .find(|reward| reward.id == "mission-campaign-217-free_jewels")
            .unwrap();
        assert_eq!(global.event_id.as_deref(), Some("campaign-217"));
        assert_eq!(global.amount, Some(500));
        assert!(!rewards
            .iter()
            .any(|reward| reward.id.starts_with("jp-master-mission-217-")));
    }

    #[test]
    fn duplicate_campaign_start_without_exact_id_stays_unlinked() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
            CREATE TABLE mission_data (
                id INTEGER, mission_type INTEGER, event_id INTEGER,
                start_date TEXT, end_date TEXT, item_category INTEGER,
                item_id INTEGER, item_num INTEGER
            );
            INSERT INTO mission_data VALUES
                (1,4,9999,'2026/08/26 22:00:00','2026/09/01 22:00:00',90,43,100);
        "#,
            )
            .unwrap();
        let timeline = json!({"events":[
            {
                "id":"campaign-217", "type":"campaign", "source":"campaign",
                "global_release_date":"2026-08-26T22:00:00Z",
                "estimated_end_date":"2026-10-09T22:00:00Z"
            },
            {
                "id":"campaign-999", "type":"campaign", "source":"campaign",
                "global_release_date":"2026-08-26T22:00:00Z",
                "estimated_end_date":"2026-10-10T22:00:00Z"
            }
        ]});
        let mut rewards = Vec::new();
        load_mission_campaign_rewards(&connection, &timeline, &mut rewards).unwrap();
        let reward = rewards
            .iter()
            .find(|reward| reward.id == "mission-campaign-9999-free_jewels")
            .unwrap();
        assert_eq!(reward.event_id, None);
    }

    #[test]
    fn extracts_daily_pack_once_across_platforms() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE daily_pack (
                    shop_data_id INTEGER NOT NULL,
                    platform_id INTEGER NOT NULL,
                    term INTEGER NOT NULL,
                    daily_free_num INTEGER NOT NULL,
                    start_date TEXT NOT NULL,
                    end_date TEXT NOT NULL
                );
                INSERT INTO daily_pack VALUES
                    (16, 1, 30, 50, '2017-01-01 12:00:00', '2030-01-10 00:00:00'),
                    (16, 2, 30, 50, '2017-01-01 12:00:00', '2030-01-10 00:00:00');
                "#,
            )
            .unwrap();
        let mut rules = Vec::new();
        load_daily_pack_rules(&connection, &mut rules).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].amount, 50);
        assert_eq!(rules[0].category, Some("paid"));
        assert!(!rules[0].default_enabled);
    }

    #[test]
    fn extracts_premium_training_pass_paid_jewels() {
        let archive = Archive {
            news: vec![ArchiveNews {
                post_id: 1,
                page_url: "https://umapyoi.net/news/1".to_string(),
                title: Some("Training Pass Updated!".to_string()),
                posted_at: Some("2026-06-29T03:00:00+00:00".to_string()),
                event_types: Vec::new(),
                raw: json!({
                    "message_english": "Purchasing the Premium Pass grants you 350 paid Jewels."
                }),
            }],
        };
        let rules = load_paid_news_income_rules(&archive);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].currency, "paid_jewels");
        assert_eq!(rules[0].amount, 350);
        assert_eq!(rules[0].provenance, "jp_fallback");
    }

    #[test]
    fn retains_champions_outcomes_as_non_counting_metadata() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE champions_schedule (
                    id INTEGER NOT NULL,
                    start_date INTEGER NOT NULL
                );
                CREATE TABLE champions_reward_rate (
                    id INTEGER NOT NULL,
                    champions_id INTEGER NOT NULL
                );
                INSERT INTO champions_schedule VALUES (1, 1755468000);
                INSERT INTO champions_reward_rate VALUES (1, 1), (2, 1);
                "#,
            )
            .unwrap();
        let timeline = json!({
            "events": [{
                "id": "champions-meeting-0",
                "type": "champions_meeting",
                "global_release_date": "2025-08-17T22:00:00Z",
                "estimated_end_date": "2025-08-24T22:00:00Z"
            }]
        });
        let mut rewards = Vec::new();
        load_competitive_reward_metadata(&connection, &timeline, &mut rewards).unwrap();
        assert_eq!(rewards.len(), 1);
        assert_eq!(rewards[0].event_id.as_deref(), Some("champions-meeting-0"));
        assert_eq!(rewards[0].amount, None);
        assert!(!rewards[0].default_enabled);
    }

    #[test]
    fn exposes_exact_champions_reward_set_rows_as_opt_in_variants() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(r#"
            CREATE TABLE champions_schedule (id INTEGER, start_date INTEGER);
            CREATE TABLE champions_reward_rate (
                id INTEGER, champions_id INTEGER, league_type INTEGER, round_id INTEGER,
                win_count INTEGER, ranking INTEGER, rate INTEGER, reward_set_id INTEGER
            );
            CREATE TABLE single_mode_reward_set (
                id INTEGER, reward_set_id INTEGER, order_min INTEGER, order_max INTEGER,
                reward_type INTEGER, bonus INTEGER, odds INTEGER,
                item_category INTEGER, item_id INTEGER, item_num INTEGER
            );
            CREATE TABLE heroes_data (heroes_id INTEGER, start_date INTEGER);
            CREATE TABLE heroes_league_rank (
                id INTEGER, league_rank_type INTEGER, league_rank INTEGER,
                league_min_value INTEGER, league_max_value INTEGER, reward_group_id INTEGER
            );
            CREATE TABLE heroes_league_rank_reward_group (
                id INTEGER, reward_group_id INTEGER, item_category INTEGER, item_id INTEGER, item_num INTEGER
            );
            INSERT INTO champions_schedule VALUES (1,1755468000);
            INSERT INTO champions_reward_rate VALUES (7,1,2,3,4,5,10000,101);
            INSERT INTO single_mode_reward_set VALUES (1,101,1,1,2,0,1000000,90,43,50);
        "#).unwrap();
        let timeline = json!({"events":[{
            "id":"champions-meeting-0", "type":"champions_meeting",
            "global_release_date":"2025-08-17T22:00:00Z",
            "estimated_end_date":"2025-08-24T22:00:00Z"
        }]});
        let variants = load_competitive_variants(&connection, &timeline).unwrap();
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].source_items[0].amount, 50);
        assert_eq!(variants[0].source_items[0].odds, Some(1_000_000));
        assert!(!variants[0].default_enabled);
    }
    #[test]
    fn projects_latest_confirmed_competition_rewards_to_missing_future_events() {
        let timeline = json!({"events": [
            {"id":"legend-old","type":"legend_race","global_release_date":"2025-01-01T00:00:00Z","is_confirmed":true},
            {"id":"legend-latest","type":"legend_race","global_release_date":"2025-02-01T00:00:00Z","is_confirmed":true},
            {"id":"champions-past","type":"champions_meeting","global_release_date":"2025-01-01T00:00:00Z","is_confirmed":true},
            {"id":"heroes-past","type":"league_of_heroes","global_release_date":"2025-01-01T00:00:00Z","is_confirmed":true},
            {"id":"team-past","type":"strongest_team","global_release_date":"2025-01-01T00:00:00Z","is_confirmed":true},
            {"id":"legend-future","type":"legend_race","global_release_date":"2026-01-01T00:00:00Z","is_confirmed":false},
            {"id":"champions-future","type":"champions_meeting","global_release_date":"2026-01-01T00:00:00Z","is_confirmed":false},
            {"id":"heroes-future","type":"league_of_heroes","global_release_date":"2026-01-01T00:00:00Z","is_confirmed":false},
            {"id":"team-future","type":"strongest_team","global_release_date":"2026-01-01T00:00:00Z","is_confirmed":false},
            {"id":"legend-exact","type":"legend_race","global_release_date":"2026-02-01T00:00:00Z","is_confirmed":false}
        ]});
        let variant =
            |id: &str, competition: &'static str, event_id: &str, master_event_id, amount| {
                super::PlannerCompetitiveVariant {
                    id: id.to_string(),
                    competition,
                    event_id: event_id.to_string(),
                    master_event_id,
                    available_at: None,
                    label: "Published reward table".to_string(),
                    source_items: vec![PlannerSourceItem {
                        item_category: 90,
                        item_id: 43,
                        amount,
                        mission_count: None,
                        odds: None,
                        order_min: None,
                        order_max: None,
                        bonus: None,
                    }],
                    provenance: "global_master",
                    confidence: "exact_variant",
                    default_enabled: false,
                }
            };
        let mut variants = vec![
            variant("legend-old", "legend_race", "legend-old", 1, 100),
            variant("legend-latest", "legend_race", "legend-latest", 2, 150),
            variant("champions", "champions_meeting", "champions-past", 3, 200),
            variant("heroes", "league_of_heroes", "heroes-past", 4, 300),
            variant("team", "strongest_team", "team-past", 5, 400),
            variant("legend-exact", "legend_race", "legend-exact", 6, 500),
        ];

        super::project_missing_competitive_variants(&timeline, &mut variants);

        assert_eq!(variants.len(), 10);
        for event_id in [
            "legend-future",
            "champions-future",
            "heroes-future",
            "team-future",
        ] {
            let projected = variants
                .iter()
                .find(|variant| variant.event_id == event_id)
                .unwrap();
            assert_eq!(projected.provenance, "global_reward_parity");
            assert_eq!(projected.confidence, "projected_global_parity");
        }
        let legend = variants
            .iter()
            .find(|variant| variant.event_id == "legend-future")
            .unwrap();
        assert_eq!(legend.master_event_id, 2);
        assert_eq!(legend.source_items[0].amount, 150);
        assert_eq!(
            variants
                .iter()
                .filter(|variant| variant.event_id == "legend-exact")
                .count(),
            1
        );
    }
    #[test]
    fn projects_builtin_competition_rewards_without_confirmed_global_templates() {
        let timeline = json!({"events": [
            {"id":"champions-future","type":"champions_meeting","global_release_date":"2026-01-01T00:00:00Z","is_confirmed":false},
            {"id":"heroes-future","type":"league_of_heroes","global_release_date":"2026-01-02T00:00:00Z","is_confirmed":false},
            {"id":"team-future","type":"strongest_team","global_release_date":"2026-01-03T00:00:00Z","is_confirmed":false},
            {"id":"legend-future","type":"legend_race","global_release_date":"2026-01-04T00:00:00Z","is_confirmed":false}
        ]});
        let mut variants = Vec::new();

        super::project_missing_competitive_variants(&timeline, &mut variants);

        for (event_id, competition) in [
            ("champions-future", "champions_meeting"),
            ("heroes-future", "league_of_heroes"),
            ("team-future", "strongest_team"),
            ("legend-future", "legend_race"),
        ] {
            let projected = variants
                .iter()
                .filter(|variant| variant.event_id == event_id)
                .collect::<Vec<_>>();
            assert_eq!(
                projected.len(),
                super::projected_competition_templates(competition).len()
            );
            assert!(projected.iter().all(|variant| {
                variant.provenance == "jp_reward_parity_template"
                    && variant.confidence == "projected_jp_parity"
            }));
        }
        assert!(variants.iter().any(|variant| {
            variant.event_id == "heroes-future"
                && variant.source_items.iter().any(|item| {
                    item.item_category == 90 && item.item_id == 43 && item.amount == 500
                })
                && variant.source_items.iter().any(|item| {
                    item.item_category == 164 && item.item_id == 149 && item.amount == 1
                })
        }));
        assert!(variants.iter().any(|variant| {
            variant.event_id == "team-future"
                && variant.label == "Event missions (full completion)"
                && variant.source_items.iter().any(|item| {
                    item.item_category == 90 && item.item_id == 43 && item.amount == 500
                })
                && variant.source_items.iter().any(|item| {
                    item.item_category == 164 && item.item_id == 150 && item.amount == 1
                })
        }));
        assert!(variants.iter().any(|variant| {
            variant.event_id == "legend-future"
                && variant.source_items.iter().any(|item| {
                    item.item_category == 90 && item.item_id == 43 && item.amount == 150
                })
        }));
    }
    #[test]
    fn projects_latest_confirmed_story_reward_to_missing_future_events() {
        let timeline = json!({"events": [
            {"id":"story-past","type":"story_event","global_release_date":"2026-01-01T00:00:00Z","is_confirmed":true},
            {"id":"story-future","type":"story_event","global_release_date":"2026-02-01T00:00:00Z","is_confirmed":false},
            {"id":"story-exact","type":"story_event","global_release_date":"2026-03-01T00:00:00Z","is_confirmed":false}
        ]});
        let reward = |id: &str, event_id: &str, amount| PlannerReward {
            id: id.to_string(),
            label: "Story event rewards".to_string(),
            event_id: Some(event_id.to_string()),
            gacha_id: None,
            currency: "free_jewels",
            amount: Some(amount),
            available_at: "2026-01-01T00:00:00Z".to_string(),
            provenance: "global_master",
            assumption: "full_completion",
            default_enabled: true,
            source_url: None,
            source_items: vec![PlannerSourceItem {
                item_category: 90,
                item_id: 43,
                amount,
                mission_count: None,
                odds: None,
                order_min: None,
                order_max: None,
                bonus: None,
            }],
            confidence: "exact_source",
            evidence: None,
        };
        let mut rewards = vec![
            reward("story-past-rewards", "story-past", 2010),
            reward("story-exact-rewards", "story-exact", 999),
        ];

        super::project_missing_story_rewards(&timeline, &mut rewards);

        assert_eq!(rewards.len(), 3);
        let projected = rewards
            .iter()
            .find(|reward| reward.event_id.as_deref() == Some("story-future"))
            .unwrap();
        assert_eq!(projected.amount, Some(2010));
        assert_eq!(projected.available_at, "2026-02-01T00:00:00Z");
        assert_eq!(projected.provenance, "global_story_reward_parity");
        assert_eq!(projected.confidence, "projected_global_parity");
        assert_eq!(
            rewards
                .iter()
                .filter(|reward| reward.event_id.as_deref() == Some("story-exact"))
                .count(),
            1
        );
    }

    #[test]
    fn projects_exact_global_exchange_stock_to_future_matching_events() {
        let timeline = json!({"events": [
            {
                "id":"skills-confirmed",
                "type":"trainer_skills_test",
                "global_release_date":"2026-01-01T00:00:00Z",
                "estimated_end_date":"2026-01-08T00:00:00Z",
                "is_confirmed":true
            },
            {
                "id":"skills-future",
                "type":"trainer_skills_test",
                "global_release_date":"2026-02-01T00:00:00Z",
                "estimated_end_date":"2026-02-08T00:00:00Z",
                "is_confirmed":false
            }
        ]});
        let reward = |currency, item_id| PlannerReward {
            id: format!("exact-{currency}"),
            label: "Trainer Skills Test exchange rewards".to_string(),
            event_id: None,
            gacha_id: None,
            currency,
            amount: Some(3),
            available_at: "2026-01-08T00:00:00Z".to_string(),
            provenance: "global_master",
            assumption: "full_exchange",
            default_enabled: true,
            source_url: None,
            source_items: vec![PlannerSourceItem {
                item_category: 40,
                item_id,
                amount: 3,
                mission_count: None,
                odds: None,
                order_min: None,
                order_max: None,
                bonus: None,
            }],
            confidence: "exact_source",
            evidence: None,
        };
        let mut rewards = vec![reward("uma_ticket", 41), reward("support_ticket", 111)];

        super::project_missing_event_exchange_rewards(
            &timeline,
            &mut rewards,
            "trainer_skills_test",
            "Trainer Skills Test exchange rewards",
            "Trainer Skills Test exchange rewards (projected Global parity)",
        );

        let projected = rewards
            .iter()
            .filter(|reward| reward.event_id.as_deref() == Some("skills-future"))
            .collect::<Vec<_>>();
        assert_eq!(projected.len(), 2);
        assert!(projected.iter().all(|reward| {
            reward.amount == Some(3)
                && reward.available_at == "2026-02-08T00:00:00Z"
                && reward.provenance == "global_exchange_reward_parity"
                && reward.confidence == "projected_global_parity"
        }));
        assert!(!rewards
            .iter()
            .any(|reward| reward.event_id.as_deref() == Some("skills-confirmed")));
    }

    #[test]
    fn legend_races_without_manual_timeline_rows_include_first_clear_rewards() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(r#"
            CREATE TABLE champions_schedule (id INTEGER, start_date INTEGER);
            CREATE TABLE champions_reward_rate (
                id INTEGER, champions_id INTEGER, league_type INTEGER, round_id INTEGER,
                win_count INTEGER, ranking INTEGER, rate INTEGER, reward_set_id INTEGER
            );
            CREATE TABLE single_mode_reward_set (
                id INTEGER, reward_set_id INTEGER, order_min INTEGER, order_max INTEGER,
                reward_type INTEGER, bonus INTEGER, odds INTEGER,
                item_category INTEGER, item_id INTEGER, item_num INTEGER
            );
            CREATE TABLE heroes_data (heroes_id INTEGER, start_date INTEGER);
            CREATE TABLE heroes_league_rank (
                id INTEGER, league_rank_type INTEGER, league_rank INTEGER,
                league_min_value INTEGER, league_max_value INTEGER, reward_group_id INTEGER
            );
            CREATE TABLE heroes_league_rank_reward_group (
                id INTEGER, reward_group_id INTEGER, item_category INTEGER, item_id INTEGER, item_num INTEGER
            );
            CREATE TABLE legend_race (
                id INTEGER, start_date INTEGER, image_id INTEGER, race_instance_id INTEGER,
                first_clear_item_category_1 INTEGER, first_clear_item_id_1 INTEGER, first_clear_item_num_1 INTEGER,
                first_clear_item_category_2 INTEGER, first_clear_item_id_2 INTEGER, first_clear_item_num_2 INTEGER,
                first_clear_item_category_3 INTEGER, first_clear_item_id_3 INTEGER, first_clear_item_num_3 INTEGER
            );
            INSERT INTO legend_race VALUES
                (130,1893456000,1130,1204201,90,43,150,102,1130,10,0,0,0),
                (131,1893715200,1131,1204202,90,43,150,102,1131,10,0,0,0);
        "#).unwrap();
        let timeline = json!({"events":[{
            "id":"nearby-campaign", "type":"campaign",
            "global_release_date":"2030-01-01T00:00:00Z",
            "jp_release_date":"2030-01-01T00:00:00Z",
            "estimated_end_date":"2030-01-10T00:00:00Z"
        }]});

        let variants = load_competitive_variants_for(
            &connection,
            &timeline,
            "jp_release_date",
            "jp_master",
            "projected_parity_variant",
        )
        .unwrap();

        assert_eq!(variants.len(), 2);
        assert!(variants.iter().all(|variant| {
            variant.event_id == "master-legend-race-120420"
                && variant.available_at.as_deref() == Some("2030-01-10T00:00:00Z")
        }));
        let mut carat_amounts = variants
            .iter()
            .map(|variant| {
                variant
                    .source_items
                    .iter()
                    .find(|item| item.item_category == 90 && item.item_id == 43)
                    .map(|item| item.amount)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        carat_amounts.sort_unstable();
        assert_eq!(carat_amounts, vec![150, 150]);
    }

    #[test]
    fn temporary_story_rewards_count_only_the_first_unlock_for_each_trainee() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
            CREATE TABLE campaign_chara_story_schedule (
                campaign_id INTEGER, chara_id INTEGER, story_id INTEGER
            );
            CREATE TABLE campaign_data (
                campaign_id INTEGER, start_time INTEGER, end_time INTEGER
            );
            CREATE TABLE chara_story_data (
                story_id INTEGER, episode_index INTEGER,
                add_reward_category_1 INTEGER, add_reward_id_1 INTEGER,
                add_reward_num_1 INTEGER
            );
            INSERT INTO campaign_data VALUES
                (10,1893456000,1893542400),
                (20,1896048000,1896134400);
            INSERT INTO campaign_chara_story_schedule VALUES
                (10,1001,1),(10,1001,2),(10,1001,3),(10,1001,4),
                (20,1001,1),(20,1001,2),(20,1001,3),(20,1001,4);
            INSERT INTO chara_story_data VALUES
                (1,1,90,43,20),(2,2,90,43,20),
                (3,3,90,43,20),(4,4,90,43,20);
        "#,
            )
            .unwrap();
        let timeline = json!({"events":[{
            "id":"trainee-release", "type":"campaign",
            "global_release_date":"2030-01-01T00:00:00Z",
            "estimated_end_date":"2030-01-02T00:00:00Z"
        }]});
        let mut rewards = Vec::new();
        let mut seen_chara_ids = BTreeSet::new();

        load_temporary_character_story_rewards_for(
            &connection,
            &timeline,
            &mut rewards,
            "global_release_date",
            "global_master",
            "exact_source",
            &mut seen_chara_ids,
        )
        .unwrap();

        assert_eq!(rewards.len(), 1);
        assert_eq!(rewards[0].amount, Some(80));
        assert_eq!(rewards[0].assumption, "temporary_character_story_read");
        assert_eq!(seen_chara_ids, BTreeSet::from([1001]));
    }

    #[test]
    fn factor_research_multiplies_each_reward_by_its_box_count() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
            CREATE TABLE factor_research_data (
                factor_research_event_id INTEGER, start_date INTEGER, end_date INTEGER
            );
            CREATE TABLE factor_research_box (
                factor_research_event_id INTEGER, box_id INTEGER
            );
            CREATE TABLE factor_research_box_reward (
                box_id INTEGER, item_category INTEGER, item_id INTEGER,
                item_num INTEGER, box_num INTEGER
            );
            INSERT INTO factor_research_data VALUES (1,1893456000,1893542400);
            INSERT INTO factor_research_box VALUES (1,10);
            INSERT INTO factor_research_box_reward VALUES (10,90,43,300,2);
        "#,
            )
            .unwrap();
        let timeline = json!({"events":[{
            "id":"factor-research-test", "type":"factor_research",
            "global_release_date":"2030-01-01T00:00:00Z",
            "estimated_end_date":"2030-01-02T00:00:00Z"
        }]});
        let mut rewards = Vec::new();

        load_factor_research_rewards_for(
            &connection,
            &timeline,
            &mut rewards,
            "global_release_date",
            "global_master",
            "exact_source",
        )
        .unwrap();

        assert_eq!(rewards.len(), 1);
        assert_eq!(rewards[0].amount, Some(600));
        assert_eq!(rewards[0].event_id.as_deref(), Some("factor-research-test"));
    }

    #[test]
    fn story_rewards_expose_components_tickets_crystals_and_only_finite_bingo() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE story_event_point_reward (
                    story_event_id INTEGER, item_category INTEGER, item_id INTEGER, item_num INTEGER
                );
                CREATE TABLE story_event_mission (
                    story_event_id INTEGER, item_category INTEGER, item_id INTEGER, item_num INTEGER
                );
                CREATE TABLE story_event_story_data (
                    story_event_id INTEGER,
                    add_reward_category_1 INTEGER, add_reward_id_1 INTEGER, add_reward_num_1 INTEGER,
                    add_reward_category_2 INTEGER, add_reward_id_2 INTEGER, add_reward_num_2 INTEGER
                );
                CREATE TABLE story_event_roulette_bingo (
                    story_event_id INTEGER, reward_set_id INTEGER, can_loop INTEGER
                );
                CREATE TABLE story_event_bingo_reward (
                    reward_set_id INTEGER, item_category INTEGER, item_id INTEGER, item_num INTEGER
                );
                CREATE TABLE story_event_data (
                    story_event_id INTEGER, start_date INTEGER, end_date INTEGER
                );
                INSERT INTO story_event_point_reward VALUES
                    (1001, 90, 43, 1500),
                    (1001, 40, 41, 2),
                    (1001, 40, 111, 2),
                    (1001, 164, 149, 3);
                INSERT INTO story_event_mission VALUES (1001, 90, 43, 150);
                INSERT INTO story_event_story_data VALUES (1001, 90, 43, 210, 0, 0, 0);
                INSERT INTO story_event_roulette_bingo VALUES
                    (1001, 10, 0), (1001, 10, 0), (1001, 11, 1);
                INSERT INTO story_event_bingo_reward VALUES
                    (10, 90, 43, 300), (10, 164, 150, 3), (11, 90, 43, 999);
                INSERT INTO story_event_data VALUES (1001, 1755468000, 1756072800);
                "#,
            )
            .unwrap();
        let timeline = json!({"events": [{
            "id": "story-event-0",
            "type": "story_event",
            "global_release_date": "2025-08-17T22:00:00Z",
            "estimated_end_date": "2025-08-24T22:00:00Z"
        }]});
        let mut rewards = Vec::new();
        load_story_rewards(
            &connection,
            &timeline,
            &mut rewards,
            "global_release_date",
            "global_master",
            "full_completion",
            "exact_source",
        )
        .unwrap();

        assert_eq!(rewards.len(), 5);
        assert!(rewards
            .iter()
            .any(|reward| reward.currency == "free_jewels" && reward.amount == Some(2460)));
        assert!(rewards
            .iter()
            .any(|reward| reward.currency == "uma_ticket" && reward.amount == Some(2)));
        assert!(rewards
            .iter()
            .any(|reward| reward.currency == "support_ticket" && reward.amount == Some(2)));
        assert!(rewards
            .iter()
            .any(|reward| reward.currency == "rainbow_crystal" && reward.amount == Some(3)));
        assert!(rewards
            .iter()
            .any(|reward| reward.currency == "gold_crystal" && reward.amount == Some(6)));
        assert!(rewards
            .iter()
            .all(|reward| reward.event_id.as_deref() == Some("story-event-0")));
    }

    #[test]
    fn extracts_ticket_event_exchanges_from_master_limits() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE item_exchange (
                    item_exchange_top_id INTEGER, change_item_limit_type INTEGER,
                    change_item_limit_num INTEGER, change_item_category INTEGER,
                    change_item_id INTEGER, change_item_num INTEGER,
                    pay_item_category INTEGER, start_date INTEGER, end_date INTEGER
                );
                INSERT INTO item_exchange VALUES
                    (11000, 1, 3, 40, 41, 1, 165, 1755468000, 1756072800),
                    (11000, 1, 3, 40, 111, 1, 165, 1755468000, 1756072800);
                "#,
            )
            .unwrap();
        let timeline = json!({"events": [{
            "id": "trainer-skills-test-0",
            "type": "trainer_skills_test",
            "global_release_date": "2025-08-17T22:00:00Z",
            "estimated_end_date": "2025-08-24T22:00:00Z"
        }]});
        let mut rewards = Vec::new();
        load_event_exchange_rewards(
            &connection,
            &timeline,
            &mut rewards,
            "global_release_date",
            "trainer_skills_test",
            TRAINER_SKILLS_TEST_CURRENCY_CATEGORY,
            "Trainer Skills Test exchange rewards",
            "global_master",
            "full_exchange",
            "exact_source",
        )
        .unwrap();

        assert_eq!(rewards.len(), 2);
        assert!(rewards
            .iter()
            .all(|reward| reward.amount == Some(3) && reward.default_enabled));
        assert!(rewards
            .iter()
            .all(|reward| reward.event_id.as_deref() == Some("trainer-skills-test-0")));
    }

    #[test]
    fn exposes_conservative_monthly_shop_ticket_toggle_from_master() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE item_exchange (
                    item_exchange_top_id INTEGER, change_item_limit_type INTEGER,
                    change_item_limit_num INTEGER, change_item_category INTEGER,
                    change_item_id INTEGER, change_item_num INTEGER,
                    pay_item_category INTEGER, start_date INTEGER, end_date INTEGER
                );
                INSERT INTO item_exchange VALUES
                    (2000, 4, 1, 40, 41, 1, 103, 1767650400, 2543011199),
                    (2000, 4, 1, 40, 111, 1, 103, 1767650400, 2543011199),
                    (4000, 4, 2, 40, 41, 1, 99, 1496275200, 2543011199),
                    (4000, 4, 2, 40, 111, 1, 99, 1496275200, 2543011199),
                    (7000, 4, 2, 40, 41, 1, 160, 1496275200, 2543011199);
                "#,
            )
            .unwrap();
        let mut rules = Vec::new();
        load_monthly_ticket_exchange_rules(&connection, &mut rules).unwrap();

        assert_eq!(rules.len(), 4);
        assert_eq!(
            rules
                .iter()
                .filter(|rule| rule.currency == "uma_ticket")
                .map(|rule| rule.amount)
                .sum::<i64>(),
            3
        );
        assert_eq!(
            rules
                .iter()
                .filter(|rule| rule.currency == "support_ticket")
                .map(|rule| rule.amount)
                .sum::<i64>(),
            3
        );
        assert!(rules.iter().all(|rule| {
            rule.scenario_group == Some("monthly_shop_tickets")
                && rule.scenario_option.as_deref() == Some("include")
                && rule.day_of_month == Some(1)
                && !rule.default_enabled
        }));
        assert!(!rules.iter().any(|rule| rule.id.contains("7000")));
    }
}

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Europe::Berlin;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::BTreeMap;

const JP_LAUNCH_TIMESTAMP: i64 = 1_614_135_600;
const GACHA_TYPE_STANDARD: i64 = 3;
const GACHA_TYPE_RERUN: i64 = 12;

#[derive(Debug, Clone, Serialize)]
pub struct CharacterBanner {
    pub gacha_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_type: Option<i64>,
    pub year: i32,
    pub image: String,
    pub start_date: String,
    pub end_date: String,
    pub pickup_card_ids: Vec<i64>,
    pub image_path: String,
    pub start_date_string: String,
    pub end_date_string: String,
    #[serde(skip_serializing)]
    pub start_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SupportBanner {
    pub gacha_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_type: Option<i64>,
    pub year: i32,
    pub image: String,
    pub start_date: String,
    pub end_date: String,
    pub pickup_card_ids: Vec<i64>,
    #[serde(skip_serializing)]
    pub start_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaidBanner {
    pub gacha_id: i64,
    pub gacha_type: i64,
    pub card_type: &'static str,
    pub year: i32,
    pub image: String,
    pub start_date: String,
    pub end_date: String,
    pub pickup_card_ids: Vec<i64>,
    #[serde(skip_serializing)]
    pub start_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct StandardGacha {
    id: i64,
    gacha_type: i64,
    card_type: i64,
    start_date: i64,
    end_date: i64,
}

#[derive(Debug, Clone)]
struct PaidGacha {
    id: i64,
    gacha_type: i64,
    card_type: i64,
    start_date: i64,
    end_date: i64,
}

#[derive(Debug, Clone)]
struct Pickup {
    card_id: i64,
    rarity: i64,
}

pub fn generate_character_banners(connection: &Connection) -> Result<Vec<CharacterBanner>> {
    let gacha_entries = load_standard_gacha(connection)?;
    let pickups_by_gacha = load_standard_pickups(connection)?;
    let mut banners = Vec::new();

    for gacha in gacha_entries
        .into_iter()
        .filter(|gacha| gacha.card_type == 1)
    {
        let start_dt = clamped_start(gacha.start_date);
        let end_dt = timestamp_to_utc(gacha.end_date);
        let year = start_dt.format("%Y").to_string().parse::<i32>()?;
        let image = format!("{}_{}.webp", year, gacha.id);
        let pickup_card_ids = pickup_ids(&pickups_by_gacha, gacha.id, false);

        banners.push(CharacterBanner {
            gacha_id: gacha.id,
            gacha_type: (gacha.gacha_type != GACHA_TYPE_STANDARD).then_some(gacha.gacha_type),
            year,
            image: image.clone(),
            start_date: format_date_iso(start_dt),
            end_date: format_date_iso(end_dt),
            pickup_card_ids,
            image_path: format!("assets/images/character/banner/{}", image),
            start_date_string: format_date_display(start_dt),
            end_date_string: format_date_display(end_dt),
            start_at: start_dt,
        });
    }

    Ok(banners)
}

pub fn generate_support_banners(connection: &Connection) -> Result<Vec<SupportBanner>> {
    let gacha_entries = load_standard_gacha(connection)?;
    let pickups_by_gacha = load_standard_pickups(connection)?;
    let mut banners = Vec::new();

    for gacha in gacha_entries
        .into_iter()
        .filter(|gacha| gacha.card_type == 2)
    {
        let start_dt = clamped_start(gacha.start_date);
        let end_dt = timestamp_to_utc(gacha.end_date);
        let year = start_dt.format("%Y").to_string().parse::<i32>()?;
        let image = format!("{}_{}.webp", year, gacha.id);
        let pickup_card_ids = pickup_ids(&pickups_by_gacha, gacha.id, false);

        banners.push(SupportBanner {
            gacha_id: gacha.id,
            gacha_type: (gacha.gacha_type != GACHA_TYPE_STANDARD).then_some(gacha.gacha_type),
            year,
            image,
            start_date: format_date_display(start_dt),
            end_date: format_date_display(end_dt),
            pickup_card_ids,
            start_at: start_dt,
        });
    }

    Ok(banners)
}

pub fn generate_paid_banners(connection: &Connection) -> Result<Vec<PaidBanner>> {
    let gacha_entries = load_paid_gacha(connection)?;
    let pickups_by_gacha = load_paid_pickups(connection)?;
    let mut banners = Vec::new();

    for gacha in gacha_entries {
        let start_dt = clamped_start(gacha.start_date);
        let end_dt = timestamp_to_utc(gacha.end_date);
        let year = start_dt.format("%Y").to_string().parse::<i32>()?;
        let pickup_card_ids = pickup_ids(&pickups_by_gacha, gacha.id, true);
        let card_type = if gacha.card_type == 1 {
            "character"
        } else {
            "support"
        };

        banners.push(PaidBanner {
            gacha_id: gacha.id,
            gacha_type: gacha.gacha_type,
            card_type,
            year,
            image: format!("{}.webp", gacha.id),
            start_date: format_date_display(start_dt),
            end_date: format_date_display(end_dt),
            pickup_card_ids,
            start_at: start_dt,
        });
    }

    Ok(banners)
}

fn load_standard_gacha(connection: &Connection) -> Result<Vec<StandardGacha>> {
    let mut statement = connection.prepare(
        r#"
        SELECT id, type, card_type, start_date, end_date
        FROM gacha_data
        WHERE type IN (?, ?)
        ORDER BY start_date ASC
        "#,
    )?;

    let rows = statement.query_map([GACHA_TYPE_STANDARD, GACHA_TYPE_RERUN], |row| {
        Ok(StandardGacha {
            id: row.get(0)?,
            gacha_type: row.get(1)?,
            card_type: row.get(2)?,
            start_date: row.get(3)?,
            end_date: row.get(4)?,
        })
    })?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn load_paid_gacha(connection: &Connection) -> Result<Vec<PaidGacha>> {
    let mut statement = connection.prepare(
        r#"
        SELECT id, type, card_type, start_date, end_date
        FROM gacha_data
        WHERE id >= 50000 AND id < 60000
        ORDER BY start_date ASC
        "#,
    )?;

    let rows = statement.query_map([], |row| {
        Ok(PaidGacha {
            id: row.get(0)?,
            gacha_type: row.get(1)?,
            card_type: row.get(2)?,
            start_date: row.get(3)?,
            end_date: row.get(4)?,
        })
    })?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn load_standard_pickups(connection: &Connection) -> Result<BTreeMap<i64, Vec<Pickup>>> {
    let mut statement = connection.prepare(
        r#"
        SELECT gacha_id, card_id, rarity
        FROM gacha_available
        WHERE is_pickup = 1
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            Pickup {
                card_id: row.get(1)?,
                rarity: row.get(2)?,
            },
        ))
    })?;
    collect_pickups(rows)
}

fn load_paid_pickups(connection: &Connection) -> Result<BTreeMap<i64, Vec<Pickup>>> {
    let mut statement = connection.prepare(
        r#"
        SELECT ga.gacha_id, ga.card_id, ga.rarity
        FROM gacha_available ga
        JOIN gacha_data gd ON ga.gacha_id = gd.id
        WHERE gd.id >= 50000 AND gd.id < 60000
          AND ga.is_pickup = 1
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            Pickup {
                card_id: row.get(1)?,
                rarity: row.get(2)?,
            },
        ))
    })?;
    collect_pickups(rows)
}

fn collect_pickups(
    rows: impl Iterator<Item = rusqlite::Result<(i64, Pickup)>>,
) -> Result<BTreeMap<i64, Vec<Pickup>>> {
    let mut pickups_by_gacha: BTreeMap<i64, Vec<Pickup>> = BTreeMap::new();
    for row in rows {
        let (gacha_id, pickup) = row?;
        pickups_by_gacha.entry(gacha_id).or_default().push(pickup);
    }
    Ok(pickups_by_gacha)
}

fn pickup_ids(
    pickups_by_gacha: &BTreeMap<i64, Vec<Pickup>>,
    gacha_id: i64,
    real_cards_only: bool,
) -> Vec<i64> {
    let mut pickups = pickups_by_gacha.get(&gacha_id).cloned().unwrap_or_default();
    pickups.sort_by_key(|pickup| (-pickup.rarity, pickup.card_id));
    pickups
        .into_iter()
        .filter(|pickup| !real_cards_only || pickup.card_id >= 30_000)
        .map(|pickup| pickup.card_id)
        .collect()
}

fn clamped_start(timestamp: i64) -> DateTime<Utc> {
    timestamp_to_utc(timestamp.max(JP_LAUNCH_TIMESTAMP))
}

fn timestamp_to_utc(timestamp: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .expect("master.mdb timestamp should be valid")
}

fn format_date_iso(date_time: DateTime<Utc>) -> String {
    date_time.format("%Y-%m-%dT%H:%M:00.000Z").to_string()
}

fn format_date_display(date_time: DateTime<Utc>) -> String {
    let local = date_time.with_timezone(&Berlin);
    format!(
        "{} {} {}, {}:{}",
        local.day(),
        local.format("%b"),
        local.year(),
        local.hour(),
        local.format("%M")
    )
}

use chrono::{Datelike, Timelike};

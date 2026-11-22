//! Reads JP's gachas and names from a master database.

use super::types::{JpGacha, JpGachaData, MasterNames, NameKind};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::collections::BTreeMap;

/// Pickup gachas: 30000s are the regular character and support banners, 50000s the paid ones.
/// Permanent, debut and ticket gachas sit below.
const PICKUP_GACHA_IDS: (i64, i64) = (30000, 59999);

/// The `text_data` category that holds each kind of name, by its id.
pub(super) fn textCategory(kind: NameKind) -> i64 {
    match kind {
        NameKind::UmaName => 6,
        NameKind::OutfitTitle => 5,
        NameKind::SupportTitle => 76,
    }
}

pub(super) fn readGachaData(jp: &Connection) -> Result<JpGachaData> {
    let card_characters = readCardCharacters(jp)?;
    Ok(JpGachaData {
        gachas: readGachas(jp, &card_characters)?,
        card_characters,
    })
}

/// Pickups are kept only when they are cards: a select gacha lists placeholder slots instead.
fn readGachas(jp: &Connection, cards: &BTreeMap<i64, i64>) -> Result<Vec<JpGacha>> {
    let mut pickups = BTreeMap::<i64, Vec<(i64, i64)>>::new();
    let mut statement = jp.prepare(
        "SELECT gacha_id, card_id, rarity FROM gacha_available WHERE is_pickup = 1 AND gacha_id BETWEEN ?1 AND ?2",
    )?;
    let rows = statement.query_map(PICKUP_GACHA_IDS, |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (gacha_id, card_id, rarity) = row?;
        if cards.contains_key(&card_id) {
            pickups.entry(gacha_id).or_default().push((rarity, card_id));
        }
    }

    let mut statement = jp.prepare(
        "SELECT id, type, card_type, start_date, end_date FROM gacha_data WHERE id BETWEEN ?1 AND ?2 ORDER BY id",
    )?;
    let rows = statement.query_map(PICKUP_GACHA_IDS, |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;

    let mut gachas = Vec::new();
    for row in rows {
        let (gacha_id, gacha_type, card_type, start, end) = row?;
        let mut cards = pickups.remove(&gacha_id).unwrap_or_default();
        cards.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        gachas.push(JpGacha {
            gacha_id,
            gacha_type,
            card_type,
            start_date: timestamp(start).with_context(|| format!("gacha {gacha_id} start"))?,
            end_date: timestamp(end).with_context(|| format!("gacha {gacha_id} end"))?,
            pickup_card_ids: cards.into_iter().map(|(_, card_id)| card_id).collect(),
        });
    }
    Ok(gachas)
}

fn timestamp(seconds: i64) -> Result<DateTime<Utc>> {
    DateTime::from_timestamp(seconds, 0).context("timestamp out of range")
}

/// The character of every character card and support card.
pub(super) fn readCardCharacters(master: &Connection) -> Result<BTreeMap<i64, i64>> {
    let mut characters = BTreeMap::new();
    for table in ["card_data", "support_card_data"] {
        let mut statement = master.prepare(&format!("SELECT id, chara_id FROM {table}"))?;
        let rows =
            statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
        for row in rows {
            let (card_id, chara_id) = row?;
            characters.insert(card_id, chara_id);
        }
    }
    Ok(characters)
}

/// Every name of every kind in a master.
pub(super) fn readAllNames(master: &Connection) -> Result<MasterNames> {
    let mut names = MasterNames::new();
    for kind in super::KINDS {
        names.insert(kind, readNames(master, kind)?);
    }
    Ok(names)
}

/// Every name of one kind in a master, by id.
pub(super) fn readNames(master: &Connection, kind: NameKind) -> Result<BTreeMap<i64, String>> {
    let mut statement =
        master.prepare(r#"SELECT "index", text FROM text_data WHERE category = ?1"#)?;
    let rows = statement.query_map([textCategory(kind)], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut names = BTreeMap::new();
    for row in rows {
        let (id, text) = row?;
        let text = text.trim();
        if !text.is_empty() {
            names.insert(id, text.to_string());
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readGachaDataKeepsOnlyCardPickupsRarestFirst() {
        let master = Connection::open_in_memory().unwrap();
        master
            .execute_batch(
                "CREATE TABLE card_data (id INTEGER, chara_id INTEGER);
                 CREATE TABLE support_card_data (id INTEGER, chara_id INTEGER);
                 CREATE TABLE gacha_data (id INTEGER, type INTEGER, card_type INTEGER, start_date INTEGER, end_date INTEGER);
                 CREATE TABLE gacha_available (gacha_id INTEGER, card_id INTEGER, rarity INTEGER, is_pickup INTEGER);
                 INSERT INTO support_card_data VALUES (30001, 1001), (20002, 1002), (10003, 1003);
                 INSERT INTO gacha_data VALUES (30100, 3, 2, 1767232800, 1767837600), (30101, 12, 2, 1767232800, 1767837600), (20001, 1, 2, 0, 0);
                 INSERT INTO gacha_available VALUES
                     (30100, 10003, 1, 1), (30100, 30001, 3, 1), (30100, 20002, 2, 1), (30100, 30005, 3, 0),
                     (30101, 301, 3, 1), (30101, 302, 3, 1),
                     (20001, 30001, 3, 1);",
            )
            .unwrap();

        let data = readGachaData(&master).unwrap();
        let gachas: Vec<(i64, Vec<i64>)> = data
            .gachas
            .into_iter()
            .map(|gacha| (gacha.gacha_id, gacha.pickup_card_ids))
            .collect();
        assert_eq!(
            gachas,
            [(30100, vec![30001, 20002, 10003]), (30101, vec![])]
        );
        assert_eq!(data.card_characters.get(&20002), Some(&1002));
    }
}

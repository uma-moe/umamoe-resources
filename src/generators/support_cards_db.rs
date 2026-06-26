use crate::generators::common;
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

const BUNDLED_JP_SUPPORT_CARDS_DB_JSON: &[u8] = include_bytes!("../jp_data/support-cards-db.json");
const SUPPORT_CARD_FULL_NAME_CATEGORY: i64 = 75;
const SUPPORT_CARD_TITLE_CATEGORY: i64 = 76;

#[derive(Debug, Clone, Serialize)]
pub struct SupportCardDbEntry {
    id: String,
    name: String,
    #[serde(rename = "character_name")]
    character_name: String,
    #[serde(rename = "card_name", skip_serializing_if = "Option::is_none")]
    card_name: Option<String>,
    #[serde(rename = "card_title", skip_serializing_if = "Option::is_none")]
    card_title: Option<String>,
    rarity: i64,
    #[serde(rename = "type")]
    card_type: String,
    release_date: String,
    #[serde(rename = "isReleased_en")]
    is_released_en: bool,
    #[serde(rename = "isReleased_tw")]
    is_released_tw: Option<bool>,
    #[serde(rename = "isReleased_cn")]
    is_released_cn: Option<bool>,
    #[serde(rename = "isReleased_jp")]
    is_released_jp: bool,
}

struct DbSupportCard {
    card_id: i64,
    chara_id: i64,
    rarity: i64,
    card_type: String,
    release_timestamp: i64,
}

pub fn generate(connection: &Connection) -> Result<Value> {
    let names = common::load_character_name_map(connection)?;
    let card_names = load_text_data_by_category(connection, SUPPORT_CARD_FULL_NAME_CATEGORY)?;
    let card_titles = load_text_data_by_category(connection, SUPPORT_CARD_TITLE_CATEGORY)?;
    let mut entries = Vec::new();

    for card in load_support_cards(connection)? {
        let id = card.card_id.to_string();
        let character_name = names
            .get(&card.chara_id)
            .cloned()
            .unwrap_or_else(|| format!("Unknown_{}", card.chara_id));
        let card_name = card_names.get(&card.card_id).cloned();
        let card_title = card_titles.get(&card.card_id).cloned();

        entries.push(SupportCardDbEntry {
            id,
            name: character_name.clone(),
            character_name,
            card_name,
            card_title,
            rarity: card.rarity,
            card_type: card.card_type,
            release_date: common::release_date(Some(card.release_timestamp)),
            is_released_en: true,
            is_released_tw: None,
            is_released_cn: None,
            is_released_jp: false,
        });
    }

    entries.sort_by(|a, b| {
        b.release_date
            .cmp(&a.release_date)
            .then_with(|| common::numeric_id(&b.id).cmp(&common::numeric_id(&a.id)))
    });

    merge_with_jp_support_cards_db(entries)
}

fn merge_with_jp_support_cards_db(generated_entries: Vec<SupportCardDbEntry>) -> Result<Value> {
    let jp_entries: Value = serde_json::from_slice(BUNDLED_JP_SUPPORT_CARDS_DB_JSON)
        .context("failed to parse bundled src/jp_data/support-cards-db.json")?;
    let jp_entries = jp_entries
        .as_array()
        .context("src/jp_data/support-cards-db.json must be a JSON array")?;

    let mut merged_by_id = BTreeMap::new();
    for entry in jp_entries {
        let object = entry
            .as_object()
            .context("src/jp_data/support-cards-db.json entries must be JSON objects")?;
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .context("src/jp_data/support-cards-db.json entries must have string id")?;
        let mut object = object.clone();
        if !object.contains_key("character_name") {
            if let Some(name) = object.get("name").and_then(Value::as_str) {
                object.insert(
                    "character_name".to_string(),
                    Value::String(name.to_string()),
                );
            }
        }
        object.insert("isReleased_en".to_string(), Value::Bool(false));
        object.insert("isReleased_tw".to_string(), Value::Null);
        object.insert("isReleased_cn".to_string(), Value::Null);
        object.insert("isReleased_jp".to_string(), Value::Bool(true));
        merged_by_id.insert(id.to_string(), object);
    }

    for generated_entry in generated_entries {
        let id = generated_entry.id.clone();
        let generated_object = serde_json::to_value(generated_entry)?
            .as_object()
            .context("generated support card entry must be a JSON object")?
            .clone();

        let entry = merged_by_id.entry(id).or_insert_with(Map::new);
        let is_released_jp = entry
            .get("isReleased_jp")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        for (key, value) in generated_object {
            entry.insert(key, value);
        }
        entry.insert("isReleased_jp".to_string(), Value::Bool(is_released_jp));
    }

    let mut merged_entries = merged_by_id.into_values().collect::<Vec<_>>();
    merged_entries.sort_by(|a, b| {
        let a_release_date = string_field(a, "release_date");
        let b_release_date = string_field(b, "release_date");
        b_release_date.cmp(a_release_date).then_with(|| {
            common::numeric_id(string_field(b, "id"))
                .cmp(&common::numeric_id(string_field(a, "id")))
        })
    });

    Ok(Value::Array(
        merged_entries.into_iter().map(Value::Object).collect(),
    ))
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

fn string_field<'a>(entry: &'a Map<String, Value>, field: &str) -> &'a str {
    entry.get(field).and_then(Value::as_str).unwrap_or_default()
}

fn load_support_cards(connection: &Connection) -> Result<Vec<DbSupportCard>> {
    let mut statement = connection.prepare(
        r#"
        SELECT id, chara_id, rarity, command_id, support_card_type, start_date
        FROM support_card_data
        ORDER BY id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        let command_id = row.get::<_, i64>(3)?;
        let support_card_type = row.get::<_, i64>(4)?;
        Ok(DbSupportCard {
            card_id: row.get(0)?,
            chara_id: row.get(1)?,
            rarity: row.get(2)?,
            card_type: common::support_card_type_name(command_id, support_card_type),
            release_timestamp: row.get(5)?,
        })
    })?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

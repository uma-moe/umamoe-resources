use crate::generators::common;
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

const BUNDLED_JP_SUPPORT_CARDS_DB_JSON: &[u8] = include_bytes!("../jp_data/support-cards-db.json");

#[derive(Debug, Clone, Serialize)]
pub struct SupportCardDbEntry {
    id: String,
    name: String,
    rarity: i64,
    #[serde(rename = "type")]
    card_type: String,
    release_date: String,
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
    let mut entries = Vec::new();

    for card in load_support_cards(connection)? {
        let id = card.card_id.to_string();
        let name = names
            .get(&card.chara_id)
            .cloned()
            .unwrap_or_else(|| format!("Unknown_{}", card.chara_id));

        entries.push(SupportCardDbEntry {
            id,
            name,
            rarity: card.rarity,
            card_type: card.card_type,
            release_date: common::release_date(Some(card.release_timestamp)),
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
        merged_by_id.insert(id.to_string(), object.clone());
    }

    for generated_entry in generated_entries {
        let id = generated_entry.id.clone();
        let generated_object = serde_json::to_value(generated_entry)?
            .as_object()
            .context("generated support card entry must be a JSON object")?
            .clone();

        let entry = merged_by_id.entry(id).or_insert_with(Map::new);
        for (key, value) in generated_object {
            entry.insert(key, value);
        }
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

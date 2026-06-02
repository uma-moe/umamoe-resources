use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const CHARACTER_NAME_CATEGORY: i64 = 6;
const BUNDLED_CHARACTER_NAMES_JSON: &[u8] = include_bytes!("../jp_data/character_names.json");

pub fn load_character_name_map(connection: &Connection) -> Result<BTreeMap<i64, String>> {
    let mut name_map = BTreeMap::new();
    let names = read_character_names()?;

    for (chara_id, entry) in names
        .as_object()
        .context("character_names.json must be a JSON object")?
    {
        if let (Ok(chara_id), Some(name)) = (
            chara_id.parse::<i64>(),
            entry.get("name").and_then(Value::as_str),
        ) {
            name_map.insert(chara_id, name.to_string());
        }
    }

    for (chara_id, global_name) in load_global_character_names(connection)? {
        name_map.insert(chara_id, global_name);
    }

    Ok(name_map)
}

pub fn read_character_names() -> Result<Value> {
    serde_json::from_slice(BUNDLED_CHARACTER_NAMES_JSON)
        .context("failed to parse bundled src/jp_data/character_names.json")
}

pub fn load_jp_character_ids() -> Result<BTreeSet<i64>> {
    let names = read_character_names()?;
    let mut ids = BTreeSet::new();

    for chara_id in names
        .as_object()
        .context("character_names.json must be a JSON object")?
        .keys()
    {
        if let Ok(chara_id) = chara_id.parse::<i64>() {
            ids.insert(chara_id);
        }
    }

    Ok(ids)
}

pub fn load_global_character_names(connection: &Connection) -> Result<Vec<(i64, String)>> {
    let mut statement = connection.prepare(
        r#"
        SELECT "index", text
        FROM text_data
        WHERE category = ?
        ORDER BY "index"
        "#,
    )?;
    let rows = statement.query_map([CHARACTER_NAME_CATEGORY], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn release_date(timestamp: Option<i64>) -> String {
    if let Some(timestamp) = timestamp {
        return Utc
            .timestamp_opt(timestamp, 0)
            .single()
            .expect("master.mdb timestamp should be valid")
            .format("%Y-%m-%d")
            .to_string();
    }

    String::new()
}

pub fn slug(value: &str) -> String {
    let mut slug = String::new();
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

pub fn numeric_id(id: &str) -> i64 {
    id.parse().unwrap_or_default()
}

pub fn support_card_type_name(command_id: i64, support_card_type: i64) -> String {
    if support_card_type == 3 {
        return "group".to_string();
    }
    match command_id {
        0 => "friend",
        101 => "speed",
        102 => "power",
        103 => "guts",
        105 => "stamina",
        106 => "intelligence",
        _ => return format!("unknown_{}", command_id),
    }
    .to_string()
}

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

/// Which regions have released a card, as a resource set built from one region's master
/// sees it.
pub enum CardReleases {
    /// Built from the Global master: every card is out in Global, and in JP when the bundled
    /// JP names know its character.
    GlobalMaster { jp_characters: BTreeSet<i64> },
    /// Built from the JP master: every card is out in JP, and in Global when the Global master
    /// has it too.
    JpMaster { global_cards: BTreeSet<i64> },
}

impl CardReleases {
    pub fn global_master() -> Result<Self> {
        Ok(Self::GlobalMaster {
            jp_characters: load_jp_character_ids()?,
        })
    }

    /// For a JP set whose cards are the rows of `table`, looked up in the Global master.
    pub fn jp_master(global: &Connection, table: CardTable) -> Result<Self> {
        Ok(Self::JpMaster {
            global_cards: load_card_ids(global, table)?,
        })
    }

    pub fn in_global(&self, card_id: i64) -> bool {
        match self {
            Self::GlobalMaster { .. } => true,
            Self::JpMaster { global_cards } => global_cards.contains(&card_id),
        }
    }

    pub fn in_jp(&self, chara_id: i64) -> bool {
        match self {
            Self::GlobalMaster { jp_characters } => jp_characters.contains(&chara_id),
            Self::JpMaster { .. } => true,
        }
    }
}

/// The master tables that list cards.
#[derive(Clone, Copy)]
pub enum CardTable {
    Characters,
    Supports,
}

fn load_card_ids(connection: &Connection, table: CardTable) -> Result<BTreeSet<i64>> {
    let sql = match table {
        CardTable::Characters => "SELECT id FROM card_data",
        CardTable::Supports => "SELECT id FROM support_card_data",
    };
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
    rows.collect::<rusqlite::Result<BTreeSet<_>>>()
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

#[cfg(test)]
mod tests {
    use super::CardReleases;
    use std::collections::BTreeSet;

    #[test]
    fn a_set_marks_releases_as_its_master_sees_them() {
        let from_global = CardReleases::GlobalMaster {
            jp_characters: BTreeSet::from([1001]),
        };
        assert!(from_global.in_global(100101));
        assert!(from_global.in_jp(1001));
        assert!(!from_global.in_jp(9999));

        let from_jp = CardReleases::JpMaster {
            global_cards: BTreeSet::from([100101]),
        };
        assert!(from_jp.in_global(100101));
        assert!(!from_jp.in_global(107501));
        assert!(from_jp.in_jp(9999));
    }
}

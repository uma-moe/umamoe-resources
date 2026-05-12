use crate::generators::common;
use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

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

pub fn generate(connection: &Connection) -> Result<Vec<SupportCardDbEntry>> {
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

    Ok(entries)
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

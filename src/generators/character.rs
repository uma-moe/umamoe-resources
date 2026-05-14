use crate::generators::common;
use anyhow::Result;
use rusqlite::Connection;

struct DbCharacterCard {
    card_id: i64,
    chara_id: i64,
    rarity: i64,
    release_timestamp: Option<i64>,
}

pub fn generate(connection: &Connection) -> Result<serde_json::Value> {
    let names = common::load_character_name_map(connection)?;
    let cards = load_character_cards(connection)?;
    let mut entries = Vec::with_capacity(cards.len());

    for card in cards {
        let id = card.card_id.to_string();
        let name = names
            .get(&card.chara_id)
            .cloned()
            .unwrap_or_else(|| format!("Unknown_{}", card.chara_id));

        entries.push(serde_json::json!({
            "id": id,
            "name": name,
            "release_date": common::release_date(card.release_timestamp),
            "rarity": card.rarity,
            "href": format!("/umamusume/characters/{}-{}", card.card_id, common::slug(&name)),
            "image": format!("chara_stand_{}.webp", card.card_id),
            "full_image": format!("tex_support_card_{}.webp", card.card_id),
            "type_icon_url": serde_json::Value::Null,
            "type_icon_alt": serde_json::Value::Null,
        }));
    }

    Ok(serde_json::Value::Array(entries))
}

fn load_character_cards(connection: &Connection) -> Result<Vec<DbCharacterCard>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            c.id,
            c.chara_id,
            c.default_rarity,
            MIN(gd.start_date)
        FROM card_data c
        LEFT JOIN gacha_available ga
            ON ga.card_id = c.id
        LEFT JOIN gacha_data gd
            ON gd.id = ga.gacha_id
        GROUP BY c.id, c.chara_id, c.default_rarity
        ORDER BY MIN(gd.start_date) DESC, c.id DESC
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok(DbCharacterCard {
            card_id: row.get(0)?,
            chara_id: row.get(1)?,
            rarity: row.get(2)?,
            release_timestamp: row.get(3)?,
        })
    })?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

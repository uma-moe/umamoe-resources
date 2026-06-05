use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const SKILL_NAME_CATEGORY: i64 = 47;
const SKILL_DESCRIPTION_CATEGORY: i64 = 48;

#[derive(Debug, Clone)]
struct DbSkill {
    skill_id: i64,
    rarity: i64,
    name: Option<String>,
    description: Option<String>,
    base_cost: Option<i64>,
    conditions: String,
    icon: String,
}

pub fn generate(connection: &Connection) -> Result<Value> {
    let db_skills = load_db_skills(connection)?;
    let (unique_skill_ids, inherited_skill_ids) = load_unique_skill_ids(connection)?;
    let character_skill_ids = load_character_skill_ids(connection)?;
    let support_skill_ids = load_support_skill_ids(connection)?;
    let mut entries = Vec::with_capacity(db_skills.len());

    for skill in db_skills.values() {
        let mut entry = serde_json::Map::new();
        entry.insert("id".to_string(), Value::Null);
        entry.insert("skill_id".to_string(), Value::from(skill.skill_id));
        entry.insert("card_id".to_string(), Value::Null);
        entry.insert("rarity".to_string(), Value::from(skill.rarity));
        entry.insert(
            "name".to_string(),
            Value::String(
                skill
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("Skill_{}", skill.skill_id)),
            ),
        );
        if let Some(description) = &skill.description {
            entry.insert(
                "description".to_string(),
                Value::String(description.clone()),
            );
        }
        if let Some(base_cost) = skill.base_cost {
            entry.insert("base_cost".to_string(), Value::from(base_cost));
        }
        if unique_skill_ids.contains(&skill.skill_id)
            || inherited_skill_ids.contains(&skill.skill_id)
        {
            entry.insert("unique".to_string(), Value::Bool(true));
        }
        if inherited_skill_ids.contains(&skill.skill_id) {
            entry.insert("inherited".to_string(), Value::Bool(true));
        }
        if !skill.conditions.is_empty() {
            entry.insert(
                "conditions".to_string(),
                Value::String(skill.conditions.clone()),
            );
        }
        if let Some(character_ids) = character_skill_ids.get(&skill.skill_id) {
            entry.insert(
                "character_ids".to_string(),
                Value::Array(character_ids.iter().copied().map(Value::from).collect()),
            );
        }
        if let Some(support_card_ids) = support_skill_ids.get(&skill.skill_id) {
            entry.insert(
                "support_card_ids".to_string(),
                Value::Array(support_card_ids.iter().copied().map(Value::from).collect()),
            );
        }
        entry.insert("icon".to_string(), Value::String(skill.icon.clone()));
        entries.push(Value::Object(entry));
    }

    Ok(Value::Array(entries))
}

fn load_db_skills(connection: &Connection) -> Result<BTreeMap<i64, DbSkill>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            sd.id,
            sd.rarity,
            sd.condition_1,
            sd.icon_id,
            name.text,
            description.text,
            need.need_skill_point
        FROM skill_data sd
        LEFT JOIN text_data name
            ON name.category = ? AND name."index" = sd.id
        LEFT JOIN text_data description
            ON description.category = ? AND description."index" = sd.id
        LEFT JOIN single_mode_skill_need_point need
            ON need.id = sd.id
        ORDER BY sd.id
        "#,
    )?;
    let rows = statement.query_map(
        params![SKILL_NAME_CATEGORY, SKILL_DESCRIPTION_CATEGORY],
        |row| {
            let skill_id = row.get::<_, i64>(0)?;
            let icon_id = row.get::<_, i64>(3)?;
            Ok((
                skill_id,
                DbSkill {
                    skill_id,
                    rarity: row.get(1)?,
                    conditions: row.get(2)?,
                    icon: format!("utx_ico_skill_{}.webp", icon_id),
                    name: row.get(4)?,
                    description: row.get(5)?,
                    base_cost: row.get(6)?,
                },
            ))
        },
    )?;

    let mut skills = rows
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()
        .map_err(anyhow::Error::from)?;

    let mut text_statement = connection.prepare(
        r#"
        SELECT
            name."index",
            name.text,
            description.text,
            need.need_skill_point
        FROM text_data name
        LEFT JOIN text_data description
            ON description.category = ? AND description."index" = name."index"
        LEFT JOIN single_mode_skill_need_point need
            ON need.id = name."index"
        WHERE name.category = ?
        ORDER BY name."index"
        "#,
    )?;
    let text_rows = text_statement.query_map(
        params![SKILL_DESCRIPTION_CATEGORY, SKILL_NAME_CATEGORY],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        },
    )?;

    for row in text_rows {
        let (skill_id, name, description, base_cost) = row?;
        skills
            .entry(skill_id)
            .and_modify(|skill| {
                skill.name = Some(name.clone());
                skill.description = description.clone();
                if base_cost.is_some() {
                    skill.base_cost = base_cost;
                }
            })
            .or_insert_with(|| DbSkill {
                skill_id,
                rarity: 1,
                name: Some(name),
                description,
                base_cost,
                conditions: String::new(),
                icon: "utx_ico_skill_10011.webp".to_string(),
            });
    }

    Ok(skills)
}

fn load_unique_skill_ids(connection: &Connection) -> Result<(BTreeSet<i64>, BTreeSet<i64>)> {
    let mut statement = connection.prepare(
        r#"
        SELECT id, unique_skill_id_1, unique_skill_id_2
        FROM skill_data
        WHERE unique_skill_id_1 > 0 OR unique_skill_id_2 > 0
        ORDER BY id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;

    let mut unique_skill_ids = BTreeSet::new();
    let mut inherited_skill_ids = BTreeSet::new();
    for row in rows {
        let (inherited_skill_id, primary_unique_skill_id, secondary_unique_skill_id) = row?;
        inherited_skill_ids.insert(inherited_skill_id);
        if primary_unique_skill_id > 0 {
            unique_skill_ids.insert(primary_unique_skill_id);
        }
        if secondary_unique_skill_id > 0 {
            unique_skill_ids.insert(secondary_unique_skill_id);
        }
    }

    Ok((unique_skill_ids, inherited_skill_ids))
}

fn load_character_skill_ids(connection: &Connection) -> Result<BTreeMap<i64, BTreeSet<i64>>> {
    let mut statement = connection.prepare(
        r#"
        SELECT c.id, s.skill_id
        FROM card_data c
        JOIN available_skill_set s
            ON s.available_skill_set_id = c.available_skill_set_id
        WHERE s.skill_id > 0
        ORDER BY s.skill_id, c.id
        "#,
    )?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
    let mut skill_ids = BTreeMap::<i64, BTreeSet<i64>>::new();
    for row in rows {
        let (card_id, skill_id) = row?;
        skill_ids.entry(skill_id).or_default().insert(card_id);
    }
    Ok(skill_ids)
}

fn load_support_skill_ids(connection: &Connection) -> Result<BTreeMap<i64, BTreeSet<i64>>> {
    let mut statement = connection.prepare(
        r#"
        SELECT sc.id, ss.skill_id1, ss.skill_id2, ss.skill_id3, ss.skill_id4, ss.skill_id5,
               ss.skill_id6, ss.skill_id7, ss.skill_id8, ss.skill_id9, ss.skill_id10
        FROM support_card_data sc
        JOIN skill_set ss
            ON ss.id = sc.skill_set_id
        ORDER BY sc.id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        let support_card_id = row.get::<_, i64>(0)?;
        let mut skill_ids = Vec::new();
        for column in 1..=10 {
            let skill_id = row.get::<_, i64>(column)?;
            if skill_id > 0 {
                skill_ids.push(skill_id);
            }
        }
        Ok((support_card_id, skill_ids))
    })?;

    let mut support_skill_ids = BTreeMap::<i64, BTreeSet<i64>>::new();
    for row in rows {
        let (support_card_id, skill_ids) = row?;
        for skill_id in skill_ids {
            support_skill_ids
                .entry(skill_id)
                .or_default()
                .insert(support_card_id);
        }
    }
    Ok(support_skill_ids)
}

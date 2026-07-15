use anyhow::{anyhow, Result};
use rusqlite::{Connection, Row};
use serde::Serialize;
use std::collections::BTreeSet;

const SCHEMA_VERSION: u32 = 1;
const EFFECTS_PER_ALTERNATIVE: usize = 3;
const EFFECT_COLUMN_WIDTH: usize = 7;
const FIRST_ALTERNATIVE_EFFECT_START: usize = 7;
const SECOND_ALTERNATIVE_PRECONDITION_INDEX: usize =
    FIRST_ALTERNATIVE_EFFECT_START + EFFECTS_PER_ALTERNATIVE * EFFECT_COLUMN_WIDTH;
const SECOND_ALTERNATIVE_CONDITION_INDEX: usize = SECOND_ALTERNATIVE_PRECONDITION_INDEX + 1;
const SECOND_ALTERNATIVE_DURATION_INDEX: usize = SECOND_ALTERNATIVE_PRECONDITION_INDEX + 2;
const SECOND_ALTERNATIVE_DURATION_USAGE_INDEX: usize = SECOND_ALTERNATIVE_PRECONDITION_INDEX + 3;
const SECOND_ALTERNATIVE_COOLDOWN_INDEX: usize = SECOND_ALTERNATIVE_PRECONDITION_INDEX + 4;
const SECOND_ALTERNATIVE_EFFECT_START: usize = SECOND_ALTERNATIVE_PRECONDITION_INDEX + 5;
const ADDITIONAL_ACTIVATE_TYPE_COLUMNS: [&str; 6] = [
    "additional_activate_type_1_1",
    "additional_activate_type_1_2",
    "additional_activate_type_1_3",
    "additional_activate_type_2_1",
    "additional_activate_type_2_2",
    "additional_activate_type_2_3",
];
const SKILL_DATA_SELECT: &str = r#"
        SELECT id, rarity,
               precondition_1, condition_1,
               float_ability_time_1, ability_time_usage_1, float_cooldown_time_1,
               ability_type_1_1, ability_value_usage_1_1, additional_activate_type_1_1, ability_value_level_usage_1_1, float_ability_value_1_1, target_type_1_1, target_value_1_1,
               ability_type_1_2, ability_value_usage_1_2, additional_activate_type_1_2, ability_value_level_usage_1_2, float_ability_value_1_2, target_type_1_2, target_value_1_2,
               ability_type_1_3, ability_value_usage_1_3, additional_activate_type_1_3, ability_value_level_usage_1_3, float_ability_value_1_3, target_type_1_3, target_value_1_3,
               precondition_2, condition_2,
               float_ability_time_2, ability_time_usage_2, float_cooldown_time_2,
               ability_type_2_1, ability_value_usage_2_1, additional_activate_type_2_1, ability_value_level_usage_2_1, float_ability_value_2_1, target_type_2_1, target_value_2_1,
               ability_type_2_2, ability_value_usage_2_2, additional_activate_type_2_2, ability_value_level_usage_2_2, float_ability_value_2_2, target_type_2_2, target_value_2_2,
               ability_type_2_3, ability_value_usage_2_3, additional_activate_type_2_3, ability_value_level_usage_2_3, float_ability_value_2_3, target_type_2_3, target_value_2_3
          FROM skill_data
         WHERE is_general_skill = 1 OR rarity >= 3
         ORDER BY id
        "#;

#[derive(Debug, Serialize)]
pub struct SimulatorSkillSet<'a> {
    pub schema_version: u32,
    pub master_version: &'a str,
    pub skills: Vec<SimulatorSkill>,
}

#[derive(Debug, Serialize)]
pub struct SimulatorSkill {
    pub skill_id: u32,
    pub rarity: u8,
    pub alternatives: Vec<SkillAlternative>,
}

#[derive(Debug, Serialize)]
pub struct SkillAlternative {
    pub precondition: String,
    pub condition: String,
    pub base_duration: f32,
    /// Raw `float_cooldown_time_*` value from `skill_data`.
    pub base_cooldown: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_scaling: Option<u8>,
    pub effects: Vec<SkillEffect>,
}

#[derive(Debug, Serialize)]
pub struct SkillEffect {
    pub effect_type: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_activate_type: Option<u8>,
    pub modifier: f32,
    pub target: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_value: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_scaling: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_level_scaling: Option<u8>,
}

pub fn generate<'a>(
    connection: &Connection,
    master_version: &'a str,
) -> Result<SimulatorSkillSet<'a>> {
    let query = skill_data_select_query(connection)?;
    let mut statement = connection.prepare(&query)?;

    let rows = statement.query_map([], skill_from_row)?;
    let mut skills = Vec::new();
    for row in rows {
        skills.push(row?);
    }

    Ok(SimulatorSkillSet {
        schema_version: SCHEMA_VERSION,
        master_version,
        skills,
    })
}

fn skill_data_select_query(connection: &Connection) -> Result<String> {
    let mut statement = connection.prepare("PRAGMA table_info(skill_data)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    let mut query = SKILL_DATA_SELECT.to_owned();

    for column in ADDITIONAL_ACTIVATE_TYPE_COLUMNS {
        if !columns.contains(column) {
            query = query.replace(column, &format!("0 AS {column}"));
        }
    }

    Ok(query)
}

fn skill_from_row(row: &Row<'_>) -> rusqlite::Result<SimulatorSkill> {
    let skill_id = row.get::<_, i64>(0)?;
    let rarity = row.get::<_, i64>(1)?;

    let mut alternatives = vec![alternative_from_row(
        row,
        2,
        3,
        4,
        5,
        6,
        FIRST_ALTERNATIVE_EFFECT_START,
    )?];
    let condition_2 = row.get::<_, String>(SECOND_ALTERNATIVE_CONDITION_INDEX)?;
    if !condition_2.is_empty() && condition_2 != "0" {
        alternatives.push(alternative_from_row(
            row,
            SECOND_ALTERNATIVE_PRECONDITION_INDEX,
            SECOND_ALTERNATIVE_CONDITION_INDEX,
            SECOND_ALTERNATIVE_DURATION_INDEX,
            SECOND_ALTERNATIVE_DURATION_USAGE_INDEX,
            SECOND_ALTERNATIVE_COOLDOWN_INDEX,
            SECOND_ALTERNATIVE_EFFECT_START,
        )?);
    }

    Ok(SimulatorSkill {
        skill_id: as_u32(skill_id, "skill_id")?,
        rarity: as_u8(rarity, "rarity")?,
        alternatives,
    })
}

fn alternative_from_row(
    row: &Row<'_>,
    precondition_index: usize,
    condition_index: usize,
    duration_index: usize,
    duration_usage_index: usize,
    cooldown_index: usize,
    first_effect_index: usize,
) -> rusqlite::Result<SkillAlternative> {
    let mut effects = Vec::new();
    for effect_index in 0..EFFECTS_PER_ALTERNATIVE {
        let effect_start = first_effect_index + effect_index * EFFECT_COLUMN_WIDTH;
        if let Some(effect) = effect_from_row(row, effect_start)? {
            effects.push(effect);
        }
    }

    Ok(SkillAlternative {
        precondition: row.get(precondition_index)?,
        condition: row.get(condition_index)?,
        base_duration: row.get::<_, f64>(duration_index)? as f32,
        base_cooldown: row.get::<_, f64>(cooldown_index)? as f32,
        duration_scaling: optional_non_direct(
            row.get(duration_usage_index)?,
            "ability_time_usage",
        )?,
        effects,
    })
}

fn effect_from_row(row: &Row<'_>, start_index: usize) -> rusqlite::Result<Option<SkillEffect>> {
    let effect_type = row.get::<_, i64>(start_index)?;
    if effect_type == 0 {
        return Ok(None);
    }

    Ok(Some(SkillEffect {
        effect_type: as_u16(effect_type, "ability_type")?,
        value_scaling: optional_non_direct(row.get(start_index + 1)?, "ability_value_usage")?,
        additional_activate_type: optional_nonzero_u8(
            row.get(start_index + 2)?,
            "additional_activate_type",
        )?,
        value_level_scaling: optional_non_direct(
            row.get(start_index + 3)?,
            "ability_value_level_usage",
        )?,
        modifier: row.get::<_, f64>(start_index + 4)? as f32,
        target: as_u8(row.get(start_index + 5)?, "target_type")?,
        target_value: optional_nonzero_u32(row.get(start_index + 6)?, "target_value")?,
    }))
}

fn optional_non_direct(value: i64, field: &'static str) -> rusqlite::Result<Option<u8>> {
    if value == 1 {
        Ok(None)
    } else {
        as_u8(value, field).map(Some)
    }
}

fn optional_nonzero_u32(value: i64, field: &'static str) -> rusqlite::Result<Option<u32>> {
    if value == 0 {
        Ok(None)
    } else {
        as_u32(value, field).map(Some)
    }
}

fn optional_nonzero_u8(value: i64, field: &'static str) -> rusqlite::Result<Option<u8>> {
    if value == 0 {
        Ok(None)
    } else {
        as_u8(value, field).map(Some)
    }
}

fn as_u8(value: i64, field: &'static str) -> rusqlite::Result<u8> {
    u8::try_from(value).map_err(|error| conversion_error(field, value, error))
}

fn as_u16(value: i64, field: &'static str) -> rusqlite::Result<u16> {
    u16::try_from(value).map_err(|error| conversion_error(field, value, error))
}

fn as_u32(value: i64, field: &'static str) -> rusqlite::Result<u32> {
    u32::try_from(value).map_err(|error| conversion_error(field, value, error))
}

fn conversion_error(
    field: &'static str,
    value: i64,
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(
        anyhow!("{field} value {value} is out of range: {error}").into(),
    )
}

#[cfg(test)]
mod tests {
    use super::{generate, skill_data_select_query, ADDITIONAL_ACTIVATE_TYPE_COLUMNS};
    use rusqlite::Connection;

    #[test]
    fn preserves_cooldowns_and_effect_metadata_for_both_alternatives() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE skill_data AS
                SELECT
                    200001 AS id,
                    3 AS rarity,
                    '' AS precondition_1,
                    'phase==1' AS condition_1,
                    30000.0 AS float_ability_time_1,
                    1 AS ability_time_usage_1,
                    15000.0 AS float_cooldown_time_1,
                    27 AS ability_type_1_1,
                    2 AS ability_value_usage_1_1,
                    3 AS additional_activate_type_1_1,
                    1 AS ability_value_level_usage_1_1,
                    2500.0 AS float_ability_value_1_1,
                    9 AS target_type_1_1,
                    123 AS target_value_1_1,
                    0 AS ability_type_1_2,
                    1 AS ability_value_usage_1_2,
                    0 AS additional_activate_type_1_2,
                    1 AS ability_value_level_usage_1_2,
                    0.0 AS float_ability_value_1_2,
                    0 AS target_type_1_2,
                    0 AS target_value_1_2,
                    0 AS ability_type_1_3,
                    1 AS ability_value_usage_1_3,
                    0 AS additional_activate_type_1_3,
                    1 AS ability_value_level_usage_1_3,
                    0.0 AS float_ability_value_1_3,
                    0 AS target_type_1_3,
                    0 AS target_value_1_3,
                    '' AS precondition_2,
                    'phase==2' AS condition_2,
                    40000.0 AS float_ability_time_2,
                    2 AS ability_time_usage_2,
                    60000.0 AS float_cooldown_time_2,
                    31 AS ability_type_2_1,
                    1 AS ability_value_usage_2_1,
                    0 AS additional_activate_type_2_1,
                    2 AS ability_value_level_usage_2_1,
                    3000.0 AS float_ability_value_2_1,
                    1 AS target_type_2_1,
                    0 AS target_value_2_1,
                    0 AS ability_type_2_2,
                    1 AS ability_value_usage_2_2,
                    0 AS additional_activate_type_2_2,
                    1 AS ability_value_level_usage_2_2,
                    0.0 AS float_ability_value_2_2,
                    0 AS target_type_2_2,
                    0 AS target_value_2_2,
                    0 AS ability_type_2_3,
                    1 AS ability_value_usage_2_3,
                    0 AS additional_activate_type_2_3,
                    1 AS ability_value_level_usage_2_3,
                    0.0 AS float_ability_value_2_3,
                    0 AS target_type_2_3,
                    0 AS target_value_2_3,
                    1 AS is_general_skill;
                "#,
            )
            .unwrap();

        let generated = generate(&connection, "test").unwrap();
        let alternatives = &generated.skills[0].alternatives;
        assert_eq!(alternatives.len(), 2);

        let first = &alternatives[0];
        assert_eq!(first.base_duration, 30_000.0);
        assert_eq!(first.base_cooldown, 15_000.0);
        assert_eq!(first.duration_scaling, None);
        assert_eq!(first.effects[0].additional_activate_type, Some(3));
        assert_eq!(first.effects[0].target_value, Some(123));
        assert_eq!(first.effects[0].value_scaling, Some(2));
        assert_eq!(first.effects[0].value_level_scaling, None);

        let second = &alternatives[1];
        assert_eq!(second.base_duration, 40_000.0);
        assert_eq!(second.base_cooldown, 60_000.0);
        assert_eq!(second.duration_scaling, Some(2));
        assert_eq!(second.effects[0].additional_activate_type, None);
        assert_eq!(second.effects[0].target_value, None);
        assert_eq!(second.effects[0].value_scaling, None);
        assert_eq!(second.effects[0].value_level_scaling, Some(2));
    }

    #[test]
    fn projects_missing_legacy_additional_activate_columns_as_zero() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE skill_data (id INTEGER, rarity INTEGER)")
            .unwrap();

        let query = skill_data_select_query(&connection).unwrap();
        for column in ADDITIONAL_ACTIVATE_TYPE_COLUMNS {
            assert!(query.contains(&format!("0 AS {column}")));
        }
    }
}

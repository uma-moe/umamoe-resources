use anyhow::{anyhow, Result};
use rusqlite::{Connection, Row};
use serde::Serialize;

const SCHEMA_VERSION: u32 = 1;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_scaling: Option<u8>,
    pub effects: Vec<SkillEffect>,
}

#[derive(Debug, Serialize)]
pub struct SkillEffect {
    pub effect_type: u16,
    pub modifier: f32,
    pub target: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_scaling: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_level_scaling: Option<u8>,
}

pub fn generate<'a>(
    connection: &Connection,
    master_version: &'a str,
) -> Result<SimulatorSkillSet<'a>> {
    let mut statement = connection.prepare(
        r#"
        SELECT id, rarity,
               precondition_1, condition_1,
               float_ability_time_1, ability_time_usage_1,
               ability_type_1_1, ability_value_usage_1_1, ability_value_level_usage_1_1, float_ability_value_1_1, target_type_1_1,
               ability_type_1_2, ability_value_usage_1_2, ability_value_level_usage_1_2, float_ability_value_1_2, target_type_1_2,
               ability_type_1_3, ability_value_usage_1_3, ability_value_level_usage_1_3, float_ability_value_1_3, target_type_1_3,
               precondition_2, condition_2,
               float_ability_time_2, ability_time_usage_2,
               ability_type_2_1, ability_value_usage_2_1, ability_value_level_usage_2_1, float_ability_value_2_1, target_type_2_1,
               ability_type_2_2, ability_value_usage_2_2, ability_value_level_usage_2_2, float_ability_value_2_2, target_type_2_2,
               ability_type_2_3, ability_value_usage_2_3, ability_value_level_usage_2_3, float_ability_value_2_3, target_type_2_3
          FROM skill_data
         WHERE is_general_skill = 1 OR rarity >= 3
         ORDER BY id
        "#,
    )?;

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

fn skill_from_row(row: &Row<'_>) -> rusqlite::Result<SimulatorSkill> {
    let skill_id = row.get::<_, i64>(0)?;
    let rarity = row.get::<_, i64>(1)?;

    let mut alternatives = vec![alternative_from_row(row, 2, 3, 4, 5, 6)?];
    let condition_2 = row.get::<_, String>(22)?;
    if !condition_2.is_empty() && condition_2 != "0" {
        alternatives.push(alternative_from_row(row, 21, 22, 23, 24, 25)?);
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
    first_effect_index: usize,
) -> rusqlite::Result<SkillAlternative> {
    let mut effects = Vec::new();
    for effect_offset in [0, 5, 10] {
        if let Some(effect) = effect_from_row(row, first_effect_index + effect_offset)? {
            effects.push(effect);
        }
    }

    Ok(SkillAlternative {
        precondition: row.get(precondition_index)?,
        condition: row.get(condition_index)?,
        base_duration: row.get::<_, f64>(duration_index)? as f32,
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
        value_level_scaling: optional_non_direct(
            row.get(start_index + 2)?,
            "ability_value_level_usage",
        )?,
        modifier: row.get::<_, f64>(start_index + 3)? as f32,
        target: as_u8(row.get(start_index + 4)?, "target_type")?,
    }))
}

fn optional_non_direct(value: i64, field: &'static str) -> rusqlite::Result<Option<u8>> {
    if value == 1 {
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

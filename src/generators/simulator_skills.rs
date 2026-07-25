use anyhow::{anyhow, Result};
use rusqlite::{Connection, Row};
use serde::Serialize;

const SCHEMA_VERSION: u32 = 2;
const EFFECTS_PER_ALTERNATIVE: usize = 3;
const EFFECT_COLUMN_WIDTH: usize = 7;
const FIRST_ALTERNATIVE_EFFECT_START: usize = 9;
const SECOND_ALTERNATIVE_PRECONDITION_INDEX: usize =
    FIRST_ALTERNATIVE_EFFECT_START + EFFECTS_PER_ALTERNATIVE * EFFECT_COLUMN_WIDTH;
const SECOND_ALTERNATIVE_CONDITION_INDEX: usize = SECOND_ALTERNATIVE_PRECONDITION_INDEX + 1;
const SECOND_ALTERNATIVE_DURATION_INDEX: usize = SECOND_ALTERNATIVE_PRECONDITION_INDEX + 2;
const SECOND_ALTERNATIVE_DURATION_USAGE_INDEX: usize = SECOND_ALTERNATIVE_PRECONDITION_INDEX + 3;
const SECOND_ALTERNATIVE_COOLDOWN_INDEX: usize = SECOND_ALTERNATIVE_PRECONDITION_INDEX + 4;
const SECOND_ALTERNATIVE_EFFECT_START: usize = SECOND_ALTERNATIVE_PRECONDITION_INDEX + 5;
const TAG_ID_INDEX: usize =
    SECOND_ALTERNATIVE_EFFECT_START + EFFECTS_PER_ALTERNATIVE * EFFECT_COLUMN_WIDTH;

#[derive(Debug, Serialize)]
pub struct SimulatorSkillSet<'a> {
    pub schema_version: u32,
    pub master_version: &'a str,
    pub skill_tag_ids_available: bool,
    pub skills: Vec<SimulatorSkill>,
}

#[derive(Debug, Serialize)]
pub struct SimulatorSkill {
    pub skill_id: u32,
    pub rarity: u8,
    /// Raw binary `skill_data.priority` update group.
    pub priority: u8,
    /// Raw binary `skill_data.activate_lot` flag.
    pub activate_lot: u8,
    /// Raw slash-delimited `skill_data.tag_id` values from the game master data.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tag_ids: Vec<u16>,
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
    let mut statement = connection.prepare(
        r#"
        SELECT id, rarity, priority, activate_lot,
               precondition_1, condition_1,
               float_ability_time_1, ability_time_usage_1, float_cooldown_time_1,
               ability_type_1_1, ability_value_usage_1_1, additional_activate_type_1_1, ability_value_level_usage_1_1, float_ability_value_1_1, target_type_1_1, target_value_1_1,
               ability_type_1_2, ability_value_usage_1_2, additional_activate_type_1_2, ability_value_level_usage_1_2, float_ability_value_1_2, target_type_1_2, target_value_1_2,
               ability_type_1_3, ability_value_usage_1_3, additional_activate_type_1_3, ability_value_level_usage_1_3, float_ability_value_1_3, target_type_1_3, target_value_1_3,
               precondition_2, condition_2,
               float_ability_time_2, ability_time_usage_2, float_cooldown_time_2,
               ability_type_2_1, ability_value_usage_2_1, additional_activate_type_2_1, ability_value_level_usage_2_1, float_ability_value_2_1, target_type_2_1, target_value_2_1,
               ability_type_2_2, ability_value_usage_2_2, additional_activate_type_2_2, ability_value_level_usage_2_2, float_ability_value_2_2, target_type_2_2, target_value_2_2,
               ability_type_2_3, ability_value_usage_2_3, additional_activate_type_2_3, ability_value_level_usage_2_3, float_ability_value_2_3, target_type_2_3, target_value_2_3,
               tag_id
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
        skill_tag_ids_available: true,
        skills,
    })
}

fn skill_from_row(row: &Row<'_>) -> rusqlite::Result<SimulatorSkill> {
    let skill_id = row.get::<_, i64>(0)?;
    let rarity = row.get::<_, i64>(1)?;
    let priority = row.get::<_, i64>(2)?;
    let activate_lot = row.get::<_, i64>(3)?;

    let mut alternatives = vec![alternative_from_row(
        row,
        4,
        5,
        6,
        7,
        8,
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
        priority: as_binary_u8(priority, "priority")?,
        activate_lot: as_binary_u8(activate_lot, "activate_lot")?,
        tag_ids: parse_tag_ids(&row.get::<_, String>(TAG_ID_INDEX)?)?,
        alternatives,
    })
}

fn parse_tag_ids(tag_ids: &str) -> rusqlite::Result<Vec<u16>> {
    tag_ids
        .split('/')
        .filter(|tag_id| !tag_id.is_empty())
        .map(|tag_id| {
            tag_id
                .parse::<i64>()
                .map_err(|error| invalid_tag_id_error(tag_id, error))
                .and_then(|value| as_u16(value, "tag_id"))
        })
        .collect()
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

fn as_binary_u8(value: i64, field: &'static str) -> rusqlite::Result<u8> {
    let value = as_u8(value, field)?;
    if value <= 1 {
        Ok(value)
    } else {
        Err(rusqlite::Error::ToSqlConversionFailure(
            anyhow!("{field} value {value} is not a binary flag").into(),
        ))
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

fn invalid_tag_id_error(
    tag_id: &str,
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(
        anyhow!("tag_id `{tag_id}` is not a valid integer: {error}").into(),
    )
}

#[cfg(test)]
mod tests {
    use super::{as_binary_u8, parse_tag_ids};

    #[test]
    fn parses_all_slash_delimited_skill_tags() {
        assert_eq!(parse_tag_ids("401/403/601").unwrap(), vec![401, 403, 601]);
    }

    #[test]
    fn rejects_non_numeric_skill_tags() {
        assert!(parse_tag_ids("401/not-a-tag/601").is_err());
    }

    #[test]
    fn accepts_only_binary_skill_flags() {
        assert_eq!(as_binary_u8(0, "priority").unwrap(), 0);
        assert_eq!(as_binary_u8(1, "priority").unwrap(), 1);
        assert!(as_binary_u8(2, "priority").is_err());
    }
}

use anyhow::Result;
use chrono::Utc;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct RaceProgramEntry {
    pub id: i64,
    pub type_num: i32,
    #[serde(rename = "type")]
    pub race_type: &'static str,
    pub race_instance_id: i64,
}

#[derive(Debug, Serialize)]
pub struct RaceProgram {
    pub races: BTreeMap<String, RaceProgramEntry>,
    pub categorized: BTreeMap<&'static str, BTreeMap<String, RaceProgramEntry>>,
    pub metadata: RaceProgramMetadata,
}

#[derive(Debug, Serialize)]
pub struct RaceProgramMetadata {
    pub total_count: usize,
    pub description: &'static str,
    pub grade_counts: BTreeMap<&'static str, usize>,
    pub grade_info: BTreeMap<&'static str, &'static str>,
    pub last_updated: String,
    pub data_hash: String,
}

pub fn generate(connection: &Connection) -> Result<RaceProgram> {
    let mut statement = connection.prepare("SELECT * FROM single_mode_program ORDER BY id")?;

    let rows = statement.query_map([], |row| {
        let race_id: i64 = row.get(0)?;
        let race_instance_id = find_legacy_race_instance_id(row)?;
        let grade = race_grade(race_instance_id);
        Ok(RaceProgramEntry {
            id: race_id,
            type_num: grade.type_num,
            race_type: grade.race_type,
            race_instance_id,
        })
    })?;

    let mut races = BTreeMap::new();
    for row_result in rows {
        let race = row_result?;
        races.insert(race.id.to_string(), race);
    }

    let mut categorized = empty_categories();
    let mut grade_counts = empty_counts();
    for (race_id, race) in &races {
        *grade_counts.entry(race.race_type).or_insert(0) += 1;
        categorized
            .entry(race.race_type)
            .or_default()
            .insert(race_id.clone(), race.clone());
    }

    let races_json = serde_json::to_vec(&races)?;
    let data_hash = format!("{:x}", md5::compute(races_json));

    Ok(RaceProgram {
        metadata: RaceProgramMetadata {
            total_count: races.len(),
            description: "Race program mappings from single_mode_program",
            grade_counts,
            grade_info: grade_info(),
            last_updated: Utc::now().to_rfc3339(),
            data_hash,
        },
        races,
        categorized,
    })
}

fn find_legacy_race_instance_id(row: &rusqlite::Row<'_>) -> rusqlite::Result<i64> {
    // Keep parity with race-program-convert.py, which scans SELECT * for the first race-looking id.
    for column_index in 0..row.as_ref().column_count() {
        let ValueRef::Integer(value) = row.get_ref(column_index)? else {
            continue;
        };
        let value_text = value.to_string();
        if value_text.len() >= 6
            && ["1", "2", "3", "4", "9"]
                .iter()
                .any(|prefix| value_text.starts_with(prefix))
        {
            return Ok(value);
        }
    }

    Ok(-1)
}

struct Grade {
    type_num: i32,
    race_type: &'static str,
}

fn race_grade(race_instance_id: i64) -> Grade {
    let race_id_text = race_instance_id.to_string();
    if race_id_text.starts_with('1') {
        Grade {
            type_num: 1,
            race_type: "G1",
        }
    } else if race_id_text.starts_with('2') {
        Grade {
            type_num: 2,
            race_type: "G2",
        }
    } else if race_id_text.starts_with('3') {
        Grade {
            type_num: 3,
            race_type: "G3",
        }
    } else if race_id_text.starts_with('4') {
        Grade {
            type_num: 4,
            race_type: "OP",
        }
    } else if race_id_text.starts_with('9') {
        Grade {
            type_num: 0,
            race_type: "EX",
        }
    } else {
        Grade {
            type_num: -1,
            race_type: "UNKNOWN",
        }
    }
}

fn empty_categories() -> BTreeMap<&'static str, BTreeMap<String, RaceProgramEntry>> {
    ["EX", "G1", "G2", "G3", "OP", "UNKNOWN"]
        .into_iter()
        .map(|race_type| (race_type, BTreeMap::new()))
        .collect()
}

fn empty_counts() -> BTreeMap<&'static str, usize> {
    ["EX", "G1", "G2", "G3", "OP", "UNKNOWN"]
        .into_iter()
        .map(|race_type| (race_type, 0))
        .collect()
}

fn grade_info() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("EX", "race_instance_id starting with 9 (type_num: 0)"),
        ("G1", "race_instance_id starting with 1 (type_num: 1)"),
        ("G2", "race_instance_id starting with 2 (type_num: 2)"),
        ("G3", "race_instance_id starting with 3 (type_num: 3)"),
        (
            "OP",
            "race_instance_id starting with 4 (type_num: 4) - OP and PreOP",
        ),
        ("UNKNOWN", "Other patterns (type_num: -1)"),
    ])
}

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize)]
pub struct WinSaddleEntry {
    pub saddle_id: i64,
    pub group_id: i64,
    pub priority: i64,
    pub condition: i64,
    pub win_saddle_type: i64,
    pub win_saddle_type_label: &'static str,
    pub required_race_instance_ids: Vec<i64>,
}

#[derive(Debug, Serialize)]
pub struct ScheduleEntry {
    pub program_id: i64,
    pub month: i64,
    pub half: i64,
    pub turn_label: String,
    pub program_group: i64,
    pub recommend_class_id: i64,
    pub race_permission: i64,
    pub need_fan_count: i64,
}

#[derive(Debug, Serialize)]
pub struct RaceEntry {
    pub race_instance_id: i64,
    pub race_id: Option<i64>,
    pub thumbnail_id: Option<i64>,
    pub grade: Option<i64>,
    pub name: Option<String>,
    pub short_name: Option<String>,
    pub schedule: Vec<ScheduleEntry>,
    pub win_saddles: Vec<WinSaddleEntry>,
}

#[derive(Debug, Serialize)]
pub struct RaceToSaddleMapping {
    pub races: Vec<RaceEntry>,
    pub metadata: serde_json::Value,
}

#[derive(Debug)]
struct RaceMetadata {
    race_id: i64,
    thumbnail_id: Option<i64>,
    grade: Option<i64>,
    name: Option<String>,
    short_name: Option<String>,
}

pub fn generate(connection: &Connection) -> Result<RaceToSaddleMapping> {
    let race_metadata = load_race_metadata(connection)?;
    let race_to_saddles = load_race_to_saddles(connection)?;
    let mut schedules = load_schedules(connection, &race_to_saddles)?;

    let mut races = Vec::with_capacity(race_to_saddles.len());
    for (race_instance_id, win_saddles) in race_to_saddles {
        let metadata = race_metadata.get(&race_instance_id);
        races.push(RaceEntry {
            race_instance_id,
            race_id: metadata.map(|value| value.race_id),
            thumbnail_id: metadata.and_then(|value| value.thumbnail_id),
            grade: metadata.and_then(|value| value.grade),
            name: metadata.and_then(|value| value.name.clone()),
            short_name: metadata.and_then(|value| value.short_name.clone()),
            schedule: schedules.remove(&race_instance_id).unwrap_or_default(),
            win_saddles,
        });
    }

    let total_races = races.len();
    Ok(RaceToSaddleMapping {
        races,
        metadata: json!({
            "exported_at": Utc::now().to_rfc3339(),
            "source": "master.mdb",
            "total_races": total_races,
            "image_convention": "thumbnail_id maps to game asset tex_race_thumb_{thumbnail_id}",
            "schedule_notes": {
                "month": "In-game calendar month (1=Jan, 4=Apr ... 12=Dec)",
                "half": "1 = first half of month, 2 = second half of month",
                "program_group": "0 = main route; other values = scenario/chara-specific routes",
                "recommend_class_id": "Minimum class required to enter (higher = stricter)"
            },
            "saddle_type": {
                "0": "Crown / multi-race set",
                "1": "G3 single race",
                "2": "G2 single race",
                "3": "G1 single race"
            }
        }),
    })
}

fn load_race_metadata(connection: &Connection) -> Result<BTreeMap<i64, RaceMetadata>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            ri.id,
            ri.race_id,
            r.thumbnail_id,
            r.grade,
            full_name.text,
            short_name.text
        FROM race_instance ri
        LEFT JOIN race r ON r.id = ri.race_id
        LEFT JOIN text_data full_name
            ON full_name.category = 28 AND full_name."index" = ri.id
        LEFT JOIN text_data short_name
            ON short_name.category = 29 AND short_name."index" = ri.id
        ORDER BY ri.id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            RaceMetadata {
                race_id: row.get(1)?,
                thumbnail_id: row.get(2)?,
                grade: row.get(3)?,
                name: row.get(4)?,
                short_name: row.get(5)?,
            },
        ))
    })?;

    let mut metadata = BTreeMap::new();
    for row in rows {
        let (race_instance_id, value) = row?;
        metadata.insert(race_instance_id, value);
    }
    Ok(metadata)
}

fn load_race_to_saddles(connection: &Connection) -> Result<BTreeMap<i64, Vec<WinSaddleEntry>>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            id,
            group_id,
            priority,
            condition,
            win_saddle_type,
            race_instance_id_1,
            race_instance_id_2,
            race_instance_id_3,
            race_instance_id_4,
            race_instance_id_5,
            race_instance_id_6,
            race_instance_id_7,
            race_instance_id_8
        FROM single_mode_wins_saddle
        ORDER BY id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        let mut required_race_instance_ids = Vec::with_capacity(8);
        for column in 5..13 {
            let race_instance_id: i64 = row.get(column)?;
            if race_instance_id != 0 {
                required_race_instance_ids.push(race_instance_id);
            }
        }
        let win_saddle_type = row.get(4)?;
        Ok(WinSaddleEntry {
            saddle_id: row.get(0)?,
            group_id: row.get(1)?,
            priority: row.get(2)?,
            condition: row.get(3)?,
            win_saddle_type,
            win_saddle_type_label: saddle_type_label(win_saddle_type),
            required_race_instance_ids,
        })
    })?;

    let mut race_to_saddles: BTreeMap<i64, Vec<WinSaddleEntry>> = BTreeMap::new();
    for row in rows {
        let saddle = row?;
        for &race_instance_id in &saddle.required_race_instance_ids {
            race_to_saddles
                .entry(race_instance_id)
                .or_default()
                .push(saddle.clone());
        }
    }
    Ok(race_to_saddles)
}

fn load_schedules(
    connection: &Connection,
    race_to_saddles: &BTreeMap<i64, Vec<WinSaddleEntry>>,
) -> Result<BTreeMap<i64, Vec<ScheduleEntry>>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            id,
            race_instance_id,
            month,
            half,
            program_group,
            recommend_class_id,
            race_permission,
            need_fan_count
        FROM single_mode_program
        ORDER BY month, half, program_group, id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        let month = row.get(2)?;
        let half = row.get(3)?;
        Ok((
            row.get::<_, i64>(1)?,
            ScheduleEntry {
                program_id: row.get(0)?,
                month,
                half,
                turn_label: turn_label(month, half),
                program_group: row.get(4)?,
                recommend_class_id: row.get(5)?,
                race_permission: row.get(6)?,
                need_fan_count: row.get(7)?,
            },
        ))
    })?;

    let mut schedules: BTreeMap<i64, Vec<ScheduleEntry>> = BTreeMap::new();
    let mut seen_slots: BTreeMap<i64, BTreeSet<(i64, i64, i64)>> = BTreeMap::new();
    for row in rows {
        let (race_instance_id, schedule) = row?;
        if !race_to_saddles.contains_key(&race_instance_id) {
            continue;
        }
        let slot = (schedule.month, schedule.half, schedule.program_group);
        if seen_slots.entry(race_instance_id).or_default().insert(slot) {
            schedules
                .entry(race_instance_id)
                .or_default()
                .push(schedule);
        }
    }
    Ok(schedules)
}

fn saddle_type_label(win_saddle_type: i64) -> &'static str {
    match win_saddle_type {
        0 => "Crown (multi-race)",
        1 => "G3",
        2 => "G2",
        3 => "G1",
        _ => "Unknown",
    }
}

fn turn_label(month: i64, half: i64) -> String {
    let month = match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => return format!("{month} {} half", if half == 1 { "1st" } else { "2nd" }),
    };
    format!("{month} {} half", if half == 1 { "1st" } else { "2nd" })
}

#[cfg(test)]
mod tests {
    use super::generate;
    use rusqlite::Connection;

    #[test]
    fn preserves_saddle_group_aliases_for_the_same_race_spark() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE race_instance (id INTEGER, race_id INTEGER);
                CREATE TABLE race (id INTEGER, thumbnail_id INTEGER, grade INTEGER);
                CREATE TABLE text_data (category INTEGER, "index" INTEGER, text TEXT);
                CREATE TABLE single_mode_wins_saddle (
                    id INTEGER, group_id INTEGER, priority INTEGER, condition INTEGER,
                    win_saddle_type INTEGER, race_instance_id_1 INTEGER,
                    race_instance_id_2 INTEGER, race_instance_id_3 INTEGER,
                    race_instance_id_4 INTEGER, race_instance_id_5 INTEGER,
                    race_instance_id_6 INTEGER, race_instance_id_7 INTEGER,
                    race_instance_id_8 INTEGER
                );
                CREATE TABLE single_mode_program (
                    id INTEGER, race_instance_id INTEGER, month INTEGER, half INTEGER,
                    program_group INTEGER, recommend_class_id INTEGER,
                    race_permission INTEGER, need_fan_count INTEGER
                );

                INSERT INTO race_instance VALUES
                    (102601, 1026), (110301, 1103), (111101, 1111);
                INSERT INTO race VALUES
                    (1026, 1026, 100), (1103, 1103, 100), (1111, 1111, 100);
                INSERT INTO text_data VALUES
                    (28, 102601, 'Kikuka Sho'),
                    (29, 102601, 'Kikuka Sho'),
                    (28, 110301, 'JBC Ladies Classic'),
                    (29, 110301, 'JBC L. Classic'),
                    (28, 111101, 'JBC Ladies Classic'),
                    (29, 111101, 'JBC L. Classic');
                INSERT INTO single_mode_wins_saddle VALUES
                    (148, 28, 1, 0, 3, 102601, 0, 0, 0, 0, 0, 0, 0),
                    (39, 41, 1, 0, 3, 110301, 0, 0, 0, 0, 0, 0, 0),
                    (156, 41, 1, 0, 3, 111101, 0, 0, 0, 0, 0, 0, 0);
                INSERT INTO single_mode_program VALUES
                    (1, 110301, 11, 1, 0, 14, 4, 12000),
                    (2, 111101, 11, 1, 0, 14, 4, 12000),
                    (3, 111101, 11, 1, 0, 14, 4, 12000);
                "#,
            )
            .unwrap();

        let mapping = generate(&connection).unwrap();
        assert_eq!(mapping.races.len(), 3);
        assert_eq!(mapping.races[0].win_saddles[0].saddle_id, 148);
        assert_eq!(mapping.races[0].win_saddles[0].group_id, 28);
        assert!(mapping.races[0].schedule.is_empty());
        assert_eq!(mapping.races[1].win_saddles[0].saddle_id, 39);
        assert_eq!(mapping.races[1].win_saddles[0].group_id, 41);
        assert_eq!(mapping.races[2].win_saddles[0].saddle_id, 156);
        assert_eq!(mapping.races[2].win_saddles[0].group_id, 41);
        assert_eq!(mapping.races[2].schedule.len(), 1);
    }
}

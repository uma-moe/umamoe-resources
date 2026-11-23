use std::collections::BTreeMap;

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;

/// The race fields skill conditions read besides the course and the conditions: `grade`
/// (G1 Averseness), `time` of day (Night Races) and `is_dirtgrade` (Collaborative Graded Races).
/// Two races on the same course only play alike when these match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RaceSkillFields {
    pub grade: i64,
    pub time: i64,
    pub is_dirtgrade: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoomMatchRace {
    pub race_instance_id: i64,
    pub name: String,
    pub short_name: Option<String>,
    pub course_set_id: i64,
    pub race_track_id: i64,
    pub distance: i64,
    pub course_ground: i64,
    #[serde(flatten)]
    pub skill_fields: RaceSkillFields,
}

/// The race a Champions Meeting on this course is run as.
#[derive(Debug, Clone, Serialize)]
pub struct ChampionsRace {
    pub course_set_id: i64,
    #[serde(flatten)]
    pub skill_fields: RaceSkillFields,
}

#[derive(Debug, Serialize)]
pub struct RoomMatchRaces {
    pub races: Vec<RoomMatchRace>,
    pub champions_races: Vec<ChampionsRace>,
    pub metadata: RoomMatchRaceMetadata,
}

#[derive(Debug, Serialize)]
pub struct RoomMatchRaceMetadata {
    pub total_count: usize,
    pub description: &'static str,
}

/// `ahead` is a master that has run more Champions Meetings than `connection` (JP for Global).
/// It fills in the Champions Meeting race of courses `connection` has not held one on yet.
pub fn generate(connection: &Connection, ahead: Option<&Connection>) -> Result<RoomMatchRaces> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            race_instance.id,
            COALESCE(full_name.text, short_name.text, CAST(race_instance.id AS TEXT)),
            short_name.text,
            course.id,
            course.race_track_id,
            course.distance,
            course.ground,
            race.grade,
            race_instance.time,
            race.is_dirtgrade
        FROM race_instance
        INNER JOIN race ON race.id = race_instance.race_id
        INNER JOIN race_course_set AS course ON course.id = race.course_set
        LEFT JOIN text_data AS full_name
            ON full_name.category = 28 AND full_name."index" = race_instance.id
        LEFT JOIN text_data AS short_name
            ON short_name.category = 29 AND short_name."index" = race_instance.id
        WHERE race_instance.id BETWEEN 800000 AND 899999
        ORDER BY full_name.text COLLATE NOCASE, race_instance.id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok(RoomMatchRace {
            race_instance_id: row.get(0)?,
            name: row.get(1)?,
            short_name: row.get(2)?,
            course_set_id: row.get(3)?,
            race_track_id: row.get(4)?,
            distance: row.get(5)?,
            course_ground: row.get(6)?,
            skill_fields: RaceSkillFields {
                grade: row.get(7)?,
                time: row.get(8)?,
                is_dirtgrade: row.get(9)?,
            },
        })
    })?;
    let races = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read Room Match race resource rows")?;

    let mut champions = champions_races(connection)?;
    if let Some(ahead) = ahead {
        for (course_set_id, skill_fields) in champions_races(ahead)? {
            champions.entry(course_set_id).or_insert(skill_fields);
        }
    }

    Ok(RoomMatchRaces {
        metadata: RoomMatchRaceMetadata {
            total_count: races.len(),
            description: "Named Room Match race instances, and the race each Champions Meeting course is run as, from master data",
        },
        races,
        champions_races: champions
            .into_iter()
            .map(|(course_set_id, skill_fields)| ChampionsRace {
                course_set_id,
                skill_fields,
            })
            .collect(),
    })
}

/// The Champions Meeting race of every course a master has held one on. Every meeting on a
/// course has used the same race so far; the latest one wins should that ever change.
fn champions_races(connection: &Connection) -> Result<BTreeMap<i64, RaceSkillFields>> {
    let mut statement = connection.prepare(
        r#"
        SELECT race.course_set, race.grade, race_instance.time, race.is_dirtgrade
        FROM champions_race_condition AS condition
        INNER JOIN race_instance ON race_instance.id = condition.race_instance_id
        INNER JOIN race ON race.id = race_instance.race_id
        ORDER BY condition.champions_id, condition.round_id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get(0)?,
            RaceSkillFields {
                grade: row.get(1)?,
                time: row.get(2)?,
                is_dirtgrade: row.get(3)?,
            },
        ))
    })?;
    rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()
        .context("failed to read Champions Meeting races")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn master(sql: &str) -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE race_instance (id INTEGER, race_id INTEGER, time INTEGER);
                CREATE TABLE race (id INTEGER, course_set INTEGER, grade INTEGER, is_dirtgrade INTEGER);
                CREATE TABLE race_course_set (id INTEGER, race_track_id INTEGER, distance INTEGER, ground INTEGER);
                CREATE TABLE text_data (category INTEGER, "index" INTEGER, text TEXT);
                CREATE TABLE champions_race_condition (champions_id INTEGER, round_id INTEGER, race_instance_id INTEGER, race_condition_id INTEGER);
                "#,
            )
            .unwrap();
        connection.execute_batch(sql).unwrap();
        connection
    }

    #[test]
    fn emits_named_room_match_races_only() {
        let connection = master(
            r#"
            INSERT INTO race_instance VALUES (800013, 10172, 2), (800014, 10173, 2), (600013, 6013, 2);
            INSERT INTO race VALUES (10172, 10501, 100, 0), (10173, 10501, 999, 0), (6013, 10701, 100, 0);
            INSERT INTO race_course_set VALUES (10501, 10005, 1200, 1), (10701, 10007, 1200, 1);
            INSERT INTO text_data VALUES
                (28, 800013, 'Sprinters Stakes'),
                (29, 800013, 'Sprinters S.'),
                (28, 800014, 'Sprinters Stakes Trial');
            "#,
        );

        let generated = generate(&connection, None).unwrap();
        assert_eq!(generated.races.len(), 2);
        let first = generated
            .races
            .iter()
            .find(|race| race.race_instance_id == 800013)
            .unwrap();
        let alias = generated
            .races
            .iter()
            .find(|race| race.race_instance_id == 800014)
            .unwrap();
        assert_eq!(first.name, "Sprinters Stakes");
        assert_eq!(first.course_set_id, 10501);
        assert_eq!(first.distance, 1200);
        assert_eq!(first.skill_fields.grade, 100);
        assert_eq!(alias.name, "Sprinters Stakes Trial");
        assert_eq!(alias.course_set_id, first.course_set_id);
        assert_eq!(alias.skill_fields.grade, 999);
    }

    #[test]
    fn champions_races_come_from_the_master_then_the_one_ahead() {
        // Global has held a meeting at Nakayama 1200; JP also at Ooi 2000, run in the evening.
        let global = master(
            r#"
            INSERT INTO race_instance VALUES (600013, 6013, 2);
            INSERT INTO race VALUES (6013, 10501, 100, 0);
            INSERT INTO champions_race_condition VALUES (9, 1, 600013, 1);
            "#,
        );
        let jp = master(
            r#"
            INSERT INTO race_instance VALUES (600013, 6013, 3), (600033, 6033, 3);
            INSERT INTO race VALUES (6013, 10501, 100, 1), (6033, 11103, 100, 1);
            INSERT INTO champions_race_condition VALUES (9, 1, 600013, 1), (17, 1, 600033, 1);
            "#,
        );

        let generated = generate(&global, Some(&jp)).unwrap();
        let fields = |course: i64| {
            generated
                .champions_races
                .iter()
                .find(|race| race.course_set_id == course)
                .map(|race| race.skill_fields)
        };
        assert_eq!(
            fields(10501),
            Some(RaceSkillFields {
                grade: 100,
                time: 2,
                is_dirtgrade: 0
            })
        );
        assert_eq!(
            fields(11103),
            Some(RaceSkillFields {
                grade: 100,
                time: 3,
                is_dirtgrade: 1
            })
        );

        let json = serde_json::to_value(&generated).unwrap();
        assert_eq!(json["champions_races"][1]["time"], 3);
    }
}

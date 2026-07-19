use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RoomMatchRace {
    pub race_instance_id: i64,
    pub name: String,
    pub short_name: Option<String>,
    pub course_set_id: i64,
    pub race_track_id: i64,
    pub distance: i64,
    pub course_ground: i64,
}

#[derive(Debug, Serialize)]
pub struct RoomMatchRaces {
    pub races: Vec<RoomMatchRace>,
    pub metadata: RoomMatchRaceMetadata,
}

#[derive(Debug, Serialize)]
pub struct RoomMatchRaceMetadata {
    pub total_count: usize,
    pub description: &'static str,
}

pub fn generate(connection: &Connection) -> Result<RoomMatchRaces> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            race_instance.id,
            COALESCE(full_name.text, short_name.text, CAST(race_instance.id AS TEXT)),
            short_name.text,
            race.course_set,
            course.race_track_id,
            course.distance,
            course.ground
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
        })
    })?;
    let races = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read Room Match race resource rows")?;
    Ok(RoomMatchRaces {
        metadata: RoomMatchRaceMetadata {
            total_count: races.len(),
            description: "Named Room Match race instances from master data",
        },
        races,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_named_room_match_races_only() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE race_instance (id INTEGER, race_id INTEGER);
                CREATE TABLE race (id INTEGER, course_set INTEGER);
                CREATE TABLE race_course_set (id INTEGER, race_track_id INTEGER, distance INTEGER, ground INTEGER);
                CREATE TABLE text_data (category INTEGER, "index" INTEGER, text TEXT);
                INSERT INTO race_instance VALUES (800013, 10172), (600013, 6013);
                INSERT INTO race VALUES (10172, 10501), (6013, 10701);
                INSERT INTO race_course_set VALUES (10501, 10005, 1200, 1), (10701, 10007, 1200, 1);
                INSERT INTO text_data VALUES (28, 800013, 'Sprinters Stakes'), (29, 800013, 'Sprinters S.');
                "#,
            )
            .unwrap();

        let generated = generate(&connection).unwrap();
        assert_eq!(generated.races.len(), 1);
        assert_eq!(generated.races[0].race_instance_id, 800013);
        assert_eq!(generated.races[0].course_set_id, 10501);
        assert_eq!(generated.races[0].name, "Sprinters Stakes");
        assert_eq!(generated.races[0].distance, 1200);
    }
}

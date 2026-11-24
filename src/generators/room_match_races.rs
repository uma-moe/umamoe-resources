use std::collections::btree_map::Entry;
use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use serde::Serialize;

use crate::generators::timeline::{master_timestamp, BannerTimelineEvent, BannerTimelineEventType};

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

/// One Champions Meeting under the game's own id: when it runs, its conditions and its races.
#[derive(Debug, Clone, Serialize)]
pub struct ChampionsMeeting {
    pub champions_id: i64,
    /// Signup opens.
    pub start_at: Option<DateTime<Utc>>,
    /// The last second before the reset that ends it, as the masters give it.
    pub end_at: Option<DateTime<Utc>>,
    /// Where `start_at` and `end_at` come from; null when no date is known.
    pub dates: Option<DateSource>,
    pub season: i64,
    pub weather: i64,
    pub ground: i64,
    /// Its distinct race instances in round order.
    pub races: Vec<ChampionsMeetingRace>,
}

/// When a meeting starts and ends.
type Schedule = (DateTime<Utc>, DateTime<Utc>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DateSource {
    GlobalMaster,
    /// The banner timeline's prediction for a meeting Global has not scheduled yet.
    Timeline,
    JpMaster,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChampionsMeetingRace {
    pub race_instance_id: i64,
    pub course_set_id: i64,
    #[serde(flatten)]
    pub skill_fields: RaceSkillFields,
}

#[derive(Debug, Serialize)]
pub struct RoomMatchRaces {
    pub races: Vec<RoomMatchRace>,
    pub champions_races: Vec<ChampionsRace>,
    pub champions_meetings: Vec<ChampionsMeeting>,
    pub metadata: RoomMatchRaceMetadata,
}

#[derive(Debug, Serialize)]
pub struct RoomMatchRaceMetadata {
    pub total_count: usize,
    pub description: &'static str,
}

/// The Global set. `ahead` is a master that has run more Champions Meetings than `connection`
/// (JP for Global). It fills in the Champions Meeting race of courses `connection` has not held one
/// on yet, and the meetings `connection` lacks, dated as `timeline` predicts them for Global.
pub fn generate(
    connection: &Connection,
    ahead: Option<&Connection>,
    timeline: &[BannerTimelineEvent],
) -> Result<RoomMatchRaces> {
    generate_set(
        connection,
        DateSource::GlobalMaster,
        ahead.map(|ahead| (ahead, timeline)),
    )
}

/// The JP set: the JP master's own races and meetings, dated by its own schedules.
pub fn generate_jp(jp: &Connection) -> Result<RoomMatchRaces> {
    generate_set(jp, DateSource::JpMaster, None)
}

/// `own_dates` names `connection`'s schedules as a source of meeting dates.
fn generate_set(
    connection: &Connection,
    own_dates: DateSource,
    ahead: Option<(&Connection, &[BannerTimelineEvent])>,
) -> Result<RoomMatchRaces> {
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
    let mut meetings = champions_meetings(connection, own_dates)?;
    if let Some((ahead, timeline)) = ahead {
        for (course_set_id, skill_fields) in champions_races(ahead)? {
            champions.entry(course_set_id).or_insert(skill_fields);
        }

        for (champions_id, mut meeting) in champions_meetings(ahead, DateSource::JpMaster)? {
            if meetings.contains_key(&champions_id) {
                continue;
            }
            // `ahead` dated it by when it ran there; Global runs it when the timeline predicts.
            let predicted = meeting
                .start_at
                .zip(meeting.end_at)
                .and_then(|jp_schedule| predicted_dates(timeline, jp_schedule));
            meeting.start_at = predicted.map(|(start, _)| start);
            meeting.end_at = predicted.map(|(_, end)| end);
            meeting.dates = predicted.map(|_| DateSource::Timeline);
            meetings.insert(champions_id, meeting);
        }
    }

    Ok(RoomMatchRaces {
        metadata: RoomMatchRaceMetadata {
            total_count: races.len(),
            description: "Named Room Match race instances, the race each Champions Meeting course is run as, and every Champions Meeting with its dates, conditions and races, from master data",
        },
        races,
        champions_races: champions
            .into_iter()
            .map(|(course_set_id, skill_fields)| ChampionsRace {
                course_set_id,
                skill_fields,
            })
            .collect(),
        champions_meetings: meetings.into_values().collect(),
    })
}

/// Every Champions Meeting a master holds, by id, dated by its own schedule as `dates`. A meeting
/// runs one race instance for its first rounds and another for the rest, under one condition.
fn champions_meetings(
    connection: &Connection,
    dates: DateSource,
) -> Result<BTreeMap<i64, ChampionsMeeting>> {
    let schedules = champions_schedules(connection)?;
    let mut statement = connection.prepare(
        r#"
        SELECT
            condition.champions_id,
            condition.race_instance_id,
            race.course_set,
            race.grade,
            race_instance.time,
            race.is_dirtgrade,
            race_condition.season,
            race_condition.weather,
            race_condition.ground
        FROM champions_race_condition AS condition
        INNER JOIN race_instance ON race_instance.id = condition.race_instance_id
        INNER JOIN race ON race.id = race_instance.race_id
        INNER JOIN race_condition ON race_condition.id = condition.race_condition_id
        ORDER BY condition.champions_id, condition.round_id
        "#,
    )?;
    let mut rows = statement.query([])?;

    let mut meetings = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let champions_id: i64 = row.get(0)?;
        let race = ChampionsMeetingRace {
            race_instance_id: row.get(1)?,
            course_set_id: row.get(2)?,
            skill_fields: RaceSkillFields {
                grade: row.get(3)?,
                time: row.get(4)?,
                is_dirtgrade: row.get(5)?,
            },
        };

        let meeting = match meetings.entry(champions_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let schedule = schedules.get(&champions_id).copied();
                entry.insert(ChampionsMeeting {
                    champions_id,
                    start_at: schedule.map(|(start, _)| start),
                    end_at: schedule.map(|(_, end)| end),
                    dates: schedule.map(|_| dates),
                    season: row.get(6)?,
                    weather: row.get(7)?,
                    ground: row.get(8)?,
                    races: Vec::new(),
                })
            }
        };
        if !meeting
            .races
            .iter()
            .any(|known| known.race_instance_id == race.race_instance_id)
        {
            meeting.races.push(race);
        }
    }
    Ok(meetings)
}

/// Each Champions Meeting's schedule in a master, by id: from signup to the last second of its
/// results.
fn champions_schedules(connection: &Connection) -> Result<BTreeMap<i64, Schedule>> {
    let mut statement = connection
        .prepare("SELECT id, MIN(start_date), MAX(end_date) FROM champions_schedule GROUP BY id")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;

    let mut schedules = BTreeMap::new();
    for row in rows {
        let (id, start, end) = row.context("failed to read Champions Meeting schedules")?;
        schedules.insert(id, (master_timestamp(start)?, master_timestamp(end)?));
    }
    Ok(schedules)
}

/// The Global start and end the timeline predicts for the Champions Meeting JP ran from
/// `jp_start` to `jp_end`. The timeline dates a meeting by its JP race start, which lies inside
/// that schedule; this is how the timeline ties the Global master's schedules to its meetings.
/// The timeline ends an event at the reset, so the end moves to the second before, as in masters.
fn predicted_dates(
    timeline: &[BannerTimelineEvent],
    (jp_start, jp_end): Schedule,
) -> Option<Schedule> {
    timeline
        .iter()
        .find(|event| {
            event.event_type == BannerTimelineEventType::ChampionsMeeting
                && event.jp_release_date >= jp_start
                && event.jp_release_date <= jp_end
        })
        .map(|event| {
            (
                event.global_release_date,
                event.estimated_end_date - Duration::seconds(1),
            )
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
    use crate::generators::timeline::{PredictionInfo, PredictionKind};
    use chrono::TimeZone;
    use serde_json::json;

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
                CREATE TABLE champions_schedule (id INTEGER, start_date INTEGER, end_date INTEGER);
                CREATE TABLE race_condition (id INTEGER, area INTEGER, season INTEGER, weather INTEGER, ground INTEGER, rate INTEGER);
                "#,
            )
            .unwrap();
        connection.execute_batch(sql).unwrap();
        connection
    }

    fn utc(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    /// A meeting's seven rounds: 0–3 run `first`, 4–6 `second`, all under `condition`.
    fn rounds(champions_id: i64, first: i64, second: i64, condition: i64) -> String {
        (0..7)
            .map(|round| {
                let race = if round < 4 { first } else { second };
                format!("({champions_id}, {round}, {race}, {condition})")
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// A schedule as masters keep it: unix seconds, ending the second before the reset.
    fn schedule(champions_id: i64, start: DateTime<Utc>, reset: DateTime<Utc>) -> String {
        format!(
            "({champions_id}, {}, {})",
            start.timestamp(),
            reset.timestamp() - 1
        )
    }

    fn timeline_event(
        event_type: BannerTimelineEventType,
        jp_release_date: DateTime<Utc>,
        global_release_date: DateTime<Utc>,
        estimated_end_date: DateTime<Utc>,
    ) -> BannerTimelineEvent {
        BannerTimelineEvent {
            id: String::new(),
            event_type,
            source: "champions",
            global_mission_event_id: None,
            gacha_id: None,
            gacha_ids: Vec::new(),
            gacha_type: None,
            gacha_type_name: None,
            card_type: None,
            year: None,
            image: String::new(),
            image_path: None,
            title: String::new(),
            description: None,
            jp_release_date,
            global_release_date,
            estimated_end_date,
            is_confirmed: false,
            banner_duration_days: 0,
            tags: Vec::new(),
            pickup_card_ids: Vec::new(),
            related_characters: Vec::new(),
            related_support_cards: Vec::new(),
            related_support_card_names: Vec::new(),
            gametora_url: None,
            umapyoi_url: None,
            prediction: PredictionInfo {
                kind: PredictionKind::Extrapolated,
                acceleration_rate: None,
                schedule_adjustment_days: None,
                calendar_likelihood: None,
                anchor_jp_date: None,
                anchor_global_date: None,
            },
        }
    }

    /// Global has run meeting 9; JP has also run 17 and 18. The timeline predicts 17 for Global
    /// and lists nothing but a story event inside 18's JP schedule.
    fn meeting_masters() -> (Connection, Connection, Vec<BannerTimelineEvent>) {
        let global = master(&format!(
            r#"
            INSERT INTO race_instance VALUES (600013, 6013, 2), (600023, 6023, 2);
            INSERT INTO race VALUES (6013, 10501, 100, 0), (6023, 10501, 100, 0);
            INSERT INTO race_condition VALUES (501, 10, 1, 1, 1, 0);
            INSERT INTO champions_race_condition VALUES {};
            INSERT INTO champions_schedule VALUES {};
            "#,
            rounds(9, 600013, 600023, 501),
            schedule(9, utc(2026, 5, 10, 22), utc(2026, 5, 19, 22)),
        ));
        let jp = master(&format!(
            r#"
            INSERT INTO race_instance VALUES
                (600013, 6013, 2), (600023, 6023, 2), (600033, 6033, 3), (600043, 6043, 3),
                (600053, 6053, 2);
            INSERT INTO race VALUES
                (6013, 10501, 100, 0), (6023, 10501, 100, 0), (6033, 11103, 100, 1),
                (6043, 11103, 100, 1), (6053, 10611, 100, 0);
            INSERT INTO race_condition VALUES
                (501, 10, 1, 3, 1, 0), (502, 10, 4, 2, 3, 0), (503, 10, 2, 1, 1, 0);
            INSERT INTO champions_race_condition VALUES {}, {}, {};
            INSERT INTO champions_schedule VALUES {}, {}, {};
            "#,
            rounds(9, 600013, 600023, 501),
            rounds(17, 600033, 600043, 502),
            rounds(18, 600053, 600053, 503),
            schedule(9, utc(2022, 1, 18, 3), utc(2022, 1, 27, 3)),
            schedule(17, utc(2022, 9, 11, 3), utc(2022, 9, 20, 3)),
            schedule(18, utc(2022, 10, 10, 3), utc(2022, 10, 19, 3)),
        ));
        let timeline = vec![
            // Meeting 9's race in JP; the Global master's dates win over this prediction.
            timeline_event(
                BannerTimelineEventType::ChampionsMeeting,
                utc(2022, 1, 22, 3),
                utc(2026, 5, 12, 22),
                utc(2026, 5, 21, 22),
            ),
            timeline_event(
                BannerTimelineEventType::ChampionsMeeting,
                utc(2022, 9, 15, 3),
                utc(2026, 8, 5, 22),
                utc(2026, 8, 14, 22),
            ),
            timeline_event(
                BannerTimelineEventType::StoryEvent,
                utc(2022, 10, 14, 3),
                utc(2026, 9, 1, 22),
                utc(2026, 9, 10, 22),
            ),
        ];
        (global, jp, timeline)
    }

    fn meeting(generated: &RoomMatchRaces, champions_id: i64) -> &ChampionsMeeting {
        generated
            .champions_meetings
            .iter()
            .find(|meeting| meeting.champions_id == champions_id)
            .unwrap()
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

        let generated = generate(&connection, None, &[]).unwrap();
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

        let generated = generate(&global, Some(&jp), &[]).unwrap();
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

    #[test]
    fn global_meetings_come_from_the_global_master() {
        let (global, jp, timeline) = meeting_masters();

        let generated = generate(&global, Some(&jp), &timeline).unwrap();
        let ids = generated
            .champions_meetings
            .iter()
            .map(|meeting| meeting.champions_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, [9, 17, 18]);

        let global_meeting = meeting(&generated, 9);
        assert_eq!(global_meeting.dates, Some(DateSource::GlobalMaster));
        assert_eq!(global_meeting.start_at, Some(utc(2026, 5, 10, 22)));
        assert_eq!(
            global_meeting.end_at,
            Some(utc(2026, 5, 19, 22) - Duration::seconds(1))
        );
        // Global's weather, not JP's.
        assert_eq!(global_meeting.weather, 1);
        let races = global_meeting
            .races
            .iter()
            .map(|race| race.race_instance_id)
            .collect::<Vec<_>>();
        assert_eq!(races, [600013, 600023]);
    }

    #[test]
    fn meetings_only_jp_has_take_the_timeline_dates() {
        let (global, jp, timeline) = meeting_masters();

        let generated = generate(&global, Some(&jp), &timeline).unwrap();
        let predicted = meeting(&generated, 17);
        assert_eq!(predicted.dates, Some(DateSource::Timeline));
        assert_eq!(predicted.start_at, Some(utc(2026, 8, 5, 22)));
        assert_eq!(
            predicted.end_at,
            Some(utc(2026, 8, 14, 22) - Duration::seconds(1))
        );
        assert_eq!(
            (predicted.season, predicted.weather, predicted.ground),
            (4, 2, 3)
        );
        assert_eq!(predicted.races.len(), 2);
        assert_eq!(
            predicted.races[0].skill_fields,
            RaceSkillFields {
                grade: 100,
                time: 3,
                is_dirtgrade: 1
            }
        );
    }

    #[test]
    fn meetings_the_timeline_does_not_list_have_no_dates() {
        let (global, jp, timeline) = meeting_masters();

        let generated = generate(&global, Some(&jp), &timeline).unwrap();
        let undated = meeting(&generated, 18);
        assert_eq!(undated.dates, None);
        assert_eq!(undated.start_at, None);
        assert_eq!(undated.end_at, None);
        // One race instance for every round.
        assert_eq!(undated.races.len(), 1);
    }

    #[test]
    fn jp_set_dates_meetings_by_the_jp_master() {
        let (_, jp, _) = meeting_masters();

        let generated = generate_jp(&jp).unwrap();
        assert_eq!(generated.champions_meetings.len(), 3);
        let jp_meeting = meeting(&generated, 17);
        assert_eq!(jp_meeting.dates, Some(DateSource::JpMaster));
        assert_eq!(jp_meeting.start_at, Some(utc(2022, 9, 11, 3)));
        assert_eq!(
            jp_meeting.end_at,
            Some(utc(2022, 9, 20, 3) - Duration::seconds(1))
        );
        assert_eq!(meeting(&generated, 9).weather, 3);
    }

    #[test]
    fn champions_meetings_serialize_beside_unchanged_champions_races() {
        let (global, jp, timeline) = meeting_masters();

        let json = serde_json::to_value(generate(&global, Some(&jp), &timeline).unwrap()).unwrap();
        assert_eq!(
            json["champions_meetings"][1],
            json!({
                "champions_id": 17,
                "start_at": "2026-08-05T22:00:00Z",
                "end_at": "2026-08-14T21:59:59Z",
                "dates": "timeline",
                "season": 4,
                "weather": 2,
                "ground": 3,
                "races": [
                    {"race_instance_id": 600033, "course_set_id": 11103, "grade": 100, "time": 3, "is_dirtgrade": 1},
                    {"race_instance_id": 600043, "course_set_id": 11103, "grade": 100, "time": 3, "is_dirtgrade": 1}
                ]
            })
        );
        assert_eq!(json["champions_meetings"][0]["dates"], "global_master");
        assert_eq!(json["champions_meetings"][2]["start_at"], json!(null));
        assert_eq!(json["champions_meetings"][2]["dates"], json!(null));
        assert_eq!(
            json["champions_races"],
            json!([
                {"course_set_id": 10501, "grade": 100, "time": 2, "is_dirtgrade": 0},
                {"course_set_id": 10611, "grade": 100, "time": 2, "is_dirtgrade": 0},
                {"course_set_id": 11103, "grade": 100, "time": 3, "is_dirtgrade": 1}
            ])
        );
    }
}

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const SCHEMA_VERSION: u32 = 1;
const COURSE_EVENT_PARAMS: &[(&str, &str)] = &[
    (
        "10101",
        include_str!("../jp_data/courseeventparams/10101.json"),
    ),
    (
        "10102",
        include_str!("../jp_data/courseeventparams/10102.json"),
    ),
    (
        "10103",
        include_str!("../jp_data/courseeventparams/10103.json"),
    ),
    (
        "10104",
        include_str!("../jp_data/courseeventparams/10104.json"),
    ),
    (
        "10105",
        include_str!("../jp_data/courseeventparams/10105.json"),
    ),
    (
        "10106",
        include_str!("../jp_data/courseeventparams/10106.json"),
    ),
    (
        "10107",
        include_str!("../jp_data/courseeventparams/10107.json"),
    ),
    (
        "10108",
        include_str!("../jp_data/courseeventparams/10108.json"),
    ),
    (
        "10201",
        include_str!("../jp_data/courseeventparams/10201.json"),
    ),
    (
        "10202",
        include_str!("../jp_data/courseeventparams/10202.json"),
    ),
    (
        "10203",
        include_str!("../jp_data/courseeventparams/10203.json"),
    ),
    (
        "10204",
        include_str!("../jp_data/courseeventparams/10204.json"),
    ),
    (
        "10205",
        include_str!("../jp_data/courseeventparams/10205.json"),
    ),
    (
        "10206",
        include_str!("../jp_data/courseeventparams/10206.json"),
    ),
    (
        "10207",
        include_str!("../jp_data/courseeventparams/10207.json"),
    ),
    (
        "10208",
        include_str!("../jp_data/courseeventparams/10208.json"),
    ),
    (
        "10301",
        include_str!("../jp_data/courseeventparams/10301.json"),
    ),
    (
        "10302",
        include_str!("../jp_data/courseeventparams/10302.json"),
    ),
    (
        "10303",
        include_str!("../jp_data/courseeventparams/10303.json"),
    ),
    (
        "10304",
        include_str!("../jp_data/courseeventparams/10304.json"),
    ),
    (
        "10305",
        include_str!("../jp_data/courseeventparams/10305.json"),
    ),
    (
        "10306",
        include_str!("../jp_data/courseeventparams/10306.json"),
    ),
    (
        "10307",
        include_str!("../jp_data/courseeventparams/10307.json"),
    ),
    (
        "10308",
        include_str!("../jp_data/courseeventparams/10308.json"),
    ),
    (
        "10309",
        include_str!("../jp_data/courseeventparams/10309.json"),
    ),
    (
        "10310",
        include_str!("../jp_data/courseeventparams/10310.json"),
    ),
    (
        "10311",
        include_str!("../jp_data/courseeventparams/10311.json"),
    ),
    (
        "10312",
        include_str!("../jp_data/courseeventparams/10312.json"),
    ),
    (
        "10401",
        include_str!("../jp_data/courseeventparams/10401.json"),
    ),
    (
        "10402",
        include_str!("../jp_data/courseeventparams/10402.json"),
    ),
    (
        "10403",
        include_str!("../jp_data/courseeventparams/10403.json"),
    ),
    (
        "10404",
        include_str!("../jp_data/courseeventparams/10404.json"),
    ),
    (
        "10405",
        include_str!("../jp_data/courseeventparams/10405.json"),
    ),
    (
        "10406",
        include_str!("../jp_data/courseeventparams/10406.json"),
    ),
    (
        "10407",
        include_str!("../jp_data/courseeventparams/10407.json"),
    ),
    (
        "10501",
        include_str!("../jp_data/courseeventparams/10501.json"),
    ),
    (
        "10502",
        include_str!("../jp_data/courseeventparams/10502.json"),
    ),
    (
        "10503",
        include_str!("../jp_data/courseeventparams/10503.json"),
    ),
    (
        "10504",
        include_str!("../jp_data/courseeventparams/10504.json"),
    ),
    (
        "10505",
        include_str!("../jp_data/courseeventparams/10505.json"),
    ),
    (
        "10506",
        include_str!("../jp_data/courseeventparams/10506.json"),
    ),
    (
        "10507",
        include_str!("../jp_data/courseeventparams/10507.json"),
    ),
    (
        "10508",
        include_str!("../jp_data/courseeventparams/10508.json"),
    ),
    (
        "10509",
        include_str!("../jp_data/courseeventparams/10509.json"),
    ),
    (
        "10510",
        include_str!("../jp_data/courseeventparams/10510.json"),
    ),
    (
        "10511",
        include_str!("../jp_data/courseeventparams/10511.json"),
    ),
    (
        "10601",
        include_str!("../jp_data/courseeventparams/10601.json"),
    ),
    (
        "10602",
        include_str!("../jp_data/courseeventparams/10602.json"),
    ),
    (
        "10603",
        include_str!("../jp_data/courseeventparams/10603.json"),
    ),
    (
        "10604",
        include_str!("../jp_data/courseeventparams/10604.json"),
    ),
    (
        "10605",
        include_str!("../jp_data/courseeventparams/10605.json"),
    ),
    (
        "10606",
        include_str!("../jp_data/courseeventparams/10606.json"),
    ),
    (
        "10607",
        include_str!("../jp_data/courseeventparams/10607.json"),
    ),
    (
        "10608",
        include_str!("../jp_data/courseeventparams/10608.json"),
    ),
    (
        "10609",
        include_str!("../jp_data/courseeventparams/10609.json"),
    ),
    (
        "10610",
        include_str!("../jp_data/courseeventparams/10610.json"),
    ),
    (
        "10611",
        include_str!("../jp_data/courseeventparams/10611.json"),
    ),
    (
        "10612",
        include_str!("../jp_data/courseeventparams/10612.json"),
    ),
    (
        "10613",
        include_str!("../jp_data/courseeventparams/10613.json"),
    ),
    (
        "10614",
        include_str!("../jp_data/courseeventparams/10614.json"),
    ),
    (
        "10701",
        include_str!("../jp_data/courseeventparams/10701.json"),
    ),
    (
        "10702",
        include_str!("../jp_data/courseeventparams/10702.json"),
    ),
    (
        "10703",
        include_str!("../jp_data/courseeventparams/10703.json"),
    ),
    (
        "10704",
        include_str!("../jp_data/courseeventparams/10704.json"),
    ),
    (
        "10705",
        include_str!("../jp_data/courseeventparams/10705.json"),
    ),
    (
        "10706",
        include_str!("../jp_data/courseeventparams/10706.json"),
    ),
    (
        "10707",
        include_str!("../jp_data/courseeventparams/10707.json"),
    ),
    (
        "10708",
        include_str!("../jp_data/courseeventparams/10708.json"),
    ),
    (
        "10709",
        include_str!("../jp_data/courseeventparams/10709.json"),
    ),
    (
        "10801",
        include_str!("../jp_data/courseeventparams/10801.json"),
    ),
    (
        "10802",
        include_str!("../jp_data/courseeventparams/10802.json"),
    ),
    (
        "10803",
        include_str!("../jp_data/courseeventparams/10803.json"),
    ),
    (
        "10804",
        include_str!("../jp_data/courseeventparams/10804.json"),
    ),
    (
        "10805",
        include_str!("../jp_data/courseeventparams/10805.json"),
    ),
    (
        "10806",
        include_str!("../jp_data/courseeventparams/10806.json"),
    ),
    (
        "10807",
        include_str!("../jp_data/courseeventparams/10807.json"),
    ),
    (
        "10808",
        include_str!("../jp_data/courseeventparams/10808.json"),
    ),
    (
        "10809",
        include_str!("../jp_data/courseeventparams/10809.json"),
    ),
    (
        "10810",
        include_str!("../jp_data/courseeventparams/10810.json"),
    ),
    (
        "10811",
        include_str!("../jp_data/courseeventparams/10811.json"),
    ),
    (
        "10812",
        include_str!("../jp_data/courseeventparams/10812.json"),
    ),
    (
        "10813",
        include_str!("../jp_data/courseeventparams/10813.json"),
    ),
    (
        "10814",
        include_str!("../jp_data/courseeventparams/10814.json"),
    ),
    (
        "10815",
        include_str!("../jp_data/courseeventparams/10815.json"),
    ),
    (
        "10901",
        include_str!("../jp_data/courseeventparams/10901.json"),
    ),
    (
        "10902",
        include_str!("../jp_data/courseeventparams/10902.json"),
    ),
    (
        "10903",
        include_str!("../jp_data/courseeventparams/10903.json"),
    ),
    (
        "10904",
        include_str!("../jp_data/courseeventparams/10904.json"),
    ),
    (
        "10905",
        include_str!("../jp_data/courseeventparams/10905.json"),
    ),
    (
        "10906",
        include_str!("../jp_data/courseeventparams/10906.json"),
    ),
    (
        "10907",
        include_str!("../jp_data/courseeventparams/10907.json"),
    ),
    (
        "10908",
        include_str!("../jp_data/courseeventparams/10908.json"),
    ),
    (
        "10909",
        include_str!("../jp_data/courseeventparams/10909.json"),
    ),
    (
        "10910",
        include_str!("../jp_data/courseeventparams/10910.json"),
    ),
    (
        "10911",
        include_str!("../jp_data/courseeventparams/10911.json"),
    ),
    (
        "10912",
        include_str!("../jp_data/courseeventparams/10912.json"),
    ),
    (
        "10913",
        include_str!("../jp_data/courseeventparams/10913.json"),
    ),
    (
        "10914",
        include_str!("../jp_data/courseeventparams/10914.json"),
    ),
    (
        "11001",
        include_str!("../jp_data/courseeventparams/11001.json"),
    ),
    (
        "11002",
        include_str!("../jp_data/courseeventparams/11002.json"),
    ),
    (
        "11003",
        include_str!("../jp_data/courseeventparams/11003.json"),
    ),
    (
        "11004",
        include_str!("../jp_data/courseeventparams/11004.json"),
    ),
    (
        "11005",
        include_str!("../jp_data/courseeventparams/11005.json"),
    ),
    (
        "11006",
        include_str!("../jp_data/courseeventparams/11006.json"),
    ),
    (
        "11007",
        include_str!("../jp_data/courseeventparams/11007.json"),
    ),
    (
        "11101",
        include_str!("../jp_data/courseeventparams/11101.json"),
    ),
    (
        "11102",
        include_str!("../jp_data/courseeventparams/11102.json"),
    ),
    (
        "11103",
        include_str!("../jp_data/courseeventparams/11103.json"),
    ),
    (
        "11201",
        include_str!("../jp_data/courseeventparams/11201.json"),
    ),
    (
        "11203",
        include_str!("../jp_data/courseeventparams/11203.json"),
    ),
];

#[derive(Debug, Serialize)]
pub struct SimulatorCourseSet<'a> {
    pub schema_version: u32,
    pub master_version: &'a str,
    pub courses: Vec<SimulatorCourse>,
}

#[derive(Debug, Serialize)]
pub struct SimulatorCourse {
    pub course_id: u32,
    pub race_track_id: u32,
    pub distance: u16,
    pub distance_type: u8,
    pub surface: u8,
    pub turn: u8,
    pub course: u8,
    pub lane_max: u32,
    pub lane_max_events: Vec<CourseLaneMaxEvent>,
    pub finish_time_min: u32,
    pub finish_time_max: u32,
    pub course_set_status: Vec<u8>,
    pub corners: Vec<CourseCorner>,
    pub straights: Vec<CourseStraight>,
    pub slopes: Vec<CourseSlope>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CourseCorner {
    pub start: f32,
    pub length: f32,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CourseStraight {
    pub start: f32,
    pub end: f32,
    pub front_type: u8,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CourseSlope {
    pub start: f32,
    pub length: f32,
    pub slope: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CourseLaneMaxEvent {
    pub start: f32,
    pub lane_max: u32,
}

#[derive(Debug, Deserialize)]
struct CourseEventFile {
    #[serde(rename = "courseParams")]
    course_params: Vec<CourseEventParam>,
}

#[derive(Debug, Deserialize)]
struct CourseEventParam {
    #[serde(rename = "_paramType")]
    param_type: i64,
    #[serde(rename = "_values")]
    values: Vec<i64>,
    #[serde(rename = "_distance")]
    distance: f64,
}

#[derive(Debug)]
struct CourseGeometry {
    corners: Vec<CourseCorner>,
    straights: Vec<CourseStraight>,
    slopes: Vec<CourseSlope>,
    lane_max_events: Vec<CourseLaneMaxEvent>,
}

pub fn generate<'a>(
    connection: &Connection,
    master_version: &'a str,
) -> Result<SimulatorCourseSet<'a>> {
    let status_by_id = load_course_set_statuses(connection)?;
    let event_params = load_course_event_params()?;
    let mut statement = connection.prepare(
        r#"
        SELECT id, race_track_id, distance, ground, inout, turn, float_lane_max,
               course_set_status_id, finish_time_min, finish_time_max
          FROM race_course_set
         ORDER BY id
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)?,
            row.get::<_, i64>(9)?,
        ))
    })?;

    let mut courses = Vec::new();
    for row in rows {
        let (
            course_id,
            race_track_id,
            distance,
            surface,
            course,
            turn,
            lane_max,
            course_set_status_id,
            finish_time_min,
            finish_time_max,
        ) = row?;

        // Matches uma-tools: Longchamp 1000m data is incomplete, and 11202 has no event params.
        if course_id == 11201 || course_id == 11202 {
            continue;
        }

        let geometry = event_params
            .get(&course_id)
            .with_context(|| format!("missing course event params for course {course_id}"))?;

        courses.push(SimulatorCourse {
            course_id: as_u32(course_id, "id")?,
            race_track_id: as_u32(race_track_id, "race_track_id")?,
            distance: as_u16(distance, "distance")?,
            distance_type: distance_type(distance),
            surface: as_u8(surface, "ground")?,
            turn: as_u8(turn, "turn")?,
            course: as_u8(course, "inout")?,
            lane_max: as_u32(lane_max, "float_lane_max")?,
            lane_max_events: geometry.lane_max_events.clone(),
            finish_time_min: as_u32(finish_time_min, "finish_time_min")?,
            finish_time_max: as_u32(finish_time_max, "finish_time_max")?,
            course_set_status: status_by_id
                .get(&course_set_status_id)
                .cloned()
                .unwrap_or_default(),
            corners: geometry.corners.clone(),
            straights: geometry.straights.clone(),
            slopes: geometry.slopes.clone(),
        });
    }

    Ok(SimulatorCourseSet {
        schema_version: SCHEMA_VERSION,
        master_version,
        courses,
    })
}

fn load_course_set_statuses(connection: &Connection) -> Result<BTreeMap<i64, Vec<u8>>> {
    let mut statement = connection.prepare(
        "SELECT course_set_status_id, target_status_1, target_status_2 FROM race_course_set_status ORDER BY course_set_status_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;

    let mut statuses = BTreeMap::new();
    for row in rows {
        let (id, status_1, status_2) = row?;
        let mut values = vec![as_u8(status_1, "target_status_1")?];
        if status_2 != 0 {
            values.push(as_u8(status_2, "target_status_2")?);
        }
        statuses.insert(id, values);
    }
    Ok(statuses)
}

fn load_course_event_params() -> Result<BTreeMap<i64, CourseGeometry>> {
    let mut courses = BTreeMap::new();
    for (course_id, json) in COURSE_EVENT_PARAMS {
        let id = course_id
            .parse::<i64>()
            .with_context(|| format!("invalid bundled course event param id {course_id}"))?;
        if id == 11201 || id == 11202 {
            continue;
        }
        courses.insert(id, parse_course_event_params(id, json)?);
    }
    Ok(courses)
}

fn parse_course_event_params(course_id: i64, json: &str) -> Result<CourseGeometry> {
    let file: CourseEventFile = serde_json::from_str(json)
        .with_context(|| format!("failed to parse course event params for {course_id}"))?;
    let mut corners = Vec::new();
    let mut straights = Vec::new();
    let mut slopes = Vec::new();
    let mut lane_max_events = Vec::new();
    let mut pending_straight: Option<(f32, u8)> = None;

    for event in file.course_params {
        match event.param_type {
            0 => corners.push(CourseCorner {
                start: distance_as_f32(event.distance, course_id, "corner.start")?,
                length: event_value_as_f32(&event, 1, course_id, "corner.length")?,
            }),
            2 => {
                match event_value(&event, 0, course_id, "straight marker")? {
                    1 => {
                        if pending_straight.is_some() {
                            bail!("course {course_id} started a straight before ending the previous one");
                        }
                        pending_straight = Some((
                            distance_as_f32(event.distance, course_id, "straight.start")?,
                            event_value_as_u8(&event, 1, course_id, "straight.front_type")?,
                        ));
                    }
                    2 => {
                        let Some((start, front_type)) = pending_straight.take() else {
                            bail!("course {course_id} ended a straight before starting one");
                        };
                        straights.push(CourseStraight {
                            start,
                            end: distance_as_f32(event.distance, course_id, "straight.end")?,
                            front_type,
                        });
                    }
                    marker => bail!("course {course_id} has unsupported straight marker {marker}"),
                }
            }
            11 => slopes.push(CourseSlope {
                start: distance_as_f32(event.distance, course_id, "slope.start")?,
                length: event_value_as_f32(&event, 1, course_id, "slope.length")?,
                slope: event_value_as_i16(&event, 0, course_id, "slope.slope")?,
            }),
            3 => lane_max_events.push(CourseLaneMaxEvent {
                start: distance_as_f32(event.distance, course_id, "lane_max.start")?,
                lane_max: as_u32(
                    event_value(&event, 0, course_id, "lane_max.value")?,
                    "lane_max.value",
                )?,
            }),
            _ => {}
        }
    }

    if pending_straight.is_some() {
        bail!("course {course_id} has an unterminated straight");
    }

    corners.sort_by(|left, right| left.start.total_cmp(&right.start));
    straights.sort_by(|left, right| left.start.total_cmp(&right.start));
    slopes.sort_by(|left, right| left.start.total_cmp(&right.start));
    lane_max_events.sort_by(|left, right| left.start.total_cmp(&right.start));

    Ok(CourseGeometry {
        corners,
        straights,
        slopes,
        lane_max_events,
    })
}

fn distance_type(distance: i64) -> u8 {
    match distance {
        ..=1400 => 1,
        1401..=1800 => 2,
        1801..=2499 => 3,
        2500.. => 4,
    }
}

fn event_value(event: &CourseEventParam, index: usize, course_id: i64, field: &str) -> Result<i64> {
    event
        .values
        .get(index)
        .copied()
        .ok_or_else(|| anyhow!("course {course_id} missing {field}"))
}

fn event_value_as_u8(
    event: &CourseEventParam,
    index: usize,
    course_id: i64,
    field: &str,
) -> Result<u8> {
    as_u8(event_value(event, index, course_id, field)?, field)
}

fn event_value_as_f32(
    event: &CourseEventParam,
    index: usize,
    course_id: i64,
    field: &str,
) -> Result<f32> {
    let value = event_value(event, index, course_id, field)? as f32;
    if !value.is_finite() {
        bail!("course {course_id} has non-finite {field} value {value}");
    }
    Ok(value)
}

fn event_value_as_i16(
    event: &CourseEventParam,
    index: usize,
    course_id: i64,
    field: &str,
) -> Result<i16> {
    i16::try_from(event_value(event, index, course_id, field)?)
        .with_context(|| format!("{field} is out of i16 range"))
}

fn distance_as_f32(distance: f64, course_id: i64, field: &str) -> Result<f32> {
    if !distance.is_finite() {
        bail!("course {course_id} has non-finite {field} distance {distance}");
    }
    Ok(distance as f32)
}

fn as_u8(value: i64, field: &str) -> Result<u8> {
    u8::try_from(value).with_context(|| format!("{field} value {value} is out of u8 range"))
}

fn as_u16(value: i64, field: &str) -> Result<u16> {
    u16::try_from(value).with_context(|| format!("{field} value {value} is out of u16 range"))
}

fn as_u32(value: i64, field: &str) -> Result<u32> {
    u32::try_from(value).with_context(|| format!("{field} value {value} is out of u32 range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_lane_max_changes_from_authoritative_course_events() {
        let course = parse_course_event_params(
            10_504,
            include_str!("../jp_data/courseeventparams/10504.json"),
        )
        .expect("Nakayama 2000m event data should parse");

        assert_eq!(
            course.lane_max_events,
            vec![
                CourseLaneMaxEvent {
                    start: 875.0,
                    lane_max: 7_000,
                },
                CourseLaneMaxEvent {
                    start: 1450.0,
                    lane_max: 10_000,
                },
            ]
        );
    }
}

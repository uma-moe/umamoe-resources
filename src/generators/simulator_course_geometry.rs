use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;

use super::simulator_courses::SimulatorCourseSet;
use super::ResourceOutput;

const SCHEMA_VERSION: u32 = 1;
const KEYFRAME_COUNT: usize = 1_001;
const SOURCE_BYTES: &[u8] = include_bytes!("../global_data/simulator_course_geometry.json.gz");
const LANE_SOURCE_BYTES: &[u8] = include_bytes!("../global_data/simulator_course_lanes.json.gz");

#[derive(Debug, Deserialize)]
struct SourceGeometrySet {
    schema_version: u32,
    source_master_version: String,
    courses: Vec<SourceCourseGeometry>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SourceCourseGeometry {
    course_id: u32,
    race_track_id: u32,
    course_distance: f64,
    source_asset: String,
    position_x: Vec<f64>,
    position_y: Vec<f64>,
    position_z: Vec<f64>,
    rotation_x: Vec<f64>,
    rotation_y: Vec<f64>,
    rotation_z: Vec<f64>,
    rotation_w: Vec<f64>,
}

#[derive(Serialize)]
struct PublishedCourseGeometrySet<'a> {
    schema_version: u32,
    master_version: &'a str,
    courses: Vec<&'a SourceCourseGeometry>,
}

/// Produces one geometry artifact with an array of current courses. The bundled
/// source payload originates from the client CourseLaneAnim assets; the
/// artifact master version deliberately records that source version rather
/// than pretending the client transforms were regenerated from master.mdb.
pub fn generate(courses: &SimulatorCourseSet<'_>) -> Result<ResourceOutput> {
    let source = decode_bundled_source()?;
    generate_from_source(courses, &source)
}

pub fn version_hash() -> Result<String> {
    let mut decoder = GzDecoder::new(SOURCE_BYTES);
    let mut bytes = Vec::new();
    decoder
        .read_to_end(&mut bytes)
        .context("failed to decompress bundled simulator course geometry")?;
    let mut digest = Sha256::new();
    digest.update(b"simulator_course_geometry.json\0");
    digest.update(SCHEMA_VERSION.to_le_bytes());
    digest.update(bytes);
    digest.update(LANE_SOURCE_BYTES);
    Ok(hex::encode(digest.finalize()))
}

/// Retained overrun lanes supplement the normal course geometry without resampling it.
#[allow(non_snake_case)]
pub fn generateLanes(courses: &SimulatorCourseSet<'_>) -> Result<ResourceOutput> {
    let mut bytes = Vec::new();
    GzDecoder::new(LANE_SOURCE_BYTES).read_to_end(&mut bytes)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let lanes = value["courses"]
        .as_array()
        .context("missing retained lane courses")?;
    let assets = value["assets"]
        .as_array()
        .context("missing retained lane assets")?;
    for course in &courses.courses {
        let lane = lanes
            .iter()
            .find(|lane| lane["courseId"].as_u64() == Some(u64::from(course.course_id)))
            .with_context(|| format!("missing overrun lane for course {}", course.course_id))?;
        let name = lane["overrun"]
            .as_str()
            .context("missing overrun asset name")?;
        anyhow::ensure!(
            assets
                .iter()
                .any(|asset| asset["resourcePath"].as_str() == Some(name)),
            "missing overrun asset {name}"
        );
    }

    Ok(ResourceOutput {
        file_name: "simulator_course_lanes.json".into(),
        value,
    })
}

fn decode_bundled_source() -> Result<SourceGeometrySet> {
    let mut decoder = GzDecoder::new(SOURCE_BYTES);
    let mut bytes = Vec::new();
    decoder
        .read_to_end(&mut bytes)
        .context("failed to decompress bundled simulator course geometry")?;
    serde_json::from_slice(&bytes).context("failed to decode bundled simulator course geometry")
}

fn generate_from_source(
    courses: &SimulatorCourseSet<'_>,
    source: &SourceGeometrySet,
) -> Result<ResourceOutput> {
    if source.schema_version != SCHEMA_VERSION {
        bail!(
            "simulator course geometry source schema must be {SCHEMA_VERSION}, got {}",
            source.schema_version
        );
    }
    if source.source_master_version.trim().is_empty() {
        bail!("simulator course geometry source master version must not be empty");
    }

    let mut source_by_course = BTreeMap::new();
    for source_course in &source.courses {
        validate_source_course(source_course)?;
        if source_by_course
            .insert(source_course.course_id, source_course)
            .is_some()
        {
            bail!(
                "simulator course geometry source has duplicate course {}",
                source_course.course_id
            );
        }
    }

    let mut published_courses = Vec::with_capacity(courses.courses.len());
    for course in &courses.courses {
        let source_course = source_by_course
            .remove(&course.course_id)
            .with_context(|| {
                format!(
                    "simulator course geometry source is missing current course {}",
                    course.course_id
                )
            })?;
        if source_course.race_track_id != course.race_track_id {
            bail!(
                "simulator course geometry source course {} has race track {}, expected {}",
                course.course_id,
                source_course.race_track_id,
                course.race_track_id
            );
        }
        if source_course.course_distance != f64::from(course.distance) {
            bail!(
                "simulator course geometry source course {} has distance {}, expected {}",
                course.course_id,
                source_course.course_distance,
                course.distance
            );
        }

        published_courses.push(source_course);
    }

    super::output(
        "simulator_course_geometry.json",
        PublishedCourseGeometrySet {
            schema_version: SCHEMA_VERSION,
            master_version: &source.source_master_version,
            courses: published_courses,
        },
    )
}

fn validate_source_course(course: &SourceCourseGeometry) -> Result<()> {
    if course.course_id == 0 {
        bail!("simulator course geometry source has zero course id");
    }
    if course.race_track_id == 0 {
        bail!(
            "simulator course geometry source course {} has zero race track id",
            course.course_id
        );
    }
    if !course.course_distance.is_finite() || course.course_distance < 1.0 {
        bail!(
            "simulator course geometry source course {} has invalid distance",
            course.course_id
        );
    }
    if course.source_asset.trim().is_empty() {
        bail!(
            "simulator course geometry source course {} has no source asset",
            course.course_id
        );
    }

    for (column, values) in [
        ("position_x", &course.position_x),
        ("position_y", &course.position_y),
        ("position_z", &course.position_z),
        ("rotation_x", &course.rotation_x),
        ("rotation_y", &course.rotation_y),
        ("rotation_z", &course.rotation_z),
        ("rotation_w", &course.rotation_w),
    ] {
        if values.len() != KEYFRAME_COUNT {
            bail!(
                "simulator course geometry source course {} column {column} has {} keyframes, expected {KEYFRAME_COUNT}",
                course.course_id,
                values.len()
            );
        }
        if values.iter().any(|value| !value.is_finite()) {
            bail!(
                "simulator course geometry source course {} column {column} contains a non-finite value",
                course.course_id
            );
        }
    }

    for index in 0..KEYFRAME_COUNT {
        let rotation_length_squared = course.rotation_x[index] * course.rotation_x[index]
            + course.rotation_y[index] * course.rotation_y[index]
            + course.rotation_z[index] * course.rotation_z[index]
            + course.rotation_w[index] * course.rotation_w[index];
        if rotation_length_squared <= 0.0 || !rotation_length_squared.is_finite() {
            bail!(
                "simulator course geometry source course {} has a zero rotation at keyframe {index}",
                course.course_id
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::simulator_courses::{
        CourseCorner, CourseLaneMaxEvent, CourseSlope, CourseStraight, SimulatorCourse,
        SimulatorRaceParameters,
    };

    fn course_set() -> SimulatorCourseSet<'static> {
        SimulatorCourseSet {
            schema_version: 1,
            master_version: "current-master",
            race_parameters: SimulatorRaceParameters::current(),
            courses: vec![SimulatorCourse {
                course_id: 10_104,
                race_track_id: 10_001,
                initial_lane_type: 0,
                enable_half_gate: false,
                run_outside: false,
                distance: 2_000,
                distance_type: 3,
                surface: 1,
                turn: 1,
                course: 1,
                lane_max: 12,
                lane_max_events: Vec::<CourseLaneMaxEvent>::new(),
                move_lane_point: 0.0,
                first_move_lane_is_in: false,
                finish_time_min: 1,
                finish_time_min_random_range: 0,
                finish_time_max: 2,
                finish_time_max_random_range: 0,
                course_set_status: Vec::new(),
                corners: Vec::<CourseCorner>::new(),
                straights: Vec::<CourseStraight>::new(),
                slopes: Vec::<CourseSlope>::new(),
            }],
        }
    }

    fn source_course() -> SourceCourseGeometry {
        SourceCourseGeometry {
            course_id: 10_104,
            race_track_id: 10_001,
            course_distance: 2_000.0,
            source_asset: "race/course/10001/pos/an_pos_race10001_00_2000_00_0_0".to_string(),
            position_x: vec![0.0; KEYFRAME_COUNT],
            position_y: vec![0.0; KEYFRAME_COUNT],
            position_z: vec![0.0; KEYFRAME_COUNT],
            rotation_x: vec![0.0; KEYFRAME_COUNT],
            rotation_y: vec![0.0; KEYFRAME_COUNT],
            rotation_z: vec![0.0; KEYFRAME_COUNT],
            rotation_w: vec![1.0; KEYFRAME_COUNT],
        }
    }

    #[test]
    fn publishes_current_courses_in_one_resource() {
        let mut courses = course_set();
        let mut second_course = course_set().courses.remove(0);
        second_course.course_id = 10_105;
        courses.courses.push(second_course);
        let mut second_source_course = source_course();
        second_source_course.course_id = 10_105;
        second_source_course.position_x[0] = 42.0;
        let source = SourceGeometrySet {
            schema_version: SCHEMA_VERSION,
            source_master_version: "source-master".to_string(),
            courses: vec![second_source_course, source_course()],
        };

        let output = generate_from_source(&courses, &source).unwrap();

        assert_eq!(output.file_name, "simulator_course_geometry.json");
        assert_eq!(output.value["schema_version"], SCHEMA_VERSION);
        assert_eq!(output.value["master_version"], "source-master");
        let published = output.value["courses"].as_array().unwrap();
        assert_eq!(published.len(), 2);
        assert_eq!(
            published[0],
            serde_json::to_value(&source.courses[1]).unwrap()
        );
        assert_eq!(
            published[1],
            serde_json::to_value(&source.courses[0]).unwrap()
        );
        assert_eq!(
            published[0]["position_x"].as_array().unwrap().len(),
            KEYFRAME_COUNT
        );
    }

    #[test]
    fn rejects_missing_or_malformed_current_geometry() {
        let missing = SourceGeometrySet {
            schema_version: SCHEMA_VERSION,
            source_master_version: "source-master".to_string(),
            courses: Vec::new(),
        };
        assert!(generate_from_source(&course_set(), &missing)
            .unwrap_err()
            .to_string()
            .contains("missing current course 10104"));

        let mut malformed_course = source_course();
        malformed_course.rotation_w.pop();
        let malformed = SourceGeometrySet {
            schema_version: SCHEMA_VERSION,
            source_master_version: "source-master".to_string(),
            courses: vec![malformed_course],
        };
        assert!(generate_from_source(&course_set(), &malformed)
            .unwrap_err()
            .to_string()
            .contains("rotation_w"));
    }

    #[test]
    fn ignores_source_geometry_for_future_courses() {
        let mut future_course = source_course();
        future_course.course_id = 11_203;
        future_course.race_track_id = 10_001;

        let source = SourceGeometrySet {
            schema_version: SCHEMA_VERSION,
            source_master_version: "newer-source-master".to_string(),
            courses: vec![source_course(), future_course],
        };

        let output = generate_from_source(&course_set(), &source).unwrap();

        let published = output.value["courses"].as_array().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0]["course_id"], 10_104);
    }

    #[test]
    fn bundled_source_decodes_all_current_keyframes() {
        let source = decode_bundled_source().unwrap();

        assert_eq!(source.schema_version, SCHEMA_VERSION);
        assert_eq!(source.courses.len(), 108);
        for course in source.courses {
            validate_source_course(&course).unwrap();
        }
    }
}

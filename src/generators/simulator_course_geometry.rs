use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;

use super::simulator_courses::SimulatorCourseSet;
use super::ResourceOutput;

const SCHEMA_VERSION: u32 = 1;
const KEYFRAME_COUNT: usize = 1_001;
const SOURCE_BYTES: &[u8] = include_bytes!("../jp_data/simulator_course_geometry.json.gz");

#[derive(Debug, Deserialize)]
struct SourceGeometrySet {
    schema_version: u32,
    source_master_version: String,
    courses: Vec<SourceCourseGeometry>,
}

#[derive(Debug, Deserialize)]
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
struct PublishedCourseGeometry<'a> {
    schema_version: u32,
    master_version: &'a str,
    course_id: u32,
    race_track_id: u32,
    course_distance: f64,
    source_asset: &'a str,
    position_x: &'a [f64],
    position_y: &'a [f64],
    position_z: &'a [f64],
    rotation_x: &'a [f64],
    rotation_y: &'a [f64],
    rotation_z: &'a [f64],
    rotation_w: &'a [f64],
}

/// Produces flat, independently fetchable geometry artifacts. The bundled
/// source payload originates from the client CourseLaneAnim assets; the
/// artifact master version deliberately records that source version rather
/// than pretending the client transforms were regenerated from master.mdb.
pub fn generate(courses: &SimulatorCourseSet<'_>) -> Result<Vec<ResourceOutput>> {
    let source = decode_bundled_source()?;
    generate_from_source(courses, &source)
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
) -> Result<Vec<ResourceOutput>> {
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

    let mut outputs = Vec::with_capacity(courses.courses.len());
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

        let artifact = PublishedCourseGeometry {
            schema_version: SCHEMA_VERSION,
            master_version: &source.source_master_version,
            course_id: source_course.course_id,
            race_track_id: source_course.race_track_id,
            course_distance: source_course.course_distance,
            source_asset: &source_course.source_asset,
            position_x: &source_course.position_x,
            position_y: &source_course.position_y,
            position_z: &source_course.position_z,
            rotation_x: &source_course.rotation_x,
            rotation_y: &source_course.rotation_y,
            rotation_z: &source_course.rotation_z,
            rotation_w: &source_course.rotation_w,
        };
        outputs.push(ResourceOutput {
            file_name: format!("simulator_course_geometry_{}.json", course.course_id),
            value: serde_json::to_value(artifact)?,
        });
    }

    Ok(outputs)
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
    };

    fn course_set() -> SimulatorCourseSet<'static> {
        SimulatorCourseSet {
            schema_version: 1,
            master_version: "current-master",
            courses: vec![SimulatorCourse {
                course_id: 10_104,
                race_track_id: 10_001,
                distance: 2_000,
                distance_type: 3,
                surface: 1,
                turn: 1,
                course: 1,
                lane_max: 12,
                lane_max_events: Vec::<CourseLaneMaxEvent>::new(),
                finish_time_min: 1,
                finish_time_max: 2,
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
    fn publishes_one_flat_resource_per_current_course() {
        let source = SourceGeometrySet {
            schema_version: SCHEMA_VERSION,
            source_master_version: "source-master".to_string(),
            courses: vec![source_course()],
        };

        let outputs = generate_from_source(&course_set(), &source).unwrap();

        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].file_name, "simulator_course_geometry_10104.json");
        assert_eq!(outputs[0].value["master_version"], "source-master");
        assert_eq!(outputs[0].value["course_id"], 10_104);
        assert_eq!(
            outputs[0].value["position_x"].as_array().unwrap().len(),
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

        let outputs = generate_from_source(&course_set(), &source).unwrap();

        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].file_name, "simulator_course_geometry_10104.json");
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

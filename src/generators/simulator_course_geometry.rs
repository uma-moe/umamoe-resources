use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read;

const SCHEMA_VERSION: u32 = 1;
const KEYFRAME_COUNT: usize = 1_001;
const SOURCE: &[u8] = include_bytes!("../jp_data/simulator_course_geometry.json.gz");

#[derive(Debug, Deserialize)]
struct GeometrySource {
    schema_version: u32,
    source_master_version: String,
    courses: Vec<SimulatorCourseGeometry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SimulatorCourseGeometry {
    schema_version: u32,
    master_version: String,
    course_id: u32,
    race_track_id: u32,
    course_distance: u16,
    source_asset: String,
    position_x: Vec<f32>,
    position_y: Vec<f32>,
    position_z: Vec<f32>,
    rotation_x: Vec<f32>,
    rotation_y: Vec<f32>,
    rotation_z: Vec<f32>,
    rotation_w: Vec<f32>,
}

pub fn version_hash() -> Result<String> {
    let json = decode_source_json()?;
    Ok(hex::encode(Sha256::digest(json)))
}

pub fn generate() -> Result<Vec<(String, SimulatorCourseGeometry)>> {
    let source = decode_source()?;
    validate_source(&source)?;
    Ok(source
        .courses
        .into_iter()
        .map(|course| {
            (
                format!("simulator_course_geometry_{}.json", course.course_id),
                course,
            )
        })
        .collect())
}

fn decode_source_json() -> Result<Vec<u8>> {
    let mut decoder = GzDecoder::new(SOURCE);
    let mut json = Vec::new();
    decoder
        .read_to_end(&mut json)
        .context("failed to decompress simulator course geometry source")?;
    Ok(json)
}

fn decode_source() -> Result<GeometrySource> {
    let json = decode_source_json()?;
    serde_json::from_slice(&json).context("failed to decode simulator course geometry source")
}

fn validate_source(source: &GeometrySource) -> Result<()> {
    if source.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported simulator course geometry source schema {}",
            source.schema_version
        );
    }
    if source.source_master_version.trim().is_empty() {
        bail!("simulator course geometry source master version is empty");
    }
    let mut course_ids = BTreeSet::new();
    for course in &source.courses {
        if course.schema_version != SCHEMA_VERSION {
            bail!(
                "course {} has unsupported geometry schema {}",
                course.course_id,
                course.schema_version
            );
        }
        if !course_ids.insert(course.course_id) {
            bail!("duplicate simulator course geometry {}", course.course_id);
        }
        if course.master_version != source.source_master_version {
            bail!(
                "course {} geometry master version does not match source",
                course.course_id
            );
        }
        if course.race_track_id == 0 || course.course_distance == 0 {
            bail!("course {} has invalid geometry identity", course.course_id);
        }
        if course.source_asset.trim().is_empty() {
            bail!("course {} has an empty source asset", course.course_id);
        }
        for (name, values) in [
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
                    "course {} geometry {name} has {} values instead of {KEYFRAME_COUNT}",
                    course.course_id,
                    values.len()
                );
            }
            if values.iter().any(|value| !value.is_finite()) {
                bail!(
                    "course {} geometry {name} contains a non-finite value",
                    course.course_id
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{decode_source, decode_source_json, validate_source, version_hash, KEYFRAME_COUNT};
    use sha2::{Digest, Sha256};

    #[test]
    fn bundled_source_is_complete_and_valid() {
        let source = decode_source().expect("decode bundled geometry source");
        validate_source(&source).expect("validate bundled geometry source");
        assert_eq!(source.courses.len(), 108);
        assert!(source
            .courses
            .iter()
            .all(|course| course.position_x.len() == KEYFRAME_COUNT));
    }
    #[test]
    fn version_hash_uses_canonical_json() {
        let json = decode_source_json().expect("decompress bundled geometry source");
        let expected = hex::encode(Sha256::digest(json));
        assert_eq!(version_hash().expect("hash geometry source"), expected);
    }
}

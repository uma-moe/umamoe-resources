//! Chibi and cut-in clips. The game code names few of them, so each keeps the part of its game
//! name that tells it apart: body clips go on the skeleton they move, a camera or position clip on
//! its one object, and the clips that move no bone become curves: a face's or ears' clip, which
//! the game plays through their controllers, and a prop's or set's clip, whose model is published
//! apart. The scripts that drive a face's expressions (`_driven`) become JSON too.
//!
//! A cut-in clip belongs to the timeline its folder is named after. A chibi clip in a
//! `chara/chr1001_00` folder belongs to that character; the others any chibi can play.

use super::catalog::objectName;
use super::types::{
    ClipKind, ClipOwner, GameClip, CHIBI_CLIP_FOLDER, CHIBI_SKELETON, CUTIN_CLIP_FOLDER,
    CUTIN_SKELETON,
};

/// The folders a chibi clip's role can have; the folders before it name its scene, those after
/// it its set.
const CHIBI_ROLES: &[&str] = &["body", "facial", "camera", "position", "prop"];

/// How a chibi or cut-in clip such as
/// `3d/motion/cutin/card/body/crd100101_001/anm_cti_crd100101_001_01` is published, or `None`
/// for other files.
pub(super) fn gameClip(name: &str) -> Option<GameClip<'_>> {
    let (skeleton, below) = clipFolder(name)?;
    let clip = objectName(below);
    if !clip.starts_with("anm_") {
        return None;
    }

    let folders: Vec<&str> = below.split('/').collect();
    let folders = &folders[..folders.len() - 1];
    let in_folder = |wanted: &str| folders.contains(&wanted);
    let kind = if clip.ends_with("_driven") {
        ClipKind::Script
    } else if clip.ends_with("_cam")
        || clip.ends_with("_pos")
        || in_folder("camera")
        || in_folder("position")
    {
        ClipKind::Node
    } else if in_folder("body") {
        ClipKind::Skeleton(skeleton)
    } else {
        ClipKind::Curves
    };

    if name.starts_with(CUTIN_CLIP_FOLDER) {
        cutinClip(kind, folders, clip)
    } else {
        chibiClip(kind, folders, clip)
    }
}

pub(super) fn clipExtension(kind: ClipKind) -> &'static str {
    match kind {
        ClipKind::Skeleton(_) | ClipKind::Node => "glb",
        ClipKind::Curves | ClipKind::Script => "json",
    }
}

pub(super) fn isChibiClip(name: &str) -> bool {
    name.starts_with(CHIBI_CLIP_FOLDER) && gameClip(name).is_some()
}

pub(super) fn isCutinClip(name: &str) -> bool {
    name.starts_with(CUTIN_CLIP_FOLDER) && gameClip(name).is_some()
}

/// `card/camera/crd100101_001/anm_cti_crd100101_001_01_cam`: category, role, timeline, clip.
fn cutinClip<'a>(kind: ClipKind, folders: &[&'a str], clip: &str) -> Option<GameClip<'a>> {
    let [_, role, timeline] = folders else {
        return None;
    };
    let unprefixed = clip
        .strip_prefix(&format!("anm_cti_{timeline}"))
        .map(|rest| rest.trim_start_matches('_'));
    let short_name = match unprefixed {
        Some("") => role,
        Some(rest) => rest,
        None => clip,
    };

    Some(GameClip {
        kind,
        owner: ClipOwner::Cutin { timeline },
        role,
        short_name: short_name.to_string(),
    })
}

/// `event/body/chara/chr1001_00/anm_min_eve_chr1001_00_idle01_e`: scene, role, set, clip.
fn chibiClip<'a>(kind: ClipKind, folders: &[&'a str], clip: &str) -> Option<GameClip<'a>> {
    let role_at = folders
        .iter()
        .position(|folder| CHIBI_ROLES.contains(folder))?;
    let (scene, rest) = folders.split_at(role_at);
    let (role, set) = (rest[0], &rest[1..]);
    let folder = scene
        .iter()
        .chain(set)
        .copied()
        .collect::<Vec<_>>()
        .join("/");
    // `anm_min_eve_`, the scene's abbreviation.
    let named = clip
        .strip_prefix("anm_min_")
        .and_then(|rest| rest.split_once('_'))
        .map_or(clip, |(_, rest)| rest);

    if let Some(chara) = characterFolder(set) {
        let short_name = named
            .strip_prefix(&format!("chr{chara:04}_"))
            .unwrap_or(named);
        return Some(GameClip {
            kind,
            owner: ClipOwner::Character { chara, folder },
            role,
            short_name: short_name.to_string(),
        });
    }

    // `type00_run02_base` in set `type00`, `0001_type00_catch01` in scene `minigame/mng_0001`.
    let mut short_name = named;
    for segment in scene.iter().skip(1).chain(set) {
        let number = segment
            .split_once('_')
            .map_or(*segment, |(_, number)| number);
        short_name = short_name
            .strip_prefix(&format!("{segment}_"))
            .or_else(|| short_name.strip_prefix(&format!("{number}_")))
            .unwrap_or(short_name);
    }
    Some(GameClip {
        kind,
        owner: ClipOwner::Library { folder },
        role,
        short_name: short_name.to_string(),
    })
}

/// The character of a `chara/chr1001_00` set.
fn characterFolder(set: &[&str]) -> Option<u32> {
    let ["chara", folder] = set else {
        return None;
    };
    let (chara, _) = folder.strip_prefix("chr")?.split_once('_')?;
    chara.parse().ok()
}

/// The skeleton of the clip's family and its path below the family's folder.
fn clipFolder(name: &str) -> Option<(&'static [&'static str], &str)> {
    if let Some(below) = name.strip_prefix(CHIBI_CLIP_FOLDER) {
        return Some((CHIBI_SKELETON, below));
    }
    if let Some(below) = name.strip_prefix(CUTIN_CLIP_FOLDER) {
        return Some((CUTIN_SKELETON, below));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(name: &str) -> Option<ClipKind> {
        gameClip(name).map(|clip| clip.kind)
    }

    fn shortName(name: &str) -> Option<String> {
        gameClip(name).map(|clip| clip.short_name)
    }

    #[test]
    fn bodyClipsGoOnTheirFamilysSkeleton() {
        let cutin = kind("3d/motion/cutin/card/body/crd100101_001/anm_cti_crd100101_001_01");
        let chibi = kind("3d/motion/mini/event/body/type01/anm_min_eve_type01_run02_base");

        assert_eq!(cutin, Some(ClipKind::Skeleton(CUTIN_SKELETON)));
        assert_eq!(chibi, Some(ClipKind::Skeleton(CHIBI_SKELETON)));
    }

    #[test]
    fn cameraAndPositionClipsMoveOneObject() {
        let camera = kind("3d/motion/cutin/card/camera/crd100101_001/anm_cti_crd100101_001_01_cam");
        let position =
            kind("3d/motion/cutin/card/position/crd100101_001/anm_cti_crd100101_001_01_pos");

        assert_eq!(camera, Some(ClipKind::Node));
        assert_eq!(position, Some(ClipKind::Node));
    }

    #[test]
    fn propFaceAndEarClipsAreCurvesAndDrivenFacesScripts() {
        let prop = kind("3d/motion/cutin/card/prop/crd100101_001/anm_cti_crd100101_001_01_prop");
        let face = kind("3d/motion/cutin/card/facial/crd100101_001/anm_cti_crd100101_001_01_face");
        let ear = kind("3d/motion/mini/event/facial/type01/anm_min_eve_type01_run02_base_ear");
        let driven =
            kind("3d/motion/cutin/card/facial/crd100101_001/anm_cti_crd100101_001_01_face_driven");

        assert_eq!(prop, Some(ClipKind::Curves));
        assert_eq!(face, Some(ClipKind::Curves));
        assert_eq!(ear, Some(ClipKind::Curves));
        assert_eq!(driven, Some(ClipKind::Script));
        assert_eq!(clipExtension(ClipKind::Curves), "json");
    }

    #[test]
    fn aCutinClipBelongsToItsTimelineAndDropsTheTimelinesName() {
        let camera =
            gameClip("3d/motion/cutin/card/camera/crd100101_001/anm_cti_crd100101_001_01_cam")
                .unwrap();

        assert_eq!(
            camera.owner,
            ClipOwner::Cutin {
                timeline: "crd100101_001"
            }
        );
        assert_eq!(camera.role, "camera");
        assert_eq!(camera.short_name, "01_cam");
    }

    #[test]
    fn aClipNamedOnlyByItsTimelineIsNamedByItsRole() {
        let body = shortName("3d/motion/cutin/chara/body/chr1001_001/anm_cti_chr1001_001");
        let camera = shortName("3d/motion/cutin/chara/camera/chr1001_001/anm_cti_chr1001_001_cam");

        assert_eq!(body.as_deref(), Some("body"));
        assert_eq!(camera.as_deref(), Some("cam"));
    }

    #[test]
    fn aCharactersChibiClipKeepsItsOutfitNumber() {
        let clip =
            gameClip("3d/motion/mini/event/body/chara/chr1001_00/anm_min_eve_chr1001_30_res01_e")
                .unwrap();

        assert_eq!(
            clip.owner,
            ClipOwner::Character {
                chara: 1001,
                folder: "event/chara/chr1001_00".to_string()
            }
        );
        assert_eq!(clip.short_name, "30_res01_e");
    }

    #[test]
    fn aSharedChibiClipDropsItsSceneAndSet() {
        let run =
            gameClip("3d/motion/mini/event/body/type00/anm_min_eve_type00_run02_base").unwrap();
        let catch = shortName(
            "3d/motion/mini/minigame/mng_0001/body/type00/anm_min_mng_0001_type00_catch01",
        );
        let circle = gameClip("3d/motion/mini/circle/position/anm_min_cir_pos01").unwrap();

        assert_eq!(
            run.owner,
            ClipOwner::Library {
                folder: "event/type00".to_string()
            }
        );
        assert_eq!(run.short_name, "run02_base");
        assert_eq!(catch.as_deref(), Some("catch01"));
        assert_eq!(
            circle.owner,
            ClipOwner::Library {
                folder: "circle".to_string()
            }
        );
        assert_eq!(circle.short_name, "pos01");
    }

    #[test]
    fn filesThatAreNotClipsAreLeftOut() {
        assert_eq!(kind("3d/motion/cutin/cmn"), None);
        assert_eq!(
            kind("3d/motion/racemain/body/type01/anm_rac_type01_run03_base"),
            None
        );
    }
}

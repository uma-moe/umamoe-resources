//! Chibi and cut-in clips. The game code names few of them, so each is served at its game path,
//! its game name kept: body clips on the skeleton they move, a camera or position clip on its one
//! object, and as curves the clips that move no bone: a face's or ears' clip, which the game plays
//! through their controllers, and a prop's or set's clip, whose model is published apart. The
//! scripts that drive a face's expressions (`_driven`) become JSON too.

use super::catalog::objectName;
use super::types::{
    ClipKind, GameClip, CHIBI_CLIP_FOLDER, CHIBI_SKELETON, CUTIN_CLIP_FOLDER, CUTIN_SKELETON,
};

/// How a chibi or cut-in clip such as
/// `3d/motion/cutin/card/body/crd100101_001/anm_cti_crd100101_001_01` is published, or `None`
/// for other files.
pub(super) fn gameClip(name: &str) -> Option<GameClip> {
    let (skeleton, below) = clipFolder(name)?;
    let clip = objectName(below);
    if !clip.starts_with("anm_") {
        return None;
    }

    let folders: Vec<&str> = below.split('/').collect();
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

    let extension = match kind {
        ClipKind::Skeleton(_) | ClipKind::Node => "glb",
        ClipKind::Curves | ClipKind::Script => "json",
    };
    Some(GameClip {
        key: format!("game-assets/{name}.{extension}"),
        kind,
    })
}

pub(super) fn isChibiClip(name: &str) -> bool {
    name.starts_with(CHIBI_CLIP_FOLDER) && gameClip(name).is_some()
}

pub(super) fn isCutinClip(name: &str) -> bool {
    name.starts_with(CUTIN_CLIP_FOLDER) && gameClip(name).is_some()
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
        let prop =
            gameClip("3d/motion/cutin/card/prop/crd100101_001/anm_cti_crd100101_001_01_prop");
        let face = kind("3d/motion/cutin/card/facial/crd100101_001/anm_cti_crd100101_001_01_face");
        let ear = kind("3d/motion/mini/event/facial/type01/anm_min_eve_type01_run02_base_ear");
        let driven = gameClip(
            "3d/motion/cutin/card/facial/crd100101_001/anm_cti_crd100101_001_01_face_driven",
        );

        assert_eq!(
            prop,
            Some(GameClip {
                key: "game-assets/3d/motion/cutin/card/prop/crd100101_001/anm_cti_crd100101_001_01_prop.json".to_string(),
                kind: ClipKind::Curves,
            })
        );
        assert_eq!(face, Some(ClipKind::Curves));
        assert_eq!(ear, Some(ClipKind::Curves));
        assert_eq!(driven.map(|clip| clip.kind), Some(ClipKind::Script));
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

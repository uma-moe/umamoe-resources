//! The chibis built from master.mdb's outfits: every outfit with a chibi and a body of its own
//! becomes one model of its chibi body, the face every chibi shares, its hair and its tail, served
//! at its uma's path. The paths follow the client's `MINI_BODY_SPECIAL_DRESS_MODEL_PATH`,
//! `MINI_HAIR_MODEL_PATH` and `MINI_TAIL_MODEL_PATH`; the face's sheets follow its face and head
//! texture paths.

use super::types::{
    CharacterModel, ChibiOutfit, MaterialTexture, OutfitRow, OutfitTraits, CHIBI_BODY_FOLDER,
    CHIBI_FACE_FOLDER, CHIBI_FACE_PREFAB, CHIBI_FACE_SHEETS, CHIBI_MODEL_FOLDER, CHIBI_SKIN_SHEETS,
};
use super::umas::{outfitPaths, outfitTitle, tailModel};

/// A chibi's record, its body's folder such as `3d/chara/mini/body/mbdy1001_00`.
pub(super) fn isChibiRecord(name: &str) -> bool {
    name.strip_prefix(CHIBI_BODY_FOLDER)
        .is_some_and(|folder| !folder.is_empty() && !folder.contains('/'))
}

/// The chibi of every outfit that has one: `game-assets/chibis/special-week/special-dreamer.glb`.
pub(super) fn chibiOutfits(rows: &[OutfitRow]) -> Vec<ChibiOutfit> {
    rows.iter()
        .zip(outfitPaths(rows))
        .filter(|(row, _)| row.has_chibi)
        .map(|(row, path)| chibiOutfit(row, format!("{CHIBI_MODEL_FOLDER}/{path}.glb")))
        .collect()
}

fn chibiOutfit(row: &OutfitRow, key: String) -> ChibiOutfit {
    let chara = row.chara;
    let body = format!("mbdy{chara:04}_{:02}", row.body_sub);
    let head = format!("mchr{chara:04}_{:02}", row.head_sub);
    let record = format!("{CHIBI_BODY_FOLDER}{body}");
    let head_folder = format!("3d/chara/mini/head/{head}");
    let mut prefabs = vec![
        format!("{record}/pfb_{body}"),
        CHIBI_FACE_PREFAB.to_string(),
        format!("{head_folder}/pfb_{head}_hair"),
    ];

    let texture = |material: &str, name: String| MaterialTexture {
        material: material.to_string(),
        slot: "_MainTex",
        name,
    };
    let skin = row.skin;
    let skin_sheets = CHIBI_SKIN_SHEETS.iter().map(|(material, suffix)| {
        let name = format!("{CHIBI_FACE_FOLDER}/textures/tex_mchr0001_00_face0_{skin}_{suffix}");
        texture(material, name)
    });
    let face_sheets = CHIBI_FACE_SHEETS.iter().map(|(material, suffix)| {
        texture(
            material,
            format!("{head_folder}/textures/tex_{head}_{suffix}"),
        )
    });
    let mut material_textures: Vec<MaterialTexture> = skin_sheets.chain(face_sheets).collect();

    // The tail's material is left empty; the game loads the character's colour into it.
    if let Some((model, variant)) = tailModel(row) {
        let tail = format!("mtail{model:04}_{variant:02}");
        let tail_folder = format!("3d/chara/mini/tail/{tail}");
        prefabs.push(format!("{tail_folder}/pfb_{tail}"));
        material_textures.push(texture(
            &format!("mtl_{tail}"),
            format!("{tail_folder}/textures/tex_{tail}_{chara:04}_diff"),
        ));
    }

    ChibiOutfit {
        record,
        key,
        model: CharacterModel {
            title: outfitTitle(row),
            prefabs,
            material_textures,
            traits: Some(OutfitTraits {
                chara,
                dress: row.dress,
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specialWeek() -> OutfitRow {
        OutfitRow {
            dress: 100101,
            chara: 1001,
            has_chibi: true,
            body_sub: 0,
            head_sub: 0,
            dress_tail: (-1, 0),
            chara_tail: 1,
            skin: 1,
            running_type: 1,
            run_personality: 1,
            finish_personality: 1,
            chara_name: Some("Special Week".to_string()),
            dress_name: Some("Special Dreamer".to_string()),
        }
    }

    fn filled(chibi: &ChibiOutfit) -> Vec<(String, String)> {
        chibi
            .model
            .material_textures
            .iter()
            .map(|texture| (texture.material.clone(), texture.name.clone()))
            .collect()
    }

    #[test]
    fn aChibiJoinsItsBodyTheSharedFaceItsHairAndItsCharactersTail() {
        let chibi = &chibiOutfits(&[specialWeek()])[0];

        assert_eq!(chibi.record, "3d/chara/mini/body/mbdy1001_00");
        assert_eq!(
            chibi.key,
            "game-assets/chibis/special-week/special-dreamer.glb"
        );
        assert_eq!(
            chibi.model.prefabs,
            vec![
                "3d/chara/mini/body/mbdy1001_00/pfb_mbdy1001_00",
                "3d/chara/mini/head/mchr0001_00/pfb_mchr0001_00_face0",
                "3d/chara/mini/head/mchr1001_00/pfb_mchr1001_00_hair",
                "3d/chara/mini/tail/mtail0001_00/pfb_mtail0001_00",
            ]
        );
    }

    #[test]
    fn theFaceWearsTheCharactersSkinAndSheetsAndTheTailItsColour() {
        let chibi = &chibiOutfits(&[specialWeek()])[0];

        let head = "3d/chara/mini/head/mchr1001_00/textures/tex_mchr1001_00";
        let face = "3d/chara/mini/head/mchr0001_00/textures/tex_mchr0001_00_face0_1";
        let tail = "3d/chara/mini/tail/mtail0001_00/textures/tex_mtail0001_00_1001_diff";
        let expected: Vec<(String, String)> = [
            ("mtl_mchr0001_00_face0", format!("{face}_diff")),
            ("mtl_mchr0001_00_cheek", format!("{face}_cheek0")),
            ("mtl_mchr0001_00_eye", format!("{head}_eye_diff")),
            ("mtl_mchr0001_00_mayu_l", format!("{head}_mayu_diff")),
            ("mtl_mchr0001_00_mayu_r", format!("{head}_mayu_diff")),
            ("mtl_mchr0001_00_mouth", format!("{head}_mouth_diff")),
            ("mtl_mtail0001_00", tail.to_string()),
        ]
        .into_iter()
        .map(|(material, name)| (material.to_string(), name))
        .collect();
        assert_eq!(filled(chibi), expected);
    }

    #[test]
    fn anOutfitWithoutAChibiHasNone() {
        let row = OutfitRow {
            has_chibi: false,
            ..specialWeek()
        };

        assert!(chibiOutfits(&[row]).is_empty());
    }
}

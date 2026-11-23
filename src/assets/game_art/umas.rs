//! The umas built from master.mdb's outfits: every outfit with a body of its own becomes one
//! model of its body, head and tail.

use super::layout::{npcUmaFolder, umaKey};
use super::race_motions::personalityFolder;
use super::types::{
    OutfitRow, SlotTexture, UmaOutfit, UmaTraits, CHARA_NAME_CATEGORY, DRESS_NAME_CATEGORY,
    TAIL_TEXTURES, UMA_BODY_FOLDER,
};
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::HashMap;

/// A uma's record, its body's folder such as `3d/chara/body/bdy1001_00`.
pub(super) fn isUmaRecord(name: &str) -> bool {
    name.strip_prefix(UMA_BODY_FOLDER)
        .is_some_and(|folder| !folder.is_empty() && !folder.contains('/'))
}

/// Each outfit worn by one character, which gives its body. `chara_type` holds a character's
/// race personalities: target scene 1, cut 1 for runs and starts and cut 4 for finishes; the
/// game falls back to the first personality without one.
pub(super) fn outfitRows(connection: &Connection) -> Result<Vec<OutfitRow>> {
    let mut statement = connection.prepare(
        "SELECT dress.id, dress.chara_id, dress.have_mini, dress.body_type_sub, dress.head_sub_id,
                dress.tail_model_id, dress.tail_model_sub_id, chara.tail_model_id,
                chara.race_running_type, COALESCE(run.value, 1), COALESCE(finish.value, 1),
                chara_name.text, dress_name.text, chara.skin
         FROM dress_data dress
         JOIN chara_data chara ON chara.id = dress.chara_id
         LEFT JOIN chara_type run ON run.chara_id = dress.chara_id
           AND run.target_scene = 1 AND run.target_cut = 1 AND run.target_type = 0
         LEFT JOIN chara_type finish ON finish.chara_id = dress.chara_id
           AND finish.target_scene = 1 AND finish.target_cut = 4 AND finish.target_type = 0
         LEFT JOIN text_data chara_name ON chara_name.category = ?1
           AND chara_name.\"index\" = dress.chara_id
         LEFT JOIN text_data dress_name ON dress_name.category = ?2
           AND dress_name.\"index\" = dress.id
         WHERE dress.chara_id > 0
         ORDER BY dress.id",
    )?;
    let rows = statement.query_map([CHARA_NAME_CATEGORY, DRESS_NAME_CATEGORY], |row| {
        Ok(OutfitRow {
            dress: row.get(0)?,
            chara: row.get(1)?,
            has_chibi: row.get::<_, i64>(2)? == 1,
            body_sub: row.get(3)?,
            head_sub: row.get(4)?,
            dress_tail: (row.get(5)?, row.get(6)?),
            chara_tail: row.get(7)?,
            running_type: row.get(8)?,
            run_personality: row.get(9)?,
            finish_personality: row.get(10)?,
            chara_name: row.get(11)?,
            dress_name: row.get(12)?,
            skin: row.get(13)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read outfits from master.mdb")
}

/// The uma of every outfit, served as `model.glb` in its uma folder: `art/umas/100101/model.glb`.
pub(super) fn umaOutfits(rows: &[OutfitRow], folders: &HashMap<u32, String>) -> Vec<UmaOutfit> {
    rows.iter()
        .map(|row| umaOutfit(row, umaKey(&outfitFolder(row, folders), "model.glb")))
        .collect()
}

/// The outfit's folder below `art/umas/`.
pub(super) fn outfitFolder(row: &OutfitRow, folders: &HashMap<u32, String>) -> String {
    folders
        .get(&row.dress)
        .cloned()
        .unwrap_or_else(|| npcUmaFolder(row.chara, row.dress))
}

/// `Special Week: Special Dreamer`, what Blender names an outfit's armature.
pub(super) fn outfitTitle(row: &OutfitRow) -> String {
    umaTitle(&charaName(row), &dressName(row))
}

fn umaOutfit(row: &OutfitRow, key: String) -> UmaOutfit {
    let chara = row.chara;
    let body = format!("bdy{chara:04}_{:02}", row.body_sub);
    let head = format!("chr{chara:04}_{:02}", row.head_sub);
    let record = format!("{UMA_BODY_FOLDER}{body}");
    let mut prefabs = vec![
        format!("{record}/pfb_{body}"),
        format!("3d/chara/head/{head}/pfb_{head}"),
    ];

    let mut tail_textures = Vec::new();
    if let Some(tail) = tailFolder(row) {
        prefabs.push(format!("3d/chara/tail/{tail}/pfb_{tail}"));
        let chara_digits = format!("{chara:04}");
        tail_textures = TAIL_TEXTURES
            .iter()
            .map(|(slot, suffix)| SlotTexture {
                slot,
                name: format!(
                    "3d/chara/tail/{tail}/textures/tex_{tail}_{}",
                    suffix.replace("{chara}", &chara_digits)
                ),
            })
            .collect();
    }

    UmaOutfit {
        record,
        prefabs,
        tail_textures,
        title: outfitTitle(row),
        key,
        traits: UmaTraits {
            chara,
            dress: row.dress,
            run_motions: motionFolder(row.run_personality),
            finish_motions: motionFolder(row.finish_personality),
            running_form: runningFormName(row.running_type),
        },
    }
}

fn tailFolder(row: &OutfitRow) -> Option<String> {
    tailModel(row).map(|(model, variant)| format!("tail{model:04}_{variant:02}"))
}

/// The outfit's own tail model and variant, else its character's common one; staff have none.
/// Chibis number their tails the same way.
pub(super) fn tailModel(row: &OutfitRow) -> Option<(i64, i64)> {
    let (dress_tail, dress_variant) = row.dress_tail;
    if dress_tail >= 0 {
        return Some((dress_tail, dress_variant));
    }
    if row.chara_tail >= 0 {
        return Some((row.chara_tail, 0));
    }
    None
}

fn charaName(row: &OutfitRow) -> String {
    row.chara_name
        .clone()
        .unwrap_or_else(|| format!("chr{:04}", row.chara))
}

fn dressName(row: &OutfitRow) -> String {
    row.dress_name
        .clone()
        .unwrap_or_else(|| format!("dress{}", row.dress))
}

/// Outfit names often name their character, `Mejiro Palmer's Casual Wear`; the others get it in
/// front, `Special Week: Special Dreamer`.
fn umaTitle(chara_name: &str, dress_name: &str) -> String {
    if dress_name.contains(chara_name) {
        dress_name.to_string()
    } else {
        format!("{chara_name}: {dress_name}")
    }
}

fn motionFolder(personality: i64) -> String {
    personalityFolder(personality).unwrap_or_else(|| "default".to_string())
}

/// The form named in the run clips it uses, as `race_motions` names them.
fn runningFormName(running_type: i64) -> &'static str {
    match running_type {
        2 => "pitch",
        3 => "stride",
        _ => "standard",
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

    fn folders() -> HashMap<u32, String> {
        HashMap::from([(100101, "100101".to_string())])
    }

    #[test]
    fn anOutfitJoinsItsBodyHeadAndItsCharactersTail() {
        let uma = &umaOutfits(&[specialWeek()], &folders())[0];

        assert_eq!(uma.record, "3d/chara/body/bdy1001_00");
        assert_eq!(
            uma.prefabs,
            vec![
                "3d/chara/body/bdy1001_00/pfb_bdy1001_00",
                "3d/chara/head/chr1001_00/pfb_chr1001_00",
                "3d/chara/tail/tail0001_00/pfb_tail0001_00",
            ]
        );
        assert_eq!(uma.key, "art/umas/100101/model.glb");
        assert_eq!(uma.title, "Special Week: Special Dreamer");
    }

    #[test]
    fn theTailIsColouredForItsCharacter() {
        let uma = &umaOutfits(&[specialWeek()], &folders())[0];

        let textures: Vec<(&str, &str)> = uma
            .tail_textures
            .iter()
            .map(|texture| (texture.slot, texture.name.as_str()))
            .collect();
        let folder = "3d/chara/tail/tail0001_00/textures";
        assert_eq!(
            textures,
            vec![
                (
                    "_MainTex",
                    format!("{folder}/tex_tail0001_00_1001_diff").as_str()
                ),
                (
                    "_ToonMap",
                    format!("{folder}/tex_tail0001_00_1001_shad_c").as_str()
                ),
                (
                    "_TripleMaskMap",
                    format!("{folder}/tex_tail0001_00_0000_base").as_str()
                ),
                (
                    "_OptionMaskMap",
                    format!("{folder}/tex_tail0001_00_0000_ctrl").as_str()
                ),
            ]
        );
    }

    #[test]
    fn anOutfitWithItsOwnTailWearsIt() {
        let row = OutfitRow {
            dress: 101330,
            chara: 1013,
            body_sub: 30,
            head_sub: 30,
            dress_tail: (1013, 30),
            ..specialWeek()
        };

        let uma = &umaOutfits(&[row], &folders())[0];

        assert_eq!(uma.prefabs[2], "3d/chara/tail/tail1013_30/pfb_tail1013_30");
        assert_eq!(
            uma.tail_textures[0].name,
            "3d/chara/tail/tail1013_30/textures/tex_tail1013_30_1013_diff"
        );
    }

    #[test]
    fn charactersWithoutATailGetNone() {
        let row = OutfitRow {
            chara_tail: -1,
            dress_tail: (-1, -1),
            ..specialWeek()
        };

        let uma = &umaOutfits(&[row], &folders())[0];

        assert_eq!(uma.prefabs.len(), 2);
        assert!(uma.tail_textures.is_empty());
    }

    #[test]
    fn outfitNamesThatNameTheCharacterStandAlone() {
        let row = OutfitRow {
            dress: 901064,
            chara: 1064,
            body_sub: 90,
            chara_name: Some("Mejiro Palmer".to_string()),
            dress_name: Some("Mejiro Palmer's Casual Wear".to_string()),
            ..specialWeek()
        };

        let uma = &umaOutfits(&[row], &folders())[0];

        assert_eq!(uma.title, "Mejiro Palmer's Casual Wear");
        assert_eq!(uma.key, "art/umas/1064-901064/model.glb");
    }

    #[test]
    fn traitsNameTheMotionFilesTheUmaUses() {
        let row = OutfitRow {
            running_type: 3,
            run_personality: 2,
            finish_personality: 3,
            ..specialWeek()
        };

        let traits = umaOutfits(&[row], &folders())[0].traits.clone();

        assert_eq!(traits.run_motions, "sporty");
        assert_eq!(traits.finish_motions, "mature");
        assert_eq!(traits.running_form, "stride");
    }

    #[test]
    fn outfitsComeWithTheirCharactersTailPersonalityAndNames() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE dress_data (id INTEGER, chara_id INTEGER, have_mini INTEGER,
                   body_type_sub INTEGER, head_sub_id INTEGER, tail_model_id INTEGER,
                   tail_model_sub_id INTEGER);
                 CREATE TABLE chara_data (id INTEGER, tail_model_id INTEGER,
                   race_running_type INTEGER, skin INTEGER);
                 CREATE TABLE chara_type (id TEXT, chara_id INTEGER, target_scene INTEGER,
                   target_cut INTEGER, target_type INTEGER, value INTEGER);
                 CREATE TABLE text_data (category INTEGER, \"index\" INTEGER, text TEXT);
                 INSERT INTO dress_data VALUES (100101, 1001, 1, 0, 0, -1, 0),
                   (1, 0, 0, 0, 0, -1, 0), (100201, 1002, 0, 0, 0, -1, 0);
                 INSERT INTO chara_data VALUES (1001, 1, 2, 1), (1002, 2, 1, 0);
                 INSERT INTO chara_type VALUES ('1', 1001, 1, 1, 0, 3), ('2', 1001, 1, 4, 0, 4),
                   ('3', 1001, 1, 2, 0, 2);
                 INSERT INTO text_data VALUES (6, 1001, 'Special Week'),
                   (14, 100101, 'Special Dreamer');",
            )
            .unwrap();

        let rows = outfitRows(&connection).unwrap();

        let outfits: Vec<(u32, bool, i64, i64, i64, Option<&str>)> = rows
            .iter()
            .map(|row| {
                (
                    row.dress,
                    row.has_chibi,
                    row.chara_tail,
                    row.run_personality,
                    row.finish_personality,
                    row.dress_name.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            outfits,
            vec![
                (100101, true, 1, 3, 4, Some("Special Dreamer")),
                (100201, false, 2, 1, 1, None),
            ]
        );
    }

    #[test]
    fn onlyBodyFoldersAreUmaRecords() {
        assert!(isUmaRecord("3d/chara/body/bdy1001_00"));
        assert!(!isUmaRecord("3d/chara/body/bdy1001_00/pfb_bdy1001_00"));
        assert!(!isUmaRecord("3d/chara/body/"));
    }
}

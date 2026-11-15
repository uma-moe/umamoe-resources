//! The umas built from master.mdb's outfits: every outfit with a body of its own becomes one
//! model of its body, head and tail.

use super::catalog::fileSlug;
use super::race_motions::personalityFolder;
use super::types::{
    OutfitRow, SlotTexture, UmaOutfit, UmaTraits, CHARA_NAME_CATEGORY, DRESS_NAME_CATEGORY,
    TAIL_TEXTURES, UMA_BODY_FOLDER, UMA_MODEL_FOLDER,
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
        "SELECT dress.id, dress.chara_id, dress.body_type_sub, dress.head_sub_id,
                dress.tail_model_id, dress.tail_model_sub_id, chara.tail_model_id,
                chara.race_running_type, COALESCE(run.value, 1), COALESCE(finish.value, 1),
                chara_name.text, dress_name.text
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
            body_sub: row.get(2)?,
            head_sub: row.get(3)?,
            dress_tail: (row.get(4)?, row.get(5)?),
            chara_tail: row.get(6)?,
            running_type: row.get(7)?,
            run_personality: row.get(8)?,
            finish_personality: row.get(9)?,
            chara_name: row.get(10)?,
            dress_name: row.get(11)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read outfits from master.mdb")
}

/// The uma of every outfit, served under its character and outfit names:
/// `game-assets/umas/special-week/special-dreamer.glb`. Outfits of one character whose names
/// read alike keep their outfit number as well.
pub(super) fn umaOutfits(rows: &[OutfitRow]) -> Vec<UmaOutfit> {
    let names: Vec<String> = rows.iter().map(namedPath).collect();
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for name in &names {
        *uses.entry(name.as_str()).or_default() += 1;
    }

    rows.iter()
        .zip(&names)
        .map(|(row, name)| {
            let is_shared = uses[name.as_str()] > 1;
            let path = if is_shared {
                format!("{name}-{}", row.dress)
            } else {
                name.clone()
            };
            umaOutfit(row, format!("{UMA_MODEL_FOLDER}/{path}.glb"))
        })
        .collect()
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
        title: umaTitle(&charaName(row), &dressName(row)),
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

/// The outfit's own tail, else its character's common one; staff have none.
fn tailFolder(row: &OutfitRow) -> Option<String> {
    let (dress_tail, dress_variant) = row.dress_tail;
    if dress_tail >= 0 {
        return Some(format!("tail{dress_tail:04}_{dress_variant:02}"));
    }
    if row.chara_tail >= 0 {
        return Some(format!("tail{:04}_00", row.chara_tail));
    }
    None
}

/// `special-week/special-dreamer`, without the outfit number that tells alike names apart.
fn namedPath(row: &OutfitRow) -> String {
    format!(
        "{}/{}",
        fileSlug(&charaName(row)),
        fileSlug(&dressName(row))
    )
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
            body_sub: 0,
            head_sub: 0,
            dress_tail: (-1, 0),
            chara_tail: 1,
            running_type: 1,
            run_personality: 1,
            finish_personality: 1,
            chara_name: Some("Special Week".to_string()),
            dress_name: Some("Special Dreamer".to_string()),
        }
    }

    #[test]
    fn anOutfitJoinsItsBodyHeadAndItsCharactersTail() {
        let uma = &umaOutfits(&[specialWeek()])[0];

        assert_eq!(uma.record, "3d/chara/body/bdy1001_00");
        assert_eq!(
            uma.prefabs,
            vec![
                "3d/chara/body/bdy1001_00/pfb_bdy1001_00",
                "3d/chara/head/chr1001_00/pfb_chr1001_00",
                "3d/chara/tail/tail0001_00/pfb_tail0001_00",
            ]
        );
        assert_eq!(uma.key, "game-assets/umas/special-week/special-dreamer.glb");
        assert_eq!(uma.title, "Special Week: Special Dreamer");
    }

    #[test]
    fn theTailIsColouredForItsCharacter() {
        let uma = &umaOutfits(&[specialWeek()])[0];

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

        let uma = &umaOutfits(&[row])[0];

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

        let uma = &umaOutfits(&[row])[0];

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

        let uma = &umaOutfits(&[row])[0];

        assert_eq!(uma.title, "Mejiro Palmer's Casual Wear");
        assert_eq!(
            uma.key,
            "game-assets/umas/mejiro-palmer/mejiro-palmers-casual-wear.glb"
        );
    }

    #[test]
    fn alikeOutfitNamesKeepTheirNumbers() {
        let summer = OutfitRow {
            dress: 100140,
            body_sub: 40,
            dress_name: Some("Summer!".to_string()),
            ..specialWeek()
        };
        let other_summer = OutfitRow {
            dress: 100150,
            body_sub: 50,
            dress_name: Some("Summer?".to_string()),
            ..specialWeek()
        };

        let umas = umaOutfits(&[summer, other_summer]);

        let keys: Vec<&str> = umas.iter().map(|uma| uma.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "game-assets/umas/special-week/summer-100140.glb",
                "game-assets/umas/special-week/summer-100150.glb",
            ]
        );
    }

    #[test]
    fn traitsNameTheMotionFilesTheUmaUses() {
        let row = OutfitRow {
            running_type: 3,
            run_personality: 2,
            finish_personality: 3,
            ..specialWeek()
        };

        let traits = umaOutfits(&[row])[0].traits.clone();

        assert_eq!(traits.run_motions, "sporty");
        assert_eq!(traits.finish_motions, "mature");
        assert_eq!(traits.running_form, "stride");
    }

    #[test]
    fn outfitsComeWithTheirCharactersTailPersonalityAndNames() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE dress_data (id INTEGER, chara_id INTEGER, body_type_sub INTEGER,
                   head_sub_id INTEGER, tail_model_id INTEGER, tail_model_sub_id INTEGER);
                 CREATE TABLE chara_data (id INTEGER, tail_model_id INTEGER,
                   race_running_type INTEGER);
                 CREATE TABLE chara_type (id TEXT, chara_id INTEGER, target_scene INTEGER,
                   target_cut INTEGER, target_type INTEGER, value INTEGER);
                 CREATE TABLE text_data (category INTEGER, \"index\" INTEGER, text TEXT);
                 INSERT INTO dress_data VALUES (100101, 1001, 0, 0, -1, 0), (1, 0, 0, 0, -1, 0),
                   (100201, 1002, 0, 0, -1, 0);
                 INSERT INTO chara_data VALUES (1001, 1, 2), (1002, 2, 1);
                 INSERT INTO chara_type VALUES ('1', 1001, 1, 1, 0, 3), ('2', 1001, 1, 4, 0, 4),
                   ('3', 1001, 1, 2, 0, 2);
                 INSERT INTO text_data VALUES (6, 1001, 'Special Week'),
                   (14, 100101, 'Special Dreamer');",
            )
            .unwrap();

        let rows = outfitRows(&connection).unwrap();

        let outfits: Vec<(u32, i64, i64, i64, Option<&str>)> = rows
            .iter()
            .map(|row| {
                (
                    row.dress,
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
                (100101, 1, 3, 4, Some("Special Dreamer")),
                (100201, 2, 1, 1, None),
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

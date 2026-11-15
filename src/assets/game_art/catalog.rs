use super::types::{
    ArtFamily, ArtOutput, AtlasPlan, AtlasSprites, CardOutfit, Catalog, Fit, PlannedOutput,
    ART_FAMILIES, ATLAS_FAMILIES, PORTRAIT_OUTPUTS, PORTRAIT_SOURCE,
};
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::path::Path;

impl Catalog {
    /// The fixed families and atlases plus a portrait for every card in this master.
    pub(super) fn load(master: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(master, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("failed to open {}", master.display()))?;
        Ok(Self::fromOutfits(&cardOutfits(&connection)?))
    }

    pub(super) fn fromOutfits(outfits: &[CardOutfit]) -> Self {
        let mut portraits: HashMap<String, Vec<PlannedOutput>> = HashMap::new();
        for outfit in outfits {
            let chara = outfit.chara.to_string();
            let card = outfit.card.to_string();
            let dress = outfit.dress.to_string();
            let file = fillPattern(PORTRAIT_SOURCE, &[("chara", &chara), ("dress", &dress)]);
            let captures = [("chara", chara.as_str()), ("card", card.as_str())];
            let outputs = plannedOutputs(PORTRAIT_OUTPUTS, &captures, objectName(&file));
            portraits.entry(file).or_default().extend(outputs);
        }

        let atlases = ATLAS_FAMILIES
            .iter()
            .map(|family| {
                let plan = AtlasPlan {
                    texture: family.texture,
                    outputs: family.sprites.iter().flat_map(spriteOutputs).collect(),
                };
                (family.atlas, plan)
            })
            .collect();

        Self { portraits, atlases }
    }

    /// The bucket keys a game file is served at, or `None` when the site does not use it.
    pub(super) fn outputs(&self, name: &str) -> Option<Vec<PlannedOutput>> {
        ART_FAMILIES
            .iter()
            .find_map(|family| familyOutputs(family, name))
            .or_else(|| self.portraits.get(name).cloned())
            .or_else(|| self.atlases.get(name).map(|plan| plan.outputs.clone()))
    }

    /// The bundle a game file needs loaded beside it, such as an atlas's texture.
    pub(super) fn dependency(&self, name: &str) -> Option<&'static str> {
        self.atlases.get(name).map(|plan| plan.texture)
    }
}

/// The CDN categories the catalog reads from: the first segment of each game path.
pub(super) fn catalogCategories() -> Vec<&'static str> {
    let mut categories: Vec<&str> = ART_FAMILIES
        .iter()
        .map(|family| family.source)
        .chain([PORTRAIT_SOURCE])
        .chain(ATLAS_FAMILIES.iter().map(|family| family.atlas))
        .filter_map(|source| source.split('/').next())
        .collect();
    categories.sort_unstable();
    categories.dedup();
    categories
}

/// The game file's name without its folders, which is also its texture's name.
fn objectName(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// Each playable card and the outfit its portrait shows. Cards that start below 3★ wear the
/// shared uniform until then, so the card's own outfit is the one at its top rarity.
fn cardOutfits(connection: &Connection) -> Result<Vec<CardOutfit>> {
    let mut statement = connection.prepare(
        "SELECT card.chara_id, card.id, rarity.race_dress_id
         FROM card_data card
         JOIN card_rarity_data rarity ON rarity.card_id = card.id
         WHERE rarity.rarity = (
           SELECT MAX(top.rarity) FROM card_rarity_data top WHERE top.card_id = card.id
         )
         ORDER BY card.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(CardOutfit {
            chara: row.get(0)?,
            card: row.get(1)?,
            dress: row.get(2)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read card outfits from master.mdb")
}

fn familyOutputs(family: &ArtFamily, name: &str) -> Option<Vec<PlannedOutput>> {
    let captures = matchPattern(family.source, name)?;
    Some(plannedOutputs(family.outputs, &captures, objectName(name)))
}

fn plannedOutputs(
    outputs: &[ArtOutput],
    captures: &[(&str, &str)],
    object: &str,
) -> Vec<PlannedOutput> {
    outputs
        .iter()
        .map(|output| PlannedOutput {
            key: fillPattern(output.path, captures),
            object: object.to_string(),
            fit: output.fit,
        })
        .collect()
}

/// Sprites are cut at the size they are drawn, which is already the size the site serves.
fn spriteOutputs(sprites: &AtlasSprites) -> Vec<PlannedOutput> {
    let sprite_output = |object: &str, key: String| PlannedOutput {
        key,
        object: object.to_string(),
        fit: Fit::Native,
    };

    match sprites {
        AtlasSprites::Named { sprite, paths } => paths
            .iter()
            .map(|path| sprite_output(sprite, path.to_string()))
            .collect(),
        AtlasSprites::Numbered {
            sprite,
            numbers,
            width,
            paths,
        } => {
            let mut outputs = Vec::new();
            for number in numbers.clone() {
                let digits = format!("{number:0width$}");
                let captures = [("n", digits.as_str())];
                let object = fillPattern(sprite, &captures);
                for path in *paths {
                    outputs.push(sprite_output(&object, fillPattern(path, &captures)));
                }
            }
            outputs
        }
    }
}

/// Matches `name` against a pattern whose `{placeholder}` segments stand for digits.
fn matchPattern<'a>(pattern: &'a str, name: &'a str) -> Option<Vec<(&'a str, &'a str)>> {
    let mut captures: Vec<(&str, &str)> = Vec::new();
    let mut pattern_rest = pattern;
    let mut name_rest = name;

    while let Some(open) = pattern_rest.find('{') {
        name_rest = name_rest.strip_prefix(&pattern_rest[..open])?;
        let close = open + pattern_rest[open..].find('}')?;
        let placeholder = &pattern_rest[open + 1..close];

        let digit_count = name_rest.bytes().take_while(u8::is_ascii_digit).count();
        if digit_count == 0 {
            return None;
        }
        let (digits, after) = name_rest.split_at(digit_count);

        match captures.iter().find(|(known, _)| *known == placeholder) {
            Some((_, earlier)) if *earlier != digits => return None,
            Some(_) => {}
            None => captures.push((placeholder, digits)),
        }

        name_rest = after;
        pattern_rest = &pattern_rest[close + 1..];
    }

    (pattern_rest == name_rest).then_some(captures)
}

fn fillPattern(template: &str, captures: &[(&str, &str)]) -> String {
    let mut filled = template.to_string();
    for (placeholder, digits) in captures {
        filled = filled.replace(&format!("{{{placeholder}}}"), digits);
    }
    filled
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Special Week's second card wears outfit 100130 and her third wears outfit 100102.
    const SPECIAL_WEEK: [CardOutfit; 3] = [
        CardOutfit {
            chara: 1001,
            card: 100101,
            dress: 100101,
        },
        CardOutfit {
            chara: 1001,
            card: 100102,
            dress: 100130,
        },
        CardOutfit {
            chara: 1001,
            card: 100103,
            dress: 100102,
        },
    ];

    fn keys(catalog: &Catalog, name: &str) -> Option<Vec<String>> {
        let outputs = catalog.outputs(name)?;
        Some(outputs.into_iter().map(|output| output.key).collect())
    }

    #[test]
    fn gameFilesMapToTheUrlsTheSiteServes() {
        let catalog = Catalog::fromOutfits(&[]);

        assert_eq!(
            keys(&catalog, "chara/chr1001/chr_icon_1001"),
            Some(vec![
                "game-assets/umamusume_icons/chr_icon_1001.webp".to_string()
            ])
        );
        assert_eq!(
            keys(&catalog, "race/racetitle/thumb/thum_race_rt_000_1001_00"),
            Some(vec![
                "game-assets/textures/race_banners/thum_race_rt_000_1001_00.webp".to_string()
            ])
        );
    }

    #[test]
    fn portraitsAreServedUnderTheCardThatWearsTheOutfit() {
        let catalog = Catalog::fromOutfits(&SPECIAL_WEEK);

        let ruler_of_japan = keys(&catalog, "chara/chr1001/chara_stand_1001_100102");
        let happy_heart = keys(&catalog, "chara/chr1001/chara_stand_1001_100130");

        assert_eq!(
            ruler_of_japan,
            Some(vec![
                "game-assets/character_thumbs/chara_stand_1001_100103.webp".to_string(),
                "assets/images/character_stand/chara_stand_100103.webp".to_string(),
            ])
        );
        assert_eq!(
            happy_heart,
            Some(vec![
                "game-assets/character_thumbs/chara_stand_1001_100102.webp".to_string(),
                "assets/images/character_stand/chara_stand_100102.webp".to_string(),
            ])
        );
    }

    #[test]
    fn outfitsNoCardWearsAreIgnored() {
        let catalog = Catalog::fromOutfits(&SPECIAL_WEEK);

        assert_eq!(
            keys(&catalog, "chara/chr1001/chara_stand_1001_000001"),
            None
        );
    }

    #[test]
    fn characterIconsKeepTheirShapeOnTheServedCanvas() {
        let catalog = Catalog::fromOutfits(&[]);

        let icon = catalog.outputs("chara/chr1001/chr_icon_1001").unwrap();

        assert_eq!(
            icon[0].fit,
            Fit::Centre {
                size: (256, 256),
                art: (232, 256)
            }
        );
    }

    #[test]
    fn atlasSpritesAreServedUnderTheSiteNames() {
        let catalog = Catalog::fromOutfits(&[]);

        let common = catalog.outputs("atlas/common/common").unwrap();
        let ranks = catalog.outputs("atlas/statusrank/statusrank").unwrap();

        let speed = common
            .iter()
            .find(|output| output.key == "assets/images/icon/stats/speed.webp")
            .unwrap();
        assert_eq!(speed.object, "utx_ico_obtain_00");
        assert_eq!(speed.fit, Fit::Native);
        assert_eq!(ranks.len(), 98 * 2);
        assert!(ranks
            .iter()
            .any(|output| output.object == "utx_ico_statusrank_97"
                && output.key == "game-assets/textures/uma_ranks/utx_ico_statusrank_97.webp"));
    }

    #[test]
    fn atlasesNeedTheirTextureAndTexturesAloneAreNotBuilt() {
        let catalog = Catalog::fromOutfits(&[]);

        assert_eq!(
            catalog.dependency("atlas/common/common"),
            Some("atlas/common/common_tex")
        );
        assert_eq!(catalog.dependency("item/item_icon_00041"), None);
        assert_eq!(keys(&catalog, "atlas/common/common_tex"), None);
    }

    #[test]
    fn noTwoOutputsShareABucketKey() {
        let catalog = Catalog::fromOutfits(&SPECIAL_WEEK);
        let mut seen = std::collections::HashSet::new();

        let duplicates: Vec<String> = catalog
            .atlases
            .values()
            .flat_map(|plan| plan.outputs.iter())
            .chain(catalog.portraits.values().flatten())
            .filter(|output| !seen.insert(output.key.clone()))
            .map(|output| output.key.clone())
            .collect();

        assert!(duplicates.is_empty(), "{duplicates:?}");
    }

    #[test]
    fn chibiSpritesKeepTheirSize() {
        let catalog = Catalog::fromOutfits(&[]);

        let chibi = catalog
            .outputs("chara/chr1001/petit/petit_chr_1001_100101_0020")
            .unwrap();
        let valentine = keys(
            &catalog,
            "outgame/campaign/valentine/chara/petit_chr_1001_100101_0100",
        );

        assert_eq!(
            chibi[0].key,
            "game-assets/petit/petit_chr_1001_100101_0020.webp"
        );
        assert_eq!(chibi[0].fit, Fit::Native);
        assert_eq!(
            valentine,
            Some(vec![
                "game-assets/petit/petit_chr_1001_100101_0100.webp".to_string()
            ])
        );
    }

    #[test]
    fn neighbouringFilesTheSiteDoesNotUseAreIgnored() {
        let catalog = Catalog::fromOutfits(&SPECIAL_WEEK);

        assert_eq!(
            keys(&catalog, "chara/chr1001/chr_icon_1001_100101_01"),
            None
        );
        assert_eq!(
            keys(&catalog, "chara/chr1001/trained_chr_icon_1001_100101_01"),
            None
        );
        assert_eq!(
            keys(&catalog, "minigame/mng_0001/icon/frm_item_icon_00000"),
            None
        );
    }

    #[test]
    fn repeatedPlaceholdersMustAgree() {
        let catalog = Catalog::fromOutfits(&[]);

        assert_eq!(keys(&catalog, "chara/chr1001/chr_icon_1002"), None);
        assert_eq!(
            keys(&catalog, "supportcard/support10001/tex_support_card_10002"),
            None
        );
    }

    #[test]
    fn categoriesComeFromTheGamePaths() {
        assert_eq!(
            catalogCategories(),
            vec![
                "atlas",
                "chara",
                "item",
                "outgame",
                "race",
                "supportcard",
                "uianimation"
            ]
        );
    }

    #[test]
    fn cardOutfitsComeFromTheTopRarity() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE card_data (id INTEGER, chara_id INTEGER, default_rarity INTEGER);
                 CREATE TABLE card_rarity_data (card_id INTEGER, rarity INTEGER, race_dress_id INTEGER);
                 INSERT INTO card_data VALUES (100102, 1001, 3), (100701, 1007, 2), (9100101, 1001, 0);
                 INSERT INTO card_rarity_data VALUES
                   (100102, 3, 100130), (100102, 5, 100130),
                   (100701, 2, 101), (100701, 3, 100701), (100701, 5, 100701);",
            )
            .unwrap();

        let outfits = cardOutfits(&connection).unwrap();

        let pairs: Vec<(u32, u32)> = outfits
            .iter()
            .map(|outfit| (outfit.card, outfit.dress))
            .collect();
        assert_eq!(pairs, vec![(100102, 100130), (100701, 100701)]);
    }
}

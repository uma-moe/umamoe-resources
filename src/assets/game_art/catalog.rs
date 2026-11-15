use super::types::{
    ArtFamily, ArtOutput, CardOutfit, Catalog, PlannedOutput, ART_FAMILIES, PORTRAIT_OUTPUTS,
    PORTRAIT_SOURCE,
};
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::path::Path;

impl Catalog {
    /// The fixed families plus a portrait for every card in this master.
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
            portraits
                .entry(file)
                .or_default()
                .extend(plannedOutputs(PORTRAIT_OUTPUTS, &captures));
        }
        Self { portraits }
    }

    /// The bucket keys a game file is served at, or `None` when the site does not use it.
    pub(super) fn outputs(&self, name: &str) -> Option<Vec<PlannedOutput>> {
        ART_FAMILIES
            .iter()
            .find_map(|family| familyOutputs(family, name))
            .or_else(|| self.portraits.get(name).cloned())
    }
}

/// The CDN categories the catalog reads from: the first segment of each game path.
pub(super) fn catalogCategories() -> Vec<&'static str> {
    let mut categories: Vec<&str> = ART_FAMILIES
        .iter()
        .map(|family| family.source)
        .chain([PORTRAIT_SOURCE])
        .filter_map(|source| source.split('/').next())
        .collect();
    categories.sort_unstable();
    categories.dedup();
    categories
}

/// The game file's name without its folders, which is also its texture's name.
pub(super) fn objectName(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// Each playable card and the outfit its portrait shows, at the rarity it starts with.
fn cardOutfits(connection: &Connection) -> Result<Vec<CardOutfit>> {
    let mut statement = connection.prepare(
        "SELECT card.chara_id, card.id, rarity.race_dress_id
         FROM card_data card
         JOIN card_rarity_data rarity
           ON rarity.card_id = card.id AND rarity.rarity = card.default_rarity
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
    Some(plannedOutputs(family.outputs, &captures))
}

fn plannedOutputs(outputs: &[ArtOutput], captures: &[(&str, &str)]) -> Vec<PlannedOutput> {
    outputs
        .iter()
        .map(|output| PlannedOutput {
            key: fillPattern(output.path, captures),
            size: output.size,
            art_size: output.art_size,
        })
        .collect()
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

        assert_eq!(icon[0].size, (256, 256));
        assert_eq!(icon[0].art_size, Some((232, 256)));
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
    fn cardOutfitsComeFromTheStartingRarity() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE card_data (id INTEGER, chara_id INTEGER, default_rarity INTEGER);
                 CREATE TABLE card_rarity_data (card_id INTEGER, rarity INTEGER, race_dress_id INTEGER);
                 INSERT INTO card_data VALUES (100102, 1001, 3), (9100101, 1001, 0);
                 INSERT INTO card_rarity_data VALUES (100102, 3, 100130), (100102, 5, 100130);",
            )
            .unwrap();

        let outfits = cardOutfits(&connection).unwrap();

        let pairs: Vec<(u32, u32)> = outfits
            .iter()
            .map(|outfit| (outfit.card, outfit.dress))
            .collect();
        assert_eq!(pairs, vec![(100102, 100130)]);
    }
}

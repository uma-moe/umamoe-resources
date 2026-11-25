use super::chibis::{chibiOutfits, isChibiRecord};
use super::clips::{isChibiClip, isCutinClip};
use super::layout::{characterKey, umaFolders, umaKey};
use super::race_motions::raceMotion;
use super::types::{
    ArtFamily, ArtOutput, AtlasPlan, AtlasSprites, CardOutfit, Catalog, CoursePart, Dependencies,
    Fit, OutfitPicture, OutfitRow, PlannedOutput, PrefabFamily, SpriteSource, SupportCard,
    ART_FAMILIES, ATLAS_FAMILIES, CHIBI_CONVERTER_REVISION, CONVERTER_REVISION, COURSE_ENVIRONMENT,
    COURSE_NAME_CATEGORY, COURSE_PARTS, COURSE_SKY, CUTIN_CONVERTER_REVISION, CUTIN_SET_FOLDER,
    CUTIN_TIMELINE_FOLDER, EFFECT_CONVERTER_REVISION, EFFECT_FOLDER, MODEL_CONVERTER_REVISION,
    OUTFIT_PICTURES, PORTRAIT_OUTPUTS, PORTRAIT_SOURCE, PROP_FOLDER, SHADER_BUNDLE,
    SHADER_CONVERTER_REVISION, SHADER_RECORD, SPRITE_SOURCES, TRACK_CATEGORY, TRACK_FOLDER,
    UMA_CONVERTER_REVISION,
};
use super::umas::{isUmaRecord, outfitRows, umaOutfits};
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

impl Catalog {
    /// The fixed families and atlases plus a portrait for every card, a name for every
    /// racecourse, a uma and chibi for every outfit and every support card in this master.
    pub(super) fn load(master: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(master, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("failed to open {}", master.display()))?;
        let mut catalog = Self::fromOutfits(&cardOutfits(&connection)?);
        catalog.course_names = courseNames(&connection)?;
        catalog.support_cards = supportCards(&connection)?;
        catalog.setOutfits(outfitRows(&connection)?);
        Ok(catalog)
    }

    /// Every outfit with a body of its own, its uma folder, uma and chibi.
    pub(super) fn setOutfits(&mut self, outfits: Vec<OutfitRow>) {
        self.uma_folders = umaFolders(&self.cards, &outfits);
        self.umas = umaOutfits(&outfits, &self.uma_folders);
        self.chibis = chibiOutfits(&outfits, &self.uma_folders);
        self.outfits = outfits;
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
                    outputs: family.sprites.iter().flat_map(atlasOutputs).collect(),
                };
                (family.atlas, plan)
            })
            .collect();

        let uma_folders = outfits
            .iter()
            .map(|outfit| (outfit.dress, outfit.card.to_string()))
            .collect();
        Self {
            portraits,
            atlases,
            course_names: HashMap::new(),
            cards: outfits.to_vec(),
            outfits: Vec::new(),
            uma_folders,
            umas: Vec::new(),
            chibis: Vec::new(),
            support_cards: Vec::new(),
        }
    }

    /// The pictures a game file is served as, or `None` when it is no picture the site uses.
    /// Models, clips and the other files the layout places are not pictures.
    pub(super) fn outputs(&self, name: &str) -> Option<Vec<PlannedOutput>> {
        ART_FAMILIES
            .iter()
            .find_map(|family| familyOutputs(family, name))
            .or_else(|| self.portraits.get(name).cloned())
            .or_else(|| self.atlases.get(name).map(|plan| plan.outputs.clone()))
            .or_else(|| {
                SPRITE_SOURCES
                    .iter()
                    .find_map(|sprites| self.spriteOutputs(sprites, name))
            })
            .or_else(|| {
                OUTFIT_PICTURES
                    .iter()
                    .find_map(|picture| self.outfitPictureOutputs(picture, name))
            })
    }

    /// An outfit's icon, dress or gacha picture in its uma folder, `art/umas/100101/icon.webp`.
    /// Outfits without a folder of their own, such as the shared uniforms, are left out.
    fn outfitPictureOutputs(
        &self,
        picture: &OutfitPicture,
        name: &str,
    ) -> Option<Vec<PlannedOutput>> {
        let captures = matchPattern(picture.source, name)?;
        let captured = |placeholder: &str| {
            captures
                .iter()
                .find(|(known, _)| *known == placeholder)
                .and_then(|(_, digits)| digits.parse::<u32>().ok())
        };

        let folder = match (captured("dress"), captured("card")) {
            (Some(dress), _) => self.uma_folders.get(&dress)?,
            (None, Some(card)) => self.cardFolder(card)?,
            (None, None) => return None,
        };
        Some(vec![PlannedOutput {
            key: umaKey(folder, picture.file),
            object: objectName(name).to_string(),
            fit: Fit::Native,
        }])
    }

    /// The uma folder of a playable card, when the card's outfit is served under it.
    fn cardFolder(&self, card: u32) -> Option<&String> {
        let outfit = self.cards.iter().find(|outfit| outfit.card == card)?;
        let folder = self.uma_folders.get(&outfit.dress)?;
        (*folder == card.to_string()).then_some(folder)
    }

    /// A chibi sprite in its outfit's uma folder, `art/umas/100101/sprites/0020.webp`, or in its
    /// character's folder for an outfit without one.
    fn spriteOutputs(&self, sprites: &SpriteSource, name: &str) -> Option<Vec<PlannedOutput>> {
        let captures = matchPattern(sprites.source, name)?;
        let captured = |placeholder: &str| {
            captures
                .iter()
                .find(|(known, _)| *known == placeholder)
                .map(|(_, digits)| *digits)
        };
        let (chara, dress, pose) = (captured("chara")?, captured("dress")?, captured("pose")?);
        let folder = dress
            .parse()
            .ok()
            .and_then(|dress: u32| self.uma_folders.get(&dress));

        let key = match folder {
            Some(folder) => umaKey(folder, &format!("{}/{pose}.webp", sprites.folder)),
            None => characterKey(
                chara.parse().ok()?,
                &format!("{}/{dress}_{pose}.webp", sprites.folder),
            ),
        };
        Some(vec![PlannedOutput {
            key,
            object: objectName(name).to_string(),
            fit: Fit::Native,
        }])
    }

    /// The bundles a game file is loaded with.
    pub(super) fn dependencies(&self, name: &str) -> Dependencies {
        if let Some(plan) = self.atlases.get(name) {
            return Dependencies::AtlasTexture(plan.texture);
        }
        if prefabFamily(name).is_some() {
            return Dependencies::Listed;
        }
        Dependencies::None
    }
}

/// The CDN categories the catalog reads from: the first segment of each picture's game path
/// (except `sourceresources`, whose files live in `_3d_cutt` and `uianimation`), the category
/// that lists race tracks and the shader bundle's, which models look shaders up in.
pub(super) fn catalogCategories() -> Vec<&'static str> {
    let mut categories: Vec<&str> = ART_FAMILIES
        .iter()
        .map(|family| family.source)
        .chain([PORTRAIT_SOURCE])
        .chain(SPRITE_SOURCES.iter().map(|sprites| sprites.source))
        .chain(OUTFIT_PICTURES.iter().map(|picture| picture.source))
        .chain(ATLAS_FAMILIES.iter().map(|family| family.atlas))
        .filter_map(|source| source.split('/').next())
        .filter(|category| *category != "sourceresources")
        .chain([TRACK_CATEGORY, SHADER_BUNDLE])
        .collect();
    categories.sort_unstable();
    categories.dedup();
    categories
}

/// Shaders do not convert, and a still model has no use for the races' animations.
pub(super) fn isUnusedDependency(name: &str) -> bool {
    name == "shader" || name.starts_with("3d/motion/")
}

pub(super) fn converterRevision(name: &str) -> u32 {
    if name == SHADER_RECORD {
        return SHADER_CONVERTER_REVISION;
    }
    if isUmaRecord(name) || raceMotion(name).is_some() {
        return UMA_CONVERTER_REVISION;
    }
    if isChibiRecord(name) || isChibiClip(name) {
        return CHIBI_CONVERTER_REVISION;
    }
    if isCutinTimeline(name) || isCutinClip(name) {
        return CUTIN_CONVERTER_REVISION;
    }
    match prefabFamily(name) {
        Some(PrefabFamily::Effect) => EFFECT_CONVERTER_REVISION,
        Some(PrefabFamily::CutinSet | PrefabFamily::Prop) => CUTIN_CONVERTER_REVISION,
        Some(PrefabFamily::Track) => MODEL_CONVERTER_REVISION,
        None if isCourse(name) => MODEL_CONVERTER_REVISION,
        None => CONVERTER_REVISION,
    }
}

/// The family of a prefab built into a model of its own: a track prefab, an effect, a cut-in set
/// or a prop such as `3d/chara/prop/prop1001_00/pfb_chr_prop1001_00`. Cloth physics prefabs draw
/// nothing and are left out.
pub(super) fn prefabFamily(name: &str) -> Option<PrefabFamily> {
    if isTrackPrefab(name) {
        return Some(PrefabFamily::Track);
    }
    if !objectName(name).starts_with("pfb_") || name.contains("/clothes/") {
        return None;
    }
    if name.starts_with(EFFECT_FOLDER) {
        return Some(PrefabFamily::Effect);
    }
    if name.starts_with(CUTIN_SET_FOLDER) {
        return Some(PrefabFamily::CutinSet);
    }
    let is_prop = name
        .strip_prefix(PROP_FOLDER)
        .is_some_and(|below| below.matches('/').count() == 1);
    is_prop.then_some(PrefabFamily::Prop)
}

/// The state shard of an effect, cut-in set or prop.
pub(super) fn prefabGroup(name: &str) -> Option<&'static str> {
    match prefabFamily(name)? {
        PrefabFamily::Effect => Some("effects"),
        PrefabFamily::CutinSet => Some("cutin-sets"),
        PrefabFamily::Prop => Some("props"),
        PrefabFamily::Track => None,
    }
}

/// A skill cut-in's timeline, in a folder of its own name:
/// `cutt/cutin/skill/crd100101_001/crd100101_001`.
pub(super) fn isCutinTimeline(name: &str) -> bool {
    name.strip_prefix(CUTIN_TIMELINE_FOLDER)
        .and_then(|below| below.split_once('/'))
        .is_some_and(|(folder, file)| folder == file)
}

/// The kind of track scenery a record below the track folder belongs to: a racecourse's
/// combined model, a part of a racecourse, a starting gate, the skydomes or the other props every
/// course shares.
pub(super) fn trackGroup(name: &str) -> Option<&'static str> {
    let rest = name.strip_prefix(TRACK_FOLDER)?;
    let Some((track, below)) = rest.split_once('/') else {
        return courseNumber(rest).map(|_| "racetracks");
    };
    trackNumber(track)?;
    let part = below.split('/').next()?;

    if part.contains("_gate") {
        return Some("starting-gates");
    }
    if courseNumber(track).is_some() {
        return Some("racetrack-parts");
    }
    if part.ends_with("_9000") {
        return Some("skyboxes");
    }
    Some("track-props")
}

/// The prefabs each racecourse's model draws, by the course's folder such as
/// `3d/env/race/race10006`: every part in `COURSE_PARTS` in the variant that fits
/// `COURSE_ENVIRONMENT` best, then the sky. A course whose track is not listed is left out.
pub(super) fn courseScenes<'a>(
    names: impl Iterator<Item = &'a str>,
) -> BTreeMap<&'a str, Vec<&'a str>> {
    let mut chosen: BTreeMap<(&str, usize), (&str, usize)> = BTreeMap::new();
    let mut sky = None;
    for name in names {
        if name == COURSE_SKY {
            sky = Some(name);
            continue;
        }
        let Some(part) = coursePart(name) else {
            continue;
        };
        let Some(fit) = environmentFit(part.variant) else {
            continue;
        };
        let slot = (part.course, part.order);
        if chosen.get(&slot).is_none_or(|(_, best)| fit > *best) {
            chosen.insert(slot, (name, fit));
        }
    }

    // Each course's parts come in scene order, so its track comes first if it is listed.
    let mut scenes: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for ((course, order), (prefab, _)) in chosen {
        if order == 0 {
            scenes.insert(course, vec![prefab]);
        } else if let Some(scene) = scenes.get_mut(course) {
            scene.push(prefab);
        }
    }
    for scene in scenes.values_mut() {
        scene.extend(sky);
    }
    scenes
}

fn coursePart(name: &str) -> Option<CoursePart<'_>> {
    let (track, _) = name.strip_prefix(TRACK_FOLDER)?.split_once('/')?;
    courseNumber(track)?;
    let (part, variant) = objectName(name)
        .strip_prefix("pfb_env_")?
        .strip_prefix(track)?
        .strip_prefix('_')?
        .split_once('_')?;
    let order = COURSE_PARTS.iter().position(|known| *known == part)?;

    Some(CoursePart {
        course: &name[..TRACK_FOLDER.len() + track.len()],
        order,
        variant,
    })
}

/// How many of a variant's digits match `COURSE_ENVIRONMENT`, or `None` when one contradicts
/// it; a `0` digit fits any season, weather or time.
fn environmentFit(variant: &str) -> Option<usize> {
    if variant.len() != COURSE_ENVIRONMENT.len() {
        return None;
    }

    let mut matched = 0;
    for (digit, wanted) in variant.bytes().zip(COURSE_ENVIRONMENT.bytes()) {
        if digit == wanted {
            matched += 1;
        } else if digit != b'0' {
            return None;
        }
    }
    Some(matched)
}

/// A track folder's number, `race00100` or `race10006`.
fn trackNumber(track: &str) -> Option<u32> {
    track.strip_prefix("race")?.parse().ok()
}

/// A racecourse's number from its folder, `race10006`. Courses are numbered from race10001; the
/// folders below hold what every course shares.
pub(super) fn courseNumber(track: &str) -> Option<u32> {
    trackNumber(track).filter(|number| *number >= 10000)
}

/// A racecourse's own record, `3d/env/race/race10006`, which its combined model is published
/// under.
fn isCourse(name: &str) -> bool {
    name.strip_prefix(TRACK_FOLDER)
        .is_some_and(|rest| !rest.contains('/') && courseNumber(rest).is_some())
}

/// Such as `3d/env/race/race10001/race10001_0000/pfb_env_race10001_0000_002`.
pub(super) fn isTrackPrefab(name: &str) -> bool {
    name.strip_prefix(TRACK_FOLDER)
        .is_some_and(|rest| objectName(rest).starts_with("pfb_"))
}

/// The season and weather textures in a track part's `textures` folder, such as
/// `3d/env/race/race10001/race10001_0000/textures/tex_env_race10001_0000_112_turf00`.
pub(super) fn isTrackTexture(name: &str) -> bool {
    name.strip_prefix(TRACK_FOLDER).is_some_and(|rest| {
        rest.rsplit_once('/')
            .is_some_and(|(folder, file)| folder.ends_with("/textures") && file.starts_with("tex_"))
    })
}

/// The folder holding the textures the game puts on a prefab's materials at runtime.
pub(super) fn runtimeTextureFolder(prefab: &str) -> Option<String> {
    if !isTrackPrefab(prefab) {
        return None;
    }
    let (part, _) = prefab.rsplit_once('/')?;
    Some(format!("{part}/textures"))
}

/// The game file's name without its folders, which is also its texture's or prefab's name.
pub(super) fn objectName(name: &str) -> &str {
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

/// Every support card and the character it shows.
fn supportCards(connection: &Connection) -> Result<Vec<SupportCard>> {
    let mut statement =
        connection.prepare("SELECT id, chara_id FROM support_card_data ORDER BY id")?;
    let rows = statement.query_map([], |row| {
        Ok(SupportCard {
            id: row.get(0)?,
            chara: row.get(1)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read support cards from master.mdb")
}

fn courseNames(connection: &Connection) -> Result<HashMap<u32, String>> {
    let mut statement =
        connection.prepare("SELECT \"index\", text FROM text_data WHERE category = ?1")?;
    let rows = statement.query_map([COURSE_NAME_CATEGORY], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<rusqlite::Result<HashMap<_, _>>>()
        .context("failed to read course names from master.mdb")
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
            object: output.object.unwrap_or(object).to_string(),
            fit: output.fit,
        })
        .collect()
}

/// Sprites are cut at the size they are drawn, which is already the size the site serves.
fn atlasOutputs(sprites: &AtlasSprites) -> Vec<PlannedOutput> {
    let sprite_output = |object: &str, key: String| PlannedOutput {
        key,
        object: object.to_string(),
        fit: Fit::Native,
    };

    match sprites {
        AtlasSprites::Named { sprite, path } => vec![sprite_output(sprite, path.to_string())],
        AtlasSprites::Numbered {
            sprite,
            numbers,
            width,
            path,
        } => numbers
            .clone()
            .map(|number| {
                let digits = format!("{number:0width$}");
                let captures = [("n", digits.as_str())];
                sprite_output(
                    &fillPattern(sprite, &captures),
                    fillPattern(path, &captures),
                )
            })
            .collect(),
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
    fn picturesAreServedInTheFolderOfWhatTheyShow() {
        let catalog = Catalog::fromOutfits(&[]);

        assert_eq!(
            keys(&catalog, "chara/chr1001/chr_icon_1001"),
            Some(vec!["art/characters/1001/icon.webp".to_string()])
        );
        assert_eq!(
            keys(&catalog, "race/racetitle/thumb/thum_race_rt_000_1001_00"),
            Some(vec!["art/races/banners/1001.webp".to_string()])
        );
        assert_eq!(
            keys(&catalog, "supportcard/support30050/support_card_s_30050"),
            Some(vec!["art/support-cards/30050/half.webp".to_string()])
        );
    }

    #[test]
    fn standArtIsServedUnderTheCardThatWearsTheOutfit() {
        let catalog = Catalog::fromOutfits(&SPECIAL_WEEK);

        let ruler_of_japan = keys(&catalog, "chara/chr1001/chara_stand_1001_100102");
        let happy_heart = keys(&catalog, "chara/chr1001/chara_stand_1001_100130");

        assert_eq!(
            ruler_of_japan,
            Some(vec![
                "art/umas/100103/portrait.webp".to_string(),
                "art/umas/100103/stand.webp".to_string()
            ])
        );
        assert_eq!(
            happy_heart,
            Some(vec![
                "art/umas/100102/portrait.webp".to_string(),
                "art/umas/100102/stand.webp".to_string()
            ])
        );
    }

    #[test]
    fn outfitPicturesAreServedInTheFolderOfTheOutfitTheyShow() {
        let catalog = Catalog::fromOutfits(&SPECIAL_WEEK);

        let icon = keys(&catalog, "chara/chr1001/chr_icon_1001_100130_02");
        let trained_icon = keys(&catalog, "chara/chr1001/trained_chr_icon_1001_100102_01");
        let dress = keys(&catalog, "outgame/dress/dress_100130_0_00");

        assert_eq!(
            icon,
            Some(vec!["art/umas/100102/icon-gold.webp".to_string()])
        );
        assert_eq!(
            trained_icon,
            Some(vec!["art/umas/100103/trained-icon.webp".to_string()])
        );
        assert_eq!(dress, Some(vec!["art/umas/100102/dress.webp".to_string()]));
    }

    #[test]
    fn gachaThumbnailsAreServedInTheFolderOfTheirCard() {
        let catalog = Catalog::fromOutfits(&SPECIAL_WEEK);

        let thumb = keys(
            &catalog,
            "outgame/note/gacha_thumb/chara_card/gacha_thumb_100102",
        );
        let unknown_card = keys(
            &catalog,
            "outgame/note/gacha_thumb/chara_card/gacha_thumb_100109",
        );

        assert_eq!(
            thumb,
            Some(vec!["art/umas/100102/gacha-thumb.webp".to_string()])
        );
        assert_eq!(unknown_card, None);
    }

    #[test]
    fn picturesOfSharedOutfitsAreIgnored() {
        let catalog = Catalog::fromOutfits(&SPECIAL_WEEK);

        assert_eq!(
            keys(&catalog, "chara/chr1001/chr_icon_1001_000001_01"),
            None
        );
        assert_eq!(keys(&catalog, "chara/chr1001/push_icon_1001_000002"), None);
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
            .find(|output| output.key == "art/ui/stats/speed.webp")
            .unwrap();
        assert_eq!(speed.object, "utx_ico_obtain_00");
        assert_eq!(speed.fit, Fit::Native);
        assert_eq!(ranks.len(), 98);
        assert!(ranks
            .iter()
            .any(|output| output.object == "utx_ico_statusrank_97"
                && output.key == "art/ui/status-ranks/97.webp"));
    }

    #[test]
    fn atlasesNeedTheirTextureAndTexturesAloneAreNotBuilt() {
        let catalog = Catalog::fromOutfits(&[]);

        assert_eq!(
            catalog.dependencies("atlas/common/common"),
            Dependencies::AtlasTexture("atlas/common/common_tex")
        );
        assert_eq!(
            catalog.dependencies("item/item_icon_00041"),
            Dependencies::None
        );
        assert_eq!(keys(&catalog, "atlas/common/common_tex"), None);
    }

    #[test]
    fn trackPrefabsAreModelsBuiltFromEverythingTheyDraw() {
        let catalog = Catalog::fromOutfits(&[]);
        let prefab = "3d/env/race/race10001/race10001_0000/pfb_env_race10001_0000_002";

        assert_eq!(keys(&catalog, prefab), None);
        assert_eq!(catalog.dependencies(prefab), Dependencies::Listed);
        assert_eq!(prefabFamily(prefab), Some(PrefabFamily::Track));
        assert!(catalogCategories().contains(&TRACK_CATEGORY));
    }

    #[test]
    fn prefabsFindTheirSeasonTexturesInTheirPartFolder() {
        let prefab = "3d/env/race/race10001/race10001_0000/pfb_env_race10001_0000_002";
        let model = "sourceresources/3d/env/race/race10001/race10001_6000/models/mdl_env_race10001_6000_000";

        let folder = runtimeTextureFolder(prefab);

        assert_eq!(
            folder.as_deref(),
            Some("3d/env/race/race10001/race10001_0000/textures")
        );
        assert_eq!(runtimeTextureFolder(model), None);
        assert!(!isTrackTexture(model));
    }

    #[test]
    fn trackFilesAreGroupedByTheSceneryTheyShow() {
        let groups = [
            "3d/env/race/race10006",
            "3d/env/race/race10006/race10006_0000/pfb_env_race10006_0000_002",
            "3d/env/race/race10006/race10006_0000/textures/tex_env_race10006_0000_112_turf00",
            "3d/env/race/race00000/race00000_9000/pfb_env_race00000_9000_112",
            "3d/env/race/race00000/race00000_gate000/pfb_env_race00000_gate000_000",
            "3d/env/race/race00100/race00100_gate000/pfb_env_race00100_gate000_000",
            "3d/env/race/race00000/race00000_rachi000/pfb_env_race00000_rachi000_000",
            "3d/env/race/audienceimpostors/ast_audience111000001",
            "chara/chr1001/chr_icon_1001",
        ]
        .map(trackGroup);

        assert_eq!(
            groups,
            [
                Some("racetracks"),
                Some("racetrack-parts"),
                Some("racetrack-parts"),
                Some("skyboxes"),
                Some("starting-gates"),
                Some("starting-gates"),
                Some("track-props"),
                None,
                None,
            ]
        );
    }

    #[test]
    fn onlyModelsFollowTheModelRevision() {
        let prefab = "3d/env/race/race10001/race10001_0000/pfb_env_race10001_0000_002";
        let texture =
            "3d/env/race/race10001/race10001_0000/textures/tex_env_race10001_0000_112_base00";

        assert_eq!(converterRevision(prefab), MODEL_CONVERTER_REVISION);
        assert_eq!(
            converterRevision("3d/env/race/race10001"),
            MODEL_CONVERTER_REVISION
        );
        assert_eq!(converterRevision(texture), CONVERTER_REVISION);
        assert_eq!(
            converterRevision("supportcard/support30050/tex_support_card_30050"),
            CONVERTER_REVISION
        );
    }

    #[test]
    fn umasAndRaceMotionsFollowTheUmaRevision() {
        let uma = "3d/chara/body/bdy1001_00";
        let motion = "3d/motion/racemain/body/type01/anm_rac_type01_run03_base";
        let body_prefab = "3d/chara/body/bdy1001_00/pfb_bdy1001_00";

        assert_eq!(converterRevision(uma), UMA_CONVERTER_REVISION);
        assert_eq!(converterRevision(motion), UMA_CONVERTER_REVISION);
        assert_eq!(converterRevision(body_prefab), CONVERTER_REVISION);
    }

    fn tokyoPrefab(part: &str) -> String {
        let folder = &part[..2];
        format!("3d/env/race/race10006/race10006_{folder}00/pfb_env_race10006_{part}")
    }

    #[test]
    fn courseScenesTakeTheSpringDayVariantOfEachPartThenTheSky() {
        let listed = [
            "0000_002",
            "0000_004",
            "0001_002",
            "1000_000",
            "2000_000",
            "6000_100",
            "6000_114",
            "6000_442",
            "7001_010",
            "7001_014",
            "7001_030",
            "7002_010",
            "7003_010",
            "7013_000",
            "cutt000_002",
            "prop002_000",
        ]
        .map(tokyoPrefab);
        let names = listed.iter().map(String::as_str).chain([COURSE_SKY]);

        let scenes = courseScenes(names);

        let expected: Vec<String> = [
            "0000_002", "0001_002", "6000_100", "7001_010", "7002_010", "7003_010",
        ]
        .map(tokyoPrefab)
        .into_iter()
        .chain([COURSE_SKY.to_string()])
        .collect();
        assert_eq!(
            scenes.keys().copied().collect::<Vec<_>>(),
            vec!["3d/env/race/race10006"]
        );
        assert_eq!(scenes["3d/env/race/race10006"], expected);
    }

    #[test]
    fn aCourseWithoutItsTrackHasNoScene() {
        let listed = ["6000_100", "7001_010"].map(tokyoPrefab);

        let scenes = courseScenes(listed.iter().map(String::as_str).chain([COURSE_SKY]));

        assert!(scenes.is_empty());
    }

    #[test]
    fn courseNamesComeFromTheirTextCategory() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE text_data (category INTEGER, \"index\" INTEGER, text TEXT);
                 INSERT INTO text_data VALUES (35, 10006, 'Tokyo'), (36, 10006, 'Not a course');",
            )
            .unwrap();

        let names = courseNames(&connection).unwrap();

        assert_eq!(names, HashMap::from([(10006, "Tokyo".to_string())]));
    }

    #[test]
    fn noTwoOutputsShareABucketKey() {
        let catalog = Catalog::fromOutfits(&SPECIAL_WEEK);
        let mut seen = std::collections::HashSet::new();

        let family_keys = ART_FAMILIES
            .iter()
            .flat_map(|family| family.outputs.iter())
            .map(|output| output.path.to_string());
        let duplicates: Vec<String> = catalog
            .atlases
            .values()
            .flat_map(|plan| plan.outputs.iter())
            .chain(catalog.portraits.values().flatten())
            .map(|output| output.key.clone())
            .chain(family_keys)
            .filter(|key| !seen.insert(key.clone()))
            .collect();

        assert!(duplicates.is_empty(), "{duplicates:?}");
    }

    #[test]
    fn gachaGatesFlashPiecesAndAtlasSpritesResolveToTheirObjects() {
        let catalog = Catalog::fromOutfits(&[]);

        let bookmarks = catalog
            .outputs("sourceresources/3d/env/gacha/gacha_support_card00/materials/mtl_env_gacha_support_card00_bookmark00")
            .unwrap();
        assert_eq!(bookmarks.len(), 1);
        assert_eq!(bookmarks[0].key, "art/gacha/gates/support-bookmarks.webp");
        assert_eq!(
            bookmarks[0].object,
            "tex_env_gacha_support_card00_bookmark00"
        );
        assert_eq!(bookmarks[0].fit, Fit::Native);

        let backs = catalog
            .outputs("uianimation/flash/chara/prefab/pf_fl_chara_supportcard00")
            .unwrap();
        assert_eq!(
            backs
                .iter()
                .map(|o| (o.object.as_str(), o.key.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("R", "art/gacha/cards/back-r.webp"),
                ("SR", "art/gacha/cards/back-sr.webp"),
                ("SSR", "art/gacha/cards/back-ssr.webp"),
            ]
        );

        let gacha_atlas = catalog.outputs("atlas/gacha/gacha").unwrap();
        assert_eq!(
            catalog.dependencies("atlas/gacha/gacha"),
            Dependencies::AtlasTexture("atlas/gacha/gacha_tex")
        );
        assert!(gacha_atlas.iter().any(|o| {
            o.object == "img_supportcard_ssr_frame_effect"
                && o.key == "art/gacha/frames/card-glow-ssr.webp"
        }));
        assert!(gacha_atlas.iter().any(|o| {
            o.object == "utx_txt_gacha_ribbontext_05_20"
                && o.key == "art/gacha/ribbons/per-day-20.webp"
        }));
    }

    #[test]
    fn chibiSpritesKeepTheirSizeInTheirOutfitsFolder() {
        let catalog = Catalog::fromOutfits(&SPECIAL_WEEK);

        let chibi = catalog
            .outputs("chara/chr1001/petit/petit_chr_1001_100130_0020")
            .unwrap();
        let valentine = keys(
            &catalog,
            "outgame/campaign/valentine/chara/petit_chr_1001_100101_0100",
        );

        assert_eq!(chibi[0].key, "art/umas/100102/sprites/0020.webp");
        assert_eq!(chibi[0].fit, Fit::Native);
        assert_eq!(
            valentine,
            Some(vec![
                "art/umas/100101/sprites/valentine/0100.webp".to_string()
            ])
        );
    }

    #[test]
    fn aSpriteOfAnOutfitWithoutAnUmaGoesToItsCharacter() {
        let catalog = Catalog::fromOutfits(&[]);

        let sprite = keys(&catalog, "chara/chr1001/petit/petit_chr_1001_100101_0020");

        assert_eq!(
            sprite,
            Some(vec![
                "art/characters/1001/sprites/100101_0020.webp".to_string()
            ])
        );
    }

    #[test]
    fn supportCardsComeWithTheirCharacter() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE support_card_data (id INTEGER, chara_id INTEGER);
                 INSERT INTO support_card_data VALUES (30050, 1050), (10001, 1001);",
            )
            .unwrap();

        let cards = supportCards(&connection).unwrap();

        assert_eq!(
            cards,
            vec![
                SupportCard {
                    id: 10001,
                    chara: 1001
                },
                SupportCard {
                    id: 30050,
                    chara: 1050
                },
            ]
        );
    }

    #[test]
    fn neighbouringFilesTheSiteDoesNotUseAreIgnored() {
        let catalog = Catalog::fromOutfits(&SPECIAL_WEEK);

        assert_eq!(keys(&catalog, "chara/chr1001/chr_icon_training_1001"), None);
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
    fn categoriesComeFromTheGamePathsTheTrackCategoryAndTheShaders() {
        assert_eq!(
            catalogCategories(),
            vec![
                "_3d_cutt",
                "atlas",
                "chara",
                "item",
                "outgame",
                "race",
                "shader",
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

//! Where every published file goes. Everything lives below `art/`: one folder per uma with what
//! only it has, its character's folder for what all its outfits share, and library folders for
//! what any uma or course can use. Folders are ids, and a file keeps the part of its game name
//! that tells it apart from its siblings: `anm_cti_crd100101_001_01_cam` becomes
//! `umas/100101/cutin/camera/01_cam.glb`.

use super::catalog::{
    courseNumber, isCutinTimeline, isTrackTexture, objectName, prefabFamily, trackGroup,
};
use super::clips::{clipExtension, gameClip};
use super::race_motions::raceMotion;
use super::types::{
    CardOutfit, Catalog, ClipOwner, Fit, GameClip, OutfitRow, PlannedOutput, PrefabFamily,
    CUTIN_SET_FOLDER, EFFECT_FOLDER, SHADER_BUNDLE, TRACK_FOLDER,
};
use crate::game_cdn::{CdnSource, ManifestEntry, WINDOWS};
use std::collections::{BTreeMap, HashMap, HashSet};

pub(super) const ART_ROOT: &str = "art";

/// The shader folder's index; each shader's file lies below it, named after the shader, with an
/// `index.json` in every folder.
pub(super) const SHADER_INDEX_KEY: &str = "art/shaders/index.json";

/// The cut-in number every card and character has one timeline of.
const FIRST_CUTIN: &str = "001";

/// Two or more files whose keys came out the same; each keeps its full game name instead.
#[derive(Debug, PartialEq)]
pub(super) struct Clash {
    pub(super) key: String,
    pub(super) names: Vec<String>,
}

/// The keys of the files the publisher builds, given what the whole file list holds: which
/// timeline plays each cut-in set, and which files would otherwise share a key.
pub(super) struct Layout {
    /// Cards with a folder of their own below `art/umas/`.
    cards: HashSet<u32>,
    /// Characters with a folder of their own below `art/characters/`.
    characters: HashSet<u32>,
    /// The timeline that plays each cut-in set, for sets exactly one timeline plays.
    set_owners: HashMap<String, String>,
    /// What each cut-in timeline plays beside its own clips: its effects, sets and props.
    timeline_parts: BTreeMap<String, Vec<String>>,
    /// Files that keep their full game name, since another file takes the same key.
    full_names: HashSet<String>,
    clashes: Vec<Clash>,
}

/// The folder below `art/umas/` of each outfit, by game outfit: the card's id for a playable
/// outfit, which the site uses, else `<character>-<outfit>`, which NPC race rows give.
pub(super) fn umaFolders(cards: &[CardOutfit], outfits: &[OutfitRow]) -> HashMap<u32, String> {
    let mut folders: HashMap<u32, String> = outfits
        .iter()
        .map(|outfit| (outfit.dress, npcUmaFolder(outfit.chara, outfit.dress)))
        .collect();
    for card in cards {
        folders.insert(card.dress, card.card.to_string());
    }
    folders
}

/// The folder of an outfit no card wears: `1066-106601`, as NPC race rows name it.
pub(super) fn npcUmaFolder(chara: u32, dress: u32) -> String {
    format!("{chara}-{dress}")
}

/// A file in an uma's folder: `art/umas/100101/model.glb`.
pub(super) fn umaKey(folder: &str, file: &str) -> String {
    format!("{ART_ROOT}/umas/{folder}/{file}")
}

/// A file in a character's folder: `art/characters/1001/icon.webp`.
pub(super) fn characterKey(chara: u32, file: &str) -> String {
    format!("{ART_ROOT}/characters/{chara}/{file}")
}

/// A racecourse's combined model, `art/racecourses/10006/model.glb` for `3d/env/race/race10006`.
pub(super) fn courseModelKey(course: &str) -> String {
    let number = objectName(course).trim_start_matches("race");
    format!("{ART_ROOT}/racecourses/{number}/model.glb")
}

/// How every document and file links a key: from the bucket root, `/art/umas/100101/model.glb`,
/// so the asset origin plus the link is the file's URL.
pub(super) fn rootLink(key: &str) -> String {
    format!("/{key}")
}

impl Layout {
    /// The layout of every file the sources list, the first source naming a file deciding its
    /// dependencies, as it decides the file's build.
    pub(super) fn new(
        catalog: &Catalog,
        sources: &[(&'static CdnSource, Vec<ManifestEntry>)],
    ) -> Self {
        let mut entries: HashMap<&str, &ManifestEntry> = HashMap::new();
        for (_, files) in sources {
            for entry in files.iter().filter(|entry| entry.platform == WINDOWS) {
                entries.entry(entry.name.as_str()).or_insert(entry);
            }
        }

        let characters = catalog
            .outfits
            .iter()
            .map(|outfit| outfit.chara)
            .chain(catalog.cards.iter().map(|card| card.chara))
            .collect();
        let mut layout = Self {
            cards: catalog.cards.iter().map(|card| card.card).collect(),
            characters,
            set_owners: HashMap::new(),
            timeline_parts: BTreeMap::new(),
            full_names: HashSet::new(),
            clashes: Vec::new(),
        };
        layout.findTimelineParts(&entries);
        layout.findClashes(entries.keys().copied());
        layout
    }

    /// Where a built file goes, or `None` for the files this layout does not place: pictures,
    /// whose keys the catalog gives, and files the publisher does not build.
    pub(super) fn key(&self, name: &str) -> Option<String> {
        let key = self.naturalKey(name)?;
        if !self.full_names.contains(name) {
            return Some(key);
        }

        let (folder, file) = key.rsplit_once('/')?;
        let extension = file.rsplit_once('.').map_or("", |(_, extension)| extension);
        Some(format!("{folder}/{}.{extension}", objectName(name)))
    }

    /// The one output of a prefab or track texture.
    pub(super) fn outputs(&self, name: &str) -> Option<Vec<PlannedOutput>> {
        if prefabFamily(name).is_none() && !isTrackTexture(name) {
            return None;
        }

        Some(vec![PlannedOutput {
            key: self.key(name)?,
            object: objectName(name).to_string(),
            fit: Fit::Native,
        }])
    }

    /// The folder of a cut-in timeline's files: its uma's for a card cut-in, its character's for
    /// a character cut-in, else one of its own below `art/cutins/`.
    pub(super) fn cutinFolder(&self, timeline: &str) -> String {
        if let Some((card, number)) = numberedId(timeline, "crd") {
            if self.cards.contains(&card) {
                return umaKey(&card.to_string(), &cutinName(number));
            }
        }
        if let Some((chara, number)) = numberedId(timeline, "chr") {
            if self.characters.contains(&chara) {
                return characterKey(chara, &cutinName(number));
            }
        }
        format!("{ART_ROOT}/cutins/{timeline}")
    }

    /// The effects, sets and props a timeline such as `crd100101_001` plays.
    pub(super) fn timelineParts(&self, timeline: &str) -> &[String] {
        self.timeline_parts
            .get(timeline)
            .map_or(&[], |parts| parts.as_slice())
    }

    pub(super) fn clashes(&self) -> &[Clash] {
        &self.clashes
    }

    fn naturalKey(&self, name: &str) -> Option<String> {
        if let Some(motion) = raceMotion(name) {
            return Some(format!(
                "{ART_ROOT}/motions/race/{}/{}.glb",
                motion.set, motion.stem
            ));
        }
        if let Some(clip) = gameClip(name) {
            return Some(self.clipKey(&clip));
        }
        if isCutinTimeline(name) {
            return Some(format!(
                "{}/timeline.json",
                self.cutinFolder(objectName(name))
            ));
        }
        if isTrackTexture(name) {
            return trackTextureKey(name);
        }

        match prefabFamily(name)? {
            PrefabFamily::Track => trackPrefabKey(name),
            PrefabFamily::Effect => Some(self.effectKey(name)),
            PrefabFamily::CutinSet => Some(self.setKey(name)),
            PrefabFamily::Prop => Some(propKey(name)),
        }
    }

    fn clipKey(&self, clip: &GameClip) -> String {
        let folder = match &clip.owner {
            ClipOwner::Cutin { timeline } => {
                format!("{}/{}", self.cutinFolder(timeline), clip.role)
            }
            ClipOwner::Character { chara, .. } if self.characters.contains(chara) => {
                characterKey(*chara, &format!("chibi-motions/{}", clip.role))
            }
            ClipOwner::Character { folder, .. } | ClipOwner::Library { folder } => {
                format!("{ART_ROOT}/motions/chibi/{folder}/{}", clip.role)
            }
        };
        format!("{folder}/{}.{}", clip.short_name, clipExtension(clip.kind))
    }

    /// A character's effect sits in its folder by set, `characters/1001/effects/00/01.glb`;
    /// others below `effects/` by the game's folders, `effects/race/skill/rac_skill_00001.glb`.
    fn effectKey(&self, name: &str) -> String {
        let below = name.strip_prefix(EFFECT_FOLDER).unwrap_or(name);
        if let Some((chara, set, effect)) = characterEffect(below) {
            if self.characters.contains(&chara) {
                return characterKey(chara, &format!("effects/{set}/{effect}.glb"));
            }
        }

        let mut segments: Vec<&str> = below.split('/').map(withoutPrefabPrefix).collect();
        // `chr91001_00/chr91001_00_01` repeats its folder; `live/live_bazooka_09` its category.
        if let [.., folder, file] = segments.as_mut_slice() {
            if let Some(rest) = file
                .strip_prefix(*folder)
                .and_then(|rest| rest.strip_prefix('_'))
            {
                *file = rest;
            }
        }
        format!("{ART_ROOT}/effects/{}.glb", segments.join("/"))
    }

    /// A cut-in set sits in the folder of the timeline that plays it, `set/00.glb`.
    fn setKey(&self, name: &str) -> String {
        let below = name.strip_prefix(CUTIN_SET_FOLDER).unwrap_or(name);
        let (folder, object) = below.split_once('/').unwrap_or(("", below));
        let set = object
            .strip_prefix(&format!("pfb_env_{folder}_"))
            .unwrap_or(object);

        match self.set_owners.get(name) {
            Some(timeline) => format!("{}/set/{set}.glb", self.cutinFolder(timeline)),
            None => format!("{ART_ROOT}/cutins/sets/{folder}/{set}.glb"),
        }
    }

    /// Walks each timeline's dependencies to the effects, sets and props it plays.
    fn findTimelineParts(&mut self, entries: &HashMap<&str, &ManifestEntry>) {
        let mut set_players: HashMap<String, Vec<String>> = HashMap::new();
        let mut timelines: Vec<&str> = entries
            .keys()
            .copied()
            .filter(|name| isCutinTimeline(name))
            .collect();
        timelines.sort_unstable();

        for name in timelines {
            let timeline = objectName(name).to_string();
            let mut parts: Vec<String> = Vec::new();
            let mut seen: HashSet<&str> = HashSet::new();
            let mut pending: Vec<&str> = entries[name]
                .dependencies
                .iter()
                .map(String::as_str)
                .collect();

            while let Some(dependency) = pending.pop() {
                if dependency == SHADER_BUNDLE || !seen.insert(dependency) {
                    continue;
                }
                if let Some(entry) = entries.get(dependency) {
                    pending.extend(entry.dependencies.iter().map(String::as_str));
                }
                match prefabFamily(dependency) {
                    Some(PrefabFamily::CutinSet) => {
                        set_players
                            .entry(dependency.to_string())
                            .or_default()
                            .push(timeline.clone());
                        parts.push(dependency.to_string());
                    }
                    Some(PrefabFamily::Effect | PrefabFamily::Prop) => {
                        parts.push(dependency.to_string());
                    }
                    Some(PrefabFamily::Track) | None => {}
                }
            }

            parts.sort_unstable();
            self.timeline_parts.insert(timeline, parts);
        }

        self.set_owners = set_players
            .into_iter()
            .filter_map(|(set, mut players)| {
                players.dedup();
                (players.len() == 1).then(|| (set, players.remove(0)))
            })
            .collect();
    }

    fn findClashes<'a>(&mut self, names: impl Iterator<Item = &'a str>) {
        let mut by_key: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for name in names {
            if let Some(key) = self.naturalKey(name) {
                by_key.entry(key).or_default().push(name.to_string());
            }
        }

        for (key, mut names) in by_key {
            if names.len() < 2 {
                continue;
            }
            names.sort_unstable();
            self.full_names.extend(names.iter().cloned());
            self.clashes.push(Clash { key, names });
        }
    }
}

/// `crd100101_001` as (100101, "001") for prefix `crd`.
fn numberedId<'a>(timeline: &'a str, prefix: &str) -> Option<(u32, &'a str)> {
    let (id, number) = timeline.strip_prefix(prefix)?.split_once('_')?;
    let is_number =
        |digits: &str| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit());
    if !is_number(id) || !is_number(number) {
        return None;
    }
    Some((id.parse().ok()?, number))
}

fn cutinName(number: &str) -> String {
    if number == FIRST_CUTIN {
        "cutin".to_string()
    } else {
        format!("cutin-{number}")
    }
}

/// `charaskill/pfb_eff_chr1001_00/pfb_eff_chr1001_00_01` as (1001, "00", "01").
fn characterEffect(below: &str) -> Option<(u32, &str, &str)> {
    let (folder, object) = below.strip_prefix("charaskill/")?.split_once('/')?;
    let (chara, set) = folder.strip_prefix("pfb_eff_chr")?.split_once('_')?;
    let is_chara = chara.len() == 4 && chara.bytes().all(|byte| byte.is_ascii_digit());
    if !is_chara {
        return None;
    }
    let effect = object.strip_prefix(folder)?.strip_prefix('_')?;
    Some((chara.parse().ok()?, set, effect))
}

fn withoutPrefabPrefix(segment: &str) -> &str {
    segment
        .strip_prefix("pfb_eff_")
        .or_else(|| segment.strip_prefix("pfb_"))
        .unwrap_or(segment)
}

/// `3d/chara/prop/prop1001_00/pfb_chr_prop1001_00` at `art/props/1001_00.glb`.
fn propKey(name: &str) -> String {
    let object = objectName(name);
    let prop = object
        .strip_prefix("pfb_chr_prop")
        .or_else(|| object.strip_prefix("pfb_prop"))
        .unwrap_or(object);
    format!("{ART_ROOT}/props/{prop}.glb")
}

/// A racecourse's part in its folder, `racecourses/10006/parts/0000_002.glb`; scenery every
/// course shares by kind, `racecourses/shared/gates/00100_gate000_000.glb`.
fn trackPrefabKey(name: &str) -> Option<String> {
    let (track, _) = name.strip_prefix(TRACK_FOLDER)?.split_once('/')?;
    let object = objectName(name);

    if courseNumber(track).is_some() {
        let number = track.trim_start_matches("race");
        let part = object
            .strip_prefix(&format!("pfb_env_{track}_"))
            .unwrap_or(object);
        return Some(format!("{ART_ROOT}/racecourses/{number}/parts/{part}.glb"));
    }

    let kind = match trackGroup(name)? {
        "skyboxes" => "skies",
        "starting-gates" => "gates",
        _ => "props",
    };
    let scenery = object.strip_prefix("pfb_env_race").unwrap_or(object);
    Some(format!(
        "{ART_ROOT}/racecourses/shared/{kind}/{scenery}.glb"
    ))
}

/// `racecourses/10001/textures/0000_112_turf00.webp`, or below `racecourses/shared/textures/`.
fn trackTextureKey(name: &str) -> Option<String> {
    let (track, _) = name.strip_prefix(TRACK_FOLDER)?.split_once('/')?;
    let object = objectName(name);

    if courseNumber(track).is_some() {
        let number = track.trim_start_matches("race");
        let texture = object
            .strip_prefix(&format!("tex_env_{track}_"))
            .unwrap_or(object);
        return Some(format!(
            "{ART_ROOT}/racecourses/{number}/textures/{texture}.webp"
        ));
    }

    let texture = object.strip_prefix("tex_env_race").unwrap_or(object);
    Some(format!(
        "{ART_ROOT}/racecourses/shared/textures/{texture}.webp"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_cdn::GLOBAL_CDN;

    const TIMELINE: &str = "cutt/cutin/skill/crd100101_001/crd100101_001";
    const OWN_SET: &str = "3d/env/cutin/crd100101_001/pfb_env_crd100101_001_00";
    const SHARED_SET: &str = "3d/env/cutin/cmn_stadium/pfb_env_cmn_stadium_01";

    fn catalog() -> Catalog {
        Catalog::fromOutfits(&[CardOutfit {
            chara: 1001,
            card: 100101,
            dress: 100101,
        }])
    }

    fn layout(names: &[&str], timelines: &[(&str, &[&str])]) -> Layout {
        let entry = |name: &str| ManifestEntry::new(name, 1, 1, 1);
        let mut files: Vec<ManifestEntry> = names.iter().map(|name| entry(name)).collect();
        for (timeline, dependencies) in timelines {
            let mut timeline = entry(timeline);
            timeline.dependencies = dependencies.iter().map(|name| name.to_string()).collect();
            files.push(timeline);
        }
        Layout::new(&catalog(), &[(&GLOBAL_CDN, files)])
    }

    #[test]
    fn eachFamilyHasItsPlaceBelowArt() {
        let layout = layout(&[], &[]);
        let table = [
            (
                "3d/motion/racemain/body/type01/anm_rac_type01_run03_base",
                "art/motions/race/bright/run-fast-standard.glb",
            ),
            (
                "3d/motion/cutin/card/camera/crd100101_001/anm_cti_crd100101_001_01_cam",
                "art/umas/100101/cutin/camera/01_cam.glb",
            ),
            (
                "cutt/cutin/skill/crd100101_002/crd100101_002",
                "art/umas/100101/cutin-002/timeline.json",
            ),
            (
                "cutt/cutin/skill/chr1001_001/chr1001_001",
                "art/characters/1001/cutin/timeline.json",
            ),
            (
                "3d/effect/charaskill/pfb_eff_chr1001_00/pfb_eff_chr1001_00_01",
                "art/characters/1001/effects/00/01.glb",
            ),
            (
                "3d/effect/race/skill/pfb_eff_rac_skill_00001",
                "art/effects/race/skill/rac_skill_00001.glb",
            ),
            (
                "3d/chara/prop/prop1001_00/pfb_chr_prop1001_00",
                "art/props/1001_00.glb",
            ),
            (
                "3d/env/race/race10006/race10006_0000/pfb_env_race10006_0000_002",
                "art/racecourses/10006/parts/0000_002.glb",
            ),
            (
                "3d/env/race/race10006/race10006_0000/textures/tex_env_race10006_0000_112_turf00",
                "art/racecourses/10006/textures/0000_112_turf00.webp",
            ),
        ];

        let keys: Vec<(&str, Option<String>)> = table
            .iter()
            .map(|(name, _)| (*name, layout.key(name)))
            .collect();

        let expected: Vec<(&str, Option<String>)> = table
            .iter()
            .map(|(name, key)| (*name, Some(key.to_string())))
            .collect();
        assert_eq!(keys, expected);
    }

    #[test]
    fn aCutinOfACardTheMasterDoesNotKnowGetsAFolderOfItsOwn() {
        let layout = layout(&[], &[]);

        let clip =
            layout.key("3d/motion/cutin/card/camera/crd109901_001/anm_cti_crd109901_001_01_cam");

        assert_eq!(
            clip.as_deref(),
            Some("art/cutins/crd109901_001/camera/01_cam.glb")
        );
    }

    #[test]
    fn aSetGoesWithTheOneTimelineThatPlaysIt() {
        let layout = layout(
            &[OWN_SET, SHARED_SET],
            &[
                (TIMELINE, &[OWN_SET, SHARED_SET]),
                (
                    "cutt/cutin/skill/crd100102_001/crd100102_001",
                    &[SHARED_SET],
                ),
            ],
        );

        assert_eq!(
            layout.key(OWN_SET).as_deref(),
            Some("art/umas/100101/cutin/set/00.glb")
        );
        assert_eq!(
            layout.key(SHARED_SET).as_deref(),
            Some("art/cutins/sets/cmn_stadium/01.glb")
        );
        assert_eq!(layout.timelineParts("crd100101_001"), [SHARED_SET, OWN_SET]);
    }

    #[test]
    fn filesWhoseKeysCollideKeepTheirFullNames() {
        let effect = "3d/effect/common/pfb_eff_fade";
        let prefab = "3d/effect/common/pfb_fade";
        let layout = layout(&[effect, prefab], &[]);

        let keys = [layout.key(effect), layout.key(prefab)];

        assert_eq!(
            keys,
            [
                Some("art/effects/common/pfb_eff_fade.glb".to_string()),
                Some("art/effects/common/pfb_fade.glb".to_string()),
            ]
        );
        assert_eq!(
            layout.clashes(),
            [Clash {
                key: "art/effects/common/fade.glb".to_string(),
                names: vec![effect.to_string(), prefab.to_string()],
            }]
        );
    }

    #[test]
    fn anOutfitNoCardWearsIsNamedAfterItsCharacterAndOutfit() {
        let cards = [CardOutfit {
            chara: 1001,
            card: 100102,
            dress: 100130,
        }];
        let outfit = |chara, dress| OutfitRow {
            dress,
            chara,
            has_chibi: false,
            body_sub: 0,
            head_sub: 0,
            dress_tail: (-1, 0),
            chara_tail: -1,
            skin: 1,
            running_type: 1,
            run_personality: 1,
            finish_personality: 1,
            chara_name: None,
            dress_name: None,
        };

        let folders = umaFolders(&cards, &[outfit(1001, 100130), outfit(1066, 106601)]);

        assert_eq!(folders[&100130], "100102");
        assert_eq!(folders[&106601], "1066-106601");
    }
}

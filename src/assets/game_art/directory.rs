//! The JSON documents that make `art/` browsable and tie related files together. One document per
//! uma lists every file it loads, from its model and race clips to its cut-ins; character,
//! racecourse and library documents list what lies below them. Every path is a link from the
//! bucket root, and a document lists only files that converted.

use super::catalog::{isCutinTimeline, objectName, prefabFamily};
use super::layout::{characterKey, rootLink, umaKey, Layout, ART_ROOT, SHADER_INDEX_KEY};
use super::race_motions::raceMotion;
use super::types::{
    ArtState, CardOutfit, Catalog, OutfitRow, PrefabFamily, PublishedArt, UmaTraits,
    DOCUMENT_SCHEMA,
};
use anyhow::Result;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use tracing::warn;

const INDEX_FILE: &str = "index.json";

/// Folders below `art/` listed file by file, for browsing; nothing needs them to load an uma.
const LIBRARY_FOLDERS: &[&str] = &[
    "motions", "effects", "props", "cutins", "skills", "races", "items", "ui",
];

const SHARED_SCENERY_FOLDER: &str = "racecourses/shared";
const TIMELINE_SUFFIX: &str = "/timeline.json";
const RUNNING_FORMS: &[&str] = &["standard", "pitch", "stride"];

/// Every directory document by bucket key, as compact JSON.
pub(super) fn directoryDocuments(
    catalog: &Catalog,
    layout: &Layout,
    state: &ArtState,
) -> Result<BTreeMap<String, Vec<u8>>> {
    let directory = Directory::new(catalog, layout, state);
    let mut documents = Documents::default();
    let mut sections: BTreeMap<&str, String> = BTreeMap::new();

    let umas = directory.umas();
    for uma in &umas {
        documents.put(umaKey(uma.folder, INDEX_FILE), &directory.umaDocument(uma))?;
    }
    if !umas.is_empty() {
        let summaries: Vec<UmaSummary> = umas.iter().map(|uma| directory.umaSummary(uma)).collect();
        sections.insert(
            "umas",
            documents.put(folderIndexKey("umas"), &UmasDocument::new(summaries))?,
        );
    }

    let cards = directory.supportCards();
    let characters = directory.characterDocuments(&umas, &cards);
    if !characters.is_empty() {
        let mut summaries = Vec::new();
        for character in &characters {
            let index = documents.put(characterKey(character.id, INDEX_FILE), character)?;
            summaries.push(CharacterSummary {
                id: character.id,
                name: character.name,
                index,
            });
        }
        let document = CharactersDocument {
            schema: DOCUMENT_SCHEMA,
            characters: summaries,
        };
        sections.insert(
            "characters",
            documents.put(folderIndexKey("characters"), &document)?,
        );
    }

    let mut courses = Vec::new();
    for course in directory.racecourses() {
        let index = documents.put(courseKey(course.id, INDEX_FILE), &course)?;
        courses.push(CourseSummary {
            id: course.id,
            name: course.name,
            index,
        });
    }
    let shared = directory.listings(SHARED_SCENERY_FOLDER);
    let shared_index =
        (!shared.is_empty()).then(|| rootLink(&folderIndexKey(SHARED_SCENERY_FOLDER)));
    documents.putListings(shared)?;
    if !courses.is_empty() || shared_index.is_some() {
        let document = RacecoursesDocument {
            schema: DOCUMENT_SCHEMA,
            racecourses: courses,
            shared: shared_index,
        };
        sections.insert(
            "racecourses",
            documents.put(folderIndexKey("racecourses"), &document)?,
        );
    }

    if !cards.is_empty() {
        let document = SupportCardsDocument {
            schema: DOCUMENT_SCHEMA,
            support_cards: cards,
        };
        sections.insert(
            "supportCards",
            documents.put(folderIndexKey("support-cards"), &document)?,
        );
    }

    for folder in LIBRARY_FOLDERS {
        let listings = directory.listings(folder);
        if !listings.is_empty() {
            sections.insert(folder, rootLink(&folderIndexKey(folder)));
        }
        documents.putListings(listings)?;
    }
    if let Some(shaders) = directory.published.link(SHADER_INDEX_KEY) {
        sections.insert("shaders", shaders);
    }

    if !sections.is_empty() {
        let document = RootDocument {
            schema: DOCUMENT_SCHEMA,
            sections,
        };
        documents.put(format!("{ART_ROOT}/{INDEX_FILE}"), &document)?;
    }
    Ok(documents.documents)
}

/// An outfit that may have an uma folder: every outfit with a body, and every card.
struct Uma<'a> {
    folder: &'a str,
    dress: u32,
    chara: u32,
    card: Option<u32>,
    name: Option<&'a str>,
    traits: Option<&'a UmaTraits>,
}

/// A cut-in timeline and the folder its clips are in.
struct Cutin<'a> {
    timeline: &'a str,
    folder: &'a str,
}

struct Directory<'a> {
    catalog: &'a Catalog,
    layout: &'a Layout,
    published: Published<'a>,
    /// Published race clips by motion set, as (file name, key) in file name order.
    race_clips: BTreeMap<String, Vec<(String, &'a str)>>,
    cutins: Vec<Cutin<'a>>,
}

impl<'a> Directory<'a> {
    fn new(catalog: &'a Catalog, layout: &'a Layout, state: &'a ArtState) -> Self {
        let published = Published::new(state);
        let mut race_clips: BTreeMap<String, Vec<(String, &str)>> = BTreeMap::new();
        let mut cutins = Vec::new();
        for name in state.files.keys() {
            let Some(key) = published.fileKey(name) else {
                continue;
            };
            if let Some(motion) = raceMotion(name) {
                race_clips
                    .entry(motion.set)
                    .or_default()
                    .push((motion.stem, key));
            } else if isCutinTimeline(name) {
                if let Some(folder) = key.strip_suffix(TIMELINE_SUFFIX) {
                    cutins.push(Cutin {
                        timeline: objectName(name),
                        folder,
                    });
                }
            }
        }
        for clips in race_clips.values_mut() {
            clips.sort_unstable();
        }
        cutins.sort_unstable_by_key(|cutin| cutin.timeline);

        Self {
            catalog,
            layout,
            published,
            race_clips,
            cutins,
        }
    }

    /// The outfits with anything published in their folder, by character and outfit.
    fn umas(&self) -> Vec<Uma<'a>> {
        let catalog = self.catalog;
        let mut umas: Vec<Uma> = catalog
            .uma_folders
            .iter()
            .filter_map(|(&dress, folder)| {
                let row: Option<&OutfitRow> = catalog.outfits.iter().find(|row| row.dress == dress);
                let card: Option<&CardOutfit> =
                    catalog.cards.iter().find(|card| card.dress == dress);
                let chara = row.map(|row| row.chara).or(card.map(|card| card.chara))?;
                Some(Uma {
                    folder,
                    dress,
                    chara,
                    card: card.map(|card| card.card),
                    name: row.and_then(|row| row.dress_name.as_deref()),
                    traits: catalog
                        .umas
                        .iter()
                        .find(|uma| uma.traits.dress == dress)
                        .map(|uma| &uma.traits),
                })
            })
            .filter(|uma| {
                self.published
                    .under(&umaKey(uma.folder, ""))
                    .next()
                    .is_some()
            })
            .collect();
        umas.sort_unstable_by_key(|uma| (uma.chara, uma.dress));
        umas
    }

    fn umaDocument(&self, uma: &Uma<'a>) -> UmaDocument<'a> {
        let model_key = umaKey(uma.folder, "model.glb");
        let model = self.published.link(&model_key).map(|file| ModelEntry {
            file,
            shaders: self.published.shaders(&model_key),
            motions: uma
                .traits
                .map(|traits| self.raceMotions(traits))
                .unwrap_or_default(),
        });

        let chibi_key = umaKey(uma.folder, "chibi.glb");
        let chibi_file = self.published.link(&chibi_key);
        let sprites = self.published.links(&umaKey(uma.folder, "sprites/"));
        let chibi = (chibi_file.is_some() || !sprites.is_empty()).then(|| ChibiEntry {
            motions: match chibi_file {
                Some(_) => self
                    .published
                    .byRole(&characterKey(uma.chara, "chibi-motions/"), &[]),
                None => BTreeMap::new(),
            },
            shaders: self.published.shaders(&chibi_key),
            file: chibi_file,
            sprites,
        });

        let own_folder = umaKey(uma.folder, "");
        let character_folder = characterKey(uma.chara, "");
        let own_cutins = self
            .cutins
            .iter()
            .filter(|cutin| cutin.folder.starts_with(&own_folder));
        let character_cutins = self
            .cutins
            .iter()
            .filter(|cutin| cutin.folder.starts_with(&character_folder));

        UmaDocument {
            schema: DOCUMENT_SCHEMA,
            id: uma.card,
            name: uma.name,
            character: self.characterLink(uma.chara),
            outfit: uma.dress,
            portrait: self.published.link(&umaKey(uma.folder, "portrait.webp")),
            model,
            chibi,
            cutins: own_cutins
                .chain(character_cutins)
                .filter_map(|cutin| self.cutinEntry(cutin))
                .collect(),
        }
    }

    /// The race clips the uma plays: runs of its running form and gate starts from its run set,
    /// and finish reactions from its finish set.
    fn raceMotions(&self, traits: &UmaTraits) -> BTreeMap<&'static str, Vec<String>> {
        let mut motions: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
        let clips_of = |set: &str| {
            self.race_clips
                .get(set)
                .map_or(&[][..], |clips| clips.as_slice())
        };

        for (stem, key) in clips_of(&traits.run_motions) {
            let group = match motionGroup(stem) {
                Some(MotionGroup::Run) if fitsRunningForm(stem, traits.running_form) => "run",
                Some(MotionGroup::Gate) => "gate",
                _ => continue,
            };
            motions.entry(group).or_default().push(rootLink(key));
        }
        for (stem, key) in clips_of(&traits.finish_motions) {
            if motionGroup(stem) == Some(MotionGroup::Finish) {
                motions.entry("finish").or_default().push(rootLink(key));
            }
        }
        motions
    }

    fn cutinEntry(&self, cutin: &Cutin) -> Option<CutinEntry> {
        let category = cutinCategory(cutin.timeline)?;
        let mut entry = CutinEntry {
            category,
            timeline: rootLink(&format!("{}{TIMELINE_SUFFIX}", cutin.folder)),
            clips: self
                .published
                .byRole(&format!("{}/", cutin.folder), &["set"]),
            sets: Vec::new(),
            effects: Vec::new(),
            props: Vec::new(),
            shaders: Vec::new(),
        };

        let mut shaders = BTreeSet::new();
        for part in self.layout.timelineParts(cutin.timeline) {
            let Some(key) = self.published.fileKey(part) else {
                continue;
            };
            let list = match prefabFamily(part) {
                Some(PrefabFamily::CutinSet) => &mut entry.sets,
                Some(PrefabFamily::Effect) => &mut entry.effects,
                Some(PrefabFamily::Prop) => &mut entry.props,
                Some(PrefabFamily::Track) | None => continue,
            };
            list.push(rootLink(key));
            shaders.extend(self.published.shaders(key));
        }
        entry.shaders = shaders.into_iter().collect();
        Some(entry)
    }

    fn characterLink(&self, chara: u32) -> CharacterLink<'a> {
        CharacterLink {
            id: chara,
            name: self.characterName(chara),
            icon: self.published.link(&characterKey(chara, "icon.webp")),
            index: rootLink(&characterKey(chara, INDEX_FILE)),
        }
    }

    fn characterName(&self, chara: u32) -> Option<&'a str> {
        self.catalog
            .outfits
            .iter()
            .filter(|row| row.chara == chara)
            .find_map(|row| row.chara_name.as_deref())
    }

    fn umaSummary(&self, uma: &Uma<'a>) -> UmaSummary<'a> {
        UmaSummary {
            id: uma.card,
            outfit: uma.dress,
            name: uma.name,
            character: CharacterName {
                id: uma.chara,
                name: self.characterName(uma.chara),
            },
            index: rootLink(&umaKey(uma.folder, INDEX_FILE)),
        }
    }

    /// A document for every character with an uma, an icon or a support card.
    fn characterDocuments(
        &self,
        umas: &[Uma<'a>],
        cards: &[SupportCardSummary],
    ) -> Vec<CharacterDocument<'a>> {
        let charas: BTreeSet<u32> = self
            .catalog
            .outfits
            .iter()
            .map(|row| row.chara)
            .chain(self.catalog.cards.iter().map(|card| card.chara))
            .chain(self.catalog.support_cards.iter().map(|card| card.chara))
            .collect();

        charas
            .into_iter()
            .filter_map(|chara| {
                let document = CharacterDocument {
                    schema: DOCUMENT_SCHEMA,
                    id: chara,
                    name: self.characterName(chara),
                    icon: self.published.link(&characterKey(chara, "icon.webp")),
                    umas: umas
                        .iter()
                        .filter(|uma| uma.chara == chara)
                        .map(|uma| UmaLink {
                            id: uma.card,
                            outfit: uma.dress,
                            name: uma.name,
                            index: rootLink(&umaKey(uma.folder, INDEX_FILE)),
                        })
                        .collect(),
                    support_cards: cards
                        .iter()
                        .filter(|card| card.character == chara)
                        .map(|card| CardLinks {
                            id: card.id,
                            card: card.card.clone(),
                            half: card.half.clone(),
                        })
                        .collect(),
                };
                let is_empty = document.icon.is_none()
                    && document.umas.is_empty()
                    && document.support_cards.is_empty();
                (!is_empty).then_some(document)
            })
            .collect()
    }

    fn supportCards(&self) -> Vec<SupportCardSummary> {
        self.catalog
            .support_cards
            .iter()
            .filter_map(|card| {
                let picture = |file: &str| {
                    self.published
                        .link(&format!("{ART_ROOT}/support-cards/{}/{file}", card.id))
                };
                let (full, half) = (picture("card.webp"), picture("half.webp"));
                if full.is_none() && half.is_none() {
                    return None;
                }
                Some(SupportCardSummary {
                    id: card.id,
                    character: card.chara,
                    card: full,
                    half,
                })
            })
            .collect()
    }

    fn racecourses(&self) -> Vec<CourseDocument<'a>> {
        let folder = format!("{ART_ROOT}/racecourses/");
        let numbers: BTreeSet<u32> = self
            .published
            .under(&folder)
            .filter_map(|key| key[folder.len()..].split_once('/')?.0.parse().ok())
            .collect();

        numbers
            .into_iter()
            .map(|id| {
                let model_key = courseKey(id, "model.glb");
                let parts_folder = courseKey(id, "parts/");
                let shaders: BTreeSet<String> = std::iter::once(model_key.as_str())
                    .chain(self.published.under(&parts_folder))
                    .flat_map(|key| self.published.shaders(key))
                    .collect();
                CourseDocument {
                    schema: DOCUMENT_SCHEMA,
                    id,
                    name: self.catalog.course_names.get(&id).map(String::as_str),
                    model: self.published.link(&model_key),
                    shaders: shaders.into_iter().collect(),
                    parts: self.published.links(&parts_folder),
                    textures: self.published.links(&courseKey(id, "textures/")),
                }
            })
            .collect()
    }

    /// A listing for every folder below `art/<folder>/`, by its document's key.
    fn listings(&self, folder: &str) -> BTreeMap<String, Listing> {
        let root = format!("{ART_ROOT}/{folder}/");
        let mut listings: BTreeMap<String, Listing> = BTreeMap::new();
        for key in self.published.under(&root) {
            let mut current = root.clone();
            let mut segments: Vec<&str> = key[root.len()..].split('/').collect();
            segments.pop();
            for segment in segments {
                let child = format!("{current}{segment}/");
                listings
                    .entry(current)
                    .or_default()
                    .folders
                    .insert(rootLink(&format!("{child}{INDEX_FILE}")));
                current = child;
            }
            listings
                .entry(current)
                .or_default()
                .files
                .push(rootLink(key));
        }

        listings
            .into_iter()
            .filter_map(|(folder, listing)| {
                let key = format!("{folder}{INDEX_FILE}");
                if self.published.link(&key).is_some() {
                    warn!(key, "a published file takes the place of a folder listing");
                    return None;
                }
                Some((key, listing))
            })
            .collect()
    }
}

/// What the bucket holds by the publisher's records: every key of a file that converted.
struct Published<'a> {
    state: &'a ArtState,
    records: BTreeMap<&'a str, &'a PublishedArt>,
}

impl<'a> Published<'a> {
    fn new(state: &'a ArtState) -> Self {
        let records = state
            .files
            .values()
            .filter(|record| record.error.is_none())
            .flat_map(|record| {
                record
                    .outputs
                    .iter()
                    .chain(&record.content_outputs)
                    .map(move |key| (key.as_str(), record))
            })
            .collect();
        Self { state, records }
    }

    fn link(&self, key: &str) -> Option<String> {
        self.records.contains_key(key).then(|| rootLink(key))
    }

    /// The keys below a folder given with its trailing slash, in key order.
    fn under<'s>(&'s self, folder: &'s str) -> impl Iterator<Item = &'a str> + 's {
        self.records
            .range::<str, _>((Bound::Included(folder), Bound::Unbounded))
            .map(|(key, _)| *key)
            .take_while(move |key| key.starts_with(folder))
    }

    fn links(&self, folder: &str) -> Vec<String> {
        self.under(folder).map(rootLink).collect()
    }

    /// The keys below a folder by the folder they are in directly, such as a cut-in's `body` and
    /// `camera`, leaving out files directly in it and the folders in `skip`.
    fn byRole(&self, folder: &str, skip: &[&str]) -> BTreeMap<String, Vec<String>> {
        let mut roles: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for key in self.under(folder) {
            let Some((role, _)) = key[folder.len()..].split_once('/') else {
                continue;
            };
            if !skip.contains(&role) {
                roles
                    .entry(role.to_string())
                    .or_default()
                    .push(rootLink(key));
            }
        }
        roles
    }

    /// The published shaders the file at `key` reported, as links.
    fn shaders(&self, key: &str) -> Vec<String> {
        let Some(record) = self.records.get(key) else {
            return Vec::new();
        };
        record
            .shaders
            .iter()
            .filter(|shader| {
                shader
                    .strip_prefix('/')
                    .is_some_and(|shader_key| self.records.contains_key(shader_key))
            })
            .cloned()
            .collect()
    }

    /// The key a game file was published at, if it converted.
    fn fileKey(&self, name: &str) -> Option<&'a str> {
        let record = self.state.files.get(name)?;
        if record.error.is_some() {
            return None;
        }
        record.outputs.first().map(String::as_str)
    }
}

#[derive(Debug, PartialEq)]
enum MotionGroup {
    Run,
    Gate,
    Finish,
}

fn motionGroup(stem: &str) -> Option<MotionGroup> {
    if stem.starts_with("run-") {
        return Some(MotionGroup::Run);
    }
    if stem.starts_with("gate-") {
        return Some(MotionGroup::Gate);
    }
    if stem.starts_with("finish-") || stem.starts_with("cooldown-") {
        return Some(MotionGroup::Finish);
    }
    None
}

/// Whether a run clip is one of the given form's, or one every form plays.
fn fitsRunningForm(stem: &str, form: &str) -> bool {
    let clip_form = RUNNING_FORMS
        .iter()
        .find(|clip_form| stem.ends_with(&format!("-{clip_form}")));
    clip_form.is_none_or(|clip_form| *clip_form == form)
}

/// A card's cut-in plays its unique skill at top rarity; a character's, the inherited one.
fn cutinCategory(timeline: &str) -> Option<&'static str> {
    if timeline.starts_with("crd") {
        return Some("unique-rare");
    }
    if timeline.starts_with("chr") {
        return Some("unique");
    }
    None
}

/// The index of a folder below `art/`: `art/umas/index.json`.
fn folderIndexKey(folder: &str) -> String {
    format!("{ART_ROOT}/{folder}/{INDEX_FILE}")
}

fn courseKey(course: u32, file: &str) -> String {
    format!("{ART_ROOT}/racecourses/{course}/{file}")
}

#[derive(Default)]
struct Documents {
    documents: BTreeMap<String, Vec<u8>>,
}

impl Documents {
    /// Adds the document and returns its link.
    fn put(&mut self, key: String, document: &impl Serialize) -> Result<String> {
        let link = rootLink(&key);
        self.documents.insert(key, serde_json::to_vec(document)?);
        Ok(link)
    }

    fn putListings(&mut self, listings: BTreeMap<String, Listing>) -> Result<()> {
        for (key, mut listing) in listings {
            listing.schema = DOCUMENT_SCHEMA;
            self.put(key, &listing)?;
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct RootDocument<'a> {
    schema: u32,
    sections: BTreeMap<&'a str, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UmaDocument<'a> {
    schema: u32,
    /// The site's id of a playable outfit: its card.
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    character: CharacterLink<'a>,
    /// The game's outfit id, which NPC race rows and outfit changes give.
    outfit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    portrait: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<ModelEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chibi: Option<ChibiEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cutins: Vec<CutinEntry>,
}

#[derive(Serialize)]
struct CharacterLink<'a> {
    id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    index: String,
}

#[derive(Serialize)]
struct ModelEntry {
    file: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shaders: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    motions: BTreeMap<&'static str, Vec<String>>,
}

#[derive(Serialize)]
struct ChibiEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shaders: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    sprites: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    motions: BTreeMap<String, Vec<String>>,
}

#[derive(Serialize)]
struct CutinEntry {
    category: &'static str,
    timeline: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    clips: BTreeMap<String, Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    sets: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    effects: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    props: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shaders: Vec<String>,
}

#[derive(Serialize)]
struct UmasDocument<'a> {
    schema: u32,
    umas: Vec<UmaSummary<'a>>,
}

impl<'a> UmasDocument<'a> {
    fn new(umas: Vec<UmaSummary<'a>>) -> Self {
        Self {
            schema: DOCUMENT_SCHEMA,
            umas,
        }
    }
}

#[derive(Serialize)]
struct UmaSummary<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u32>,
    outfit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    character: CharacterName<'a>,
    index: String,
}

#[derive(Serialize)]
struct CharacterName<'a> {
    id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CharacterDocument<'a> {
    schema: u32,
    id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    umas: Vec<UmaLink<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    support_cards: Vec<CardLinks>,
}

#[derive(Serialize)]
struct UmaLink<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u32>,
    outfit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    index: String,
}

#[derive(Serialize)]
struct CardLinks {
    id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    card: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    half: Option<String>,
}

#[derive(Serialize)]
struct CharactersDocument<'a> {
    schema: u32,
    characters: Vec<CharacterSummary<'a>>,
}

#[derive(Serialize)]
struct CharacterSummary<'a> {
    id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    index: String,
}

#[derive(Serialize)]
struct CourseDocument<'a> {
    schema: u32,
    id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shaders: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    parts: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    textures: Vec<String>,
}

#[derive(Serialize)]
struct RacecoursesDocument<'a> {
    schema: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    racecourses: Vec<CourseSummary<'a>>,
    /// The listing of scenery every course shares: skies, gates and props.
    #[serde(skip_serializing_if = "Option::is_none")]
    shared: Option<String>,
}

#[derive(Serialize)]
struct CourseSummary<'a> {
    id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    index: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SupportCardsDocument {
    schema: u32,
    support_cards: Vec<SupportCardSummary>,
}

#[derive(Serialize)]
struct SupportCardSummary {
    id: u32,
    character: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    card: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    half: Option<String>,
}

/// A library folder's links to the listings of the folders in it and to its files.
#[derive(Default, Serialize)]
struct Listing {
    schema: u32,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    folders: BTreeSet<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::game_art::types::SHADER_BUNDLE;
    use crate::game_cdn::{CdnSource, ManifestEntry, GLOBAL_CDN};
    use serde_json::{json, Value};

    const TIMELINE: &str = "cutt/cutin/skill/crd100101_001/crd100101_001";
    const CAMERA_CLIP: &str =
        "3d/motion/cutin/card/camera/crd100101_001/anm_cti_crd100101_001_01_cam";
    const BODY_CLIP: &str = "3d/motion/cutin/card/body/crd100101_001/anm_cti_crd100101_001_01";
    const EFFECT: &str = "3d/effect/charaskill/pfb_eff_chr1001_00/pfb_eff_chr1001_00_01";
    const FAST_RUN: &str = "3d/motion/racemain/body/type01/anm_rac_type01_run03_base";
    const PITCH_RUN: &str = "3d/motion/racemain/body/type01/anm_rac_type01_run03_pitch";
    const BLOCKED_RUN: &str = "3d/motion/racemain/body/type01/anm_rac_type01_run04";
    const GATE_START: &str = "3d/motion/racemain/body/type01/anm_rac_type01_start_go_l";
    const TOON: &str = "/art/shaders/gallop/3d/chara/toon/tser.json";
    const ADDITIVE: &str = "/art/shaders/cygames/effect/additive.json";

    fn outfitRow(chara: u32, dress: u32, chara_name: &str, dress_name: &str) -> OutfitRow {
        OutfitRow {
            dress,
            chara,
            has_chibi: true,
            body_sub: 0,
            head_sub: 0,
            dress_tail: (-1, 0),
            chara_tail: 1,
            skin: 1,
            running_type: 1,
            run_personality: 1,
            finish_personality: 1,
            chara_name: Some(chara_name.to_string()),
            dress_name: Some(dress_name.to_string()),
        }
    }

    /// Special Week with her first card, and Twin Turbo, whom no card shows.
    fn catalog() -> Catalog {
        let mut catalog = Catalog::fromOutfits(&[CardOutfit {
            chara: 1001,
            card: 100101,
            dress: 100101,
        }]);
        catalog.setOutfits(vec![
            outfitRow(1001, 100101, "Special Week", "Special Dreamer"),
            outfitRow(1066, 106601, "Twin Turbo", "Turbo Racewear"),
        ]);
        catalog
    }

    fn sources() -> Vec<(&'static CdnSource, Vec<ManifestEntry>)> {
        let entry = |name: &str| ManifestEntry::new(name, 1, 1, 1);
        let mut timeline = entry(TIMELINE);
        timeline.dependencies = vec![EFFECT.to_string()];
        let files = [
            CAMERA_CLIP,
            BODY_CLIP,
            EFFECT,
            FAST_RUN,
            PITCH_RUN,
            BLOCKED_RUN,
            GATE_START,
        ]
        .into_iter()
        .map(entry)
        .chain(std::iter::once(timeline))
        .collect();
        vec![(&GLOBAL_CDN, files)]
    }

    fn record(key: &str, shaders: &[&str]) -> PublishedArt {
        PublishedArt {
            region: "global".to_string(),
            checksum: 1,
            converter: 1,
            outputs: vec![key.to_string()],
            dependency_checksums: Vec::new(),
            content_outputs: Vec::new(),
            textures: Vec::new(),
            error: None,
            shaders: shaders.iter().map(|shader| shader.to_string()).collect(),
            links: None,
        }
    }

    /// Every file above published, except the cut-in body clip, which failed.
    fn publishedState(catalog: &Catalog, layout: &Layout) -> ArtState {
        let mut state = ArtState::default();
        let mut publish = |name: &str, key: String, shaders: &[&str]| {
            state.files.insert(name.to_string(), record(&key, shaders));
        };
        for uma in &catalog.umas {
            publish(&uma.record, uma.key.clone(), &[TOON]);
        }
        for name in [
            TIMELINE,
            CAMERA_CLIP,
            FAST_RUN,
            PITCH_RUN,
            BLOCKED_RUN,
            GATE_START,
        ] {
            publish(name, layout.key(name).unwrap(), &[]);
        }
        publish(EFFECT, layout.key(EFFECT).unwrap(), &[ADDITIVE]);
        publish(
            "chara/chr1001/chr_icon_1001",
            characterKey(1001, "icon.webp"),
            &[],
        );
        let mut shaders = record(SHADER_INDEX_KEY, &[]);
        shaders.content_outputs = [TOON, ADDITIVE].map(|link| link[1..].to_string()).to_vec();
        state.files.insert(SHADER_BUNDLE.to_string(), shaders);

        let mut failed = record(&layout.key(BODY_CLIP).unwrap(), &[]);
        failed.error = Some("broken".to_string());
        state.files.insert(BODY_CLIP.to_string(), failed);
        state
    }

    fn documents() -> BTreeMap<String, Vec<u8>> {
        let catalog = catalog();
        let layout = Layout::new(&catalog, &sources());
        directoryDocuments(&catalog, &layout, &publishedState(&catalog, &layout)).unwrap()
    }

    fn document(documents: &BTreeMap<String, Vec<u8>>, key: &str) -> Value {
        serde_json::from_slice(&documents[key]).unwrap()
    }

    fn collectLinks(value: &Value, found: &mut Vec<String>) {
        match value {
            Value::String(text) if text.starts_with('/') => found.push(text.clone()),
            Value::Array(items) => items.iter().for_each(|item| collectLinks(item, found)),
            Value::Object(fields) => fields.values().for_each(|field| collectLinks(field, found)),
            _ => {}
        }
    }

    #[test]
    fn anUmasDocumentLinksEverythingItLoadsFromTheBucketRoot() {
        let documents = documents();

        let uma = document(&documents, "art/umas/100101/index.json");

        assert_eq!(uma["id"], 100101);
        assert_eq!(uma["name"], "Special Dreamer");
        assert_eq!(uma["outfit"], 100101);
        assert_eq!(
            uma["character"],
            json!({
                "id": 1001,
                "name": "Special Week",
                "icon": "/art/characters/1001/icon.webp",
                "index": "/art/characters/1001/index.json",
            })
        );
        assert_eq!(
            uma["model"],
            json!({
                "file": "/art/umas/100101/model.glb",
                "shaders": [TOON],
                "motions": {
                    "gate": ["/art/motions/race/bright/gate-break-left-foot.glb"],
                    "run": [
                        "/art/motions/race/bright/run-blocked.glb",
                        "/art/motions/race/bright/run-fast-standard.glb",
                    ],
                },
            })
        );
    }

    #[test]
    fn aCutinListsWhatItPlaysButNotWhatFailed() {
        let documents = documents();

        let uma = document(&documents, "art/umas/100101/index.json");

        assert_eq!(
            uma["cutins"],
            json!([{
                "category": "unique-rare",
                "timeline": "/art/umas/100101/cutin/timeline.json",
                "clips": {"camera": ["/art/umas/100101/cutin/camera/01_cam.glb"]},
                "effects": ["/art/characters/1001/effects/00/01.glb"],
                "shaders": [ADDITIVE],
            }])
        );
    }

    #[test]
    fn anOutfitNoCardShowsIsFoundThroughItsCharacter() {
        let documents = documents();

        let umas = document(&documents, "art/umas/index.json");
        let character = document(&documents, "art/characters/1066/index.json");

        assert_eq!(
            umas["umas"][1],
            json!({
                "outfit": 106601,
                "name": "Turbo Racewear",
                "character": {"id": 1066, "name": "Twin Turbo"},
                "index": "/art/umas/1066-106601/index.json",
            })
        );
        assert_eq!(
            character["umas"],
            json!([{
                "outfit": 106601,
                "name": "Turbo Racewear",
                "index": "/art/umas/1066-106601/index.json",
            }])
        );
    }

    #[test]
    fn libraryFoldersAreListedFolderByFolder() {
        let documents = documents();

        let motions = document(&documents, "art/motions/index.json");
        let bright = document(&documents, "art/motions/race/bright/index.json");

        assert_eq!(
            motions,
            json!({"schema": 1, "folders": ["/art/motions/race/index.json"]})
        );
        assert_eq!(bright["files"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn everyLinkInEveryDocumentIsAFileOrADocument() {
        let catalog = catalog();
        let layout = Layout::new(&catalog, &sources());
        let state = publishedState(&catalog, &layout);
        let documents = directoryDocuments(&catalog, &layout, &state).unwrap();
        let published = Published::new(&state);

        let mut found = Vec::new();
        for bytes in documents.values() {
            collectLinks(&serde_json::from_slice(bytes).unwrap(), &mut found);
        }

        let missing: Vec<&String> = found
            .iter()
            .filter(|link| {
                let key = &link[1..];
                published.link(key).is_none() && !documents.contains_key(key)
            })
            .collect();
        assert!(!found.is_empty());
        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn theSameRecordsGiveTheSameBytes() {
        let first = documents();

        let second = documents();

        assert_eq!(first, second);
    }
}

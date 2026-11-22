//! JP's gachas, JP's own names, and the English names of what only JP has released. The sync
//! job reads both masters, writes them into `src/jp_data`, and translates new names with Claude;
//! the resources read the bundled copies, so the servers never call the model.

#![allow(non_snake_case)]

#[path = "../types/jp_names.rs"]
mod types;

mod book;
mod master;
mod translate;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tracing::{info, warn};
use translate::{Translator, BATCH_SIZE};
pub use types::{CardNames, JpGacha, JpGachaData, NameBook, NameKind, NameLoc};
use types::{
    MachineName, MasterNames, NameOverrides, NameTranslations, TranslationExample, TranslationItem,
    GACHAS_FILE, JAPANESE_NAMES_FILE, TRANSLATIONS_FILE, TRANSLATION_MODEL,
};

const BUNDLED_GACHAS: &[u8] = include_bytes!("../jp_data/jp_gachas.json");
const BUNDLED_TRANSLATIONS: &[u8] = include_bytes!("../jp_data/name_translations.json");
const BUNDLED_OVERRIDES: &[u8] = include_bytes!("../jp_data/name_overrides.json");
const BUNDLED_JAPANESE_NAMES: &[u8] = include_bytes!("../jp_data/jp_names.json");
const OVERRIDES_FILE: &str = "name_overrides.json";

/// Uma names first, so titles can name their character in English.
const KINDS: [NameKind; 3] = [
    NameKind::UmaName,
    NameKind::OutfitTitle,
    NameKind::SupportTitle,
];

/// Feeds the bundled files into a resource version, so a sync regenerates the resources.
pub fn hashBundled(digest: &mut Sha256) {
    digest.update(BUNDLED_GACHAS);
    digest.update(BUNDLED_TRANSLATIONS);
    digest.update(BUNDLED_OVERRIDES);
    digest.update(BUNDLED_JAPANESE_NAMES);
}

pub fn bundledGachaData() -> Result<JpGachaData> {
    serde_json::from_slice(BUNDLED_GACHAS).context("failed to parse bundled jp_gachas.json")
}

/// JP's gachas with pickups that are newer than the newest of `known` in the same id range. Ids
/// grow with time, so these are the gachas a curated list has not reached yet. A gacha without
/// fixed pickups, such as a select or monthly one, is left to the news archive, which names what
/// it features.
pub fn gachasAfter(known: &BTreeSet<i64>) -> Result<Vec<JpGacha>> {
    Ok(newerGachas(bundledGachaData()?.gachas, known))
}

fn newerGachas(gachas: Vec<JpGacha>, known: &BTreeSet<i64>) -> Vec<JpGacha> {
    let newest = |range: i64| {
        known
            .iter()
            .filter(|id| *id / 10000 == range)
            .max()
            .copied()
    };
    gachas
        .into_iter()
        .filter(|gacha| !gacha.pickup_card_ids.is_empty())
        .filter(|gacha| newest(gacha.gacha_id / 10000).is_some_and(|id| gacha.gacha_id > id))
        .collect()
}

fn bundledJapaneseNames() -> Result<MasterNames> {
    serde_json::from_slice(BUNDLED_JAPANESE_NAMES).context("failed to parse bundled jp_names.json")
}

pub fn bundledNames() -> Result<NameLoc> {
    let translations: NameTranslations = serde_json::from_slice(BUNDLED_TRANSLATIONS)
        .context("failed to parse bundled name_translations.json")?;
    let overrides: NameOverrides = serde_json::from_slice(BUNDLED_OVERRIDES)
        .context("failed to parse bundled name_overrides.json")?;
    Ok(NameLoc {
        translations: translations
            .names
            .into_iter()
            .map(|(kind, names)| {
                (
                    kind,
                    names.into_iter().map(|(id, name)| (id, name.en)).collect(),
                )
            })
            .collect(),
        overrides,
    })
}

impl NameLoc {
    /// Machine translations of one kind, for entries no other source names.
    pub fn translations(&self, kind: NameKind) -> impl Iterator<Item = (i64, &str)> {
        self.translations
            .get(&kind)
            .into_iter()
            .flatten()
            .map(|(id, name)| (*id, name.as_str()))
    }

    /// Hand-written names of one kind, which replace every source but Global's master.
    pub fn overrides(&self, kind: NameKind) -> impl Iterator<Item = (i64, &str)> {
        self.overrides
            .get(&kind)
            .into_iter()
            .flatten()
            .map(|(id, name)| (*id, name.as_str()))
    }
}

/// Writes JP's gachas and names from its master into `out_dir`, then translates the names only
/// JP has released that `out_dir` has no current translation or override for.
pub async fn syncJpData(
    global_master: &Path,
    jp_master: &Path,
    out_dir: &Path,
    translate: bool,
) -> Result<()> {
    let jp = openMaster(jp_master)?;
    let gachas = master::readGachaData(&jp)?;
    writeJson(&out_dir.join(GACHAS_FILE), &gachas)?;
    info!(
        gachas = gachas.gachas.len(),
        cards = gachas.card_characters.len(),
        "wrote JP gachas"
    );
    let japanese = master::readAllNames(&jp)?;
    writeJson(&out_dir.join(JAPANESE_NAMES_FILE), &japanese)?;
    if !translate {
        info!("translation skipped");
        return Ok(());
    }

    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
        .context("ANTHROPIC_API_KEY is not set; set it, or pass --skip-translation")?;
    let global = openMaster(global_master)?;
    let previous: NameTranslations = readJson(&out_dir.join(TRANSLATIONS_FILE))?;
    let overrides: NameOverrides = readJson(&out_dir.join(OVERRIDES_FILE))?;
    let official = master::readAllNames(&global)?;
    let translations = updateTranslations(
        &official,
        &japanese,
        &gachas.card_characters,
        previous,
        &overrides,
        api_key,
    )
    .await?;
    writeJson(&out_dir.join(TRANSLATIONS_FILE), &translations)
}

/// `official` holds Global's names and `japanese` JP's.
async fn updateTranslations(
    official: &MasterNames,
    japanese: &MasterNames,
    card_characters: &BTreeMap<i64, i64>,
    previous: NameTranslations,
    overrides: &NameOverrides,
    api_key: String,
) -> Result<NameTranslations> {
    let examples = examples(official, japanese, card_characters);

    let mut translations = NameTranslations {
        model: TRANSLATION_MODEL.to_string(),
        names: BTreeMap::new(),
    };
    let mut pending = BTreeMap::<NameKind, BTreeMap<&str, Vec<i64>>>::new();
    for kind in KINDS {
        let kept = translations.names.entry(kind).or_default();
        for (id, ja) in &japanese[&kind] {
            let overridden = overrides
                .get(&kind)
                .is_some_and(|names| names.contains_key(id));
            if official[&kind].contains_key(id) || overridden {
                continue;
            }
            match previous.names.get(&kind).and_then(|names| names.get(id)) {
                Some(name) if name.ja == *ja => {
                    kept.insert(*id, name.clone());
                }
                _ => pending
                    .entry(kind)
                    .or_default()
                    .entry(ja.as_str())
                    .or_default()
                    .push(*id),
            }
        }
    }
    let total: usize = pending.values().map(BTreeMap::len).sum();
    info!(
        names = total,
        examples = examples.len(),
        "names to translate"
    );
    if total == 0 {
        return Ok(translations);
    }

    let translator = Translator::new(api_key, &examples)?;
    for kind in KINDS {
        let Some(names) = pending.remove(&kind) else {
            continue;
        };
        let items: Vec<(TranslationItem, Vec<i64>)> = names
            .into_iter()
            .enumerate()
            .map(|(index, (ja, ids))| {
                let character = characterOf(
                    kind,
                    &ids,
                    card_characters,
                    official,
                    &translations,
                    overrides,
                );
                let item = TranslationItem {
                    id: index.to_string(),
                    kind,
                    ja: ja.to_string(),
                    character,
                };
                (item, ids)
            })
            .collect();

        for batch in items.chunks(BATCH_SIZE) {
            let asked: Vec<TranslationItem> = batch.iter().map(|(item, _)| item.clone()).collect();
            let answers = translator.translate(&asked).await?;
            for (item, ids) in batch {
                let Some(en) = answers.get(&item.id) else {
                    warn!(
                        ?kind,
                        ja = item.ja,
                        "no translation returned; it is retried on the next sync"
                    );
                    continue;
                };
                let names = translations.names.entry(kind).or_default();
                for id in ids {
                    names.insert(
                        *id,
                        MachineName {
                            ja: item.ja.clone(),
                            en: en.clone(),
                        },
                    );
                }
            }
            info!(?kind, batch = batch.len(), "translated names");
        }
    }
    Ok(translations)
}

/// Global's shipped names that JP has too, once per Japanese text, with the title's character.
fn examples(
    official: &BTreeMap<NameKind, BTreeMap<i64, String>>,
    japanese: &BTreeMap<NameKind, BTreeMap<i64, String>>,
    card_characters: &BTreeMap<i64, i64>,
) -> Vec<TranslationExample> {
    let mut seen = BTreeSet::new();
    let mut examples = Vec::new();
    for kind in KINDS {
        for (id, en) in &official[&kind] {
            let Some(ja) = japanese[&kind].get(id) else {
                continue;
            };
            if !seen.insert((kind, ja.clone())) {
                continue;
            }
            let character = (kind != NameKind::UmaName)
                .then(|| card_characters.get(id))
                .flatten()
                .and_then(|chara| official[&NameKind::UmaName].get(chara))
                .cloned();
            examples.push(TranslationExample {
                kind,
                ja: ja.clone(),
                en: en.clone(),
                character,
            });
        }
    }
    examples
}

/// The English name of the character the titles with this text belong to, when they all share
/// one and it has an English name.
fn characterOf(
    kind: NameKind,
    ids: &[i64],
    card_characters: &BTreeMap<i64, i64>,
    official: &BTreeMap<NameKind, BTreeMap<i64, String>>,
    translations: &NameTranslations,
    overrides: &NameOverrides,
) -> Option<String> {
    if kind == NameKind::UmaName {
        return None;
    }
    let characters: BTreeSet<i64> = ids
        .iter()
        .filter_map(|id| card_characters.get(id).copied())
        .collect();
    let [chara] = characters.into_iter().collect::<Vec<_>>()[..] else {
        return None;
    };
    official[&NameKind::UmaName]
        .get(&chara)
        .or_else(|| {
            overrides
                .get(&NameKind::UmaName)
                .and_then(|names| names.get(&chara))
        })
        .or_else(|| {
            translations
                .names
                .get(&NameKind::UmaName)
                .and_then(|names| names.get(&chara))
                .map(|name| &name.en)
        })
        .cloned()
}

fn openMaster(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open master at {}", path.display()))
}

/// A missing file reads as empty, so the first sync starts from nothing.
fn readJson<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn writeJson<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    crate::pipeline::writeAtomic(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    fn gacha(gacha_id: i64, pickup_card_ids: Vec<i64>) -> JpGacha {
        let date = DateTime::from_timestamp(0, 0).unwrap();
        JpGacha {
            gacha_id,
            gacha_type: 3,
            card_type: 1,
            start_date: date,
            end_date: date,
            pickup_card_ids,
        }
    }

    #[test]
    fn newerGachasTakesPickupGachasPastEachRangesNewestKnownId() {
        let gachas = vec![
            gacha(30010, vec![100101]),
            gacha(30011, vec![100201]),
            gacha(30012, vec![]),
            gacha(50005, vec![30001]),
            gacha(50006, vec![30002]),
        ];
        let known = BTreeSet::from([30009, 30010, 50005]);
        let ids: Vec<i64> = newerGachas(gachas, &known)
            .iter()
            .map(|gacha| gacha.gacha_id)
            .collect();
        assert_eq!(ids, [30011, 50006]);
    }

    #[test]
    fn newerGachasSkipsRangesWithoutKnownIds() {
        assert!(newerGachas(vec![gacha(50001, vec![30001])], &BTreeSet::from([30001])).is_empty());
    }

    #[test]
    fn bundledGachasParse() {
        let data = bundledGachaData().unwrap();
        assert!(data.gachas.iter().all(|gacha| gacha
            .pickup_card_ids
            .iter()
            .all(|id| data.card_characters.contains_key(id))));
        bundledNames().unwrap();
    }
}

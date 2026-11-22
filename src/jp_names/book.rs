//! The English and Japanese names of characters and cards that every resource reads.

use super::types::{CardNames, MasterNames, NameBook, NameKind};
use super::{bundledGachaData, bundledJapaneseNames, bundledNames, master, KINDS};
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::BTreeMap;

impl NameBook {
    /// English names: Global's master wins, then the hand-written overrides, then the bundled
    /// character names, then the translations. Japanese names come from `jp` when the set is
    /// built with JP's master, and otherwise from the copy the sync job bundles.
    pub fn load(global: &Connection, jp: Option<&Connection>) -> Result<Self> {
        let loc = bundledNames()?;
        let mut card_characters = bundledGachaData()?.card_characters;
        let japanese = match jp {
            Some(jp) => {
                card_characters.extend(master::readCardCharacters(jp)?);
                master::readAllNames(jp)?
            }
            None => bundledJapaneseNames()?,
        };
        card_characters.extend(master::readCardCharacters(global)?);

        let mut english = MasterNames::new();
        for kind in KINDS {
            let names = english.entry(kind).or_default();
            if kind == NameKind::UmaName {
                names.extend(bundledCharacterNames()?);
            }
            for (id, name) in loc.translations(kind) {
                names.entry(id).or_insert_with(|| name.to_string());
            }
            for (id, name) in loc.overrides(kind) {
                names.insert(id, name.to_string());
            }
            names.extend(master::readNames(global, kind)?);
        }

        Ok(Self {
            english,
            japanese,
            card_characters,
        })
    }

    /// Every English name of one kind, by id.
    pub fn english(&self, kind: NameKind) -> &BTreeMap<i64, String> {
        static EMPTY: BTreeMap<i64, String> = BTreeMap::new();
        self.english.get(&kind).unwrap_or(&EMPTY)
    }

    /// A character's English and Japanese name.
    pub fn character(&self, chara_id: i64) -> (Option<String>, Option<String>) {
        (
            self.name(&self.english, NameKind::UmaName, chara_id),
            self.name(&self.japanese, NameKind::UmaName, chara_id),
        )
    }

    /// A character card's or support card's character name and title in both languages.
    pub fn card(&self, card_id: i64) -> CardNames {
        let (name_en, name_jp) = self
            .charaId(card_id)
            .map(|chara_id| self.character(chara_id))
            .unwrap_or_default();
        let kind = titleKind(card_id);
        let title = |names: &MasterNames| {
            self.name(names, kind, card_id)
                .map(|title| stripBrackets(&title))
        };
        CardNames {
            name_en,
            name_jp,
            title_en: title(&self.english),
            title_jp: title(&self.japanese),
        }
    }

    fn charaId(&self, card_id: i64) -> Option<i64> {
        self.card_characters
            .get(&card_id)
            .copied()
            .or_else(|| (titleKind(card_id) == NameKind::OutfitTitle).then_some(card_id / 100))
    }

    fn name(&self, names: &MasterNames, kind: NameKind, id: i64) -> Option<String> {
        names.get(&kind)?.get(&id).cloned()
    }
}

/// Character cards have six-digit ids such as 100101, support cards five-digit ones.
fn titleKind(card_id: i64) -> NameKind {
    if card_id >= 100_000 {
        NameKind::OutfitTitle
    } else {
        NameKind::SupportTitle
    }
}

fn stripBrackets(title: &str) -> String {
    title.trim().trim_matches(['[', ']']).trim().to_string()
}

fn bundledCharacterNames() -> Result<BTreeMap<i64, String>> {
    let names = crate::generators::common::read_character_names()?;
    let names = names
        .as_object()
        .context("character_names.json must be a JSON object")?;
    Ok(names
        .iter()
        .filter_map(|(id, entry)| {
            let name = entry.get("name").and_then(Value::as_str)?;
            Some((id.parse().ok()?, name.to_string()))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> NameBook {
        let names = |kind: NameKind, entries: &[(i64, &str)]| {
            (
                kind,
                entries
                    .iter()
                    .map(|(id, name)| (*id, name.to_string()))
                    .collect(),
            )
        };
        NameBook {
            english: MasterNames::from([
                names(NameKind::UmaName, &[(1001, "Special Week")]),
                names(NameKind::OutfitTitle, &[(100101, "[Special Dreamer]")]),
            ]),
            japanese: MasterNames::from([
                names(NameKind::UmaName, &[(1001, "スペシャルウィーク")]),
                names(NameKind::SupportTitle, &[(30001, "[日本一のウマ娘]")]),
            ]),
            card_characters: BTreeMap::from([(30001, 1001)]),
        }
    }

    #[test]
    fn cardsNameTheirCharacterAndTitleInBothLanguages() {
        let book = book();
        assert_eq!(
            book.card(100101),
            CardNames {
                name_en: Some("Special Week".into()),
                name_jp: Some("スペシャルウィーク".into()),
                title_en: Some("Special Dreamer".into()),
                title_jp: None,
            }
        );
        let support = book.card(30001);
        assert_eq!(support.name_en.as_deref(), Some("Special Week"));
        assert_eq!(support.title_jp.as_deref(), Some("日本一のウマ娘"));
        assert_eq!(support.title_en, None);
        assert_eq!(book.card(20999), CardNames::default());
    }
}

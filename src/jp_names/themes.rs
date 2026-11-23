//! Short outfit labels, such as Halloween, for the JP outfits the store's hand-kept character list
//! has no label for. uma.moe shows a card as "<character> [<label>]". JP's master has no themes,
//! so Claude writes them from the outfit's title, its character and its debut date.

use super::book::stripBrackets;
use super::englishName;
use super::translate::{Translator, BATCH_SIZE, THEME_INSTRUCTIONS};
use super::types::{
    BundledCharacter, JpGacha, MachineName, MasterNames, NameKind, NameOverrides, NameTranslations,
    ThemeExample, ThemeItem, ThemeOutfit, ALT_VERSION_SKIN, ORIGINAL_SKIN,
};
use anyhow::Result;
use chrono::NaiveDate;
use chrono_tz::Asia::Tokyo;
use std::collections::BTreeMap;
use tracing::{info, warn};

/// The hand-kept list keys a character's outfit labels by the last two digits of the card id.
pub fn skinKey(card_id: i64) -> String {
    format!("{:02}", card_id % 100)
}

/// Whether the hand-kept list leaves an outfit to its theme: it has no label for it, or only the
/// "Alt Version" placeholder.
pub fn awaitsTheme(label: Option<&str>) -> bool {
    label.is_none_or(|label| label == ALT_VERSION_SKIN)
}

/// The day each card first featured among a JP gacha's pickups, in Japan.
pub(super) fn debutDates(gachas: &[JpGacha]) -> BTreeMap<i64, NaiveDate> {
    let mut debuts = BTreeMap::<i64, NaiveDate>::new();
    for gacha in gachas {
        let day = gacha.start_date.with_timezone(&Tokyo).date_naive();
        for card_id in &gacha.pickup_card_ids {
            debuts
                .entry(*card_id)
                .and_modify(|debut| *debut = (*debut).min(day))
                .or_insert(day);
        }
    }
    debuts
}

/// The JP outfits that need a label: character cards past the first outfit, whose character the
/// hand-kept list has, without a hand-kept label of their own and without an override. Returns
/// the previous labels whose outfit title is unchanged, and the cards to ask the model about.
pub(super) fn pendingThemes(
    titles: &BTreeMap<i64, String>,
    card_characters: &BTreeMap<i64, i64>,
    bundled: &BTreeMap<i64, BundledCharacter>,
    overrides: &NameOverrides,
    previous: &NameTranslations,
) -> (BTreeMap<i64, MachineName>, Vec<i64>) {
    let overridden = overrides.get(&NameKind::OutfitTheme);
    let cached = previous.names.get(&NameKind::OutfitTheme);
    let mut kept = BTreeMap::new();
    let mut pending = Vec::new();
    for (card_id, title) in titles {
        // Support cards have five-digit ids, and every character's first outfit is "Original".
        if *card_id < 100_000 || card_id % 100 == 1 {
            continue;
        }
        let Some(character) = card_characters
            .get(card_id)
            .and_then(|chara_id| bundled.get(chara_id))
        else {
            continue;
        };
        if !awaitsTheme(character.skins.get(&skinKey(*card_id)).map(String::as_str)) {
            continue;
        }
        if overridden.is_some_and(|labels| labels.contains_key(card_id)) {
            continue;
        }
        match cached.and_then(|labels| labels.get(card_id)) {
            Some(label) if label.ja == *title => {
                kept.insert(*card_id, label.clone());
            }
            _ => pending.push(*card_id),
        }
    }
    (kept, pending)
}

/// Writes a label into `translations` for each JP outfit the hand-kept list leaves unlabelled:
/// the previous one while the outfit title is unchanged, else the model's. The hand-kept labels
/// are the examples. An outfit the model leaves out, or whose batch fails, is asked about again
/// on the next run, so a failure here keeps the names translated before it.
#[allow(clippy::too_many_arguments)]
pub(super) async fn labelOutfits(
    translations: &mut NameTranslations,
    previous: &NameTranslations,
    official: &MasterNames,
    japanese: &MasterNames,
    card_characters: &BTreeMap<i64, i64>,
    overrides: &NameOverrides,
    bundled: &BTreeMap<i64, BundledCharacter>,
    debuts: &BTreeMap<i64, NaiveDate>,
    api_key: &str,
) -> Result<()> {
    let titles = &japanese[&NameKind::OutfitTitle];
    let (kept, pending) = pendingThemes(titles, card_characters, bundled, overrides, previous);

    let describe = |card_id: i64, chara_id: i64, character: &BundledCharacter, title: &str| {
        let character = official[&NameKind::UmaName]
            .get(&chara_id)
            .unwrap_or(&character.name);
        ThemeOutfit {
            character: character.clone(),
            title_ja: stripBrackets(title),
            title_en: englishName(
                NameKind::OutfitTitle,
                card_id,
                official,
                overrides,
                translations,
            )
            .map(|title| stripBrackets(title)),
            debut: debuts
                .get(&card_id)
                .map(|day| day.format("%Y-%m-%d").to_string()),
        }
    };

    let mut examples = Vec::new();
    for (chara_id, character) in bundled {
        for (key, label) in &character.skins {
            if label == ORIGINAL_SKIN || label == ALT_VERSION_SKIN {
                continue;
            }
            let Ok(suffix) = key.parse::<i64>() else {
                continue;
            };
            let card_id = chara_id * 100 + suffix;
            let Some(title) = titles.get(&card_id) else {
                continue;
            };
            examples.push(ThemeExample {
                outfit: describe(card_id, *chara_id, character, title),
                label: label.clone(),
            });
        }
    }

    let items: Vec<(ThemeItem, i64, &String)> = pending
        .into_iter()
        .filter_map(|card_id| {
            let chara_id = *card_characters.get(&card_id)?;
            let title = titles.get(&card_id)?;
            let item = ThemeItem {
                id: card_id.to_string(),
                outfit: describe(card_id, chara_id, bundled.get(&chara_id)?, title),
            };
            Some((item, card_id, title))
        })
        .collect();

    let labels = translations.names.entry(NameKind::OutfitTheme).or_default();
    labels.extend(kept);
    info!(
        outfits = items.len(),
        examples = examples.len(),
        "outfits to label"
    );
    if items.is_empty() {
        return Ok(());
    }

    let translator = Translator::new(api_key.to_string(), THEME_INSTRUCTIONS, &examples)?;
    for batch in items.chunks(BATCH_SIZE) {
        let asked: Vec<&ThemeItem> = batch.iter().map(|(item, _, _)| item).collect();
        let answers = match translator.translate(&asked).await {
            Ok(answers) => answers,
            Err(error) => {
                warn!(
                    error = format!("{error:#}"),
                    "outfit labels failed; they are asked for again on the next sync"
                );
                continue;
            }
        };
        for (item, card_id, title) in batch {
            let Some(label) = answers.get(&item.id) else {
                warn!(
                    card = card_id,
                    "no outfit label returned; it is asked for again on the next sync"
                );
                continue;
            };
            labels.insert(
                *card_id,
                MachineName {
                    ja: (*title).clone(),
                    en: label.clone(),
                },
            );
        }
        info!(batch = batch.len(), "labelled outfits");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    fn character(skins: &[(&str, &str)]) -> BundledCharacter {
        BundledCharacter {
            name: "Special Week".into(),
            skins: skins
                .iter()
                .map(|(key, label)| (key.to_string(), label.to_string()))
                .collect(),
        }
    }

    fn label(ja: &str, en: &str) -> MachineName {
        MachineName {
            ja: ja.into(),
            en: en.into(),
        }
    }

    #[test]
    fn pendingThemesAskForMissingAndAltVersionLabelsOnly() {
        let titles: BTreeMap<i64, String> = [
            (100101, "[初期]"),
            (100102, "[夏]"),
            (100103, "[新衣装]"),
            (100104, "[別衣装]"),
            (100105, "[上書き]"),
            (100106, "[同じ題]"),
            (100107, "[新しい題]"),
            (100202, "[一覧にない]"),
        ]
        .into_iter()
        .map(|(id, title)| (id, title.to_string()))
        .collect();
        let card_characters = titles.keys().map(|id| (*id, id / 100)).collect();
        let bundled = BTreeMap::from([(
            1001,
            character(&[("01", "Original"), ("02", "Summer"), ("04", "Alt Version")]),
        )]);
        let overrides = NameOverrides::from([(
            NameKind::OutfitTheme,
            BTreeMap::from([(100105, "Overridden".to_string())]),
        )]);
        let previous = NameTranslations {
            model: String::new(),
            names: BTreeMap::from([(
                NameKind::OutfitTheme,
                BTreeMap::from([
                    (100102, label("[夏]", "Stale")),
                    (100106, label("[同じ題]", "Cached")),
                    (100107, label("[古い題]", "Renamed")),
                ]),
            )]),
        };

        let (kept, pending) =
            pendingThemes(&titles, &card_characters, &bundled, &overrides, &previous);
        assert_eq!(
            kept,
            BTreeMap::from([(100106, label("[同じ題]", "Cached"))])
        );
        assert_eq!(pending, [100103, 100104, 100107]);
    }

    #[test]
    fn debutIsTheFirstPickupGachaDayInJapan() {
        let gacha = |gacha_id: i64, start: i64, pickup_card_ids: Vec<i64>| JpGacha {
            gacha_id,
            gacha_type: 3,
            card_type: 1,
            start_date: DateTime::from_timestamp(start, 0).unwrap(),
            end_date: DateTime::from_timestamp(start, 0).unwrap(),
            pickup_card_ids,
            recommended_card_ids: vec![100103],
            select_card_ids: Vec::new(),
        };
        // 2026-10-13 03:00 UTC is noon in Japan; 2026-10-19 16:00 UTC is already the 20th there.
        let debuts = debutDates(&[
            gacha(30478, 1_792_465_200, vec![100102]),
            gacha(30476, 1_791_860_400, vec![100102]),
            gacha(30480, 1_792_425_600, vec![100104]),
        ]);
        let day = |text: &str| NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap();
        assert_eq!(
            debuts,
            BTreeMap::from([(100102, day("2026-10-13")), (100104, day("2026-10-20"))])
        );
    }
}

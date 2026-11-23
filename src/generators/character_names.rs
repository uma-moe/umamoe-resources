use crate::generators::common;
use crate::jp_names::{awaitsTheme, skinKey, NameBook, NameKind};
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub fn generate(connection: &Connection, names: &NameBook) -> Result<Value> {
    let mut characters = common::read_character_names()?;
    let characters_object = characters
        .as_object_mut()
        .context("character_names.json must be a JSON object")?;

    for (chara_id, global_name) in common::load_global_character_names(connection)? {
        if let Some(entry) = characters_object
            .get_mut(&chara_id.to_string())
            .and_then(Value::as_object_mut)
        {
            entry.insert("name".to_string(), Value::String(global_name));
        }
    }
    for (chara_id, entry) in characters_object.iter_mut() {
        if let (Ok(chara_id), Some(entry)) = (chara_id.parse(), entry.as_object_mut()) {
            insert_language_names(entry, names, chara_id);
        }
    }
    fillOutfitThemes(
        characters_object,
        names.cardCharacters(),
        names.english(NameKind::OutfitTheme),
    );

    Ok(characters)
}

/// Every character the JP master names, by id. The bundled list and its English skin names
/// belong to the Global set.
pub fn generate_jp(connection: &Connection, names: &NameBook) -> Result<Value> {
    let characters = common::load_global_character_names(connection)?
        .into_iter()
        .map(|(chara_id, name)| {
            let mut entry = Map::from_iter([("name".to_string(), Value::String(name))]);
            insert_language_names(&mut entry, names, chara_id);
            (chara_id.to_string(), Value::Object(entry))
        })
        .collect::<Map<_, _>>();
    Ok(Value::Object(characters))
}

/// `name` stays in the set's own language; these name the character in both.
fn insert_language_names(entry: &mut Map<String, Value>, names: &NameBook, chara_id: i64) {
    let (name_en, name_jp) = names.character(chara_id);
    entry.insert("name_en".to_string(), name_en.into());
    entry.insert("name_jp".to_string(), name_jp.into());
}

/// Gives each listed character's outfits that the bundled list has no skin name for, or only
/// "Alt Version", their outfit theme. The bundled skin names always win.
#[allow(non_snake_case)]
fn fillOutfitThemes(
    characters: &mut Map<String, Value>,
    card_characters: &BTreeMap<i64, i64>,
    themes: &BTreeMap<i64, String>,
) {
    for (card_id, chara_id) in card_characters {
        // Support cards have five-digit ids.
        if *card_id < 100_000 {
            continue;
        }
        let Some(theme) = themes.get(card_id) else {
            continue;
        };
        let Some(entry) = characters
            .get_mut(&chara_id.to_string())
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        let Some(skins) = entry
            .entry("skins")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
        else {
            continue;
        };
        let key = skinKey(*card_id);
        if awaitsTheme(skins.get(&key).and_then(Value::as_str)) {
            skins.insert(key, Value::String(theme.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    #[allow(non_snake_case)]
    fn outfitThemesFillOnlyMissingAndAltVersionSkins() {
        let mut characters = json!({
            "1045": {"name": "Super Creek", "skins": {"01": "Original", "02": "Halloween"}},
            "1093": {"name": "K.S.Miracle", "skins": {"01": "Original", "02": "Alt Version"}},
            "1108": {"name": "Duramente", "skins": {"01": "Original"}},
        });
        let card_characters = BTreeMap::from([
            (104502, 1045),
            (109302, 1093),
            (110802, 1108),
            (110803, 1108),
            (120102, 1201),
            (30108, 1108),
        ]);
        let themes = BTreeMap::from([
            (104502, "Spooky".to_string()),
            (109302, "Autumn Andante".to_string()),
            (110802, "Halloween".to_string()),
            (120102, "Unlisted".to_string()),
            (30108, "Support".to_string()),
        ]);

        fillOutfitThemes(
            characters.as_object_mut().unwrap(),
            &card_characters,
            &themes,
        );
        assert_eq!(
            characters,
            json!({
                "1045": {"name": "Super Creek", "skins": {"01": "Original", "02": "Halloween"}},
                "1093": {"name": "K.S.Miracle", "skins": {"01": "Original", "02": "Autumn Andante"}},
                "1108": {"name": "Duramente", "skins": {"01": "Original", "02": "Halloween"}},
            })
        );
    }
}

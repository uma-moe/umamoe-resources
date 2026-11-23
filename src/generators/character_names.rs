use crate::generators::common;
use crate::jp_names::NameBook;
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::{Map, Value};

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

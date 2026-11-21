use crate::generators::common;
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::Value;

pub fn generate(connection: &Connection) -> Result<Value> {
    let mut names = common::read_character_names()?;
    let names_object = names
        .as_object_mut()
        .context("character_names.json must be a JSON object")?;

    for (chara_id, global_name) in common::load_global_character_names(connection)? {
        if let Some(entry) = names_object
            .get_mut(&chara_id.to_string())
            .and_then(Value::as_object_mut)
        {
            entry.insert("name".to_string(), Value::String(global_name));
        }
    }

    Ok(names)
}

/// Every character the JP master names, by id. The bundled list and its English skin names
/// belong to the Global set.
pub fn generate_jp(connection: &Connection) -> Result<Value> {
    let names = common::load_global_character_names(connection)?
        .into_iter()
        .map(|(chara_id, name)| (chara_id.to_string(), serde_json::json!({ "name": name })))
        .collect::<serde_json::Map<_, _>>();
    Ok(Value::Object(names))
}

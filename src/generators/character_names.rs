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

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

pub mod affinity;
pub mod banners;
pub mod character;
pub mod character_names;
pub mod common;
pub mod factors;
pub mod race_program;
pub mod skills;
pub mod support_cards_db;
pub mod supports;

pub struct ResourceOutput {
    pub file_name: &'static str,
    pub value: serde_json::Value,
}

pub fn generate_all(connection: &Connection) -> Result<Vec<ResourceOutput>> {
    let mut outputs = vec![
        output("factors.json", factors::generate(connection)?)?,
        output("race_program.json", race_program::generate(connection)?)?,
        output(
            "character_banners.json",
            banners::generate_character_banners(connection)?,
        )?,
        output(
            "supports_banners.json",
            banners::generate_support_banners(connection)?,
        )?,
        output(
            "paid_gacha_banners.json",
            banners::generate_paid_banners(connection)?,
        )?,
        output("affinity.json", affinity::generate(connection)?)?,
    ];

    outputs.push(output(
        "character_names.json",
        character_names::generate(connection)?,
    )?);
    outputs.push(output("character.json", character::generate(connection)?)?);
    outputs.push(output("supports.json", supports::generate(connection)?)?);
    outputs.push(output(
        "support-cards-db.json",
        support_cards_db::generate(connection)?,
    )?);
    outputs.push(output("skills.json", skills::generate(connection)?)?);

    Ok(outputs)
}

fn output<T>(file_name: &'static str, value: T) -> Result<ResourceOutput>
where
    T: Serialize,
{
    Ok(ResourceOutput {
        file_name,
        value: serde_json::to_value(value)?,
    })
}

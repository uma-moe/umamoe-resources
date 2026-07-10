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
pub mod room_match_races;
pub mod skills;
pub mod support_cards_db;
pub mod supports;
pub mod timeline;

pub struct ResourceOutput {
    pub file_name: &'static str,
    pub value: serde_json::Value,
}

pub fn generate_all(connection: &Connection) -> Result<Vec<ResourceOutput>> {
    let character_banners = banners::generate_character_banners(connection)?;
    let support_banners = banners::generate_support_banners(connection)?;
    let paid_banners = banners::generate_paid_banners(connection)?;

    let mut outputs = vec![
        output("factors.json", factors::generate(connection)?)?,
        output("race_program.json", race_program::generate(connection)?)?,
        output(
            "room_match_races.json",
            room_match_races::generate(connection)?,
        )?,
        output("character_banners.json", &character_banners)?,
        output("supports_banners.json", &support_banners)?,
        output("paid_gacha_banners.json", &paid_banners)?,
        output(
            "banner_timeline.json",
            timeline::generate(
                connection,
                &character_banners,
                &support_banners,
                &paid_banners,
            )?,
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

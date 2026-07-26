use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

pub mod affinity;
pub mod banners;
pub mod character;
pub mod character_names;
pub mod common;
pub mod factors;
pub mod jp_events;
pub mod planner;
pub mod race_program;
pub mod race_to_saddle;
pub mod room_match_races;
pub mod simulator_course_geometry;
pub mod simulator_courses;
pub mod simulator_skills;
pub mod skills;
pub mod support_cards_db;
pub mod supports;
pub mod timeline;

#[derive(Debug)]
pub struct ResourceOutput {
    pub file_name: String,
    pub value: serde_json::Value,
}

pub struct GeneratedResources {
    pub public: Vec<ResourceOutput>,
    pub planner: Vec<ResourceOutput>,
}

pub fn generate_all(
    connection: &Connection,
    jp_connection: Option<&Connection>,
    master_version: &str,
) -> Result<GeneratedResources> {
    let character_banners = banners::generate_character_banners(connection)?;
    let support_banners = banners::generate_support_banners(connection)?;
    let paid_banners = banners::generate_paid_banners(connection)?;
    let mut timeline = serde_json::to_value(timeline::generate(
        connection,
        &character_banners,
        &support_banners,
        &paid_banners,
    )?)?;
    let planner = planner::generate(
        connection,
        jp_connection,
        &character_banners,
        &support_banners,
        &paid_banners,
        &timeline,
    )?;
    planner.annotate_timeline(&mut timeline);
    let simulator_courses = simulator_courses::generate(connection, master_version)?;

    let mut public = vec![
        output("factors.json", factors::generate(connection)?)?,
        output("race_program.json", race_program::generate(connection)?)?,
        output(
            "race_to_saddle_mapping.json",
            race_to_saddle::generate(connection)?,
        )?,
        output(
            "room_match_races.json",
            room_match_races::generate(connection)?,
        )?,
        output(
            "simulator_skills.json",
            simulator_skills::generate(connection, master_version)?,
        )?,
        output("simulator_courses.json", &simulator_courses)?,
        output("character_banners.json", &character_banners)?,
        output("supports_banners.json", &support_banners)?,
        output("paid_gacha_banners.json", &paid_banners)?,
        output("banner_timeline.json", timeline)?,
        output("affinity.json", affinity::generate(connection)?)?,
    ];

    public.extend(simulator_course_geometry::generate(&simulator_courses)?);

    public.push(output(
        "character_names.json",
        character_names::generate(connection)?,
    )?);
    public.push(output("character.json", character::generate(connection)?)?);
    public.push(output("supports.json", supports::generate(connection)?)?);
    public.push(output(
        "support-cards-db.json",
        support_cards_db::generate(connection)?,
    )?);
    public.push(output("skills.json", skills::generate(connection)?)?);

    let mut protected = vec![
        output("planner_core.json", planner.core)?,
        output("planner_income.json", planner.income)?,
        output("planner_rewards.json", planner.rewards)?,
    ];
    for shard in planner.gacha_shards {
        protected.push(output(
            format!("planner_gacha_{}.json", shard.shard),
            shard,
        )?);
    }

    Ok(GeneratedResources {
        public,
        planner: protected,
    })
}

fn output<T>(file_name: impl Into<String>, value: T) -> Result<ResourceOutput>
where
    T: Serialize,
{
    Ok(ResourceOutput {
        file_name: file_name.into(),
        value: serde_json::to_value(value)?,
    })
}

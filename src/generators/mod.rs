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
pub mod timeline_images;

#[derive(Debug)]
pub struct ResourceOutput {
    pub file_name: String,
    pub value: serde_json::Value,
}

pub struct GeneratedResources {
    pub public: Vec<ResourceOutput>,
    pub planner: Vec<ResourceOutput>,
}

/// `jp_connection` is the JP master the planner reads its rewards from, given only when the
/// bundled reward catalogue is refreshed. `jp_names` is the JP master the server keeps, for
/// Japanese names, JP's newest banners and tying the Global master's schedules to JP events.
pub fn generate_all(
    connection: &Connection,
    jp_connection: Option<&Connection>,
    jp_names: Option<&Connection>,
    master_version: &str,
) -> Result<GeneratedResources> {
    let character_banners = banners::generate_character_banners(connection)?;
    let support_banners = banners::generate_support_banners(connection)?;
    let paid_banners = banners::generate_paid_banners(connection)?;
    // English and Japanese names for every resource that names characters and cards.
    let names = crate::jp_names::NameBook::load(connection, jp_names)?;
    let master_gachas = match jp_names {
        Some(jp) => crate::jp_names::masterGachas(jp)?,
        None => Vec::new(),
    };
    let mut timeline = serde_json::to_value(timeline::generate(
        connection,
        jp_connection,
        jp_names,
        &names,
        &master_gachas,
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

    public.push(simulator_course_geometry::generate(&simulator_courses)?);
    public.push(simulator_course_geometry::generateLanes(
        &simulator_courses,
    )?);

    public.push(output(
        "character_names.json",
        character_names::generate(connection, &names)?,
    )?);
    public.push(output(
        "character.json",
        character::generate(connection, &names)?,
    )?);
    public.push(output(
        "supports.json",
        supports::generate(connection, &names)?,
    )?);
    public.push(output(
        "support-cards-db.json",
        support_cards_db::generate(connection, &names)?,
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

/// The JP set: what the JP master alone describes, under the names the Global set uses, so a
/// page can read either set. Text is Japanese, apart from the `_en` names. The timeline, planner and simulator resources are
/// built around Global's release schedule and bundled Global data, and have no JP counterpart.
pub fn generate_jp(jp: &Connection, global: &Connection) -> Result<Vec<ResourceOutput>> {
    let characters = common::CardReleases::jp_master(global, common::CardTable::Characters)?;
    let supports = common::CardReleases::jp_master(global, common::CardTable::Supports)?;
    let names = crate::jp_names::NameBook::load(global, Some(jp))?;

    Ok(vec![
        output("factors.json", factors::generate(jp)?)?,
        output("race_program.json", race_program::generate(jp)?)?,
        output("race_to_saddle_mapping.json", race_to_saddle::generate(jp)?)?,
        output("room_match_races.json", room_match_races::generate(jp)?)?,
        output(
            "character_banners.json",
            banners::generate_character_banners(jp)?,
        )?,
        output(
            "supports_banners.json",
            banners::generate_support_banners(jp)?,
        )?,
        output(
            "paid_gacha_banners.json",
            banners::generate_paid_banners(jp)?,
        )?,
        output("affinity.json", affinity::generate(jp)?)?,
        output(
            "character_names.json",
            character_names::generate_jp(jp, &names)?,
        )?,
        output(
            "character.json",
            character::generate_for(jp, &characters, &names)?,
        )?,
        output(
            "supports.json",
            supports::generate_for(jp, &characters, &names)?,
        )?,
        output(
            "support-cards-db.json",
            support_cards_db::generate_jp(jp, &supports, &names)?,
        )?,
        output("skills.json", skills::generate(jp)?)?,
    ])
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

use anyhow::{ensure, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize)]
pub struct AffinityData {
    pub chars: Vec<i32>,
    pub aff2: Vec<u8>,
    pub aff3: Vec<u8>,
}

pub fn generate(connection: &Connection) -> Result<AffinityData> {
    let relation_points = load_relation_points(connection)?;
    let chara_relations = load_chara_relations(connection)?;
    let chars = chara_relations.keys().copied().collect::<Vec<_>>();
    let n = chars.len();

    let mut aff2 = vec![0u8; n * n];
    for (a_index, &a) in chars.iter().enumerate() {
        let relations_a = &chara_relations[&a];
        for (b_index, &b) in chars.iter().enumerate() {
            if a == b {
                continue;
            }

            let score = shared_relation_score(
                relations_a.intersection(&chara_relations[&b]).copied(),
                &relation_points,
            )?;
            aff2[a_index * n + b_index] = score;
        }
    }

    let mut aff3 = vec![0u8; n * n * n];
    for (a_index, &a) in chars.iter().enumerate() {
        let relations_a = &chara_relations[&a];
        for (b_index, &b) in chars.iter().enumerate() {
            if a == b {
                continue;
            }

            let ab_common = relations_a
                .intersection(&chara_relations[&b])
                .copied()
                .collect::<BTreeSet<_>>();
            if ab_common.is_empty() {
                continue;
            }

            for (c_index, &c) in chars.iter().enumerate() {
                if a == c || b == c {
                    continue;
                }

                let score = shared_relation_score(
                    ab_common.intersection(&chara_relations[&c]).copied(),
                    &relation_points,
                )?;
                aff3[a_index * n * n + b_index * n + c_index] = score;
            }
        }
    }

    Ok(AffinityData { chars, aff2, aff3 })
}

fn load_relation_points(connection: &Connection) -> Result<BTreeMap<i32, i32>> {
    let mut statement = connection.prepare(
        r#"
        SELECT relation_type, relation_point
        FROM succession_relation
        ORDER BY relation_type
        "#,
    )?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;

    let mut relation_points = BTreeMap::new();
    for row in rows {
        let (relation_type, relation_point) = row?;
        relation_points.insert(relation_type, relation_point);
    }

    Ok(relation_points)
}

fn load_chara_relations(connection: &Connection) -> Result<BTreeMap<i32, BTreeSet<i32>>> {
    let mut statement = connection.prepare(
        r#"
        SELECT chara_id, relation_type
        FROM succession_relation_member
        ORDER BY chara_id, relation_type
        "#,
    )?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;

    let mut chara_relations: BTreeMap<i32, BTreeSet<i32>> = BTreeMap::new();
    for row in rows {
        let (chara_id, relation_type) = row?;
        chara_relations
            .entry(chara_id)
            .or_default()
            .insert(relation_type);
    }

    Ok(chara_relations)
}

fn shared_relation_score(
    relation_types: impl Iterator<Item = i32>,
    relation_points: &BTreeMap<i32, i32>,
) -> Result<u8> {
    let score = relation_types
        .filter_map(|relation_type| relation_points.get(&relation_type))
        .sum::<i32>();
    ensure!(
        (0..=u8::MAX as i32).contains(&score),
        "affinity score {} does not fit in u8",
        score
    );
    Ok(score as u8)
}

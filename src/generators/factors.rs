use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct Factor {
    pub id: String,
    pub text: String,
    #[serde(rename = "type")]
    pub factor_type: i32,
}

pub fn generate(connection: &Connection) -> Result<Vec<Factor>> {
    let mut statement = connection.prepare(
        r#"
        SELECT "index", text
        FROM text_data
        WHERE category = 147
        ORDER BY "index"
        "#,
    )?;

    let rows = statement.query_map([], |row| {
        let factor_index: i64 = row.get(0)?;
        let factor_text: String = row.get(1)?;
        let base_factor = factor_base(factor_index);
        Ok((
            base_factor,
            Factor {
                id: base_factor.to_string(),
                text: factor_text,
                factor_type: factor_type(factor_index),
            },
        ))
    })?;

    let mut factors = BTreeMap::new();
    for row_result in rows {
        let (base_factor, factor) = row_result?;
        factors.entry(base_factor).or_insert(factor);
    }

    Ok(factors.into_values().collect())
}

fn factor_base(factor_index: i64) -> i64 {
    let factor_text = factor_index.to_string();
    if factor_text.len() > 1 {
        factor_text[..factor_text.len() - 1]
            .parse()
            .unwrap_or(factor_index)
    } else {
        factor_index
    }
}

fn factor_type(factor_index: i64) -> i32 {
    let factor_text = factor_index.to_string();
    match (factor_text.len(), factor_text.as_bytes().first().copied()) {
        (3, _) => 0,
        (4, _) => 1,
        (7, Some(b'1')) => 2,
        (7, Some(b'2')) => 3,
        (7, Some(b'3')) => 4,
        (8, _) => 5,
        _ => -1,
    }
}

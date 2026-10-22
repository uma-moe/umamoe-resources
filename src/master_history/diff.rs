use super::types::{DiffSummary, ModifiedRow, TableDiff, TableSchema};
use anyhow::{Context, Result};
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Compares every game table of two master databases, matching rows by primary key.
/// Only tables with changes are returned.
pub(super) fn diffMasters(
    previous: &Connection,
    current: &Connection,
) -> Result<BTreeMap<String, TableDiff>> {
    let previous_tables = readTableNames(previous)?;
    let current_tables = readTableNames(current)?;
    let mut tables = BTreeMap::new();
    for table in previous_tables.union(&current_tables) {
        let in_previous = previous_tables.contains(table);
        let in_current = current_tables.contains(table);

        // A dropped table keeps its last schema, so its rows can be listed as removed.
        let schema_source = if in_current { current } else { previous };
        let schema = readTableSchema(schema_source, table)?;
        let previous_rows = if in_previous {
            readRows(previous, table, &schema)?
        } else {
            Vec::new()
        };
        let current_rows = if in_current {
            readRows(current, table, &schema)?
        } else {
            Vec::new()
        };

        let diff = diffRows(&schema, previous_rows, current_rows);
        let changed =
            !diff.added.is_empty() || !diff.removed.is_empty() || !diff.modified.is_empty();
        if changed {
            tables.insert(table.clone(), diff);
        }
    }
    Ok(tables)
}

pub(super) fn summarize(tables: &BTreeMap<String, TableDiff>) -> DiffSummary {
    let mut summary = DiffSummary {
        tables_changed: tables.len(),
        ..DiffSummary::default()
    };
    for table in tables.values() {
        summary.added += table.added.len();
        summary.removed += table.removed.len();
        summary.modified += table.modified.len();
    }
    summary
}

pub(crate) fn sqliteValueToJson(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => Value::from(value),
        ValueRef::Real(value) => Value::from(value),
        ValueRef::Text(value) => Value::String(String::from_utf8_lossy(value).into()),
        ValueRef::Blob(value) => serde_json::json!({ "blob_hex": hex::encode(value) }),
    }
}

fn diffRows(
    schema: &TableSchema,
    previous_rows: Vec<Vec<Value>>,
    current_rows: Vec<Vec<Value>>,
) -> TableDiff {
    // Previous rows waiting for a partner, by key. Keys repeat only in tables without a
    // primary key, where identical rows pair up in order.
    let mut unmatched_by_key: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, row) in previous_rows.iter().enumerate().rev() {
        let key = rowKey(schema, row).to_string();
        unmatched_by_key.entry(key).or_default().push(index);
    }
    let mut previous_rows = previous_rows.into_iter().map(Some).collect::<Vec<_>>();

    let mut diff = TableDiff {
        columns: schema.columns.clone(),
        ..TableDiff::default()
    };
    for row in current_rows {
        let key = rowKey(schema, &row);
        let partner = unmatched_by_key
            .get_mut(&key.to_string())
            .and_then(Vec::pop)
            .and_then(|index| previous_rows[index].take());
        let Some(before) = partner else {
            diff.added.push(row);
            continue;
        };

        if before != row {
            diff.modified.push(ModifiedRow {
                key,
                before,
                after: row,
            });
        }
    }
    diff.removed = previous_rows.into_iter().flatten().collect();
    diff
}

/// Without a primary key the whole row is its identity, so such rows are only added or removed.
fn rowKey(schema: &TableSchema, row: &[Value]) -> Value {
    match schema.key_columns.as_slice() {
        [] => Value::Array(row.to_vec()),
        [column] => row[*column].clone(),
        columns => Value::Array(columns.iter().map(|column| row[*column].clone()).collect()),
    }
}

fn readTableNames(connection: &Connection) -> Result<BTreeSet<String>> {
    // sqlite_stat1 and other internal tables describe the file, not the game.
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'",
    )?;
    let names = statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(names)
}

fn readTableSchema(connection: &Connection, table: &str) -> Result<TableSchema> {
    let mut statement =
        connection.prepare("SELECT name, pk FROM pragma_table_info(?1) ORDER BY cid")?;
    let mut rows = statement.query([table])?;
    let mut columns = Vec::new();
    let mut key_positions = Vec::new();
    while let Some(row) = rows.next()? {
        // pk is the column's 1-based position in the primary key, or 0 outside it.
        let key_position: i64 = row.get(1)?;
        if key_position > 0 {
            key_positions.push((key_position, columns.len()));
        }
        columns.push(row.get(0)?);
    }

    key_positions.sort_unstable();
    Ok(TableSchema {
        columns,
        key_columns: key_positions
            .into_iter()
            .map(|(_, column)| column)
            .collect(),
    })
}

/// Reads rows as `schema.columns`, ordered by key. Columns this database lacks read as NULL,
/// so rows still pair up across a schema change.
fn readRows(connection: &Connection, table: &str, schema: &TableSchema) -> Result<Vec<Vec<Value>>> {
    let available_columns = readTableSchema(connection, table)?.columns;
    let select_list = schema
        .columns
        .iter()
        .map(|column| {
            if available_columns.contains(column) {
                quoteIdentifier(column)
            } else {
                "NULL".to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    // ORDER BY takes 1-based result positions, which also cover the NULL placeholders.
    let order_columns = if schema.key_columns.is_empty() {
        (0..schema.columns.len()).collect()
    } else {
        schema.key_columns.clone()
    };
    let order_by = order_columns
        .iter()
        .map(|column| (column + 1).to_string())
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "SELECT {select_list} FROM {} ORDER BY {order_by}",
        quoteIdentifier(table)
    );
    let mut statement = connection
        .prepare(&sql)
        .with_context(|| format!("failed to read master table {table}"))?;
    let mut rows = statement.query([])?;
    let mut values = Vec::new();
    while let Some(row) = rows.next()? {
        let mut cells = Vec::with_capacity(schema.columns.len());
        for index in 0..schema.columns.len() {
            cells.push(sqliteValueToJson(row.get_ref(index)?));
        }
        values.push(cells);
    }
    Ok(values)
}

fn quoteIdentifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::diffMasters;
    use crate::master_history::types::ModifiedRow;
    use rusqlite::Connection;
    use serde_json::json;

    fn database(sql: &str) -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(sql).unwrap();
        connection
    }

    #[test]
    fn matchesRowsBySinglePrimaryKey() {
        let previous = database(
            "CREATE TABLE text_data (id INTEGER PRIMARY KEY, text TEXT);
             INSERT INTO text_data VALUES (1, 'same'), (2, 'old'), (3, 'gone');",
        );
        let current = database(
            "CREATE TABLE text_data (id INTEGER PRIMARY KEY, text TEXT);
             INSERT INTO text_data VALUES (1, 'same'), (2, 'new'), (4, 'fresh');",
        );

        let tables = diffMasters(&previous, &current).unwrap();

        let text_data = &tables["text_data"];
        assert_eq!(text_data.columns, ["id", "text"]);
        assert_eq!(text_data.added, [vec![json!(4), json!("fresh")]]);
        assert_eq!(text_data.removed, [vec![json!(3), json!("gone")]]);
        assert_eq!(
            text_data.modified,
            [ModifiedRow {
                key: json!(2),
                before: vec![json!(2), json!("old")],
                after: vec![json!(2), json!("new")],
            }]
        );
    }

    #[test]
    fn matchesRowsByEveryCompositeKeyColumn() {
        let schema = "CREATE TABLE character_system_text (
            text TEXT, voice_id INTEGER, character_id INTEGER,
            PRIMARY KEY (character_id, voice_id));";
        let previous = database(&format!(
            "{schema} INSERT INTO character_system_text VALUES ('a', 1, 1001), ('b', 2, 1001);"
        ));
        let current = database(&format!(
            "{schema} INSERT INTO character_system_text VALUES ('a', 1, 1001), ('c', 2, 1001), ('d', 3, 1001);"
        ));

        let tables = diffMasters(&previous, &current).unwrap();

        let system_text = &tables["character_system_text"];
        assert_eq!(system_text.added, [vec![json!("d"), json!(3), json!(1001)]]);
        assert!(system_text.removed.is_empty());
        assert_eq!(system_text.modified.len(), 1);
        assert_eq!(system_text.modified[0].key, json!([1001, 2]));
    }

    #[test]
    fn treatsWholeRowsAsKeysWithoutPrimaryKey() {
        let previous = database(
            "CREATE TABLE home_eat (chara_id INTEGER, motion TEXT);
             INSERT INTO home_eat VALUES (1, 'a'), (1, 'a'), (2, 'b');",
        );
        let current = database(
            "CREATE TABLE home_eat (chara_id INTEGER, motion TEXT);
             INSERT INTO home_eat VALUES (1, 'a'), (2, 'c');",
        );

        let tables = diffMasters(&previous, &current).unwrap();

        let home_eat = &tables["home_eat"];
        assert_eq!(home_eat.added, [vec![json!(2), json!("c")]]);
        assert_eq!(
            home_eat.removed,
            [vec![json!(1), json!("a")], vec![json!(2), json!("b")]]
        );
        assert!(home_eat.modified.is_empty());
    }

    #[test]
    fn listsAddedAndDroppedTablesAndSkipsUnchangedOnes() {
        let previous = database(
            "CREATE TABLE dropped (id INTEGER PRIMARY KEY); INSERT INTO dropped VALUES (7);
             CREATE TABLE unchanged (id INTEGER PRIMARY KEY); INSERT INTO unchanged VALUES (1);
             ANALYZE;",
        );
        let current = database(
            "CREATE TABLE created (id INTEGER PRIMARY KEY); INSERT INTO created VALUES (8);
             CREATE TABLE unchanged (id INTEGER PRIMARY KEY); INSERT INTO unchanged VALUES (1);",
        );

        let tables = diffMasters(&previous, &current).unwrap();

        assert_eq!(
            tables.keys().collect::<Vec<_>>(),
            ["created", "dropped"],
            "unchanged and internal sqlite_stat1 tables are left out"
        );
        assert_eq!(tables["created"].added, [vec![json!(8)]]);
        assert_eq!(tables["dropped"].removed, [vec![json!(7)]]);
    }

    #[test]
    fn readsColumnsMissingFromThePreviousReleaseAsNull() {
        let previous = database(
            "CREATE TABLE card_data (id INTEGER PRIMARY KEY); INSERT INTO card_data VALUES (1);",
        );
        let current = database(
            "CREATE TABLE card_data (id INTEGER PRIMARY KEY, rarity INTEGER);
             INSERT INTO card_data VALUES (1, 3);",
        );

        let tables = diffMasters(&previous, &current).unwrap();

        let card_data = &tables["card_data"];
        assert!(card_data.added.is_empty());
        assert_eq!(card_data.modified[0].before, [json!(1), json!(null)]);
        assert_eq!(card_data.modified[0].after, [json!(1), json!(3)]);
    }
}

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Releases are named by the first 12 hex digits of the master's SHA-256.
pub const SHORT_HASH_LENGTH: usize = 12;

/// One entry of `masterdata/versions.json`, oldest release first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MasterRelease {
    pub hash: String,
    pub short_hash: String,
    pub date: String,
    pub previous_hash: Option<String>,
    /// Absent for the first release, and when the previous database was unavailable.
    pub summary: Option<DiffSummary>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffSummary {
    pub tables_changed: usize,
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
}

/// Contents of `masterdata/diffs/<short_hash>.json.gz`.
#[derive(Debug, Serialize, Deserialize)]
pub struct MasterDiff {
    pub from_hash: String,
    pub to_hash: String,
    pub date: String,
    pub summary: DiffSummary,
    pub tables: BTreeMap<String, TableDiff>,
}

/// Rows are cell arrays in `columns` order.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TableDiff {
    pub columns: Vec<String>,
    pub added: Vec<Vec<Value>>,
    pub removed: Vec<Vec<Value>>,
    pub modified: Vec<ModifiedRow>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct ModifiedRow {
    /// The primary-key value, or an array of values for a composite key.
    pub key: Value,
    pub before: Vec<Value>,
    pub after: Vec<Value>,
}

/// A published master database: its hash and the immutable snapshot it was generated from.
pub struct MasterSnapshot<'a> {
    pub sha256: &'a str,
    pub path: &'a std::path::Path,
}

/// `GET /api/ver/history`: the live game version and every earlier one, newest first.
#[derive(Debug, Deserialize)]
pub struct GameVersionHistory {
    pub current: GameVersionRecord,
    pub history: Vec<GameVersionRecord>,
}

#[derive(Debug, Deserialize)]
pub struct GameVersionRecord {
    pub resource_version: String,
    /// When uma.moe first saw this version, as `YYYY-MM-DDTHH:MM:SS` in UTC.
    pub updated_at: String,
}

/// A table's columns in declaration order and the indexes of its primary-key columns in key order.
pub(super) struct TableSchema {
    pub(super) columns: Vec<String>,
    pub(super) key_columns: Vec<usize>,
}

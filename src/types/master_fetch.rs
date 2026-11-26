use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The site's version endpoint. It answers for Global, and for another region as `?region=`.
pub(super) const VERSION_URL: &str = "https://uma.moe/api/ver";

/// Points the version check at another backend, such as a local one.
pub(super) const VERSION_URL_VARIABLE: &str = "GAME_VERSION_URL";

/// Global CDN resource versions advance on multiples of ten, with hotfixes in between (such as +5).
pub(super) const EARLY_PROBE_STEP: u64 = 5;

/// Look ahead up to +1000 from the live Global resource version (200 steps of +5).
pub(super) const EARLY_PROBE_MAX_STEPS: u64 = 200;

/// Concurrent HEAD probes when scanning ahead for an unreleased Global CDN manifest.
pub(super) const EARLY_PROBE_CONCURRENCY: usize = 8;

#[derive(Deserialize)]
pub(super) struct GameVersion {
    pub(super) app_version: String,
    pub(super) resource_version: String,
}

/// Metadata recorded beside `early-master.mdb` when an unreleased Global CDN manifest is found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EarlyMasterMeta {
    pub resource_version: String,
    pub live_resource_version: String,
    pub detected_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
}

/// A downloaded unreleased Global master kept beside the live one for preliminary timeline data.
#[derive(Debug, Clone)]
pub struct EarlyMaster {
    pub path: PathBuf,
    pub meta: EarlyMasterMeta,
}

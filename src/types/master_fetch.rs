pub(super) const BASE_URL: &str = "https://assets-umamusume-en.akamaized.net";

pub(super) const VERSION_URL: &str = "https://uma.moe/api/ver";

#[derive(serde::Deserialize)]
pub(super) struct GameVersion {
    pub(super) app_version: String,
    pub(super) resource_version: String,
}

pub(super) const LZ4_FRAME_MAGIC: [u8; 4] = [0x04, 0x22, 0x4D, 0x18];

pub(super) const BSV_MAGIC: u8 = 0xBF;

pub(super) const BSV_FORMAT_VERSION: u8 = 1;

pub(super) const BSV_FORMAT_ANONYMOUS: u8 = 1;

pub(super) const USER_AGENT: &str =
    "UnityPlayer/2022.3.46f1 (UnityWebRequest/1.0, libcurl/8.5.0-DEV)";

pub(super) const PLATFORM: &str = "Windows";

// ---------------------------------------------------------------------------
// BSV binary format parser
// ---------------------------------------------------------------------------

pub(super) enum BsvValue {
    Text(String),
    Int(u64),
}

pub(super) struct BsvParser<'a> {
    pub(super) data: &'a [u8],
    pub(super) offset: usize,
}

// ---------------------------------------------------------------------------
// Manifest parsing
// ---------------------------------------------------------------------------

pub(super) struct ManifestEntry {
    pub(super) name: String,
    #[allow(dead_code)]
    pub(super) size: u64,
    #[allow(dead_code)]
    pub(super) checksum: u64,
    pub(super) hname: String,
}

/// One of the game's asset CDNs. When sources are merged, earlier ones take precedence.
pub struct CdnSource {
    pub region: &'static str,
    pub base_url: &'static str,
}

pub static GLOBAL_CDN: CdnSource = CdnSource {
    region: "global",
    base_url: "https://assets-umamusume-en.akamaized.net",
};

pub(super) const LZ4_FRAME_MAGIC: [u8; 4] = [0x04, 0x22, 0x4D, 0x18];

pub(super) const BSV_MAGIC: u8 = 0xBF;

pub(super) const BSV_FORMAT_VERSION: u8 = 1;

pub(super) const BSV_FORMAT_ANONYMOUS: u8 = 1;

/// Column type of the per-file key: a raw big-endian u64, not a VLQ.
pub(super) const BSV_RAW_U64: u8 = 0x13;

pub(super) const USER_AGENT: &str =
    "UnityPlayer/2022.3.46f1 (UnityWebRequest/1.0, libcurl/8.5.0-DEV)";

pub(super) const PLATFORM: &str = "Windows";

/// Asset bundles keep their first bytes in the clear; encryption starts after them.
pub(super) const ASSET_BUNDLE_HEADER_SIZE: usize = 256;

pub(super) enum BsvValue {
    Text(String),
    Int(u64),
}

pub(super) struct BsvParser<'a> {
    pub(super) data: &'a [u8],
    pub(super) offset: usize,
}

/// A file the CDN lists: its game path, size, checksum and per-file decryption key.
#[derive(Clone, Debug)]
pub struct ManifestEntry {
    pub name: String,
    pub size: u64,
    pub checksum: u64,
    /// Zero for files that are not encrypted.
    pub key: u64,
    /// The bundles the game loads with this one, by name.
    pub dependencies: Vec<String>,
    pub(super) hname: String,
}

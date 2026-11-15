use crate::game_cdn::{CdnSource, ManifestEntry};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Raise when conversion output changes, so every image is rebuilt once.
pub(super) const CONVERTER_REVISION: u32 = 1;

/// The publisher's record in the bucket, outside every path the site routes there.
pub(super) const STATE_KEY: &str = "_state/game-art.json";

pub(super) const CONVERTER_PYTHON: &str = "/opt/game-art/bin/python";

pub(super) const CONVERTER_SCRIPT: &str = "/app/scripts/game_art_convert.py";

/// Bundles per converter process; Python and UnityPy take about a second to start.
pub(super) const CONVERT_BATCH_SIZE: usize = 32;

pub(super) const DOWNLOAD_CONCURRENCY: usize = 8;

pub(super) const UPLOAD_CONCURRENCY: usize = 16;

/// Game files of one kind and where the site serves each. A `{name}` segment matches digits, and
/// a name used twice must match the same digits both times.
pub(super) struct ArtFamily {
    pub(super) source: &'static str,
    pub(super) outputs: &'static [ArtOutput],
}

pub(super) struct ArtOutput {
    pub(super) path: &'static str,
    /// Width and height the site serves. The game squeezes textures to powers of two.
    pub(super) size: (u32, u32),
    /// The art's real shape when it is narrower than the served canvas; it is centred there.
    pub(super) art_size: Option<(u32, u32)>,
}

impl ArtOutput {
    const fn resized(path: &'static str, size: (u32, u32)) -> Self {
        Self {
            path,
            size,
            art_size: None,
        }
    }

    const fn centred(path: &'static str, size: (u32, u32), art_size: (u32, u32)) -> Self {
        Self {
            path,
            size,
            art_size: Some(art_size),
        }
    }
}

pub(super) const ART_FAMILIES: &[ArtFamily] = &[
    ArtFamily {
        source: "chara/chr{chara}/chr_icon_{chara}",
        outputs: &[ArtOutput::centred(
            "game-assets/umamusume_icons/chr_icon_{chara}.webp",
            (256, 256),
            (232, 256),
        )],
    },
    ArtFamily {
        source: "supportcard/support{card}/tex_support_card_{card}",
        outputs: &[ArtOutput::resized(
            "game-assets/umamusume_cards/tex_support_card_{card}.webp",
            (450, 600),
        )],
    },
    ArtFamily {
        source: "supportcard/support{card}/support_card_s_{card}",
        outputs: &[ArtOutput::resized(
            "assets/images/support_card/half/support_card_s_{card}.webp",
            (128, 128),
        )],
    },
    ArtFamily {
        source: "outgame/skillicon/utx_ico_skill_{icon}",
        outputs: &[ArtOutput::resized(
            "assets/images/skills/utx_ico_skill_{icon}.webp",
            (96, 96),
        )],
    },
    ArtFamily {
        source: "race/racetitle/thumb/thum_race_rt_000_{race}_00",
        outputs: &[ArtOutput::resized(
            "game-assets/textures/race_banners/thum_race_rt_000_{race}_00.webp",
            (128, 64),
        )],
    },
    ArtFamily {
        source: "uianimation/flash/singlemode/rank/utx_txt_rank_{rank}",
        outputs: &[ArtOutput::resized(
            "assets/images/icon/ranks/utx_txt_rank_{rank}.webp",
            (154, 154),
        )],
    },
    ArtFamily {
        source: "item/item_icon_{item}",
        outputs: &[ArtOutput::resized(
            "assets/images/item/item_icon_{item}.webp",
            (128, 128),
        )],
    },
];

/// Character portraits. The game names each by the outfit it shows, the site by card, so the
/// outfit of every card comes from master.mdb.
pub(super) const PORTRAIT_SOURCE: &str = "chara/chr{chara}/chara_stand_{chara}_{dress}";

pub(super) const PORTRAIT_OUTPUTS: &[ArtOutput] = &[
    ArtOutput::resized(
        "game-assets/character_thumbs/chara_stand_{chara}_{card}.webp",
        (128, 128),
    ),
    ArtOutput::resized(
        "assets/images/character_stand/chara_stand_{card}.webp",
        (128, 128),
    ),
];

#[derive(Clone, Copy, Debug)]
pub(super) struct CardOutfit {
    pub(super) chara: u32,
    pub(super) card: u32,
    pub(super) dress: u32,
}

/// The game files the site uses and the bucket keys built from each.
pub(super) struct Catalog {
    /// Portrait outputs by game file; the fixed families are matched by name instead.
    pub(super) portraits: HashMap<String, Vec<PlannedOutput>>,
}

/// What the publisher has put in the bucket, by game file name.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct ArtState {
    /// The resource version whose files were all handled.
    pub(super) resource_version: Option<String>,
    pub(super) files: BTreeMap<String, PublishedArt>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct PublishedArt {
    pub(super) region: String,
    pub(super) checksum: u64,
    pub(super) converter: u32,
    /// The bucket keys built from it, so a new card showing an existing outfit is built too.
    #[serde(default)]
    pub(super) outputs: Vec<String>,
    /// Set when this exact file could not be converted; it is retried once the file or the
    /// converter changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
}

/// A game file to build, the source it comes from and the bucket keys it is served at.
pub(super) struct ArtJob {
    pub(super) source: &'static CdnSource,
    pub(super) entry: ManifestEntry,
    pub(super) outputs: Vec<PlannedOutput>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PlannedOutput {
    pub(super) key: String,
    pub(super) size: (u32, u32),
    pub(super) art_size: Option<(u32, u32)>,
}

/// Where built images go: the asset bucket, or a local folder to look at.
pub(super) enum ArtDestination<'a> {
    Bucket(&'a crate::assets::Bucket),
    Folder(&'a std::path::Path),
}

pub struct GameArtOptions {
    /// The game's asset bundle base key (UmaViewer's ABKeyText).
    pub(super) base_key: Vec<u8>,
    pub(super) purge_origins: Vec<String>,
}

#[derive(Debug, Default)]
pub struct GameArtSummary {
    pub built: usize,
    pub uploaded: usize,
    /// Files that will not convert; recorded so they are not retried until they change.
    pub conversion_failures: usize,
    /// Files that could not be downloaded; retried on the next run.
    pub download_failures: usize,
}

#[derive(Serialize)]
pub(super) struct ConvertRequest<'a> {
    pub(super) jobs: Vec<ConvertJob<'a>>,
}

#[derive(Serialize)]
pub(super) struct ConvertJob<'a> {
    pub(super) bundle: String,
    pub(super) outputs: Vec<ConvertOutput<'a>>,
}

#[derive(Serialize)]
pub(super) struct ConvertOutput<'a> {
    pub(super) object: &'a str,
    pub(super) path: String,
    pub(super) format: &'static str,
    pub(super) size: [u32; 2],
    pub(super) art_size: Option<[u32; 2]>,
}

#[derive(Deserialize)]
pub(super) struct ConvertResponse {
    pub(super) results: Vec<ConvertResult>,
}

#[derive(Deserialize)]
pub(super) struct ConvertResult {
    pub(super) bundle: String,
    pub(super) error: Option<String>,
}

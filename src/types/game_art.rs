use crate::game_cdn::{CdnSource, ManifestEntry};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::ops::RangeInclusive;

/// Raise when conversion output changes, so every image is rebuilt once.
pub(super) const CONVERTER_REVISION: u32 = 1;

/// The publisher's record in the bucket, outside every path the site routes there: an
/// `index.json` naming the shards, and one JSON file of records per shard.
pub(super) const STATE_FOLDER: &str = "_state/game-art";

/// The single file earlier versions kept the record in; it is split into shards once.
pub(super) const LEGACY_STATE_KEY: &str = "_state/game-art.json";

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

/// How a picture is fitted to the size the site serves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Fit {
    /// Kept as drawn, as atlas sprites already are.
    Native,
    /// Resized to this width and height; the game squeezes textures to powers of two.
    Resize(u32, u32),
    /// Resized to `art` and centred on a transparent canvas of `size`, for art narrower than it.
    Centre { size: (u32, u32), art: (u32, u32) },
}

pub(super) struct ArtOutput {
    pub(super) path: &'static str,
    pub(super) fit: Fit,
}

impl ArtOutput {
    const fn native(path: &'static str) -> Self {
        Self {
            path,
            fit: Fit::Native,
        }
    }

    const fn resized(path: &'static str, size: (u32, u32)) -> Self {
        Self {
            path,
            fit: Fit::Resize(size.0, size.1),
        }
    }

    const fn centred(path: &'static str, size: (u32, u32), art: (u32, u32)) -> Self {
        Self {
            path,
            fit: Fit::Centre { size, art },
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
    // Chibi sprites keep their size: most are 256x256, scenes with props are wider.
    ArtFamily {
        source: "chara/chr{chara}/petit/petit_chr_{chara}_{dress}_{pose}",
        outputs: &[ArtOutput::native(
            "game-assets/petit/petit_chr_{chara}_{dress}_{pose}.webp",
        )],
    },
    ArtFamily {
        source: "outgame/campaign/valentine/chara/petit_chr_{chara}_{dress}_{pose}",
        outputs: &[ArtOutput::native(
            "game-assets/petit/petit_chr_{chara}_{dress}_{pose}.webp",
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

/// Sprites cut from one of the game's atlases. The sprites name their pictures in a second
/// bundle, so both are loaded together.
pub(super) struct AtlasFamily {
    pub(super) atlas: &'static str,
    pub(super) texture: &'static str,
    pub(super) sprites: &'static [AtlasSprites],
}

pub(super) enum AtlasSprites {
    /// One sprite and the paths it is served at.
    Named {
        sprite: &'static str,
        paths: &'static [&'static str],
    },
    /// A run of sprites whose `{n}`, zero-padded to `width` digits, also names their paths.
    Numbered {
        sprite: &'static str,
        numbers: RangeInclusive<u32>,
        width: usize,
        paths: &'static [&'static str],
    },
}

const fn named(sprite: &'static str, paths: &'static [&'static str]) -> AtlasSprites {
    AtlasSprites::Named { sprite, paths }
}

const fn numbered(
    sprite: &'static str,
    numbers: RangeInclusive<u32>,
    width: usize,
    paths: &'static [&'static str],
) -> AtlasSprites {
    AtlasSprites::Numbered {
        sprite,
        numbers,
        width,
        paths,
    }
}

pub(super) const ATLAS_FAMILIES: &[AtlasFamily] = &[
    AtlasFamily {
        atlas: "atlas/statusrank/statusrank",
        texture: "atlas/statusrank/statusrank_tex",
        sprites: &[numbered(
            "utx_ico_statusrank_{n}",
            0..=97,
            2,
            &[
                "game-assets/textures/uma_ranks/utx_ico_statusrank_{n}.webp",
                "assets/images/icon/statusrank/utx_ico_statusrank_{n}.webp",
            ],
        )],
    },
    AtlasFamily {
        atlas: "atlas/circle/circle",
        texture: "atlas/circle/circle_tex",
        sprites: &[numbered(
            "utx_ico_circle_rank_{n}",
            1..=11,
            2,
            &["assets/images/icon/circle_rank/utx_ico_circle_rank_{n}.webp"],
        )],
    },
    AtlasFamily {
        atlas: "atlas/common/common",
        texture: "atlas/common/common_tex",
        sprites: &[
            named(
                "utx_ico_weather_00",
                &[
                    "game-assets/textures/utx_ico_weather_00.png",
                    "game-assets/track_details/Sunny.webp",
                ],
            ),
            named(
                "utx_ico_weather_01",
                &[
                    "game-assets/textures/utx_ico_weather_01.png",
                    "game-assets/track_details/Cloudy.webp",
                ],
            ),
            named(
                "utx_ico_weather_02",
                &[
                    "game-assets/textures/utx_ico_weather_02.png",
                    "game-assets/track_details/Rainy.webp",
                ],
            ),
            named(
                "utx_ico_weather_03",
                &[
                    "game-assets/textures/utx_ico_weather_03.png",
                    "game-assets/track_details/Snowy.webp",
                ],
            ),
            numbered(
                "utx_ico_timezone_{n}",
                0..=2,
                2,
                &["game-assets/textures/utx_ico_timezone_{n}.png"],
            ),
            numbered(
                "utx_ico_runstyle_{n}",
                0..=3,
                2,
                &["game-assets/textures/utx_ico_runstyle_{n}.webp"],
            ),
            named("utx_txt_runstyle_00", &["game-assets/textures/front.webp"]),
            named("utx_txt_runstyle_01", &["game-assets/textures/pace.webp"]),
            named("utx_txt_runstyle_02", &["game-assets/textures/late.webp"]),
            named("utx_txt_runstyle_03", &["game-assets/textures/end.webp"]),
            named(
                "utx_ico_obtain_00",
                &[
                    "assets/images/icon/stats/speed.webp",
                    "game-assets/textures/speed.webp",
                ],
            ),
            named(
                "utx_ico_obtain_01",
                &[
                    "assets/images/icon/stats/stamina.webp",
                    "game-assets/textures/stamina.webp",
                ],
            ),
            named(
                "utx_ico_obtain_02",
                &[
                    "assets/images/icon/stats/power.webp",
                    "game-assets/textures/power.webp",
                ],
            ),
            named(
                "utx_ico_obtain_03",
                &[
                    "assets/images/icon/stats/guts.webp",
                    "game-assets/textures/guts.webp",
                ],
            ),
            named(
                "utx_ico_obtain_04",
                &[
                    "assets/images/icon/stats/wit.webp",
                    "game-assets/textures/wit.webp",
                ],
            ),
            named(
                "utx_ico_obtain_05",
                &["assets/images/icon/stats/friend.webp"],
            ),
            named(
                "utx_ico_obtain_06",
                &["assets/images/icon/stats/group.webp"],
            ),
            named("utx_ico_rarity_00", &["game-assets/support-rarity/r.png"]),
            named("utx_ico_rarity_01", &["game-assets/support-rarity/sr.png"]),
            named("utx_ico_rarity_02", &["game-assets/support-rarity/ssr.png"]),
            named(
                "utx_txt_season_00",
                &["game-assets/track_details/Spring.webp"],
            ),
            named(
                "utx_txt_season_01",
                &["game-assets/track_details/Summer.webp"],
            ),
            named(
                "utx_txt_season_02",
                &["game-assets/track_details/Fall.webp"],
            ),
            named(
                "utx_txt_season_03",
                &["game-assets/track_details/Winter.webp"],
            ),
        ],
    },
    AtlasFamily {
        atlas: "atlas/race/race",
        texture: "atlas/race/race_tex",
        sprites: &[
            named(
                "utx_txt_raceresult_condition_1",
                &["game-assets/track_details/Firm.webp"],
            ),
            named(
                "utx_txt_raceresult_condition_2",
                &["game-assets/track_details/Good.webp"],
            ),
            named(
                "utx_txt_raceresult_condition_3",
                &["game-assets/track_details/Soft.webp"],
            ),
            named(
                "utx_txt_raceresult_condition_4",
                &["game-assets/track_details/Heavy.webp"],
            ),
        ],
    },
    AtlasFamily {
        atlas: "atlas/racecommon/racecommon",
        texture: "atlas/racecommon/racecommon_tex",
        sprites: &[
            named(
                "utx_ico_motivation_s_00",
                &["game-assets/textures/awful.webp"],
            ),
            named(
                "utx_ico_motivation_s_01",
                &["game-assets/textures/bad.webp"],
            ),
            named(
                "utx_ico_motivation_s_02",
                &["game-assets/textures/normal.webp"],
            ),
            named(
                "utx_ico_motivation_s_03",
                &["game-assets/textures/good.webp"],
            ),
            named(
                "utx_ico_motivation_s_04",
                &["game-assets/textures/great.webp"],
            ),
        ],
    },
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
    pub(super) atlases: HashMap<&'static str, AtlasPlan>,
}

/// The sprites cut from one atlas and the texture bundle they draw from.
pub(super) struct AtlasPlan {
    pub(super) texture: &'static str,
    pub(super) outputs: Vec<PlannedOutput>,
}

/// What the publisher has put in the bucket, by game file name.
#[derive(Default, Deserialize)]
pub(super) struct ArtState {
    /// The resource version whose files were all handled.
    pub(super) resource_version: Option<String>,
    pub(super) files: BTreeMap<String, PublishedArt>,
}

/// `index.json` in the state folder: the shards and how many records each holds.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct StateIndex {
    pub(super) resource_version: Option<String>,
    pub(super) shards: BTreeMap<String, usize>,
}

/// One shard file of records.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct StateShard<Record> {
    pub(super) files: BTreeMap<String, Record>,
}

/// Where the record was read from; the single legacy file is split on the first real run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum StateOrigin {
    Empty,
    Shards,
    Legacy,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct PublishedArt {
    pub(super) region: String,
    pub(super) checksum: u64,
    pub(super) converter: u32,
    /// The bucket keys built from it, so a new card showing an existing outfit is built too.
    #[serde(default)]
    pub(super) outputs: Vec<String>,
    /// Checksums of the bundles it draws from, such as an atlas's texture.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) dependency_checksums: Vec<u64>,
    /// Set when this exact file could not be converted; it is retried once the file or the
    /// converter changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
}

/// A game file to build, the source it comes from and the bucket keys it is served at.
pub(super) struct ArtJob {
    pub(super) source: &'static CdnSource,
    pub(super) entry: ManifestEntry,
    /// Bundles loaded with it, such as an atlas's texture.
    pub(super) dependencies: Vec<ManifestEntry>,
    pub(super) outputs: Vec<PlannedOutput>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PlannedOutput {
    pub(super) key: String,
    /// The sprite or texture in the bundle that is drawn.
    pub(super) object: String,
    pub(super) fit: Fit,
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
    pub(super) dependencies: Vec<String>,
    pub(super) outputs: Vec<ConvertOutput<'a>>,
}

#[derive(Serialize)]
pub(super) struct ConvertOutput<'a> {
    pub(super) object: &'a str,
    pub(super) path: String,
    pub(super) format: &'static str,
    /// Absent to keep the picture's own size.
    pub(super) size: Option<[u32; 2]>,
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

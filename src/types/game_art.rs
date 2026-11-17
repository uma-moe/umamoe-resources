use crate::game_cdn::{CdnSource, ManifestEntry};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::ops::RangeInclusive;

/// Raise when picture output changes, so every picture is rebuilt once.
pub(super) const CONVERTER_REVISION: u32 = 1;

/// Raise when model output changes, so every track model and course model is rebuilt once.
/// Revision 2 embeds the textures in the model; 3 names each material's shader and keywords.
pub(super) const MODEL_CONVERTER_REVISION: u32 = 3;

/// Raise when uma or race motion output changes, so every one is rebuilt once.
/// Revision 4 names each material's shader and keywords.
pub(super) const UMA_CONVERTER_REVISION: u32 = 4;

/// Raise when the shader files change, so they are written again.
pub(super) const SHADER_CONVERTER_REVISION: u32 = 2;

/// Raise when chibi or chibi clip output changes, so every one is rebuilt once.
pub(super) const CHIBI_CONVERTER_REVISION: u32 = 1;

/// Raise when the output of cut-in timelines, clips, sets or props changes, so every one is
/// rebuilt once.
pub(super) const CUTIN_CONVERTER_REVISION: u32 = 1;

/// Raise when effect output changes, so every effect is rebuilt once.
pub(super) const EFFECT_CONVERTER_REVISION: u32 = 1;

/// The bundle of every shader the game draws with, named so in each platform's file list and
/// category. Models load the Windows one to name their materials' shaders; the Android one, whose
/// programs are GLSL ES, is published.
pub(super) const SHADER_BUNDLE: &str = "shader";

/// The published shaders' record, named with its platform since Windows lists a bundle of the same
/// name.
pub(super) const SHADER_RECORD: &str = "android/shader";

/// The publisher's record in the bucket, outside every path the site routes there: an
/// `index.json` linking the shards, and one JSON file of records per top-level game folder.
pub(super) const STATE_FOLDER: &str = "_state/game-art";

/// The directory documents last published and the MD5 of each, so only changed ones are written
/// and those no longer built are deleted.
pub(super) const DIRECTORY_STATE_KEY: &str = "_state/game-art/_directory.json";

/// Raise when a directory document changes shape, so a page can refuse a shape it does not know.
pub(super) const DOCUMENT_SCHEMA: u32 = 1;

/// Documents larger than this are reported; an uma's should stay far below it.
pub(super) const DOCUMENT_SIZE_LIMIT: usize = 64 * 1024;

pub(super) const CONVERTER_PYTHON: &str = "/opt/game-art/bin/python";

pub(super) const CONVERTER_SCRIPT: &str = "/app/scripts/game_art_convert.py";

/// Bundles per converter process in a batch; Python and UnityPy take about a second to start.
pub(super) const JOBS_PER_CONVERTER: usize = 8;

/// Sets how many converter processes run at once.
pub(super) const CONVERTERS_VARIABLE: &str = "GAME_ART_CONVERTERS";

pub(super) const DEFAULT_CONVERTER_PROCESSES: usize = 4;

/// Each converter holds its bundles' decoded textures in memory.
pub(super) const MAX_CONVERTER_PROCESSES: usize = 8;

pub(super) const DOWNLOAD_CONCURRENCY: usize = 16;

pub(super) const UPLOAD_CONCURRENCY: usize = 16;

/// Orphaned records are left alone when more than this many, and this share of all records,
/// look orphaned at once: an incomplete file list is likelier than such a change.
pub(super) const ORPHAN_ALERT_MINIMUM: usize = 20;

pub(super) const ORPHAN_ALERT_PERCENT: usize = 20;

/// Race track prefabs: every `pfb_` bundle below this folder becomes one model.
pub(super) const TRACK_FOLDER: &str = "3d/env/race/";

/// The CDN category that lists race tracks and every bundle they draw from.
pub(super) const TRACK_CATEGORY: &str = "_3d_cutt";

/// Course parts the game places where they are modelled: the track, its turf, the trees and the
/// stands. Rails, signs and flower beds are placed along the track at runtime, and 7013 only
/// marks where 3D spectators stand.
pub(super) const COURSE_PARTS: &[&str] = &["0000", "0001", "6000", "7001", "7002", "7003"];

/// The weather a course model shows, in the game's variant digits: spring, sunny, day.
pub(super) const COURSE_ENVIRONMENT: &str = "112";

/// The sky of `COURSE_ENVIRONMENT`, which every course shares.
pub(super) const COURSE_SKY: &str =
    "3d/env/race/race00000/race00000_9000/pfb_env_race00000_9000_112";

/// The master.mdb `text_data` category of racecourse names, indexed by course number.
pub(super) const COURSE_NAME_CATEGORY: u32 = 35;

/// Outfit bodies, one folder each: `3d/chara/body/bdy1001_00`, which names the uma's record.
pub(super) const UMA_BODY_FOLDER: &str = "3d/chara/body/";

/// The `text_data` categories of character names by character and outfit names by outfit.
pub(super) const CHARA_NAME_CATEGORY: u32 = 6;
pub(super) const DRESS_NAME_CATEGORY: u32 = 14;

/// Race clips that move the whole body, one folder per motion set.
pub(super) const RACE_MOTION_SOURCE: &str = "3d/motion/racemain/body/";

/// The skeleton a race motion is exported on. Every body and every tail share one rest skeleton,
/// so Special Week's racewear and the common tail stand for all of them.
pub(super) const MOTION_SKELETON: &[&str] = &[
    "3d/chara/body/bdy1001_00/pfb_bdy1001_00",
    "3d/chara/tail/tail0001_00/pfb_tail0001_00",
];

/// The textures the game loads into a tail's material for its character
/// (`ModelLoader.GetTailTextures`), as `tex_tail0001_00_{suffix}` in the tail's `textures`
/// folder. Each goes in the slot the game's own body and head materials hold that kind in.
pub(super) const TAIL_TEXTURES: &[(&str, &str)] = &[
    ("_MainTex", "{chara}_diff"),
    ("_ToonMap", "{chara}_shad_c"),
    ("_TripleMaskMap", "0000_base"),
    ("_OptionMaskMap", "0000_ctrl"),
];

/// Chibi bodies, one folder per outfit: `3d/chara/mini/body/mbdy1001_00`, which names the
/// chibi's record.
pub(super) const CHIBI_BODY_FOLDER: &str = "3d/chara/mini/body/";

/// The face every chibi shares, the only one the file list holds; its materials are empty until
/// the game loads the character's sheets into them.
pub(super) const CHIBI_FACE_FOLDER: &str = "3d/chara/mini/head/mchr0001_00";
pub(super) const CHIBI_FACE_PREFAB: &str = "3d/chara/mini/head/mchr0001_00/pfb_mchr0001_00_face0";

/// The face materials the game fills from the character's head (`MINI_EYE_TEXTURE_PATH`,
/// `MINI_MAYU_TEXTURE_PATH`, `MINI_MOUTH_TEXTURE_PATH`), with `tex_mchr1001_00_{suffix}` from the
/// head's `textures` folder.
pub(super) const CHIBI_FACE_SHEETS: &[(&str, &str)] = &[
    ("mtl_mchr0001_00_eye", "eye_diff"),
    ("mtl_mchr0001_00_mayu_l", "mayu_diff"),
    ("mtl_mchr0001_00_mayu_r", "mayu_diff"),
    ("mtl_mchr0001_00_mouth", "mouth_diff"),
];

/// The face materials the game fills by the character's skin colour (`MINI_FACE_TEXTURE_PATH`,
/// `MINI_CHEEK_TEXTURE_PATH`), with `tex_mchr0001_00_face0_{skin}_{suffix}`.
pub(super) const CHIBI_SKIN_SHEETS: &[(&str, &str)] = &[
    ("mtl_mchr0001_00_face0", "diff"),
    ("mtl_mchr0001_00_cheek", "cheek0"),
];

/// Chibi clips: what every chibi can play by scene, and what one character plays alone.
pub(super) const CHIBI_CLIP_FOLDER: &str = "3d/motion/mini/";

/// The skeleton chibi body and face clips are exported on: Special Week's racewear chibi, her
/// hair and the common tail.
pub(super) const CHIBI_SKELETON: &[&str] = &[
    "3d/chara/mini/body/mbdy1001_00/pfb_mbdy1001_00",
    "3d/chara/mini/head/mchr1001_00/pfb_mchr1001_00_hair",
    "3d/chara/mini/tail/mtail0001_00/pfb_mtail0001_00",
];

/// Cut-in clips, one folder per timeline: per shot the uma's body, face, ears and position, the
/// camera, and the props and sets in it.
pub(super) const CUTIN_CLIP_FOLDER: &str = "3d/motion/cutin/";

/// The skeleton cut-in body and face clips are exported on: `MOTION_SKELETON` with Special Week's
/// head, whose face and ear bones the face clips move.
pub(super) const CUTIN_SKELETON: &[&str] = &[
    "3d/chara/body/bdy1001_00/pfb_bdy1001_00",
    "3d/chara/head/chr1001_00/pfb_chr1001_00",
    "3d/chara/tail/tail0001_00/pfb_tail0001_00",
];

/// Skill cut-in timelines, the umas' ults among them, each in a folder of its name:
/// `cutt/cutin/skill/crd100101_001/crd100101_001`.
pub(super) const CUTIN_TIMELINE_FOLDER: &str = "cutt/cutin/skill/";

/// The sets cut-ins play in, each played by one timeline.
pub(super) const CUTIN_SET_FOLDER: &str = "3d/env/cutin/";

/// The props umas hold in cut-ins, one folder each: `3d/chara/prop/prop1001_00`.
pub(super) const PROP_FOLDER: &str = "3d/chara/prop/";

/// The game's visual effects; every `pfb_` bundle below it is one effect.
pub(super) const EFFECT_FOLDER: &str = "3d/effect/";

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
            "art/characters/{chara}/icon.webp",
            (256, 256),
            (232, 256),
        )],
    },
    ArtFamily {
        source: "supportcard/support{card}/tex_support_card_{card}",
        outputs: &[ArtOutput::resized(
            "art/support-cards/{card}/card.webp",
            (450, 600),
        )],
    },
    ArtFamily {
        source: "supportcard/support{card}/support_card_s_{card}",
        outputs: &[ArtOutput::resized(
            "art/support-cards/{card}/half.webp",
            (128, 128),
        )],
    },
    ArtFamily {
        source: "outgame/skillicon/utx_ico_skill_{icon}",
        outputs: &[ArtOutput::resized("art/skills/icons/{icon}.webp", (96, 96))],
    },
    ArtFamily {
        source: "race/racetitle/thumb/thum_race_rt_000_{race}_00",
        outputs: &[ArtOutput::resized(
            "art/races/banners/{race}.webp",
            (128, 64),
        )],
    },
    ArtFamily {
        source: "uianimation/flash/singlemode/rank/utx_txt_rank_{rank}",
        outputs: &[ArtOutput::resized("art/ui/ranks/{rank}.webp", (154, 154))],
    },
    ArtFamily {
        source: "item/item_icon_{item}",
        outputs: &[ArtOutput::resized("art/items/{item}.webp", (128, 128))],
    },
];

/// Chibi sprites, served in their outfit's uma folder. Most are 256x256 and scenes with props are
/// wider, so they keep their size. The Valentine campaign's poses are kept apart.
pub(super) const SPRITE_SOURCES: &[SpriteSource] = &[
    SpriteSource {
        source: "chara/chr{chara}/petit/petit_chr_{chara}_{dress}_{pose}",
        folder: "sprites",
    },
    SpriteSource {
        source: "outgame/campaign/valentine/chara/petit_chr_{chara}_{dress}_{pose}",
        folder: "sprites/valentine",
    },
];

pub(super) struct SpriteSource {
    pub(super) source: &'static str,
    /// The folder below the uma's, where each pose is `{pose}.webp`.
    pub(super) folder: &'static str,
}

/// Character portraits. The game names each by the outfit it shows; it is served in the folder of
/// the card that wears it, so the outfit of every card comes from master.mdb.
pub(super) const PORTRAIT_SOURCE: &str = "chara/chr{chara}/chara_stand_{chara}_{dress}";

pub(super) const PORTRAIT_OUTPUTS: &[ArtOutput] = &[
    ArtOutput::resized("art/umas/{card}/portrait.webp", (128, 128)),
    ArtOutput::resized("art/umas/{card}/stand.webp", (512, 512)),
];

/// The other pictures of an outfit, kept at the game's size in its uma folder. A `{dress}`
/// picture is named by the outfit it shows, a `{card}` picture by the card.
pub(super) const OUTFIT_PICTURES: &[OutfitPicture] = &[
    OutfitPicture {
        source: "chara/chr{chara}/chr_icon_{chara}_{dress}_01",
        name: "icon",
        file: "icon.webp",
    },
    OutfitPicture {
        source: "chara/chr{chara}/chr_icon_{chara}_{dress}_02",
        name: "iconGold",
        file: "icon-gold.webp",
    },
    OutfitPicture {
        source: "chara/chr{chara}/trained_chr_icon_{chara}_{dress}_01",
        name: "trainedIcon",
        file: "trained-icon.webp",
    },
    OutfitPicture {
        source: "chara/chr{chara}/trained_chr_icon_{chara}_{dress}_02",
        name: "trainedIconGold",
        file: "trained-icon-gold.webp",
    },
    OutfitPicture {
        source: "chara/chr{chara}/push_icon_{chara}_{dress}",
        name: "pushIcon",
        file: "push-icon.webp",
    },
    OutfitPicture {
        source: "outgame/dress/dress_{dress}_0_00",
        name: "dress",
        file: "dress.webp",
    },
    OutfitPicture {
        source: "outgame/note/gacha_thumb/chara_card/gacha_thumb_{card}",
        name: "gachaThumb",
        file: "gacha-thumb.webp",
    },
];

pub(super) struct OutfitPicture {
    pub(super) source: &'static str,
    /// The key the uma document lists the picture under.
    pub(super) name: &'static str,
    pub(super) file: &'static str,
}

/// Sprites cut from one of the game's atlases. The sprites name their pictures in a second
/// bundle, so both are loaded together.
pub(super) struct AtlasFamily {
    pub(super) atlas: &'static str,
    pub(super) texture: &'static str,
    pub(super) sprites: &'static [AtlasSprites],
}

pub(super) enum AtlasSprites {
    /// One sprite and the path it is served at.
    Named {
        sprite: &'static str,
        path: &'static str,
    },
    /// A run of sprites whose `{n}`, zero-padded to `width` digits, also names their path.
    Numbered {
        sprite: &'static str,
        numbers: RangeInclusive<u32>,
        width: usize,
        path: &'static str,
    },
}

const fn named(sprite: &'static str, path: &'static str) -> AtlasSprites {
    AtlasSprites::Named { sprite, path }
}

const fn numbered(
    sprite: &'static str,
    numbers: RangeInclusive<u32>,
    width: usize,
    path: &'static str,
) -> AtlasSprites {
    AtlasSprites::Numbered {
        sprite,
        numbers,
        width,
        path,
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
            "art/ui/status-ranks/{n}.webp",
        )],
    },
    AtlasFamily {
        atlas: "atlas/circle/circle",
        texture: "atlas/circle/circle_tex",
        sprites: &[numbered(
            "utx_ico_circle_rank_{n}",
            1..=11,
            2,
            "art/ui/circle-ranks/{n}.webp",
        )],
    },
    AtlasFamily {
        atlas: "atlas/common/common",
        texture: "atlas/common/common_tex",
        sprites: &[
            named("utx_ico_weather_00", "art/ui/weather/sunny.webp"),
            named("utx_ico_weather_01", "art/ui/weather/cloudy.webp"),
            named("utx_ico_weather_02", "art/ui/weather/rainy.webp"),
            named("utx_ico_weather_03", "art/ui/weather/snowy.webp"),
            numbered(
                "utx_ico_timezone_{n}",
                0..=2,
                2,
                "art/ui/times-of-day/{n}.webp",
            ),
            named(
                "utx_ico_runstyle_00",
                "art/ui/running-styles/icons/front.webp",
            ),
            named(
                "utx_ico_runstyle_01",
                "art/ui/running-styles/icons/pace.webp",
            ),
            named(
                "utx_ico_runstyle_02",
                "art/ui/running-styles/icons/late.webp",
            ),
            named(
                "utx_ico_runstyle_03",
                "art/ui/running-styles/icons/end.webp",
            ),
            named(
                "utx_txt_runstyle_00",
                "art/ui/running-styles/labels/front.webp",
            ),
            named(
                "utx_txt_runstyle_01",
                "art/ui/running-styles/labels/pace.webp",
            ),
            named(
                "utx_txt_runstyle_02",
                "art/ui/running-styles/labels/late.webp",
            ),
            named(
                "utx_txt_runstyle_03",
                "art/ui/running-styles/labels/end.webp",
            ),
            named("utx_ico_obtain_00", "art/ui/stats/speed.webp"),
            named("utx_ico_obtain_01", "art/ui/stats/stamina.webp"),
            named("utx_ico_obtain_02", "art/ui/stats/power.webp"),
            named("utx_ico_obtain_03", "art/ui/stats/guts.webp"),
            named("utx_ico_obtain_04", "art/ui/stats/wit.webp"),
            named("utx_ico_obtain_05", "art/ui/stats/friend.webp"),
            named("utx_ico_obtain_06", "art/ui/stats/group.webp"),
            named("utx_ico_rarity_00", "art/ui/support-rarities/r.webp"),
            named("utx_ico_rarity_01", "art/ui/support-rarities/sr.webp"),
            named("utx_ico_rarity_02", "art/ui/support-rarities/ssr.webp"),
            named("utx_txt_season_00", "art/ui/seasons/spring.webp"),
            named("utx_txt_season_01", "art/ui/seasons/summer.webp"),
            named("utx_txt_season_02", "art/ui/seasons/fall.webp"),
            named("utx_txt_season_03", "art/ui/seasons/winter.webp"),
        ],
    },
    AtlasFamily {
        atlas: "atlas/race/race",
        texture: "atlas/race/race_tex",
        sprites: &[
            named(
                "utx_txt_raceresult_condition_1",
                "art/ui/ground-conditions/firm.webp",
            ),
            named(
                "utx_txt_raceresult_condition_2",
                "art/ui/ground-conditions/good.webp",
            ),
            named(
                "utx_txt_raceresult_condition_3",
                "art/ui/ground-conditions/soft.webp",
            ),
            named(
                "utx_txt_raceresult_condition_4",
                "art/ui/ground-conditions/heavy.webp",
            ),
        ],
    },
    AtlasFamily {
        atlas: "atlas/racecommon/racecommon",
        texture: "atlas/racecommon/racecommon_tex",
        sprites: &[
            named("utx_ico_motivation_s_00", "art/ui/moods/awful.webp"),
            named("utx_ico_motivation_s_01", "art/ui/moods/bad.webp"),
            named("utx_ico_motivation_s_02", "art/ui/moods/normal.webp"),
            named("utx_ico_motivation_s_03", "art/ui/moods/good.webp"),
            named("utx_ico_motivation_s_04", "art/ui/moods/great.webp"),
        ],
    },
];

#[derive(Clone, Copy, Debug)]
pub(super) struct CardOutfit {
    pub(super) chara: u32,
    pub(super) card: u32,
    pub(super) dress: u32,
}

/// What master.mdb says about the game files the site uses, and the pictures built from them.
pub(super) struct Catalog {
    /// Portrait outputs by game file; the fixed families are matched by name instead.
    pub(super) portraits: HashMap<String, Vec<PlannedOutput>>,
    pub(super) atlases: HashMap<&'static str, AtlasPlan>,
    /// Racecourse names by course number, `Tokyo` for 10006.
    pub(super) course_names: HashMap<u32, String>,
    /// Each playable card and the outfit it wears at its top rarity.
    pub(super) cards: Vec<CardOutfit>,
    /// Every outfit with a body of its own, in outfit order.
    pub(super) outfits: Vec<OutfitRow>,
    /// The folder below `art/umas/` of each outfit, by game outfit.
    pub(super) uma_folders: HashMap<u32, String>,
    /// Every outfit with a body of its own; those whose files are listed become umas.
    pub(super) umas: Vec<UmaOutfit>,
    /// Every such outfit with a chibi; those whose files are listed become chibis.
    pub(super) chibis: Vec<ChibiOutfit>,
    pub(super) support_cards: Vec<SupportCard>,
}

/// A support card and the character it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SupportCard {
    pub(super) id: u32,
    pub(super) chara: u32,
}

/// An outfit with a body of its own, as master.mdb describes it with its character.
#[derive(Clone, Debug)]
pub(super) struct OutfitRow {
    pub(super) dress: u32,
    pub(super) chara: u32,
    /// Whether the outfit has a chibi (`have_mini`).
    pub(super) has_chibi: bool,
    pub(super) body_sub: i64,
    pub(super) head_sub: i64,
    /// The outfit's own tail model and variant; model -1 when it wears its character's.
    pub(super) dress_tail: (i64, i64),
    /// The character's tail model, -1 for none.
    pub(super) chara_tail: i64,
    /// The character's skin colour, which picks its face textures.
    pub(super) skin: i64,
    /// `RaceRunningType`: 1 standard, 2 pitch, 3 stride.
    pub(super) running_type: i64,
    /// `RacePersonalityType` of its runs and gate starts, and of its finish reactions.
    pub(super) run_personality: i64,
    pub(super) finish_personality: i64,
    pub(super) chara_name: Option<String>,
    pub(super) dress_name: Option<String>,
}

/// An outfit with its own body, and the uma built from it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct UmaOutfit {
    /// The record it is published under: its body's folder, which no other outfit wears.
    pub(super) record: String,
    /// Body, head and tail prefabs, merged in this order; staff have no tail.
    pub(super) prefabs: Vec<String>,
    /// The tail textures of this character. A uma job keeps those the file list has.
    pub(super) tail_textures: Vec<SlotTexture>,
    /// What Blender names the armature: `Special Week: Special Dreamer`.
    pub(super) title: String,
    pub(super) key: String,
    pub(super) traits: UmaTraits,
}

/// What a uma's model records about it, shown in Blender as the armature's custom properties.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UmaTraits {
    pub(super) chara: u32,
    pub(super) dress: u32,
    /// The motion set folder of its runs and gate starts, `bright`.
    pub(super) run_motions: String,
    /// The motion set folder of its finish reactions, which the game picks separately.
    pub(super) finish_motions: String,
    /// `standard`, `pitch` or `stride`: which run files of the set it uses.
    pub(super) running_form: &'static str,
}

/// A texture the game puts in a material slot at runtime.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SlotTexture {
    pub(super) slot: &'static str,
    pub(super) name: String,
}

/// An outfit's chibi, built from its own body like its uma.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ChibiOutfit {
    /// The record it is published under: its body's folder, `3d/chara/mini/body/mbdy1001_00`.
    pub(super) record: String,
    pub(super) key: String,
    /// Body, hair and tail prefabs and their runtime textures. A chibi job keeps the textures the
    /// file list has.
    pub(super) model: CharacterModel,
}

/// A skinned model: an outfit's chibi, whose prefabs are merged on one skeleton in this order, or
/// a prop.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CharacterModel {
    /// What Blender names the armature: `Special Week: Special Dreamer`.
    pub(super) title: String,
    pub(super) prefabs: Vec<String>,
    /// Textures the game puts in a material's slot at runtime, such as a chibi's face sheets and
    /// its tail's colour.
    pub(super) material_textures: Vec<MaterialTexture>,
    /// The outfit, shown in Blender as the armature's custom properties.
    pub(super) traits: Option<OutfitTraits>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct OutfitTraits {
    pub(super) chara: u32,
    pub(super) dress: u32,
}

/// A texture the game puts in one material's slot at runtime: `mtl_mtail0001_00`'s `_MainTex`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct MaterialTexture {
    pub(super) material: String,
    pub(super) slot: &'static str,
    pub(super) name: String,
}

/// How a chibi or cut-in clip is published, by what it moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ClipKind {
    /// A uma's or chibi's own bones, on this skeleton.
    Skeleton(&'static [&'static str]),
    /// The one object it is bound to, such as a camera or a uma's position.
    Node,
    /// Bones of a prop or set, whose models are published apart: its curves as JSON.
    Curves,
    /// The script driving a face's expressions over a clip, as JSON.
    Script,
}

/// A chibi or cut-in clip: how it is published and whose it is.
#[derive(Debug, PartialEq)]
pub(super) struct GameClip<'a> {
    pub(super) kind: ClipKind,
    pub(super) owner: ClipOwner<'a>,
    /// The game's folder for what the clip moves: `body`, `camera`, `position`, `facial`, `prop`,
    /// `bg` or `belt`.
    pub(super) role: &'a str,
    /// The clip's game name without the prefix its siblings share, `01_cam`; the role when nothing
    /// is left, as for a character cut-in's one body clip.
    pub(super) short_name: String,
}

/// Whose a clip is, which decides its folder.
#[derive(Debug, PartialEq)]
pub(super) enum ClipOwner<'a> {
    /// A cut-in clip, made for the timeline of the same name: `crd100101_001`.
    Cutin { timeline: &'a str },
    /// A chibi clip only this character plays, with its scene and set folders for when the
    /// character has no folder of its own: `event/chara/chr1001_00`.
    Character { chara: u32, folder: String },
    /// A chibi clip any chibi can play, by scene and set below the role: `event/type00`.
    Library { folder: String },
}

/// The prefab families built into one model each.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PrefabFamily {
    Track,
    Effect,
    CutinSet,
    Prop,
}

/// A race clip, named after what it shows.
#[derive(Debug, PartialEq)]
pub(super) struct RaceMotion {
    /// The motion set's folder, `bright`.
    pub(super) set: String,
    /// The file name within the set, `run-fast-standard`.
    pub(super) stem: String,
    /// What Blender names the action: `Bright: Fast run, standard form`.
    pub(super) title: String,
}

/// One part of a racecourse's scene, from a prefab such as `pfb_env_race10006_7001_010`.
pub(super) struct CoursePart<'a> {
    /// The course's folder, `3d/env/race/race10006`.
    pub(super) course: &'a str,
    /// The part's place in `COURSE_PARTS`, which is its place in the scene.
    pub(super) order: usize,
    /// Season, weather and time digits, `010` for sunny at any season and time.
    pub(super) variant: &'a str,
}

/// The bundles a game file is loaded with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Dependencies {
    None,
    /// The texture bundle an atlas's sprites draw from.
    AtlasTexture(&'static str),
    /// Every bundle the game's file list says it needs, directly or through another, as a
    /// track prefab needs its meshes, materials and textures.
    Listed,
}

/// The sprites cut from one atlas and the texture bundle they draw from.
pub(super) struct AtlasPlan {
    pub(super) texture: &'static str,
    pub(super) outputs: Vec<PlannedOutput>,
}

/// What the publisher has put in the bucket, by game file name.
#[derive(Clone, Default, Deserialize)]
pub(super) struct ArtState {
    /// The resource version whose files were all handled.
    pub(super) resource_version: Option<String>,
    pub(super) files: BTreeMap<String, PublishedArt>,
}

/// `index.json` in the state folder, which links every shard like the resources manifest
/// links its artifacts.
#[derive(Serialize)]
pub(super) struct StateIndex {
    pub(super) resource_version: Option<String>,
    pub(super) generated_at: String,
    pub(super) shards: Vec<ShardLink>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct ShardLink {
    pub(super) name: String,
    /// From the bucket root, `/_state/game-art/chara.json`.
    pub(super) path: String,
    pub(super) files: usize,
}

/// `index.json` as read back, in the current layout or the earlier one.
#[derive(Deserialize)]
pub(super) struct StoredIndex {
    pub(super) resource_version: Option<String>,
    pub(super) shards: StoredShards,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum StoredShards {
    Links(Vec<ShardLink>),
    /// Earlier indexes counted records by shard name and sharded characters and tracks apart.
    Counts(BTreeMap<String, usize>),
}

/// One shard file of records.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct StateShard<Record> {
    pub(super) files: BTreeMap<String, Record>,
}

/// Whether the record in the bucket is in the current layout.
#[derive(Debug, PartialEq)]
pub(super) enum StateLayout {
    Current,
    /// Every shard is written again in the current layout on the first real run, and these
    /// keys of the earlier one are removed. Also when records belong to another shard now.
    Outdated {
        obsolete_keys: Vec<String>,
    },
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
    /// Keys the converter named after the file's contents, beside `outputs`: one per shader of the
    /// shader bundle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) content_outputs: Vec<String>,
    /// Textures earlier converters wrote beside a model. Models now embed their textures, so
    /// these are deleted once no record lists them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) textures: Vec<String>,
    /// Set when this exact file could not be converted; it is retried once the file or the
    /// converter changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
    /// The shader files its model's materials use, as links from the bucket root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) shaders: Vec<String>,
    /// MD5 of the published files a cut-in timeline links, so it is rebuilt when one moves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) links: Option<String>,
}

/// What the converter reported about a built file beside its planned outputs.
#[derive(Debug, Default, PartialEq)]
pub(super) struct ConvertedArt {
    /// Keys it named after the file's contents: one per shader of the shader bundle.
    pub(super) content_outputs: Vec<String>,
    /// The shader files a model's materials use.
    pub(super) shaders: Vec<String>,
}

/// A game file to build, the source it comes from and the bucket keys it is served at.
pub(super) struct ArtJob {
    pub(super) source: &'static CdnSource,
    /// The record it is published under: its game file, or for the model combining a
    /// racecourse, the course's folder.
    pub(super) name: String,
    /// The bundle that is converted; a course model's track prefab.
    pub(super) entry: ManifestEntry,
    /// Bundles loaded with it, such as an atlas's texture.
    pub(super) dependencies: Vec<ManifestEntry>,
    /// Bundles loaded only so references into them resolve; unlike `dependencies`, a change to
    /// them does not rebuild the job. A model looks its materials' shader names up in the shader
    /// bundle, and a cut-in timeline names the clips, effects and sets it plays.
    pub(super) lookups: Vec<ManifestEntry>,
    /// The published key of each lookup the publisher builds, by game file, which a timeline
    /// links beside the game name.
    pub(super) links: BTreeMap<String, String>,
    /// What a model draws; `None` for pictures.
    pub(super) model: Option<ArtModel>,
    pub(super) outputs: Vec<PlannedOutput>,
}

/// What a `.glb` or `.json` output holds.
pub(super) enum ArtModel {
    Scenery(SceneryModel),
    /// An outfit's body, head and tail joined on one skeleton in its rest pose, a T-pose.
    Uma(UmaOutfit),
    /// An outfit's chibi or a prop, skinned on its own bones in its rest pose.
    Character(CharacterModel),
    /// The job's clip on a skeleton, without meshes.
    Motion(MotionModel),
    /// The job's clip on the one object it moves, without a skeleton.
    NodeMotion {
        title: String,
    },
    /// The job's clip as JSON curves by bone path hash, for a model published apart.
    ClipCurves,
    /// The scripts of the job's bundle as JSON, references into its `lookups` named.
    Scripts,
    /// Every shader of the job's bundle, each in a file of its own, listed in the job's output.
    Shaders,
}

/// Prefabs drawn side by side in one model, each in its own coordinates.
pub(super) struct SceneryModel {
    pub(super) prefabs: Vec<String>,
    /// Textures the game puts on the prefabs' materials at runtime, one set per season and
    /// weather. They are published on their own and are among the job's `dependencies`, so the
    /// model can embed the first set and link the others.
    pub(super) runtime_textures: Vec<PlacedTexture>,
    /// Whether each object keeps its particle systems and scripts, as effects and cut-in sets do.
    pub(super) components: bool,
}

/// A texture published on its own, by game file and bucket key.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PlacedTexture {
    pub(super) name: String,
    pub(super) key: String,
}

pub(super) struct MotionModel {
    pub(super) title: String,
    /// A skeleton such as `MOTION_SKELETON`, among the job's `dependencies`.
    pub(super) skeleton: Vec<String>,
    /// Whether the clip's curves on the skeleton's root are kept, as a cut-in's are.
    pub(super) moves_root: bool,
}

/// One file built from a game file; its key's extension says whether it is a picture or a
/// `.glb` model.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PlannedOutput {
    pub(super) key: String,
    /// The sprite or texture in the bundle that is drawn; a model's prefabs are in its job.
    pub(super) object: String,
    /// How a picture is fitted; a model keeps `Fit::Native`.
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
    /// Objects it had published that no record lists any more.
    pub deleted: usize,
}

#[derive(Serialize)]
pub(super) struct ConvertJob<'a> {
    pub(super) bundle: String,
    pub(super) dependencies: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) model: Option<ConvertModel<'a>>,
    pub(super) outputs: Vec<ConvertOutput<'a>>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum ConvertModel<'a> {
    Scenery {
        prefabs: Vec<ScenePrefab<'a>>,
        runtime_textures: Vec<RuntimeTexture<'a>>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        components: bool,
    },
    Uma {
        title: &'a str,
        prefabs: Vec<ScenePrefab<'a>>,
        tail_textures: Vec<SlotBundle<'a>>,
        traits: &'a UmaTraits,
    },
    Character {
        title: &'a str,
        prefabs: Vec<ScenePrefab<'a>>,
        material_textures: Vec<MaterialBundle<'a>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        traits: Option<&'a OutfitTraits>,
    },
    /// The clip is the job's own bundle, as for the kinds below.
    Motion {
        title: &'a str,
        skeleton: Vec<ScenePrefab<'a>>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        moves_root: bool,
    },
    NodeMotion {
        title: &'a str,
    },
    ClipCurves,
    /// Every downloaded bundle by game name, so references into them can be named.
    Scripts {
        bundles: Vec<NamedBundle<'a>>,
    },
    Shaders,
}

#[derive(Serialize)]
pub(super) struct ScenePrefab<'a> {
    /// The prefab's root object, named like its file.
    pub(super) object: &'a str,
    /// The downloaded bundle: the job's own or one of its `dependencies`.
    pub(super) bundle: String,
}

#[derive(Serialize)]
pub(super) struct SlotBundle<'a> {
    pub(super) slot: &'a str,
    /// The downloaded texture bundle, one of the job's `dependencies`.
    pub(super) bundle: String,
}

#[derive(Serialize)]
pub(super) struct MaterialBundle<'a> {
    pub(super) material: &'a str,
    pub(super) slot: &'a str,
    /// The downloaded texture bundle, one of the job's `dependencies`.
    pub(super) bundle: String,
}

#[derive(Serialize)]
pub(super) struct NamedBundle<'a> {
    /// The game file, `3d/effect/charaskill/pfb_eff_chr1001_00/pfb_eff_chr1001_00_01`.
    pub(super) name: &'a str,
    pub(super) bundle: String,
    /// Where it is published, relative to the output, when the publisher builds it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) file: Option<String>,
}

#[derive(Serialize)]
pub(super) struct RuntimeTexture<'a> {
    pub(super) name: &'a str,
    pub(super) key: String,
    /// The downloaded bundle, one of the job's `dependencies`.
    pub(super) bundle: String,
}

#[derive(Serialize)]
pub(super) struct ConvertOutput<'a> {
    pub(super) object: &'a str,
    /// The bucket key, which a model's links to its other season textures are relative to.
    pub(super) key: &'a str,
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
    /// Files the converter named after the bundle's contents, beside the planned outputs.
    #[serde(default)]
    pub(super) files: Vec<ConvertedFile>,
    /// The shader files the model's materials use, below the shader folder.
    #[serde(default)]
    pub(super) shaders: Vec<String>,
    pub(super) error: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ConvertedFile {
    pub(super) key: String,
    pub(super) path: String,
}

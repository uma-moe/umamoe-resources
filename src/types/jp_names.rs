//! Shapes of JP's gachas and names, and of the English names the shared store keeps for what
//! only JP has released.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The model that translates names Global has not released.
pub const TRANSLATION_MODEL: &str = "claude-opus-5-5";

/// A pickup gacha as JP's master describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct JpGacha {
    pub gacha_id: i64,
    pub gacha_type: i64,
    /// 1 for characters, 2 for support cards.
    pub card_type: i64,
    pub start_date: DateTime<Utc>,
    pub end_date: DateTime<Utc>,
    /// Rarest first, then by card id.
    pub pickup_card_ids: Vec<i64>,
    /// The cards the gacha recommends (`recommend_order`), in the same order. A Twinkle
    /// Collection names its featured cards only this way, without pickups.
    pub recommended_card_ids: Vec<i64>,
}

/// What a translated name names; each kind is one text category of the masters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameKind {
    /// A character's name, by character id.
    UmaName,
    /// A character card's outfit title in brackets, by card id.
    OutfitTitle,
    /// A support card's title in brackets, by support card id.
    SupportTitle,
}

/// A translation, with the Japanese it was made from so a changed name is translated again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MachineName {
    pub ja: String,
    pub en: String,
}

/// English names for entries Global has not released, by kind and id.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct NameTranslations {
    pub model: String,
    pub names: BTreeMap<NameKind, BTreeMap<i64, MachineName>>,
}

/// Hand-written names that replace a translation until Global ships its own, by kind and id.
pub type NameOverrides = BTreeMap<NameKind, BTreeMap<i64, String>>;

/// Names as one master has them, by kind and id.
pub type MasterNames = BTreeMap<NameKind, BTreeMap<i64, String>>;

/// The English and Japanese names of every character and card, for each resource that names
/// them.
#[derive(Debug, Default)]
pub struct NameBook {
    pub(super) english: MasterNames,
    pub(super) japanese: MasterNames,
    /// The character of each character card and support card, by card id.
    pub(super) card_characters: BTreeMap<i64, i64>,
}

/// A card's character name and title in English and Japanese; titles have no brackets.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CardNames {
    pub name_en: Option<String>,
    pub name_jp: Option<String>,
    pub title_en: Option<String>,
    pub title_jp: Option<String>,
}

/// The names the timeline reads: overrides, then translations.
#[derive(Debug, Default)]
pub struct NameLoc {
    pub(super) translations: BTreeMap<NameKind, BTreeMap<i64, String>>,
    pub(super) overrides: NameOverrides,
}

/// One name sent to the model, with the character it belongs to when that helps.
#[derive(Debug, Clone, Serialize)]
pub(super) struct TranslationItem {
    pub(super) id: String,
    pub(super) kind: NameKind,
    pub(super) ja: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) character: Option<String>,
}

/// A shipped Global name the model copies the style of.
#[derive(Debug, Clone, Serialize)]
pub(super) struct TranslationExample {
    pub(super) kind: NameKind,
    pub(super) ja: String,
    pub(super) en: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) character: Option<String>,
}

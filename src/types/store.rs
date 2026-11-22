//! Shapes of the shared data store: what every server generates from, beside the masters. It
//! lives in a bucket, not in git, and each server keeps a copy.

use std::sync::Arc;
use std::time::Duration;

/// A file of the store. Collected files are fetched from outside sources by the publisher, whose
/// copy is the source of truth. Provided files are static data a maintainer uploads; the bucket's
/// copy is the source of truth for every server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StoreFile {
    /// JP news, gachas and support cards from umapyoi. Collected.
    UmapyoiArchive,
    /// Official English news posts. Collected.
    GlobalNewsArchive,
    /// Official English X posts. Collected.
    GlobalSocialArchive,
    /// English names of what only JP has released. Collected.
    NameTranslations,
    /// Character names by character id.
    CharacterNames,
    /// The JP support card database.
    SupportCardsDb,
    TimelineCharacterBanners,
    TimelineSupportBanners,
    TimelinePaidBanners,
    TimelineStoryEvents,
    TimelineChampionsMeeting,
    TimelineLegendRaces,
    TimelineCampaigns,
    PlannerGachaRates,
    PlannerMasterRewards,
    PlannerMissionRewards,
    /// English mission campaigns whose posts left the news feed.
    EnglishMissionCampaignAssets,
    /// Gzipped course geometry keyframes, stored as-is.
    SimulatorCourseGeometry,
    /// Gzipped course lane data, stored as-is.
    SimulatorCourseLanes,
    /// JP course event tables, packed into one object by course id.
    CourseEventParamsJp,
    /// Global course event tables that replace JP's, packed like the JP ones.
    CourseEventParamsGlobal,
}

/// A store file's content: what this server last loaded, or the copy built into the binary
/// while it has loaded none.
#[derive(Clone)]
pub enum StoreBytes {
    Loaded(Arc<[u8]>),
    Bundled(&'static [u8]),
}

/// The folder the store lives in, in the bucket STORE_S3_BUCKET names or else the asset bucket,
/// so a bucket can hold other things beside it.
pub(super) const BUCKET_FOLDER: &str = "store";

/// The folder of the data directory that holds this server's copy.
pub const LOCAL_FOLDER: &str = "store";

/// Store files change in place, so nothing between the bucket and a server may cache them.
pub(super) const CACHE_CONTROL: &str = "no-store";

/// Delay between umapyoi requests, well below its hourly limit.
pub(super) const UMAPYOI_REQUEST_INTERVAL_MS: u64 = 1000;

/// How long names the model left unanswered wait before they are asked again, so a name it
/// keeps skipping is not paid for on every sync. A new master is translated at once.
pub(super) const TRANSLATION_RETRY: Duration = Duration::from_secs(6 * 60 * 60);

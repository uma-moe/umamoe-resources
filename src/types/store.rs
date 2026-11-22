//! Shapes of the shared data store: what the publishing server collects from outside sources and
//! every server generates from. It lives in a bucket, not in git, and each server keeps a copy.

use std::sync::Arc;
use std::time::Duration;

/// A file of the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StoreFile {
    /// JP news, gachas and support cards from umapyoi.
    UmapyoiArchive,
    /// Official English news posts.
    GlobalNewsArchive,
    /// Official English X posts.
    GlobalSocialArchive,
    /// English names of what only JP has released.
    NameTranslations,
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

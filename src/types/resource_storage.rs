use std::time::Duration;

/// The folder of the data directory that holds every artifact once, as `<sha256>.json.gz`.
pub const BLOB_FOLDER: &str = "blobs";

/// The folder of the data directory that holds every Global master snapshot once, as
/// `<sha256>.mdb` and `<sha256>.mdb.gz`.
pub const MASTER_FOLDER: &str = "masters";

/// How old an unreferenced blob or master must be before pruning removes it. The generation lock
/// holds only inside one process: a one-shot command or another server on the same data directory
/// may have written a file (or its `.next` temporary) whose manifest is not on disk yet.
pub const MIN_UNREFERENCED_AGE: Duration = Duration::from_secs(60 * 60);

/// How many versions of each set stay on disk after a generation, the current one included.
pub const KEPT_VERSIONS: usize = 10;

/// Folders of a data directory that are never a version, whatever they contain. `masterdata` is
/// a volume of its own in production.
pub const RESERVED_FOLDERS: [&str; 6] = [
    BLOB_FOLDER,
    MASTER_FOLDER,
    "masterdata",
    "store",
    "planner",
    "jp",
];

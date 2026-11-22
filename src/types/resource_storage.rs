/// The folder of the data directory that holds every artifact once, as `<sha256>.json.gz`.
pub const BLOB_FOLDER: &str = "blobs";

/// The folder of the data directory that holds every Global master snapshot once, as
/// `<sha256>.mdb` and `<sha256>.mdb.gz`.
pub const MASTER_FOLDER: &str = "masters";

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

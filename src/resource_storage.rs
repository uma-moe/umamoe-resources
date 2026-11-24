//! Where generated files live on disk. Every artifact of every set is stored once in `blobs/`,
//! named by the SHA-256 of its JSON, and every Global master snapshot once in `masters/`; a
//! version folder holds only its manifest. Versions written before this layout keep their files
//! inside the version folder, so every read falls back to that path.

#![allow(non_snake_case)]

#[path = "types/resource_storage.rs"]
mod types;

use crate::pipeline::{
    jp_data_dir, read_manifest, writeAtomic, ResourceArtifact, ResourceManifest,
};
use anyhow::{Context, Result};
use chrono::DateTime;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::{info, warn};
use types::{BLOB_FOLDER, KEPT_VERSIONS, MASTER_FOLDER, MIN_UNREFERENCED_AGE, RESERVED_FOLDERS};

/// Where an artifact with this JSON hash is stored.
pub fn blobFile(data_dir: &Path, sha256: &str) -> PathBuf {
    data_dir.join(BLOB_FOLDER).join(format!("{sha256}.json.gz"))
}

/// Where a Global master snapshot with this hash is stored, gzipped or not.
pub fn masterFile(data_dir: &Path, sha256: &str, compressed: bool) -> PathBuf {
    let extension = if compressed { "mdb.gz" } else { "mdb" };
    data_dir
        .join(MASTER_FOLDER)
        .join(format!("{sha256}.{extension}"))
}

/// Stores an artifact unless the same JSON is stored already.
pub fn writeBlob(data_dir: &Path, sha256: &str, gzip_bytes: &[u8]) -> Result<()> {
    let path = blobFile(data_dir, sha256);
    if path.is_file() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    writeAtomic(&path, gzip_bytes).with_context(|| format!("failed to write {}", path.display()))
}

/// The gzip file of an artifact of the set in `set_dir`, from `blobs/` or, for a version in the
/// old layout, from its version folder.
pub fn artifactPath(
    data_dir: &Path,
    set_dir: &Path,
    version: &str,
    artifact: &ResourceArtifact,
) -> PathBuf {
    if isSha256(&artifact.sha256) {
        let blob = blobFile(data_dir, &artifact.sha256);
        if blob.is_file() {
            return blob;
        }
    }
    set_dir.join(version).join(format!("{}.gz", artifact.name))
}

/// The Global master a version was generated from, from `masters/` or, for a version in the old
/// layout, from its version folder.
pub fn masterPath(data_dir: &Path, version: &str, sha256: &str, compressed: bool) -> PathBuf {
    if isSha256(sha256) {
        let snapshot = masterFile(data_dir, sha256, compressed);
        if snapshot.is_file() {
            return snapshot;
        }
    }
    let file_name = if compressed {
        "master.mdb.gz"
    } else {
        "master.mdb"
    };
    data_dir.join(version).join(file_name)
}

/// Removes the versions of the sets in `set_dirs` beyond the newest few, then every stored file
/// no remaining manifest of any set refers to and that is older than `MIN_UNREFERENCED_AGE`.
/// Called under the generation lock, so this process writes nothing meanwhile; the age keeps what
/// another process may have just written. Retention is housekeeping, so a failure is logged and
/// never fails the generation; a manifest that cannot be read stops the step that needs it from
/// deleting.
pub fn pruneVersions(data_dir: &Path, set_dirs: &[&Path]) {
    pruneVersionsAt(data_dir, set_dirs, SystemTime::now());
}

/// `pruneVersions` with the time the age of stored files is measured from.
fn pruneVersionsAt(data_dir: &Path, set_dirs: &[&Path], now: SystemTime) {
    for set_dir in set_dirs {
        if let Err(error) = pruneSetVersions(set_dir) {
            warn!(set = %set_dir.display(), error = %error, "skipped pruning old versions");
        }
    }
    if let Err(error) = pruneUnreferencedFiles(data_dir, now) {
        warn!(error = %error, "skipped pruning unreferenced stored files");
    }
}

/// Keeps the current version and the newest others up to `KEPT_VERSIONS`, by generation time.
fn pruneSetVersions(set_dir: &Path) -> Result<()> {
    let current = read_manifest(set_dir)?;
    let mut others = Vec::new();
    for (name, manifest) in versionManifests(set_dir)? {
        if name == current.version {
            continue;
        }
        let generated_at = DateTime::parse_from_rfc3339(&manifest.generated_at)
            .with_context(|| format!("version {name} has no valid generation time"))?;
        others.push((generated_at, name));
    }
    others.sort_by(|left, right| right.0.cmp(&left.0));

    for (_, name) in others.iter().skip(KEPT_VERSIONS.saturating_sub(1)) {
        let folder = set_dir.join(name);
        match fs::remove_dir_all(&folder) {
            Ok(()) => info!(version = %folder.display(), "removed old resource version"),
            Err(error) => {
                warn!(version = %folder.display(), error = %error, "failed to remove old resource version")
            }
        }
    }
    Ok(())
}

/// Deletes the blobs and master snapshots that no manifest of the three sets lists, unless they
/// were written recently, since another process may be about to list them.
fn pruneUnreferencedFiles(data_dir: &Path, now: SystemTime) -> Result<()> {
    let mut blobs = HashSet::new();
    let mut masters = HashSet::new();
    for set_dir in [
        data_dir.to_path_buf(),
        data_dir.join("planner"),
        jp_data_dir(data_dir),
    ] {
        for manifest in setManifests(&set_dir)? {
            blobs.extend(
                manifest
                    .artifacts
                    .iter()
                    .map(|artifact| format!("{}.json.gz", artifact.sha256)),
            );
            masters.insert(format!("{}.mdb", manifest.master.sha256));
            masters.insert(format!("{}.mdb.gz", manifest.master.sha256));
            if let Some(preliminary) = &manifest.preliminary_master {
                masters.insert(format!("{}.mdb", preliminary.sha256));
                masters.insert(format!("{}.mdb.gz", preliminary.sha256));
            }
        }
    }

    removeUnlisted(&data_dir.join(BLOB_FOLDER), &blobs, now)?;
    removeUnlisted(&data_dir.join(MASTER_FOLDER), &masters, now)
}

/// The current manifest of a set, when it has one, and the manifest of each of its versions.
fn setManifests(set_dir: &Path) -> Result<Vec<ResourceManifest>> {
    let mut manifests: Vec<ResourceManifest> = versionManifests(set_dir)?
        .into_iter()
        .map(|(_, manifest)| manifest)
        .collect();
    if set_dir.join("manifest.json").exists() {
        manifests.push(read_manifest(set_dir)?);
    }
    Ok(manifests)
}

/// Each version of a set with its manifest. A version is a folder directly inside the set's
/// folder that holds a `manifest.json` and is not a reserved folder; nothing else ever is.
fn versionManifests(set_dir: &Path) -> Result<Vec<(String, ResourceManifest)>> {
    let entries = match fs::read_dir(set_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to list {}", set_dir.display()))
        }
    };

    let mut versions = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to list {}", set_dir.display()))?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if RESERVED_FOLDERS.contains(&name.as_str())
            || !entry.path().join("manifest.json").is_file()
        {
            continue;
        }
        let manifest = read_manifest(&entry.path())?;
        versions.push((name, manifest));
    }
    Ok(versions)
}

/// Removes the files of `folder` whose names are not in `kept` and that are old enough.
fn removeUnlisted(folder: &Path, kept: &HashSet<String>, now: SystemTime) -> Result<()> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to list {}", folder.display()))
        }
    };

    let mut removed = 0;
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to list {}", folder.display()))?;
        let listed = entry
            .file_name()
            .to_str()
            .is_some_and(|name| kept.contains(name));
        if listed || !entry.file_type()?.is_file() || !isOldEnough(&entry, now) {
            continue;
        }
        match fs::remove_file(entry.path()) {
            Ok(()) => removed += 1,
            Err(error) => {
                warn!(file = %entry.path().display(), error = %error, "failed to remove unreferenced file")
            }
        }
    }
    if removed > 0 {
        info!(folder = %folder.display(), removed, "removed unreferenced stored files");
    }
    Ok(())
}

/// Whether a file was last written at least `MIN_UNREFERENCED_AGE` before `now`. A time that
/// cannot be read, or one after `now`, keeps the file.
fn isOldEnough(entry: &fs::DirEntry, now: SystemTime) -> bool {
    entry
        .metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age >= MIN_UNREFERENCED_AGE)
}

/// File names are built from manifest hashes, so anything else never becomes a path.
fn isSha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::{blobFile, masterFile, pruneVersionsAt, MIN_UNREFERENCED_AGE};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    /// A time from which every file written by the test is old enough to be removed.
    fn later() -> SystemTime {
        SystemTime::now() + MIN_UNREFERENCED_AGE + Duration::from_secs(60)
    }

    fn temporaryDirectory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "resource-storage-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    /// A hash made of one repeated hex digit, so each version's files are told apart.
    fn hash(digit: usize) -> String {
        format!("{digit:x}").repeat(64)
    }

    fn manifestJson(version: &str, second: usize, digit: usize) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "version": version,
            "generated_at": format!("2026-11-01T00:00:{second:02}+00:00"),
            "base_path": format!("/resources/en/{version}"),
            "master": { "marker": "test", "sha256": hash(digit) },
            "artifacts": [{
                "name": "skills.json",
                "path": "",
                "current_path": "",
                "content_type": "application/json; charset=utf-8",
                "content_encoding": "gzip",
                "etag": "",
                "sha256": hash(digit),
                "json_bytes": 2,
                "gzip_bytes": 2
            }]
        }))
        .unwrap()
    }

    fn writeVersion(set_dir: &Path, version: &str, second: usize, digit: usize) {
        fs::create_dir_all(set_dir.join(version)).unwrap();
        fs::write(
            set_dir.join(version).join("manifest.json"),
            manifestJson(version, second, digit),
        )
        .unwrap();
    }

    /// Twelve Global versions v00 to v11 with their own blob and master each, the current one
    /// being v03; a planner and a JP set with one version each; stray stored files; and folders
    /// that are never versions.
    fn dataDirectory(name: &str) -> PathBuf {
        let root = temporaryDirectory(name);
        for index in 0..12 {
            writeVersion(&root, &format!("v{index:02}"), index, index);
        }
        fs::write(root.join("manifest.json"), manifestJson("v03", 3, 3)).unwrap();
        writeVersion(&root.join("planner"), "v11", 11, 12);
        fs::write(
            root.join("planner/manifest.json"),
            manifestJson("v11", 11, 12),
        )
        .unwrap();
        writeVersion(&root.join("jp"), "j1", 0, 13);
        fs::write(root.join("jp/manifest.json"), manifestJson("j1", 0, 13)).unwrap();

        // Old enough to be pruned, and unreadable, if either were taken for a version.
        writeVersion(&root, "store", 0, 0);
        fs::create_dir_all(root.join("masterdata")).unwrap();
        fs::write(root.join("masterdata/manifest.json"), b"{").unwrap();
        fs::create_dir_all(root.join("notes")).unwrap();

        fs::create_dir_all(root.join("blobs")).unwrap();
        fs::create_dir_all(root.join("masters")).unwrap();
        for digit in 0..15 {
            fs::write(blobFile(&root, &hash(digit)), b"gz").unwrap();
            fs::write(masterFile(&root, &hash(digit), false), b"mdb").unwrap();
            fs::write(masterFile(&root, &hash(digit), true), b"gz").unwrap();
        }
        root
    }

    fn names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn pruningKeepsTheNewestVersionsAndTheCurrentOneAndTheFilesTheyUse() {
        let root = dataDirectory("prune");

        pruneVersionsAt(&root, &[&root, &root.join("planner")], later());

        let mut expected: Vec<String> = [2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
            .iter()
            .map(|index| format!("v{index:02}"))
            .collect();
        expected.extend(
            [
                "blobs",
                "jp",
                "manifest.json",
                "masterdata",
                "masters",
                "notes",
            ]
            .map(String::from),
        );
        expected.extend(["planner", "store"].map(String::from));
        expected.sort();
        assert_eq!(names(&root), expected);
        assert_eq!(names(&root.join("planner")), ["manifest.json", "v11"]);
        assert_eq!(names(&root.join("jp")), ["j1", "manifest.json"]);

        // v00 and v01 were the only users of digits 0 and 1; nothing ever used digit 14.
        let kept: Vec<String> = (2..14).map(hash).collect();
        let mut blobs: Vec<String> = kept.iter().map(|hash| format!("{hash}.json.gz")).collect();
        blobs.sort();
        assert_eq!(names(&root.join("blobs")), blobs);
        let mut masters: Vec<String> = kept
            .iter()
            .flat_map(|hash| [format!("{hash}.mdb"), format!("{hash}.mdb.gz")])
            .collect();
        masters.sort();
        assert_eq!(names(&root.join("masters")), masters);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pruningDeletesNothingWhenAManifestCannotBeRead() {
        let root = dataDirectory("unreadable");
        fs::write(root.join("v05/manifest.json"), b"{").unwrap();
        let before = [
            names(&root),
            names(&root.join("blobs")),
            names(&root.join("masters")),
        ];

        pruneVersionsAt(&root, &[&root, &root.join("planner")], later());

        let after = [
            names(&root),
            names(&root.join("blobs")),
            names(&root.join("masters")),
        ];
        assert_eq!(before, after);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recentlyWrittenUnreferencedFilesSurviveUntilTheyAreOldEnough() {
        let root = temporaryDirectory("recent");
        writeVersion(&root, "v1", 0, 1);
        fs::write(root.join("manifest.json"), manifestJson("v1", 0, 1)).unwrap();
        fs::create_dir_all(root.join("blobs")).unwrap();
        fs::write(blobFile(&root, &hash(1)), b"gz").unwrap();
        // Another process's blob, and its temporary, whose manifest is not written yet.
        fs::write(blobFile(&root, &hash(2)), b"gz").unwrap();
        fs::write(
            root.join("blobs").join(format!("{}.json.next", hash(3))),
            b"gz",
        )
        .unwrap();

        pruneVersionsAt(&root, &[&root], SystemTime::now());
        assert_eq!(names(&root.join("blobs")).len(), 3);

        pruneVersionsAt(&root, &[&root], later());
        assert_eq!(names(&root.join("blobs")), [format!("{}.json.gz", hash(1))]);
        fs::remove_dir_all(root).unwrap();
    }
}

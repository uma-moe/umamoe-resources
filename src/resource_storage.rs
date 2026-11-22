//! Where generated files live on disk. Every artifact of every set is stored once in `blobs/`,
//! named by the SHA-256 of its JSON, and every Global master snapshot once in `masters/`; a
//! version folder holds only its manifest. Versions written before this layout keep their files
//! inside the version folder, so every read falls back to that path.

#![allow(non_snake_case)]

#[path = "types/resource_storage.rs"]
mod types;

use crate::pipeline::{writeAtomic, ResourceArtifact};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use types::{BLOB_FOLDER, MASTER_FOLDER};

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

/// File names are built from manifest hashes, so anything else never becomes a path.
fn isSha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

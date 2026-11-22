//! The maintainer's store commands: fetch what the bucket holds, or upload a provided file.

use super::types::{StoreFile, LOCAL_FOLDER};
use super::{bucketFromEnv, fetchFile, upload};
use crate::assets::Bucket;
use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use std::io::Read;
use std::path::Path;

/// Fetches every file the bucket holds a different version of into the store folder of
/// `data_dir`, and prints the files it fetched.
pub async fn pullCommand(data_dir: &Path) -> Result<()> {
    let bucket = requireBucket()?;
    let client = reqwest::Client::new();
    let dir = data_dir.join(LOCAL_FOLDER);
    let mut fetched = 0;
    for file in StoreFile::ALL {
        if fetchFile(&bucket, &client, &dir, file).await? {
            println!("fetched {}", file.fileName());
            fetched += 1;
        }
    }
    println!(
        "{fetched} of {} store files fetched into {}; the others are current or not in the bucket",
        StoreFile::ALL.len(),
        dir.display()
    );
    Ok(())
}

/// Uploads the file at `path` as the provided store file `name`, once it parses.
pub async fn putCommand(name: &str, path: &Path) -> Result<()> {
    let file = providedFile(name)?;
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    checkParses(file, &bytes)?;
    let bucket = requireBucket()?;
    let size = bytes.len();
    upload(&bucket, &reqwest::Client::new(), file, bytes).await?;
    println!("uploaded {} as {name} ({size} bytes)", path.display());
    Ok(())
}

fn requireBucket() -> Result<Bucket> {
    bucketFromEnv().context(
        "the store's bucket is not configured: set ASSETS_S3_ENDPOINT, ASSETS_S3_ACCESS_KEY_ID and ASSETS_S3_SECRET_ACCESS_KEY",
    )
}

/// The provided file `name` names. Collected files are refused: the publisher would overwrite
/// an upload with its own copy.
fn providedFile(name: &str) -> Result<StoreFile> {
    let Some(file) = StoreFile::fromFileName(name) else {
        let names = StoreFile::ALL
            .into_iter()
            .filter(|file| file.isProvided())
            .map(StoreFile::fileName)
            .collect::<Vec<_>>()
            .join(", ");
        bail!("{name} is not a store file; the provided files are: {names}");
    };
    if !file.isProvided() {
        bail!("{name} is collected by the publishing server, which would overwrite an upload");
    }
    Ok(file)
}

/// Refuses bytes that are not JSON, or for a `.json.gz` file not gzipped JSON.
pub(super) fn checkParses(file: StoreFile, bytes: &[u8]) -> Result<()> {
    let name = file.fileName();
    let mut unpacked = Vec::new();
    let json = match name.ends_with(".gz") {
        true => {
            GzDecoder::new(bytes)
                .read_to_end(&mut unpacked)
                .with_context(|| format!("{name} must be gzipped"))?;
            &unpacked
        }
        false => bytes,
    };
    serde_json::from_slice::<serde::de::IgnoredAny>(json)
        .with_context(|| format!("{name} must hold JSON"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onlyProvidedFilesCanBePut() {
        assert!(providedFile("timeline_campaigns.json").is_ok());
        assert!(providedFile("umapyoi_archive.json").is_err());
        assert!(providedFile("master.mdb").is_err());
    }

    #[test]
    fn putRefusesWhatDoesNotParse() {
        let json = StoreFile::TimelineCampaigns;
        assert!(checkParses(json, b"[]").is_ok());
        assert!(checkParses(json, b"[1,").is_err());

        let gzipped = StoreFile::SimulatorCourseLanes;
        assert!(checkParses(gzipped, &crate::pipeline::gzip(b"{}").unwrap()).is_ok());
        assert!(checkParses(gzipped, b"{}").is_err());
        assert!(checkParses(gzipped, &crate::pipeline::gzip(b"{").unwrap()).is_err());
    }
}

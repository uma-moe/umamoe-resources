//! Fitting downloaded pictures to the size the timeline shows, with
//! `scripts/timeline_art_convert.py`.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;

const CONVERTER_PYTHON: &str = "/opt/game-art/bin/python";
const CONVERTER_SCRIPT: &str = "/app/scripts/timeline_art_convert.py";

#[derive(Clone, Debug, Serialize)]
pub(super) struct ConvertJob {
    pub(super) source: PathBuf,
    pub(super) output: PathBuf,
    #[serde(flatten)]
    pub(super) fit: PictureFit,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "fit", rename_all = "snake_case")]
pub(super) enum PictureFit {
    /// Japanese art, at most 512 px wide.
    Japanese,
    /// English art fitted into the size of the banner's Japanese art: `target` when known, else
    /// the size of `reference`, Japanese art when `reference_is_japanese`.
    English {
        target: Option<(u32, u32)>,
        reference: Option<PathBuf>,
        reference_is_japanese: bool,
    },
}

#[derive(Debug, Deserialize)]
pub(super) struct ConvertResult {
    pub(super) output: PathBuf,
    #[serde(default)]
    pub(super) size: Option<(u32, u32)>,
    #[serde(default)]
    pub(super) error: Option<String>,
}

#[derive(Serialize)]
struct ConvertRequest<'a> {
    jobs: &'a [ConvertJob],
}

#[derive(Deserialize)]
struct ConvertResponse {
    results: Vec<ConvertResult>,
}

/// Converts the jobs in one process at the lowest CPU priority, since the server's cores also
/// serve the site.
pub(super) async fn convertPictures(jobs: &[ConvertJob]) -> Result<Vec<ConvertResult>> {
    if jobs.is_empty() {
        return Ok(Vec::new());
    }

    let mut child = tokio::process::Command::new("nice")
        .args(["-n", "19", CONVERTER_PYTHON, CONVERTER_SCRIPT])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start the timeline art converter")?;

    let request = serde_json::to_vec(&ConvertRequest { jobs })?;
    let mut stdin = child.stdin.take().context("the converter has no stdin")?;
    stdin.write_all(&request).await?;
    drop(stdin);

    let output = child.wait_with_output().await?;
    if !output.status.success() {
        bail!(
            "the timeline art converter failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let response: ConvertResponse = serde_json::from_slice(&output.stdout)
        .context("the timeline art converter printed no valid JSON")?;
    Ok(response.results)
}

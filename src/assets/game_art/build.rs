use super::catalog::objectName;
use super::types::{
    ArtJob, ConvertJob, ConvertOutput, ConvertRequest, ConvertResponse, CONVERTER_PYTHON,
    CONVERTER_SCRIPT, DOWNLOAD_CONCURRENCY,
};
use crate::game_cdn::{self, CdnSource, ManifestEntry};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::task::JoinSet;

/// Scratch space for decrypted bundles and converted images. Nothing in it outlives a run.
pub(super) struct WorkDir {
    root: PathBuf,
}

impl WorkDir {
    pub(super) fn create() -> Result<Self> {
        let root = std::env::temp_dir().join("umamoe-game-art");
        // A run that crashed may have left files behind.
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)
            .with_context(|| format!("failed to create {}", root.display()))?;
        Ok(Self { root })
    }

    /// An empty folder for one batch; the previous batch's files are removed.
    fn batchDir(&self) -> Result<PathBuf> {
        let dir = self.root.join("batch");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
        Ok(dir)
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(super) enum JobOutcome {
    /// Converted images as (bucket key, local file).
    Built(Vec<(String, PathBuf)>),
    /// The bundle downloaded but does not convert.
    Unconvertible(String),
    /// The bundle could not be fetched; worth retrying on the next run.
    NotDownloaded(String),
}

/// Downloads, decrypts and converts one batch, reporting an outcome per job in order.
pub(super) async fn buildBatch(
    client: &reqwest::Client,
    work: &WorkDir,
    jobs: &[ArtJob],
    base_key: &Arc<Vec<u8>>,
) -> Result<Vec<JobOutcome>> {
    let dir = work.batchDir()?;
    let bundles = downloadBundles(client, &dir, jobs, base_key).await;

    let mut outcomes: Vec<Option<JobOutcome>> = (0..jobs.len()).map(|_| None).collect();
    let mut conversions: Vec<(usize, ConvertJob)> = Vec::new();
    for (index, (job, bundle)) in jobs.iter().zip(bundles).enumerate() {
        match bundle {
            Ok(path) => conversions.push((index, convertJob(job, &path, &dir, index))),
            Err(error) => outcomes[index] = Some(JobOutcome::NotDownloaded(format!("{error:#}"))),
        }
    }

    for (index, error) in convertAll(conversions).await? {
        outcomes[index] = Some(match error {
            Some(error) => JobOutcome::Unconvertible(error),
            None => JobOutcome::Built(outputFiles(&jobs[index], &dir, index)),
        });
    }

    let outcomes = outcomes
        .into_iter()
        .map(|outcome| {
            outcome.unwrap_or_else(|| {
                JobOutcome::Unconvertible("the converter did not report this bundle".to_string())
            })
        })
        .collect();
    Ok(outcomes)
}

async fn downloadBundles(
    client: &reqwest::Client,
    dir: &Path,
    jobs: &[ArtJob],
    base_key: &Arc<Vec<u8>>,
) -> Vec<Result<PathBuf>> {
    let mut results: Vec<Option<Result<PathBuf>>> = (0..jobs.len()).map(|_| None).collect();
    let mut tasks = JoinSet::new();
    let mut next = 0;

    while next < jobs.len() || !tasks.is_empty() {
        while tasks.len() < DOWNLOAD_CONCURRENCY && next < jobs.len() {
            let job = &jobs[next];
            tasks.spawn(fetchBundle(
                client.clone(),
                job.source,
                job.entry.clone(),
                base_key.clone(),
                dir.join(format!("{next}.bundle")),
                next,
            ));
            next += 1;
        }
        if let Some(Ok((index, result))) = tasks.join_next().await {
            results[index] = Some(result);
        }
    }

    results
        .into_iter()
        .map(|result| result.unwrap_or_else(|| bail!("the download task stopped unexpectedly")))
        .collect()
}

async fn fetchBundle(
    client: reqwest::Client,
    source: &'static CdnSource,
    entry: ManifestEntry,
    base_key: Arc<Vec<u8>>,
    path: PathBuf,
    index: usize,
) -> (usize, Result<PathBuf>) {
    let result = async {
        let mut bundle = game_cdn::downloadAssetBundle(&client, source, &entry).await?;
        game_cdn::decryptAssetBundle(&mut bundle, &base_key, entry.key);
        tokio::fs::write(&path, bundle)
            .await
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(path)
    }
    .await;
    (index, result)
}

fn outputFiles(job: &ArtJob, dir: &Path, index: usize) -> Vec<(String, PathBuf)> {
    job.outputs
        .iter()
        .enumerate()
        .map(|(position, output)| {
            let file = dir.join(format!("{index}-{position}.webp"));
            (output.key.clone(), file)
        })
        .collect()
}

fn convertJob<'a>(job: &'a ArtJob, bundle: &Path, dir: &Path, index: usize) -> ConvertJob<'a> {
    let outputs = job
        .outputs
        .iter()
        .zip(outputFiles(job, dir, index))
        .map(|(output, (_, file))| ConvertOutput {
            object: objectName(&job.entry.name),
            path: file.to_string_lossy().into_owned(),
            format: "webp",
            size: [output.size.0, output.size.1],
            art_size: output.art_size.map(|(width, height)| [width, height]),
        })
        .collect();
    ConvertJob {
        bundle: bundle.to_string_lossy().into_owned(),
        outputs,
    }
}

/// Converts the bundles in one converter process. If the process itself fails, each bundle gets
/// its own process, so one bundle that crashes the decoder cannot hold back the others.
async fn convertAll(
    conversions: Vec<(usize, ConvertJob<'_>)>,
) -> Result<Vec<(usize, Option<String>)>> {
    if conversions.is_empty() {
        return Ok(Vec::new());
    }

    let indexes: HashMap<String, usize> = conversions
        .iter()
        .map(|(index, job)| (job.bundle.clone(), *index))
        .collect();
    let (positions, jobs): (Vec<usize>, Vec<ConvertJob>) = conversions.into_iter().unzip();
    let request = ConvertRequest { jobs };

    match runConverter(&request).await {
        Ok(response) => Ok(response
            .results
            .into_iter()
            .filter_map(|result| {
                let index = *indexes.get(&result.bundle)?;
                Some((index, result.error))
            })
            .collect()),
        Err(error) if request.jobs.len() == 1 => {
            Ok(vec![(positions[0], Some(format!("{error:#}")))])
        }
        Err(_) => {
            let mut results = Vec::with_capacity(request.jobs.len());
            for (index, job) in positions.into_iter().zip(request.jobs) {
                let single = ConvertRequest { jobs: vec![job] };
                let error = match runConverter(&single).await {
                    Ok(response) => response
                        .results
                        .into_iter()
                        .next()
                        .and_then(|result| result.error),
                    Err(error) => Some(format!("{error:#}")),
                };
                results.push((index, error));
            }
            Ok(results)
        }
    }
}

async fn runConverter(request: &ConvertRequest<'_>) -> Result<ConvertResponse> {
    let input = serde_json::to_vec(request)?;
    let mut child = tokio::process::Command::new(CONVERTER_PYTHON)
        .arg(CONVERTER_SCRIPT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("failed to start the game art converter")?;

    // The converter reads all of stdin before it writes anything, so this cannot deadlock.
    let mut stdin = child.stdin.take().context("converter stdin is not piped")?;
    stdin.write_all(&input).await?;
    drop(stdin);

    let output = child.wait_with_output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.lines().last().unwrap_or("no output");
        bail!("the converter exited with {}: {reason}", output.status);
    }
    serde_json::from_slice(&output.stdout).context("the converter printed invalid JSON")
}

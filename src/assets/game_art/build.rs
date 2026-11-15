use super::catalog::{modelTextureFolder, trackTextureKey};
use super::types::{
    ArtJob, ConvertJob, ConvertOutput, ConvertRequest, ConvertResponse, ConvertResult, Fit,
    RuntimeTexture, WrittenTexture, CONVERTER_PYTHON, CONVERTER_SCRIPT, DOWNLOAD_CONCURRENCY,
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
    /// Converted files as (bucket key, local file), and the textures a model wrote beside them.
    Built {
        outputs: Vec<(String, PathBuf)>,
        textures: Vec<(String, PathBuf)>,
    },
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
    for (index, (job, files)) in jobs.iter().zip(bundles).enumerate() {
        match files {
            Ok(files) => conversions.push((index, convertJob(job, &files, &dir, index))),
            Err(error) => outcomes[index] = Some(JobOutcome::NotDownloaded(format!("{error:#}"))),
        }
    }

    for (index, conversion) in convertAll(conversions).await? {
        outcomes[index] = Some(match conversion {
            Err(error) => JobOutcome::Unconvertible(error),
            Ok(textures) => JobOutcome::Built {
                outputs: outputFiles(&jobs[index], &dir, index),
                textures: textures
                    .into_iter()
                    .map(|texture| (texture.key, texture.path))
                    .collect(),
            },
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

/// Each job's bundle followed by the bundles it needs beside it.
fn jobBundles(job: &ArtJob) -> impl Iterator<Item = &ManifestEntry> {
    std::iter::once(&job.entry).chain(&job.dependencies)
}

/// Downloads every bundle of every job; a job's files come back in `jobBundles` order.
async fn downloadBundles(
    client: &reqwest::Client,
    dir: &Path,
    jobs: &[ArtJob],
    base_key: &Arc<Vec<u8>>,
) -> Vec<Result<Vec<PathBuf>>> {
    let mut files: Vec<Vec<Option<Result<PathBuf>>>> = jobs
        .iter()
        .map(|job| jobBundles(job).map(|_| None).collect())
        .collect();
    // Collected first: an iterator of borrowing closures held across `await` is not `Send`.
    let downloads: Vec<(usize, usize, &'static CdnSource, ManifestEntry)> = jobs
        .iter()
        .enumerate()
        .flat_map(|(job_index, job)| {
            jobBundles(job)
                .enumerate()
                .map(move |(position, entry)| (job_index, position, job.source, entry.clone()))
        })
        .collect();
    let mut pending = downloads.into_iter();
    let mut tasks = JoinSet::new();

    loop {
        while tasks.len() < DOWNLOAD_CONCURRENCY {
            let Some((job_index, position, source, entry)) = pending.next() else {
                break;
            };
            tasks.spawn(fetchBundle(
                client.clone(),
                source,
                entry,
                base_key.clone(),
                dir.join(format!("bundle-{job_index}-{position}")),
                (job_index, position),
            ));
        }

        let Some(joined) = tasks.join_next().await else {
            break;
        };
        if let Ok(((job_index, position), result)) = joined {
            files[job_index][position] = Some(result);
        }
    }

    files
        .into_iter()
        .map(|bundles| {
            bundles
                .into_iter()
                .map(|bundle| {
                    bundle.unwrap_or_else(|| bail!("the download task stopped unexpectedly"))
                })
                .collect()
        })
        .collect()
}

async fn fetchBundle(
    client: reqwest::Client,
    source: &'static CdnSource,
    entry: ManifestEntry,
    base_key: Arc<Vec<u8>>,
    path: PathBuf,
    slot: (usize, usize),
) -> ((usize, usize), Result<PathBuf>) {
    let result = async {
        let mut bundle = game_cdn::downloadAssetBundle(&client, source, &entry).await?;
        game_cdn::decryptAssetBundle(&mut bundle, &base_key, entry.key);
        tokio::fs::write(&path, bundle)
            .await
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(path)
    }
    .await;
    (slot, result)
}

fn outputFiles(job: &ArtJob, dir: &Path, index: usize) -> Vec<(String, PathBuf)> {
    job.outputs
        .iter()
        .enumerate()
        .map(|(position, output)| {
            let file = dir.join(format!("{index}-{position}.{}", outputFormat(&output.key)));
            (output.key.clone(), file)
        })
        .collect()
}

/// Track prefabs become `.glb` models and the site keeps a few PNG paths; every other picture
/// is WebP.
fn outputFormat(key: &str) -> &'static str {
    if key.ends_with(".glb") {
        "glb"
    } else if key.ends_with(".png") {
        "png"
    } else {
        "webp"
    }
}

fn convertJob<'a>(job: &'a ArtJob, files: &[PathBuf], dir: &Path, index: usize) -> ConvertJob<'a> {
    let outputs = job
        .outputs
        .iter()
        .zip(outputFiles(job, dir, index))
        .map(|(output, (key, file))| {
            let (size, art_size) = match output.fit {
                Fit::Native => (None, None),
                Fit::Resize(width, height) => (Some([width, height]), None),
                Fit::Centre { size, art } => (Some([size.0, size.1]), Some([art.0, art.1])),
            };
            ConvertOutput {
                object: &output.object,
                key: &output.key,
                path: file.to_string_lossy().into_owned(),
                format: outputFormat(&key),
                size,
                art_size,
            }
        })
        .collect();

    let builds_model = job
        .outputs
        .iter()
        .any(|output| outputFormat(&output.key) == "glb");
    let texture_folders = if builds_model {
        jobBundles(job)
            .map(|entry| modelTextureFolder(&entry.name))
            .collect()
    } else {
        Vec::new()
    };

    let runtime_textures = job
        .runtime_textures
        .iter()
        .map(|name| RuntimeTexture {
            name,
            key: trackTextureKey(name),
        })
        .collect();

    let path = |file: &PathBuf| file.to_string_lossy().into_owned();
    ConvertJob {
        bundle: path(&files[0]),
        dependencies: files[1..].iter().map(path).collect(),
        texture_folders,
        runtime_textures,
        outputs,
    }
}

/// The textures a converted job wrote, or why it did not convert.
type Conversion = std::result::Result<Vec<WrittenTexture>, String>;

fn conversion(result: ConvertResult) -> Conversion {
    match result.error {
        Some(error) => Err(error),
        None => Ok(result.textures),
    }
}

/// Converts the bundles in one converter process. If the process itself fails, each bundle gets
/// its own process, so one bundle that crashes the decoder cannot hold back the others.
async fn convertAll(conversions: Vec<(usize, ConvertJob<'_>)>) -> Result<Vec<(usize, Conversion)>> {
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
                Some((index, conversion(result)))
            })
            .collect()),
        Err(error) if request.jobs.len() == 1 => {
            Ok(vec![(positions[0], Err(format!("{error:#}")))])
        }
        Err(_) => {
            let mut results = Vec::with_capacity(request.jobs.len());
            for (index, job) in positions.into_iter().zip(request.jobs) {
                let single = ConvertRequest { jobs: vec![job] };
                let outcome = match runConverter(&single).await {
                    Ok(response) => response.results.into_iter().next().map_or_else(
                        || Err("the converter did not report this bundle".to_string()),
                        conversion,
                    ),
                    Err(error) => Err(format!("{error:#}")),
                };
                results.push((index, outcome));
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

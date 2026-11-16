use super::catalog::{objectName, trackTextureKey};
use super::types::{
    ArtJob, ArtModel, ConvertJob, ConvertModel, ConvertOutput, ConvertRequest, ConvertResponse,
    ConvertResult, Fit, MaterialBundle, NamedBundle, RuntimeTexture, ScenePrefab, SlotBundle,
    CONVERTER_PYTHON, CONVERTER_SCRIPT, DOWNLOAD_CONCURRENCY,
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
    /// Converted files as (bucket key, local file): the planned outputs, then those the converter
    /// named after the bundle's contents, whose keys `content_outputs` lists.
    Built {
        outputs: Vec<(String, PathBuf)>,
        content_outputs: Vec<String>,
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
            Ok(content_files) => {
                let content_outputs = content_files.iter().map(|(key, _)| key.clone()).collect();
                let mut outputs = outputFiles(&jobs[index], &dir, index);
                outputs.extend(content_files);
                JobOutcome::Built {
                    outputs,
                    content_outputs,
                }
            }
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

/// Each job's bundle followed by the bundles it needs beside it and those it looks things up in.
fn jobBundles(job: &ArtJob) -> impl Iterator<Item = &ManifestEntry> {
    std::iter::once(&job.entry)
        .chain(&job.dependencies)
        .chain(&job.lookups)
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

/// Track prefabs become `.glb` models, the shader index is JSON and the site keeps a few PNG
/// paths; every other picture is WebP.
fn outputFormat(key: &str) -> &'static str {
    if key.ends_with(".glb") {
        "glb"
    } else if key.ends_with(".json") {
        "json"
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

    let path = |file: &PathBuf| file.to_string_lossy().into_owned();
    // `files` holds the job's bundle, then its dependencies and its lookups in order.
    let downloaded = |name: &str| {
        if name == job.entry.name {
            return Some(path(&files[0]));
        }
        let position = job
            .dependencies
            .iter()
            .position(|entry| entry.name == name)?;
        Some(path(&files[position + 1]))
    };
    let prefabs = |names: &'a [String]| -> Vec<ScenePrefab<'a>> {
        names
            .iter()
            .filter_map(|name| {
                Some(ScenePrefab {
                    object: objectName(name),
                    bundle: downloaded(name)?,
                })
            })
            .collect()
    };
    let model = job.model.as_ref().map(|model| match model {
        ArtModel::Scenery(scene) => ConvertModel::Scenery {
            prefabs: prefabs(&scene.prefabs),
            runtime_textures: scene
                .runtime_textures
                .iter()
                .filter_map(|name| {
                    Some(RuntimeTexture {
                        name,
                        key: trackTextureKey(name),
                        bundle: downloaded(name)?,
                    })
                })
                .collect(),
            components: scene.components,
        },
        ArtModel::Uma(uma) => ConvertModel::Uma {
            title: &uma.title,
            prefabs: prefabs(&uma.prefabs),
            tail_textures: uma
                .tail_textures
                .iter()
                .filter_map(|texture| {
                    Some(SlotBundle {
                        slot: texture.slot,
                        bundle: downloaded(&texture.name)?,
                    })
                })
                .collect(),
            traits: &uma.traits,
        },
        ArtModel::Character(character) => ConvertModel::Character {
            title: &character.title,
            prefabs: prefabs(&character.prefabs),
            material_textures: character
                .material_textures
                .iter()
                .filter_map(|texture| {
                    Some(MaterialBundle {
                        material: &texture.material,
                        slot: texture.slot,
                        bundle: downloaded(&texture.name)?,
                    })
                })
                .collect(),
            traits: character.traits.as_ref(),
        },
        ArtModel::Motion(motion) => ConvertModel::Motion {
            title: &motion.title,
            skeleton: prefabs(&motion.skeleton),
            moves_root: motion.moves_root,
        },
        ArtModel::NodeMotion { title } => ConvertModel::NodeMotion { title },
        ArtModel::ClipCurves => ConvertModel::ClipCurves,
        ArtModel::Scripts => ConvertModel::Scripts {
            bundles: jobBundles(job)
                .zip(files)
                .map(|(entry, file)| NamedBundle {
                    name: &entry.name,
                    bundle: path(file),
                })
                .collect(),
        },
        ArtModel::Shaders => ConvertModel::Shaders,
    });

    ConvertJob {
        bundle: path(&files[0]),
        dependencies: files[1..].iter().map(path).collect(),
        model,
        outputs,
    }
}

/// The files a job's converter named after its bundle's contents as (bucket key, local file), or
/// why it did not convert.
type Conversion = std::result::Result<Vec<(String, PathBuf)>, String>;

fn conversion(result: ConvertResult) -> Conversion {
    if let Some(error) = result.error {
        return Err(error);
    }
    Ok(result
        .files
        .into_iter()
        .map(|file| (file.key, PathBuf::from(file.path)))
        .collect())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::game_art::types::{
        PlannedOutput, SceneryModel, SlotTexture, UmaOutfit, UmaTraits,
    };
    use crate::game_cdn::GLOBAL_CDN;
    use serde_json::json;

    const TRACK: &str = "3d/env/race/race10006/race10006_0000/pfb_env_race10006_0000_002";
    const TREES: &str = "3d/env/race/race10006/race10006_6000/pfb_env_race10006_6000_100";
    const TURF_TEXTURE: &str =
        "3d/env/race/race10006/race10006_0000/textures/tex_env_race10006_0000_112_turf00";

    #[test]
    fn filesTheConverterNamedArePublishedBesideThePlannedOnes() {
        let result: ConvertResult = serde_json::from_value(json!({
            "bundle": "shader",
            "files": [{"key": "game-assets/shaders/gallop/ui/default.json", "path": "work/default.json"}],
            "error": null,
        }))
        .unwrap();

        let named = conversion(result);

        assert_eq!(
            named,
            Ok(vec![(
                "game-assets/shaders/gallop/ui/default.json".to_string(),
                PathBuf::from("work/default.json")
            )])
        );
    }

    #[test]
    fn aCourseModelTellsTheConverterWhereEachPrefabAndTextureWasDownloaded() {
        let entry = |name: &str| ManifestEntry::new(name, 1, 1, 1);
        let job = ArtJob {
            source: &GLOBAL_CDN,
            name: "3d/env/race/race10006".to_string(),
            entry: entry(TRACK),
            dependencies: vec![entry(TURF_TEXTURE), entry(TREES)],
            lookups: Vec::new(),
            model: Some(ArtModel::Scenery(SceneryModel {
                prefabs: vec![TRACK.to_string(), TREES.to_string()],
                runtime_textures: vec![TURF_TEXTURE.to_string()],
                components: false,
            })),
            outputs: vec![PlannedOutput {
                key: "game-assets/racetracks/tokyo.glb".to_string(),
                object: "race10006".to_string(),
                fit: Fit::Native,
            }],
        };
        let files = ["track", "turf", "trees"].map(PathBuf::from);

        let request = serde_json::to_value(convertJob(&job, &files, Path::new("work"), 0)).unwrap();

        assert_eq!(
            request["model"],
            json!({
                "kind": "scenery",
                "prefabs": [
                    {"object": "pfb_env_race10006_0000_002", "bundle": "track"},
                    {"object": "pfb_env_race10006_6000_100", "bundle": "trees"},
                ],
                "runtime_textures": [{
                    "name": TURF_TEXTURE,
                    "key": format!("game-assets/{TURF_TEXTURE}.webp"),
                    "bundle": "turf",
                }],
            })
        );
        assert_eq!(request["outputs"][0]["format"], "glb");
    }

    #[test]
    fn aUmaTellsTheConverterItsPrefabsTailTexturesAndTraits() {
        let body = "3d/chara/body/bdy1001_00/pfb_bdy1001_00";
        let head = "3d/chara/head/chr1001_00/pfb_chr1001_00";
        let diffuse = "3d/chara/tail/tail0001_00/textures/tex_tail0001_00_1001_diff";
        let entry = |name: &str| ManifestEntry::new(name, 1, 1, 1);
        let uma = UmaOutfit {
            record: "3d/chara/body/bdy1001_00".to_string(),
            prefabs: vec![body.to_string(), head.to_string()],
            tail_textures: vec![SlotTexture {
                slot: "_MainTex",
                name: diffuse.to_string(),
            }],
            title: "Special Week: Special Dreamer".to_string(),
            key: "game-assets/umas/special-week/special-dreamer.glb".to_string(),
            traits: UmaTraits {
                chara: 1001,
                dress: 100101,
                run_motions: "bright".to_string(),
                finish_motions: "bright".to_string(),
                running_form: "standard",
            },
        };
        let job = ArtJob {
            source: &GLOBAL_CDN,
            name: uma.record.clone(),
            entry: entry(body),
            dependencies: vec![entry(diffuse), entry(head)],
            lookups: Vec::new(),
            outputs: vec![PlannedOutput {
                key: uma.key.clone(),
                object: "pfb_bdy1001_00".to_string(),
                fit: Fit::Native,
            }],
            model: Some(ArtModel::Uma(uma)),
        };
        let files = ["body", "diffuse", "head"].map(PathBuf::from);

        let request = serde_json::to_value(convertJob(&job, &files, Path::new("work"), 0)).unwrap();

        assert_eq!(
            request["model"],
            json!({
                "kind": "uma",
                "title": "Special Week: Special Dreamer",
                "prefabs": [
                    {"object": "pfb_bdy1001_00", "bundle": "body"},
                    {"object": "pfb_chr1001_00", "bundle": "head"},
                ],
                "tail_textures": [{"slot": "_MainTex", "bundle": "diffuse"}],
                "traits": {
                    "chara": 1001,
                    "dress": 100101,
                    "runMotions": "bright",
                    "finishMotions": "bright",
                    "runningForm": "standard",
                },
            })
        );
    }
}

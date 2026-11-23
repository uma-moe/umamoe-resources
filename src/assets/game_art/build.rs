use super::catalog::objectName;
use super::layout::rootLink;
use super::types::{
    ArtJob, ArtModel, ConvertJob, ConvertModel, ConvertOutput, ConvertResponse, ConvertResult,
    ConvertedArt, Fit, MaterialBundle, NamedBundle, RuntimeTexture, ScenePrefab, SlotBundle,
    CONVERTERS_VARIABLE, CONVERTER_PYTHON, CONVERTER_SCRIPT, DEFAULT_CONVERTER_PROCESSES,
    DOWNLOAD_CONCURRENCY, MAX_CONVERTER_PROCESSES,
};
use crate::game_cdn::{self, CdnSource, ManifestEntry};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::task::JoinSet;
use tracing::warn;

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

    /// An empty folder for one batch; the files of the batch that last used the slot are removed.
    fn batchDir(&self, slot: usize) -> Result<PathBuf> {
        let dir = self.root.join(format!("batch-{slot}"));
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
    /// named after the bundle's contents, which `converted` lists with the shaders a model uses.
    Built {
        outputs: Vec<(String, PathBuf)>,
        converted: ConvertedArt,
    },
    /// The bundle downloaded but does not convert.
    Unconvertible(String),
    /// The bundle could not be fetched; worth retrying on the next run.
    NotDownloaded(String),
}

/// A batch's bundles as each job's files, or why they did not download.
pub(super) struct DownloadedBatch {
    dir: PathBuf,
    bundles: Vec<Result<Vec<PathBuf>>>,
    took: Duration,
}

/// A batch's outcome per job in order, and how long its stages took.
pub(super) struct BuiltBatch {
    pub(super) outcomes: Vec<JobOutcome>,
    pub(super) download: Duration,
    pub(super) convert: Duration,
}

/// Downloads and decrypts one batch's bundles into the work folder's `slot`.
pub(super) async fn downloadBatch(
    client: &reqwest::Client,
    work: &WorkDir,
    slot: usize,
    jobs: &[ArtJob],
    base_key: &Arc<Vec<u8>>,
) -> Result<DownloadedBatch> {
    let dir = work.batchDir(slot)?;
    let started = Instant::now();
    let bundles = downloadBundles(client, &dir, jobs, base_key).await;
    Ok(DownloadedBatch {
        dir,
        bundles,
        took: started.elapsed(),
    })
}

/// Converts a downloaded batch; the converted files stay in its folder until the slot is reused.
pub(super) async fn convertBatch(
    jobs: &[ArtJob],
    downloaded: DownloadedBatch,
) -> Result<BuiltBatch> {
    let DownloadedBatch { dir, bundles, took } = downloaded;
    let mut outcomes: Vec<Option<JobOutcome>> = (0..jobs.len()).map(|_| None).collect();
    let mut conversions: Vec<(usize, ConvertJob)> = Vec::new();
    for (index, (job, files)) in jobs.iter().zip(bundles).enumerate() {
        match files {
            Ok(files) => conversions.push((index, convertJob(job, &files, &dir, index))),
            Err(error) => outcomes[index] = Some(JobOutcome::NotDownloaded(format!("{error:#}"))),
        }
    }

    let convert_started = Instant::now();
    let converted = convertAll(conversions).await?;
    let convert = convert_started.elapsed();

    for (index, conversion) in converted {
        outcomes[index] = Some(match conversion {
            Err(error) => JobOutcome::Unconvertible(error),
            Ok(reported) => {
                let converted = ConvertedArt {
                    content_outputs: reported.files.iter().map(|(key, _)| key.clone()).collect(),
                    shaders: reported.shaders,
                };
                let mut outputs = outputFiles(&jobs[index], &dir, index);
                outputs.extend(reported.files);
                JobOutcome::Built { outputs, converted }
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
    Ok(BuiltBatch {
        outcomes,
        download: took,
        convert,
    })
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
                .filter_map(|texture| {
                    Some(RuntimeTexture {
                        name: &texture.name,
                        key: texture.key.clone(),
                        bundle: downloaded(&texture.name)?,
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
                    file: job.links.get(&entry.name).map(|key| rootLink(key)),
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

/// What a job's converter reported beside the planned outputs, or why it did not convert.
type Conversion = std::result::Result<ReportedFiles, String>;

#[derive(Debug, PartialEq)]
struct ReportedFiles {
    /// Files named after the bundle's contents as (bucket key, local file).
    files: Vec<(String, PathBuf)>,
    shaders: Vec<String>,
}

fn conversion(result: ConvertResult) -> Conversion {
    if let Some(error) = result.error {
        return Err(error);
    }
    Ok(ReportedFiles {
        files: result
            .files
            .into_iter()
            .map(|file| (file.key, PathBuf::from(file.path)))
            .collect(),
        shaders: result.shaders,
    })
}

/// One job ready for a converter process: its place in the batch, its bundle (which names its
/// result) and its request.
struct PreparedJob {
    index: usize,
    bundle: String,
    request: serde_json::Value,
}

/// Converter processes a batch runs at once: `GAME_ART_CONVERTERS`, else the default. The server
/// shares its cores with the site, so this is a fixed count, not one per core.
pub(super) fn converterProcesses() -> usize {
    let configured = std::env::var(CONVERTERS_VARIABLE).ok();
    converterCount(configured.as_deref())
}

fn converterCount(configured: Option<&str>) -> usize {
    let Some(value) = configured else {
        return DEFAULT_CONVERTER_PROCESSES;
    };
    match value.trim().parse::<usize>() {
        Ok(count) if count > 0 => count.min(MAX_CONVERTER_PROCESSES),
        _ => {
            warn!(
                value,
                "{CONVERTERS_VARIABLE} is not a positive count; using the default"
            );
            DEFAULT_CONVERTER_PROCESSES
        }
    }
}

/// Converts the bundles in several converter processes at once, each taking every n-th job so
/// the heavy files of a shard spread over all of them.
async fn convertAll(conversions: Vec<(usize, ConvertJob<'_>)>) -> Result<Vec<(usize, Conversion)>> {
    let processes = converterProcesses().min(conversions.len()).max(1);
    let mut groups: Vec<Vec<PreparedJob>> = (0..processes).map(|_| Vec::new()).collect();
    for (position, (index, job)) in conversions.into_iter().enumerate() {
        groups[position % processes].push(PreparedJob {
            index,
            bundle: job.bundle.clone(),
            request: serde_json::to_value(&job)?,
        });
    }

    let mut tasks = JoinSet::new();
    for group in groups.into_iter().filter(|group| !group.is_empty()) {
        tasks.spawn(convertGroup(group));
    }
    let mut results = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        results.extend(joined.context("a converter task stopped unexpectedly")??);
    }
    Ok(results)
}

/// Converts the jobs in one converter process. If the process itself fails, each job gets its
/// own process, so one bundle that crashes the decoder cannot hold back the others.
async fn convertGroup(jobs: Vec<PreparedJob>) -> Result<Vec<(usize, Conversion)>> {
    let requests: Vec<&serde_json::Value> = jobs.iter().map(|job| &job.request).collect();
    let indexes: HashMap<&str, usize> = jobs
        .iter()
        .map(|job| (job.bundle.as_str(), job.index))
        .collect();

    match runConverter(&requests).await {
        Ok(response) => Ok(response
            .results
            .into_iter()
            .filter_map(|result| {
                let index = *indexes.get(result.bundle.as_str())?;
                Some((index, conversion(result)))
            })
            .collect()),
        Err(error) if jobs.len() == 1 => Ok(vec![(jobs[0].index, Err(format!("{error:#}")))]),
        Err(_) => {
            let mut results = Vec::with_capacity(jobs.len());
            for job in &jobs {
                let outcome = match runConverter(&[&job.request]).await {
                    Ok(response) => response.results.into_iter().next().map_or_else(
                        || Err("the converter did not report this bundle".to_string()),
                        conversion,
                    ),
                    Err(error) => Err(format!("{error:#}")),
                };
                results.push((job.index, outcome));
            }
            Ok(results)
        }
    }
}

async fn runConverter(jobs: &[&serde_json::Value]) -> Result<ConvertResponse> {
    let input = serde_json::to_vec(&serde_json::json!({ "jobs": jobs }))?;
    // At the lowest priority, so the site's processes on the same server always come first.
    let mut child = tokio::process::Command::new("nice")
        .args(["-n", "19", CONVERTER_PYTHON, CONVERTER_SCRIPT])
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
        PlacedTexture, PlannedOutput, SceneryModel, SlotTexture, UmaOutfit, UmaTraits,
    };
    use crate::game_cdn::GLOBAL_CDN;
    use serde_json::json;
    use std::collections::BTreeMap;

    const TRACK: &str = "3d/env/race/race10006/race10006_0000/pfb_env_race10006_0000_002";
    const TREES: &str = "3d/env/race/race10006/race10006_6000/pfb_env_race10006_6000_100";
    const TURF_TEXTURE: &str =
        "3d/env/race/race10006/race10006_0000/textures/tex_env_race10006_0000_112_turf00";

    #[test]
    fn filesTheConverterNamedArePublishedBesideThePlannedOnes() {
        let result: ConvertResult = serde_json::from_value(json!({
            "bundle": "shader",
            "files": [{"key": "art/shaders/gallop/ui/default.json", "path": "work/default.json"}],
            "error": null,
        }))
        .unwrap();

        let reported = conversion(result);

        assert_eq!(
            reported,
            Ok(ReportedFiles {
                files: vec![(
                    "art/shaders/gallop/ui/default.json".to_string(),
                    PathBuf::from("work/default.json")
                )],
                shaders: Vec::new(),
            })
        );
    }

    #[test]
    fn aModelReportsTheShadersItsMaterialsUse() {
        let result: ConvertResult = serde_json::from_value(json!({
            "bundle": "body",
            "shaders": ["/art/shaders/gallop/3d/chara/toon/tser.json"],
            "error": null,
        }))
        .unwrap();

        let reported = conversion(result).unwrap();

        assert_eq!(
            reported.shaders,
            vec!["/art/shaders/gallop/3d/chara/toon/tser.json"]
        );
    }

    #[test]
    fn aTimelineLinksWhatItPlaysFromTheBucketRoot() {
        let entry = |name: &str| ManifestEntry::new(name, 1, 1, 1);
        let timeline = "cutt/cutin/skill/crd100101_001/crd100101_001";
        let camera = "3d/motion/cutin/card/camera/crd100101_001/anm_cti_crd100101_001_01_cam";
        let effect = "3d/effect/charaskill/pfb_eff_chr1001_00/pfb_eff_chr1001_00_01";
        let job = ArtJob {
            source: &GLOBAL_CDN,
            name: timeline.to_string(),
            entry: entry(timeline),
            dependencies: Vec::new(),
            lookups: vec![entry(camera), entry(effect)],
            links: BTreeMap::from([
                (
                    camera.to_string(),
                    "art/umas/100101/cutin/camera/01_cam.glb".to_string(),
                ),
                (
                    effect.to_string(),
                    "art/characters/1001/effects/00/01.glb".to_string(),
                ),
            ]),
            model: Some(ArtModel::Scripts),
            outputs: vec![PlannedOutput {
                key: "art/umas/100101/cutin/timeline.json".to_string(),
                object: "crd100101_001".to_string(),
                fit: Fit::Native,
            }],
        };
        let files = ["timeline", "camera", "effect"].map(PathBuf::from);

        let request = serde_json::to_value(convertJob(&job, &files, Path::new("work"), 0)).unwrap();

        assert_eq!(
            request["model"]["bundles"],
            json!([
                {"name": timeline, "bundle": "timeline"},
                {
                    "name": camera,
                    "bundle": "camera",
                    "file": "/art/umas/100101/cutin/camera/01_cam.glb",
                },
                {
                    "name": effect,
                    "bundle": "effect",
                    "file": "/art/characters/1001/effects/00/01.glb",
                },
            ])
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
            links: BTreeMap::new(),
            model: Some(ArtModel::Scenery(SceneryModel {
                prefabs: vec![TRACK.to_string(), TREES.to_string()],
                runtime_textures: vec![PlacedTexture {
                    name: TURF_TEXTURE.to_string(),
                    key: "art/racecourses/10006/textures/0000_112_turf00.webp".to_string(),
                }],
                components: false,
            })),
            outputs: vec![PlannedOutput {
                key: "art/racecourses/10006/model.glb".to_string(),
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
                    "key": "art/racecourses/10006/textures/0000_112_turf00.webp",
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
            key: "art/umas/100101/model.glb".to_string(),
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
            links: BTreeMap::new(),
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

    #[test]
    fn theConverterCountIsFixedUnlessConfigured() {
        assert_eq!(converterCount(None), DEFAULT_CONVERTER_PROCESSES);
        assert_eq!(converterCount(Some("3")), 3);
        assert_eq!(converterCount(Some("64")), MAX_CONVERTER_PROCESSES);
        assert_eq!(converterCount(Some("0")), DEFAULT_CONVERTER_PROCESSES);
        assert_eq!(converterCount(Some("all")), DEFAULT_CONVERTER_PROCESSES);
    }
}

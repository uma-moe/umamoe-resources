use super::catalog::{
    converterRevision, courseScenes, isTrackPrefab, isTrackTexture, isUnusedDependency,
    runtimeTextureFolder,
};
use super::types::{
    ArtJob, ArtState, Catalog, Dependencies, ModelScene, PlannedOutput, PublishedArt,
};
use crate::game_cdn::{CdnSource, ManifestEntry};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Picks every catalogued file, and every racecourse to combine into one model, from the first
/// source that lists it, so a Global file replaces the JP one as soon as Global has it, and keeps
/// those whose published copy is missing or stale.
pub(super) fn planJobs(
    sources: &[(&'static CdnSource, Vec<ManifestEntry>)],
    catalog: &Catalog,
    state: &ArtState,
) -> Vec<ArtJob> {
    let mut claimed: HashSet<&str> = HashSet::new();
    let mut jobs = Vec::new();

    for (source, files) in sources {
        let listing = Listing::new(files);

        for entry in files {
            if !claimed.insert(entry.name.as_str()) {
                continue;
            }
            if let Some(job) = fileJob(source, entry, &listing, catalog) {
                jobs.push(job);
            }
        }

        for (course, scene) in courseScenes(files.iter().map(|entry| entry.name.as_str())) {
            if !claimed.insert(course) {
                continue;
            }
            if let Some(job) = courseJob(source, course, &scene, &listing, catalog) {
                jobs.push(job);
            }
        }
    }

    jobs.retain(|job| {
        let published = state.files.get(&job.name);
        !published.is_some_and(|record| isCurrent(record, job))
    });
    jobs
}

pub(super) fn publishedRecord(job: &ArtJob, error: Option<String>) -> PublishedArt {
    PublishedArt {
        region: job.source.region.to_string(),
        checksum: job.entry.checksum,
        converter: converterRevision(&job.name),
        outputs: outputKeys(&job.outputs),
        dependency_checksums: checksums(&job.dependencies),
        textures: Vec::new(),
        error,
    }
}

/// One source's file list, looked up by name.
struct Listing<'a> {
    by_name: HashMap<&'a str, &'a ManifestEntry>,
    /// Track textures by the folder that holds them, in file list order.
    track_textures: HashMap<&'a str, Vec<String>>,
}

impl<'a> Listing<'a> {
    fn new(files: &'a [ManifestEntry]) -> Self {
        let by_name = files
            .iter()
            .map(|entry| (entry.name.as_str(), entry))
            .collect();

        let mut track_textures: HashMap<&str, Vec<String>> = HashMap::new();
        for entry in files.iter().filter(|entry| isTrackTexture(&entry.name)) {
            if let Some((folder, _)) = entry.name.rsplit_once('/') {
                track_textures
                    .entry(folder)
                    .or_default()
                    .push(entry.name.clone());
            }
        }

        Self {
            by_name,
            track_textures,
        }
    }

    /// The textures the game puts on a track prefab's materials at runtime.
    fn runtimeTextures(&self, prefab: &str) -> Vec<String> {
        runtimeTextureFolder(prefab)
            .and_then(|folder| self.track_textures.get(folder.as_str()))
            .cloned()
            .unwrap_or_default()
    }

    /// Every bundle `entry` needs, directly or through another, sorted by name so the recorded
    /// checksums keep their order. `None` when the file list lacks one of them.
    fn listedDependencies(&self, entry: &ManifestEntry) -> Option<Vec<ManifestEntry>> {
        let mut found: BTreeMap<&str, &ManifestEntry> = BTreeMap::new();
        let mut pending: Vec<&str> = entry.dependencies.iter().map(String::as_str).collect();

        while let Some(name) = pending.pop() {
            let is_needed = name != entry.name && !isUnusedDependency(name);
            if !is_needed || found.contains_key(name) {
                continue;
            }
            let dependency = *self.by_name.get(name)?;
            found.insert(name, dependency);
            pending.extend(dependency.dependencies.iter().map(String::as_str));
        }

        Some(found.into_values().cloned().collect())
    }
}

/// The job building one catalogued file, or `None` when the site does not use it or the file
/// list lacks what it is built from.
fn fileJob(
    source: &'static CdnSource,
    entry: &ManifestEntry,
    listing: &Listing,
    catalog: &Catalog,
) -> Option<ArtJob> {
    let outputs = catalog.outputs(&entry.name)?;
    let mut dependencies = match catalog.dependencies(&entry.name) {
        Dependencies::None => Vec::new(),
        // An atlas cannot be cut without its texture.
        Dependencies::AtlasTexture(texture) => vec![(*listing.by_name.get(texture)?).clone()],
        // A model is not built from part of what it draws.
        Dependencies::Listed => listing.listedDependencies(entry)?,
    };

    let model = isTrackPrefab(&entry.name).then(|| ModelScene {
        prefabs: vec![entry.name.clone()],
        runtime_textures: listing.runtimeTextures(&entry.name),
    });
    if let Some(scene) = &model {
        // The model embeds a set of them, so it is rebuilt when one changes.
        dependencies.extend(
            scene
                .runtime_textures
                .iter()
                .filter_map(|name| listing.by_name.get(name.as_str()))
                .map(|texture| (*texture).clone()),
        );
    }

    Some(ArtJob {
        source,
        name: entry.name.clone(),
        entry: entry.clone(),
        dependencies,
        model,
        outputs,
    })
}

/// The job combining a racecourse's scene into one model, published under the course's folder.
/// It is rebuilt when any of its prefabs, or anything they draw, changes.
fn courseJob(
    source: &'static CdnSource,
    course: &str,
    scene: &[&str],
    listing: &Listing,
    catalog: &Catalog,
) -> Option<ArtJob> {
    let track = scene.first()?;
    let entry = *listing.by_name.get(track)?;
    let mut needed: BTreeMap<String, ManifestEntry> = BTreeMap::new();
    let mut runtime_textures: Vec<String> = Vec::new();

    for prefab in scene {
        let prefab_entry = *listing.by_name.get(prefab)?;
        if prefab != track {
            needed.insert(prefab.to_string(), prefab_entry.clone());
        }
        // A model is not built from part of what it draws.
        for dependency in listing.listedDependencies(prefab_entry)? {
            needed.insert(dependency.name.clone(), dependency);
        }
        for texture in listing.runtimeTextures(prefab) {
            if !runtime_textures.contains(&texture) {
                runtime_textures.push(texture);
            }
        }
    }
    for texture in &runtime_textures {
        if let Some(texture_entry) = listing.by_name.get(texture.as_str()) {
            needed.insert(texture.clone(), (*texture_entry).clone());
        }
    }

    Some(ArtJob {
        source,
        name: course.to_string(),
        entry: entry.clone(),
        dependencies: needed.into_values().collect(),
        model: Some(ModelScene {
            prefabs: scene.iter().map(|prefab| prefab.to_string()).collect(),
            runtime_textures,
        }),
        outputs: catalog.courseOutputs(course),
    })
}

fn isCurrent(record: &PublishedArt, job: &ArtJob) -> bool {
    record.region == job.source.region
        && record.checksum == job.entry.checksum
        && record.dependency_checksums == checksums(&job.dependencies)
        && record.converter == converterRevision(&job.name)
        && record.outputs == outputKeys(&job.outputs)
}

fn checksums(entries: &[ManifestEntry]) -> Vec<u64> {
    entries.iter().map(|entry| entry.checksum).collect()
}

fn outputKeys(outputs: &[PlannedOutput]) -> Vec<String> {
    outputs.iter().map(|output| output.key.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::game_art::types::{CardOutfit, CONVERTER_REVISION, COURSE_SKY};
    use crate::game_cdn::GLOBAL_CDN;

    static JAPAN_CDN: CdnSource = CdnSource {
        region: "jp",
        base_url: "https://jp.invalid",
    };

    const CARD: &str = "supportcard/support30050/tex_support_card_30050";
    const SKILL: &str = "outgame/skillicon/utx_ico_skill_10011";
    const OUTFIT: &str = "chara/chr1001/chara_stand_1001_100102";

    fn entry(name: &str, checksum: u64) -> ManifestEntry {
        ManifestEntry::new(name, 100, checksum, 1)
    }

    fn catalog() -> Catalog {
        Catalog::fromOutfits(&[])
    }

    fn record(name: &str, region: &str, checksum: u64) -> PublishedArt {
        let outputs = catalog().outputs(name).unwrap();
        PublishedArt {
            region: region.to_string(),
            checksum,
            converter: CONVERTER_REVISION,
            outputs: outputKeys(&outputs),
            dependency_checksums: Vec::new(),
            textures: Vec::new(),
            error: None,
        }
    }

    fn planned(jobs: &[ArtJob]) -> Vec<(&str, &str)> {
        jobs.iter()
            .map(|job| (job.name.as_str(), job.source.region))
            .collect()
    }

    #[test]
    fn firstRunBuildsEveryCataloguedFile() {
        let sources = vec![(
            &GLOBAL_CDN,
            vec![entry(CARD, 1), entry(SKILL, 2), entry("sound/bgm/x", 3)],
        )];

        let jobs = planJobs(&sources, &catalog(), &ArtState::default());

        assert_eq!(planned(&jobs), vec![(CARD, "global"), (SKILL, "global")]);
    }

    #[test]
    fn unchangedFilesAreSkippedAndChangedOnesRebuilt() {
        let sources = vec![(&GLOBAL_CDN, vec![entry(CARD, 1), entry(SKILL, 9)])];
        let mut state = ArtState::default();
        state
            .files
            .insert(CARD.to_string(), record(CARD, "global", 1));
        state
            .files
            .insert(SKILL.to_string(), record(SKILL, "global", 2));

        let jobs = planJobs(&sources, &catalog(), &state);

        assert_eq!(planned(&jobs), vec![(SKILL, "global")]);
    }

    #[test]
    fn japanFillsInUntilGlobalHasTheFile() {
        let japan_only = vec![(&GLOBAL_CDN, vec![]), (&JAPAN_CDN, vec![entry(CARD, 5)])];
        let both = vec![
            (&GLOBAL_CDN, vec![entry(CARD, 7)]),
            (&JAPAN_CDN, vec![entry(CARD, 5)]),
        ];
        let mut state = ArtState::default();

        let from_japan = planJobs(&japan_only, &catalog(), &state);
        state
            .files
            .insert(CARD.to_string(), publishedRecord(&from_japan[0], None));
        let from_global = planJobs(&both, &catalog(), &state);

        assert_eq!(planned(&from_japan), vec![(CARD, "jp")]);
        assert_eq!(planned(&from_global), vec![(CARD, "global")]);
    }

    #[test]
    fn unconvertibleFilesWaitForANewVersionOfTheFile() {
        let mut state = ArtState::default();
        state.files.insert(
            CARD.to_string(),
            PublishedArt {
                error: Some("no texture".to_string()),
                ..record(CARD, "global", 1)
            },
        );

        let same_file = planJobs(&[(&GLOBAL_CDN, vec![entry(CARD, 1)])], &catalog(), &state);
        let new_file = planJobs(&[(&GLOBAL_CDN, vec![entry(CARD, 2)])], &catalog(), &state);

        assert!(same_file.is_empty());
        assert_eq!(planned(&new_file), vec![(CARD, "global")]);
    }

    #[test]
    fn aNewConverterRevisionRebuildsEverything() {
        let mut state = ArtState::default();
        state.files.insert(
            CARD.to_string(),
            PublishedArt {
                converter: CONVERTER_REVISION - 1,
                ..record(CARD, "global", 1)
            },
        );

        let jobs = planJobs(&[(&GLOBAL_CDN, vec![entry(CARD, 1)])], &catalog(), &state);

        assert_eq!(planned(&jobs), vec![(CARD, "global")]);
    }

    const ATLAS: &str = "atlas/common/common";
    const ATLAS_TEXTURE: &str = "atlas/common/common_tex";

    #[test]
    fn atlasesAreBuiltWithTheirTextureAndRebuiltWhenItChanges() {
        let first = vec![(&GLOBAL_CDN, vec![entry(ATLAS, 1), entry(ATLAS_TEXTURE, 2)])];
        let new_texture = vec![(&GLOBAL_CDN, vec![entry(ATLAS, 1), entry(ATLAS_TEXTURE, 3)])];
        let mut state = ArtState::default();

        let first_run = planJobs(&first, &catalog(), &state);
        state
            .files
            .insert(ATLAS.to_string(), publishedRecord(&first_run[0], None));
        let unchanged = planJobs(&first, &catalog(), &state);
        let rebuilt = planJobs(&new_texture, &catalog(), &state);

        assert_eq!(planned(&first_run), vec![(ATLAS, "global")]);
        assert_eq!(first_run[0].dependencies[0].name, ATLAS_TEXTURE);
        assert!(unchanged.is_empty());
        assert_eq!(planned(&rebuilt), vec![(ATLAS, "global")]);
    }

    #[test]
    fn anAtlasWithoutItsTextureWaits() {
        let sources = vec![(&GLOBAL_CDN, vec![entry(ATLAS, 1)])];

        let jobs = planJobs(&sources, &catalog(), &ArtState::default());

        assert!(jobs.is_empty());
    }

    #[test]
    fn aNewCardWearingAPublishedOutfitRebuildsThatFile() {
        let first_card = CardOutfit {
            chara: 1001,
            card: 100103,
            dress: 100102,
        };
        let second_card = CardOutfit {
            card: 100104,
            ..first_card
        };
        let sources = vec![(&GLOBAL_CDN, vec![entry(OUTFIT, 1)])];
        let mut state = ArtState::default();
        let first_run = planJobs(&sources, &Catalog::fromOutfits(&[first_card]), &state);
        state
            .files
            .insert(OUTFIT.to_string(), publishedRecord(&first_run[0], None));

        let unchanged = planJobs(&sources, &Catalog::fromOutfits(&[first_card]), &state);
        let with_new_card = planJobs(
            &sources,
            &Catalog::fromOutfits(&[first_card, second_card]),
            &state,
        );

        assert!(unchanged.is_empty());
        assert_eq!(with_new_card[0].outputs.len(), 4);
    }

    const COURSE: &str = "3d/env/race/race10001";
    const PREFAB: &str = "3d/env/race/race10001/race10001_0000/pfb_env_race10001_0000_002";
    const MODEL: &str = "3d/probuilderexport/mdl_race10001/base01";
    const MATERIAL: &str =
        "sourceresources/3d/env/race/race10001/race10001_0000/materials/mtl_env_race10001_0000_002_base00";

    fn entryNeeding(name: &str, checksum: u64, dependencies: &[&str]) -> ManifestEntry {
        let mut needing = entry(name, checksum);
        needing.dependencies = dependencies.iter().map(|name| name.to_string()).collect();
        needing
    }

    #[test]
    fn trackPrefabsNeedEverythingTheyDrawButNotShadersOrAnimations() {
        let sources = vec![(
            &GLOBAL_CDN,
            vec![
                entryNeeding(PREFAB, 1, &[MODEL, "shader", "3d/motion/racemain/gate"]),
                entryNeeding(MODEL, 2, &[MATERIAL]),
                entryNeeding(MATERIAL, 3, &["shader"]),
            ],
        )];

        let jobs = planJobs(&sources, &catalog(), &ArtState::default());

        let needed: Vec<&str> = jobs[0]
            .dependencies
            .iter()
            .map(|dependency| dependency.name.as_str())
            .collect();
        assert_eq!(planned(&jobs), vec![(PREFAB, "global"), (COURSE, "global")]);
        assert_eq!(needed, vec![MODEL, MATERIAL]);
    }

    #[test]
    fn trackPrefabsCarryTheSeasonTexturesOfTheirPart() {
        let summer =
            "3d/env/race/race10001/race10001_0000/textures/tex_env_race10001_0000_212_turf00";
        let other_part =
            "3d/env/race/race10001/race10001_7000/textures/tex_env_race10001_7000_210_alpha00";
        let sources = vec![(
            &GLOBAL_CDN,
            vec![
                entryNeeding(PREFAB, 1, &[]),
                entry(summer, 2),
                entry(other_part, 3),
            ],
        )];

        let jobs = planJobs(&sources, &catalog(), &ArtState::default());

        let prefab = jobs.iter().find(|job| job.name == PREFAB).unwrap();
        let downloaded: Vec<&str> = prefab
            .dependencies
            .iter()
            .map(|dependency| dependency.name.as_str())
            .collect();
        let scene = prefab.model.as_ref().unwrap();
        assert_eq!(scene.prefabs, vec![PREFAB]);
        assert_eq!(scene.runtime_textures, vec![summer]);
        assert_eq!(downloaded, vec![summer]);
    }

    const TURF: &str = "3d/env/race/race10001/race10001_0000/pfb_env_race10001_0001_002";
    const NIGHT_TRACK: &str = "3d/env/race/race10001/race10001_0000/pfb_env_race10001_0000_004";
    const SPRING_TREES: &str = "3d/env/race/race10001/race10001_6000/pfb_env_race10001_6000_100";
    const SNOWY_TREES: &str = "3d/env/race/race10001/race10001_6000/pfb_env_race10001_6000_442";
    const STANDS: &str = "3d/env/race/race10001/race10001_7000/pfb_env_race10001_7001_010";
    const TREE_MODEL: &str =
        "sourceresources/3d/env/race/race10001/race10001_6000/models/mdl_env_race10001_6000_000";
    const TRACK_TEXTURE: &str =
        "3d/env/race/race10001/race10001_0000/textures/tex_env_race10001_0000_112_turf00";
    const STANDS_TEXTURE: &str =
        "3d/env/race/race10001/race10001_7000/textures/tex_env_race10001_7000_210_alpha00";

    fn sapporoFiles(tree_checksum: u64) -> Vec<ManifestEntry> {
        vec![
            entryNeeding(PREFAB, 1, &[MODEL]),
            entryNeeding(MODEL, 2, &[MATERIAL]),
            entry(MATERIAL, 3),
            entryNeeding(NIGHT_TRACK, 4, &[MODEL]),
            entryNeeding(TURF, 5, &[MATERIAL]),
            entryNeeding(SPRING_TREES, 6, &[TREE_MODEL]),
            entryNeeding(SNOWY_TREES, 7, &[TREE_MODEL]),
            entry(TREE_MODEL, tree_checksum),
            entry(STANDS, 9),
            entry(COURSE_SKY, 10),
            entry(TRACK_TEXTURE, 11),
            entry(STANDS_TEXTURE, 12),
        ]
    }

    #[test]
    fn eachRacecourseBecomesOneModelOfItsScene() {
        let mut catalog = catalog();
        catalog.course_names.insert(10001, "Sapporo".to_string());
        let sources = vec![(&GLOBAL_CDN, sapporoFiles(8))];

        let jobs = planJobs(&sources, &catalog, &ArtState::default());

        let course = jobs.iter().find(|job| job.name == COURSE).unwrap();
        let scene = course.model.as_ref().unwrap();
        let downloaded: Vec<&str> = course
            .dependencies
            .iter()
            .map(|dependency| dependency.name.as_str())
            .collect();
        assert_eq!(course.entry.name, PREFAB);
        assert_eq!(
            scene.prefabs,
            vec![PREFAB, TURF, SPRING_TREES, STANDS, COURSE_SKY]
        );
        assert_eq!(scene.runtime_textures, vec![TRACK_TEXTURE, STANDS_TEXTURE]);
        assert_eq!(
            downloaded,
            vec![
                COURSE_SKY,
                TURF,
                TRACK_TEXTURE,
                SPRING_TREES,
                STANDS,
                STANDS_TEXTURE,
                MODEL,
                MATERIAL,
                TREE_MODEL,
            ]
        );
        assert_eq!(course.outputs[0].key, "game-assets/racetracks/sapporo.glb");
    }

    #[test]
    fn aCourseModelIsRebuiltWhenAnythingItDrawsChanges() {
        let mut state = ArtState::default();
        let first_run = planJobs(&[(&GLOBAL_CDN, sapporoFiles(8))], &catalog(), &state);
        for job in &first_run {
            state
                .files
                .insert(job.name.clone(), publishedRecord(job, None));
        }

        let unchanged = planJobs(&[(&GLOBAL_CDN, sapporoFiles(8))], &catalog(), &state);
        let new_trees = planJobs(&[(&GLOBAL_CDN, sapporoFiles(80))], &catalog(), &state);

        assert!(unchanged.is_empty());
        assert_eq!(
            planned(&new_trees),
            vec![
                (SPRING_TREES, "global"),
                (SNOWY_TREES, "global"),
                (COURSE, "global")
            ]
        );
    }

    #[test]
    fn aTrackPrefabWaitsForEverythingItNeeds() {
        let sources = vec![(&GLOBAL_CDN, vec![entryNeeding(PREFAB, 1, &[MODEL])])];

        let jobs = planJobs(&sources, &catalog(), &ArtState::default());

        assert!(jobs.is_empty());
    }
}

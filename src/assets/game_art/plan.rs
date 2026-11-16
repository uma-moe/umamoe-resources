use super::catalog::{
    converterRevision, courseScenes, isCutinTimeline, isTrackTexture, isUnusedDependency,
    objectName, prefabFamily, runtimeTextureFolder,
};
use super::clips::gameClip;
use super::layout::{courseModelKey, Layout, SHADER_INDEX_KEY};
use super::race_motions::raceMotion;
use super::types::{
    ArtJob, ArtModel, ArtState, Catalog, CharacterModel, ChibiOutfit, ClipKind, ConvertedArt,
    Dependencies, Fit, MotionModel, PlacedTexture, PlannedOutput, PrefabFamily, PublishedArt,
    SceneryModel, SlotTexture, UmaOutfit, MOTION_SKELETON, SHADER_BUNDLE, SHADER_RECORD,
};
use crate::game_cdn::{CdnSource, ManifestEntry, ANDROID, WINDOWS};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Picks every catalogued file and clip, every racecourse to combine into one model, every outfit
/// to build a uma and chibi of and the shaders from the first source that lists them, so a Global
/// file replaces the JP one as soon as Global has it, and keeps those whose published copy is
/// missing or stale.
pub(super) fn planJobs(
    sources: &[(&'static CdnSource, Vec<ManifestEntry>)],
    catalog: &Catalog,
    layout: &Layout,
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
            let job = fileJob(source, entry, &listing, catalog, layout)
                .or_else(|| motionJob(source, entry, &listing, layout))
                .or_else(|| clipJob(source, entry, &listing, layout))
                .or_else(|| timelineJob(source, entry, &listing, layout));
            jobs.extend(job);
        }

        for (course, scene) in courseScenes(files.iter().map(|entry| entry.name.as_str())) {
            if !claimed.insert(course) {
                continue;
            }
            if let Some(job) = courseJob(source, course, &scene, &listing, layout) {
                jobs.push(job);
            }
        }

        // An outfit whose files this source lacks is left to the next source.
        for uma in &catalog.umas {
            if claimed.contains(uma.record.as_str()) {
                continue;
            }
            if let Some(job) = umaJob(source, uma, &listing) {
                claimed.insert(uma.record.as_str());
                jobs.push(job);
            }
        }

        for chibi in &catalog.chibis {
            if claimed.contains(chibi.record.as_str()) {
                continue;
            }
            if let Some(job) = chibiJob(source, chibi, &listing) {
                claimed.insert(chibi.record.as_str());
                jobs.push(job);
            }
        }

        if !claimed.contains(SHADER_RECORD) {
            if let Some(job) = shaderJob(source, &listing) {
                claimed.insert(SHADER_RECORD);
                jobs.push(job);
            }
        }
    }

    jobs.retain(|job| {
        let published = state.files.get(&job.name);
        !published.is_some_and(|record| isCurrent(record, job))
    });
    // One quick job that viewers of every model need; it should not wait behind thousands.
    jobs.sort_by_key(|job| job.name != SHADER_RECORD);
    jobs
}

/// Records of what the catalog no longer builds: a file no source lists any more or the
/// catalog stopped serving, a racecourse whose track is gone and an outfit that left master.mdb
/// or whose body is gone. Records waiting for a missing dependency are not orphans.
pub(super) fn orphanedRecords(
    sources: &[(&'static CdnSource, Vec<ManifestEntry>)],
    catalog: &Catalog,
    state: &ArtState,
) -> Vec<String> {
    let mut built: HashSet<&str> = HashSet::new();
    for (_, files) in sources {
        let names: HashSet<&str> = files.iter().map(|entry| entry.name.as_str()).collect();
        let is_catalogued = |name: &str| {
            catalog.outputs(name).is_some()
                || prefabFamily(name).is_some()
                || isTrackTexture(name)
                || raceMotion(name).is_some()
                || gameClip(name).is_some()
                || isCutinTimeline(name)
        };
        built.extend(names.iter().copied().filter(|name| is_catalogued(name)));
        built.extend(courseScenes(names.iter().copied()).into_keys());

        let outfits = catalog
            .umas
            .iter()
            .map(|uma| (uma.record.as_str(), &uma.prefabs))
            .chain(
                catalog
                    .chibis
                    .iter()
                    .map(|chibi| (chibi.record.as_str(), &chibi.model.prefabs)),
            );
        for (record, prefabs) in outfits {
            let body = prefabs.first().map(String::as_str);
            if body.is_some_and(|body| names.contains(body)) {
                built.insert(record);
            }
        }

        if files.iter().any(isShaderPrograms) {
            built.insert(SHADER_RECORD);
        }
    }

    state
        .files
        .keys()
        .filter(|name| !built.contains(name.as_str()))
        .cloned()
        .collect()
}

/// The record of a built job with what its converter reported beside the planned outputs.
pub(super) fn publishedRecord(
    job: &ArtJob,
    error: Option<String>,
    converted: ConvertedArt,
) -> PublishedArt {
    PublishedArt {
        region: job.source.region.to_string(),
        checksum: job.entry.checksum,
        converter: converterRevision(&job.name),
        outputs: outputKeys(&job.outputs),
        dependency_checksums: checksums(&job.dependencies),
        content_outputs: converted.content_outputs,
        textures: Vec::new(),
        error,
        shaders: converted.shaders,
        links: linksDigest(&job.links),
    }
}

/// One source's file list, looked up by name.
struct Listing<'a> {
    /// The Windows files, which models and pictures are built from.
    by_name: HashMap<&'a str, &'a ManifestEntry>,
    /// Track textures by the folder that holds them, in file list order.
    track_textures: HashMap<&'a str, Vec<String>>,
    /// The Android shader bundle, which is published.
    shader_programs: Option<&'a ManifestEntry>,
}

impl<'a> Listing<'a> {
    fn new(files: &'a [ManifestEntry]) -> Self {
        let by_name = files
            .iter()
            .filter(|entry| entry.platform == WINDOWS)
            .map(|entry| (entry.name.as_str(), entry))
            .collect();
        let shader_programs = files.iter().find(|entry| isShaderPrograms(entry));

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
            shader_programs,
        }
    }

    /// What a model loads to name its materials' shaders: the Windows shader bundle, as the
    /// materials are Windows ones. `None` while the file list lacks it.
    fn materialLookups(&self) -> Option<Vec<ManifestEntry>> {
        Some(vec![(*self.by_name.get(SHADER_BUNDLE)?).clone()])
    }

    /// The Windows entries of these files, or `None` when one is not listed.
    fn entries<'n>(&self, names: impl IntoIterator<Item = &'n str>) -> Option<Vec<ManifestEntry>> {
        names
            .into_iter()
            .map(|name| self.by_name.get(name).map(|entry| (*entry).clone()))
            .collect()
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

    /// Every bundle a model of these prefabs is built from beside the first, which is the job's
    /// own: the other prefabs and everything each one needs. `None` when one is not listed.
    fn sceneDependencies(&self, prefabs: &[String]) -> Option<BTreeMap<String, ManifestEntry>> {
        let first = prefabs.first()?;
        let mut needed = BTreeMap::new();

        for prefab in prefabs {
            let prefab_entry = *self.by_name.get(prefab.as_str())?;
            if prefab != first {
                needed.insert(prefab.clone(), prefab_entry.clone());
            }
            // A model is not built from part of what it draws.
            for dependency in self.listedDependencies(prefab_entry)? {
                needed.insert(dependency.name.clone(), dependency);
            }
        }
        Some(needed)
    }
}

/// The job building one picture, prefab or track texture, or `None` when the site does not use
/// the file or the file list lacks what it is built from.
fn fileJob(
    source: &'static CdnSource,
    entry: &ManifestEntry,
    listing: &Listing,
    catalog: &Catalog,
    layout: &Layout,
) -> Option<ArtJob> {
    let outputs = catalog
        .outputs(&entry.name)
        .or_else(|| layout.outputs(&entry.name))?;
    let mut dependencies = match catalog.dependencies(&entry.name) {
        Dependencies::None => Vec::new(),
        // An atlas cannot be cut without its texture.
        Dependencies::AtlasTexture(texture) => vec![(*listing.by_name.get(texture)?).clone()],
        // A model is not built from part of what it draws.
        Dependencies::Listed => listing.listedDependencies(entry)?,
    };

    let model = prefabFamily(&entry.name).map(|family| prefabModel(family, entry, listing, layout));
    let lookups = if model.is_some() {
        listing.materialLookups()?
    } else {
        Vec::new()
    };
    if let Some(ArtModel::Scenery(scene)) = &model {
        // The model embeds a set of them, so it is rebuilt when one changes.
        dependencies.extend(
            scene
                .runtime_textures
                .iter()
                .filter_map(|texture| listing.by_name.get(texture.name.as_str()))
                .map(|texture| (*texture).clone()),
        );
    }

    Some(ArtJob {
        source,
        name: entry.name.clone(),
        entry: entry.clone(),
        dependencies,
        lookups,
        links: BTreeMap::new(),
        model,
        outputs,
    })
}

/// Textures published on their own, each with its key.
fn placedTextures(names: Vec<String>, layout: &Layout) -> Vec<PlacedTexture> {
    names
        .into_iter()
        .filter_map(|name| {
            let key = layout.key(&name)?;
            Some(PlacedTexture { name, key })
        })
        .collect()
}

/// A track prefab with its season textures, an effect or cut-in set with each object's particle
/// systems and scripts, or a prop on its own bones.
fn prefabModel(
    family: PrefabFamily,
    entry: &ManifestEntry,
    listing: &Listing,
    layout: &Layout,
) -> ArtModel {
    let prefabs = vec![entry.name.clone()];
    match family {
        PrefabFamily::Track => ArtModel::Scenery(SceneryModel {
            runtime_textures: placedTextures(listing.runtimeTextures(&entry.name), layout),
            prefabs,
            components: false,
        }),
        PrefabFamily::Effect | PrefabFamily::CutinSet => ArtModel::Scenery(SceneryModel {
            prefabs,
            runtime_textures: Vec::new(),
            components: true,
        }),
        PrefabFamily::Prop => ArtModel::Character(CharacterModel {
            title: objectName(&entry.name).to_string(),
            prefabs,
            material_textures: Vec::new(),
            traits: None,
        }),
    }
}

/// The job combining a racecourse's scene into one model, published under the course's folder.
/// It is rebuilt when any of its prefabs, or anything they draw, changes.
fn courseJob(
    source: &'static CdnSource,
    course: &str,
    scene: &[&str],
    listing: &Listing,
    layout: &Layout,
) -> Option<ArtJob> {
    let prefabs: Vec<String> = scene.iter().map(|prefab| prefab.to_string()).collect();
    let entry = *listing.by_name.get(prefabs.first()?.as_str())?;
    let mut needed = listing.sceneDependencies(&prefabs)?;

    let mut runtime_textures: Vec<String> = Vec::new();
    for prefab in scene {
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
        lookups: listing.materialLookups()?,
        links: BTreeMap::new(),
        model: Some(ArtModel::Scenery(SceneryModel {
            prefabs,
            runtime_textures: placedTextures(runtime_textures, layout),
            components: false,
        })),
        outputs: vec![PlannedOutput {
            key: courseModelKey(course),
            object: objectName(course).to_string(),
            fit: Fit::Native,
        }],
    })
}

/// The job joining an outfit's body, head and tail into one uma, published under its body's
/// folder. It is rebuilt when any of its prefabs, anything they draw or a tail texture changes.
fn umaJob(source: &'static CdnSource, uma: &UmaOutfit, listing: &Listing) -> Option<ArtJob> {
    let body = uma.prefabs.first()?;
    let entry = *listing.by_name.get(body.as_str())?;
    let mut needed = listing.sceneDependencies(&uma.prefabs)?;

    // Characters added to a tail later can lack some of its textures; the tail then keeps the
    // material's own.
    let mut tail_textures: Vec<SlotTexture> = Vec::new();
    for texture in &uma.tail_textures {
        if let Some(texture_entry) = listing.by_name.get(texture.name.as_str()) {
            needed.insert(texture.name.clone(), (*texture_entry).clone());
            tail_textures.push(texture.clone());
        }
    }

    Some(ArtJob {
        source,
        name: uma.record.clone(),
        entry: entry.clone(),
        dependencies: needed.into_values().collect(),
        lookups: listing.materialLookups()?,
        links: BTreeMap::new(),
        model: Some(ArtModel::Uma(UmaOutfit {
            tail_textures,
            ..uma.clone()
        })),
        outputs: vec![PlannedOutput {
            key: uma.key.clone(),
            object: objectName(body).to_string(),
            fit: Fit::Native,
        }],
    })
}

/// The job joining an outfit's chibi body, the shared face, its hair and tail into one chibi,
/// published under its body's folder. It is rebuilt when any of its prefabs, anything they draw
/// or a runtime texture changes.
fn chibiJob(source: &'static CdnSource, chibi: &ChibiOutfit, listing: &Listing) -> Option<ArtJob> {
    let body = chibi.model.prefabs.first()?;
    let entry = *listing.by_name.get(body.as_str())?;
    let mut needed = listing.sceneDependencies(&chibi.model.prefabs)?;

    // A character added later can lack a tail colour or face sheet; the model then goes without.
    let mut model = chibi.model.clone();
    model
        .material_textures
        .retain(|texture| match listing.by_name.get(texture.name.as_str()) {
            Some(texture_entry) => {
                needed.insert(texture.name.clone(), (*texture_entry).clone());
                true
            }
            None => false,
        });

    Some(ArtJob {
        source,
        name: chibi.record.clone(),
        entry: entry.clone(),
        dependencies: needed.into_values().collect(),
        lookups: listing.materialLookups()?,
        links: BTreeMap::new(),
        model: Some(ArtModel::Character(model)),
        outputs: vec![PlannedOutput {
            key: chibi.key.clone(),
            object: objectName(body).to_string(),
            fit: Fit::Native,
        }],
    })
}

/// The job putting one race clip on the skeleton every uma shares, named after what it shows.
/// It is rebuilt when the clip or the skeleton's prefabs change.
fn motionJob(
    source: &'static CdnSource,
    entry: &ManifestEntry,
    listing: &Listing,
    layout: &Layout,
) -> Option<ArtJob> {
    let motion = raceMotion(&entry.name)?;
    let key = layout.key(&entry.name)?;
    let skeleton: Vec<String> = MOTION_SKELETON
        .iter()
        .map(|prefab| prefab.to_string())
        .collect();
    let dependencies = listing.entries(MOTION_SKELETON.iter().copied())?;

    Some(ArtJob {
        source,
        name: entry.name.clone(),
        entry: entry.clone(),
        dependencies,
        lookups: Vec::new(),
        links: BTreeMap::new(),
        model: Some(ArtModel::Motion(MotionModel {
            title: motion.title,
            skeleton,
            moves_root: false,
        })),
        outputs: vec![PlannedOutput {
            key,
            object: objectName(&entry.name).to_string(),
            fit: Fit::Native,
        }],
    })
}

/// The job publishing one chibi or cut-in clip by what it moves. A clip on a skeleton is rebuilt
/// when the skeleton's prefabs change too.
fn clipJob(
    source: &'static CdnSource,
    entry: &ManifestEntry,
    listing: &Listing,
    layout: &Layout,
) -> Option<ArtJob> {
    let clip = gameClip(&entry.name)?;
    let key = layout.key(&entry.name)?;
    let title = objectName(&entry.name).to_string();
    let (model, dependencies) = match clip.kind {
        ClipKind::Skeleton(prefabs) => {
            let motion = MotionModel {
                title,
                skeleton: prefabs.iter().map(|prefab| prefab.to_string()).collect(),
                moves_root: true,
            };
            (
                ArtModel::Motion(motion),
                listing.entries(prefabs.iter().copied())?,
            )
        }
        ClipKind::Node => (ArtModel::NodeMotion { title }, Vec::new()),
        ClipKind::Curves => (ArtModel::ClipCurves, Vec::new()),
        ClipKind::Script => (ArtModel::Scripts, Vec::new()),
    };

    Some(ArtJob {
        source,
        name: entry.name.clone(),
        entry: entry.clone(),
        dependencies,
        lookups: Vec::new(),
        links: BTreeMap::new(),
        model: Some(model),
        outputs: vec![PlannedOutput {
            key,
            object: objectName(&entry.name).to_string(),
            fit: Fit::Native,
        }],
    })
}

/// The job publishing a cut-in timeline's scripts as JSON. It loads the bundles the timeline
/// plays, its clips, effects and set, only to name and link them, so a change to them does not
/// rebuild it; a move of one does.
fn timelineJob(
    source: &'static CdnSource,
    entry: &ManifestEntry,
    listing: &Listing,
    layout: &Layout,
) -> Option<ArtJob> {
    if !isCutinTimeline(&entry.name) {
        return None;
    }
    let key = layout.key(&entry.name)?;
    let played = entry
        .dependencies
        .iter()
        .map(String::as_str)
        .filter(|name| *name != SHADER_BUNDLE);
    let lookups = listing.entries(played)?;
    let links = lookups
        .iter()
        .filter_map(|lookup| Some((lookup.name.clone(), layout.key(&lookup.name)?)))
        .collect();

    Some(ArtJob {
        source,
        name: entry.name.clone(),
        entry: entry.clone(),
        dependencies: Vec::new(),
        lookups,
        links,
        model: Some(ArtModel::Scripts),
        outputs: vec![PlannedOutput {
            key,
            object: objectName(&entry.name).to_string(),
            fit: Fit::Native,
        }],
    })
}

/// The Android shader bundle, whose shaders are published.
fn isShaderPrograms(entry: &ManifestEntry) -> bool {
    entry.platform == ANDROID && entry.name == SHADER_BUNDLE
}

/// The job publishing every shader of the Android shader bundle, each in a file of its own
/// below the shader index.
fn shaderJob(source: &'static CdnSource, listing: &Listing) -> Option<ArtJob> {
    let entry = listing.shader_programs?;
    Some(ArtJob {
        source,
        name: SHADER_RECORD.to_string(),
        entry: entry.clone(),
        dependencies: Vec::new(),
        lookups: Vec::new(),
        links: BTreeMap::new(),
        model: Some(ArtModel::Shaders),
        outputs: vec![PlannedOutput {
            key: SHADER_INDEX_KEY.to_string(),
            object: SHADER_BUNDLE.to_string(),
            fit: Fit::Native,
        }],
    })
}

fn isCurrent(record: &PublishedArt, job: &ArtJob) -> bool {
    record.region == job.source.region
        && record.checksum == job.entry.checksum
        && record.dependency_checksums == checksums(&job.dependencies)
        && record.converter == converterRevision(&job.name)
        && record.outputs == outputKeys(&job.outputs)
        && record.links == linksDigest(&job.links)
}

/// MD5 of a timeline's links, `None` for a job that links nothing.
fn linksDigest(links: &BTreeMap<String, String>) -> Option<String> {
    if links.is_empty() {
        return None;
    }

    let mut context = md5::Context::new();
    for (name, key) in links {
        context.consume(name.as_bytes());
        context.consume(b"\t");
        context.consume(key.as_bytes());
        context.consume(b"\n");
    }
    Some(format!("{:x}", context.compute()))
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
    use crate::assets::game_art::types::{
        CardOutfit, MaterialTexture, UmaTraits, CONVERTER_REVISION, COURSE_SKY, CUTIN_SKELETON,
        UMA_CONVERTER_REVISION,
    };
    use crate::game_cdn::ANDROID;
    use crate::game_cdn::GLOBAL_CDN;
    use std::collections::BTreeMap;

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

    fn plan(
        sources: &[(&'static CdnSource, Vec<ManifestEntry>)],
        catalog: &Catalog,
        state: &ArtState,
    ) -> Vec<ArtJob> {
        planJobs(sources, catalog, &Layout::new(catalog, sources), state)
    }

    /// Special Week's first card, whose outfit has a folder of its own.
    fn cardCatalog() -> Catalog {
        Catalog::fromOutfits(&[CardOutfit {
            chara: 1001,
            card: 100101,
            dress: 100101,
        }])
    }

    fn record(name: &str, region: &str, checksum: u64) -> PublishedArt {
        let outputs = catalog().outputs(name).unwrap();
        PublishedArt {
            region: region.to_string(),
            checksum,
            converter: CONVERTER_REVISION,
            outputs: outputKeys(&outputs),
            dependency_checksums: Vec::new(),
            content_outputs: Vec::new(),
            textures: Vec::new(),
            error: None,
            shaders: Vec::new(),
            links: None,
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

        let jobs = plan(&sources, &catalog(), &ArtState::default());

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

        let jobs = plan(&sources, &catalog(), &state);

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

        let from_japan = plan(&japan_only, &catalog(), &state);
        state.files.insert(
            CARD.to_string(),
            publishedRecord(&from_japan[0], None, ConvertedArt::default()),
        );
        let from_global = plan(&both, &catalog(), &state);

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

        let same_file = plan(&[(&GLOBAL_CDN, vec![entry(CARD, 1)])], &catalog(), &state);
        let new_file = plan(&[(&GLOBAL_CDN, vec![entry(CARD, 2)])], &catalog(), &state);

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

        let jobs = plan(&[(&GLOBAL_CDN, vec![entry(CARD, 1)])], &catalog(), &state);

        assert_eq!(planned(&jobs), vec![(CARD, "global")]);
    }

    const ATLAS: &str = "atlas/common/common";
    const ATLAS_TEXTURE: &str = "atlas/common/common_tex";

    #[test]
    fn atlasesAreBuiltWithTheirTextureAndRebuiltWhenItChanges() {
        let first = vec![(&GLOBAL_CDN, vec![entry(ATLAS, 1), entry(ATLAS_TEXTURE, 2)])];
        let new_texture = vec![(&GLOBAL_CDN, vec![entry(ATLAS, 1), entry(ATLAS_TEXTURE, 3)])];
        let mut state = ArtState::default();

        let first_run = plan(&first, &catalog(), &state);
        state.files.insert(
            ATLAS.to_string(),
            publishedRecord(&first_run[0], None, ConvertedArt::default()),
        );
        let unchanged = plan(&first, &catalog(), &state);
        let rebuilt = plan(&new_texture, &catalog(), &state);

        assert_eq!(planned(&first_run), vec![(ATLAS, "global")]);
        assert_eq!(first_run[0].dependencies[0].name, ATLAS_TEXTURE);
        assert!(unchanged.is_empty());
        assert_eq!(planned(&rebuilt), vec![(ATLAS, "global")]);
    }

    #[test]
    fn anAtlasWithoutItsTextureWaits() {
        let sources = vec![(&GLOBAL_CDN, vec![entry(ATLAS, 1)])];

        let jobs = plan(&sources, &catalog(), &ArtState::default());

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
        let first_run = plan(&sources, &Catalog::fromOutfits(&[first_card]), &state);
        state.files.insert(
            OUTFIT.to_string(),
            publishedRecord(&first_run[0], None, ConvertedArt::default()),
        );

        let unchanged = plan(&sources, &Catalog::fromOutfits(&[first_card]), &state);
        let with_new_card = plan(
            &sources,
            &Catalog::fromOutfits(&[first_card, second_card]),
            &state,
        );

        assert!(unchanged.is_empty());
        // The portrait and the full stand art, for each of the two cards.
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

    fn scenery(job: &ArtJob) -> &SceneryModel {
        match &job.model {
            Some(ArtModel::Scenery(scene)) => scene,
            _ => panic!("{} should be a scenery model", job.name),
        }
    }

    fn names(entries: &[ManifestEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    #[test]
    fn trackPrefabsNeedEverythingTheyDrawAndLookTheirShadersUp() {
        let sources = vec![(
            &GLOBAL_CDN,
            vec![
                entryNeeding(PREFAB, 1, &[MODEL, "shader", "3d/motion/racemain/gate"]),
                entryNeeding(MODEL, 2, &[MATERIAL]),
                entryNeeding(MATERIAL, 3, &["shader"]),
                entry(SHADER_BUNDLE, 4),
            ],
        )];

        let jobs = plan(&sources, &catalog(), &ArtState::default());

        let needed: Vec<&str> = jobs[0]
            .dependencies
            .iter()
            .map(|dependency| dependency.name.as_str())
            .collect();
        assert_eq!(planned(&jobs), vec![(PREFAB, "global"), (COURSE, "global")]);
        assert_eq!(needed, vec![MODEL, MATERIAL]);
        assert_eq!(names(&jobs[0].lookups), vec![SHADER_BUNDLE]);
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
                entry(SHADER_BUNDLE, 4),
            ],
        )];

        let jobs = plan(&sources, &catalog(), &ArtState::default());

        let prefab = jobs.iter().find(|job| job.name == PREFAB).unwrap();
        let downloaded: Vec<&str> = prefab
            .dependencies
            .iter()
            .map(|dependency| dependency.name.as_str())
            .collect();
        let scene = scenery(prefab);
        assert_eq!(scene.prefabs, vec![PREFAB]);
        assert_eq!(
            scene.runtime_textures,
            vec![PlacedTexture {
                name: summer.to_string(),
                key: "art/racecourses/10001/textures/0000_212_turf00.webp".to_string(),
            }]
        );
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
            entry(SHADER_BUNDLE, 13),
        ]
    }

    #[test]
    fn eachRacecourseBecomesOneModelOfItsScene() {
        let mut catalog = catalog();
        catalog.course_names.insert(10001, "Sapporo".to_string());
        let sources = vec![(&GLOBAL_CDN, sapporoFiles(8))];

        let jobs = plan(&sources, &catalog, &ArtState::default());

        let course = jobs.iter().find(|job| job.name == COURSE).unwrap();
        let scene = scenery(course);
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
        let textures: Vec<&str> = scene
            .runtime_textures
            .iter()
            .map(|texture| texture.name.as_str())
            .collect();
        assert_eq!(textures, vec![TRACK_TEXTURE, STANDS_TEXTURE]);
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
        assert_eq!(course.outputs[0].key, "art/racecourses/10001/model.glb");
    }

    #[test]
    fn aCourseModelIsRebuiltWhenAnythingItDrawsChanges() {
        let mut state = ArtState::default();
        let first_run = plan(&[(&GLOBAL_CDN, sapporoFiles(8))], &catalog(), &state);
        for job in &first_run {
            state.files.insert(
                job.name.clone(),
                publishedRecord(job, None, ConvertedArt::default()),
            );
        }

        let unchanged = plan(&[(&GLOBAL_CDN, sapporoFiles(8))], &catalog(), &state);
        let new_trees = plan(&[(&GLOBAL_CDN, sapporoFiles(80))], &catalog(), &state);

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

        let jobs = plan(&sources, &catalog(), &ArtState::default());

        assert!(jobs.is_empty());
    }

    const BODY: &str = "3d/chara/body/bdy1001_00/pfb_bdy1001_00";
    const HEAD: &str = "3d/chara/head/chr1001_00/pfb_chr1001_00";
    const TAIL: &str = "3d/chara/tail/tail0001_00/pfb_tail0001_00";
    const BODY_MATERIAL: &str = "sourceresources/3d/chara/body/bdy1001_00/materials/mtl_bdy1001_00";
    const HEAD_MATERIAL: &str =
        "sourceresources/3d/chara/head/chr1001_00/materials/mtl_chr1001_00_face";
    const TAIL_DIFFUSE: &str = "3d/chara/tail/tail0001_00/textures/tex_tail0001_00_1001_diff";
    const TAIL_TOON: &str = "3d/chara/tail/tail0001_00/textures/tex_tail0001_00_1001_shad_c";
    const RUN_CLIP: &str = "3d/motion/racemain/body/type01/anm_rac_type01_run03_base";

    fn specialWeek() -> UmaOutfit {
        let texture = |slot, name: &str| SlotTexture {
            slot,
            name: name.to_string(),
        };
        UmaOutfit {
            record: "3d/chara/body/bdy1001_00".to_string(),
            prefabs: [BODY, HEAD, TAIL].map(str::to_string).to_vec(),
            tail_textures: vec![
                texture("_MainTex", TAIL_DIFFUSE),
                texture("_ToonMap", TAIL_TOON),
            ],
            title: "Special Week: Special Dreamer".to_string(),
            key: "art/umas/100101/model.glb".to_string(),
            traits: UmaTraits {
                chara: 1001,
                dress: 100101,
                run_motions: "bright".to_string(),
                finish_motions: "bright".to_string(),
                running_form: "standard",
            },
        }
    }

    fn umaCatalog() -> Catalog {
        let mut catalog = catalog();
        catalog.umas = vec![specialWeek()];
        catalog
    }

    fn umaFiles(body_checksum: u64) -> Vec<ManifestEntry> {
        vec![
            entryNeeding(BODY, body_checksum, &[BODY_MATERIAL, "shader"]),
            entry(BODY_MATERIAL, 2),
            entryNeeding(
                HEAD,
                3,
                &[HEAD_MATERIAL, "3d/motion/racemain/body/type01/x"],
            ),
            entry(HEAD_MATERIAL, 4),
            entry(TAIL, 5),
            entry(TAIL_DIFFUSE, 6),
            entry(SHADER_BUNDLE, 7),
        ]
    }

    #[test]
    fn aUmaLooksItsShadersUpWithoutBeingRebuiltWhenTheyChange() {
        let mut state = ArtState::default();
        let first_run = plan(&[(&GLOBAL_CDN, umaFiles(1))], &umaCatalog(), &state);
        state.files.insert(
            first_run[0].name.clone(),
            publishedRecord(&first_run[0], None, ConvertedArt::default()),
        );
        let mut new_shaders = without(umaFiles(1), SHADER_BUNDLE);
        new_shaders.push(entry(SHADER_BUNDLE, 70));

        let replanned = plan(&[(&GLOBAL_CDN, new_shaders)], &umaCatalog(), &state);

        assert_eq!(names(&first_run[0].lookups), vec![SHADER_BUNDLE]);
        assert!(replanned.is_empty());
    }

    #[test]
    fn aUmaWaitsForTheShaderBundle() {
        let files = without(umaFiles(1), SHADER_BUNDLE);

        let jobs = plan(&[(&GLOBAL_CDN, files)], &umaCatalog(), &ArtState::default());

        assert!(jobs.is_empty());
    }

    fn androidShaders(checksum: u64) -> ManifestEntry {
        let mut shaders = entry(SHADER_BUNDLE, checksum);
        shaders.platform = ANDROID;
        shaders
    }

    #[test]
    fn theShadersArePublishedFromTheAndroidBundle() {
        let sources = vec![(
            &GLOBAL_CDN,
            vec![entry(SHADER_BUNDLE, 1), androidShaders(2)],
        )];

        let jobs = plan(&sources, &catalog(), &ArtState::default());

        assert_eq!(jobs.len(), 1);
        let shaders = &jobs[0];
        assert_eq!(shaders.name, SHADER_RECORD);
        assert_eq!(
            (shaders.entry.platform, shaders.entry.checksum),
            (ANDROID, 2)
        );
        assert!(matches!(shaders.model, Some(ArtModel::Shaders)));
        assert_eq!(outputKeys(&shaders.outputs), vec![SHADER_INDEX_KEY]);
    }

    #[test]
    fn theShadersAreBuiltBeforeEverythingElse() {
        let sources = vec![(&GLOBAL_CDN, vec![entry(CARD, 1), androidShaders(2)])];

        let jobs = plan(&sources, &catalog(), &ArtState::default());

        assert_eq!(
            planned(&jobs),
            vec![(SHADER_RECORD, "global"), (CARD, "global")]
        );
    }

    #[test]
    fn thePublishedShadersAreAnOrphanOnceAndroidStopsListingThem() {
        let state = recordedFiles(&[SHADER_RECORD]);
        let listed = vec![(&GLOBAL_CDN, vec![androidShaders(1)])];
        let gone = vec![(&GLOBAL_CDN, vec![entry(SHADER_BUNDLE, 1)])];

        let while_listed = orphanedRecords(&listed, &catalog(), &state);
        let once_gone = orphanedRecords(&gone, &catalog(), &state);

        assert!(while_listed.is_empty());
        assert_eq!(once_gone, vec![SHADER_RECORD]);
    }

    #[test]
    fn aUmaIsBuiltFromItsPrefabsWhatTheyDrawAndItsListedTailTextures() {
        let sources = vec![(&GLOBAL_CDN, umaFiles(1))];

        let jobs = plan(&sources, &umaCatalog(), &ArtState::default());

        let uma = &jobs[0];
        let Some(ArtModel::Uma(outfit)) = &uma.model else {
            panic!("the job should build a uma");
        };
        let kept_textures: Vec<&str> = outfit
            .tail_textures
            .iter()
            .map(|texture| texture.name.as_str())
            .collect();
        assert_eq!(planned(&jobs), vec![("3d/chara/body/bdy1001_00", "global")]);
        assert_eq!(uma.entry.name, BODY);
        assert_eq!(
            names(&uma.dependencies),
            vec![HEAD, TAIL, TAIL_DIFFUSE, BODY_MATERIAL, HEAD_MATERIAL]
        );
        assert_eq!(kept_textures, vec![TAIL_DIFFUSE]);
        assert_eq!(uma.outputs[0].key, "art/umas/100101/model.glb");
    }

    #[test]
    fn aUmaIsRebuiltWhenItsBodyChanges() {
        let mut state = ArtState::default();
        let first_run = plan(&[(&GLOBAL_CDN, umaFiles(1))], &umaCatalog(), &state);
        state.files.insert(
            first_run[0].name.clone(),
            publishedRecord(&first_run[0], None, ConvertedArt::default()),
        );

        let unchanged = plan(&[(&GLOBAL_CDN, umaFiles(1))], &umaCatalog(), &state);
        let new_body = plan(&[(&GLOBAL_CDN, umaFiles(10))], &umaCatalog(), &state);

        assert_eq!(
            state.files[&first_run[0].name].converter,
            UMA_CONVERTER_REVISION
        );
        assert!(unchanged.is_empty());
        assert_eq!(
            planned(&new_body),
            vec![("3d/chara/body/bdy1001_00", "global")]
        );
    }

    #[test]
    fn aUmaWaitsForItsHead() {
        let files: Vec<ManifestEntry> = umaFiles(1)
            .into_iter()
            .filter(|entry| entry.name != HEAD)
            .collect();

        let jobs = plan(&[(&GLOBAL_CDN, files)], &umaCatalog(), &ArtState::default());

        assert!(jobs.is_empty());
    }

    #[test]
    fn raceClipsAreBuiltOnTheSharedSkeleton() {
        let sources = vec![(
            &GLOBAL_CDN,
            vec![entry(RUN_CLIP, 1), entry(BODY, 2), entry(TAIL, 3)],
        )];

        let jobs = plan(&sources, &catalog(), &ArtState::default());

        let Some(ArtModel::Motion(motion)) = &jobs[0].model else {
            panic!("the job should build a motion");
        };
        assert_eq!(planned(&jobs), vec![(RUN_CLIP, "global")]);
        assert_eq!(names(&jobs[0].dependencies), vec![BODY, TAIL]);
        assert_eq!(motion.skeleton, vec![BODY, TAIL]);
        assert_eq!(motion.title, "Bright: Fast run, standard form");
        assert_eq!(
            jobs[0].outputs[0].key,
            "art/motions/race/bright/run-fast-standard.glb"
        );
    }

    #[test]
    fn aRaceClipWaitsForTheSkeleton() {
        let sources = vec![(&GLOBAL_CDN, vec![entry(RUN_CLIP, 1), entry(BODY, 2)])];

        let jobs = plan(&sources, &catalog(), &ArtState::default());

        assert!(jobs.is_empty());
    }

    const SPECIAL_WEEK: &str = "3d/chara/body/bdy1001_00";

    fn recordedFiles(names: &[&str]) -> ArtState {
        let mut state = ArtState::default();
        for name in names {
            let record = PublishedArt {
                region: "global".to_string(),
                checksum: 1,
                converter: CONVERTER_REVISION,
                outputs: Vec::new(),
                dependency_checksums: Vec::new(),
                content_outputs: Vec::new(),
                textures: Vec::new(),
                error: None,
                shaders: Vec::new(),
                links: None,
            };
            state.files.insert(name.to_string(), record);
        }
        state
    }

    fn without(files: Vec<ManifestEntry>, gone: &str) -> Vec<ManifestEntry> {
        files
            .into_iter()
            .filter(|entry| !entry.name.contains(gone))
            .collect()
    }

    #[test]
    fn aFileNoSourceListsAnyMoreIsAnOrphan() {
        let state = recordedFiles(&[CARD, SKILL, RUN_CLIP]);
        let sources = vec![(&GLOBAL_CDN, vec![entry(CARD, 1), entry(RUN_CLIP, 2)])];

        let orphans = orphanedRecords(&sources, &catalog(), &state);

        assert_eq!(orphans, vec![SKILL]);
    }

    #[test]
    fn aFileTheCatalogStoppedServingIsAnOrphan() {
        let card = CardOutfit {
            chara: 1001,
            card: 100103,
            dress: 100102,
        };
        let state = recordedFiles(&[OUTFIT]);
        let sources = vec![(&GLOBAL_CDN, vec![entry(OUTFIT, 1)])];

        let served = orphanedRecords(&sources, &Catalog::fromOutfits(&[card]), &state);
        let dropped = orphanedRecords(&sources, &catalog(), &state);

        assert!(served.is_empty());
        assert_eq!(dropped, vec![OUTFIT]);
    }

    #[test]
    fn filesWaitingForWhatTheyNeedAreNoOrphans() {
        let state = recordedFiles(&[ATLAS, SPECIAL_WEEK]);
        let mut files = without(umaFiles(1), HEAD);
        files.push(entry(ATLAS, 1));

        let orphans = orphanedRecords(&[(&GLOBAL_CDN, files)], &umaCatalog(), &state);

        assert!(orphans.is_empty());
    }

    #[test]
    fn anOutfitIsAnOrphanOnceItsBodyOrItsDressIsGone() {
        let state = recordedFiles(&[SPECIAL_WEEK]);
        let without_body = without(umaFiles(1), BODY);

        let body_gone = orphanedRecords(&[(&GLOBAL_CDN, without_body)], &umaCatalog(), &state);
        let dress_gone = orphanedRecords(&[(&GLOBAL_CDN, umaFiles(1))], &catalog(), &state);

        assert_eq!(body_gone, vec![SPECIAL_WEEK]);
        assert_eq!(dress_gone, vec![SPECIAL_WEEK]);
    }

    #[test]
    fn aRacecourseIsAnOrphanOnceItsTrackIsGone() {
        let state = recordedFiles(&[COURSE]);
        let without_track = without(sapporoFiles(8), "pfb_env_race10001_0000_");

        let listed = orphanedRecords(&[(&GLOBAL_CDN, sapporoFiles(8))], &catalog(), &state);
        let track_gone = orphanedRecords(&[(&GLOBAL_CDN, without_track)], &catalog(), &state);

        assert!(listed.is_empty());
        assert_eq!(track_gone, vec![COURSE]);
    }

    const CHIBI_BODY: &str = "3d/chara/mini/body/mbdy1001_00/pfb_mbdy1001_00";
    const CHIBI_HAIR: &str = "3d/chara/mini/head/mchr1001_00/pfb_mchr1001_00_hair";
    const CHIBI_EYE: &str = "3d/chara/mini/head/mchr1001_00/textures/tex_mchr1001_00_eye_diff";
    const CHIBI_MOUTH: &str = "3d/chara/mini/head/mchr1001_00/textures/tex_mchr1001_00_mouth_diff";

    fn chibiCatalog() -> Catalog {
        let sheet = |material: &str, name: &str| MaterialTexture {
            material: material.to_string(),
            slot: "_MainTex",
            name: name.to_string(),
        };
        let mut catalog = catalog();
        catalog.chibis = vec![ChibiOutfit {
            record: "3d/chara/mini/body/mbdy1001_00".to_string(),
            key: "art/umas/100101/chibi.glb".to_string(),
            model: CharacterModel {
                title: "Special Week: Special Dreamer".to_string(),
                prefabs: vec![CHIBI_BODY.to_string(), CHIBI_HAIR.to_string()],
                material_textures: vec![
                    sheet("mtl_mchr0001_00_eye", CHIBI_EYE),
                    sheet("mtl_mchr0001_00_mouth", CHIBI_MOUTH),
                ],
                traits: None,
            },
        }];
        catalog
    }

    #[test]
    fn aChibiNeedsItsPrefabsAndFaceSheetsAndGoesWithoutAMissingSheet() {
        let files = vec![
            entryNeeding(CHIBI_BODY, 1, &["shader"]),
            entry(CHIBI_HAIR, 2),
            entry(CHIBI_EYE, 3),
            entry(SHADER_BUNDLE, 4),
        ];

        let jobs = plan(
            &[(&GLOBAL_CDN, files)],
            &chibiCatalog(),
            &ArtState::default(),
        );

        let Some(ArtModel::Character(model)) = &jobs[0].model else {
            panic!("a chibi is a character model");
        };
        let filled: Vec<&str> = model
            .material_textures
            .iter()
            .map(|texture| texture.material.as_str())
            .collect();
        assert_eq!(
            planned(&jobs),
            vec![("3d/chara/mini/body/mbdy1001_00", "global")]
        );
        assert_eq!(names(&jobs[0].dependencies), vec![CHIBI_HAIR, CHIBI_EYE]);
        assert_eq!(names(&jobs[0].lookups), vec![SHADER_BUNDLE]);
        assert_eq!(filled, vec!["mtl_mchr0001_00_eye"]);
        assert_eq!(jobs[0].outputs[0].key, "art/umas/100101/chibi.glb");
    }

    const CUTIN_BODY_CLIP: &str =
        "3d/motion/cutin/card/body/crd100101_001/anm_cti_crd100101_001_01";
    const CUTIN_CAMERA_CLIP: &str =
        "3d/motion/cutin/card/camera/crd100101_001/anm_cti_crd100101_001_01_cam";

    #[test]
    fn aBodyClipIsRebuiltWithItsSkeletonAndACameraClipStandsAlone() {
        let mut files: Vec<ManifestEntry> = CUTIN_SKELETON
            .iter()
            .enumerate()
            .map(|(checksum, prefab)| entry(prefab, checksum as u64))
            .collect();
        files.push(entry(CUTIN_BODY_CLIP, 10));
        files.push(entry(CUTIN_CAMERA_CLIP, 11));

        let jobs = plan(
            &[(&GLOBAL_CDN, files)],
            &cardCatalog(),
            &ArtState::default(),
        );

        let body = jobs.iter().find(|job| job.name == CUTIN_BODY_CLIP).unwrap();
        let camera = jobs
            .iter()
            .find(|job| job.name == CUTIN_CAMERA_CLIP)
            .unwrap();
        assert_eq!(names(&body.dependencies), CUTIN_SKELETON.to_vec());
        assert!(matches!(
            &body.model,
            Some(ArtModel::Motion(MotionModel {
                moves_root: true,
                ..
            }))
        ));
        assert!(camera.dependencies.is_empty());
        assert!(matches!(camera.model, Some(ArtModel::NodeMotion { .. })));
        assert_eq!(
            camera.outputs[0].key,
            "art/umas/100101/cutin/camera/01_cam.glb"
        );
    }

    const TIMELINE: &str = "cutt/cutin/skill/crd100101_001/crd100101_001";

    #[test]
    fn aTimelineLooksUpWhatItPlaysAndWaitsUntilAllOfItIsListed() {
        let timeline = entryNeeding(TIMELINE, 1, &[CUTIN_CAMERA_CLIP, "shader"]);
        let complete = vec![timeline.clone(), entry(CUTIN_CAMERA_CLIP, 2)];
        let missing_clip = vec![timeline];

        let jobs = plan(
            &[(&GLOBAL_CDN, complete)],
            &cardCatalog(),
            &ArtState::default(),
        );
        let waiting = plan(
            &[(&GLOBAL_CDN, missing_clip)],
            &cardCatalog(),
            &ArtState::default(),
        );

        let timeline_job = jobs.iter().find(|job| job.name == TIMELINE).unwrap();
        assert!(timeline_job.dependencies.is_empty());
        assert_eq!(names(&timeline_job.lookups), vec![CUTIN_CAMERA_CLIP]);
        assert_eq!(
            timeline_job.outputs[0].key,
            "art/umas/100101/cutin/timeline.json"
        );
        assert_eq!(
            timeline_job.links,
            BTreeMap::from([(
                CUTIN_CAMERA_CLIP.to_string(),
                "art/umas/100101/cutin/camera/01_cam.glb".to_string()
            )])
        );
        assert!(waiting.is_empty());
    }

    #[test]
    fn aTimelineIsRebuiltWhenAFileItLinksMoves() {
        let files = vec![
            entryNeeding(TIMELINE, 1, &[CUTIN_CAMERA_CLIP]),
            entry(CUTIN_CAMERA_CLIP, 2),
        ];
        let sources = vec![(&GLOBAL_CDN, files)];
        let mut state = ArtState::default();
        for job in plan(&sources, &cardCatalog(), &state) {
            state.files.insert(
                job.name.clone(),
                publishedRecord(&job, None, ConvertedArt::default()),
            );
        }

        let unchanged = plan(&sources, &cardCatalog(), &state);
        let card_gone = plan(&sources, &catalog(), &state);

        assert!(unchanged.is_empty());
        let rebuilt: Vec<&str> = card_gone.iter().map(|job| job.name.as_str()).collect();
        assert!(rebuilt.contains(&TIMELINE));
        assert_eq!(
            card_gone
                .iter()
                .find(|job| job.name == TIMELINE)
                .map(|job| job.outputs[0].key.as_str()),
            Some("art/cutins/crd100101_001/timeline.json")
        );
    }

    #[test]
    fn effectsKeepTheirComponentsAndPropsStandOnTheirBones() {
        let effect = "3d/effect/race/skill/pfb_eff_rac_skill_00001";
        let prop = "3d/chara/prop/prop0001/pfb_prop0001";
        let files = vec![entry(effect, 1), entry(prop, 2), entry(SHADER_BUNDLE, 3)];

        let jobs = plan(&[(&GLOBAL_CDN, files)], &catalog(), &ArtState::default());

        let effect_job = jobs.iter().find(|job| job.name == effect).unwrap();
        let prop_job = jobs.iter().find(|job| job.name == prop).unwrap();
        assert!(scenery(effect_job).components);
        assert!(scenery(effect_job).runtime_textures.is_empty());
        assert!(matches!(prop_job.model, Some(ArtModel::Character(_))));
        assert_eq!(prop_job.outputs[0].key, "art/props/0001.glb");
    }
}

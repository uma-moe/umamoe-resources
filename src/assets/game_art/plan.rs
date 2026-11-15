use super::catalog::{isTrackTexture, isUnusedDependency, runtimeTextureFolder};
use super::types::{
    ArtJob, ArtState, Catalog, Dependencies, PlannedOutput, PublishedArt, CONVERTER_REVISION,
};
use crate::game_cdn::{CdnSource, ManifestEntry};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Picks every catalogued file from the first source that lists it, so a Global file replaces
/// the JP one as soon as Global has it, and keeps those whose published copy is missing or stale.
pub(super) fn planJobs(
    sources: &[(&'static CdnSource, Vec<ManifestEntry>)],
    catalog: &Catalog,
    state: &ArtState,
) -> Vec<ArtJob> {
    let mut claimed: HashSet<&str> = HashSet::new();
    let mut jobs = Vec::new();

    for (source, files) in sources {
        let by_name: HashMap<&str, &ManifestEntry> = files
            .iter()
            .map(|entry| (entry.name.as_str(), entry))
            .collect();
        let track_textures = trackTexturesByFolder(files);

        for entry in files {
            if !claimed.insert(entry.name.as_str()) {
                continue;
            }
            let Some(outputs) = catalog.outputs(&entry.name) else {
                continue;
            };

            let dependencies = match catalog.dependencies(&entry.name) {
                Dependencies::None => Vec::new(),
                Dependencies::AtlasTexture(texture) => match by_name.get(texture) {
                    Some(dependency) => vec![(*dependency).clone()],
                    // An atlas cannot be cut without its texture.
                    None => continue,
                },
                Dependencies::Listed => match listedDependencies(entry, &by_name) {
                    Some(dependencies) => dependencies,
                    // A model is not built from part of what it draws.
                    None => continue,
                },
            };

            let published = state.files.get(&entry.name);
            let is_current = published
                .is_some_and(|record| isCurrent(record, source, entry, &dependencies, &outputs));
            if is_current {
                continue;
            }

            let runtime_textures = runtimeTextureFolder(&entry.name)
                .and_then(|folder| track_textures.get(folder.as_str()))
                .cloned()
                .unwrap_or_default();

            jobs.push(ArtJob {
                source,
                entry: entry.clone(),
                dependencies,
                runtime_textures,
                outputs,
            });
        }
    }

    jobs
}

pub(super) fn publishedRecord(
    job: &ArtJob,
    textures: Vec<String>,
    error: Option<String>,
) -> PublishedArt {
    PublishedArt {
        region: job.source.region.to_string(),
        checksum: job.entry.checksum,
        converter: CONVERTER_REVISION,
        outputs: outputKeys(&job.outputs),
        dependency_checksums: checksums(&job.dependencies),
        textures,
        error,
    }
}

/// Track textures by the folder that holds them, in file list order.
fn trackTexturesByFolder(files: &[ManifestEntry]) -> HashMap<&str, Vec<String>> {
    let mut by_folder: HashMap<&str, Vec<String>> = HashMap::new();
    for entry in files.iter().filter(|entry| isTrackTexture(&entry.name)) {
        if let Some((folder, _)) = entry.name.rsplit_once('/') {
            by_folder
                .entry(folder)
                .or_default()
                .push(entry.name.clone());
        }
    }
    by_folder
}

/// Every bundle `entry` needs, directly or through another, sorted by name so the recorded
/// checksums keep their order. `None` when the file list lacks one of them.
fn listedDependencies(
    entry: &ManifestEntry,
    by_name: &HashMap<&str, &ManifestEntry>,
) -> Option<Vec<ManifestEntry>> {
    let mut found: BTreeMap<&str, &ManifestEntry> = BTreeMap::new();
    let mut pending: Vec<&str> = entry.dependencies.iter().map(String::as_str).collect();

    while let Some(name) = pending.pop() {
        let is_needed = name != entry.name && !isUnusedDependency(name);
        if !is_needed || found.contains_key(name) {
            continue;
        }
        let dependency = *by_name.get(name)?;
        found.insert(name, dependency);
        pending.extend(dependency.dependencies.iter().map(String::as_str));
    }

    Some(found.into_values().cloned().collect())
}

fn isCurrent(
    record: &PublishedArt,
    source: &CdnSource,
    entry: &ManifestEntry,
    dependencies: &[ManifestEntry],
    outputs: &[PlannedOutput],
) -> bool {
    record.region == source.region
        && record.checksum == entry.checksum
        && record.dependency_checksums == checksums(dependencies)
        && record.converter == CONVERTER_REVISION
        && record.outputs == outputKeys(outputs)
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
    use crate::assets::game_art::types::CardOutfit;
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
            .map(|job| (job.entry.name.as_str(), job.source.region))
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
        state.files.insert(
            CARD.to_string(),
            publishedRecord(&from_japan[0], Vec::new(), None),
        );
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
        state.files.insert(
            ATLAS.to_string(),
            publishedRecord(&first_run[0], Vec::new(), None),
        );
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
        state.files.insert(
            OUTFIT.to_string(),
            publishedRecord(&first_run[0], Vec::new(), None),
        );

        let unchanged = planJobs(&sources, &Catalog::fromOutfits(&[first_card]), &state);
        let with_new_card = planJobs(
            &sources,
            &Catalog::fromOutfits(&[first_card, second_card]),
            &state,
        );

        assert!(unchanged.is_empty());
        assert_eq!(with_new_card[0].outputs.len(), 4);
    }

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
        assert_eq!(planned(&jobs), vec![(PREFAB, "global")]);
        assert_eq!(needed, vec![MODEL, MATERIAL]);
    }

    #[test]
    fn trackPrefabsLinkTheSeasonTexturesOfTheirPart() {
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

        let prefab = jobs.iter().find(|job| job.entry.name == PREFAB).unwrap();
        assert_eq!(prefab.runtime_textures, vec![summer]);
        assert_eq!(jobs.len(), 3);
    }

    #[test]
    fn aTrackPrefabWaitsForEverythingItNeeds() {
        let sources = vec![(&GLOBAL_CDN, vec![entryNeeding(PREFAB, 1, &[MODEL])])];

        let jobs = planJobs(&sources, &catalog(), &ArtState::default());

        assert!(jobs.is_empty());
    }
}

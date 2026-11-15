use super::types::{ArtJob, ArtState, Catalog, PlannedOutput, PublishedArt, CONVERTER_REVISION};
use crate::game_cdn::{CdnSource, ManifestEntry};
use std::collections::HashSet;

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
        for entry in files {
            if !claimed.insert(entry.name.as_str()) {
                continue;
            }
            let Some(outputs) = catalog.outputs(&entry.name) else {
                continue;
            };

            let published = state.files.get(&entry.name);
            if published.is_some_and(|record| isCurrent(record, source, entry, &outputs)) {
                continue;
            }

            jobs.push(ArtJob {
                source,
                entry: entry.clone(),
                outputs,
            });
        }
    }

    jobs
}

pub(super) fn publishedRecord(job: &ArtJob, error: Option<String>) -> PublishedArt {
    PublishedArt {
        region: job.source.region.to_string(),
        checksum: job.entry.checksum,
        converter: CONVERTER_REVISION,
        outputs: outputKeys(&job.outputs),
        error,
    }
}

fn isCurrent(
    record: &PublishedArt,
    source: &CdnSource,
    entry: &ManifestEntry,
    outputs: &[PlannedOutput],
) -> bool {
    record.region == source.region
        && record.checksum == entry.checksum
        && record.converter == CONVERTER_REVISION
        && record.outputs == outputKeys(outputs)
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
}

//! Comparing a plan with the sources the frontend's image sync recorded for the art it holds,
//! so the Rust port is checked against every picture the site shows today.

use super::{frontend_overlay_path, BannerImagePlan, SyncedLocale};
use std::collections::{BTreeMap, BTreeSet};

/// The sync's state files map `src/assets/timeline-images/<locale>/…` to `<source URL>#<fit>`.
pub type FrontendSources = BTreeMap<String, String>;

#[derive(Debug, Default)]
pub struct ParityReport {
    pub japanese: LocaleParity,
    pub english: LocaleParity,
}

#[derive(Debug, Default)]
pub struct LocaleParity {
    /// Banners whose recorded source the plan picks too.
    pub matching: usize,
    pub mismatches: Vec<SourceMismatch>,
    /// Recorded art the plan picks no source for.
    pub unplanned: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub struct SourceMismatch {
    pub image_path: String,
    pub planned: Option<String>,
    pub recorded: String,
}

pub fn compare_with_frontend_sources(
    plan: &BannerImagePlan,
    japanese: &FrontendSources,
    english: &FrontendSources,
) -> ParityReport {
    let planned_japanese = plan
        .banners
        .iter()
        .map(|(image_path, banner)| (image_path.as_str(), banner.japanese.clone()));
    let planned_english = plan.banners.iter().map(|(image_path, banner)| {
        let url = banner.english.as_ref().map(|source| source.url.clone());
        (image_path.as_str(), url)
    });

    ParityReport {
        japanese: compare_locale(planned_japanese, japanese, SyncedLocale::Japanese),
        english: compare_locale(planned_english, english, SyncedLocale::English),
    }
}

fn compare_locale<'a>(
    planned: impl Iterator<Item = (&'a str, Option<String>)>,
    recorded: &FrontendSources,
    locale: SyncedLocale,
) -> LocaleParity {
    let mut parity = LocaleParity::default();
    let mut covered = BTreeSet::new();

    for (image_path, planned_url) in planned {
        let state_key = format!("src/{}", frontend_overlay_path(image_path, locale));
        let Some(recorded_value) = recorded.get(&state_key) else {
            continue;
        };
        covered.insert(state_key);

        let recorded_url = without_fit(recorded_value);
        let same = planned_url
            .as_deref()
            .is_some_and(|url| is_same_source(url, recorded_url));
        if same {
            parity.matching += 1;
        } else {
            parity.mismatches.push(SourceMismatch {
                image_path: image_path.to_string(),
                planned: planned_url,
                recorded: recorded_url.to_string(),
            });
        }
    }

    parity.unplanned = recorded
        .keys()
        .filter(|key| !covered.contains(*key))
        .cloned()
        .collect();
    parity
}

fn without_fit(value: &str) -> &str {
    value.rsplit_once('#').map_or(value, |(url, _)| url)
}

/// The sync stored the thumbnail it preferred for a campaign header.
fn is_same_source(planned: &str, recorded: &str) -> bool {
    planned == recorded || planned.replace("/Header/header_", "/Thumbnail/banner_") == recorded
}

#[cfg(test)]
mod tests {
    use super::super::{EnglishSource, PlannedBanner};
    use super::*;

    #[test]
    fn the_report_separates_matches_mismatches_and_unplanned_art() {
        let mut plan = BannerImagePlan::default();
        plan.banners.insert(
            "assets/images/campaign/100.webp".to_string(),
            PlannedBanner {
                japanese: Some("https://jp/100.png".to_string()),
                english: Some(EnglishSource {
                    url: "https://en/Header/header_1.png".to_string(),
                    campaign: true,
                }),
                cdn_gacha_ids: Vec::new(),
                prefers_english: true,
            },
        );
        plan.banners.insert(
            "assets/images/story/01.webp".to_string(),
            PlannedBanner::default(),
        );
        let japanese = FrontendSources::from([
            (
                "src/assets/timeline-images/jp/images/campaign/100.webp".to_string(),
                "https://jp/100.png#v2".to_string(),
            ),
            (
                "src/assets/timeline-images/jp/images/story/01.webp".to_string(),
                "https://jp/01.png#v2".to_string(),
            ),
            (
                "src/assets/timeline-images/jp/images/gone.webp".to_string(),
                "https://jp/gone.png#v2".to_string(),
            ),
        ]);
        let english = FrontendSources::from([(
            "src/assets/timeline-images/en/images/campaign/100.webp".to_string(),
            "https://en/Thumbnail/banner_1.png#v9".to_string(),
        )]);

        let report = compare_with_frontend_sources(&plan, &japanese, &english);

        assert_eq!(report.japanese.matching, 1);
        assert_eq!(report.japanese.mismatches.len(), 1);
        assert_eq!(
            report.japanese.unplanned,
            vec!["src/assets/timeline-images/jp/images/gone.webp"]
        );
        assert_eq!(report.english.matching, 1);
    }
}

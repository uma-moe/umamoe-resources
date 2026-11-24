//! What was published for each banner, kept in the bucket, and what a run tries next.

use super::site::SiteOverlays;
use crate::assets::types::Bucket;
use crate::generators::timeline_images::{EnglishSource, PlannedBanner};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const RECORDS_KEY: &str = "_state/timeline-art/index.json";

const RECORDS_SCHEMA: u32 = 1;

/// Raised when a fit changes, so every banner is rebuilt from its source.
pub(super) const FIT_REVISION: u32 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub(super) struct TimelineArtRecords {
    pub(super) schema: u32,
    /// By the timeline's `image_path`.
    pub(super) banners: BTreeMap<String, BannerRecord>,
    /// English sources past their release window that never appeared; they are not asked again.
    pub(super) retired: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(super) struct BannerRecord {
    pub(super) variant: Variant,
    /// The planned source this picture answers. A copy of the site's English art answers only
    /// its own path, so the planned English source is still asked for.
    pub(super) planned: String,
    /// The URL the picture was downloaded from.
    pub(super) source: String,
    pub(super) revision: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) size: Option<(u32, u32)>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Variant {
    English,
    Japanese,
    /// The site's own file, for a banner without an official source.
    Original,
}

/// One way to get a banner's picture, tried in order until one works.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Attempt {
    English(EnglishSource),
    Japanese(String),
    /// The file the site serves at `path`.
    SiteCopy {
        path: String,
        variant: Variant,
        planned: String,
    },
}

/// Earlier runs briefly published in-game `gachaselect` CDN textures under `planned: "cdn:..."`;
/// ignore those records so they are replaced by official webview art (English or Japanese).
pub(super) fn isLegacyCdnRecord(record: &BannerRecord) -> bool {
    record.planned.starts_with("cdn:")
}

/// A banner as a run finds it.
pub(super) struct BannerState<'a> {
    pub(super) image_path: &'a str,
    pub(super) planned: &'a PlannedBanner,
    pub(super) record: Option<&'a BannerRecord>,
    pub(super) site: SiteOverlays<'a>,
}

/// What to try for a banner, best first; nothing when it already shows its best picture.
/// English art is never traded back for Japanese, and the site's copies are only asked for a
/// banner that has no picture yet.
pub(super) fn attempts(banner: &BannerState, retired: &BTreeSet<String>) -> Vec<Attempt> {
    let record = banner.record.filter(|record| !isLegacyCdnRecord(record));
    let is_current = |variant: Variant, planned: &str| {
        record.is_some_and(|record| {
            record.variant == variant
                && record.planned == planned
                && record.revision == FIT_REVISION
        })
    };
    let first_picture = record.is_none();
    let mut attempts = Vec::new();

    let english = banner
        .planned
        .english
        .as_ref()
        .filter(|english| !retired.contains(&english.url));
    if let Some(english) = english {
        if is_current(Variant::English, &english.url) {
            return Vec::new();
        }
        attempts.push(Attempt::English(english.clone()));
    }
    let site_english = banner
        .site
        .english
        .filter(|_| first_picture && banner.planned.prefers_english);
    if let Some(path) = site_english {
        // It may be older art than the planned source, such as last year's cup of a meeting.
        attempts.push(Attempt::SiteCopy {
            path: path.to_string(),
            variant: Variant::English,
            planned: path.to_string(),
        });
    }

    let shows_english = record.is_some_and(|record| record.variant == Variant::English);
    if shows_english {
        return attempts;
    }

    if let Some(japanese) = &banner.planned.japanese {
        if is_current(Variant::Japanese, japanese) {
            return attempts;
        }
        attempts.push(Attempt::Japanese(japanese.clone()));
    }
    if let Some(path) = banner.site.japanese.filter(|_| first_picture) {
        // The site synced its Japanese copies from the same sources the plan picks.
        let planned = banner.planned.japanese.as_deref().unwrap_or(path);
        attempts.push(Attempt::SiteCopy {
            path: path.to_string(),
            variant: Variant::Japanese,
            planned: planned.to_string(),
        });
    }

    if first_picture {
        attempts.push(Attempt::SiteCopy {
            path: banner.image_path.to_string(),
            variant: Variant::Original,
            planned: banner.image_path.to_string(),
        });
    }
    attempts
}

pub(super) async fn loadRecords(
    client: &reqwest::Client,
    bucket: &Bucket,
) -> Result<TimelineArtRecords> {
    let Some(bytes) = bucket.getObject(client, RECORDS_KEY).await? else {
        return Ok(TimelineArtRecords {
            schema: RECORDS_SCHEMA,
            ..TimelineArtRecords::default()
        });
    };
    serde_json::from_slice(&bytes)
        .context("the timeline art records in the bucket are not valid JSON")
}

pub(super) async fn saveRecords(
    client: &reqwest::Client,
    bucket: &Bucket,
    records: &TimelineArtRecords,
) -> Result<()> {
    let body = serde_json::to_vec(records)?;
    bucket
        .putObject(client, RECORDS_KEY, body, "application/json")
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE_PATH: &str = "assets/images/campaign/100.webp";
    const SITE_ENGLISH: &str = "assets/timeline-images/en/images/campaign/100.webp";
    const SITE_JAPANESE: &str = "assets/timeline-images/jp/images/campaign/100.webp";
    const BOTH_SITE_COPIES: SiteOverlays<'static> = SiteOverlays {
        english: Some(SITE_ENGLISH),
        japanese: Some(SITE_JAPANESE),
    };

    fn planned(japanese: Option<&str>, english: Option<&str>) -> PlannedBanner {
        PlannedBanner {
            japanese: japanese.map(str::to_string),
            english: english.map(|url| EnglishSource {
                url: url.to_string(),
                fallback_urls: Vec::new(),
                campaign: true,
            }),
            prefers_english: true,
        }
    }

    fn record(variant: Variant, planned: &str) -> BannerRecord {
        BannerRecord {
            variant,
            planned: planned.to_string(),
            source: planned.to_string(),
            revision: FIT_REVISION,
            size: Some((512, 200)),
        }
    }

    fn attemptsFor(
        planned: &PlannedBanner,
        record: Option<&BannerRecord>,
        site: SiteOverlays,
        retired: &BTreeSet<String>,
    ) -> Vec<Attempt> {
        let banner = BannerState {
            image_path: IMAGE_PATH,
            planned,
            record,
            site,
        };
        attempts(&banner, retired)
    }

    fn siteCopy(path: &str, variant: Variant, planned: &str) -> Attempt {
        Attempt::SiteCopy {
            path: path.to_string(),
            variant,
            planned: planned.to_string(),
        }
    }

    #[test]
    fn aNewBannerTriesItsSourcesThenTheSitesCopies() {
        let banner = planned(Some("https://jp/a.png"), Some("https://en/a.png"));

        let tried = attemptsFor(&banner, None, BOTH_SITE_COPIES, &BTreeSet::new());

        assert_eq!(
            tried,
            vec![
                Attempt::English(banner.english.clone().unwrap()),
                siteCopy(SITE_ENGLISH, Variant::English, SITE_ENGLISH),
                Attempt::Japanese("https://jp/a.png".to_string()),
                siteCopy(SITE_JAPANESE, Variant::Japanese, "https://jp/a.png"),
                siteCopy(IMAGE_PATH, Variant::Original, IMAGE_PATH),
            ]
        );
    }

    #[test]
    fn aNewBannerKeepsTheSitesEnglishArtWithoutAnEnglishSource() {
        let banner = planned(Some("https://jp/a.png"), None);

        let tried = attemptsFor(&banner, None, BOTH_SITE_COPIES, &BTreeSet::new());

        assert_eq!(
            tried[0],
            siteCopy(SITE_ENGLISH, Variant::English, SITE_ENGLISH)
        );
    }

    #[test]
    fn aPredictedMeetingSkipsTheSitesEnglishArt() {
        let mut banner = planned(Some("https://jp/a.png"), None);
        banner.prefers_english = false;

        let tried = attemptsFor(&banner, None, BOTH_SITE_COPIES, &BTreeSet::new());

        assert_eq!(tried[0], Attempt::Japanese("https://jp/a.png".to_string()));
    }

    #[test]
    fn theSitesEnglishCopyWaitsForItsPlannedSource() {
        let banner = planned(Some("https://jp/a.png"), Some("https://en/a.png"));
        let shown = record(Variant::English, SITE_ENGLISH);

        let tried = attemptsFor(&banner, Some(&shown), BOTH_SITE_COPIES, &BTreeSet::new());

        assert_eq!(
            tried,
            vec![Attempt::English(banner.english.clone().unwrap())]
        );
    }

    #[test]
    fn theSitesEnglishCopyStaysWithoutAnEnglishSource() {
        let banner = planned(Some("https://jp/a.png"), None);
        let shown = record(Variant::English, SITE_ENGLISH);

        let tried = attemptsFor(&banner, Some(&shown), BOTH_SITE_COPIES, &BTreeSet::new());

        assert!(tried.is_empty());
    }

    #[test]
    fn aCurrentBannerNeedsNothing() {
        let banner = planned(Some("https://jp/a.png"), Some("https://en/a.png"));
        let shown = record(Variant::English, "https://en/a.png");

        let tried = attemptsFor(&banner, Some(&shown), BOTH_SITE_COPIES, &BTreeSet::new());

        assert!(tried.is_empty());
    }

    #[test]
    fn japaneseArtWaitsForItsEnglishReplacement() {
        let banner = planned(Some("https://jp/a.png"), Some("https://en/a.png"));
        let shown = record(Variant::Japanese, "https://jp/a.png");

        let tried = attemptsFor(&banner, Some(&shown), BOTH_SITE_COPIES, &BTreeSet::new());

        assert_eq!(
            tried,
            vec![Attempt::English(banner.english.clone().unwrap())]
        );
    }

    #[test]
    fn aRetiredEnglishSourceIsNotAskedAgain() {
        let banner = planned(Some("https://jp/a.png"), Some("https://en/a.png"));
        let shown = record(Variant::Japanese, "https://jp/a.png");
        let retired = BTreeSet::from(["https://en/a.png".to_string()]);

        let tried = attemptsFor(&banner, Some(&shown), BOTH_SITE_COPIES, &retired);

        assert!(tried.is_empty());
    }

    #[test]
    fn englishArtIsKeptWhenItsSourceChanges() {
        let banner = planned(Some("https://jp/a.png"), Some("https://en/new.png"));
        let shown = record(Variant::English, "https://en/old.png");

        let tried = attemptsFor(&banner, Some(&shown), BOTH_SITE_COPIES, &BTreeSet::new());

        assert_eq!(
            tried,
            vec![Attempt::English(banner.english.clone().unwrap())]
        );
    }

    #[test]
    fn theSitesOriginalGivesWayToOfficialArt() {
        let banner = planned(Some("https://jp/a.png"), None);
        let shown = record(Variant::Original, IMAGE_PATH);

        let tried = attemptsFor(
            &banner,
            Some(&shown),
            SiteOverlays::default(),
            &BTreeSet::new(),
        );

        assert_eq!(
            tried,
            vec![Attempt::Japanese("https://jp/a.png".to_string())]
        );
    }

    #[test]
    fn aNewFitRebuildsTheSamePicture() {
        let banner = planned(Some("https://jp/a.png"), None);
        let mut shown = record(Variant::Japanese, "https://jp/a.png");
        shown.revision = FIT_REVISION - 1;

        let tried = attemptsFor(
            &banner,
            Some(&shown),
            SiteOverlays::default(),
            &BTreeSet::new(),
        );

        assert_eq!(
            tried,
            vec![Attempt::Japanese("https://jp/a.png".to_string())]
        );
    }

    #[test]
    fn aLegacyCdnRecordIsReplacedByOfficialEnglishOrJapaneseArt() {
        let with_english = planned(Some("https://jp/30151.png"), Some("https://en/30151.png"));
        let jp_only = planned(Some("https://jp/30152.png"), None);
        let legacy_cdn = record(
            Variant::English,
            "cdn:gachaselect/30151/img_bnr_gacha_30151@111",
        );

        assert_eq!(
            attemptsFor(
                &with_english,
                Some(&legacy_cdn),
                SiteOverlays::default(),
                &BTreeSet::new(),
            ),
            vec![
                Attempt::English(with_english.english.clone().unwrap()),
                Attempt::Japanese("https://jp/30151.png".to_string()),
                siteCopy(IMAGE_PATH, Variant::Original, IMAGE_PATH),
            ]
        );
        assert_eq!(
            attemptsFor(
                &jp_only,
                Some(&legacy_cdn),
                SiteOverlays::default(),
                &BTreeSet::new(),
            ),
            vec![
                Attempt::Japanese("https://jp/30152.png".to_string()),
                siteCopy(IMAGE_PATH, Variant::Original, IMAGE_PATH),
            ]
        );
    }
}

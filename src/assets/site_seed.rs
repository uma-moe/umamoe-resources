//! A one-time copy of art the site showed before it loaded pictures from the bucket, for keys no
//! game-art family builds yet: JP-only cards, skills and items, the scenario logos and the skill
//! point icon. A key the bucket already holds is left alone, and game art replaces a seeded copy
//! once the game ships its own.

use super::contentType;
use super::timeline_art::{isWebp, SourceFetcher};
use super::types::Bucket;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use tracing::{info, warn};

/// Made from the frontend's asset folders when the bucket took over; they cannot be read again
/// once the site stops serving them.
const SEED_LIST: &[u8] = include_bytes!("site_seed.json");

const SEED_SCHEMA: u32 = 1;

const RECORDS_KEY: &str = "_state/site-seed/index.json";

const RECORDS_SCHEMA: u32 = 1;

const SITE_ORIGIN: &str = "https://uma.moe";

/// Records are saved after this many copies, so a stopped run does not copy them again.
const SAVE_EVERY: usize = 32;

/// Key families the site links; a seed key outside them would never be shown.
const SEEDED_FAMILIES: &[(&str, &str)] = &[
    ("art/umas/", "/portrait.webp"),
    ("art/support-cards/", "/half.webp"),
    ("art/support-cards/", "/card.webp"),
    ("art/skills/icons/", ".webp"),
    ("art/items/", ".webp"),
    ("art/scenarios/", "/logo.webp"),
    ("art/ui/skill-points", ".webp"),
];

#[derive(Debug, Deserialize)]
struct SeedList {
    schema: u32,
    /// The site path of each file, by bucket key.
    files: BTreeMap<String, String>,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
struct SeedRecords {
    schema: u32,
    /// Copied from the site: its path by bucket key.
    seeded: BTreeMap<String, String>,
    /// Keys the bucket already held when the seed reached them.
    existing: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub struct SiteSeedSummary {
    pub files: usize,
    pub seeded: usize,
    pub existing: usize,
    /// Keys a dry run would copy.
    pub missing: usize,
    /// Copies that failed; retried on the next run.
    pub failures: usize,
}

impl SiteSeedSummary {
    /// Whether every seed key has been copied or found in the bucket.
    pub fn isComplete(&self) -> bool {
        self.missing == 0 && self.failures == 0
    }
}

/// Copies the seed files the bucket does not hold yet and records every decision.
pub async fn publishSiteSeed(bucket: &Bucket, dry_run: bool) -> Result<SiteSeedSummary> {
    let seed = readSeedList(SEED_LIST)?;
    let client = reqwest::Client::new();
    let mut records = loadRecords(&client, bucket).await?;
    let mut fetcher = SourceFetcher::new()?;
    let undecided = undecidedFiles(&seed, &records);
    let mut summary = SiteSeedSummary {
        files: seed.files.len(),
        ..SiteSeedSummary::default()
    };
    let mut unsaved = 0;

    for (key, site_path) in undecided {
        if bucket.objectEtag(&client, key).await?.is_some() {
            records.existing.insert(key.to_string());
            summary.existing += 1;
            unsaved += 1;
            continue;
        }
        if dry_run {
            summary.missing += 1;
            continue;
        }

        match copySiteFile(&mut fetcher, site_path).await {
            Ok(bytes) => {
                bucket
                    .putObject(&client, key, bytes, contentType(key))
                    .await?;
                records
                    .seeded
                    .insert(key.to_string(), site_path.to_string());
                summary.seeded += 1;
                unsaved += 1;
            }
            Err(error) => {
                summary.failures += 1;
                warn!(key, error = format!("{error:#}"), "site seed file failed");
            }
        }

        if unsaved >= SAVE_EVERY && !dry_run {
            saveRecords(&client, bucket, &records).await?;
            unsaved = 0;
        }
    }

    if unsaved > 0 && !dry_run {
        saveRecords(&client, bucket, &records).await?;
    }
    info!(
        files = summary.files,
        seeded = summary.seeded,
        existing = summary.existing,
        missing = summary.missing,
        failures = summary.failures,
        dry_run,
        "site seed pass"
    );
    Ok(summary)
}

async fn copySiteFile(fetcher: &mut SourceFetcher, site_path: &str) -> Result<Vec<u8>> {
    let url = format!("{SITE_ORIGIN}/{site_path}");
    let Some(bytes) = fetcher.download(&url).await? else {
        bail!("{url} is gone");
    };
    if !isWebp(&bytes) {
        bail!("{url} is not a WebP picture");
    }
    Ok(bytes)
}

fn readSeedList(bytes: &[u8]) -> Result<SeedList> {
    let seed: SeedList =
        serde_json::from_slice(bytes).context("the site seed list is not valid")?;
    if seed.schema != SEED_SCHEMA {
        bail!("the site seed list has schema {}", seed.schema);
    }

    for (key, site_path) in &seed.files {
        if !isSeededKey(key) {
            bail!("site seed key {key} is not a family the site links");
        }
        if site_path.starts_with('/') || site_path.contains("..") || !site_path.ends_with(".webp") {
            bail!("site seed path {site_path} is not a WebP file below the site root");
        }
    }
    Ok(seed)
}

fn isSeededKey(key: &str) -> bool {
    SEEDED_FAMILIES.iter().any(|(prefix, suffix)| {
        key.strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(suffix))
            .is_some_and(|name| !name.contains('/'))
    })
}

/// Seed files neither copied nor found in the bucket before.
fn undecidedFiles<'a>(seed: &'a SeedList, records: &SeedRecords) -> Vec<(&'a str, &'a str)> {
    seed.files
        .iter()
        .filter(|(key, _)| {
            !records.seeded.contains_key(key.as_str()) && !records.existing.contains(key.as_str())
        })
        .map(|(key, site_path)| (key.as_str(), site_path.as_str()))
        .collect()
}

async fn loadRecords(client: &reqwest::Client, bucket: &Bucket) -> Result<SeedRecords> {
    let Some(bytes) = bucket.getObject(client, RECORDS_KEY).await? else {
        return Ok(SeedRecords {
            schema: RECORDS_SCHEMA,
            ..SeedRecords::default()
        });
    };
    serde_json::from_slice(&bytes).context("the site seed records in the bucket are not valid JSON")
}

async fn saveRecords(
    client: &reqwest::Client,
    bucket: &Bucket,
    records: &SeedRecords,
) -> Result<()> {
    let body = serde_json::to_vec(records)?;
    bucket
        .putObject(client, RECORDS_KEY, body, "application/json")
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seedList(files: &[(&str, &str)]) -> SeedList {
        SeedList {
            schema: SEED_SCHEMA,
            files: files
                .iter()
                .map(|(key, path)| (key.to_string(), path.to_string()))
                .collect(),
        }
    }

    #[test]
    fn theShippedSeedListIsValid() {
        let seed = readSeedList(SEED_LIST).unwrap();

        assert!(seed.files.len() > 500);
        assert_eq!(
            seed.files["art/scenarios/1/logo.webp"],
            "assets/images/scenario/ura_finals_logo.webp"
        );
        assert_eq!(
            seed.files["art/ui/skill-points.webp"],
            "game-assets/textures/hint.webp"
        );
    }

    #[test]
    fn onlyFamiliesTheSiteLinksAreSeeded() {
        assert!(isSeededKey("art/umas/100202/portrait.webp"));
        assert!(isSeededKey("art/support-cards/10088/half.webp"));
        assert!(isSeededKey("art/skills/icons/20045.webp"));
        assert!(isSeededKey("art/ui/skill-points.webp"));

        assert!(!isSeededKey("art/umas/100202/stand.webp"));
        assert!(!isSeededKey("art/timeline/campaign/100.webp"));
        assert!(!isSeededKey("art/skills/icons/sub/20045.webp"));
    }

    #[test]
    fn aSeedPathMustBeAWebpFileBelowTheSiteRoot() {
        for path in [
            "/game-assets/a.webp",
            "assets/../secret.webp",
            "assets/a.png",
        ] {
            let json = format!(r#"{{"schema":1,"files":{{"art/items/00001.webp":"{path}"}}}}"#);

            assert!(readSeedList(json.as_bytes()).is_err(), "{path}");
        }
    }

    #[test]
    fn decidedKeysAreNotAskedAgain() {
        let seed = seedList(&[
            (
                "art/items/00001.webp",
                "assets/images/item/item_icon_00001.webp",
            ),
            (
                "art/items/00002.webp",
                "assets/images/item/item_icon_00002.webp",
            ),
            (
                "art/items/00003.webp",
                "assets/images/item/item_icon_00003.webp",
            ),
        ]);
        let records = SeedRecords {
            schema: RECORDS_SCHEMA,
            seeded: BTreeMap::from([(
                "art/items/00001.webp".to_string(),
                "assets/images/item/item_icon_00001.webp".to_string(),
            )]),
            existing: BTreeSet::from(["art/items/00002.webp".to_string()]),
        };

        let undecided = undecidedFiles(&seed, &records);

        assert_eq!(
            undecided,
            vec![(
                "art/items/00003.webp",
                "assets/images/item/item_icon_00003.webp"
            )]
        );
    }
}

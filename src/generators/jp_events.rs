use anyhow::{Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const UMAPYOI_ARCHIVE: &[u8] = include_bytes!("../jp_data/umapyoi_archive.json");
const CHARACTER_BANNERS: &[u8] = include_bytes!("../jp_data/timeline_character_banners.json");
const SUPPORT_BANNERS: &[u8] = include_bytes!("../jp_data/timeline_support_banners.json");
const PAID_BANNERS: &[u8] = include_bytes!("../jp_data/timeline_paid_banners.json");
const ALGORITHM_VERSION: u8 = 2;

pub fn generate() -> Result<Value> {
    let mut archive: Value = serde_json::from_slice(UMAPYOI_ARCHIVE)
        .context("failed to parse bundled umapyoi archive")?;
    let known_ids = known_timeline_gacha_ids()?;
    let candidates = additional_gacha_banners(&archive, &known_ids);
    archive
        .as_object_mut()
        .context("bundled umapyoi archive must be a JSON object")?
        .insert("additional_gacha_banners".to_string(), candidates);
    Ok(archive)
}

pub fn version_hash() -> String {
    let mut digest = Sha256::new();
    digest.update(UMAPYOI_ARCHIVE);
    digest.update([ALGORITHM_VERSION]);
    hex::encode(digest.finalize())
}

fn known_timeline_gacha_ids() -> Result<BTreeSet<i64>> {
    let mut ids = BTreeSet::new();
    for (name, bytes) in [
        ("character", CHARACTER_BANNERS),
        ("support", SUPPORT_BANNERS),
        ("paid", PAID_BANNERS),
    ] {
        let banners: Vec<Value> = serde_json::from_slice(bytes)
            .with_context(|| format!("failed to parse bundled {name} timeline banners"))?;
        ids.extend(
            banners
                .iter()
                .filter_map(|banner| banner.get("gacha_id").and_then(Value::as_i64)),
        );
    }
    Ok(ids)
}

fn additional_gacha_banners(archive: &Value, known_ids: &BTreeSet<i64>) -> Value {
    let mut candidates = BTreeMap::<i64, Value>::new();
    for post in archive
        .get("news")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let posted_at = post
            .get("posted_at")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let post_id = post.get("post_id").and_then(Value::as_i64);
        let title = post.get("title").and_then(Value::as_str);
        for banner in post
            .get("gacha_banners")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(gacha_id) = banner.get("gacha_id").and_then(Value::as_i64) else {
                continue;
            };
            if known_ids.contains(&gacha_id) {
                continue;
            }
            let image_url = banner
                .get("image_url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let candidate = json!({
                "gacha_id": gacha_id,
                "type": inferred_gacha_type(gacha_id),
                "jp_start_at": posted_at,
                "image_url": image_url,
                "source_post_id": post_id,
                "title": title,
                "source": "umapyoi_news"
            });
            let should_replace = candidates
                .get(&gacha_id)
                .and_then(|candidate| candidate.get("jp_start_at"))
                .and_then(Value::as_str)
                .is_none_or(|existing| posted_at > existing);
            if should_replace {
                candidates.insert(gacha_id, candidate);
            }
        }
    }
    Value::Array(candidates.into_values().collect())
}

fn inferred_gacha_type(gacha_id: i64) -> &'static str {
    match gacha_id {
        30_000..=39_999 if gacha_id % 2 == 0 => "character_banner",
        30_000..=39_999 => "support_card_banner",
        50_000..=59_999 => "paid_banner",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_archive_is_valid_json() {
        let archive = super::generate().expect("bundled umapyoi archive should parse");
        assert!(archive
            .get("news")
            .and_then(|value| value.as_array())
            .is_some());
        assert!(archive
            .get("additional_gacha_banners")
            .and_then(|value| value.as_array())
            .is_some_and(|banners| !banners.is_empty()));
        assert!(archive
            .get("gachas")
            .and_then(|value| value.as_array())
            .is_some());
        assert!(archive
            .get("supports")
            .and_then(|value| value.as_array())
            .is_some());
    }
}

//! Where each banner is served: the timeline's `image_path` below `art/timeline/`.

use std::collections::{BTreeMap, BTreeSet};

pub(super) const TIMELINE_FOLDER: &str = "art/timeline/";

pub(super) const INDEX_KEY: &str = "art/timeline/index.json";

/// The site's two trees of banners, whose top folders never overlap.
const SITE_BANNER_TREES: &[&str] = &["assets/images/", "assets/timeline-images/"];

/// `assets/images/campaign/100.webp` is served at `art/timeline/campaign/100.webp`.
pub(super) fn timelineKey(image_path: &str) -> Option<String> {
    let relative = SITE_BANNER_TREES
        .iter()
        .find_map(|tree| image_path.strip_prefix(tree))?;
    let is_file = !relative.is_empty() && !relative.ends_with('/');
    let stays_inside = relative
        .split('/')
        .all(|part| !part.is_empty() && part != "..");
    (is_file && stays_inside).then(|| format!("{TIMELINE_FOLDER}{relative}"))
}

/// Keys by `image_path`. A path that gets no key, or a key another path already took, is
/// returned apart so the run can report it.
pub(super) fn bannerKeys<'a>(
    image_paths: impl IntoIterator<Item = &'a str>,
) -> (BTreeMap<&'a str, String>, Vec<&'a str>) {
    let mut keys: BTreeMap<&str, String> = BTreeMap::new();
    let mut taken = BTreeSet::new();
    let mut unplaced = Vec::new();
    for image_path in image_paths {
        match timelineKey(image_path) {
            Some(key) if taken.insert(key.clone()) => {
                keys.insert(image_path, key);
            }
            _ => unplaced.push(image_path),
        }
    }
    (keys, unplaced)
}

/// The link a document gives for a key, from the bucket root.
pub(super) fn rootLink(key: &str) -> String {
    format!("/{key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bannersKeepTheirPathBelowTheTimelineFolder() {
        let cases = [
            (
                "assets/images/character/banner/2021_30002.webp",
                "art/timeline/character/banner/2021_30002.webp",
            ),
            (
                "assets/images/campaign/100.webp",
                "art/timeline/campaign/100.webp",
            ),
            (
                "assets/timeline-images/events/campaign/1001.webp",
                "art/timeline/events/campaign/1001.webp",
            ),
            (
                "assets/timeline-images/gacha/support/30230.webp",
                "art/timeline/gacha/support/30230.webp",
            ),
        ];

        for (image_path, key) in cases {
            assert_eq!(
                timelineKey(image_path).as_deref(),
                Some(key),
                "{image_path}"
            );
        }
    }

    #[test]
    fn pathsOutsideTheBannerTreesGetNoKey() {
        assert_eq!(timelineKey("https://gametora.com/legend.png"), None);
        assert_eq!(timelineKey("assets/images/legend/"), None);
        assert_eq!(timelineKey("assets/images/../logo.webp"), None);
    }

    #[test]
    fn aKeyIsGivenOnce() {
        let paths = [
            "assets/images/events/campaign/1.webp",
            "assets/timeline-images/events/campaign/1.webp",
            "elsewhere.webp",
        ];

        let (keys, unplaced) = bannerKeys(paths);

        assert_eq!(keys.len(), 1);
        assert_eq!(
            unplaced,
            vec![
                "assets/timeline-images/events/campaign/1.webp",
                "elsewhere.webp"
            ]
        );
    }
}

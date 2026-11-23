//! Reading the picture URLs of news posts.

use super::{EnglishPost, PostImage};

pub(super) const ENGLISH_ASSET_HOST: &str = "https://assets-webview-umamusume-en.akamaized.net/";

/// Where official English headers live; a banner's header is `<identity>_L<release>.png`.
pub(super) const ENGLISH_HEADER_ROOT: &str =
    "https://assets-webview-umamusume-en.akamaized.net/contents/assets/images/uploads/Header";

pub(super) fn is_http(url: &str) -> bool {
    url.starts_with("http:") || url.starts_with("https:")
}

pub(super) fn decode_html(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&#x2F;", "/")
        .replace("&quot;", "\"")
}

/// The picture of a post most likely to be its banner.
pub(super) fn preferred_post_image(images: &[PostImage]) -> Option<&str> {
    let mut candidates: Vec<&PostImage> = Vec::new();
    for image in images {
        let seen = candidates.iter().any(|known| known.url == image.url);
        if is_http(&image.url) && !seen {
            candidates.push(image);
        }
    }
    candidates.sort_by_key(|image| std::cmp::Reverse(image_score(image)));
    candidates.first().map(|image| image.url.as_str())
}

fn image_score(image: &PostImage) -> i32 {
    let url = image.url.to_ascii_lowercase();
    let points = |earned: bool, points: i32| if earned { points } else { 0 };

    points(image.likely_banner, 30)
        + points(url.contains("/thumbnail/banner_"), 25)
        + points(image.field_path == "$.image", 20)
        + points(image.field_path == "$.article_image", 15)
        + points(url.contains("/header/"), 12)
        + points(url.contains("gacha_banner_"), 10)
        + points(url.contains("header_"), 6)
        - points(url.contains("announce_banner_"), 8)
}

/// The name an official picture keeps across its uploads: `banner_30310` for
/// `…/Thumbnail/banner_30310_L1753135200.png`, lower case.
pub(super) fn asset_identity(url: &str) -> Option<String> {
    let url = decode_html(url).to_ascii_lowercase();
    let folders = ["/thumbnail/", "/header/"];

    url.match_indices('/').find_map(|(position, _)| {
        let rest = &url[position..];
        let folder = folders.iter().find(|folder| rest.starts_with(**folder))?;
        identity_in_file_name(&rest[folder.len()..])
    })
}

/// The file name must end the URL or its path; a release suffix `_l<digits>` is dropped.
fn identity_in_file_name(rest: &str) -> Option<String> {
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    if rest[end..].starts_with('/') {
        return None;
    }

    let file_name = &rest[..end];
    let stem = [".png", ".jpg", ".jpeg", ".webp"]
        .iter()
        .find_map(|extension| file_name.strip_suffix(extension))
        .filter(|stem| !stem.is_empty())?;
    Some(without_release_suffix(stem).to_string())
}

fn without_release_suffix(stem: &str) -> &str {
    let Some(marker) = stem.rfind("_l") else {
        return stem;
    };
    let digits = &stem[marker + 2..];
    let is_release =
        marker > 0 && !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit());
    if is_release {
        &stem[..marker]
    } else {
        stem
    }
}

/// Every official picture a post shows: its image and those its message links.
pub(super) fn official_post_asset_urls(post: &EnglishPost) -> Vec<String> {
    let mut urls = Vec::new();
    if let Some(image) = post.image.as_deref().filter(|image| is_http(image)) {
        urls.push(decode_html(image));
    }
    if let Some(message) = post.message.as_deref() {
        urls.extend(linked_english_assets(message).map(|url| decode_html(url)));
    }

    let mut unique: Vec<String> = Vec::new();
    for url in urls {
        if !unique.contains(&url) {
            unique.push(url);
        }
    }
    unique
}

fn linked_english_assets(message: &str) -> impl Iterator<Item = &str> {
    let lower = message.to_ascii_lowercase();
    let starts: Vec<usize> = lower
        .match_indices(ENGLISH_ASSET_HOST)
        .map(|(start, _)| start)
        .collect();

    starts.into_iter().filter_map(move |start| {
        let rest = &message[start + ENGLISH_ASSET_HOST.len()..];
        let length = rest.find(['"', '\'', '<', ' ', '>']).unwrap_or(rest.len());
        (length > 0).then(|| &message[start..start + ENGLISH_ASSET_HOST.len() + length])
    })
}

/// The story event banner id in a picture URL: `30120` in `…/Thumbnail/banner_30120.png`.
pub(super) fn story_banner_id(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    let prefixes = [
        "/thumbnail/banner_",
        "/thumbnail/header_",
        "/header/banner_",
        "/header/header_",
    ];

    lower.match_indices('/').find_map(|(position, _)| {
        let rest = &lower[position..];
        let prefix = prefixes.iter().find(|prefix| rest.starts_with(**prefix))?;
        story_id_digits(&rest[prefix.len()..])
    })
}

fn story_id_digits(rest: &str) -> Option<String> {
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (digits.starts_with("301") && digits.len() > 3).then_some(digits)
}

/// A picture made for one story event: `banner_301…` or `header_301…`.
pub(super) fn is_story_event_picture(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["banner_", "header_"].iter().any(|prefix| {
        lower
            .match_indices(prefix)
            .any(|(start, _)| story_id_digits(&lower[start + prefix.len()..]).is_some())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(url: &str, field_path: &str, likely_banner: bool) -> PostImage {
        PostImage {
            url: url.to_string(),
            field_path: field_path.to_string(),
            likely_banner,
        }
    }

    #[test]
    fn the_thumbnail_banner_of_apost_is_preferred() {
        let images = [
            image("https://x/Header/header_1.png", "$.image", false),
            image("https://x/Thumbnail/banner_1.png", "$.message", true),
            image("ftp://x/banner.png", "$.image", true),
        ];

        assert_eq!(
            preferred_post_image(&images),
            Some("https://x/Thumbnail/banner_1.png")
        );
    }

    #[test]
    fn an_identity_drops_the_release_suffix_and_the_query() {
        let header =
            asset_identity("https://h/uploads/Header/header_21000001_L1753135200.png?c=20250616");
        let thumbnail = asset_identity("https://h/uploads/Thumbnail/Banner_30310.PNG");
        let nested = asset_identity("https://h/Header/more/banner.png");

        assert_eq!(header.as_deref(), Some("header_21000001"));
        assert_eq!(thumbnail.as_deref(), Some("banner_30310"));
        assert_eq!(nested, None);
    }

    #[test]
    fn a_message_links_official_pictures_until_the_attribute_ends() {
        let post = EnglishPost {
            image: Some("https://assets-webview-umamusume-en.akamaized.net/a.png".to_string()),
            message: Some(
                "<img src=\"https://assets-webview-umamusume-en.akamaized.net/b.png?x=1&amp;y=2\"> \
                 <img src='https://assets-webview-umamusume-en.akamaized.net/a.png'>"
                    .to_string(),
            ),
            ..EnglishPost::default()
        };

        let urls = official_post_asset_urls(&post);

        assert_eq!(
            urls,
            vec![
                "https://assets-webview-umamusume-en.akamaized.net/a.png",
                "https://assets-webview-umamusume-en.akamaized.net/b.png?x=1&y=2",
            ]
        );
    }

    #[test]
    fn story_banner_ids_start_with_301() {
        assert_eq!(
            story_banner_id("https://h/Thumbnail/banner_30120.png").as_deref(),
            Some("30120")
        );
        assert_eq!(
            story_banner_id("https://h/Thumbnail/banner_30220.png"),
            None
        );
        assert!(is_story_event_picture("https://h/Header/HEADER_3015.png"));
        assert!(!is_story_event_picture("https://h/Header/header_301.png"));
    }
}

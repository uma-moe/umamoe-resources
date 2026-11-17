//! The official Japanese picture of each banner: the gacha's own banner, the picture the
//! timeline links, or the news post announcing the event.

use super::campaigns::campaign_semantic_key;
use super::news_images::{
    is_http, is_story_event_picture, preferred_post_image, ENGLISH_ASSET_HOST,
};
use super::{days_between, instant_ms, JapanesePost, TimelineEvent, DAY_MS};
use crate::generators::timeline::timeline_title_similarity;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// How long after a campaign post its later phases still use the post's picture.
const CAMPAIGN_PHASE_DAYS: i64 = 56;

/// Archive event types by timeline event type, for events matched to their news post.
const ARCHIVE_EVENT_TYPES: &[(&str, &str)] = &[
    ("story_event", "story_event"),
    ("champions_meeting", "champions_meeting"),
    ("legend_race", "legend_race"),
    ("league_of_heroes", "league_of_heroes"),
    ("masters_challenge", "masters_challenge"),
    ("trainer_skills_test", "trainer_skills_test"),
    ("factor_research", "factor_research"),
    ("strongest_team", "strongest_team"),
    ("racing_carnival", "racing_carnival"),
    ("scenario_release", "training_scenario"),
];

struct TypedPost<'a> {
    post: &'a JapanesePost,
    posted: i64,
    picture: &'a str,
    event_types: BTreeSet<&'a str>,
}

struct CampaignBanner<'a> {
    image_path: &'a str,
    release: i64,
    phase: Option<String>,
}

struct PostMatch<'a> {
    quality: usize,
    delta_days: f64,
    picture: &'a str,
}

/// Source URLs by `image_path`. A later event or post naming the same picture wins.
pub(super) fn japanese_sources(
    events: &[TimelineEvent],
    posts: &[JapanesePost],
) -> BTreeMap<String, String> {
    let gacha_pictures = gacha_pictures(posts);
    let typed_posts: Vec<TypedPost> = posts.iter().filter_map(typed_post).collect();
    let mut sources = BTreeMap::new();
    let mut campaigns = Vec::new();

    for event in events {
        let Some(image_path) = event
            .image_path
            .as_deref()
            .filter(|path| path.ends_with(".webp"))
        else {
            continue;
        };
        if event.image.starts_with(ENGLISH_ASSET_HOST) {
            continue;
        }

        let gacha_picture = event
            .gacha_ids()
            .into_iter()
            .find_map(|id| gacha_pictures.get(&id).copied());
        if let Some(picture) = gacha_picture {
            sources.insert(image_path.to_string(), picture.to_string());
            continue;
        }
        if is_http(&event.image) {
            sources.insert(image_path.to_string(), event.image.clone());
        }

        let Some(release) = instant_ms(&event.jp_release_date) else {
            continue;
        };
        if event.event_type == "campaign" {
            campaigns.push(CampaignBanner {
                image_path,
                release,
                phase: campaign_semantic_key(&event.title, true),
            });
            continue;
        }
        if let Some(picture) = announcing_post_picture(event, release, &typed_posts) {
            sources.insert(image_path.to_string(), picture.to_string());
        }
    }

    for post in typed_posts
        .iter()
        .filter(|post| post.event_types.contains("campaign"))
    {
        let Some(phase) = campaign_semantic_key(post.post.title.as_deref().unwrap_or(""), true)
        else {
            continue;
        };
        let phase_start = post.posted - 2 * DAY_MS;
        let phase_end = post.posted + CAMPAIGN_PHASE_DAYS * DAY_MS;
        for campaign in &campaigns {
            let same_phase = campaign.phase.as_deref() == Some(phase.as_str());
            if same_phase && (phase_start..=phase_end).contains(&campaign.release) {
                sources.insert(campaign.image_path.to_string(), post.picture.to_string());
            }
        }
    }
    sources
}

/// Each gacha's banner picture; a later post naming the gacha wins.
fn gacha_pictures(posts: &[JapanesePost]) -> HashMap<i64, &str> {
    posts
        .iter()
        .flat_map(|post| &post.gacha_banners)
        .map(|banner| (banner.gacha_id, banner.image_url.as_str()))
        .collect()
}

fn typed_post(post: &JapanesePost) -> Option<TypedPost<'_>> {
    Some(TypedPost {
        post,
        posted: post.posted_at.as_deref().and_then(instant_ms)?,
        picture: preferred_post_image(&post.images)?,
        event_types: post.event_types.iter().map(String::as_str).collect(),
    })
}

/// The picture of the post of the event's type within two days of its release, the closest in
/// title first. A post that only announces a story event with its own banner needs no title.
fn announcing_post_picture<'a>(
    event: &TimelineEvent,
    release: i64,
    posts: &'a [TypedPost<'a>],
) -> Option<&'a str> {
    let archive_type = ARCHIVE_EVENT_TYPES
        .iter()
        .find(|(event_type, _)| *event_type == event.event_type)
        .map(|(_, archive_type)| *archive_type)?;

    let mut matches: Vec<PostMatch> = Vec::new();
    for post in posts {
        let delta_days = days_between(post.posted, release);
        if !post.event_types.contains(archive_type) || delta_days > 2.0 {
            continue;
        }

        let similarity =
            timeline_title_similarity(&event.title, post.post.title.as_deref().unwrap_or(""));
        let dedicated_story = archive_type == "story_event"
            && post.event_types.len() == 1
            && is_story_event_picture(post.picture);
        if similarity == 0 && !dedicated_story {
            continue;
        }
        matches.push(PostMatch {
            quality: similarity * 10 + if dedicated_story { 30 } else { 0 },
            delta_days,
            picture: post.picture,
        });
    }

    matches.sort_by(|left, right| {
        right
            .quality
            .cmp(&left.quality)
            .then(left.delta_days.total_cmp(&right.delta_days))
    });
    matches.first().map(|best| best.picture)
}

#[cfg(test)]
mod tests {
    use super::super::{GachaBannerImage, PostImage};
    use super::*;

    fn post(title: &str, posted_at: &str, event_types: &[&str], picture: &str) -> JapanesePost {
        JapanesePost {
            title: Some(title.to_string()),
            posted_at: Some(posted_at.to_string()),
            event_types: event_types.iter().map(|value| value.to_string()).collect(),
            images: vec![PostImage {
                url: picture.to_string(),
                field_path: "$.image".to_string(),
                likely_banner: true,
            }],
            gacha_banners: Vec::new(),
        }
    }

    fn event(event_type: &str, title: &str, image_path: &str) -> TimelineEvent {
        TimelineEvent {
            event_type: event_type.to_string(),
            title: title.to_string(),
            image_path: Some(image_path.to_string()),
            jp_release_date: "2023-05-10T03:00:00Z".to_string(),
            ..TimelineEvent::default()
        }
    }

    #[test]
    fn a_gacha_banner_uses_its_gachas_picture() {
        let mut gacha_post = post(
            "Pickup",
            "2023-05-10T03:00:00+00:00",
            &["gacha"],
            "https://jp/p.png",
        );
        gacha_post.gacha_banners.push(GachaBannerImage {
            gacha_id: 30101,
            image_url: "https://jp/gacha_banner_30101.png".to_string(),
        });
        let mut banner = event(
            "character_banner",
            "Pickup",
            "assets/images/character/banner/30101.webp",
        );
        banner.gacha_id = Some(30101);
        banner.image = "https://elsewhere/30101.png".to_string();

        let sources = japanese_sources(&[banner], &[gacha_post]);

        assert_eq!(
            sources["assets/images/character/banner/30101.webp"],
            "https://jp/gacha_banner_30101.png"
        );
    }

    #[test]
    fn an_event_uses_the_closest_titled_post_of_its_type() {
        let posts = [
            post(
                "Legend Race Kikka Sho",
                "2023-05-11T03:00:00+00:00",
                &["legend_race"],
                "https://jp/a.png",
            ),
            post(
                "Legend Race Kikka Sho Final",
                "2023-05-09T03:00:00+00:00",
                &["legend_race"],
                "https://jp/b.png",
            ),
            post(
                "Kikka Sho Final",
                "2023-05-20T03:00:00+00:00",
                &["legend_race"],
                "https://jp/late.png",
            ),
        ];
        let race = event(
            "legend_race",
            "Kikka Sho Final",
            "assets/timeline-images/events/legend-race/7.webp",
        );

        let sources = japanese_sources(&[race], &posts);

        assert_eq!(
            sources["assets/timeline-images/events/legend-race/7.webp"],
            "https://jp/b.png"
        );
    }

    #[test]
    fn later_campaign_phases_use_their_phases_post() {
        let posts = [post(
            "Spring G1 Celebration Part 2",
            "2023-05-01T03:00:00+00:00",
            &["campaign"],
            "https://jp/spring2.png",
        )];
        let campaign = event(
            "campaign",
            "Spring G1 Missions Part 2",
            "assets/images/campaign/90.webp",
        );

        let sources = japanese_sources(&[campaign], &posts);

        assert_eq!(
            sources["assets/images/campaign/90.webp"],
            "https://jp/spring2.png"
        );
    }

    #[test]
    fn events_showing_english_art_are_left_to_the_english_pass() {
        let mut meeting = event(
            "champions_meeting",
            "Taurus Cup",
            "assets/timeline-images/events/champions-meeting/global-5.webp",
        );
        meeting.image = format!("{ENGLISH_ASSET_HOST}contents/x.png");

        let sources = japanese_sources(&[meeting], &[]);

        assert!(sources.is_empty());
    }
}

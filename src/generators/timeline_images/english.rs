//! The official English picture of each confirmed banner: the picture Global's announcement
//! shows, or else the header URL Global uploads for a release, which may not exist yet.

use super::campaigns::campaign_semantic_key;
use super::news_images::{
    asset_identity, decode_html, official_post_asset_urls, preferred_post_image, story_banner_id,
    ENGLISH_ASSET_HOST, ENGLISH_HEADER_ROOT,
};
use super::{
    date_prefix, days_between, instant_ms, BannerNews, EnglishPost, EnglishSource, JapanesePost,
    RecoveredCampaign, TimelineEvent, DAY_MS,
};
use crate::generators::timeline::normalized_timeline_title;
use std::collections::{BTreeMap, HashMap};

/// How long after an English campaign post its later phases still use the post's picture.
const CAMPAIGN_PHASE_DAYS: i64 = 56;

const GACHA_BANNER_TYPES: &[&str] = &["character_banner", "support_card_banner", "paid_banner"];

/// Event types whose English header carries the identity of their Japanese picture.
const HEADER_EVENT_TYPES: &[&str] = &[
    "campaign",
    "champions_meeting",
    "factor_research",
    "league_of_heroes",
    "legend_race",
    "masters_challenge",
    "racing_carnival",
    "scenario_release",
    "strongest_team",
    "trainer_skills_test",
];

/// A post that announces a mission campaign, with the window its phases run in.
struct CampaignAnnouncement {
    title: String,
    posted: Option<i64>,
    available_until: Option<i64>,
    picture: String,
}

struct CampaignBanner<'a> {
    image_path: &'a str,
    release: i64,
    phase: Option<String>,
}

/// Source URLs by `image_path`: the announced pictures, over the predicted headers.
pub(super) fn english_sources(
    events: &[TimelineEvent],
    news: &BannerNews,
) -> BTreeMap<String, EnglishSource> {
    let mut sources = predicted_header_sources(events, &news.japanese);
    sources.extend(announced_sources(
        events,
        &news.japanese,
        &news.english,
        &news.recovered_campaigns,
    ));
    sources
}

fn announced_sources(
    events: &[TimelineEvent],
    japanese_posts: &[JapanesePost],
    posts: &[EnglishPost],
    recovered: &[RecoveredCampaign],
) -> BTreeMap<String, EnglishSource> {
    let pictures_by_identity = pictures_by_identity(posts);
    let posts_by_id: HashMap<i64, &EnglishPost> =
        posts.iter().map(|post| (post.announce_id, post)).collect();
    let story_identities = story_identities(events, japanese_posts);
    let mut sources = BTreeMap::new();

    for event in events {
        let Some(image_path) = event
            .image_path
            .as_deref()
            .filter(|path| path.ends_with(".webp"))
        else {
            continue;
        };
        if !event.has_global_schedule() {
            continue;
        }

        let mut identities: Vec<String> = asset_identity(&event.image).into_iter().collect();
        identities.extend(
            event
                .gacha_ids()
                .iter()
                .map(|id| format!("gacha_banner_{id}")),
        );
        identities.extend(story_identities.get(event.id.as_str()).cloned());

        let linked_picture = linked_post_id(event)
            .and_then(|id| posts_by_id.get(&id))
            .and_then(|post| post.image.as_deref())
            .filter(|picture| picture.starts_with(ENGLISH_ASSET_HOST));
        let links_its_post = matches!(event.event_type.as_str(), "story_event" | "campaign");
        let mut source = match linked_picture {
            Some(picture) if links_its_post => Some(decode_html(picture)),
            _ => identities
                .iter()
                .find_map(|identity| pictures_by_identity.get(identity).cloned()),
        };

        if event.event_type == "story_event" {
            // English titles and even banner ids can differ, so the one launch notice of the
            // release wins over the Japanese identity.
            if let Some(launch) = story_launch_picture(event, posts) {
                source = Some(launch);
            }
        }
        if event.event_type == "champions_meeting" && event.source == "champions" {
            source = champions_meeting_picture(event, posts);
        }

        if let Some(url) = source.filter(|url| !url.is_empty()) {
            let campaign = event.event_type == "campaign";
            sources.insert(image_path.to_string(), EnglishSource { url, campaign });
        }
    }

    add_campaign_phase_sources(
        &mut sources,
        events,
        &campaign_announcements(posts, recovered),
    );
    sources
}

/// Every official picture by identity; a later post showing the same identity wins.
fn pictures_by_identity(posts: &[EnglishPost]) -> HashMap<String, String> {
    let mut pictures = HashMap::new();
    for post in posts {
        for url in official_post_asset_urls(post) {
            if let Some(identity) = asset_identity(&url) {
                pictures.insert(identity, url);
            }
        }
    }
    pictures
}

fn linked_post_id(event: &TimelineEvent) -> Option<i64> {
    let rest = event
        .umapyoi_url
        .as_deref()?
        .strip_prefix("https://umapyoi.net/en/news/")?;
    let length = rest.bytes().take_while(u8::is_ascii_digit).count();
    let ends_cleanly = matches!(rest[length..].chars().next(), None | Some('?' | '#'));
    if length == 0 || !ends_cleanly {
        return None;
    }
    rest[..length].parse().ok()
}

fn story_launch_picture(event: &TimelineEvent, posts: &[EnglishPost]) -> Option<String> {
    let release = instant_ms(&event.global_release_date)?;
    let launches: Vec<&str> = posts
        .iter()
        .filter(|post| {
            normalized_timeline_title(post.title.as_deref().unwrap_or("")).contains("story event")
        })
        .filter_map(|post| {
            let picture = post.image.as_deref()?;
            let posted = post.post_at? * 1000;
            let is_launch = picture.starts_with(ENGLISH_ASSET_HOST)
                && asset_identity(picture)
                    .is_some_and(|identity| identity.starts_with("banner_301"))
                && days_between(posted, release) <= 2.0;
            is_launch.then_some(picture)
        })
        .collect();

    match launches.as_slice() {
        [launch] => Some(decode_html(launch)),
        _ => None,
    }
}

/// The banner of the announcement naming this Champions Meeting, posted from two days before to
/// ten days after its release. Only a Champions Meeting banner (`banner_3031…`) counts.
fn champions_meeting_picture(event: &TimelineEvent, posts: &[EnglishPost]) -> Option<String> {
    let event_title = normalized_timeline_title(&event.title);
    let release = instant_ms(&event.global_release_date)?;
    if !event.has_global_schedule() || event_title.is_empty() || event_title == "champions meeting"
    {
        return None;
    }

    let mut candidates: Vec<(i32, i64, String)> = Vec::new();
    for post in posts {
        let title = normalized_timeline_title(post.title.as_deref().unwrap_or(""));
        let Some(picture) = post.image.as_deref() else {
            continue;
        };
        let Some(posted_seconds) = post.post_at else {
            continue;
        };
        if !title.contains("champions meeting") || !title.contains(&event_title) {
            continue;
        }
        let posted = posted_seconds * 1000;
        if posted < release - 2 * DAY_MS || posted > release + 10 * DAY_MS {
            continue;
        }

        let identity = asset_identity(picture).unwrap_or_default();
        let points = |earned: bool, points: i32| if earned { points } else { 0 };
        let score = points(identity.starts_with("banner_3031"), 8)
            + points(title.contains("is here"), 4)
            - points(title.contains("coming"), 4)
            - points(title.contains("league selection"), 4);
        candidates.push((score, posted_seconds, decode_html(picture)));
    }

    candidates.sort_by(|left, right| right.0.cmp(&left.0).then(right.1.cmp(&left.1)));
    candidates
        .into_iter()
        .next()
        .filter(|(score, _, _)| *score >= 8)
        .map(|(_, _, picture)| picture)
}

fn campaign_announcements(
    posts: &[EnglishPost],
    recovered: &[RecoveredCampaign],
) -> Vec<CampaignAnnouncement> {
    let mut announcements: Vec<CampaignAnnouncement> = recovered
        .iter()
        .map(|campaign| CampaignAnnouncement {
            title: campaign.title.clone(),
            posted: instant_ms(&campaign.post_at),
            available_until: campaign.available_until.as_deref().and_then(instant_ms),
            picture: campaign.image.clone(),
        })
        .collect();

    for post in posts {
        let title = post.title.clone().unwrap_or_default();
        let message = normalized_timeline_title(post.message.as_deref().unwrap_or(""));
        let names_missions = message
            .split(' ')
            .any(|word| matches!(word, "mission" | "missions"));
        let mission_post = normalized_timeline_title(&title).contains("mission")
            || (campaign_semantic_key(&title, true).is_some() && names_missions);
        let (Some(picture), Some(posted)) =
            (post.image.as_deref(), post.post_at.filter(|at| *at != 0))
        else {
            continue;
        };
        if mission_post {
            announcements.push(CampaignAnnouncement {
                title,
                posted: Some(posted * 1000),
                available_until: None,
                picture: decode_html(picture),
            });
        }
    }
    announcements
}

/// A campaign post covers the campaigns of its phase that start within its window, or else the
/// one campaign starting within a week of it. Pictures already picked are kept.
fn add_campaign_phase_sources(
    sources: &mut BTreeMap<String, EnglishSource>,
    events: &[TimelineEvent],
    announcements: &[CampaignAnnouncement],
) {
    let campaigns: Vec<CampaignBanner> = events
        .iter()
        .filter(|event| event.event_type == "campaign")
        .filter_map(|event| {
            Some(CampaignBanner {
                image_path: event.image_path.as_deref()?,
                release: instant_ms(&event.global_release_date)?,
                phase: campaign_semantic_key(&event.title, false),
            })
        })
        .collect();

    for announcement in announcements {
        let Some(posted) = announcement.posted else {
            continue;
        };
        let phase = campaign_semantic_key(&announcement.title, true);
        let mut matches: Vec<&CampaignBanner> = campaigns
            .iter()
            .filter(|campaign| covers(announcement, posted, phase.as_deref(), campaign))
            .collect();
        matches.sort_by(|left, right| {
            days_between(left.release, posted).total_cmp(&days_between(right.release, posted))
        });

        let covered = if phase.is_some() || matches.len() == 1 {
            matches
        } else {
            Vec::new()
        };
        for campaign in covered {
            sources
                .entry(campaign.image_path.to_string())
                .or_insert_with(|| EnglishSource {
                    url: announcement.picture.clone(),
                    campaign: true,
                });
        }
    }
}

fn covers(
    announcement: &CampaignAnnouncement,
    posted: i64,
    phase: Option<&str>,
    campaign: &CampaignBanner,
) -> bool {
    let within_availability = announcement
        .available_until
        .is_some_and(|until| (posted..=until).contains(&campaign.release));

    match phase {
        Some(phase) if campaign.phase.as_deref() != Some(phase) => false,
        Some(_) => {
            let phase_window = posted - 2 * DAY_MS..=posted + CAMPAIGN_PHASE_DAYS * DAY_MS;
            within_availability || phase_window.contains(&campaign.release)
        }
        None => within_availability || days_between(campaign.release, posted) <= 7.0,
    }
}

/// The header URL Global uploads for a confirmed release, which exists once it is announced:
/// story events by their Japanese banner id, gachas by their id, other events by the identity
/// of their Japanese picture.
fn predicted_header_sources(
    events: &[TimelineEvent],
    japanese_posts: &[JapanesePost],
) -> BTreeMap<String, EnglishSource> {
    let story_ids = story_banner_ids(japanese_posts);
    let mut sources = BTreeMap::new();

    for event in stories_by_japanese_release(events) {
        let Some(image_path) = event
            .image_path
            .as_deref()
            .filter(|path| path.starts_with("assets/images/story/"))
        else {
            continue;
        };
        let Some(release) = instant_ms(&event.global_release_date) else {
            continue;
        };
        let Some(banner_id) = story_ids.get(date_prefix(&event.jp_release_date)) else {
            continue;
        };
        if event.has_global_schedule() {
            let url = header_url(&format!("banner_{banner_id}"), release);
            sources.insert(
                image_path.to_string(),
                EnglishSource {
                    url,
                    campaign: false,
                },
            );
        }
    }

    for event in events.iter().filter(|event| event.has_global_schedule()) {
        let Some(image_path) = event.image_path.as_deref() else {
            continue;
        };
        let Some(release) = instant_ms(&event.global_release_date) else {
            continue;
        };

        let event_type = event.event_type.as_str();
        let first_gacha = event.gacha_ids().first().copied();
        let identity = match first_gacha {
            Some(gacha_id) if GACHA_BANNER_TYPES.contains(&event_type) => {
                Some(format!("gacha_banner_{gacha_id}"))
            }
            _ if HEADER_EVENT_TYPES.contains(&event_type) => asset_identity(&event.image),
            _ => None,
        };
        if let Some(identity) = identity {
            let url = header_url(&identity, release);
            let campaign = event_type == "campaign";
            sources.insert(image_path.to_string(), EnglishSource { url, campaign });
        }
    }
    sources
}

fn header_url(identity: &str, release_ms: i64) -> String {
    let release_seconds = release_ms.div_euclid(1000);
    format!("{ENGLISH_HEADER_ROOT}/{identity}_L{release_seconds}.png")
}

/// Japanese story banner ids by the date of their announcement; the first post of a day wins.
fn story_banner_ids(posts: &[JapanesePost]) -> HashMap<&str, String> {
    let mut ids = HashMap::new();
    for post in posts {
        let Some(posted_at) = post.posted_at.as_deref() else {
            continue;
        };
        if !post
            .event_types
            .iter()
            .any(|event_type| event_type == "story_event")
        {
            continue;
        }
        let banner_id = preferred_post_image(&post.images).and_then(story_banner_id);
        if let Some(banner_id) = banner_id {
            ids.entry(date_prefix(posted_at)).or_insert(banner_id);
        }
    }
    ids
}

/// The Japanese story banner identity of each story event, by event id.
fn story_identities<'a>(
    events: &'a [TimelineEvent],
    japanese_posts: &[JapanesePost],
) -> HashMap<&'a str, String> {
    let story_ids = story_banner_ids(japanese_posts);
    stories_by_japanese_release(events)
        .into_iter()
        .filter_map(|event| {
            let banner_id = story_ids.get(date_prefix(&event.jp_release_date))?;
            Some((event.id.as_str(), format!("banner_{banner_id}")))
        })
        .collect()
}

fn stories_by_japanese_release(events: &[TimelineEvent]) -> Vec<&TimelineEvent> {
    let mut stories: Vec<&TimelineEvent> = events
        .iter()
        .filter(|event| event.event_type == "story_event")
        .collect();
    stories.sort_by(|left, right| left.jp_release_date.cmp(&right.jp_release_date));
    stories
}

#[cfg(test)]
mod tests {
    use super::*;

    fn confirmed(event_type: &str, image_path: &str) -> TimelineEvent {
        TimelineEvent {
            id: format!("{event_type}-1"),
            event_type: event_type.to_string(),
            image_path: Some(image_path.to_string()),
            jp_release_date: "2023-05-10T03:00:00Z".to_string(),
            global_release_date: "2025-07-21T22:00:00Z".to_string(),
            is_confirmed: true,
            ..TimelineEvent::default()
        }
    }

    fn english_post(id: i64, title: &str, picture: &str, posted: &str) -> EnglishPost {
        EnglishPost {
            announce_id: id,
            title: Some(title.to_string()),
            message: Some(String::new()),
            image: Some(picture.to_string()),
            post_at: instant_ms(posted).map(|ms| ms / 1000),
        }
    }

    fn news(english: Vec<EnglishPost>) -> BannerNews {
        BannerNews {
            english,
            ..BannerNews::default()
        }
    }

    #[test]
    fn a_confirmed_gacha_falls_back_to_its_predicted_header() {
        let mut banner = confirmed(
            "character_banner",
            "assets/images/character/banner/30101.webp",
        );
        banner.gacha_id = Some(30101);

        let sources = english_sources(&[banner], &BannerNews::default());

        assert_eq!(
            sources["assets/images/character/banner/30101.webp"].url,
            format!("{ENGLISH_HEADER_ROOT}/gacha_banner_30101_L1753135200.png")
        );
    }

    #[test]
    fn an_announced_picture_replaces_the_predicted_header() {
        let mut banner = confirmed(
            "character_banner",
            "assets/images/character/banner/30101.webp",
        );
        banner.gacha_id = Some(30101);
        let announced = format!("{ENGLISH_ASSET_HOST}contents/Thumbnail/gacha_banner_30101.png");
        let post = english_post(1, "New scout", &announced, "2025-07-20T00:00:00Z");

        let sources = english_sources(&[banner], &news(vec![post]));

        assert_eq!(
            sources["assets/images/character/banner/30101.webp"].url,
            announced
        );
    }

    #[test]
    fn unconfirmed_events_get_no_english_picture() {
        let mut banner = confirmed(
            "character_banner",
            "assets/images/character/banner/30101.webp",
        );
        banner.gacha_id = Some(30101);
        banner.is_confirmed = false;

        let sources = english_sources(&[banner], &BannerNews::default());

        assert!(sources.is_empty());
    }

    #[test]
    fn a_champions_meeting_needs_its_own_banner() {
        let mut meeting = confirmed(
            "champions_meeting",
            "assets/timeline-images/events/champions-meeting/global-5.webp",
        );
        meeting.source = "champions".to_string();
        meeting.title = "Taurus Cup".to_string();
        let banner = format!("{ENGLISH_ASSET_HOST}Thumbnail/banner_30315.png");
        let posts = vec![
            english_post(
                2,
                "Champions Meeting: Taurus Cup is here!",
                &banner,
                "2025-07-22T00:00:00Z",
            ),
            english_post(
                3,
                "Champions Meeting: Taurus Cup coming",
                &format!("{ENGLISH_ASSET_HOST}Header/header_9.png"),
                "2025-07-20T00:00:00Z",
            ),
        ];

        let sources = english_sources(&[meeting], &news(posts));

        assert_eq!(
            sources["assets/timeline-images/events/champions-meeting/global-5.webp"].url,
            banner
        );
    }

    #[test]
    fn a_recovered_campaign_covers_the_campaigns_of_its_phase() {
        let mut first = confirmed("campaign", "assets/images/campaign/90.webp");
        first.title = "Spring G1 Celebration Missions Part 1".to_string();
        let recovered = RecoveredCampaign {
            title: "Spring G1 Celebration Missions Part 1".to_string(),
            post_at: "2025-07-21T22:00:00".to_string(),
            available_until: Some("2025-08-10T14:59:59".to_string()),
            image: format!("{ENGLISH_HEADER_ROOT}/header_21000001_L1753135200.png"),
        };
        let news = BannerNews {
            recovered_campaigns: vec![recovered],
            ..BannerNews::default()
        };

        let sources = english_sources(&[first], &news);

        let source = &sources["assets/images/campaign/90.webp"];
        assert!(source.url.ends_with("header_21000001_L1753135200.png"));
        assert!(source.campaign);
    }

    #[test]
    fn preliminary_gacha_gets_english_header_and_cdn_fallback() {
        let mut banner = confirmed(
            "support_card_banner",
            "assets/images/support/banner/2023_30151.webp",
        );
        banner.gacha_id = Some(30151);
        banner.is_confirmed = false;
        banner.is_preliminary = true;

        let plan = super::super::plan_from_news(&[banner], &BannerNews::default());
        let planned = &plan.banners["assets/images/support/banner/2023_30151.webp"];
        assert_eq!(planned.cdn_gacha_ids, vec![30151]);
        assert_eq!(
            planned.english.as_ref().map(|e| e.url.as_str()),
            Some(format!("{ENGLISH_HEADER_ROOT}/gacha_banner_30151_L1753135200.png").as_str())
        );
    }
}

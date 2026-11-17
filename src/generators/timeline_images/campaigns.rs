//! Naming the release phase of a mission campaign, so a Japanese or English post finds the
//! timeline campaigns of the same phase although their titles and ids differ.

use crate::generators::timeline::normalized_timeline_title;

/// `spring-g1-missions-part-2` for "Spring G1 Celebration Missions Part 2". English titles
/// count only when they name missions; `japanese` titles, which the archive translates, always
/// count, and a few phases exist only in English.
pub(super) fn campaign_semantic_key(title: &str, japanese: bool) -> Option<String> {
    let title = normalized_timeline_title(title);
    if !japanese && !title.contains("mission") {
        return None;
    }

    let tokens: Vec<&str> = title.split(' ').collect();
    let part = numbered_part(&tokens).unwrap_or("1");
    let contains_any = |terms: &[&str]| terms.iter().any(|term| title.contains(term));

    if title.contains("g1 celebration missions") && title.contains("february stakes") {
        return Some("g1-celebration-missions-february-stakes".to_string());
    }
    if title.contains("half anniversary") {
        return Some(format!("half-anniversary-missions-part-{part}"));
    }
    if let Some(anniversary) = anniversary_number(&tokens).filter(|_| title.contains("anniversary"))
    {
        return Some(format!("{anniversary}-anniversary-missions-part-{part}"));
    }
    if contains_any(&["spring g1", "spring gi"]) {
        return Some(format!("spring-g1-missions-part-{part}"));
    }
    if contains_any(&["fall g1", "fall gi", "autumn g1", "autumn gi"]) {
        return Some(format!("fall-g1-missions-part-{part}"));
    }
    if contains_any(&["golshi", "gw special"]) {
        return Some("golshi-week-special-missions".to_string());
    }
    if !japanese && title.contains("tracen special") && title.contains("training the trainer") {
        return Some(format!("training-the-trainer-part-{part}"));
    }
    if let Some(chapter) = main_story_chapter(&title) {
        if !japanese && title.contains("release celebration") {
            return Some(format!("main-story-chapter-{chapter}-missions"));
        }
    }
    if !japanese && title.contains("release celebration missions") {
        return Some(format!("release-celebration-missions-part-{part}"));
    }
    if contains_any(&["gi campaign", "g1 campaign"]) {
        return Some("g1-celebration-missions".to_string());
    }
    if title.starts_with("g1 celebration missions") {
        let names_part_one = numbered_parts(&tokens).any(|number| number == "1");
        if part == "1" && !names_part_one {
            return Some("g1-celebration-missions".to_string());
        }
        return Some(format!("g1-celebration-missions-part-{part}"));
    }
    None
}

/// Every number named as "part N" or "vol N".
fn numbered_parts<'a>(tokens: &'a [&'a str]) -> impl Iterator<Item = &'a str> + 'a {
    tokens.windows(2).filter_map(|pair| {
        let is_part = matches!(pair[0], "part" | "vol");
        (is_part && is_number(pair[1])).then_some(pair[1])
    })
}

fn numbered_part<'a>(tokens: &'a [&'a str]) -> Option<&'a str> {
    numbered_parts(tokens).next()
}

/// `3` in "3rd anniversary" or "3rd anniv".
fn anniversary_number<'a>(tokens: &'a [&'a str]) -> Option<&'a str> {
    tokens.windows(2).find_map(|pair| {
        if !matches!(pair[1], "anniv" | "anniversary") {
            return None;
        }
        ["st", "nd", "rd", "th"]
            .iter()
            .find_map(|suffix| pair[0].strip_suffix(suffix))
            .filter(|number| is_number(number))
    })
}

/// `7` in "main story chapter 7" or "main story ch 7".
fn main_story_chapter(title: &str) -> Option<&str> {
    ["main story ch ", "main story chapter "]
        .iter()
        .filter_map(|prefix| {
            title.match_indices(prefix).find_map(|(start, _)| {
                let rest = &title[start + prefix.len()..];
                let length = rest.bytes().take_while(u8::is_ascii_digit).count();
                (length > 0).then(|| (start, &rest[..length]))
            })
        })
        .min_by_key(|(start, _)| *start)
        .map(|(_, chapter)| chapter)
}

fn is_number(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mission_phases_are_named_by_their_season_and_part() {
        let spring = campaign_semantic_key("Spring G1 Celebration Missions Part 2", false);
        let anniversary = campaign_semantic_key("3rd Anniversary Missions Vol. 1", false);
        let chapter = campaign_semantic_key("Main Story Ch. 7 Release Celebration Missions", false);

        assert_eq!(spring.as_deref(), Some("spring-g1-missions-part-2"));
        assert_eq!(
            anniversary.as_deref(),
            Some("3-anniversary-missions-part-1")
        );
        assert_eq!(chapter.as_deref(), Some("main-story-chapter-7-missions"));
    }

    #[test]
    fn english_titles_count_only_when_they_name_missions() {
        assert_eq!(campaign_semantic_key("Spring G1 Celebration", false), None);
        assert_eq!(
            campaign_semantic_key("Spring G1 Celebration", true).as_deref(),
            Some("spring-g1-missions-part-1")
        );
    }

    #[test]
    fn the_first_g1_celebration_has_no_part_unless_it_names_one() {
        let first = campaign_semantic_key("G1 Celebration Missions", true);
        let named = campaign_semantic_key("G1 Celebration Missions Part 1", true);

        assert_eq!(first.as_deref(), Some("g1-celebration-missions"));
        assert_eq!(named.as_deref(), Some("g1-celebration-missions-part-1"));
    }
}

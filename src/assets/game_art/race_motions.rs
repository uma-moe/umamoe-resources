//! Plain names for the race clips, taken from the game client's own enums: personalities from
//! `RacePersonalityType`, running forms from `RaceRunningType`, paces in the order of
//! `AddCommonEarMotionPath`, start sides from `_rightFootFirst`, finish placings from
//! `GetOverRunFinishOrderCategoryMotion` and finish sides from the course's direction.

use super::catalog::objectName;
use super::types::{RaceMotion, RACE_MOTION_FOLDER, RACE_MOTION_SOURCE};

/// A clip's file name and title within its motion set.
#[derive(Debug, PartialEq)]
struct MotionName {
    stem: String,
    title: String,
}

/// Where a race clip such as `3d/motion/racemain/body/type01/anm_rac_type01_run03_base` is served
/// and what it is called, or `None` for files that are not race clips or whose meaning the game
/// code does not give.
pub(super) fn raceMotion(name: &str) -> Option<RaceMotion> {
    let set_and_clip = name.strip_prefix(RACE_MOTION_SOURCE)?;
    let clip = objectName(set_and_clip).strip_prefix("anm_rac_type")?;
    let (kind, rest) = clip.split_at_checked(2)?;
    let personality = personalityName(kind)?;
    let motion = motionName(rest.strip_prefix('_')?)?;

    Some(RaceMotion {
        key: format!(
            "{RACE_MOTION_FOLDER}/{}/{}.glb",
            folderName(&personality),
            motion.stem
        ),
        title: format!("{personality}: {}", motion.title),
    })
}

/// The motion set folder of a `RacePersonalityType` value from master.mdb, `bright` for 1.
pub(super) fn personalityFolder(personality: i64) -> Option<String> {
    let kind = format!("{personality:02}");
    personalityName(&kind).map(|name| folderName(&name))
}

/// Clip names number personalities: 01 Bright, 02 Sporty, 03 Mature and 04 Lolita, which the
/// English client calls Cute; `xx` is the set used when none applies, and higher numbers are
/// special sets the game swaps in. Set 00 holds clips the game code does not name.
fn personalityName(kind: &str) -> Option<String> {
    let name = match kind {
        "01" => "Bright",
        "02" => "Sporty",
        "03" => "Mature",
        "04" => "Cute",
        "xx" => "Default",
        "00" => return None,
        special if special.bytes().all(|byte| byte.is_ascii_digit()) => {
            return Some(format!("Special {special}"));
        }
        _ => return None,
    };
    Some(name.to_string())
}

fn folderName(personality: &str) -> String {
    personality.to_ascii_lowercase().replace(' ', "-")
}

fn motionName(clip: &str) -> Option<MotionName> {
    let parts: Vec<&str> = clip.split('_').collect();
    match parts.as_slice() {
        ["start", phase, side] => startMotion(phase, side),
        [first, rest @ ..] if first.starts_with("overrun") => {
            overrunMotion(&first["overrun".len()..], rest)
        }
        [first, rest @ ..] if first.starts_with("run") => runMotion(&first["run".len()..], rest),
        _ => None,
    }
}

fn runMotion(pace: &str, modifiers: &[&str]) -> Option<MotionName> {
    let pace = match pace {
        "01" => "Slow",
        "02" => "Normal",
        "03" => "Fast",
        "04" => "Blocked",
        "05" => "Tired",
        _ => return None,
    };
    let (sweating, form) = match modifiers {
        [] => (false, None),
        ["sweat"] => (true, None),
        ["sweat", form] => (true, Some(runningForm(form)?)),
        [form] => (false, Some(runningForm(form)?)),
        _ => return None,
    };

    let mut stem = format!("run-{}", pace.to_ascii_lowercase());
    let mut title = format!("{pace} run");
    if sweating {
        stem.push_str("-sweating");
        title.push_str(", sweating");
    }
    if let Some(form) = form {
        stem.push_str(&format!("-{form}"));
        title.push_str(&format!(", {form} form"));
    }
    Some(MotionName { stem, title })
}

fn runningForm(clip_form: &str) -> Option<&'static str> {
    match clip_form {
        "base" => Some("standard"),
        "pitch" => Some("pitch"),
        "stride" => Some("stride"),
        _ => None,
    }
}

/// Each runner leads with a random foot out of the gate.
fn startMotion(phase: &str, side: &str) -> Option<MotionName> {
    let (stem, title) = match phase {
        "loop" => ("gate-waiting", "Waiting in the gate"),
        "set" => ("gate-set", "Set in the gate"),
        "go" => ("gate-break", "Breaking from the gate"),
        _ => return None,
    };
    let foot = sideName(side)?;

    Some(MotionName {
        stem: format!("{stem}-{foot}-foot"),
        title: format!("{title}, {foot} foot first"),
    })
}

/// After the finish line: the cool-down jog, then the winner's or the others' reaction, mirrored
/// for courses run to the left or to the right.
fn overrunMotion(kind: &str, rest: &[&str]) -> Option<MotionName> {
    let (stem, title) = match kind {
        "01" => ("cooldown-jog", "Cool-down jog"),
        "02" => ("finish-best", "Finish, best placing"),
        "03" => ("finish-other", "Finish, other placings"),
        "finishorder" => ("finish-reaction", "Finish placeholder"),
        _ => return None,
    };
    let (part, side) = match rest {
        [part] => (*part, None),
        [part, side] => (*part, Some(sideName(side)?)),
        _ => return None,
    };
    let part = match part {
        "start" => "begin",
        "loop" => "loop",
        _ => return None,
    };

    let mut motion = MotionName {
        stem: format!("{stem}-{part}"),
        title: format!("{title}, {part}"),
    };
    if let Some(hand) = side {
        motion.stem.push_str(&format!("-{hand}-handed-course"));
        motion.title.push_str(&format!(", {hand}-handed course"));
    }
    Some(motion)
}

fn sideName(side: &str) -> Option<&'static str> {
    match side {
        "l" => Some("left"),
        "r" => Some("right"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Blender cuts object and action names after this many bytes.
    const BLENDER_NAME_LIMIT: usize = 63;

    fn clip(set: &str, clip: &str) -> String {
        format!("{RACE_MOTION_SOURCE}{set}/anm_rac_{clip}")
    }

    fn named(set: &str, name: &str) -> Option<(String, String)> {
        let motion = raceMotion(&clip(set, name))?;
        Some((motion.key, motion.title))
    }

    #[test]
    fn runsAreNamedByPersonalityPaceAndForm() {
        let fast = named("type01", "type01_run03_base");
        let tired = named("type03", "type03_run02_sweat_stride");
        let blocked = named("type02", "type02_run04");

        assert_eq!(
            fast,
            Some((
                "game-assets/uma-motions/race/bright/run-fast-standard.glb".to_string(),
                "Bright: Fast run, standard form".to_string()
            ))
        );
        assert_eq!(
            tired,
            Some((
                "game-assets/uma-motions/race/mature/run-normal-sweating-stride.glb".to_string(),
                "Mature: Normal run, sweating, stride form".to_string()
            ))
        );
        assert_eq!(
            blocked,
            Some((
                "game-assets/uma-motions/race/sporty/run-blocked.glb".to_string(),
                "Sporty: Blocked run".to_string()
            ))
        );
    }

    #[test]
    fn gateStartsNameTheLeadingFoot() {
        let start = named("type04", "type04_start_go_r");

        assert_eq!(
            start,
            Some((
                "game-assets/uma-motions/race/cute/gate-break-right-foot.glb".to_string(),
                "Cute: Breaking from the gate, right foot first".to_string()
            ))
        );
    }

    #[test]
    fn finishesNameThePlacingAndTheCourseDirection() {
        let winner = named("type01", "type01_overrun02_start_l");
        let placeholder = named("typexx", "typexx_overrunfinishorder_loop");

        assert_eq!(
            winner,
            Some((
                "game-assets/uma-motions/race/bright/finish-best-begin-left-handed-course.glb"
                    .to_string(),
                "Bright: Finish, best placing, begin, left-handed course".to_string()
            ))
        );
        assert_eq!(
            placeholder,
            Some((
                "game-assets/uma-motions/race/default/finish-reaction-loop.glb".to_string(),
                "Default: Finish placeholder, loop".to_string()
            ))
        );
    }

    #[test]
    fn specialSetsKeepTheirNumber() {
        let special = named("type99", "type12_run03_pitch");

        assert_eq!(
            special.map(|(key, _)| key),
            Some("game-assets/uma-motions/race/special-12/run-fast-pitch.glb".to_string())
        );
    }

    #[test]
    fn clipsTheGameCodeDoesNotNameAreLeftOut() {
        assert_eq!(named("type00", "type00_run_far_plus"), None);
        assert_eq!(named("type01", "type01_run03_base_ear"), None);
        assert_eq!(named("type01", "type01_jump"), None);
        assert_eq!(
            raceMotion("3d/motion/racemain/facial/type01/anm_rac_type01_run03_base"),
            None
        );
    }

    #[test]
    fn theLongestTitleFitsBlendersNameLimit() {
        let longest = named("type99", "type14_overrun03_start_r").unwrap().1;

        assert_eq!(
            longest,
            "Special 14: Finish, other placings, begin, right-handed course"
        );
        assert!(longest.len() <= BLENDER_NAME_LIMIT);
    }

    #[test]
    fn masterPersonalitiesNameTheirMotionFolder() {
        assert_eq!(personalityFolder(1).as_deref(), Some("bright"));
        assert_eq!(personalityFolder(4).as_deref(), Some("cute"));
        assert_eq!(personalityFolder(11).as_deref(), Some("special-11"));
    }
}

//! Which moves each scene plays, independent of what draws them.

use crate::chip::{Show, words_of};
use crate::figure::CAPSULE;

/// What a scene plays. Each names one gesture, or the fade that stands in
/// for it under Reduce Motion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Rise,
    FadeIn,
    Emerge,
    Kindle,
    KindleFade,
    Hover,
    Orbit,
    Tilt,
    Open,
    OpenFade,
    Cups,
    Shake,
    Strain,
    Breaks,
    Words,
    WordsFade,
    Float,
    FrameSink,
    FailureExit,
    FadeOut,
}

/// How a scene's moves join what showed before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entrance {
    /// The panel had nothing on screen.
    Fresh,
    /// A scene already on screen, and its words carry over unchanged.
    SameWords,
    /// A scene already on screen; this one starts its own words from nothing.
    NewWords,
}

/// The moves a scene plays as its serial moves. `calm` is Reduce Motion.
pub fn moves(show: &Show, entrance: Entrance, calm: bool) -> Vec<Move> {
    let mut moves = Vec::new();
    if entrance == Entrance::Fresh {
        moves.push(if calm { Move::FadeIn } else { Move::Rise });
        if !calm && matches!(show, Show::Recording(_)) {
            moves.push(Move::Emerge);
        }
    }
    if entrance == Entrance::SameWords {
        moves.extend_from_slice(opening(calm));
        return moves;
    }
    moves.extend_from_slice(match (show, calm) {
        (Show::Recording(_), false) => &[Move::Kindle][..],
        (Show::Recording(_), true) => &[Move::KindleFade],
        (Show::Working, false) => &[Move::Hover, Move::Orbit],
        (Show::YourTurn, false) => &[Move::Cups],
        (Show::NothingHeard(_), false) => &[Move::Shake, Move::Breaks],
        (Show::Broken(_), false) => &[Move::Strain, Move::Breaks],
        (Show::Done(_), false) => &[Move::Float, Move::FrameSink],
        (Show::Done(_), true) => &[Move::FadeOut],
        (Show::Copied, _) => &[],
        (Show::Answering(_), calm) => opening(calm),
        (Show::Working | Show::YourTurn | Show::NothingHeard(_) | Show::Broken(_), true) => &[],
    });
    if words_of(show).is_some() {
        moves.push(if calm { Move::WordsFade } else { Move::Words });
    }
    moves
}

/// What the answer plays as it opens: the fill, and the head tilting in to
/// listen.
pub fn opening(calm: bool) -> &'static [Move] {
    if calm {
        &[Move::OpenFade]
    } else {
        &[Move::Open, Move::Tilt]
    }
}

/// Your turn becomes the answer in place: the capsule and the words stay.
pub fn answer_opens(before: Option<&Show>, after: &Show) -> bool {
    matches!((before, after), (Some(Show::YourTurn), Show::Answering(_)))
}

/// The states whose figure fills and moves with the microphone level.
pub fn holds_the_voice(show: &Show) -> bool {
    matches!(show, Show::Recording(_) | Show::Answering(_))
}

pub fn centred(midline: f64, width: f64) -> f64 {
    midline - width / 2.0
}

/// How a scene leaves when none follows it. `None` for Done: its float has
/// already carried it out.
pub fn leaving(show: &Show, calm: bool) -> Option<Move> {
    match show {
        Show::Done(_) => None,
        _ => Some(if calm {
            Move::FadeOut
        } else {
            Move::FailureExit
        }),
    }
}

/// A surface holds the capsule at its top, the clear space below it, and room
/// on both sides during a width transition. Only a point in the rounded shape
/// of the centred capsule, `capsule` wide, counts. `y` runs down from the top.
pub fn in_capsule(view_width: f64, capsule: f64, x: f64, y: f64) -> bool {
    let radius = CAPSULE / 2.0;
    let x = x - (view_width - capsule) / 2.0;
    let centre_x = x.clamp(radius, (capsule - radius).max(radius));
    (x - centre_x).hypot(y - radius) <= radius
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chip::Done;
    use crate::figure::TRAVEL;

    #[test]
    fn a_click_lands_only_inside_the_capsules_rounded_shape() {
        for (x, y) in [(2.0, 22.0), (98.0, 22.0), (50.0, 22.0)] {
            assert!(in_capsule(100.0, 100.0, x, y), "({x}, {y})");
        }
        for (x, y) in [(0.0, 0.0), (100.0, 0.0), (0.0, 44.0), (100.0, 44.0)] {
            assert!(!in_capsule(100.0, 100.0, x, y), "({x}, {y})");
        }
    }

    #[test]
    fn a_click_in_the_clear_space_below_the_capsule_misses() {
        for y in [
            CAPSULE + 1.0,
            CAPSULE + TRAVEL / 2.0,
            CAPSULE + TRAVEL - 1.0,
        ] {
            assert!(!in_capsule(100.0, 100.0, 50.0, y), "y {y}");
        }
    }

    #[test]
    fn a_click_beside_a_narrower_centred_capsule_misses() {
        for x in [2.0, 18.0, 122.0, 138.0] {
            assert!(!in_capsule(140.0, 100.0, x, 22.0), "x {x}");
        }
        for x in [22.0, 70.0, 118.0] {
            assert!(in_capsule(140.0, 100.0, x, 22.0), "x {x}");
        }
    }

    #[test]
    fn copied_shows_its_word_with_no_failure_move() {
        use Move::*;
        assert_eq!(moves(&Show::Copied, Entrance::NewWords, false), [Words]);
        assert_eq!(moves(&Show::Copied, Entrance::Fresh, false), [Rise, Words]);
        assert_eq!(moves(&Show::Copied, Entrance::NewWords, true), [WordsFade]);
        assert_eq!(leaving(&Show::Copied, false), Some(FailureExit));
    }

    fn failed() -> [Show; 2] {
        [
            Show::NothingHeard("Nothing heard".into()),
            Show::Broken("Microphone blocked".into()),
        ]
    }

    fn every_show() -> Vec<Show> {
        let [nothing_heard, broken] = failed();
        vec![
            Show::Recording(String::new()),
            Show::Working,
            Show::YourTurn,
            Show::Answering(String::new()),
            nothing_heard,
            broken,
            Show::Done(Done::Typed),
            Show::Copied,
        ]
    }

    #[test]
    fn only_a_scene_from_none_rises_and_only_recording_emerges() {
        for show in every_show() {
            let entering = moves(&show, Entrance::Fresh, false);
            let staying = moves(&show, Entrance::NewWords, false);
            assert_eq!(entering.first(), Some(&Move::Rise), "{show:?}");
            assert!(!staying.contains(&Move::Rise), "{show:?}");
            assert_eq!(
                entering.contains(&Move::Emerge),
                matches!(show, Show::Recording(_)),
                "{show:?}"
            );
            assert!(!staying.contains(&Move::Emerge), "{show:?}");
        }
    }

    #[test]
    fn each_state_plays_its_own_gesture() {
        use Move::*;
        assert_eq!(
            moves(&Show::YourTurn, Entrance::NewWords, false),
            [Cups, Words]
        );
        assert_eq!(
            moves(&Show::Answering(String::new()), Entrance::NewWords, false),
            [Open, Tilt, Words]
        );
        let [nothing_heard, broken] = failed();
        assert_eq!(
            moves(&nothing_heard, Entrance::NewWords, false),
            [Shake, Breaks, Words]
        );
        assert_eq!(
            moves(&broken, Entrance::NewWords, false),
            [Strain, Breaks, Words]
        );
        let done = Show::Done(Done::Typed);
        assert_eq!(moves(&done, Entrance::NewWords, false), [Float, FrameSink]);
    }

    #[test]
    fn the_answer_tilts_the_head_in_as_it_fills_and_calm_only_fills() {
        use Move::*;
        assert_eq!(opening(false), [Open, Tilt]);
        assert_eq!(opening(true), [OpenFade]);
    }

    #[test]
    fn answering_in_place_only_opens_and_drops_the_words_reveal() {
        use Move::*;
        assert_eq!(
            moves(&Show::Answering(String::new()), Entrance::SameWords, false),
            [Open, Tilt]
        );
        assert_eq!(
            moves(&Show::Answering(String::new()), Entrance::SameWords, true),
            [OpenFade]
        );
    }

    #[test]
    fn only_your_turn_opens_into_the_answer() {
        assert!(answer_opens(
            Some(&Show::YourTurn),
            &Show::Answering(String::new())
        ));
        for (before, after) in [
            (None, Show::Answering(String::new())),
            (Some(Show::Working), Show::Answering(String::new())),
            (
                Some(Show::Recording(String::new())),
                Show::Answering(String::new()),
            ),
            (
                Some(Show::Answering(String::new())),
                Show::Answering(String::new()),
            ),
            (Some(Show::YourTurn), Show::YourTurn),
            (Some(Show::YourTurn), Show::Recording(String::new())),
        ] {
            assert!(
                !answer_opens(before.as_ref(), &after),
                "{before:?} to {after:?}"
            );
        }
    }

    #[test]
    fn only_a_recording_and_an_answer_hold_the_voice() {
        for show in every_show() {
            assert_eq!(
                holds_the_voice(&show),
                matches!(show, Show::Recording(_) | Show::Answering(_)),
                "{show:?}"
            );
        }
    }

    #[test]
    fn the_capsule_is_centred_on_the_midline() {
        assert_eq!(centred(500.0, 44.0), 478.0);
        assert_eq!(centred(500.0, 120.0), 440.0);
    }

    #[test]
    fn reduce_motion_keeps_only_the_fades() {
        use Move::*;
        assert_eq!(
            moves(&Show::Recording(String::new()), Entrance::Fresh, true),
            [FadeIn, KindleFade]
        );
        assert_eq!(moves(&Show::Working, Entrance::NewWords, true), []);
        assert_eq!(
            moves(&Show::YourTurn, Entrance::Fresh, true),
            [FadeIn, WordsFade]
        );
        assert_eq!(
            moves(&Show::Answering(String::new()), Entrance::NewWords, true),
            [OpenFade, WordsFade]
        );
        for fault in failed() {
            assert_eq!(moves(&fault, Entrance::NewWords, true), [WordsFade]);
        }
        assert_eq!(
            moves(&Show::Done(Done::Sent), Entrance::NewWords, true),
            [FadeOut]
        );
    }

    #[test]
    fn done_leaves_by_its_float_and_everything_else_sinks() {
        assert_eq!(leaving(&Show::Done(Done::Typed), false), None);
        for show in [
            Show::Recording(String::new()),
            Show::Working,
            Show::YourTurn,
            Show::Answering(String::new()),
        ]
        .into_iter()
        .chain(failed())
        {
            assert_eq!(leaving(&show, false), Some(Move::FailureExit), "{show:?}");
            assert_eq!(leaving(&show, true), Some(Move::FadeOut), "{show:?}");
        }
    }
}

//! Which moves each scene plays, independent of what draws them.

use crate::chip::{Done, Scene, Show, words_of};
use crate::figure::{self, Pose, WORKING_DEPTH};

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
    Pulse,
    Float,
    FrameSink,
    FailureExit,
    FadeOut,
}

/// How a scene's moves join what showed before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entrance {
    /// Nothing was on screen.
    Fresh,
    /// A scene already on screen, and its words carry over unchanged.
    SameWords,
    /// A scene already on screen; this one starts its own words from nothing.
    NewWords,
}

/// The moves a scene plays as its serial moves. `calm` is Reduce Motion.
pub fn moves(scene: &Scene, entrance: Entrance, calm: bool) -> Vec<Move> {
    let show = &scene.show;
    let mut moves = Vec::new();
    if is_card(scene) {
        match entrance {
            Entrance::Fresh => moves.push(if calm { Move::FadeIn } else { Move::Rise }),
            Entrance::SameWords => return moves,
            Entrance::NewWords => {}
        }
        if matches!(show, Show::Working) && !calm {
            moves.push(Move::Pulse);
        }
        moves.push(Move::WordsFade);
        return moves;
    }
    if entrance == Entrance::Fresh {
        moves.push(if calm { Move::FadeIn } else { Move::Rise });
        if !calm && matches!(show, Show::Recording(_)) {
            moves.push(Move::Emerge);
        }
    }
    if matches!(show, Show::Done(Done::Sent)) {
        moves.push(Move::WordsFade);
        return moves;
    }
    if entrance == Entrance::SameWords {
        moves.extend_from_slice(opening(calm));
        return moves;
    }
    moves.extend_from_slice(match (show, calm) {
        (Show::Recording(_), false) => &[Move::Kindle][..],
        (Show::Recording(_), true) => &[Move::KindleFade],
        (Show::Working, false) => &[Move::Hover, Move::Orbit],
        (Show::YourTurn | Show::Reading, false) => &[Move::Cups],
        (Show::NothingHeard(_), false) => &[Move::Shake, Move::Breaks],
        (Show::Broken(_), false) => &[Move::Strain, Move::Breaks],
        (Show::Done(_), false) => &[Move::Float, Move::FrameSink],
        (Show::Done(_), true) => &[Move::FadeOut],
        (Show::Copied | Show::Missed { .. }, _) => &[],
        (Show::Answering(_), calm) => opening(calm),
        (
            Show::Working
            | Show::YourTurn
            | Show::Reading
            | Show::NothingHeard(_)
            | Show::Broken(_),
            true,
        ) => &[],
    });
    if words_of(show).is_some() {
        moves.push(if calm { Move::WordsFade } else { Move::Words });
    }
    moves
}

pub fn is_card(scene: &Scene) -> bool {
    scene.question.is_some()
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

pub fn failed(show: &Show) -> bool {
    matches!(show, Show::NothingHeard(_) | Show::Broken(_))
}

pub fn wears_headphones(show: &Show) -> bool {
    matches!(show, Show::YourTurn | Show::Answering(_))
}

/// A click on the capsule opens the window.
pub fn takes_clicks(show: &Show) -> bool {
    matches!(show, Show::Broken(_) | Show::Missed { .. })
}

/// The figure's pose outside the voice states: the hem deeper while Working,
/// unless `calm` holds it still.
pub fn rest_pose(show: &Show, calm: bool) -> Pose {
    let rest = figure::pose(0.0, 0.0, calm);
    if matches!(show, Show::Working) && !calm {
        Pose {
            depth: WORKING_DEPTH,
            ..rest
        }
    } else {
        rest
    }
}

pub fn centred(midline: f64, width: f64) -> f64 {
    midline - width / 2.0
}

/// How a scene leaves when none follows it. `None` for a typed Done: its
/// float has already carried it out.
pub fn leaving(show: &Show, calm: bool) -> Option<Move> {
    match show {
        Show::Done(Done::Typed) => None,
        _ => Some(if calm {
            Move::FadeOut
        } else {
            Move::FailureExit
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_shows_its_word_with_no_failure_move() {
        use Move::*;
        assert_eq!(
            moves(&Scene::of(Show::Copied, 1), Entrance::NewWords, false),
            [Words]
        );
        assert_eq!(
            moves(&Scene::of(Show::Copied, 1), Entrance::Fresh, false),
            [Rise, Words]
        );
        assert_eq!(
            moves(&Scene::of(Show::Copied, 1), Entrance::NewWords, true),
            [WordsFade]
        );
        assert_eq!(leaving(&Show::Copied, false), Some(FailureExit));
    }

    fn card(show: Show) -> Scene {
        Scene {
            question: Some("q?".into()),
            ..Scene::of(show, 1)
        }
    }

    #[test]
    fn a_card_plays_only_the_fade_in_the_pulse_and_the_words_fade() {
        use Move::*;
        assert_eq!(
            moves(&card(Show::Working), Entrance::NewWords, false),
            [Pulse, WordsFade]
        );
        assert_eq!(
            moves(&card(Show::Reading), Entrance::Fresh, false),
            [Rise, WordsFade]
        );
        assert_eq!(
            moves(
                &card(Show::Answering("yes".into())),
                Entrance::SameWords,
                false
            ),
            []
        );
        assert_eq!(
            moves(&card(Show::Working), Entrance::NewWords, true),
            [WordsFade]
        );
        assert_eq!(
            moves(&card(Show::Reading), Entrance::Fresh, true),
            [FadeIn, WordsFade]
        );
    }

    fn failures() -> [Show; 2] {
        [
            Show::NothingHeard("Nothing heard".into()),
            Show::Broken("Microphone blocked".into()),
        ]
    }

    fn every_show() -> Vec<Show> {
        let [nothing_heard, broken] = failures();
        vec![
            Show::Recording(String::new()),
            Show::Working,
            Show::YourTurn,
            Show::Answering(String::new()),
            nothing_heard,
            broken,
            Show::Done(Done::Typed),
            Show::Copied,
            Show::Reading,
        ]
    }

    #[test]
    fn only_a_scene_from_none_rises_and_only_recording_emerges() {
        for show in every_show() {
            let entering = moves(&Scene::of(show.clone(), 1), Entrance::Fresh, false);
            let staying = moves(&Scene::of(show.clone(), 1), Entrance::NewWords, false);
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
            moves(&Scene::of(Show::YourTurn, 1), Entrance::NewWords, false),
            [Cups, Words]
        );
        assert_eq!(
            moves(
                &Scene::of(Show::Answering(String::new()), 1),
                Entrance::NewWords,
                false
            ),
            [Open, Tilt, Words]
        );
        let [nothing_heard, broken] = failures();
        assert_eq!(
            moves(&Scene::of(nothing_heard, 1), Entrance::NewWords, false),
            [Shake, Breaks, Words]
        );
        assert_eq!(
            moves(&Scene::of(broken, 1), Entrance::NewWords, false),
            [Strain, Breaks, Words]
        );
        let done = Show::Done(Done::Typed);
        assert_eq!(
            moves(&Scene::of(done, 1), Entrance::NewWords, false),
            [Float, FrameSink]
        );
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
            moves(
                &Scene::of(Show::Answering(String::new()), 1),
                Entrance::SameWords,
                false
            ),
            [Open, Tilt]
        );
        assert_eq!(
            moves(
                &Scene::of(Show::Answering(String::new()), 1),
                Entrance::SameWords,
                true
            ),
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
            moves(
                &Scene::of(Show::Recording(String::new()), 1),
                Entrance::Fresh,
                true
            ),
            [FadeIn, KindleFade]
        );
        assert_eq!(
            moves(&Scene::of(Show::Working, 1), Entrance::NewWords, true),
            []
        );
        assert_eq!(
            moves(&Scene::of(Show::YourTurn, 1), Entrance::Fresh, true),
            [FadeIn, WordsFade]
        );
        assert_eq!(
            moves(
                &Scene::of(Show::Answering(String::new()), 1),
                Entrance::NewWords,
                true
            ),
            [OpenFade, WordsFade]
        );
        for fault in failures() {
            assert_eq!(
                moves(&Scene::of(fault, 1), Entrance::NewWords, true),
                [WordsFade]
            );
        }
        assert_eq!(
            moves(
                &Scene::of(Show::Done(Done::Typed), 1),
                Entrance::NewWords,
                true
            ),
            [FadeOut]
        );
    }

    #[test]
    fn sent_fades_its_words_in_and_sinks_out() {
        use Move::*;
        let sent = Show::Done(Done::Sent);
        for calm in [false, true] {
            assert_eq!(
                moves(&Scene::of(sent.clone(), 1), Entrance::NewWords, calm),
                [WordsFade]
            );
        }
        assert_eq!(leaving(&sent, false), Some(FailureExit));
        assert_eq!(leaving(&sent, true), Some(FadeOut));
        assert_eq!(leaving(&Show::Done(Done::Typed), false), None);
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
        .chain(failures())
        {
            assert_eq!(leaving(&show, false), Some(Move::FailureExit), "{show:?}");
            assert_eq!(leaving(&show, true), Some(Move::FadeOut), "{show:?}");
        }
    }

    #[test]
    fn a_broken_scene_and_a_missed_pill_take_clicks() {
        let missed = Show::Missed {
            count: 1,
            ask: String::new(),
        };
        assert!(takes_clicks(&missed));
        assert!(takes_clicks(&Show::Broken("x".into())));
        for show in [
            Show::Recording(String::new()),
            Show::Working,
            Show::YourTurn,
            Show::Answering(String::new()),
            Show::NothingHeard("x".into()),
            Show::Done(Done::Typed),
            Show::Copied,
            Show::Reading,
        ] {
            assert!(!takes_clicks(&show), "{show:?}");
        }
    }
}

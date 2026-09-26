//! What the chip shows. Events go in, one state comes out, and nothing here
//! touches AppKit, so every transition is a test.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use banshee_common::cue::{Reason, ReasonCode, Signal, Target};
use banshee_common::flag;

/// Unmeasured, like QUIET below.
pub const FAULT_HOLD: Duration = Duration::from_secs(3);
/// Unmeasured: how long every busy flag stays down before a job with no closing signal ends.
pub const QUIET: Duration = Duration::from_secs(1);
/// Done stays this long, so its float out plays to the end.
pub const FLOAT: Duration = Duration::from_millis(440);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Show {
    Recording,
    Working,
    YourTurn,
    Answering,
    NothingHeard(String),
    Broken(String),
    Done(Done),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    Typed,
    Sent,
    Finished,
}

impl Done {
    pub fn words(self) -> &'static str {
        match self {
            Done::Typed => "Typed",
            Done::Sent => "Sent to the agent",
            Done::Finished => "Done",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scene {
    pub show: Show,
    pub serial: u64,
}

/// The flags that say a job still runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub recording: bool,
    pub armed: bool,
    pub transcribing: bool,
    pub telling: bool,
    pub loading_model: bool,
}

impl Live {
    pub fn of(state: &serde_json::Value) -> Self {
        Live {
            recording: flag(state, "recording"),
            armed: flag(state, "armed"),
            transcribing: flag(state, "transcribing"),
            telling: flag(state, "telling"),
            loading_model: flag(state, "loading_model"),
        }
    }

    fn busy(self) -> bool {
        self.recording || self.armed || self.transcribing || self.telling || self.loading_model
    }
}

pub enum Input {
    Signal(Signal),
    Live(Live),
    /// The subscribe reply: the state a tray that starts mid-session inherits.
    Seed(Live),
    Gone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    Recording(Target),
    Working(Target),
    /// Working on a job the seed cannot name, so a failure of any press
    /// target ends it.
    Seeded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Question {
    Asking,
    Answering,
    Holding,
    Working,
}

#[derive(Default)]
pub struct Chip {
    press: Option<Press>,
    question: Option<Question>,
    tells: usize,
    held: Option<(Show, Instant)>,
    waiting: VecDeque<Show>,
    done: Option<(Done, Instant)>,
    quiet_since: Option<Instant>,
    live: Option<Live>,
    shown: Option<Show>,
    renewed: bool,
    serial: u64,
}

impl Chip {
    pub fn feed(&mut self, input: Input, now: Instant) {
        match input {
            Input::Signal(signal) => {
                self.quiet_since = match self.live {
                    Some(live) if !live.busy() => Some(now),
                    _ => None,
                };
                self.signal(signal, now);
            }
            Input::Live(live) => {
                self.live = Some(live);
                self.quiet_since = if live.busy() {
                    None
                } else {
                    self.quiet_since.or(Some(now))
                };
            }
            Input::Seed(live) => {
                let serial = self.serial;
                *self = Chip {
                    serial,
                    ..Chip::default()
                };
                self.seed(live);
            }
            Input::Gone => {
                let serial = self.serial;
                *self = Chip {
                    serial,
                    ..Chip::default()
                };
            }
        }
        self.tick(now);
    }

    pub fn tick(&mut self, now: Instant) {
        if self
            .held
            .as_ref()
            .is_some_and(|(_, at)| now >= *at + FAULT_HOLD)
        {
            self.held = None;
        }
        if self.done.is_some_and(|(_, at)| now >= at + FLOAT) {
            self.done = None;
        }
        if self.running() && self.quiet_since.is_some_and(|since| now >= since + QUIET) {
            self.press = None;
            self.question = None;
            self.tells = 0;
            self.quiet_since = None;
        }
        if !self.foreground()
            && self.held.is_none()
            && self.done.is_none()
            && let Some(next) = self.waiting.pop_front()
        {
            self.hold(next, now);
        }
        self.settle();
    }

    #[cfg(any(target_os = "macos", test))]
    pub fn deadline(&self) -> Option<Instant> {
        [
            self.held.as_ref().map(|(_, at)| *at + FAULT_HOLD),
            self.done.map(|(_, at)| at + FLOAT),
            self.quiet_since
                .filter(|_| self.running())
                .map(|since| since + QUIET),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub fn scene(&self) -> Option<Scene> {
        self.shown.clone().map(|show| Scene {
            show,
            serial: self.serial,
        })
    }

    fn seed(&mut self, live: Live) {
        self.live = Some(live);
        if live.armed {
            self.question = Some(Question::Asking);
        } else if live.recording {
            self.press = Some(Press::Recording(Target::Dictate));
        } else if live.transcribing {
            self.press = Some(Press::Seeded);
        }
        self.tells = usize::from(live.telling);
    }

    fn signal(&mut self, signal: Signal, now: Instant) {
        match signal {
            Signal::RecordStart {
                target: Target::Answer,
            } => {
                self.question = Some(Question::Holding);
                self.begin();
            }
            Signal::RecordStart { target } => {
                self.hand_the_tell_off();
                self.press = Some(Press::Recording(target));
                self.begin();
            }
            Signal::RecordStop {
                target: Target::Answer,
            } => {
                // The release ends the answer at once, so no Answering or Asking state is real here.
                if matches!(self.question, Some(Question::Holding)) {
                    self.question = Some(Question::Working);
                }
            }
            Signal::RecordStop { target } => self.press = Some(Press::Working(target)),
            Signal::Cancelled { .. } => self.press = None,
            Signal::Arm => {
                self.hand_the_tell_off();
                if !matches!(self.question, Some(Question::Holding)) {
                    self.question = Some(Question::Asking);
                }
                self.begin();
            }
            Signal::Disarm => self.question = Some(Question::Working),
            Signal::Answered { heard: true } => {
                self.question = None;
                self.finish(Done::Sent, now);
            }
            Signal::Answered { heard: false } => {
                self.question = None;
                let words = Reason::new(ReasonCode::EmptyTranscript, None).text;
                self.surface(Show::NothingHeard(words), false, now);
            }
            Signal::Told => {
                self.end_a_tell();
                self.finish(Done::Finished, now);
            }
            Signal::Ready { target } => {
                if self.working_on(target) {
                    self.press = None;
                }
                let done = match target {
                    Target::Dictate => Done::Typed,
                    Target::Mailbox => Done::Sent,
                    Target::Tell | Target::Answer => Done::Finished,
                };
                self.finish(done, now);
            }
            Signal::Error { reason, target } => self.error(reason, target, now),
            Signal::Onset => {
                if self.question == Some(Question::Asking) {
                    self.question = Some(Question::Answering);
                }
            }
        }
    }

    fn error(&mut self, reason: Reason, target: Option<Target>, now: Instant) {
        self.end_what_failed(reason.code, target);
        let waits_behind_others = matches!(
            reason.code,
            ReasonCode::TellFailed | ReasonCode::TellWarned | ReasonCode::SpeechFailed
        );
        let fault = match reason.code {
            ReasonCode::NoSpeech
            | ReasonCode::EmptyTranscript
            | ReasonCode::Silence
            | ReasonCode::Closed => Show::NothingHeard(reason.text),
            ReasonCode::ListenFailed
            | ReasonCode::TypeFailed
            | ReasonCode::TranscriptionFailed
            | ReasonCode::Starting
            | ReasonCode::PipelineBroken
            | ReasonCode::SpeechFailed
            | ReasonCode::TellFailed
            | ReasonCode::TellWarned => Show::Broken(reason.text),
        };
        self.surface(fault, waits_behind_others, now);
    }

    /// Ends the job the failure is about, so `surface` reads what is left in
    /// front, not what just failed. The reason code decides only when the
    /// daemon names no target.
    fn end_what_failed(&mut self, code: ReasonCode, target: Option<Target>) {
        // A refused press never began, so its target names no job to end.
        if matches!(code, ReasonCode::Starting | ReasonCode::PipelineBroken) {
            return;
        }
        match target {
            Some(Target::Answer) => self.question = None,
            Some(Target::Tell) => match code {
                ReasonCode::TellFailed | ReasonCode::TellWarned => self.end_a_tell(),
                _ if self.working_on(Target::Tell) => self.press = None,
                _ => self.end_a_tell(),
            },
            Some(target) => {
                if self.working_on(target) {
                    self.press = None;
                }
            }
            None => match code {
                ReasonCode::TellFailed | ReasonCode::TellWarned => self.end_a_tell(),
                ReasonCode::Silence | ReasonCode::Closed | ReasonCode::ListenFailed => {
                    self.question = None;
                }
                ReasonCode::SpeechFailed | ReasonCode::Starting | ReasonCode::PipelineBroken => {}
                ReasonCode::NoSpeech
                | ReasonCode::EmptyTranscript
                | ReasonCode::TypeFailed
                | ReasonCode::TranscriptionFailed => {
                    if matches!(self.press, Some(Press::Working(_) | Press::Seeded)) {
                        self.press = None;
                    } else if self.question == Some(Question::Working) {
                        self.question = None;
                    }
                }
            },
        }
    }

    /// Holds a failure now, or queues it while a job is still in front. A
    /// background job's failure also waits behind a held failure or a Done
    /// float in progress.
    fn surface(&mut self, fault: Show, waits_behind_others: bool, now: Instant) {
        let defer = self.foreground()
            || (waits_behind_others && (self.held.is_some() || self.done.is_some()));
        if defer {
            if self.waiting.back() != Some(&fault) {
                self.waiting.push_back(fault);
            }
        } else {
            self.hold(fault, now);
        }
    }

    fn working_on(&self, target: Target) -> bool {
        match self.press {
            Some(Press::Working(working)) => working == target,
            Some(Press::Seeded) => true,
            Some(Press::Recording(_)) | None => false,
        }
    }

    fn telling_in_front(&self) -> bool {
        self.press == Some(Press::Working(Target::Tell))
    }

    fn hand_the_tell_off(&mut self) {
        if self.telling_in_front() {
            self.press = None;
            self.tells += 1;
        }
    }

    fn end_a_tell(&mut self) {
        if self.tells > 0 {
            self.tells -= 1;
        } else if self.telling_in_front() {
            self.press = None;
        }
    }

    /// A new state cancels a pending hide and an exit in progress.
    fn begin(&mut self) {
        self.held = None;
        self.done = None;
    }

    fn hold(&mut self, fault: Show, now: Instant) {
        self.done = None;
        self.held = Some((fault, now));
        self.renewed = true;
    }

    fn finish(&mut self, done: Done, now: Instant) {
        self.done = Some((done, now));
    }

    fn foreground(&self) -> bool {
        self.press.is_some() || self.question.is_some()
    }

    fn running(&self) -> bool {
        self.foreground() || self.tells > 0
    }

    fn resolve(&self) -> Option<Show> {
        if let Some(press) = self.press {
            return Some(match press {
                Press::Recording(_) => Show::Recording,
                Press::Working(_) | Press::Seeded => Show::Working,
            });
        }
        if let Some(question) = self.question {
            return Some(match question {
                Question::Asking => Show::YourTurn,
                Question::Answering => Show::Answering,
                Question::Holding => Show::Recording,
                Question::Working => Show::Working,
            });
        }
        if let Some((fault, _)) = &self.held {
            return Some(fault.clone());
        }
        if let Some((done, _)) = self.done {
            return Some(Show::Done(done));
        }
        (self.tells > 0).then_some(Show::Working)
    }

    fn settle(&mut self) {
        let resolved = self.resolve();
        let renewed = std::mem::take(&mut self.renewed)
            && matches!(resolved, Some(Show::NothingHeard(_) | Show::Broken(_)));
        if resolved != self.shown || renewed {
            self.serial += 1;
            self.shown = resolved;
        }
    }
}

/// What VoiceOver hears as a state begins. Nothing while recording or while
/// an answer is awaited: the microphone is open, so the words would become
/// the answer.
pub fn announcement(before: Option<&Scene>, after: Option<&Scene>) -> Option<String> {
    let after = after?;
    if before.map(|scene| scene.serial) == Some(after.serial) {
        return None;
    }
    match &after.show {
        Show::Working => Some("Working".to_string()),
        Show::Done(done) => Some(done.words().to_string()),
        Show::NothingHeard(text) | Show::Broken(text) => Some(text.clone()),
        Show::Recording | Show::YourTurn | Show::Answering => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    struct Clock(Instant);

    impl Clock {
        fn new() -> Self {
            Clock(Instant::now())
        }
        fn at(&self, millis: u64) -> Instant {
            self.0 + Duration::from_millis(millis)
        }
    }

    fn fed(chip: &mut Chip, at: Instant, signals: impl IntoIterator<Item = Signal>) {
        for signal in signals {
            chip.feed(Input::Signal(signal), at);
        }
    }

    fn shown(chip: &Chip) -> Option<Show> {
        chip.scene().map(|scene| scene.show)
    }

    fn error(code: ReasonCode) -> Signal {
        Signal::Error {
            reason: Reason::new(code, Some("AirPods")),
            target: None,
        }
    }

    fn error_of(code: ReasonCode, target: Target) -> Signal {
        Signal::Error {
            reason: Reason::new(code, Some("AirPods")),
            target: Some(target),
        }
    }

    const IDLE: Live = Live {
        recording: false,
        armed: false,
        transcribing: false,
        telling: false,
        loading_model: false,
    };

    #[test]
    fn a_dictation_records_works_floats_and_rests() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [Signal::RecordStart {
                target: Target::Dictate,
            }],
        );
        assert_eq!(shown(&chip), Some(Show::Recording));
        fed(
            &mut chip,
            clock.at(900),
            [Signal::RecordStop {
                target: Target::Dictate,
            }],
        );
        assert_eq!(shown(&chip), Some(Show::Working));
        fed(
            &mut chip,
            clock.at(1400),
            [Signal::Ready {
                target: Target::Dictate,
            }],
        );
        assert_eq!(shown(&chip), Some(Show::Done(Done::Typed)));
        chip.tick(clock.at(1400) + FLOAT);
        assert_eq!(shown(&chip), None);
    }

    #[test]
    fn an_earlier_dictations_ready_leaves_a_newer_press_recording() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Dictate,
                },
                Signal::RecordStop {
                    target: Target::Dictate,
                },
                Signal::RecordStart {
                    target: Target::Dictate,
                },
            ],
        );
        assert_eq!(shown(&chip), Some(Show::Recording));
        fed(
            &mut chip,
            clock.at(100),
            [Signal::Ready {
                target: Target::Dictate,
            }],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Recording),
            "the newer press keeps recording, so its levels still draw"
        );
        fed(
            &mut chip,
            clock.at(200),
            [
                Signal::RecordStop {
                    target: Target::Dictate,
                },
                Signal::Ready {
                    target: Target::Dictate,
                },
            ],
        );
        assert_eq!(shown(&chip), Some(Show::Done(Done::Typed)));
    }

    #[test]
    fn a_press_queued_behind_a_model_load_outlives_the_fallback() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Dictate,
                },
                Signal::RecordStop {
                    target: Target::Dictate,
                },
            ],
        );
        chip.feed(
            Input::Live(Live {
                loading_model: true,
                ..IDLE
            }),
            clock.at(100),
        );
        assert_eq!(chip.deadline(), None);
        chip.tick(clock.at(100) + QUIET);
        assert_eq!(shown(&chip), Some(Show::Working));
    }

    #[test]
    fn a_cancelled_press_ends_on_cancelled_with_no_float() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Dictate,
                },
                Signal::Cancelled {
                    target: Target::Dictate,
                },
            ],
        );
        assert_eq!(shown(&chip), None);
    }

    #[test]
    fn a_press_shows_over_a_question_and_a_question_over_a_tell() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Tell,
                },
                Signal::RecordStop {
                    target: Target::Tell,
                },
                Signal::Arm,
            ],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::YourTurn),
            "the tell went to the background"
        );
        fed(
            &mut chip,
            clock.at(100),
            [Signal::RecordStart {
                target: Target::Dictate,
            }],
        );
        assert_eq!(shown(&chip), Some(Show::Recording));
        fed(
            &mut chip,
            clock.at(200),
            [Signal::Cancelled {
                target: Target::Dictate,
            }],
        );
        assert_eq!(shown(&chip), Some(Show::YourTurn));
        fed(
            &mut chip,
            clock.at(300),
            [Signal::Disarm, Signal::Answered { heard: true }],
        );
        assert_eq!(shown(&chip), Some(Show::Done(Done::Sent)));
        chip.tick(clock.at(300) + FLOAT);
        assert_eq!(shown(&chip), Some(Show::Working), "the tell still runs");
        fed(&mut chip, clock.at(5000), [Signal::Told]);
        assert_eq!(shown(&chip), Some(Show::Done(Done::Finished)));
    }

    #[test]
    fn a_background_failure_waits_for_the_press_to_end() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Dictate,
                },
                error(ReasonCode::SpeechFailed),
            ],
        );
        assert_eq!(shown(&chip), Some(Show::Recording));
        fed(
            &mut chip,
            clock.at(500),
            [
                Signal::RecordStop {
                    target: Target::Dictate,
                },
                Signal::Ready {
                    target: Target::Dictate,
                },
            ],
        );
        assert_eq!(shown(&chip), Some(Show::Done(Done::Typed)));
        chip.tick(clock.at(500) + FLOAT);
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("The reply did not play".to_string()))
        );
    }

    #[test]
    fn a_new_press_cancels_a_held_failure() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Dictate,
                },
                Signal::RecordStop {
                    target: Target::Dictate,
                },
                error(ReasonCode::NoSpeech),
            ],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::NothingHeard("Nothing heard on AirPods".to_string()))
        );
        fed(
            &mut chip,
            clock.at(400),
            [Signal::RecordStart {
                target: Target::Dictate,
            }],
        );
        assert_eq!(shown(&chip), Some(Show::Recording));
        fed(
            &mut chip,
            clock.at(500),
            [Signal::Cancelled {
                target: Target::Dictate,
            }],
        );
        assert_eq!(
            shown(&chip),
            None,
            "the held failure was cancelled, not merely covered"
        );
    }

    #[test]
    fn a_new_press_cancels_a_done_float_in_progress() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Dictate,
                },
                Signal::RecordStop {
                    target: Target::Dictate,
                },
                Signal::Ready {
                    target: Target::Dictate,
                },
            ],
        );
        assert_eq!(shown(&chip), Some(Show::Done(Done::Typed)));
        fed(
            &mut chip,
            clock.at(100),
            [Signal::RecordStart {
                target: Target::Dictate,
            }],
        );
        assert_eq!(shown(&chip), Some(Show::Recording));
        fed(
            &mut chip,
            clock.at(200),
            [Signal::Cancelled {
                target: Target::Dictate,
            }],
        );
        assert_eq!(
            shown(&chip),
            None,
            "the float was cancelled, not merely covered"
        );
    }

    #[test]
    fn a_held_failure_hides_after_its_hold() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(&mut chip, clock.at(0), [error(ReasonCode::Starting)]);
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("Banshee is still starting".to_string()))
        );
        assert_eq!(chip.deadline(), Some(clock.at(0) + FAULT_HOLD));
        chip.tick(clock.at(0) + FAULT_HOLD);
        assert_eq!(shown(&chip), None);
    }

    #[test]
    fn a_state_that_repeats_keeps_its_serial() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(&mut chip, clock.at(0), [Signal::Arm]);
        let first = chip.scene().unwrap().serial;
        fed(&mut chip, clock.at(10), [Signal::Arm]);
        assert_eq!(chip.scene().unwrap().serial, first);
    }

    #[test]
    fn an_arm_during_an_armed_hold_keeps_recording() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Answer,
                },
                Signal::Arm,
            ],
        );
        assert_eq!(shown(&chip), Some(Show::Recording));
        fed(
            &mut chip,
            clock.at(900),
            [Signal::RecordStop {
                target: Target::Answer,
            }],
        );
        assert_eq!(shown(&chip), Some(Show::Working));
    }

    #[test]
    fn a_press_on_a_closed_pipeline_shows_its_error_again() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(&mut chip, clock.at(0), [error(ReasonCode::PipelineBroken)]);
        let first = chip.scene().unwrap().serial;
        fed(
            &mut chip,
            clock.at(800),
            [error(ReasonCode::PipelineBroken)],
        );
        assert_eq!(
            chip.scene().unwrap().serial,
            first + 1,
            "the gesture plays again"
        );
        chip.tick(clock.at(0) + FAULT_HOLD);
        assert!(
            chip.scene().is_some(),
            "the hold starts again from the repeat"
        );
    }

    #[test]
    fn a_refused_press_does_not_end_a_tell_shown_in_front() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Tell,
                },
                Signal::RecordStop {
                    target: Target::Tell,
                },
            ],
        );
        assert_eq!(shown(&chip), Some(Show::Working));
        for refused in [
            error(ReasonCode::Starting),
            error_of(ReasonCode::Starting, Target::Tell),
        ] {
            fed(&mut chip, clock.at(100), [refused]);
            assert_eq!(
                shown(&chip),
                Some(Show::Working),
                "the tell keeps running; the refusal ends nothing"
            );
        }
        fed(&mut chip, clock.at(200), [Signal::Told]);
        assert_eq!(shown(&chip), Some(Show::Done(Done::Finished)));
    }

    #[test]
    fn a_failure_under_a_recording_press_ends_the_question_not_the_press() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::Arm,
                Signal::Disarm,
                Signal::RecordStart {
                    target: Target::Dictate,
                },
            ],
        );
        assert_eq!(shown(&chip), Some(Show::Recording));
        fed(
            &mut chip,
            clock.at(100),
            [error(ReasonCode::TranscriptionFailed)],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Recording),
            "the user's press keeps showing"
        );
        // A failure held at t=100, not queued, would expire unseen before this.
        let ends = clock.at(100) + FAULT_HOLD + Duration::from_millis(500);
        fed(
            &mut chip,
            ends,
            [Signal::Cancelled {
                target: Target::Dictate,
            }],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("Could not transcribe".to_string())),
            "the question's failure surfaces once the press ends"
        );
    }

    #[test]
    fn a_background_tells_failure_waits_behind_the_foreground_tell() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Tell,
                },
                Signal::RecordStop {
                    target: Target::Tell,
                },
                Signal::RecordStart {
                    target: Target::Tell,
                },
                Signal::RecordStop {
                    target: Target::Tell,
                },
            ],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Working),
            "B runs in front, A backgrounded"
        );
        fed(
            &mut chip,
            clock.at(100),
            [error_of(ReasonCode::TellFailed, Target::Tell)],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Working),
            "A's failure waits behind B, not held and ticking unseen"
        );
        // A failure held at t=100, not queued, would expire unseen before this.
        let ends = clock.at(100) + FAULT_HOLD + Duration::from_millis(500);
        fed(
            &mut chip,
            ends,
            [Signal::Cancelled {
                target: Target::Tell,
            }],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("The agent run failed".to_string())),
            "A's failure surfaces once B ends"
        );
    }

    #[test]
    fn a_repeated_refusal_under_a_question_does_not_stack_in_waiting() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(&mut chip, clock.at(0), [Signal::Arm]);
        fed(
            &mut chip,
            clock.at(100),
            [
                error(ReasonCode::Starting),
                error(ReasonCode::Starting),
                error(ReasonCode::Starting),
            ],
        );
        assert_eq!(shown(&chip), Some(Show::YourTurn), "still queued");
        fed(
            &mut chip,
            clock.at(200),
            [Signal::Disarm, Signal::Answered { heard: true }],
        );
        let starts = clock.at(200) + FLOAT;
        chip.tick(starts);
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("Banshee is still starting".to_string())),
            "the refusal shows once"
        );
        chip.tick(starts + FAULT_HOLD);
        assert_eq!(
            shown(&chip),
            None,
            "one hold's worth of time clears it: it was not queued three deep"
        );
    }

    #[test]
    fn ending_one_background_tell_leaves_the_foreground_tell_running() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Tell,
                },
                Signal::RecordStop {
                    target: Target::Tell,
                },
                Signal::RecordStart {
                    target: Target::Tell,
                },
                Signal::RecordStop {
                    target: Target::Tell,
                },
            ],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Working),
            "B runs in front, A backgrounded"
        );
        fed(&mut chip, clock.at(100), [Signal::Told]);
        assert_eq!(
            shown(&chip),
            Some(Show::Working),
            "A ends; B's Working stays"
        );
        fed(&mut chip, clock.at(200), [error(ReasonCode::NoSpeech)]);
        assert_eq!(
            shown(&chip),
            Some(Show::NothingHeard("Nothing heard on AirPods".to_string())),
            "B ends on its own failure"
        );
    }

    #[test]
    fn a_refused_press_under_a_question_shows_once_the_question_ends() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(&mut chip, clock.at(0), [Signal::Arm]);
        assert_eq!(shown(&chip), Some(Show::YourTurn));
        fed(&mut chip, clock.at(100), [error(ReasonCode::Starting)]);
        assert_eq!(
            shown(&chip),
            Some(Show::YourTurn),
            "the refusal waits under the question, not shown or held yet"
        );
        // A refusal held at t=100, not queued, would expire unseen before this.
        let ends = clock.at(100) + FAULT_HOLD + Duration::from_millis(500);
        fed(
            &mut chip,
            ends,
            [Signal::Disarm, Signal::Answered { heard: true }],
        );
        chip.tick(ends + FLOAT);
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("Banshee is still starting".to_string())),
            "the refusal surfaces once the question and its float both end"
        );
    }

    #[test]
    fn a_background_failure_waits_behind_a_held_failure_not_only_a_foreground_job() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Tell,
                },
                Signal::RecordStop {
                    target: Target::Tell,
                },
                error(ReasonCode::TellFailed),
            ],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("The agent run failed".to_string()))
        );
        fed(&mut chip, clock.at(500), [error(ReasonCode::SpeechFailed)]);
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("The agent run failed".to_string())),
            "the held failure stays until its own hold ends"
        );
        chip.tick(clock.at(0) + FAULT_HOLD);
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("The reply did not play".to_string()))
        );
    }

    #[test]
    fn a_background_failure_waits_behind_a_done_float() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Dictate,
                },
                Signal::RecordStop {
                    target: Target::Dictate,
                },
                Signal::Ready {
                    target: Target::Dictate,
                },
            ],
        );
        assert_eq!(shown(&chip), Some(Show::Done(Done::Typed)));
        fed(&mut chip, clock.at(100), [error(ReasonCode::SpeechFailed)]);
        assert_eq!(
            shown(&chip),
            Some(Show::Done(Done::Typed)),
            "the float keeps showing until it ends"
        );
        chip.tick(clock.at(0) + FLOAT);
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("The reply did not play".to_string())),
            "the failure shows once the float ends"
        );
    }

    #[test]
    fn the_fallback_ends_a_job_whose_closing_signal_never_came() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [Signal::RecordStart {
                target: Target::Dictate,
            }],
        );
        chip.feed(Input::Live(IDLE), clock.at(100));
        assert_eq!(chip.deadline(), Some(clock.at(100) + QUIET));
        chip.tick(clock.at(100) + QUIET);
        assert_eq!(shown(&chip), None);
    }

    #[test]
    fn a_signal_after_the_flags_fall_restarts_the_fallback() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [Signal::RecordStart {
                target: Target::Dictate,
            }],
        );
        chip.feed(Input::Live(IDLE), clock.at(100));
        fed(
            &mut chip,
            clock.at(900),
            [Signal::RecordStop {
                target: Target::Dictate,
            }],
        );
        chip.tick(clock.at(100) + QUIET);
        assert_eq!(shown(&chip), Some(Show::Working));
        chip.tick(clock.at(900) + QUIET);
        assert_eq!(shown(&chip), None);
    }

    #[test]
    fn a_signal_after_a_seeded_press_goes_idle_restarts_the_fallback() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        chip.feed(
            Input::Seed(Live {
                transcribing: true,
                ..IDLE
            }),
            clock.at(0),
        );
        assert_eq!(shown(&chip), Some(Show::Working));
        chip.feed(Input::Live(IDLE), clock.at(50));
        fed(&mut chip, clock.at(700), [Signal::Answered { heard: true }]);
        assert_eq!(
            shown(&chip),
            Some(Show::Working),
            "the fallback restarted rather than being cancelled"
        );
        chip.tick(clock.at(700) + QUIET);
        assert_eq!(shown(&chip), None);
    }

    #[test]
    fn told_ends_a_tell_while_telling_is_still_true() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        chip.feed(
            Input::Seed(Live {
                telling: true,
                ..IDLE
            }),
            clock.at(0),
        );
        assert_eq!(shown(&chip), Some(Show::Working));
        fed(&mut chip, clock.at(100), [Signal::Told]);
        assert_eq!(shown(&chip), Some(Show::Done(Done::Finished)));
    }

    #[test]
    fn a_tray_that_starts_mid_question_shows_your_turn() {
        let mut chip = Chip::default();
        chip.feed(
            Input::Seed(Live {
                recording: true,
                armed: true,
                ..IDLE
            }),
            Instant::now(),
        );
        assert_eq!(shown(&chip), Some(Show::YourTurn));
    }

    #[test]
    fn a_daemon_that_goes_away_takes_the_chip_with_it() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [Signal::Arm, error(ReasonCode::SpeechFailed)],
        );
        chip.feed(Input::Gone, clock.at(10));
        assert_eq!(shown(&chip), None);
        assert_eq!(chip.deadline(), None);
    }

    #[test]
    fn an_answer_heard_as_nothing_fails_rather_than_floats() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::Arm,
                Signal::Disarm,
                Signal::Answered { heard: false },
            ],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::NothingHeard("Nothing heard".to_string()))
        );
    }

    #[test]
    fn an_answer_heard_as_nothing_waits_behind_a_press() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::Arm,
                Signal::Disarm,
                Signal::RecordStart {
                    target: Target::Dictate,
                },
            ],
        );
        fed(
            &mut chip,
            clock.at(100),
            [Signal::Answered { heard: false }],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Recording),
            "the press keeps showing"
        );
        let ends = clock.at(100) + FAULT_HOLD + Duration::from_millis(500);
        fed(
            &mut chip,
            ends,
            [Signal::Cancelled {
                target: Target::Dictate,
            }],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::NothingHeard("Nothing heard".to_string())),
            "the answer's failure surfaces once the press ends"
        );
    }

    #[test]
    fn a_seed_gives_the_fallback_the_flags_before_the_first_push() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        chip.feed(Input::Seed(IDLE), clock.at(0));
        fed(
            &mut chip,
            clock.at(100),
            [Signal::RecordStart {
                target: Target::Dictate,
            }],
        );
        assert_eq!(chip.deadline(), Some(clock.at(100) + QUIET));
    }

    #[test]
    fn the_answers_failure_ends_the_question_not_the_dictation() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::Arm,
                Signal::Disarm,
                Signal::RecordStart {
                    target: Target::Dictate,
                },
                Signal::RecordStop {
                    target: Target::Dictate,
                },
            ],
        );
        assert_eq!(shown(&chip), Some(Show::Working));
        fed(
            &mut chip,
            clock.at(100),
            [error_of(ReasonCode::TranscriptionFailed, Target::Answer)],
        );
        fed(
            &mut chip,
            clock.at(200),
            [Signal::Ready {
                target: Target::Dictate,
            }],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Done(Done::Typed)),
            "the dictation outlived the answer's failure"
        );
        chip.tick(clock.at(200) + FLOAT);
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("Could not transcribe".to_string())),
            "the question ended, so nothing in front holds the failure back"
        );
    }

    #[test]
    fn a_dictations_no_speech_ends_the_press_not_the_question() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Dictate,
                },
                Signal::RecordStop {
                    target: Target::Dictate,
                },
                Signal::Arm,
                Signal::Disarm,
            ],
        );
        fed(
            &mut chip,
            clock.at(100),
            [error_of(ReasonCode::NoSpeech, Target::Dictate)],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Working),
            "the question still works on its answer"
        );
        fed(&mut chip, clock.at(200), [Signal::Answered { heard: true }]);
        assert_eq!(shown(&chip), Some(Show::Done(Done::Sent)));
        chip.tick(clock.at(200) + FLOAT);
        assert_eq!(
            shown(&chip),
            Some(Show::NothingHeard("Nothing heard on AirPods".to_string())),
            "the press ended on its own failure"
        );
    }

    #[test]
    fn a_dictations_failure_under_a_newer_press_leaves_the_question_alone() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::Arm,
                Signal::Disarm,
                Signal::RecordStart {
                    target: Target::Dictate,
                },
                Signal::RecordStop {
                    target: Target::Dictate,
                },
                Signal::RecordStart {
                    target: Target::Dictate,
                },
            ],
        );
        fed(
            &mut chip,
            clock.at(100),
            [error_of(ReasonCode::NoSpeech, Target::Dictate)],
        );
        fed(
            &mut chip,
            clock.at(200),
            [Signal::Cancelled {
                target: Target::Dictate,
            }],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Working),
            "the question still works on its answer"
        );
    }

    #[test]
    fn a_mailbox_failure_leaves_a_dictation_working_in_front() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Mailbox,
                },
                Signal::RecordStop {
                    target: Target::Mailbox,
                },
                Signal::RecordStart {
                    target: Target::Dictate,
                },
                Signal::RecordStop {
                    target: Target::Dictate,
                },
            ],
        );
        fed(
            &mut chip,
            clock.at(100),
            [error_of(ReasonCode::TranscriptionFailed, Target::Mailbox)],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Working),
            "the dictation still works"
        );
    }

    #[test]
    fn a_tell_press_that_hears_nothing_ends_itself_not_the_tell_behind_it() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        let tell_press = [
            Signal::RecordStart {
                target: Target::Tell,
            },
            Signal::RecordStop {
                target: Target::Tell,
            },
        ];
        fed(&mut chip, clock.at(0), tell_press.clone());
        fed(&mut chip, clock.at(50), tell_press);
        fed(
            &mut chip,
            clock.at(100),
            [error_of(ReasonCode::NoSpeech, Target::Tell)],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::NothingHeard("Nothing heard on AirPods".to_string())),
            "the second press ends on its own failure"
        );
        chip.tick(clock.at(100) + FAULT_HOLD);
        assert_eq!(
            shown(&chip),
            Some(Show::Working),
            "the first tell still runs"
        );
    }

    #[test]
    fn a_seeded_press_ends_on_a_failure_of_any_press_target() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        chip.feed(
            Input::Seed(Live {
                transcribing: true,
                ..IDLE
            }),
            clock.at(0),
        );
        fed(
            &mut chip,
            clock.at(100),
            [error_of(ReasonCode::TranscriptionFailed, Target::Mailbox)],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("Could not transcribe".to_string()))
        );
    }

    #[test]
    fn silence_ends_the_question_with_its_own_words() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [Signal::Arm, Signal::Disarm, error(ReasonCode::Silence)],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::NothingHeard("No answer heard".to_string()))
        );
    }

    #[test]
    fn onset_opens_the_answer_under_the_same_question() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(&mut chip, clock.at(0), [Signal::Arm]);
        let waiting = chip.scene().unwrap();
        assert_eq!(waiting.show, Show::YourTurn);
        fed(&mut chip, clock.at(900), [Signal::Onset]);
        let answering = chip.scene().unwrap();
        assert_eq!(answering.show, Show::Answering);
        assert_ne!(answering.serial, waiting.serial);
        fed(&mut chip, clock.at(1200), [Signal::Onset]);
        assert_eq!(
            chip.scene(),
            Some(answering),
            "a second onset changes nothing"
        );
    }

    #[test]
    fn onset_outside_a_waiting_question_changes_nothing() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(&mut chip, clock.at(0), [Signal::Onset]);
        assert_eq!(shown(&chip), None);
        fed(
            &mut chip,
            clock.at(100),
            [
                Signal::Arm,
                Signal::RecordStart {
                    target: Target::Answer,
                },
                Signal::Onset,
            ],
        );
        assert_eq!(shown(&chip), Some(Show::Recording));
        fed(&mut chip, clock.at(200), [Signal::Disarm, Signal::Onset]);
        assert_eq!(shown(&chip), Some(Show::Working));
    }

    #[test]
    fn a_hold_during_the_answer_records() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::Arm,
                Signal::Onset,
                Signal::RecordStart {
                    target: Target::Answer,
                },
            ],
        );
        assert_eq!(shown(&chip), Some(Show::Recording));
    }

    #[test]
    fn a_holds_release_moves_straight_to_working() {
        let clock = Clock::new();
        let hold = [
            Signal::RecordStart {
                target: Target::Answer,
            },
            Signal::RecordStop {
                target: Target::Answer,
            },
        ];

        let mut chip = Chip::default();
        fed(&mut chip, clock.at(0), [Signal::Arm, Signal::Onset]);
        fed(&mut chip, clock.at(100), hold.clone());
        assert_eq!(shown(&chip), Some(Show::Working), "after the onset");

        let mut chip = Chip::default();
        fed(&mut chip, clock.at(0), [Signal::Arm]);
        fed(&mut chip, clock.at(100), hold);
        assert_eq!(shown(&chip), Some(Show::Working), "before the onset");
    }

    #[test]
    fn every_end_of_a_question_ends_it_from_the_answer() {
        let clock = Clock::new();
        let answering = || {
            let mut chip = Chip::default();
            fed(&mut chip, clock.at(0), [Signal::Arm, Signal::Onset]);
            assert_eq!(shown(&chip), Some(Show::Answering));
            chip
        };

        let mut chip = answering();
        fed(&mut chip, clock.at(900), [Signal::Disarm]);
        assert_eq!(shown(&chip), Some(Show::Working));
        fed(
            &mut chip,
            clock.at(1400),
            [Signal::Answered { heard: true }],
        );
        assert_eq!(shown(&chip), Some(Show::Done(Done::Sent)));

        for (code, words) in [
            (ReasonCode::Silence, "No answer heard"),
            (ReasonCode::Closed, "The question was closed"),
        ] {
            let mut chip = answering();
            fed(&mut chip, clock.at(900), [error_of(code, Target::Answer)]);
            assert_eq!(shown(&chip), Some(Show::NothingHeard(words.to_string())));
        }

        let mut chip = answering();
        chip.feed(Input::Live(IDLE), clock.at(900));
        chip.tick(clock.at(900) + QUIET);
        assert_eq!(shown(&chip), None, "the fallback ends it");
    }

    #[test]
    fn nothing_is_announced_while_recording_or_while_an_answer_is_awaited() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        let mut before = chip.scene();
        let mut said = Vec::new();
        for signal in [
            Signal::RecordStart {
                target: Target::Dictate,
            },
            Signal::RecordStop {
                target: Target::Dictate,
            },
            Signal::Ready {
                target: Target::Dictate,
            },
            Signal::Arm,
            Signal::Onset,
            Signal::Disarm,
            error(ReasonCode::Closed),
        ] {
            chip.feed(Input::Signal(signal), clock.at(0));
            let after = chip.scene();
            said.push(announcement(before.as_ref(), after.as_ref()));
            before = after;
        }
        assert_eq!(
            said,
            vec![
                None,
                Some("Working".to_string()),
                Some("Typed".to_string()),
                None,
                None,
                Some("Working".to_string()),
                Some("The question was closed".to_string()),
            ]
        );
    }
}

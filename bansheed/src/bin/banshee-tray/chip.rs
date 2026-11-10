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
pub const SENT_HOLD: Duration = Duration::from_secs(1);

/// The capsule's word and the VoiceOver announcement for a copied dictation.
pub const COPIED: &str = "Copied";
pub const YOUR_TURN: &str = "Your turn";
pub const YOUR_TURN_NEXT: &str = "Your turn next";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Show {
    /// With the committed words heard so far.
    Recording(String),
    Working,
    YourTurn,
    /// With the committed words heard so far.
    Answering(String),
    NothingHeard(String),
    Broken(String),
    Done(Done),
    Copied,
    Reading,
    /// The questions nobody answered, and the ask of the latest.
    Missed {
        count: u32,
        ask: String,
    },
}

pub fn missed_label(count: u32) -> String {
    match count {
        1 => "You missed a question".to_string(),
        count => format!("You missed {count} questions"),
    }
}

pub fn missed_name(count: u32) -> String {
    match count {
        1 => "You missed a question. Open it in History.".to_string(),
        count => format!("You missed {count} questions. Open them in History."),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    Typed,
    Sent,
}

impl Done {
    pub fn words(self) -> &'static str {
        match self {
            Done::Typed => "Typed",
            Done::Sent => "Sent",
        }
    }

    fn lasts(self) -> Duration {
        match self {
            Done::Typed => FLOAT,
            Done::Sent => SENT_HOLD,
        }
    }
}

/// Which end of a line too long for the capsule gives way.
#[cfg(any(target_os = "macos", target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trim {
    Start,
    End,
}

/// The line a scene shows, and the end cut when it does not fit. Live words
/// keep their newest end; every other line keeps its start.
#[cfg(any(target_os = "macos", target_os = "linux", test))]
pub fn words_of(show: &Show) -> Option<(&str, Trim)> {
    match show {
        Show::Recording(words) | Show::Answering(words) if !words.is_empty() => {
            Some((words, Trim::Start))
        }
        Show::YourTurn | Show::Answering(_) => Some((YOUR_TURN, Trim::End)),
        Show::NothingHeard(text) | Show::Broken(text) => Some((text, Trim::End)),
        Show::Copied => Some((COPIED, Trim::End)),
        Show::Reading => Some((YOUR_TURN_NEXT, Trim::End)),
        Show::Recording(_) | Show::Working | Show::Done(_) | Show::Missed { .. } => None,
    }
}

/// The silence run an answer is in, as the daemon timed it. `teach` shows
/// the hint under the answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Countdown {
    pub since: Instant,
    pub lasts: Duration,
    pub teach: bool,
}

/// Above each share of a countdown left, that many segments stay lit.
const STEPS: [(f64, usize); 3] = [(0.6, 5), (0.2, 3), (0.0, 1)];

#[cfg(any(target_os = "macos", target_os = "linux", test))]
pub fn segments_lit(share: f64) -> usize {
    STEPS
        .iter()
        .find(|&&(above, _)| share > above)
        .map_or(0, |&(_, lit)| lit)
}

pub fn step_deadlines(countdown: Countdown) -> [Instant; 3] {
    STEPS.map(|(above, _)| countdown.since + countdown.lasts.mul_f64(1.0 - above))
}

const BIDI_CONTROLS: [char; 9] = [
    '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}',
    '\u{2069}',
];

fn clean(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_whitespace() || !(c.is_control() || BIDI_CONTROLS.contains(c)))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct Split {
    pub context: String,
    pub ask: String,
    pub after: String,
}

pub fn split(question: &str) -> Split {
    let mut sentences: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    let mut chars = question.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if start.is_none() && !c.is_whitespace() {
            start = Some(at);
        }
        let ends = matches!(c, '.' | '!' | '?')
            && chars.peek().is_none_or(|(_, next)| next.is_whitespace());
        if ends && let Some(from) = start.take() {
            sentences.push((from, at + c.len_utf8()));
        }
    }
    if let Some(from) = start {
        sentences.push((from, question.trim_end().len()));
    }
    let ask = sentences
        .iter()
        .rposition(|&(_, end)| question[..end].ends_with('?'))
        .or(sentences.len().checked_sub(1));
    let Some((from, end)) = ask.map(|index| sentences[index]) else {
        return Split {
            context: String::new(),
            ask: String::new(),
            after: String::new(),
        };
    };
    Split {
        context: question[..from].trim().to_string(),
        ask: question[from..end].to_string(),
        after: question[end..].trim().to_string(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scene {
    pub show: Show,
    pub serial: u64,
    /// Cleaned.
    pub question: Option<String>,
    /// Who asks `question`. Cleaned.
    pub client: Option<String>,
    pub countdown: Option<Countdown>,
}

impl Scene {
    #[cfg(test)]
    pub fn of(show: Show, serial: u64) -> Self {
        Scene {
            show,
            serial,
            question: None,
            client: None,
            countdown: None,
        }
    }
}

/// The flags that say a job still runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub recording: bool,
    pub armed: bool,
    pub transcribing: bool,
    pub loading_model: bool,
}

impl Live {
    pub fn of(state: &serde_json::Value) -> Self {
        Live {
            recording: flag(state, "recording"),
            armed: flag(state, "armed"),
            transcribing: flag(state, "transcribing"),
            loading_model: flag(state, "loading_model"),
        }
    }

    fn busy(self) -> bool {
        self.recording || self.armed || self.transcribing || self.loading_model
    }
}

pub enum Input {
    Signal(Signal),
    Live(Live),
    /// The subscribe reply: the state a tray that starts mid-session inherits.
    Seed(Live),
    /// The committed words of the recording in progress.
    Words(String),
    /// What the agent is asking, and the program that asks. Empty when the ask has ended.
    Question {
        text: String,
        client: Option<String>,
    },
    Clicked,
    Gone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    Recording,
    Working,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Question {
    Reading,
    Asking,
    Answering,
    Holding,
    Working,
}

#[derive(Default)]
pub struct Chip {
    press: Option<Press>,
    question: Option<Question>,
    held: Option<(Show, Instant)>,
    waiting: VecDeque<Show>,
    done: Option<(Done, Instant)>,
    missed: Option<(u32, String)>,
    quiet_since: Option<Instant>,
    live: Option<Live>,
    words: String,
    text: Option<String>,
    client: Option<String>,
    countdown: Option<Countdown>,
    taught: bool,
    ticked: Option<Instant>,
    shown: Option<Shown>,
    renewed: bool,
    serial: u64,
}

type Shown = (Show, Option<String>, Option<String>, Option<Countdown>);

impl Chip {
    pub fn feed(&mut self, input: Input, now: Instant) {
        match input {
            Input::Signal(signal) => {
                self.restart_quiet(now);
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
                self.reset();
                self.seed(live);
            }
            Input::Words(words) => self.words = words,
            Input::Question { text, client } => self.ask(&text, client.as_deref(), now),
            Input::Clicked => self.missed = None,
            Input::Gone => self.reset(),
        }
        self.tick(now);
    }

    pub fn tick(&mut self, now: Instant) {
        self.ticked = Some(now);
        if self
            .held
            .as_ref()
            .is_some_and(|(_, at)| now >= *at + FAULT_HOLD)
        {
            self.held = None;
        }
        if self.done.is_some_and(|(done, at)| now >= at + done.lasts()) {
            self.done = None;
        }
        if self.foreground() && self.quiet_since.is_some_and(|since| now >= since + QUIET) {
            self.press = None;
            self.question = None;
            self.quiet_since = None;
        }
        if self.question != Some(Question::Answering) {
            self.countdown = None;
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

    pub fn deadline(&self) -> Option<Instant> {
        let ticked = self.ticked;
        let steps = (self.countdown.into_iter())
            .flat_map(step_deadlines)
            .filter(|step| ticked.is_none_or(|at| *step > at));
        [
            self.held.as_ref().map(|(_, at)| *at + FAULT_HOLD),
            self.done.map(|(done, at)| at + done.lasts()),
            self.quiet_since
                .filter(|_| self.foreground())
                .map(|since| since + QUIET),
        ]
        .into_iter()
        .flatten()
        .chain(steps)
        .min()
    }

    pub fn scene(&self) -> Option<Scene> {
        self.shown
            .clone()
            .map(|(show, question, client, countdown)| Scene {
                show,
                serial: self.serial,
                question,
                client,
                countdown,
            })
    }

    /// Keeps `serial`, so a later scene still reads as new, and `taught`, so
    /// the hint shows once per tray run.
    fn reset(&mut self) {
        *self = Chip {
            serial: self.serial,
            taught: self.taught,
            ..Chip::default()
        };
    }

    fn restart_quiet(&mut self, now: Instant) {
        self.quiet_since = match self.live {
            Some(live) if !live.busy() => Some(now),
            _ => None,
        };
    }

    fn ask(&mut self, text: &str, client: Option<&str>, now: Instant) {
        if text.is_empty() {
            self.countdown = None;
            self.text = None;
            self.client = None;
            if self.question == Some(Question::Reading) {
                self.question = None;
            }
        } else {
            self.text = Some(clean(text));
            self.client = client.map(clean).filter(|name| !name.is_empty());
            self.question.get_or_insert(Question::Reading);
            self.restart_quiet(now);
            self.begin();
        }
    }

    fn seed(&mut self, live: Live) {
        self.live = Some(live);
        if live.armed {
            self.question = Some(Question::Asking);
        } else if live.recording {
            self.press = Some(Press::Recording);
        } else if live.transcribing {
            self.press = Some(Press::Working);
        }
    }

    fn signal(&mut self, signal: Signal, now: Instant) {
        match signal {
            Signal::RecordStart {
                target: Target::Answer,
            } => {
                self.question = Some(Question::Holding);
                self.words.clear();
                self.begin();
            }
            Signal::RecordStart {
                target: Target::Dictate,
            } => {
                self.press = Some(Press::Recording);
                self.words.clear();
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
            Signal::RecordStop {
                target: Target::Dictate,
            } => self.press = Some(Press::Working),
            Signal::Cancelled { .. } => self.press = None,
            Signal::Arm => {
                if !matches!(self.question, Some(Question::Holding)) {
                    self.question = Some(Question::Asking);
                    self.words.clear();
                }
                self.begin();
            }
            Signal::Disarm => {
                self.question = Some(Question::Working);
            }
            Signal::Answered { heard: true } => {
                self.question = None;
                self.finish(Done::Sent, now);
            }
            Signal::Answered { heard: false } => {
                self.question = None;
                let words = Reason::new(ReasonCode::EmptyTranscript, None).text;
                self.surface(Show::NothingHeard(words), false, now);
            }
            Signal::Ready { .. } => {
                self.release_working_press();
                self.finish(Done::Typed, now);
            }
            Signal::Copied => {
                self.release_working_press();
                self.surface(Show::Copied, false, now);
            }
            Signal::Error { reason, target } => self.error(reason, target, now),
            Signal::Onset => {
                if self.question == Some(Question::Asking) {
                    self.question = Some(Question::Answering);
                }
            }
            Signal::Quiet { remaining_ms } => {
                if self.question == Some(Question::Answering) {
                    let teach = !self.taught || self.countdown.is_some_and(|c| c.teach);
                    self.taught = true;
                    self.countdown = Some(Countdown {
                        since: now,
                        lasts: Duration::from_millis(remaining_ms),
                        teach,
                    });
                }
            }
            Signal::Speech => self.countdown = None,
        }
    }

    fn error(&mut self, reason: Reason, target: Option<Target>, now: Instant) {
        self.countdown = None;
        if reason.code == ReasonCode::Silence && self.question.is_some() {
            self.miss();
            self.end_what_failed(reason.code, target);
            return;
        }
        self.end_what_failed(reason.code, target);
        let waits_behind_others = reason.code == ReasonCode::SpeechFailed;
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
            | ReasonCode::SpeechFailed => Show::Broken(reason.text),
        };
        self.surface(fault, waits_behind_others, now);
    }

    fn miss(&mut self) {
        let ask = self
            .text
            .as_deref()
            .map_or_else(String::new, |text| split(text).ask);
        let count = self.missed.as_ref().map_or(0, |(count, _)| *count) + 1;
        self.missed = Some((count, ask));
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
            Some(Target::Dictate) => {
                if self.working() {
                    self.press = None;
                }
            }
            None => match code {
                ReasonCode::Silence | ReasonCode::Closed | ReasonCode::ListenFailed => {
                    self.question = None;
                }
                ReasonCode::SpeechFailed | ReasonCode::Starting | ReasonCode::PipelineBroken => {}
                ReasonCode::NoSpeech
                | ReasonCode::EmptyTranscript
                | ReasonCode::TypeFailed
                | ReasonCode::TranscriptionFailed => {
                    if self.working() {
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

    fn working(&self) -> bool {
        self.press == Some(Press::Working)
    }

    fn release_working_press(&mut self) {
        if self.working() {
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

    fn resolve(&self) -> Option<Show> {
        if let Some(press) = self.press {
            return Some(match press {
                Press::Recording => Show::Recording(self.words.clone()),
                Press::Working => Show::Working,
            });
        }
        if let Some(question) = self.question {
            return Some(match question {
                Question::Reading => Show::Reading,
                Question::Asking => Show::YourTurn,
                Question::Answering => Show::Answering(self.words.clone()),
                Question::Holding => Show::Recording(self.words.clone()),
                Question::Working => Show::Working,
            });
        }
        if let Some((fault, _)) = &self.held {
            return Some(fault.clone());
        }
        if let Some((done, _)) = self.done {
            return Some(Show::Done(done));
        }
        if let Some((count, ask)) = &self.missed {
            return Some(Show::Missed {
                count: *count,
                ask: ask.clone(),
            });
        }
        None
    }

    fn settle(&mut self) {
        let resolved = self.resolve();
        let renewed = std::mem::take(&mut self.renewed)
            && matches!(resolved, Some(Show::NothingHeard(_) | Show::Broken(_)));
        let question = self.question.and(self.text.clone());
        let client = question.as_ref().and(self.client.clone());
        let shown_show = self.shown.as_ref().map(|(show, ..)| show);
        // New words are the recording going on, not a new state.
        let reworded = matches!(
            (&resolved, shown_show),
            (Some(Show::Recording(_)), Some(Show::Recording(_)))
                | (Some(Show::Answering(_)), Some(Show::Answering(_)))
        );
        let moved = resolved.as_ref() != shown_show;
        let next = resolved.map(|show| (show, question, client, self.countdown));
        if next != self.shown || renewed {
            if (moved || renewed) && !reworded {
                self.serial += 1;
            }
            self.shown = next;
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
        Show::Copied => Some(COPIED.to_string()),
        Show::NothingHeard(text) | Show::Broken(text) => Some(text.clone()),
        Show::Missed { count, .. } => Some(missed_name(*count)),
        Show::Recording(_) | Show::YourTurn | Show::Answering(_) | Show::Reading => None,
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

    fn missed(count: u32, ask: &str) -> Show {
        Show::Missed {
            count,
            ask: ask.to_string(),
        }
    }

    const RECORDING: Show = Show::Recording(String::new());

    const IDLE: Live = Live {
        recording: false,
        armed: false,
        transcribing: false,
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
        assert_eq!(shown(&chip), Some(RECORDING));
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
        assert_eq!(shown(&chip), Some(RECORDING));
        fed(
            &mut chip,
            clock.at(100),
            [Signal::Ready {
                target: Target::Dictate,
            }],
        );
        assert_eq!(
            shown(&chip),
            Some(RECORDING),
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
        assert_eq!(shown(&chip), Some(RECORDING));
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
    fn a_new_press_cancels_a_held_failure_and_a_done_float() {
        let press = Signal::RecordStart {
            target: Target::Dictate,
        };
        let release = Signal::RecordStop {
            target: Target::Dictate,
        };
        let cases = [
            (
                "a held failure",
                vec![press.clone(), release.clone(), error(ReasonCode::NoSpeech)],
                Show::NothingHeard("Nothing heard on AirPods".to_string()),
                400,
                500,
            ),
            (
                "a done float in progress",
                vec![
                    press.clone(),
                    release,
                    Signal::Ready {
                        target: Target::Dictate,
                    },
                ],
                Show::Done(Done::Typed),
                100,
                200,
            ),
        ];

        for (name, signals, shown_first, pressed_at, cancelled_at) in cases {
            let clock = Clock::new();
            let mut chip = Chip::default();
            fed(&mut chip, clock.at(0), signals);
            assert_eq!(shown(&chip), Some(shown_first), "{name}");
            fed(&mut chip, clock.at(pressed_at), [press.clone()]);
            assert_eq!(shown(&chip), Some(RECORDING), "{name}");
            fed(
                &mut chip,
                clock.at(cancelled_at),
                [Signal::Cancelled {
                    target: Target::Dictate,
                }],
            );
            assert_eq!(
                shown(&chip),
                None,
                "{name} was cancelled, not merely covered"
            );
        }
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
    fn a_hold_while_a_question_is_asked_records_whichever_signal_came_first() {
        let hold = Signal::RecordStart {
            target: Target::Answer,
        };
        let cases = [
            (
                "an arm during an armed hold",
                vec![hold.clone(), Signal::Arm],
            ),
            (
                "a hold during the answer",
                vec![Signal::Arm, Signal::Onset, hold],
            ),
        ];

        for (name, signals) in cases {
            let clock = Clock::new();
            let mut chip = Chip::default();
            fed(&mut chip, clock.at(0), signals);
            assert_eq!(shown(&chip), Some(RECORDING), "{name}");
            fed(
                &mut chip,
                clock.at(900),
                [Signal::RecordStop {
                    target: Target::Answer,
                }],
            );
            assert_eq!(shown(&chip), Some(Show::Working), "{name}");
        }
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
        assert_eq!(shown(&chip), Some(RECORDING));
        fed(
            &mut chip,
            clock.at(100),
            [error(ReasonCode::TranscriptionFailed)],
        );
        assert_eq!(
            shown(&chip),
            Some(RECORDING),
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
        let starts = clock.at(200) + SENT_HOLD;
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
        chip.tick(ends + SENT_HOLD);
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
                    target: Target::Dictate,
                },
                Signal::RecordStop {
                    target: Target::Dictate,
                },
                error(ReasonCode::TranscriptionFailed),
            ],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("Could not transcribe".to_string()))
        );
        fed(&mut chip, clock.at(500), [error(ReasonCode::SpeechFailed)]);
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("Could not transcribe".to_string())),
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
        assert_eq!(shown(&chip), Some(RECORDING), "the press keeps showing");
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
        chip.tick(clock.at(200) + SENT_HOLD);
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
    fn a_seeded_press_ends_on_a_dictation_failure() {
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
            [error_of(ReasonCode::TranscriptionFailed, Target::Dictate)],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::Broken("Could not transcribe".to_string()))
        );
    }

    #[test]
    fn a_seeded_press_shows_working_and_survives_a_signal_that_is_not_a_press() {
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
    fn silence_leaves_the_missed_pill_with_the_ask() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        asked(
            &mut chip,
            clock.at(0),
            "The migration renames user_id. Should I update the reports?",
        );
        fed(
            &mut chip,
            clock.at(100),
            [
                Signal::Arm,
                Signal::Disarm,
                error_of(ReasonCode::Silence, Target::Answer),
            ],
        );
        let pill = missed(1, "Should I update the reports?");
        assert_eq!(shown(&chip), Some(pill.clone()));
        assert_eq!(chip.deadline(), None);
        chip.tick(clock.at(100) + Duration::from_secs(3600));
        assert_eq!(shown(&chip), Some(pill));
        chip.feed(Input::Clicked, clock.at(100) + Duration::from_secs(3601));
        assert_eq!(shown(&chip), None);
    }

    #[test]
    fn the_missed_pill_steps_aside_for_a_question_and_counts_a_second_miss() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        asked(&mut chip, clock.at(0), "Deploy now?");
        fed(
            &mut chip,
            clock.at(100),
            [
                Signal::Arm,
                Signal::Disarm,
                error_of(ReasonCode::Silence, Target::Answer),
            ],
        );
        assert_eq!(shown(&chip), Some(missed(1, "Deploy now?")));

        asked(&mut chip, clock.at(200), "Deploy now?");
        assert_eq!(shown(&chip), Some(Show::Reading));
        fed(
            &mut chip,
            clock.at(300),
            [
                Signal::Arm,
                Signal::Disarm,
                Signal::Answered { heard: true },
            ],
        );
        assert_eq!(shown(&chip), Some(Show::Done(Done::Sent)));
        chip.tick(clock.at(300) + SENT_HOLD);
        assert_eq!(shown(&chip), Some(missed(1, "Deploy now?")));

        let later = clock.at(5000);
        asked(&mut chip, later, "Deploy now?");
        fed(
            &mut chip,
            later,
            [
                Signal::Arm,
                Signal::Disarm,
                error_of(ReasonCode::Silence, Target::Answer),
            ],
        );
        assert_eq!(shown(&chip), Some(missed(2, "Deploy now?")));

        fed(
            &mut chip,
            clock.at(6000),
            [Signal::RecordStart {
                target: Target::Dictate,
            }],
        );
        assert_eq!(shown(&chip), Some(RECORDING));
        fed(
            &mut chip,
            clock.at(6100),
            [Signal::Cancelled {
                target: Target::Dictate,
            }],
        );
        assert_eq!(shown(&chip), Some(missed(2, "Deploy now?")));
    }

    #[test]
    fn a_silence_with_no_question_text_still_counts() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        chip.feed(
            Input::Seed(Live {
                armed: true,
                ..IDLE
            }),
            clock.at(0),
        );
        fed(
            &mut chip,
            clock.at(100),
            [
                Signal::Disarm,
                error_of(ReasonCode::Silence, Target::Answer),
            ],
        );
        assert_eq!(shown(&chip), Some(missed(1, "")));
    }

    #[test]
    fn a_closed_question_is_not_a_miss() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [Signal::Arm, error_of(ReasonCode::Closed, Target::Answer)],
        );
        assert_eq!(
            shown(&chip),
            Some(Show::NothingHeard("The question was closed".to_string()))
        );
        assert_eq!(chip.missed, None);
    }

    #[test]
    fn voiceover_names_the_missed_pill_as_a_button() {
        let after = Scene::of(missed(2, "Deploy now?"), 1);
        assert_eq!(
            announcement(None, Some(&after)).as_deref(),
            Some("You missed 2 questions. Open them in History.")
        );
        assert_eq!(missed_label(1), "You missed a question");
        assert_eq!(missed_label(2), "You missed 2 questions");
        assert_eq!(missed_name(1), "You missed a question. Open it in History.");
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
        assert_eq!(shown(&chip), Some(missed(1, "")));
    }

    // The arm cue waits for the microphone's first sound, so an answer that
    // ends before any sound came disarms with no arm before it.
    #[test]
    fn an_answer_that_ends_before_its_arm_cue_shows_how_it_ended() {
        for (code, show) in [
            (ReasonCode::Silence, missed(1, "")),
            (
                ReasonCode::Closed,
                Show::NothingHeard("The question was closed".to_string()),
            ),
            (
                ReasonCode::ListenFailed,
                Show::Broken("Could not listen".to_string()),
            ),
        ] {
            let clock = Clock::new();
            let mut chip = Chip::default();
            fed(
                &mut chip,
                clock.at(0),
                [Signal::Disarm, error_of(code, Target::Answer)],
            );
            assert_eq!(shown(&chip), Some(show), "{code:?}");
            assert_eq!(chip.question, None, "{code:?}");
        }
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
        assert_eq!(answering.show, Show::Answering(String::new()));
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
        assert_eq!(shown(&chip), Some(RECORDING));
        fed(&mut chip, clock.at(200), [Signal::Disarm, Signal::Onset]);
        assert_eq!(shown(&chip), Some(Show::Working));
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
            assert_eq!(shown(&chip), Some(Show::Answering(String::new())));
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

        for (code, show) in [
            (ReasonCode::Silence, missed(1, "")),
            (
                ReasonCode::Closed,
                Show::NothingHeard("The question was closed".to_string()),
            ),
        ] {
            let mut chip = answering();
            fed(&mut chip, clock.at(900), [error_of(code, Target::Answer)]);
            assert_eq!(shown(&chip), Some(show));
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
        chip.feed(
            Input::Question {
                text: "Ship it?".to_string(),
                client: None,
            },
            clock.at(0),
        );
        assert_eq!(
            announcement(before.as_ref(), chip.scene().as_ref()),
            None,
            "the reading state is not announced"
        );
        chip.feed(
            Input::Question {
                text: String::new(),
                client: None,
            },
            clock.at(0),
        );
        before = chip.scene();
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

    fn dictating(clock: &Clock, words: &str) -> Chip {
        let mut chip = Chip::default();
        fed(
            &mut chip,
            clock.at(0),
            [Signal::RecordStart {
                target: Target::Dictate,
            }],
        );
        chip.feed(Input::Words(words.to_string()), clock.at(300));
        chip
    }

    #[test]
    fn live_words_keep_the_newest_end() {
        let clock = Clock::new();
        let long = "so the first thing I want to say is that the newest words matter most";
        let chip = dictating(&clock, long);
        let show = shown(&chip).expect("a dictation shows");
        assert_eq!(show, Show::Recording(long.to_string()));
        assert_eq!(words_of(&show), Some((long, Trim::Start)));
        let failed = Show::Broken(long.to_string());
        assert_eq!(
            words_of(&failed),
            Some((long, Trim::End)),
            "a failure keeps the start, where it says what broke"
        );
        assert_eq!(words_of(&RECORDING), None, "no words, no line");
    }

    #[test]
    fn a_new_recording_starts_with_no_words() {
        let clock = Clock::new();
        let mut typed = dictating(&clock, "the last dictation");
        fed(
            &mut typed,
            clock.at(900),
            [
                Signal::RecordStop {
                    target: Target::Dictate,
                },
                Signal::Ready {
                    target: Target::Dictate,
                },
            ],
        );
        let mut cancelled = dictating(&clock, "a mistake");
        fed(
            &mut cancelled,
            clock.at(900),
            [Signal::Cancelled {
                target: Target::Dictate,
            }],
        );
        for (name, mut chip) in [("after a typed one", typed), ("after a cancel", cancelled)] {
            fed(
                &mut chip,
                clock.at(1000),
                [Signal::RecordStart {
                    target: Target::Dictate,
                }],
            );
            assert_eq!(shown(&chip), Some(RECORDING), "{name}");
        }

        let mut held = Chip::default();
        fed(&mut held, clock.at(0), [Signal::Arm]);
        held.feed(Input::Words("an earlier answer".to_string()), clock.at(100));
        fed(
            &mut held,
            clock.at(200),
            [Signal::RecordStart {
                target: Target::Answer,
            }],
        );
        assert_eq!(shown(&held), Some(RECORDING), "a held answer");
    }

    #[test]
    fn voiceover_hears_no_live_words() {
        let clock = Clock::new();
        let mut chip = dictating(&clock, "");
        let before = chip.scene();
        chip.feed(Input::Words("hello there".to_string()), clock.at(400));
        let after = chip.scene();
        assert_eq!(
            after.as_ref().map(|scene| &scene.show),
            Some(&Show::Recording("hello there".to_string()))
        );
        assert_eq!(
            before.as_ref().map(|scene| scene.serial),
            after.as_ref().map(|scene| scene.serial),
            "new words are the same state, so nothing plays or speaks again"
        );
        assert_eq!(announcement(before.as_ref(), after.as_ref()), None);
        assert_eq!(announcement(None, after.as_ref()), None);
    }

    #[test]
    fn an_answer_shows_its_words_once_any_arrive() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        chip.feed(Input::Words("the last dictation".to_string()), clock.at(0));
        fed(&mut chip, clock.at(0), [Signal::Arm, Signal::Onset]);
        let before = chip.scene();
        let waiting = before.as_ref().map(|scene| scene.show.clone()).unwrap();
        assert_eq!(waiting, Show::Answering(String::new()), "a new question");
        assert_eq!(words_of(&waiting), Some(("Your turn", Trim::End)));

        let long = "yes please go ahead and run the whole suite before you push it";
        chip.feed(Input::Words(long.to_string()), clock.at(500));
        let after = chip.scene();
        let speaking = after.as_ref().map(|scene| scene.show.clone()).unwrap();
        assert_eq!(speaking, Show::Answering(long.to_string()));
        assert_eq!(words_of(&speaking), Some((long, Trim::Start)));
        assert_eq!(
            before.as_ref().map(|scene| scene.serial),
            after.as_ref().map(|scene| scene.serial),
            "new words are the same state"
        );
        assert_eq!(announcement(before.as_ref(), after.as_ref()), None);
        assert_eq!(announcement(None, after.as_ref()), None);
    }

    #[test]
    fn a_copied_dictation_holds_its_word_then_hides() {
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
        fed(&mut chip, clock.at(500), [Signal::Copied]);
        assert_eq!(shown(&chip), Some(Show::Copied));
        assert_eq!(words_of(&Show::Copied), Some((COPIED, Trim::End)));
        assert_eq!(chip.deadline(), Some(clock.at(500) + FAULT_HOLD));
        chip.tick(clock.at(500) + FAULT_HOLD);
        assert_eq!(shown(&chip), None);
    }

    #[test]
    fn a_copy_under_a_newer_press_shows_once_the_press_ends() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        let press = Signal::RecordStart {
            target: Target::Dictate,
        };
        let release = Signal::RecordStop {
            target: Target::Dictate,
        };
        fed(
            &mut chip,
            clock.at(0),
            [press.clone(), release.clone(), press],
        );
        fed(&mut chip, clock.at(100), [Signal::Copied]);
        assert_eq!(shown(&chip), Some(RECORDING));
        fed(
            &mut chip,
            clock.at(200),
            [
                release,
                Signal::Ready {
                    target: Target::Dictate,
                },
            ],
        );
        assert_eq!(shown(&chip), Some(Show::Done(Done::Typed)));
        chip.tick(clock.at(200) + FLOAT);
        assert_eq!(shown(&chip), Some(Show::Copied));
    }

    #[test]
    fn voiceover_hears_copied() {
        let after = Scene::of(Show::Copied, 1);
        assert_eq!(announcement(None, Some(&after)).as_deref(), Some(COPIED));
    }

    fn asked(chip: &mut Chip, at: Instant, text: &str) {
        chip.feed(
            Input::Question {
                text: text.to_string(),
                client: None,
            },
            at,
        );
    }

    fn question_of(chip: &Chip) -> Option<String> {
        chip.scene().and_then(|scene| scene.question)
    }

    fn asked_by(chip: &mut Chip, at: Instant, text: &str, client: Option<&str>) {
        chip.feed(
            Input::Question {
                text: text.to_string(),
                client: client.map(str::to_string),
            },
            at,
        );
    }

    fn client_of(chip: &Chip) -> Option<String> {
        chip.scene().and_then(|scene| scene.client)
    }

    #[test]
    fn a_question_shows_who_asks_and_an_ended_one_shows_nobody() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        asked_by(&mut chip, clock.at(0), "Ship it?", Some("claude"));
        assert_eq!(client_of(&chip).as_deref(), Some("claude"));
        asked_by(&mut chip, clock.at(0), "Ship it now?", Some("codex"));
        assert_eq!(client_of(&chip).as_deref(), Some("codex"));
        fed(&mut chip, clock.at(0), [Signal::Arm, Signal::Onset]);
        assert_eq!(client_of(&chip).as_deref(), Some("codex"));
        asked_by(&mut chip, clock.at(0), "", None);
        let ended = chip.scene().expect("the answer still shows");
        assert_eq!((ended.question, ended.client), (None, None));
        asked_by(&mut chip, clock.at(0), "Ship it?", None);
        assert_eq!(question_of(&chip).as_deref(), Some("Ship it?"));
        assert_eq!(client_of(&chip), None);
    }

    #[test]
    fn a_dictation_scene_has_no_client() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        asked_by(&mut chip, clock.at(0), "Ship it?", Some("claude"));
        asked_by(&mut chip, clock.at(0), "", None);
        fed(
            &mut chip,
            clock.at(0),
            [Signal::RecordStart {
                target: Target::Dictate,
            }],
        );
        assert_eq!(shown(&chip), Some(RECORDING));
        assert_eq!(client_of(&chip), None);
    }

    #[test]
    fn a_question_shows_as_reading_then_your_turn_with_its_text() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        asked(&mut chip, clock.at(0), "Ship it?");
        assert_eq!(shown(&chip), Some(Show::Reading));
        assert_eq!(question_of(&chip).as_deref(), Some("Ship it?"));
        fed(&mut chip, clock.at(0), [Signal::Arm]);
        assert_eq!(shown(&chip), Some(Show::YourTurn));
        assert_eq!(question_of(&chip).as_deref(), Some("Ship it?"));
        fed(&mut chip, clock.at(0), [Signal::Onset]);
        assert_eq!(shown(&chip), Some(Show::Answering(String::new())));
        assert_eq!(question_of(&chip).as_deref(), Some("Ship it?"));
        fed(&mut chip, clock.at(0), [Signal::Disarm]);
        assert_eq!(shown(&chip), Some(Show::Working));
        assert_eq!(question_of(&chip).as_deref(), Some("Ship it?"));
        fed(&mut chip, clock.at(0), [Signal::Answered { heard: true }]);
        assert_eq!(shown(&chip), Some(Show::Done(Done::Sent)));
        assert_eq!(question_of(&chip), None);
    }

    #[test]
    fn a_sent_answer_says_sent_alone_for_the_sent_hold() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        chip.feed(
            Input::Question {
                text: "Update them?".into(),
                client: None,
            },
            clock.at(0),
        );
        fed(&mut chip, clock.at(0), [Signal::Arm, Signal::Onset]);
        chip.feed(Input::Words("yes, update them".into()), clock.at(100));
        fed(
            &mut chip,
            clock.at(200),
            [Signal::Disarm, Signal::Answered { heard: true }],
        );
        let sent = Show::Done(Done::Sent);
        assert_eq!(shown(&chip), Some(sent.clone()));
        let scene = Scene::of(sent, 1);
        assert_eq!(announcement(None, Some(&scene)), Some("Sent".to_string()));
        assert_eq!(chip.deadline(), Some(clock.at(200) + SENT_HOLD));
        chip.tick(clock.at(200) + SENT_HOLD);
        assert_eq!(shown(&chip), None);
    }

    #[test]
    fn a_question_withdrawn_before_the_arm_leaves_no_card() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        asked(&mut chip, clock.at(0), "Ship it?");
        asked(&mut chip, clock.at(0), "");
        assert_eq!(shown(&chip), None);
        asked(&mut chip, clock.at(0), "Ship it?");
        fed(&mut chip, clock.at(0), [Signal::Arm, Signal::Onset]);
        asked(&mut chip, clock.at(0), "");
        assert_eq!(shown(&chip), Some(Show::Answering(String::new())));
        assert_eq!(question_of(&chip), None);
    }

    #[test]
    fn a_hold_during_the_question_keeps_the_card() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        asked(&mut chip, clock.at(0), "Ship it?");
        fed(
            &mut chip,
            clock.at(0),
            [Signal::RecordStart {
                target: Target::Answer,
            }],
        );
        assert_eq!(shown(&chip), Some(RECORDING));
        assert_eq!(question_of(&chip).as_deref(), Some("Ship it?"));
    }

    #[test]
    fn a_tray_seeded_mid_question_shows_the_card_once_the_text_arrives() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        chip.feed(
            Input::Seed(Live {
                armed: true,
                ..IDLE
            }),
            clock.at(0),
        );
        let seeded = chip.scene().expect("the seeded ask shows");
        assert_eq!(seeded.show, Show::YourTurn);
        assert_eq!(seeded.question, None);
        asked(&mut chip, clock.at(0), "Ship it?");
        let arrived = chip.scene().expect("the ask still shows");
        assert_eq!(arrived.show, Show::YourTurn);
        assert_eq!(arrived.question.as_deref(), Some("Ship it?"));
        assert_eq!(arrived.serial, seeded.serial);
    }

    #[test]
    fn a_dictation_cancel_leaves_the_question_alone() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        asked(&mut chip, clock.at(0), "Ship it?");
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::Arm,
                Signal::Cancelled {
                    target: Target::Dictate,
                },
            ],
        );
        assert_eq!(shown(&chip), Some(Show::YourTurn));
        assert_eq!(question_of(&chip).as_deref(), Some("Ship it?"));
    }

    #[test]
    fn a_text_change_is_not_a_new_state() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        asked(&mut chip, clock.at(0), "Ship it?");
        let first = chip.scene().expect("the question shows");
        asked(&mut chip, clock.at(0), "Ship it now?");
        let second = chip.scene().expect("the question still shows");
        assert_eq!(second.show, Show::Reading);
        assert_eq!(second.question.as_deref(), Some("Ship it now?"));
        assert_eq!(second.serial, first.serial);
    }

    #[test]
    fn a_question_after_an_idle_spell_shows_at_once() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        chip.feed(Input::Live(IDLE), clock.at(0));
        asked(&mut chip, clock.at(5000), "Ship it?");
        assert_eq!(shown(&chip), Some(Show::Reading));
        assert_eq!(question_of(&chip).as_deref(), Some("Ship it?"));
    }

    #[test]
    fn the_shown_text_is_cleaned() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        asked(&mut chip, clock.at(0), "A\u{202E}b?\n");
        assert_eq!(question_of(&chip).as_deref(), Some("Ab?"));
    }

    #[test]
    fn the_shown_name_is_cleaned_and_a_name_of_controls_alone_is_none() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        asked_by(&mut chip, clock.at(0), "Ship it?", Some("cla\u{202E}ude\n"));
        assert_eq!(client_of(&chip).as_deref(), Some("claude"));
        asked_by(&mut chip, clock.at(0), "Ship it?", Some("\u{202E}\u{7}"));
        assert_eq!(client_of(&chip), None);
    }

    fn answering(chip: &mut Chip, at: Instant) {
        chip.feed(
            Input::Question {
                text: "Retry it?".into(),
                client: None,
            },
            at,
        );
        fed(chip, at, [Signal::Arm, Signal::Onset]);
    }

    fn quiet(millis: u64) -> Signal {
        Signal::Quiet {
            remaining_ms: millis,
        }
    }

    fn counting(chip: &Chip) -> Option<Countdown> {
        chip.scene().and_then(|scene| scene.countdown)
    }

    #[test]
    fn a_pause_starts_the_countdown_the_daemon_names() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        answering(&mut chip, clock.at(0));
        let serial = chip.scene().unwrap().serial;
        fed(&mut chip, clock.at(900), [quiet(2464)]);
        let scene = chip.scene().unwrap();
        assert_eq!(
            scene.countdown,
            Some(Countdown {
                since: clock.at(900),
                lasts: Duration::from_millis(2464),
                teach: true,
            })
        );
        assert_eq!(scene.serial, serial);
        fed(&mut chip, clock.at(1500), [Signal::Speech]);
        let scene = chip.scene().unwrap();
        assert_eq!((scene.countdown, scene.serial), (None, serial));
        fed(&mut chip, clock.at(2000), [quiet(2464)]);
        assert_eq!(counting(&chip).map(|c| c.teach), Some(false));
    }

    #[test]
    fn a_second_quiet_refills_the_countdown() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        answering(&mut chip, clock.at(0));
        fed(&mut chip, clock.at(1000), [quiet(2500)]);
        fed(&mut chip, clock.at(1500), [quiet(2500)]);
        assert_eq!(counting(&chip).unwrap().since, clock.at(1500));
    }

    #[test]
    fn the_countdown_waits_on_its_steps() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        answering(&mut chip, clock.at(0));
        fed(&mut chip, clock.at(100), [quiet(2500)]);
        assert_eq!(chip.deadline(), Some(clock.at(1100)));
        chip.tick(clock.at(1100));
        assert!(counting(&chip).is_some());
        assert_eq!(chip.deadline(), Some(clock.at(2100)));
    }

    #[test]
    fn nothing_counts_before_speech_or_while_the_key_is_held() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        chip.feed(
            Input::Question {
                text: "Retry it?".into(),
                client: None,
            },
            clock.at(0),
        );
        fed(&mut chip, clock.at(0), [Signal::Arm, quiet(2500)]);
        assert_eq!(counting(&chip), None);
        let mut chip = Chip::default();
        chip.feed(
            Input::Question {
                text: "Retry it?".into(),
                client: None,
            },
            clock.at(0),
        );
        fed(
            &mut chip,
            clock.at(0),
            [
                Signal::RecordStart {
                    target: Target::Answer,
                },
                quiet(2500),
            ],
        );
        assert_eq!(counting(&chip), None);
    }

    #[test]
    fn every_end_of_the_answer_ends_the_countdown() {
        let clock = Clock::new();
        let ends = [
            Input::Signal(Signal::Disarm),
            Input::Signal(Signal::RecordStart {
                target: Target::Answer,
            }),
            Input::Signal(error_of(ReasonCode::Silence, Target::Answer)),
            Input::Question {
                text: String::new(),
                client: None,
            },
            Input::Gone,
        ];
        for end in ends {
            let mut chip = Chip::default();
            answering(&mut chip, clock.at(0));
            fed(&mut chip, clock.at(100), [quiet(2500)]);
            assert!(counting(&chip).is_some());
            chip.feed(end, clock.at(200));
            assert_eq!(counting(&chip), None);
        }
    }

    #[test]
    fn words_do_not_reset_the_countdown() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        answering(&mut chip, clock.at(0));
        fed(&mut chip, clock.at(100), [quiet(2500)]);
        chip.feed(Input::Words("more".into()), clock.at(300));
        assert_eq!(counting(&chip).unwrap().since, clock.at(100));
    }

    #[test]
    fn the_hint_shows_once_per_tray_run() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        answering(&mut chip, clock.at(0));
        fed(&mut chip, clock.at(100), [quiet(2500)]);
        assert_eq!(counting(&chip).map(|c| c.teach), Some(true));
        fed(&mut chip, clock.at(200), [Signal::Speech]);
        fed(
            &mut chip,
            clock.at(300),
            [Signal::Disarm, Signal::Answered { heard: true }],
        );
        chip.feed(
            Input::Question {
                text: String::new(),
                client: None,
            },
            clock.at(300),
        );
        answering(&mut chip, clock.at(5000));
        fed(&mut chip, clock.at(5100), [quiet(2500)]);
        assert_eq!(counting(&chip).map(|c| c.teach), Some(false));
    }

    #[test]
    fn a_second_quiet_keeps_the_hint() {
        let clock = Clock::new();
        let mut chip = Chip::default();
        answering(&mut chip, clock.at(0));
        fed(&mut chip, clock.at(1000), [quiet(2500)]);
        fed(&mut chip, clock.at(1500), [quiet(2500)]);
        assert_eq!(counting(&chip).map(|c| c.teach), Some(true));
    }

    #[test]
    fn controls_and_bidi_marks_leave_the_shown_text() {
        assert_eq!(
            clean("Ship\u{202E}it?\u{0007} now\n\tplease"),
            "Shipit? now please"
        );
        assert_eq!(clean("a \u{7} b"), "a b");
        assert_eq!(
            clean("Keep \u{2067}this\u{2069} \u{202E} file?"),
            "Keep this file?"
        );
    }

    #[test]
    fn the_ask_is_the_last_question_sentence() {
        let parts =
            split("The migration renames user_id. Should I update the reports? I lean yes.");
        assert_eq!(parts.context, "The migration renames user_id.");
        assert_eq!(parts.ask, "Should I update the reports?");
        assert_eq!(parts.after, "I lean yes.");

        let parts = split("Both files set it. Tell me which to keep.");
        assert_eq!(parts.ask, "Tell me which to keep.");
        assert_eq!(parts.after, "");

        assert_eq!(split("Deploy?").context, "");
    }

    fn countdown(millis: u64) -> Countdown {
        Countdown {
            since: Instant::now(),
            lasts: Duration::from_millis(millis),
            teach: false,
        }
    }

    #[test]
    fn segments_go_out_from_both_ends() {
        for (share, lit) in [
            (1.0, 5),
            (0.61, 5),
            (0.6, 3),
            (0.21, 3),
            (0.2, 1),
            (0.01, 1),
            (0.0, 0),
        ] {
            assert_eq!(segments_lit(share), lit, "share {share}");
        }
    }

    #[test]
    fn step_deadlines_fall_where_the_lit_count_changes() {
        let c = countdown(2500);
        let at = |millis| c.since + Duration::from_millis(millis);
        assert_eq!(step_deadlines(c), [at(1000), at(2000), at(2500)]);
        for (step, lit) in step_deadlines(c).into_iter().zip([3, 1, 0]) {
            let share = 1.0 - (step - c.since).as_secs_f64() / c.lasts.as_secs_f64();
            assert_eq!(segments_lit(share), lit);
        }
    }
}

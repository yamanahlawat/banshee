use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rodio::source::{SineWave, Source};
use tokio::sync::{broadcast, watch};

use crate::config::FeedbackMode;
use crate::text_to_speech::ActiveUtterance;
use crate::text_to_speech::output::{Chunk, Output};

pub use banshee_common::cue::{Reason, ReasonCode, Signal, Target};

/// A receiver that falls this far behind loses the oldest signals.
const SIGNAL_BACKLOG: usize = 128;

/// How often a cue that is playing is given the chance to follow the device.
const CUE_POLL: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    RecordStart,
    RecordStop,
    Ready,
    Error,
    // The only signal that an armed mic went hot or shut
    Arm,
    Disarm,
}

impl Cue {
    // (frequency Hz, duration ms) pairs played back to back
    const fn tones(self) -> &'static [(f32, u64)] {
        match self {
            Cue::RecordStart => &[(660.0, 70), (880.0, 90)],
            Cue::RecordStop => &[(880.0, 70), (660.0, 90)],
            Cue::Ready => &[(523.0, 90), (784.0, 140)],
            Cue::Error => &[(220.0, 120), (196.0, 160)],
            Cue::Arm => &[(523.0, 70), (1046.0, 120)],
            Cue::Disarm => &[(1046.0, 70), (523.0, 120)],
        }
    }

    /// For a caller that waits for the cue to finish.
    pub const fn duration_ms(self) -> u64 {
        let tones = self.tones();
        let mut total = 0;
        let mut index = 0;
        while index < tones.len() {
            total += tones[index].1;
            index += 1;
        }
        total
    }
}

pub fn sounds(mode: FeedbackMode, drawn: bool) -> bool {
    match mode {
        FeedbackMode::Off => false,
        FeedbackMode::Sound | FeedbackMode::Both => true,
        FeedbackMode::Visual => !drawn,
    }
}

impl From<crate::state::TranscribeTarget> for Target {
    fn from(target: crate::state::TranscribeTarget) -> Self {
        match target {
            crate::state::TranscribeTarget::Dictate => Target::Dictate,
            crate::state::TranscribeTarget::Mailbox => Target::Mailbox,
            crate::state::TranscribeTarget::Tell => Target::Tell,
        }
    }
}

pub fn broken(fault: &crate::state::RecordingError) -> Reason {
    Reason {
        code: ReasonCode::PipelineBroken,
        text: crate::speech_to_text::local::languages::capitalised(&fault.consequence()),
    }
}

pub fn cue_of(signal: &Signal) -> Option<Cue> {
    match signal {
        Signal::RecordStart { .. } => Some(Cue::RecordStart),
        Signal::RecordStop { .. } => Some(Cue::RecordStop),
        Signal::Ready { .. } => Some(Cue::Ready),
        Signal::Error { .. } => Some(Cue::Error),
        Signal::Arm => Some(Cue::Arm),
        Signal::Disarm => Some(Cue::Disarm),
        Signal::Answered { .. } | Signal::Told | Signal::Cancelled { .. } | Signal::Onset => None,
    }
}

#[derive(Clone)]
struct Gate {
    mode: Arc<watch::Sender<FeedbackMode>>,
    drawers: Arc<AtomicUsize>,
}

impl Gate {
    fn mode(&self) -> FeedbackMode {
        *self.mode.borrow()
    }

    fn sounds(&self) -> bool {
        sounds(self.mode(), self.drawers.load(Ordering::Relaxed) > 0)
    }
}

#[derive(Clone)]
pub struct Cues {
    sender: mpsc::Sender<Cue>,
    gate: Gate,
    signals: broadcast::Sender<Signal>,
}

impl Cues {
    /// A cue nobody can hear is not an error, so this swallows a dead player.
    fn send(&self, cue: Cue) {
        if self.gate.sounds() {
            let _ = self.sender.send(cue);
        }
    }

    /// The signal goes out whatever the mode.
    pub fn emit(&self, signal: Signal) {
        if let Some(cue) = cue_of(&signal) {
            self.send(cue);
        }
        // `send` fails only when no one subscribes, which is not a fault.
        let _ = self.signals.send(signal);
    }

    pub fn subscribe_signals(&self) -> broadcast::Receiver<Signal> {
        self.signals.subscribe()
    }

    pub fn mode(&self) -> FeedbackMode {
        self.gate.mode()
    }

    pub fn set_mode(&self, mode: FeedbackMode) {
        self.gate.mode.send_replace(mode);
    }

    pub fn subscribe_mode(&self) -> watch::Receiver<FeedbackMode> {
        self.gate.mode.subscribe()
    }

    fn with_sender(sender: mpsc::Sender<Cue>, mode: FeedbackMode) -> Self {
        Cues {
            sender,
            gate: Gate {
                mode: Arc::new(watch::channel(mode).0),
                drawers: Arc::new(AtomicUsize::new(0)),
            },
            signals: broadcast::channel(SIGNAL_BACKLOG).0,
        }
    }

    #[cfg(test)]
    pub fn silent() -> Self {
        Cues::with_sender(mpsc::channel().0, FeedbackMode::Off)
    }

    #[cfg(test)]
    pub fn recording() -> (Self, mpsc::Receiver<Cue>) {
        Cues::recording_in(FeedbackMode::Both)
    }

    #[cfg(test)]
    pub fn recording_in(mode: FeedbackMode) -> (Self, mpsc::Receiver<Cue>) {
        let (sender, receiver) = mpsc::channel();
        (Cues::with_sender(sender, mode), receiver)
    }

    #[cfg(test)]
    pub fn drawers(&self) -> usize {
        self.gate.drawers.load(Ordering::Relaxed)
    }

    pub fn drawn(&self) -> Drawn {
        self.gate.drawers.fetch_add(1, Ordering::Relaxed);
        Drawn(Arc::clone(&self.gate.drawers))
    }
}

/// One client that draws the cues. Its drop is the client going away.
pub struct Drawn(Arc<AtomicUsize>);

impl Drop for Drawn {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Opens no output device until the first cue it must play, so a mode with no
/// sound holds no audio hardware.
pub fn start_cue_player(mode: FeedbackMode, output: Arc<Output>) -> Cues {
    let (sender, receiver) = mpsc::channel::<Cue>();
    let cues = Cues::with_sender(sender, mode);
    let gate = cues.gate.clone();

    // The thread lives whatever the mode, so a mode that sounds again finds it listening.
    thread::spawn(move || {
        // A cue that cannot play is not a reply that was not spoken, so its
        // faults stay out of `last_speech_error`, and the receiver goes rather
        // than buffering them for the life of the daemon. `play` logs them.
        let (faults, unread) = mpsc::channel();
        drop(unread);
        serve_cues(receiver, &gate, |cue| play(&output, cue, &faults));
    });

    cues
}

/// The mode can change while a cue waits behind another, so each is asked again
/// as it comes up.
fn serve_cues(receiver: mpsc::Receiver<Cue>, gate: &Gate, mut play: impl FnMut(Cue)) {
    for cue in receiver {
        if gate.sounds() {
            play(cue);
        }
    }
}

/// Plays one cue through the daemon's output and stays with it to the end, so a
/// device that dies mid-cue is replaced the way it is mid-sentence. The wait is
/// what keeps two cues from overlapping.
fn play(output: &Arc<Output>, cue: Cue, faults: &mpsc::Sender<crate::text_to_speech::Fault>) {
    let tones: Vec<Chunk> = cue
        .tones()
        .iter()
        .map(|&(frequency, ms)| chunk(frequency, ms))
        .collect();
    let mut playing = match output.play(tones.into_iter(), faults.clone()) {
        Ok(playing) => playing,
        Err(e) => {
            log::warn!("no cue was played, there is no output device: {e}");
            return;
        }
    };
    while !playing.is_finished() {
        thread::sleep(CUE_POLL);
    }
}

/// The same tone the cue player has always sounded, as samples the output takes.
fn chunk(frequency: f32, ms: u64) -> Chunk {
    let source = tone(frequency, ms);
    let rate = source.sample_rate();
    let channels = source.channels();
    Chunk {
        samples: source.collect(),
        rate,
        channels,
    }
}

fn tone(frequency: f32, ms: u64) -> impl Source + Send {
    let mut tone = SineWave::new(frequency).take_duration(Duration::from_millis(ms));
    // Fade the tail to avoid an audible click at the cut
    tone.set_filter_fadeout();
    // Measured at 7.1 dB above the voice it plays beside.
    tone.amplify(0.20)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_silent_signals_have_no_cue() {
        assert_eq!(cue_of(&Signal::Answered { heard: true }), None);
        assert_eq!(cue_of(&Signal::Told), None);
        assert_eq!(cue_of(&Signal::Onset), None);
        assert_eq!(
            cue_of(&Signal::Cancelled {
                target: Target::Dictate
            }),
            None
        );
    }

    #[test]
    fn a_signal_reaches_a_subscriber_whatever_the_mode() {
        let (cues, heard) = Cues::recording_in(FeedbackMode::Off);
        let mut signals = cues.subscribe_signals();
        cues.emit(Signal::Ready {
            target: Target::Dictate,
        });
        assert!(matches!(signals.try_recv(), Ok(Signal::Ready { .. })));
        assert!(heard.try_recv().is_err(), "none still plays nothing");
    }

    #[test]
    fn nothing_heard_names_the_device_when_there_is_one() {
        assert_eq!(
            Reason::new(ReasonCode::NoSpeech, None).text,
            "Nothing heard"
        );
        assert_eq!(
            Reason::new(ReasonCode::EmptyTranscript, Some("USB Mic")).text,
            "Nothing heard on USB Mic"
        );
    }

    #[test]
    fn every_cue_has_audible_tones() {
        for cue in [
            Cue::RecordStart,
            Cue::RecordStop,
            Cue::Ready,
            Cue::Error,
            Cue::Arm,
            Cue::Disarm,
        ] {
            for &(frequency, ms) in cue.tones() {
                assert!((100.0..=2000.0).contains(&frequency));
                assert!((30..=500).contains(&ms));
            }
        }
    }

    #[test]
    fn the_mode_and_whether_a_chip_draws_decide_what_sounds() {
        let cases = [
            (FeedbackMode::Off, false, false),
            (FeedbackMode::Off, true, false),
            (FeedbackMode::Sound, false, true),
            (FeedbackMode::Sound, true, true),
            (FeedbackMode::Both, false, true),
            (FeedbackMode::Both, true, true),
            (FeedbackMode::Visual, false, true),
            (FeedbackMode::Visual, true, false),
        ];
        for (mode, drawn, heard) in cases {
            assert_eq!(sounds(mode, drawn), heard, "{mode:?} drawn {drawn}");
        }
    }

    // The gate reads only the mode and the drawn count, so a dictation, an
    // agent's question, a tell failure and a speech failure share one answer.
    #[test]
    fn a_chip_in_visual_silences_every_job_and_no_chip_sounds_them_all() {
        let jobs = [
            Signal::RecordStart {
                target: Target::Dictate,
            },
            Signal::Error {
                reason: Reason::new(ReasonCode::ListenFailed, None),
                target: Some(Target::Answer),
            },
            Signal::Error {
                reason: Reason::new(ReasonCode::TellFailed, None),
                target: Some(Target::Tell),
            },
            Signal::Error {
                reason: Reason::new(ReasonCode::SpeechFailed, None),
                target: None,
            },
        ];

        let (cues, heard) = Cues::recording_in(FeedbackMode::Visual);
        let chip = cues.drawn();
        for job in jobs.clone() {
            cues.emit(job);
        }
        assert!(
            heard.try_recv().is_err(),
            "a chip drawn in visual silences every job"
        );

        drop(chip);
        for job in jobs {
            cues.emit(job);
        }
        assert_eq!(
            heard.try_iter().count(),
            4,
            "with no chip, every job sounds again"
        );
    }

    #[test]
    fn a_cue_the_mode_silences_never_reaches_the_player() {
        let (cues, heard) = Cues::recording_in(FeedbackMode::Off);
        cues.send(Cue::Ready);
        assert!(heard.try_recv().is_err());
    }

    #[test]
    fn a_drawn_chip_silences_visual_until_it_drops() {
        let (cues, heard) = Cues::recording_in(FeedbackMode::Visual);
        let chip = cues.drawn();
        cues.send(Cue::Ready);
        assert!(heard.try_recv().is_err(), "a chip drawn silences visual");

        drop(chip);
        cues.send(Cue::Ready);
        assert!(
            matches!(heard.try_recv(), Ok(Cue::Ready)),
            "with no chip left, visual must sound again"
        );
    }

    #[test]
    fn a_queued_cue_the_mode_now_silences_does_not_play() {
        let (cues, queued) = Cues::recording_in(FeedbackMode::Both);
        cues.send(Cue::Ready);
        cues.set_mode(FeedbackMode::Off);
        let gate = cues.gate.clone();
        drop(cues);

        let mut played = Vec::new();
        serve_cues(queued, &gate, |cue| played.push(cue));
        assert!(played.is_empty(), "played {played:?}");
    }

    // The cue and the voice must come out of one device, which they can only do
    // by going through one output.
    #[test]
    fn a_cue_plays_through_the_daemon_output() {
        let (output, mut mixed) = Output::readable();
        let cues = start_cue_player(FeedbackMode::Both, Arc::new(output));
        cues.send(Cue::Ready);

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut loudest = 0.0f32;
        while std::time::Instant::now() < deadline && loudest == 0.0 {
            if let Some(sample) = mixed.next() {
                loudest = loudest.max(sample.abs());
            }
        }
        assert!(loudest > 0.0, "no cue reached the device");
    }

    // A cue is as long as a sentence is short, and a speaker can die inside it.
    // Then the cue moves to the device that is there, like everything else.
    #[test]
    fn a_cue_follows_a_device_that_dies_under_it() {
        let (output, opened) = Output::counting();
        let cues = start_cue_player(FeedbackMode::Both, Arc::new(output));
        cues.send(Cue::Ready);

        // Nothing takes the audio from a counting output, so the cue stalls
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        while std::time::Instant::now() < deadline
            && opened.load(std::sync::atomic::Ordering::Relaxed) < 2
        {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            opened.load(std::sync::atomic::Ordering::Relaxed) >= 2,
            "a cue that nothing plays must open the device again"
        );
    }

    // Cues off must not end the player, or turning them on would need a
    // restart to get a thread back.
    #[test]
    fn a_player_that_starts_off_still_takes_cues() {
        let cues = start_cue_player(FeedbackMode::Off, Arc::new(Output::silent()));
        assert!(
            cues.sender.send(Cue::Ready).is_ok(),
            "the player must still hold the receiver, or turning sound on would \
             need a restart to get a thread back"
        );
    }
}

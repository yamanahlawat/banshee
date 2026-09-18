use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rodio::source::{SineWave, Source};

use crate::text_to_speech::ActiveUtterance;
use crate::text_to_speech::output::{Chunk, Output};

/// How often a cue that is playing is given the chance to follow the device.
const CUE_POLL: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, Debug)]
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

/// The cue channel and the switch that decides whether a cue sounds. One value,
/// because a cue sent while cues are off must still reach a live player for the
/// moment they come back on.
#[derive(Clone)]
pub struct Cues {
    sender: mpsc::Sender<Cue>,
    enabled: Arc<AtomicBool>,
}

impl Cues {
    /// A cue nobody can hear is not an error, so this swallows a dead player.
    pub fn send(&self, cue: Cue) {
        let _ = self.sender.send(cue);
    }

    /// The player thread reads the flag itself, so this serves the tests that
    /// ask whether a write reached it.
    #[cfg(test)]
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    /// No player behind it, for tests that never sound a cue.
    #[cfg(test)]
    pub fn silent() -> Self {
        Cues {
            sender: mpsc::channel().0,
            enabled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// A live receiver, for a test that asks which cue a path sounds.
    #[cfg(test)]
    pub fn recording() -> (Self, mpsc::Receiver<Cue>) {
        let (sender, receiver) = mpsc::channel();
        (
            Cues {
                sender,
                enabled: Arc::new(AtomicBool::new(true)),
            },
            receiver,
        )
    }
}

fn next_playable(receiver: &mpsc::Receiver<Cue>, enabled: &AtomicBool) -> Option<Cue> {
    loop {
        let cue = receiver.recv().ok()?;
        if enabled.load(Ordering::Relaxed) {
            return Some(cue);
        }
    }
}

/// The player holds the receiver whether or not cues sound, so turning them on
/// reaches a thread that is already listening. It opens no output device until
/// the first cue it must play, so cues left off hold no audio hardware.
pub fn start_cue_player(enabled: bool, output: Arc<Output>) -> Cues {
    let (sender, receiver) = mpsc::channel::<Cue>();
    let cues = Cues {
        sender,
        enabled: Arc::new(AtomicBool::new(enabled)),
    };
    let enabled = cues.enabled.clone();

    thread::spawn(move || {
        let Some(mut cue) = next_playable(&receiver, &enabled) else {
            return;
        };
        // A cue that cannot play is not a reply that was not spoken, so its
        // faults stay out of `last_speech_error`, and the receiver goes rather
        // than buffering them for the life of the daemon. `play` logs them.
        let (faults, unread) = mpsc::channel();
        drop(unread);
        loop {
            play(&output, cue, &faults);
            cue = match next_playable(&receiver, &enabled) {
                Some(next) => next,
                None => return,
            };
        }
    });

    cues
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
        playing.keep_playing();
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

    // The cue and the voice must come out of one device, which they can only do
    // by going through one output.
    #[test]
    fn a_cue_plays_through_the_daemon_output() {
        let (output, mut mixed) = Output::readable();
        let cues = start_cue_player(true, Arc::new(output));
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
        let cues = start_cue_player(true, Arc::new(output));
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
        let cues = start_cue_player(false, Arc::new(Output::silent()));
        assert!(
            cues.sender.send(Cue::Ready).is_ok(),
            "the player must still hold the receiver, or turning cues on would \
             need a restart to get a thread back"
        );
    }

    #[test]
    fn nothing_is_played_while_cues_are_off() {
        let (sender, receiver) = mpsc::channel();
        sender.send(Cue::Ready).unwrap();
        sender.send(Cue::Error).unwrap();
        drop(sender);

        assert!(
            next_playable(&receiver, &AtomicBool::new(false)).is_none(),
            "a cue that arrives while cues are off must not reach the speaker"
        );
    }

    #[test]
    fn the_first_cue_after_cues_come_on_is_played() {
        let (sender, receiver) = mpsc::channel();
        sender.send(Cue::Ready).unwrap();
        sender.send(Cue::Error).unwrap();

        assert!(matches!(
            next_playable(&receiver, &AtomicBool::new(true)),
            Some(Cue::Ready)
        ));
    }
}

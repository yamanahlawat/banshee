use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rodio::source::{SineWave, Source};
use rodio::{DeviceSinkBuilder, Player};

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
    fn tones(self) -> &'static [(f32, u64)] {
        match self {
            Cue::RecordStart => &[(660.0, 70), (880.0, 90)],
            Cue::RecordStop => &[(880.0, 70), (660.0, 90)],
            Cue::Ready => &[(523.0, 90), (784.0, 140)],
            Cue::Error => &[(220.0, 120), (196.0, 160)],
            Cue::Arm => &[(523.0, 70), (1046.0, 120)],
            Cue::Disarm => &[(1046.0, 70), (523.0, 120)],
        }
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
pub fn start_cue_player(enabled: bool) -> Cues {
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
        // The device sink is !Send, so it must live on this thread
        let sink = match DeviceSinkBuilder::open_default_sink() {
            Ok(sink) => sink,
            Err(e) => {
                eprintln!("Audio cues disabled, no output device: {e}");
                return;
            }
        };
        // Player queues tones back to back; the mixer alone would overlap them
        let player = Player::connect_new(sink.mixer());

        loop {
            for &(frequency, ms) in cue.tones() {
                player.append(tone(frequency, ms));
            }
            cue = match next_playable(&receiver, &enabled) {
                Some(next) => next,
                None => return,
            };
        }
    });

    cues
}

fn tone(frequency: f32, ms: u64) -> impl Source + Send {
    let mut tone = SineWave::new(frequency).take_duration(Duration::from_millis(ms));
    // Fade the tail to avoid an audible click at the cut
    tone.set_filter_fadeout();
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

    // Cues off must not end the player, or turning them on would need a
    // restart to get a thread back.
    #[test]
    fn a_player_that_starts_off_still_takes_cues() {
        let cues = start_cue_player(false);
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

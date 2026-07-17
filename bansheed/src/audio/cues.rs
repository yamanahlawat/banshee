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

// When disabled, the receiver is dropped so sends become silent no-ops
pub fn start_cue_player(enabled: bool) -> mpsc::Sender<Cue> {
    let (sender, receiver) = mpsc::channel::<Cue>();
    if !enabled {
        return sender;
    }

    thread::spawn(move || {
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

        while let Ok(cue) = receiver.recv() {
            for &(frequency, ms) in cue.tones() {
                player.append(tone(frequency, ms));
            }
        }
    });

    sender
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

    #[test]
    fn disabled_player_swallows_sends() {
        let sender = start_cue_player(false);
        assert!(sender.send(Cue::Ready).is_err());
    }
}

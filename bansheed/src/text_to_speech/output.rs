use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use banshee_common::error::BansheeError;
use rodio::buffer::SamplesBuffer;
use rodio::mixer::Mixer;
use rodio::{DeviceSinkBuilder, Player};

use crate::text_to_speech::{ActiveUtterance, lock};

/// The rate the mixer runs at, and the rate Kokoro synthesises at.
pub const SAMPLE_RATE: std::num::NonZero<u32> = std::num::NonZero::new(24_000).unwrap();
pub const CHANNELS: std::num::NonZero<u16> = std::num::NonZero::new(1).unwrap();

/// rodio converts each chunk to the mixer's rate, so two chunks may differ.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub samples: Vec<f32>,
    pub rate: std::num::NonZero<u32>,
    pub channels: std::num::NonZero<u16>,
}

/// How long a live device may go without taking audio. Measured 2026-09-17
/// across two device changes in the middle of a reply: the longest gap a
/// working device left was 279 ms, and a dead one never took audio again.
const DEAD_OUTPUT: Duration = Duration::from_secs(1);

/// True when the device stopped taking the audio that is waiting for it. Audio
/// must be queued, because a player that is empty between sentences is waiting
/// for synthesis and no device is late.
fn output_died(queued: usize, since_last_pull: Duration) -> bool {
    queued > 0 && since_last_pull > DEAD_OUTPUT
}

/// The machine's default output, opened once. Every backend appends to this one
/// mixer, so two of them never fight for the device.
pub struct Output {
    mixer: Mixer,
    /// One step for every 20 ms of audio the device took. A device that stops
    /// taking audio is the only thing that stops this.
    pulls: Arc<AtomicU64>,
}

impl Output {
    pub fn open() -> Result<Self, BansheeError> {
        let sink = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| BansheeError::Other(format!("No audio output device: {e}")))?;
        let mixer = sink.mixer().clone();
        // The !Send sink only has to stay alive, never move; leaking it
        // keeps the output stream open for the daemon's lifetime
        std::mem::forget(sink);
        Ok(Self {
            mixer,
            pulls: Arc::new(AtomicU64::new(0)),
        })
    }

    /// A mixer nobody reads: the output device is gone, so no sample is ever
    /// pulled and a queued chunk stays queued for a test to count.
    #[cfg(test)]
    pub fn silent() -> Self {
        let (mixer, _never_read) = rodio::mixer::mixer(CHANNELS, SAMPLE_RATE);
        Self {
            mixer,
            pulls: Arc::new(AtomicU64::new(0)),
        }
    }

    /// An output whose mixer a test reads, which is what a device does. The
    /// samples a device would have been given can be counted, and a player on
    /// it drains, so an utterance on it can finish.
    #[cfg(test)]
    pub fn readable() -> (Self, rodio::mixer::MixerSource) {
        let (mixer, source) = rodio::mixer::mixer(CHANNELS, SAMPLE_RATE);
        (
            Self {
                mixer,
                pulls: Arc::new(AtomicU64::new(0)),
            },
            source,
        )
    }

    pub fn pulls(&self) -> u64 {
        self.pulls.load(Ordering::Relaxed)
    }

    /// One player per utterance: rodio's `append` sleeps until a stopped player
    /// drains, and a player whose device is gone never drains.
    pub fn play(&self, chunks: impl Iterator<Item = Chunk> + Send + 'static) -> PlayerUtterance {
        let player = Arc::new(Player::connect_new(&self.mixer));
        let cancelled = Arc::new(Mutex::new(false));
        let thread_player = Arc::clone(&player);
        let thread_cancelled = Arc::clone(&cancelled);
        let thread_pulls = Arc::clone(&self.pulls);
        #[cfg(test)]
        let heard: Arc<Mutex<Vec<Chunk>>> = Arc::default();
        #[cfg(test)]
        let thread_heard = Arc::clone(&heard);
        let worker = thread::spawn(move || {
            for chunk in chunks {
                if chunk.samples.is_empty() {
                    continue;
                }
                // Append under the lock so stop() can never race a chunk into a
                // stopped player, where append would sleep
                let guard = lock(&thread_cancelled);
                if *guard {
                    break;
                }
                #[cfg(test)]
                lock(&thread_heard).push(chunk.clone());
                let stamp = Arc::clone(&thread_pulls);
                thread_player.append(rodio::Source::periodic_access(
                    SamplesBuffer::new(chunk.channels, chunk.rate, chunk.samples),
                    Duration::from_millis(20),
                    move |_| {
                        stamp.fetch_add(1, Ordering::Relaxed);
                    },
                ));
            }
        });
        PlayerUtterance {
            cancelled,
            worker,
            player,
            #[cfg(test)]
            heard,
        }
    }
}

pub struct PlayerUtterance {
    cancelled: Arc<Mutex<bool>>,
    // is_finished after a panic too, unlike a hand-rolled done flag
    worker: thread::JoinHandle<()>,
    player: Arc<Player>,
    /// Every chunk the player took, for a test that reads what was heard. The
    /// device consumes the samples themselves, so nothing else can.
    #[cfg(test)]
    heard: Arc<Mutex<Vec<Chunk>>>,
}

impl PlayerUtterance {
    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.player.len()
    }

    #[cfg(test)]
    pub fn heard(&self) -> Vec<Chunk> {
        lock(&self.heard).clone()
    }

    #[cfg(test)]
    pub fn worker_finished(&self) -> bool {
        self.worker.is_finished()
    }
}

impl ActiveUtterance for PlayerUtterance {
    fn is_finished(&mut self) -> bool {
        // empty() only drops when the device pulls samples; a dead device keeps
        // an utterance unfinished until stop()
        self.worker.is_finished() && self.player.empty()
    }

    fn stop(&mut self) {
        let mut guard = lock(&self.cancelled);
        *guard = true;
        self.player.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::{CHANNELS, Chunk, Output, SAMPLE_RATE};
    use crate::text_to_speech::ActiveUtterance;
    use std::thread;

    use std::time::Duration;

    // The gaps between sentences are the player running dry while Kokoro
    // synthesises, and they are not a device fault.
    #[test]
    fn an_empty_player_is_never_a_dead_device() {
        assert!(!super::output_died(0, Duration::from_secs(10)));
    }

    #[test]
    fn audio_nobody_takes_for_longer_than_the_bound_is_a_dead_device() {
        assert!(super::output_died(
            1,
            super::DEAD_OUTPUT + Duration::from_millis(1)
        ));
    }

    #[test]
    fn a_device_that_took_audio_within_the_bound_is_alive() {
        assert!(!super::output_died(1, super::DEAD_OUTPUT));
        assert!(!super::output_died(3, Duration::from_millis(279)));
    }

    // A device proves it is alive by taking samples. Nothing else in the daemon
    // can see that, because the device consumes them.
    #[test]
    fn the_counter_rises_only_when_a_device_takes_the_audio() {
        let (output, mut mixed) = Output::readable();
        let utterance = output.play(one_second_of_silence());
        wait_until("the chunk is queued", || utterance.queued() == 1);
        assert_eq!(output.pulls(), 0, "nothing has been taken yet");

        // A tenth of a second of audio, which is five of the counter's steps
        for _ in 0..2_400 {
            mixed.next();
        }
        assert!(
            output.pulls() > 0,
            "the device took audio and the counter stood still"
        );
    }

    #[test]
    fn a_dead_device_never_moves_the_counter() {
        let output = Output::silent();
        let utterance = output.play(one_second_of_silence());
        wait_until("the chunk is queued", || utterance.queued() == 1);
        thread::sleep(Duration::from_millis(100));
        assert_eq!(output.pulls(), 0);
    }

    fn kokoros_chunk(samples: usize) -> Chunk {
        Chunk {
            samples: vec![0.0; samples],
            rate: SAMPLE_RATE,
            channels: CHANNELS,
        }
    }

    fn one_second_of_silence() -> impl Iterator<Item = Chunk> + Send + 'static {
        std::iter::once(kokoros_chunk(SAMPLE_RATE.get() as usize))
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !done() {
            assert!(
                std::time::Instant::now() < deadline,
                "{what} did not happen within 2s"
            );
            thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn a_stopped_utterance_on_a_dead_device_does_not_block_the_next_one() {
        let output = Output::silent();
        let mut first = output.play(one_second_of_silence());
        wait_until("the first sentence is queued", || first.queued() == 1);
        first.stop();

        let second = output.play(one_second_of_silence());
        wait_until("the second utterance's thread finishes", || {
            second.worker_finished()
        });
        assert_eq!(
            second.queued(),
            1,
            "the second sentence was queued on its own player"
        );
    }

    #[test]
    fn a_sentence_after_stop_is_never_appended() {
        let output = Output::silent();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let chunks = std::iter::once(kokoros_chunk(240)).chain(std::iter::once_with(move || {
            let _ = release_rx.recv();
            kokoros_chunk(240)
        }));
        let mut utterance = output.play(chunks);
        wait_until("the first chunk is queued", || utterance.queued() == 1);
        utterance.stop();
        release_tx.send(()).unwrap();
        wait_until("the thread ends", || utterance.worker_finished());
        assert_eq!(utterance.queued(), 1, "no chunk may follow a stop");
    }

    // Half a second at half the mixer's rate comes out as half a second.
    #[test]
    fn a_chunk_at_another_rate_reaches_the_mixer_as_the_same_length_of_audio() {
        let (output, mut mixed) = Output::readable();
        let utterance = output.play(std::iter::once(Chunk {
            samples: vec![0.5; 6_000],
            rate: std::num::NonZero::new(12_000).unwrap(),
            channels: CHANNELS,
        }));
        wait_until("the chunk is queued", || utterance.queued() == 1);

        let heard = (0..30_000)
            .filter_map(|_| mixed.next())
            .filter(|sample| *sample != 0.0)
            .count();
        assert!(
            (11_000..=13_000).contains(&heard),
            "6000 samples at 12 kHz came out as {heard} at 24 kHz"
        );
    }
}

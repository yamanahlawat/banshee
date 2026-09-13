use std::sync::{Arc, Mutex};
use std::thread;

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

/// The machine's default output, opened once. Every backend appends to this one
/// mixer, so two of them never fight for the device.
pub struct Output {
    mixer: Mixer,
}

impl Output {
    pub fn open() -> Result<Self, BansheeError> {
        let sink = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| BansheeError::Other(format!("No audio output device: {e}")))?;
        let mixer = sink.mixer().clone();
        // The !Send sink only has to stay alive, never move; leaking it
        // keeps the output stream open for the daemon's lifetime
        std::mem::forget(sink);
        Ok(Self { mixer })
    }

    /// A mixer nobody reads: the output device is gone, so no sample is ever
    /// pulled and a queued chunk stays queued for a test to count.
    #[cfg(test)]
    pub fn silent() -> Self {
        let (mixer, _never_read) = rodio::mixer::mixer(CHANNELS, SAMPLE_RATE);
        Self { mixer }
    }

    /// An output whose mixer a test reads, which is what a device does. The
    /// samples a device would have been given can be counted, and a player on
    /// it drains, so an utterance on it can finish.
    #[cfg(test)]
    pub fn readable() -> (Self, rodio::mixer::MixerSource) {
        let (mixer, source) = rodio::mixer::mixer(CHANNELS, SAMPLE_RATE);
        (Self { mixer }, source)
    }

    /// One player per utterance: rodio's `append` sleeps until a stopped player
    /// drains, and a player whose device is gone never drains.
    pub fn play(&self, chunks: impl Iterator<Item = Chunk> + Send + 'static) -> PlayerUtterance {
        let player = Arc::new(Player::connect_new(&self.mixer));
        let cancelled = Arc::new(Mutex::new(false));
        let thread_player = Arc::clone(&player);
        let thread_cancelled = Arc::clone(&cancelled);
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
                thread_player.append(SamplesBuffer::new(
                    chunk.channels,
                    chunk.rate,
                    chunk.samples,
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

    #[test]
    fn kokoro_synthesises_at_24_khz_in_mono() {
        assert_eq!(SAMPLE_RATE.get(), 24_000);
        assert_eq!(CHANNELS.get(), 1);
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

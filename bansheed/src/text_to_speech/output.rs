use std::collections::VecDeque;
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

/// Sentences queued ahead of the one playing. One ahead keeps speech smooth,
/// and the bound is what keeps a dead device from holding a whole reply.
const LOOK_AHEAD: usize = 2;

/// How often the worker asks the player for room.
const ROOM_POLL: Duration = Duration::from_millis(20);

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

/// The device the daemon plays through, and the way to let it go. The sink is
/// `!Send`, so a thread of its own holds it and leaves when the sender drops.
pub struct Device {
    mixer: Mixer,
    _closer: std::sync::mpsc::Sender<()>,
}

type Opener = Box<dyn Fn() -> Result<Device, BansheeError> + Send + Sync>;

/// The machine's default output. Every backend appends to the mixer this holds,
/// so two of them never fight for the device, and a device that dies is
/// replaced underneath them.
pub struct Output {
    device: Mutex<Device>,
    opener: Opener,
    /// One step for every 20 ms of audio the device took. A device that stops
    /// taking audio is the only thing that stops this.
    pulls: Arc<AtomicU64>,
}

impl Output {
    pub fn open() -> Result<Self, BansheeError> {
        Self::from_opener(Box::new(default_device))
    }

    fn from_opener(opener: Opener) -> Result<Self, BansheeError> {
        let device = opener()?;
        Ok(Self {
            device: Mutex::new(device),
            opener,
            pulls: Arc::new(AtomicU64::new(0)),
        })
    }

    fn mixer(&self) -> Mixer {
        lock(&self.device).mixer.clone()
    }

    /// Opens the machine's default output again and plays through that one.
    /// The old device's thread leaves with its sink, which releases it.
    pub fn reopen(&self) -> Result<(), BansheeError> {
        let device = (self.opener)()?;
        *lock(&self.device) = device;
        self.pulls.store(0, Ordering::Relaxed);
        Ok(())
    }

    /// A mixer nobody reads: the output device is gone, so no sample is ever
    /// pulled and a queued chunk stays queued for a test to count.
    #[cfg(test)]
    pub fn silent() -> Self {
        Self::from_opener(Box::new(Self::test_device_ok)).expect("a mixer needs no device")
    }

    #[cfg(test)]
    fn test_device_ok() -> Result<Device, BansheeError> {
        Ok(Self::test_device())
    }

    #[cfg(test)]
    pub fn test_device() -> Device {
        let (mixer, _never_read) = rodio::mixer::mixer(CHANNELS, SAMPLE_RATE);
        let (closer, _closed) = std::sync::mpsc::channel();
        Device {
            mixer,
            _closer: closer,
        }
    }

    /// An output whose mixer a test reads, which is what a device does. The
    /// samples a device would have been given can be counted, and a player on
    /// it drains, so an utterance on it can finish.
    #[cfg(test)]
    pub fn readable() -> (Self, rodio::mixer::MixerSource) {
        let (mixer, source) = rodio::mixer::mixer(CHANNELS, SAMPLE_RATE);
        let (closer, _closed) = std::sync::mpsc::channel();
        let output = Self {
            device: Mutex::new(Device {
                mixer,
                _closer: closer,
            }),
            opener: Box::new(Self::test_device_ok),
            pulls: Arc::new(AtomicU64::new(0)),
        };
        (output, source)
    }

    pub fn pulls(&self) -> u64 {
        self.pulls.load(Ordering::Relaxed)
    }

    /// One player per utterance: rodio's `append` sleeps until a stopped player
    /// drains, and a player whose device is gone never drains.
    pub fn play(&self, chunks: impl Iterator<Item = Chunk> + Send + 'static) -> PlayerUtterance {
        let player = Arc::new(Player::connect_new(&self.mixer()));
        let cancelled = Arc::new(Mutex::new(false));
        let thread_player = Arc::clone(&player);
        let thread_cancelled = Arc::clone(&cancelled);
        let thread_pulls = Arc::clone(&self.pulls);
        let held: Arc<Mutex<VecDeque<Chunk>>> = Arc::default();
        let thread_held = Arc::clone(&held);
        #[cfg(test)]
        let heard: Arc<Mutex<Vec<Chunk>>> = Arc::default();
        #[cfg(test)]
        let thread_heard = Arc::clone(&heard);
        let worker = thread::spawn(move || {
            for chunk in chunks {
                if chunk.samples.is_empty() {
                    continue;
                }
                // The device decides the pace. Running ahead of it would hand a
                // whole reply to a device that may be gone by the next sentence.
                loop {
                    {
                        let mut queue = lock(&thread_held);
                        while queue.len() > thread_player.len() {
                            queue.pop_front();
                        }
                        if queue.len() < LOOK_AHEAD {
                            break;
                        }
                    }
                    if *lock(&thread_cancelled) {
                        return;
                    }
                    thread::sleep(ROOM_POLL);
                }
                // Append under the lock so stop() can never race a chunk into a
                // stopped player, where append would sleep
                let guard = lock(&thread_cancelled);
                if *guard {
                    break;
                }
                lock(&thread_held).push_back(chunk.clone());
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
            held,
            #[cfg(test)]
            heard,
        }
    }
}

pub struct PlayerUtterance {
    cancelled: Arc<Mutex<bool>>,
    /// The sentences the player has been given and the device has not finished.
    /// A device that dies owes exactly these.
    held: Arc<Mutex<VecDeque<Chunk>>>,
    // is_finished after a panic too, unlike a hand-rolled done flag
    worker: thread::JoinHandle<()>,
    player: Arc<Player>,
    /// Every chunk the player took, for a test that reads what was heard. The
    /// device consumes the samples themselves, so nothing else can.
    #[cfg(test)]
    heard: Arc<Mutex<Vec<Chunk>>>,
}

impl PlayerUtterance {
    pub fn held(&self) -> Vec<Chunk> {
        let mut queue = lock(&self.held);
        while queue.len() > self.player.len() {
            queue.pop_front();
        }
        queue.iter().cloned().collect()
    }

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

/// Opens the machine's default output on a thread that keeps it alive. The
/// thread leaves when the `Device` it answers with is dropped, which closes the
/// stream and releases the device.
fn default_device() -> Result<Device, BansheeError> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (closer, closed) = std::sync::mpsc::channel::<()>();
    thread::spawn(move || {
        let sink = match DeviceSinkBuilder::open_default_sink() {
            Ok(sink) => sink,
            Err(e) => {
                let _ = ready_tx.send(Err(BansheeError::Other(format!(
                    "No audio output device: {e}"
                ))));
                return;
            }
        };
        if ready_tx.send(Ok(sink.mixer().clone())).is_err() {
            return;
        }
        // Parks holding the !Send sink. Dropping it here closes the stream, and
        // the only way out is the Device at the other end going away.
        let _ = closed.recv();
    });
    let mixer = ready_rx
        .recv()
        .map_err(|_| BansheeError::Other("No audio output device".to_string()))??;
    Ok(Device {
        mixer,
        _closer: closer,
    })
}

#[cfg(test)]
mod tests {
    use super::{CHANNELS, Chunk, Output, SAMPLE_RATE};
    use crate::text_to_speech::ActiveUtterance;
    use banshee_common::error::BansheeError;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
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

    // What a swap re-appends. A device that took nothing still owes the whole
    // look-ahead; one that finished a sentence owes the rest.
    #[test]
    fn a_device_that_took_nothing_holds_every_sentence_it_was_given() {
        let output = Output::silent();
        let utterance = output.play(std::iter::repeat_with(|| kokoros_chunk(240)).take(5));
        wait_until("the look-ahead fills", || {
            utterance.queued() == super::LOOK_AHEAD
        });
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            utterance.held().len(),
            super::LOOK_AHEAD,
            "a dead device may not be given the whole reply"
        );
    }

    // A swap re-appends what the player still owes, so the held set may never
    // outlive what the player holds, whatever the device has taken.
    #[test]
    fn a_sentence_the_device_finished_is_no_longer_held() {
        let (output, mut mixed) = Output::readable();
        let utterance =
            output.play(std::iter::repeat_with(|| kokoros_chunk(240)).take(super::LOOK_AHEAD));
        wait_until("both sentences are queued", || {
            utterance.queued() == super::LOOK_AHEAD
        });

        for taken in 1..=480 {
            mixed.next();
            assert_eq!(
                utterance.held().len(),
                utterance.queued(),
                "the held set and the player disagreed after {taken} samples"
            );
        }
        // The player reports a sentence done on a later poll than its last
        // sample, so keep taking audio until it says the queue is empty.
        for _ in 0..2_000 {
            if utterance.queued() == 0 {
                break;
            }
            mixed.next();
        }
        assert_eq!(utterance.queued(), 0, "the device took every sentence");
        assert!(utterance.held().is_empty());
    }

    // The swap has to give the next player a different mixer, or an utterance
    // would reconnect to the device that is gone.
    #[test]
    fn reopening_asks_for_the_default_device_again() {
        let opened = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&opened);
        let output = Output::from_opener(Box::new(move || {
            counted.fetch_add(1, Ordering::Relaxed);
            Ok(Output::test_device())
        }))
        .expect("the first open");
        assert_eq!(opened.load(Ordering::Relaxed), 1);

        output.reopen().expect("the second open");
        assert_eq!(opened.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn an_output_that_cannot_open_says_so() {
        let output = Output::from_opener(Box::new(|| {
            Err(BansheeError::Other("no device".to_string()))
        }));
        assert!(
            output.is_err(),
            "an output with no device carries the fault"
        );
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

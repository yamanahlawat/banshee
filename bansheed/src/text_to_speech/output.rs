use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use banshee_common::error::BansheeError;
use rodio::buffer::SamplesBuffer;
use rodio::mixer::Mixer;
use rodio::{DeviceSinkBuilder, Player};

use crate::text_to_speech::{ActiveUtterance, Fault, lock};

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

/// One sentence, wrapped so the device's own consumption is counted.
fn stamped(chunk: Chunk, pulls: Arc<AtomicU64>) -> impl rodio::Source + Send + 'static {
    rodio::Source::periodic_access(
        SamplesBuffer::new(chunk.channels, chunk.rate, chunk.samples),
        Duration::from_millis(20),
        move |_| {
            pulls.fetch_add(1, Ordering::Relaxed);
        },
    )
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
    /// `None` until something plays. A daemon with cues off and nothing to say
    /// holds no audio hardware at all.
    device: Mutex<Option<Device>>,
    opener: Opener,
    /// One step for every 20 ms of audio the device took. A device that stops
    /// taking audio is the only thing that stops this.
    pulls: Arc<AtomicU64>,
}

impl Output {
    pub fn lazy() -> Self {
        Self::from_opener(Box::new(default_device))
    }

    fn from_opener(opener: Opener) -> Self {
        Self {
            device: Mutex::new(None),
            opener,
            pulls: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The mixer of the device that is open, opening one if none is. Every
    /// sound the daemon makes goes through this, so the voice and the cues
    /// cannot end up on two different devices.
    fn mixer(&self) -> Result<Mixer, BansheeError> {
        let mut device = lock(&self.device);
        if device.is_none() {
            *device = Some((self.opener)()?);
        }
        Ok(device
            .as_ref()
            .expect("a device was just opened")
            .mixer
            .clone())
    }

    /// Opens the machine's default output again and plays through that one.
    /// The old device's thread leaves with its sink, which releases it.
    pub fn reopen(&self) -> Result<(), BansheeError> {
        let device = (self.opener)()?;
        *lock(&self.device) = Some(device);
        self.pulls.store(0, Ordering::Relaxed);
        Ok(())
    }

    /// A mixer nobody reads: the output device is gone, so no sample is ever
    /// pulled and a queued chunk stays queued for a test to count.
    #[cfg(test)]
    pub fn silent() -> Self {
        Self::from_opener(Box::new(Self::test_device_ok))
    }

    /// An output that counts the devices it opens, which is how a test sees a
    /// swap without a device to listen to.
    #[cfg(test)]
    pub fn counting() -> (Self, Arc<AtomicU64>) {
        let opened = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&opened);
        let output = Self::from_opener(Box::new(move || {
            counted.fetch_add(1, Ordering::Relaxed);
            Ok(Self::test_device())
        }));
        (output, opened)
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
            device: Mutex::new(Some(Device {
                mixer,
                _closer: closer,
            })),
            opener: Box::new(Self::test_device_ok),
            pulls: Arc::new(AtomicU64::new(0)),
        };
        (output, source)
    }

    pub fn pulls(&self) -> u64 {
        self.pulls.load(Ordering::Relaxed)
    }

    fn stamp(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.pulls)
    }

    /// An output that opens once and never again, for the case where a reply
    /// is playing and the machine has no device left to take it.
    #[cfg(test)]
    pub fn dead_after_first() -> Self {
        let first = std::sync::atomic::AtomicBool::new(true);
        Self::from_opener(Box::new(move || {
            if first.swap(false, Ordering::Relaxed) {
                Ok(Self::test_device())
            } else {
                Err(BansheeError::Other("no device".to_string()))
            }
        }))
    }

    /// One player per utterance: rodio's `append` sleeps until a stopped player
    /// drains, and a player whose device is gone never drains.
    pub fn play(
        self: &Arc<Self>,
        chunks: impl Iterator<Item = Chunk> + Send + 'static,
        faults: std::sync::mpsc::Sender<Fault>,
    ) -> Result<PlayerUtterance, BansheeError> {
        let player = Arc::new(Mutex::new(Arc::new(Player::connect_new(&self.mixer()?))));
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
                        let player = lock(&thread_player).clone();
                        while queue.len() > player.len() {
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
                // Held and the player move together, so a swap never re-appends
                // the sentence this thread is appending.
                let mut queue = lock(&thread_held);
                let player = lock(&thread_player).clone();
                queue.push_back(chunk.clone());
                #[cfg(test)]
                lock(&thread_heard).push(chunk.clone());
                player.append(stamped(chunk, Arc::clone(&thread_pulls)));
            }
        });
        Ok(PlayerUtterance {
            cancelled,
            worker,
            player,
            held,
            output: Arc::clone(self),
            faults,
            seen: (self.pulls(), std::time::Instant::now()),
            gave_up: false,
            earned: true,
            #[cfg(test)]
            heard,
        })
    }
}

pub struct PlayerUtterance {
    cancelled: Arc<Mutex<bool>>,
    /// The sentences the player has been given and the device has not finished.
    /// A device that dies owes exactly these.
    held: Arc<Mutex<VecDeque<Chunk>>>,
    // is_finished after a panic too, unlike a hand-rolled done flag
    worker: thread::JoinHandle<()>,
    player: Arc<Mutex<Arc<Player>>>,
    output: Arc<Output>,
    faults: std::sync::mpsc::Sender<Fault>,
    /// The counter, and when it was last seen to move. A counter that stands
    /// still while audio waits is a device that is gone.
    seen: (u64, std::time::Instant),
    /// Set when there is nowhere left to play. A stopped player on a device
    /// that is gone never empties, because emptying is the device's own doing.
    gave_up: bool,
    /// A swap is earned by audio that was taken since the last one. Without it
    /// a device nobody can play through is reopened for ever.
    earned: bool,
    /// Every chunk the player took, for a test that reads what was heard. The
    /// device consumes the samples themselves, so nothing else can.
    #[cfg(test)]
    heard: Arc<Mutex<Vec<Chunk>>>,
}

impl PlayerUtterance {
    /// The sentences a swap would re-append. The swap reads the same deque
    /// under the same lock; this is how a test sees it.
    #[cfg(test)]
    pub fn held(&self) -> Vec<Chunk> {
        let mut queue = lock(&self.held);
        let player = lock(&self.player).clone();
        while queue.len() > player.len() {
            queue.pop_front();
        }
        queue.iter().cloned().collect()
    }

    #[cfg(test)]
    pub fn queued(&self) -> usize {
        lock(&self.player).len()
    }

    /// Ends a reply nothing can play. `speaking` clears with it, so the next
    /// reply is not queued behind this one.
    fn give_up(&mut self, reason: String) {
        log::error!("{reason}");
        let _ = self.faults.send(Fault::Failed(reason));
        self.stop();
        self.gave_up = true;
    }

    /// Moves what is left of this reply to the device that is there now. The
    /// player and its queue belong to the dead device, so both are replaced.
    fn swap_device(&mut self) -> Result<(), BansheeError> {
        self.output.reopen()?;
        let fresh = Arc::new(Player::connect_new(&self.output.mixer()?));
        {
            let queue = lock(&self.held);
            for chunk in queue.iter() {
                fresh.append(stamped(chunk.clone(), self.output.stamp()));
            }
            let mut player = lock(&self.player);
            player.stop();
            *player = fresh;
        }
        self.seen = (self.output.pulls(), std::time::Instant::now());
        Ok(())
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
        // an utterance unfinished until keep_playing swaps it or gives up
        self.gave_up || (self.worker.is_finished() && lock(&self.player).empty())
    }

    fn stop(&mut self) {
        let mut guard = lock(&self.cancelled);
        *guard = true;
        lock(&self.player).stop();
    }

    fn keep_playing(&mut self) {
        let pulls = self.output.pulls();
        if pulls != self.seen.0 {
            self.seen = (pulls, std::time::Instant::now());
            self.earned = true;
            return;
        }
        let queued = lock(&self.player).len();
        if !output_died(queued, self.seen.1.elapsed()) {
            return;
        }
        if !self.earned {
            self.give_up("the speaker went away and the next one took nothing".to_string());
            return;
        }
        log::warn!("the speaker stopped taking audio; opening the default device again");
        self.earned = false;
        if let Err(error) = self.swap_device() {
            self.give_up(format!(
                "the speaker went away and no other could be opened: {error}"
            ));
        }
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
            // rodio prints its own line to stderr when a sink drops, which lands
            // in the daemon log without a clock or a level. A swap drops one on
            // purpose, and the log says so itself.
            Ok(mut sink) => {
                sink.log_on_drop(false);
                sink
            }
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
    use crate::text_to_speech::{ActiveUtterance, Fault};
    use banshee_common::error::BansheeError;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
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
        let output = Arc::new(output);
        let utterance = output
            .play(one_second_of_silence(), ignored_faults())
            .expect("a test device opens");
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
        let output = Arc::new(Output::silent());
        let utterance = output
            .play(one_second_of_silence(), ignored_faults())
            .expect("a test device opens");
        wait_until("the chunk is queued", || utterance.queued() == 1);
        thread::sleep(Duration::from_millis(100));
        assert_eq!(output.pulls(), 0);
    }

    // What a swap re-appends. A device that took nothing still owes the whole
    // look-ahead; one that finished a sentence owes the rest.
    #[test]
    fn a_device_that_took_nothing_holds_every_sentence_it_was_given() {
        let output = Arc::new(Output::silent());
        let utterance = output
            .play(
                std::iter::repeat_with(|| kokoros_chunk(240)).take(5),
                ignored_faults(),
            )
            .expect("a test device opens");
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
        let output = Arc::new(output);
        let utterance = output
            .play(
                std::iter::repeat_with(|| kokoros_chunk(240)).take(super::LOOK_AHEAD),
                ignored_faults(),
            )
            .expect("a test device opens");
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
    // Cues turned off and nothing speaking must leave the machine's audio
    // hardware alone, which is what the cue player gave up when it held a
    // device of its own.
    #[test]
    fn an_output_holds_no_device_until_something_plays() {
        let (output, opened) = Output::counting();
        let output = Arc::new(output);
        assert_eq!(opened.load(Ordering::Relaxed), 0, "nothing has played yet");

        let _utterance = output
            .play(one_second_of_silence(), ignored_faults())
            .expect("the device opens for the first sentence");
        assert_eq!(opened.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_machine_with_no_device_says_so_when_it_plays() {
        let output = Arc::new(Output::from_opener(Box::new(|| {
            Err(BansheeError::Other("no device".to_string()))
        })));
        let played = output.play(one_second_of_silence(), ignored_faults());
        assert!(played.is_err(), "nothing can play without a device");
    }

    #[test]
    fn reopening_asks_for_the_default_device_again() {
        let (output, opened) = Output::counting();
        output.reopen().expect("the first open");
        assert_eq!(opened.load(Ordering::Relaxed), 1);

        output.reopen().expect("the second open");
        assert_eq!(opened.load(Ordering::Relaxed), 2);
    }

    // The device is gone and the reply has to carry on somewhere. The sentences
    // it never took are the ones the new device starts with.
    #[test]
    fn a_dead_device_hands_its_sentences_to_the_new_one() {
        let output = Arc::new(Output::silent());
        let (faults, heard) = std::sync::mpsc::channel();
        let mut utterance = output
            .play(
                std::iter::repeat_with(|| kokoros_chunk(240)).take(5),
                faults,
            )
            .expect("a test device opens");
        wait_until("the look-ahead fills", || {
            utterance.queued() == super::LOOK_AHEAD
        });

        let owed = utterance.held().len();
        thread::sleep(super::DEAD_OUTPUT + Duration::from_millis(30));
        utterance.keep_playing();

        assert_eq!(
            utterance.queued(),
            owed,
            "the held sentences moved to the new player"
        );
        assert!(heard.try_recv().is_err(), "a swap that worked is no fault");
    }

    // Kokoro takes its time over a long sentence, and the player is empty until
    // it answers. Reading that quiet as a dead device would swap the speaker in
    // the middle of a reply that is only waiting for words.
    #[test]
    fn a_player_waiting_for_the_next_sentence_swaps_nothing() {
        let (output, opened) = Output::counting();
        let output = Arc::new(output);
        let mut utterance = output
            .play(std::iter::empty(), ignored_faults())
            .expect("a test device opens");
        wait_until("the worker ends with nothing to play", || {
            utterance.is_finished()
        });

        thread::sleep(super::DEAD_OUTPUT + Duration::from_millis(30));
        utterance.keep_playing();

        assert_eq!(
            opened.load(Ordering::Relaxed),
            1,
            "a quiet player is not a dead device"
        );
    }

    // A swap that changes nothing must not be tried again for ever. Measured
    // 2026-09-18: without this the daemon reopened the device about forty times
    // a minute, for hours, and stayed mute the whole time.
    #[test]
    fn a_second_device_that_takes_nothing_ends_the_reply() {
        let (output, opened) = Output::counting();
        let output = Arc::new(output);
        let (faults, heard) = std::sync::mpsc::channel();
        let mut utterance = output
            .play(
                std::iter::repeat_with(|| kokoros_chunk(240)).take(5),
                faults,
            )
            .expect("a test device opens");
        wait_until("the look-ahead fills", || {
            utterance.queued() == super::LOOK_AHEAD
        });

        thread::sleep(super::DEAD_OUTPUT + Duration::from_millis(30));
        utterance.keep_playing();
        assert_eq!(
            opened.load(Ordering::Relaxed),
            2,
            "the first swap is allowed"
        );

        thread::sleep(super::DEAD_OUTPUT + Duration::from_millis(30));
        utterance.keep_playing();
        assert_eq!(
            opened.load(Ordering::Relaxed),
            2,
            "a device that took nothing earns no second swap"
        );
        assert!(utterance.is_finished(), "the reply ends instead of looping");
        match heard.try_recv() {
            Ok(Fault::Failed(reason)) => assert!(reason.contains("speaker"), "{reason}"),
            other => panic!("a reply that was cut must be reported: {other:?}"),
        }
    }

    // Buds that go, come back and go again are two deaths, not one. A device
    // that took audio has earned the reply another move.
    #[test]
    fn a_device_that_played_earns_the_next_swap() {
        let (output, opened) = Output::counting();
        let output = Arc::new(output);
        let mut utterance = output
            .play(
                std::iter::repeat_with(|| kokoros_chunk(240)).take(5),
                ignored_faults(),
            )
            .expect("a test device opens");
        wait_until("the look-ahead fills", || {
            utterance.queued() == super::LOOK_AHEAD
        });

        thread::sleep(super::DEAD_OUTPUT + Duration::from_millis(30));
        utterance.keep_playing();
        assert_eq!(opened.load(Ordering::Relaxed), 2, "the first swap");

        // The new device takes audio, which is what the counter is for
        output.pulls.fetch_add(1, Ordering::Relaxed);
        utterance.keep_playing();

        thread::sleep(super::DEAD_OUTPUT + Duration::from_millis(30));
        utterance.keep_playing();
        assert_eq!(
            opened.load(Ordering::Relaxed),
            3,
            "a device that played earns another swap"
        );
    }

    #[test]
    fn an_utterance_with_nowhere_to_play_ends_and_says_why() {
        let output = Arc::new(Output::dead_after_first());
        let (faults, heard) = std::sync::mpsc::channel();
        let mut utterance = output
            .play(
                std::iter::repeat_with(|| kokoros_chunk(240)).take(5),
                faults,
            )
            .expect("a test device opens");
        wait_until("the look-ahead fills", || {
            utterance.queued() == super::LOOK_AHEAD
        });

        thread::sleep(super::DEAD_OUTPUT + Duration::from_millis(30));
        utterance.keep_playing();

        wait_until("the utterance concludes", || utterance.is_finished());
        match heard.try_recv() {
            Ok(Fault::Failed(reason)) => assert!(reason.contains("speaker"), "{reason}"),
            other => panic!("a reply that was cut must be reported: {other:?}"),
        }
    }

    /// A fault channel a test does not read. The daemon's own runs on a thread
    /// that drains it.
    fn ignored_faults() -> std::sync::mpsc::Sender<Fault> {
        std::sync::mpsc::channel().0
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
        let output = Arc::new(Output::silent());
        let mut first = output
            .play(one_second_of_silence(), ignored_faults())
            .expect("a test device opens");
        wait_until("the first sentence is queued", || first.queued() == 1);
        first.stop();

        let second = output
            .play(one_second_of_silence(), ignored_faults())
            .expect("a test device opens");
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
        let output = Arc::new(Output::silent());
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let chunks = std::iter::once(kokoros_chunk(240)).chain(std::iter::once_with(move || {
            let _ = release_rx.recv();
            kokoros_chunk(240)
        }));
        let mut utterance = output
            .play(chunks, ignored_faults())
            .expect("a test device opens");
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
        let output = Arc::new(output);
        let utterance = output
            .play(
                std::iter::once(Chunk {
                    samples: vec![0.5; 6_000],
                    rate: std::num::NonZero::new(12_000).unwrap(),
                    channels: CHANNELS,
                }),
                ignored_faults(),
            )
            .expect("a test device opens");
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

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

/// Audio queued ahead of the device. It must outlast the producer's worst gap
/// or the device runs dry, and it is the ceiling on what a dead device holds
/// and a swap replays.
///
/// Measured against a remote speaker: the reply arrives in pieces of 25 ms and
/// the longest wait for the next piece was 1.22 s.
const LOOK_AHEAD: Duration = Duration::from_secs(2);

/// The sentence playing and one behind it, however long they are. Kokoro hands
/// over a whole sentence at a time, and a spoken sentence outlasts the bound
/// above, which alone would leave the player dry at every sentence boundary.
const ALWAYS_QUEUED: usize = 2;

const ROOM_POLL: Duration = Duration::from_millis(20);

/// One step of the counter. `DEAD_OUTPUT` is read against it, so it is named
/// rather than written at the one call that passes it.
const PULL_STEP: Duration = Duration::from_millis(20);

/// How long a live device may go without taking audio. Measured at 279 ms
/// across a device change; a dead device never takes audio again.
const DEAD_OUTPUT: Duration = Duration::from_secs(1);

/// The bound, for a test in another module that has to outwait it.
#[cfg(test)]
pub fn dead_output() -> Duration {
    DEAD_OUTPUT
}

/// True when the device stopped taking the audio that is waiting for it. Audio
/// must be queued, because a player that is empty between sentences is waiting
/// for synthesis and no device is late.
fn output_died(queued: usize, since_last_pull: Duration) -> bool {
    queued > 0 && since_last_pull > DEAD_OUTPUT
}

/// The time a chunk plays for. Its samples are interleaved, so a stereo chunk
/// carries half the audio of a mono one of the same length.
///
/// rodio answers the same question, but only from a `SamplesBuffer`, and
/// building one copies every sample into a shared array.
fn playtime(chunk: &Chunk) -> Duration {
    let frames = chunk.samples.len() as f64 / f64::from(chunk.channels.get());
    Duration::from_secs_f64(frames / f64::from(chunk.rate.get()))
}

/// Drops the sentences the player has finished. The held set is what a swap
/// re-appends, so it may never outlive what the player still holds.
fn trim(held: &mut VecDeque<Chunk>, queued: usize) {
    while held.len() > queued {
        held.pop_front();
    }
}

/// One sentence, wrapped so the device's own consumption is counted.
fn stamped(chunk: Chunk, pulls: Arc<AtomicU64>) -> impl rodio::Source + Send + 'static {
    rodio::Source::periodic_access(
        SamplesBuffer::new(chunk.channels, chunk.rate, chunk.samples),
        PULL_STEP,
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
    /// Rises with every device this output opens. Two sounds playing at once
    /// hold players on the same device, and this is how the second one learns
    /// that the first has already replaced it.
    generation: AtomicU64,
}

impl Output {
    pub fn lazy() -> Self {
        Self::from_opener(Box::new(default_device))
    }

    fn from_opener(opener: Opener) -> Self {
        Self {
            device: Mutex::new(None),
            opener,
            generation: AtomicU64::new(0),
        }
    }

    /// The mixer of the device that is open, opening one if none is. Every
    /// sound the daemon makes goes through this, so the voice and the cues
    /// cannot end up on two different devices.
    fn mixer(&self) -> Result<(Mixer, u64), BansheeError> {
        self.take_device(None)
    }

    /// The device to play on now, opening the machine's default again when
    /// `seen` is still the current one. A caller holding an older generation
    /// takes the device that replaced it rather than opening a third.
    /// The old device's thread leaves with its sink, which releases it.
    fn device_after(&self, seen: u64) -> Result<(Mixer, u64), BansheeError> {
        self.take_device(Some(seen))
    }

    /// `replace_when` is the generation the caller found dead. `None` asks only
    /// for whatever is open.
    fn take_device(&self, replace_when: Option<u64>) -> Result<(Mixer, u64), BansheeError> {
        let mut device = lock(&self.device);
        let now = self.generation.load(Ordering::Relaxed);
        if device.is_none() || replace_when == Some(now) {
            *device = Some((self.opener)()?);
            self.generation.store(now + 1, Ordering::Relaxed);
        }
        Ok((
            device.as_ref().expect("a device is open").mixer.clone(),
            self.generation.load(Ordering::Relaxed),
        ))
    }

    /// Mixers nobody reads: the output device is gone, so no sample is ever
    /// pulled and a queued chunk stays queued for a test to count.
    #[cfg(test)]
    pub fn silent() -> Self {
        Self::counting().0
    }

    /// The same, counting the devices it opens, which is how a test sees a swap
    /// without a device to listen to.
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
            opener: Box::new(|| Ok(Self::test_device())),
            generation: AtomicU64::new(0),
        };
        (output, source)
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

    /// One player per utterance, and one thread that owns it. That thread feeds
    /// the sentences and then stays with the reply until the device has taken
    /// it, so no caller can forget to look after a reply on a device that dies.
    pub fn play(
        self: &Arc<Self>,
        chunks: impl Iterator<Item = Chunk> + Send + 'static,
        faults: std::sync::mpsc::Sender<Fault>,
    ) -> Result<PlayerUtterance, BansheeError> {
        let (mixer, device) = self.mixer()?;
        let player = Arc::new(Mutex::new(Arc::new(Player::connect_new(&mixer))));
        let cancelled = Arc::new(Mutex::new(false));
        let held: Arc<Mutex<VecDeque<Chunk>>> = Arc::default();
        let pulls = Arc::new(AtomicU64::new(0));
        let on = Arc::new(AtomicU64::new(device));
        #[cfg(test)]
        let heard: Arc<Mutex<Vec<Chunk>>> = Arc::default();

        let mut playing = Playing {
            player: Arc::clone(&player),
            held: Arc::clone(&held),
            cancelled: Arc::clone(&cancelled),
            output: Arc::clone(self),
            faults,
            pulls: Arc::clone(&pulls),
            on: Arc::clone(&on),
            seen_pulls: 0,
            owed_since: std::time::Instant::now(),
            earned: true,
            #[cfg(test)]
            heard: Arc::clone(&heard),
        };
        let worker = thread::spawn(move || playing.feed(chunks));
        Ok(PlayerUtterance {
            cancelled,
            worker,
            player,
            #[cfg(test)]
            held,
            #[cfg(test)]
            pulls,
            #[cfg(test)]
            on,
            #[cfg(test)]
            heard,
        })
    }
}

/// Everything the thread that owns a reply holds. The device watch lives here
/// rather than on the utterance, because a caller that has to remember to look
/// after a reply is a caller that will one day forget.
struct Playing {
    player: Arc<Mutex<Arc<Player>>>,
    /// The sentences the player has been given and the device has not finished.
    /// A device that dies owes exactly these.
    held: Arc<Mutex<VecDeque<Chunk>>>,
    cancelled: Arc<Mutex<bool>>,
    output: Arc<Output>,
    faults: std::sync::mpsc::Sender<Fault>,
    /// One step per `PULL_STEP` of this reply the device took. Its own, because
    /// audio another sound's device takes says nothing about this one's.
    pulls: Arc<AtomicU64>,
    /// The generation of the device this player is on.
    on: Arc<AtomicU64>,
    /// The counter as the last look found it. A counter that stands still while
    /// audio waits is a device that is gone.
    seen_pulls: u64,
    /// When the device was first owed the audio it has not taken. It starts at
    /// the sentence that ends a quiet, because a device owes nothing while the
    /// player is empty.
    owed_since: std::time::Instant,
    /// A swap is earned by audio that was taken since the last one. Without it
    /// a device nobody can play through is reopened for ever.
    earned: bool,
    #[cfg(test)]
    heard: Arc<Mutex<Vec<Chunk>>>,
}

impl Playing {
    fn feed(&mut self, chunks: impl Iterator<Item = Chunk>) {
        for chunk in chunks {
            if chunk.samples.is_empty() {
                continue;
            }
            // The device decides the pace. Running ahead of it would hand a
            // whole reply to a device that may be gone by the next sentence.
            while !self.has_room() {
                if self.stopped() || !self.follows_the_device() {
                    return;
                }
                thread::sleep(ROOM_POLL);
            }
            let ended_the_quiet = {
                // Append under the lock so stop() can never race a chunk into a
                // stopped player, where append would sleep
                let guard = lock(&self.cancelled);
                if *guard {
                    return;
                }
                // Held and the player move together, so a swap never re-appends
                // the sentence this thread is appending.
                let mut queue = lock(&self.held);
                let player = lock(&self.player).clone();
                let quiet = player.len() == 0;
                queue.push_back(chunk.clone());
                #[cfg(test)]
                lock(&self.heard).push(chunk.clone());
                player.append(stamped(chunk, Arc::clone(&self.pulls)));
                quiet
            };
            if ended_the_quiet {
                self.owed_since = std::time::Instant::now();
            }
        }

        // Every sentence is given. Stay with the reply until the device has
        // taken it: this thread is the only one that can move it to another.
        while !self.stopped() && self.queued() > 0 {
            if !self.follows_the_device() {
                return;
            }
            thread::sleep(ROOM_POLL);
        }
    }

    fn stopped(&self) -> bool {
        *lock(&self.cancelled)
    }

    fn queued(&self) -> usize {
        lock(&self.player).len()
    }

    fn has_room(&self) -> bool {
        let mut queue = lock(&self.held);
        trim(&mut queue, lock(&self.player).len());
        queue.len() < ALWAYS_QUEUED || queue.iter().map(playtime).sum::<Duration>() < LOOK_AHEAD
    }

    /// False when the reply is over because nothing can play it.
    fn follows_the_device(&mut self) -> bool {
        let pulls = self.pulls.load(Ordering::Relaxed);
        if pulls != self.seen_pulls {
            self.seen_pulls = pulls;
            self.owed_since = std::time::Instant::now();
            self.earned = true;
            return true;
        }
        if !output_died(self.queued(), self.owed_since.elapsed()) {
            return true;
        }
        if !self.earned {
            self.give_up("the speaker went away and the next one took nothing".to_string());
            return false;
        }
        log::warn!("the speaker stopped taking audio; opening the default device again");
        self.earned = false;
        match self.swap_device() {
            Ok(()) => true,
            Err(error) => {
                self.give_up(format!(
                    "the speaker went away and no other could be opened: {error}"
                ));
                false
            }
        }
    }

    /// Ends a reply nothing can play. The player stops with it, so the next
    /// reply is not queued behind this one.
    fn give_up(&mut self, reason: String) {
        log::error!("{reason}");
        let _ = self.faults.send(Fault::Failed(reason));
        *lock(&self.cancelled) = true;
        lock(&self.player).stop();
    }

    /// Moves what is left of this reply to the device that is there now. The
    /// player and its queue belong to the dead device, so both are replaced.
    fn swap_device(&mut self) -> Result<(), BansheeError> {
        let (mixer, device) = self.output.device_after(self.on.load(Ordering::Relaxed))?;
        let fresh = Arc::new(Player::connect_new(&mixer));
        {
            let queue = lock(&self.held);
            for chunk in queue.iter() {
                fresh.append(stamped(chunk.clone(), Arc::clone(&self.pulls)));
            }
            let mut player = lock(&self.player);
            player.stop();
            *player = fresh;
        }
        self.on.store(device, Ordering::Relaxed);
        self.seen_pulls = self.pulls.load(Ordering::Relaxed);
        self.owed_since = std::time::Instant::now();
        Ok(())
    }
}

pub struct PlayerUtterance {
    cancelled: Arc<Mutex<bool>>,
    // is_finished after a panic too, unlike a hand-rolled done flag
    worker: thread::JoinHandle<()>,
    player: Arc<Mutex<Arc<Player>>>,
    #[cfg(test)]
    held: Arc<Mutex<VecDeque<Chunk>>>,
    #[cfg(test)]
    pulls: Arc<AtomicU64>,
    #[cfg(test)]
    on: Arc<AtomicU64>,
    /// Every chunk the player took, for a test that reads what was heard. The
    /// device consumes the samples themselves, so nothing else can.
    #[cfg(test)]
    heard: Arc<Mutex<Vec<Chunk>>>,
}

impl PlayerUtterance {
    /// The sentences a swap would re-append.
    #[cfg(test)]
    pub fn held(&self) -> Vec<Chunk> {
        let mut queue = lock(&self.held);
        trim(&mut queue, lock(&self.player).len());
        queue.iter().cloned().collect()
    }

    /// Stands for the device taking this reply's audio, which a test has no
    /// device to do for it.
    #[cfg(test)]
    pub fn took_audio(&self) {
        self.pulls.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub fn pulls(&self) -> u64 {
        self.pulls.load(Ordering::Relaxed)
    }

    /// The generation of the device this reply is on. It changes only when the
    /// reply moves, which is what a test watches.
    #[cfg(test)]
    pub fn device(&self) -> u64 {
        self.on.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn queued(&self) -> usize {
        lock(&self.player).len()
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
    /// The thread ends when the device has taken the reply, when a stop cuts it
    /// short, or when nothing could play it. Nothing else ends it.
    fn is_finished(&mut self) -> bool {
        self.worker.is_finished()
    }

    fn stop(&mut self) {
        let mut guard = lock(&self.cancelled);
        *guard = true;
        lock(&self.player).stop();
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
        // After the mixer is handed over, so the first sentence never waits for
        // a line in the log. rodio opens another device when the default will
        // not open, and names neither, so the shape is all this may claim.
        let config = sink.config();
        log::info!(
            "Output opened at {} Hz, {} ch",
            config.sample_rate(),
            config.channel_count()
        );
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
        assert_eq!(utterance.pulls(), 0, "nothing has been taken yet");

        // A tenth of a second of audio, which is five of the counter's steps
        for _ in 0..2_400 {
            mixed.next();
        }
        assert!(
            utterance.pulls() > 0,
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
        assert_eq!(utterance.pulls(), 0);
    }

    // What a swap re-appends. A device that took nothing still owes the whole
    // look-ahead; one that finished a sentence owes the rest.
    #[test]
    fn a_device_that_took_nothing_holds_every_sentence_it_was_given() {
        let output = Arc::new(Output::silent());
        let utterance = output
            .play(std::iter::repeat_with(a_sentence).take(5), ignored_faults())
            .expect("a test device opens");
        wait_until("the look-ahead fills", || {
            utterance.queued() == super::ALWAYS_QUEUED
        });
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            utterance.held().len(),
            super::ALWAYS_QUEUED,
            "a dead device may not be given the whole reply"
        );
    }

    // A swap re-appends what the player still owes, so the held set may never
    // outlive what the player holds, whatever the device has taken.
    #[test]
    fn a_sentence_the_device_finished_is_no_longer_held() {
        let (output, mut mixed) = Output::readable();
        let output = Arc::new(output);
        let both = 2;
        let utterance = output
            .play(
                std::iter::repeat_with(|| kokoros_chunk(240)).take(both),
                ignored_faults(),
            )
            .expect("a test device opens");
        wait_until("both sentences are queued", || utterance.queued() == both);

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

    // The caller that finds the device dead opens another; a caller still
    // holding the one before it takes what replaced it instead of a third.
    #[test]
    fn a_device_is_opened_once_however_many_players_find_it_dead() {
        let (output, opened) = Output::counting();
        let (_mixer, first) = output.mixer().expect("the first open");
        assert_eq!(opened.load(Ordering::Relaxed), 1);

        let (_mixer, second) = output.device_after(first).expect("the second open");
        assert_eq!(opened.load(Ordering::Relaxed), 2);
        assert_ne!(second, first, "a new device is a new generation");

        let (_mixer, taken) = output
            .device_after(first)
            .expect("the one that replaced it");
        assert_eq!(
            opened.load(Ordering::Relaxed),
            2,
            "a player behind by a generation opens nothing"
        );
        assert_eq!(taken, second);
    }

    // The device is gone and the reply has to carry on somewhere. The sentences
    // it never took are the ones the new device starts with.
    #[test]
    fn a_dead_device_hands_its_sentences_to_the_new_one() {
        let output = Arc::new(Output::silent());
        let (faults, heard) = std::sync::mpsc::channel();
        let utterance = output
            .play(std::iter::repeat_with(a_sentence).take(5), faults)
            .expect("a test device opens");
        wait_until("the look-ahead fills", || {
            utterance.queued() == super::ALWAYS_QUEUED
        });

        let owed = utterance.held().len();
        let device = utterance.device();
        wait_for_the_swap("the reply moves by itself", || utterance.device() != device);

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

        assert_eq!(
            opened.load(Ordering::Relaxed),
            1,
            "a quiet player is not a dead device"
        );
    }

    // Kokoro hands over a whole sentence at a time, and a spoken sentence runs
    // longer than the bound. The next one must still stand behind the one
    // playing, or the player runs dry at every sentence boundary.
    #[test]
    fn a_sentence_longer_than_the_bound_keeps_the_next_one_behind_it() {
        let output = Arc::new(Output::silent());
        let four_seconds = SAMPLE_RATE.get() as usize * 4;
        let utterance = output
            .play(
                std::iter::repeat_with(move || kokoros_chunk(four_seconds)).take(5),
                ignored_faults(),
            )
            .expect("a test device opens");

        wait_until("the next sentence stands behind the one playing", || {
            utterance.queued() == super::ALWAYS_QUEUED
        });
    }

    // A remote reply arrives in pieces of 25 ms, where Kokoro's arrive as whole
    // sentences. The bound holds the same audio whatever the pieces are.
    #[test]
    fn a_reply_that_arrives_in_small_pieces_still_fills_the_look_ahead() {
        let output = Arc::new(Output::silent());
        let utterance = output
            .play(
                std::iter::repeat_with(|| kokoros_chunk(600)).take(400),
                ignored_faults(),
            )
            .expect("a test device opens");

        wait_until("the look-ahead fills", || {
            queued_audio(&utterance.held()) >= super::LOOK_AHEAD
        });
    }

    /// The audio a set of chunks carries, which is what the look-ahead bounds.
    fn queued_audio(chunks: &[Chunk]) -> Duration {
        chunks.iter().map(super::playtime).sum()
    }

    // A remote speaker answers in its own time, and the player is empty until
    // it does. The device is owed nothing in that quiet, so reading it as a
    // dead device reopens the speaker in the middle of the first word.
    #[test]
    fn a_reply_that_waited_for_its_first_sentence_swaps_nothing() {
        let (output, opened) = Output::counting();
        let output = Arc::new(output);
        let late = std::iter::once_with(|| {
            thread::sleep(super::DEAD_OUTPUT + Duration::from_millis(30));
            kokoros_chunk(240)
        });
        let utterance = output
            .play(late, ignored_faults())
            .expect("a test device opens");

        wait_until("the late sentence is queued", || utterance.queued() == 1);

        assert_eq!(
            opened.load(Ordering::Relaxed),
            1,
            "the wait for the first sentence was read as a dead device"
        );
    }

    // A swap that changes nothing must not be tried again for ever: a device
    // nobody can play through would be reopened until the daemon restarts.
    #[test]
    fn a_second_device_that_takes_nothing_ends_the_reply() {
        let (output, opened) = Output::counting();
        let output = Arc::new(output);
        let (faults, heard) = std::sync::mpsc::channel();
        let mut utterance = output
            .play(std::iter::repeat_with(a_sentence).take(5), faults)
            .expect("a test device opens");
        wait_until("the look-ahead fills", || {
            utterance.queued() == super::ALWAYS_QUEUED
        });

        wait_for_the_swap("the reply moves once", || {
            opened.load(Ordering::Relaxed) == 2
        });
        wait_for_the_swap("the reply ends instead of looping", || {
            utterance.is_finished()
        });
        assert_eq!(
            opened.load(Ordering::Relaxed),
            2,
            "a device that took nothing earns no second swap"
        );
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
        let utterance = output
            .play(std::iter::repeat_with(a_sentence).take(5), ignored_faults())
            .expect("a test device opens");
        wait_until("the look-ahead fills", || {
            utterance.queued() == super::ALWAYS_QUEUED
        });

        wait_for_the_swap("the first swap", || opened.load(Ordering::Relaxed) == 2);

        // The new device takes audio, which is what the counter is for
        utterance.took_audio();

        wait_for_the_swap("a device that played earns another swap", || {
            opened.load(Ordering::Relaxed) == 3
        });
    }

    // A cue and a reply play at once through one output. When one of them moves
    // to a new device, the audio the new device takes says nothing about the
    // other, which is still pointed at the device that died.
    #[test]
    fn one_utterance_moving_leaves_the_other_still_looking_at_a_dead_device() {
        let (output, opened) = Output::counting();
        let output = Arc::new(output);
        let cue = output
            .play(std::iter::repeat_with(a_sentence).take(5), ignored_faults())
            .expect("a test device opens");
        let reply = output
            .play(std::iter::repeat_with(a_sentence).take(5), ignored_faults())
            .expect("the same device serves both");
        wait_until("both fill their look-ahead", || {
            cue.queued() == super::ALWAYS_QUEUED && reply.queued() == super::ALWAYS_QUEUED
        });

        // Whichever moves first opens the one replacement
        wait_for_the_swap("a sound moves to the device that is there", || {
            opened.load(Ordering::Relaxed) == 2
        });

        // The new device takes the cue's audio, which says nothing about the reply
        cue.took_audio();
        assert_eq!(
            reply.pulls(),
            0,
            "the audio the cue's device took is not the reply's"
        );

        wait_for_the_swap("both sounds end up on it", || {
            cue.device() == reply.device() && cue.device() != 0
        });
        assert_eq!(
            opened.load(Ordering::Relaxed),
            2,
            "the device was already replaced, so nothing opens a third"
        );
    }

    #[test]
    fn an_utterance_with_nowhere_to_play_ends_and_says_why() {
        let output = Arc::new(Output::dead_after_first());
        let (faults, heard) = std::sync::mpsc::channel();
        let mut utterance = output
            .play(std::iter::repeat_with(a_sentence).take(5), faults)
            .expect("a test device opens");
        wait_until("the look-ahead fills", || {
            utterance.queued() == super::ALWAYS_QUEUED
        });

        wait_for_the_swap("the utterance concludes", || utterance.is_finished());
        match heard.try_recv() {
            Ok(Fault::Failed(reason)) => assert!(reason.contains("speaker"), "{reason}"),
            other => panic!("a reply that was cut must be reported: {other:?}"),
        }
    }

    /// A fault channel a test does not read.
    fn ignored_faults() -> std::sync::mpsc::Sender<Fault> {
        std::sync::mpsc::channel().0
    }

    /// A sentence that fills the look-ahead on its own, as Kokoro's do. The
    /// player then holds it and the one the floor lets in behind it, which is
    /// what the tests that watch a full player wait for.
    fn a_sentence() -> Chunk {
        kokoros_chunk(SAMPLE_RATE.get() as usize * super::LOOK_AHEAD.as_secs() as usize)
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

    /// Waits past the dead-device bound, which is what the reply's own thread
    /// has to outwait before it moves.
    fn wait_for_the_swap(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + super::DEAD_OUTPUT * 4;
        while !done() {
            assert!(
                std::time::Instant::now() < deadline,
                "{what} did not happen within four times the bound"
            );
            thread::sleep(Duration::from_millis(20));
        }
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
        wait_until("the second sentence is queued", || second.queued() == 1);
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

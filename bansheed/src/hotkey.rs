use ringbuf::traits::{Consumer, Observer, Split};
use ringbuf::wrap::Wrap;
use ringbuf::{HeapCons, HeapProd};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rdev::listen;

use crate::audio::cues::{Cues, Reason, ReasonCode, Signal, Target};
use crate::audio::utils::{StreamingResampler, resample_audio};
use crate::binding::{Hotkey, HotkeyAction, HotkeyTracker};
use crate::config::{HotkeyMode, Provider, STTConfig};
use crate::dictation::type_text;
use crate::speech_to_text::vad::{Detector, VAD_CHUNK};
use crate::speech_to_text::{Listener, Route, SAMPLE_RATE, Speech, Transcriber};
use crate::state::{
    AskCommand, ConsumerCommand, DaemonState, Listening, RecordingError, RecordingMode,
};
use crate::text_to_speech::lock;
use banshee_common::error::BansheeError;

// Lowers the flag however the job leaves, including an unwind: a panic would
// otherwise leave every client reading Working for good.
#[must_use = "dropping this at once raises the flag and lowers it again"]
struct Raised<'a> {
    state: &'a DaemonState,
    set: fn(&DaemonState, bool),
}

impl<'a> Raised<'a> {
    fn on(state: &'a DaemonState, set: fn(&DaemonState, bool)) -> Self {
        set(state, true);
        Self { state, set }
    }
}

impl Drop for Raised<'_> {
    fn drop(&mut self) {
        (self.set)(self.state, false);
    }
}

const CHUNK_MS: u64 = (VAD_CHUNK * 1000) as u64 / SAMPLE_RATE as u64;
/// Consecutive speech that confirms an onset, and the speech kept before it so
/// the model hears the first word start. Durations, because a change to the
/// detector's window must move the counts and not these.
const ONSET_MS: u64 = 384;
const PREROLL_MS: u64 = 256;
const ONSET_CHUNKS: usize = (ONSET_MS / CHUNK_MS) as usize;
const PREROLL_CHUNKS: usize = (PREROLL_MS / CHUNK_MS) as usize;
const ARMED_POLL: Duration = Duration::from_millis(30);
/// Unmeasured. Covers handing audio to the mixer before the first sample leaves.
const PLAYBACK_LATENCY_MS: u64 = 60;
/// Long enough for the arm cue to finish sounding.
const CUE_SETTLE: Duration =
    Duration::from_millis(crate::audio::cues::Cue::Arm.duration_ms() + PLAYBACK_LATENCY_MS);
// Ceiling on one answer past the onset timeout; nothing may hang the session
const MAX_ANSWER: Duration = Duration::from_secs(60);
// Past this ratio of wall time to audio length, the model is too heavy
const SLOW_TRANSCRIBE_FACTOR: f32 = 2.0;
/// 160 ms at 16 kHz, the chunk the stream window takes.
const FEED: usize = 2560;

/// The sentence a person reads. `Transcription` already says "Transcription
/// failed", so its reason stands alone.
fn reason(error: &banshee_common::error::BansheeError) -> String {
    match error {
        banshee_common::error::BansheeError::Transcription(reason) => reason.clone(),
        other => other.to_string(),
    }
}

/// A load on demand that failed, then what to do. The model stays on disk,
/// so the next use reads it again; a restart reports a model that never loads.
fn failed_load(what: &str, error: &banshee_common::error::BansheeError) -> String {
    format!(
        "{what} would not load: {}. The next use tries again. If it fails every time, restart: banshee start",
        reason(error).trim_end_matches('.')
    )
}

/// They change together on a rebind, so they travel together.
pub struct CaptureSource {
    pub consumer: HeapCons<f32>,
    pub sample_rate: u32,
}

impl CaptureSource {
    /// What capture reads before any device has opened. Its one slot is no
    /// ring a device could reuse, since a ring holds `RING_SECS` at its rate.
    pub fn unopened() -> Self {
        let (_, consumer) = ringbuf::HeapRb::new(1).split();
        Self {
            consumer,
            sample_rate: SAMPLE_RATE,
        }
    }

    fn pop_into(&mut self, batch: &mut Vec<f32>) {
        batch.extend(self.consumer.pop_iter());
    }

    /// Throws the ring away in one pass. A cancelled press discards seconds of
    /// audio at 48 kHz, and this runs on every armed poll while Banshee speaks.
    fn drop_all(&mut self) {
        self.consumer.clear();
    }
}

/// The capture the consumer thread reads, and the way the watchdog replaces it.
/// A rebind cannot travel as a command: an armed question holds that thread for
/// as long as the person takes to answer, and the ring it is reading dies with
/// the stream the watchdog just dropped.
pub struct Capture {
    source: Mutex<CaptureSource>,
    generation: AtomicU64,
    /// Why the last open for a use failed, until a device opens again.
    failure: Mutex<Option<String>>,
}

impl Capture {
    pub fn new(source: CaptureSource) -> Self {
        Self {
            source: Mutex::new(source),
            generation: AtomicU64::new(0),
            failure: Mutex::new(None),
        }
    }

    /// Plays the new device to everything that reads capture, including a
    /// question that is already listening.
    pub fn swap(&self, source: CaptureSource) {
        let mut held = lock(&self.source);
        *held = source;
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// A writer into the ring capture already holds, when that ring was built
    /// for `rate` and no stream writes into it. Audio a released dictation
    /// left unread then survives the next open.
    pub fn reuse(&self, rate: u32) -> Option<HeapProd<f32>> {
        let held = lock(&self.source);
        let fits = held.sample_rate == rate
            && held.consumer.capacity().get() == rate as usize * crate::audio::RING_SECS;
        (fits && !held.consumer.write_is_held())
            .then(|| HeapProd::new(Arc::clone(held.consumer.rb_ref())))
    }

    pub fn forget_failure(&self) {
        *lock(&self.failure) = None;
    }

    pub fn fail(&self, reason: String) {
        *lock(&self.failure) = Some(reason);
    }

    pub fn failure(&self) -> Option<String> {
        lock(&self.failure).clone()
    }

    /// Rises with every swap. A reader holding a resampler or a partial window
    /// built for the old device learns from this that both are stale.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn sample_rate(&self) -> u32 {
        lock(&self.source).sample_rate
    }

    /// The audio waiting, with the rate it was captured at and the device it
    /// came from, read under one lock. Two reads would pair one device's audio
    /// with another's rate whenever the watchdog rebinds between them.
    pub fn take(&self, batch: &mut Vec<f32>) -> (u32, u64) {
        let mut source = lock(&self.source);
        source.pop_into(batch);
        (source.sample_rate, self.generation.load(Ordering::Relaxed))
    }

    #[cfg(test)]
    pub fn drain(&self) -> Vec<f32> {
        let mut batch = Vec::new();
        self.take(&mut batch);
        batch
    }

    pub fn discard(&self) {
        lock(&self.source).drop_all();
    }
}

/// Builds the listener a config names, for a switch while the daemon runs.
pub type BuildListener = Box<dyn FnMut(&STTConfig) -> Result<Listener, RecordingError> + Send>;

// Everything the audio consumer thread owns. The listening model and the
// detector load and drop on this thread alone.
pub struct Pipeline {
    pub source: Arc<Capture>,
    pub speech_to_text: Box<dyn Transcriber>,
    pub vad: Detector,
    pub state: Arc<DaemonState>,
    pub cues: Cues,
    /// When the models last served, which their idle unload counts from.
    pub used: Instant,
    pub build_listener: BuildListener,
    /// The listener withdrawals this consumer has acted on.
    pub withdrawals_seen: u64,
}

/// Both routes send to the same server. The model may differ.
fn same_server(one: &Route, other: &Route) -> bool {
    matches!(
        (one, other),
        (Route::Remote { base_url: a, .. }, Route::Remote { base_url: b, .. }) if a == b
    )
}

/// In force while no engine is: one away on a request, or one that would not
/// build. Every use fails with why.
struct Unbuilt(String);

impl Transcriber for Unbuilt {
    fn transcribe(&mut self, _audio: &[f32]) -> Result<String, BansheeError> {
        Err(BansheeError::Transcription(self.0.clone()))
    }
    fn set_vocabulary(&mut self, _words: &[String]) {}
    fn set_speech(&mut self, _speech: Speech) {}
    fn reload(
        &mut self,
        _model: &'static banshee_common::listening::ListeningModel,
    ) -> Result<Option<&'static banshee_common::listening::ListeningModel>, BansheeError> {
        Err(BansheeError::Transcription(self.0.clone()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Waiting { speech_run: usize },
    InSpeech { silence_run: usize, start: usize },
    // The user is holding the hotkey; the release ends the answer
    Manual { start: usize },
}

#[derive(Debug)]
enum Heard {
    Audio(Vec<f32>),
    Silence,
    Closed,
}

/// One detector verdict moves the endpointing. `processed` is the sample the
/// chunk ended at, so an onset keeps `PREROLL_CHUNKS` of audio before the run
/// that confirmed it, and never reaches before the first sample. The end of
/// speech is the caller's to read off `silence_run`, against its own bound.
fn advance(phase: Phase, is_speech: bool, processed: usize) -> Phase {
    match phase {
        Phase::Waiting { speech_run } => {
            let speech_run = if is_speech { speech_run + 1 } else { 0 };
            if speech_run < ONSET_CHUNKS {
                return Phase::Waiting { speech_run };
            }
            Phase::InSpeech {
                silence_run: 0,
                start: processed.saturating_sub((ONSET_CHUNKS + PREROLL_CHUNKS) * VAD_CHUNK),
            }
        }
        Phase::InSpeech { silence_run, start } => Phase::InSpeech {
            silence_run: if is_speech { 0 } else { silence_run + 1 },
            start,
        },
        Phase::Manual { .. } => phase,
    }
}

/// The detector has just confirmed that the answer began.
fn onset(before: Phase, after: Phase) -> bool {
    matches!(
        (before, after),
        (Phase::Waiting { .. }, Phase::InSpeech { .. })
    )
}

/// What an answer's `timeout` leaves its listen once the wait for the arm cue
/// has taken `waited`. One timeout covers the whole answer.
fn listen_budget(timeout: Duration, waited: Duration) -> Duration {
    timeout.saturating_sub(waited)
}

/// The resampler for a device at this rate. One place, because a question that
/// outlives a device change builds a second one.
fn resampler_for(rate: u32) -> Result<StreamingResampler, String> {
    StreamingResampler::new(rate, SAMPLE_RATE).map_err(|e| {
        log::error!("Failed to create resampler: {e}");
        reason(&e)
    })
}

/// Drops what the VAD and the phase learned from audio that does not run on
/// into what comes next: the daemon's own speech, or another device's.
fn afresh(vad: &mut Detector, phase: &mut Phase) {
    vad.reset_state();
    match phase {
        Phase::Waiting { speech_run } => *speech_run = 0,
        Phase::InSpeech { silence_run, .. } => *silence_run = 0,
        Phase::Manual { .. } => {}
    }
}

pub fn hotkey_listener(
    pipeline: Pipeline,
    commands: mpsc::Receiver<ConsumerCommand>,
) -> thread::JoinHandle<()> {
    // The handle lets shutdown join it, dropping the listening model before atexit
    thread::spawn(move || consume(pipeline, &commands, &type_text))
}

/// The next command. A command already waiting is served first, so a use
/// that arrives as the models idle out finds them still loaded; otherwise the
/// wait ends when they are due to unload, and they unload here, between jobs.
fn next_command(
    pipeline: &mut Pipeline,
    commands: &mpsc::Receiver<ConsumerCommand>,
) -> Option<ConsumerCommand> {
    loop {
        match commands.try_recv() {
            Ok(command) => return Some(command),
            Err(mpsc::TryRecvError::Disconnected) => return None,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        let now = Instant::now();
        let Some(due) = pipeline.unload_idle(now) else {
            return commands.recv().ok();
        };
        match commands.recv_timeout(due.saturating_duration_since(now)) {
            Ok(command) => return Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
        }
    }
}

fn consume(
    mut pipeline: Pipeline,
    commands: &mpsc::Receiver<ConsumerCommand>,
    type_words: &dyn Fn(&str) -> Result<(), BansheeError>,
) {
    // Commands that arrived while a dictation streamed, in the order they came
    let mut deferred = VecDeque::new();
    while let Some(command) = deferred
        .pop_front()
        .or_else(|| next_command(&mut pipeline, commands))
    {
        if pipeline
            .serve(command, commands, &mut deferred, type_words)
            .is_break()
        {
            break;
        }
    }
}

/// Feeds the open stream the next whole 160 ms of audio past `fed`, and shows
/// a change to the committed words. `fed` is `None` while no stream is open,
/// and a feed that fails ends the stream and the live words. Answers whether
/// another whole window waits, so a caller can look at its commands between
/// feeds.
fn feed_stream(
    speech_to_text: &mut dyn Transcriber,
    state: &DaemonState,
    audio: &[f32],
    fed: &mut Option<usize>,
) -> bool {
    let Some(at) = fed else { return false };
    if audio.len() - *at < FEED {
        return false;
    }
    match speech_to_text.feed(&audio[*at..*at + FEED]) {
        Ok(words) => {
            if let Some(words) = words {
                state.publish_words(&words);
            }
            *at += FEED;
            audio.len() - *at >= FEED
        }
        Err(error) => {
            log::warn!("the live words stopped: {}", reason(&error));
            speech_to_text.cancel_stream();
            *fed = None;
            false
        }
    }
}

/// Capture read at 16 kHz. The resampler is built for the rate the first audio
/// comes at, and again after the device moves, since capture may open only
/// after the reading starts.
struct Resampled {
    device: u64,
    resampler: Option<StreamingResampler>,
    batch: Vec<f32>,
    /// Some audio came, so a new ring after it is another device mid-use and
    /// not capture opening for this one.
    heard: bool,
}

impl Resampled {
    fn new(source: &Capture) -> Self {
        Self {
            device: source.generation(),
            resampler: None,
            batch: Vec::new(),
            heard: false,
        }
    }

    /// Appends what capture holds to `audio`, and answers whether the device
    /// moved under audio already read.
    fn read(&mut self, source: &Capture, audio: &mut Vec<f32>) -> Result<bool, String> {
        self.batch.clear();
        let (rate, device) = source.take(&mut self.batch);
        let swapped = device != self.device;
        if swapped {
            self.device = device;
            self.resampler = None;
        }
        let moved = swapped && self.heard;
        if self.batch.is_empty() {
            return Ok(moved);
        }
        self.heard = true;
        let resampler = match self.resampler.take() {
            Some(built) => self.resampler.insert(built),
            None => self.resampler.insert(resampler_for(rate)?),
        };
        resampler.push(&self.batch, audio).map_err(|e| {
            log::error!("resampling failed: {e}");
            reason(&e)
        })?;
        Ok(moved)
    }

    /// Drops the partial window, so no spliced frame reaches the detector.
    fn reset(&mut self) {
        if let Some(resampler) = self.resampler.as_mut() {
            resampler.reset();
        }
    }

    /// The last partial window, so the full pass hears the audio to its end.
    fn finish(&mut self, audio: &mut Vec<f32>) -> Result<(), String> {
        let Some(resampler) = self.resampler.as_mut() else {
            return Ok(());
        };
        resampler.finish(audio).map_err(|e| {
            log::error!("resampling failed: {e}");
            reason(&e)
        })
    }
}

/// A held dictation, read off the ring while it is spoken.
struct Dictation {
    capture: Resampled,
    /// The whole clip at 16 kHz, which the voice check and the model read on release.
    audio: Vec<f32>,
    /// How far into `audio` the stream has heard, and `None` once a feed failed.
    fed: Option<usize>,
    /// Why the ring could not be read, said on release.
    broken: Option<String>,
}

impl Dictation {
    fn read(&mut self, source: &Capture) {
        if self.broken.is_some() {
            // Nothing hears this audio, and left in the ring it would open the next dictation
            source.discard();
            return;
        }
        match self.capture.read(source, &mut self.audio) {
            Ok(true) => log::info!("the microphone moved while a dictation was held"),
            Ok(false) => {}
            Err(why) => self.broken = Some(why),
        }
    }

    fn finish(&mut self) {
        if self.broken.is_none()
            && let Err(why) = self.capture.finish(&mut self.audio)
        {
            self.broken = Some(why);
        }
    }

    /// Answers whether another whole window waits.
    fn feed(&mut self, speech_to_text: &mut dyn Transcriber, state: &DaemonState) -> bool {
        self.broken.is_none() && feed_stream(speech_to_text, state, &self.audio, &mut self.fed)
    }
}

// Said by the daemon at startup and by the checklist, so the two cannot drift apart.
#[cfg(all(unix, not(target_os = "macos")))]
pub const WAYLAND_HOTKEY_HINT: &str = "the global hotkey needs X11. Bind \
     `banshee record start` on press and `banshee record stop` on release in \
     your compositor instead";

/// Whether the daemon binds the global hotkey itself in this session. False on
/// Wayland: no protocol grants a global grab there, so the compositor holds the
/// binding. A client reads this before it names a key, because naming one the
/// daemon never listens for is a promise it cannot keep.
pub fn listens() -> bool {
    #[cfg(all(unix, not(target_os = "macos")))]
    if crate::dictation::is_wayland() {
        return false;
    }
    true
}

pub fn usage_hint(hotkey: Hotkey, hotkey_mode: HotkeyMode) -> String {
    #[cfg(all(unix, not(target_os = "macos")))]
    if crate::dictation::is_wayland() {
        return format!("{WAYLAND_HOTKEY_HINT}.");
    }
    bound_key_hint(hotkey, hotkey_mode)
}

// Split from `usage_hint`, which answers from the live session. A test cannot
// choose the session it runs under, so it reads this half instead.
fn bound_key_hint(hotkey: Hotkey, hotkey_mode: HotkeyMode) -> String {
    let press = match hotkey_mode {
        HotkeyMode::Toggle => format!("Tap {hotkey} and speak, then tap it again to stop."),
        HotkeyMode::Hold => format!("Hold {hotkey} and speak, then release to stop."),
    };
    format!("{press} The text lands in whatever app has focus.")
}

// Registered by the daemon, not by the pipeline: a press must still reach
// record_start when recording is unavailable, or it answers with silence.
// rdev needs X11's XRecord, which wayland does not serve: listen either errors
// or attaches to Xwayland and never sees a key. Say so instead of looking broken.
pub fn start_global_hotkey(key_state: Arc<DaemonState>) {
    if !listens() {
        #[cfg(all(unix, not(target_os = "macos")))]
        log::info!("Wayland session: {WAYLAND_HOTKEY_HINT}.");
        return;
    }

    thread::spawn(move || {
        let (hotkey, mode, generation) = key_state.binding();
        let mut bound = Bound::new(hotkey, mode, generation);
        if let Err(error) = listen(move |event| bound.handle(&key_state, &event.event_type)) {
            // Names the capability that is gone, not just the error type
            log::error!(
                "Global hotkey listener stopped: {error:?}. `banshee record start` \
                 and `banshee record stop` still work."
            );
        }
    });
}

/// The listener's tracker, kept to the binding the daemon holds. A second
/// rdev listen is unsound, so a new binding swaps the key inside the one
/// callback, once no key is held.
pub(crate) struct Bound {
    tracker: HotkeyTracker,
    generation: u64,
    // Whether this listener's own start opened the session in flight.
    // Cancel discards audio, so it must never touch a session another
    // caller opened while this listener's start was refused as busy.
    owned: bool,
}

impl Bound {
    pub(crate) fn new(hotkey: Hotkey, mode: HotkeyMode, generation: u64) -> Bound {
        Bound {
            tracker: HotkeyTracker::new(hotkey, mode),
            generation,
            owned: false,
        }
    }

    pub(crate) fn handle(&mut self, state: &DaemonState, event: &rdev::EventType) {
        // The daemon's own paste presses modifier keys on this same
        // event stream; they must not re-enter the tracker
        if state.is_typing() {
            return;
        }
        let generation = state.binding_generation();
        self.follow(state, generation);
        if let Some(action) = self.tracker.on_event(event) {
            act(state, action, &mut self.owned);
        }
        // The release that ends a hold takes a waiting binding at once
        self.follow(state, generation);
    }

    fn follow(&mut self, state: &DaemonState, generation: u64) {
        if generation == self.generation {
            return;
        }
        let (hotkey, mode, generation) = state.binding();
        if self.tracker.rebind(hotkey, mode) {
            self.generation = generation;
        }
    }
}

/// What one hotkey action does to the daemon. `owned` says whether this
/// listener's own start opened the session in flight.
pub(crate) fn act(state: &DaemonState, action: HotkeyAction, owned: &mut bool) {
    match action {
        HotkeyAction::Start => *owned = state.record_start(),
        HotkeyAction::Toggle => *owned = state.record_toggle(),
        HotkeyAction::Stop => state.record_stop(),
        HotkeyAction::Cancel if *owned => state.record_cancel(),
        HotkeyAction::Cancel => {}
    }
}

impl Pipeline {
    /// Runs one command. Breaks on the shutdown.
    fn serve(
        &mut self,
        command: ConsumerCommand,
        commands: &mpsc::Receiver<ConsumerCommand>,
        deferred: &mut VecDeque<ConsumerCommand>,
        type_words: &dyn Fn(&str) -> Result<(), BansheeError>,
    ) -> std::ops::ControlFlow<()> {
        let records = matches!(
            command,
            ConsumerCommand::Stream(_)
                | ConsumerCommand::Transcribe(_)
                | ConsumerCommand::Discard
                | ConsumerCommand::Ask(_)
        );
        let uses_a_model = records || matches!(command, ConsumerCommand::Reload(_));
        match command {
            ConsumerCommand::Stream(press) => self.dictate(press, commands, deferred, type_words),
            ConsumerCommand::Transcribe(ring) => self.transcribe_utterance(ring, type_words),
            ConsumerCommand::Discard => self.discard(),
            ConsumerCommand::Ask(ask) => self.ask(ask),
            ConsumerCommand::Retune(words) => self.speech_to_text.set_vocabulary(&words),
            ConsumerCommand::Speak(speech) => self.speech_to_text.set_speech(speech),
            // The load takes seconds and holds this thread. Nothing is lost:
            // a press queues behind it and the ring still holds the audio.
            ConsumerCommand::Reload(model) => {
                let _loading = Raised::on(&self.state, DaemonState::set_loading_model);
                match self.speech_to_text.reload(model) {
                    Ok(loaded) => self.state.listening_model_loaded(loaded),
                    Err(error) => {
                        log::error!("the listening model did not load: {error}");
                        self.state.listening_model_failed(model, error.to_string());
                    }
                }
            }
            ConsumerCommand::Replace(stt) => self.replace(&stt),
            ConsumerCommand::UnloadAfter => {}
            ConsumerCommand::Shutdown => return std::ops::ControlFlow::Break(()),
        }
        // A recording's live words end with it, however it ended
        if records {
            self.state.publish_words("");
        }
        if uses_a_model {
            self.used = Instant::now();
        }
        std::ops::ControlFlow::Continue(())
    }

    /// Puts the listener `stt` names in force, unless its route already is.
    /// One that will not build leaves the listener in force, and says why.
    /// A server config.toml does not name is not installed: a later write
    /// already chose otherwise. A listener that would not build is built again.
    fn replace(&mut self, stt: &STTConfig) {
        let route = Route::of(stt);
        if route.is_remote() && !same_server(&route, &Route::of(&self.state.config().stt)) {
            return;
        }
        let unbuilt = matches!(self.state.listening(), Listening::Failed(..));
        if (route != self.state.listener_route() || unbuilt)
            && let Err(error) = self.install(stt)
        {
            log::error!("the listener did not switch: {error}");
            self.state.set_last_error(Some(error.to_string()));
        }
    }

    /// Builds the listener `stt` names and puts it in force. Answers what it
    /// has of its model.
    fn install(&mut self, stt: &STTConfig) -> Result<Listening, RecordingError> {
        let listener = (self.build_listener)(stt)?;
        let listening = match stt.provider {
            Provider::Remote => listener
                .refused
                .map_or(Listening::Remote, Listening::Refused),
            Provider::Local => {
                let model = crate::speech_to_text::listening_model_of(stt)?;
                match listener.awaiting {
                    Some(_) => Listening::Missing(model),
                    None => Listening::Available(model),
                }
            }
        };
        self.speech_to_text = listener.engine;
        self.state
            .install_listener(Route::of(stt), listening.clone());
        Ok(listening)
    }

    /// One transcription by the listener in force. Audio for a server goes
    /// through `through_server`, so a withdrawal can stop it.
    fn transcribe(&mut self, audio: &[f32]) -> Result<String, BansheeError> {
        if self.speech_to_text.sends_out() {
            self.through_server(audio)
        } else {
            self.speech_to_text.transcribe(audio)
        }
    }

    /// The request may not go: a withdrawal landed, or config.toml does not
    /// name the server in force.
    fn withdrawn(&self) -> bool {
        self.state.listener_withdrawn_since(self.withdrawals_seen)
            || !same_server(
                &self.state.listener_route(),
                &Route::of(&self.state.config().stt),
            )
    }

    /// The request on its way has to stop: a withdrawal landed after the
    /// count `begun`, even one a later switch settled, or config.toml names
    /// no server. One that names another server lets it finish.
    fn stops(&self, begun: u64) -> bool {
        self.state.listener_withdrawals() > begun || !self.state.config().stt.provider.is_remote()
    }

    /// Sends `audio` on a thread that owns the engine for the request, and
    /// stops waiting for it once it `stops`.
    pub fn through_server(&mut self, audio: &[f32]) -> Result<String, BansheeError> {
        // Read before the check, so a withdrawal between the two still stops it
        let begun = self.state.listener_withdrawals();
        if self.withdrawn() {
            return self.after_withdrawal(audio);
        }
        let away = Box::new(Unbuilt("the listener is on a request".to_string()));
        let mut engine = std::mem::replace(&mut self.speech_to_text, away);
        let (done, answered) = mpsc::channel();
        let sent = audio.to_vec();
        let stop = Arc::new(tokio::sync::Notify::new());
        let stopping = Arc::clone(&stop);
        let request = thread::spawn(move || {
            let answer = engine.transcribe_until(&sent, &stopping);
            let _ = done.send((engine, answer));
        });
        loop {
            match answered.recv_timeout(ARMED_POLL) {
                Ok((engine, answer)) => {
                    self.speech_to_text = engine;
                    return answer;
                }
                // The request keeps the old engine, and nothing reads its answer
                Err(mpsc::RecvTimeoutError::Timeout) if self.stops(begun) => {
                    stop.notify_one();
                    return self.after_withdrawal(audio);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => match request.join() {
                    Err(panic) => std::panic::resume_unwind(panic),
                    Ok(()) => unreachable!("the request answers before it ends"),
                },
            }
        }
    }

    /// Acts on a stop, for `audio` its server is not to get. A turn-off hands
    /// over to the listener it names. Otherwise the listener config.toml names
    /// takes over. After a removed key, its own server gets nothing until a key
    /// is on file again, so this audio is transcribed here. Another server gets
    /// it when no withdrawal stopped it, since config.toml names that server.
    /// After a withdrawal it is transcribed here once.
    fn after_withdrawal(&mut self, audio: &[f32]) -> Result<String, BansheeError> {
        let withdrawn = self.state.listener_withdrawn_since(self.withdrawals_seen);
        self.withdrawals_seen = self.state.listener_withdrawals();
        let meant_for = self.state.listener_route();
        let next = self
            .state
            .take_withdrawn_listener()
            .unwrap_or_else(|| self.state.config().stt.clone());
        let listening = match self.install(&next) {
            Ok(listening) => listening,
            Err(error) => return Err(self.unbuilt(&next, error)),
        };
        let same = same_server(&meant_for, &Route::of(&next));
        match listening {
            Listening::Missing(model) => Err(BansheeError::Transcription(
                RecordingError::NoListeningModel(model.id).to_string(),
            )),
            Listening::Remote if !same && withdrawn => self.once_here(&next, audio, None),
            Listening::Refused(_) if !same => self.once_here(&next, audio, None),
            Listening::Refused(_) => {
                self.once_here(&next, audio, Some("no key for the remote listener"))
            }
            _ => {
                self.ready().map_err(BansheeError::Transcription)?;
                self.transcribe(audio)
            }
        }
    }

    /// Transcribes `audio` on this machine once. The engine drops after this
    /// use. `cause` is why the server did not get it, said beside a model that
    /// is not downloaded.
    fn once_here(
        &mut self,
        named: &STTConfig,
        audio: &[f32],
        cause: Option<&str>,
    ) -> Result<String, BansheeError> {
        let here = STTConfig {
            provider: Provider::Local,
            ..named.clone()
        };
        let mut once = (self.build_listener)(&here)
            .map_err(|error| BansheeError::Transcription(error.to_string()))?;
        if let Some(model) = once.awaiting {
            return Err(BansheeError::Transcription(match cause {
                Some(cause) => format!(
                    "{cause}, and the listening model {} is not downloaded",
                    model.id
                ),
                None => RecordingError::NoListeningModel(model.id).to_string(),
            }));
        }
        once.engine.transcribe(audio)
    }

    /// The listener `stt` names would not build, so no engine is in force and
    /// nothing is sent until a switch builds one.
    fn unbuilt(&mut self, stt: &STTConfig, error: RecordingError) -> BansheeError {
        let why = error.to_string();
        log::error!("the listener did not switch: {why}");
        self.speech_to_text = Box::new(Unbuilt(why.clone()));
        let listening = match stt.listening_model() {
            Some(model) => Listening::Failed(model, why.clone()),
            None => Listening::Refused(error),
        };
        self.state.install_listener(Route::Local, listening);
        BansheeError::Transcription(why)
    }

    /// Frees the listening model and the detector once they have idled past
    /// `models.unload_after`, and answers when they are due. `None` once they
    /// are freed, or when they stay loaded: only a use loads them again.
    fn unload_idle(&mut self, now: Instant) -> Option<Instant> {
        let due = crate::models::unload_due(self.used, self.state.unload_after())?;
        if now < due {
            return Some(due);
        }
        let listener = self.speech_to_text.unload();
        if listener {
            self.state.listening_model_unloaded();
        }
        let detector = self.vad.unload();
        if detector || listener {
            crate::models::return_freed_memory();
            let freed = match (listener, detector) {
                (true, true) => "listening model and voice detector",
                (true, false) => "listening model",
                _ => "voice detector",
            };
            log::info!("The {freed} unloaded after idling");
        }
        None
    }

    /// Loads what idled out before a use needs it: the detector, then the
    /// listening model, which reads as loading until it is in memory. A load
    /// that fails fails this use alone, and the next use tries again.
    fn ready(&mut self) -> Result<(), String> {
        if self.vad.resident() && self.speech_to_text.resident() {
            return Ok(());
        }
        let _loading = Raised::on(&self.state, DaemonState::set_loading_model);
        self.vad
            .load()
            .map_err(|error| failed_load("the voice detection model", &error))?;
        if self.speech_to_text.resident() {
            return Ok(());
        }
        let ours = self.state.listening_model_loading();
        let loaded = self.speech_to_text.load();
        if ours {
            self.state.listening_model_read(loaded.is_ok());
        }
        loaded.map_err(|error| {
            log::error!("the listening model did not load: {error}");
            failed_load("the listening model", &error)
        })
    }

    // A session opened while this command sat in the queue owns the ring now:
    // the discard skips, the cancelled lead-in stays.
    fn discard(&self) {
        if !self.state.is_recording() {
            self.source.discard();
        }
    }

    // Push-to-talk ended: the ring holds the whole utterance
    fn transcribe_utterance(
        &mut self,
        ring: crate::state::RingClaim,
        type_words: impl FnOnce(&str) -> Result<(), BansheeError>,
    ) {
        let state = Arc::clone(&self.state);
        let _transcribing = Raised::on(&state, DaemonState::set_transcribing);
        let mut audio_data = Vec::new();
        let (rate, _) = self.source.take(&mut audio_data);
        drop(ring);

        log::debug!("Downsampling audio from {rate} Hz to {SAMPLE_RATE} Hz...");

        let final_data = match resample_audio(&audio_data, rate, SAMPLE_RATE) {
            Ok(data) => data,
            Err(e) => {
                log::error!("resampling failed: {e}");
                self.dictation_failed(reason(&e));
                return;
            }
        };
        match self.ready() {
            Ok(()) => self.deliver(&final_data, type_words),
            Err(why) => self.dictation_failed(why),
        }
    }

    /// The audio of a dictation already refused, dropped with its claim.
    fn drop_refused(&mut self, ring: crate::state::RingClaim) {
        self.source.discard();
        drop(ring);
    }

    fn dictation_failed(&self, why: String) {
        self.state.set_last_error(Some(why));
        self.cues.emit(Signal::Error {
            reason: Reason::new(ReasonCode::TranscriptionFailed, None),
            target: Some(Target::Dictate),
        });
    }

    /// The live words have a viewer and the model streams them.
    fn begin_live_words(&mut self, what: &str) -> bool {
        if !self.state.is_words_watched() {
            return false;
        }
        match self.speech_to_text.begin_stream() {
            Ok(streams) => streams,
            Err(error) => {
                log::warn!(
                    "the {what} is transcribed whole: the stream did not start: {}",
                    reason(&error)
                );
                false
            }
        }
    }

    /// A push-to-talk began. Ends on the release or the cancel, and any other
    /// command waits in `deferred`. While the model streams, it is fed as the
    /// key is held; with no stream, the ring keeps the audio for the release.
    fn dictate(
        &mut self,
        press: u64,
        commands: &mpsc::Receiver<ConsumerCommand>,
        deferred: &mut VecDeque<ConsumerCommand>,
        type_words: impl FnOnce(&str) -> Result<(), BansheeError>,
    ) {
        // At the press, so a model that idled out loads while the words are spoken
        let refused = match self.ready() {
            Ok(()) => false,
            Err(why) => {
                if self.state.refuse_dictation(press, why) {
                    self.discard();
                    return;
                }
                true
            }
        };
        let mut live = (!refused && self.begin_live_words("dictation")).then(|| Dictation {
            capture: Resampled::new(&self.source),
            audio: Vec::new(),
            fed: Some(0),
            broken: None,
        });
        loop {
            let command = match live {
                Some(_) => commands.try_recv(),
                None => commands
                    .recv()
                    .map_err(|_| mpsc::TryRecvError::Disconnected),
            };
            match command {
                Ok(ConsumerCommand::Transcribe(ring)) => {
                    match (live, refused) {
                        (_, true) => self.drop_refused(ring),
                        (Some(mut dictation), false) => {
                            dictation.read(&self.source);
                            drop(ring);
                            dictation.finish();
                            self.finish_dictation(dictation, type_words);
                        }
                        (None, false) => self.transcribe_utterance(ring, type_words),
                    }
                    return;
                }
                Ok(ConsumerCommand::Discard) => {
                    self.speech_to_text.cancel_stream();
                    self.discard();
                    return;
                }
                Ok(ConsumerCommand::Shutdown) | Err(mpsc::TryRecvError::Disconnected) => {
                    self.speech_to_text.cancel_stream();
                    deferred.push_back(ConsumerCommand::Shutdown);
                    return;
                }
                Ok(other) => deferred.push_back(other),
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if let Some(dictation) = live.as_mut() {
                dictation.read(&self.source);
                if !dictation.feed(&mut *self.speech_to_text, &self.state) {
                    thread::sleep(ARMED_POLL);
                }
            }
        }
    }

    fn finish_dictation(
        &mut self,
        mut dictation: Dictation,
        type_words: impl FnOnce(&str) -> Result<(), BansheeError>,
    ) {
        let state = Arc::clone(&self.state);
        let _transcribing = Raised::on(&state, DaemonState::set_transcribing);
        // The live words were the stream's; the typed text is one full pass
        self.speech_to_text.cancel_stream();
        if let Some(why) = dictation.broken.take() {
            self.dictation_failed(why);
        } else {
            self.deliver(&dictation.audio, type_words);
        }
    }

    /// Checks the whole clip for speech, transcribes it and types the text.
    fn deliver(
        &mut self,
        final_data: &[f32],
        type_words: impl FnOnce(&str) -> Result<(), BansheeError>,
    ) {
        let device = self.state.audio_device();
        if log::log_enabled!(log::Level::Debug) {
            let (min_amplitude, max_amplitude) = final_data
                .iter()
                .fold((0.0f32, 0.0f32), |(min, max), &sample| {
                    (min.min(sample), max.max(sample))
                });
            log::debug!("Audio range: [{min_amplitude}, {max_amplitude}]");
        }

        log::debug!("Total audio samples after resampling: {}", final_data.len());

        let mut speech_chunks = 0;
        let mut total_chunks = 0;

        self.vad.reset_state();
        let vad_threshold = self.state.vad_threshold();

        for chunk in final_data.chunks(VAD_CHUNK) {
            if chunk.len() < VAD_CHUNK {
                continue;
            }
            match self.vad.check_speech(chunk, SAMPLE_RATE) {
                Ok(probability) => {
                    if probability > vad_threshold {
                        speech_chunks += 1;
                    }
                }
                Err(e) => {
                    log::error!("VAD error: {e}");
                    continue;
                }
            };
            total_chunks += 1;
        }

        log::debug!("VAD detected speech in {speech_chunks} out of {total_chunks} chunks.");

        let speech_ratio = if total_chunks > 0 {
            speech_chunks as f32 / total_chunks as f32
        } else {
            0.0
        };

        if speech_ratio < 0.1 {
            log::info!(
                "Only detected speech in {:.2}% of the audio. Skipping transcription.",
                speech_ratio * 100.0
            );
            self.cues.emit(Signal::Error {
                reason: Reason::new(ReasonCode::NoSpeech, device.as_deref()),
                target: Some(Target::Dictate),
            });
            return;
        }

        if speech_chunks < 2 {
            log::info!("No speech detected in the audio. Skipping transcription.");
            self.cues.emit(Signal::Error {
                reason: Reason::new(ReasonCode::NoSpeech, device.as_deref()),
                target: Some(Target::Dictate),
            });
            return;
        }

        log::debug!("Transcribing...");
        let transcribe_started = Instant::now();
        let transcribed = self.transcribe(final_data);
        // A client refetches its history when `transcribing` falls, so the
        // row has to be stored before the guard drops.
        store(&self.state, transcribed.as_deref().ok());
        match transcribed {
            Ok(transcription) => {
                self.state.set_last_error(None);
                let audio_secs = final_data.len() as f32 / SAMPLE_RATE as f32;
                let elapsed = transcribe_started.elapsed().as_secs_f32();
                log::info!("Transcribed {audio_secs:.1}s of audio in {elapsed:.2}s");
                // A slow CPU reads as a dead microphone rather than a slow one
                let slowdown = elapsed / audio_secs.max(0.001);
                if slowdown > SLOW_TRANSCRIBE_FACTOR
                    && let Some(advice) = self.speech_to_text.slow_advice()
                {
                    log::warn!(
                        "Transcription ran {slowdown:.0}x slower than realtime on this \
                         machine. {advice}"
                    );
                }
                log::debug!("Transcription: {transcription}");

                // The model can return nothing for noise; skip before it reaches the clipboard
                if transcription.is_empty() {
                    log::info!("Empty transcription. Skipping.");
                    self.cues.emit(Signal::Error {
                        reason: Reason::new(ReasonCode::EmptyTranscript, device.as_deref()),
                        target: Some(Target::Dictate),
                    });
                    return;
                }

                // Ready only after the utterance is actually delivered
                log::debug!("Dictating: {}", transcription);
                self.state.set_typing(true);
                let typed = type_words(&transcription);
                self.state.set_typing(false);
                match typed {
                    Ok(_) => {
                        self.cues.emit(Signal::Ready {
                            target: Target::Dictate,
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to type text: {e}");
                        self.cues.emit(Signal::Error {
                            reason: Reason::new(ReasonCode::TypeFailed, None),
                            target: Some(Target::Dictate),
                        });
                    }
                }
            }
            Err(error) => {
                let why = reason(&error);
                log::error!("Transcription failed: {why}");
                self.state.set_last_error(Some(why));
                self.cues.emit(Signal::Error {
                    reason: Reason::new(ReasonCode::TranscriptionFailed, None),
                    target: Some(Target::Dictate),
                });
            }
        }
    }

    // Armed listening: the mode is Armed and the question has finished playing
    fn ask(&mut self, ask: AskCommand) {
        // The ring holds echo captured while the question played
        self.source.discard();
        // An earlier use's failure is not this answer's
        self.source.forget_failure();
        // Before capture opens: a load that outlasted the open and the arm
        // cue's settle would drop the start of the answer with the cue.
        let listened = self.ready().and_then(|()| {
            self.state.hear_answer(ask.session);
            let asked = Instant::now();
            match self.await_arm_cue(asked + ask.timeout, ask.session)? {
                None => {
                    self.state.open_answer(ask.session);
                    let left = listen_budget(ask.timeout, asked.elapsed());
                    self.listen_for_answer(left, ask.session)
                }
                Some(ended) => Ok(ended),
            }
        });

        // Close the mic before the slow transcription; every exit disarms
        self.state.disarm(ask.session);
        self.cues.emit(Signal::Disarm);

        // The live words were the stream's; the answer is one full pass
        self.speech_to_text.cancel_stream();
        let state = Arc::clone(&self.state);
        let cues = self.cues.clone();
        let text = settle_answer(&state, &cues, listened, |audio| self.transcribe(audio));
        let _ = ask.reply.send(text);
    }

    /// Waits while capture opens for the answer, until its first sound plays
    /// the arm cue, then lets the cue leave the speaker. Everything captured
    /// until then is dropped at once, so the detector never hears the cue.
    /// `None` when the answer is to be listened for: the cue has played, or a
    /// hold took over. A mic that only ever delivers silence waits out
    /// `deadline`.
    fn await_arm_cue(&mut self, deadline: Instant, session: u64) -> Result<Option<Heard>, String> {
        loop {
            thread::sleep(ARMED_POLL);
            match self.state.armed_mode(session) {
                Some(RecordingMode::ArmedHold) => return Ok(None),
                Some(RecordingMode::Armed) => {}
                _ => return Ok(Some(Heard::Closed)),
            }
            if let Some(why) = self.source.failure() {
                return Err(why);
            }
            if self.state.arm_cue_played() {
                break;
            }
            if Instant::now() >= deadline {
                return Ok(Some(Heard::Silence));
            }
        }
        thread::sleep(CUE_SETTLE);
        self.source.discard();
        Ok(None)
    }

    /// Opens the answer's stream at `start` in its audio, after any stream
    /// opened before. `None` for a model that does not stream.
    fn open_answer_stream(&mut self, start: usize) -> Option<usize> {
        self.speech_to_text.cancel_stream();
        if !self.begin_live_words("answer") {
            return None;
        }
        // A hold restarts the stream, and the words before it are not the answer
        self.state.publish_words("");
        Some(start)
    }

    // Confirms onset, then ends on trailing silence; the audio comes back at
    // 16 kHz. `Heard::Silence` and `Heard::Closed` are an answer that never
    // came; `Err` is a listen that broke.
    fn listen_for_answer(&mut self, timeout: Duration, session: u64) -> Result<Heard, String> {
        // The device under this answer may change at any moment, and the rate
        // is not shared between devices.
        let mut capture = Resampled::new(&self.source);
        self.vad.reset_state();
        let vad_threshold = self.state.vad_threshold();
        let endpoint_chunks = (self.state.endpoint_silence_ms() / CHUNK_MS).max(1) as usize;
        let deadline = Instant::now() + timeout;
        let hard_deadline = deadline + MAX_ANSWER;

        let mut audio: Vec<f32> = Vec::new();
        let mut processed = 0;
        let mut phase = Phase::Waiting { speech_run: 0 };
        let mut suppressed = false;
        // How far into `audio` the open stream has heard, from the onset on
        let mut fed: Option<usize> = None;

        loop {
            thread::sleep(ARMED_POLL);

            match self.state.armed_mode(session) {
                Some(RecordingMode::ArmedHold) => {
                    if !matches!(phase, Phase::Manual { .. }) {
                        // The hold replaces whatever endpointing had collected
                        phase = Phase::Manual { start: audio.len() };
                        fed = self.open_answer_stream(audio.len());
                    }
                }
                Some(RecordingMode::Armed) => {
                    if let Phase::Manual { start } = phase {
                        // The hotkey release ends the manual answer
                        capture.finish(&mut audio)?;
                        audio.drain(..start);
                        return Ok(Heard::Audio(audio));
                    }
                }
                // The session was closed from outside
                _ => return Ok(Heard::Closed),
            }
            if let Some(why) = self.source.failure() {
                return Err(why);
            }

            // Checked before suppression so stuck speech cannot hang the session
            if Instant::now() >= hard_deadline {
                return match phase {
                    Phase::InSpeech { start, .. } | Phase::Manual { start } => {
                        capture.finish(&mut audio)?;
                        audio.drain(..start);
                        Ok(Heard::Audio(audio))
                    }
                    Phase::Waiting { .. } => Ok(Heard::Silence),
                };
            }

            // Half-duplex: drop capture while the daemon itself is speaking
            if self.state.speech().is_speaking() {
                self.source.discard();
                suppressed = true;
                continue;
            }
            if suppressed {
                suppressed = false;
                capture.reset();
                afresh(&mut self.vad, &mut phase);
            }

            // The microphone moved under this answer. What was said already is
            // kept: it is resampled and belongs to the answer. What cannot be
            // kept is a window holding the old device's audio.
            if capture.read(&self.source, &mut audio)? {
                log::info!("the microphone moved while a question was listening");
                afresh(&mut self.vad, &mut phase);
            }

            // Manual capture needs no VAD; the release is the endpoint
            while !matches!(phase, Phase::Manual { .. }) && audio.len() - processed >= VAD_CHUNK {
                let chunk = &audio[processed..processed + VAD_CHUNK];
                processed += VAD_CHUNK;
                let is_speech = match self.vad.check_speech(chunk, SAMPLE_RATE) {
                    Ok(probability) => probability > vad_threshold,
                    Err(e) => {
                        log::error!("VAD error: {e}");
                        false
                    }
                };
                let next = advance(phase, is_speech, processed);
                if onset(phase, next) {
                    self.cues.emit(Signal::Onset);
                    if let Phase::InSpeech { start, .. } = next {
                        fed = self.open_answer_stream(start);
                    }
                }
                phase = next;
            }
            while feed_stream(&mut *self.speech_to_text, &self.state, &audio, &mut fed) {}

            if let Phase::InSpeech { silence_run, start } = phase
                && silence_run >= endpoint_chunks
            {
                capture.finish(&mut audio)?;
                audio.drain(..start);
                return Ok(Heard::Audio(audio));
            }
            if matches!(phase, Phase::Waiting { .. }) && Instant::now() >= deadline {
                return Ok(Heard::Silence);
            }
        }
    }
}

fn settle_answer(
    state: &DaemonState,
    cues: &Cues,
    listened: Result<Heard, String>,
    transcribe: impl FnOnce(&[f32]) -> Result<String, BansheeError>,
) -> Result<String, String> {
    match listened {
        Ok(Heard::Audio(audio)) => {
            let transcribing = Raised::on(state, DaemonState::set_transcribing);
            let transcribed = transcribe(&audio);
            store(state, transcribed.as_deref().ok());
            drop(transcribing);
            match transcribed {
                Ok(text) => {
                    state.set_last_error(None);
                    log::debug!("Answer: {text}");
                    cues.emit(Signal::Answered {
                        heard: !text.is_empty(),
                    });
                    Ok(text)
                }
                Err(e) => {
                    let why = reason(&e);
                    log::error!("Transcription failed: {why}");
                    state.set_last_error(Some(why.clone()));
                    cues.emit(Signal::Error {
                        reason: Reason::new(ReasonCode::TranscriptionFailed, None),
                        target: Some(Target::Answer),
                    });
                    Err(why)
                }
            }
        }
        Ok(heard @ (Heard::Silence | Heard::Closed)) => {
            // last_error is left alone: silence is not a transcription, so
            // it neither clears the last failure nor is one
            let code = if matches!(heard, Heard::Silence) {
                ReasonCode::Silence
            } else {
                ReasonCode::Closed
            };
            cues.emit(Signal::Error {
                reason: Reason::new(code, None),
                target: Some(Target::Answer),
            });
            Ok(String::new())
        }
        Err(why) => {
            state.set_last_error(Some(why.clone()));
            cues.emit(Signal::Error {
                reason: Reason::new(ReasonCode::ListenFailed, None),
                target: Some(Target::Answer),
            });
            Err(why)
        }
    }
}

/// Stores an utterance worth keeping. The model can answer an empty string
/// for noise, and that is not a dictation.
fn store(state: &DaemonState, transcription: Option<&str>) {
    if let Some(text) = transcription
        && !text.is_empty()
    {
        save_history(state, text);
    }
}

fn save_history(state: &DaemonState, transcription: &str) {
    let stored =
        state.with_history(|c| crate::history::TranscriptionHistory::insert(c, transcription));
    if let Some(Err(e)) = stored {
        log::error!("Failed to insert transcription into database: {e}");
    }
}

#[cfg(test)]
mod answer_tests {
    use super::*;
    use crate::audio::cues::Cue;

    #[test]
    fn a_load_that_unwinds_still_clears_the_flag() {
        let (state, _lines) =
            crate::test_support::daemon_state_recording_speech_for(std::sync::mpsc::channel().0);
        assert!(!state.is_loading_model());

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _loading = super::Raised::on(&state, DaemonState::set_loading_model);
            assert!(
                state.is_loading_model(),
                "the flag is set while the load runs"
            );
            panic!("the engine gave up");
        }));

        assert!(unwound.is_err(), "the panic must reach the caller");
        assert!(
            !state.is_loading_model(),
            "the guard clears the flag on the way out"
        );
    }

    type Settled = (
        Result<String, String>,
        Signal,
        Vec<Cue>,
        std::sync::Arc<DaemonState>,
    );

    fn settled(
        listened: Result<Heard, String>,
        transcribed: Result<String, banshee_common::error::BansheeError>,
    ) -> Settled {
        let (cues, sounds) = Cues::recording();
        let mut signals = cues.subscribe_signals();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let text = super::settle_answer(&state, &cues, listened, |_| transcribed);
        let signal = signals.try_recv().expect("the answer sends one signal");
        (text, signal, sounds.try_iter().collect(), state)
    }

    #[test]
    fn an_answer_is_heard_when_it_has_words_and_sounds_nothing() {
        for (name, transcribed, heard) in [
            ("an answer with words", "yes please", true),
            ("an answer the model hears as nothing", "", false),
        ] {
            let (text, signal, sounds, state) =
                settled(Ok(Heard::Audio(vec![0.0; 16])), Ok(transcribed.to_string()));
            assert_eq!(text, Ok(transcribed.to_string()), "{name}");
            assert_eq!(signal, Signal::Answered { heard }, "{name}");
            assert!(sounds.is_empty(), "{name}: answered has no earcon");
            assert!(
                !state.is_transcribing(),
                "{name}: the flag falls on every exit"
            );
        }
    }

    #[test]
    fn silence_closed_and_a_broken_listen_are_three_reasons() {
        for (listened, code, text) in [
            (Ok(Heard::Silence), ReasonCode::Silence, Ok(String::new())),
            (Ok(Heard::Closed), ReasonCode::Closed, Ok(String::new())),
            (
                Err("the microphone went away".to_string()),
                ReasonCode::ListenFailed,
                Err("the microphone went away".to_string()),
            ),
        ] {
            let (answer, signal, sounds, _) = settled(listened, Ok("unused".to_string()));
            assert_eq!(answer, text);
            let Signal::Error { reason, target } = signal else {
                panic!("{code:?} must send an error");
            };
            assert_eq!(reason.code, code);
            assert_eq!(target, Some(Target::Answer), "{code:?}");
            assert_eq!(sounds, vec![Cue::Error], "{code:?}");
        }
    }

    #[test]
    fn a_failed_transcription_of_an_answer_keeps_its_reason_for_status() {
        let (answer, signal, _, state) = settled(
            Ok(Heard::Audio(vec![0.0; 16])),
            Err(banshee_common::error::BansheeError::Other(
                "model gone".to_string(),
            )),
        );
        assert!(answer.is_err());
        let Signal::Error { reason, target } = signal else {
            panic!("a failed transcription sends an error");
        };
        assert_eq!(reason.code, ReasonCode::TranscriptionFailed);
        assert_eq!(target, Some(Target::Answer));
        assert!(state.last_error().is_some());
    }
}

#[cfg(test)]
mod utterance_tests {
    use super::*;
    use crate::speech_to_text::Speech;
    use banshee_common::error::BansheeError;
    use banshee_common::listening::ListeningModel;
    use ringbuf::traits::{Producer, Split};
    use std::sync::atomic::AtomicBool;

    const FAILED_LOAD_FIX: &str =
        ". The next use tries again. If it fails every time, restart: banshee start";

    /// Answers one fixed result, and notes whether the flag was up and a
    /// dictation still in the ring when asked. One that streams refuses a
    /// second stream while one is open, as the model's compute lease does.
    struct Scripted {
        state: Arc<DaemonState>,
        answer: Result<String, String>,
        saw_the_flag: Arc<AtomicBool>,
        saw_a_dictation_in_ring: Arc<AtomicBool>,
        streams: bool,
        /// The committed words the first feed answers.
        words: Option<String>,
        /// Told of every feed.
        fed: Option<mpsc::Sender<()>>,
        /// Each feed waits here before it returns, until the sender goes.
        gate: Option<mpsc::Receiver<()>>,
        /// Every feed fails.
        feed_fails: bool,
        /// Every reload fails.
        reload_fails: bool,
        open: bool,
        calls: Arc<Mutex<Vec<Call>>>,
        /// The model is in memory.
        resident: bool,
        /// This many loads fail before one succeeds.
        loads_fail: usize,
        /// What the daemon reported while each load ran.
        during_load: Arc<Mutex<Vec<DuringLoad>>>,
        /// Told of every unload that freed the model.
        unloaded: Option<mpsc::Sender<()>>,
        /// Told of every load as it begins.
        loading: Option<mpsc::Sender<()>>,
        /// Each load waits here before it ends.
        load_gate: Option<mpsc::Receiver<()>>,
        sends_out: bool,
        /// Each transcribe waits here before it returns, until the sender goes
        /// or five seconds pass.
        transcribe_gate: Option<mpsc::Receiver<()>>,
        /// Told as each transcribe begins.
        transcribing: Option<mpsc::Sender<()>>,
    }

    /// The loading flag, the listening fact, the live state and whether the
    /// microphone was wanted, as a load saw them.
    type DuringLoad = (bool, crate::state::Listening, serde_json::Value, bool);

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Transcribe,
        Begin,
        Feed(usize),
        /// With the words on show when the stream was cancelled.
        Cancel(String),
        Reload,
        Load,
        Unload,
    }

    impl Scripted {
        fn new(state: &Arc<DaemonState>, answer: Result<&str, &str>) -> Self {
            Self {
                state: Arc::clone(state),
                answer: answer.map(str::to_string).map_err(str::to_string),
                saw_the_flag: Arc::default(),
                saw_a_dictation_in_ring: Arc::default(),
                streams: false,
                words: None,
                fed: None,
                gate: None,
                feed_fails: false,
                reload_fails: false,
                open: false,
                calls: Arc::default(),
                resident: true,
                loads_fail: 0,
                during_load: Arc::default(),
                unloaded: None,
                loading: None,
                load_gate: None,
                sends_out: false,
                transcribe_gate: None,
                transcribing: None,
            }
        }

        fn note(&self, call: Call) {
            self.calls.lock().unwrap().push(call);
        }
    }

    impl Transcriber for Scripted {
        fn transcribe(&mut self, _audio: &[f32]) -> Result<String, BansheeError> {
            self.note(Call::Transcribe);
            self.saw_the_flag
                .store(self.state.is_transcribing(), Ordering::Relaxed);
            self.saw_a_dictation_in_ring
                .store(self.state.dictation_in_ring(), Ordering::Relaxed);
            if let Some(transcribing) = &self.transcribing {
                let _ = transcribing.send(());
            }
            if let Some(gate) = &self.transcribe_gate {
                let _ = gate.recv_timeout(Duration::from_secs(5));
            }
            self.answer.clone().map_err(BansheeError::Transcription)
        }
        fn sends_out(&self) -> bool {
            self.sends_out
        }
        fn begin_stream(&mut self) -> Result<bool, BansheeError> {
            if !self.streams {
                return Ok(false);
            }
            if self.open {
                return Err(BansheeError::Transcription(
                    "a stream is already active on this model".to_string(),
                ));
            }
            self.open = true;
            self.note(Call::Begin);
            Ok(true)
        }
        fn feed(&mut self, pcm_16k: &[f32]) -> Result<Option<String>, BansheeError> {
            self.note(Call::Feed(pcm_16k.len()));
            if let Some(fed) = &self.fed {
                let _ = fed.send(());
            }
            if let Some(gate) = &self.gate {
                let _ = gate.recv();
            }
            if self.feed_fails {
                return Err(BansheeError::Transcription("the stream broke".to_string()));
            }
            Ok(self.words.take())
        }
        fn cancel_stream(&mut self) {
            if std::mem::take(&mut self.open) {
                self.note(Call::Cancel(self.state.subscribe_words().borrow().clone()));
            }
        }
        fn set_vocabulary(&mut self, _words: &[String]) {}
        fn set_speech(&mut self, _speech: Speech) {}
        fn reload(
            &mut self,
            model: &'static ListeningModel,
        ) -> Result<Option<&'static ListeningModel>, BansheeError> {
            self.note(Call::Reload);
            if self.reload_fails {
                return Err(BansheeError::Other("the file is damaged".to_string()));
            }
            Ok(Some(model))
        }
        fn resident(&self) -> bool {
            self.resident
        }
        fn load(&mut self) -> Result<(), BansheeError> {
            self.note(Call::Load);
            self.during_load.lock().unwrap().push((
                self.state.is_loading_model(),
                self.state.listening(),
                crate::api::live_state(&self.state),
                self.state.wants_mic(),
            ));
            if let Some(loading) = &self.loading {
                let _ = loading.send(());
            }
            if let Some(gate) = &self.load_gate {
                let _ = gate.recv_timeout(Duration::from_secs(5));
            }
            if self.loads_fail > 0 {
                self.loads_fail -= 1;
                return Err(BansheeError::Other(
                    "the listening model did not load: the file is damaged".to_string(),
                ));
            }
            self.resident = true;
            Ok(())
        }
        fn unload(&mut self) -> bool {
            if !std::mem::take(&mut self.resident) {
                return false;
            }
            self.note(Call::Unload);
            if let Some(unloaded) = &self.unloaded {
                let _ = unloaded.send(());
            }
            true
        }
    }

    struct Handled {
        signals: Vec<Signal>,
        rose: bool,
        up_while_transcribing: bool,
        state: Arc<DaemonState>,
    }

    fn handle(
        audio: &[f32],
        answer: Result<&str, &str>,
        type_words: impl FnOnce(&DaemonState, &str) -> Result<(), BansheeError>,
    ) -> Handled {
        let (cues, _sounds) = Cues::recording();
        let mut subscribed = cues.subscribe_signals();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let scripted = Scripted::new(&state, answer);
        let saw_the_flag = Arc::clone(&scripted.saw_the_flag);
        let mut pipeline = holding(audio, &state, cues, scripted);
        let mut flag = state.subscribe_transcribing();
        flag.mark_unchanged();
        let typing = Arc::clone(&state);
        pipeline.transcribe_utterance(state.claim_ring(), move |words| type_words(&typing, words));
        let rose = flag.has_changed().unwrap();
        let mut signals = Vec::new();
        while let Ok(signal) = subscribed.try_recv() {
            signals.push(signal);
        }
        Handled {
            signals,
            rose,
            up_while_transcribing: saw_the_flag.load(Ordering::Relaxed),
            state,
        }
    }

    #[test]
    fn a_transcription_frees_the_ring_before_it_transcribes() {
        let (commands, requests) = std::sync::mpsc::channel();
        let state = crate::test_support::daemon_state(commands);
        assert!(state.record_start());
        state.record_stop();
        assert!(
            matches!(requests.try_recv(), Ok(ConsumerCommand::Stream(_))),
            "the press streams first"
        );
        let Ok(ConsumerCommand::Transcribe(ring)) = requests.try_recv() else {
            panic!("the stop sends the dictation to the consumer");
        };
        assert!(state.dictation_in_ring());
        let saw_a_dictation_in_ring = Arc::new(AtomicBool::new(true));
        let scripted = Scripted {
            saw_a_dictation_in_ring: Arc::clone(&saw_a_dictation_in_ring),
            ..Scripted::new(&state, Ok("words"))
        };
        let mut pipeline = holding(&spoken_then_quiet(), &state, Cues::recording().0, scripted);

        pipeline.transcribe_utterance(ring, |_| Ok(()));

        assert!(
            !saw_a_dictation_in_ring.load(Ordering::Relaxed),
            "a waiting question must not sit through the transcription itself"
        );
        assert!(!state.dictation_in_ring());
    }

    /// A pipeline whose capture already holds `audio`, at the rate the detector reads.
    fn holding(
        audio: &[f32],
        state: &Arc<DaemonState>,
        cues: Cues,
        speech_to_text: Scripted,
    ) -> Pipeline {
        let (pipeline, mut producer) = fillable(audio.len(), state, cues, speech_to_text);
        producer.push_slice(audio);
        pipeline
    }

    /// A pipeline whose capture holds up to `room` samples, and the end the
    /// test fills it from.
    fn fillable(
        room: usize,
        state: &Arc<DaemonState>,
        cues: Cues,
        speech_to_text: Scripted,
    ) -> (Pipeline, HeapProd<f32>) {
        let (producer, consumer) = ringbuf::HeapRb::<f32>::new(room).split();
        state.set_endpoint_silence_ms(800);
        let pipeline = Pipeline {
            source: Arc::new(Capture::new(CaptureSource {
                consumer,
                sample_rate: SAMPLE_RATE,
            })),
            speech_to_text: Box::new(speech_to_text),
            vad: Detector::open(crate::test_support::silero()).unwrap(),
            state: Arc::clone(state),
            cues,
            used: Instant::now(),
            build_listener: Box::new(|_| {
                Err(RecordingError::Model(
                    "this test builds no listener".to_string(),
                ))
            }),
            withdrawals_seen: 0,
        };
        (pipeline, producer)
    }

    /// Listens once for the answer to an armed question whose capture holds
    /// `audio`, and gives back what it heard with every signal it sent. `held`
    /// holds the hotkey from the start and releases it that long after.
    fn listened(audio: &[f32], held: Option<Duration>) -> (Result<Heard, String>, Vec<Signal>) {
        let (heard, signals, _, _) = listened_by(audio, held, false);
        (heard, signals)
    }

    /// As `listened`, by a model that streams when `streams` says so, with
    /// every call the model took and the words on show at the end. Words from
    /// an earlier recording are on show when it starts.
    fn listened_by(
        audio: &[f32],
        held: Option<Duration>,
        streams: bool,
    ) -> (Result<Heard, String>, Vec<Signal>, Vec<Call>, String) {
        let (cues, _sounds) = Cues::recording();
        let mut subscribed = cues.subscribe_signals();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let session = state.arm_for_ask().unwrap();
        assert!(state.open_answer(session));
        let scripted = Scripted {
            streams,
            ..Scripted::new(&state, Ok(""))
        };
        let calls = Arc::clone(&scripted.calls);
        let mut pipeline = holding(audio, &state, cues, scripted);
        let _viewer = state.subscribe_words();
        state.publish_words("an earlier recording");
        let release = held.map(|after| {
            assert!(state.try_transition(RecordingMode::Armed, RecordingMode::ArmedHold));
            let releasing = Arc::clone(&state);
            thread::spawn(move || {
                thread::sleep(after);
                releasing.try_transition(RecordingMode::ArmedHold, RecordingMode::Armed)
            })
        });
        let heard = pipeline.listen_for_answer(Duration::from_millis(300), session);
        if let Some(release) = release {
            assert!(release.join().unwrap(), "the hold was still on at release");
        }
        let mut signals = Vec::new();
        while let Ok(signal) = subscribed.try_recv() {
            signals.push(signal);
        }
        let calls = calls.lock().unwrap().clone();
        let words = state.subscribe_words().borrow().clone();
        (heard, signals, calls, words)
    }

    fn spoken_then_quiet() -> Vec<f32> {
        let mut audio = crate::speech_to_text::vad::test_speech();
        audio.extend(std::iter::repeat_n(0.0, SAMPLE_RATE as usize));
        audio
    }

    #[test]
    fn an_answer_sends_onset_once_as_the_detector_confirms_speech() {
        let (heard, signals) = listened(&spoken_then_quiet(), None);
        assert!(matches!(heard, Ok(Heard::Audio(_))));
        assert_eq!(signals, [Signal::Onset]);
    }

    #[test]
    fn a_quiet_room_sends_no_onset() {
        let (heard, signals) = listened(&[0.0; SAMPLE_RATE as usize], None);
        assert!(matches!(heard, Ok(Heard::Silence)));
        assert_eq!(signals, []);
    }

    #[test]
    fn a_held_answer_sends_no_onset() {
        let (heard, signals) = listened(&spoken_then_quiet(), Some(Duration::from_millis(200)));
        assert!(matches!(heard, Ok(Heard::Audio(_))));
        assert_eq!(signals, []);
    }

    /// Speech, three seconds of quiet, speech, then four seconds of quiet.
    fn speech_gap_speech() -> Vec<f32> {
        let mut audio = crate::speech_to_text::vad::test_speech();
        audio.extend(std::iter::repeat_n(0.0, 3 * SAMPLE_RATE as usize));
        audio.extend(crate::speech_to_text::vad::test_speech());
        audio.extend(std::iter::repeat_n(0.0, 4 * SAMPLE_RATE as usize));
        audio
    }

    /// `listened_by` with the daemon holding `quiet_ms` before the answer
    /// opens. The audio arrives in sixteenth-second slices, so the endpoint is
    /// judged between slices as it is on a live microphone.
    fn listened_with_quiet_length(audio: &[f32], quiet_ms: u64) -> Vec<f32> {
        let (cues, _sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let session = state.arm_for_ask().unwrap();
        assert!(state.open_answer(session));
        let scripted = Scripted::new(&state, Ok(""));
        let (mut pipeline, mut producer) = fillable(audio.len(), &state, cues, scripted);
        state.set_endpoint_silence_ms(quiet_ms);
        let samples = audio.to_vec();
        let feeding = thread::spawn(move || {
            for slice in samples.chunks(SAMPLE_RATE as usize / 16) {
                producer.push_slice(slice);
                thread::sleep(Duration::from_millis(10));
            }
        });
        // The feed reaches speech about 100 ms in on a fast machine, and the
        // wait for it is wall time, so it is long enough for a slow runner.
        let heard = pipeline.listen_for_answer(Duration::from_secs(5), session);
        feeding.join().unwrap();
        match heard {
            Ok(Heard::Audio(samples)) => samples,
            _ => panic!("the answer must be heard as audio"),
        }
    }

    #[test]
    fn an_answer_ends_on_the_quiet_length_the_daemon_holds() {
        let short = listened_with_quiet_length(&speech_gap_speech(), 800);
        let long = listened_with_quiet_length(&speech_gap_speech(), 3500);
        let gap = SAMPLE_RATE as usize;
        assert!(
            short.len() + gap < long.len(),
            "800 ms ends on the first gap, 3500 ms hears past it: {} then {}",
            short.len(),
            long.len()
        );
    }

    fn untyped(_: &DaemonState, _: &str) -> Result<(), BansheeError> {
        panic!("this utterance never reaches the typer")
    }

    fn failed(code: ReasonCode, target: Target) -> impl Fn(&Signal) -> bool {
        move |signal| {
            matches!(signal, Signal::Error { reason, target: named }
                if reason.code == code && *named == Some(target))
        }
    }

    #[test]
    fn every_exit_of_a_dictation_that_is_not_typed_signals_its_reason_and_lowers_the_flag() {
        let quiet = vec![0.0; SAMPLE_RATE as usize];
        let speech = crate::speech_to_text::vad::test_speech();
        let cases = [
            (
                "no speech: the flag covers the voice detector too",
                quiet,
                Ok("unused"),
                ReasonCode::NoSpeech,
                false,
            ),
            (
                "an empty transcript",
                speech.clone(),
                Ok(""),
                ReasonCode::EmptyTranscript,
                true,
            ),
            (
                "a failed transcription",
                speech,
                Err("the model is gone"),
                ReasonCode::TranscriptionFailed,
                true,
            ),
        ];

        for (name, audio, answer, code, up_while_transcribing) in cases {
            let handled = handle(&audio, answer, untyped);
            assert!(
                handled.signals.iter().any(failed(code, Target::Dictate)),
                "{name}: {:?}",
                handled.signals
            );
            assert!(handled.rose, "{name}");
            assert_eq!(
                handled.up_while_transcribing, up_while_transcribing,
                "{name}"
            );
            assert!(
                !handled.state.is_transcribing(),
                "{name}: the exit lowers it"
            );
        }
    }

    #[test]
    fn the_flag_stays_up_while_the_words_are_typed() {
        let mut up_while_typing = false;
        let handled = handle(
            &crate::speech_to_text::vad::test_speech(),
            Ok("hello there"),
            |state, _| {
                up_while_typing = state.is_transcribing();
                Err(BansheeError::Other("no accessibility grant".to_string()))
            },
        );
        assert!(
            handled
                .signals
                .iter()
                .any(failed(ReasonCode::TypeFailed, Target::Dictate)),
            "{:?}",
            handled.signals
        );
        assert!(up_while_typing, "delivery is part of the job");
        assert!(!handled.state.is_transcribing());
    }

    /// What a consumer thread did with the commands `commands` sends, then a
    /// shutdown, with a capture that holds `audio`.
    struct Consumed {
        typed: Vec<String>,
        calls: Vec<Call>,
        words_after: String,
    }

    /// With a client that shows the words, as the macOS chip does.
    fn consumed(
        scripted: impl FnOnce(&Arc<DaemonState>) -> Scripted,
        audio: &[f32],
        commands: impl FnOnce(&Arc<DaemonState>, &mpsc::Sender<ConsumerCommand>),
    ) -> Consumed {
        consumed_with(true, scripted, audio, commands)
    }

    fn consumed_with(
        watched: bool,
        scripted: impl FnOnce(&Arc<DaemonState>) -> Scripted,
        audio: &[f32],
        commands: impl FnOnce(&Arc<DaemonState>, &mpsc::Sender<ConsumerCommand>),
    ) -> Consumed {
        consumed_by(watched, Sent::WhileItRuns, scripted, audio, commands)
    }

    /// As `consumed`, with every command and the shutdown queued before the
    /// consumer starts. A capture that holds audio is fed on the first poll
    /// that finds no command, so only this order fixes what the model hears.
    fn consumed_queued(
        scripted: impl FnOnce(&Arc<DaemonState>) -> Scripted,
        audio: &[f32],
        commands: impl FnOnce(&Arc<DaemonState>, &mpsc::Sender<ConsumerCommand>),
    ) -> Consumed {
        consumed_by(true, Sent::BeforeItStarts, scripted, audio, commands)
    }

    #[derive(PartialEq)]
    enum Sent {
        WhileItRuns,
        BeforeItStarts,
    }

    fn consumed_by(
        watched: bool,
        sent: Sent,
        scripted: impl FnOnce(&Arc<DaemonState>) -> Scripted,
        audio: &[f32],
        commands: impl FnOnce(&Arc<DaemonState>, &mpsc::Sender<ConsumerCommand>),
    ) -> Consumed {
        let (cues, _sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let _viewer = watched.then(|| state.subscribe_words());
        let scripted = scripted(&state);
        let calls = Arc::clone(&scripted.calls);
        let pipeline = holding(audio, &state, cues, scripted);
        let typed = Arc::new(Mutex::new(Vec::new()));
        let typing = Arc::clone(&typed);
        let (sender, receiver) = mpsc::channel();
        let (go, gate) = mpsc::channel::<()>();
        let consumer = thread::spawn(move || {
            let _ = gate.recv();
            consume(pipeline, &receiver, &move |words: &str| {
                typing.lock().unwrap().push(words.to_string());
                Ok(())
            })
        });
        if sent == Sent::WhileItRuns {
            let _ = go.send(());
        }
        commands(&state, &sender);
        let _ = sender.send(ConsumerCommand::Shutdown);
        let _ = go.send(());
        consumer.join().unwrap();
        let typed = typed.lock().unwrap().clone();
        let calls = calls.lock().unwrap().clone();
        let words_after = state.subscribe_words().borrow().clone();
        Consumed {
            typed,
            calls,
            words_after,
        }
    }

    fn streaming(state: &Arc<DaemonState>, answer: &str) -> Scripted {
        Scripted {
            streams: true,
            ..Scripted::new(state, Ok(answer))
        }
    }

    #[test]
    fn dictation_streams_while_the_key_is_held() {
        let (fed, feeding) = mpsc::channel();
        let mut fed_before_release = false;
        let done = consumed(
            |state| Scripted {
                words: Some("hello".to_string()),
                fed: Some(fed),
                ..streaming(state, "hello there")
            },
            &spoken_then_quiet(),
            |state, commands| {
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                fed_before_release = feeding.recv_timeout(Duration::from_secs(5)).is_ok();
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
            },
        );
        assert!(fed_before_release, "the model hears the words as they come");
        assert_eq!(done.calls.first(), Some(&Call::Begin));
        assert_eq!(
            done.calls[done.calls.len() - 2..],
            [Call::Cancel("hello".to_string()), Call::Transcribe],
            "the words show until the release, then one full pass runs: {:?}",
            done.calls
        );
        assert_eq!(done.typed, ["hello there"], "the full pass is typed");
        assert_eq!(done.words_after, "", "the recording's end clears the words");
    }

    #[test]
    fn a_dictation_nobody_watches_is_not_streamed() {
        let done = consumed_with(
            false,
            |state| streaming(state, "typed whole"),
            &spoken_then_quiet(),
            |state, commands| {
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
            },
        );
        assert_eq!(done.calls, [Call::Transcribe]);
        assert_eq!(done.typed, ["typed whole"]);
    }

    #[test]
    fn an_answer_streams_only_while_a_client_shows_the_words() {
        let (cues, _sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let scripted = streaming(&state, "");
        let calls = Arc::clone(&scripted.calls);
        let mut pipeline = holding(&spoken_then_quiet(), &state, cues, scripted);
        assert_eq!(pipeline.open_answer_stream(0), None, "nobody watches");
        assert!(calls.lock().unwrap().is_empty());
        let _viewer = state.subscribe_words();
        assert_eq!(pipeline.open_answer_stream(0), Some(0));
        assert_eq!(*calls.lock().unwrap(), [Call::Begin]);
    }

    #[test]
    fn a_feed_that_fails_still_types_the_dictation_whole() {
        let (fed, feeding) = mpsc::channel();
        let done = consumed(
            |state| Scripted {
                fed: Some(fed),
                feed_fails: true,
                ..streaming(state, "typed whole")
            },
            &spoken_then_quiet(),
            |state, commands| {
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                feeding
                    .recv_timeout(Duration::from_secs(5))
                    .expect("the dictation streams");
                // Polls enough for a stream that kept going to be fed again
                thread::sleep(ARMED_POLL * 4);
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
            },
        );
        assert_eq!(done.typed, ["typed whole"]);
        assert_eq!(
            done.calls
                .iter()
                .filter(|call| matches!(call, Call::Feed(_)))
                .count(),
            1,
            "a failed feed stops the stream: {:?}",
            done.calls
        );
        assert_eq!(done.calls.last(), Some(&Call::Transcribe));
    }

    // The stream falls behind when the model is slow to take a window. A
    // release that lands while it catches up ends the dictation at the next
    // window, not after the whole backlog.
    #[test]
    fn a_release_during_a_backlog_lands_between_two_feeds() {
        let (fed, feeding) = mpsc::channel();
        let (opens, gate) = mpsc::channel::<()>();
        let audio = spoken_then_quiet();
        assert!(audio.len() / FEED > 2, "a backlog of several windows");
        let done = consumed(
            |state| Scripted {
                fed: Some(fed),
                gate: Some(gate),
                ..streaming(state, "words")
            },
            &audio,
            |state, commands| {
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                feeding
                    .recv_timeout(Duration::from_secs(5))
                    .expect("the first window is fed");
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
                drop(opens);
            },
        );
        let feeds = done
            .calls
            .iter()
            .filter(|call| matches!(call, Call::Feed(_)))
            .count();
        assert_eq!(feeds, 1, "{:?}", done.calls);
        assert_eq!(done.typed, ["words"]);
    }

    // Capture opens for an answer while the arm cue plays, so the listen can
    // begin on a capture no device has filled yet.
    #[test]
    fn a_listen_that_begins_before_capture_opens_waits_for_its_audio() {
        let (cues, _sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let session = state.arm_for_ask().unwrap();
        let mut pipeline = holding(&[0.0], &state, cues, Scripted::new(&state, Ok("")));
        // No resampler can be built for the rate an unopened capture reads at
        let (_, silent) = ringbuf::HeapRb::<f32>::new(1).split();
        pipeline.source = Arc::new(Capture::new(CaptureSource {
            consumer: silent,
            sample_rate: 0,
        }));

        let heard = pipeline.listen_for_answer(Duration::from_millis(300), session);

        assert!(matches!(heard, Ok(Heard::Silence)), "{heard:?}");
    }

    #[test]
    fn the_answer_reader_builds_its_resampler_for_the_first_audio() {
        let (_, silent) = ringbuf::HeapRb::<f32>::new(1).split();
        let capture = Capture::new(CaptureSource {
            consumer: silent,
            sample_rate: 0,
        });
        let mut reader = Resampled::new(&capture);
        let mut audio = Vec::new();
        assert_eq!(reader.read(&capture, &mut audio), Ok(false));

        let (mut producer, consumer) = ringbuf::HeapRb::<f32>::new(4800).split();
        producer.push_slice(&[0.25; 4800]);
        capture.swap(CaptureSource {
            consumer,
            sample_rate: 48000,
        });
        assert_eq!(
            reader.read(&capture, &mut audio),
            Ok(false),
            "capture opened under a reader that had heard nothing"
        );
        let (mut producer, consumer) = ringbuf::HeapRb::<f32>::new(160).split();
        producer.push_slice(&[0.25; 160]);
        capture.swap(CaptureSource {
            consumer,
            sample_rate: 48000,
        });
        assert_eq!(
            reader.read(&capture, &mut audio),
            Ok(true),
            "a new ring under audio already read is another device"
        );
        reader.finish(&mut audio).unwrap();

        assert!(
            audio.len() < 4800 / 2,
            "48 kHz read at 16 kHz keeps about a third: {}",
            audio.len()
        );
    }

    const BUILT_IN: &str = "MacBook Pro Microphone";
    const WAIT: Duration = Duration::from_secs(5);

    /// A question whose answer is listened for on its own thread, over a
    /// bench, with the thread that owns capture running.
    struct Asking {
        state: Arc<DaemonState>,
        bench: crate::test_support::Bench,
        owner: crate::audio::watchdog::Handle,
        told: mpsc::Receiver<crate::test_support::BenchCall>,
        signals: tokio::sync::broadcast::Receiver<Signal>,
        sounds: mpsc::Receiver<crate::audio::cues::Cue>,
        session: u64,
        answer: thread::JoinHandle<Result<String, String>>,
    }

    impl Asking {
        fn start(bench: crate::test_support::Bench, timeout: Duration) -> Self {
            let (cues, sounds) = Cues::recording();
            let signals = cues.subscribe_signals();
            let state = crate::test_support::daemon_state_with_cues(cues.clone());
            let told = bench.tell();
            let mut pipeline = holding(&[0.0], &state, cues, Scripted::new(&state, Ok("")));
            let owner = crate::audio::watchdog::spawn(
                Arc::clone(&state),
                Arc::clone(&pipeline.source),
                bench.clone(),
            );
            assert_eq!(told.recv_timeout(WAIT), Ok(select()), "the seed");
            let session = state.arm_for_ask().unwrap();
            let answer = thread::spawn(move || {
                let (reply, answer) = tokio::sync::oneshot::channel();
                pipeline.ask(AskCommand {
                    reply,
                    timeout,
                    session,
                });
                answer.blocking_recv().expect("every ask replies")
            });
            Self {
                state,
                bench,
                owner,
                told,
                signals,
                sounds,
                session,
                answer,
            }
        }

        fn opened(&self) {
            assert_eq!(self.told.recv_timeout(WAIT), Ok(select()));
            assert_eq!(
                self.told.recv_timeout(WAIT),
                Ok(crate::test_support::BenchCall::Open(BUILT_IN.to_string()))
            );
        }

        /// The reply, and every signal sent until it came.
        fn end(mut self) -> (Result<String, String>, Vec<Signal>) {
            let answered = self.answer.join().unwrap();
            let signals = std::iter::from_fn(|| self.signals.try_recv().ok()).collect();
            self.owner.stop();
            (answered, signals)
        }
    }

    fn select() -> crate::test_support::BenchCall {
        crate::test_support::BenchCall::Select("default".to_string())
    }

    fn arms(signals: &[Signal]) -> usize {
        signals
            .iter()
            .filter(|signal| matches!(signal, Signal::Arm))
            .count()
    }

    #[test]
    fn an_answer_closed_while_capture_opens_closes_capture() {
        let asking = Asking::start(
            crate::test_support::Bench::with(&[BUILT_IN], Some(BUILT_IN)),
            WAIT,
        );
        asking.opened();

        // A client that goes away ends the session from outside
        asking.state.disarm(asking.session);

        assert_eq!(
            asking.told.recv_timeout(WAIT),
            Ok(crate::test_support::BenchCall::Close(BUILT_IN.to_string()))
        );
        let (answered, signals) = asking.end();
        assert_eq!(answered, Ok(String::new()));
        assert_eq!(arms(&signals), 0, "no sound came: {signals:?}");
        assert!(signals.contains(&Signal::Disarm), "{signals:?}");
    }

    #[test]
    fn the_first_sound_plays_the_arm_cue() {
        let asking = Asking::start(
            crate::test_support::Bench::with(&[BUILT_IN], Some(BUILT_IN)),
            WAIT,
        );
        asking.opened();
        asking.bench.play(&[0.0; 1600]);
        asking.bench.play(&[0.25; 160]);
        assert_eq!(
            asking.sounds.recv_timeout(WAIT),
            Ok(crate::audio::cues::Cue::Arm)
        );
        asking.state.disarm(asking.session);

        let (answered, signals) = asking.end();
        assert_eq!(answered, Ok(String::new()));
        assert_eq!(arms(&signals), 1, "{signals:?}");
    }

    // What capture held when the cue played, and what came while it left the
    // speaker, is the question's tail and the cue's echo, never the answer.
    #[test]
    fn what_capture_holds_once_the_arm_cue_has_played_is_dropped() {
        let (cues, sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let session = state.arm_for_ask().unwrap();
        assert!(state.hear_answer(session));
        state.heard_sound();
        state.announce_sound();
        assert_eq!(sounds.try_recv().ok(), Some(crate::audio::cues::Cue::Arm));
        let mut pipeline = holding(
            &spoken_then_quiet(),
            &state,
            cues,
            Scripted::new(&state, Ok("")),
        );

        let waited = pipeline.await_arm_cue(Instant::now() + WAIT, session);

        assert!(matches!(waited, Ok(None)), "{waited:?}");
        assert!(
            pipeline.source.drain().is_empty(),
            "the listen starts empty"
        );
    }

    #[test]
    fn an_answer_waits_for_its_arm_cue_until_the_deadline() {
        let (cues, _sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let session = state.arm_for_ask().unwrap();
        assert!(state.hear_answer(session));
        let mut pipeline = holding(&[0.0], &state, cues, Scripted::new(&state, Ok("")));

        let waited = pipeline.await_arm_cue(Instant::now() + Duration::from_millis(100), session);

        assert!(matches!(waited, Ok(Some(Heard::Silence))), "{waited:?}");
    }

    #[test]
    fn the_listen_has_what_the_wait_for_the_arm_cue_left_of_the_timeout() {
        let timeout = Duration::from_secs(5);
        assert_eq!(
            listen_budget(timeout, Duration::from_secs(2)),
            Duration::from_secs(3)
        );
        assert_eq!(listen_budget(timeout, Duration::ZERO), timeout);
        assert_eq!(
            listen_budget(timeout, Duration::from_secs(6)),
            Duration::ZERO,
            "a wait that took it all leaves nothing"
        );
    }

    #[test]
    fn a_microphone_that_delivers_only_silence_plays_no_arm_cue() {
        let asking = Asking::start(
            crate::test_support::Bench::with(&[BUILT_IN], Some(BUILT_IN)),
            Duration::from_millis(300),
        );
        asking.opened();
        asking.bench.play(&[0.0; 1600]);

        let (answered, signals) = asking.end();
        assert_eq!(answered, Ok(String::new()));
        assert_eq!(arms(&signals), 0, "{signals:?}");
        assert!(
            signals
                .iter()
                .any(failed(ReasonCode::Silence, Target::Answer)),
            "the answer deadline ends the wait: {signals:?}"
        );
    }

    #[test]
    fn a_hold_while_capture_opens_plays_only_its_own_cue() {
        let asking = Asking::start(
            crate::test_support::Bench::with(&[BUILT_IN], Some(BUILT_IN)),
            WAIT,
        );
        asking.opened();
        assert!(asking.state.record_start(), "the hold answers");
        asking.bench.play(&[0.25; 160]);
        assert_eq!(
            asking.sounds.recv_timeout(WAIT),
            Ok(crate::audio::cues::Cue::RecordStart)
        );
        asking.state.disarm(asking.session);

        let (_, signals) = asking.end();
        assert_eq!(arms(&signals), 0, "{signals:?}");
        assert_eq!(
            signals
                .iter()
                .filter(|signal| matches!(signal, Signal::RecordStart { .. }))
                .collect::<Vec<_>>(),
            [&Signal::RecordStart {
                target: Target::Answer
            }]
        );
    }

    #[test]
    fn an_answer_whose_microphone_will_not_open_fails_with_its_name() {
        let bench = crate::test_support::Bench::with(&[BUILT_IN], Some(BUILT_IN));
        bench.refuse(Some(BUILT_IN));
        let asking = Asking::start(bench, WAIT);
        let state = Arc::clone(&asking.state);

        let (answered, signals) = asking.end();

        assert!(
            matches!(&answered, Err(why) if why.contains(BUILT_IN)),
            "{answered:?}"
        );
        assert!(
            signals
                .iter()
                .any(failed(ReasonCode::ListenFailed, Target::Answer)),
            "the listen says it failed: {signals:?}"
        );
        assert_eq!(arms(&signals), 0, "{signals:?}");
        assert!(!state.wants_mic());
    }

    #[test]
    fn a_command_that_arrives_mid_hold_waits_for_a_model_that_does_not_stream() {
        let done = consumed(
            |state| Scripted::new(state, Ok("words")),
            &spoken_then_quiet(),
            |state, commands| {
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                commands
                    .send(ConsumerCommand::Reload(
                        banshee_common::listening::find(banshee_common::listening::DEFAULT_MODEL)
                            .unwrap(),
                    ))
                    .unwrap();
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
            },
        );
        assert_eq!(done.typed, ["words"]);
        assert_eq!(done.calls, [Call::Transcribe, Call::Reload]);
    }

    #[test]
    fn the_resampler_is_built_for_the_rate_the_first_audio_has() {
        let (cues, _sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let _viewer = state.subscribe_words();
        let (fed, feeding) = mpsc::channel();
        let scripted = Scripted {
            fed: Some(fed),
            ..streaming(&state, "words")
        };
        let calls = Arc::clone(&scripted.calls);
        // No resampler can be built for the rate the press finds
        let mut pipeline = holding(&[0.0], &state, cues, scripted);
        let (_, silent) = ringbuf::HeapRb::<f32>::new(1).split();
        pipeline.source = Arc::new(Capture::new(CaptureSource {
            consumer: silent,
            sample_rate: 0,
        }));
        let capture = Arc::clone(&pipeline.source);
        let (sender, receiver) = mpsc::channel();
        let consumer = thread::spawn(move || consume(pipeline, &receiver, &|_: &str| Ok(())));
        sender.send(ConsumerCommand::Stream(0)).unwrap();
        thread::sleep(ARMED_POLL * 4);
        let audio = spoken_then_quiet();
        let (mut producer, consumer_end) = ringbuf::HeapRb::<f32>::new(audio.len()).split();
        producer.push_slice(&audio);
        capture.swap(CaptureSource {
            consumer: consumer_end,
            sample_rate: SAMPLE_RATE,
        });
        let heard = feeding.recv_timeout(Duration::from_secs(5)).is_ok();
        sender
            .send(ConsumerCommand::Transcribe(state.claim_ring()))
            .unwrap();
        sender.send(ConsumerCommand::Shutdown).unwrap();
        consumer.join().unwrap();
        assert!(
            heard,
            "the stream heard the audio: {:?}",
            calls.lock().unwrap()
        );
        assert_eq!(calls.lock().unwrap().first(), Some(&Call::Begin));
    }

    /// A daemon whose pipeline waits for Parakeet, given its file, with the
    /// calls the engine took and the pipeline it ends with.
    fn arrived(reload_fails: bool) -> (Vec<Call>, crate::state::Pipeline) {
        let (commands, receiver) = mpsc::channel();
        let state = crate::test_support::daemon_state(commands);
        let parakeet =
            banshee_common::listening::find(banshee_common::listening::DEFAULT_MODEL).unwrap();
        state.set_listening(crate::state::Listening::Missing(parakeet));
        assert_eq!(
            state.pipeline(),
            crate::state::Pipeline::Broken(crate::state::RecordingError::NoListeningModel(
                parakeet.id
            ))
        );
        let scripted = Scripted {
            reload_fails,
            ..Scripted::new(&state, Ok(""))
        };
        let calls = Arc::clone(&scripted.calls);
        let pipeline = holding(&[0.0], &state, Cues::recording().0, scripted);
        let consumer = thread::spawn(move || consume(pipeline, &receiver, &|_: &str| Ok(())));

        std::fs::write(state.models_dir().join(parakeet.file), b"").unwrap();
        crate::settings::reapply_pending(&state);
        state.commands().send(ConsumerCommand::Shutdown).unwrap();
        consumer.join().unwrap();
        let calls = calls.lock().unwrap().clone();
        (calls, state.pipeline())
    }

    #[test]
    fn a_listening_model_that_arrives_opens_the_pipeline_with_no_restart() {
        let (calls, pipeline) = arrived(false);
        assert_eq!(calls, [Call::Reload]);
        assert_eq!(pipeline, crate::state::Pipeline::Open);
    }

    #[test]
    fn a_listening_model_that_arrives_and_will_not_load_says_so() {
        let (calls, pipeline) = arrived(true);
        assert_eq!(calls, [Call::Reload]);
        let Some(crate::state::RecordingError::Model(why)) = pipeline.fault() else {
            panic!("the file is here, so the fault is the load: {pipeline:?}");
        };
        assert!(why.contains("the file is damaged"), "{why}");
    }

    fn parakeet() -> &'static ListeningModel {
        banshee_common::listening::find(banshee_common::listening::DEFAULT_MODEL).unwrap()
    }

    const LIMIT: Duration = Duration::from_secs(300);

    #[test]
    fn the_models_unload_once_they_idle_past_the_limit_and_not_before() {
        let state = crate::test_support::daemon_state_with_cues(Cues::silent());
        let scripted = Scripted::new(&state, Ok(""));
        let calls = Arc::clone(&scripted.calls);
        let mut pipeline = holding(&[0.0], &state, Cues::silent(), scripted);
        state.set_unload_after(Some(LIMIT));
        let used = pipeline.used;
        let returns = crate::models::returns_on_this_thread();

        let early = used + LIMIT - Duration::from_secs(1);
        assert_eq!(pipeline.unload_idle(early), Some(used + LIMIT));
        assert!(calls.lock().unwrap().is_empty(), "a second early");
        assert!(pipeline.vad.resident());
        assert_eq!(crate::models::returns_on_this_thread(), returns);

        assert_eq!(pipeline.unload_idle(used + LIMIT), None);
        assert_eq!(*calls.lock().unwrap(), [Call::Unload]);
        assert!(!pipeline.vad.resident(), "the detector goes with it");
        assert_eq!(
            crate::models::returns_on_this_thread(),
            returns + 1,
            "the freed memory goes back to the system"
        );
        assert_eq!(
            state.listening(),
            crate::state::Listening::Available(parakeet())
        );
        assert_eq!(state.pipeline(), crate::state::Pipeline::Open);
    }

    #[test]
    fn the_idle_wait_counts_from_the_last_use() {
        let state = crate::test_support::daemon_state_with_cues(Cues::silent());
        let scripted = Scripted::new(&state, Ok("used"));
        let calls = Arc::clone(&scripted.calls);
        let mut pipeline = holding(&spoken_then_quiet(), &state, Cues::silent(), scripted);
        state.set_unload_after(Some(LIMIT));
        let (_commands, receiver) = mpsc::channel();
        let before = Instant::now();

        let served = pipeline.serve(
            ConsumerCommand::Transcribe(state.claim_ring()),
            &receiver,
            &mut VecDeque::new(),
            &|_: &str| Ok(()),
        );

        assert!(served.is_continue());
        assert!(pipeline.unload_idle(before + LIMIT).is_some());
        assert_eq!(*calls.lock().unwrap(), [Call::Transcribe], "not yet due");
    }

    #[test]
    fn zero_keeps_the_models_loaded() {
        let state = crate::test_support::daemon_state_with_cues(Cues::silent());
        let scripted = Scripted::new(&state, Ok(""));
        let calls = Arc::clone(&scripted.calls);
        let mut pipeline = holding(&[0.0], &state, Cues::silent(), scripted);
        state.set_unload_after(None);
        let returns = crate::models::returns_on_this_thread();

        let a_year = Duration::from_secs(365 * 24 * 60 * 60);
        assert_eq!(pipeline.unload_idle(pipeline.used + a_year), None);
        assert!(calls.lock().unwrap().is_empty());
        assert!(pipeline.vad.resident());
        assert_eq!(crate::models::returns_on_this_thread(), returns);
    }

    #[test]
    fn a_use_after_the_models_idled_out_reads_them_again_and_shows_loading_meanwhile() {
        let (cues, _sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let scripted = Scripted::new(&state, Ok("back again"));
        let (calls, during) = (
            Arc::clone(&scripted.calls),
            Arc::clone(&scripted.during_load),
        );
        let mut pipeline = holding(&spoken_then_quiet(), &state, cues, scripted);
        state.set_unload_after(Some(LIMIT));
        pipeline.unload_idle(pipeline.used + LIMIT);

        let mut typed = Vec::new();
        pipeline.transcribe_utterance(state.claim_ring(), |words| {
            typed.push(words.to_string());
            Ok(())
        });

        assert_eq!(typed, ["back again"]);
        assert_eq!(
            *calls.lock().unwrap(),
            [Call::Unload, Call::Load, Call::Transcribe]
        );
        let during = during.lock().unwrap();
        let (flag, listening, live, _) = &during[0];
        assert!(flag, "the loading flag is up while the model reads");
        assert_eq!(*listening, crate::state::Listening::Loading(parakeet()));
        assert_eq!(live["loading_model"], true, "status says so");
        assert_eq!(live["activity"], "busy", "the chip shows it working");
        assert_eq!(live["pipeline"], "open", "a press still queues");
        assert!(!state.is_loading_model());
        assert_eq!(
            state.listening(),
            crate::state::Listening::Loaded(parakeet())
        );
        assert!(pipeline.vad.resident(), "the detector is read again too");
    }

    #[test]
    fn a_model_that_will_not_load_fails_the_use_with_its_cause_and_the_next_use_tries_again() {
        let (cues, _sounds) = Cues::recording();
        let mut signals = cues.subscribe_signals();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        state.listening_model_unloaded();
        let scripted = Scripted {
            resident: false,
            loads_fail: 1,
            ..Scripted::new(&state, Ok("second time"))
        };
        let calls = Arc::clone(&scripted.calls);
        let speech = spoken_then_quiet();
        let (mut pipeline, mut producer) = fillable(speech.len(), &state, cues, scripted);

        producer.push_slice(&speech);
        pipeline.transcribe_utterance(state.claim_ring(), |_| panic!("nothing is typed"));

        assert_eq!(*calls.lock().unwrap(), [Call::Load]);
        let why = state.last_error().expect("the cause is kept for status");
        assert!(why.contains("the file is damaged"), "{why}");
        assert!(
            why.starts_with("the listening model would not load: ")
                && why.ends_with(FAILED_LOAD_FIX),
            "status shows the cause, then the fix: {why}"
        );
        assert_eq!(
            crate::api::live_state(&state)["last_error"].as_str(),
            Some(why.as_str())
        );
        assert!(matches!(
            signals.try_recv(),
            Ok(Signal::Error { reason, target: Some(Target::Dictate) })
                if reason.code == ReasonCode::TranscriptionFailed
        ));
        assert_eq!(
            state.pipeline(),
            crate::state::Pipeline::Open,
            "the next press is not refused"
        );
        assert_eq!(
            state.listening(),
            crate::state::Listening::Available(parakeet())
        );

        producer.push_slice(&speech);
        let mut typed = Vec::new();
        pipeline.transcribe_utterance(state.claim_ring(), |words| {
            typed.push(words.to_string());
            Ok(())
        });
        assert_eq!(typed, ["second time"]);
        assert_eq!(
            *calls.lock().unwrap(),
            [Call::Load, Call::Load, Call::Transcribe]
        );
    }

    #[test]
    fn a_detector_that_will_not_load_fails_the_use_with_its_cause() {
        let (cues, _sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let scripted = Scripted::new(&state, Ok("never typed"));
        let calls = Arc::clone(&scripted.calls);
        let mut pipeline = holding(&spoken_then_quiet(), &state, cues, scripted);
        let model = crate::test_support::unique_scratch("vad").join("silero_vad.onnx");
        std::fs::copy(crate::test_support::silero(), &model).unwrap();
        pipeline.vad = Detector::open(model.clone()).unwrap();
        pipeline.vad.unload();
        std::fs::remove_file(&model).unwrap();

        pipeline.transcribe_utterance(state.claim_ring(), |_| panic!("nothing is typed"));

        assert!(calls.lock().unwrap().is_empty());
        let why = state.last_error().expect("the cause is kept for status");
        assert!(why.contains("silero_vad.onnx"), "{why}");
        assert!(
            why.starts_with("the voice detection model would not load: ")
                && why.ends_with(FAILED_LOAD_FIX),
            "{why}"
        );
        assert!(!state.is_loading_model());
    }

    #[test]
    fn a_dictation_released_before_its_model_fails_is_dropped_without_a_second_try() {
        let mut kept = None;
        let done = consumed_queued(
            |state| {
                kept = Some(Arc::clone(state));
                state.listening_model_unloaded();
                Scripted {
                    resident: false,
                    loads_fail: 2,
                    ..streaming(state, "never typed")
                }
            },
            &spoken_then_quiet(),
            |state, commands| {
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
            },
        );
        assert_eq!(done.calls, [Call::Load], "loaded once, at the press");
        assert!(done.typed.is_empty());
        let state = kept.unwrap();
        assert!(state.last_error().unwrap().contains("the file is damaged"));
        assert!(!state.dictation_in_ring());
    }

    /// A pipeline whose listening model idled out, with what a test reads.
    struct Failing {
        pipeline: Pipeline,
        state: Arc<DaemonState>,
        calls: Arc<Mutex<Vec<Call>>>,
        during: Arc<Mutex<Vec<DuringLoad>>>,
        signals: tokio::sync::broadcast::Receiver<Signal>,
    }

    /// The model fails its next `fails` loads.
    fn failing_loads(fails: usize, audio: &[f32]) -> Failing {
        let (cues, _sounds) = Cues::recording();
        let signals = cues.subscribe_signals();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        state.listening_model_unloaded();
        let scripted = Scripted {
            resident: false,
            loads_fail: fails,
            ..Scripted::new(&state, Ok("heard"))
        };
        let (calls, during) = (
            Arc::clone(&scripted.calls),
            Arc::clone(&scripted.during_load),
        );
        let pipeline = holding(audio, &state, cues, scripted);
        Failing {
            pipeline,
            state,
            calls,
            during,
            signals,
        }
    }

    fn failures(
        signals: &mut tokio::sync::broadcast::Receiver<Signal>,
    ) -> Vec<(ReasonCode, Target)> {
        std::iter::from_fn(|| signals.try_recv().ok())
            .filter_map(|signal| match signal {
                Signal::Error {
                    reason,
                    target: Some(target),
                } => Some((reason.code, target)),
                _ => None,
            })
            .collect()
    }

    fn asked(pipeline: &mut Pipeline, state: &DaemonState) -> Result<String, String> {
        let session = state.arm_for_ask().unwrap();
        let (reply, answer) = tokio::sync::oneshot::channel();
        pipeline.ask(AskCommand {
            reply,
            timeout: Duration::from_millis(100),
            session,
        });
        answer.blocking_recv().expect("every ask replies")
    }

    #[test]
    fn an_answer_whose_model_will_not_load_fails_with_its_cause_and_the_next_retries() {
        let Failing {
            mut pipeline,
            state,
            calls,
            during,
            mut signals,
        } = failing_loads(1, &[0.0]);

        let first = asked(&mut pipeline, &state);

        let why = first.expect_err("the load failed");
        assert!(why.contains("the file is damaged"), "{why}");
        assert_eq!(state.last_error().as_deref(), Some(why.as_str()));
        assert_eq!(
            failures(&mut signals),
            [(ReasonCode::ListenFailed, Target::Answer)]
        );
        assert_eq!(state.pipeline(), crate::state::Pipeline::Open);

        assert_eq!(
            asked(&mut pipeline, &state),
            Ok(String::new()),
            "the retry loads, and the answer then waits out its timeout"
        );
        assert_eq!(*calls.lock().unwrap(), [Call::Load, Call::Load]);
        assert!(
            during.lock().unwrap().iter().all(|(.., wanted)| !wanted),
            "capture opens only once the model is in memory"
        );
    }

    #[test]
    fn a_dictation_whose_model_will_not_load_is_refused_at_its_press() {
        let Failing {
            mut pipeline,
            state,
            calls,
            mut signals,
            ..
        } = failing_loads(1, &spoken_then_quiet());
        assert!(state.record_start());
        // No release ever comes, so only the refusal ends the press
        let (_, receiver) = mpsc::channel();

        pipeline.dictate(state.press(), &receiver, &mut VecDeque::new(), |_| {
            panic!("nothing is typed")
        });

        assert_eq!(*calls.lock().unwrap(), [Call::Load]);
        assert!(state.last_error().unwrap().contains("the file is damaged"));
        assert_eq!(
            failures(&mut signals),
            [(ReasonCode::TranscriptionFailed, Target::Dictate)]
        );
        assert_eq!(
            state.recording_mode(),
            RecordingMode::Idle,
            "the press ended with the refusal"
        );
        assert!(!state.wants_mic());
        state.record_stop();
        assert!(
            !state.dictation_in_ring(),
            "the release finds nothing to stop"
        );
    }

    #[test]
    fn a_dictation_whose_key_is_up_before_its_model_fails_says_so_once() {
        for cancelled in [false, true] {
            let Failing {
                mut pipeline,
                state,
                calls,
                mut signals,
                ..
            } = failing_loads(1, &spoken_then_quiet());
            assert!(state.record_start());
            let (commands, receiver) = mpsc::channel();
            if cancelled {
                state.record_cancel();
                commands.send(ConsumerCommand::Discard).unwrap();
            } else {
                state.record_stop();
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
            }

            pipeline.dictate(state.press(), &receiver, &mut VecDeque::new(), |_| {
                panic!("nothing is typed")
            });

            assert_eq!(*calls.lock().unwrap(), [Call::Load], "{cancelled}");
            assert!(
                state.last_error().unwrap().contains("the file is damaged"),
                "{cancelled}"
            );
            assert_eq!(
                failures(&mut signals),
                [(ReasonCode::TranscriptionFailed, Target::Dictate)],
                "{cancelled}"
            );
            assert!(!state.dictation_in_ring(), "{cancelled}");
        }
    }

    #[test]
    fn a_refusal_of_one_press_never_ends_the_next() {
        let (cues, _sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        state.listening_model_unloaded();
        let (loading_to, loading) = mpsc::channel();
        let (opens, gate) = mpsc::channel();
        let scripted = Scripted {
            resident: false,
            loads_fail: 1,
            loading: Some(loading_to),
            load_gate: Some(gate),
            ..Scripted::new(&state, Ok("the second press"))
        };
        let calls = Arc::clone(&scripted.calls);
        let speech = spoken_then_quiet();
        let (pipeline, mut producer) = fillable(speech.len(), &state, cues, scripted);
        let (commands, receiver) = mpsc::channel();
        assert!(state.record_start());
        commands
            .send(ConsumerCommand::Stream(state.press()))
            .unwrap();
        state.record_stop();
        commands
            .send(ConsumerCommand::Transcribe(state.claim_ring()))
            .unwrap();
        assert!(state.record_start());
        commands
            .send(ConsumerCommand::Stream(state.press()))
            .unwrap();
        let typed = Arc::new(Mutex::new(Vec::new()));
        let typing = Arc::clone(&typed);
        let consumer = thread::spawn(move || {
            consume(pipeline, &receiver, &move |words: &str| {
                typing.lock().unwrap().push(words.to_string());
                Ok(())
            })
        });

        loading
            .recv_timeout(Duration::from_secs(5))
            .expect("the first press loads");
        opens.send(()).unwrap();
        loading
            .recv_timeout(Duration::from_secs(5))
            .expect("the second press loads once the first is refused");
        assert_eq!(
            state.recording_mode(),
            RecordingMode::PushToTalk,
            "the first press's refusal left the second held"
        );
        opens.send(()).unwrap();
        producer.push_slice(&speech);
        state.record_stop();
        commands
            .send(ConsumerCommand::Transcribe(state.claim_ring()))
            .unwrap();
        commands.send(ConsumerCommand::Shutdown).unwrap();
        consumer.join().unwrap();

        assert_eq!(*typed.lock().unwrap(), ["the second press"]);
        assert_eq!(
            *calls.lock().unwrap(),
            [Call::Load, Call::Load, Call::Transcribe]
        );
    }

    #[test]
    fn a_use_that_finds_a_claimed_load_leaves_it_to_its_reload() {
        let Failing {
            mut pipeline,
            state,
            ..
        } = failing_loads(1, &[0.0]);
        state.set_listening(crate::state::Listening::Missing(parakeet()));
        state.claim_listening_model(parakeet(), true);

        assert!(pipeline.ready().is_err());

        assert_eq!(
            state.listening(),
            crate::state::Listening::Loading(parakeet()),
            "the claim ends with its own reload"
        );
    }

    #[test]
    fn a_use_waiting_as_the_models_fall_due_is_served_before_they_unload() {
        let done = consumed_queued(
            |state| {
                state.set_unload_after(Some(Duration::from_nanos(1)));
                Scripted::new(state, Ok("still here"))
            },
            &spoken_then_quiet(),
            |state, commands| {
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
            },
        );
        assert_eq!(done.calls, [Call::Transcribe]);
        assert_eq!(done.typed, ["still here"]);
    }

    #[test]
    fn a_use_after_the_models_unloaded_waits_for_them_to_load() {
        let (cues, _sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        state.set_unload_after(Some(Duration::from_nanos(1)));
        state.listening_model_unloaded();
        let (unloaded_to, unloaded) = mpsc::channel();
        let scripted = Scripted {
            resident: false,
            unloaded: Some(unloaded_to),
            ..Scripted::new(&state, Ok("again"))
        };
        let calls = Arc::clone(&scripted.calls);
        let speech = spoken_then_quiet();
        let (pipeline, mut producer) = fillable(speech.len(), &state, cues, scripted);
        let typed = Arc::new(Mutex::new(Vec::new()));
        let typing = Arc::clone(&typed);
        let (commands, receiver) = mpsc::channel();
        let consumer = thread::spawn(move || {
            consume(pipeline, &receiver, &move |words: &str| {
                typing.lock().unwrap().push(words.to_string());
                Ok(())
            })
        });
        let dictate = |producer: &mut HeapProd<f32>| {
            producer.push_slice(&speech);
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
        };

        dictate(&mut producer);
        unloaded
            .recv_timeout(Duration::from_secs(5))
            .expect("the models idled out after the first use");
        dictate(&mut producer);
        commands.send(ConsumerCommand::Shutdown).unwrap();
        consumer.join().unwrap();

        let calls = calls.lock().unwrap();
        assert!(
            calls.starts_with(&[
                Call::Load,
                Call::Transcribe,
                Call::Unload,
                Call::Load,
                Call::Transcribe
            ]),
            "{calls:?}"
        );
        assert_eq!(*typed.lock().unwrap(), ["again", "again"]);
    }

    #[test]
    fn the_models_never_unload_inside_a_dictation_however_long_it_is_held() {
        let (fed, feeding) = mpsc::channel();
        let done = consumed(
            |state| {
                state.set_unload_after(Some(Duration::from_nanos(1)));
                state.listening_model_unloaded();
                Scripted {
                    resident: false,
                    fed: Some(fed),
                    ..streaming(state, "held long")
                }
            },
            &spoken_then_quiet(),
            |state, commands| {
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                feeding
                    .recv_timeout(Duration::from_secs(5))
                    .expect("the dictation is fed while it is held");
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
            },
        );
        let transcribed = done
            .calls
            .iter()
            .position(|call| *call == Call::Transcribe)
            .expect("the dictation is transcribed");
        assert!(
            !done.calls[..transcribed].contains(&Call::Unload),
            "{:?}",
            done.calls
        );
        assert_eq!(done.typed, ["held long"]);
    }

    #[test]
    fn a_short_dictation_types_the_final_text() {
        let done = consumed_queued(
            |state| streaming(state, "yes"),
            &spoken_then_quiet(),
            |state, commands| {
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
            },
        );
        assert_eq!(
            done.calls,
            [Call::Begin, Call::Cancel(String::new()), Call::Transcribe],
            "no words were committed before the release"
        );
        assert_eq!(done.typed, ["yes"]);
    }

    #[test]
    fn a_model_that_does_not_stream_transcribes_on_release() {
        let done = consumed(
            |state| Scripted::new(state, Ok("typed whole")),
            &spoken_then_quiet(),
            |state, commands| {
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
            },
        );
        assert_eq!(done.typed, ["typed whole"]);
        assert_eq!(done.calls, [Call::Transcribe]);
        assert_eq!(done.words_after, "");
    }

    #[test]
    fn a_second_dictation_after_a_stop_streams_again() {
        for (name, end) in [("a stop", true), ("a cancel", false)] {
            let done = consumed(
                |state| streaming(state, "first"),
                &spoken_then_quiet(),
                |state, commands| {
                    commands.send(ConsumerCommand::Stream(0)).unwrap();
                    let first = if end {
                        ConsumerCommand::Transcribe(state.claim_ring())
                    } else {
                        ConsumerCommand::Discard
                    };
                    commands.send(first).unwrap();
                    // The ring is empty now, so neither of these hears speech
                    for _ in 0..2 {
                        commands.send(ConsumerCommand::Stream(0)).unwrap();
                        commands
                            .send(ConsumerCommand::Transcribe(state.claim_ring()))
                            .unwrap();
                    }
                },
            );
            let streams: Vec<&Call> = done
                .calls
                .iter()
                .filter(|call| matches!(call, Call::Begin | Call::Cancel(_)))
                .collect();
            assert_eq!(streams.len(), 6, "after {name}: {:?}", done.calls);
            for pair in streams.chunks(2) {
                assert!(
                    matches!(pair, [Call::Begin, Call::Cancel(_)]),
                    "after {name}, each stream ends before the next: {:?}",
                    done.calls
                );
            }
            assert_eq!(done.words_after, "", "after {name}");
        }
    }

    #[test]
    fn a_shutdown_mid_hold_ends_the_stream_and_clears_the_words() {
        let done = consumed_queued(
            |state| streaming(state, "unused"),
            &spoken_then_quiet(),
            |state, commands| {
                state.publish_words("stale");
                commands.send(ConsumerCommand::Stream(0)).unwrap();
            },
        );
        assert_eq!(
            done.calls,
            [Call::Begin, Call::Cancel("stale".to_string())],
            "the stream ends before the consumer stops"
        );
        assert_eq!(done.words_after, "");
        assert!(done.typed.is_empty());
    }

    #[test]
    fn a_barge_in_that_closes_a_streaming_answer_ends_its_stream() {
        let (cues, _sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let _viewer = state.subscribe_words();
        let session = state.arm_for_ask().unwrap();
        let scripted = streaming(&state, "");
        let calls = Arc::clone(&scripted.calls);
        let mut pipeline = holding(&[0.0; SAMPLE_RATE as usize], &state, cues, scripted);
        assert!(state.try_transition(RecordingMode::Armed, RecordingMode::ArmedHold));
        let closing = Arc::clone(&state);
        let watching = Arc::clone(&calls);
        let closer = thread::spawn(move || {
            while !watching.lock().unwrap().contains(&Call::Begin) {
                thread::sleep(Duration::from_millis(5));
            }
            closing.disarm(session);
        });
        let (reply, _answer) = tokio::sync::oneshot::channel();
        pipeline.ask(AskCommand {
            reply,
            timeout: Duration::from_secs(10),
            session,
        });
        closer.join().unwrap();
        assert!(
            matches!(
                calls.lock().unwrap().as_slice(),
                [Call::Begin, Call::Cancel(_)]
            ),
            "{:?}",
            calls.lock().unwrap()
        );
        assert_eq!(
            pipeline.open_answer_stream(0),
            Some(0),
            "the next utterance streams again"
        );
    }

    #[test]
    fn a_command_that_arrives_mid_stream_runs_after_it_ends() {
        let done = consumed(
            |state| streaming(state, "words"),
            &spoken_then_quiet(),
            |state, commands| {
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                commands
                    .send(ConsumerCommand::Reload(
                        banshee_common::listening::find(banshee_common::listening::DEFAULT_MODEL)
                            .unwrap(),
                    ))
                    .unwrap();
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
            },
        );
        assert_eq!(
            done.typed,
            ["words"],
            "the reload did not end the dictation"
        );
        assert_eq!(
            done.calls[done.calls.len() - 2..],
            [Call::Transcribe, Call::Reload],
            "{:?}",
            done.calls
        );
    }

    #[test]
    fn a_broken_dictation_leaves_nothing_for_the_next() {
        let (cues, _sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let _viewer = state.subscribe_words();
        let (fed, feeding) = mpsc::channel();
        let scripted = Scripted {
            fed: Some(fed),
            ..streaming(&state, "words")
        };
        let pipeline = holding(&spoken_then_quiet(), &state, cues, scripted);
        let capture = Arc::clone(&pipeline.source);
        let (sender, receiver) = mpsc::channel();
        let consumer = thread::spawn(move || consume(pipeline, &receiver, &|_: &str| Ok(())));
        sender.send(ConsumerCommand::Stream(0)).unwrap();
        feeding
            .recv_timeout(Duration::from_secs(5))
            .expect("the dictation streams");
        // A device no resampler can be built for breaks the dictation
        let (mut producer, consumer_end) =
            ringbuf::HeapRb::<f32>::new(SAMPLE_RATE as usize).split();
        capture.swap(CaptureSource {
            consumer: consumer_end,
            sample_rate: 0,
        });
        thread::sleep(ARMED_POLL * 4);
        producer.push_slice(&[0.5; 4800]);
        thread::sleep(ARMED_POLL * 4);
        sender
            .send(ConsumerCommand::Transcribe(state.claim_ring()))
            .unwrap();
        sender.send(ConsumerCommand::Shutdown).unwrap();
        consumer.join().unwrap();
        assert!(
            state.last_error().is_some(),
            "the broken dictation says why"
        );
        assert!(capture.drain().is_empty(), "the hold's audio is gone");
    }

    fn fed_samples(calls: &[Call]) -> usize {
        calls
            .iter()
            .map(|call| match call {
                Call::Feed(samples) => *samples,
                _ => 0,
            })
            .sum()
    }

    #[test]
    fn an_answer_streams_from_its_onset() {
        let audio = spoken_then_quiet();
        let (whole, _, _, _) = listened_by(&audio, None, false);
        let Ok(Heard::Audio(whole)) = whole else {
            panic!("the answer is heard");
        };
        let (streamed, signals, calls, _) = listened_by(&audio, None, true);
        let Ok(Heard::Audio(streamed)) = streamed else {
            panic!("the answer is heard");
        };
        assert_eq!(
            streamed, whole,
            "the full pass reads the same audio either way"
        );
        assert_eq!(signals, [Signal::Onset]);
        assert_eq!(calls.first(), Some(&Call::Begin));
        assert_eq!(
            fed_samples(&calls),
            whole.len() / FEED * FEED,
            "the stream hears the answer from its onset, preroll included"
        );
    }

    /// What a consumer did while its listener was switched, and what it
    /// ended with.
    struct Switched {
        typed: Vec<String>,
        remote_calls: Vec<Call>,
        local_calls: Vec<Call>,
        last_error: Option<String>,
        route: Route,
        listening: Listening,
    }

    fn local_stt() -> STTConfig {
        STTConfig::default()
    }

    /// Makes config.toml name `stt`, as a write leaves it.
    fn saved(state: &DaemonState, stt: STTConfig) {
        state.set_config(Arc::new(crate::config::Config {
            stt,
            ..Default::default()
        }));
    }

    fn remote_stt() -> STTConfig {
        STTConfig {
            provider: Provider::Remote,
            remote: crate::config::RemoteSttConfig {
                base_url: "https://api.example.test/v1".to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn remote_saying(state: &Arc<DaemonState>) -> Scripted {
        Scripted {
            sends_out: true,
            ..Scripted::new(state, Ok("said there"))
        }
    }

    /// `remote_saying`, whose transcription tells `started` it began and then
    /// waits for `gate`.
    fn remote_held(
        gate: mpsc::Receiver<()>,
        started: mpsc::Sender<()>,
    ) -> impl FnOnce(&Arc<DaemonState>) -> Scripted {
        move |state| Scripted {
            transcribe_gate: Some(gate),
            transcribing: Some(started),
            ..remote_saying(state)
        }
    }

    /// The server turned off, as a write of `stt.provider = "local"` does.
    fn turn_server_off(state: &DaemonState) {
        state.withdraw_listener(Some(local_stt()));
        state.replace_listener(local_stt());
    }

    /// Runs a consumer whose engine in force is `remote`, with a
    /// `build_listener` that answers a local Scripted saying "said here"
    /// (awaiting its file unless `model_here`) for a local config, and a
    /// remote Scripted saying "said there" for a remote one. `drive` sends
    /// the commands and the withdrawals.
    fn switched(
        remote: impl FnOnce(&Arc<DaemonState>) -> Scripted,
        model_here: bool,
        drive: impl FnOnce(&Arc<DaemonState>, &mpsc::Sender<ConsumerCommand>),
    ) -> Switched {
        let local = if model_here {
            LocalModel::Here
        } else {
            LocalModel::Absent
        };
        switched_by(remote_stt(), remote, local, Sent::WhileItRuns, drive)
    }

    /// What `build_listener` answers for a local config.
    #[derive(Clone, Copy, PartialEq)]
    enum LocalModel {
        Here,
        Absent,
        /// The first local build fails, and later ones answer `Here`.
        Unbuildable,
    }

    /// `switched` with a local Scripted saying "said here" in force.
    fn switched_from_local(
        drive: impl FnOnce(&Arc<DaemonState>, &mpsc::Sender<ConsumerCommand>),
    ) -> Switched {
        switched_by(
            local_stt(),
            |state| Scripted::new(state, Ok("said here")),
            LocalModel::Here,
            Sent::WhileItRuns,
            drive,
        )
    }

    fn switched_by(
        in_force: STTConfig,
        engine: impl FnOnce(&Arc<DaemonState>) -> Scripted,
        local: LocalModel,
        sent: Sent,
        drive: impl FnOnce(&Arc<DaemonState>, &mpsc::Sender<ConsumerCommand>),
    ) -> Switched {
        let (cues, _sounds) = Cues::recording();
        let (sender, receiver) = mpsc::channel();
        let state = crate::test_support::daemon_state_hearing(cues.clone(), sender.clone());
        saved(&state, in_force.clone());
        let remote = in_force.provider == Provider::Remote;
        let listening = match in_force.listening_model() {
            Some(model) if !remote => Listening::Available(model),
            _ => Listening::Remote,
        };
        state.install_listener(Route::of(&in_force), listening);
        let engine = engine(&state);
        let (remote_calls, local_calls) = if remote {
            (Arc::clone(&engine.calls), Arc::default())
        } else {
            (Arc::default(), Arc::clone(&engine.calls))
        };
        let mut pipeline = holding(&spoken_then_quiet(), &state, cues, engine);
        let building = Arc::clone(&state);
        let (remote_log, local_log) = (Arc::clone(&remote_calls), Arc::clone(&local_calls));
        let mut first_local = true;
        pipeline.build_listener = Box::new(move |stt: &STTConfig| {
            Ok(match stt.provider {
                Provider::Local
                    if local == LocalModel::Unbuildable && std::mem::take(&mut first_local) =>
                {
                    return Err(RecordingError::Model(
                        "the models folder is gone".to_string(),
                    ));
                }
                Provider::Local => Listener {
                    engine: Box::new(Scripted {
                        calls: Arc::clone(&local_log),
                        ..Scripted::new(&building, Ok("said here"))
                    }),
                    awaiting: (local == LocalModel::Absent).then(|| stt.listening_model().unwrap()),
                    refused: None,
                },
                Provider::Remote => Listener {
                    engine: Box::new(Scripted {
                        calls: Arc::clone(&remote_log),
                        ..remote_saying(&building)
                    }),
                    awaiting: None,
                    refused: crate::speech_to_text::key_refusal(building.credentials_path()),
                },
            })
        });
        let typed = Arc::new(Mutex::new(Vec::new()));
        let typing = Arc::clone(&typed);
        let (go, gate) = mpsc::channel::<()>();
        let consumer = thread::spawn(move || {
            let _ = gate.recv();
            consume(pipeline, &receiver, &move |words: &str| {
                typing.lock().unwrap().push(words.to_string());
                Ok(())
            })
        });
        if sent == Sent::WhileItRuns {
            let _ = go.send(());
        }
        drive(&state, &sender);
        let _ = sender.send(ConsumerCommand::Shutdown);
        let _ = go.send(());
        consumer.join().unwrap();
        let typed = typed.lock().unwrap().clone();
        let remote_calls = remote_calls.lock().unwrap().clone();
        let local_calls = local_calls.lock().unwrap().clone();
        Switched {
            typed,
            remote_calls,
            local_calls,
            last_error: state.last_error(),
            route: state.listener_route(),
            listening: state.listening(),
        }
    }

    #[test]
    fn a_dictation_released_after_the_server_is_turned_off_is_transcribed_here() {
        let done = switched(remote_saying, true, |state, commands| {
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            state.withdraw_listener(Some(local_stt()));
            commands
                .send(ConsumerCommand::Replace(Box::new(local_stt())))
                .unwrap();
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
        });
        assert!(
            !done.remote_calls.contains(&Call::Transcribe),
            "{:?}",
            done.remote_calls
        );
        assert_eq!(done.typed, ["said here"]);
        assert_eq!(done.local_calls, [Call::Transcribe]);
        assert_eq!(done.route, Route::Local);
    }

    #[test]
    fn a_request_in_flight_when_the_server_is_turned_off_stops_and_goes_local() {
        let (_gate_open, gate) = mpsc::channel();
        let (started, starting) = mpsc::channel();
        let done = switched(remote_held(gate, started), true, |state, commands| {
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
            starting.recv_timeout(Duration::from_secs(5)).unwrap();
            state.withdraw_listener(Some(local_stt()));
            commands
                .send(ConsumerCommand::Replace(Box::new(local_stt())))
                .unwrap();
        });
        assert_eq!(
            done.typed,
            ["said here"],
            "typed while the server's gate is still shut"
        );
    }

    #[test]
    fn a_switch_to_a_server_mid_dictation_waits_for_the_dictation() {
        let done = switched_from_local(|state, commands| {
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            saved(state, remote_stt());
            commands
                .send(ConsumerCommand::Replace(Box::new(remote_stt())))
                .unwrap();
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
        });
        assert_eq!(
            done.typed,
            ["said here"],
            "the dictation started local and ends local"
        );
        assert_eq!(
            done.route,
            Route::of(&remote_stt()),
            "the next use goes to the server"
        );
    }

    #[test]
    fn turning_the_server_off_without_a_local_model_sends_nothing() {
        let done = switched(remote_saying, false, |state, commands| {
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            state.withdraw_listener(Some(local_stt()));
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
        });
        assert!(!done.remote_calls.contains(&Call::Transcribe));
        assert!(done.typed.is_empty());
        assert!(
            done.local_calls.is_empty(),
            "nothing loads: {:?}",
            done.local_calls
        );
        assert!(done.last_error.unwrap().contains("is not downloaded"));
    }

    #[test]
    fn on_off_on_during_one_request_types_once_and_ends_on_the_last_route() {
        let (_gate_open, gate) = mpsc::channel();
        let (started, starting) = mpsc::channel();
        let done = switched(remote_held(gate, started), true, |state, commands| {
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
            starting.recv_timeout(Duration::from_secs(5)).unwrap();
            state.withdraw_listener(Some(local_stt()));
            commands
                .send(ConsumerCommand::Replace(Box::new(local_stt())))
                .unwrap();
            commands
                .send(ConsumerCommand::Replace(Box::new(remote_stt())))
                .unwrap();
        });
        assert_eq!(done.typed, ["said here"]);
        assert_eq!(
            done.remote_calls
                .iter()
                .filter(|call| **call == Call::Transcribe)
                .count(),
            1,
            "the blocked send is the only one"
        );
        assert_eq!(done.route, Route::of(&remote_stt()));
    }

    #[test]
    fn a_removed_key_mid_request_transcribes_locally_when_the_model_is_here() {
        let (_gate_open, gate) = mpsc::channel();
        let (started, starting) = mpsc::channel();
        let done = switched(remote_held(gate, started), true, |state, commands| {
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
            starting.recv_timeout(Duration::from_secs(5)).unwrap();
            state.withdraw_listener(None);
        });
        assert_eq!(done.typed, ["said here"]);
        assert!(done.route.is_remote(), "the remote listener stays in force");
    }

    #[test]
    fn a_removed_key_mid_request_with_no_local_model_names_both_causes() {
        let (_gate_open, gate) = mpsc::channel();
        let (started, starting) = mpsc::channel();
        let done = switched(remote_held(gate, started), false, |state, commands| {
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
            starting.recv_timeout(Duration::from_secs(5)).unwrap();
            state.withdraw_listener(None);
        });
        assert!(done.typed.is_empty());
        assert_eq!(
            done.remote_calls
                .iter()
                .filter(|call| **call == Call::Transcribe)
                .count(),
            1,
            "no second send"
        );
        let said = done.last_error.unwrap();
        assert!(
            said.contains("no key for the remote listener") && said.contains("is not downloaded"),
            "{said}"
        );
    }

    #[test]
    fn a_server_chosen_again_after_it_was_turned_off_takes_the_next_dictation() {
        let done = switched(remote_saying, true, |state, commands| {
            turn_server_off(state);
            state.replace_listener(remote_stt());
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
        });
        assert_eq!(
            done.typed,
            ["said there"],
            "the withdrawal ended with the switch it came with"
        );
    }

    fn queued_from_local(
        drive: impl FnOnce(&Arc<DaemonState>, &mpsc::Sender<ConsumerCommand>),
    ) -> Switched {
        switched_by(
            local_stt(),
            |state| Scripted::new(state, Ok("said here")),
            LocalModel::Here,
            Sent::BeforeItStarts,
            drive,
        )
    }

    #[test]
    fn toggles_queued_behind_a_busy_listener_send_nothing_after_the_last_turn_off() {
        let done = queued_from_local(|state, commands| {
            commands
                .send(ConsumerCommand::Replace(Box::new(remote_stt())))
                .unwrap();
            state.withdraw_listener(Some(local_stt()));
            commands
                .send(ConsumerCommand::Replace(Box::new(local_stt())))
                .unwrap();
            commands
                .send(ConsumerCommand::Replace(Box::new(remote_stt())))
                .unwrap();
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
            state.withdraw_listener(Some(local_stt()));
            commands
                .send(ConsumerCommand::Replace(Box::new(local_stt())))
                .unwrap();
        });
        assert!(
            !done.remote_calls.contains(&Call::Transcribe),
            "{:?}",
            done.remote_calls
        );
        assert_eq!(done.typed, ["said here"]);
        assert_eq!(done.route, Route::Local);
    }

    /// The same toggles as the settings writes send them.
    #[test]
    fn toggles_written_behind_a_busy_listener_send_nothing_after_the_last_turn_off() {
        let done = queued_from_local(|state, commands| {
            state.replace_listener(remote_stt());
            turn_server_off(state);
            state.replace_listener(remote_stt());
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
            turn_server_off(state);
        });
        assert!(
            !done.remote_calls.contains(&Call::Transcribe),
            "{:?}",
            done.remote_calls
        );
        assert_eq!(done.typed, ["said here"]);
        assert_eq!(done.route, Route::Local);
    }

    #[test]
    fn a_key_set_again_before_the_use_sends_it_to_the_server() {
        let done = switched(remote_saying, false, |state, commands| {
            state.withdraw_listener(None);
            crate::test_support::write_key(
                state.credentials_path(),
                crate::credentials::RemoteKey::Stt,
                "sk-again",
            );
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
        });
        assert_eq!(done.typed, ["said there"]);
        assert_eq!(done.last_error, None);
        assert_eq!(done.listening, Listening::Remote);
    }

    fn unbuildable(
        drive: impl FnOnce(&Arc<DaemonState>, &mpsc::Sender<ConsumerCommand>),
    ) -> Switched {
        switched_by(
            remote_stt(),
            remote_saying,
            LocalModel::Unbuildable,
            Sent::WhileItRuns,
            drive,
        )
    }

    #[test]
    fn a_switch_that_will_not_build_names_no_server() {
        let done = unbuildable(|state, commands| {
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            state.withdraw_listener(Some(local_stt()));
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
        });
        assert!(!done.remote_calls.contains(&Call::Transcribe));
        assert!(done.typed.is_empty());
        assert_eq!(done.route, Route::Local, "nothing goes to the server");
        assert!(
            matches!(&done.listening, Listening::Failed(_, why) if why.contains("the models folder is gone")),
            "{:?}",
            done.listening
        );
    }

    /// Waits up to five seconds for the first dictation to fail.
    fn failed_once(state: &DaemonState) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.last_error().is_none() {
            assert!(Instant::now() < deadline, "the dictation never failed");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn turning_the_server_on_after_a_switch_that_would_not_build_recovers() {
        let done = unbuildable(|state, commands| {
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            state.withdraw_listener(Some(local_stt()));
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
            failed_once(state);
            state.replace_listener(remote_stt());
        });
        assert_eq!(done.route, Route::of(&remote_stt()));
        assert!(
            matches!(done.listening, Listening::Remote | Listening::Refused(_)),
            "{:?}",
            done.listening
        );
    }

    #[test]
    fn a_turn_off_stops_the_upload_already_under_way() {
        use std::io::Read;
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", server.local_addr().unwrap());
        let (accepted, accepting) = mpsc::channel();
        let (read, reading) = mpsc::channel::<()>();
        let received = thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            accepted.send(()).unwrap();
            let _ = reading.recv();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut total = 0;
            let mut buffer = vec![0; 64 * 1024];
            while let Ok(read) = stream.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                total += read;
            }
            total
        });
        let (cues, _sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        crate::test_support::write_key(
            state.credentials_path(),
            crate::credentials::RemoteKey::Stt,
            "sk-test",
        );
        let stt = STTConfig {
            provider: Provider::Remote,
            remote: crate::config::RemoteSttConfig {
                base_url,
                ..Default::default()
            },
            ..Default::default()
        };
        saved(&state, stt.clone());
        state.install_listener(Route::of(&stt), Listening::Remote);
        let mut pipeline = holding(&[0.0], &state, cues, Scripted::new(&state, Ok("unused")));
        pipeline.speech_to_text =
            crate::speech_to_text::select_on_demand(&stt, state.credentials_path())
                .unwrap()
                .engine;
        let building = Arc::clone(&state);
        pipeline.build_listener = Box::new(move |_: &STTConfig| {
            Ok(Listener {
                engine: Box::new(Scripted::new(&building, Ok("said here"))),
                awaiting: None,
                refused: None,
            })
        });
        let withdrawing = Arc::clone(&state);
        let turn_off = thread::spawn(move || {
            accepting.recv_timeout(Duration::from_secs(5)).unwrap();
            withdrawing.withdraw_listener(Some(local_stt()));
        });
        let said = pipeline.through_server(&vec![0.0; 8_000_000]);
        turn_off.join().unwrap();
        read.send(()).unwrap();
        let received = received.join().unwrap();
        assert_eq!(said.unwrap(), "said here");
        assert!(
            received < 8_000_000,
            "{received} of 16 MB reached the server after the turn-off"
        );
    }

    #[test]
    fn writing_local_again_after_a_switch_that_would_not_build_recovers() {
        let done = unbuildable(|state, commands| {
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            state.withdraw_listener(Some(local_stt()));
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
            failed_once(state);
            commands
                .send(ConsumerCommand::Replace(Box::new(local_stt())))
                .unwrap();
        });
        assert_eq!(done.route, Route::Local);
        assert!(
            matches!(done.listening, Listening::Available(_)),
            "{:?}",
            done.listening
        );
    }

    /// A pipeline over `in_force`, served by direct calls in the order a
    /// busy consumer would serve its queue.
    fn served_in_order(in_force: STTConfig) -> (Pipeline, Arc<DaemonState>, Arc<Mutex<Vec<Call>>>) {
        let (cues, _sounds) = Cues::recording();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        saved(&state, in_force.clone());
        let remote = in_force.provider == Provider::Remote;
        let listening = match in_force.listening_model() {
            Some(model) if !remote => Listening::Available(model),
            _ => Listening::Remote,
        };
        state.install_listener(Route::of(&in_force), listening);
        let engine = if remote {
            remote_saying(&state)
        } else {
            Scripted::new(&state, Ok("said here"))
        };
        let remote_calls = if remote {
            Arc::clone(&engine.calls)
        } else {
            Arc::default()
        };
        let mut pipeline = holding(&[0.0], &state, cues, engine);
        let building = Arc::clone(&state);
        let remote_log = Arc::clone(&remote_calls);
        pipeline.build_listener = Box::new(move |stt: &STTConfig| {
            Ok(Listener {
                engine: Box::new(match stt.provider {
                    Provider::Local => Scripted::new(&building, Ok("said here")),
                    Provider::Remote => Scripted {
                        calls: Arc::clone(&remote_log),
                        ..remote_saying(&building)
                    },
                }),
                awaiting: None,
                refused: None,
            })
        });
        (pipeline, state, remote_calls)
    }

    /// Server on, dictate D1, off, on, dictate D2, off, each a write the
    /// listener serves only after the last of them.
    #[test]
    fn a_dictation_queued_between_two_turn_offs_stays_here() {
        let (mut pipeline, state, remote) = served_in_order(local_stt());
        for stt in [remote_stt(), local_stt(), remote_stt(), local_stt()] {
            if stt.provider == Provider::Local {
                state.withdraw_listener(Some(stt.clone()));
            }
            saved(&state, stt.clone());
            state.replace_listener(stt);
        }
        pipeline.replace(&remote_stt());
        let first = pipeline.transcribe(&[0.0; 160]);
        pipeline.replace(&local_stt());
        pipeline.replace(&remote_stt());
        let second = pipeline.transcribe(&[0.0; 160]);
        pipeline.replace(&local_stt());
        assert_eq!(first.unwrap(), "said here");
        assert_eq!(second.unwrap(), "said here");
        assert!(
            !remote.lock().unwrap().contains(&Call::Transcribe),
            "{:?}",
            remote.lock().unwrap()
        );
        assert_eq!(state.listener_route(), Route::Local);
    }

    /// Server A in force: dictate D1, switch to server B, dictate D2, turn
    /// the server off.
    #[test]
    fn a_server_change_queued_before_a_turn_off_sends_nothing() {
        let other = STTConfig {
            provider: Provider::Remote,
            remote: crate::config::RemoteSttConfig {
                base_url: "https://other.example.test/v1".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let (mut pipeline, state, remote) = served_in_order(remote_stt());
        saved(&state, other.clone());
        state.replace_listener(other.clone());
        state.withdraw_listener(Some(local_stt()));
        saved(&state, local_stt());
        state.replace_listener(local_stt());
        let first = pipeline.transcribe(&[0.0; 160]);
        pipeline.replace(&other);
        let second = pipeline.transcribe(&[0.0; 160]);
        pipeline.replace(&local_stt());
        assert_eq!(first.unwrap(), "said here");
        assert_eq!(second.unwrap(), "said here");
        assert!(
            !remote.lock().unwrap().contains(&Call::Transcribe),
            "{:?}",
            remote.lock().unwrap()
        );
        assert_eq!(state.listener_route(), Route::Local);
    }

    fn other_server() -> STTConfig {
        STTConfig {
            provider: Provider::Remote,
            remote: crate::config::RemoteSttConfig {
                base_url: "https://other.example.test/v1".to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// A dictation on its way to the server in force, with a key on file.
    /// `then` runs once the request has started; the request answers when
    /// the test sends on the gate, or after 5 s.
    fn while_a_request_runs(
        then: impl FnOnce(&Arc<DaemonState>, &mpsc::Sender<ConsumerCommand>, &mpsc::Sender<()>),
    ) -> Switched {
        let (gate_open, gate) = mpsc::channel();
        let (started, starting) = mpsc::channel();
        switched(remote_held(gate, started), true, |state, commands| {
            crate::test_support::write_key(
                state.credentials_path(),
                crate::credentials::RemoteKey::Stt,
                "sk-test",
            );
            commands.send(ConsumerCommand::Stream(0)).unwrap();
            commands
                .send(ConsumerCommand::Transcribe(state.claim_ring()))
                .unwrap();
            starting.recv_timeout(Duration::from_secs(5)).unwrap();
            then(state, commands, &gate_open);
        })
    }

    /// The consumer polls every `ARMED_POLL`, so 200 ms gives it several
    /// looks at the new server before the first one answers.
    #[test]
    fn a_request_in_flight_when_the_server_changes_finishes_there() {
        let done = while_a_request_runs(|state, _commands, gate_open| {
            saved(state, other_server());
            state.replace_listener(other_server());
            thread::sleep(Duration::from_millis(200));
            gate_open.send(()).unwrap();
        });
        assert_eq!(done.typed, ["said there"]);
        assert_eq!(
            done.route,
            Route::of(&other_server()),
            "the next use goes to the new server"
        );
    }

    /// Server A in force with a key on file: a dictation records on A, and
    /// config.toml names server B before its release.
    fn released_after_a_switch(model_here: bool) -> Switched {
        switched(
            |state| Scripted {
                sends_out: true,
                ..Scripted::new(state, Ok("said on a"))
            },
            model_here,
            |state, commands| {
                crate::test_support::write_key(
                    state.credentials_path(),
                    crate::credentials::RemoteKey::Stt,
                    "sk-test",
                );
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                saved(state, other_server());
                state.replace_listener(other_server());
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
            },
        )
    }

    #[test]
    fn a_dictation_released_after_a_switch_to_another_server_goes_there() {
        for model_here in [true, false] {
            let done = released_after_a_switch(model_here);
            assert_eq!(done.typed, ["said there"], "model here: {model_here}");
            assert!(done.local_calls.is_empty(), "{:?}", done.local_calls);
            assert_eq!(done.last_error, None);
            assert_eq!(done.route, Route::of(&other_server()));
        }
    }

    /// Server A, off, server B, all written while one dictation records.
    #[test]
    fn a_dictation_released_after_off_then_another_server_goes_to_that_server() {
        let done = switched(
            |state| Scripted {
                sends_out: true,
                ..Scripted::new(state, Ok("said on a"))
            },
            true,
            |state, commands| {
                crate::test_support::write_key(
                    state.credentials_path(),
                    crate::credentials::RemoteKey::Stt,
                    "sk-test",
                );
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                saved(state, local_stt());
                turn_server_off(state);
                saved(state, other_server());
                state.replace_listener(other_server());
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
            },
        );
        assert_eq!(done.typed, ["said there"]);
        assert!(done.local_calls.is_empty(), "{:?}", done.local_calls);
    }

    /// Server A in force: the key is removed, set again, and config.toml
    /// names server B, all while one dictation records.
    #[test]
    fn a_dictation_released_after_a_key_removal_then_another_server_stays_here() {
        let done = switched(
            |state| Scripted {
                sends_out: true,
                ..Scripted::new(state, Ok("said on a"))
            },
            true,
            |state, commands| {
                let key = |value| {
                    crate::test_support::write_key(
                        state.credentials_path(),
                        crate::credentials::RemoteKey::Stt,
                        value,
                    );
                };
                key("sk-test");
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                key("");
                state.withdraw_listener(None);
                key("sk-again");
                saved(state, other_server());
                state.replace_listener(other_server());
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
            },
        );
        assert_eq!(
            done.typed,
            ["said here"],
            "a removal during the dictation keeps it here"
        );
        assert_eq!(done.local_calls, [Call::Transcribe]);
        assert_eq!(
            done.route,
            Route::of(&other_server()),
            "the next use goes to the new server"
        );
    }

    /// A write saves the config before it withdraws, so the consumer can see
    /// the saved "off" first.
    #[test]
    fn a_request_in_flight_stops_once_the_saved_config_names_no_server() {
        let done = while_a_request_runs(|state, _commands, _gate_open| {
            saved(state, local_stt());
        });
        assert_eq!(
            done.typed,
            ["said here"],
            "typed while the server's gate is still shut"
        );
    }

    #[test]
    fn a_request_in_flight_stops_when_the_new_server_is_turned_off() {
        let done = while_a_request_runs(|state, _commands, _gate_open| {
            saved(state, other_server());
            state.replace_listener(other_server());
            thread::sleep(Duration::from_millis(200));
            saved(state, local_stt());
            turn_server_off(state);
        });
        assert_eq!(
            done.typed,
            ["said here"],
            "typed while the first server's gate is still shut"
        );
        assert_eq!(done.route, Route::Local);
    }

    /// "Off" and server B are written together while a request is on its way
    /// to server A, whose gate stays shut until its five seconds pass. Any
    /// poll that sees the turn-off stops the request, whether B settled it
    /// yet or not.
    #[test]
    fn a_request_in_flight_stops_when_off_and_another_server_land_together() {
        let (_gate_open, gate) = mpsc::channel();
        let (started, starting) = mpsc::channel();
        let done = switched(
            |state| Scripted {
                transcribe_gate: Some(gate),
                transcribing: Some(started),
                sends_out: true,
                ..Scripted::new(state, Ok("said on a"))
            },
            true,
            |state, commands| {
                crate::test_support::write_key(
                    state.credentials_path(),
                    crate::credentials::RemoteKey::Stt,
                    "sk-test",
                );
                commands.send(ConsumerCommand::Stream(0)).unwrap();
                commands
                    .send(ConsumerCommand::Transcribe(state.claim_ring()))
                    .unwrap();
                starting.recv_timeout(Duration::from_secs(5)).unwrap();
                saved(state, local_stt());
                turn_server_off(state);
                saved(state, other_server());
                state.replace_listener(other_server());
            },
        );
        assert!(
            done.typed == ["said there"] || done.typed == ["said here"],
            "{:?}",
            done.typed
        );
        assert_eq!(done.route, Route::of(&other_server()));
    }

    #[test]
    fn a_switch_to_a_server_config_toml_no_longer_names_installs_nothing() {
        let (mut pipeline, state, remote) = served_in_order(local_stt());
        pipeline.replace(&remote_stt());
        assert_eq!(state.listener_route(), Route::Local);
        assert!(remote.lock().unwrap().is_empty(), "nothing was built");
    }

    #[test]
    fn a_switch_names_the_listener_it_installed() {
        let done = switched(remote_saying, false, |state, commands| {
            state.withdraw_listener(Some(local_stt()));
            commands
                .send(ConsumerCommand::Replace(Box::new(local_stt())))
                .unwrap();
        });
        assert!(matches!(done.listening, Listening::Missing(_)));
    }

    #[test]
    fn a_held_answer_streams_from_the_hold() {
        let (heard, _, calls, words) =
            listened_by(&spoken_then_quiet(), Some(Duration::from_millis(200)), true);
        assert!(matches!(heard, Ok(Heard::Audio(_))), "{heard:?}");
        assert_eq!(calls.first(), Some(&Call::Begin));
        assert_eq!(words, "", "the words from before the hold are gone");
    }
}

#[cfg(test)]
mod hint_tests {
    use super::*;
    use crate::binding::hotkey;

    #[test]
    fn the_hint_matches_the_mode_in_effect() {
        let toggle = bound_key_hint(Hotkey::default(), HotkeyMode::Toggle);
        assert!(toggle.contains("again"), "toggle must say to press twice");
        assert!(!toggle.contains("release"), "toggle must not say release");

        let hold = bound_key_hint(Hotkey::default(), HotkeyMode::Hold);
        assert!(hold.contains("release"), "hold must say to release");
        assert!(!hold.contains("again"), "hold must not say to press twice");
    }

    // A non-default key, so a regression to a constant goes red
    #[test]
    fn the_hint_names_the_key_the_listener_matches() {
        let rebound = hotkey("F6").unwrap();
        for mode in [HotkeyMode::Toggle, HotkeyMode::Hold] {
            let hint = bound_key_hint(rebound, mode);
            assert!(hint.contains("F6"), "the bound key must be named: {hint}");
            assert!(
                !hint.contains("RightOption"),
                "the default must not leak in: {hint}"
            );
        }
    }
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    use rdev::EventType;

    fn bound_to_the_default(state: &DaemonState) -> Bound {
        let (hotkey, mode, generation) = state.binding();
        Bound::new(hotkey, mode, generation)
    }

    #[test]
    fn a_press_of_the_new_key_records_and_the_old_key_does_nothing() {
        let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
        let mut bound = bound_to_the_default(&state);
        state.set_binding(crate::binding::hotkey("F6").unwrap(), HotkeyMode::Hold);
        bound.handle(&state, &EventType::KeyPress(rdev::Key::AltGr));
        assert!(!state.is_recording(), "the old key no longer records");
        bound.handle(&state, &EventType::KeyRelease(rdev::Key::AltGr));
        bound.handle(&state, &EventType::KeyPress(rdev::Key::F6));
        assert!(state.is_recording(), "the new key records");
    }

    #[test]
    fn a_rebind_during_a_hold_ends_that_recording_at_its_release() {
        let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
        let mut bound = bound_to_the_default(&state);
        bound.handle(&state, &EventType::KeyPress(rdev::Key::AltGr));
        assert!(state.is_recording());
        state.set_binding(crate::binding::hotkey("F6").unwrap(), HotkeyMode::Hold);
        bound.handle(&state, &EventType::KeyRelease(rdev::Key::AltGr));
        assert!(
            !state.is_recording(),
            "the old key's release still owes its stop"
        );
        bound.handle(&state, &EventType::KeyPress(rdev::Key::F6));
        assert!(
            state.is_recording(),
            "the rebind applied once the hold ended"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::traits::{Producer, Split};

    fn source_holding(
        samples: &[f32],
        sample_rate: u32,
    ) -> (CaptureSource, ringbuf::HeapProd<f32>) {
        let (mut producer, consumer) = ringbuf::HeapRb::<f32>::new(64).split();
        producer.push_slice(samples);
        (
            CaptureSource {
                consumer,
                sample_rate,
            },
            producer,
        )
    }

    // The reader is a question that is already listening. It holds the capture
    // from before the swap, and must read the device that is there now.
    #[test]
    fn a_swap_reaches_a_reader_that_already_holds_the_capture() {
        let (source, _old_producer) = source_holding(&[1.0, 2.0], 16000);
        let capture = Arc::new(Capture::new(source));
        let listening = Arc::clone(&capture);
        assert_eq!(listening.drain(), vec![1.0, 2.0]);
        assert_eq!(listening.sample_rate(), 16000);

        // A headset at 16 kHz gives way to the built in mic at 48 kHz
        let (replacement, _new_producer) = source_holding(&[7.0], 48000);
        capture.swap(replacement);

        assert_eq!(listening.drain(), vec![7.0]);
        assert_eq!(
            listening.sample_rate(),
            48000,
            "a stale rate resamples by the wrong ratio and distorts silently"
        );
    }

    // A reader keeps a partial window and a resampler built for one device.
    // The generation is how it learns that both belong to a device that is gone.
    #[test]
    fn every_swap_moves_the_generation_on() {
        let (source, _old_producer) = source_holding(&[1.0], 16000);
        let capture = Capture::new(source);
        let before = capture.generation();

        let (replacement, _new_producer) = source_holding(&[2.0], 16000);
        capture.swap(replacement);

        assert_ne!(
            capture.generation(),
            before,
            "a swap at the same rate is still another device"
        );
    }

    // A rebind between the two reads would hand one device's audio to the other
    // device's rate, and resample it by the wrong ratio.
    #[test]
    fn the_audio_and_the_rate_it_was_captured_at_come_out_together() {
        let (source, _old_producer) = source_holding(&[1.0, 2.0], 16000);
        let capture = Capture::new(source);

        let mut batch = Vec::new();
        let (rate, device) = capture.take(&mut batch);
        assert_eq!((batch.as_slice(), rate), ([1.0, 2.0].as_slice(), 16000));

        let (replacement, _new_producer) = source_holding(&[7.0], 48000);
        capture.swap(replacement);

        batch.clear();
        let (rate, moved) = capture.take(&mut batch);
        assert_eq!((batch.as_slice(), rate), ([7.0].as_slice(), 48000));
        assert_ne!(moved, device, "the reader is told the device changed");
    }

    #[test]
    fn discarding_a_source_empties_it() {
        let (source, _producer) = source_holding(&[1.0, 2.0, 3.0], 16000);
        let capture = Capture::new(source);
        capture.discard();
        assert!(capture.drain().is_empty());
    }
}

#[cfg(test)]
mod phase_tests {
    use super::{ONSET_CHUNKS, PREROLL_CHUNKS, Phase, advance, onset};
    use crate::speech_to_text::vad::VAD_CHUNK;

    #[test]
    fn speech_shorter_than_the_onset_stays_waiting() {
        let mut phase = Phase::Waiting { speech_run: 0 };
        for chunk in 1..ONSET_CHUNKS {
            phase = advance(phase, true, chunk * VAD_CHUNK);
        }
        assert_eq!(
            phase,
            Phase::Waiting {
                speech_run: ONSET_CHUNKS - 1
            }
        );
    }

    #[test]
    fn a_quiet_chunk_restarts_the_onset_count() {
        let almost = Phase::Waiting {
            speech_run: ONSET_CHUNKS - 1,
        };
        assert_eq!(
            advance(almost, false, 40 * VAD_CHUNK),
            Phase::Waiting { speech_run: 0 }
        );
    }

    #[test]
    fn the_onset_keeps_the_preroll_before_the_run_that_confirmed_it() {
        let processed = 100 * VAD_CHUNK;
        let almost = Phase::Waiting {
            speech_run: ONSET_CHUNKS - 1,
        };
        assert_eq!(
            advance(almost, true, processed),
            Phase::InSpeech {
                silence_run: 0,
                start: processed - (ONSET_CHUNKS + PREROLL_CHUNKS) * VAD_CHUNK,
            }
        );
    }

    #[test]
    fn an_onset_at_the_start_of_the_buffer_keeps_from_its_first_sample() {
        let almost = Phase::Waiting {
            speech_run: ONSET_CHUNKS - 1,
        };
        assert_eq!(
            advance(almost, true, ONSET_CHUNKS * VAD_CHUNK),
            Phase::InSpeech {
                silence_run: 0,
                start: 0
            }
        );
    }

    #[test]
    fn silence_inside_speech_counts_and_a_word_resets_it() {
        let speaking = Phase::InSpeech {
            silence_run: 3,
            start: 512,
        };
        assert_eq!(
            advance(speaking, false, 0),
            Phase::InSpeech {
                silence_run: 4,
                start: 512
            }
        );
        assert_eq!(
            advance(speaking, true, 0),
            Phase::InSpeech {
                silence_run: 0,
                start: 512
            }
        );
    }

    #[test]
    fn only_the_move_from_waiting_into_speech_is_the_onset() {
        let waiting = Phase::Waiting { speech_run: 3 };
        let speaking = Phase::InSpeech {
            silence_run: 0,
            start: 0,
        };
        let held = Phase::Manual { start: 0 };
        assert!(onset(waiting, speaking));
        for (before, after) in [
            (waiting, waiting),
            (speaking, speaking),
            (held, held),
            (waiting, held),
            (speaking, held),
        ] {
            assert!(!onset(before, after), "{before:?} to {after:?}");
        }
    }

    #[test]
    fn a_held_key_ignores_the_detector() {
        let held = Phase::Manual { start: 7 };
        assert_eq!(advance(held, true, 99), held);
        assert_eq!(advance(held, false, 99), held);
    }
}

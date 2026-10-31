use ringbuf::HeapCons;
use ringbuf::traits::Consumer;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rdev::listen;

use crate::audio::cues::{Cues, Reason, ReasonCode, Signal, Target};
use crate::audio::utils::{StreamingResampler, resample_audio};
use crate::binding::{Hotkey, HotkeyAction, HotkeyTracker};
use crate::config::HotkeyMode;
use crate::dictation::type_text;
use crate::speech_to_text::vad::{VAD_CHUNK, VADEngine};
use crate::speech_to_text::{SAMPLE_RATE, Transcriber};
use crate::state::{AskCommand, ConsumerCommand, DaemonState, RecordingMode};
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

/// They change together on a rebind, so they travel together.
pub struct CaptureSource {
    pub consumer: HeapCons<f32>,
    pub sample_rate: u32,
}

impl CaptureSource {
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
}

impl Capture {
    pub fn new(source: CaptureSource) -> Self {
        Self {
            source: Mutex::new(source),
            generation: AtomicU64::new(0),
        }
    }

    /// Plays the new device to everything that reads capture, including a
    /// question that is already listening.
    pub fn swap(&self, source: CaptureSource) {
        let mut held = lock(&self.source);
        *held = source;
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Rises with every swap. A reader holding a resampler or a partial window
    /// built for the old device learns from this that both are stale.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

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

// Everything the audio consumer thread owns
pub struct Pipeline {
    pub source: Arc<Capture>,
    pub speech_to_text: Box<dyn Transcriber>,
    pub vad: VADEngine,
    pub state: Arc<DaemonState>,
    pub cues: Cues,
    pub endpoint_silence_ms: u64,
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
fn afresh(vad: &mut VADEngine, phase: &mut Phase) {
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

fn consume(
    mut pipeline: Pipeline,
    commands: &mpsc::Receiver<ConsumerCommand>,
    type_words: &dyn Fn(&str) -> Result<(), BansheeError>,
) {
    // Commands that arrived while a dictation streamed, in the order they came
    let mut deferred = VecDeque::new();
    while let Some(command) = deferred.pop_front().or_else(|| commands.recv().ok()) {
        let records = matches!(
            command,
            ConsumerCommand::Stream
                | ConsumerCommand::Transcribe(_)
                | ConsumerCommand::Discard
                | ConsumerCommand::Ask(_)
        );
        match command {
            ConsumerCommand::Stream => pipeline.dictate(commands, &mut deferred, type_words),
            ConsumerCommand::Transcribe(ring) => pipeline.transcribe_utterance(ring, type_words),
            ConsumerCommand::Discard => pipeline.discard(),
            ConsumerCommand::Ask(ask) => pipeline.ask(ask),
            ConsumerCommand::Retune(words) => pipeline.speech_to_text.set_vocabulary(&words),
            ConsumerCommand::Speak(speech) => pipeline.speech_to_text.set_speech(speech),
            // The load takes seconds and holds this thread. Nothing is lost:
            // a press queues behind it and the ring still holds the audio.
            ConsumerCommand::Reload(model) => {
                let loading = Raised::on(&pipeline.state, DaemonState::set_loading_model);
                match pipeline.speech_to_text.reload(model) {
                    Ok(loaded) => {
                        pipeline.state.set_stt_model(loaded);
                        pipeline.state.listening_model_loaded();
                    }
                    Err(error) => {
                        log::error!("the listening model did not load: {error}");
                        pipeline.state.listening_model_failed(error.to_string());
                    }
                }
                drop(loading);
            }
            ConsumerCommand::Shutdown => break,
        }
        // A recording's live words end with it, however it ended
        if records {
            pipeline.state.publish_words("");
        }
    }
}

/// Feeds the open stream each whole 160 ms of audio past `fed`, and shows each
/// change to the committed words. `fed` is `None` while no stream is open, and
/// a feed that fails ends the stream and the live words.
fn feed_stream(
    speech_to_text: &mut dyn Transcriber,
    state: &DaemonState,
    audio: &[f32],
    fed: &mut Option<usize>,
) {
    let Some(at) = fed else { return };
    while audio.len() - *at >= FEED {
        match speech_to_text.feed(&audio[*at..*at + FEED]) {
            Ok(words) => {
                if let Some(words) = words {
                    state.publish_words(&words);
                }
                *at += FEED;
            }
            Err(error) => {
                log::warn!("the live words stopped: {}", reason(&error));
                speech_to_text.cancel_stream();
                *fed = None;
                return;
            }
        }
    }
}

/// A held dictation, read off the ring while it is spoken.
struct Dictation {
    device: u64,
    /// Built for the rate the first audio comes at, and again after a move.
    resampler: Option<StreamingResampler>,
    batch: Vec<f32>,
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
        self.batch.clear();
        let (rate, moved) = source.take(&mut self.batch);
        if moved != self.device {
            self.device = moved;
            log::info!("the microphone moved while a dictation was held");
            self.resampler = None;
        }
        if self.batch.is_empty() {
            return;
        }
        let resampler = match self.resampler.take() {
            Some(resampler) => self.resampler.insert(resampler),
            None => match resampler_for(rate) {
                Ok(resampler) => self.resampler.insert(resampler),
                Err(why) => {
                    self.broken = Some(why);
                    return;
                }
            },
        };
        if let Err(e) = resampler.push(&self.batch, &mut self.audio) {
            log::error!("resampling failed: {e}");
            self.broken = Some(reason(&e));
        }
    }

    /// The last partial window, so the full pass hears the clip to its end.
    fn finish(&mut self) {
        let Some(resampler) = self.resampler.as_mut().filter(|_| self.broken.is_none()) else {
            return;
        };
        if let Err(e) = resampler.finish(&mut self.audio) {
            log::error!("resampling failed: {e}");
            self.broken = Some(reason(&e));
        }
    }

    fn feed(&mut self, speech_to_text: &mut dyn Transcriber, state: &DaemonState) {
        if self.broken.is_none() {
            feed_stream(speech_to_text, state, &self.audio, &mut self.fed);
        }
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
pub fn start_global_hotkey(key_state: Arc<DaemonState>, hotkey: Hotkey, hotkey_mode: HotkeyMode) {
    if !listens() {
        #[cfg(all(unix, not(target_os = "macos")))]
        log::info!("Wayland session: {WAYLAND_HOTKEY_HINT}.");
        return;
    }

    thread::spawn(move || {
        let mut tracker = HotkeyTracker::new(hotkey, hotkey_mode);
        // Whether this listener's own start opened the session in flight.
        // Cancel discards audio, so it must never touch a session another
        // caller opened while our start was refused as busy.
        let mut owned = false;
        if let Err(error) = listen(move |event| {
            // The daemon's own paste presses modifier keys on this same
            // event stream; they must not re-enter the tracker
            if key_state.is_typing() {
                return;
            }
            match tracker.on_event(&event.event_type) {
                Some(HotkeyAction::Start) => owned = key_state.record_start(),
                Some(HotkeyAction::Toggle) => owned = key_state.record_toggle(),
                Some(HotkeyAction::Stop) => key_state.record_stop(),
                Some(HotkeyAction::Cancel) if owned => key_state.record_cancel(),
                Some(HotkeyAction::Cancel) | None => {}
            }
        }) {
            // Names the capability that is gone, not just the error type
            log::error!(
                "Global hotkey listener stopped: {error:?}. `banshee record start` \
                 and `banshee record stop` still work."
            );
        }
    });
}

impl Pipeline {
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
                self.resampling_failed(reason(&e));
                return;
            }
        };
        self.deliver(&final_data, type_words);
    }

    fn resampling_failed(&self, why: String) {
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
        commands: &mpsc::Receiver<ConsumerCommand>,
        deferred: &mut VecDeque<ConsumerCommand>,
        type_words: impl FnOnce(&str) -> Result<(), BansheeError>,
    ) {
        let mut live = self.begin_live_words("dictation").then(|| Dictation {
            device: self.source.generation(),
            resampler: None,
            batch: Vec::new(),
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
                    match live {
                        Some(mut dictation) => {
                            dictation.read(&self.source);
                            drop(ring);
                            dictation.finish();
                            self.finish_dictation(dictation, type_words);
                        }
                        None => self.transcribe_utterance(ring, type_words),
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
                dictation.feed(&mut *self.speech_to_text, &self.state);
                thread::sleep(ARMED_POLL);
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
            self.resampling_failed(why);
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
        let transcribed = self.speech_to_text.transcribe(final_data);
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
        self.cues.emit(Signal::Arm);
        // Let the arm cue leave the speaker before the VAD listens
        thread::sleep(CUE_SETTLE);
        self.source.discard();
        self.state.open_answer(ask.session);

        let listened = self.listen_for_answer(ask.timeout, ask.session);

        // Close the mic before the slow transcription; every exit disarms
        self.state.disarm(ask.session);
        self.cues.emit(Signal::Disarm);

        // The live words were the stream's; the answer is one full pass
        self.speech_to_text.cancel_stream();
        let speech_to_text = &mut self.speech_to_text;
        let text = settle_answer(&self.state, &self.cues, listened, |audio| {
            speech_to_text.transcribe(audio)
        });
        let _ = ask.reply.send(text);
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
        // The device this answer started on. The watchdog may put another one
        // under it at any moment, and the rate is not shared between devices.
        let mut device = self.source.generation();
        let mut resampler = resampler_for(self.source.sample_rate())?;
        self.vad.reset_state();
        let vad_threshold = self.state.vad_threshold();
        let endpoint_chunks = (self.endpoint_silence_ms / CHUNK_MS).max(1) as usize;
        let deadline = Instant::now() + timeout;
        let hard_deadline = deadline + MAX_ANSWER;

        let mut audio: Vec<f32> = Vec::new();
        // Reused across polls so the loop allocates nothing in steady state
        let mut batch: Vec<f32> = Vec::new();
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
                        resampler.finish(&mut audio).map_err(|e| reason(&e))?;
                        audio.drain(..start);
                        return Ok(Heard::Audio(audio));
                    }
                }
                // The session was closed from outside
                _ => return Ok(Heard::Closed),
            }

            // Checked before suppression so stuck speech cannot hang the session
            if Instant::now() >= hard_deadline {
                return match phase {
                    Phase::InSpeech { start, .. } | Phase::Manual { start } => {
                        resampler.finish(&mut audio).map_err(|e| reason(&e))?;
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
                // Drop the pre-gap partial window so no spliced frame reaches the VAD
                resampler.reset();
                afresh(&mut self.vad, &mut phase);
            }

            batch.clear();
            let (rate, moved) = self.source.take(&mut batch);

            // The microphone moved under this answer. What was said already is
            // kept: it is resampled and belongs to the answer. What cannot be
            // kept is a resampler built for the old rate, and a window holding
            // the old device's audio.
            if moved != device {
                device = moved;
                log::info!("the microphone moved while a question was listening");
                resampler = resampler_for(rate)?;
                afresh(&mut self.vad, &mut phase);
            }
            if let Err(e) = resampler.push(&batch, &mut audio) {
                log::error!("Resampling failed: {e}");
                return Err(reason(&e));
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
            feed_stream(&mut *self.speech_to_text, &self.state, &audio, &mut fed);

            if let Phase::InSpeech { silence_run, start } = phase
                && silence_run >= endpoint_chunks
            {
                resampler.finish(&mut audio).map_err(|e| reason(&e))?;
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
        /// Every feed fails.
        feed_fails: bool,
        /// Every reload fails.
        reload_fails: bool,
        open: bool,
        calls: Arc<Mutex<Vec<Call>>>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Transcribe,
        Begin,
        Feed(usize),
        /// With the words on show when the stream was cancelled.
        Cancel(String),
        Reload,
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
                feed_fails: false,
                reload_fails: false,
                open: false,
                calls: Arc::default(),
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
            self.answer.clone().map_err(BansheeError::Transcription)
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
            _model: &'static ListeningModel,
        ) -> Result<Option<&'static ListeningModel>, BansheeError> {
            self.note(Call::Reload);
            if self.reload_fails {
                return Err(BansheeError::Other("the file is damaged".to_string()));
            }
            Ok(None)
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
            matches!(requests.try_recv(), Ok(ConsumerCommand::Stream)),
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
        let (mut producer, consumer) = ringbuf::HeapRb::<f32>::new(audio.len()).split();
        producer.push_slice(audio);
        Pipeline {
            source: Arc::new(Capture::new(CaptureSource {
                consumer,
                sample_rate: SAMPLE_RATE,
            })),
            speech_to_text: Box::new(speech_to_text),
            vad: VADEngine::open(&crate::test_support::silero()).unwrap(),
            state: Arc::clone(state),
            cues,
            endpoint_silence_ms: 800,
        }
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
                commands.send(ConsumerCommand::Stream).unwrap();
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
                commands.send(ConsumerCommand::Stream).unwrap();
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
                commands.send(ConsumerCommand::Stream).unwrap();
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

    #[test]
    fn a_command_that_arrives_mid_hold_waits_for_a_model_that_does_not_stream() {
        let done = consumed(
            |state| Scripted::new(state, Ok("words")),
            &spoken_then_quiet(),
            |state, commands| {
                commands.send(ConsumerCommand::Stream).unwrap();
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
        sender.send(ConsumerCommand::Stream).unwrap();
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
        state.await_listening_model(parakeet.id);
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

    #[test]
    fn a_short_dictation_types_the_final_text() {
        let done = consumed_queued(
            |state| streaming(state, "yes"),
            &spoken_then_quiet(),
            |state, commands| {
                commands.send(ConsumerCommand::Stream).unwrap();
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
                commands.send(ConsumerCommand::Stream).unwrap();
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
                    commands.send(ConsumerCommand::Stream).unwrap();
                    let first = if end {
                        ConsumerCommand::Transcribe(state.claim_ring())
                    } else {
                        ConsumerCommand::Discard
                    };
                    commands.send(first).unwrap();
                    // The ring is empty now, so neither of these hears speech
                    for _ in 0..2 {
                        commands.send(ConsumerCommand::Stream).unwrap();
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
                commands.send(ConsumerCommand::Stream).unwrap();
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
                commands.send(ConsumerCommand::Stream).unwrap();
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
        sender.send(ConsumerCommand::Stream).unwrap();
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

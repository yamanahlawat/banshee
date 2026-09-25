use ringbuf::HeapCons;
use ringbuf::traits::Consumer;
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
use crate::state::{AskCommand, ConsumerCommand, DaemonState, RecordingMode, TranscribeTarget};
use crate::text_to_speech::lock;

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
/// Whisper hears the first word start. Durations, because a change to the
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
    // The handle lets shutdown join it, dropping the Whisper context before atexit
    thread::spawn(move || {
        let mut pipeline = pipeline;
        while let Ok(command) = commands.recv() {
            match command {
                ConsumerCommand::Transcribe(action) => {
                    pipeline.transcribe_utterance(action, type_text)
                }
                // A session opened while this command sat in the queue owns
                // the ring now: the discard skips, the cancelled lead-in stays.
                ConsumerCommand::Discard => {
                    if !pipeline.state.is_recording() {
                        pipeline.source.discard();
                    }
                }
                ConsumerCommand::Ask(ask) => pipeline.ask(ask),
                ConsumerCommand::Retune(words) => pipeline.speech_to_text.set_vocabulary(&words),
                ConsumerCommand::Speak(speech) => pipeline.speech_to_text.set_speech(speech),
                // The load takes seconds and holds this thread. Nothing is lost:
                // a press queues behind it and the ring still holds the audio.
                ConsumerCommand::Reload(preset) => {
                    let loading = Raised::on(&pipeline.state, DaemonState::set_loading_model);
                    match pipeline.speech_to_text.reload(preset) {
                        Ok(loaded) => pipeline.state.set_stt_model(loaded),
                        Err(error) => {
                            log::error!("the transcription model did not load: {error}")
                        }
                    }
                    drop(loading);
                }
                ConsumerCommand::Shutdown => break,
            }
        }
    })
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
    format!(
        "{press} The text lands in whatever app has focus.\n\
         Shift + {hotkey} sends it to `banshee listen` instead of typing it."
    )
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
                Some(HotkeyAction::Start(target)) => owned = key_state.record_start(target),
                Some(HotkeyAction::Toggle(target)) => owned = key_state.record_toggle(target),
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
    // Push-to-talk ended: the ring holds the whole utterance
    fn transcribe_utterance(
        &mut self,
        action: TranscribeTarget,
        type_words: impl FnOnce(&str) -> Result<(), banshee_common::error::BansheeError>,
    ) {
        let _transcribing = Raised::on(&self.state, DaemonState::set_transcribing);
        let device = self.state.audio_device();
        let mut audio_data = Vec::new();
        let (rate, _) = self.source.take(&mut audio_data);

        log::debug!("Downsampling audio from {rate} Hz to {SAMPLE_RATE} Hz...");

        let final_data = match resample_audio(&audio_data, rate, SAMPLE_RATE) {
            Ok(data) => data,
            Err(e) => {
                log::error!("resampling failed: {e}");
                self.state.set_last_error(Some(reason(&e)));
                self.cues.emit(Signal::Error {
                    reason: Reason::new(ReasonCode::TranscriptionFailed, None),
                    target: Some(action.into()),
                });
                return;
            }
        };

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
                target: Some(action.into()),
            });
            return;
        }

        if speech_chunks < 2 {
            log::info!("No speech detected in the audio. Skipping transcription.");
            self.cues.emit(Signal::Error {
                reason: Reason::new(ReasonCode::NoSpeech, device.as_deref()),
                target: Some(action.into()),
            });
            return;
        }

        log::debug!("Transcribing...");
        let transcribe_started = Instant::now();
        let transcribed = self.speech_to_text.transcribe(&final_data);
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

                // Whisper can return nothing for noise; skip before it reaches the ring or clipboard
                if transcription.is_empty() {
                    log::info!("Empty transcription. Skipping.");
                    self.cues.emit(Signal::Error {
                        reason: Reason::new(ReasonCode::EmptyTranscript, device.as_deref()),
                        target: Some(action.into()),
                    });
                    return;
                }

                // Ready only after the utterance is actually delivered
                match action {
                    TranscribeTarget::Mailbox => {
                        self.state.push_transcription(transcription);
                        self.cues.emit(Signal::Ready {
                            target: Target::Mailbox,
                        });
                    }
                    TranscribeTarget::Dictate => {
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
                    TranscribeTarget::Tell => {
                        log::debug!("Telling the agent: {transcription}");
                        let config = self.state.config();
                        let cues = self.cues.clone();
                        let state = Arc::clone(&self.state);
                        let words = transcription;
                        // Off the hotkey thread: an agent run takes tens of
                        // seconds, and the key must answer the next press.
                        std::thread::spawn(move || {
                            deliver_tell(&state, &cues, || {
                                crate::tell::run(&words, &config.tell, &|line| log::info!("{line}"))
                            })
                        });
                    }
                }
            }
            Err(error) => {
                log::error!("Transcription failed: {error}");
                self.state.set_last_error(Some(reason(&error)));
                self.cues.emit(Signal::Error {
                    reason: Reason::new(ReasonCode::TranscriptionFailed, None),
                    target: Some(action.into()),
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

        let speech_to_text = &self.speech_to_text;
        let text = settle_answer(&self.state, &self.cues, listened, |audio| {
            speech_to_text.transcribe(audio)
        });
        let _ = ask.reply.send(text);
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

        loop {
            thread::sleep(ARMED_POLL);

            match self.state.armed_mode(session) {
                Some(RecordingMode::ArmedHold) => {
                    if !matches!(phase, Phase::Manual { .. }) {
                        // The hold replaces whatever endpointing had collected
                        phase = Phase::Manual { start: audio.len() };
                    }
                }
                Some(RecordingMode::Armed) => {
                    if let Phase::Manual { start } = phase {
                        // The hotkey release ends the manual answer
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
                }
                phase = next;
            }

            if let Phase::InSpeech { silence_run, start } = phase
                && silence_run >= endpoint_chunks
            {
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
    transcribe: impl FnOnce(&[f32]) -> Result<String, banshee_common::error::BansheeError>,
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
                    log::error!("Transcription failed: {e}");
                    let why = reason(&e);
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

/// Stores an utterance worth keeping. Whisper answers an empty string for
/// noise, and that is not a dictation.
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

/// Runs one agent turn and answers for it. A run that worked sounds no cue:
/// the agent already spoke through Banshee's MCP server.
///
/// The catch is here on purpose. Without it the thread ends with no cue and no
/// error, and the user waits for nothing.
fn deliver_tell(
    state: &DaemonState,
    cues: &Cues,
    run: impl FnOnce() -> Result<crate::tell::Told, banshee_common::error::BansheeError>,
) {
    let _telling = Raised::on(state, |state, on| {
        if on {
            state.telling_started()
        } else {
            state.telling_ended()
        }
    });
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(Ok(told)) => warn_tell(state, cues, &told.warnings),
        Ok(Err(error)) => fail_tell(state, cues, error.to_string()),
        Err(panic) => fail_tell(
            state,
            cues,
            format!("The command stopped on a fault: {}", panic_reason(panic)),
        ),
    }
}

/// What a run that exited 0 still got wrong. The hotkey path has no terminal,
/// so `banshee status` names the warnings instead of the journal. A refused
/// tool also sounds the failure cue: it sounds like a run that worked.
fn warn_tell(state: &DaemonState, cues: &Cues, warnings: &[crate::tell::Warning]) {
    if warnings.is_empty() {
        state.set_tell_error(None);
        cues.emit(Signal::Told);
        return;
    }
    let reason = warnings
        .iter()
        .map(crate::tell::Warning::text)
        .collect::<Vec<_>>()
        .join(" ");
    log::warn!("tell warned: {reason}");
    state.set_tell_error(Some(reason));
    cues.emit(Signal::Error {
        reason: Reason::new(ReasonCode::TellWarned, None),
        target: Some(Target::Tell),
    });
}

fn fail_tell(state: &DaemonState, cues: &Cues, reason: String) {
    log::error!("tell failed: {reason}");
    state.set_tell_error(Some(reason));
    cues.emit(Signal::Error {
        reason: Reason::new(ReasonCode::TellFailed, None),
        target: Some(Target::Tell),
    });
}

/// What a panic carried. `catch_unwind` answers with a boxed payload, and
/// `panic!` builds either a `&str` or a `String`.
fn panic_reason(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|text| (*text).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_string())
}

#[cfg(test)]
mod tell_tests {
    use super::*;
    use crate::audio::cues::Cue;
    use crate::tell::Told;

    #[test]
    fn a_load_that_unwinds_still_clears_the_flag() {
        let (state, _lines) = crate::test_support::daemon_state_recording_speech();
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

    /// Whether the player said nothing. The watcher thread hands a queued line
    /// to the backend, so the wait comes before the answer.
    fn said_nothing(lines: &crate::test_support::SpokenLines) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if !lines.lock().unwrap().is_empty() {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
        true
    }

    #[test]
    fn a_run_that_worked_says_nothing_and_adds_no_cue_of_its_own() {
        let (state, lines) = crate::test_support::daemon_state_recording_speech();
        let (cues, sounded) = Cues::recording();
        let mut signals = cues.subscribe_signals();
        deliver_tell(&state, &cues, || {
            Ok(Told {
                reply: Some("The gap is five.".to_string()),
                warnings: vec![],
            })
        });
        assert!(
            said_nothing(&lines),
            "the agent speaks for itself, and Banshee adds no voice of its own"
        );
        assert!(
            sounded.try_recv().is_err(),
            "the agent has already spoken, so a cue behind it says the same thing twice"
        );
        assert!(
            matches!(signals.try_recv(), Ok(crate::audio::cues::Signal::Told)),
            "a run with no warnings still tells a subscriber it finished"
        );
        assert_eq!(
            state.last_error(),
            None,
            "a clean run leaves status nothing to name"
        );
    }

    // The user hears the failure cue and dictates one message on the way to a
    // terminal. Status is the only place left that holds why the run failed.
    #[test]
    fn a_dictation_that_works_leaves_the_tell_failure_for_status() {
        let (state, _lines) = crate::test_support::daemon_state_recording_speech();
        let (cues, _sounded) = Cues::recording();
        deliver_tell(&state, &cues, || {
            Err(banshee_common::error::BansheeError::Rejected(
                "opencode exited exit status: 1".to_string(),
            ))
        });

        state.set_last_error(None);

        assert_eq!(
            state.last_error(),
            Some("opencode exited exit status: 1".to_string()),
            "the cue has already sounded, so status is all the user has left"
        );
    }

    #[test]
    fn a_run_that_worked_clears_the_last_run_that_failed() {
        let (state, _lines) = crate::test_support::daemon_state_recording_speech();
        let (cues, _sounded) = Cues::recording();
        deliver_tell(&state, &cues, || {
            Err(banshee_common::error::BansheeError::Rejected(
                "opencode exited exit status: 1".to_string(),
            ))
        });

        deliver_tell(&state, &cues, || Ok(Told::default()));

        assert_eq!(
            state.last_error(),
            None,
            "a stale reason sends the user after a failure that is already fixed"
        );
    }

    // "start over" reaches no agent and sounds nothing. The user chose silence
    // here over a cue of its own. A reset that fails is still an error, so it
    // sounds the error cue and `banshee status` names it.
    #[test]
    fn a_command_banshee_answers_itself_sounds_nothing() {
        let (state, lines) = crate::test_support::daemon_state_recording_speech();
        let (cues, sounded) = Cues::recording();
        deliver_tell(&state, &cues, || {
            Ok(Told {
                reply: Some("Thread cleared.".to_string()),
                warnings: vec![],
            })
        });
        assert!(
            sounded.try_recv().is_err(),
            "the user asked for no sound on a reset that worked"
        );
        assert!(said_nothing(&lines));
    }

    #[test]
    fn a_refused_tool_sounds_the_cue_and_keeps_the_warning_for_status() {
        let (state, lines) = crate::test_support::daemon_state_recording_speech();
        let (cues, sounded) = Cues::recording();
        let mut signals = cues.subscribe_signals();
        deliver_tell(&state, &cues, || {
            Ok(Told {
                reply: Some("The gap is five.".to_string()),
                warnings: vec![crate::tell::Warning::DeniedTools(
                    "claude was refused these tools, so it may have worked in silence: \
                     mcp__banshee__speak_status"
                        .to_string(),
                )],
            })
        });
        assert!(
            matches!(sounded.try_recv(), Ok(Cue::Error)),
            "a refused tool sounds exactly like a run that worked, so it needs a cue"
        );
        match signals.try_recv() {
            Ok(crate::audio::cues::Signal::Error { reason, target }) => {
                assert_eq!(reason.code, crate::audio::cues::ReasonCode::TellWarned);
                assert_eq!(target, Some(Target::Tell));
            }
            other => panic!("expected tell_warned, got {other:?}"),
        }
        let reason = state.last_error().expect("the warning must be kept");
        assert!(
            reason.contains("mcp__banshee__speak_status"),
            "status must name the tool: {reason}"
        );
        assert!(said_nothing(&lines));
    }

    #[test]
    fn a_failed_run_sounds_the_cue_and_keeps_the_reason_for_status() {
        let (state, lines) = crate::test_support::daemon_state_recording_speech();
        let (cues, sounded) = Cues::recording();
        let mut signals = cues.subscribe_signals();
        deliver_tell(&state, &cues, || {
            Err(banshee_common::error::BansheeError::Rejected(
                "opencode exited exit status: 1".to_string(),
            ))
        });
        assert!(matches!(sounded.try_recv(), Ok(Cue::Error)));
        match signals.try_recv() {
            Ok(crate::audio::cues::Signal::Error { reason, target }) => {
                assert_eq!(reason.code, crate::audio::cues::ReasonCode::TellFailed);
                assert_eq!(target, Some(Target::Tell));
            }
            other => panic!("expected tell_failed, got {other:?}"),
        }
        assert_eq!(
            state.last_error(),
            Some("opencode exited exit status: 1".to_string()),
            "the cue says that something went wrong, and status says what"
        );
        assert!(
            said_nothing(&lines),
            "the reason is a machine string, and a voice reading it out is unpleasant"
        );
    }

    #[test]
    fn a_run_that_panics_sounds_the_cue_and_keeps_the_fault_for_status() {
        let (state, lines) = crate::test_support::daemon_state_recording_speech();
        let (cues, sounded) = Cues::recording();
        let mut signals = cues.subscribe_signals();
        deliver_tell(&state, &cues, || panic!("attempt to add with overflow"));
        assert!(matches!(sounded.try_recv(), Ok(Cue::Error)));
        match signals.try_recv() {
            Ok(crate::audio::cues::Signal::Error { reason, target }) => {
                assert_eq!(reason.code, crate::audio::cues::ReasonCode::TellFailed);
                assert_eq!(target, Some(Target::Tell));
            }
            other => panic!("expected tell_failed, got {other:?}"),
        }
        let reason = state.last_error().expect("the fault must be kept");
        assert!(
            reason.contains("attempt to add with overflow"),
            "the fault must name itself: {reason}"
        );
        assert!(said_nothing(&lines));
    }

    #[test]
    fn a_run_raises_the_telling_flag_and_lowers_it_on_every_exit() {
        let (state, _lines) = crate::test_support::daemon_state_recording_speech();
        let (cues, _sounded) = Cues::recording();
        assert!(!state.is_telling(), "the flag starts down");

        let mut raised = false;
        deliver_tell(&state, &cues, || {
            raised = state.is_telling();
            Ok(Told {
                reply: None,
                warnings: vec![],
            })
        });
        assert!(raised, "the flag must be up while the agent runs");
        assert!(!state.is_telling(), "a run that worked lowers it");

        deliver_tell(&state, &cues, || {
            Err(banshee_common::error::BansheeError::Rejected(
                "opencode exited exit status: 1".to_string(),
            ))
        });
        assert!(!state.is_telling(), "a run that failed lowers it");

        deliver_tell(&state, &cues, || panic!("attempt to add with overflow"));
        assert!(!state.is_telling(), "a run that panicked lowers it");
    }

    #[test]
    fn a_second_press_that_ends_first_leaves_the_flag_up_for_the_run_still_going() {
        let (state, _lines) = crate::test_support::daemon_state_recording_speech();
        let (cues, _sounded) = Cues::recording();

        let mut still_up = false;
        deliver_tell(&state, &cues, || {
            // The second press takes a delivery of its own, which the run lock
            // rejects while the first still holds it.
            deliver_tell(&state, &cues, || {
                Err(banshee_common::error::BansheeError::Rejected(
                    "a command is already running. Wait for it to finish.".to_string(),
                ))
            });
            still_up = state.is_telling();
            Ok(Told {
                reply: None,
                warnings: vec![],
            })
        });

        assert!(
            still_up,
            "a rejected second press must leave the busy state up for the first run"
        );
        assert!(!state.is_telling(), "the last delivery to end lowers it");
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
    fn an_answer_with_words_is_heard_and_sounds_nothing() {
        let (text, signal, sounds, state) = settled(
            Ok(Heard::Audio(vec![0.0; 16])),
            Ok("yes please".to_string()),
        );
        assert_eq!(text, Ok("yes please".to_string()));
        assert_eq!(signal, Signal::Answered { heard: true });
        assert!(sounds.is_empty(), "answered has no earcon");
        assert!(!state.is_transcribing(), "the flag falls on every exit");
    }

    #[test]
    fn an_answer_whisper_hears_as_nothing_is_not_heard() {
        let (text, signal, _, _) = settled(Ok(Heard::Audio(vec![0.0; 16])), Ok(String::new()));
        assert_eq!(text, Ok(String::new()));
        assert_eq!(signal, Signal::Answered { heard: false });
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
    use crate::config::STTPreset;
    use crate::speech_to_text::Speech;
    use banshee_common::SileroVADConfig;
    use banshee_common::error::BansheeError;
    use ringbuf::traits::{Producer, Split};
    use std::sync::atomic::AtomicBool;

    /// Answers one fixed result, and notes whether the flag was up when asked.
    struct Scripted {
        state: Arc<DaemonState>,
        answer: Result<String, String>,
        saw_the_flag: Arc<AtomicBool>,
    }

    impl Transcriber for Scripted {
        fn transcribe(&self, _audio: &[f32]) -> Result<String, BansheeError> {
            self.saw_the_flag
                .store(self.state.is_transcribing(), Ordering::Relaxed);
            self.answer.clone().map_err(BansheeError::Transcription)
        }
        fn set_vocabulary(&mut self, _words: &[String]) {}
        fn set_speech(&mut self, _speech: Speech) {}
        fn reload(&mut self, _preset: STTPreset) -> Result<Option<&'static str>, BansheeError> {
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
        action: TranscribeTarget,
        type_words: impl FnOnce(&DaemonState, &str) -> Result<(), BansheeError>,
    ) -> Handled {
        let (cues, _sounds) = Cues::recording();
        let mut subscribed = cues.subscribe_signals();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let saw_the_flag = Arc::new(AtomicBool::new(false));
        let scripted = Scripted {
            state: Arc::clone(&state),
            answer: answer.map(str::to_string).map_err(str::to_string),
            saw_the_flag: Arc::clone(&saw_the_flag),
        };
        let mut pipeline = holding(audio, &state, cues, scripted);
        let mut flag = state.subscribe_transcribing();
        flag.mark_unchanged();
        let typing = Arc::clone(&state);
        pipeline.transcribe_utterance(action, move |words| type_words(&typing, words));
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
            vad: VADEngine::new(SileroVADConfig::new(crate::models::VAD_MODEL)).unwrap(),
            state: Arc::clone(state),
            cues,
            endpoint_silence_ms: 800,
        }
    }

    /// Listens once for the answer to an armed question whose capture holds
    /// `audio`, and gives back what it heard with every signal it sent. `held`
    /// holds the hotkey from the start and releases it that long after.
    fn listened(audio: &[f32], held: Option<Duration>) -> (Result<Heard, String>, Vec<Signal>) {
        let (cues, _sounds) = Cues::recording();
        let mut subscribed = cues.subscribe_signals();
        let state = crate::test_support::daemon_state_with_cues(cues.clone());
        let session = state.arm_for_ask().unwrap();
        assert!(state.open_answer(session));
        let scripted = Scripted {
            state: Arc::clone(&state),
            answer: Ok(String::new()),
            saw_the_flag: Arc::new(AtomicBool::new(false)),
        };
        let mut pipeline = holding(audio, &state, cues, scripted);
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
        (heard, signals)
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
    fn no_speech_raises_the_flag_and_lowers_it() {
        let handled = handle(
            &[0.0; SAMPLE_RATE as usize],
            Ok("unused"),
            TranscribeTarget::Mailbox,
            untyped,
        );
        assert!(
            handled
                .signals
                .iter()
                .any(failed(ReasonCode::NoSpeech, Target::Mailbox)),
            "{:?}",
            handled.signals
        );
        assert!(handled.rose, "the flag covers the voice detector too");
        assert!(!handled.state.is_transcribing(), "the exit lowers it");
    }

    #[test]
    fn an_empty_transcript_keeps_the_flag_up_and_lowers_it_on_the_way_out() {
        let handled = handle(
            &crate::speech_to_text::vad::test_speech(),
            Ok(""),
            TranscribeTarget::Dictate,
            untyped,
        );
        assert!(
            handled
                .signals
                .iter()
                .any(failed(ReasonCode::EmptyTranscript, Target::Dictate)),
            "{:?}",
            handled.signals
        );
        assert!(handled.up_while_transcribing);
        assert!(!handled.state.is_transcribing());
    }

    #[test]
    fn a_failed_transcription_lowers_the_flag() {
        let handled = handle(
            &crate::speech_to_text::vad::test_speech(),
            Err("the model is gone"),
            TranscribeTarget::Mailbox,
            untyped,
        );
        assert!(
            handled
                .signals
                .iter()
                .any(failed(ReasonCode::TranscriptionFailed, Target::Mailbox)),
            "{:?}",
            handled.signals
        );
        assert!(handled.up_while_transcribing);
        assert!(!handled.state.is_transcribing());
    }

    #[test]
    fn the_flag_stays_up_while_the_words_are_typed() {
        let mut up_while_typing = false;
        let handled = handle(
            &crate::speech_to_text::vad::test_speech(),
            Ok("hello there"),
            TranscribeTarget::Dictate,
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

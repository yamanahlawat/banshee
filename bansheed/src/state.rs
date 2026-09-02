use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64},
    },
    time::{Duration, Instant},
};

use banshee_common::DownloadProgress;
use tokio::sync::{broadcast, watch};

use crate::audio::cues::{Cue, Cues};
use crate::config::{BargeInMode, Config};
use crate::text_to_speech::SpeechPlayer;

const TRANSCRIPTION_RING_CAPACITY: usize = 16;

// One file emits at most 101 notifications, so a subscriber has to fall a whole
// file behind before the channel starts dropping any of them
const DOWNLOAD_BACKLOG: usize = 128;

// A start with no stop otherwise holds the mic for the life of the daemon.
// The ring only holds RING_SECS, so nothing is lost by capping it there.
pub const MAX_PUSH_TO_TALK: Duration = Duration::from_secs(crate::audio::RING_SECS as u64);

// Capture is treated as dead after this long with no callback. Measured
// healthy gaps are 11 ms to 18 ms, so this keeps 55 to 90 times headroom.
pub const CAPTURE_SILENCE_LIMIT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscribeTarget {
    Mailbox,
    Dictate,
}

pub struct AskCommand {
    pub reply: tokio::sync::oneshot::Sender<String>,
    pub timeout: Duration,
}

pub enum ConsumerCommand {
    Transcribe(TranscribeTarget),
    // The consumer empties the ring, so a cancelled session does not feed
    // the next transcription
    Discard,
    Ask(AskCommand),
    // The listener owns the engine, so both of these reach it the way a rebind
    // does. A push-to-talk leaves that thread idle until the key is released,
    // so either can land while the microphone is open. The ring holds the audio,
    // so the dictation that follows is whole.
    Retune(Vec<String>),
    Speak(crate::speech_to_text::whisper::Speech),
    Reload(&'static str),
    // A new stream opened, so the old ring is dead. The rate comes with it:
    // devices do not share one.
    Rebind {
        consumer: ringbuf::HeapCons<f32>,
        sample_rate: u32,
    },
    Shutdown,
}

// Stored in an AtomicU8: the audio callback reads it and must not lock
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RecordingMode {
    Idle = 0,
    PushToTalk = 1,
    Armed = 2,
    // Armed, but the user is holding the hotkey to answer manually
    ArmedHold = 3,
}

impl RecordingMode {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Idle,
            1 => Self::PushToTalk,
            2 => Self::Armed,
            3 => Self::ArmedHold,
            _ => unreachable!("invalid recording mode {value}"),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TranscriptionEntry {
    pub id: u64,
    pub text: String,
}

struct TranscriptionRing {
    next_id: u64,
    entries: VecDeque<TranscriptionEntry>,
}

/// Why the recording pipeline did not start. A missing mic and a missing model
/// need different fixes, so they stay distinct out to the RPC error code.
#[derive(Clone)]
pub enum RecordingError {
    Microphone(String),
    Model(String),
}

impl std::fmt::Display for RecordingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordingError::Microphone(e) => write!(f, "the microphone would not open: {e}"),
            RecordingError::Model(e) => write!(f, "a model would not load: {e}"),
        }
    }
}

impl RecordingError {
    /// Trimmed at the source: the wrapped error ends in a period and other blocker prose does not.
    /// A model fault drops the engine's words, which name a path and are stale by the time anyone
    /// reads them. Display keeps them for the log.
    pub fn consequence(&self) -> String {
        match self {
            RecordingError::Model(_) => "a model would not load".to_string(),
            RecordingError::Microphone(_) => self.to_string().trim_end_matches('.').to_string(),
        }
    }

    pub fn command(&self) -> Option<&'static str> {
        match self {
            RecordingError::Microphone(_) | RecordingError::Model(_) => Some("banshee start"),
        }
    }

    pub fn fix(&self) -> &'static str {
        match self {
            // The watchdog rescans after a fault, so most microphones recover
            // on their own. A capture that failed at startup has no watchdog.
            RecordingError::Microphone(_) => {
                "connect the microphone, grant it in Privacy & Security, or fix \
                 [audio] input_device. If recording does not recover on its own, \
                 restart: banshee start"
            }
            RecordingError::Model(_) => "restart it: banshee start",
        }
    }
}

/// The right to download, held for as long as one runs.
// Released on drop rather than by hand: a panic in the download would otherwise
// strand the slot for the life of the daemon. A stable `.part` name is what
// makes resume possible, so it cannot be shared by two writers.
pub struct DownloadSlot {
    state: Arc<DaemonState>,
}

impl Drop for DownloadSlot {
    fn drop(&mut self) {
        self.state
            .downloading
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

fn replace_if_new(field: &RwLock<Option<String>>, name: Option<String>) -> bool {
    let mut held = field.write().unwrap();
    if *held == name {
        return false;
    }
    *held = name;
    true
}

pub struct DaemonState {
    version: &'static str,
    // The model the listener has loaded, not the one the file names. The
    // window reads the blockers built from this, so a preset that was asked
    // for and never loaded must not clear them.
    stt_model: RwLock<&'static str>,
    vad_model: &'static str,
    vad_threshold: AtomicU32,
    audio_device: RwLock<Option<String>>,
    // The named device the config asks for while something else records. Never
    // a blocker: a daemon on a substitute records correctly.
    missing_device: RwLock<Option<String>>,
    // What `audio.input_device` asks for. The watchdog reads it and rebuilds
    // capture when it differs from the device it opened.
    wanted_device: Mutex<String>,
    // Follows a live `tts.voice`, so the voice reported is the one now loaded
    tts_voice: RwLock<Option<String>>,
    wanted_downloads: RwLock<Vec<crate::models::download::Download>>,
    // The file as last parsed. `vad_threshold`, `wanted_device` and `barge_in`
    // beside it are live values the file may no longer agree with.
    config: RwLock<Arc<Config>>,
    // A key with no live apply is read once, when the daemon starts, so this is
    // what those keys run for the daemon's whole life. Without it a write cannot
    // tell a change from a change back.
    running_config: Arc<Config>,
    // Keys the daemon accepted and wrote but has not applied. A restart empties
    // it by nature; a live path clears its own key.
    pending: Mutex<std::collections::BTreeSet<String>>,
    // Why recording is off, when it is. The microphone half clears when the
    // watchdog rebinds; the model half still needs a restart.
    recording_error: RwLock<Option<RecordingError>>,
    recording: AtomicU8,
    started_at: Instant,
    db_connection: Mutex<Option<rusqlite::Connection>>,
    transcriptions: Mutex<TranscriptionRing>,
    latest_transcription_id: watch::Sender<u64>,
    recording_active: watch::Sender<bool>,
    transcribing: watch::Sender<bool>,
    // One counter for the whole device picture. It moves only when a setter
    // writes a value that differs, because the watchdog rewrites the same one
    // every rescan and each move wakes the push task of every subscriber.
    device_changes: watch::Sender<u64>,
    downloads: broadcast::Sender<DownloadProgress>,
    downloading: AtomicBool,
    speech: Arc<SpeechPlayer>,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
    cues: Cues,
    barge_in: Mutex<BargeInMode>,
    // Start and stop can be separate RPC calls, so this cannot live on a stack
    pending_dictate: AtomicBool,
    // enigo posts to the same HID stream rdev listens at, so while this is
    // true the hotkey listener drops events: the paste's own modifier presses
    // would otherwise cancel or open sessions.
    typing: AtomicBool,
    // Milliseconds since `started_at` at which an open push-to-talk is stuck.
    // A deadline rather than a start keeps the watchdog to one load and compare.
    push_to_talk_deadline: AtomicU64,
    // Milliseconds since `started_at` at which the capture callback last ran.
    // Zero means it has never run, which reads as stalled.
    capture_tick: AtomicU64,
    shutdown: tokio::sync::Notify,
}

impl DaemonState {
    pub fn new(
        config: Arc<Config>,
        db_connection: Option<rusqlite::Connection>,
        speech: SpeechPlayer,
        commands: std::sync::mpsc::Sender<ConsumerCommand>,
        cues: Cues,
    ) -> Self {
        let wanted_downloads = crate::models::download::wanted(&config);
        Self {
            version: env!("CARGO_PKG_VERSION"),
            stt_model: RwLock::new(config.stt.preset.model_name()),
            vad_model: crate::models::VAD_MODEL,
            vad_threshold: AtomicU32::new(config.stt.vad_threshold.to_bits()),
            audio_device: RwLock::new(None),
            missing_device: RwLock::new(None),
            wanted_device: Mutex::new(config.audio.input_device.clone()),
            tts_voice: RwLock::new(None),
            wanted_downloads: RwLock::new(wanted_downloads),
            barge_in: Mutex::new(config.audio.barge_in),
            running_config: Arc::clone(&config),
            config: RwLock::new(config),
            pending: Mutex::new(std::collections::BTreeSet::new()),
            recording_error: RwLock::new(None),
            recording: AtomicU8::new(RecordingMode::Idle as u8),
            started_at: Instant::now(),
            db_connection: Mutex::new(db_connection),
            transcriptions: Mutex::new(TranscriptionRing {
                next_id: 0,
                entries: VecDeque::with_capacity(TRANSCRIPTION_RING_CAPACITY),
            }),
            latest_transcription_id: watch::channel(0).0,
            recording_active: watch::channel(false).0,
            transcribing: watch::channel(false).0,
            device_changes: watch::channel(0).0,
            downloads: broadcast::channel(DOWNLOAD_BACKLOG).0,
            downloading: AtomicBool::new(false),
            speech: Arc::new(speech),
            commands,
            cues,
            pending_dictate: AtomicBool::new(false),
            typing: AtomicBool::new(false),
            push_to_talk_deadline: AtomicU64::new(0),
            capture_tick: AtomicU64::new(0),
            shutdown: tokio::sync::Notify::new(),
        }
    }

    // False when another session already owns the microphone.
    pub fn record_start(&self, action: TranscribeTarget) -> bool {
        // The hotkey arrives here too, so a deaf daemon answers a press with the
        // error cue. Arming a session nothing can transcribe would be silent.
        if self.recording_error.read().unwrap().is_some() {
            self.cues.send(Cue::Error);
            return false;
        }
        if self.try_transition(RecordingMode::Armed, RecordingMode::ArmedHold) {
            // Manual override of an armed session: hold to answer
            if matches!(self.barge_in(), BargeInMode::Stop) {
                self.speech.stop();
            }
            self.cues.send(Cue::RecordStart);
            true
        } else if self.try_transition(RecordingMode::Idle, RecordingMode::PushToTalk) {
            // Silence the daemon's own voice before the mic opens
            if matches!(self.barge_in(), BargeInMode::Stop) {
                self.speech.stop();
            }
            self.push_to_talk_deadline.store(
                (self.started_at.elapsed() + MAX_PUSH_TO_TALK).as_millis() as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
            self.pending_dictate.store(
                matches!(action, TranscribeTarget::Dictate),
                std::sync::atomic::Ordering::Relaxed,
            );
            self.cues.send(Cue::RecordStart);
            println!("Recording started...");
            true
        } else {
            false
        }
    }

    // A stop with nothing in flight is a no-op, so release keybinds can fire
    // unconditionally.
    pub fn record_stop(&self) {
        if self.try_transition(RecordingMode::PushToTalk, RecordingMode::Idle) {
            println!("Recording stopped");
            self.cues.send(Cue::RecordStop);
            let action = if self
                .pending_dictate
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                TranscribeTarget::Dictate
            } else {
                TranscribeTarget::Mailbox
            };
            let _ = self.commands.send(ConsumerCommand::Transcribe(action));
        } else if self.try_transition(RecordingMode::ArmedHold, RecordingMode::Armed) {
            self.cues.send(Cue::RecordStop);
        }
    }

    // Resolved here, not in the tracker: a tracker-side read races a mode change. True when it
    // starts a session.
    pub fn record_toggle(&self, action: TranscribeTarget) -> bool {
        if matches!(
            self.recording_mode(),
            RecordingMode::PushToTalk | RecordingMode::ArmedHold
        ) {
            self.record_stop();
            false
        } else {
            self.record_start(action)
        }
    }

    // The user pressed a chord through the hotkey, so the session it opened
    // is a mistake: discard the audio rather than route it. No stop cue for
    // push-to-talk: the start cue was already noise, a second cue doubles it.
    pub fn record_cancel(&self) {
        if self.try_transition(RecordingMode::PushToTalk, RecordingMode::Idle) {
            println!("Recording cancelled");
            let _ = self.commands.send(ConsumerCommand::Discard);
        } else if self.try_transition(RecordingMode::ArmedHold, RecordingMode::Armed) {
            // The armed session keeps its audio; only the manual hold ends.
            // This cue answers the start cue the hold played.
            self.cues.send(Cue::RecordStop);
        }
    }

    pub fn set_typing(&self, typing: bool) {
        self.typing
            .store(typing, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_typing(&self) -> bool {
        self.typing.load(std::sync::atomic::Ordering::Relaxed)
    }

    // Releases a session that never got its stop. Stops rather than discards:
    // the ring holds real audio and record_stop is what routes it.
    pub fn expire_stuck_recording(&self) -> bool {
        if self.recording_mode() != RecordingMode::PushToTalk {
            return false;
        }
        let deadline = self
            .push_to_talk_deadline
            .load(std::sync::atomic::Ordering::Relaxed);
        if (self.started_at.elapsed().as_millis() as u64) < deadline {
            return false;
        }
        eprintln!(
            "Push-to-talk ran past {}s with no stop; releasing the microphone.",
            MAX_PUSH_TO_TALK.as_secs()
        );
        self.record_stop();
        true
    }

    pub fn speech(&self) -> &Arc<SpeechPlayer> {
        &self.speech
    }

    pub fn commands(&self) -> &std::sync::mpsc::Sender<ConsumerCommand> {
        &self.commands
    }

    pub fn shutdown(&self) -> &tokio::sync::Notify {
        &self.shutdown
    }

    fn ring(&self) -> std::sync::MutexGuard<'_, TranscriptionRing> {
        self.transcriptions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn push_transcription(&self, text: String) -> u64 {
        let mut ring = self.ring();
        ring.next_id += 1;
        let id = ring.next_id;
        ring.entries.push_back(TranscriptionEntry { id, text });
        if ring.entries.len() > TRANSCRIPTION_RING_CAPACITY {
            ring.entries.pop_front();
        }
        drop(ring);
        self.latest_transcription_id.send_replace(id);
        id
    }

    pub fn transcriptions_since(&self, since_id: u64) -> Vec<TranscriptionEntry> {
        let ring = self.ring();
        ring.entries
            .iter()
            .filter(|entry| entry.id > since_id)
            .cloned()
            .collect()
    }

    pub fn subscribe_transcriptions(&self) -> watch::Receiver<u64> {
        self.latest_transcription_id.subscribe()
    }

    pub fn recording_mode(&self) -> RecordingMode {
        RecordingMode::from_u8(self.recording.load(std::sync::atomic::Ordering::Relaxed))
    }

    pub fn set_recording_mode(&self, mode: RecordingMode) {
        self.recording
            .store(mode as u8, std::sync::atomic::Ordering::Relaxed);
        self.publish_recording();
    }

    // Concurrent transitions race (RPC arm vs hotkey vs session end);
    // compare_exchange makes losing a race a no-op instead of a stuck mode
    pub fn try_transition(&self, from: RecordingMode, to: RecordingMode) -> bool {
        let moved = self
            .recording
            .compare_exchange(
                from as u8,
                to as u8,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok();
        if moved {
            self.publish_recording();
        }
        moved
    }

    // Re-reads the atomic rather than deriving the bool from the mode just
    // written: another thread may have moved on, and subscribers want now.
    fn publish_recording(&self) {
        self.recording_active.send_replace(self.is_recording());
    }

    pub fn subscribe_recording(&self) -> watch::Receiver<bool> {
        self.recording_active.subscribe()
    }

    pub fn set_transcribing(&self, on: bool) {
        self.transcribing.send_replace(on);
    }

    pub fn is_transcribing(&self) -> bool {
        *self.transcribing.borrow()
    }

    pub fn subscribe_transcribing(&self) -> watch::Receiver<bool> {
        self.transcribing.subscribe()
    }

    pub fn subscribe_downloads(&self) -> broadcast::Receiver<DownloadProgress> {
        self.downloads.subscribe()
    }

    pub fn report_download(&self, progress: DownloadProgress) {
        let _ = self.downloads.send(progress);
    }

    /// Takes the download slot, or `None` when one is already running. The
    /// slot is released when the returned value drops.
    pub fn start_downloading(self: &Arc<Self>) -> Option<DownloadSlot> {
        self.downloading
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok()
            .then(|| DownloadSlot {
                state: Arc::clone(self),
            })
    }

    pub fn is_recording(&self) -> bool {
        self.recording_mode() != RecordingMode::Idle
    }

    /// True while the daemon holds the microphone open for a spoken answer.
    pub fn is_armed(&self) -> bool {
        matches!(
            self.recording_mode(),
            RecordingMode::Armed | RecordingMode::ArmedHold
        )
    }

    pub fn uptime(&self) -> std::time::Duration {
        self.started_at.elapsed()
    }

    pub fn version(&self) -> &'static str {
        self.version
    }

    pub fn stt_model(&self) -> &'static str {
        *self
            .stt_model
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// The listener records what it loaded, as the speech backend does.
    pub fn set_stt_model(&self, model: &'static str) {
        *self
            .stt_model
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = model;
    }

    pub fn vad_model(&self) -> &str {
        self.vad_model
    }

    pub fn audio_device(&self) -> Option<String> {
        self.audio_device.read().unwrap().clone()
    }

    /// `None` when the device will not name itself, so status says so rather
    /// than keeping the last name it knew.
    pub fn set_audio_device(&self, name: Option<String>) {
        if !replace_if_new(&self.audio_device, name) {
            return;
        }
        self.publish_device_change();
    }

    pub fn missing_device(&self) -> Option<String> {
        self.missing_device.read().unwrap().clone()
    }

    pub fn set_missing_device(&self, name: Option<String>) {
        if !replace_if_new(&self.missing_device, name) {
            return;
        }
        self.publish_device_change();
    }

    /// The raw setting, not a resolved device. `choose` is the one place that
    /// turns a blank name into the default.
    pub fn wanted_device(&self) -> String {
        self.locked_wanted_device().clone()
    }

    pub fn set_wanted_device(&self, name: String) {
        *self.locked_wanted_device() = name;
    }

    // A device name has no invariant a panic can break, and a panic here kills
    // capture silently, so the value stands
    fn locked_wanted_device(&self) -> std::sync::MutexGuard<'_, String> {
        self.wanted_device
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn publish_device_change(&self) {
        self.device_changes
            .send_modify(|generation| *generation += 1);
    }

    /// Moves when the device a client can see changes. The value is a
    /// generation, not a device: the reader reads the fields themselves.
    pub fn device_changes(&self) -> watch::Receiver<u64> {
        self.device_changes.subscribe()
    }

    pub fn recording_error(&self) -> Option<RecordingError> {
        self.recording_error.read().unwrap().clone()
    }

    pub fn set_recording_error(&self, reason: RecordingError) {
        *self.recording_error.write().unwrap() = Some(reason);
    }

    pub fn clear_recording_error(&self) {
        *self.recording_error.write().unwrap() = None;
    }

    /// Takes the armed-listening lock for `ask_user`. Shares the availability
    /// gate with `record_start`, so no caller can arm a mic that cannot record.
    pub fn arm_for_ask(&self) -> bool {
        self.recording_error.read().unwrap().is_none()
            && self.try_transition(RecordingMode::Idle, RecordingMode::Armed)
    }

    /// The voice the speech backend actually loaded, which `config.toml` may no
    /// longer agree with.
    pub fn tts_voice(&self) -> Option<String> {
        self.tts_voice
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// True when the backend took the change. The voice the window marks as
    /// current comes from the backend, not from the file, so a backend that
    /// speaks in something else is never reported as speaking in this.
    pub fn set_tts(&self, tts: &crate::config::TTSConfig) -> bool {
        let Some(voice) = self.speech.reconfigure(tts) else {
            return false;
        };
        self.set_tts_voice(voice);
        true
    }

    pub fn set_tts_voice(&self, voice: String) {
        *self
            .tts_voice
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(voice);
    }

    /// Every file this daemon's own config needs. Set before the socket
    /// accepts, so a caller never sees it unset.
    pub fn wanted_downloads(&self) -> Vec<crate::models::download::Download> {
        self.wanted_downloads
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// Tests need a list of their own, because what is missing otherwise
    /// depends on which models the machine happens to hold.
    #[cfg(test)]
    pub fn set_wanted_downloads(&self, wanted: Vec<crate::models::download::Download>) {
        *self
            .wanted_downloads
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = wanted;
    }

    pub fn config(&self) -> Arc<Config> {
        Arc::clone(&self.config.read().unwrap())
    }

    /// The models the daemon wants follow the config that names them.
    pub fn set_config(&self, config: Arc<Config>) {
        *self
            .wanted_downloads
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) =
            crate::models::download::wanted(&config);
        *self.config.write().unwrap() = config;
    }

    /// The config the restart-only keys are running.
    pub fn running_config(&self) -> Arc<Config> {
        Arc::clone(&self.running_config)
    }

    pub fn pending(&self) -> Vec<String> {
        self.pending.lock().unwrap().iter().cloned().collect()
    }

    /// One key is either applied or waiting, never both.
    pub fn record_outcome(&self, applied: &[String], restart_required: &[String]) {
        let mut pending = self.pending.lock().unwrap();
        for key in applied {
            pending.remove(key);
        }
        for key in restart_required {
            pending.insert(key.clone());
        }
    }

    // The player reads this as each cue reaches it, so a write between two
    // dictations decides whether the next one is heard.
    pub fn set_cues_enabled(&self, on: bool) {
        self.cues.set_enabled(on);
    }

    #[cfg(test)]
    pub fn cues_enabled(&self) -> bool {
        self.cues.enabled()
    }

    pub fn barge_in(&self) -> BargeInMode {
        *self.locked_barge_in()
    }

    pub fn set_barge_in(&self, mode: BargeInMode) {
        *self.locked_barge_in() = mode;
    }

    // A mode has no invariant a panic can break, and a panic here would stop
    // every later dictation reading it, so the value stands
    fn locked_barge_in(&self) -> std::sync::MutexGuard<'_, BargeInMode> {
        self.barge_in
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub fn set_vocabulary(&self, words: Vec<String>) -> bool {
        self.commands.send(ConsumerCommand::Retune(words)).is_ok()
    }

    /// What language the next utterance is read as, and whether it answers in
    /// English. Whisper reads both per utterance, so no model moves.
    pub fn set_speech(&self, speech: crate::speech_to_text::whisper::Speech) -> bool {
        self.commands.send(ConsumerCommand::Speak(speech)).is_ok()
    }

    pub fn load_stt_model(&self, model: &'static str) -> bool {
        self.commands.send(ConsumerCommand::Reload(model)).is_ok()
    }

    pub fn set_vad_threshold(&self, threshold: f32) {
        self.vad_threshold
            .store(threshold.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    pub fn vad_threshold(&self) -> f32 {
        let bits = self
            .vad_threshold
            .load(std::sync::atomic::Ordering::Relaxed);
        f32::from_bits(bits)
    }

    /// Runs `job` against the history file, or answers `None` when history is
    /// off. The one place that holds the lock, so no caller repeats the two
    /// steps that stand between it and the table.
    pub fn with_history<T>(&self, job: impl FnOnce(&rusqlite::Connection) -> T) -> Option<T> {
        let held = self
            .db_connection
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        held.as_ref().map(job)
    }

    pub fn history_enabled(&self) -> bool {
        self.with_history(|_| ()).is_some()
    }

    /// Opening and closing the file is the whole of `daemon.save_history`, so a
    /// write between two dictations decides whether the next one is kept.
    pub fn set_history(&self, connection: Option<rusqlite::Connection>) {
        *self
            .db_connection
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = connection;
    }

    /// Called from the real time audio thread, so it must not lock or allocate.
    pub fn mark_capture_alive(&self) {
        let millis = self.started_at.elapsed().as_millis() as u64 + 1;
        self.capture_tick
            .store(millis, std::sync::atomic::Ordering::Relaxed);
    }

    /// A device that disappears stops the callback; it does not deliver silence.
    pub fn capture_is_stalled(&self) -> bool {
        let last = self.capture_tick.load(std::sync::atomic::Ordering::Relaxed);
        if last == 0 {
            return true;
        }
        let now = self.started_at.elapsed().as_millis() as u64;
        Duration::from_millis(now.saturating_sub(last)) > CAPTURE_SILENCE_LIMIT
    }
}

#[cfg(test)]
mod tests;

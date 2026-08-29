use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, OnceLock, RwLock,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64},
    },
    time::{Duration, Instant},
};

use banshee_common::DownloadProgress;
use tokio::sync::{broadcast, watch};

use crate::audio::cues::Cue;
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

// Where a finished transcription is delivered
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscribeTarget {
    Mailbox,
    Dictate,
}

pub struct AskCommand {
    pub reply: tokio::sync::oneshot::Sender<String>,
    pub timeout: Duration,
}

// Work items for the audio consumer thread
pub enum ConsumerCommand {
    Transcribe(TranscribeTarget),
    // The consumer empties the ring, so a cancelled session does not feed
    // the next transcription
    Discard,
    Ask(AskCommand),
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
    /// Trimmed at the source, not in each renderer: the wrapped error ends in a
    /// period and the other blocker prose does not.
    pub fn consequence(&self) -> String {
        self.to_string().trim_end_matches('.').to_string()
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
    stt_model: &'static str,
    vad_model: &'static str,
    vad_threshold: AtomicU32,
    audio_device: RwLock<Option<String>>,
    // The named device the config asks for while something else records. Never
    // a blocker: a daemon on a substitute records correctly.
    missing_device: RwLock<Option<String>>,
    // What `audio.input_device` asks for. The watchdog reads it and rebuilds
    // capture when it differs from the device it opened.
    wanted_device: Mutex<String>,
    tts_voice: OnceLock<String>,
    wanted_downloads: OnceLock<Vec<crate::models::download::Download>>,
    // The file as last parsed. `vad_threshold`, `wanted_device` and `barge_in`
    // beside it are live values the file may no longer agree with.
    config: RwLock<Arc<Config>>,
    // Keys the daemon accepted and wrote but has not applied. A restart empties
    // it by nature; a live path clears its own key.
    pending: Mutex<std::collections::BTreeSet<String>>,
    // Why recording is off, when it is. The microphone half clears when the
    // watchdog rebinds; the model half still needs a restart.
    recording_error: RwLock<Option<RecordingError>>,
    recording: AtomicU8,
    started_at: Instant,
    db_connection: Option<Mutex<rusqlite::Connection>>,
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
    cues: std::sync::mpsc::Sender<Cue>,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: &'static str,
        stt_model: &'static str,
        vad_model: &'static str,
        initial_vad_threshold: f32,
        wanted_device: String,
        db_connection: Option<Mutex<rusqlite::Connection>>,
        speech: SpeechPlayer,
        commands: std::sync::mpsc::Sender<ConsumerCommand>,
        cues: std::sync::mpsc::Sender<Cue>,
        barge_in: BargeInMode,
    ) -> Self {
        Self {
            version,
            stt_model,
            vad_model,
            vad_threshold: AtomicU32::new(initial_vad_threshold.to_bits()),
            audio_device: RwLock::new(None),
            missing_device: RwLock::new(None),
            wanted_device: Mutex::new(wanted_device),
            tts_voice: OnceLock::new(),
            wanted_downloads: OnceLock::new(),
            config: RwLock::new(Arc::new(Config::default())),
            pending: Mutex::new(std::collections::BTreeSet::new()),
            recording_error: RwLock::new(None),
            recording: AtomicU8::new(RecordingMode::Idle as u8),
            started_at: Instant::now(),
            db_connection,
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
            barge_in: Mutex::new(barge_in),
            pending_dictate: AtomicBool::new(false),
            typing: AtomicBool::new(false),
            push_to_talk_deadline: AtomicU64::new(0),
            capture_tick: AtomicU64::new(0),
            shutdown: tokio::sync::Notify::new(),
        }
    }

    // Push-to-talk press, shared by the hotkey listener and the record RPC.
    // Returns false when another session already owns the microphone.
    pub fn record_start(&self, action: TranscribeTarget) -> bool {
        // The hotkey arrives here too, so a deaf daemon answers a press with the
        // error cue. Arming a session nothing can transcribe would be silent.
        if self.recording_error.read().unwrap().is_some() {
            let _ = self.cues.send(Cue::Error);
            return false;
        }
        if self.try_transition(RecordingMode::Armed, RecordingMode::ArmedHold) {
            // Manual override of an armed session: hold to answer
            if matches!(self.barge_in(), BargeInMode::Stop) {
                self.speech.stop();
            }
            let _ = self.cues.send(Cue::RecordStart);
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
            let _ = self.cues.send(Cue::RecordStart);
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
            let _ = self.cues.send(Cue::RecordStop);
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
            let _ = self.cues.send(Cue::RecordStop);
        }
    }

    // A toggle press ends the session a press began, or begins one. Resolved
    // here rather than in the tracker: the modes live here, and a tracker-side
    // read races a mode change between the read and the call. Returns true
    // when it starts a session.
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
            let _ = self.cues.send(Cue::RecordStop);
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

    pub fn stt_model(&self) -> &str {
        self.stt_model
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

    /// Why recording is unavailable, or `None` when it works.
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
    pub fn tts_voice(&self) -> Option<&str> {
        self.tts_voice.get().map(String::as_str)
    }

    pub fn set_tts_voice(&self, voice: String) {
        let _ = self.tts_voice.set(voice);
    }

    /// Every file this daemon's own config needs. Set before the socket
    /// accepts, so a caller never sees it unset.
    pub fn wanted_downloads(&self) -> &[crate::models::download::Download] {
        self.wanted_downloads.get().map_or(&[], Vec::as_slice)
    }

    pub fn set_wanted_downloads(&self, wanted: Vec<crate::models::download::Download>) {
        let _ = self.wanted_downloads.set(wanted);
    }

    pub fn config(&self) -> Arc<Config> {
        Arc::clone(&self.config.read().unwrap())
    }

    pub fn set_config(&self, config: Arc<Config>) {
        *self.config.write().unwrap() = config;
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

    // A record start reads this, so a write between two dictations changes
    // what the next one does.
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

    pub fn db_connection(&self) -> Option<&Mutex<rusqlite::Connection>> {
        self.db_connection.as_ref()
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
mod tests {
    use super::*;

    /// A client offers the command behind a Copy button, so `command` has to
    /// be the command the sentence names, not a near miss.
    #[test]
    fn a_recording_error_names_the_same_command_its_sentence_does() {
        for error in [
            RecordingError::Microphone(String::new()),
            RecordingError::Model(String::new()),
        ] {
            let command = error.command().expect("both faults name a command");
            assert!(
                error.fix().ends_with(command),
                "`{}` does not end with `{command}`",
                error.fix()
            );
        }
    }

    // A watchdog rescans after a fault, but `start_recording` returning Err
    // spawns none, so that one microphone fault needs a restart.
    #[test]
    fn a_microphone_fix_names_the_restart_that_some_faults_need() {
        let fix = RecordingError::Microphone("no device".to_string()).fix();
        assert!(fix.contains("connect the microphone"), "{fix}");
        assert!(fix.contains("[audio] input_device"), "{fix}");
        assert!(
            fix.contains("banshee start"),
            "a fault at startup recovers only on a restart: {fix}"
        );
        // The renderer prints "fix: {}" and adds no period of its own
        assert!(!fix.ends_with('.'), "{fix}");
    }

    // Hands back the receiver: dropping it discards whatever record_stop queues
    fn test_state_with_commands() -> (DaemonState, std::sync::mpsc::Receiver<ConsumerCommand>) {
        let (commands, requests) = std::sync::mpsc::channel();
        let state = DaemonState::new(
            "0.0.0",
            "stt",
            "vad",
            0.5,
            "default".to_string(),
            None,
            crate::text_to_speech::SpeechPlayer::default(),
            commands,
            std::sync::mpsc::channel().0,
            BargeInMode::Stop,
        );
        (state, requests)
    }

    fn test_state() -> DaemonState {
        test_state_with_commands().0
    }

    // A deaf daemon must refuse the press rather than open a session that no
    // consumer thread exists to drain.
    #[test]
    fn record_start_is_refused_without_a_pipeline() {
        let (state, transcribe_requests) = test_state_with_commands();
        state.set_recording_error(RecordingError::Microphone("no device".to_string()));

        assert!(!state.record_start(TranscribeTarget::Mailbox));
        assert_eq!(state.recording_mode(), RecordingMode::Idle);
        // Nothing may reach the consumer: there is nothing on the other end
        assert!(transcribe_requests.try_recv().is_err());

        // record_stop stays a no-op, so a release keybind cannot wedge the mode
        state.record_stop();
        assert_eq!(state.recording_mode(), RecordingMode::Idle);
    }

    #[test]
    fn recording_error_keeps_the_cause_it_was_given() {
        let state = test_state();
        assert!(state.recording_error().is_none());
        // An armed session is available while nothing is wrong
        assert!(state.arm_for_ask());
        state.set_recording_mode(RecordingMode::Idle);

        state.set_recording_error(RecordingError::Model("missing file".to_string()));
        assert!(matches!(
            state.recording_error(),
            Some(RecordingError::Model(_))
        ));
        // The same gate record_start uses, so ask_user cannot arm a deaf mic
        assert!(!state.arm_for_ask());
        assert_eq!(state.recording_mode(), RecordingMode::Idle);
    }

    #[test]
    fn audio_device_refreshes() {
        let (state, _requests) = test_state_with_commands();
        assert_eq!(state.audio_device(), None);

        state.set_audio_device(Some("OnePlus Buds 3".to_string()));
        assert_eq!(state.audio_device().as_deref(), Some("OnePlus Buds 3"));

        // The OnceLock discarded this. A rebind depends on it landing.
        state.set_audio_device(Some("MacBook Pro Microphone".to_string()));
        assert_eq!(
            state.audio_device().as_deref(),
            Some("MacBook Pro Microphone")
        );

        state.set_audio_device(None);
        assert_eq!(state.audio_device(), None);
    }

    #[test]
    fn missing_device_names_what_the_config_waits_for() {
        let (state, _requests) = test_state_with_commands();
        assert_eq!(state.missing_device(), None);

        state.set_missing_device(Some("yeti".to_string()));
        assert_eq!(state.missing_device().as_deref(), Some("yeti"));

        // A substitute does not block recording, so no blocker appears with it
        assert!(state.recording_error().is_none());
        assert!(state.record_start(TranscribeTarget::Mailbox));
        state.record_stop();

        // The named device returns, so nothing is missing any more
        state.set_missing_device(None);
        assert_eq!(state.missing_device(), None);
    }

    // A panic here kills capture silently, and a device name has no invariant
    // a panic can break, so a poisoned lock must still answer
    #[test]
    fn the_wanted_device_survives_a_poisoned_lock() {
        let state = Arc::new(test_state());
        let holder = Arc::clone(&state);
        let poisoning = std::thread::spawn(move || {
            let _held = holder.wanted_device.lock().unwrap();
            panic!("the writer died holding the lock");
        });
        assert!(poisoning.join().is_err());

        assert_eq!(state.wanted_device(), "default");
        state.set_wanted_device("yeti".to_string());
        assert_eq!(state.wanted_device(), "yeti");
    }

    // While a named device stays absent the watchdog rewrites the same name
    // every rescan, which is every 5 seconds. Each rewrite that counts as a
    // change wakes the push task of every subscriber.
    #[test]
    fn a_rewritten_device_name_is_not_a_change() {
        let (state, _requests) = test_state_with_commands();
        let mut changes = state.device_changes();
        assert_eq!(*changes.borrow_and_update(), 0);

        state.set_missing_device(Some("yeti".to_string()));
        state.set_missing_device(Some("yeti".to_string()));
        state.set_missing_device(Some("yeti".to_string()));
        assert!(
            changes.has_changed().unwrap(),
            "the first write went unheard"
        );
        assert_eq!(*changes.borrow_and_update(), 1, "a rewrite counted");
        assert!(!changes.has_changed().unwrap());

        state.set_audio_device(Some("MacBook Pro Microphone".to_string()));
        state.set_audio_device(Some("MacBook Pro Microphone".to_string()));
        assert!(
            changes.has_changed().unwrap(),
            "the open device went unheard"
        );
        assert_eq!(*changes.borrow_and_update(), 2, "a rewrite counted");

        // One counter for the whole picture, so either field moving wakes the push
        state.set_missing_device(None);
        assert!(
            changes.has_changed().unwrap(),
            "the device returning went unheard"
        );
        assert_eq!(*changes.borrow_and_update(), 3);
    }

    #[test]
    fn recording_error_clears_when_capture_recovers() {
        let (state, _requests) = test_state_with_commands();
        state.set_recording_error(RecordingError::Microphone("gone".to_string()));
        assert!(state.recording_error().is_some());

        state.clear_recording_error();
        assert!(state.recording_error().is_none());

        // A second fault after a recovery must still report
        state.set_recording_error(RecordingError::Microphone("gone again".to_string()));
        assert!(matches!(
            state.recording_error(),
            Some(RecordingError::Microphone(_))
        ));
    }

    // A wrong branch here drops the utterance or leaves the mic open
    #[test]
    fn a_toggle_stops_the_session_a_toggle_started() {
        let (state, requests) = test_state_with_commands();

        assert!(
            state.record_toggle(TranscribeTarget::Dictate),
            "idle: starts"
        );
        assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);

        assert!(
            !state.record_toggle(TranscribeTarget::Dictate),
            "in flight: stops"
        );
        assert_eq!(state.recording_mode(), RecordingMode::Idle);
        assert!(matches!(
            requests.try_recv(),
            Ok(ConsumerCommand::Transcribe(TranscribeTarget::Dictate))
        ));

        // A manual override counts as in flight and ends as a stop
        state.set_recording_mode(RecordingMode::ArmedHold);
        assert!(!state.record_toggle(TranscribeTarget::Dictate));
        assert_eq!(state.recording_mode(), RecordingMode::Armed);
    }

    // A wrong routing here turns typed-with-the-modifier noise into dictation
    #[test]
    fn cancel_discards_the_session_instead_of_routing_it() {
        let (state, requests) = test_state_with_commands();

        assert!(state.record_start(TranscribeTarget::Dictate));
        state.record_cancel();
        assert_eq!(state.recording_mode(), RecordingMode::Idle);
        assert!(matches!(requests.try_recv(), Ok(ConsumerCommand::Discard)));
        assert!(requests.try_recv().is_err(), "nothing may be transcribed");

        // Nothing in flight, so a cancel is a no-op, like record_stop
        state.record_cancel();
        assert_eq!(state.recording_mode(), RecordingMode::Idle);
        assert!(requests.try_recv().is_err());

        // A manual override returns to armed and keeps its audio
        state.set_recording_mode(RecordingMode::ArmedHold);
        state.record_cancel();
        assert_eq!(state.recording_mode(), RecordingMode::Armed);
        assert!(
            requests.try_recv().is_err(),
            "the armed session owns the ring"
        );
    }

    #[test]
    fn watchdog_releases_a_push_to_talk_that_never_stopped() {
        let (state, transcribe_requests) = test_state_with_commands();

        assert!(state.record_start(TranscribeTarget::Mailbox));
        assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);

        // Nowhere near the ceiling yet: the mic stays open
        assert!(!state.expire_stuck_recording());
        assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);

        // Bring the deadline forward instead of waiting out MAX_PUSH_TO_TALK
        state
            .push_to_talk_deadline
            .store(0, std::sync::atomic::Ordering::Relaxed);

        // Past it, the mic comes back and the utterance is still transcribed
        assert!(state.expire_stuck_recording());
        assert_eq!(state.recording_mode(), RecordingMode::Idle);
        assert!(matches!(
            transcribe_requests.try_recv(),
            Ok(ConsumerCommand::Transcribe(TranscribeTarget::Mailbox))
        ));

        // And a fresh start is accepted rather than refused as busy
        assert!(state.record_start(TranscribeTarget::Mailbox));
    }

    #[test]
    fn watchdog_leaves_armed_listening_alone() {
        let state = test_state();
        // ask_user sessions run their own timeouts; the watchdog must not
        // yank the microphone out from under one
        state.set_recording_mode(RecordingMode::Armed);
        state
            .push_to_talk_deadline
            .store(0, std::sync::atomic::Ordering::Relaxed);
        assert!(!state.expire_stuck_recording());
        assert_eq!(state.recording_mode(), RecordingMode::Armed);
    }

    // Three writers move the mic: the hotkey and the record RPCs through
    // try_transition, the consumer thread through a direct write.
    #[test]
    fn every_path_that_moves_the_mic_publishes_it() {
        let (state, _requests) = test_state_with_commands();
        let mut recording = state.subscribe_recording();
        assert!(!*recording.borrow_and_update());

        assert!(state.record_start(TranscribeTarget::Mailbox));
        assert!(
            recording.has_changed().unwrap(),
            "record_start went unheard"
        );
        assert!(*recording.borrow_and_update());

        state.record_stop();
        assert!(recording.has_changed().unwrap(), "record_stop went unheard");
        assert!(!*recording.borrow_and_update());

        state.set_recording_mode(RecordingMode::Armed);
        assert!(
            recording.has_changed().unwrap(),
            "a direct write went unheard"
        );
        assert!(*recording.borrow_and_update());
    }

    #[test]
    fn the_voice_reported_is_the_one_the_daemon_loaded() {
        let state = test_state();
        assert_eq!(state.tts_voice(), None);
        state.set_tts_voice("af_sky".to_string());
        assert_eq!(state.tts_voice(), Some("af_sky"));
        state.set_tts_voice("am_adam".to_string());
        assert_eq!(
            state.tts_voice(),
            Some("af_sky"),
            "a second write must not replace what is already loaded"
        );
    }

    #[test]
    fn recording_mode_roundtrips_and_derives_is_recording() {
        let state = test_state();
        assert_eq!(state.recording_mode(), RecordingMode::Idle);
        assert!(!state.is_recording());
        for mode in [RecordingMode::PushToTalk, RecordingMode::Armed] {
            state.set_recording_mode(mode);
            assert_eq!(state.recording_mode(), mode);
            assert!(state.is_recording());
        }
    }

    #[test]
    fn ring_evicts_oldest_and_filters_by_cursor() {
        let state = test_state();
        for i in 1..=20 {
            state.push_transcription(format!("utterance {i}"));
        }

        let all = state.transcriptions_since(0);
        assert_eq!(all.len(), TRANSCRIPTION_RING_CAPACITY);
        assert_eq!(all.first().unwrap().id, 5);
        assert_eq!(all.last().unwrap().id, 20);

        let newer = state.transcriptions_since(18);
        assert_eq!(newer.len(), 2);
        assert_eq!(newer[0].text, "utterance 19");

        assert!(state.transcriptions_since(20).is_empty());
        assert_eq!(*state.subscribe_transcriptions().borrow(), 20);
    }

    #[test]
    fn capture_is_stalled_until_the_callback_stamps_it() {
        let (state, _requests) = test_state_with_commands();
        // Nothing has captured yet, so a watchdog must not call this healthy
        assert!(state.capture_is_stalled());

        state.mark_capture_alive();
        assert!(!state.capture_is_stalled());
    }

    #[test]
    fn the_silence_limit_clears_every_measured_callback_rate() {
        // Measured: 55 callbacks per second on Bluetooth, 93 on the built in mic.
        // The slowest gives the longest gap, so it sets the headroom.
        let slowest_gap = Duration::from_millis(1000 / 55);
        assert!(
            CAPTURE_SILENCE_LIMIT >= slowest_gap * 20,
            "the limit must keep well clear of a healthy gap, or a busy \
             machine trips it"
        );
    }
}

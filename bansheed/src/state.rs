use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use banshee_common::DownloadProgress;
use tokio::sync::{broadcast, watch};

use crate::audio::cues::Cue;
use crate::config::BargeInMode;
use crate::text_to_speech::SpeechPlayer;

const TRANSCRIPTION_RING_CAPACITY: usize = 16;

// One file emits at most 101 notifications, so a subscriber has to fall a whole
// file behind before the channel starts dropping any of them
const DOWNLOAD_BACKLOG: usize = 128;

// A start with no stop otherwise holds the mic for the life of the daemon.
// The ring only holds RING_SECS, so nothing is lost by capping it there.
pub const MAX_PUSH_TO_TALK: Duration = Duration::from_secs(crate::audio::RING_SECS as u64);

/// How long the input stream may stay quiet before we treat it as dead.
/// Measured on macOS BT disconnect: callbacks drop to zero within ~1s and
/// never resume (see #47). Two seconds leaves headroom for scheduling jitter
/// without waiting forever on a ghost mic.
pub const INPUT_CALLBACK_STALE: Duration = Duration::from_secs(2);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

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

    pub fn fix(&self) -> &'static str {
        match self {
            RecordingError::Microphone(_) => {
                "connect the microphone, grant it in Privacy & Security, or fix \
                 [audio] input_device, then restart: banshee start"
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

pub struct DaemonState {
    version: &'static str,
    stt_model: &'static str,
    vad_model: &'static str,
    vad_threshold: AtomicU32,
    // Mutable: a Bluetooth disconnect must clear the name so status/tray do
    // not keep advertising a mic that no longer exists (#47).
    audio_device: Mutex<Option<String>>,
    // Preferred config string ("default" or a substring). Used by the health
    // poll to decide whether the bound device is still in the OS list.
    configured_input: OnceLock<String>,
    // Wall-clock ms of the last cpal input callback. Zero until the first one.
    last_input_callback_ms: AtomicU64,
    // True after capture has produced at least one callback (stream is live).
    input_stream_seen_data: AtomicBool,
    // Sticky: once the bound input is gone, recording must fail closed until restart.
    input_lost: AtomicBool,
    tts_voice: OnceLock<String>,
    wanted_downloads: OnceLock<Vec<crate::models::download::Download>>,
    // Why recording is off, when it is. Set once at startup or on input loss;
    // the mic and models do not hot-reload without a restart today.
    recording_error: OnceLock<RecordingError>,
    recording: AtomicU8,
    started_at: Instant,
    db_connection: Option<Mutex<rusqlite::Connection>>,
    transcriptions: Mutex<TranscriptionRing>,
    latest_transcription_id: watch::Sender<u64>,
    recording_active: watch::Sender<bool>,
    downloads: broadcast::Sender<DownloadProgress>,
    downloading: AtomicBool,
    speech: Arc<SpeechPlayer>,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
    cues: std::sync::mpsc::Sender<Cue>,
    barge_in: BargeInMode,
    // Start and stop can be separate RPC calls, so this cannot live on a stack
    pending_dictate: AtomicBool,
    // enigo posts to the same HID stream rdev listens at, so while this is
    // true the hotkey listener drops events: the paste's own modifier presses
    // would otherwise cancel or open sessions.
    typing: AtomicBool,
    // Milliseconds since `started_at` at which an open push-to-talk is stuck.
    // A deadline rather than a start keeps the watchdog to one load and compare.
    push_to_talk_deadline: AtomicU64,
    shutdown: tokio::sync::Notify,
}

impl DaemonState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: &'static str,
        stt_model: &'static str,
        vad_model: &'static str,
        initial_vad_threshold: f32,
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
            audio_device: Mutex::new(None),
            configured_input: OnceLock::new(),
            last_input_callback_ms: AtomicU64::new(0),
            input_stream_seen_data: AtomicBool::new(false),
            input_lost: AtomicBool::new(false),
            tts_voice: OnceLock::new(),
            wanted_downloads: OnceLock::new(),
            recording_error: OnceLock::new(),
            recording: AtomicU8::new(RecordingMode::Idle as u8),
            started_at: Instant::now(),
            db_connection,
            transcriptions: Mutex::new(TranscriptionRing {
                next_id: 0,
                entries: VecDeque::with_capacity(TRANSCRIPTION_RING_CAPACITY),
            }),
            latest_transcription_id: watch::channel(0).0,
            recording_active: watch::channel(false).0,
            downloads: broadcast::channel(DOWNLOAD_BACKLOG).0,
            downloading: AtomicBool::new(false),
            speech: Arc::new(speech),
            commands,
            cues,
            barge_in,
            pending_dictate: AtomicBool::new(false),
            typing: AtomicBool::new(false),
            push_to_talk_deadline: AtomicU64::new(0),
            shutdown: tokio::sync::Notify::new(),
        }
    }

    // Push-to-talk press, shared by the hotkey listener and the record RPC.
    // Returns false when another session already owns the microphone.
    pub fn record_start(&self, action: TranscribeTarget) -> bool {
        // The hotkey arrives here too, so a deaf daemon answers a press with the
        // error cue. Arming a session nothing can transcribe would be silent.
        if self.recording_blocked() {
            let _ = self.cues.send(Cue::Error);
            return false;
        }
        if self.try_transition(RecordingMode::Armed, RecordingMode::ArmedHold) {
            // Manual override of an armed session: hold to answer
            if matches!(self.barge_in, BargeInMode::Stop) {
                self.speech.stop();
            }
            let _ = self.cues.send(Cue::RecordStart);
            true
        } else if self.try_transition(RecordingMode::Idle, RecordingMode::PushToTalk) {
            // Silence the daemon's own voice before the mic opens
            if matches!(self.barge_in, BargeInMode::Stop) {
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
        self.audio_device
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Why recording is unavailable, or `None` when it works.
    pub fn recording_error(&self) -> Option<&RecordingError> {
        self.recording_error.get()
    }

    pub fn set_recording_error(&self, reason: RecordingError) {
        let _ = self.recording_error.set(reason);
    }

    /// True when startup failed or the bound input later disappeared (#47).
    pub fn recording_blocked(&self) -> bool {
        self.recording_error.get().is_some() || self.input_lost.load(Ordering::Relaxed)
    }

    /// Takes the armed-listening lock for `ask_user`. Shares the availability
    /// gate with `record_start`, so no caller can arm a mic that cannot record.
    pub fn arm_for_ask(&self) -> bool {
        !self.recording_blocked()
            && self.try_transition(RecordingMode::Idle, RecordingMode::Armed)
    }

    pub fn set_audio_device(&self, device_name: String) {
        *self
            .audio_device
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(device_name);
    }

    pub fn clear_audio_device(&self) {
        *self
            .audio_device
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
    }

    pub fn set_configured_input(&self, input_device: String) {
        let _ = self.configured_input.set(input_device);
    }

    pub fn configured_input(&self) -> Option<&str> {
        self.configured_input.get().map(String::as_str)
    }

    /// Called from the real-time cpal input callback. Must not allocate or lock
    /// anything heavier than atomics.
    pub fn note_input_callback(&self) {
        self.last_input_callback_ms.store(now_ms(), Ordering::Relaxed);
        self.input_stream_seen_data.store(true, Ordering::Relaxed);
    }

    /// Fail closed when the bound input is gone: clear the advertised name,
    /// refuse new recording sessions, stop any open session without routing
    /// its (empty) ring, and set a sticky mic error for status/RPC.
    ///
    /// This is deliberately not a display-only refresh: naming a different
    /// default mic while capture is still bound to the dead device would look
    /// healthy and stay broken (see #47).
    pub fn mark_input_lost(&self, detail: impl Into<String>) {
        if self.input_lost.swap(true, Ordering::Relaxed) {
            return;
        }
        let detail = detail.into();
        eprintln!("Input device lost: {detail}");
        self.clear_audio_device();
        let _ = self
            .recording_error
            .set(RecordingError::Microphone(detail));
        // Drop an in-flight session rather than transcribe an empty ring.
        if self.is_recording() {
            self.record_cancel();
            // Cancel leaves Armed alone; force idle so the UI is not stuck.
            if self.recording_mode() != RecordingMode::Idle {
                self.set_recording_mode(RecordingMode::Idle);
            }
        }
        let _ = self.cues.send(Cue::Error);
        self.publish_recording();
    }

    /// Portable health check from maintainer measurements on #47:
    /// 1) the configured device leaves `input_devices()`, and/or
    /// 2) data callbacks stop (cpal error callback does *not* fire on BT drop).
    ///
    /// `listed` is whether the configured preference still resolves in the OS
    /// device list (caller enumerates; this stays unit-testable).
    pub fn should_mark_input_lost(&self, configured_still_listed: bool) -> bool {
        if self.input_lost.load(Ordering::Relaxed) {
            return false;
        }
        // Never opened a live stream — startup already set recording_error.
        if !self.input_stream_seen_data.load(Ordering::Relaxed) {
            return false;
        }
        if !configured_still_listed {
            return true;
        }
        let last = self.last_input_callback_ms.load(Ordering::Relaxed);
        if last == 0 {
            return false;
        }
        now_ms().saturating_sub(last) >= INPUT_CALLBACK_STALE.as_millis() as u64
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
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hands back the receiver: dropping it discards whatever record_stop queues
    fn test_state_with_commands() -> (DaemonState, std::sync::mpsc::Receiver<ConsumerCommand>) {
        let (commands, requests) = std::sync::mpsc::channel();
        let state = DaemonState::new(
            "0.0.0",
            "stt",
            "vad",
            0.5,
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
    fn audio_device_name_can_be_cleared_after_set() {
        let state = test_state();
        assert_eq!(state.audio_device(), None);
        state.set_audio_device("OnePlus Buds 3".to_string());
        assert_eq!(state.audio_device().as_deref(), Some("OnePlus Buds 3"));
        state.clear_audio_device();
        assert_eq!(state.audio_device(), None);
    }

    // #47: once callbacks have been seen, leaving the OS list is enough.
    #[test]
    fn input_loss_when_configured_device_leaves_the_list() {
        let state = test_state();
        state.note_input_callback();
        assert!(state.should_mark_input_lost(false));
        assert!(!state.should_mark_input_lost(true));
    }

    // #47: callbacks going quiet is itself the signal (cpal errors stay at 0).
    #[test]
    fn input_loss_when_callbacks_go_stale() {
        let state = test_state();
        // Simulate a last callback well past the stale window.
        state
            .last_input_callback_ms
            .store(1, std::sync::atomic::Ordering::Relaxed);
        state
            .input_stream_seen_data
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(state.should_mark_input_lost(true));
    }

    #[test]
    fn mark_input_lost_fails_closed_and_clears_the_advertised_name() {
        let (state, requests) = test_state_with_commands();
        state.set_audio_device("gone-headset".to_string());
        assert!(state.record_start(TranscribeTarget::Mailbox));

        state.mark_input_lost("input device no longer listed");

        assert_eq!(state.audio_device(), None);
        assert!(state.recording_blocked());
        assert_eq!(state.recording_mode(), RecordingMode::Idle);
        assert!(matches!(requests.try_recv(), Ok(ConsumerCommand::Discard)));
        // New sessions must not open on a ghost mic.
        assert!(!state.record_start(TranscribeTarget::Mailbox));
        // Second mark is a no-op (sticky).
        state.mark_input_lost("again");
        assert!(matches!(
            state.recording_error(),
            Some(RecordingError::Microphone(_))
        ));
    }

    #[test]
    fn no_false_input_loss_before_the_stream_has_ticked() {
        let state = test_state();
        // Capture never opened (or never delivered a callback yet).
        assert!(!state.should_mark_input_lost(false));
        assert!(!state.should_mark_input_lost(true));
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
}

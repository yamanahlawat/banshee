use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU8, AtomicU32},
    },
    time::{Duration, Instant},
};

use tokio::sync::watch;

use crate::text_to_speech::SpeechPlayer;

const TRANSCRIPTION_RING_CAPACITY: usize = 16;

#[derive(Clone, Copy)]
pub enum HotKeyAction {
    Mailbox,
    Dictate,
}

pub struct AskCommand {
    pub reply: tokio::sync::oneshot::Sender<String>,
    pub timeout: Duration,
}

// Work items for the audio consumer thread
pub enum ConsumerCommand {
    Transcribe(HotKeyAction),
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
    // Armed, but the user is holding F5 to answer manually
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

pub struct DaemonState {
    version: &'static str,
    stt_model: &'static str,
    vad_model: &'static str,
    vad_threshold: AtomicU32,
    audio_device: OnceLock<String>,
    recording: AtomicU8,
    started_at: Instant,
    db_connection: Option<Mutex<rusqlite::Connection>>,
    transcriptions: Mutex<TranscriptionRing>,
    latest_transcription_id: watch::Sender<u64>,
    speech: Arc<SpeechPlayer>,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
    shutdown: tokio::sync::Notify,
}

impl DaemonState {
    pub fn new(
        version: &'static str,
        stt_model: &'static str,
        vad_model: &'static str,
        initial_vad_threshold: f32,
        db_connection: Option<Mutex<rusqlite::Connection>>,
        speech: SpeechPlayer,
        commands: std::sync::mpsc::Sender<ConsumerCommand>,
    ) -> Self {
        Self {
            version,
            stt_model,
            vad_model,
            vad_threshold: AtomicU32::new(initial_vad_threshold.to_bits()),
            audio_device: OnceLock::new(),
            recording: AtomicU8::new(RecordingMode::Idle as u8),
            started_at: Instant::now(),
            db_connection,
            transcriptions: Mutex::new(TranscriptionRing {
                next_id: 0,
                entries: VecDeque::with_capacity(TRANSCRIPTION_RING_CAPACITY),
            }),
            latest_transcription_id: watch::channel(0).0,
            speech: Arc::new(speech),
            commands,
            shutdown: tokio::sync::Notify::new(),
        }
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
    }

    // Concurrent transitions race (RPC arm vs hotkey vs session end);
    // compare_exchange makes losing a race a no-op instead of a stuck mode
    pub fn try_transition(&self, from: RecordingMode, to: RecordingMode) -> bool {
        self.recording
            .compare_exchange(
                from as u8,
                to as u8,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok()
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

    pub fn audio_device(&self) -> Option<&str> {
        self.audio_device.get().map(String::as_str)
    }

    pub fn set_audio_device(&self, device_name: String) {
        let _ = self.audio_device.set(device_name);
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

    #[test]
    fn recording_mode_roundtrips_and_derives_is_recording() {
        let state = DaemonState::new(
            "0.0.0",
            "stt",
            "vad",
            0.5,
            None,
            crate::text_to_speech::SpeechPlayer::default(),
            std::sync::mpsc::channel().0,
        );
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
        let state = DaemonState::new(
            "0.0.0",
            "stt",
            "vad",
            0.5,
            None,
            crate::text_to_speech::SpeechPlayer::default(),
            std::sync::mpsc::channel().0,
        );
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

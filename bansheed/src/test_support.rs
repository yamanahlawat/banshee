use std::sync::Arc;

use crate::config::BargeInMode;
use crate::history::TranscriptionHistory;
use crate::state::{ConsumerCommand, DaemonState};
use crate::text_to_speech::{ActiveUtterance, SpeechPlayer, TtsBackend};

// Silent backend, so no test spawns a real `say` process
struct NullBackend;
struct Done;

impl ActiveUtterance for Done {
    fn is_finished(&mut self) -> bool {
        true
    }
    fn stop(&mut self) {}
}

impl TtsBackend for NullBackend {
    fn start(
        &self,
        _text: &str,
        _voice: Option<&str>,
    ) -> std::io::Result<Box<dyn ActiveUtterance>> {
        Ok(Box::new(Done))
    }
}

/// The voice and rate the last live `[tts]` write handed the backend.
pub type RecordedTts = Arc<std::sync::Mutex<Vec<(String, f32)>>>;

struct RecordingBackend(RecordedTts);

impl TtsBackend for RecordingBackend {
    fn start(
        &self,
        _text: &str,
        _voice: Option<&str>,
    ) -> std::io::Result<Box<dyn ActiveUtterance>> {
        Ok(Box::new(Done))
    }

    fn reconfigure(&self, tts: &crate::config::TTSConfig) -> Option<String> {
        self.0.lock().unwrap().push((tts.voice.clone(), tts.speed));
        Some(tts.voice.clone())
    }
}

/// A daemon state whose backend keeps what it was last told to speak in.
pub fn daemon_state_recording_tts() -> (Arc<DaemonState>, RecordedTts) {
    let recorded: RecordedTts = Arc::default();
    let speech = SpeechPlayer::new(Box::new(RecordingBackend(recorded.clone())));
    (state(None, speech, std::sync::mpsc::channel().0), recorded)
}

fn state(
    history: Option<rusqlite::Connection>,
    speech: SpeechPlayer,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
) -> Arc<DaemonState> {
    Arc::new(DaemonState::new(
        "0.0.0",
        "stt",
        "vad",
        0.5,
        "default".to_string(),
        history,
        speech,
        commands,
        crate::audio::cues::Cues::silent(),
        BargeInMode::Stop,
    ))
}

/// Pass a real sender only when the test reads the command receiver.
pub fn daemon_state(commands: std::sync::mpsc::Sender<ConsumerCommand>) -> Arc<DaemonState> {
    state(None, SpeechPlayer::new(Box::new(NullBackend)), commands)
}

/// A daemon state whose history holds `rows`, oldest first.
pub fn daemon_state_with_history(rows: &[&str]) -> Arc<DaemonState> {
    state(
        Some(seeded_history(rows)),
        SpeechPlayer::new(Box::new(NullBackend)),
        std::sync::mpsc::channel().0,
    )
}

/// An in-memory history holding `rows`, oldest first.
pub fn seeded_history(rows: &[&str]) -> rusqlite::Connection {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    TranscriptionHistory::create_table(&connection).unwrap();
    for text in rows {
        TranscriptionHistory::insert(&connection, text).unwrap();
    }
    connection
}

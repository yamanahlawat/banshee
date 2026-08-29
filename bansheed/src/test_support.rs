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

/// Pass a real sender only when the test reads the command receiver.
pub fn daemon_state(commands: std::sync::mpsc::Sender<ConsumerCommand>) -> Arc<DaemonState> {
    Arc::new(DaemonState::new(
        "0.0.0",
        "stt",
        "vad",
        0.5,
        "default".to_string(),
        None,
        SpeechPlayer::new(Box::new(NullBackend)),
        commands,
        crate::audio::cues::Cues::silent(),
        BargeInMode::Stop,
    ))
}

/// A daemon state whose history holds `rows`, oldest first.
pub fn daemon_state_with_history(rows: &[&str]) -> Arc<DaemonState> {
    Arc::new(DaemonState::new(
        "0.0.0",
        "stt",
        "vad",
        0.5,
        "default".to_string(),
        Some(seeded_history(rows)),
        SpeechPlayer::new(Box::new(NullBackend)),
        std::sync::mpsc::channel().0,
        crate::audio::cues::Cues::silent(),
        BargeInMode::Stop,
    ))
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

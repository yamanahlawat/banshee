use std::sync::Arc;

use crate::config::Config;
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

struct Held {
    until: std::time::Instant,
    cut_short: Arc<std::sync::atomic::AtomicBool>,
}

impl ActiveUtterance for Held {
    fn is_finished(&mut self) -> bool {
        std::time::Instant::now() >= self.until
    }
    fn stop(&mut self) {
        self.cut_short
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

struct HoldingBackend {
    hold: std::time::Duration,
    cut_short: Arc<std::sync::atomic::AtomicBool>,
}

impl TtsBackend for HoldingBackend {
    fn start(
        &self,
        _text: &str,
        _voice: Option<&str>,
    ) -> std::io::Result<Box<dyn ActiveUtterance>> {
        Ok(Box::new(Held {
            until: std::time::Instant::now() + self.hold,
            cut_short: Arc::clone(&self.cut_short),
        }))
    }
}

/// A daemon state whose speech plays for `hold`. The flag goes true if an
/// utterance is stopped before it finishes.
pub fn daemon_state_holding_speech(
    hold: std::time::Duration,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
) -> (Arc<DaemonState>, Arc<std::sync::atomic::AtomicBool>) {
    let cut_short: Arc<std::sync::atomic::AtomicBool> = Arc::default();
    let speech = SpeechPlayer::new(Box::new(HoldingBackend {
        hold,
        cut_short: Arc::clone(&cut_short),
    }));
    (state(None, speech, commands), cut_short)
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
    state_running(Config::default(), history, speech, commands)
}

fn state_running(
    config: Config,
    history: Option<rusqlite::Connection>,
    speech: SpeechPlayer,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
) -> Arc<DaemonState> {
    Arc::new(DaemonState::new(
        Arc::new(config),
        history,
        speech,
        commands,
        crate::audio::cues::Cues::silent(),
    ))
}

/// A daemon state that started with `config`, so its restart-only keys run those values.
pub fn daemon_state_running(
    config: Config,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
) -> Arc<DaemonState> {
    state_running(
        config,
        None,
        SpeechPlayer::new(Box::new(NullBackend)),
        commands,
    )
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

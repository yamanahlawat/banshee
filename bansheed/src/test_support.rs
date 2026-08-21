use std::sync::Arc;

use crate::config::BargeInMode;
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
    fn start(&self, _text: &str) -> std::io::Result<Box<dyn ActiveUtterance>> {
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
        None,
        SpeechPlayer::new(Box::new(NullBackend)),
        commands,
        std::sync::mpsc::channel().0,
        BargeInMode::Stop,
    ))
}

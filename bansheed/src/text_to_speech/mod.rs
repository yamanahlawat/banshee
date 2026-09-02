pub mod kokoro;
pub mod oov;
pub mod pronunciation;
pub mod sanitizer;
pub mod say;
pub mod voices;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use banshee_common::{KokoroTTSConfig, error::BansheeError};
use tokio::sync::watch;

use crate::config::{TTSConfig, TTSFallback};
use kokoro::{KokoroBackend, KokoroEngine};
use say::SayBackend;

const MAX_QUEUED_UTTERANCES: usize = 8;

// A backend starts one utterance at a time; SpeechPlayer serializes them
pub trait TtsBackend: Send + Sync {
    fn start(&self, text: &str, voice: Option<&str>) -> std::io::Result<Box<dyn ActiveUtterance>>;

    /// A live `[tts]` change. Answers the voice utterances now speak in, or
    /// `None` when the backend cannot honour the change: the system fallback
    /// speaks in whatever voice the OS is set to and takes no rate.
    fn reconfigure(&self, _tts: &TTSConfig) -> Option<String> {
        None
    }
}

pub trait ActiveUtterance: Send {
    fn is_finished(&mut self) -> bool;
    fn stop(&mut self);
}

/// The backend, and the voice it speaks in. `None` under the system fallback,
/// which speaks in whatever voice the OS is set to.
pub fn select_backend(
    tts_config: &TTSConfig,
) -> Result<(Box<dyn TtsBackend>, Option<String>), BansheeError> {
    let kokoro_config = KokoroTTSConfig::new(&tts_config.voice);
    match KokoroEngine::new(&kokoro_config, tts_config.speed).and_then(KokoroBackend::new) {
        Ok(backend) => {
            println!("TTS: Kokoro (voice {})", tts_config.voice);
            Ok((Box::new(backend), Some(tts_config.voice.clone())))
        }
        Err(e) => match tts_config.fallback {
            TTSFallback::System => {
                eprintln!("Kokoro unavailable, falling back to system TTS: {e}");
                Ok((Box::new(SayBackend), None))
            }
            TTSFallback::None => Err(e),
        },
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Playback {
    utterance_id: u64,
    active: Option<Box<dyn ActiveUtterance>>,
    queue: VecDeque<(String, Option<String>)>,
    watcher_running: bool,
}

pub struct SpeechPlayer {
    backend: Box<dyn TtsBackend>,
    playback: Mutex<Playback>,
    speaking: watch::Sender<bool>,
}

impl SpeechPlayer {
    /// What a live `[tts]` write reaches. An utterance already speaking keeps
    /// the voice and rate it started with.
    pub fn reconfigure(&self, tts: &TTSConfig) -> Option<String> {
        self.backend.reconfigure(tts)
    }

    pub fn new(backend: Box<dyn TtsBackend>) -> Self {
        Self {
            backend,
            playback: Mutex::new(Playback {
                utterance_id: 0,
                active: None,
                queue: VecDeque::new(),
                watcher_running: false,
            }),
            speaking: watch::channel(false).0,
        }
    }

    pub fn speak(
        self: &Arc<Self>,
        text: &str,
        interrupt: bool,
        voice: Option<&str>,
    ) -> Result<u64, std::io::Error> {
        let normalized = pronunciation::normalize(text);
        let text = normalized.as_str();
        let mut playback = self.lock();
        if interrupt {
            playback.queue.clear();
            stop_active(&mut playback);
        }

        playback.utterance_id += 1;
        let utterance_id = playback.utterance_id;

        // normalize can leave nothing speakable (e.g. input was only underscores);
        // keep the id sequence but start no playback
        if text.is_empty() {
            drop(playback);
            if interrupt {
                self.speaking.send_replace(false);
            }
            return Ok(utterance_id);
        }

        if playback.active.is_some() {
            playback
                .queue
                .push_back((text.to_string(), voice.map(str::to_string)));
            // drop the oldest backlog rather than droning through stale updates
            if playback.queue.len() > MAX_QUEUED_UTTERANCES {
                playback.queue.pop_front();
            }
            return Ok(utterance_id);
        }

        playback.active = Some(self.backend.start(text, voice)?);
        let needs_watcher = !playback.watcher_running;
        playback.watcher_running = true;
        drop(playback);
        self.speaking.send_replace(true);

        if needs_watcher {
            let player = Arc::clone(self);
            thread::spawn(move || player.watch_playback());
        }
        Ok(utterance_id)
    }

    pub fn stop(&self) {
        let mut playback = self.lock();
        playback.queue.clear();
        stop_active(&mut playback);
        drop(playback);
        self.speaking.send_replace(false);
    }

    pub fn is_speaking(&self) -> bool {
        *self.speaking.borrow()
    }

    pub fn subscribe_speaking(&self) -> watch::Receiver<bool> {
        self.speaking.subscribe()
    }

    fn watch_playback(&self) {
        loop {
            thread::sleep(Duration::from_millis(50));
            let mut playback = self.lock();
            let Some(active) = playback.active.as_mut() else {
                // stopped; a speak arriving before we exit reuses this watcher
                playback.watcher_running = false;
                return;
            };
            if !active.is_finished() {
                continue;
            }
            playback.active = None;
            match playback.queue.pop_front() {
                Some((next, voice)) => match self.backend.start(&next, voice.as_deref()) {
                    Ok(utterance) => playback.active = Some(utterance),
                    Err(e) => {
                        eprintln!("Failed to speak queued utterance: {e}");
                        playback.queue.clear();
                        playback.watcher_running = false;
                        drop(playback);
                        self.speaking.send_replace(false);
                        return;
                    }
                },
                None => {
                    playback.watcher_running = false;
                    drop(playback);
                    self.speaking.send_replace(false);
                    return;
                }
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, Playback> {
        lock(&self.playback)
    }
}

impl Default for SpeechPlayer {
    fn default() -> Self {
        Self::new(Box::new(SayBackend))
    }
}

fn stop_active(playback: &mut Playback) {
    if let Some(mut active) = playback.active.take() {
        active.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utterance_ids_increment_and_stop_clears() {
        let player = Arc::new(SpeechPlayer::default());
        assert!(!player.is_speaking());

        let first = player.speak("", false, None).unwrap();
        let second = player.speak("", false, None).unwrap();
        assert_eq!((first, second), (1, 2));

        player.stop();
        assert!(!player.is_speaking());
    }

    #[tokio::test]
    async fn queued_utterances_drain_and_signal_completion() {
        let player = Arc::new(SpeechPlayer::default());
        let mut speaking = player.subscribe_speaking();
        player.speak("", false, None).unwrap();
        player.speak("", false, None).unwrap();

        tokio::time::timeout(Duration::from_secs(3), speaking.wait_for(|s| !s))
            .await
            .expect("playback completion was never signalled")
            .expect("speaking sender dropped");
    }
}

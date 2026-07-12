pub mod kokoro;
pub mod sanitizer;
pub mod say;

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
    fn start(&self, text: &str) -> std::io::Result<Box<dyn ActiveUtterance>>;
}

pub trait ActiveUtterance: Send {
    fn is_finished(&mut self) -> bool;
    fn stop(&mut self);
}

// Kokoro when its model is on disk, otherwise whatever the fallback allows
pub fn select_backend(tts_config: &TTSConfig) -> Result<Box<dyn TtsBackend>, BansheeError> {
    let kokoro_config = KokoroTTSConfig::new(&tts_config.voice);
    match KokoroEngine::new(&kokoro_config, tts_config.speed).and_then(KokoroBackend::new) {
        Ok(backend) => {
            println!("TTS: Kokoro (voice {})", tts_config.voice);
            Ok(Box::new(backend))
        }
        Err(e) => match tts_config.fallback {
            TTSFallback::System => {
                eprintln!("Kokoro unavailable, falling back to system TTS: {e}");
                Ok(Box::new(SayBackend))
            }
            TTSFallback::None => Err(e),
        },
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Playback {
    utterance_id: u64,
    active: Option<Box<dyn ActiveUtterance>>,
    queue: VecDeque<String>,
    watcher_running: bool,
}

pub struct SpeechPlayer {
    backend: Box<dyn TtsBackend>,
    playback: Mutex<Playback>,
    speaking: watch::Sender<bool>,
}

impl SpeechPlayer {
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

    // Utterances play one at a time, in order; interrupt jumps the queue
    pub fn speak(self: &Arc<Self>, text: &str, interrupt: bool) -> Result<u64, std::io::Error> {
        let mut playback = self.lock();
        if interrupt {
            playback.queue.clear();
            stop_active(&mut playback);
        }

        playback.utterance_id += 1;
        let utterance_id = playback.utterance_id;

        if playback.active.is_some() {
            playback.queue.push_back(text.to_string());
            // drop the oldest backlog rather than droning through stale updates
            if playback.queue.len() > MAX_QUEUED_UTTERANCES {
                playback.queue.pop_front();
            }
            return Ok(utterance_id);
        }

        playback.active = Some(self.backend.start(text)?);
        let needs_watcher = !playback.watcher_running;
        playback.watcher_running = true;
        drop(playback);
        self.speaking.send_replace(true);

        // Watcher chains queued utterances and flips `speaking` off at the end
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

    // completion signal; first production caller is banshee.ask_user
    #[allow(dead_code)]
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
                Some(next) => match self.backend.start(&next) {
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

        let first = player.speak("", false).unwrap();
        let second = player.speak("", false).unwrap();
        assert_eq!((first, second), (1, 2));

        player.stop();
        assert!(!player.is_speaking());
    }

    #[tokio::test]
    async fn queued_utterances_drain_and_signal_completion() {
        let player = Arc::new(SpeechPlayer::default());
        let mut speaking = player.subscribe_speaking();
        player.speak("", false).unwrap();
        player.speak("", false).unwrap();

        tokio::time::timeout(Duration::from_secs(3), speaking.wait_for(|s| !s))
            .await
            .expect("playback completion was never signalled")
            .expect("speaking sender dropped");
    }
}

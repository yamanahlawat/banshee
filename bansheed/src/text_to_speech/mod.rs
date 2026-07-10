pub mod sanitizer;

use std::collections::VecDeque;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use tokio::sync::watch;

const MAX_QUEUED_UTTERANCES: usize = 8;

struct Playback {
    utterance_id: u64,
    child: Option<Child>,
    queue: VecDeque<String>,
    watcher_running: bool,
}

pub struct SpeechPlayer {
    playback: Mutex<Playback>,
    speaking: watch::Sender<bool>,
}

impl SpeechPlayer {
    pub fn new() -> Self {
        Self {
            playback: Mutex::new(Playback {
                utterance_id: 0,
                child: None,
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
            kill_child(&mut playback);
        }

        playback.utterance_id += 1;
        let utterance_id = playback.utterance_id;

        if playback.child.is_some() {
            playback.queue.push_back(text.to_string());
            // drop the oldest backlog rather than droning through stale updates
            if playback.queue.len() > MAX_QUEUED_UTTERANCES {
                playback.queue.pop_front();
            }
            return Ok(utterance_id);
        }

        playback.child = Some(Command::new("say").arg(text).spawn()?);
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
        kill_child(&mut playback);
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
            let Some(child) = playback.child.as_mut() else {
                // stopped; a speak arriving before we exit reuses this watcher
                playback.watcher_running = false;
                return;
            };
            // try_wait returns Some(status) once the process has exited
            match child.try_wait() {
                Ok(None) => {}
                Ok(Some(_)) | Err(_) => {
                    playback.child = None;
                    match playback.queue.pop_front() {
                        Some(next) => match Command::new("say").arg(&next).spawn() {
                            Ok(child) => playback.child = Some(child),
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
        }
    }

    fn lock(&self) -> MutexGuard<'_, Playback> {
        self.playback
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for SpeechPlayer {
    fn default() -> Self {
        Self::new()
    }
}

// kill only signals; wait reaps the zombie process entry
fn kill_child(playback: &mut Playback) {
    if let Some(mut child) = playback.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utterance_ids_increment_and_stop_clears() {
        let player = Arc::new(SpeechPlayer::new());
        assert!(!player.is_speaking());

        let first = player.speak("", false).unwrap();
        let second = player.speak("", false).unwrap();
        assert_eq!((first, second), (1, 2));

        player.stop();
        assert!(!player.is_speaking());
    }

    #[tokio::test]
    async fn queued_utterances_drain_and_signal_completion() {
        let player = Arc::new(SpeechPlayer::new());
        let mut speaking = player.subscribe_speaking();
        player.speak("", false).unwrap();
        player.speak("", false).unwrap();

        tokio::time::timeout(Duration::from_secs(3), speaking.wait_for(|s| !s))
            .await
            .expect("playback completion was never signalled")
            .expect("speaking sender dropped");
    }
}

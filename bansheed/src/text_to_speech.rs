pub mod local;
pub mod output;
pub mod pronunciation;
pub mod remote;
pub mod sanitizer;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use banshee_common::{KokoroTTSConfig, error::BansheeError};
use tokio::sync::watch;

use crate::config::{Provider, TTSConfig, TTSFallback};
use local::kokoro::{KokoroBackend, KokoroEngine};
use local::say::SayBackend;
use output::Output;
use remote::openai_compatible::RemoteSpeechBackend;

/// Unmeasured. A bound against growth, not a latency target.
const MAX_QUEUED_UTTERANCES: usize = 8;

/// The end of speech is noticed this late at worst, which is under the cue that
/// follows it.
const PLAYBACK_POLL: Duration = Duration::from_millis(50);

/// What a speech backend has to say about one utterance. The backend cannot
/// reach the player the state owns or the cue channel, so it sends this and a
/// thread in `daemon.rs` acts on it.
#[derive(Debug)]
pub enum Fault {
    Failed(String),
    Played,
}

/// Drains the channel until every sender is gone. Runs on a thread of its own,
/// started once the state exists.
pub fn drain_faults(
    state: Arc<crate::state::DaemonState>,
    cues: crate::audio::cues::Cues,
    faults: std::sync::mpsc::Receiver<Fault>,
) {
    for fault in faults {
        match fault {
            Fault::Failed(reason) => {
                log::error!("the reply was not spoken: {reason}");
                cues.send(crate::audio::cues::Cue::Error);
                state.set_last_speech_error(Some(reason));
            }
            Fault::Played => state.set_last_speech_error(None),
        }
    }
}

// A backend starts one utterance at a time; SpeechPlayer serializes them
pub trait TtsBackend: Send + Sync {
    /// `Rejected` for a voice this backend cannot take; anything else is the
    /// backend failing to start.
    fn start(
        &self,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Box<dyn ActiveUtterance>, BansheeError>;

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

/// Which speaker started, said by the branch that built it. `Configured` is the
/// one `[tts]` asks for, with the voice it speaks in; `Fallback` is the OS voice
/// or silence, which speaks in whatever the OS is set to and sends no text out.
pub enum Speaker {
    Configured(String),
    Fallback,
}

impl Speaker {
    /// Whether the speaker `[tts]` names is the one running.
    pub fn started(&self) -> bool {
        matches!(self, Speaker::Configured(_))
    }

    pub fn voice(self) -> Option<String> {
        match self {
            Speaker::Configured(voice) => Some(voice),
            Speaker::Fallback => None,
        }
    }
}

/// `fault` holds the reason the speaker `[tts]` names did not start. It has no
/// utterance in flight, so the fault channel never carries that reason.
pub struct Selection {
    pub backend: Box<dyn TtsBackend>,
    pub speaker: Speaker,
    pub fault: Option<String>,
}

/// The selection `[tts]` asks for. `faults` is where a backend reports an
/// utterance it could not speak.
pub fn select_backend(
    tts_config: &TTSConfig,
    faults: std::sync::mpsc::Sender<Fault>,
    output: Arc<Output>,
) -> Result<Selection, BansheeError> {
    match tts_config.provider {
        Provider::Local => select_local_backend(tts_config, faults, output),
        Provider::Remote => {
            // Not propagated: a credentials file that will not parse holds no
            // key the speaker can use, so it takes the path a missing key takes
            // and the daemon stays up.
            let api_key = crate::credentials::Credentials::load().map(|credentials| {
                credentials
                    .key(crate::credentials::RemoteKey::Tts)
                    .map(str::to_string)
            });
            select_remote_backend(tts_config, faults, api_key, output)
        }
    }
}

/// `api_key` is the key, its absence, or the fault that hid it.
fn select_remote_backend(
    tts_config: &TTSConfig,
    faults: std::sync::mpsc::Sender<Fault>,
    api_key: Result<Option<String>, BansheeError>,
    output: Arc<Output>,
) -> Result<Selection, BansheeError> {
    let built = match api_key {
        Err(unreadable) => Err(unreadable),
        Ok(None) => Err(BansheeError::Other(
            crate::credentials::RemoteKey::Tts.no_key(),
        )),
        Ok(Some(key)) => RemoteSpeechBackend::new(
            &tts_config.remote,
            key,
            tts_config.speed,
            fallback_for(tts_config),
            output,
            faults.clone(),
        ),
    };
    match built {
        Ok(backend) => {
            log::info!(
                "TTS: {} as {}",
                tts_config.remote.host(),
                tts_config.remote.voice
            );
            let voice = tts_config.remote.voice.clone();
            Ok(Selection {
                backend: Box::new(backend),
                speaker: Speaker::Configured(voice),
                fault: None,
            })
        }
        // The daemon stays up: a speaker is not the recording pipeline, and an
        // agent's question still has to be heard.
        Err(error) => {
            let reason = error.to_string();
            log::warn!("The remote speaker will not start: {reason}");
            let backend: Box<dyn TtsBackend> = match tts_config.fallback {
                TTSFallback::System => Box::new(SayBackend),
                TTSFallback::None => Box::new(Silent {
                    reason: reason.clone(),
                }),
            };
            Ok(Selection {
                backend,
                speaker: Speaker::Fallback,
                fault: Some(reason),
            })
        }
    }
}

/// What speaks when the server does not. `tts.fallback = "none"` asks for
/// silence, and silence needs no backend.
fn fallback_for(tts_config: &TTSConfig) -> Option<Arc<dyn TtsBackend>> {
    match tts_config.fallback {
        TTSFallback::System => Some(Arc::new(SayBackend)),
        TTSFallback::None => None,
    }
}

/// Answers every utterance with the reason there is no speaker, so `speak`
/// fails with it rather than reporting an utterance nobody will hear.
struct Silent {
    reason: String,
}

impl TtsBackend for Silent {
    fn start(
        &self,
        _text: &str,
        _voice: Option<&str>,
    ) -> Result<Box<dyn ActiveUtterance>, BansheeError> {
        Err(BansheeError::Other(self.reason.clone()))
    }
}

/// Kokoro, or the OS voice when `tts.fallback` allows it and Kokoro cannot load.
/// The output opens here rather than in `daemon.rs`: a machine with no output
/// device has to reach the system voice, not stop the daemon.
fn select_local_backend(
    tts_config: &TTSConfig,
    faults: std::sync::mpsc::Sender<Fault>,
    output: Arc<Output>,
) -> Result<Selection, BansheeError> {
    let kokoro_config = KokoroTTSConfig::new(&tts_config.voice);
    let loaded = KokoroEngine::new(&kokoro_config, tts_config.speed)
        .map(|engine| KokoroBackend::new(engine, output, faults));
    match loaded {
        Ok(backend) => {
            log::info!("TTS: Kokoro (voice {})", tts_config.voice);
            Ok(Selection {
                backend: Box::new(backend),
                speaker: Speaker::Configured(tts_config.voice.clone()),
                fault: None,
            })
        }
        Err(e) => match tts_config.fallback {
            TTSFallback::System => {
                log::warn!("Kokoro unavailable, falling back to system TTS: {e}");
                Ok(Selection {
                    backend: Box::new(SayBackend),
                    speaker: Speaker::Fallback,
                    fault: Some(e.to_string()),
                })
            }
            TTSFallback::None => Err(e),
        },
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
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
    ) -> Result<u64, BansheeError> {
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
            self.publish(&playback);
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

        match self.backend.start(text, voice) {
            Ok(started) => playback.active = Some(started),
            // An interrupt stopped whatever was speaking and nothing replaced
            // it. The hotkey listener discards every chunk it captures while
            // this reads true, so a reply that never starts would leave the
            // daemon deaf.
            Err(error) => {
                self.publish(&playback);
                return Err(error);
            }
        }
        let needs_watcher = !playback.watcher_running;
        playback.watcher_running = true;
        self.publish(&playback);
        drop(playback);

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
        self.publish(&playback);
    }

    pub fn is_speaking(&self) -> bool {
        *self.speaking.borrow()
    }

    pub fn subscribe_speaking(&self) -> watch::Receiver<bool> {
        self.speaking.subscribe()
    }

    fn watch_playback(&self) {
        loop {
            thread::sleep(PLAYBACK_POLL);
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
                        log::error!("Failed to speak queued utterance: {e}");
                        playback.queue.clear();
                        playback.watcher_running = false;
                        self.publish(&playback);
                        return;
                    }
                },
                None => {
                    playback.watcher_running = false;
                    self.publish(&playback);
                    return;
                }
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, Playback> {
        lock(&self.playback)
    }

    fn publish(&self, playback: &Playback) {
        self.speaking.send_if_modified(|speaking| {
            std::mem::replace(speaking, playback.active.is_some()) != playback.active.is_some()
        });
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

    fn refusal_of(backend: &dyn TtsBackend) -> String {
        // `expect_err` would ask a boxed utterance for a Debug line, and a
        // trait object has none
        match backend.start("Anything.", None) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("this speaker must refuse the utterance"),
        }
    }

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

    #[test]
    fn a_remote_speaker_with_no_key_falls_back_and_says_why() {
        let tts = crate::config::TTSConfig {
            provider: crate::config::Provider::Remote,
            remote: crate::config::RemoteTtsConfig {
                voice: "marin".to_string(),
                ..Default::default()
            },
            fallback: crate::config::TTSFallback::System,
            ..Default::default()
        };
        let (faults, _reports) = std::sync::mpsc::channel();

        let selected =
            select_remote_backend(&tts, faults, Ok(None), Arc::new(Output::silent())).unwrap();
        assert!(
            !selected.speaker.started(),
            "the system voice is not the speaker the config names"
        );
        let reason = selected.fault.expect("the missing key must be reported");
        assert!(
            reason.contains("banshee config set tts.remote.api_key"),
            "{reason}"
        );
    }

    #[test]
    fn a_credentials_file_that_will_not_parse_falls_back_and_says_why() {
        let tts = crate::config::TTSConfig {
            provider: crate::config::Provider::Remote,
            remote: crate::config::RemoteTtsConfig {
                voice: "marin".to_string(),
                ..Default::default()
            },
            fallback: crate::config::TTSFallback::System,
            ..Default::default()
        };
        let (faults, _reports) = std::sync::mpsc::channel();
        let unreadable = Err(BansheeError::Other(
            "/tmp/credentials.toml does not parse; fix it or delete it and set the keys again"
                .to_string(),
        ));

        let selected =
            select_remote_backend(&tts, faults, unreadable, Arc::new(Output::silent())).unwrap();
        assert!(
            !selected.speaker.started(),
            "the system voice is not the speaker the config names"
        );
        let reason = selected.fault.expect("the parse fault must be reported");
        assert!(reason.contains("does not parse"), "{reason}");
    }

    #[test]
    fn with_no_fallback_every_utterance_fails_with_the_reason() {
        let tts = crate::config::TTSConfig {
            provider: crate::config::Provider::Remote,
            remote: crate::config::RemoteTtsConfig {
                voice: "marin".to_string(),
                ..Default::default()
            },
            fallback: crate::config::TTSFallback::None,
            ..Default::default()
        };
        let (faults, _reasons) = std::sync::mpsc::channel();

        let selected =
            select_remote_backend(&tts, faults, Ok(None), Arc::new(Output::silent())).unwrap();
        let reason = refusal_of(selected.backend.as_ref());
        assert!(
            reason.contains("banshee config set tts.remote.api_key"),
            "{reason}"
        );
    }

    #[test]
    fn a_remote_speaker_with_no_voice_names_the_voice_key() {
        let tts = crate::config::TTSConfig {
            provider: crate::config::Provider::Remote,
            fallback: crate::config::TTSFallback::None,
            ..Default::default()
        };
        let (faults, _reasons) = std::sync::mpsc::channel();

        let selected = select_remote_backend(
            &tts,
            faults,
            Ok(Some("sk-test".to_string())),
            Arc::new(Output::silent()),
        )
        .unwrap();
        let reason = refusal_of(selected.backend.as_ref());
        assert!(reason.contains("tts.remote.voice"), "{reason}");
    }

    #[test]
    fn a_kokoro_start_failure_falls_back_and_says_why() {
        let tts = crate::config::TTSConfig {
            voice: "banshee-test-nonexistent-voice".to_string(),
            fallback: crate::config::TTSFallback::System,
            ..Default::default()
        };
        let kokoro_config = KokoroTTSConfig::new(&tts.voice);
        let expected = KokoroEngine::new(&kokoro_config, tts.speed)
            .err()
            .expect("a nonexistent voice must not load")
            .to_string();

        let selected = select_local_backend(
            &tts,
            std::sync::mpsc::channel().0,
            Arc::new(Output::silent()),
        )
        .unwrap();
        assert!(
            !selected.speaker.started(),
            "the system voice is not the speaker the config names"
        );
        let reason = selected
            .fault
            .expect("the Kokoro start failure must be reported");
        assert_eq!(reason, expected);
    }

    /// A voice that goes missing between two replies, which `installed` refuses
    /// by name.
    struct RefusesAfterFirst(std::sync::atomic::AtomicUsize);

    struct NeverEnds;

    impl ActiveUtterance for NeverEnds {
        fn is_finished(&mut self) -> bool {
            false
        }
        fn stop(&mut self) {}
    }

    impl TtsBackend for RefusesAfterFirst {
        fn start(
            &self,
            _text: &str,
            _voice: Option<&str>,
        ) -> Result<Box<dyn ActiveUtterance>, BansheeError> {
            if self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
                Ok(Box::new(NeverEnds))
            } else {
                Err(BansheeError::Rejected(
                    "Voice af_sky is not installed on this machine.".into(),
                ))
            }
        }
    }

    #[tokio::test]
    async fn a_refused_interrupt_leaves_nothing_speaking() {
        let player = Arc::new(SpeechPlayer::new(Box::new(RefusesAfterFirst(
            std::sync::atomic::AtomicUsize::new(0),
        ))));
        let mut speaking = player.subscribe_speaking();

        player.speak("The first reply.", false, None).unwrap();
        assert!(player.is_speaking(), "the first reply is playing");

        player
            .speak("The second reply.", true, Some("af_sky"))
            .expect_err("the backend refuses the voice");

        tokio::time::timeout(Duration::from_secs(3), speaking.wait_for(|s| !s))
            .await
            .expect("a reply that never started left the daemon deaf to the microphone")
            .expect("speaking sender dropped");
    }
}

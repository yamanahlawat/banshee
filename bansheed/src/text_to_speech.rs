pub mod local;
pub mod output;
pub mod pronunciation;
pub mod remote;
pub mod sanitizer;

use std::collections::VecDeque;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread;
use std::time::Duration;

use banshee_common::error::BansheeError;
use tokio::sync::watch;

use crate::audio::cues::{Cues, Reason, ReasonCode, Signal};
use crate::config::{Provider, TTSConfig, TTSFallback};
use local::{Engine, OnFirstUse, local_backend, local_on_first_use};
use output::Output;
use remote::openai_compatible::RemoteSpeechBackend;

/// The end of speech is noticed this late at worst, which is under the cue that
/// follows it.
const PLAYBACK_POLL: Duration = Duration::from_millis(50);

/// What a speech backend has to say about one utterance. The backend cannot
/// reach the player the state owns or the cue channel, so it sends this and a
/// thread in `daemon.rs` acts on it.
#[derive(Debug)]
pub enum Fault {
    Failed(String),
    /// The speaker `[tts]` names failed before its first sample, and the
    /// fallback voice took the sentence. The reason is that speaker's.
    SpokenByFallback(String),
    /// An utterance played its first sample.
    Played,
    /// The local voice `[tts]` names loaded after the daemon started.
    Loaded,
}

impl Fault {
    fn log_line(&self) -> Option<(log::Level, String)> {
        match self {
            Fault::Failed(reason) => Some((
                log::Level::Error,
                format!("the reply was not spoken: {reason}"),
            )),
            Fault::SpokenByFallback(reason) => Some((
                log::Level::Warn,
                format!("the fallback voice took over the reply: {reason}"),
            )),
            Fault::Played | Fault::Loaded => None,
        }
    }

    pub fn log(&self) {
        if let Some((level, line)) = self.log_line() {
            log::log!(level, "{line}");
        }
    }
}

/// The fault channel as one speaker holds it. The voice standing in for a
/// remote speaker reports what fails and nothing else, so its playing clears no
/// reason the remote speaker left and its load does not start that speaker.
#[derive(Clone)]
pub struct Faults {
    sender: Sender<Fault>,
    reports_progress: bool,
}

impl Faults {
    pub fn new(sender: Sender<Fault>) -> Self {
        Self {
            sender,
            reports_progress: true,
        }
    }

    pub(crate) fn standing_in(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            reports_progress: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn reports_progress(&self) -> bool {
        self.reports_progress
    }

    pub fn send(&self, fault: Fault) {
        if self.reports_progress || !matches!(fault, Fault::Played | Fault::Loaded) {
            let _ = self.sender.send(fault);
        }
    }
}

/// Drains the channel until every sender is gone. Runs on a thread of its own,
/// started once the state exists.
pub fn drain_faults(
    state: Arc<crate::state::DaemonState>,
    cues: Cues,
    faults: std::sync::mpsc::Receiver<Fault>,
) {
    let failed = |reason: String| {
        cues.emit(Signal::Error {
            reason: Reason::new(ReasonCode::SpeechFailed, None),
            target: None,
        });
        state.set_last_speech_error(Some(reason));
    };
    for fault in faults {
        fault.log();
        match fault {
            Fault::Played => state.set_last_speech_error(None),
            Fault::Loaded => state.set_speaker_started(true),
            Fault::Failed(reason) | Fault::SpokenByFallback(reason) => failed(reason),
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

    /// `start` for a caller that holds no lock, so a backend that loads on
    /// first use may load here rather than on a thread of its own.
    fn start_unlocked(
        &self,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Box<dyn ActiveUtterance>, BansheeError> {
        self.start(text, voice)
    }

    /// A live `[tts]` change. Answers the voice utterances now speak in, or
    /// `None` when the backend cannot honour the change, as silence cannot.
    fn reconfigure(&self, _tts: &TTSConfig) -> Option<String> {
        None
    }
}

impl<T: TtsBackend + ?Sized> TtsBackend for Arc<T> {
    fn start(
        &self,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Box<dyn ActiveUtterance>, BansheeError> {
        (**self).start(text, voice)
    }

    fn start_unlocked(
        &self,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Box<dyn ActiveUtterance>, BansheeError> {
        (**self).start_unlocked(text, voice)
    }

    fn reconfigure(&self, tts: &TTSConfig) -> Option<String> {
        (**self).reconfigure(tts)
    }
}

pub trait ActiveUtterance: Send {
    fn is_finished(&mut self) -> bool;
    fn stop(&mut self);
}

/// Which speaker started, said by the branch that built it. `Configured` is the
/// one `[tts]` asks for, with the voice it speaks in; `Fallback` is a local
/// voice still to load, or silence behind a remote speaker that did not start.
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
    models: &Path,
    faults: Sender<Fault>,
    output: Arc<Output>,
) -> Selection {
    let faults = Faults::new(faults);
    match tts_config.provider {
        Provider::Local => select_local_backend(tts_config, models, faults, output),
        Provider::Remote => {
            // Not propagated: a credentials file that will not parse holds no
            // key the speaker can use, so it takes the path a missing key takes
            // and the daemon stays up.
            let api_key = crate::credentials::Credentials::load().map(|credentials| {
                credentials
                    .key(crate::credentials::RemoteKey::Tts)
                    .map(str::to_string)
            });
            select_remote_backend(tts_config, models, faults, api_key, output)
        }
    }
}

/// `api_key` is the key, its absence, or the fault that hid it.
fn select_remote_backend(
    tts_config: &TTSConfig,
    models: &Path,
    faults: Faults,
    api_key: Result<Option<String>, BansheeError>,
    output: Arc<Output>,
) -> Selection {
    let fallback = fallback_for(tts_config, models, Arc::clone(&output), &faults);
    let built = match api_key {
        Err(unreadable) => Err(unreadable),
        Ok(None) => Err(BansheeError::Other(
            crate::credentials::RemoteKey::Tts.no_key(),
        )),
        Ok(Some(key)) => RemoteSpeechBackend::new(
            &tts_config.remote,
            key,
            fallback.clone().map(|f| f as Arc<dyn TtsBackend>),
            output,
            faults,
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
            Selection {
                backend: Box::new(backend),
                speaker: Speaker::Configured(voice),
                fault: None,
            }
        }
        // The daemon stays up: a speaker is not the recording pipeline, and an
        // agent's question still has to be heard.
        Err(error) => {
            let reason = error.to_string();
            log::warn!("The remote speaker will not start: {reason}");
            let backend: Box<dyn TtsBackend> = match fallback {
                Some(fallback) => Box::new(fallback),
                None => Box::new(Silent {
                    reason: reason.clone(),
                }),
            };
            Selection {
                backend,
                speaker: Speaker::Fallback,
                fault: Some(reason),
            }
        }
    }
}

/// What speaks when the server does not. `tts.fallback = "none"` asks for
/// silence, and silence needs no backend.
fn fallback_for(
    tts_config: &TTSConfig,
    models: &Path,
    output: Arc<Output>,
    faults: &Faults,
) -> Option<Arc<OnFirstUse>> {
    match tts_config.fallback {
        TTSFallback::Local => Some(Arc::new(local_on_first_use(
            models,
            &tts_config.voice,
            output,
            faults.standing_in(),
        ))),
        TTSFallback::None => None,
    }
}

/// A remote speaker that did not start, under `tts.fallback = "none"`. Answers
/// every utterance with the reason, so `speak` fails with it rather than
/// reporting an utterance nobody will hear.
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

/// The local voice. One that will not load is tried again by each utterance,
/// whatever `tts.fallback` says: the speaker is not the recording pipeline,
/// and its files may still be arriving.
fn select_local_backend(
    tts_config: &TTSConfig,
    models: &Path,
    faults: Faults,
    output: Arc<Output>,
) -> Selection {
    match local_backend(
        models,
        &tts_config.voice,
        Arc::clone(&output),
        faults.clone(),
    ) {
        Ok(backend) => {
            log::info!(
                "TTS: {} (voice {})",
                Engine::of(&tts_config.voice).name(),
                tts_config.voice
            );
            Selection {
                backend,
                speaker: Speaker::Configured(tts_config.voice.clone()),
                fault: None,
            }
        }
        Err(error) => {
            let reason = error.to_string();
            log::warn!("The local voice will not load: {reason}");
            Selection {
                backend: Box::new(local_on_first_use(
                    models,
                    &tts_config.voice,
                    output,
                    faults,
                )),
                speaker: Speaker::Fallback,
                fault: Some(reason),
            }
        }
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn lock_read<T>(value: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    value
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn lock_write<T>(value: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    value
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Line {
    utterance_id: u64,
    text: String,
    voice: Option<String>,
}

struct Hold(Option<u64>);

impl Hold {
    fn keeps_back(&self, utterance_id: u64) -> bool {
        self.0.is_some_and(|after| utterance_id > after)
    }

    fn is_question(&self, utterance_id: u64) -> bool {
        self.0 == Some(utterance_id)
    }
}

struct Active {
    utterance_id: u64,
    utterance: Box<dyn ActiveUtterance>,
}

struct Playback {
    utterance_id: u64,
    last_started: u64,
    active: Option<Active>,
    queue: VecDeque<Line>,
    watcher_running: bool,
    hold: Hold,
}

impl Playback {
    fn stop_active(&mut self) {
        if let Some(mut active) = self.active.take() {
            active.utterance.stop();
        }
    }

    fn begin(&mut self, utterance_id: u64, utterance: Box<dyn ActiveUtterance>) {
        self.last_started = utterance_id;
        self.active = Some(Active {
            utterance_id,
            utterance,
        });
    }

    fn drop_unheld(&mut self) {
        self.queue
            .retain(|line| self.hold.keeps_back(line.utterance_id));
    }

    fn drop_before_question(&mut self) {
        self.queue.retain(|line| {
            self.hold.keeps_back(line.utterance_id) || self.hold.is_question(line.utterance_id)
        });
    }
}

pub struct SpeechPlayer {
    backend: Box<dyn TtsBackend>,
    playback: Mutex<Playback>,
    playing: watch::Sender<Option<u64>>,
}

impl SpeechPlayer {
    /// What a live `[tts]` write reaches. An utterance already speaking keeps
    /// the voice it started with.
    pub fn reconfigure(&self, tts: &TTSConfig) -> Option<String> {
        self.backend.reconfigure(tts)
    }

    pub fn new(backend: Box<dyn TtsBackend>) -> Self {
        Self {
            backend,
            playback: Mutex::new(Playback {
                utterance_id: 0,
                last_started: 0,
                active: None,
                queue: VecDeque::new(),
                watcher_running: false,
                hold: Hold(None),
            }),
            playing: watch::channel(None).0,
        }
    }

    pub fn speak(
        self: &Arc<Self>,
        text: &str,
        interrupt: bool,
        voice: Option<&str>,
    ) -> Result<u64, BansheeError> {
        self.submit(text, interrupt, voice, false)
    }

    /// Speaks `text` and, in the same step, keeps back every utterance after
    /// it until `release`. What is already playing or queued up to it still plays.
    pub fn speak_and_hold(self: &Arc<Self>, text: &str) -> Result<u64, BansheeError> {
        self.submit(text, false, None, true)
    }

    pub fn has_started(&self, utterance_id: u64) -> bool {
        self.lock().last_started >= utterance_id
    }

    /// Stops `utterance_id` if it still plays, drops the lines queued before
    /// the question, and starts the question.
    pub fn skip_to_question(self: &Arc<Self>, utterance_id: u64) {
        let mut playback = self.lock();
        if playback
            .active
            .as_ref()
            .is_none_or(|active| active.utterance_id != utterance_id)
        {
            return;
        }
        playback.stop_active();
        playback.drop_before_question();
        if self.start_next(&mut playback) {
            self.publish_and_watch(playback);
        } else {
            self.publish(&playback);
        }
    }

    fn submit(
        self: &Arc<Self>,
        text: &str,
        interrupt: bool,
        voice: Option<&str>,
        hold: bool,
    ) -> Result<u64, BansheeError> {
        let normalized = pronunciation::normalize(text);
        let text = normalized.as_str();
        let mut playback = self.lock();
        if interrupt {
            playback.stop_active();
            playback.queue.clear();
        }

        playback.utterance_id += 1;
        let utterance_id = playback.utterance_id;
        if hold {
            playback.hold = Hold(Some(utterance_id));
        }

        // normalize can leave nothing speakable (e.g. input was only underscores);
        // keep the id sequence but start no playback
        if text.is_empty() {
            self.publish(&playback);
            return Ok(utterance_id);
        }

        if playback.active.is_some() || playback.hold.keeps_back(utterance_id) {
            playback.queue.push_back(Line {
                utterance_id,
                text: text.to_string(),
                voice: voice.map(str::to_string),
            });
            // The interrupt above may have stopped what played
            self.publish(&playback);
            return Ok(utterance_id);
        }

        match self.backend.start(text, voice) {
            Ok(utterance) => playback.begin(utterance_id, utterance),
            // An interrupt stopped whatever was speaking and nothing replaced
            // it. The hotkey listener discards every chunk it captures while
            // this reads true, so a reply that never starts would leave the
            // daemon deaf.
            Err(error) => {
                self.publish(&playback);
                return Err(error);
            }
        }
        self.publish_and_watch(playback);
        Ok(utterance_id)
    }

    pub fn stop(&self) {
        let mut playback = self.lock();
        playback.stop_active();
        playback.drop_unheld();
        self.publish(&playback);
    }

    /// Ends the hold. A question still queued is dropped: no session waits for
    /// its answer.
    pub fn release(self: &Arc<Self>) {
        let mut playback = self.lock();
        let hold = std::mem::replace(&mut playback.hold, Hold(None));
        playback
            .queue
            .retain(|line| !hold.is_question(line.utterance_id));
        if playback.active.is_none() && self.start_next(&mut playback) {
            self.publish_and_watch(playback);
        }
    }

    pub fn is_speaking(&self) -> bool {
        self.playing.borrow().is_some()
    }

    /// The id of the utterance on the speaker. It changes as each one starts
    /// and ends.
    pub fn subscribe_playing(&self) -> watch::Receiver<Option<u64>> {
        self.playing.subscribe()
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
            if !active.utterance.is_finished() {
                continue;
            }
            playback.active = None;
            let next = self.start_next(&mut playback);
            self.publish(&playback);
            if !next {
                playback.watcher_running = false;
                return;
            }
        }
    }

    fn start_next(&self, playback: &mut Playback) -> bool {
        while let Some(line) = playback
            .queue
            .pop_front_if(|line| !playback.hold.keeps_back(line.utterance_id))
        {
            match self.backend.start(&line.text, line.voice.as_deref()) {
                Ok(utterance) => {
                    playback.begin(line.utterance_id, utterance);
                    return true;
                }
                Err(e) => {
                    log::error!("Failed to speak queued utterance: {e}");
                    playback.drop_before_question();
                }
            }
        }
        false
    }

    fn publish_and_watch(self: &Arc<Self>, mut playback: MutexGuard<'_, Playback>) {
        let needs_watcher = !playback.watcher_running;
        playback.watcher_running = true;
        self.publish(&playback);
        drop(playback);

        if needs_watcher {
            let player = Arc::clone(self);
            thread::spawn(move || player.watch_playback());
        }
    }

    fn lock(&self) -> MutexGuard<'_, Playback> {
        lock(&self.playback)
    }

    fn publish(&self, playback: &Playback) {
        let playing = playback.active.as_ref().map(|active| active.utterance_id);
        self.playing
            .send_if_modified(|was| std::mem::replace(was, playing) != playing);
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
    fn a_reply_the_fallback_took_is_logged_as_taken_not_as_unspoken() {
        let taken = Fault::SpokenByFallback("the remote speaker refused the key".into());
        assert_eq!(
            taken.log_line(),
            Some((
                log::Level::Warn,
                "the fallback voice took over the reply: the remote speaker refused the key"
                    .to_string()
            ))
        );
        let failed = Fault::Failed("the remote speaker refused the key".into());
        assert_eq!(
            failed.log_line(),
            Some((
                log::Level::Error,
                "the reply was not spoken: the remote speaker refused the key".to_string()
            ))
        );
        assert_eq!(Fault::Played.log_line(), None);
    }

    #[test]
    fn utterance_ids_increment() {
        let player = Arc::new(crate::test_support::quiet_speech());
        assert!(!player.is_speaking());

        let first = player.speak("", false, None).unwrap();
        let second = player.speak("", false, None).unwrap();
        assert_eq!((first, second), (1, 2));
    }

    #[tokio::test]
    async fn queued_utterances_drain_and_signal_completion() {
        let player = Arc::new(crate::test_support::quiet_speech());
        let mut playing = player.subscribe_playing();
        player.speak("", false, None).unwrap();
        player.speak("", false, None).unwrap();

        tokio::time::timeout(Duration::from_secs(3), playing.wait_for(Option::is_none))
            .await
            .expect("playback completion was never signalled")
            .expect("playing sender dropped");
    }

    fn remote_without_a_key(
        fallback: TTSFallback,
        api_key: Result<Option<String>, BansheeError>,
    ) -> Selection {
        let tts = TTSConfig {
            provider: Provider::Remote,
            remote: crate::config::RemoteTtsConfig {
                voice: "marin".to_string(),
                ..Default::default()
            },
            fallback,
            ..Default::default()
        };
        select_remote_backend(
            &tts,
            &crate::test_support::unique_scratch("remote-models"),
            Faults::new(std::sync::mpsc::channel().0),
            api_key,
            Arc::new(Output::silent()),
        )
    }

    #[test]
    fn a_remote_speaker_with_no_key_falls_back_and_says_why() {
        let selected = remote_without_a_key(TTSFallback::Local, Ok(None));
        assert!(
            !selected.speaker.started(),
            "the local voice is not the speaker the config names"
        );
        let reason = selected.fault.expect("the missing key must be reported");
        assert!(
            reason.contains("banshee config set tts.remote.api_key"),
            "{reason}"
        );
    }

    #[test]
    fn a_credentials_file_that_will_not_parse_falls_back_and_says_why() {
        let unreadable = Err(BansheeError::Other(
            "/tmp/credentials.toml does not parse; fix it or delete it and set the keys again"
                .to_string(),
        ));
        let selected = remote_without_a_key(TTSFallback::Local, unreadable);
        assert!(
            !selected.speaker.started(),
            "the local voice is not the speaker the config names"
        );
        let reason = selected.fault.expect("the parse fault must be reported");
        assert!(reason.contains("does not parse"), "{reason}");
    }

    #[test]
    fn with_no_fallback_every_utterance_fails_with_the_reason() {
        let selected = remote_without_a_key(TTSFallback::None, Ok(None));
        let reason = refusal_of(selected.backend.as_ref());
        assert!(
            reason.contains("banshee config set tts.remote.api_key"),
            "{reason}"
        );
    }

    #[test]
    fn a_remote_speaker_with_no_voice_names_the_voice_key() {
        let tts = TTSConfig {
            provider: Provider::Remote,
            fallback: TTSFallback::None,
            ..Default::default()
        };

        let selected = select_remote_backend(
            &tts,
            &crate::test_support::unique_scratch("remote-models"),
            Faults::new(std::sync::mpsc::channel().0),
            Ok(Some("sk-test".to_string())),
            Arc::new(Output::silent()),
        );
        let reason = refusal_of(selected.backend.as_ref());
        assert!(reason.contains("tts.remote.voice"), "{reason}");
    }

    #[test]
    fn the_local_fallback_stands_in_and_the_local_speaker_does_not() {
        let tts = TTSConfig::default();
        let models = crate::test_support::companion_models();
        let faults = Faults::new(std::sync::mpsc::channel().0);

        let fallback = fallback_for(&tts, &models, Arc::new(Output::silent()), &faults)
            .expect("a local fallback");
        assert!(!fallback.faults().reports_progress());
        assert!(faults.reports_progress());
    }

    /// The companion in an empty models folder, which is a first run before
    /// `banshee setup` or a download cut short.
    fn unloadable(fallback: TTSFallback) -> Selection {
        let tts = TTSConfig {
            voice: banshee_common::COMPANION_VOICE.to_string(),
            fallback,
            ..Default::default()
        };
        select_local_backend(
            &tts,
            &crate::test_support::unique_scratch("empty-models"),
            Faults::new(std::sync::mpsc::channel().0),
            Arc::new(Output::silent()),
        )
    }

    fn refused_with_a_reason(selected: Selection) {
        assert!(
            !selected.speaker.started(),
            "a voice that did not load is not the speaker the config names"
        );
        let reason = selected.fault.expect("the load failure must be reported");
        assert!(reason.contains("banshee setup"), "{reason}");
        assert_eq!(refusal_of(selected.backend.as_ref()), reason);
    }

    #[test]
    fn a_local_voice_that_will_not_load_keeps_the_daemon_up_with_a_reason() {
        refused_with_a_reason(unloadable(TTSFallback::Local));
    }

    #[test]
    fn none_also_stays_up() {
        refused_with_a_reason(unloadable(TTSFallback::None));
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

    const REFUSED: &str = "A line the backend refuses.";

    /// Keeps each line it starts and refuses `REFUSED`. Every utterance plays
    /// until `ends` goes true.
    struct Gated {
        started: crate::test_support::SpokenLines,
        ends: Arc<std::sync::atomic::AtomicBool>,
    }

    struct UntilEnds(Arc<std::sync::atomic::AtomicBool>);

    impl ActiveUtterance for UntilEnds {
        fn is_finished(&mut self) -> bool {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn stop(&mut self) {}
    }

    impl TtsBackend for Gated {
        fn start(
            &self,
            text: &str,
            _voice: Option<&str>,
        ) -> Result<Box<dyn ActiveUtterance>, BansheeError> {
            if text == REFUSED {
                return Err(BansheeError::Other("refused".to_string()));
            }
            self.started.lock().unwrap().push(text.to_string());
            Ok(Box::new(UntilEnds(Arc::clone(&self.ends))))
        }
    }

    fn gated() -> (
        Arc<SpeechPlayer>,
        crate::test_support::SpokenLines,
        Arc<std::sync::atomic::AtomicBool>,
    ) {
        let started: crate::test_support::SpokenLines = Arc::default();
        let ends: Arc<std::sync::atomic::AtomicBool> = Arc::default();
        let player = Arc::new(SpeechPlayer::new(Box::new(Gated {
            started: Arc::clone(&started),
            ends: Arc::clone(&ends),
        })));
        (player, started, ends)
    }

    async fn falls_silent(player: &SpeechPlayer) {
        let mut playing = player.subscribe_playing();
        tokio::time::timeout(Duration::from_secs(10), playing.wait_for(Option::is_none))
            .await
            .expect("playback never fell silent")
            .expect("playing sender dropped");
    }

    fn started_lines(started: &Mutex<Vec<String>>) -> Vec<String> {
        started.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn a_line_sent_after_the_question_waits_for_the_release() {
        let (player, started, ends) = gated();
        ends.store(true, std::sync::atomic::Ordering::SeqCst);
        player.speak_and_hold("The question.").unwrap();
        falls_silent(&player).await;

        player.speak("Agent B reports.", false, None).unwrap();
        assert_eq!(started_lines(&started), ["The question."]);
        assert!(
            !player.is_speaking(),
            "held speech must not deafen the microphone"
        );

        player.release();
        assert_eq!(
            started_lines(&started),
            ["The question.", "Agent B reports."]
        );
    }

    #[tokio::test]
    async fn lines_queued_up_to_the_question_play_and_the_rest_wait() {
        let (player, started, ends) = gated();
        player
            .speak("Agent C was mid-sentence.", false, None)
            .unwrap();
        player.speak_and_hold("The question.").unwrap();
        player.speak("Agent B reports.", false, None).unwrap();

        ends.store(true, std::sync::atomic::Ordering::SeqCst);
        falls_silent(&player).await;
        assert_eq!(
            started_lines(&started),
            ["Agent C was mid-sentence.", "The question."]
        );

        player.release();
        assert_eq!(
            started_lines(&started),
            [
                "Agent C was mid-sentence.",
                "The question.",
                "Agent B reports."
            ]
        );
    }

    #[test]
    fn a_stop_during_a_hold_drops_the_backlog_and_keeps_the_lines_it_holds() {
        let (player, started, _ends) = gated();
        player
            .speak("Agent C was mid-sentence.", false, None)
            .unwrap();
        player.speak_and_hold("The question.").unwrap();
        player.speak("Agent B reports.", false, None).unwrap();

        player.stop();
        assert!(!player.is_speaking());

        player.release();
        assert_eq!(
            started_lines(&started),
            ["Agent C was mid-sentence.", "Agent B reports."]
        );
    }

    #[test]
    fn an_interrupt_the_hold_keeps_back_leaves_nothing_speaking() {
        let (player, _started, _ends) = gated();
        player.speak_and_hold("The question.").unwrap();

        player.speak("Agent B interrupts.", true, None).unwrap();
        assert!(
            !player.is_speaking(),
            "a speaking flag with nothing playing deafens the microphone"
        );
    }

    #[test]
    fn an_interrupt_during_a_hold_leaves_only_itself_to_play() {
        let (player, started, _ends) = gated();
        player.speak_and_hold("The question.").unwrap();
        player
            .speak("Agent B is still testing.", false, None)
            .unwrap();

        player.speak("Agent B is done.", true, None).unwrap();
        player.release();
        assert_eq!(
            started_lines(&started),
            ["The question.", "Agent B is done."]
        );
    }

    #[tokio::test]
    async fn a_line_that_will_not_start_keeps_the_lines_the_hold_keeps_back() {
        let (player, started, ends) = gated();
        player
            .speak("Agent C was mid-sentence.", false, None)
            .unwrap();
        player.speak_and_hold(REFUSED).unwrap();
        player.speak("Agent B reports.", false, None).unwrap();

        ends.store(true, std::sync::atomic::Ordering::SeqCst);
        falls_silent(&player).await;
        player.release();
        assert_eq!(
            started_lines(&started),
            ["Agent C was mid-sentence.", "Agent B reports."]
        );
    }

    #[tokio::test]
    async fn a_line_that_will_not_start_before_the_question_leaves_the_question_to_play() {
        let (player, started, ends) = gated();
        player
            .speak("Agent C was mid-sentence.", false, None)
            .unwrap();
        player.speak(REFUSED, false, None).unwrap();
        player.speak_and_hold("The question.").unwrap();

        ends.store(true, std::sync::atomic::Ordering::SeqCst);
        falls_silent(&player).await;
        assert_eq!(
            started_lines(&started),
            ["Agent C was mid-sentence.", "The question."]
        );
    }

    #[tokio::test]
    async fn a_release_drops_a_question_that_never_started() {
        let (player, started, ends) = gated();
        player
            .speak("Agent C was mid-sentence.", false, None)
            .unwrap();
        player.speak_and_hold("The question.").unwrap();
        player.speak("Agent B reports.", false, None).unwrap();

        player.release();
        ends.store(true, std::sync::atomic::Ordering::SeqCst);
        falls_silent(&player).await;
        assert_eq!(
            started_lines(&started),
            ["Agent C was mid-sentence.", "Agent B reports."]
        );
    }

    #[tokio::test]
    async fn a_burst_plays_every_line_in_order() {
        let (player, started, ends) = gated();
        let burst: Vec<String> = (1..=25).map(|id| format!("Status {id}.")).collect();
        for line in &burst {
            player.speak(line, false, None).unwrap();
        }

        ends.store(true, std::sync::atomic::Ordering::SeqCst);
        falls_silent(&player).await;
        assert_eq!(started_lines(&started), burst);
    }

    #[test]
    fn a_skip_drops_the_backlog_and_starts_the_question() {
        let (player, started, _ends) = gated();
        let stalled = player.speak("Agent C stalls.", false, None).unwrap();
        player.speak("Agent B reports.", false, None).unwrap();
        let question = player.speak_and_hold("The question.").unwrap();
        player.speak("Agent A reports.", false, None).unwrap();
        assert!(!player.has_started(question));

        player.skip_to_question(stalled);
        assert_eq!(
            started_lines(&started),
            ["Agent C stalls.", "The question."]
        );
        assert!(player.has_started(question));

        player.skip_to_question(stalled);
        assert_eq!(
            started_lines(&started),
            ["Agent C stalls.", "The question."],
            "a skip that lands after its line ended must leave the question playing"
        );
        assert!(player.is_speaking());
    }

    #[tokio::test]
    async fn a_refused_interrupt_leaves_nothing_speaking() {
        let player = Arc::new(SpeechPlayer::new(Box::new(RefusesAfterFirst(
            std::sync::atomic::AtomicUsize::new(0),
        ))));
        let mut playing = player.subscribe_playing();

        player.speak("The first reply.", false, None).unwrap();
        assert!(player.is_speaking(), "the first reply is playing");

        player
            .speak("The second reply.", true, Some("af_sky"))
            .expect_err("the backend refuses the voice");

        tokio::time::timeout(Duration::from_secs(3), playing.wait_for(Option::is_none))
            .await
            .expect("a reply that never started left the daemon deaf to the microphone")
            .expect("playing sender dropped");
    }
}

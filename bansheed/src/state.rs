use std::{
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64},
    },
    time::{Duration, Instant},
};

use banshee_common::DownloadProgress;
use banshee_common::listening::ListeningModel;
use tokio::sync::{broadcast, watch};

use crate::audio::cues::{Cues, Reason, ReasonCode, Signal, Target, broken};
use crate::config::{BargeInMode, Config, FeedbackMode};
use crate::text_to_speech::{SpeechPlayer, lock};

// One file emits at most 101 notifications, so a subscriber has to fall a whole
// file behind before the channel starts dropping any of them
const DOWNLOAD_BACKLOG: usize = 128;

// A start with no stop otherwise holds the mic for the life of the daemon.
// The ring only holds RING_SECS, so nothing is lost by capping it there.
pub const MAX_PUSH_TO_TALK: Duration = Duration::from_secs(crate::audio::RING_SECS as u64);

// Capture is treated as dead after this long with no callback. Measured
// healthy gaps are 11 ms to 18 ms, so this keeps 55 to 90 times headroom.
pub const CAPTURE_SILENCE_LIMIT: Duration = Duration::from_secs(1);

// The cue a use owes until capture's first sound, as an `AtomicU8`: the
// capture callback moves it from waiting to heard, and that callback must not
// lock. A press owes its start cue, and an answer its arm cue.
const NO_START_CUE: u8 = 0;
const DICTATE_WAITS: u8 = 1;
const ANSWER_WAITS: u8 = 2;
const DICTATE_HEARD: u8 = 3;
const ANSWER_HEARD: u8 = 4;
const ARM_WAITS: u8 = 5;
const ARM_HEARD: u8 = 6;
const ARM_PLAYED: u8 = 7;

// EX_TEMPFAIL in sysexits.h. Any nonzero status restarts the daemon under
// launchd's `SuccessfulExit=false` and systemd's `Restart=on-failure`.
const RESTART_EXIT: u8 = 75;

/// What the daemon reads from the machine it runs on: its files, and whether
/// VoiceOver is on. Handed in, so a test decides.
pub struct Machine {
    pub models: std::path::PathBuf,
    pub config: std::path::PathBuf,
    pub voiceover: fn() -> bool,
}

/// A stopped dictation's place in the ring. Dropped, it gives the place back,
/// whether the consumer took the audio or the command was never read.
pub type RingClaim = Counted;

pub struct AskCommand {
    pub reply: tokio::sync::oneshot::Sender<Result<String, String>>,
    pub timeout: Duration,
    pub session: u64,
}

pub enum ConsumerCommand {
    // A push-to-talk began, numbered so a refusal ends this press and no
    // later one. The consumer reads the ring while the key is held and ends
    // on the `Transcribe` or `Discard` that follows.
    Stream(u64),
    Transcribe(RingClaim),
    // The consumer empties the ring, so a cancelled session does not feed
    // the next transcription
    Discard,
    Ask(AskCommand),
    // The listener owns the engine, so these reach it the way a rebind does.
    // One that lands while a dictation streams waits until the dictation ends,
    // so a model never changes under the stream that uses it.
    Retune(Vec<String>),
    Speak(crate::speech_to_text::Speech),
    Reload(&'static banshee_common::listening::ListeningModel),
    /// `models.unload_after` moved, so the idle wait is measured again.
    UnloadAfter,
    Shutdown,
}

// Stored in an AtomicU8: the audio callback reads it and must not lock
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RecordingMode {
    Idle = 0,
    PushToTalk = 1,
    Armed = 2,
    // Armed, but the user is holding the hotkey to answer manually
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

/// Why the recording pipeline did not start. A missing mic, a missing model, an
/// unreadable key file and a remote listener that will not answer need different
/// fixes, so they stay distinct out to the RPC error code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordingError {
    Microphone(String),
    /// macOS refused the grant, read before any open was tried.
    MicrophoneRefused(Refusal),
    Model(String),
    /// The id of a listening model whose file is not downloaded.
    NoListeningModel(&'static str),
    Provider(String),
    KeyFile(String),
}

/// Why macOS will not let capture open, read from the grant without opening.
/// No other platform has a grant to read, so nothing builds one there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "macos", test)), expect(dead_code))]
pub enum Refusal {
    Denied,
    /// An administrator or a profile decides, so nobody on this Mac can allow it.
    Restricted,
}

impl Refusal {
    pub const fn reason(self) -> &'static str {
        match self {
            Refusal::Denied => "macOS does not allow Banshee to use it",
            Refusal::Restricted => "this Mac restricts it",
        }
    }

    pub const fn fix(self) -> &'static str {
        match self {
            Refusal::Denied => "allow Banshee in System Settings > Privacy & Security > Microphone",
            Refusal::Restricted => "ask the administrator of this Mac for microphone access",
        }
    }
}

/// What the recording pipeline is. The daemon builds it on its own thread and
/// answers clients before it exists, so "not broken" is not the same as "open".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pipeline {
    Opening,
    Open,
    Broken(RecordingError),
}

impl Pipeline {
    /// The word this answer takes on the wire, so a rename is a protocol change.
    pub fn as_str(&self) -> &'static str {
        match self {
            Pipeline::Opening => "opening",
            Pipeline::Open => "open",
            Pipeline::Broken(_) => "broken",
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(self, Pipeline::Open)
    }

    pub fn fault(&self) -> Option<&RecordingError> {
        match self {
            Pipeline::Broken(error) => Some(error),
            _ => None,
        }
    }
}

impl std::fmt::Display for RecordingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordingError::Microphone(e) => write!(f, "the microphone would not open: {e}"),
            RecordingError::MicrophoneRefused(refusal) => {
                write!(f, "the microphone would not open: {}", refusal.reason())
            }
            RecordingError::Model(e) => write!(f, "a model would not load: {e}"),
            RecordingError::NoListeningModel(id) => {
                write!(
                    f,
                    "the listening model {id} is not downloaded: run banshee setup"
                )
            }
            RecordingError::Provider(e) => write!(f, "the remote listener is not reachable: {e}"),
            RecordingError::KeyFile(e) => {
                write!(f, "the remote listener's key file is unreadable: {e}")
            }
        }
    }
}

impl RecordingError {
    /// Trimmed at the source: the wrapped error ends in a period and other blocker prose does not.
    /// A model fault drops the engine's words, which name a path and are stale by the time anyone
    /// reads them. Display keeps them for the log.
    pub fn consequence(&self) -> String {
        match self {
            RecordingError::Model(_) => "a model would not load".to_string(),
            RecordingError::NoListeningModel(_) => {
                "no listening model yet: run banshee setup".to_string()
            }
            RecordingError::Provider(_) => {
                "dictation and ask_user do not work until the key or server is fixed".to_string()
            }
            RecordingError::KeyFile(_) => {
                "dictation and ask_user do not work until the key file is fixed".to_string()
            }
            RecordingError::Microphone(_) | RecordingError::MicrophoneRefused(_) => {
                self.to_string().trim_end_matches('.').to_string()
            }
        }
    }

    /// `None` where no one command clears the fault.
    pub fn command(&self) -> Option<&'static str> {
        match self {
            RecordingError::Microphone(_)
            | RecordingError::Model(_)
            | RecordingError::Provider(_) => Some("banshee start"),
            RecordingError::NoListeningModel(_) => Some("banshee setup"),
            // A grant is a switch in System Settings, not a command.
            RecordingError::KeyFile(_) | RecordingError::MicrophoneRefused(_) => None,
        }
    }

    /// What went wrong, then what to do, for a use whose microphone would not
    /// open: `last_error` and an agent's reply carry both, since nothing else
    /// names the fix. The next use opens again, so a device needs no restart.
    pub fn failed_use(&self) -> String {
        let fix = match self {
            RecordingError::Microphone(_) => {
                "Connect the microphone, or choose another in Listening. The next use tries again."
                    .to_string()
            }
            other => format!("{}.", crate::audio::cues::capitalised(&other.fix())),
        };
        format!("{}. {fix}", self.to_string().trim_end_matches('.'))
    }

    pub fn fix(&self) -> String {
        match self {
            // Capture held open rescans after a fault, and capture opened for a
            // use tries the device again at the next use.
            RecordingError::Microphone(_) => {
                "connect the microphone, grant it in Privacy & Security, or fix \
                 [audio] input_device. If recording does not recover on its own, \
                 restart: banshee start"
                    .to_string()
            }
            RecordingError::MicrophoneRefused(refusal) => refusal.fix().to_string(),
            RecordingError::Model(_) => "restart it: banshee start".to_string(),
            RecordingError::NoListeningModel(_) => "download it: banshee setup".to_string(),
            RecordingError::Provider(_) => "set the key: banshee config set stt.remote.api_key, \
                 or fix [stt.remote] base_url, then restart: banshee start"
                .to_string(),
            // A key written again reads the same file first, so the fix starts
            // by removing it.
            RecordingError::KeyFile(_) => crate::credentials::Credentials::remove_command(),
        }
    }
}

/// The microphone, as recording sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mic {
    /// The pipeline around capture is still being built.
    Starting,
    /// Shut between uses. The next use opens it, so it is ready.
    Closed,
    Open,
    /// Capture failed, or the pipeline around it did not build.
    Faulted(RecordingError),
}

/// What the listener has of its model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Listening {
    /// A listener that loads no file.
    Remote,
    /// On disk and not in memory. It loads when used, so it is no fault.
    Available(&'static ListeningModel),
    /// Being read into memory. A use waits for it, so it is no fault either.
    Loading(&'static ListeningModel),
    Loaded(&'static ListeningModel),
    /// The model whose file arrived and did not load, and why.
    Failed(&'static ListeningModel, String),
    /// The model whose file is still to download.
    Missing(&'static ListeningModel),
}

impl Listening {
    /// `None` for a listener that loads no file.
    pub fn model(&self) -> Option<&'static ListeningModel> {
        match self {
            Listening::Remote => None,
            Listening::Available(model)
            | Listening::Loading(model)
            | Listening::Loaded(model)
            | Listening::Failed(model, _)
            | Listening::Missing(model) => Some(model),
        }
    }

    pub fn serves(&self) -> bool {
        matches!(
            self,
            Listening::Remote
                | Listening::Available(_)
                | Listening::Loading(_)
                | Listening::Loaded(_)
        )
    }
}

/// A microphone fault wins: it is the first thing to fix, since no model
/// records without capture.
pub fn pipeline(mic: &Mic, listening: &Listening) -> Pipeline {
    match (mic, listening) {
        (Mic::Faulted(error), _) => Pipeline::Broken(error.clone()),
        (Mic::Starting, _) => Pipeline::Opening,
        (Mic::Closed | Mic::Open, Listening::Failed(_, error)) => {
            Pipeline::Broken(RecordingError::Model(error.clone()))
        }
        (Mic::Closed | Mic::Open, Listening::Missing(model)) => {
            Pipeline::Broken(RecordingError::NoListeningModel(model.id))
        }
        (
            Mic::Closed | Mic::Open,
            Listening::Remote
            | Listening::Available(_)
            | Listening::Loading(_)
            | Listening::Loaded(_),
        ) => Pipeline::Open,
    }
}

struct Facts {
    mic: Mic,
    listening: Listening,
}

/// The right to download, held for as long as one runs.
// Released on drop rather than by hand: a panic in the download would otherwise
// strand the slot for the life of the daemon. A stable `.part` name is what
// makes resume possible, so it cannot be shared by two writers.
pub struct DownloadSlot {
    state: Arc<DaemonState>,
}

impl Drop for DownloadSlot {
    fn drop(&mut self) {
        self.state
            .downloading
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

/// One holder of a count. Its drop gives the count back.
pub struct Counted(Arc<watch::Sender<usize>>);

impl Counted {
    fn take(count: &Arc<watch::Sender<usize>>) -> Self {
        count.send_modify(|holders| *holders += 1);
        Self(Arc::clone(count))
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.send_modify(|holders| *holders -= 1);
    }
}

/// One client that shows the level. Its drop is the client going away.
pub type LevelWatch = Counted;

fn replace_if_new(field: &RwLock<Option<String>>, name: Option<String>) -> bool {
    let mut held = field.write().unwrap();
    if *held == name {
        return false;
    }
    *held = name;
    true
}

/// Each move wakes the push task of every subscriber, so a value that has not
/// changed sends nothing.
fn send_if_new<T: PartialEq>(channel: &watch::Sender<Option<T>>, value: Option<T>) {
    channel.send_if_modified(|current| {
        if *current == value {
            return false;
        }
        *current = value;
        true
    });
}

pub struct DaemonState {
    version: &'static str,
    vad_model: &'static str,
    vad_threshold: AtomicU32,
    audio_device: RwLock<Option<String>>,
    // The named device the config asks for while something else records. Never
    // a blocker: a daemon on a substitute records correctly.
    missing_device: RwLock<Option<String>>,
    // What `audio.input_device` asks for. The watchdog reads it and rebuilds
    // capture when it differs from the device it opened.
    wanted_device: Mutex<String>,
    // Follows a live `tts.voice`, so the voice reported is the one now loaded
    tts_voice: RwLock<Option<String>>,
    // Said by the branch that built the speaker, and again by a local voice that
    // loads later. A live `[tts]` write moves the voice above and not this: a
    // backend that takes a voice name is still the fallback.
    speaker_started: AtomicBool,
    wanted_downloads: RwLock<Vec<crate::models::download::Download>>,
    // The file as last parsed. `vad_threshold`, `wanted_device` and `barge_in`
    // beside it are live values the file may no longer agree with.
    config: RwLock<Arc<Config>>,
    // A key with no live apply is read once, when the daemon starts, so this is
    // what those keys run for the daemon's whole life. Without it a write cannot
    // tell a change from a change back.
    running_config: Arc<Config>,
    // Keys the daemon accepted and wrote but has not applied. A restart empties
    // it by nature; a live path clears its own key.
    pending: Mutex<std::collections::BTreeSet<String>>,
    // Always `pipeline(mic, listening)` of `facts`, sent under its lock, so
    // two writers cannot publish their changes out of order. The watchdog
    // reopens capture on every rebind, and a value that has not moved wakes
    // no subscriber.
    pipeline: watch::Sender<Pipeline>,
    // `listening` names the model the listener serves, or the one it waits for
    // while it serves none. A model the file names while another serves is not
    // in it until that model loads.
    facts: Mutex<Facts>,
    recording: AtomicU8,
    started_at: Instant,
    db_connection: Mutex<Option<rusqlite::Connection>>,
    recording_active: watch::Sender<bool>,
    transcribing: watch::Sender<bool>,
    // Reading a model off disk can take seconds, and the listener answers
    // with the old model until it lands. Watched, so the window is told when it
    // does: `stt_model` is not a live field and nothing else reports the end.
    loading_model: watch::Sender<bool>,
    // Why the last hotkey-path attempt failed. Dictation writes it from
    // resampling, transcribing, and listening for an answer.
    last_error: watch::Sender<Option<String>>,
    // Why the last spoken reply failed, cleared by the next one that plays.
    last_speech_error: watch::Sender<Option<String>>,
    // One counter for the whole device picture. It moves only when a setter
    // writes a value that differs, because the watchdog rewrites the same one
    // every rescan and each move wakes the push task of every subscriber.
    device_changes: watch::Sender<u64>,
    downloads: broadcast::Sender<DownloadProgress>,
    downloading: AtomicBool,
    /// Set when the daemon leaves to be started again, so the supervisor reads
    /// a nonzero exit rather than a stop it would not undo.
    restart_wanted: AtomicBool,
    speech: Arc<SpeechPlayer>,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
    cues: Cues,
    barge_in: Mutex<BargeInMode>,
    // The press's deadline. Start and stop can be separate RPC calls, so this
    // cannot live on a stack
    push_to_talk: Mutex<Instant>,
    // The number of the latest press, moved under the push-to-talk lock.
    presses: AtomicU64,
    armed_session: Mutex<u64>,
    ask_turn: tokio::sync::Mutex<()>,
    dictations_in_ring: Arc<watch::Sender<usize>>,
    // enigo posts to the same HID stream rdev listens at, so while this is
    // true the hotkey listener drops events: the paste's own modifier presses
    // would otherwise cancel or open sessions.
    typing: AtomicBool,
    // Milliseconds since `started_at` at which the capture callback last ran.
    // Zero means it has never run, which reads as stalled.
    capture_tick: AtomicU64,
    start_cue: AtomicU8,
    keep_open: AtomicBool,
    // The last read of the microphone grant. Every read goes through its
    // setter, so a move reaches every client.
    microphone_refusal: Mutex<Option<Refusal>>,
    // A question has played and its answer is being listened for.
    answering: AtomicBool,
    // Woken whenever `wants_mic` may have changed, and when a press's first
    // sound arrives.
    mic_owner: std::sync::OnceLock<std::thread::Thread>,
    unload_after: Mutex<Option<Duration>>,
    // Woken when `unload_after` moves.
    voice_keeper: std::sync::OnceLock<std::thread::Thread>,
    shutdown: tokio::sync::Notify,
    spoken: crate::turns::SpokenTurns,
    machine: Machine,
    peak: AtomicU32,
    answer_open: AtomicBool,
    level: watch::Sender<f32>,
    level_watchers: Arc<watch::Sender<usize>>,
    // The committed words of the recording in progress, empty between recordings.
    words: watch::Sender<String>,
}

impl DaemonState {
    pub fn new(
        config: Arc<Config>,
        db_connection: Option<rusqlite::Connection>,
        speech: SpeechPlayer,
        speaker: crate::text_to_speech::Speaker,
        commands: std::sync::mpsc::Sender<ConsumerCommand>,
        cues: Cues,
        machine: Machine,
    ) -> Self {
        let wanted_downloads = crate::models::download::wanted(&config);
        let keep_open = config.audio.keep_open;
        let unload_after = config.models.unload_limit();
        let facts = Facts {
            mic: Mic::Starting,
            listening: match crate::models::stt_model(&config) {
                None => Listening::Remote,
                Some(model)
                    if crate::models::missing_in(&machine.models, &[model.file]).is_empty() =>
                {
                    Listening::Available(model)
                }
                Some(model) => Listening::Missing(model),
            },
        };
        Self {
            machine,
            version: env!("CARGO_PKG_VERSION"),
            vad_model: crate::models::VAD_MODEL,
            vad_threshold: AtomicU32::new(config.stt.vad_threshold.to_bits()),
            audio_device: RwLock::new(None),
            missing_device: RwLock::new(None),
            wanted_device: Mutex::new(config.audio.input_device.clone()),
            speaker_started: AtomicBool::new(speaker.started()),
            tts_voice: RwLock::new(speaker.voice()),
            wanted_downloads: RwLock::new(wanted_downloads),
            barge_in: Mutex::new(config.audio.barge_in),
            running_config: Arc::clone(&config),
            config: RwLock::new(config),
            pending: Mutex::new(std::collections::BTreeSet::new()),
            pipeline: watch::channel(pipeline(&facts.mic, &facts.listening)).0,
            facts: Mutex::new(facts),
            recording: AtomicU8::new(RecordingMode::Idle as u8),
            started_at: Instant::now(),
            db_connection: Mutex::new(db_connection),
            recording_active: watch::channel(false).0,
            transcribing: watch::channel(false).0,
            loading_model: watch::channel(false).0,
            last_error: watch::channel(None).0,
            last_speech_error: watch::channel(None).0,
            device_changes: watch::channel(0).0,
            downloads: broadcast::channel(DOWNLOAD_BACKLOG).0,
            downloading: AtomicBool::new(false),
            restart_wanted: AtomicBool::new(false),
            speech: Arc::new(speech),
            commands,
            cues,
            push_to_talk: Mutex::new(Instant::now()),
            presses: AtomicU64::new(0),
            armed_session: Mutex::new(0),
            ask_turn: tokio::sync::Mutex::new(()),
            dictations_in_ring: Arc::new(watch::channel(0).0),
            typing: AtomicBool::new(false),
            capture_tick: AtomicU64::new(0),
            start_cue: AtomicU8::new(NO_START_CUE),
            keep_open: AtomicBool::new(keep_open),
            microphone_refusal: Mutex::new(None),
            answering: AtomicBool::new(false),
            mic_owner: std::sync::OnceLock::new(),
            unload_after: Mutex::new(unload_after),
            voice_keeper: std::sync::OnceLock::new(),
            shutdown: tokio::sync::Notify::new(),
            spoken: crate::turns::SpokenTurns::default(),
            peak: AtomicU32::new(0),
            answer_open: AtomicBool::new(false),
            level: watch::channel(0.0).0,
            level_watchers: Arc::new(watch::channel(0).0),
            words: watch::channel(String::new()).0,
        }
    }

    // False when another session already owns the microphone.
    pub fn record_start(&self) -> bool {
        // The hotkey arrives here too, so a deaf daemon answers a press with the
        // error cue. Arming a session nothing can transcribe would be silent.
        let reason = match self.pipeline() {
            Pipeline::Open => None,
            Pipeline::Broken(fault) => Some(broken(&fault)),
            Pipeline::Opening => Some(Reason::new(ReasonCode::Starting, None)),
        };
        if let Some(reason) = reason {
            let target = if self.is_armed() {
                Target::Answer
            } else {
                Target::Dictate
            };
            self.cues.emit(Signal::Error {
                reason,
                target: Some(target),
            });
            return false;
        }
        if self.try_transition(RecordingMode::Armed, RecordingMode::ArmedHold) {
            // Manual override of an armed session: hold to answer
            if matches!(self.barge_in(), BargeInMode::Stop) {
                self.speech.stop();
            }
            self.start_cue
                .store(ANSWER_WAITS, std::sync::atomic::Ordering::Relaxed);
            true
        } else if self.start_push_to_talk() {
            // Silence the daemon's own voice before the mic opens
            if matches!(self.barge_in(), BargeInMode::Stop) {
                self.speech.stop();
            }
            log::info!("Recording started...");
            true
        } else {
            false
        }
    }

    // A stop with nothing in flight is a no-op, so release keybinds can fire
    // unconditionally.
    pub fn record_stop(&self) {
        if self.stop_push_to_talk(lock(&self.push_to_talk)) {
            return;
        }
        let session = lock(&self.push_to_talk);
        let held = self.try_transition(RecordingMode::ArmedHold, RecordingMode::Armed);
        drop(session);
        if held {
            self.cues.emit(Signal::RecordStop {
                target: Target::Answer,
            });
        }
    }

    fn start_push_to_talk(&self) -> bool {
        let mut deadline = lock(&self.push_to_talk);
        if !self.try_transition(RecordingMode::Idle, RecordingMode::PushToTalk) {
            return false;
        }
        // Discarded: a press starts from silence, not the last session's peak.
        self.take_peak();
        self.start_cue
            .store(DICTATE_WAITS, std::sync::atomic::Ordering::Relaxed);
        *deadline = Instant::now() + MAX_PUSH_TO_TALK;
        let press = self
            .presses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        // Under the push-to-talk lock, so it reaches the consumer before this
        // press's stop or cancel.
        let _ = self.commands.send(ConsumerCommand::Stream(press));
        true
    }

    fn stop_push_to_talk(&self, deadline: std::sync::MutexGuard<'_, Instant>) -> bool {
        // Claimed under the push-to-talk lock and before the mode changes, so
        // `arm_after_dictations` never sees the microphone free and the ring unclaimed.
        let ring = self.claim_ring();
        if !self.try_transition(RecordingMode::PushToTalk, RecordingMode::Idle) {
            return false;
        }
        drop(deadline);
        log::info!("Recording stopped");
        self.cues.emit(Signal::RecordStop {
            target: Target::Dictate,
        });
        let _ = self.commands.send(ConsumerCommand::Transcribe(ring));
        true
    }

    pub fn claim_ring(&self) -> RingClaim {
        Counted::take(&self.dictations_in_ring)
    }

    pub fn dictation_in_ring(&self) -> bool {
        *self.dictations_in_ring.borrow() > 0
    }

    // Resolved here, not in the tracker: a tracker-side read races a mode change. True when it
    // starts a session.
    pub fn record_toggle(&self) -> bool {
        if matches!(
            self.recording_mode(),
            RecordingMode::PushToTalk | RecordingMode::ArmedHold
        ) {
            self.record_stop();
            false
        } else {
            self.record_start()
        }
    }

    // The user pressed a chord through the hotkey, so the session it opened
    // is a mistake: discard the audio rather than route it. A push-to-talk
    // cancel plays no sound, because the start cue is already noise and a
    // second cue doubles it. A silent signal still tells a subscriber the
    // session is gone.
    pub fn record_cancel(&self) {
        let session = lock(&self.push_to_talk);
        if self.try_transition(RecordingMode::PushToTalk, RecordingMode::Idle) {
            log::info!("Recording cancelled");
            drop(session);
            self.cues.emit(Signal::Cancelled {
                target: Target::Dictate,
            });
            let _ = self.commands.send(ConsumerCommand::Discard);
        } else {
            let held = self.try_transition(RecordingMode::ArmedHold, RecordingMode::Armed);
            drop(session);
            if held {
                // The armed session keeps its audio; only the manual hold ends.
                // This cue answers the start cue the hold played.
                self.cues.emit(Signal::RecordStop {
                    target: Target::Answer,
                });
            }
        }
    }

    pub fn set_typing(&self, typing: bool) {
        self.typing
            .store(typing, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_typing(&self) -> bool {
        self.typing.load(std::sync::atomic::Ordering::Relaxed)
    }

    // Releases a session that never got its stop. Stops rather than discards:
    // the ring holds real audio and record_stop is what routes it.
    pub fn expire_stuck_recording(&self) -> bool {
        let deadline = lock(&self.push_to_talk);
        if self.recording_mode() != RecordingMode::PushToTalk || Instant::now() < *deadline {
            return false;
        }
        log::warn!(
            "Push-to-talk ran past {}s with no stop; releasing the microphone.",
            MAX_PUSH_TO_TALK.as_secs()
        );
        self.stop_push_to_talk(deadline)
    }

    pub fn speech(&self) -> &Arc<SpeechPlayer> {
        &self.speech
    }

    pub fn spoken(&self) -> &crate::turns::SpokenTurns {
        &self.spoken
    }

    pub fn commands(&self) -> &std::sync::mpsc::Sender<ConsumerCommand> {
        &self.commands
    }

    pub fn shutdown(&self) -> &tokio::sync::Notify {
        &self.shutdown
    }

    pub fn recording_mode(&self) -> RecordingMode {
        RecordingMode::from_u8(self.recording.load(std::sync::atomic::Ordering::Relaxed))
    }

    #[cfg(test)]
    pub fn set_recording_mode(&self, mode: RecordingMode) {
        self.recording
            .store(mode as u8, std::sync::atomic::Ordering::Relaxed);
        self.publish_recording();
    }

    // Concurrent transitions race (RPC arm vs hotkey vs session end);
    // compare_exchange makes losing a race a no-op instead of a stuck mode
    pub fn try_transition(&self, from: RecordingMode, to: RecordingMode) -> bool {
        let moved = self
            .recording
            .compare_exchange(
                from as u8,
                to as u8,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok();
        if moved {
            self.publish_recording();
        }
        moved
    }

    // Re-reads the atomic rather than deriving the bool from the mode just
    // written: another thread may have moved on, and subscribers want now.
    fn publish_recording(&self) {
        self.recording_active.send_replace(self.is_recording());
        self.wake_mic();
    }

    /// Whether capture should be open now: for a press, for an answer being
    /// listened for, or for good under `audio.keep_open`. A question that is
    /// still playing does not need it.
    pub fn wants_mic(&self) -> bool {
        self.keep_open()
            || self.answering.load(std::sync::atomic::Ordering::Relaxed)
            || matches!(
                self.recording_mode(),
                RecordingMode::PushToTalk | RecordingMode::ArmedHold
            )
    }

    pub fn keep_open(&self) -> bool {
        self.keep_open.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_keep_open(&self, keep: bool) {
        self.keep_open
            .store(keep, std::sync::atomic::Ordering::Relaxed);
        self.wake_mic();
    }

    pub fn set_mic_owner(&self, owner: std::thread::Thread) {
        let _ = self.mic_owner.set(owner);
    }

    /// Called from the real time audio thread too: an unpark neither locks
    /// nor allocates.
    fn wake_mic(&self) {
        if let Some(owner) = self.mic_owner.get() {
            owner.unpark();
        }
    }

    /// The question has played, so capture opens for its answer, and the arm
    /// cue waits for capture's first sound. A hold already answering owes its
    /// own start cue instead. False when `session` is no longer the one armed.
    pub fn hear_answer(&self, session: u64) -> bool {
        let current = lock(&self.armed_session);
        let armed = *current == session && self.is_armed();
        if armed {
            if self.recording_mode() == RecordingMode::Armed {
                self.start_cue
                    .store(ARM_WAITS, std::sync::atomic::Ordering::Relaxed);
            }
            self.answering
                .store(true, std::sync::atomic::Ordering::Relaxed);
            self.wake_mic();
        }
        armed
    }

    /// Whether the arm cue of the answer being listened for has played.
    pub fn arm_cue_played(&self) -> bool {
        self.start_cue.load(std::sync::atomic::Ordering::Relaxed) == ARM_PLAYED
    }

    /// The capture callback heard a sample that is not silence. Called from
    /// the real time audio thread, so it must not lock or allocate.
    pub fn heard_sound(&self) {
        let (waits, heard) = match self.start_cue.load(std::sync::atomic::Ordering::Relaxed) {
            DICTATE_WAITS => (DICTATE_WAITS, DICTATE_HEARD),
            ANSWER_WAITS => (ANSWER_WAITS, ANSWER_HEARD),
            ARM_WAITS => (ARM_WAITS, ARM_HEARD),
            _ => return,
        };
        if self
            .start_cue
            .compare_exchange(
                waits,
                heard,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok()
        {
            self.wake_mic();
        }
    }

    /// Plays the cue a use owes once its first sound has arrived, and only
    /// while that use still holds capture: a press its start cue, an answer
    /// its arm cue. Under the push-to-talk lock, as every stop and cancel of a
    /// press or a hold is, so none can land between the check and the cue.
    pub fn announce_sound(&self) {
        let _session = lock(&self.push_to_talk);
        let mode = self.recording_mode();
        let heard = self.start_cue.load(std::sync::atomic::Ordering::Relaxed);
        let (signal, holds, after) = match heard {
            DICTATE_HEARD => (
                Signal::RecordStart {
                    target: Target::Dictate,
                },
                mode == RecordingMode::PushToTalk,
                NO_START_CUE,
            ),
            ANSWER_HEARD => (
                Signal::RecordStart {
                    target: Target::Answer,
                },
                mode == RecordingMode::ArmedHold,
                NO_START_CUE,
            ),
            ARM_HEARD => (
                Signal::Arm,
                mode == RecordingMode::Armed
                    && self.answering.load(std::sync::atomic::Ordering::Relaxed),
                ARM_PLAYED,
            ),
            _ => return,
        };
        let after = if holds { after } else { NO_START_CUE };
        let taken = self
            .start_cue
            .compare_exchange(
                heard,
                after,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok();
        if taken && holds {
            self.cues.emit(signal);
        }
    }

    /// The dictation `press` began, whose model would not load, is refused.
    /// While that press is still held it ends, and its release finds nothing
    /// to stop. Answers false when it is no longer held: every way a press
    /// ends sends its `Transcribe` or `Discard`, which is on its way behind
    /// its `Stream`, and a later press is left alone.
    pub fn refuse_dictation(&self, press: u64, why: String) -> bool {
        self.set_last_error(Some(why));
        let session = lock(&self.push_to_talk);
        let ended = self.presses.load(std::sync::atomic::Ordering::Relaxed) == press
            && self.try_transition(RecordingMode::PushToTalk, RecordingMode::Idle);
        drop(session);
        self.cues.emit(Signal::Error {
            reason: Reason::new(ReasonCode::TranscriptionFailed, None),
            target: Some(Target::Dictate),
        });
        ended
    }

    /// The microphone would not open for the use in progress, so that use
    /// fails with the reason. An answer learns it from the capture it reads.
    pub fn capture_failed(&self, fault: RecordingError) {
        self.set_last_error(Some(fault.failed_use()));
        let answering = self
            .answering
            .swap(false, std::sync::atomic::Ordering::Relaxed);
        let session = lock(&self.push_to_talk);
        let target = if self.try_transition(RecordingMode::PushToTalk, RecordingMode::Idle) {
            let _ = self.commands.send(ConsumerCommand::Discard);
            Some(Target::Dictate)
        } else if self.try_transition(RecordingMode::ArmedHold, RecordingMode::Armed) && !answering
        {
            Some(Target::Answer)
        } else {
            None
        };
        drop(session);
        if let Some(target) = target {
            self.cues.emit(Signal::Error {
                reason: broken(&fault),
                target: Some(target),
            });
        }
    }

    /// The number of the latest press.
    #[cfg(test)]
    pub fn press(&self) -> u64 {
        self.presses.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn subscribe_recording(&self) -> watch::Receiver<bool> {
        self.recording_active.subscribe()
    }

    pub fn set_transcribing(&self, on: bool) {
        self.transcribing.send_replace(on);
    }

    pub fn is_transcribing(&self) -> bool {
        *self.transcribing.borrow()
    }

    pub fn is_loading_model(&self) -> bool {
        *self.loading_model.borrow()
    }

    pub fn set_loading_model(&self, loading: bool) {
        self.loading_model.send_if_modified(|held| {
            let moved = *held != loading;
            *held = loading;
            moved
        });
    }

    pub fn subscribe_loading_model(&self) -> watch::Receiver<bool> {
        self.loading_model.subscribe()
    }

    pub fn subscribe_transcribing(&self) -> watch::Receiver<bool> {
        self.transcribing.subscribe()
    }

    /// Why the last recording failed. `None` clears it.
    pub fn set_last_error(&self, error: Option<String>) {
        send_if_new(&self.last_error, error);
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.borrow().clone()
    }

    pub fn subscribe_last_error(&self) -> watch::Receiver<Option<String>> {
        self.last_error.subscribe()
    }

    /// The speaking half of the pair. One field per side, so a failed listen
    /// and a failed reply never overwrite each other.
    pub fn set_last_speech_error(&self, error: Option<String>) {
        send_if_new(&self.last_speech_error, error);
    }

    pub fn last_speech_error(&self) -> Option<String> {
        self.last_speech_error.borrow().clone()
    }

    pub fn subscribe_last_speech_error(&self) -> watch::Receiver<Option<String>> {
        self.last_speech_error.subscribe()
    }

    pub fn subscribe_downloads(&self) -> broadcast::Receiver<DownloadProgress> {
        self.downloads.subscribe()
    }

    pub fn report_download(&self, progress: DownloadProgress) {
        let _ = self.downloads.send(progress);
    }

    pub fn is_downloading(&self) -> bool {
        self.downloading.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Shuts the daemon down to exit with the status that has its supervisor
    /// start it again.
    pub fn leave_to_restart(&self) {
        self.restart_wanted
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.shutdown.notify_one();
    }

    /// The status the daemon exits with once it shuts down.
    pub fn exit_code(&self) -> u8 {
        if self
            .restart_wanted
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            RESTART_EXIT
        } else {
            0
        }
    }

    /// Takes the download slot, or `None` when one is already running. The
    /// slot is released when the returned value drops.
    pub fn start_downloading(self: &Arc<Self>) -> Option<DownloadSlot> {
        self.downloading
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok()
            .then(|| DownloadSlot {
                state: Arc::clone(self),
            })
    }

    pub fn is_recording(&self) -> bool {
        self.recording_mode() != RecordingMode::Idle
    }

    /// True while a question waits for its spoken answer, from before it plays
    /// until the answer ends.
    pub fn is_armed(&self) -> bool {
        matches!(
            self.recording_mode(),
            RecordingMode::Armed | RecordingMode::ArmedHold
        )
    }

    pub fn uptime(&self) -> std::time::Duration {
        self.started_at.elapsed()
    }

    pub fn models_dir(&self) -> &std::path::Path {
        &self.machine.models
    }

    pub fn config_path(&self) -> &std::path::Path {
        &self.machine.config
    }

    pub fn voiceover_on(&self) -> bool {
        (self.machine.voiceover)()
    }

    pub fn version(&self) -> &'static str {
        self.version
    }

    pub fn stt_model(&self) -> Option<&'static ListeningModel> {
        self.listening().model()
    }

    pub fn vad_model(&self) -> &str {
        self.vad_model
    }

    pub fn audio_device(&self) -> Option<String> {
        self.audio_device.read().unwrap().clone()
    }

    /// `None` when the device will not name itself, so status says so rather
    /// than keeping the last name it knew.
    pub fn set_audio_device(&self, name: Option<String>) {
        if !replace_if_new(&self.audio_device, name) {
            return;
        }
        self.publish_device_change();
    }

    pub fn missing_device(&self) -> Option<String> {
        self.missing_device.read().unwrap().clone()
    }

    pub fn set_missing_device(&self, name: Option<String>) {
        if !replace_if_new(&self.missing_device, name) {
            return;
        }
        self.publish_device_change();
    }

    /// The raw setting, not a resolved device. `choose` is the one place that
    /// turns a blank name into the default.
    pub fn wanted_device(&self) -> String {
        self.locked_wanted_device().clone()
    }

    pub fn set_wanted_device(&self, name: String) {
        *self.locked_wanted_device() = name;
    }

    // A device name has no invariant a panic can break, and a panic here kills
    // capture silently, so the value stands
    fn locked_wanted_device(&self) -> std::sync::MutexGuard<'_, String> {
        self.wanted_device
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn publish_device_change(&self) {
        self.device_changes
            .send_modify(|generation| *generation += 1);
    }

    /// Moves when the device a client can see changes. The value is a
    /// generation, not a device: the reader reads the fields themselves.
    pub fn device_changes(&self) -> watch::Receiver<u64> {
        self.device_changes.subscribe()
    }

    pub fn pipeline(&self) -> Pipeline {
        self.pipeline.borrow().clone()
    }

    /// A client reports whether capture is open, and nothing else wakes its
    /// push when only that moves, so a move counts as a device change.
    pub fn set_mic(&self, mic: Mic) {
        let mut moved = false;
        self.update(|facts| {
            moved = (facts.mic == Mic::Open) != (mic == Mic::Open);
            facts.mic = mic;
        });
        if moved {
            self.publish_device_change();
        }
    }

    /// Pushed, so a tray tells a refused microphone from an idle one.
    pub fn set_microphone_refusal(&self, refusal: Option<Refusal>) {
        let moved = std::mem::replace(&mut *lock(&self.microphone_refusal), refusal) != refusal;
        if moved {
            self.publish_device_change();
        }
    }

    pub fn microphone_refusal(&self) -> Option<Refusal> {
        *lock(&self.microphone_refusal)
    }

    pub fn microphone_open(&self) -> bool {
        lock(&self.facts).mic == Mic::Open
    }

    #[cfg(test)]
    pub fn mic(&self) -> Mic {
        lock(&self.facts).mic.clone()
    }

    pub fn listening(&self) -> Listening {
        lock(&self.facts).listening.clone()
    }

    /// The pipeline stands, and listens once `model`'s file arrives and loads.
    pub fn await_listening_model(&self, model: &'static ListeningModel) {
        self.update(|facts| facts.listening = Listening::Missing(model));
    }

    /// A listener that serves no model takes `model` from now on: loading
    /// when `here` says its file is, and waiting for it otherwise. One that
    /// serves keeps its model, and a load in flight serves, so a second caller
    /// finds it and sends no second load. Answers what the listener had, read
    /// under the same lock as the write.
    pub fn claim_listening_model(&self, model: &'static ListeningModel, here: bool) -> Listening {
        let mut had = Listening::Remote;
        self.update(|facts| {
            had = facts.listening.clone();
            if !had.serves() {
                facts.listening = if here {
                    Listening::Loading(model)
                } else {
                    Listening::Missing(model)
                };
            }
        });
        had
    }

    /// `None` for a listener that loads no file.
    pub fn listening_model_loaded(&self, model: Option<&'static ListeningModel>) {
        self.update(|facts| {
            facts.listening = model.map_or(Listening::Remote, Listening::Loaded);
        });
    }

    /// A model that already serves listens on, so only a listener with none,
    /// or one that is loading, takes the failure as its fault.
    pub fn listening_model_failed(&self, model: &'static ListeningModel, error: String) {
        self.update(|facts| {
            if matches!(facts.listening, Listening::Loading(_)) || !facts.listening.serves() {
                facts.listening = Listening::Failed(model, error);
            }
        });
    }

    /// The listener reads the model it idled out of back into memory.
    /// Answers whether this caller owns the load; a load claimed for a file
    /// that arrived ends with its own `Reload`.
    pub fn listening_model_loading(&self) -> bool {
        let mut ours = false;
        self.update(|facts| {
            if let Listening::Available(model) = facts.listening {
                facts.listening = Listening::Loading(model);
                ours = true;
            }
        });
        ours
    }

    /// The load `listening_model_loading` gave this caller ended: the model is
    /// in memory, or it stays on disk for the next use to try again.
    pub fn listening_model_read(&self, loaded: bool) {
        self.update(|facts| {
            if let Listening::Loading(model) = facts.listening {
                facts.listening = if loaded {
                    Listening::Loaded(model)
                } else {
                    Listening::Available(model)
                };
            }
        });
    }

    /// The model idled out of memory. The next use reads it again.
    pub fn listening_model_unloaded(&self) {
        self.update(|facts| {
            if let Listening::Loaded(model) = facts.listening {
                facts.listening = Listening::Available(model);
            }
        });
    }

    /// How long a local model stays loaded after its last use, or `None`
    /// when it stays for good.
    pub fn unload_after(&self) -> Option<Duration> {
        *lock(&self.unload_after)
    }

    /// The listener and the voice each measure their idle wait again.
    pub fn set_unload_after(&self, limit: Option<Duration>) {
        *lock(&self.unload_after) = limit;
        if let Some(keeper) = self.voice_keeper.get() {
            keeper.unpark();
        }
        let _ = self.commands.send(ConsumerCommand::UnloadAfter);
    }

    pub fn set_voice_keeper(&self, keeper: std::thread::Thread) {
        let _ = self.voice_keeper.set(keeper);
    }

    fn update(&self, change: impl FnOnce(&mut Facts)) {
        let mut facts = lock(&self.facts);
        change(&mut facts);
        let now = pipeline(&facts.mic, &facts.listening);
        self.pipeline.send_if_modified(|held| {
            let moved = *held != now;
            *held = now;
            moved
        });
    }

    pub fn subscribe_pipeline(&self) -> watch::Receiver<Pipeline> {
        self.pipeline.subscribe()
    }

    /// Takes the armed-listening lock for `ask_user`. Shares the availability
    /// gate with `record_start`, so no caller can arm a mic that cannot record.
    pub fn arm_for_ask(&self) -> Option<u64> {
        let mut session = lock(&self.armed_session);
        if !self.pipeline().is_open()
            || !self.try_transition(RecordingMode::Idle, RecordingMode::Armed)
        {
            return None;
        }
        *session += 1;
        Some(*session)
    }

    /// Waits for this question's turn, a free microphone and a ring with no
    /// dictation left in it, then arms. Tokio's mutex is fair, so questions arm
    /// in the order they asked. Gives back the pipeline instead once it is not
    /// open, whichever wait it breaks.
    pub async fn arm_in_turn(&self) -> Result<(tokio::sync::MutexGuard<'_, ()>, u64), Pipeline> {
        let mut modes = self.subscribe_recording();
        let mut pipelines = self.subscribe_pipeline();
        let mut dictations = self.dictations_in_ring.subscribe();
        // Every sender lives in `self`, so no wait below can end in an error.
        let turn = tokio::select! {
            turn = self.ask_turn.lock() => turn,
            closed = pipelines.wait_for(|pipeline| !pipeline.is_open()) => {
                return Err(closed.expect("the sender lives in self").clone());
            }
        };
        loop {
            let pipeline = self.pipeline();
            if !pipeline.is_open() {
                return Err(pipeline);
            }
            if let Some(session) = self.arm_after_dictations() {
                return Ok((turn, session));
            }
            tokio::select! {
                _ = modes.changed() => {}
                _ = pipelines.changed() => {}
                _ = dictations.changed() => {}
            }
        }
    }

    /// Under the push-to-talk lock, so no stop claims the ring between the count and the arm.
    fn arm_after_dictations(&self) -> Option<u64> {
        let _session = lock(&self.push_to_talk);
        if self.dictation_in_ring() {
            return None;
        }
        self.arm_for_ask()
    }

    pub fn armed_mode(&self, session: u64) -> Option<RecordingMode> {
        let current = lock(&self.armed_session);
        let mode = self.recording_mode();
        (*current == session && self.is_armed()).then_some(mode)
    }

    /// Opens the answer flag only if `session` is still the one armed, and
    /// answers whether it opened.
    pub fn open_answer(&self, session: u64) -> bool {
        // The same lock as `disarm`, so no disarm lands between the check and the open.
        let current = lock(&self.armed_session);
        let opened = *current == session && self.is_armed();
        if opened {
            self.set_answer_open(true);
        }
        opened
    }

    /// Also lets through the speech `ask_user` held back while the session listened.
    pub fn disarm(&self, session: u64) {
        let current = lock(&self.armed_session);
        if *current == session {
            self.answering
                .store(false, std::sync::atomic::Ordering::Relaxed);
            for owed in [ARM_WAITS, ARM_HEARD, ARM_PLAYED] {
                let _ = self.start_cue.compare_exchange(
                    owed,
                    NO_START_CUE,
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
            self.wake_mic();
            let _ = self.try_transition(RecordingMode::Armed, RecordingMode::Idle)
                || self.try_transition(RecordingMode::ArmedHold, RecordingMode::Idle);
            self.set_answer_open(false);
            // Under the lock, so no ask armed after this one has its hold lifted
            self.speech.release();
        }
    }

    /// The voice the speech backend actually loaded, which `config.toml` may no
    /// longer agree with.
    pub fn tts_voice(&self) -> Option<String> {
        self.tts_voice
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// Whether the speaker `[tts]` names is the one running. False under the
    /// fallback or silence, which send no text out whatever the config asks for.
    pub fn speaker_started(&self) -> bool {
        self.speaker_started
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_speaker_started(&self, started: bool) {
        self.speaker_started
            .store(started, std::sync::atomic::Ordering::Relaxed);
    }

    /// True when the backend took the change. The voice the window marks as
    /// current comes from the backend, not from the file, so a backend that
    /// speaks in something else is never reported as speaking in this.
    pub fn set_tts(&self, tts: &crate::config::TTSConfig) -> bool {
        let Some(voice) = self.speech.reconfigure(tts) else {
            return false;
        };
        self.set_tts_voice(voice);
        true
    }

    pub fn set_tts_voice(&self, voice: String) {
        *self
            .tts_voice
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(voice);
    }

    /// Every file this daemon's own config needs. Set before the socket
    /// accepts, so a caller never sees it unset.
    pub fn wanted_downloads(&self) -> Vec<crate::models::download::Download> {
        self.wanted_downloads
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// Tests need a list of their own, because what is missing otherwise
    /// depends on which models the machine happens to hold.
    #[cfg(test)]
    pub fn set_wanted_downloads(&self, wanted: Vec<crate::models::download::Download>) {
        *self
            .wanted_downloads
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = wanted;
    }

    pub fn config(&self) -> Arc<Config> {
        Arc::clone(&self.config.read().unwrap())
    }

    /// The models the daemon wants follow the config that names them.
    pub fn set_config(&self, config: Arc<Config>) {
        *self
            .wanted_downloads
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) =
            crate::models::download::wanted(&config);
        *self.config.write().unwrap() = config;
    }

    /// The config the restart-only keys are running.
    pub fn running_config(&self) -> Arc<Config> {
        Arc::clone(&self.running_config)
    }

    pub fn pending(&self) -> Vec<String> {
        self.pending.lock().unwrap().iter().cloned().collect()
    }

    /// One key is either applied or waiting, never both.
    pub fn record_outcome(&self, applied: &[String], restart_required: &[String]) {
        let mut pending = self.pending.lock().unwrap();
        for key in applied {
            pending.remove(key);
        }
        for key in restart_required {
            pending.insert(key.clone());
        }
    }

    pub fn set_feedback_mode(&self, mode: FeedbackMode) {
        self.cues.set_mode(mode);
    }

    #[cfg(test)]
    pub fn feedback_mode(&self) -> FeedbackMode {
        self.cues.mode()
    }

    pub fn subscribe_feedback(&self) -> watch::Receiver<FeedbackMode> {
        self.cues.subscribe_mode()
    }

    pub fn barge_in(&self) -> BargeInMode {
        *self.locked_barge_in()
    }

    pub fn set_barge_in(&self, mode: BargeInMode) {
        *self.locked_barge_in() = mode;
    }

    // A mode has no invariant a panic can break, and a panic here would stop
    // every later dictation reading it, so the value stands
    fn locked_barge_in(&self) -> std::sync::MutexGuard<'_, BargeInMode> {
        self.barge_in
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub fn set_vocabulary(&self, words: Vec<String>) -> bool {
        self.commands.send(ConsumerCommand::Retune(words)).is_ok()
    }

    /// Whether the next utterance answers in English. The engine reads it per
    /// utterance, so no model moves.
    pub fn set_speech(&self, speech: crate::speech_to_text::Speech) -> bool {
        self.commands.send(ConsumerCommand::Speak(speech)).is_ok()
    }

    pub fn load_stt_model(
        &self,
        model: &'static banshee_common::listening::ListeningModel,
    ) -> bool {
        self.commands.send(ConsumerCommand::Reload(model)).is_ok()
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

    /// Runs `job` against the history file, or answers `None` when history is
    /// off. The one place that holds the lock, so no caller repeats the two
    /// steps that stand between it and the table.
    pub fn with_history<T>(&self, job: impl FnOnce(&rusqlite::Connection) -> T) -> Option<T> {
        let held = self
            .db_connection
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        held.as_ref().map(job)
    }

    pub fn history_enabled(&self) -> bool {
        self.with_history(|_| ()).is_some()
    }

    /// Opening and closing the file is the whole of `daemon.save_history`, so a
    /// write between two dictations decides whether the next one is kept.
    pub fn set_history(&self, connection: Option<rusqlite::Connection>) {
        *self
            .db_connection
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = connection;
    }

    /// Called from the real time audio thread, so it must not lock or allocate.
    pub fn mark_capture_alive(&self) {
        let millis = self.started_at.elapsed().as_millis() as u64 + 1;
        self.capture_tick
            .store(millis, std::sync::atomic::Ordering::Relaxed);
    }

    /// A device that disappears stops the callback; it does not deliver silence.
    pub fn capture_is_stalled(&self) -> bool {
        let last = self.capture_tick.load(std::sync::atomic::Ordering::Relaxed);
        if last == 0 {
            return true;
        }
        let now = self.started_at.elapsed().as_millis() as u64;
        Duration::from_millis(now.saturating_sub(last)) > CAPTURE_SILENCE_LIMIT
    }

    // The bits of a non-negative f32 order as its values do, so the max bits are the max peak.
    pub fn note_peak(&self, peak: f32) {
        self.peak
            .fetch_max(peak.abs().to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, std::sync::atomic::Ordering::Relaxed))
    }

    fn set_answer_open(&self, open: bool) {
        self.answer_open
            .store(open, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_answer_open(&self) -> bool {
        self.answer_open.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Every call wakes each receiver, the same value too, so a client's
    /// smoothing settles when the level stops moving.
    pub fn publish_level(&self, level: f32) {
        self.level.send_replace(level.min(1.0));
    }

    pub fn subscribe_level(&self) -> watch::Receiver<f32> {
        self.level.subscribe()
    }

    pub fn watch_level(&self) -> LevelWatch {
        Counted::take(&self.level_watchers)
    }

    pub fn is_level_watched(&self) -> bool {
        *self.level_watchers.borrow() > 0
    }

    pub fn subscribe_level_watchers(&self) -> watch::Receiver<usize> {
        self.level_watchers.subscribe()
    }

    #[cfg(test)]
    pub fn level_watchers(&self) -> usize {
        *self.level_watchers.borrow()
    }

    /// Every call wakes each receiver, so a recording that ends with no words
    /// still sends its empty text.
    pub fn publish_words(&self, text: &str) {
        self.words.send_replace(text.to_string());
    }

    pub fn subscribe_words(&self) -> watch::Receiver<String> {
        self.words.subscribe()
    }

    /// Whether a client shows the words. Each subscriber's push task holds a
    /// receiver until its connection ends.
    pub fn is_words_watched(&self) -> bool {
        self.words.receiver_count() > 0
    }

    pub fn cues(&self) -> &Cues {
        &self.cues
    }
}

#[cfg(test)]
mod tests;

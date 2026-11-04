use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::config::Config;
use crate::history::TranscriptionHistory;
use crate::state::{ConsumerCommand, DaemonState};
use crate::text_to_speech::local::OnDemand;
use crate::text_to_speech::output::Output;
use crate::text_to_speech::{
    ActiveUtterance, BuildSpeaker, Faults, Selection, Speaker, SpeechPlayer, TtsBackend,
};

pub fn silero() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/silero_vad.onnx")
}

/// Reads one HTTP request off a loopback stream: the head, then as many body
/// bytes as its Content-Length names, and none when it names no length.
pub fn read_request(stream: &mut std::net::TcpStream) -> String {
    use std::io::Read;

    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut content_length = None;
    let mut header_end = None;
    loop {
        let read = stream.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if header_end.is_none()
            && let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n")
        {
            header_end = Some(end + 4);
            let head = String::from_utf8_lossy(&buffer[..end]).to_lowercase();
            content_length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok());
        }
        if let Some(end) = header_end
            && content_length.is_none_or(|length| buffer.len() >= end + length)
        {
            break;
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

/// A fresh temp directory no other test in this process shares, for a caller
/// that runs many times under one name.
pub fn unique_scratch(prefix: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    scratch(&format!("{prefix}-{serial}"))
}

/// A key file holding `text`, under a fresh temp directory.
pub fn key_file(text: &str) -> PathBuf {
    let path = unique_scratch("keys").join("credentials.toml");
    std::fs::write(&path, text).unwrap();
    path
}

/// A key file holding `key` for `side`, under a fresh temp directory.
pub fn key_file_holding(side: crate::credentials::RemoteKey, key: &str) -> PathBuf {
    let path = unique_scratch("keys").join("credentials.toml");
    write_key(&path, side, key);
    path
}

/// `key` for `side` written to the key file at `path`. An empty key removes it.
pub fn write_key(path: &Path, side: crate::credentials::RemoteKey, key: &str) {
    crate::credentials::Credentials::set_many_at(path, &[(side, key)]).unwrap();
}

/// A remote listener on the default server.
pub fn remote_stt_default() -> crate::config::STTConfig {
    crate::config::STTConfig {
        provider: crate::config::Provider::Remote,
        ..Default::default()
    }
}

/// A models folder, a config path and a key file path inside a fresh temp
/// directory, and VoiceOver off, so no test reads or writes the machine that
/// runs it.
pub fn scratch_machine(prefix: &str) -> crate::state::Machine {
    let dir = unique_scratch(prefix);
    let models = dir.join("models");
    std::fs::create_dir_all(&models).unwrap();
    crate::state::Machine {
        models,
        config: dir.join("config.toml"),
        credentials: dir.join("credentials.toml"),
        voiceover: || false,
    }
}

/// The daemon runs under a supervisor that hands it a short PATH, so a system
/// tool is named by where it lives. This checks it lives there.
pub fn tool_is_installed(path: &str) {
    assert!(
        std::path::Path::new(path).exists(),
        "{path} is not on this machine"
    );
}

/// A child process writes the file, so this process never holds it open for
/// writing. A process another test starts meanwhile would inherit that handle,
/// and the script would then fail to run with "Text file busy".
pub fn write_executable(path: &std::path::Path, body: &str) {
    use std::io::Write;
    let mut writer = std::process::Command::new("/bin/sh")
        .args(["-c", r#"cat > "$1" && chmod 755 "$1""#, "sh"])
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    writer
        .stdin
        .take()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    assert!(
        writer.wait().unwrap().success(),
        "could not write {}",
        path.display()
    );
}

/// A models folder holding an empty file for each name, under a fresh temp
/// directory. Enough for a check that files are there; nothing loads them.
pub fn models_holding(names: &[&str]) -> PathBuf {
    let models = unique_scratch("models-holding");
    write_empty(&models, names);
    models
}

/// An empty file in `models` for each name.
pub fn write_empty(models: &Path, names: &[&str]) {
    for name in names {
        let path = models.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
    }
}

/// A models folder holding the companion's files, and Kokoro's with each of `voices`.
pub fn companion_models_with(voices: &[&str]) -> PathBuf {
    let models = companion_models();
    for voice in voices {
        let files = crate::models::download::voice_files(voice);
        write_empty(
            &models,
            &files
                .iter()
                .map(|file| file.name.as_str())
                .collect::<Vec<_>>(),
        );
    }
    models
}

/// A models folder holding the companion's files.
pub fn companion_models() -> PathBuf {
    let files = crate::models::download::pocket_files();
    models_holding(
        &files
            .iter()
            .map(|file| file.name.as_str())
            .collect::<Vec<_>>(),
    )
}

/// A fault channel a test does not read.
pub fn ignored_faults() -> Faults {
    Faults::new(std::sync::mpsc::channel().0)
}

/// The companion on `models`, loaded by `load` on its first utterance.
pub fn on_demand(
    models: &Path,
    load: impl Fn(&str) -> Result<Box<dyn TtsBackend>, banshee_common::error::BansheeError>
    + Send
    + Sync
    + 'static,
    faults: Faults,
) -> OnDemand {
    OnDemand::new(
        models,
        banshee_common::COMPANION_VOICE,
        load,
        Arc::new(Output::silent()),
        faults,
    )
}

/// Makes a fresh temp directory. Deletes a leftover from a killed run first.
pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("banshee-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct NullBackend;
pub struct Done;

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
    ) -> Result<Box<dyn ActiveUtterance>, banshee_common::error::BansheeError> {
        Ok(Box::new(Done))
    }
}

/// What a backend was asked to say, and in which voice.
pub type Asked = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// Keeps what it is asked to say. Every line finishes at once.
pub struct Listening(pub Asked);

impl TtsBackend for Listening {
    fn start(
        &self,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Box<dyn ActiveUtterance>, banshee_common::error::BansheeError> {
        crate::text_to_speech::lock(&self.0).push((text.to_string(), voice.map(str::to_string)));
        Ok(Box::new(Done))
    }
}

/// A player whose every line finishes at once.
pub fn quiet_speech() -> SpeechPlayer {
    SpeechPlayer::new(Box::new(NullBackend))
}

/// A line with this text never ends, which is what a stalled backend looks like.
pub const STALLS: &str = "This line stalls.";

struct Held {
    until: Option<std::time::Instant>,
    cut_short: Arc<std::sync::atomic::AtomicBool>,
}

impl ActiveUtterance for Held {
    fn is_finished(&mut self) -> bool {
        self.until
            .is_some_and(|until| std::time::Instant::now() >= until)
    }
    fn stop(&mut self) {
        self.cut_short
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

struct HoldingBackend {
    hold: std::time::Duration,
    cut_short: Arc<std::sync::atomic::AtomicBool>,
    spoken: SpokenLines,
    sends_to: Option<String>,
}

impl TtsBackend for HoldingBackend {
    fn start(
        &self,
        text: &str,
        _voice: Option<&str>,
    ) -> Result<Box<dyn ActiveUtterance>, banshee_common::error::BansheeError> {
        self.spoken.lock().unwrap().push(text.to_string());
        Ok(Box::new(Held {
            until: (text != STALLS).then(|| std::time::Instant::now() + self.hold),
            cut_short: Arc::clone(&self.cut_short),
        }))
    }

    fn sends_to(&self) -> Option<&str> {
        self.sends_to.as_deref()
    }
}

/// Speech held for `hold` per line, and what each line said. The flag goes
/// true if an utterance is stopped before it finishes.
pub struct HoldingSpeech {
    pub state: Arc<DaemonState>,
    pub cut_short: Arc<std::sync::atomic::AtomicBool>,
    pub spoken: SpokenLines,
}

/// Its lines go to a server.
pub fn daemon_state_holding_speech(
    hold: std::time::Duration,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
) -> HoldingSpeech {
    holding_speech(hold, Some(HELD_SERVER.to_string()), |speech| {
        state(None, speech, commands)
    })
}

const HELD_SERVER: &str = "https://held.example.test/v1";

/// Speech held for `hold` per line, on a daemon that started on `config` and
/// built `speaker`, which sends its lines to `sends_to`.
pub fn daemon_state_holding_speech_under(
    config: Config,
    speaker: Speaker,
    hold: std::time::Duration,
    sends_to: Option<String>,
) -> HoldingSpeech {
    holding_speech(hold, sends_to, |speech| {
        state_running(config, None, speech, speaker, std::sync::mpsc::channel().0)
    })
}

fn holding_speech(
    hold: std::time::Duration,
    sends_to: Option<String>,
    state: impl FnOnce(SpeechPlayer) -> Arc<DaemonState>,
) -> HoldingSpeech {
    let cut_short: Arc<std::sync::atomic::AtomicBool> = Arc::default();
    let spoken: SpokenLines = Arc::default();
    let speech = SpeechPlayer::new(Box::new(HoldingBackend {
        hold,
        cut_short: Arc::clone(&cut_short),
        spoken: Arc::clone(&spoken),
        sends_to,
    }));
    HoldingSpeech {
        state: state(speech),
        cut_short,
        spoken,
    }
}

/// The voice each live `[tts]` write handed the backend.
pub type RecordedTts = Arc<Mutex<Vec<String>>>;

struct RecordingBackend(RecordedTts);

impl TtsBackend for RecordingBackend {
    fn start(
        &self,
        _text: &str,
        _voice: Option<&str>,
    ) -> Result<Box<dyn ActiveUtterance>, banshee_common::error::BansheeError> {
        Ok(Box::new(Done))
    }

    fn reconfigure(&self, tts: &crate::config::TTSConfig) -> Option<String> {
        self.0.lock().unwrap().push(tts.voice.clone());
        Some(tts.voice.clone())
    }
}

/// The lines the speech player was asked to say.
pub type SpokenLines = Arc<Mutex<Vec<String>>>;

struct SpeakingBackend(SpokenLines);

/// A backend that keeps every line it is asked to speak. Every line finishes
/// at once.
pub fn speaking_backend(lines: SpokenLines) -> Box<dyn TtsBackend> {
    Box::new(SpeakingBackend(lines))
}

impl TtsBackend for SpeakingBackend {
    fn start(
        &self,
        text: &str,
        _voice: Option<&str>,
    ) -> Result<Box<dyn ActiveUtterance>, banshee_common::error::BansheeError> {
        self.0.lock().unwrap().push(text.to_string());
        Ok(Box::new(Done))
    }
}

/// A speaker that keeps what it says in `lines`, as `speaker`.
pub fn speaking_selection(lines: SpokenLines, speaker: Speaker) -> Selection {
    Selection {
        backend: speaking_backend(lines),
        speaker,
        fault: None,
    }
}

/// A builder whose every speaker keeps what it says in `lines`, in `voice`.
pub fn here(lines: &SpokenLines, voice: &str) -> BuildSpeaker {
    let (lines, voice) = (lines.clone(), voice.to_string());
    Box::new(move |_| speaking_selection(lines.clone(), Speaker::Configured(voice.clone())))
}

/// A builder that waits up to 5 s for the gate before it hands back what
/// `select` builds. The receiver hears each build begin, and the sender is the
/// gate.
pub fn gated_builder(
    select: impl Fn(&crate::config::TTSConfig) -> Selection + Send + Sync + 'static,
) -> (
    BuildSpeaker,
    std::sync::mpsc::Receiver<()>,
    std::sync::mpsc::Sender<()>,
) {
    let (started, building) = std::sync::mpsc::channel();
    let (gate, opens) = std::sync::mpsc::channel::<()>();
    let opens = Mutex::new(opens);
    let build: BuildSpeaker = Box::new(move |tts| {
        started.send(()).unwrap();
        let _ = opens
            .lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(5));
        select(tts)
    });
    (build, building, gate)
}

/// A line that plays until its flag goes true.
pub struct UntilEnds(pub Arc<std::sync::atomic::AtomicBool>);

impl ActiveUtterance for UntilEnds {
    fn is_finished(&mut self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn stop(&mut self) {}
}

/// A backend whose every line refuses to start.
pub struct Refusing;

impl TtsBackend for Refusing {
    fn start(
        &self,
        _text: &str,
        _voice: Option<&str>,
    ) -> Result<Box<dyn ActiveUtterance>, banshee_common::error::BansheeError> {
        Err(banshee_common::error::BansheeError::Other(
            "no voice here".into(),
        ))
    }
}

/// A daemon state that keeps every line it was asked to speak. A queued line
/// reaches the backend from the watcher thread, so a test waits for it rather
/// than reading the list at once.
pub fn daemon_state_recording_speech_for(
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
) -> (Arc<DaemonState>, SpokenLines) {
    let spoken: SpokenLines = Arc::default();
    let speech = SpeechPlayer::new(Box::new(SpeakingBackend(spoken.clone())));
    (state(None, speech, commands), spoken)
}

/// A daemon caught before its pipeline stands that keeps every line it was
/// asked to speak.
pub fn daemon_state_before_the_pipeline_recording_speech() -> (Arc<DaemonState>, SpokenLines) {
    let spoken: SpokenLines = Arc::default();
    let state = built(
        Config::default(),
        None,
        SpeechPlayer::new(Box::new(SpeakingBackend(spoken.clone()))),
        Speaker::Fallback,
        std::sync::mpsc::channel().0,
        crate::audio::cues::Cues::silent(),
    );
    (state, spoken)
}

/// A running daemon that speaks through `speech`.
pub fn daemon_state_speaking_through(speech: SpeechPlayer) -> Arc<DaemonState> {
    state(None, speech, std::sync::mpsc::channel().0)
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
    state_running(
        Config::default(),
        history,
        speech,
        Speaker::Fallback,
        commands,
    )
}

/// A daemon caught before its pipeline stands, which is what the real one is
/// from `claim()` until the build thread finishes.
pub fn daemon_state_before_the_pipeline() -> Arc<DaemonState> {
    daemon_state_before_the_pipeline_with_cues(crate::audio::cues::Cues::silent())
}

/// Every other fixture stands for a daemon whose pipeline is up, which is the
/// premise of any test that records, arms or reports a fault.
fn state_running(
    config: Config,
    history: Option<rusqlite::Connection>,
    speech: SpeechPlayer,
    speaker: Speaker,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
) -> Arc<DaemonState> {
    let state = fresh(config, history, speech, speaker, commands);
    state.set_mic(crate::state::Mic::Closed);
    state
}

pub fn daemon_state_with_cues(cues: crate::audio::cues::Cues) -> Arc<DaemonState> {
    daemon_state_hearing(cues, std::sync::mpsc::channel().0)
}

/// A running daemon whose cues and consumer commands the test reads.
pub fn daemon_state_hearing(
    cues: crate::audio::cues::Cues,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
) -> Arc<DaemonState> {
    let state = built(
        Config::default(),
        None,
        SpeechPlayer::new(Box::new(NullBackend)),
        Speaker::Fallback,
        commands,
        cues,
    );
    state.set_mic(crate::state::Mic::Closed);
    state
}

pub fn daemon_state_before_the_pipeline_with_cues(
    cues: crate::audio::cues::Cues,
) -> Arc<DaemonState> {
    built(
        Config::default(),
        None,
        SpeechPlayer::new(Box::new(NullBackend)),
        Speaker::Fallback,
        std::sync::mpsc::channel().0,
        cues,
    )
}

fn fresh(
    config: Config,
    history: Option<rusqlite::Connection>,
    speech: SpeechPlayer,
    speaker: Speaker,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
) -> Arc<DaemonState> {
    built(
        config,
        history,
        speech,
        speaker,
        commands,
        crate::audio::cues::Cues::silent(),
    )
}

fn built(
    config: Config,
    history: Option<rusqlite::Connection>,
    speech: SpeechPlayer,
    speaker: Speaker,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
    cues: crate::audio::cues::Cues,
) -> Arc<DaemonState> {
    let listener = crate::models::stt_model(&config);
    let state = Arc::new(DaemonState::new(
        Arc::new(config),
        history,
        speech,
        speaker,
        commands,
        cues,
        scratch_machine("models"),
    ));
    // As a start does once the listener is built, whatever the scratch
    // directory holds.
    state.listening_model_loaded(listener);
    state
}

/// A daemon state whose `[tts]` built `speaker`, for the reply that says which
/// one started.
pub fn daemon_state_speaking(
    config: Config,
    speaker: Speaker,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
) -> Arc<DaemonState> {
    state_running(
        config,
        None,
        SpeechPlayer::new(Box::new(NullBackend)),
        speaker,
        commands,
    )
}

/// A daemon that started on `config`, built `speaker`, and speaks through
/// `speech`.
pub fn daemon_state_speaking_on(
    config: Config,
    speaker: Speaker,
    speech: SpeechPlayer,
) -> Arc<DaemonState> {
    state_running(config, None, speech, speaker, std::sync::mpsc::channel().0)
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

/// What a bench of microphones was asked, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchCall {
    Select(String),
    Open(String),
    Close(String),
    DefaultName,
}

type Slot = Arc<Mutex<Option<crate::audio::CaptureSink>>>;

/// Microphones a test sets out. A stream plays nothing until the test plays
/// audio into it, and dropping the stream drops its sink, as cpal does.
#[derive(Clone, Default)]
pub struct Bench {
    present: Arc<Mutex<Vec<String>>>,
    default: Arc<Mutex<Option<String>>>,
    refuses: Arc<Mutex<Option<String>>>,
    // What the grant answers. Not noted as a call: a read is not an open.
    access: Arc<Mutex<Option<crate::state::Refusal>>>,
    asked: Arc<Mutex<Vec<BenchCall>>>,
    told: Arc<Mutex<Option<std::sync::mpsc::Sender<BenchCall>>>>,
    streams: Arc<Mutex<Vec<Slot>>>,
}

pub struct BenchStream {
    name: String,
    sink: Slot,
    bench: Bench,
}

impl Drop for BenchStream {
    fn drop(&mut self) {
        *self.sink.lock().unwrap() = None;
        self.bench.note(BenchCall::Close(self.name.clone()));
    }
}

impl Bench {
    pub fn with(present: &[&str], default: Option<&str>) -> Self {
        let bench = Self::default();
        *bench.present.lock().unwrap() = present.iter().map(|name| name.to_string()).collect();
        bench.set_default(default);
        bench
    }

    pub fn unplug(&self, name: &str) {
        self.present
            .lock()
            .unwrap()
            .retain(|present| present != name);
    }

    pub fn set_default(&self, default: Option<&str>) {
        *self.default.lock().unwrap() = default.map(str::to_string);
    }

    /// What the grant answers.
    pub fn refuse_access(&self, refusal: Option<crate::state::Refusal>) {
        *self.access.lock().unwrap() = refusal;
    }

    /// The device of this name lists itself and will not open.
    pub fn refuse(&self, name: Option<&str>) {
        *self.refuses.lock().unwrap() = name.map(str::to_string);
    }

    pub fn asked(&self) -> Vec<BenchCall> {
        self.asked.lock().unwrap().clone()
    }

    /// Every later question is sent here as well.
    pub fn tell(&self) -> std::sync::mpsc::Receiver<BenchCall> {
        let (told, telling) = std::sync::mpsc::channel();
        *self.told.lock().unwrap() = Some(told);
        telling
    }

    /// Plays `samples` through the stream opened last, as its callback would.
    pub fn play(&self, samples: &[f32]) {
        let slot = Arc::clone(
            self.streams
                .lock()
                .unwrap()
                .last()
                .expect("a stream opened"),
        );
        slot.lock()
            .unwrap()
            .as_mut()
            .expect("the stream still plays")
            .take(samples);
    }

    fn note(&self, asked: BenchCall) {
        self.asked.lock().unwrap().push(asked.clone());
        if let Some(told) = self.told.lock().unwrap().as_ref() {
            let _ = told.send(asked);
        }
    }
}

impl crate::audio::watchdog::Devices for Bench {
    type Device = ();
    type Stream = BenchStream;

    fn select(&mut self, wanted: &str) -> Result<crate::audio::Selection<()>, String> {
        self.note(BenchCall::Select(wanted.to_string()));
        let present = self.present.lock().unwrap().clone();
        let default = self.default.lock().unwrap().clone();
        let (index, missing) = crate::audio::pick(&present, wanted, default.as_deref())?;
        Ok(crate::audio::Selection {
            device: (),
            open: present[index].clone(),
            missing,
        })
    }

    fn open(
        &mut self,
        selection: &crate::audio::Selection<()>,
        sink: &mut dyn FnMut(u32, usize) -> crate::audio::CaptureSink,
    ) -> Result<BenchStream, String> {
        if self.refuses.lock().unwrap().as_deref() == Some(selection.open.as_str()) {
            return Err(crate::audio::open_failure(&selection.open, "Host is down"));
        }
        let slot: Slot = Arc::new(Mutex::new(Some(sink(
            crate::speech_to_text::SAMPLE_RATE,
            1,
        ))));
        self.streams.lock().unwrap().push(Arc::clone(&slot));
        self.note(BenchCall::Open(selection.open.clone()));
        Ok(BenchStream {
            name: selection.open.clone(),
            sink: slot,
            bench: self.clone(),
        })
    }

    fn default_name(&mut self) -> Option<String> {
        self.note(BenchCall::DefaultName);
        self.default.lock().unwrap().clone()
    }

    fn refusal(&mut self) -> Option<crate::state::Refusal> {
        *self.access.lock().unwrap()
    }
}

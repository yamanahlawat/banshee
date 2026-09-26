use std::path::PathBuf;
use std::sync::Arc;

use crate::config::Config;
use crate::history::TranscriptionHistory;
use crate::state::{ConsumerCommand, DaemonState};
use crate::text_to_speech::{ActiveUtterance, Speaker, SpeechPlayer, TtsBackend};

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

/// A models folder and a config path inside a fresh temp directory, and
/// VoiceOver off, so no test reads or writes the machine that runs it.
pub fn scratch_machine(prefix: &str) -> crate::state::Machine {
    let dir = unique_scratch(prefix);
    let models = dir.join("models");
    std::fs::create_dir_all(&models).unwrap();
    crate::state::Machine {
        models,
        config: dir.join("config.toml"),
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

/// Writes `body` to `path` as an executable script.
///
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

/// Makes a fresh temp directory. Deletes a leftover from a killed run first.
pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("banshee-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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
    ) -> Result<Box<dyn ActiveUtterance>, banshee_common::error::BansheeError> {
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
    ) -> Result<Box<dyn ActiveUtterance>, banshee_common::error::BansheeError> {
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
    ) -> Result<Box<dyn ActiveUtterance>, banshee_common::error::BansheeError> {
        Ok(Box::new(Done))
    }

    fn reconfigure(&self, tts: &crate::config::TTSConfig) -> Option<String> {
        self.0.lock().unwrap().push((tts.voice.clone(), tts.speed));
        Some(tts.voice.clone())
    }
}

/// The lines the speech player was asked to say.
pub type SpokenLines = Arc<std::sync::Mutex<Vec<String>>>;

struct SpeakingBackend(SpokenLines);

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

/// A daemon state that keeps every line it was asked to speak. A queued line
/// reaches the backend from the watcher thread, so a test waits for it rather
/// than reading the list at once.
pub fn daemon_state_recording_speech() -> (Arc<DaemonState>, SpokenLines) {
    let spoken: SpokenLines = Arc::default();
    let speech = SpeechPlayer::new(Box::new(SpeakingBackend(spoken.clone())));
    (state(None, speech, std::sync::mpsc::channel().0), spoken)
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
    state.set_pipeline(crate::state::Pipeline::Open);
    state
}

pub fn daemon_state_with_cues(cues: crate::audio::cues::Cues) -> Arc<DaemonState> {
    let state = daemon_state_before_the_pipeline_with_cues(cues);
    state.set_pipeline(crate::state::Pipeline::Open);
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
    Arc::new(DaemonState::new(
        Arc::new(config),
        history,
        speech,
        speaker,
        commands,
        cues,
        scratch_machine("models"),
    ))
}

/// A daemon state that started with `config`, so its restart-only keys run those values.
pub fn daemon_state_running(
    config: Config,
    commands: std::sync::mpsc::Sender<ConsumerCommand>,
) -> Arc<DaemonState> {
    daemon_state_speaking(config, Speaker::Fallback, commands)
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

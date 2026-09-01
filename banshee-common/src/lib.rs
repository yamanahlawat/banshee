use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod error;
pub mod utils;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Version {
    #[default]
    #[serde(rename = "2.0")]
    V2,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub id: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum JsonRpcResponse {
    Success {
        jsonrpc: Version,
        result: Value,
        id: Option<Value>,
    },
    Error {
        jsonrpc: Version,
        error: JsonRpcError,
        id: Option<Value>,
    },
}

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        JsonRpcResponse::Success {
            jsonrpc: Version::V2,
            result,
            id,
        }
    }

    pub fn error(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        JsonRpcResponse::Error {
            jsonrpc: Version::V2,
            error: JsonRpcError {
                code,
                message: message.into(),
            },
            id,
        }
    }
}

/// A message the daemon sends unprompted. JSON-RPC marks these by the absent
/// `id`, and expects no reply.
#[derive(Serialize, Deserialize, Debug)]
pub struct JsonRpcNotification {
    pub jsonrpc: Version,
    pub method: String,
    pub params: Value,
}

impl JsonRpcNotification {
    pub fn new(method: &str, params: Value) -> Self {
        Self {
            jsonrpc: Version::V2,
            method: method.to_string(),
            params,
        }
    }
}

pub const BANSHEE_SPEAK: &str = "banshee.speak";
pub const BANSHEE_STOP_SPEAKING: &str = "banshee.stop_speaking";
pub const BANSHEE_STATUS: &str = "banshee.status";
pub const BANSHEE_CONFIGURE: &str = "banshee.configure";
pub const BANSHEE_GET_TRANSCRIPTION: &str = "banshee.get_transcription";
pub const BANSHEE_HISTORY: &str = "banshee.history";
pub const BANSHEE_CLEAR_HISTORY: &str = "banshee.clear_history";
pub const BANSHEE_ASK_USER: &str = "banshee.ask_user";
pub const BANSHEE_STOP: &str = "banshee.stop";
pub const BANSHEE_RECORD_START: &str = "banshee.record_start";
pub const BANSHEE_RECORD_STOP: &str = "banshee.record_stop";
pub const BANSHEE_LIST_INPUT_DEVICES: &str = "banshee.list_input_devices";
pub const BANSHEE_LIST_VOICES: &str = "banshee.list_voices";
pub const BANSHEE_LIST_LANGUAGES: &str = "banshee.list_languages";
pub const BANSHEE_DOWNLOAD_MODELS: &str = "banshee.download_models";
pub const BANSHEE_SUBSCRIBE: &str = "banshee.subscribe";
pub const BANSHEE_AGENTS: &str = "banshee.agents";
pub const BANSHEE_CONNECT_PLAN: &str = "banshee.connect_plan";
pub const BANSHEE_CONNECT_APPLY: &str = "banshee.connect_apply";
pub const BANSHEE_OPEN_PERMISSION: &str = "banshee.open_permission";
// Sent by the daemon, not called by a client
pub const BANSHEE_STATE_CHANGED: &str = "banshee.state_changed";
pub const BANSHEE_DOWNLOAD_PROGRESS: &str = "banshee.download_progress";

// What `banshee.subscribe` accepts in `events`, spelled once for both sides
pub const EVENT_STATE: &str = "state";
pub const EVENT_DOWNLOADS: &str = "downloads";

/// The microphone the daemon records from. A `banshee.status` reply and a
/// `state_changed` push both carry it, so a subscriber reads it from every
/// update rather than once on open.
pub fn audio_device(status: &Value) -> Option<&str> {
    status.get("audio_device").and_then(Value::as_str)
}

/// The device the config waits for while a substitute records. Not a blocker:
/// a daemon on a substitute records correctly. `None` whenever the wanted
/// device is open, and whenever nothing at all is open.
pub fn missing_device(status: &Value) -> Option<&str> {
    status.get("missing_device").and_then(Value::as_str)
}

/// What `IOHIDCheckAccess` answered in the daemon: `granted`, `denied` or
/// `undetermined`. `None` from a daemon older than the field.
pub fn key_press_access(status: &Value) -> Option<&str> {
    status.get("key_press_access").and_then(Value::as_str)
}

/// The sentence every surface shows for these two fields. Each client adds its
/// own lead-in and nothing else, so all of them spell the absent case alike.
pub fn microphone_label(open: Option<&str>, missing: Option<&str>) -> String {
    let open = open.unwrap_or("No microphone");
    match missing {
        // The quotes keep two multi-word names apart
        Some(missing) => format!("{open} (waiting for \"{missing}\")"),
        None => open.to_string(),
    }
}

/// What the daemon is doing, read from the `recording` and `speaking` flags.
/// Both a `banshee.status` reply and a `state_changed` push carry them. Each
/// surface names these for itself; only the ranking lives here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Recording,
    Speaking,
    Listening,
}

impl Activity {
    // The microphone outranks the speaker: it is what the user is waiting on,
    // and both are true at once when barge-in is off. Waiting on an answer
    // outranks both: the daemon opens the microphone to hear one, so `armed`
    // arrives with `recording` already true, and it is the only one of the
    // three where doing nothing is the wrong response.
    pub fn of(state: &Value) -> Self {
        let flag = |name| state.get(name).and_then(Value::as_bool) == Some(true);
        if flag("armed") {
            Activity::Listening
        } else if flag("recording") {
            Activity::Recording
        } else if flag("speaking") {
            Activity::Speaking
        } else {
            Activity::Idle
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    Downloading,
    Done,
    Failed,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DownloadProgress {
    pub model: String,
    /// What the file is, in the user's words. `model` stays the filename.
    #[serde(default)]
    pub label: String,
    /// One-based place in this run, and how many files the run has.
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub count: usize,
    pub bytes: u64,
    /// None when the server sends no `Content-Length`, so a client shows a
    /// spinner rather than a bar.
    pub total: Option<u64>,
    pub state: DownloadState,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InputDevice {
    pub name: String,
    pub default: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlockerKind {
    Permission,
    Model,
    Pipeline,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Blocker {
    pub kind: BlockerKind,
    pub id: String,
    pub name: String,
    /// Which file this is, for a client that has to tell one from another.
    /// Absent on a blocker that names no file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<FileRole>,
    /// What clears it. A daemon older than this field names only a `command`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remedy: Option<Remedy>,
    pub consequence: String,
    pub fix: String,
    /// The command that clears this blocker, where one exists. `fix` says the
    /// same thing in a sentence; this is the part a client can run or copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// What a model file is. The prose beside it is for reading; this is what a
/// client routes on, so rewording one cannot move the other.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileRole {
    Speech,
    Detector,
    Engine,
    Voice,
}

/// What clears a blocker. `kind` says which part is at fault and `command` is
/// the line a person could run; neither answers this.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Remedy {
    Download,
    Restart,
    Grant,
}

/// A language Whisper can read, as the engine itself spells it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Language {
    /// What `stt.language` carries, as in `en` or `hi`.
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Voice {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Whether the file is on this machine. A client that can fetch one offers
    /// every voice; one that cannot shows only the voices that work today.
    /// Absent from a daemon older than this field, which listed only what it
    /// held, so the voices it names are all installed.
    #[serde(default = "yes")]
    pub downloaded: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRow {
    /// The slug `banshee connect <id>` takes.
    pub id: String,
    pub name: String,
    /// "connected", "found" or "absent".
    pub presence: String,
    /// One line for the row, in the user's words.
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedChange {
    /// The file the change writes. None when the change runs a command.
    pub path: Option<String>,
    /// What `banshee connect` prints for this change, unchanged.
    pub diff: String,
}

// Whisper model configuration
pub struct WhisperConfig {
    pub model_name: String,
    pub download_url: String,
}

impl WhisperConfig {
    pub fn new(model_name: &str) -> Self {
        Self {
            model_name: model_name.to_string(),
            download_url: format!(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
                model_name
            ),
        }
    }
}

// Silero VAD configuration
pub struct SileroVADConfig {
    pub model_name: String,
    pub download_url: String,
}

impl SileroVADConfig {
    pub fn new(model_name: &str) -> Self {
        Self {
            model_name: model_name.to_string(),
            download_url: format!(
                "https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/{}",
                model_name
            ),
        }
    }
}

// Kokoro TTS configuration: the model plus one style file per voice,
// pinned to a revision so a repo update can't silently change the model
pub struct KokoroTTSConfig {
    pub model_name: String,
    pub model_url: String,
    pub voice_name: String,
    pub voice_url: String,
}

/// The engine every voice speaks through, which is not itself a voice.
pub const KOKORO_MODEL: &str = "kokoro-v1.0.onnx";

impl KokoroTTSConfig {
    pub fn new(voice: &str) -> Self {
        const KOKORO_REPO: &str = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/1939ad2a8e416c0acfeecc08a694d14ef25f2231";
        Self {
            model_name: KOKORO_MODEL.to_string(),
            model_url: format!("{KOKORO_REPO}/onnx/model.onnx"),
            voice_name: format!("{voice}.bin"),
            voice_url: format!("{KOKORO_REPO}/voices/{voice}.bin"),
        }
    }
}

#[cfg(test)]
mod wire_tests {
    use super::{
        Activity, BANSHEE_STATE_CHANGED, Blocker, BlockerKind, DownloadProgress, DownloadState,
        InputDevice, JsonRpcNotification,
    };

    #[test]
    fn a_blocker_that_names_a_command_puts_it_on_the_wire() {
        let blocker = Blocker {
            kind: BlockerKind::Model,
            role: None,
            remedy: None,
            id: "silero_vad.onnx".to_string(),
            name: "silero_vad.onnx".to_string(),
            consequence: "recording does not work".to_string(),
            fix: "run: banshee setup".to_string(),
            command: Some("banshee setup".to_string()),
        };
        let wire = serde_json::to_value(&blocker).unwrap();
        assert_eq!(wire["command"], "banshee setup");
    }

    /// A grant has no command, and the key stays off the wire rather than
    /// reaching a client as a null it has to test for.
    #[test]
    fn a_blocker_serializes_with_the_keys_clients_read() {
        let blocker = Blocker {
            kind: BlockerKind::Permission,
            role: None,
            remedy: None,
            id: "accessibility".to_string(),
            name: "Accessibility".to_string(),
            consequence: "dictation cannot type".to_string(),
            fix: "grant it in System Settings".to_string(),
            command: None,
        };
        let wire = serde_json::to_value(&blocker).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({
                "kind": "permission",
                "id": "accessibility",
                "name": "Accessibility",
                "consequence": "dictation cannot type",
                "fix": "grant it in System Settings",
            })
        );
    }

    #[test]
    fn the_model_kind_is_snake_case_too() {
        let wire = serde_json::to_value(BlockerKind::Model).unwrap();
        assert_eq!(wire, serde_json::json!("model"));
    }

    #[test]
    fn a_notification_carries_no_id() {
        let wire = serde_json::to_value(JsonRpcNotification::new(
            BANSHEE_STATE_CHANGED,
            serde_json::json!({"recording": true, "speaking": false}),
        ))
        .unwrap();
        assert_eq!(
            wire,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "banshee.state_changed",
                "params": {"recording": true, "speaking": false},
            })
        );
    }

    #[test]
    fn progress_serializes_with_the_keys_clients_read() {
        let wire = serde_json::to_value(DownloadProgress {
            model: "ggml-base.en.bin".to_string(),
            label: "Speech model".to_string(),
            index: 1,
            count: 3,
            bytes: 512,
            total: Some(1024),
            state: DownloadState::Downloading,
        })
        .unwrap();
        assert_eq!(
            wire,
            serde_json::json!({
                "model": "ggml-base.en.bin",
                "label": "Speech model",
                "index": 1,
                "count": 3,
                "bytes": 512,
                "total": 1024,
                "state": "downloading",
            })
        );
    }

    #[test]
    fn an_unknown_total_stays_on_the_wire_as_null() {
        let wire = serde_json::to_value(DownloadProgress {
            model: "af_sky.bin".to_string(),
            label: "Voice".to_string(),
            index: 1,
            count: 1,
            bytes: 7,
            total: None,
            state: DownloadState::Failed,
        })
        .unwrap();
        assert!(wire["total"].is_null(), "{wire}");
        assert_eq!(wire["state"], "failed");
    }

    #[test]
    fn the_microphone_outranks_the_speaker() {
        let both = serde_json::json!({"recording": true, "speaking": true});
        assert_eq!(
            Activity::of(&both),
            Activity::Recording,
            "the mic is what the user waits on"
        );
        let speaking = serde_json::json!({"recording": false, "speaking": true});
        assert_eq!(Activity::of(&speaking), Activity::Speaking);
        let idle = serde_json::json!({"recording": false, "speaking": false});
        assert_eq!(Activity::of(&idle), Activity::Idle);
    }

    // An older daemon says less than this build reads
    #[test]
    fn a_payload_missing_its_fields_reads_as_idle() {
        assert_eq!(Activity::of(&serde_json::json!({})), Activity::Idle);
    }

    // The daemon sets `recording` whenever it sets `armed`, so an armed state
    // that is ranked on `recording` first reads as ordinary dictation. It is
    // the one state where doing nothing is the wrong answer, so it outranks.
    #[test]
    fn waiting_on_an_answer_outranks_the_microphone_it_opened() {
        let armed = serde_json::json!({"recording": true, "armed": true, "speaking": false});
        assert_eq!(Activity::of(&armed), Activity::Listening);

        let dictating = serde_json::json!({"recording": true, "armed": false});
        assert_eq!(Activity::of(&dictating), Activity::Recording);
    }

    #[test]
    fn a_device_serializes_with_the_keys_clients_read() {
        let wire = serde_json::to_value(InputDevice {
            name: "Blue Yeti".to_string(),
            default: true,
        })
        .unwrap();
        assert_eq!(
            wire,
            serde_json::json!({"name": "Blue Yeti", "default": true})
        );
    }
}

#[cfg(test)]
mod label_tests {
    use super::microphone_label;

    // The tray, `banshee status` and `banshee watch --waybar` all show this
    // sentence, so it is spelled once and tested once
    #[test]
    fn the_microphone_label_covers_both_fields() {
        for (open, missing, expected) in [
            (
                Some("MacBook Pro Microphone"),
                Some("yeti"),
                "MacBook Pro Microphone (waiting for \"yeti\")",
            ),
            (
                Some("MacBook Pro Microphone"),
                None,
                "MacBook Pro Microphone",
            ),
            (None, Some("yeti"), "No microphone (waiting for \"yeti\")"),
            (None, None, "No microphone"),
        ] {
            assert_eq!(microphone_label(open, missing), expected);
        }
    }
}

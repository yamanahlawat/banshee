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
pub const BANSHEE_DOWNLOAD_MODELS: &str = "banshee.download_models";
pub const BANSHEE_SUBSCRIBE: &str = "banshee.subscribe";
// Sent by the daemon, not called by a client
pub const BANSHEE_STATE_CHANGED: &str = "banshee.state_changed";
pub const BANSHEE_DOWNLOAD_PROGRESS: &str = "banshee.download_progress";

// What `banshee.subscribe` accepts in `events`, spelled once for both sides
pub const EVENT_STATE: &str = "state";
pub const EVENT_DOWNLOADS: &str = "downloads";

/// What the daemon is doing, derived from a `state_changed` payload. Each
/// surface names these for itself; only the ranking lives here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Recording,
    Speaking,
}

impl Activity {
    // The microphone outranks the speaker: it is what the user is waiting on,
    // and both are true at once when barge-in is off
    pub fn of(state: &Value) -> Self {
        let flag = |name| state.get(name).and_then(Value::as_bool) == Some(true);
        if flag("recording") {
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
    pub consequence: String,
    pub fix: String,
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

impl KokoroTTSConfig {
    pub fn new(voice: &str) -> Self {
        const KOKORO_REPO: &str = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/1939ad2a8e416c0acfeecc08a694d14ef25f2231";
        Self {
            model_name: "kokoro-v1.0.onnx".to_string(),
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
    fn a_blocker_serializes_with_the_keys_clients_read() {
        let blocker = Blocker {
            kind: BlockerKind::Permission,
            id: "input_monitoring".to_string(),
            name: "Input Monitoring".to_string(),
            consequence: "the hotkey receives no key presses".to_string(),
            fix: "grant it in System Settings".to_string(),
        };
        let wire = serde_json::to_value(&blocker).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({
                "kind": "permission",
                "id": "input_monitoring",
                "name": "Input Monitoring",
                "consequence": "the hotkey receives no key presses",
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
            bytes: 512,
            total: Some(1024),
            state: DownloadState::Downloading,
        })
        .unwrap();
        assert_eq!(
            wire,
            serde_json::json!({
                "model": "ggml-base.en.bin",
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

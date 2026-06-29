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

pub const BANSHEE_SPEAK: &str = "banshee.speak";
pub const BANSHEE_STATUS: &str = "banshee.status";
pub const BANSHEE_CONFIGURE: &str = "banshee.configure";
pub const BANSHEE_GET_TRANSCRIPTION: &str = "banshee.get_transcription";
pub const BANSHEE_HISTORY: &str = "banshee.history";
pub const BANSHEE_CLEAR_HISTORY: &str = "banshee.clear_history";

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

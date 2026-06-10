use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod utils;

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
        jsonrpc: String,
        result: Value,
        id: Option<Value>,
    },
    Error {
        jsonrpc: String,
        error: JsonRpcError,
        id: Option<Value>,
    },
}

pub const BANSHEE_SPEAK: &str = "banshee.speak";
pub const BANSHEE_STATUS: &str = "banshee.status";
pub const BANSHEE_CONFIGURE: &str = "banshee.configure";
pub const BANSHEE_GET_TRANSCRIPTION: &str = "banshee.get_transcription";

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

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod utils;

#[derive(Serialize, Deserialize, Debug)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Value>,
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

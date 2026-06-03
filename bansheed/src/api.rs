use banshee_common::{BANSHEE_CONFIGURE, BANSHEE_GET_TRANSCRIPTION, BANSHEE_SPEAK, BANSHEE_STATUS};
use banshee_common::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use serde_json;

pub fn dispatch(request: JsonRpcRequest) -> JsonRpcResponse {
    match request.method.as_str() {
        BANSHEE_SPEAK => JsonRpcResponse::Success {
            jsonrpc: "2.0".to_string(),
            result: serde_json::json!({"ok": true}),
            id: request.id,
        },
        BANSHEE_GET_TRANSCRIPTION => JsonRpcResponse::Success {
            jsonrpc: "2.0".to_string(),
            result: serde_json::json!({"ok": true}),
            id: request.id,
        },
        BANSHEE_CONFIGURE => JsonRpcResponse::Success {
            jsonrpc: "2.0".to_string(),
            result: serde_json::json!({"ok": true}),
            id: request.id,
        },
        BANSHEE_STATUS => JsonRpcResponse::Success {
            jsonrpc: "2.0".to_string(),
            result: serde_json::json!({"ok": true}),
            id: request.id,
        },
        _ => JsonRpcResponse::Error {
            jsonrpc: "2.0".to_string(),
            error: JsonRpcError {
                code: -32601,
                message: "Method not found!".to_string(),
            },
            id: request.id,
        },
    }
}

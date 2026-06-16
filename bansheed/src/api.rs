use std::sync::Arc;

use banshee_common::{BANSHEE_CONFIGURE, BANSHEE_GET_TRANSCRIPTION, BANSHEE_SPEAK, BANSHEE_STATUS};
use banshee_common::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use serde_json;

use crate::speech_to_text::mailbox::TRANSCRIPTION_MAILBOX;
use crate::state::DaemonState;
use crate::text_to_speech::sanitizer::sanitize;

pub fn dispatch(request: JsonRpcRequest, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    match request.method.as_str() {
        BANSHEE_SPEAK => {
            daemon_state.set_speaking(true);
            if let Some(params) = &request.params {
                if let Some(raw_text) = params.get("text").and_then(|v| v.as_str()) {
                    let clean_text = sanitize(&raw_text);
                    println!("The sanitized text: {clean_text}");

                    std::process::Command::new("say")
                        .arg(&clean_text)
                        .spawn()
                        .expect("Failed to run 'say' command");
                }
            }
            JsonRpcResponse::Success {
                jsonrpc: "2.0".to_string(),
                result: serde_json::json!({"ok": true}),
                id: request.id,
            }
        }
        BANSHEE_GET_TRANSCRIPTION => {
            let transcription_text = TRANSCRIPTION_MAILBOX.lock().unwrap().take();
            JsonRpcResponse::Success {
                jsonrpc: "2.0".to_string(),
                result: serde_json::json!({"ok": true, "transcription": transcription_text}),
                id: request.id,
            }
        }
        BANSHEE_CONFIGURE => JsonRpcResponse::Success {
            jsonrpc: "2.0".to_string(),
            result: serde_json::json!({"ok": true}),
            id: request.id,
        },
        BANSHEE_STATUS => JsonRpcResponse::Success {
            jsonrpc: "2.0".to_string(),
            result: serde_json::json!({
                "running": true,
                "version": daemon_state.version(),
                "stt_model": daemon_state.stt_model(),
                "vad_model": daemon_state.vad_model(),
                "audio_device": daemon_state.audio_device(),
                "recording": daemon_state.is_recording(),
                "speaking": daemon_state.is_speaking(),
                "uptime_seconds": &daemon_state.uptime().as_secs(),
            }),
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

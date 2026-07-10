use std::sync::Arc;
use std::time::Duration;

use banshee_common::{
    BANSHEE_CLEAR_HISTORY, BANSHEE_CONFIGURE, BANSHEE_GET_TRANSCRIPTION, BANSHEE_HISTORY,
    BANSHEE_SPEAK, BANSHEE_STATUS,
};
use banshee_common::{JsonRpcRequest, JsonRpcResponse};

use crate::state::DaemonState;
use crate::text_to_speech::sanitizer::sanitize;

const MAX_WAIT_MS: u64 = 30_000;

pub async fn dispatch(request: JsonRpcRequest, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    match request.method.as_str() {
        BANSHEE_SPEAK => {
            if let Some(params) = &request.params
                && let Some(raw_text) = params.get("text").and_then(|v| v.as_str())
            {
                let clean_text = sanitize(raw_text);
                println!("The sanitized text: {clean_text}");

                // TODO: say is a temporary shim, replaced by a proper TTS engine in the future
                if let Err(e) = std::process::Command::new("say").arg(&clean_text).spawn() {
                    eprintln!("Failed to run 'say' command: {e}");
                }
            }

            JsonRpcResponse::success(request.id, serde_json::json!({}))
        }
        BANSHEE_GET_TRANSCRIPTION => {
            let mut since_id: u64 = 0;
            let mut wait_ms: u64 = 0;
            if let Some(params) = &request.params {
                if let Some(value) = params.get("since_id") {
                    let Some(parsed) = value.as_u64() else {
                        return JsonRpcResponse::error(
                            request.id,
                            -32602,
                            "'since_id' must be a non-negative integer.",
                        );
                    };
                    since_id = parsed;
                }
                if let Some(value) = params.get("wait_ms") {
                    let Some(parsed) = value.as_u64() else {
                        return JsonRpcResponse::error(
                            request.id,
                            -32602,
                            "'wait_ms' must be a non-negative integer.",
                        );
                    };
                    wait_ms = parsed.min(MAX_WAIT_MS);
                }
            }

            // Subscribe before the first read so a push landing in between
            // is still seen by wait_for
            let mut latest_id = daemon_state.subscribe_transcriptions();

            // since_id ahead of the newest id = stale cursor from an older daemon run
            if since_id > *latest_id.borrow() {
                since_id = 0;
            }

            let mut transcriptions = daemon_state.transcriptions_since(since_id);

            if transcriptions.is_empty() && wait_ms > 0 {
                let _ = tokio::time::timeout(
                    Duration::from_millis(wait_ms),
                    latest_id.wait_for(|id| *id > since_id),
                )
                .await;
                transcriptions = daemon_state.transcriptions_since(since_id);
            }

            JsonRpcResponse::success(
                request.id,
                serde_json::json!({ "transcriptions": transcriptions }),
            )
        }
        BANSHEE_CONFIGURE => {
            if let Some(params) = &request.params
                && let Some(threshold_val) = params.get("vad_threshold")
            {
                let Some(vad_threshold) = threshold_val.as_f64() else {
                    return JsonRpcResponse::error(
                        request.id,
                        -32602,
                        "'vad_threshold' must be a numeric float.",
                    );
                };

                if !(0.0..=1.0).contains(&vad_threshold) {
                    return JsonRpcResponse::error(
                        request.id,
                        -32602,
                        format!(
                            "Invalid VAD threshold: {}. Must be between 0.0 and 1.0",
                            vad_threshold
                        ),
                    );
                }
                daemon_state.set_vad_threshold(vad_threshold as f32);
            }

            JsonRpcResponse::success(request.id, serde_json::json!({}))
        }
        BANSHEE_STATUS => JsonRpcResponse::success(
            request.id,
            serde_json::json!({
                "running": true,
                "version": daemon_state.version(),
                "stt_model": daemon_state.stt_model(),
                "vad_model": daemon_state.vad_model(),
                "audio_device": daemon_state.audio_device(),
                "recording": daemon_state.is_recording(),
                "uptime_seconds": &daemon_state.uptime().as_secs(),
                "vad_threshold": daemon_state.vad_threshold(),
                "history_enabled": daemon_state.db_connection().is_some(),
            }),
        ),
        BANSHEE_HISTORY => {
            if let Some(db) = daemon_state.db_connection() {
                match db.lock() {
                    Ok(connection) => {
                        match crate::history::TranscriptionHistory::list(&connection) {
                            Ok(history) => JsonRpcResponse::success(
                                request.id,
                                serde_json::json!({ "history": history }),
                            ),
                            Err(e) => JsonRpcResponse::error(
                                request.id,
                                -32603,
                                format!("Failed to retrieve history: {e}"),
                            ),
                        }
                    }
                    Err(e) => JsonRpcResponse::error(
                        request.id,
                        -32603,
                        format!("Failed to lock database connection: {e}"),
                    ),
                }
            } else {
                JsonRpcResponse::error(request.id, -32003, "History is not enabled.")
            }
        }
        BANSHEE_CLEAR_HISTORY => {
            if let Some(db) = daemon_state.db_connection() {
                match db.lock() {
                    Ok(connection) => {
                        match crate::history::TranscriptionHistory::clear(&connection) {
                            Ok(_) => JsonRpcResponse::success(request.id, serde_json::json!({})),
                            Err(e) => JsonRpcResponse::error(
                                request.id,
                                -32003,
                                format!("Failed to clear history: {e}"),
                            ),
                        }
                    }
                    Err(e) => JsonRpcResponse::error(
                        request.id,
                        -32603,
                        format!("Failed to lock database connection: {e}"),
                    ),
                }
            } else {
                JsonRpcResponse::error(request.id, -32003, "History is not enabled.")
            }
        }
        _ => JsonRpcResponse::error(
            request.id,
            -32601,
            format!("Method '{}' not found!", request.method),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_transcription_request(params: serde_json::Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: BANSHEE_GET_TRANSCRIPTION.to_string(),
            params: Some(params),
            id: Some(serde_json::json!(1)),
        }
    }

    #[tokio::test]
    async fn stale_cursor_is_clamped_to_zero() {
        let state = Arc::new(DaemonState::new("0.0.0", "stt", "vad", 0.5, None));
        state.push_transcription("hello".to_string());

        let request = get_transcription_request(serde_json::json!({"since_id": 999}));
        let response = dispatch(request, &state).await;

        let JsonRpcResponse::Success { result, .. } = response else {
            panic!("expected success response");
        };
        let transcriptions = result["transcriptions"].as_array().unwrap();
        assert_eq!(transcriptions.len(), 1);
        assert_eq!(transcriptions[0]["text"], "hello");
    }

    #[tokio::test]
    async fn caught_up_cursor_returns_empty() {
        let state = Arc::new(DaemonState::new("0.0.0", "stt", "vad", 0.5, None));
        state.push_transcription("hello".to_string());

        let request = get_transcription_request(serde_json::json!({"since_id": 1}));
        let response = dispatch(request, &state).await;

        let JsonRpcResponse::Success { result, .. } = response else {
            panic!("expected success response");
        };
        assert!(result["transcriptions"].as_array().unwrap().is_empty());
    }
}

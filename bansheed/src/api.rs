use std::sync::Arc;

use banshee_common::{BANSHEE_CONFIGURE, BANSHEE_GET_TRANSCRIPTION, BANSHEE_SPEAK, BANSHEE_STATUS};
use banshee_common::{JsonRpcRequest, JsonRpcResponse};

use crate::speech_to_text::mailbox::TRANSCRIPTION_MAILBOX;
use crate::state::DaemonState;
use crate::text_to_speech::sanitizer::sanitize;

pub fn dispatch(request: JsonRpcRequest, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    match request.method.as_str() {
        BANSHEE_SPEAK => {
            daemon_state.set_speaking(true);
            if let Some(params) = &request.params
                && let Some(raw_text) = params.get("text").and_then(|v| v.as_str())
            {
                let clean_text = sanitize(raw_text);
                println!("The sanitized text: {clean_text}");

                if let Err(e) = std::process::Command::new("say").arg(&clean_text).spawn() {
                    eprintln!("Failed to run 'say' command: {e}");
                }
            }

            JsonRpcResponse::success(request.id, serde_json::json!({"ok": true}))
        }
        BANSHEE_GET_TRANSCRIPTION => {
            let transcription_text = TRANSCRIPTION_MAILBOX
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
            JsonRpcResponse::success(
                request.id,
                serde_json::json!({"ok": true, "transcription": transcription_text}),
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

            JsonRpcResponse::success(request.id, serde_json::json!({"ok": true}))
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
                "speaking": daemon_state.is_speaking(),
                "uptime_seconds": &daemon_state.uptime().as_secs(),
                "vad_threshold": daemon_state.vad_threshold(),
            }),
        ),
        _ => JsonRpcResponse::error(
            request.id,
            -32601,
            format!("Method '{}' not found!", request.method),
        ),
    }
}

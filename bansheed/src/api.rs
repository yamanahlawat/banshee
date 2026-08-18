use std::sync::Arc;
use std::time::Duration;

use banshee_common::{
    BANSHEE_ASK_USER, BANSHEE_CLEAR_HISTORY, BANSHEE_CONFIGURE, BANSHEE_GET_TRANSCRIPTION,
    BANSHEE_HISTORY, BANSHEE_READINESS, BANSHEE_RECORD_START, BANSHEE_RECORD_STOP, BANSHEE_SPEAK,
    BANSHEE_STATUS, BANSHEE_STOP, BANSHEE_STOP_SPEAKING,
};
use banshee_common::{JsonRpcRequest, JsonRpcResponse};

use crate::readiness;
use crate::state::{
    AskCommand, ConsumerCommand, DaemonState, RecordingError, RecordingMode, TranscribeTarget,
};
use crate::text_to_speech::sanitizer::sanitize;

const MAX_WAIT_MS: u64 = 30_000;
const DEFAULT_ASK_WAIT_MS: u64 = 30_000;
const MAX_ASK_WAIT_MS: u64 = 120_000;
// Budget scales with question length; the per-word figure is a speech-rate
// estimate, not real audio duration.
const PLAYBACK_BASE_MS: u64 = 15_000;
const PLAYBACK_PER_WORD_MS: u64 = 700;
const MAX_PLAYBACK_WAIT_MS: u64 = 120_000;

fn str_param<'a>(
    params: Option<&'a serde_json::Value>,
    key: &str,
    id: &Option<serde_json::Value>,
) -> Result<&'a str, Box<JsonRpcResponse>> {
    params
        .and_then(|p| p.get(key))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            Box::new(JsonRpcResponse::error(
                id.clone(),
                -32602,
                format!("'{key}' is required and must be a string."),
            ))
        })
}

// absent → default; present but not a u64 → -32602 naming the field
fn u64_param(
    params: Option<&serde_json::Value>,
    key: &str,
    default: u64,
    id: &Option<serde_json::Value>,
) -> Result<u64, Box<JsonRpcResponse>> {
    match params.and_then(|p| p.get(key)) {
        None => Ok(default),
        Some(value) => value.as_u64().ok_or_else(|| {
            Box::new(JsonRpcResponse::error(
                id.clone(),
                -32602,
                format!("'{key}' must be a non-negative integer."),
            ))
        }),
    }
}

// The daemon started without a pipeline. The code says which fix applies, so a
// client can prompt for a microphone or re-run setup instead of retrying.
fn unavailable(id: Option<serde_json::Value>, error: &RecordingError) -> JsonRpcResponse {
    let code = match error {
        RecordingError::Microphone(_) => -32000,
        RecordingError::Model(_) => -32002,
    };
    JsonRpcResponse::error(id, code, format!("Recording is unavailable: {error}"))
}

pub async fn dispatch(request: JsonRpcRequest, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    match request.method.as_str() {
        BANSHEE_SPEAK => {
            let raw_text = match str_param(request.params.as_ref(), "text", &request.id) {
                Ok(value) => value,
                Err(response) => return *response,
            };
            let interrupt = request
                .params
                .as_ref()
                .and_then(|p| p.get("interrupt"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let clean_text = sanitize(raw_text);

            match daemon_state.speech().speak(&clean_text, interrupt) {
                Ok(utterance_id) => JsonRpcResponse::success(
                    request.id,
                    serde_json::json!({"ok": true, "utterance_id": utterance_id}),
                ),
                Err(e) => JsonRpcResponse::error(
                    request.id,
                    -32603,
                    format!("Failed to start speech playback: {e}"),
                ),
            }
        }
        BANSHEE_STOP_SPEAKING => {
            daemon_state.speech().stop();
            JsonRpcResponse::success(request.id, serde_json::json!({"ok": true}))
        }
        BANSHEE_STOP => {
            daemon_state.shutdown().notify_one();
            JsonRpcResponse::success(request.id, serde_json::json!({"ok": true}))
        }
        BANSHEE_RECORD_START => {
            let dictate = request
                .params
                .as_ref()
                .and_then(|p| p.get("dictate"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let action = if dictate {
                TranscribeTarget::Dictate
            } else {
                TranscribeTarget::Mailbox
            };
            // Checked before the transition, so -32004 keeps meaning "busy"
            if let Some(reason) = daemon_state.recording_error() {
                return unavailable(request.id, reason);
            }
            if daemon_state.record_start(action) {
                JsonRpcResponse::success(request.id, serde_json::json!({"ok": true}))
            } else {
                JsonRpcResponse::error(
                    request.id,
                    -32004,
                    "Microphone is busy with another recording or listening session.",
                )
            }
        }
        BANSHEE_RECORD_STOP => {
            daemon_state.record_stop();
            JsonRpcResponse::success(request.id, serde_json::json!({"ok": true}))
        }
        BANSHEE_ASK_USER => {
            let question = match str_param(request.params.as_ref(), "question", &request.id) {
                Ok(value) => value,
                Err(response) => return *response,
            };
            let timeout_ms = match u64_param(
                request.params.as_ref(),
                "timeout_ms",
                DEFAULT_ASK_WAIT_MS,
                &request.id,
            ) {
                Ok(value) => value.min(MAX_ASK_WAIT_MS),
                Err(response) => return *response,
            };

            if let Some(reason) = daemon_state.recording_error() {
                return unavailable(request.id, reason);
            }

            // One armed session at a time; the mode is the lock
            if !daemon_state.arm_for_ask() {
                return JsonRpcResponse::error(
                    request.id,
                    -32004,
                    "Microphone is busy with another recording or listening session.",
                );
            }

            // Interrupt: the question must not queue behind stale status speech
            let clean_question = sanitize(question);
            if let Err(e) = daemon_state.speech().speak(&clean_question, true) {
                daemon_state.set_recording_mode(RecordingMode::Idle);
                return JsonRpcResponse::error(
                    request.id,
                    -32603,
                    format!("Failed to speak question: {e}"),
                );
            }

            // Echo avoidance by ordering: listen only after playback ends.
            // Bounded so a stalled backend cannot hold the mic armed forever
            let words = clean_question.split_whitespace().count() as u64;
            let playback_budget = Duration::from_millis(
                (PLAYBACK_BASE_MS + words * PLAYBACK_PER_WORD_MS).min(MAX_PLAYBACK_WAIT_MS),
            );
            let mut speaking = daemon_state.speech().subscribe_speaking();
            if tokio::time::timeout(playback_budget, speaking.wait_for(|s| !s))
                .await
                .is_err()
            {
                daemon_state.speech().stop();
                daemon_state.set_recording_mode(RecordingMode::Idle);
                return JsonRpcResponse::error(
                    request.id,
                    -32603,
                    "Question playback did not finish.",
                );
            }

            let (reply, answer) = tokio::sync::oneshot::channel();
            let command = ConsumerCommand::Ask(AskCommand {
                reply,
                timeout: Duration::from_millis(timeout_ms),
            });
            if daemon_state.commands().send(command).is_err() {
                daemon_state.set_recording_mode(RecordingMode::Idle);
                return JsonRpcResponse::error(
                    request.id,
                    -32603,
                    "Audio pipeline is not running.",
                );
            }

            match answer.await {
                Ok(text) => {
                    JsonRpcResponse::success(request.id, serde_json::json!({ "text": text }))
                }
                Err(_) => {
                    daemon_state.set_recording_mode(RecordingMode::Idle);
                    JsonRpcResponse::error(
                        request.id,
                        -32603,
                        "Listening session ended unexpectedly.",
                    )
                }
            }
        }
        BANSHEE_GET_TRANSCRIPTION => {
            let mut since_id = match u64_param(request.params.as_ref(), "since_id", 0, &request.id)
            {
                Ok(value) => value,
                Err(response) => return *response,
            };
            let wait_ms = match u64_param(request.params.as_ref(), "wait_ms", 0, &request.id) {
                Ok(value) => value.min(MAX_WAIT_MS),
                Err(response) => return *response,
            };

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
        BANSHEE_READINESS => {
            let blockers = readiness::blockers(daemon_state);
            JsonRpcResponse::success(
                request.id,
                serde_json::json!({ "ready": blockers.is_empty(), "blockers": blockers }),
            )
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
                // null when recording works, so one field cannot contradict another
                "recording_error": daemon_state.recording_error().map(|e| e.to_string()),
                "speaking": daemon_state.speech().is_speaking(),
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
    use crate::config::BargeInMode;
    use crate::text_to_speech::{ActiveUtterance, SpeechPlayer, TtsBackend};

    fn get_transcription_request(params: serde_json::Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: BANSHEE_GET_TRANSCRIPTION.to_string(),
            params: Some(params),
            id: Some(serde_json::json!(1)),
        }
    }

    // Silent backend so tests never spawn a real `say` process
    struct NullBackend;
    struct Done;
    impl ActiveUtterance for Done {
        fn is_finished(&mut self) -> bool {
            true
        }
        fn stop(&mut self) {}
    }
    impl TtsBackend for NullBackend {
        fn start(&self, _text: &str) -> std::io::Result<Box<dyn ActiveUtterance>> {
            Ok(Box::new(Done))
        }
    }

    // Pass a real sender only when the test reads the command receiver
    fn test_state(commands: std::sync::mpsc::Sender<ConsumerCommand>) -> Arc<DaemonState> {
        Arc::new(DaemonState::new(
            "0.0.0",
            "stt",
            "vad",
            0.5,
            None,
            SpeechPlayer::new(Box::new(NullBackend)),
            commands,
            std::sync::mpsc::channel().0,
            BargeInMode::Stop,
        ))
    }

    fn ask_user_request(params: serde_json::Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: BANSHEE_ASK_USER.to_string(),
            params: Some(params),
            id: Some(serde_json::json!(1)),
        }
    }

    #[tokio::test]
    async fn ask_user_returns_the_scoped_answer() {
        let (commands, command_receiver) = std::sync::mpsc::channel();
        let state = test_state(commands);

        // Stand-in for the consumer thread: answer and disarm like a session
        let session_state = Arc::clone(&state);
        std::thread::spawn(move || {
            let Ok(ConsumerCommand::Ask(ask)) = command_receiver.recv() else {
                return;
            };
            assert_eq!(session_state.recording_mode(), RecordingMode::Armed);
            session_state.set_recording_mode(RecordingMode::Idle);
            let _ = ask.reply.send("yes, ship it".to_string());
        });

        let request = ask_user_request(serde_json::json!({"question": "Ready to ship?"}));
        let response = dispatch(request, &state).await;

        let JsonRpcResponse::Success { result, .. } = response else {
            panic!("expected success response");
        };
        assert_eq!(result["text"], "yes, ship it");
        assert_eq!(state.recording_mode(), RecordingMode::Idle);
    }

    #[tokio::test]
    async fn concurrent_ask_user_is_refused_while_armed() {
        let state = test_state(std::sync::mpsc::channel().0);
        state.set_recording_mode(RecordingMode::Armed);

        let request = ask_user_request(serde_json::json!({"question": "Also ready?"}));
        let response = dispatch(request, &state).await;

        let JsonRpcResponse::Error { error, .. } = response else {
            panic!("expected error response");
        };
        assert_eq!(error.code, -32004);
        // The refused call must not disturb the session that owns the mic
        assert_eq!(state.recording_mode(), RecordingMode::Armed);
    }

    #[tokio::test]
    async fn stop_replies_ok_and_signals_shutdown() {
        let state = test_state(std::sync::mpsc::channel().0);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: BANSHEE_STOP.to_string(),
            params: None,
            id: Some(serde_json::json!(1)),
        };
        let response = dispatch(request, &state).await;

        let JsonRpcResponse::Success { result, .. } = response else {
            panic!("expected success response");
        };
        assert_eq!(result["ok"], true);
        // notify_one stores a permit, so a later notified() must resolve
        tokio::time::timeout(Duration::from_millis(100), state.shutdown().notified())
            .await
            .expect("shutdown was not signaled");
    }

    fn record_request(method: &str, params: Option<serde_json::Value>) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Some(serde_json::json!(1)),
        }
    }

    // "No microphone" and "busy" send the caller to different fixes, so the
    // codes must not collapse into one another.
    #[tokio::test]
    async fn recording_rpcs_report_the_cause_not_a_busy_mic() {
        for (cause, expected) in [
            (RecordingError::Microphone("no device".to_string()), -32000),
            (RecordingError::Model("missing file".to_string()), -32002),
        ] {
            let state = test_state(std::sync::mpsc::channel().0);
            state.set_recording_error(cause);

            let start = record_request(BANSHEE_RECORD_START, None);
            let JsonRpcResponse::Error { error, .. } = dispatch(start, &state).await else {
                panic!("expected error response");
            };
            assert_eq!(error.code, expected);
            assert_eq!(state.recording_mode(), RecordingMode::Idle);

            let ask = ask_user_request(serde_json::json!({"question": "ready?"}));
            let JsonRpcResponse::Error { error, .. } = dispatch(ask, &state).await else {
                panic!("expected error response");
            };
            assert_eq!(error.code, expected);
            // The armed lock must not be taken by a refused question
            assert_eq!(state.recording_mode(), RecordingMode::Idle);
        }
    }

    #[tokio::test]
    async fn record_start_and_stop_drive_push_to_talk() {
        let (commands, command_receiver) = std::sync::mpsc::channel();
        let state = test_state(commands);

        let start = record_request(
            BANSHEE_RECORD_START,
            Some(serde_json::json!({"dictate": true})),
        );
        let JsonRpcResponse::Success { .. } = dispatch(start, &state).await else {
            panic!("expected success response");
        };
        assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);

        // A second start must be refused while recording
        let again = record_request(BANSHEE_RECORD_START, None);
        let JsonRpcResponse::Error { error, .. } = dispatch(again, &state).await else {
            panic!("expected error response");
        };
        assert_eq!(error.code, -32004);
        assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);

        let stop = record_request(BANSHEE_RECORD_STOP, None);
        let JsonRpcResponse::Success { .. } = dispatch(stop, &state).await else {
            panic!("expected success response");
        };
        assert_eq!(state.recording_mode(), RecordingMode::Idle);
        // The dictate choice from start must reach the consumer command
        let Ok(ConsumerCommand::Transcribe(TranscribeTarget::Dictate)) =
            command_receiver.try_recv()
        else {
            panic!("expected a dictate transcribe command");
        };
    }

    #[tokio::test]
    async fn record_stop_while_idle_is_a_no_op() {
        let (commands, command_receiver) = std::sync::mpsc::channel();
        let state = test_state(commands);

        let stop = record_request(BANSHEE_RECORD_STOP, None);
        let JsonRpcResponse::Success { result, .. } = dispatch(stop, &state).await else {
            panic!("expected success response");
        };
        assert_eq!(result["ok"], true);
        assert_eq!(state.recording_mode(), RecordingMode::Idle);
        assert!(command_receiver.try_recv().is_err(), "no command expected");
    }

    #[tokio::test]
    async fn stale_cursor_is_clamped_to_zero() {
        let state = test_state(std::sync::mpsc::channel().0);
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
        let state = test_state(std::sync::mpsc::channel().0);
        state.push_transcription("hello".to_string());

        let request = get_transcription_request(serde_json::json!({"since_id": 1}));
        let response = dispatch(request, &state).await;

        let JsonRpcResponse::Success { result, .. } = response else {
            panic!("expected success response");
        };
        assert!(result["transcriptions"].as_array().unwrap().is_empty());
    }
}

use std::sync::Arc;
use std::time::Duration;

use banshee_common::{
    BANSHEE_ASK_USER, BANSHEE_CLEAR_HISTORY, BANSHEE_CONFIGURE, BANSHEE_DOWNLOAD_MODELS,
    BANSHEE_GET_TRANSCRIPTION, BANSHEE_HISTORY, BANSHEE_LIST_INPUT_DEVICES, BANSHEE_LIST_VOICES,
    BANSHEE_RECORD_START, BANSHEE_RECORD_STOP, BANSHEE_SPEAK, BANSHEE_STATUS, BANSHEE_STOP,
    BANSHEE_STOP_SPEAKING, BANSHEE_SUBSCRIBE,
};
use banshee_common::{JsonRpcRequest, JsonRpcResponse};

use crate::state::{
    AskCommand, ConsumerCommand, DaemonState, RecordingError, RecordingMode, TranscribeTarget,
};
use crate::text_to_speech::sanitizer::sanitize;
use crate::{readiness, settings};

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

/// Everything `banshee.status` reports.
pub fn status_payload(daemon_state: &DaemonState) -> serde_json::Value {
    let blockers = readiness::blockers(daemon_state);
    serde_json::json!({
        "running": true,
        "version": daemon_state.version(),
        "stt_model": daemon_state.stt_model(),
        "vad_model": daemon_state.vad_model(),
        "audio_device": daemon_state.audio_device(),
        "missing_device": daemon_state.missing_device(),
        "recording": daemon_state.is_recording(),
        "speaking": daemon_state.speech().is_speaking(),
        "uptime_seconds": daemon_state.uptime().as_secs(),
        "vad_threshold": daemon_state.vad_threshold(),
        "history_enabled": daemon_state.db_connection().is_some(),
        // Stated, so no client invents a narrower definition of ready
        "ready": blockers.is_empty(),
        "blockers": blockers,
    })
}

/// The `banshee.state_changed` params: what moves without a client touching it.
/// The two device fields move on their own, because the watchdog rebinds while
/// the daemon idles. `vad_threshold` moves at runtime too, but only when a
/// `configure` call asks it to, and that call already answers.
pub fn live_state(daemon_state: &DaemonState) -> serde_json::Value {
    serde_json::json!({
        "recording": daemon_state.is_recording(),
        "speaking": daemon_state.speech().is_speaking(),
        "audio_device": daemon_state.audio_device(),
        "missing_device": daemon_state.missing_device(),
    })
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
                return unavailable(request.id, &reason);
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
                return unavailable(request.id, &reason);
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
            let Some(requested) = request.params.as_ref().and_then(|p| p.get("settings")) else {
                return JsonRpcResponse::error(
                    request.id,
                    -32602,
                    "'settings' is required, as in {\"stt.language\": \"de\"}.",
                );
            };
            let assignments: settings::Assignments = match serde_json::from_value(requested.clone())
            {
                Ok(assignments) => assignments,
                Err(error) => {
                    return JsonRpcResponse::error(
                        request.id,
                        -32602,
                        format!("'settings' must map dotted keys to values: {error}"),
                    );
                }
            };
            let persist = request
                .params
                .as_ref()
                .and_then(|p| p.get("persist"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            match settings::configure(Some(daemon_state), &assignments, persist) {
                Ok(outcome) => JsonRpcResponse::success(
                    request.id,
                    serde_json::json!({
                        "ok": true,
                        "applied": outcome.applied,
                        "restart_required": outcome.restart_required,
                    }),
                ),
                Err(error) => {
                    JsonRpcResponse::error(request.id, error.rpc_code(), error.rpc_message())
                }
            }
        }
        BANSHEE_DOWNLOAD_MODELS => {
            let dir = match crate::models::download::models_dir() {
                Ok(dir) => dir,
                Err(error) => {
                    return JsonRpcResponse::error(request.id, -32603, error.to_string());
                }
            };
            let missing =
                crate::models::download::still_missing(daemon_state.wanted_downloads(), &dir);
            if missing.is_empty() {
                return JsonRpcResponse::success(
                    request.id,
                    serde_json::json!({"ok": true, "downloading": []}),
                );
            }
            let Some(slot) = daemon_state.start_downloading() else {
                return JsonRpcResponse::error(
                    request.id,
                    -32005,
                    "A download is already running.",
                );
            };

            let names: Vec<&str> = missing.iter().map(|d| d.name.as_str()).collect();
            let response = JsonRpcResponse::success(
                request.id,
                serde_json::json!({"ok": true, "downloading": names}),
            );

            let state = Arc::clone(daemon_state);
            let dir = dir.clone();
            let slot = slot;
            tokio::spawn(async move {
                let mut report = |progress| state.report_download(progress);
                if let Err(error) =
                    crate::models::download::download_all(&dir, &missing, &mut report).await
                {
                    eprintln!("Download failed: {error}");
                }
                drop(slot);
            });
            response
        }
        BANSHEE_LIST_VOICES => JsonRpcResponse::success(
            request.id,
            serde_json::json!({
                "voices": crate::models::installed_voices(),
                "current": daemon_state.tts_voice(),
            }),
        ),
        BANSHEE_LIST_INPUT_DEVICES => JsonRpcResponse::success(
            request.id,
            serde_json::json!({
                "devices": crate::audio::input_devices(),
                "current": daemon_state.audio_device(),
            }),
        ),
        // Subscribing answers with the poll, so a client needs no first poll and
        // cannot miss a change in the gap before the pushes start. `daemon.rs`
        // owns the pushing itself, because only it holds the connection.
        BANSHEE_STATUS | BANSHEE_SUBSCRIBE => {
            JsonRpcResponse::success(request.id, status_payload(daemon_state))
        }
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
    use crate::test_support::daemon_state as test_state;

    fn get_transcription_request(params: serde_json::Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: BANSHEE_GET_TRANSCRIPTION.to_string(),
            params: Some(params),
            id: Some(serde_json::json!(1)),
        }
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

    fn request(method: &str, params: Option<serde_json::Value>) -> JsonRpcRequest {
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

            let start = request(BANSHEE_RECORD_START, None);
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

        let start = request(
            BANSHEE_RECORD_START,
            Some(serde_json::json!({"dictate": true})),
        );
        let JsonRpcResponse::Success { .. } = dispatch(start, &state).await else {
            panic!("expected success response");
        };
        assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);

        // A second start must be refused while recording
        let again = request(BANSHEE_RECORD_START, None);
        let JsonRpcResponse::Error { error, .. } = dispatch(again, &state).await else {
            panic!("expected error response");
        };
        assert_eq!(error.code, -32004);
        assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);

        let stop = request(BANSHEE_RECORD_STOP, None);
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

        let stop = request(BANSHEE_RECORD_STOP, None);
        let JsonRpcResponse::Success { result, .. } = dispatch(stop, &state).await else {
            panic!("expected success response");
        };
        assert_eq!(result["ok"], true);
        assert_eq!(state.recording_mode(), RecordingMode::Idle);
        assert!(command_receiver.try_recv().is_err(), "no command expected");
    }

    // One payload, so a subscriber's first read and a poller's read cannot
    // disagree about what the daemon reports.
    #[tokio::test]
    async fn subscribe_answers_with_the_status_payload() {
        let state = test_state(std::sync::mpsc::channel().0);
        state.set_recording_mode(RecordingMode::PushToTalk);

        let JsonRpcResponse::Success { result: polled, .. } =
            dispatch(request(BANSHEE_STATUS, None), &state).await
        else {
            panic!("expected success response");
        };
        let JsonRpcResponse::Success {
            result: subscribed, ..
        } = dispatch(request(BANSHEE_SUBSCRIBE, None), &state).await
        else {
            panic!("expected success response");
        };

        assert_eq!(polled["recording"], true, "the fixture must discriminate");
        // Every key but the clock, which advances between the two calls
        assert!(polled["uptime_seconds"].is_number());
        for (key, value) in polled.as_object().expect("an object") {
            if key != "uptime_seconds" {
                assert_eq!(value, &subscribed[key], "'{key}' differs");
            }
        }
        assert_eq!(
            polled.as_object().map(serde_json::Map::len),
            subscribed.as_object().map(serde_json::Map::len),
            "one payload carries a key the other does not"
        );
    }

    // Two spellings of one fact drift apart; this is what notices.
    #[test]
    fn a_pushed_change_agrees_with_what_status_reports() {
        let state = test_state(std::sync::mpsc::channel().0);
        state.set_recording_mode(RecordingMode::PushToTalk);
        // Both device fields carry a value, or `null == null` passes for them
        state.set_audio_device(Some("MacBook Pro Microphone".to_string()));
        state.set_missing_device(Some("Yeti Nano".to_string()));

        let status = status_payload(&state);
        let live = live_state(&state);

        assert_eq!(live["recording"], true, "the fixture must discriminate");
        assert_eq!(live["audio_device"], "MacBook Pro Microphone");
        assert_eq!(live["missing_device"], "Yeti Nano");
        for key in live.as_object().expect("live_state is an object").keys() {
            assert_eq!(live[key], status[key], "'{key}' disagrees with status");
        }
    }

    #[test]
    fn the_pushed_state_carries_the_device_and_what_it_waits_for() {
        let state = test_state(std::sync::mpsc::channel().0);
        state.set_audio_device(Some("OnePlus Buds 3".to_string()));
        let bound = live_state(&state);
        assert_eq!(bound["audio_device"], "OnePlus Buds 3");
        assert!(bound["missing_device"].is_null());

        // A substitution must reach a subscriber, or the tray shows the dead device
        state.set_audio_device(Some("MacBook Pro Microphone".to_string()));
        state.set_missing_device(Some("OnePlus Buds 3".to_string()));
        let substituted = live_state(&state);
        assert_eq!(substituted["audio_device"], "MacBook Pro Microphone");
        assert_eq!(substituted["missing_device"], "OnePlus Buds 3");
        assert_ne!(
            bound, substituted,
            "push_changes suppresses an unchanged state"
        );
    }

    #[tokio::test]
    async fn a_fallback_backend_reports_no_current_voice() {
        let state = test_state(std::sync::mpsc::channel().0);

        let JsonRpcResponse::Success { result, .. } =
            dispatch(request(BANSHEE_LIST_VOICES, None), &state).await
        else {
            panic!("expected success response");
        };
        assert!(result["current"].is_null());
        assert!(result["voices"].is_array(), "{result}");

        state.set_tts_voice("af_sky".to_string());
        let JsonRpcResponse::Success { result, .. } =
            dispatch(request(BANSHEE_LIST_VOICES, None), &state).await
        else {
            panic!("expected success response");
        };
        assert_eq!(result["current"], "af_sky");
    }

    #[tokio::test]
    async fn a_second_download_is_refused_while_one_runs() {
        let state = test_state(std::sync::mpsc::channel().0);
        // Something to fetch, or the call answers "nothing to do" and never
        // reaches the slot at all
        state.set_wanted_downloads(vec![crate::models::download::Download {
            name: "no-such-model-9f3a.bin".to_string(),
            url: "https://example.invalid/no-such-model-9f3a.bin".to_string(),
        }]);
        let slot = state.start_downloading().expect("the slot starts free");

        let JsonRpcResponse::Error { error, .. } =
            dispatch(request(BANSHEE_DOWNLOAD_MODELS, None), &state).await
        else {
            panic!("expected the busy error");
        };
        assert_eq!(error.code, -32005);

        drop(slot);
        assert!(state.start_downloading().is_some(), "the slot came back");
    }

    #[tokio::test]
    async fn downloading_nothing_reports_an_empty_list() {
        let state = test_state(std::sync::mpsc::channel().0);
        state.set_wanted_downloads(Vec::new());

        let JsonRpcResponse::Success { result, .. } =
            dispatch(request(BANSHEE_DOWNLOAD_MODELS, None), &state).await
        else {
            panic!("expected success response");
        };
        assert_eq!(result["ok"], true);
        assert_eq!(result["downloading"].as_array().unwrap().len(), 0);
        assert!(
            state.start_downloading().is_some(),
            "a call that fetched nothing must not hold the slot"
        );
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

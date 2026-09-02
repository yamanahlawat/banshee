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

fn connect_plan_request(params: serde_json::Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: BANSHEE_CONNECT_PLAN.to_string(),
        params: Some(params),
        id: Some(serde_json::json!(1)),
    }
}

fn connect_apply_request(params: serde_json::Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: BANSHEE_CONNECT_APPLY.to_string(),
        params: Some(params),
        id: Some(serde_json::json!(1)),
    }
}

fn history_request(params: serde_json::Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: BANSHEE_HISTORY.to_string(),
        params: Some(params),
        id: Some(serde_json::json!(1)),
    }
}

#[tokio::test]
async fn history_honours_a_limit() {
    let state = crate::test_support::daemon_state_with_history(&["first", "second", "third"]);

    let request = history_request(serde_json::json!({ "limit": 1 }));
    let response = dispatch(request, &state).await;

    let JsonRpcResponse::Success { result, .. } = response else {
        panic!("expected success response");
    };
    let history = result["history"].as_array().expect("a history array");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["text"], "third");
}

#[tokio::test]
async fn a_disconnect_that_is_not_a_boolean_is_refused() {
    let state = test_state(std::sync::mpsc::channel().0);

    for request in [
        connect_plan_request(serde_json::json!({"agent": "cursor", "disconnect": "true"})),
        connect_apply_request(serde_json::json!({"agent": "cursor", "disconnect": "true"})),
    ] {
        let JsonRpcResponse::Error { error, .. } = dispatch(request, &state).await else {
            panic!("a string disconnect must not reach the plan");
        };
        assert_eq!(error.code, -32602);
        assert!(error.message.contains("disconnect"), "{}", error.message);
    }
}

/// `-32003` names one cause: history is off. NOT COVERED: the database
/// failure beside it, which needs a database that refuses a clear, and the
/// harness cannot build one.
#[tokio::test]
async fn only_history_being_off_answers_its_own_code() {
    let state = test_state(std::sync::mpsc::channel().0);

    for method in [BANSHEE_HISTORY, BANSHEE_CLEAR_HISTORY] {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params: Some(serde_json::json!({})),
            id: Some(serde_json::json!(1)),
        };
        let JsonRpcResponse::Error { error, .. } = dispatch(request, &state).await else {
            panic!("{method} must refuse when nothing is kept");
        };
        assert_eq!(error.code, -32003, "{method}");
        assert!(error.message.contains("not enabled"), "{}", error.message);
    }
}

#[tokio::test]
async fn a_history_limit_that_does_not_fit_is_refused() {
    let state = crate::test_support::daemon_state_with_history(&["first"]);

    let request = history_request(serde_json::json!({ "limit": 4_294_967_296u64 }));
    let JsonRpcResponse::Error { error, .. } = dispatch(request, &state).await else {
        panic!("a limit past 32 bits must not truncate");
    };
    assert_eq!(error.code, -32602);
}

#[tokio::test]
async fn history_takes_an_explicit_zero_literally() {
    let state = crate::test_support::daemon_state_with_history(&["first", "second", "third"]);

    let request = history_request(serde_json::json!({ "limit": 0 }));
    let response = dispatch(request, &state).await;

    let JsonRpcResponse::Success { result, .. } = response else {
        panic!("expected success response");
    };
    let history = result["history"].as_array().expect("a history array");
    assert!(
        history.is_empty(),
        "an explicit zero must not mean every row"
    );
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
    let Ok(ConsumerCommand::Transcribe(TranscribeTarget::Dictate)) = command_receiver.try_recv()
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
fn status_carries_the_config_the_daemon_parsed() {
    let state = test_state(std::sync::mpsc::channel().0);
    let mut config = crate::config::Config::default();
    config.stt.language = "de".to_string();
    config.stt.vocabulary = vec!["Tauri".to_string(), "Svelte".to_string()];
    config.tts.voice = "af_sky".to_string();
    state.set_config(std::sync::Arc::new(config));

    let status = status_payload(&state);
    assert_eq!(status["config"]["stt"]["language"], "de");
    assert_eq!(
        status["config"]["stt"]["vocabulary"],
        serde_json::json!(["Tauri", "Svelte"])
    );
    assert_eq!(status["config"]["tts"]["voice"], "af_sky");
    assert!(status["config"]["audio"]["hotkey"].is_string());
}

#[test]
fn status_reports_nothing_pending_on_a_fresh_daemon() {
    let state = test_state(std::sync::mpsc::channel().0);
    assert_eq!(status_payload(&state)["pending"], serde_json::json!([]));
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

#[test]
fn live_state_reports_armed_while_a_question_waits() {
    let state = test_state(std::sync::mpsc::channel().0);
    assert_eq!(live_state(&state)["armed"], serde_json::json!(false));

    assert!(state.arm_for_ask(), "the fixture must discriminate");
    let live = live_state(&state);
    assert_eq!(live["armed"], serde_json::json!(true));
    // The microphone is open while armed
    assert_eq!(live["recording"], serde_json::json!(true));

    state.set_recording_mode(RecordingMode::Idle);
    assert_eq!(live_state(&state)["armed"], serde_json::json!(false));
}

#[test]
fn push_to_talk_records_without_arming() {
    let state = test_state(std::sync::mpsc::channel().0);
    state.set_recording_mode(RecordingMode::PushToTalk);
    let live = live_state(&state);
    assert_eq!(live["recording"], serde_json::json!(true));
    assert_eq!(live["armed"], serde_json::json!(false));
}

#[test]
fn live_state_reports_transcribing() {
    let state = test_state(std::sync::mpsc::channel().0);
    assert_eq!(live_state(&state)["transcribing"], serde_json::json!(false));
    state.set_transcribing(true);
    assert_eq!(live_state(&state)["transcribing"], serde_json::json!(true));
    state.set_transcribing(false);
    assert_eq!(live_state(&state)["transcribing"], serde_json::json!(false));
}

#[test]
fn setting_transcribing_wakes_a_subscriber() {
    let state = test_state(std::sync::mpsc::channel().0);
    let mut changes = state.subscribe_transcribing();
    state.set_transcribing(true);
    assert!(changes.has_changed().expect("the sender outlives this"));
    assert!(*changes.borrow_and_update());
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
        megabytes: 1,
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

#[tokio::test]
async fn an_unknown_agent_slug_is_refused_by_plan_and_apply() {
    let state = test_state(std::sync::mpsc::channel().0);

    let JsonRpcResponse::Error {
        error: plan_error, ..
    } = dispatch(
        connect_plan_request(serde_json::json!({"agent": "notatool", "disconnect": false})),
        &state,
    )
    .await
    else {
        panic!("expected an error response");
    };
    assert!(
        plan_error.message.contains("notatool"),
        "{}",
        plan_error.message
    );

    let JsonRpcResponse::Error {
        error: apply_error, ..
    } = dispatch(
        connect_apply_request(serde_json::json!({"agent": "notatool", "disconnect": false})),
        &state,
    )
    .await
    else {
        panic!("expected an error response");
    };
    assert!(
        apply_error.message.contains("notatool"),
        "{}",
        apply_error.message
    );
}

#[tokio::test]
async fn disconnect_true_is_refused_before_the_agent_is_resolved() {
    let state = test_state(std::sync::mpsc::channel().0);
    let refusal =
        "Disconnect is not available yet. Remove Banshee from the agent's config by hand.";

    for request in [
        connect_plan_request(serde_json::json!({"agent": "cursor", "disconnect": true})),
        connect_apply_request(serde_json::json!({"agent": "cursor", "disconnect": true})),
        connect_plan_request(serde_json::json!({"agent": "notatool", "disconnect": true})),
        connect_apply_request(serde_json::json!({"agent": "notatool", "disconnect": true})),
    ] {
        let response = dispatch(request, &state).await;
        let JsonRpcResponse::Error { error, .. } = response else {
            panic!("expected an error response");
        };
        assert_eq!(error.message, refusal);
    }
}

struct VoiceCapture(Arc<std::sync::Mutex<Vec<Option<String>>>>);

struct RecordedUtterance;

impl crate::text_to_speech::ActiveUtterance for RecordedUtterance {
    fn is_finished(&mut self) -> bool {
        true
    }
    fn stop(&mut self) {}
}

impl crate::text_to_speech::TtsBackend for VoiceCapture {
    fn start(
        &self,
        _text: &str,
        voice: Option<&str>,
    ) -> std::io::Result<Box<dyn crate::text_to_speech::ActiveUtterance>> {
        self.0.lock().unwrap().push(voice.map(str::to_string));
        Ok(Box::new(RecordedUtterance))
    }
}

#[tokio::test]
async fn speak_passes_the_voice_parameter_to_the_backend() {
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let speech =
        crate::text_to_speech::SpeechPlayer::new(Box::new(VoiceCapture(Arc::clone(&captured))));
    let state = Arc::new(DaemonState::new(
        "0.0.0",
        "stt",
        "vad",
        0.5,
        "default".to_string(),
        None,
        speech,
        std::sync::mpsc::channel().0,
        crate::audio::cues::Cues::silent(),
        crate::config::BargeInMode::Stop,
    ));

    let response = dispatch(
        request(
            BANSHEE_SPEAK,
            Some(serde_json::json!({"text": "hello", "voice": "am_adam"})),
        ),
        &state,
    )
    .await;

    assert!(matches!(response, JsonRpcResponse::Success { .. }));
    assert_eq!(*captured.lock().unwrap(), vec![Some("am_adam".to_string())]);
}

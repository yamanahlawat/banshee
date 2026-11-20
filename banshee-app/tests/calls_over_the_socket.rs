mod common;

use banshee_app::calls;
use banshee_app::socket::{Client, REPLY_DEADLINE};
use banshee_common::{
    BANSHEE_AGENTS, BANSHEE_CLEAR_HISTORY, BANSHEE_CONFIGURE, BANSHEE_CONNECT_APPLY,
    BANSHEE_CONNECT_PLAN, BANSHEE_DOWNLOAD_MODELS, BANSHEE_HISTORY, BANSHEE_HISTORY_AUDIO,
    BANSHEE_LIST_INPUT_DEVICES, BANSHEE_LIST_VOICES, BANSHEE_OPEN_PERMISSION, BANSHEE_RETRY,
    BANSHEE_SPEAK, BANSHEE_STATUS,
};
use common::{recording_daemon, recording_error_daemon};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;

#[tokio::test]
async fn status_returns_the_reply_untouched() {
    let (path, mut seen, _guard) =
        recording_daemon(serde_json::json!({"running": true, "recording": false})).await;
    let mut client = Client::connect(&path).await.unwrap();

    let reply = calls::status(&mut client).await.unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_STATUS);
    assert_eq!(
        reply,
        serde_json::json!({"running": true, "recording": false})
    );
}

#[tokio::test]
async fn a_setting_is_written_with_persist_true() {
    let (path, mut seen, _guard) =
        recording_daemon(serde_json::json!({"waiting": ["feedback.mode"]})).await;
    let mut client = Client::connect(&path).await.unwrap();

    let waiting = calls::set_setting(&mut client, "feedback.mode", serde_json::json!("none"))
        .await
        .unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_CONFIGURE);
    assert_eq!(
        request.params.unwrap(),
        serde_json::json!({
            "settings": {"feedback.mode": "none"},
            "persist": true,
        })
    );
    assert_eq!(waiting, vec!["feedback.mode".to_string()]);
}

#[tokio::test]
async fn a_settings_reply_with_the_wrong_shape_is_an_error_not_an_empty_list() {
    // "waiting" as a string, not an array: a wrong-shaped reply must not
    // read as "nothing waits".
    let (path, _seen, _guard) =
        recording_daemon(serde_json::json!({"waiting": "feedback.mode"})).await;
    let mut client = Client::connect(&path).await.unwrap();

    let error = calls::set_setting(&mut client, "feedback.mode", serde_json::json!("none"))
        .await
        .unwrap_err();

    assert_eq!(error.code, -32700);
}

#[tokio::test]
async fn list_devices_reads_both_fields() {
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({
        "devices": [{"name": "Blue Yeti", "default": true}],
        "current": "Blue Yeti",
    }))
    .await;
    let mut client = Client::connect(&path).await.unwrap();

    let devices = calls::list_devices(&mut client).await.unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_LIST_INPUT_DEVICES);
    assert_eq!(devices.devices.len(), 1);
    assert_eq!(devices.devices[0].name, "Blue Yeti");
    assert!(devices.devices[0].default);
    assert_eq!(devices.current.as_deref(), Some("Blue Yeti"));
}

#[tokio::test]
async fn list_voices_reads_both_fields() {
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({
        "voices": [{"id": "am_adam", "name": "Adam", "description": "US male", "downloaded": true}],
        "current": "am_adam",
    }))
    .await;
    let mut client = Client::connect(&path).await.unwrap();

    let voices = calls::list_voices(&mut client).await.unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_LIST_VOICES);
    assert_eq!(voices.voices.len(), 1);
    assert_eq!(voices.voices[0].id, "am_adam");
    assert_eq!(voices.current.as_deref(), Some("am_adam"));
}

#[tokio::test]
#[allow(clippy::len_zero)]
async fn a_voice_preview_speaks_one_sentence_in_that_voice() {
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({"ok": true})).await;
    let mut client = Client::connect(&path).await.unwrap();

    calls::preview_voice(&mut client, "am_adam").await.unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_SPEAK);
    let params = request.params.unwrap();
    assert_eq!(params["voice"], "am_adam");
    assert!(params["text"].as_str().unwrap().len() > 0);
    // A preview must not change the configured voice
    assert!(params.get("persist").is_none());
}

#[tokio::test]
async fn download_models_sends_no_params_and_returns_nothing() {
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({"ok": true})).await;
    let mut client = Client::connect(&path).await.unwrap();

    calls::download_models(&mut client).await.unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_DOWNLOAD_MODELS);
    assert_eq!(
        request.params,
        Some(serde_json::json!({})),
        "the daemon takes no argument, and a stray one would be refused"
    );
}

#[tokio::test]
async fn the_agents_wrapper_key_never_reaches_a_caller() {
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({
        "agents": [{"id": "cursor", "name": "Cursor", "presence": "found", "note": "Ready to connect."}]
    }))
    .await;
    let mut client = Client::connect(&path).await.unwrap();

    let agents = calls::detect_agents(&mut client).await.unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_AGENTS);
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].id, "cursor");
}

#[tokio::test]
async fn the_changes_wrapper_key_never_reaches_a_caller() {
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({
        "changes": [{"path": "/Users/x/.cursor/mcp.json", "diff": "+ banshee"}]
    }))
    .await;
    let mut client = Client::connect(&path).await.unwrap();

    let changes = calls::plan_connect(&mut client, "cursor", false)
        .await
        .unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_CONNECT_PLAN);
    assert_eq!(
        request.params.unwrap(),
        serde_json::json!({"agent": "cursor", "disconnect": false})
    );
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].diff, "+ banshee");
}

#[tokio::test]
async fn a_daemon_error_reaches_the_caller_as_its_own_code_and_sentence() {
    let (path, _seen, _guard) =
        recording_error_daemon(-32602, "Disconnect is not available yet.").await;
    let mut client = Client::connect(&path).await.unwrap();
    let refused = calls::plan_connect(&mut client, "cursor", true)
        .await
        .unwrap_err();
    assert_eq!(refused.code, -32602);
    assert_eq!(refused.message, "Disconnect is not available yet.");
}

#[tokio::test]
async fn apply_connect_sends_the_same_params_plan_connect_does() {
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({"ok": true})).await;
    let mut client = Client::connect(&path).await.unwrap();

    calls::apply_connect(&mut client, "cursor", false)
        .await
        .unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_CONNECT_APPLY);
    assert_eq!(
        request.params.unwrap(),
        serde_json::json!({"agent": "cursor", "disconnect": false})
    );
}

#[tokio::test]
async fn the_history_wrapper_key_never_reaches_a_caller() {
    let (path, _seen, _guard) = recording_daemon(serde_json::json!({
        "history": [{"id": 1, "text": "Yes.", "timestamp": "2026-08-26T13:47:00Z"}]
    }))
    .await;
    let mut client = Client::connect(&path).await.unwrap();
    let rows = calls::history(&mut client, None).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["text"], "Yes.");
}

#[tokio::test]
async fn a_history_call_without_a_limit_omits_the_key() {
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({"history": []})).await;
    let mut client = Client::connect(&path).await.unwrap();

    calls::history(&mut client, None).await.unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_HISTORY);
    assert_eq!(request.params.unwrap(), serde_json::json!({}));
}

#[tokio::test]
async fn a_history_call_with_a_limit_forwards_it() {
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({"history": []})).await;
    let mut client = Client::connect(&path).await.unwrap();

    calls::history(&mut client, Some(20)).await.unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.params.unwrap(), serde_json::json!({"limit": 20}));
}

#[tokio::test]
async fn clear_history_sends_no_params_and_returns_nothing() {
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({})).await;
    let mut client = Client::connect(&path).await.unwrap();

    calls::clear_history(&mut client).await.unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_CLEAR_HISTORY);
    assert_eq!(request.params, Some(serde_json::json!({})));
}

#[tokio::test]
async fn history_audio_sends_the_id_and_reads_the_path() {
    let (path, mut seen, _guard) =
        recording_daemon(serde_json::json!({"path": "/tmp/audio/7.wav"})).await;
    let mut client = Client::connect(&path).await.unwrap();

    let audio = calls::history_audio(&mut client, 7).await.unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_HISTORY_AUDIO);
    assert_eq!(request.params.unwrap(), serde_json::json!({"id": 7}));
    assert_eq!(audio, std::path::PathBuf::from("/tmp/audio/7.wav"));
}

#[tokio::test]
async fn retry_sends_the_id_and_answers_the_row() {
    let (path, mut seen, _guard) =
        recording_daemon(serde_json::json!({"row": {"id": 7, "text": "Again."}})).await;
    let mut client = Client::connect(&path).await.unwrap();

    let row = calls::retry(&mut client, 7).await.unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_RETRY);
    assert_eq!(request.params.unwrap(), serde_json::json!({"id": 7}));
    assert_eq!(row, serde_json::json!({"id": 7, "text": "Again."}));
}

#[tokio::test]
async fn a_retry_reply_without_a_row_object_is_an_error_not_null() {
    let (path, _seen, _guard) = recording_daemon(serde_json::json!({"id": 7})).await;
    let mut client = Client::connect(&path).await.unwrap();

    let error = calls::retry(&mut client, 7).await.unwrap_err();

    assert_eq!(error.code, -32700);
}

/// A retry queues behind a dictation or an open question, so its reply can come
/// long after the deadline every other call keeps. The paused clock passes that
/// deadline at once while the daemon below still holds the request.
#[tokio::test(start_paused = true)]
async fn a_retry_answered_after_the_reply_deadline_still_answers_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("banshee.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let daemon = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let request: banshee_common::JsonRpcRequest = serde_json::from_str(&line).unwrap();
        released.recv().unwrap();
        let reply = banshee_common::JsonRpcResponse::success(
            request.id,
            serde_json::json!({"row": {"id": 7, "text": "Again."}}),
        );
        writeln!(writer, "{}", serde_json::to_string(&reply).unwrap()).unwrap();
    });
    let mut client = Client::connect(&path).await.unwrap();

    let later = async {
        tokio::time::sleep(REPLY_DEADLINE * 2).await;
        release.send(()).unwrap();
    };
    let (row, ()) = tokio::join!(calls::retry(&mut client, 7), later);

    assert_eq!(row.unwrap(), serde_json::json!({"id": 7, "text": "Again."}));
    daemon.join().unwrap();
}

#[tokio::test]
async fn open_permission_pane_sends_the_id_key() {
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({"ok": true})).await;
    let mut client = Client::connect(&path).await.unwrap();

    calls::open_permission_pane(&mut client, "accessibility")
        .await
        .unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_OPEN_PERMISSION);
    assert_eq!(
        request.params.unwrap(),
        serde_json::json!({"id": "accessibility"})
    );
}

/// A reply left in the buffer by an abandoned call must not be handed to the
/// next caller as its own answer.
#[tokio::test]
async fn a_stale_reply_is_read_past_not_returned() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("banshee.sock");
    let listener = UnixListener::bind(&path).unwrap();

    let daemon = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut lines = BufReader::new(stream).lines();
        let line = lines.next().unwrap().unwrap();
        let request: banshee_common::JsonRpcRequest = serde_json::from_str(&line).unwrap();
        // One reply for a call nobody is waiting on, then this call's own.
        for reply in [
            banshee_common::JsonRpcResponse::success(
                Some(serde_json::json!(999)),
                serde_json::json!({"running": false}),
            ),
            banshee_common::JsonRpcResponse::success(
                request.id,
                serde_json::json!({"running": true}),
            ),
        ] {
            let mut text = serde_json::to_string(&reply).unwrap();
            text.push('\n');
            writer.write_all(text.as_bytes()).unwrap();
        }
    });

    let mut client = Client::connect(&path).await.unwrap();
    let status = calls::status(&mut client).await.unwrap();

    assert_eq!(
        status["running"], true,
        "the stale reply was returned instead"
    );
    daemon.join().unwrap();
}

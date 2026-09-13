use super::*;
use crate::state::RecordingMode;
use banshee_common::{BANSHEE_ASK_USER, BANSHEE_STATUS};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

type Incoming = tokio::io::Lines<BufReader<OwnedReadHalf>>;

// Long enough that a slow machine cannot fail a test that would pass
const ARRIVES: Duration = Duration::from_secs(2);
// Short: this one is spent in full every time nothing is expected
const SILENT: Duration = Duration::from_millis(200);

async fn next_message(lines: &mut Incoming) -> serde_json::Value {
    let line = tokio::time::timeout(ARRIVES, lines.next_line())
        .await
        .expect("nothing arrived")
        .expect("the read failed")
        .expect("the connection closed");
    serde_json::from_str(&line).expect("the daemon wrote something that is not JSON")
}

// Named by the constants the daemon dispatches on, so a renamed method
// fails to compile rather than going quietly unanswered
async fn send(writer: &mut OwnedWriteHalf, method: &str, params: serde_json::Value) {
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params: Some(params),
        id: Some(serde_json::json!(1)),
    };
    write_line(writer, &request)
        .await
        .expect("the write failed");
}

// Hands back the writer: dropping it closes the connection under the server
fn connect(state: &Arc<DaemonState>) -> (Incoming, OwnedWriteHalf) {
    let (client, server) = UnixStream::pair().expect("no socket pair");
    tokio::spawn(serve(server, Arc::clone(state)));
    let (reader, writer) = client.into_split();
    (BufReader::new(reader).lines(), writer)
}

/// Connected and subscribed, with the subscribe reply already read.
async fn subscribed(state: &Arc<DaemonState>) -> (Incoming, OwnedWriteHalf, serde_json::Value) {
    let (mut lines, mut writer) = connect(state);
    send(&mut writer, BANSHEE_SUBSCRIBE, serde_json::json!({})).await;
    let reply = next_message(&mut lines).await;
    (lines, writer, reply)
}

#[test]
fn a_subscribe_with_no_events_still_means_state() {
    let asked = requested_events(None);
    assert!(asked.state);
    assert!(!asked.downloads);

    let empty = serde_json::json!({});
    assert!(requested_events(Some(&empty)).state);
}

#[test]
fn each_event_is_asked_for_by_name() {
    let downloads = serde_json::json!({"events": ["downloads"]});
    let asked = requested_events(Some(&downloads));
    assert!(asked.downloads);
    assert!(!asked.state, "asking for one must not deliver the other");

    let both = serde_json::json!({"events": ["state", "downloads"]});
    let asked = requested_events(Some(&both));
    assert!(asked.state && asked.downloads);
}

#[test]
fn an_unknown_event_is_passed_over() {
    let params = serde_json::json!({"events": ["state", "telemetry"]});
    let asked = requested_events(Some(&params));
    assert!(asked.state);
    assert!(!asked.downloads);
}

// The select loop is the only thing that writes a notification, so no unit
// test reaches it. These drive a real socket.
#[tokio::test]
async fn a_subscriber_hears_the_microphone_open() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, _writer, reply) = subscribed(&state).await;
    assert_eq!(reply["result"]["recording"], false);

    // A bare write, not record_start: that also silences the speaker, which
    // would wake the loop through the other arm and prove nothing here
    state.set_recording_mode(RecordingMode::PushToTalk);

    let pushed = next_message(&mut lines).await;
    assert_eq!(pushed["method"], BANSHEE_STATE_CHANGED);
    assert_eq!(pushed["params"]["recording"], true);
    assert_eq!(pushed["params"]["speaking"], false);
    assert!(pushed.get("id").is_none(), "a notification carries no id");
}

// ask_user arms the microphone and then parks inside dispatch, for up to two
// minutes, waiting for the answer. A subscriber that hears nothing while the
// microphone is open is the whole reason not to poll.
#[tokio::test]
async fn a_long_call_does_not_hold_up_this_connection_s_pushes() {
    let (commands, _never_answered) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let (mut lines, mut writer, _) = subscribed(&state).await;

    send(
        &mut writer,
        BANSHEE_ASK_USER,
        serde_json::json!({"question": "ready?"}),
    )
    .await;

    let pushed = next_message(&mut lines).await;
    assert_eq!(
        pushed["method"], "banshee.state_changed",
        "the call that opened the microphone is still parked, so this is a push"
    );
    assert_eq!(pushed["params"]["recording"], true);
}

#[tokio::test]
async fn a_later_subscribe_adds_what_the_first_did_not() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, mut writer) = connect(&state);

    send(
        &mut writer,
        BANSHEE_SUBSCRIBE,
        serde_json::json!({"events": ["state"]}),
    )
    .await;
    next_message(&mut lines).await;
    send(
        &mut writer,
        BANSHEE_SUBSCRIBE,
        serde_json::json!({"events": ["downloads"]}),
    )
    .await;
    next_message(&mut lines).await;

    state.report_download(DownloadProgress {
        model: "silero_vad.onnx".to_string(),
        label: "Voice detection model".to_string(),
        index: 1,
        count: 1,
        bytes: 1,
        total: Some(2),
        state: banshee_common::DownloadState::Downloading,
    });

    let pushed = next_message(&mut lines).await;
    assert_eq!(pushed["method"], BANSHEE_DOWNLOAD_PROGRESS);
    assert_eq!(pushed["params"]["model"], "silero_vad.onnx");
}

#[tokio::test]
async fn a_subscriber_hears_the_daemon_start_speaking() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, _writer, _) = subscribed(&state).await;

    state.speech().speak("anything", false, None).unwrap();

    let pushed = next_message(&mut lines).await;
    assert_eq!(pushed["method"], BANSHEE_STATE_CHANGED);
}

#[tokio::test]
async fn a_connection_that_never_subscribed_hears_nothing() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, mut writer) = connect(&state);

    send(&mut writer, BANSHEE_STATUS, serde_json::json!({})).await;
    assert_eq!(next_message(&mut lines).await["result"]["recording"], false);

    state.set_recording_mode(RecordingMode::PushToTalk);

    assert!(
        tokio::time::timeout(SILENT, lines.next_line())
            .await
            .is_err(),
        "a poller must not be sent pushes it never asked for"
    );
}

#[tokio::test]
async fn a_write_that_moves_nothing_is_not_pushed() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, _writer, _) = subscribed(&state).await;

    // Idle over Idle: the mode is written, but nothing a client sees moves
    state.set_recording_mode(RecordingMode::Idle);

    assert!(
        tokio::time::timeout(SILENT, lines.next_line())
            .await
            .is_err(),
        "a write that moves nothing a client sees must push nothing"
    );
}

// The watchdog rebinds while the daemon idles, so neither the recording nor
// the speaking flag moves with it.
#[tokio::test]
async fn a_subscriber_hears_a_rebind_with_nothing_else_moving() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    state.set_audio_device(Some("OnePlus Buds 3".to_string()));
    let (mut lines, _writer, reply) = subscribed(&state).await;
    assert_eq!(reply["result"]["audio_device"], "OnePlus Buds 3");

    state.set_audio_device(Some("MacBook Pro Microphone".to_string()));
    state.set_missing_device(Some("OnePlus Buds 3".to_string()));

    let pushed = next_message(&mut lines).await;
    assert_eq!(pushed["method"], BANSHEE_STATE_CHANGED);
    assert_eq!(pushed["params"]["audio_device"], "MacBook Pro Microphone");
    assert_eq!(pushed["params"]["recording"], false);
}

// While the named device stays absent the watchdog writes the same name
// every rescan, which is every 5 seconds.
#[tokio::test]
async fn a_rewritten_device_name_pushes_nothing() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, _writer, _) = subscribed(&state).await;

    state.set_missing_device(Some("yeti".to_string()));
    assert_eq!(
        next_message(&mut lines).await["params"]["missing_device"],
        "yeti"
    );

    state.set_missing_device(Some("yeti".to_string()));
    state.set_missing_device(Some("yeti".to_string()));

    assert!(
        tokio::time::timeout(SILENT, lines.next_line())
            .await
            .is_err(),
        "a rescan that finds the same device absent must push nothing"
    );
}

fn test_socket_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("banshee-{name}-{}.sock", std::process::id()))
}

#[tokio::test]
async fn stale_socket_is_reclaimed() {
    let path = test_socket_path("stale");
    // bind then drop: the file stays behind, like a crashed daemon
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
    assert!(path.exists());

    // Between fork and exec a `say` child holds a copy of the dead listener
    // fd, so the probe can transiently see the socket as alive
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let listener = loop {
        match claim_socket(&path) {
            Ok(listener) => break listener,
            Err(e) if std::time::Instant::now() < deadline => {
                assert_eq!(e.kind(), io::ErrorKind::AddrInUse);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(e) => panic!("stale socket not reclaimed: {e}"),
        }
    };
    drop(listener);
    let _ = fs::remove_file(&path);
}

#[tokio::test]
async fn live_socket_refuses_second_instance() {
    let path = test_socket_path("live");
    let _first = std::os::unix::net::UnixListener::bind(&path).unwrap();

    let error = claim_socket(&path).expect_err("second instance not refused");
    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    let _ = fs::remove_file(&path);
}

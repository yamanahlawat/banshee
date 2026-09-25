use super::*;
use crate::state::RecordingMode;
use banshee_common::{BANSHEE_ASK_USER, BANSHEE_STATUS, DownloadProgress};
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
        jsonrpc: banshee_common::Version::V2,
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

#[test]
fn cues_level_and_draws_are_asked_for_by_name() {
    let params = serde_json::json!({"events": ["cues", "level"], "draws": true});
    let asked = requested_events(Some(&params));
    assert!(asked.cues && asked.level && asked.draws);
    assert!(!asked.state);

    let quiet = serde_json::json!({"events": ["cues"]});
    assert!(
        !requested_events(Some(&quiet)).draws,
        "draws is off unless it is said"
    );
}

#[tokio::test]
async fn a_cue_subscriber_hears_a_signal() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, mut writer) = connect(&state);
    send(
        &mut writer,
        BANSHEE_SUBSCRIBE,
        serde_json::json!({"events": ["cues"]}),
    )
    .await;
    let _reply = next_message(&mut lines).await;

    state.cues().emit(crate::audio::cues::Signal::Arm);

    let pushed = next_message(&mut lines).await;
    assert_eq!(pushed["method"], banshee_common::BANSHEE_CUE);
    assert_eq!(pushed["params"]["cue"], "arm");
}

// The channel holds 128 signals, so 300 emits leave this subscriber behind.
#[tokio::test]
async fn a_cue_subscriber_that_lags_still_hears_what_comes_next() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, mut writer) = connect(&state);
    send(
        &mut writer,
        BANSHEE_SUBSCRIBE,
        serde_json::json!({"events": ["cues"]}),
    )
    .await;
    let _reply = next_message(&mut lines).await;

    for _ in 0..300 {
        state.cues().emit(crate::audio::cues::Signal::Disarm);
    }
    state.cues().emit(crate::audio::cues::Signal::Told);

    let told = tokio::time::timeout(ARRIVES * 5, async {
        loop {
            let pushed = next_message(&mut lines).await;
            if pushed["params"]["cue"] == "told" {
                return pushed;
            }
        }
    })
    .await
    .expect("the signal after the lag never arrived");
    assert_eq!(told["method"], banshee_common::BANSHEE_CUE);
}

#[tokio::test]
async fn a_drawing_subscriber_counts_as_a_chip_until_it_goes() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, mut writer) = connect(&state);
    send(
        &mut writer,
        BANSHEE_SUBSCRIBE,
        serde_json::json!({"events": ["cues"], "draws": true}),
    )
    .await;
    let _reply = next_message(&mut lines).await;
    assert_eq!(state.cues().drawers(), 1);

    drop(writer);
    drop(lines);
    let deadline = std::time::Instant::now() + ARRIVES;
    while state.cues().drawers() != 0 && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        state.cues().drawers(),
        0,
        "a chip that left must not keep the sound off"
    );
}

#[tokio::test]
async fn two_subscribes_on_one_connection_count_one_chip() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, mut writer) = connect(&state);
    send(
        &mut writer,
        BANSHEE_SUBSCRIBE,
        serde_json::json!({"events": ["cues"], "draws": true}),
    )
    .await;
    next_message(&mut lines).await;

    send(
        &mut writer,
        BANSHEE_SUBSCRIBE,
        serde_json::json!({"events": ["cues", "level"], "draws": true}),
    )
    .await;
    next_message(&mut lines).await;

    assert_eq!(
        state.cues().drawers(),
        1,
        "one connection is one chip, however many times it subscribes"
    );
}

#[tokio::test]
async fn a_level_subscriber_counts_as_a_watcher_until_it_goes() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, mut writer) = connect(&state);
    send(
        &mut writer,
        BANSHEE_SUBSCRIBE,
        serde_json::json!({"events": ["level"]}),
    )
    .await;
    next_message(&mut lines).await;
    send(
        &mut writer,
        BANSHEE_SUBSCRIBE,
        serde_json::json!({"events": ["cues", "level"]}),
    )
    .await;
    next_message(&mut lines).await;
    assert_eq!(
        state.level_watchers(),
        1,
        "one connection is one watcher, however many times it subscribes"
    );

    drop(writer);
    drop(lines);
    let deadline = std::time::Instant::now() + ARRIVES;
    while state.level_watchers() != 0 && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        state.level_watchers(),
        0,
        "a watcher that left must not keep the sampler running"
    );
}

#[tokio::test]
async fn a_subscriber_that_asks_for_no_level_is_no_watcher() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, mut writer) = connect(&state);
    send(
        &mut writer,
        BANSHEE_SUBSCRIBE,
        serde_json::json!({"events": ["cues"], "draws": true}),
    )
    .await;
    next_message(&mut lines).await;
    assert_eq!(state.level_watchers(), 0);
}

#[tokio::test]
async fn a_level_subscriber_hears_the_level() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, mut writer) = connect(&state);
    send(
        &mut writer,
        BANSHEE_SUBSCRIBE,
        serde_json::json!({"events": ["level"]}),
    )
    .await;
    let _reply = next_message(&mut lines).await;

    state.publish_level(0.5);

    let pushed = next_message(&mut lines).await;
    assert_eq!(pushed["method"], banshee_common::BANSHEE_LEVEL);
    assert_eq!(pushed["params"]["level"], 0.5);

    state.publish_level(0.1234);

    let pushed = next_message(&mut lines).await;
    assert_eq!(
        pushed["params"]["level"], 0.123,
        "the wire carries 3 decimal places, not an f32 widened to its f64 tail"
    );

    state.publish_level(0.1234);

    let pushed = next_message(&mut lines).await;
    assert_eq!(
        pushed["params"]["level"], 0.123,
        "a repeated sample is pushed again"
    );
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

#[tokio::test]
async fn a_subscriber_hears_the_feedback_mode_change() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, _writer, reply) = subscribed(&state).await;
    assert_eq!(
        reply["result"]["feedback"], "none",
        "the fixture must start elsewhere"
    );

    state.set_feedback_mode(crate::config::FeedbackMode::Sound);

    let pushed = next_message(&mut lines).await;
    assert_eq!(pushed["method"], banshee_common::BANSHEE_STATE_CHANGED);
    assert_eq!(pushed["params"]["feedback"], "sound");
}

// A window connects while the pipeline opens and reads "not ready". Without
// this push it keeps that answer until it happens to read the status again.
#[tokio::test]
async fn a_subscriber_hears_the_pipeline_open() {
    let state = crate::test_support::daemon_state_before_the_pipeline();
    let (mut lines, _writer, reply) = subscribed(&state).await;
    assert_eq!(reply["result"]["pipeline"], "opening");

    state.set_pipeline(crate::state::Pipeline::Open);

    let pushed = next_message(&mut lines).await;
    assert_eq!(pushed["method"], BANSHEE_STATE_CHANGED);
    assert_eq!(pushed["params"]["pipeline"], "open");
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
    let claimed = loop {
        match claim_socket(&path) {
            Ok(claimed) => break claimed,
            Err(e) if std::time::Instant::now() < deadline => {
                assert_eq!(e.kind(), io::ErrorKind::AddrInUse);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(e) => panic!("stale socket not reclaimed: {e}"),
        }
    };
    drop(claimed);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(path.with_extension("lock"));
}

#[tokio::test]
async fn live_socket_refuses_second_instance() {
    let path = test_socket_path("live");
    let _first = std::os::unix::net::UnixListener::bind(&path).unwrap();

    let error = claim_socket(&path).expect_err("second instance not refused");
    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    let _ = fs::remove_file(&path);
}

#[tokio::test]
async fn a_line_that_is_not_a_request_is_answered_with_a_parse_error() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (mut lines, mut writer) = connect(&state);

    writer
        .write_all(b"{\"jsonrpc\": \"2.0\", \"method\": 3\n")
        .await
        .unwrap();

    let reply = next_message(&mut lines).await;
    assert_eq!(reply["error"]["code"], banshee_common::rpc_code::PARSE);
    assert_eq!(
        reply["id"],
        serde_json::Value::Null,
        "a request that did not parse has no id to answer on"
    );
}

// The socket is bound in `claim()`, before the microphone is open. A client
// that connects in that gap must be answered, not left waiting for a device.
#[tokio::test]
async fn a_client_is_answered_while_the_pipeline_is_still_opening() {
    let state = crate::test_support::daemon_state_before_the_pipeline();

    let (mut lines, mut writer) = connect(&state);
    send(&mut writer, BANSHEE_STATUS, serde_json::json!({})).await;

    let reply = next_message(&mut lines).await;
    assert_eq!(reply["result"]["pipeline"], "opening");
}

// The grant path leaves this way rather than through `exit`, so the file that
// doubles as the single-instance lock cannot outlive the process that held it.
#[tokio::test]
async fn a_requested_shutdown_takes_the_socket_file_with_it() {
    let path = test_socket_path("shutdown");
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).expect("bind");
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);

    let serving = tokio::spawn({
        let state = Arc::clone(&state);
        let path = path.clone();
        async move { super::run(&state, path, listener).await }
    });
    state.shutdown().notify_one();
    serving.await.expect("the loop ends").expect("no io error");

    assert!(
        !path.exists(),
        "a clean exit leaves no socket to be called stale"
    );
}

// A call can park for minutes inside dispatch, and the loop reads no more of
// the connection while it does. A client that leaves must not be waited for.
#[tokio::test]
async fn a_client_that_leaves_ends_the_call_it_parked() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (client, server) = UnixStream::pair().expect("no socket pair");
    let serving = tokio::spawn(serve(server, Arc::clone(&state)));

    let (reader, mut writer) = client.into_split();
    send(
        &mut writer,
        banshee_common::BANSHEE_GET_TRANSCRIPTION,
        serde_json::json!({ "wait_ms": 5000 }),
    )
    .await;
    drop(writer);
    drop(reader);

    // Well inside the five seconds the call would otherwise hold: a bound for
    // the test, not a limit the daemon promises.
    tokio::time::timeout(std::time::Duration::from_secs(1), serving)
        .await
        .expect("the loop ends when the client does")
        .expect("no panic");
}

// The loop now reads the connection while a call runs, so a request sent
// before the first one answered must be held and served, never swallowed.
#[tokio::test]
async fn a_request_sent_during_a_call_is_still_answered() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (client, server) = UnixStream::pair().expect("no socket pair");
    tokio::spawn(serve(server, Arc::clone(&state)));
    let (reader, mut writer) = client.into_split();
    let mut lines = BufReader::new(reader).lines();

    // The first parks; the second arrives while it does
    send(
        &mut writer,
        banshee_common::BANSHEE_GET_TRANSCRIPTION,
        serde_json::json!({ "wait_ms": 300 }),
    )
    .await;
    send(&mut writer, BANSHEE_STATUS, serde_json::json!({})).await;

    let first = next_message(&mut lines).await;
    assert!(first["result"]["transcriptions"].is_array(), "{first}");
    let second = next_message(&mut lines).await;
    assert_eq!(second["result"]["running"], true, "{second}");
}

#[tokio::test]
async fn a_stale_socket_is_left_to_the_claim_that_holds_the_lock() {
    let path = test_socket_path("locked");
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
    let held = fs::File::create(path.with_extension("lock")).unwrap();
    held.try_lock().unwrap();

    let error = claim_socket(&path).expect_err("a second claim must not take the socket");

    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    assert!(path.exists());
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(path.with_extension("lock"));
}

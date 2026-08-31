mod common;

use banshee_app::socket::{Client, NO_REPLY, backoff};
use banshee_common::BANSHEE_STATUS;
use common::recording_daemon;

#[tokio::test]
async fn a_status_call_round_trips_over_a_unix_socket() {
    let (path, mut seen, _guard) =
        recording_daemon(serde_json::json!({"running": true, "version": "test"})).await;
    let mut client = Client::connect(&path).await.unwrap();
    let status = client
        .call(BANSHEE_STATUS, serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(status["running"], true);
    assert_eq!(status["version"], "test");
    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_STATUS);
}

#[test]
fn backoff_doubles_from_a_quarter_second_and_caps_at_five() {
    assert_eq!(backoff(0).as_millis(), 250);
    assert_eq!(backoff(1).as_millis(), 500);
    assert_eq!(backoff(2).as_millis(), 1000);
    assert_eq!(backoff(3).as_millis(), 2000);
    // 4 is the last attempt still doubling; 5 is where the 5 s cap first bites
    assert_eq!(backoff(4).as_millis(), 4000);
    assert_eq!(backoff(5).as_millis(), 5000);
    assert_eq!(backoff(6).as_millis(), 5000);
    assert_eq!(backoff(10).as_millis(), 5000);
    assert_eq!(backoff(63).as_millis(), 5000);
    assert_eq!(backoff(u32::MAX).as_millis(), 5000);
}

/// Every window command takes the same client lock before it calls, so one call
/// that never gets its reply stops the status, the history and every setting.
// Paused time, so the 30 s deadline is proved without spending 30 s here.
#[tokio::test(start_paused = true)]
async fn a_reply_that_never_arrives_ends_the_call() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("banshee.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let (stream, _) = listener.accept().unwrap();
        // Reads the request, writes nothing, and ends when the client drops.
        for _ in std::io::BufReader::new(stream).lines() {}
    });

    let mut client = Client::connect(&path).await.unwrap();
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        client.call(BANSHEE_STATUS, serde_json::json!({})),
    )
    .await
    .expect("the call has to end on its own, not on this test's deadline")
    .expect_err("no reply arrived, so this cannot be a result");

    // The message, because `SOCKET_CLOSED` carries the same two flags: without
    // it a server thread that died on accept would read as a deadline.
    assert_eq!(error.message, NO_REPLY);
    assert!(error.transport, "a dead wait is a transport failure");
    assert!(
        error.sent,
        "the request went out, so it may already have run"
    );
}

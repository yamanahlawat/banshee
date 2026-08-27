//! Covers Findings 1 and 2 from fix round 1: a window that has never
//! connected and a window whose connection just died repair themselves
//! through the same functions, and a missing daemon is a normal error, not
//! a panic that would keep the window from opening at all.

mod common;

use banshee_app::calls;
use banshee_app::commands::{ensure_connected, force_reconnect};
use banshee_app::socket::{Client, SOCKET_CLOSED};
use common::recording_daemon;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;

#[tokio::test]
async fn a_never_connected_slot_connects_on_first_use() {
    let (path, _seen, _guard) = recording_daemon(serde_json::json!({"ok": true})).await;
    let mut slot: Option<Client> = None;

    ensure_connected(&mut slot, &path).await.unwrap();

    assert!(slot.is_some());
}

#[tokio::test]
async fn an_already_connected_slot_is_left_alone() {
    let (path, _seen, _guard) = recording_daemon(serde_json::json!({"ok": true})).await;
    let mut slot: Option<Client> = Some(Client::connect(&path).await.unwrap());
    // A path nothing listens on: if `ensure_connected` reconnected anyway,
    // this would fail.
    let bogus = std::path::PathBuf::from("/nonexistent/banshee.sock");

    ensure_connected(&mut slot, &bogus).await.unwrap();

    assert!(slot.is_some());
}

#[tokio::test]
async fn force_reconnect_replaces_whatever_was_there() {
    let (path, _seen, _guard) = recording_daemon(serde_json::json!({"ok": true})).await;
    let mut slot: Option<Client> = None;

    force_reconnect(&mut slot, &path).await.unwrap();

    assert!(slot.is_some());
}

/// The core of Finding 1: connecting to a socket nothing is listening on
/// must come back as an `Err` a command can return to its caller, not a
/// panic that would stop the whole window from opening.
#[tokio::test]
async fn a_missing_daemon_is_a_normal_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("no-daemon-here.sock");
    let mut slot: Option<Client> = None;

    let error = ensure_connected(&mut slot, &path).await.unwrap_err();

    assert!(slot.is_none());
    assert!(!error.message.is_empty());
}

/// The core of Finding 2: a connection that goes dead mid-session is
/// detected and repaired, and the retried call against the new connection
/// succeeds.
#[tokio::test]
async fn a_dead_connection_is_repaired_and_the_retried_call_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("banshee.sock");

    // First "daemon": accepts, then closes without answering, simulating a
    // restart mid-request.
    let listener = UnixListener::bind(&path).unwrap();
    let dying = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        // Read the request so the client's write succeeds, then drop
        // without answering: the client's next read sees a clean EOF,
        // exactly like a daemon that exits between request and reply.
        let mut lines = BufReader::new(stream).lines();
        lines.next().unwrap().unwrap();
    });

    let mut slot = Some(Client::connect(&path).await.unwrap());
    let dead = calls::status(slot.as_mut().unwrap()).await.unwrap_err();
    assert_eq!(dead.code, -32000);
    assert_eq!(dead.message, SOCKET_CLOSED);
    dying.join().unwrap();
    std::fs::remove_file(&path).unwrap();

    // A second daemon takes over the same path, standing in for the
    // restarted process.
    let listener = UnixListener::bind(&path).unwrap();
    let revived = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut lines = BufReader::new(stream).lines();
        let line = lines.next().unwrap().unwrap();
        let request: banshee_common::JsonRpcRequest = serde_json::from_str(&line).unwrap();
        let reply = banshee_common::JsonRpcResponse::success(
            request.id,
            serde_json::json!({"running": true}),
        );
        let mut text = serde_json::to_string(&reply).unwrap();
        text.push('\n');
        writer.write_all(text.as_bytes()).unwrap();
    });

    force_reconnect(&mut slot, &path).await.unwrap();
    let status = calls::status(slot.as_mut().unwrap()).await.unwrap();

    assert_eq!(status["running"], true);
    revived.join().unwrap();
}

//! Covers Findings 1 and 2 from fix round 1: a window that has never
//! connected and a window whose connection just died repair themselves
//! through the same functions, and a missing daemon is a normal error, not
//! a panic that would keep the window from opening at all.

mod common;

use banshee_app::calls;
use banshee_app::commands::{ensure_connected, force_reconnect, retrying};
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
    let (old_path, mut old_seen, _old_guard) =
        recording_daemon(serde_json::json!({"running": false})).await;
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({"running": true})).await;
    let mut slot: Option<Client> = Some(Client::connect(&old_path).await.unwrap());

    force_reconnect(&mut slot, &path).await.unwrap();
    let status = calls::status(slot.as_mut().unwrap()).await.unwrap();

    assert_eq!(status["running"], true, "the call went to the new daemon");
    assert!(seen.recv().await.is_some());
    assert!(
        old_seen.try_recv().is_err(),
        "the connection that was there is not asked anything"
    );
}

/// A daemon that drops its first connection unread, says so, and answers the
/// next connection normally: what a client holding a connection to a daemon
/// that has since restarted sees.
fn daemon_that_restarted(
    path: &std::path::Path,
) -> (std::thread::JoinHandle<()>, std::sync::mpsc::Receiver<()>) {
    let listener = UnixListener::bind(path).unwrap();
    let (dropped_tx, dropped_rx) = std::sync::mpsc::channel();
    let daemon = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        drop(stream);
        dropped_tx.send(()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        let request: banshee_common::JsonRpcRequest = serde_json::from_str(&line).unwrap();
        let reply = banshee_common::JsonRpcResponse::success(
            request.id,
            serde_json::json!({"running": true}),
        );
        let mut text = serde_json::to_string(&reply).unwrap();
        text.push('\n');
        writer.write_all(text.as_bytes()).unwrap();
    });
    (daemon, dropped_rx)
}

/// The guarantee the window exists for: a daemon restart between two commands
/// is invisible to the caller, because the request never reached the old one.
#[tokio::test]
async fn a_command_survives_a_daemon_that_restarted_since_the_last_call() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("banshee.sock");
    let (daemon, dropped) = daemon_that_restarted(&path);
    let mut slot: Option<Client> = Some(Client::connect(&path).await.unwrap());
    dropped.recv().unwrap();

    let status = retrying(&path, &mut slot, |client| Box::pin(calls::status(client)))
        .await
        .unwrap();

    assert_eq!(status["running"], true);
    assert!(
        slot.is_some(),
        "the retry's connection is kept for the next call"
    );
    daemon.join().unwrap();
}

/// A replay would speak a preview twice, or write an agent's config twice.
#[tokio::test]
async fn a_request_the_daemon_read_is_not_sent_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("banshee.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let daemon = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        drop(stream);
        listener.set_nonblocking(true).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        // A second connection would be the replay this test forbids
        listener.accept().is_ok()
    });
    let mut slot: Option<Client> = None;

    let error = retrying(&path, &mut slot, |client| Box::pin(calls::status(client)))
        .await
        .unwrap_err();

    assert!(error.transport, "{error:?}");
    assert!(slot.is_none(), "a dead connection is dropped");
    assert!(!daemon.join().unwrap(), "no second connection was opened");
}

/// The core of Finding 1: connecting to a socket nothing is listening on
/// must come back as an `Err` a command can return to its caller, not a
/// panic that would stop the whole window from opening.
#[tokio::test]
async fn a_missing_daemon_is_a_normal_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("no-daemon-here.sock");
    let mut slot: Option<Client> = None;

    let Err(error) = ensure_connected(&mut slot, &path).await else {
        panic!("nothing listens at {}", path.display());
    };

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

/// The path a real restart takes: the write fails, and the operating system
/// names that error itself. No message match can recognise it.
#[tokio::test]
async fn a_write_to_a_departed_daemon_reads_as_a_dead_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("banshee.sock");

    let listener = UnixListener::bind(&path).unwrap();
    let departing = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        drop(stream);
    });

    let mut slot = Some(Client::connect(&path).await.unwrap());
    departing.join().unwrap();

    // The first call after the peer left. Either half can fail first, so the
    // test pins what both must report rather than which one wins the race.
    let mut dead = calls::status(slot.as_mut().unwrap()).await;
    if dead.is_ok() {
        dead = calls::status(slot.as_mut().unwrap()).await;
    }
    let error = dead.unwrap_err();

    assert!(
        error.transport,
        "a departed daemon must read as a transport failure, got {error:?}"
    );
    assert!(
        !error.sent,
        "a write that failed cannot have reached the daemon, got {error:?}"
    );
}

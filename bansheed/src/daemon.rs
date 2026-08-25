use banshee_common::utils::get_socket_path;
use banshee_common::{
    BANSHEE_DOWNLOAD_PROGRESS, BANSHEE_STATE_CHANGED, BANSHEE_SUBSCRIBE, DownloadProgress,
    JsonRpcNotification, JsonRpcRequest,
};
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Mutex, broadcast, watch};

use crate::api::{dispatch, live_state};
use crate::state::DaemonState;

// Claimed before model loading, so a lost single-instance race stays cheap
pub fn claim() -> Result<(std::path::PathBuf, UnixListener), io::Error> {
    let socket_path = get_socket_path()
        .ok_or_else(|| io::Error::other("could not find home directory for the socket path"))?;

    if let Some(parent_dir) = socket_path.parent() {
        fs::create_dir_all(parent_dir)?;
    }

    let listener = claim_socket(&socket_path)?;
    // owner-only: the socket is a command channel into the mic and speakers
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    Ok((socket_path, listener))
}

pub async fn run(
    daemon_state: &Arc<DaemonState>,
    socket_path: std::path::PathBuf,
    listener: UnixListener,
) -> Result<(), std::io::Error> {
    println!("Listening on {}", socket_path.display());

    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    // Coarse tick: the ceiling it enforces is measured in minutes
    let mut watchdog = tokio::time::interval(Duration::from_secs(30));
    // Input loss lands within ~1s of a BT drop (#47); one second is enough.
    let mut input_health = tokio::time::interval(Duration::from_secs(1));
    // First tick is immediate; skip so we do not false-positive before cpal
    // has delivered its opening callbacks.
    input_health.tick().await;

    loop {
        tokio::select! {
            _ = sigint.recv() => break,
            _ = sigterm.recv() => break,
            _ = watchdog.tick() => {
                daemon_state.expire_stuck_recording();
            }
            _ = input_health.tick() => {
                if let Some(configured) = daemon_state.configured_input() {
                    crate::audio::poll_input_health(daemon_state, configured);
                }
            }
            // Stop RPC: wait a beat so the client task can flush its response
            _ = daemon_state.shutdown().notified() => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                break;
            }
            accepted = listener.accept() => match accepted {
                Ok((stream, _addr)) => {
                    println!("New client connected!");
                    tokio::spawn(serve(stream, Arc::clone(daemon_state)));
                }
                Err(error) => println!("Connection failed, Error: {error}"),
            }
        }
    }

    println!("Shutting down.");
    daemon_state.speech().stop();
    let _ = fs::remove_file(&socket_path);
    Ok(())
}

async fn write_line(
    writer: &mut (impl AsyncWriteExt + Unpin),
    message: &impl serde::Serialize,
) -> io::Result<()> {
    let mut line = serde_json::to_string(message).map_err(io::Error::other)?;
    line.push('\n');
    writer.write_all(line.as_bytes()).await
}

struct Events {
    state: bool,
    downloads: bool,
}

// What a `subscribe` call asked to be sent. Absent means state alone, so a
// client that names no events is unaffected by there being more than one
fn requested_events(params: Option<&serde_json::Value>) -> Events {
    let Some(named) = params
        .and_then(|params| params.get("events"))
        .and_then(|events| events.as_array())
    else {
        return Events {
            state: true,
            downloads: false,
        };
    };
    let asked = |name: &str| named.iter().any(|event| event.as_str() == Some(name));
    Events {
        state: asked(banshee_common::EVENT_STATE),
        downloads: asked(banshee_common::EVENT_DOWNLOADS),
    }
}

// Sends one connection every download notification the daemon raises
async fn push_downloads(
    writer: Arc<Mutex<OwnedWriteHalf>>,
    mut downloads: broadcast::Receiver<DownloadProgress>,
) {
    loop {
        let progress = match downloads.recv().await {
            Ok(progress) => progress,
            // Too far behind to catch up on the ones it missed, but the ones
            // still coming are worth having
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        };
        let Ok(params) = serde_json::to_value(progress) else {
            continue;
        };
        let notification = JsonRpcNotification::new(BANSHEE_DOWNLOAD_PROGRESS, params);
        if write_line(&mut *writer.lock().await, &notification)
            .await
            .is_err()
        {
            break;
        }
    }
}

/// Sends one connection its state changes, until the daemon stops or the client
/// does. `told` is the state that client already has, which a push is judged
/// against: an unchanged one is not worth a line.
async fn push_changes(
    state: Arc<DaemonState>,
    writer: Arc<Mutex<OwnedWriteHalf>>,
    mut recording: watch::Receiver<bool>,
    mut speaking: watch::Receiver<bool>,
    mut told: serde_json::Value,
) {
    loop {
        // Both arms only wake the task; the state is read fresh below
        let woken = tokio::select! {
            woken = recording.changed() => woken,
            woken = speaking.changed() => woken,
        };
        if woken.is_err() {
            break;
        }
        let now = live_state(&state);
        if now == told {
            continue;
        }
        let notification = JsonRpcNotification::new(BANSHEE_STATE_CHANGED, now);
        if write_line(&mut *writer.lock().await, &notification)
            .await
            .is_err()
        {
            break;
        }
        told = notification.params;
    }
}

/// Answers this connection's requests, and pushes state changes to it once it
/// has subscribed. The subscription lives and dies with the connection.
async fn serve(stream: UnixStream, state: Arc<DaemonState>) {
    let (reader, writer) = stream.into_split();
    // Pushing is a task of its own, because ask_user parks this one inside
    // dispatch for minutes while it holds the microphone open
    let writer = Arc::new(Mutex::new(writer));
    let mut lines = BufReader::new(reader).lines();
    // The handles are the bookkeeping: a later subscribe opens only the kind
    // that has none
    let mut pushing_state: Option<tokio::task::JoinHandle<()>> = None;
    let mut pushing_downloads: Option<tokio::task::JoinHandle<()>> = None;

    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(request) = serde_json::from_str::<JsonRpcRequest>(&line) else {
            continue;
        };
        // Taken before the reply is built: a change landing in between then
        // costs a duplicate push, where the other order would lose it
        let asked = if request.method == BANSHEE_SUBSCRIBE {
            requested_events(request.params.as_ref())
        } else {
            Events {
                state: false,
                downloads: false,
            }
        };
        let opening_state = (asked.state && pushing_state.is_none()).then(|| {
            (
                state.subscribe_recording(),
                state.speech().subscribe_speaking(),
                live_state(&state),
            )
        });
        let opening_downloads =
            (asked.downloads && pushing_downloads.is_none()).then(|| state.subscribe_downloads());

        let response = dispatch(request, &state).await;
        if write_line(&mut *writer.lock().await, &response)
            .await
            .is_err()
        {
            break;
        }

        if let Some((recording, speaking, told)) = opening_state {
            pushing_state = Some(tokio::spawn(push_changes(
                Arc::clone(&state),
                Arc::clone(&writer),
                recording,
                speaking,
                told,
            )));
        }
        if let Some(downloads) = opening_downloads {
            pushing_downloads = Some(tokio::spawn(push_downloads(Arc::clone(&writer), downloads)));
        }
    }

    for task in [pushing_state, pushing_downloads].into_iter().flatten() {
        task.abort();
    }
}

// Probe with std's blocking connect: tokio's nonblocking UDS connect on
// macOS reports success against a dead socket
pub fn socket_answers(socket_path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket_path).is_ok()
}

// The socket file doubles as the single-instance lock
fn claim_socket(socket_path: &Path) -> io::Result<UnixListener> {
    if socket_path.exists() {
        if socket_answers(socket_path) {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "another banshee daemon is already running",
            ));
        }
        // nobody answered: stale socket left by an unclean exit
        println!("Removing stale socket at {}", socket_path.display());
        fs::remove_file(socket_path)?;
    }
    UnixListener::bind(socket_path)
}

#[cfg(test)]
mod tests {
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

        state.speech().speak("anything", false).unwrap();

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
}

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

    loop {
        tokio::select! {
            _ = sigint.recv() => break,
            _ = sigterm.recv() => break,
            _ = watchdog.tick() => {
                daemon_state.expire_stuck_recording();
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
    mut transcribing: watch::Receiver<bool>,
    mut devices: watch::Receiver<u64>,
    mut told: serde_json::Value,
) {
    loop {
        // Every arm only wakes the task; the state is read fresh below
        let woken = tokio::select! {
            woken = recording.changed() => woken,
            woken = speaking.changed() => woken,
            woken = transcribing.changed() => woken,
            woken = devices.changed() => woken,
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

/// The subscription lives and dies with the connection.
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
                state.subscribe_transcribing(),
                state.device_changes(),
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

        if let Some((recording, speaking, transcribing, devices, told)) = opening_state {
            pushing_state = Some(tokio::spawn(push_changes(
                Arc::clone(&state),
                Arc::clone(&writer),
                recording,
                speaking,
                transcribing,
                devices,
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
mod tests;

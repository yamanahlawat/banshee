use banshee_common::utils::get_socket_path;
use banshee_common::{JsonRpcRequest, JsonRpcResponse};
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};

use crate::api::dispatch;
use crate::state::DaemonState;

pub async fn run(daemon_state: &Arc<DaemonState>) -> Result<(), std::io::Error> {
    println!("Starting unix socket listener...");

    let socket_path = get_socket_path()
        .ok_or_else(|| io::Error::other("could not find home directory for the socket path"))?;

    if let Some(parent_dir) = socket_path.parent() {
        fs::create_dir_all(parent_dir)?;
    }

    let listener = claim_socket(&socket_path)?;
    // owner-only: the socket is a command channel into the mic and speakers
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    println!("Listening on {}", socket_path.display());

    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;

    loop {
        tokio::select! {
            _ = sigint.recv() => break,
            _ = sigterm.recv() => break,
            accepted = listener.accept() => match accepted {
                Ok((mut stream, _addr)) => {
                    println!("New client connected!");
                    let state = Arc::clone(daemon_state);
                    // Spawn a new task to handle the client connection
                    tokio::spawn(async move {
                        let (reader, mut writer) = stream.split();
                        let reader = BufReader::new(reader);
                        let mut lines = reader.lines();

                        while let Ok(Some(line)) = lines.next_line().await {
                            // Try to parse the incoming line as a JSON-RPC request
                            if let Ok(request) = serde_json::from_str::<JsonRpcRequest>(&line) {
                                let response: JsonRpcResponse = dispatch(request, &state).await;
                                if let Ok(mut response_string) = serde_json::to_string(&response) {
                                    response_string.push('\n');
                                    let _ = writer.write_all(response_string.as_bytes()).await;
                                }
                            }
                        }
                    });
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

// The socket file doubles as the single-instance lock.
// Probe with std's blocking connect: tokio's nonblocking UDS connect on
// macOS reports success against a dead socket
fn claim_socket(socket_path: &Path) -> io::Result<UnixListener> {
    if socket_path.exists() {
        match std::os::unix::net::UnixStream::connect(socket_path) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "another banshee daemon is already running",
                ));
            }
            Err(_) => {
                // nobody answered: stale socket left by an unclean exit
                println!("Removing stale socket at {}", socket_path.display());
                fs::remove_file(socket_path)?;
            }
        }
    }
    UnixListener::bind(socket_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_socket_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("banshee-{name}-{}.sock", std::process::id()))
    }

    #[tokio::test]
    async fn stale_socket_is_reclaimed() {
        let path = test_socket_path("stale");
        // bind then drop: the file stays behind, like a crashed daemon
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists());

        // parallel tests fork `say`; between fork and exec the child briefly
        // holds a copy of the dead listener fd, so the probe can transiently
        // see the socket as alive: retry past the window
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

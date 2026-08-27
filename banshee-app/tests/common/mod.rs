use banshee_common::{JsonRpcRequest, JsonRpcResponse};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread::JoinHandle;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// Joins the daemon thread on drop, so a panic inside it (a malformed
/// request, a write that fails) fails the test that dropped this guard
/// instead of vanishing in a detached task nothing observes.
pub struct DaemonGuard(Option<JoinHandle<()>>);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take()
            && let Err(panic) = handle.join()
        {
            std::panic::resume_unwind(panic);
        }
    }
}

/// A fake daemon that answers every request with `respond` and reports the
/// request it saw on the returned channel. The listener binds before this
/// function returns, so a caller can connect immediately with no sleep.
async fn daemon(
    respond: impl Fn(Option<serde_json::Value>) -> JsonRpcResponse + Send + 'static,
) -> (PathBuf, UnboundedReceiver<JsonRpcRequest>, DaemonGuard) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("banshee.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let (methods_tx, methods_rx) = unbounded_channel();
    let handle = std::thread::spawn(move || {
        let _dir = dir;
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let line = line.unwrap();
            let request: JsonRpcRequest = serde_json::from_str(&line).unwrap();
            let reply = respond(request.id.clone());
            methods_tx.send(request).unwrap();
            let mut text = serde_json::to_string(&reply).unwrap();
            text.push('\n');
            writer.write_all(text.as_bytes()).unwrap();
        }
    });
    (path, methods_rx, DaemonGuard(Some(handle)))
}

/// Answers every request with `reply` as the JSON-RPC result.
pub async fn recording_daemon(
    reply: serde_json::Value,
) -> (PathBuf, UnboundedReceiver<JsonRpcRequest>, DaemonGuard) {
    daemon(move |id| JsonRpcResponse::success(id, reply.clone())).await
}

/// Answers every request with a JSON-RPC error. Only `calls_over_the_socket.rs`
/// uses this today, so the other binary that includes this module sees it as
/// dead code.
#[allow(dead_code)]
pub async fn recording_error_daemon(
    code: i32,
    message: &str,
) -> (PathBuf, UnboundedReceiver<JsonRpcRequest>, DaemonGuard) {
    let message = message.to_string();
    daemon(move |id| JsonRpcResponse::error(id, code, message.clone())).await
}

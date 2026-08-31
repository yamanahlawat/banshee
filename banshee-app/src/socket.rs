use banshee_common::{BANSHEE_SUBSCRIBE, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// What a dropped connection reads as. Distinct from any message the daemon
/// itself writes, so a caller can tell a dead socket from a real refusal.
pub const SOCKET_CLOSED: &str = "the daemon closed the socket";

/// What a deadline reached with no answer reads as. Distinct from
/// `SOCKET_CLOSED`: both are transport failures, and only the message
/// separates a peer that went away from one that is still there and silent.
pub const NO_REPLY: &str = "the daemon did not answer";

/// The window's slowest call is `preview_voice`, and the daemon answers that
/// with an utterance id rather than waiting on playback. So this covers daemon
/// work only, and is short enough that a wedged daemon does not hold every
/// command for a minute. `ask_user`, which the daemon bounds at 120 s, is not
/// a call the window makes.
const REPLY_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    /// True when the connection failed rather than the daemon refusing. The
    /// daemon writes `-32000` for its own refusals too, so the code alone
    /// cannot tell the two apart.
    pub transport: bool,
    /// True once the request reached the socket. A request that failed on
    /// the way out cannot have been acted on, so it is safe to send again;
    /// one that failed while waiting for the reply may already have run.
    pub sent: bool,
}

pub struct Client {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    next_id: u64,
}

impl Client {
    pub async fn connect(path: &Path) -> std::io::Result<Client> {
        let stream = UnixStream::connect(path).await?;
        let (read, writer) = stream.into_split();
        Ok(Client {
            reader: BufReader::new(read),
            writer,
            next_id: 1,
        })
    }

    pub async fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params: Some(params),
            id: Some(serde_json::json!(id)),
        };
        let mut line = serde_json::to_string(&request).map_err(io_error)?;
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .await
            .map_err(unsent)?;
        // Around the whole read, not around each line: a peer that keeps
        // sending lines that answer some other call would reset a per-line
        // deadline for ever.
        tokio::time::timeout(REPLY_DEADLINE, self.read_reply(&request.id))
            .await
            .unwrap_or_else(|_| Err(io_error(NO_REPLY)))
    }

    /// Reads until the reply to `id` arrives. Cancelling this leaves the bytes
    /// it consumed nowhere, so its caller must not use the connection again.
    async fn read_reply(
        &mut self,
        id: &Option<serde_json::Value>,
    ) -> Result<serde_json::Value, RpcError> {
        loop {
            let mut reply = String::new();
            let read = self.reader.read_line(&mut reply).await.map_err(io_error)?;
            if read == 0 {
                return Err(RpcError {
                    code: -32000,
                    message: SOCKET_CLOSED.to_string(),
                    transport: true,
                    sent: true,
                });
            }
            // A notification carries no `result` and no `error`, so the untagged
            // enum below never matches one; only a reply to some call parses.
            if let Ok(response) = serde_json::from_str::<JsonRpcResponse>(&reply) {
                // A reply left behind by an abandoned call is not this call's
                // answer, whatever it holds. Read past it.
                if !answers(&response, id) {
                    continue;
                }
                match response {
                    JsonRpcResponse::Success { result, .. } => return Ok(result),
                    JsonRpcResponse::Error { error, .. } => {
                        return Err(RpcError {
                            code: error.code,
                            message: error.message,
                            transport: false,
                            sent: true,
                        });
                    }
                }
            }
        }
    }

    /// Runs until the socket drops; every notification is passed to `on_event`.
    /// The daemon answers the subscribe call with the same status its pushes
    /// measure against, so `on_open` sees a snapshot with no gap after it.
    pub async fn subscribe(
        mut self,
        events: &[&str],
        on_open: impl FnOnce(serde_json::Value),
        mut on_event: impl FnMut(JsonRpcNotification),
    ) -> std::io::Result<()> {
        let opening = self
            .call(BANSHEE_SUBSCRIBE, serde_json::json!({ "events": events }))
            .await
            .map_err(|e| std::io::Error::other(e.message))?;
        on_open(opening);
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line).await? == 0 {
                return Ok(());
            }
            if let Ok(notification) = serde_json::from_str::<JsonRpcNotification>(&line) {
                on_event(notification);
            }
        }
    }
}

pub fn answers(response: &JsonRpcResponse, id: &Option<serde_json::Value>) -> bool {
    let reply_id = match response {
        JsonRpcResponse::Success { id, .. } => id,
        JsonRpcResponse::Error { id, .. } => id,
    };
    reply_id == id
}

/// A transport failure that happened before the request left the client.
fn unsent(error: impl std::fmt::Display) -> RpcError {
    RpcError {
        sent: false,
        ..io_error(error)
    }
}

fn io_error(error: impl std::fmt::Display) -> RpcError {
    RpcError {
        code: -32000,
        message: error.to_string(),
        transport: true,
        sent: true,
    }
}

pub fn backoff(attempt: u32) -> Duration {
    Duration::from_millis((250u64 << attempt.min(5)).min(5000))
}

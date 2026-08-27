use banshee_common::{BANSHEE_SUBSCRIBE, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// What a dropped connection reads as. Distinct from any message the daemon
/// itself writes, so a caller can tell a dead socket from a real refusal.
pub const SOCKET_CLOSED: &str = "the daemon closed the socket";

#[derive(Debug)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
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
            .map_err(io_error)?;
        loop {
            let mut reply = String::new();
            let read = self.reader.read_line(&mut reply).await.map_err(io_error)?;
            if read == 0 {
                return Err(RpcError {
                    code: -32000,
                    message: SOCKET_CLOSED.to_string(),
                });
            }
            // A notification carries no `result` and no `error`, so the untagged
            // enum below never matches one; only a reply to this call parses.
            if let Ok(response) = serde_json::from_str::<JsonRpcResponse>(&reply) {
                match response {
                    JsonRpcResponse::Success { result, .. } => return Ok(result),
                    JsonRpcResponse::Error { error, .. } => {
                        return Err(RpcError {
                            code: error.code,
                            message: error.message,
                        });
                    }
                }
            }
        }
    }

    /// Runs until the socket drops; every notification is passed to `on_event`.
    pub async fn subscribe(
        mut self,
        events: &[&str],
        mut on_event: impl FnMut(JsonRpcNotification),
    ) -> std::io::Result<()> {
        self.call(BANSHEE_SUBSCRIBE, serde_json::json!({ "events": events }))
            .await
            .map_err(|e| std::io::Error::other(e.message))?;
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

fn io_error(error: impl std::fmt::Display) -> RpcError {
    RpcError {
        code: -32000,
        message: error.to_string(),
    }
}

pub fn backoff(attempt: u32) -> Duration {
    Duration::from_millis((250u64 << attempt.min(5)).min(5000))
}

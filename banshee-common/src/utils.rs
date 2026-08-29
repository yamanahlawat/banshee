use dirs;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;

use crate::error::BansheeError;
use crate::{BANSHEE_SUBSCRIBE, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

/// A binary that ships beside this one, inside the same `Banshee.app`. A
/// symlink reaches the CLI, so canonicalize finds the directory it lives in.
pub fn sibling(exe: &Path, name: &str) -> Result<PathBuf, BansheeError> {
    let real = std::fs::canonicalize(exe)?;
    let found = real
        .parent()
        .ok_or_else(|| BansheeError::Other("banshee is not inside a directory".into()))?
        .join(name);
    if !found.exists() {
        return Err(BansheeError::Other(format!(
            "{} not found; reinstall so {name} ships beside the CLI",
            found.display()
        )));
    }
    Ok(found)
}

/// The sibling above, ready to run. Both binaries in the bundle reach the
/// others this way.
pub fn sibling_command(name: &str) -> Result<std::process::Command, BansheeError> {
    let exe = std::env::current_exe()?;
    Ok(std::process::Command::new(sibling(&exe, name)?))
}

pub fn get_socket_path() -> Option<PathBuf> {
    let base_path = dirs::home_dir()?;
    Some(base_path.join(".banshee").join("banshee.sock"))
}

pub fn get_models_path() -> Option<PathBuf> {
    let base_path = dirs::home_dir()?;
    Some(base_path.join(".banshee").join("models"))
}

pub fn get_config_path() -> Option<PathBuf> {
    let base_path = dirs::home_dir()?;
    Some(base_path.join(".banshee").join("config.toml"))
}

pub fn get_db_path() -> Option<PathBuf> {
    let base_path = dirs::home_dir()?;
    Some(base_path.join(".banshee").join("banshee.db"))
}

pub fn get_oov_log_path() -> Option<PathBuf> {
    let base_path = dirs::home_dir()?;
    Some(base_path.join(".banshee").join("oov-words.log"))
}

pub async fn call_daemon(method: &str, params: Value) -> Result<Value, BansheeError> {
    Ok(call(method, params).await?.0)
}

/// A connection held open for pushed state changes. It ends when the daemon
/// closes the socket, and nothing here reconnects.
pub struct Subscription {
    lines: Lines<BufReader<UnixStream>>,
}

impl Subscription {
    /// The daemon's state at the moment of subscribing, and the connection that
    /// carries every later notification. The two differ in width: the opening
    /// state is the whole `banshee.status` reply, and a change carries only the
    /// fields that move on their own. Re-read `banshee.status` for the rest.
    pub async fn open(events: &[&str]) -> Result<(Value, Self), BansheeError> {
        let (state, lines) =
            call(BANSHEE_SUBSCRIBE, serde_json::json!({ "events": events })).await?;
        Ok((state, Subscription { lines }))
    }

    /// The next notification of one method, skipping the kinds this caller did
    /// not ask about. `None` once the daemon closes the connection.
    pub async fn next_of(&mut self, method: &str) -> Result<Option<Value>, BansheeError> {
        loop {
            let Some(line) = self.lines.next_line().await? else {
                return Ok(None);
            };
            let pushed: JsonRpcNotification = serde_json::from_str(&line)?;
            if pushed.method == method {
                return Ok(Some(pushed.params));
            }
        }
    }
}

/// Hands back the connection along with the reply, so a caller that wants more
/// than one message keeps reading the same one.
async fn call(
    method: &str,
    params: Value,
) -> Result<(Value, Lines<BufReader<UnixStream>>), BansheeError> {
    let socket_path = get_socket_path()
        .ok_or_else(|| BansheeError::Other("Could not find home directory".to_string()))?;

    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params: Some(params),
        id: Some(serde_json::json!(1)),
    };

    let mut request_string = serde_json::to_string(&request)?;

    let mut stream = UnixStream::connect(socket_path).await?;

    request_string.push('\n');
    stream.write_all(request_string.as_bytes()).await?;

    let mut lines = BufReader::new(stream).lines();
    // Empty when the daemon closed without answering. Deliberately not guarded
    // here: callers read the decode failure that follows as an orphaned socket
    let response = lines.next_line().await?.unwrap_or_default();

    match serde_json::from_str::<JsonRpcResponse>(&response)? {
        JsonRpcResponse::Success { result, .. } => Ok((result, lines)),
        JsonRpcResponse::Error { error, .. } => Err(BansheeError::Rpc {
            code: error.code,
            message: error.message,
        }),
    }
}

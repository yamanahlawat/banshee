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

/// launchd's names for the two login jobs. The daemon binary installs them and
/// the window starts them, so the spelling is shared.
pub const DAEMON_AGENT: &str = "com.banshee.daemon";
pub const TRAY_AGENT: &str = "com.banshee.tray";

/// Writes `bytes` to `path` through a staged file and a rename, so a partial
/// write never truncates a file the user hand-edits. `mode` applies from the
/// first byte on disk, and the rename carries it with the inode.
pub fn write_atomically(path: &Path, bytes: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let extension = path.extension().unwrap_or_default().to_string_lossy();
    // A staging name shared between processes lets two of them interleave their
    // bytes.
    let staged = path.with_extension(format!("{extension}.{}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    if let Some(mode) = mode {
        options.mode(mode);
    }
    options.open(&staged)?.write_all(bytes)?;
    std::fs::rename(&staged, path)
}

/// systemd's name for the daemon's user unit. `bansheed` writes the file and
/// `banshee-app` starts it, so the spelling is shared.
pub const DAEMON_UNIT: &str = "banshee.service";

/// systemd's name for the tray's user unit.
pub const TRAY_UNIT: &str = "banshee-tray.service";

/// The unit that runs the job a launchd label names.
pub fn systemd_unit(label: &str) -> Option<&'static str> {
    match label {
        DAEMON_AGENT => Some(DAEMON_UNIT),
        TRAY_AGENT => Some(TRAY_UNIT),
        _ => None,
    }
}

/// What launchctl calls one job of the logged-in user.
pub fn launchd_target(label: &str) -> String {
    format!("gui/{}/{label}", uid())
}

pub fn uid() -> u32 {
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    unsafe { getuid() }
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

pub fn get_credentials_path() -> Option<PathBuf> {
    let base_path = dirs::home_dir()?;
    Some(base_path.join(".banshee").join("credentials.toml"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_daemon_label_names_its_own_unit() {
        assert_eq!(systemd_unit(DAEMON_AGENT), Some(DAEMON_UNIT));
    }

    #[test]
    fn the_tray_label_names_its_own_unit() {
        assert_eq!(systemd_unit(TRAY_AGENT), Some("banshee-tray.service"));
    }
}

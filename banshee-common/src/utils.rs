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
/// write never truncates a file the user hand-edits, and the bytes reach the
/// disk before the rename, so a power loss leaves the old file or the new one.
/// `mode` applies from the first byte on disk, and the rename carries it with
/// the inode. A symlink at `path` is followed, so the write lands on the file
/// it names, and the link stays. A link whose target is missing fails the
/// write, and the error names the link and the target.
pub fn write_atomically(path: &Path, bytes: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    // A rename replaces a symlink, so a dotfile manager's link would become a
    // plain file.
    let resolved = if path.is_symlink() {
        std::fs::canonicalize(path).map_err(|error| {
            let target = std::fs::read_link(path).unwrap_or_default();
            std::io::Error::new(
                error.kind(),
                format!("{} links to {}: {error}", path.display(), target.display()),
            )
        })?
    } else {
        path.to_path_buf()
    };
    let path = resolved.as_path();

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
    let mut file = options.open(&staged)?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| std::fs::rename(&staged, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    written
}

/// systemd's name for the daemon's user unit. `bansheed` writes the file and
/// `banshee-app` starts it, so the spelling is shared.
const DAEMON_UNIT: &str = "banshee.service";

/// systemd's name for the tray's user unit.
const TRAY_UNIT: &str = "banshee-tray.service";

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
    // SAFETY: getuid takes nothing, cannot fail, and uid_t is a 32-bit unsigned
    // integer on macOS and Linux, the two platforms this builds for.
    unsafe { getuid() }
}

/// Everything Banshee keeps on this machine lives under one directory.
pub fn banshee_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".banshee"))
}

pub fn socket_path() -> Option<PathBuf> {
    Some(banshee_dir()?.join("banshee.sock"))
}

pub fn models_path() -> Option<PathBuf> {
    Some(banshee_dir()?.join("models"))
}

pub fn config_path() -> Option<PathBuf> {
    Some(banshee_dir()?.join("config.toml"))
}

pub fn credentials_path() -> Option<PathBuf> {
    Some(banshee_dir()?.join("credentials.toml"))
}

pub fn db_path() -> Option<PathBuf> {
    Some(banshee_dir()?.join("banshee.db"))
}

pub fn oov_log_path() -> Option<PathBuf> {
    Some(banshee_dir()?.join("oov-words.log"))
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
        Self::subscribe(serde_json::json!({ "events": events })).await
    }

    /// As `open`, and the daemon counts this connection as a chip that draws
    /// the cues, which silences the earcons `visual` gives to the screen.
    pub async fn open_to_draw(events: &[&str]) -> Result<(Value, Self), BansheeError> {
        Self::subscribe(serde_json::json!({ "events": events, "draws": true })).await
    }

    async fn subscribe(params: Value) -> Result<(Value, Self), BansheeError> {
        let (state, lines) = call(BANSHEE_SUBSCRIBE, params).await?;
        Ok((state, Subscription { lines }))
    }

    pub async fn next(&mut self) -> Result<Option<JsonRpcNotification>, BansheeError> {
        let Some(line) = self.lines.next_line().await? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_str(&line)?))
    }

    /// The next notification of one method, skipping the kinds this caller did
    /// not ask about. `None` once the daemon closes the connection.
    pub async fn next_of(&mut self, method: &str) -> Result<Option<Value>, BansheeError> {
        loop {
            let Some(pushed) = self.next().await? else {
                return Ok(None);
            };
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
    let socket_path = socket_path()
        .ok_or_else(|| BansheeError::Other("Could not find home directory".to_string()))?;

    let request = JsonRpcRequest {
        jsonrpc: crate::Version::V2,
        method: method.to_string(),
        params: Some(params),
        id: Some(serde_json::json!(1)),
    };

    let mut request_string = serde_json::to_string(&request)?;

    let mut stream = UnixStream::connect(socket_path).await?;

    request_string.push('\n');
    stream.write_all(request_string.as_bytes()).await?;

    let mut lines = BufReader::new(stream).lines();
    let response = lines.next_line().await?.unwrap_or_default();
    Ok((decode(&response)?, lines))
}

/// One line off the socket, read as a reply. Nothing at all is the daemon
/// closing without answering, which a decoder reports as a parse failure at
/// line 1 column 0 and no reader can act on.
fn decode(response: &str) -> Result<Value, BansheeError> {
    if response.trim().is_empty() {
        return Err(BansheeError::NoAnswer);
    }
    match serde_json::from_str::<JsonRpcResponse>(response)? {
        JsonRpcResponse::Success { result, .. } => Ok(result),
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
    fn a_connection_that_closes_without_a_reply_is_not_a_parse_failure() {
        for nothing in ["", "   ", "\t"] {
            assert!(
                matches!(decode(nothing), Err(BansheeError::NoAnswer)),
                "{nothing:?} is a daemon that went away"
            );
        }
    }

    #[test]
    fn a_reply_that_is_not_json_is_still_a_parse_failure() {
        assert!(matches!(decode("{not json"), Err(BansheeError::Serde(_))));
    }

    #[test]
    fn a_reply_carries_its_result_and_an_error_carries_its_code() {
        let ok = decode(r#"{"jsonrpc":"2.0","result":{"running":true},"id":1}"#).expect("a result");
        assert_eq!(ok["running"], true);

        let refused =
            decode(r#"{"jsonrpc":"2.0","error":{"code":-32004,"message":"busy"},"id":1}"#);
        assert!(matches!(
            refused,
            Err(BansheeError::Rpc { code: -32004, .. })
        ));
    }

    #[test]
    fn the_daemon_label_names_its_own_unit() {
        assert_eq!(systemd_unit(DAEMON_AGENT), Some(DAEMON_UNIT));
    }

    #[test]
    fn the_tray_label_names_its_own_unit() {
        assert_eq!(systemd_unit(TRAY_AGENT), Some("banshee-tray.service"));
    }

    struct TempDir(PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_failed_write_leaves_no_staging_file_so_the_next_write_succeeds() {
        let dir = TempDir(std::env::temp_dir().join(format!(
            "banshee-common-test-{}-{}",
            std::process::id(),
            "a_failed_write_leaves_no_staging_file_so_the_next_write_succeeds"
        )));
        std::fs::create_dir_all(&dir.0).unwrap();
        let target = dir.0.join("config.toml");

        // A rename onto a non-empty directory fails on macOS and Linux.
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("inner"), b"x").unwrap();

        assert!(write_atomically(&target, b"first", Some(0o600)).is_err());

        std::fs::remove_dir_all(&target).unwrap();

        let result = write_atomically(&target, b"second", Some(0o600));
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(std::fs::read(&target).unwrap(), b"second");

        let extension = target.extension().unwrap_or_default().to_string_lossy();
        let staged = target.with_extension(format!("{extension}.{}", std::process::id()));
        assert!(!staged.exists());
    }

    #[test]
    fn a_write_through_a_symlink_changes_the_target_and_keeps_the_link() {
        let dir = TempDir(std::env::temp_dir().join(format!(
            "banshee-common-test-{}-{}",
            std::process::id(),
            "a_write_through_a_symlink_changes_the_target_and_keeps_the_link"
        )));
        std::fs::create_dir_all(&dir.0).unwrap();
        let target = dir.0.join("dotfiles-config.toml");
        std::fs::write(&target, "old").unwrap();
        let link = dir.0.join("config.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        write_atomically(&link, b"new", None).unwrap();

        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    }

    #[test]
    fn a_write_through_a_dangling_symlink_names_the_link_and_its_missing_target() {
        let dir = TempDir(std::env::temp_dir().join(format!(
            "banshee-common-test-{}-{}",
            std::process::id(),
            "a_write_through_a_dangling_symlink_names_the_link_and_its_missing_target"
        )));
        std::fs::create_dir_all(&dir.0).unwrap();
        let target = dir.0.join("gone").join("dotfiles-config.toml");
        let link = dir.0.join("config.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let refusal = write_atomically(&link, b"new", None)
            .unwrap_err()
            .to_string();

        assert!(refusal.contains(&*link.to_string_lossy()), "{refusal}");
        assert!(refusal.contains(&*target.to_string_lossy()), "{refusal}");
        assert!(!target.exists(), "the write fails and makes no target");
    }
}

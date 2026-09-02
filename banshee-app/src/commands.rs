use crate::calls::{self, CommandError, Devices, Voices};
use crate::socket::Client;
use banshee_common::{AgentRow, PlannedChange, utils};
use std::path::{Path, PathBuf};
use tauri::State;
use tokio::sync::Mutex;

pub const NO_HOME_DIR: &str = "Banshee cannot find your home directory.";

/// The path resolves once, so a missing home directory cannot stop the window opening.
pub struct Daemon {
    path: Option<PathBuf>,
    client: Mutex<Option<Client>>,
}

impl Daemon {
    pub fn new() -> Self {
        Daemon {
            path: utils::get_socket_path(),
            client: Mutex::new(None),
        }
    }

    pub fn socket_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn path_or_error(&self) -> Result<&Path, CommandError> {
        self.path.as_deref().ok_or_else(|| CommandError {
            code: -32000,
            message: NO_HOME_DIR.to_string(),
            transport: false,
            sent: false,
        })
    }
}

impl Default for Daemon {
    fn default() -> Self {
        Self::new()
    }
}

/// True only for a connection that died before the request left the client.
/// A request the daemon received may already have run, so a replay could
/// speak a preview twice or rewrite an agent's config twice.
fn is_safe_to_retry(error: &CommandError) -> bool {
    error.transport && !error.sent
}

/// The one function that opens a connection; first use and a dead connection's retry both run it.
pub async fn force_reconnect(slot: &mut Option<Client>, path: &Path) -> Result<(), CommandError> {
    let client = Client::connect(path).await.map_err(|error| CommandError {
        code: -32000,
        message: error.to_string(),
        transport: true,
        sent: false,
    })?;
    *slot = Some(client);
    Ok(())
}

/// An empty slot, from a window opened before the daemon or after one died, repairs itself the same
/// way.
pub async fn ensure_connected(slot: &mut Option<Client>, path: &Path) -> Result<(), CommandError> {
    if slot.is_none() {
        force_reconnect(slot, path).await?;
    }
    Ok(())
}

/// Every transport failure leaves the framing unknown, so the connection is dropped; only a request
/// that never reached the daemon is sent again. A macro because an AsyncFn closure that borrows its
/// arguments hits a known rustc limitation.
macro_rules! retrying {
    ($daemon:expr, $client:expr, $body:expr) => {{
        let path = $daemon.path_or_error()?;
        ensure_connected(&mut $client, path).await?;
        match $body {
            Err(error) if error.transport => {
                *$client = None;
                if is_safe_to_retry(&error) {
                    ensure_connected(&mut $client, path).await?;
                    $body
                } else {
                    Err(error)
                }
            }
            other => other,
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::{Daemon, is_safe_to_retry};
    use crate::calls::CommandError;
    use crate::socket::{RpcError, SOCKET_CLOSED};
    use std::path::PathBuf;
    use tokio::sync::Mutex;

    fn daemon(path: Option<&str>) -> Daemon {
        Daemon {
            path: path.map(PathBuf::from),
            client: Mutex::new(None),
        }
    }

    #[test]
    fn a_resolved_socket_path_reaches_every_command() {
        let daemon = daemon(Some("/home/someone/.banshee/banshee.sock"));
        assert_eq!(
            daemon.path_or_error().unwrap(),
            PathBuf::from("/home/someone/.banshee/banshee.sock")
        );
        assert!(daemon.socket_path().is_some());
    }

    #[test]
    fn no_home_directory_is_a_sentence_a_command_returns_not_a_panic() {
        let daemon = daemon(None);
        let error = daemon.path_or_error().unwrap_err();
        assert!(!error.message.is_empty());
        // There is nothing to reconnect to, so the retry must not run.
        assert!(!error.transport);
        assert!(daemon.socket_path().is_none());
    }

    fn from_socket(code: i32, message: &str, transport: bool, sent: bool) -> CommandError {
        RpcError {
            code,
            message: message.to_string(),
            transport,
            sent,
        }
        .into()
    }

    #[test]
    fn a_request_that_never_left_the_client_is_safe_to_send_again() {
        // The write side fails first on a restart, and the operating system
        // names that one itself.
        assert!(is_safe_to_retry(&from_socket(
            -32000,
            "Broken pipe (os error 32)",
            true,
            false
        )));
    }

    #[test]
    fn a_request_the_daemon_may_have_run_is_never_replayed() {
        // An EOF while waiting for the reply: the daemon held the request and
        // may have acted on it before it died.
        assert!(!is_safe_to_retry(&from_socket(
            -32000,
            SOCKET_CLOSED,
            true,
            true
        )));
    }

    #[test]
    fn a_refusal_the_daemon_wrote_is_never_a_dead_connection() {
        assert!(!is_safe_to_retry(&from_socket(
            -32602,
            "Disconnect is not available yet.",
            false,
            true
        )));
        // The daemon writes `-32000` for its own refusals too, so the code
        // alone must not decide this.
        assert!(!is_safe_to_retry(&from_socket(
            -32000,
            "Microphone unavailable.",
            false,
            true
        )));
    }
}

#[tauri::command]
pub async fn status(daemon: State<'_, Daemon>) -> Result<serde_json::Value, CommandError> {
    let mut client = daemon.client.lock().await;
    retrying!(
        daemon,
        client,
        calls::status(client.as_mut().unwrap()).await
    )
}

#[tauri::command]
pub async fn set_setting(
    daemon: State<'_, Daemon>,
    key: String,
    value: serde_json::Value,
) -> Result<Vec<String>, CommandError> {
    let mut client = daemon.client.lock().await;
    retrying!(
        daemon,
        client,
        calls::set_setting(client.as_mut().unwrap(), &key, value.clone()).await
    )
}

#[tauri::command]
pub async fn list_devices(daemon: State<'_, Daemon>) -> Result<Devices, CommandError> {
    let mut client = daemon.client.lock().await;
    retrying!(
        daemon,
        client,
        calls::list_devices(client.as_mut().unwrap()).await
    )
}

#[tauri::command]
pub async fn list_voices(daemon: State<'_, Daemon>) -> Result<Voices, CommandError> {
    let mut client = daemon.client.lock().await;
    retrying!(
        daemon,
        client,
        calls::list_voices(client.as_mut().unwrap()).await
    )
}

#[tauri::command]
pub async fn list_languages(daemon: State<'_, Daemon>) -> Result<calls::Languages, CommandError> {
    let mut client = daemon.client.lock().await;
    retrying!(
        daemon,
        client,
        calls::list_languages(client.as_mut().unwrap()).await
    )
}

#[tauri::command]
pub async fn preview_voice(daemon: State<'_, Daemon>, id: String) -> Result<(), CommandError> {
    let mut client = daemon.client.lock().await;
    retrying!(
        daemon,
        client,
        calls::preview_voice(client.as_mut().unwrap(), &id).await
    )
}

#[tauri::command]
pub async fn download_models(daemon: State<'_, Daemon>) -> Result<(), CommandError> {
    let mut client = daemon.client.lock().await;
    retrying!(
        daemon,
        client,
        calls::download_models(client.as_mut().unwrap()).await
    )
}

#[tauri::command]
pub async fn detect_agents(daemon: State<'_, Daemon>) -> Result<Vec<AgentRow>, CommandError> {
    let mut client = daemon.client.lock().await;
    retrying!(
        daemon,
        client,
        calls::detect_agents(client.as_mut().unwrap()).await
    )
}

#[tauri::command]
pub async fn plan_connect(
    daemon: State<'_, Daemon>,
    id: String,
    disconnect: bool,
) -> Result<Vec<PlannedChange>, CommandError> {
    let mut client = daemon.client.lock().await;
    retrying!(
        daemon,
        client,
        calls::plan_connect(client.as_mut().unwrap(), &id, disconnect).await
    )
}

#[tauri::command]
pub async fn apply_connect(
    daemon: State<'_, Daemon>,
    id: String,
    disconnect: bool,
) -> Result<(), CommandError> {
    let mut client = daemon.client.lock().await;
    retrying!(
        daemon,
        client,
        calls::apply_connect(client.as_mut().unwrap(), &id, disconnect).await
    )
}

#[tauri::command]
pub async fn history(
    daemon: State<'_, Daemon>,
    limit: Option<u32>,
) -> Result<Vec<serde_json::Value>, CommandError> {
    let mut client = daemon.client.lock().await;
    retrying!(
        daemon,
        client,
        calls::history(client.as_mut().unwrap(), limit).await
    )
}

#[tauri::command]
pub async fn clear_history(daemon: State<'_, Daemon>) -> Result<(), CommandError> {
    let mut client = daemon.client.lock().await;
    retrying!(
        daemon,
        client,
        calls::clear_history(client.as_mut().unwrap()).await
    )
}

#[tauri::command]
pub async fn open_permission_pane(
    daemon: State<'_, Daemon>,
    id: String,
) -> Result<(), CommandError> {
    let mut client = daemon.client.lock().await;
    retrying!(
        daemon,
        client,
        calls::open_permission_pane(client.as_mut().unwrap(), &id).await
    )
}

#[tauri::command]
pub async fn copy_text(app: tauri::AppHandle, text: String) -> Result<(), CommandError> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard()
        .write_text(text)
        .map_err(|error| CommandError {
            code: -32603,
            message: error.to_string(),
            transport: false,
            sent: true,
        })
}

fn failed(message: String) -> CommandError {
    CommandError {
        code: -32603,
        message,
        transport: false,
        sent: false,
    }
}

/// The socket exists only while the daemon runs, so starting one cannot go over it.
pub fn run_cli(subcommand: &str) -> Result<(), CommandError> {
    let status = utils::sibling_command("banshee")
        .map_err(|error| failed(error.to_string()))?
        .arg(subcommand)
        .status()
        .map_err(|error| failed(error.to_string()))?;
    if status.success() {
        return Ok(());
    }
    Err(failed(format!("banshee {subcommand} did not finish")))
}

/// Starts one login job. Without `replace`, kickstart leaves a job that already
/// runs alone, so a daemon part-way through loading its models survives it;
/// with it, the running job is torn down and started again, which is the only
/// thing that clears a pipeline that died at startup.
///
/// It fails when the job was never bootstrapped, and the subcommand that
/// installs it runs only then, because installing tears a running job down.
fn kickstart(label: &str, install: &str, replace: bool) -> Result<(), CommandError> {
    let target = utils::launchd_target(label);
    let mut args = vec!["kickstart"];
    if replace {
        args.push("-k");
    }
    args.push(&target);
    let started = std::process::Command::new("launchctl")
        .args(&args)
        .status()
        .map_err(|error| failed(error.to_string()))?;
    if started.success() {
        return Ok(());
    }
    run_cli(install)
}

/// Puts the menu bar icon up. Not a second copy of the binary, which the
/// icon's own lock refuses while launchd keeps retrying it.
pub fn open_the_tray() -> Result<(), CommandError> {
    kickstart(utils::TRAY_AGENT, "tray", false)
}

// `banshee start` waits on launchd, so running it on a worker thread would
// hold that thread and stall every other command the window sends.
#[tauri::command]
pub async fn start_daemon() -> Result<(), CommandError> {
    tauri::async_runtime::spawn_blocking(|| kickstart(utils::DAEMON_AGENT, "start", false))
        .await
        .map_err(|error| failed(error.to_string()))?
}

/// A setting the daemon reads once, and a pipeline dead at startup, both need the process renewed.
#[tauri::command]
pub async fn restart_daemon() -> Result<(), CommandError> {
    tauri::async_runtime::spawn_blocking(|| kickstart(utils::DAEMON_AGENT, "start", true))
        .await
        .map_err(|error| failed(error.to_string()))?
}

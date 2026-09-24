use crate::calls::{self, CommandError, Devices, Voices};
use crate::socket::Client;
use banshee_common::{AgentRow, PlannedChange, rpc_code, utils};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
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
            path: utils::socket_path(),
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

    /// One call to the daemon, over a connection held for the call. A dead
    /// connection is repaired once.
    async fn call<T>(
        &self,
        body: impl for<'client> FnMut(&'client mut Client) -> Attempt<'client, T>,
    ) -> Result<T, CommandError> {
        let path = self.path_or_error()?;
        let mut client = self.client.lock().await;
        retrying(path, &mut client, body).await
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
/// Answers the client now in the slot.
pub async fn force_reconnect<'slot>(
    slot: &'slot mut Option<Client>,
    path: &Path,
) -> Result<&'slot mut Client, CommandError> {
    let client = Client::connect(path).await.map_err(|error| CommandError {
        code: -32000,
        message: error.to_string(),
        transport: true,
        sent: false,
    })?;
    Ok(slot.insert(client))
}

/// The client in the slot. An empty slot, from a window opened before the daemon or after one
/// died, repairs itself the same way.
pub async fn ensure_connected<'slot>(
    slot: &'slot mut Option<Client>,
    path: &Path,
) -> Result<&'slot mut Client, CommandError> {
    match slot {
        Some(client) => Ok(client),
        None => force_reconnect(slot, path).await,
    }
}

/// One attempt at a call, borrowing the client for as long as the call runs. Boxed, because an
/// async closure that borrows its argument cannot be proved `Send` for every lifetime inside a
/// Tauri command.
pub type Attempt<'client, T> =
    Pin<Box<dyn Future<Output = Result<T, CommandError>> + Send + 'client>>;

/// Every transport failure leaves the framing unknown, so the connection is dropped; only a request
/// that never reached the daemon is sent again.
pub async fn retrying<T>(
    path: &Path,
    slot: &mut Option<Client>,
    mut body: impl for<'client> FnMut(&'client mut Client) -> Attempt<'client, T>,
) -> Result<T, CommandError> {
    let client = ensure_connected(slot, path).await?;
    match body(client).await {
        Err(error) if error.transport => {
            *slot = None;
            if !is_safe_to_retry(&error) {
                return Err(error);
            }
            body(force_reconnect(slot, path).await?).await
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{Daemon, is_safe_to_retry};
    use crate::calls::CommandError;
    use crate::socket::{RpcError, SOCKET_CLOSED};
    use banshee_common::rpc_code;
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
            rpc_code::INVALID_PARAMS,
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

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn starting_the_daemon_leaves_a_running_unit_alone() {
        assert_eq!(
            super::systemctl_args(banshee_common::utils::DAEMON_AGENT, false),
            Some(vec![
                "--user".to_string(),
                "start".to_string(),
                "banshee.service".to_string(),
            ])
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn restarting_the_daemon_tears_the_unit_down_first() {
        assert_eq!(
            super::systemctl_args(banshee_common::utils::DAEMON_AGENT, true),
            Some(vec![
                "--user".to_string(),
                "restart".to_string(),
                "banshee.service".to_string(),
            ])
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn starting_the_tray_names_its_unit() {
        assert_eq!(
            super::systemctl_args(banshee_common::utils::TRAY_AGENT, false),
            Some(vec![
                "--user".to_string(),
                "start".to_string(),
                "banshee-tray.service".to_string(),
            ])
        );
    }
}

#[tauri::command]
pub async fn status(daemon: State<'_, Daemon>) -> Result<serde_json::Value, CommandError> {
    daemon.call(|client| Box::pin(calls::status(client))).await
}

#[tauri::command]
pub async fn set_setting(
    daemon: State<'_, Daemon>,
    key: String,
    value: serde_json::Value,
) -> Result<Vec<String>, CommandError> {
    daemon
        .call(|client| {
            let key = key.clone();
            let value = value.clone();
            Box::pin(async move { calls::set_setting(client, &key, value).await })
        })
        .await
}

#[tauri::command]
pub async fn list_devices(daemon: State<'_, Daemon>) -> Result<Devices, CommandError> {
    daemon
        .call(|client| Box::pin(calls::list_devices(client)))
        .await
}

#[tauri::command]
pub async fn list_voices(daemon: State<'_, Daemon>) -> Result<Voices, CommandError> {
    daemon
        .call(|client| Box::pin(calls::list_voices(client)))
        .await
}

#[tauri::command]
pub async fn list_languages(daemon: State<'_, Daemon>) -> Result<calls::Languages, CommandError> {
    daemon
        .call(|client| Box::pin(calls::list_languages(client)))
        .await
}

#[tauri::command]
pub async fn preview_voice(daemon: State<'_, Daemon>, id: String) -> Result<(), CommandError> {
    daemon
        .call(|client| {
            let id = id.clone();
            Box::pin(async move { calls::preview_voice(client, &id).await })
        })
        .await
}

#[tauri::command]
pub async fn download_models(daemon: State<'_, Daemon>) -> Result<(), CommandError> {
    daemon
        .call(|client| Box::pin(calls::download_models(client)))
        .await
}

#[tauri::command]
pub async fn detect_agents(daemon: State<'_, Daemon>) -> Result<Vec<AgentRow>, CommandError> {
    daemon
        .call(|client| Box::pin(calls::detect_agents(client)))
        .await
}

#[tauri::command]
pub async fn plan_connect(
    daemon: State<'_, Daemon>,
    id: String,
    disconnect: bool,
) -> Result<Vec<PlannedChange>, CommandError> {
    daemon
        .call(|client| {
            let id = id.clone();
            Box::pin(async move { calls::plan_connect(client, &id, disconnect).await })
        })
        .await
}

#[tauri::command]
pub async fn apply_connect(
    daemon: State<'_, Daemon>,
    id: String,
    disconnect: bool,
) -> Result<Option<String>, CommandError> {
    daemon
        .call(|client| {
            let id = id.clone();
            Box::pin(async move { calls::apply_connect(client, &id, disconnect).await })
        })
        .await
}

#[tauri::command]
pub async fn history(
    daemon: State<'_, Daemon>,
    limit: Option<u32>,
) -> Result<Vec<serde_json::Value>, CommandError> {
    daemon
        .call(|client| Box::pin(calls::history(client, limit)))
        .await
}

#[tauri::command]
pub async fn clear_history(daemon: State<'_, Daemon>) -> Result<(), CommandError> {
    daemon
        .call(|client| Box::pin(calls::clear_history(client)))
        .await
}

#[tauri::command]
pub async fn open_permission_pane(
    daemon: State<'_, Daemon>,
    id: String,
) -> Result<(), CommandError> {
    daemon
        .call(|client| {
            let id = id.clone();
            Box::pin(async move { calls::open_permission_pane(client, &id).await })
        })
        .await
}

#[tauri::command]
pub async fn copy_text(app: tauri::AppHandle, text: String) -> Result<(), CommandError> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard()
        .write_text(text)
        .map_err(|error| CommandError {
            code: rpc_code::INTERNAL,
            message: error.to_string(),
            transport: false,
            sent: true,
        })
}

fn failed(message: String) -> CommandError {
    CommandError {
        code: rpc_code::INTERNAL,
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
#[cfg(target_os = "macos")]
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

/// The systemd arm. `start` leaves a running unit alone and `restart` tears it
/// down, which is what `replace` means on launchd. Any failure falls through to
/// the CLI: the label may name a unit nobody wrote yet, such as a fresh
/// install where `banshee start` has not run.
#[cfg(not(target_os = "macos"))]
fn kickstart(label: &str, install: &str, replace: bool) -> Result<(), CommandError> {
    if let Some(args) = systemctl_args(label, replace) {
        let started = std::process::Command::new("systemctl").args(&args).status();
        if matches!(&started, Ok(status) if status.success()) {
            return Ok(());
        }
    }
    run_cli(install)
}

/// Split from the call, so a test can read the argv without starting a unit.
#[cfg(not(target_os = "macos"))]
fn systemctl_args(label: &str, replace: bool) -> Option<Vec<String>> {
    let unit = utils::systemd_unit(label)?;
    let verb = if replace { "restart" } else { "start" };
    Some(vec![
        "--user".to_string(),
        verb.to_string(),
        unit.to_string(),
    ])
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

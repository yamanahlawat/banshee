//! The `#[tauri::command]` wrappers TypeScript calls by name. Each locks the
//! held connection and calls the matching `calls` function.

use crate::calls::{self, CommandError, Devices, Voices};
use crate::socket::{Client, SOCKET_CLOSED};
use banshee_common::{AgentRow, PlannedChange, utils};
use std::path::Path;
use tauri::State;
use tokio::sync::Mutex;

/// True only for the error `Client::call` produces when the daemon side of
/// the connection is gone (an EOF, most often a restart): both the code and
/// the message it always sets together must match, not just one. Every other
/// error, including a live refusal the daemon wrote itself, is left alone,
/// so a real refusal is never mistaken for a dead connection.
fn is_dead_connection(error: &CommandError) -> bool {
    error.code == -32000 && error.message == SOCKET_CLOSED
}

/// Connects fresh through `path`, replacing whatever was in `slot`. The one
/// function that opens a connection, used both the first time a command
/// needs one and every time a later one has died.
pub async fn force_reconnect(slot: &mut Option<Client>, path: &Path) -> Result<(), CommandError> {
    let client = Client::connect(path).await.map_err(|error| CommandError {
        code: -32000,
        message: error.to_string(),
    })?;
    *slot = Some(client);
    Ok(())
}

/// Connects only when `slot` is empty. A window opened before the daemon, or
/// reopened after one, reaches its first command with an empty slot; this
/// runs the same `force_reconnect` a dead connection's retry runs, so
/// "never connected" and "connected then died" repair themselves one way.
pub async fn ensure_connected(slot: &mut Option<Client>, path: &Path) -> Result<(), CommandError> {
    if slot.is_none() {
        force_reconnect(slot, path).await?;
    }
    Ok(())
}

/// Ensures `$client` holds a connection, runs `$body`, and on a dead
/// connection reconnects once and runs `$body` again. Any other error,
/// success included, returns from its try untouched. A generic function
/// here hits a known rustc limitation with `AsyncFn` closures that borrow
/// their arguments, so this is a macro.
macro_rules! retrying {
    ($client:expr, $path:expr, $body:expr) => {{
        ensure_connected(&mut $client, $path).await?;
        match $body {
            Err(error) if is_dead_connection(&error) => {
                force_reconnect(&mut $client, $path).await?;
                $body
            }
            other => other,
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::is_dead_connection;
    use crate::calls::CommandError;
    use crate::socket::SOCKET_CLOSED;

    fn error(code: i32, message: &str) -> CommandError {
        CommandError {
            code,
            message: message.to_string(),
        }
    }

    #[test]
    fn only_the_closed_socket_code_and_message_together_read_as_dead() {
        assert!(is_dead_connection(&error(-32000, SOCKET_CLOSED)));
        assert!(!is_dead_connection(&error(
            -32602,
            "Disconnect is not available yet."
        )));
        // The daemon's own -32000 (a microphone error) must not be mistaken
        // for a transport failure just because the code matches.
        assert!(!is_dead_connection(&error(
            -32000,
            "Microphone unavailable."
        )));
        // A message that happens to match with the wrong code is not enough.
        assert!(!is_dead_connection(&error(-32603, SOCKET_CLOSED)));
    }
}

#[tauri::command]
pub async fn status(
    client: State<'_, Mutex<Option<Client>>>,
) -> Result<serde_json::Value, CommandError> {
    let mut client = client.lock().await;
    let path = utils::get_socket_path().expect("a home directory");
    retrying!(client, &path, calls::status(client.as_mut().unwrap()).await)
}

#[tauri::command]
pub async fn set_setting(
    client: State<'_, Mutex<Option<Client>>>,
    key: String,
    value: serde_json::Value,
) -> Result<Vec<String>, CommandError> {
    let mut client = client.lock().await;
    let path = utils::get_socket_path().expect("a home directory");
    retrying!(
        client,
        &path,
        calls::set_setting(client.as_mut().unwrap(), &key, value.clone()).await
    )
}

#[tauri::command]
pub async fn list_devices(
    client: State<'_, Mutex<Option<Client>>>,
) -> Result<Devices, CommandError> {
    let mut client = client.lock().await;
    let path = utils::get_socket_path().expect("a home directory");
    retrying!(
        client,
        &path,
        calls::list_devices(client.as_mut().unwrap()).await
    )
}

#[tauri::command]
pub async fn list_voices(client: State<'_, Mutex<Option<Client>>>) -> Result<Voices, CommandError> {
    let mut client = client.lock().await;
    let path = utils::get_socket_path().expect("a home directory");
    retrying!(
        client,
        &path,
        calls::list_voices(client.as_mut().unwrap()).await
    )
}

#[tauri::command]
pub async fn preview_voice(
    client: State<'_, Mutex<Option<Client>>>,
    id: String,
) -> Result<(), CommandError> {
    let mut client = client.lock().await;
    let path = utils::get_socket_path().expect("a home directory");
    retrying!(
        client,
        &path,
        calls::preview_voice(client.as_mut().unwrap(), &id).await
    )
}

#[tauri::command]
pub async fn download_models(client: State<'_, Mutex<Option<Client>>>) -> Result<(), CommandError> {
    let mut client = client.lock().await;
    let path = utils::get_socket_path().expect("a home directory");
    retrying!(
        client,
        &path,
        calls::download_models(client.as_mut().unwrap()).await
    )
}

#[tauri::command]
pub async fn detect_agents(
    client: State<'_, Mutex<Option<Client>>>,
) -> Result<Vec<AgentRow>, CommandError> {
    let mut client = client.lock().await;
    let path = utils::get_socket_path().expect("a home directory");
    retrying!(
        client,
        &path,
        calls::detect_agents(client.as_mut().unwrap()).await
    )
}

#[tauri::command]
pub async fn plan_connect(
    client: State<'_, Mutex<Option<Client>>>,
    id: String,
    disconnect: bool,
) -> Result<Vec<PlannedChange>, CommandError> {
    let mut client = client.lock().await;
    let path = utils::get_socket_path().expect("a home directory");
    retrying!(
        client,
        &path,
        calls::plan_connect(client.as_mut().unwrap(), &id, disconnect).await
    )
}

#[tauri::command]
pub async fn apply_connect(
    client: State<'_, Mutex<Option<Client>>>,
    id: String,
    disconnect: bool,
) -> Result<(), CommandError> {
    let mut client = client.lock().await;
    let path = utils::get_socket_path().expect("a home directory");
    retrying!(
        client,
        &path,
        calls::apply_connect(client.as_mut().unwrap(), &id, disconnect).await
    )
}

#[tauri::command]
pub async fn history(
    client: State<'_, Mutex<Option<Client>>>,
    limit: Option<u32>,
) -> Result<Vec<serde_json::Value>, CommandError> {
    let mut client = client.lock().await;
    let path = utils::get_socket_path().expect("a home directory");
    retrying!(
        client,
        &path,
        calls::history(client.as_mut().unwrap(), limit).await
    )
}

#[tauri::command]
pub async fn clear_history(client: State<'_, Mutex<Option<Client>>>) -> Result<(), CommandError> {
    let mut client = client.lock().await;
    let path = utils::get_socket_path().expect("a home directory");
    retrying!(
        client,
        &path,
        calls::clear_history(client.as_mut().unwrap()).await
    )
}

#[tauri::command]
pub async fn open_permission_pane(
    client: State<'_, Mutex<Option<Client>>>,
    id: String,
) -> Result<(), CommandError> {
    let mut client = client.lock().await;
    let path = utils::get_socket_path().expect("a home directory");
    retrying!(
        client,
        &path,
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
        })
}

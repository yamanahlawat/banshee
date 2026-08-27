//! The `#[tauri::command]` wrappers TypeScript calls by name. Each locks the
//! held connection and calls the matching `calls` function.

use crate::calls::{self, Devices, Voices};
use crate::socket::{Client, SOCKET_CLOSED};
use banshee_common::{AgentRow, PlannedChange, utils};
use tauri::State;
use tokio::sync::Mutex;

/// True only for the message `Client::call` reports when the daemon side of
/// the connection is gone (an EOF, most often a restart). Every other
/// message, including a live refusal the daemon wrote itself, is left alone,
/// so a real refusal is never mistaken for a dead connection.
fn is_dead_connection(message: &str) -> bool {
    message == SOCKET_CLOSED
}

/// Runs `$body` against `$client`; on a dead connection, reconnects once and
/// runs `$body` again. Any other error returns from the first try untouched.
/// A generic function here hits a known rustc limitation with `AsyncFn`
/// closures that borrow their arguments, so this is a macro.
macro_rules! retrying {
    ($client:expr, $body:expr) => {{
        match $body {
            Err(message) if is_dead_connection(&message) => {
                let path =
                    utils::get_socket_path().ok_or_else(|| "no home directory".to_string())?;
                *$client = Client::connect(&path)
                    .await
                    .map_err(|error| error.to_string())?;
                $body
            }
            other => other,
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::is_dead_connection;
    use crate::socket::SOCKET_CLOSED;

    #[test]
    fn only_the_closed_socket_message_reads_as_a_dead_connection() {
        assert!(is_dead_connection(SOCKET_CLOSED));
        assert!(!is_dead_connection("Disconnect is not available yet."));
        assert!(!is_dead_connection("'limit' must fit in 32 bits."));
    }
}

#[tauri::command]
pub async fn status(client: State<'_, Mutex<Client>>) -> Result<serde_json::Value, String> {
    let mut client = client.lock().await;
    retrying!(client, calls::status(&mut client).await)
}

#[tauri::command]
pub async fn set_setting(
    client: State<'_, Mutex<Client>>,
    key: String,
    value: serde_json::Value,
) -> Result<Vec<String>, String> {
    let mut client = client.lock().await;
    retrying!(
        client,
        calls::set_setting(&mut client, &key, value.clone()).await
    )
}

#[tauri::command]
pub async fn list_devices(client: State<'_, Mutex<Client>>) -> Result<Devices, String> {
    let mut client = client.lock().await;
    retrying!(client, calls::list_devices(&mut client).await)
}

#[tauri::command]
pub async fn list_voices(client: State<'_, Mutex<Client>>) -> Result<Voices, String> {
    let mut client = client.lock().await;
    retrying!(client, calls::list_voices(&mut client).await)
}

#[tauri::command]
pub async fn preview_voice(client: State<'_, Mutex<Client>>, id: String) -> Result<(), String> {
    let mut client = client.lock().await;
    retrying!(client, calls::preview_voice(&mut client, &id).await)
}

#[tauri::command]
pub async fn download_models(client: State<'_, Mutex<Client>>) -> Result<(), String> {
    let mut client = client.lock().await;
    retrying!(client, calls::download_models(&mut client).await)
}

#[tauri::command]
pub async fn detect_agents(client: State<'_, Mutex<Client>>) -> Result<Vec<AgentRow>, String> {
    let mut client = client.lock().await;
    retrying!(client, calls::detect_agents(&mut client).await)
}

#[tauri::command]
pub async fn plan_connect(
    client: State<'_, Mutex<Client>>,
    id: String,
    disconnect: bool,
) -> Result<Vec<PlannedChange>, String> {
    let mut client = client.lock().await;
    retrying!(
        client,
        calls::plan_connect(&mut client, &id, disconnect).await
    )
}

#[tauri::command]
pub async fn apply_connect(
    client: State<'_, Mutex<Client>>,
    id: String,
    disconnect: bool,
) -> Result<(), String> {
    let mut client = client.lock().await;
    retrying!(
        client,
        calls::apply_connect(&mut client, &id, disconnect).await
    )
}

#[tauri::command]
pub async fn history(
    client: State<'_, Mutex<Client>>,
    limit: Option<u32>,
) -> Result<Vec<serde_json::Value>, String> {
    let mut client = client.lock().await;
    retrying!(client, calls::history(&mut client, limit).await)
}

#[tauri::command]
pub async fn clear_history(client: State<'_, Mutex<Client>>) -> Result<(), String> {
    let mut client = client.lock().await;
    retrying!(client, calls::clear_history(&mut client).await)
}

#[tauri::command]
pub async fn open_permission_pane(
    client: State<'_, Mutex<Client>>,
    id: String,
) -> Result<(), String> {
    let mut client = client.lock().await;
    retrying!(client, calls::open_permission_pane(&mut client, &id).await)
}

#[tauri::command]
pub async fn copy_text(app: tauri::AppHandle, text: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard()
        .write_text(text)
        .map_err(|error| error.to_string())
}

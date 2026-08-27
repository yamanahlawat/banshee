//! One function per job the window does. Each shapes one RPC call and maps
//! `RpcError` to the daemon's own message, so a caller sees exactly what the
//! daemon said.

use crate::socket::Client;
use banshee_common::{
    AgentRow, BANSHEE_AGENTS, BANSHEE_CLEAR_HISTORY, BANSHEE_CONFIGURE, BANSHEE_CONNECT_APPLY,
    BANSHEE_CONNECT_PLAN, BANSHEE_DOWNLOAD_MODELS, BANSHEE_HISTORY, BANSHEE_LIST_INPUT_DEVICES,
    BANSHEE_LIST_VOICES, BANSHEE_OPEN_PERMISSION, BANSHEE_SPEAK, BANSHEE_STATUS, InputDevice,
    PlannedChange, Voice,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const PREVIEW_SENTENCE: &str = "This is how I sound.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Devices {
    pub devices: Vec<InputDevice>,
    pub current: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Voices {
    pub voices: Vec<Voice>,
    pub current: Option<String>,
}

pub async fn status(client: &mut Client) -> Result<Value, String> {
    client
        .call(BANSHEE_STATUS, json!({}))
        .await
        .map_err(|error| error.message)
}

pub async fn set_setting(
    client: &mut Client,
    key: &str,
    value: Value,
) -> Result<Vec<String>, String> {
    let result = client
        .call(
            BANSHEE_CONFIGURE,
            json!({
                "settings": { key: value },
                "persist": true,
            }),
        )
        .await
        .map_err(|error| error.message)?;
    Ok(serde_json::from_value(result["restart_required"].clone()).unwrap_or_default())
}

pub async fn list_devices(client: &mut Client) -> Result<Devices, String> {
    let result = client
        .call(BANSHEE_LIST_INPUT_DEVICES, json!({}))
        .await
        .map_err(|error| error.message)?;
    serde_json::from_value(result).map_err(|error| error.to_string())
}

pub async fn list_voices(client: &mut Client) -> Result<Voices, String> {
    let result = client
        .call(BANSHEE_LIST_VOICES, json!({}))
        .await
        .map_err(|error| error.message)?;
    serde_json::from_value(result).map_err(|error| error.to_string())
}

pub async fn preview_voice(client: &mut Client, id: &str) -> Result<(), String> {
    client
        .call(
            BANSHEE_SPEAK,
            json!({"text": PREVIEW_SENTENCE, "voice": id}),
        )
        .await
        .map_err(|error| error.message)?;
    Ok(())
}

pub async fn download_models(client: &mut Client) -> Result<(), String> {
    client
        .call(BANSHEE_DOWNLOAD_MODELS, json!({}))
        .await
        .map_err(|error| error.message)?;
    Ok(())
}

pub async fn detect_agents(client: &mut Client) -> Result<Vec<AgentRow>, String> {
    let result = client
        .call(BANSHEE_AGENTS, json!({}))
        .await
        .map_err(|error| error.message)?;
    Ok(serde_json::from_value(result["agents"].clone()).unwrap_or_default())
}

pub async fn plan_connect(
    client: &mut Client,
    id: &str,
    disconnect: bool,
) -> Result<Vec<PlannedChange>, String> {
    let result = client
        .call(
            BANSHEE_CONNECT_PLAN,
            json!({"agent": id, "disconnect": disconnect}),
        )
        .await
        .map_err(|error| error.message)?;
    Ok(serde_json::from_value(result["changes"].clone()).unwrap_or_default())
}

pub async fn apply_connect(client: &mut Client, id: &str, disconnect: bool) -> Result<(), String> {
    client
        .call(
            BANSHEE_CONNECT_APPLY,
            json!({"agent": id, "disconnect": disconnect}),
        )
        .await
        .map_err(|error| error.message)?;
    Ok(())
}

/// `limit` is forwarded when present and omitted entirely when absent: the
/// daemon reads an absent `limit` as every row, and an explicit `0` as none.
pub async fn history(client: &mut Client, limit: Option<u32>) -> Result<Vec<Value>, String> {
    let mut params = serde_json::Map::new();
    if let Some(limit) = limit {
        params.insert("limit".to_string(), json!(limit));
    }
    let result = client
        .call(BANSHEE_HISTORY, Value::Object(params))
        .await
        .map_err(|error| error.message)?;
    Ok(serde_json::from_value(result["history"].clone()).unwrap_or_default())
}

pub async fn clear_history(client: &mut Client) -> Result<(), String> {
    client
        .call(BANSHEE_CLEAR_HISTORY, json!({}))
        .await
        .map_err(|error| error.message)?;
    Ok(())
}

pub async fn open_permission_pane(client: &mut Client, id: &str) -> Result<(), String> {
    client
        .call(BANSHEE_OPEN_PERMISSION, json!({"id": id}))
        .await
        .map_err(|error| error.message)?;
    Ok(())
}

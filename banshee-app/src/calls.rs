//! A daemon's error reaches the caller with its own code and message; a failure before the
//! daemon answered keeps the transport's own.

use crate::socket::{Client, RpcError};
use banshee_common::{
    AgentRow, BANSHEE_AGENTS, BANSHEE_CLEAR_HISTORY, BANSHEE_CONFIGURE, BANSHEE_CONNECT_APPLY,
    BANSHEE_CONNECT_PLAN, BANSHEE_DOWNLOAD_MODELS, BANSHEE_HISTORY, BANSHEE_LIST_INPUT_DEVICES,
    BANSHEE_LIST_VOICES, BANSHEE_OPEN_PERMISSION, BANSHEE_SPEAK, BANSHEE_STATUS, InputDevice,
    PlannedChange, Voice,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const PREVIEW_SENTENCE: &str = "This is how I sound.";

/// A JSON-RPC parse error's code (-32700), reused here for a reply this
/// client could not read, since that failure is ours, not the daemon's.
const SHAPE_MISMATCH: i32 = -32700;

/// The daemon's code and message when it answered; the transport's or the window's own
/// message when it did not.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommandError {
    pub code: i32,
    pub message: String,
    #[serde(skip)]
    pub transport: bool,
    #[serde(skip)]
    pub sent: bool,
}

impl From<RpcError> for CommandError {
    fn from(error: RpcError) -> Self {
        CommandError {
            code: error.code,
            message: error.message,
            transport: error.transport,
            sent: error.sent,
        }
    }
}

fn shape_mismatch(error: serde_json::Error) -> CommandError {
    CommandError {
        code: SHAPE_MISMATCH,
        message: error.to_string(),
        transport: false,
        sent: true,
    }
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Languages {
    pub languages: Vec<banshee_common::Language>,
}

pub async fn status(client: &mut Client) -> Result<Value, CommandError> {
    client
        .call(BANSHEE_STATUS, json!({}))
        .await
        .map_err(CommandError::from)
}

pub async fn set_setting(
    client: &mut Client,
    key: &str,
    value: Value,
) -> Result<Vec<String>, CommandError> {
    let result = client
        .call(
            BANSHEE_CONFIGURE,
            json!({
                "settings": { key: value },
                "persist": true,
            }),
        )
        .await
        .map_err(CommandError::from)?;
    serde_json::from_value(result["restart_required"].clone()).map_err(shape_mismatch)
}

pub async fn list_devices(client: &mut Client) -> Result<Devices, CommandError> {
    let result = client
        .call(BANSHEE_LIST_INPUT_DEVICES, json!({}))
        .await
        .map_err(CommandError::from)?;
    serde_json::from_value(result).map_err(shape_mismatch)
}

pub async fn list_voices(client: &mut Client) -> Result<Voices, CommandError> {
    let result = client
        .call(BANSHEE_LIST_VOICES, json!({}))
        .await
        .map_err(CommandError::from)?;
    serde_json::from_value(result).map_err(shape_mismatch)
}

pub async fn list_languages(client: &mut Client) -> Result<Languages, CommandError> {
    let result = client
        .call(banshee_common::BANSHEE_LIST_LANGUAGES, json!({}))
        .await
        .map_err(CommandError::from)?;
    serde_json::from_value(result).map_err(shape_mismatch)
}

pub async fn preview_voice(client: &mut Client, id: &str) -> Result<(), CommandError> {
    client
        .call(
            BANSHEE_SPEAK,
            json!({"text": PREVIEW_SENTENCE, "voice": id}),
        )
        .await
        .map_err(CommandError::from)?;
    Ok(())
}

pub async fn download_models(client: &mut Client) -> Result<(), CommandError> {
    client
        .call(BANSHEE_DOWNLOAD_MODELS, json!({}))
        .await
        .map_err(CommandError::from)?;
    Ok(())
}

pub async fn detect_agents(client: &mut Client) -> Result<Vec<AgentRow>, CommandError> {
    let result = client
        .call(BANSHEE_AGENTS, json!({}))
        .await
        .map_err(CommandError::from)?;
    serde_json::from_value(result["agents"].clone()).map_err(shape_mismatch)
}

pub async fn plan_connect(
    client: &mut Client,
    id: &str,
    disconnect: bool,
) -> Result<Vec<PlannedChange>, CommandError> {
    let result = client
        .call(
            BANSHEE_CONNECT_PLAN,
            json!({"agent": id, "disconnect": disconnect}),
        )
        .await
        .map_err(CommandError::from)?;
    serde_json::from_value(result["changes"].clone()).map_err(shape_mismatch)
}

pub async fn apply_connect(
    client: &mut Client,
    id: &str,
    disconnect: bool,
) -> Result<(), CommandError> {
    client
        .call(
            BANSHEE_CONNECT_APPLY,
            json!({"agent": id, "disconnect": disconnect}),
        )
        .await
        .map_err(CommandError::from)?;
    Ok(())
}

/// `limit` is forwarded when present and omitted entirely when absent: the
/// daemon reads an absent `limit` as every row, and an explicit `0` as none.
pub async fn history(client: &mut Client, limit: Option<u32>) -> Result<Vec<Value>, CommandError> {
    let mut params = serde_json::Map::new();
    if let Some(limit) = limit {
        params.insert("limit".to_string(), json!(limit));
    }
    let result = client
        .call(BANSHEE_HISTORY, Value::Object(params))
        .await
        .map_err(CommandError::from)?;
    serde_json::from_value(result["history"].clone()).map_err(shape_mismatch)
}

pub async fn clear_history(client: &mut Client) -> Result<(), CommandError> {
    client
        .call(BANSHEE_CLEAR_HISTORY, json!({}))
        .await
        .map_err(CommandError::from)?;
    Ok(())
}

pub async fn open_permission_pane(client: &mut Client, id: &str) -> Result<(), CommandError> {
    client
        .call(BANSHEE_OPEN_PERMISSION, json!({"id": id}))
        .await
        .map_err(CommandError::from)?;
    Ok(())
}

use dirs;
use serde_json::Value;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::error::BansheeError;
use crate::{JsonRpcRequest, JsonRpcResponse};

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

    let mut reader = BufReader::new(stream);

    let mut response = String::new();
    reader.read_line(&mut response).await?;

    let json_response: JsonRpcResponse = serde_json::from_str(&response)?;

    match json_response {
        JsonRpcResponse::Success { result, .. } => Ok(result),
        JsonRpcResponse::Error { error, .. } => Err(BansheeError::Rpc {
            code: error.code,
            message: error.message,
        }),
    }
}

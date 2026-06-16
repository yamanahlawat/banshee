use dirs;
use serde_json::Value;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::{JsonRpcRequest, JsonRpcResponse};

pub fn get_socket_path() -> Option<PathBuf> {
    let Some(base_path) = dirs::home_dir() else {
        return None;
    };

    Some(base_path.join(".banshee").join("banshee.sock"))
}

pub fn get_models_path() -> Option<PathBuf> {
    let Some(base_path) = dirs::home_dir() else {
        return None;
    };

    Some(base_path.join(".banshee").join("models"))
}

pub async fn call_daemon(method: &str, params: Value) -> Result<Value, String> {
    let socket_path = get_socket_path().ok_or("Could not find home directory".to_string())?;

    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params: Some(params),
        id: Some(serde_json::json!(1)),
    };

    let mut request_string = serde_json::to_string(&request).map_err(|e| e.to_string())?;

    let mut stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| e.to_string())?;

    request_string.push('\n');
    stream
        .write_all(request_string.as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(stream);

    let mut response = String::new();
    reader
        .read_line(&mut response)
        .await
        .map_err(|e| e.to_string())?;

    let json_response: JsonRpcResponse =
        serde_json::from_str(&response).map_err(|e| e.to_string())?;

    match json_response {
        JsonRpcResponse::Success { result, .. } => Ok(result),
        JsonRpcResponse::Error { error, .. } => {
            Err(format!("Daemon returned an RPC error {:?}", error))
        }
    }
}

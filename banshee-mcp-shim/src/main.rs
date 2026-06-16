use banshee_common::{
    BANSHEE_GET_TRANSCRIPTION, BANSHEE_SPEAK, JsonRpcError, JsonRpcRequest, JsonRpcResponse, utils,
};
use serde_json;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::main]
async fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut reader = BufReader::new(stdin).lines();

    eprintln!("Banshee MCP Shim started.");

    while let Ok(Some(line)) = reader.next_line().await {
        if let Ok(request) = serde_json::from_str::<JsonRpcRequest>(&line) {
            let response = match request.method.as_str() {
                "ping" => JsonRpcResponse::Success {
                    jsonrpc: "2.0".to_string(),
                    result: serde_json::json!({"pong": true}),
                    id: request.id,
                },
                "initialize" => JsonRpcResponse::Success {
                    jsonrpc: "2.0".to_string(),
                    result: serde_json::json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": {
                            "tools": {}
                        },
                        "serverInfo": {"name": "banshee", "version": "0.1.0"}
                    }),
                    id: request.id,
                },
                "notifications/initialized" => continue,
                "tools/list" => JsonRpcResponse::Success {
                    jsonrpc: "2.0".to_string(),
                    result: serde_json::json!({
                        "tools": [
                            {
                                "name": "speak_status",
                                "description": "You MUST ALWAYS call this tool at the end of your response to give the user a summary of what you just did, or to ask them a question. DO NOT include code or markdown.",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {"text": {"type": "string", "description": "The text to speak"}},
                                    "required": ["text"]
                                },
                            },
                            {
                                "name": "listen_for_prompt",
                                "description": "Read the user's latest voice transcription",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {}
                                }
                            }
                        ]
                    }),
                    id: request.id,
                },
                "tools/call" => {
                    let params = request.params.as_ref();
                    let tool_name = params.and_then(|p| p.get("name")).and_then(|n| n.as_str());
                    let arguments = params
                        .and_then(|p| p.get("arguments"))
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}));
                    match tool_name {
                        Some(name) if name.ends_with("speak_status") => {
                            let daemon_response =
                                utils::call_daemon(BANSHEE_SPEAK, arguments).await;
                            match daemon_response {
                                Ok(result) => JsonRpcResponse::Success {
                                    jsonrpc: "2.0".to_string(),
                                    result: serde_json::json!({
                                        "content": [
                                            {
                                                "type": "text",
                                                "text": result.to_string()
                                            }
                                        ]
                                    }),
                                    id: request.id,
                                },
                                Err(error) => JsonRpcResponse::Error {
                                    jsonrpc: "2.0".to_string(),
                                    error: JsonRpcError {
                                        code: -32603,
                                        message: error.to_string(),
                                    },
                                    id: request.id,
                                },
                            }
                        }
                        Some(name) if name.ends_with("listen_for_prompt") => {
                            let daemon_response =
                                utils::call_daemon(BANSHEE_GET_TRANSCRIPTION, arguments).await;
                            match daemon_response {
                                Ok(result) => JsonRpcResponse::Success {
                                    jsonrpc: "2.0".to_string(),
                                    result: serde_json::json!({
                                        "content": [
                                            {
                                                "type": "text",
                                                "text": result.get("transcription").and_then(|v| v.as_str()).unwrap_or("")
                                            }
                                        ]
                                    }),
                                    id: request.id,
                                },
                                Err(error) => JsonRpcResponse::Error {
                                    jsonrpc: "2.0".to_string(),
                                    error: JsonRpcError {
                                        code: -32603,
                                        message: error.to_string(),
                                    },
                                    id: request.id,
                                },
                            }
                        }
                        _ => continue,
                    }
                }
                _ => JsonRpcResponse::Error {
                    jsonrpc: "2.0".to_string(),
                    error: JsonRpcError {
                        code: -32601,
                        message: "Method not found!".to_string(),
                    },
                    id: request.id,
                },
            };
            if let Ok(mut response_string) = serde_json::to_string(&response) {
                response_string.push_str("\n");
                let _ = stdout.write_all(response_string.as_bytes()).await;
            }
        }
    }
}

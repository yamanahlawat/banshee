use banshee_common::{
    BANSHEE_GET_TRANSCRIPTION, BANSHEE_SPEAK, JsonRpcRequest, JsonRpcResponse, utils,
};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::main]
async fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut reader = BufReader::new(stdin).lines();

    // Cursor into the daemon's transcription ring
    let mut last_seen_id: u64 = 0;

    eprintln!("Banshee MCP Shim started.");

    while let Ok(Some(line)) = reader.next_line().await {
        if let Ok(request) = serde_json::from_str::<JsonRpcRequest>(&line) {
            let response = match request.method.as_str() {
                "ping" => JsonRpcResponse::success(request.id, serde_json::json!({"pong": true})),
                "initialize" => JsonRpcResponse::success(
                    request.id,
                    serde_json::json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": {
                            "tools": {}
                        },
                        "serverInfo": {"name": "banshee", "version": "0.1.0"}
                    }),
                ),
                "notifications/initialized" => continue,
                "tools/list" => JsonRpcResponse::success(
                    request.id,
                    serde_json::json!({
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
                                "description": "Read the user's voice transcriptions since your last call. Pass timeout_ms to wait for the user to finish speaking.",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {"timeout_ms": {"type": "number", "description": "Wait up to this many milliseconds for new speech before returning"}}
                                }
                            }
                        ]
                    }),
                ),
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
                                Ok(result) => JsonRpcResponse::success(
                                    request.id,
                                    serde_json::json!({
                                        "content": [
                                            {
                                                "type": "text",
                                                "text": result.to_string()
                                            }
                                        ]
                                    }),
                                ),
                                Err(error) => {
                                    JsonRpcResponse::error(request.id, -32603, error.to_string())
                                }
                            }
                        }
                        Some(name) if name.ends_with("listen_for_prompt") => {
                            let wait_ms = arguments
                                .get("timeout_ms")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let daemon_response = utils::call_daemon(
                                BANSHEE_GET_TRANSCRIPTION,
                                serde_json::json!({"since_id": last_seen_id, "wait_ms": wait_ms}),
                            )
                            .await;
                            match daemon_response {
                                Ok(result) => {
                                    let transcriptions = result
                                        .get("transcriptions")
                                        .and_then(|v| v.as_array())
                                        .cloned()
                                        .unwrap_or_default();
                                    for item in &transcriptions {
                                        if let Some(id) = item.get("id").and_then(|v| v.as_u64()) {
                                            last_seen_id = last_seen_id.max(id);
                                        }
                                    }
                                    let text = transcriptions
                                        .iter()
                                        .filter_map(|item| {
                                            item.get("text").and_then(|v| v.as_str())
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    JsonRpcResponse::success(
                                        request.id,
                                        serde_json::json!({
                                            "content": [
                                                {
                                                    "type": "text",
                                                    "text": text
                                                }
                                            ]
                                        }),
                                    )
                                }

                                Err(error) => {
                                    JsonRpcResponse::error(request.id, -32603, error.to_string())
                                }
                            }
                        }
                        _ => continue,
                    }
                }
                _ => JsonRpcResponse::error(request.id, -32601, "Method not found!"),
            };
            if let Ok(mut response_string) = serde_json::to_string(&response) {
                response_string.push('\n');
                let _ = stdout.write_all(response_string.as_bytes()).await;
            }
        }
    }
}

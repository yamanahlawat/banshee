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
                        "serverInfo": {"name": "banshee", "version": env!("CARGO_PKG_VERSION")}
                    }),
                ),
                "notifications/initialized" => continue,
                "tools/list" => JsonRpcResponse::success(
                    request.id,
                    serde_json::json!({
                        "tools": [
                            {
                                "name": "speak_status",
                                "description": "Speak a short message aloud to the user, who is working eyes-free and not reading the screen. Use ONLY for: decisions you need input on, questions, and completion of a significant phase of work. Do not narrate routine steps or tool activity. When an implementation is done, tell the user it is ready for review.",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {"text": {"type": "string", "description": "One or two conversational sentences, as if speaking to a colleague. Refer to code, files, and identifiers by their spoken names, for example 'the hotkey listener' rather than a file path or function signature. Keep exact paths, code, URLs, and lists in your normal text output; they do not read well aloud."}},
                                    "required": ["text"]
                                },
                            },
                            {
                                "name": "listen_for_prompt",
                                "description": "Read what the user has said since your last call. After asking a question with speak_status, call this with a timeout_ms to wait for their spoken answer. Returns empty text if the user said nothing.",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {"timeout_ms": {"type": "number", "description": "Wait up to this many milliseconds for new speech before returning, e.g. 30000 when expecting an answer"}}
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

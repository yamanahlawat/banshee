use banshee_common::error::BansheeError;
use banshee_common::{
    BANSHEE_ASK_USER, BANSHEE_GET_TRANSCRIPTION, BANSHEE_SPEAK, JsonRpcRequest, JsonRpcResponse,
    rpc_code, utils,
};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

const INSTRUCTIONS: &str = "The user is working eyes-free and is not reading the screen. \
     When you need a decision or an answer, ask it with ask_user, never with an on-screen \
     prompt, menu or written question: the user cannot see those. Speak your status with \
     speak_status, and keep written output for what has to be read, such as code, paths, \
     commands and tables.";

fn tool_text(id: Option<serde_json::Value>, text: &str) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        serde_json::json!({"content": [{"type": "text", "text": text}]}),
    )
}

/// A tool that ran and failed answers a result, marked so the model does not
/// read the text as an answer. MCP keeps JSON-RPC errors for the protocol.
fn tool_failed(id: Option<serde_json::Value>, error: BansheeError) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        serde_json::json!({"content": [{"type": "text", "text": error.rpc_message()}], "isError": true}),
    )
}

fn latest_id(result: &serde_json::Value) -> Option<u64> {
    result
        .get("transcriptions")?
        .as_array()?
        .iter()
        .filter_map(|item| item.get("id").and_then(|v| v.as_u64()))
        .max()
}

/// The model's arguments with `agent_pid` set to this shim's agent, over any the model sent.
fn for_agent(mut arguments: serde_json::Value, agent_pid: u32) -> serde_json::Value {
    if let Some(object) = arguments.as_object_mut() {
        object.insert("agent_pid".into(), serde_json::json!(agent_pid));
    }
    arguments
}

/// What `tools/list` advertises. The MCP client sends a name from here back
/// in `tools/call`.
fn tools_list() -> serde_json::Value {
    serde_json::json!({
    "tools": [
        {
            "name": "speak_status",
            "description": "Speak a short message aloud to the user, who is working eyes-free and not reading the screen. This spoken message is your reply to them, so do not also repeat it as written text; reserve written output for what must be read on screen, such as code, file paths, commands, URLs, and lists. Use it to say what you decided or finished; when you need an answer back, use ask_user instead, which speaks and listens in one step. Talk like a colleague in the room: natural, warm, and varied, never scripted. When you finish, say what got done and flag anything still pending, then hand back to the user in your own words each time. When an implementation is done, mention it is ready for review. Do not narrate routine steps or tool activity in between.",
            "inputSchema": {
                "type": "object",
                "properties": {"text": {"type": "string", "description": "One or two conversational sentences, as if speaking to a colleague. Refer to code, files, and identifiers by their spoken names, for example 'the hotkey listener' rather than a file path or function signature. Keep exact paths, code, URLs, and lists in your normal text output; they do not read well aloud."}},
                "required": ["text"]
            },
        },
        {
            "name": "ask_user",
            "description": "Ask the user a question aloud and wait for their spoken answer. The user is working eyes-free and cannot see the screen, so every question goes through this tool: never write a question as text, and never put one in an on-screen prompt or menu. Use it when you need a decision or clarification: the question is spoken, the microphone opens once it finishes playing, and the transcribed reply comes back scoped to you. Ask one focused question per call; when you have several, ask the most important first and wait for the answer before asking the next, so the user is never holding multiple questions in their head. Returns empty text if the user stayed silent, and an error if the listening itself failed, so silence and a failed listen are never confused.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": {"type": "string", "description": "One or two conversational sentences, as if asking a colleague. Refer to code, files, and identifiers by their spoken names rather than paths or signatures."},
                    "timeout_ms": {"type": "number", "description": "How long to wait for the user to start answering, in milliseconds. Defaults to 30000, and is capped at 120000."}
                },
                "required": ["question"]
            }
        },
        {
            "name": "listen_for_prompt",
            "description": "Read what the user has said since your last call. Use it to pick up speech you did not explicitly ask for; when you have a question, prefer ask_user, which speaks it and waits in one step. It returns at once unless you pass timeout_ms, and empty text if the user said nothing.",
            "inputSchema": {
                "type": "object",
                "properties": {"timeout_ms": {"type": "number", "description": "Wait up to this many milliseconds for new speech before returning, e.g. 30000 when expecting an answer. Capped at 30000."}}
            }
        }
    ]
    })
}

/// One `tools/call`: the daemon method the tool names, and its reply shaped as
/// the tool's text.
async fn tools_call(
    id: Option<serde_json::Value>,
    params: Option<&serde_json::Value>,
    last_seen_id: &mut u64,
    agent_pid: u32,
    daemon: &impl AsyncFn(&str, serde_json::Value) -> Result<serde_json::Value, BansheeError>,
) -> JsonRpcResponse {
    let tool_name = params.and_then(|p| p.get("name")).and_then(|n| n.as_str());
    // Whether a client ever sends a prefixed name decides the match below.
    log::debug!("tools/call {}", tool_name.unwrap_or("(no name)"));
    let arguments = params
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    match tool_name {
        Some(name) if name.ends_with("speak_status") => {
            match daemon(BANSHEE_SPEAK, for_agent(arguments, agent_pid)).await {
                Ok(result) => tool_text(id, &result.to_string()),
                Err(error) => tool_failed(id, error),
            }
        }
        Some(name) if name.ends_with("ask_user") => {
            match daemon(BANSHEE_ASK_USER, for_agent(arguments, agent_pid)).await {
                Ok(result) => {
                    let text = result
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    tool_text(id, text)
                }
                Err(error) => tool_failed(id, error),
            }
        }
        Some(name) if name.ends_with("listen_for_prompt") => {
            let wait_ms = arguments
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let daemon_response = daemon(
                BANSHEE_GET_TRANSCRIPTION,
                serde_json::json!({"since_id": *last_seen_id, "wait_ms": wait_ms}),
            )
            .await;
            match daemon_response {
                Ok(result) => {
                    if let Some(id) = latest_id(&result) {
                        *last_seen_id = (*last_seen_id).max(id);
                    }
                    let transcriptions = result
                        .get("transcriptions")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let text = transcriptions
                        .iter()
                        .filter_map(|item| item.get("text").and_then(|v| v.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    tool_text(id, &text)
                }
                Err(error) => tool_failed(id, error),
            }
        }
        Some(other) => JsonRpcResponse::error(
            id,
            rpc_code::INVALID_PARAMS,
            format!("Unknown tool: {other}"),
        ),
        None => JsonRpcResponse::error(id, rpc_code::INVALID_PARAMS, "tools/call needs a name"),
    }
}

/// The reply one stdin line owes, or `None` where the protocol expects silence.
async fn respond(
    line: &str,
    last_seen_id: &mut u64,
    agent_pid: u32,
    daemon: impl AsyncFn(&str, serde_json::Value) -> Result<serde_json::Value, BansheeError>,
) -> Option<JsonRpcResponse> {
    let request = match serde_json::from_str::<JsonRpcRequest>(line) {
        Ok(request) => request,
        Err(error) => return Some(JsonRpcResponse::parse_error(&error)),
    };
    // A notification carries no id, and a reply to one is a protocol error.
    request.id.as_ref()?;
    Some(match request.method.as_str() {
        "ping" => JsonRpcResponse::success(request.id, serde_json::json!({"pong": true})),
        "initialize" => JsonRpcResponse::success(
            request.id,
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {"name": "banshee", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS
            }),
        ),
        "tools/list" => JsonRpcResponse::success(request.id, tools_list()),
        "tools/call" => {
            tools_call(
                request.id,
                request.params.as_ref(),
                last_seen_id,
                agent_pid,
                &daemon,
            )
            .await
        }
        _ => JsonRpcResponse::error(request.id, rpc_code::METHOD_NOT_FOUND, "Method not found!"),
    })
}

#[tokio::main]
async fn main() {
    banshee_common::logging::install();
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut reader = BufReader::new(stdin).lines();

    // Ring cursor, primed so the first poll skips pre-session speech.
    // On error the daemon is down and its ring will start empty, so 0 is right.
    let mut last_seen_id: u64 = utils::call_daemon(
        BANSHEE_GET_TRANSCRIPTION,
        serde_json::json!({"since_id": 0, "wait_ms": 0}),
    )
    .await
    .ok()
    .and_then(|result| latest_id(&result))
    .unwrap_or(0);

    let agent_pid = std::os::unix::process::parent_id();

    log::info!("Banshee MCP shim started");

    while let Ok(Some(line)) = reader.next_line().await {
        let Some(response) = respond(&line, &mut last_seen_id, agent_pid, utils::call_daemon).await
        else {
            continue;
        };
        if let Ok(mut response_string) = serde_json::to_string(&response) {
            response_string.push('\n');
            let _ = stdout.write_all(response_string.as_bytes()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: u32 = 4242;

    async fn no_daemon(
        _method: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, BansheeError> {
        unreachable!("these requests are answered without the daemon")
    }

    /// One stdin line, as a client writes it.
    fn request(method: &str, params: serde_json::Value, id: Option<u64>) -> String {
        serde_json::to_string(&JsonRpcRequest {
            jsonrpc: banshee_common::Version::V2,
            method: method.into(),
            params: Some(params),
            id: id.map(serde_json::Value::from),
        })
        .unwrap()
    }

    #[tokio::test]
    async fn a_ping_is_answered_on_its_own_id() {
        let reply = respond(
            &request("ping", serde_json::json!({}), Some(3)),
            &mut 0,
            AGENT,
            no_daemon,
        )
        .await;

        match reply {
            Some(JsonRpcResponse::Success { id, .. }) => assert_eq!(id, Some(serde_json::json!(3))),
            other => panic!("a ping is answered, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_notification_gets_no_reply() {
        let reply = respond(
            &request(
                "notifications/cancelled",
                serde_json::json!({"requestId": 3}),
                None,
            ),
            &mut 0,
            AGENT,
            no_daemon,
        )
        .await;

        assert!(
            reply.is_none(),
            "a notification has no id and expects no reply, got {reply:?}"
        );
    }

    #[tokio::test]
    async fn an_unknown_method_with_an_id_is_method_not_found() {
        let reply = respond(
            &request("resources/list", serde_json::json!({}), Some(4)),
            &mut 0,
            AGENT,
            no_daemon,
        )
        .await;

        match reply {
            Some(JsonRpcResponse::Error { error, id, .. }) => {
                assert_eq!(error.code, rpc_code::METHOD_NOT_FOUND);
                assert_eq!(id, Some(serde_json::json!(4)));
            }
            other => panic!("a request with an id is always answered, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unknown_tool_is_refused_as_invalid_params() {
        let reply = respond(
            &request(
                "tools/call",
                serde_json::json!({"name": "frobnicate", "arguments": {}}),
                Some(7),
            ),
            &mut 0,
            AGENT,
            no_daemon,
        )
        .await;

        match reply {
            Some(JsonRpcResponse::Error { error, id, .. }) => {
                assert_eq!(error.code, rpc_code::INVALID_PARAMS);
                assert!(
                    error.message.contains("frobnicate"),
                    "the refusal names the tool: {}",
                    error.message
                );
                assert_eq!(id, Some(serde_json::json!(7)));
            }
            other => panic!("an unknown tool is refused, not left waiting, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_tool_call_with_no_name_says_so() {
        let reply = respond(
            &request("tools/call", serde_json::json!({"arguments": {}}), Some(8)),
            &mut 0,
            AGENT,
            no_daemon,
        )
        .await;

        match reply {
            Some(JsonRpcResponse::Error { error, .. }) => {
                assert_eq!(error.code, rpc_code::INVALID_PARAMS);
                assert!(
                    error.message.contains("name"),
                    "the refusal says what is missing: {}",
                    error.message
                );
            }
            other => panic!("a call with no tool name is refused, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_listen_never_moves_the_cursor_backwards() {
        let older_ring = async |_method: &str, params: serde_json::Value| {
            assert_eq!(
                params["since_id"], 20,
                "the daemon is asked from the cursor"
            );
            Ok(serde_json::json!({"transcriptions": [
                {"id": 4, "text": "first"},
                {"id": 9, "text": "second"}
            ]}))
        };
        let mut cursor = 20;

        let reply = respond(
            &request(
                "tools/call",
                serde_json::json!({"name": "listen_for_prompt", "arguments": {}}),
                Some(9),
            ),
            &mut cursor,
            AGENT,
            older_ring,
        )
        .await;

        assert_eq!(cursor, 20, "ids below the cursor do not pull it back");
        match reply {
            Some(JsonRpcResponse::Success { result, .. }) => {
                assert_eq!(result["content"][0]["text"], "first\nsecond");
            }
            other => panic!("a listen answers the text it read, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_daemon_refusal_is_a_tool_result_the_model_can_read() {
        let refusing = async |_method: &str, _params: serde_json::Value| {
            Err(BansheeError::Rpc {
                code: rpc_code::INVALID_PARAMS,
                message: "'text' is required and must be a string.".into(),
            })
        };

        let reply = respond(
            &request(
                "tools/call",
                serde_json::json!({"name": "speak_status", "arguments": {}}),
                Some(5),
            ),
            &mut 0,
            AGENT,
            refusing,
        )
        .await;

        match reply {
            Some(JsonRpcResponse::Success { result, .. }) => {
                assert_eq!(result["isError"], true);
                assert_eq!(
                    result["content"][0]["text"],
                    "'text' is required and must be a string."
                );
            }
            other => panic!("a failed tool call is a result marked isError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_line_that_is_not_a_request_is_answered_with_a_parse_error() {
        let reply = respond(
            "{\"jsonrpc\": \"2.0\", \"method\": 3",
            &mut 0,
            AGENT,
            no_daemon,
        )
        .await;

        match reply {
            Some(JsonRpcResponse::Error { error, id, .. }) => {
                assert_eq!(error.code, rpc_code::PARSE);
                assert_eq!(
                    id, None,
                    "a request that did not parse has no id to answer on"
                );
            }
            other => panic!("a bad line is answered, not left waiting, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn both_voice_tools_name_the_agent_over_what_the_model_sent() {
        for (tool, arguments) in [
            (
                "speak_status",
                serde_json::json!({"text": "hi", "agent_pid": 1}),
            ),
            (
                "ask_user",
                serde_json::json!({"question": "ready?", "agent_pid": 1}),
            ),
        ] {
            let sent = std::sync::Mutex::new(None);
            let recording = async |_method: &str, params: serde_json::Value| {
                *sent.lock().unwrap() = Some(params);
                Ok(serde_json::json!({"ok": true, "text": ""}))
            };
            let mut last_seen_id = 0;
            respond(
                &request(
                    "tools/call",
                    serde_json::json!({"name": tool, "arguments": arguments}),
                    Some(1),
                ),
                &mut last_seen_id,
                AGENT,
                recording,
            )
            .await;
            assert_eq!(
                sent.lock().unwrap().as_ref().unwrap()["agent_pid"],
                AGENT,
                "{tool}"
            );
        }
    }

    #[tokio::test]
    async fn a_listen_names_no_agent() {
        let sent = std::sync::Mutex::new(None);
        let recording = async |_method: &str, params: serde_json::Value| {
            *sent.lock().unwrap() = Some(params);
            Ok(serde_json::json!({"transcriptions": []}))
        };
        let mut last_seen_id = 0;
        respond(
            &request(
                "tools/call",
                serde_json::json!({"name": "listen_for_prompt", "arguments": {}}),
                Some(1),
            ),
            &mut last_seen_id,
            AGENT,
            recording,
        )
        .await;
        assert!(
            sent.lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .get("agent_pid")
                .is_none()
        );
    }
}

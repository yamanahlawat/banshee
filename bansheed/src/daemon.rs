use banshee_common::utils::get_socket_path;
use banshee_common::{JsonRpcRequest, JsonRpcResponse};
use std::fs;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

use crate::api::dispatch;
use crate::state::DaemonState;

pub async fn run(daemon_state: &Arc<DaemonState>) -> Result<(), std::io::Error> {
    println!("Starting unix socket listener...");

    let Some(socket_path) = get_socket_path() else {
        println!("Could not find home directory.");
        return Ok(());
    };

    let _ = fs::remove_file(&socket_path);

    if let Some(parent_dir) = socket_path.parent() {
        fs::create_dir_all(parent_dir)?;
    }

    println!("No socket found. Creating one...");
    let listener = UnixListener::bind(&socket_path)?;

    loop {
        match listener.accept().await {
            Ok((mut stream, _addr)) => {
                println!("New client connected!");
                let state = Arc::clone(daemon_state);
                // Spawn a new task to handle the client connection
                tokio::spawn(async move {
                    let (reader, mut writer) = stream.split();
                    let reader = BufReader::new(reader);
                    let mut lines = reader.lines();

                    while let Ok(Some(line)) = lines.next_line().await {
                        // Try to parse the incoming line as a JSON-RPC request
                        if let Ok(request) = serde_json::from_str::<JsonRpcRequest>(&line) {
                            let response: JsonRpcResponse = dispatch(request, &state);
                            if let Ok(mut response_string) = serde_json::to_string(&response) {
                                response_string.push('\n');
                                let _ = writer.write_all(response_string.as_bytes()).await;
                            }
                        }
                    }
                });
            }
            Err(error) => println!("Connection failed, Error: {error}"),
        }
    }
}

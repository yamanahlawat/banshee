use banshee_app::socket::{Client, backoff};
use banshee_common::{BANSHEE_STATUS, JsonRpcRequest, JsonRpcResponse};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc::UnboundedSender;

/// A stand-in daemon. Every request it decodes is sent to `methods` so the
/// test body can assert on it; a swallowed panic inside `tokio::spawn` cannot
/// fail the test the way an assertion in the test body can.
async fn fake_daemon(path: std::path::PathBuf, methods: UnboundedSender<String>) {
    let listener = UnixListener::bind(&path).unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let request: JsonRpcRequest = serde_json::from_str(&line).unwrap();
        methods.send(request.method.clone()).unwrap();
        let reply = JsonRpcResponse::success(
            request.id,
            serde_json::json!({"running": true, "version": "test"}),
        );
        let mut text = serde_json::to_string(&reply).unwrap();
        text.push('\n');
        writer.write_all(text.as_bytes()).await.unwrap();
    }
}

#[tokio::test]
async fn a_status_call_round_trips_over_a_unix_socket() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("banshee.sock");
    let (methods_tx, mut methods_rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(fake_daemon(path.clone(), methods_tx));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let mut client = Client::connect(&path).await.unwrap();
    let status = client
        .call(BANSHEE_STATUS, serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(status["running"], true);
    assert_eq!(status["version"], "test");
    assert_eq!(methods_rx.recv().await.unwrap(), BANSHEE_STATUS);
}

#[test]
fn backoff_doubles_from_a_quarter_second_and_caps_at_five() {
    assert_eq!(backoff(0).as_millis(), 250);
    assert_eq!(backoff(1).as_millis(), 500);
    assert_eq!(backoff(2).as_millis(), 1000);
    assert_eq!(backoff(3).as_millis(), 2000);
    // 4 is the last attempt still doubling; 5 is where the 5 s cap first bites
    assert_eq!(backoff(4).as_millis(), 4000);
    assert_eq!(backoff(5).as_millis(), 5000);
    assert_eq!(backoff(6).as_millis(), 5000);
    assert_eq!(backoff(10).as_millis(), 5000);
    assert_eq!(backoff(63).as_millis(), 5000);
    assert_eq!(backoff(u32::MAX).as_millis(), 5000);
}

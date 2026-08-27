mod common;

use banshee_app::socket::{Client, backoff};
use banshee_common::BANSHEE_STATUS;
use common::recording_daemon;

#[tokio::test]
async fn a_status_call_round_trips_over_a_unix_socket() {
    let (path, mut seen, _guard) =
        recording_daemon(serde_json::json!({"running": true, "version": "test"})).await;
    let mut client = Client::connect(&path).await.unwrap();
    let status = client
        .call(BANSHEE_STATUS, serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(status["running"], true);
    assert_eq!(status["version"], "test");
    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_STATUS);
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

mod common;

use banshee_app::calls;
use banshee_app::socket::Client;
use banshee_common::{BANSHEE_CONFIGURE, BANSHEE_HISTORY, BANSHEE_SPEAK};
use common::{recording_daemon, recording_error_daemon};

#[tokio::test]
async fn a_setting_is_written_with_persist_true() {
    let (path, mut seen, _guard) =
        recording_daemon(serde_json::json!({"restart_required": ["audio.cues.enabled"]})).await;
    let mut client = Client::connect(&path).await.unwrap();

    let restart = calls::set_setting(&mut client, "audio.cues.enabled", serde_json::json!(true))
        .await
        .unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_CONFIGURE);
    assert_eq!(
        request.params.unwrap(),
        serde_json::json!({
            "settings": {"audio.cues.enabled": true},
            "persist": true,
        })
    );
    assert_eq!(restart, vec!["audio.cues.enabled".to_string()]);
}

#[tokio::test]
#[allow(clippy::len_zero)]
async fn a_voice_preview_speaks_one_sentence_in_that_voice() {
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({"ok": true})).await;
    let mut client = Client::connect(&path).await.unwrap();

    calls::preview_voice(&mut client, "am_adam").await.unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_SPEAK);
    let params = request.params.unwrap();
    assert_eq!(params["voice"], "am_adam");
    assert!(params["text"].as_str().unwrap().len() > 0);
    // A preview must not change the configured voice
    assert!(params.get("persist").is_none());
}

#[tokio::test]
async fn the_history_wrapper_key_never_reaches_a_caller() {
    let (path, _seen, _guard) = recording_daemon(serde_json::json!({
        "history": [{"id": 1, "text": "Yes.", "timestamp": "2026-08-26T13:47:00Z"}]
    }))
    .await;
    let mut client = Client::connect(&path).await.unwrap();
    let rows = calls::history(&mut client, None).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["text"], "Yes.");
}

#[tokio::test]
async fn a_history_call_without_a_limit_omits_the_key() {
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({"history": []})).await;
    let mut client = Client::connect(&path).await.unwrap();

    calls::history(&mut client, None).await.unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.method, BANSHEE_HISTORY);
    assert_eq!(request.params.unwrap(), serde_json::json!({}));
}

#[tokio::test]
async fn a_history_call_with_a_limit_forwards_it() {
    let (path, mut seen, _guard) = recording_daemon(serde_json::json!({"history": []})).await;
    let mut client = Client::connect(&path).await.unwrap();

    calls::history(&mut client, Some(20)).await.unwrap();

    let request = seen.recv().await.unwrap();
    assert_eq!(request.params.unwrap(), serde_json::json!({"limit": 20}));
}

#[tokio::test]
async fn a_daemon_error_reaches_the_caller_as_its_own_sentence() {
    let (path, _seen, _guard) =
        recording_error_daemon(-32602, "Disconnect is not available yet.").await;
    let mut client = Client::connect(&path).await.unwrap();
    let refused = calls::plan_connect(&mut client, "cursor", true)
        .await
        .unwrap_err();
    assert_eq!(refused, "Disconnect is not available yet.");
}

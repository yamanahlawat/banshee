use crate::socket::{Client, SOCKET_CLOSED, backoff};
use banshee_common::{
    BANSHEE_DOWNLOAD_PROGRESS, BANSHEE_STATE_CHANGED, EVENT_DOWNLOADS, EVENT_STATE,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

pub fn event_name(method: &str) -> Option<&'static str> {
    match method {
        m if m == BANSHEE_STATE_CHANGED => Some("daemon:state"),
        m if m == BANSHEE_DOWNLOAD_PROGRESS => Some("daemon:downloads"),
        _ => None,
    }
}

/// What the next retry count should be, given how long the session that just
/// ended lasted. A session that outlived the delay it took to reach it earns
/// a reset to 0; anything shorter climbs, so a daemon that accepts a
/// connection and immediately drops it cannot hold the delay at its floor.
pub fn next_attempt(attempt: u32, session: Duration) -> u32 {
    if session >= backoff(attempt) {
        0
    } else {
        attempt.saturating_add(1)
    }
}

/// Connects, reads status, subscribes, forwards events; on any drop, emits
/// daemon:down and reconnects with backoff. Never returns.
pub async fn run(app: AppHandle, path: PathBuf) {
    let mut attempt = 0u32;
    loop {
        attempt = match Client::connect(&path).await {
            Ok(client) => {
                let started = Instant::now();
                let opened = app.clone();
                let handle = app.clone();
                let result = client
                    .subscribe(
                        &[EVENT_STATE, EVENT_DOWNLOADS],
                        move |status| {
                            let _ = opened.emit("daemon:status", status);
                        },
                        move |notification| {
                            if let Some(name) = event_name(&notification.method) {
                                let _ = handle.emit(name, notification.params);
                            }
                        },
                    )
                    .await;
                let reason = result
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| SOCKET_CLOSED.to_string());
                let _ = app.emit("daemon:down", serde_json::json!({ "reason": reason }));
                next_attempt(attempt, started.elapsed())
            }
            Err(error) => {
                let _ = app.emit(
                    "daemon:down",
                    serde_json::json!({ "reason": error.to_string() }),
                );
                attempt.saturating_add(1)
            }
        };
        tokio::time::sleep(backoff(attempt)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{backoff, event_name, next_attempt};
    use banshee_common::{BANSHEE_DOWNLOAD_PROGRESS, BANSHEE_STATE_CHANGED};
    use std::time::Duration;

    #[test]
    fn every_notification_the_window_reads_has_an_event_name() {
        assert_eq!(event_name(BANSHEE_STATE_CHANGED), Some("daemon:state"));
        assert_eq!(
            event_name(BANSHEE_DOWNLOAD_PROGRESS),
            Some("daemon:downloads")
        );
        assert_eq!(event_name("banshee.something_else"), None);
    }

    #[test]
    fn a_session_that_outlasts_its_delay_resets_the_count() {
        let attempt = 5;
        let session = backoff(attempt) + Duration::from_millis(1);
        assert_eq!(next_attempt(attempt, session), 0);
    }

    #[test]
    fn an_instant_session_climbs_by_one() {
        assert_eq!(next_attempt(0, Duration::ZERO), 1);
    }

    #[test]
    fn an_instant_session_at_a_high_attempt_saturates_instead_of_wrapping() {
        assert_eq!(next_attempt(u32::MAX, Duration::ZERO), u32::MAX);
    }

    #[test]
    fn a_session_exactly_as_long_as_its_delay_still_earns_the_reset() {
        let attempt = 3;
        assert_eq!(next_attempt(attempt, backoff(attempt)), 0);
    }
}

use std::time::Duration;

use banshee_common::error::BansheeError;
use reqwest::blocking::Client;

use crate::credentials::{self, RemoteKey};

/// The read bound is the caller's: an upload and a speech read wait for
/// different things.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Built on a thread of its own, and joined, because reqwest's blocking builder
/// makes and drops a temporary runtime, and tokio refuses that drop on a worker.
/// Both backends are selected on one.
pub fn build_client(read_timeout: Duration, what: &str) -> Result<Client, BansheeError> {
    let build = move || {
        Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(read_timeout)
            .build()
    };
    std::thread::spawn(build)
        .join()
        .map_err(|_| BansheeError::Other(format!("the client for the remote {what} panicked")))?
        .map_err(|error| BansheeError::Other(error.to_string()))
}

/// The sentence a status carries on its own, for an answer whose body names no
/// fault. A 404 names what the side asked for by name: a path for the
/// listener, a voice for the speaker.
pub fn describe_status(side: RemoteKey, status: reqwest::StatusCode) -> String {
    let noun = side.side();
    match status.as_u16() {
        401 | 403 => format!("the remote {noun} refused the key"),
        404 => {
            let asked = match side {
                RemoteKey::Stt => "path",
                RemoteKey::Tts => "voice",
            };
            format!("the remote {noun} has no such model or {asked}")
        }
        429 => format!("the remote {noun} asked to slow down"),
        code => format!("the remote {noun} answered {code}"),
    }
}

/// What a non-success answer leaves in the daemon log: one line, with every
/// key-shaped run replaced. `said` is the body as the server sent it, or
/// nothing for a body that was never read.
pub fn log_line(
    side: RemoteKey,
    status: reqwest::StatusCode,
    said: Option<&str>,
    api_key: &str,
) -> String {
    let noun = side.side();
    match said {
        Some(body) => format!(
            "the remote {noun} answered {status}: {}",
            credentials::one_line(&credentials::redacted(body, api_key))
        ),
        None => format!("the remote {noun} answered {status}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::RemoteKey;
    use reqwest::StatusCode;

    /// Shaped like a key and issued by nobody.
    const FAKE_KEY: &str = "sk-proj-7Qm4Xb2vR8tL1yWn3cZa";

    #[test]
    fn each_side_names_itself_in_a_refusal() {
        let listener = describe_status(RemoteKey::Stt, StatusCode::NOT_FOUND);
        let speaker = describe_status(RemoteKey::Tts, StatusCode::NOT_FOUND);
        assert_eq!(listener, "the remote listener has no such model or path");
        assert_eq!(speaker, "the remote speaker has no such model or voice");
        assert_eq!(
            describe_status(RemoteKey::Tts, StatusCode::UNAUTHORIZED),
            "the remote speaker refused the key"
        );
    }

    // The daemon log is a surface no key may reach, whichever side answered.
    #[test]
    fn a_log_line_never_carries_the_key_for_either_side() {
        for side in [RemoteKey::Stt, RemoteKey::Tts] {
            let line = log_line(
                side,
                StatusCode::BAD_REQUEST,
                Some("model=sk-proj-9f3c2ab7d14e5b6079f3 is unknown"),
                "sk-proj-9f3c2ab7d14e5b6079f3",
            );
            assert!(!line.contains("9f3c2ab7"), "{line}");
            assert!(line.contains("model=<redacted> is unknown"), "{line}");
            assert!(line.contains(side.side()), "{line}");
        }
    }

    #[test]
    fn a_body_that_was_never_read_leaves_no_body_in_the_log() {
        assert_eq!(
            log_line(RemoteKey::Tts, StatusCode::UNAUTHORIZED, None, "sk-test"),
            "the remote speaker answered 401 Unauthorized"
        );
    }

    // A server that echoes the key it was sent, or any other key, is redacted
    // before the line is made.
    #[test]
    fn a_key_a_server_echoes_never_reaches_the_log() {
        let body = format!(
            r#"{{"error":"/audio/transcriptions: Invalid model name passed in model={FAKE_KEY}. Call `/v1/models`"}}"#
        );
        let line = log_line(
            RemoteKey::Stt,
            StatusCode::BAD_REQUEST,
            Some(&body),
            "gsk_8Hn2Qv6Lp0Rt4Ws9",
        );
        assert!(!line.contains("sk-proj"), "{line}");
        assert!(line.contains("model=<redacted>"), "{line}");
        assert!(line.contains("Invalid model name"), "{line}");

        let held = log_line(
            RemoteKey::Stt,
            StatusCode::BAD_REQUEST,
            Some("kokoro-9f3c2ab7d14e5b6079f3 is not a model"),
            "kokoro-9f3c2ab7d14e5b6079f3",
        );
        assert!(!held.contains("kokoro-9f3c"), "{held}");
        assert!(held.contains("<redacted> is not a model"), "{held}");
    }

    // One answer leaves one line, so a body cannot forge a line of its own in
    // the daemon log.
    #[test]
    fn a_body_that_arrives_in_lines_leaves_one_line() {
        let line = log_line(
            RemoteKey::Stt,
            StatusCode::BAD_GATEWAY,
            Some("no such model\n00:00:00.000 INFO  daemon: the listener is fine and idle"),
            FAKE_KEY,
        );
        assert!(!line.chars().any(char::is_control), "{line}");
        assert!(line.lines().count() == 1, "{line}");
        assert!(line.contains("no such model"), "{line}");
    }
}

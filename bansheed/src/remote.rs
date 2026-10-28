use std::time::Duration;

use banshee_common::error::BansheeError;
use reqwest::blocking::Client;

use crate::credentials::{self, RemoteKey};

/// The read bound is the caller's: an upload and a speech read wait for
/// different things.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// reqwest's own default, which a custom redirect policy replaces.
const REDIRECT_LIMIT: usize = 10;

/// Built on a thread of its own, and joined, because reqwest's blocking builder
/// makes and drops a temporary runtime, and tokio refuses that drop on a worker.
/// Both backends are selected on one.
pub fn build_client(
    side: RemoteKey,
    base_url: &str,
    read_timeout: Duration,
) -> Result<Client, BansheeError> {
    crate::config::refuse_plain_http_to_the_internet(side, base_url)
        .map_err(BansheeError::Other)?;
    let build = move || {
        Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(read_timeout)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= REDIRECT_LIMIT {
                    return attempt.error(format!("redirected more than {REDIRECT_LIMIT} times"));
                }
                match crate::config::cleartext_internet_host(attempt.url()) {
                    Some(host) => {
                        let reason = format!("redirected to plain http at {host}");
                        attempt.error(reason)
                    }
                    None => attempt.follow(),
                }
            }))
            .build()
    };
    std::thread::spawn(build)
        .join()
        .map_err(|_| {
            BansheeError::Other(format!(
                "the client for the remote {} panicked",
                side.side()
            ))
        })?
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

    /// Answers `times` requests with `head` and no body. Loopback only.
    fn answering(head: String, times: usize) -> (String, std::thread::JoinHandle<()>) {
        use std::io::Write;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            for _ in 0..times {
                let (mut stream, _) = listener.accept().unwrap();
                crate::test_support::read_request(&mut stream);
                let response = format!("{head}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        (base_url, handle)
    }

    fn redirect_to(location: &str) -> String {
        format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}")
    }

    #[test]
    fn a_redirect_that_stays_on_your_network_is_followed() {
        let (target, served) = answering("HTTP/1.1 200 OK".to_string(), 1);
        let (base_url, redirected) = answering(redirect_to(&format!("{target}/models")), 1);
        let client = build_client(RemoteKey::Stt, &base_url, CONNECT_TIMEOUT).unwrap();
        let response = client.get(format!("{base_url}/models")).send().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        redirected.join().unwrap();
        served.join().unwrap();
    }

    /// The server answers exactly the limit, so a policy that never stops makes
    /// one more request and meets a closed port instead.
    #[test]
    fn a_redirect_loop_stops_at_the_limit() {
        let (base_url, redirected) = answering(redirect_to("/v1/models"), REDIRECT_LIMIT);
        let client = build_client(RemoteKey::Stt, &base_url, CONNECT_TIMEOUT).unwrap();
        let error = client
            .get(format!("{base_url}/models"))
            .send()
            .expect_err("a loop must not be followed for ever");
        redirected.join().unwrap();
        let sentence = credentials::describe_send_error("127.0.0.1", &error);
        assert!(
            sentence.contains("redirected more than 10 times"),
            "{sentence}"
        );
    }

    /// 203.0.113.1 is a documentation address: should the policy break, the
    /// client waits out the connect bound and sends nothing anywhere.
    #[test]
    fn a_redirect_to_plain_http_on_the_internet_is_not_followed() {
        let (base_url, redirected) = answering(redirect_to("http://203.0.113.1/v1/models"), 1);
        let client = build_client(RemoteKey::Stt, &base_url, CONNECT_TIMEOUT).unwrap();
        let error = client
            .get(format!("{base_url}/models"))
            .send()
            .expect_err("the redirect must not be followed");
        redirected.join().unwrap();
        let sentence = credentials::describe_send_error("127.0.0.1", &error);
        assert!(
            sentence.contains("redirected to plain http at 203.0.113.1"),
            "{sentence}"
        );
    }

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

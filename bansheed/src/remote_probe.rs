use reqwest::blocking::Client;

use crate::credentials;
use crate::speech_to_text::remote::openai_compatible::CONNECT_TIMEOUT;

/// What one remote server says when it is asked whether the key works.
#[derive(Debug, PartialEq)]
pub enum Probe {
    Answers,
    KeyRefused,
    NoModelsPath,
    Failed(u16),
    Unreachable(String),
}

/// GET `<base_url>/models` with the key. One request, the connect bound the
/// transcriber uses, and the same bound for the answer.
pub fn probe(base_url: &str, api_key: &str) -> Probe {
    // On a thread of its own, and joined, because reqwest's blocking client
    // makes and drops a temporary runtime, and a debug build of tokio refuses
    // that drop on a worker. The checklist runs on one.
    std::thread::scope(|scope| scope.spawn(|| ask(base_url, api_key)).join().unwrap())
}

fn ask(base_url: &str, api_key: &str) -> Probe {
    let host = crate::config::host_of(base_url);
    let client = match Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(CONNECT_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(error) => return Probe::Unreachable(credentials::describe_send_error(&host, &error)),
    };
    let endpoint = format!("{}/models", base_url.trim_end_matches('/'));
    // The body is never read: a refusal is the likeliest place for a server to
    // echo the key it refused, and the status says all the checklist reports.
    let response = match client.get(endpoint).bearer_auth(api_key).send() {
        Ok(response) => response,
        Err(error) => return Probe::Unreachable(credentials::describe_send_error(&host, &error)),
    };
    match response.status().as_u16() {
        401 | 403 => Probe::KeyRefused,
        404 => Probe::NoModelsPath,
        200..=299 => Probe::Answers,
        code => Probe::Failed(code),
    }
}

#[cfg(test)]
mod tests {
    use super::{Probe, probe};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Shaped like a key and issued by nobody.
    const FAKE_KEY: &str = "sk-proj-7Qm4Xb2vR8tL1yWn3cZa";

    /// Answers one HTTP request with `status` and `body`, then returns the raw
    /// request it read. Loopback only: nothing leaves this machine in a test.
    fn serve_once(
        status: &'static str,
        body: &'static str,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 4096];
            // A GET carries no body, so the blank line ends the request
            while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&buffer).to_string()
        });
        (base_url, handle)
    }

    #[test]
    fn a_server_that_answers_passes() {
        let (base_url, served) = serve_once("200 OK", r#"{"data": []}"#);
        assert_eq!(probe(&base_url, FAKE_KEY), Probe::Answers);
        served.join().unwrap();
    }

    #[test]
    fn a_refused_key_is_named_as_such() {
        let (base_url, served) = serve_once("401 Unauthorized", r#"{"error": "bad key"}"#);
        assert_eq!(probe(&base_url, FAKE_KEY), Probe::KeyRefused);
        served.join().unwrap();
    }

    #[test]
    fn a_server_without_a_models_path_is_a_note() {
        let (base_url, served) = serve_once("404 Not Found", "");
        assert_eq!(probe(&base_url, FAKE_KEY), Probe::NoModelsPath);
        served.join().unwrap();
    }

    #[test]
    fn a_server_error_is_a_failure_and_names_the_code() {
        let (base_url, served) = serve_once("500 Internal Server Error", "");
        assert_eq!(probe(&base_url, FAKE_KEY), Probe::Failed(500));
        served.join().unwrap();
    }

    #[test]
    fn a_closed_port_names_the_host() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        drop(listener);
        let Probe::Unreachable(reason) = probe(&base_url, FAKE_KEY) else {
            panic!("a closed port is unreachable");
        };
        assert!(reason.contains("127.0.0.1"), "{reason}");
        assert!(!reason.contains(FAKE_KEY), "the key is in the reason");
    }

    #[test]
    fn the_request_carries_the_key_as_a_bearer_header_and_asks_for_models() {
        let (base_url, served) = serve_once("200 OK", r#"{"data": []}"#);
        // A trailing slash on the base URL leaves the same path
        assert_eq!(probe(&format!("{base_url}/"), FAKE_KEY), Probe::Answers);
        let request = served.join().unwrap();
        assert!(request.starts_with("GET /v1/models HTTP/1.1"), "{request}");
        assert!(
            request.contains(&format!("authorization: Bearer {FAKE_KEY}"))
                || request.contains(&format!("Authorization: Bearer {FAKE_KEY}")),
            "{request}"
        );
    }
}

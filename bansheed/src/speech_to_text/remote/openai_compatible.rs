use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use banshee_common::error::BansheeError;
use reqwest::blocking::multipart;

use banshee_common::listening::ListeningModel;

use crate::config::RemoteSttConfig;
use crate::credentials::{self, RemoteKey};
use crate::speech_to_text::{SAMPLE_RATE, Speech, Transcriber};

use super::wav::pcm16_wav;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// One OpenAI-compatible transcription server. The listener thread waits on
/// each request, which runs on a runtime of its own so a stop can drop it
/// mid-upload. It holds the key file's path and never the key.
pub struct RemoteTranscriber {
    client: reqwest::Client,
    base_url: String,
    model: String,
    credentials: PathBuf,
    prompt: Option<String>,
    speech: Speech,
}

#[derive(serde::Deserialize)]
struct Reply {
    text: String,
}

fn prompt_for(vocabulary: &[String]) -> Option<String> {
    (!vocabulary.is_empty()).then(|| vocabulary.join(", "))
}

impl RemoteTranscriber {
    pub fn new(
        remote: &RemoteSttConfig,
        credentials: PathBuf,
        vocabulary: &[String],
        speech: Speech,
    ) -> Result<Self, BansheeError> {
        Ok(Self {
            client: crate::remote::async_builder(RemoteKey::Stt, &remote.base_url)?
                .timeout(REQUEST_TIMEOUT)
                .build()
                .map_err(|error| BansheeError::Other(error.to_string()))?,
            base_url: remote.base_url.trim_end_matches('/').to_string(),
            model: remote.model.clone(),
            credentials,
            prompt: prompt_for(vocabulary),
            speech,
        })
    }

    fn endpoint(&self) -> String {
        let path = if self.speech.translate {
            "translations"
        } else {
            "transcriptions"
        };
        format!("{}/audio/{path}", self.base_url)
    }

    /// The name a person recognises, read off the URL.
    fn host(&self) -> String {
        crate::config::host_of(&self.base_url)
    }

    /// The key on file, read for one request.
    fn key(&self) -> Result<String, BansheeError> {
        credentials::Credentials::key_at(&self.credentials, RemoteKey::Stt)
            .map_err(|error| BansheeError::Transcription(error.to_string()))?
            .ok_or_else(|| BansheeError::Transcription(RemoteKey::Stt.no_key()))
    }

    /// The multipart body and its content type.
    fn form(&self, audio: &[f32]) -> Result<(String, Vec<u8>), BansheeError> {
        let file = multipart::Part::bytes(pcm16_wav(audio, SAMPLE_RATE))
            .file_name("utterance.wav")
            .mime_str("audio/wav")
            .map_err(|error| BansheeError::Other(error.to_string()))?;
        let mut form = multipart::Form::new()
            .part("file", file)
            .text("model", self.model.clone())
            .text("response_format", "json");
        if let Some(prompt) = &self.prompt {
            form = form.text("prompt", prompt.clone());
        }
        let content_type = format!("multipart/form-data; boundary={}", form.boundary());
        let mut body = Vec::new();
        form.into_reader()
            .read_to_end(&mut body)
            .map_err(|error| BansheeError::Other(error.to_string()))?;
        Ok((content_type, body))
    }

    async fn request(
        &self,
        api_key: &str,
        content_type: String,
        body: Vec<u8>,
    ) -> Result<String, BansheeError> {
        let response = self
            .client
            .post(self.endpoint())
            .bearer_auth(api_key)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body)
            .send()
            .await
            .map_err(|error| {
                BansheeError::Transcription(credentials::describe_send_error(&self.host(), &error))
            })?;

        let status = response.status();
        if !status.is_success() {
            // The body may name the fault; it goes to the log, never to a person
            let said = if credentials::may_read_body(status) {
                Some(credentials::read_body(response).await)
            } else {
                None
            };
            log::warn!(
                "{}",
                crate::remote::log_line(RemoteKey::Stt, status, said.as_deref(), api_key)
            );
            return Err(BansheeError::Transcription(crate::remote::describe_status(
                RemoteKey::Stt,
                status,
            )));
        }
        let reply: Reply = response.json().await.map_err(|error| {
            log::error!(
                "{} answered something that was not a transcription: {error}",
                self.host()
            );
            BansheeError::Transcription(
                "the remote listener answered something that was not a transcription".to_string(),
            )
        })?;
        Ok(reply.text.trim().to_string())
    }
}

impl Transcriber for RemoteTranscriber {
    fn transcribe(&mut self, audio: &[f32]) -> Result<String, BansheeError> {
        self.transcribe_until(audio, &tokio::sync::Notify::new())
    }

    fn transcribe_until(
        &mut self,
        audio: &[f32],
        stop: &tokio::sync::Notify,
    ) -> Result<String, BansheeError> {
        let api_key = self.key()?;
        let (content_type, body) = self.form(audio)?;
        let runtime = crate::remote::request_runtime()
            .map_err(|error| BansheeError::Other(error.to_string()))?;
        let answered = crate::remote::run_until_stopped(&runtime, stop, || {
            self.request(&api_key, content_type, body)
        });
        // Dropping the runtime closes the connection, so on a stop the rest of
        // the upload never leaves. What the system already holds to send
        // still goes.
        drop(runtime);
        answered.unwrap_or_else(|| {
            Err(BansheeError::Transcription(
                "the request to the remote listener was stopped".to_string(),
            ))
        })
    }

    fn set_vocabulary(&mut self, words: &[String]) {
        self.prompt = prompt_for(words);
    }

    fn set_speech(&mut self, speech: Speech) {
        self.speech = speech;
    }

    fn reload(
        &mut self,
        _model: &'static ListeningModel,
    ) -> Result<Option<&'static ListeningModel>, BansheeError> {
        Ok(None)
    }

    fn sends_out(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::RemoteTranscriber;
    use crate::config::RemoteSttConfig;
    use crate::speech_to_text::{Speech, Transcriber};
    use banshee_common::error::BansheeError;
    use std::io::Write;
    use std::net::TcpListener;

    struct Served {
        request: String,
    }

    /// Answers one HTTP request with `status` and `body`, then returns the raw
    /// request it read. Loopback only: nothing leaves this machine in a test.
    fn serve_once(
        status: &'static str,
        body: &'static str,
    ) -> (String, std::thread::JoinHandle<Served>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = crate::test_support::read_request(&mut stream);
            stream
                .write_all(json_response(status, body).as_bytes())
                .unwrap();
            Served { request }
        });
        (base_url, handle)
    }

    /// One HTTP answer carrying `body` as JSON, after which the server closes.
    fn json_response(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn transcriber(base_url: String, translate: bool) -> RemoteTranscriber {
        built_transcriber(base_url, translate).unwrap()
    }

    fn built_transcriber(
        base_url: String,
        translate: bool,
    ) -> Result<RemoteTranscriber, BansheeError> {
        let remote = RemoteSttConfig {
            base_url,
            model: "whisper-1".to_string(),
        };
        RemoteTranscriber::new(
            &remote,
            crate::test_support::key_file_holding(crate::credentials::RemoteKey::Stt, "sk-test"),
            &["banshee".to_string(), "tokio".to_string()],
            Speech { translate },
        )
    }

    #[test]
    fn a_transcription_posts_the_wav_the_model_and_the_prompt_with_the_key_and_no_language() {
        let (base_url, served) = serve_once("200 OK", r#"{"text": " hallo welt "}"#);
        let text = transcriber(base_url, false)
            .transcribe(&[0.0; 1600])
            .unwrap();
        assert_eq!(text, "hallo welt");

        let request = served.join().unwrap().request;
        assert!(
            request.starts_with("POST /v1/audio/transcriptions HTTP/1.1"),
            "{request}"
        );
        assert!(
            request.contains("authorization: Bearer sk-test")
                || request.contains("Authorization: Bearer sk-test")
        );
        assert!(request.contains("name=\"file\"; filename=\"utterance.wav\""));
        assert!(request.contains("name=\"model\"\r\n\r\nwhisper-1"));
        assert!(!request.contains("name=\"language\""), "{request}");
        assert!(request.contains("name=\"prompt\"\r\n\r\nbanshee, tokio"));
        assert!(request.contains("name=\"response_format\"\r\n\r\njson"));
        assert!(
            request.contains("RIFF"),
            "the WAV body must be in the request"
        );
    }

    #[test]
    fn translating_goes_to_the_translations_path_without_a_language() {
        let (base_url, served) = serve_once("200 OK", r#"{"text": "hello world"}"#);
        transcriber(base_url, true).transcribe(&[0.0; 160]).unwrap();
        let request = served.join().unwrap().request;
        assert!(
            request.starts_with("POST /v1/audio/translations HTTP/1.1"),
            "{request}"
        );
        assert!(!request.contains("name=\"language\""));
    }

    #[test]
    fn a_malformed_success_body_names_no_raw_error() {
        let (base_url, served) = serve_once("200 OK", "not json");
        let error = transcriber(base_url, false)
            .transcribe(&[0.0; 160])
            .unwrap_err();
        served.join().unwrap();
        assert!(
            error
                .to_string()
                .ends_with("the remote listener answered something that was not a transcription"),
            "{error}"
        );
        assert!(!error.to_string().contains("http"), "{error}");
        assert!(!error.to_string().contains("sk-test"), "{error}");
    }

    #[test]
    fn a_refused_key_is_named_as_such() {
        let (base_url, served) = serve_once("401 Unauthorized", r#"{"error": "bad key"}"#);
        let error = transcriber(base_url, false)
            .transcribe(&[0.0; 160])
            .unwrap_err();
        served.join().unwrap();
        assert!(error.to_string().contains("refused the key"), "{error}");
    }

    #[test]
    fn an_unknown_model_or_path_is_named_as_such() {
        let (base_url, served) = serve_once("404 Not Found", "");
        let error = transcriber(base_url, false)
            .transcribe(&[0.0; 160])
            .unwrap_err();
        served.join().unwrap();
        assert!(
            error.to_string().contains("no such model or path"),
            "{error}"
        );
    }

    #[test]
    fn a_server_that_is_not_there_names_the_host() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        drop(listener);
        let error = transcriber(base_url, false)
            .transcribe(&[0.0; 160])
            .unwrap_err();
        assert!(error.to_string().contains("127.0.0.1"), "{error}");
        assert!(
            error.to_string().contains("could not be reached"),
            "{error}"
        );
    }

    /// A server that keeps the first connection open, then closes it while
    /// it idles, before the next request.
    #[test]
    fn a_connection_the_server_closed_while_idle_does_not_fail_the_next_request() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", server.local_addr().unwrap());
        let answer = |stream: &mut std::net::TcpStream, close: bool| {
            let body = r#"{"text": "ok"}"#;
            let connection = if close { "Connection: close\r\n" } else { "" };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{connection}\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        };
        let served = std::thread::spawn(move || {
            let (mut first, _) = server.accept().unwrap();
            crate::test_support::read_request(&mut first);
            answer(&mut first, false);
            std::thread::sleep(std::time::Duration::from_millis(50));
            drop(first);
            server.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while std::time::Instant::now() < deadline {
                if let Ok((mut second, _)) = server.accept() {
                    second.set_nonblocking(false).unwrap();
                    crate::test_support::read_request(&mut second);
                    answer(&mut second, true);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        });
        let mut listener = transcriber(base_url, false);
        assert_eq!(listener.transcribe(&[0.0; 160]).unwrap(), "ok");
        std::thread::sleep(std::time::Duration::from_millis(300));
        let second = listener.transcribe(&[0.0; 160]);
        served.join().unwrap();
        assert_eq!(second.unwrap(), "ok");
    }

    #[tokio::test]
    async fn the_client_builds_inside_a_runtime() {
        RemoteTranscriber::new(
            &RemoteSttConfig::default(),
            crate::test_support::key_file_holding(crate::credentials::RemoteKey::Stt, "sk-test"),
            &[],
            Speech { translate: false },
        )
        .expect("the client must build where the daemon builds it");
    }

    #[test]
    fn plain_http_to_the_internet_builds_no_listener() {
        let Err(error) = built_transcriber("http://api.example.com/v1".to_string(), false) else {
            panic!("plain http to the internet must build no listener");
        };
        assert!(
            error
                .to_string()
                .contains("stt.remote.base_url needs https for api.example.com"),
            "{error}"
        );
    }

    #[test]
    fn a_transcription_with_no_key_on_file_sends_nothing_and_names_the_key() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let remote = RemoteSttConfig {
            base_url: format!("http://{}/v1", server.local_addr().unwrap()),
            model: "whisper-1".to_string(),
        };
        let keys = crate::test_support::unique_scratch("keys").join("credentials.toml");
        let mut listener =
            RemoteTranscriber::new(&remote, keys, &[], Speech { translate: false }).unwrap();
        let error = listener
            .transcribe(&[0.0; 1600])
            .expect_err("no key is on file");
        assert!(error.to_string().contains("stt.remote.api_key"), "{error}");
        server.set_nonblocking(true).unwrap();
        assert!(server.accept().is_err(), "nothing connected to the server");
    }

    #[test]
    fn a_key_written_between_two_requests_is_the_key_the_second_sends() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let remote = RemoteSttConfig {
            base_url: format!("http://{}/v1", server.local_addr().unwrap()),
            model: "whisper-1".to_string(),
        };
        let served = std::thread::spawn(move || {
            (0..2)
                .map(|_| {
                    let (mut stream, _) = server.accept().unwrap();
                    let request = crate::test_support::read_request(&mut stream);
                    stream
                        .write_all(json_response("200 OK", r#"{"text": "ok"}"#).as_bytes())
                        .unwrap();
                    request.to_lowercase()
                })
                .collect::<Vec<_>>()
        });
        let keys =
            crate::test_support::key_file_holding(crate::credentials::RemoteKey::Stt, "sk-first");
        let mut listener =
            RemoteTranscriber::new(&remote, keys.clone(), &[], Speech { translate: false })
                .unwrap();
        listener.transcribe(&[0.0; 160]).unwrap();
        crate::test_support::write_key(&keys, crate::credentials::RemoteKey::Stt, "sk-second");
        listener.transcribe(&[0.0; 160]).unwrap();

        let requests = served.join().unwrap();
        assert!(
            requests[0].contains("authorization: bearer sk-first"),
            "{}",
            requests[0]
        );
        assert!(
            requests[1].contains("authorization: bearer sk-second"),
            "{}",
            requests[1]
        );
    }

    #[test]
    fn reload_loads_no_file() {
        let (base_url, _served) = serve_once("200 OK", "{}");
        let mut remote = transcriber(base_url, false);
        let turbo = banshee_common::listening::find("whisper-large-v3-turbo").unwrap();
        assert_eq!(remote.reload(turbo).unwrap(), None);
    }
}

use std::io::Read;
use std::time::Duration;

use banshee_common::error::BansheeError;
use reqwest::blocking::{Client, multipart};

use crate::config::{RemoteSttConfig, STTPreset};
use crate::credentials::{self, RemoteKey};
use crate::speech_to_text::{SAMPLE_RATE, Speech, Transcriber};

use super::wav::pcm16_wav;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// One OpenAI-compatible transcription server. Blocking, because the listener
/// thread owns it and an utterance is one request.
pub struct RemoteTranscriber {
    client: Client,
    base_url: String,
    model: String,
    api_key: String,
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
        api_key: String,
        vocabulary: &[String],
        speech: Speech,
    ) -> Result<Self, BansheeError> {
        Ok(Self {
            client: crate::remote::build_client(REQUEST_TIMEOUT, "listener")?,
            base_url: remote.base_url.trim_end_matches('/').to_string(),
            model: remote.model.clone(),
            api_key,
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
}

impl Transcriber for RemoteTranscriber {
    fn transcribe(&self, audio: &[f32]) -> Result<String, BansheeError> {
        let file = multipart::Part::bytes(pcm16_wav(audio, SAMPLE_RATE))
            .file_name("utterance.wav")
            .mime_str("audio/wav")
            .map_err(|error| BansheeError::Other(error.to_string()))?;
        let mut form = multipart::Form::new()
            .part("file", file)
            .text("model", self.model.clone())
            .text("response_format", "json");
        // The translations endpoint takes no language: the answer is English
        if let Some(language) = &self.speech.language
            && !self.speech.translate
        {
            form = form.text("language", language.clone());
        }
        if let Some(prompt) = &self.prompt {
            form = form.text("prompt", prompt.clone());
        }

        // One buffer, not the form's reader: the blocking client reports a
        // streamed body's channel closing ahead of a refused connection.
        let content_type = format!("multipart/form-data; boundary={}", form.boundary());
        let mut body = Vec::new();
        form.into_reader()
            .read_to_end(&mut body)
            .map_err(|error| BansheeError::Other(error.to_string()))?;
        let response = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.api_key)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body)
            .send()
            .map_err(|error| {
                BansheeError::Transcription(credentials::describe_send_error(&self.host(), &error))
            })?;

        let status = response.status();
        if !status.is_success() {
            // The body may name the fault; it goes to the log, never to a person
            let said = credentials::may_read_body(status).then(|| credentials::read_body(response));
            log::warn!(
                "{}",
                crate::remote::log_line(RemoteKey::Stt, status, said.as_deref(), &self.api_key)
            );
            return Err(BansheeError::Transcription(crate::remote::describe_status(
                RemoteKey::Stt,
                status,
            )));
        }
        let reply: Reply = response.json().map_err(|error| {
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

    fn set_vocabulary(&mut self, words: &[String]) {
        self.prompt = prompt_for(words);
    }

    fn set_speech(&mut self, speech: Speech) {
        self.speech = speech;
    }

    fn reload(&mut self, _preset: STTPreset) -> Result<Option<&'static str>, BansheeError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::RemoteTranscriber;
    use crate::config::RemoteSttConfig;
    use crate::speech_to_text::{Speech, Transcriber};
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
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            Served { request }
        });
        (base_url, handle)
    }

    fn transcriber(base_url: String, translate: bool) -> RemoteTranscriber {
        let remote = RemoteSttConfig {
            base_url,
            model: "whisper-1".to_string(),
        };
        RemoteTranscriber::new(
            &remote,
            "sk-test".to_string(),
            &["banshee".to_string(), "tokio".to_string()],
            Speech {
                language: Some("de".to_string()),
                translate,
            },
        )
        .unwrap()
    }

    #[test]
    fn a_transcription_posts_the_wav_the_model_the_language_and_the_prompt_with_the_key() {
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
        assert!(request.contains("name=\"language\"\r\n\r\nde"));
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

    #[tokio::test]
    async fn the_client_builds_inside_a_runtime() {
        RemoteTranscriber::new(
            &RemoteSttConfig::default(),
            "sk-test".to_string(),
            &[],
            Speech {
                language: None,
                translate: false,
            },
        )
        .expect("the client must build where the daemon builds it");
    }

    #[test]
    fn reload_loads_no_file() {
        let (base_url, _served) = serve_once("200 OK", "{}");
        let mut remote = transcriber(base_url, false);
        assert_eq!(remote.reload(crate::config::STTPreset::Fast).unwrap(), None);
    }
}

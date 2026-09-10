use std::io::Read;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use banshee_common::error::BansheeError;
use reqwest::blocking::Client;

use crate::config::{RemoteTtsConfig, SpeechFormat, TTSConfig};
use crate::credentials;
use crate::text_to_speech::output::Output;
use crate::text_to_speech::remote::arrival::{Arrival, declared};
use crate::text_to_speech::{ActiveUtterance, Fault, TtsBackend, lock};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
// Between bytes and not in total: an utterance may run for minutes, and a
// server that has stopped sending is the only failure a bound can catch. The
// test bound is short, because a fifteen-second test is a test nobody runs, and
// the fixture divides it for the pause it leaves between two chunks.
const READ_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_secs(1)
} else {
    Duration::from_secs(15)
};
const READ_SIZE: usize = 8 * 1024;
/// Long enough for a server that says what to do about the refusal: the
/// longest message met so far is 188 characters. Short enough for the status
/// line and the Voice panel to stay readable.
const MESSAGE_LIMIT: usize = 240;
/// How far back the search for a word end may reach: the last quarter of what
/// the cap kept. A space further back than that is no word end near the cut,
/// and a cut there would answer a couple of characters in place of a message.
const CUT_WINDOW: usize = 4;

/// One OpenAI-compatible speech server. Blocking, because the utterance owns a
/// thread and the response's `Read` is how the stream is consumed.
pub struct RemoteSpeechBackend {
    client: Client,
    base_url: String,
    host: String,
    model: String,
    voice: String,
    instructions: Option<String>,
    response_format: SpeechFormat,
    sample_rate: Option<std::num::NonZero<u32>>,
    speed: RwLock<f32>,
    api_key: String,
    output: Arc<Output>,
    fallback: Option<Arc<dyn TtsBackend>>,
    faults: std::sync::mpsc::Sender<Fault>,
}

/// Built on a thread of its own, and joined, because reqwest's blocking builder
/// makes and drops a temporary runtime, and a debug build of tokio refuses that
/// drop on a worker.
fn build_client() -> Result<Client, BansheeError> {
    std::thread::spawn(|| {
        Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(READ_TIMEOUT)
            .build()
    })
    .join()
    .map_err(|_| BansheeError::Other("the client for the remote speaker panicked".to_string()))?
    .map_err(|error| BansheeError::Other(error.to_string()))
}

/// The sentence a status carries on its own, for an answer whose body names no
/// fault.
fn describe_status(status: reqwest::StatusCode) -> String {
    match status.as_u16() {
        401 | 403 => "the remote speaker refused the key".to_string(),
        404 => "the remote speaker has no such model or voice".to_string(),
        429 => "the remote speaker asked to slow down".to_string(),
        code => format!("the remote speaker answered {code}"),
    }
}

/// The reason a refused answer leaves. The server's own words win, because a
/// status code says nothing a person can act on. `said` is the redacted body,
/// or nothing for a body that was never read.
fn describe_refusal(
    host: &str,
    status: reqwest::StatusCode,
    said: Option<&str>,
    api_key: &str,
) -> String {
    match said.and_then(|body| server_message(body, api_key)) {
        Some(message) => format!("{host} says {message}"),
        None => describe_status(status),
    }
}

/// What a non-success answer leaves in the daemon log, on one line.
fn log_line(status: reqwest::StatusCode, said: Option<&str>) -> String {
    match said {
        Some(body) => format!(
            "banshee: the remote speaker answered {status}: {}",
            credentials::one_line(body)
        ),
        None => format!("banshee: the remote speaker answered {status}"),
    }
}

/// The server's own words, from the shapes an OpenAI-compatible server refuses
/// in. The first shape that carries a non-empty string wins. Redaction runs
/// again here, because a key written as a JSON escape is not in the raw body
/// the first pass read.
fn server_message(body: &str, api_key: &str) -> Option<String> {
    let answer: serde_json::Value = serde_json::from_str(body).ok()?;
    let said = [
        "/error/message",
        "/error",
        "/detail/error",
        "/detail",
        "/message",
    ]
    .into_iter()
    .filter_map(|shape| answer.pointer(shape))
    .find_map(|value| {
        value
            .as_str()
            .map(str::trim)
            .filter(|said| !said.is_empty())
    })?;
    Some(shortened(&credentials::redacted(said, api_key)))
}

/// A message the status line and the Voice panel can hold, on one line.
fn shortened(message: &str) -> String {
    let said = credentials::one_line(message);
    if said.chars().count() <= MESSAGE_LIMIT {
        return said;
    }
    let kept: String = said.chars().take(MESSAGE_LIMIT - 1).collect();
    let cut = kept
        .char_indices()
        .rev()
        .take(kept.chars().count() / CUT_WINDOW)
        .find(|(_, character)| *character == ' ')
        .map_or(kept.len(), |(at, _)| at);
    format!("{}…", &kept[..cut])
}

impl RemoteSpeechBackend {
    pub fn new(
        remote: &RemoteTtsConfig,
        api_key: String,
        speed: f32,
        fallback: Option<Arc<dyn TtsBackend>>,
        output: Arc<Output>,
        faults: std::sync::mpsc::Sender<Fault>,
    ) -> Result<Self, BansheeError> {
        if remote.voice.is_empty() {
            return Err(BansheeError::Other(
                "the remote speaker has no voice; name one with: banshee config set tts.remote.voice"
                    .to_string(),
            ));
        }
        let base_url = remote.base_url.trim_end_matches('/').to_string();
        Ok(Self {
            client: build_client()?,
            host: crate::config::host_of(&base_url),
            base_url,
            model: remote.model.clone(),
            voice: remote.voice.clone(),
            instructions: (!remote.instructions.is_empty()).then(|| remote.instructions.clone()),
            response_format: remote.response_format,
            sample_rate: remote.sample_rate,
            speed: RwLock::new(speed),
            api_key,
            output,
            fallback,
            faults,
        })
    }

    fn body(&self, text: &str, voice: Option<&str>) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.model,
            "input": text,
            "voice": voice.unwrap_or(&self.voice),
            "response_format": self.response_format,
            "speed": *self.speed.read().unwrap_or_else(|poison| poison.into_inner()),
        });
        if let Some(instructions) = &self.instructions {
            body["instructions"] = instructions.clone().into();
        }
        // Sent only when the user named one: a server that takes no such field
        // may refuse the whole request over it
        if let Some(rate) = self.sample_rate {
            body["sample_rate"] = rate.get().into();
        }
        body
    }

    /// Sends the request and hands back the body to read, or the reason it
    /// failed before a single byte of audio.
    fn open(&self, text: &str, voice: Option<&str>) -> Result<reqwest::blocking::Response, String> {
        let response = self
            .client
            .post(format!("{}/audio/speech", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&self.body(text, voice))
            .send()
            .map_err(|error| credentials::describe_send_error(&self.host, &error))?;
        let status = response.status();
        if !status.is_success() {
            // A refused key is the one body nothing reads: it is the likeliest
            // place for a server to echo the key back, and the sentence the
            // status carries is already the whole fault
            let said = credentials::may_read_body(status)
                .then(|| credentials::redacted(&credentials::read_body(response), &self.api_key));
            eprintln!("{}", log_line(status, said.as_deref()));
            return Err(describe_refusal(
                &self.host,
                status,
                said.as_deref(),
                &self.api_key,
            ));
        }
        // A 200 that carries an error body says so here, before a byte of it
        // is read as audio
        declared(
            response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
        )?;
        Ok(response)
    }
}

/// The response being read, and what its bytes have said so far.
struct Reading {
    body: reqwest::blocking::Response,
    arrival: Arrival,
}

impl TtsBackend for RemoteSpeechBackend {
    /// Stores the rate and answers the voice utterances now speak in. The
    /// `[tts.remote]` keys are read when the daemon starts, so the voice does
    /// not move here.
    fn reconfigure(&self, tts: &TTSConfig) -> Option<String> {
        *self
            .speed
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = tts.speed;
        Some(self.voice.clone())
    }

    fn start(&self, text: &str, voice: Option<&str>) -> std::io::Result<Box<dyn ActiveUtterance>> {
        Ok(Box::new(self.speak(text, voice)))
    }
}

impl RemoteSpeechBackend {
    /// The utterance unboxed: `start` boxes it for the trait.
    fn speak(&self, text: &str, voice: Option<&str>) -> RemoteUtterance {
        let handover: Arc<Mutex<Handover>> = Arc::default();
        let worker_handover = Arc::clone(&handover);
        let fallback = self.fallback.clone();
        let faults = self.faults.clone();
        let text = text.to_string();
        let voice = voice.map(str::to_string);
        // Cloned rather than borrowed: the chunk iterator outlives this call
        let opener = self.clone_for_worker();

        let mut reading: Option<Reading> = None;
        let mut played = false;
        let mut buffer = [0u8; READ_SIZE];
        // The two numbers the proof records: how long the first words waited,
        // and how long the whole reply took.
        let started = std::time::Instant::now();
        let chunks = std::iter::from_fn(move || {
            // Every failure before the first sample: say why, and let the
            // fallback speak the sentence nobody has heard yet.
            let unheard = |reason: String| {
                // Under the one lock a stop takes, so the fallback never
                // starts on a sentence nothing can stop
                let mut handover = lock(&worker_handover);
                if handover.stopped {
                    return;
                }
                let _ = faults.send(Fault::Failed(reason));
                if let Some(fallback) = &fallback
                    && let Ok(utterance) = fallback.start(&text, None)
                {
                    handover.speaking = Some(utterance);
                }
            };
            loop {
                let source = match &mut reading {
                    Some(source) => source,
                    None => match opener.open(&text, voice.as_deref()) {
                        Ok(body) => reading.insert(Reading {
                            body,
                            arrival: Arrival::new(opener.response_format, opener.sample_rate),
                        }),
                        Err(reason) => {
                            unheard(reason);
                            return None;
                        }
                    },
                };
                let read = match source.body.read(&mut buffer) {
                    Ok(0) => {
                        match source.arrival.ended() {
                            // `ended` answers Ok for a body that said what it
                            // was, which a header with no samples behind it did
                            Ok(()) if !played => {
                                unheard(format!("{} sent no audio", opener.host));
                            }
                            Ok(()) => println!(
                                "Spoke through {} in {:.2}s",
                                opener.host,
                                started.elapsed().as_secs_f32()
                            ),
                            Err(reason) => unheard(reason),
                        }
                        return None;
                    }
                    Ok(read) => read,
                    Err(error) => {
                        let reason = format!("{} stopped answering: {error}", opener.host);
                        // Words already heard are not spoken again in another
                        // voice, so only a silent failure reaches the fallback
                        if played {
                            if !lock(&worker_handover).stopped {
                                let _ = faults.send(Fault::Failed(reason));
                            }
                        } else {
                            unheard(reason);
                        }
                        return None;
                    }
                };
                // A refusal can only land before the first sample, because
                // the answer is identified once and then only decoded
                let chunk = match source.arrival.feed(&buffer[..read]) {
                    Ok(None) => continue,
                    Ok(Some(chunk)) => chunk,
                    Err(reason) => {
                        unheard(reason);
                        return None;
                    }
                };
                if !played {
                    played = true;
                    println!(
                        "First audio from {} in {:.2}s",
                        opener.host,
                        started.elapsed().as_secs_f32()
                    );
                    let _ = faults.send(Fault::Played);
                }
                return Some(chunk);
            }
        });

        RemoteUtterance {
            player: self.output.play(chunks),
            handover,
        }
    }

    /// Everything the worker needs to send the request, without the fallback it
    /// must not touch. The client is an `Arc` inside, so this clone shares the
    /// connection pool.
    fn clone_for_worker(&self) -> Self {
        Self {
            client: self.client.clone(),
            base_url: self.base_url.clone(),
            host: self.host.clone(),
            model: self.model.clone(),
            voice: self.voice.clone(),
            instructions: self.instructions.clone(),
            response_format: self.response_format,
            sample_rate: self.sample_rate,
            speed: RwLock::new(*self.speed.read().unwrap_or_else(|p| p.into_inner())),
            api_key: self.api_key.clone(),
            output: Arc::clone(&self.output),
            fallback: None,
            faults: self.faults.clone(),
        }
    }
}

/// What the worker may hand the utterance to, and whether a stop got there
/// first. One lock over both: a stop that lands between the two would leave the
/// system voice speaking a cancelled sentence, with nothing holding it.
#[derive(Default)]
struct Handover {
    stopped: bool,
    /// The system voice speaking instead, when the request failed before a byte.
    speaking: Option<Box<dyn ActiveUtterance>>,
}

struct RemoteUtterance {
    player: crate::text_to_speech::output::PlayerUtterance,
    handover: Arc<Mutex<Handover>>,
}

#[cfg(test)]
impl RemoteUtterance {
    fn queued(&self) -> usize {
        self.player.queued()
    }

    /// Every chunk that reached the player, with the shape it carried.
    fn heard(&self) -> Vec<crate::text_to_speech::output::Chunk> {
        self.player.heard()
    }

    /// The whole reply has been read.
    fn spoken(&self) -> bool {
        // A test cannot wait for `is_finished` here, because a silent output
        // never pulls the chunk it was handed
        self.player.worker_finished()
    }
}

impl ActiveUtterance for RemoteUtterance {
    fn is_finished(&mut self) -> bool {
        self.player.is_finished()
            && lock(&self.handover)
                .speaking
                .as_mut()
                .is_none_or(|utterance| utterance.is_finished())
    }

    fn stop(&mut self) {
        self.player.stop();
        let mut handover = lock(&self.handover);
        handover.stopped = true;
        if let Some(utterance) = handover.speaking.as_mut() {
            utterance.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{READ_TIMEOUT, RemoteSpeechBackend, lock};
    use crate::config::{RemoteTtsConfig, SpeechFormat};
    use crate::text_to_speech::output::Output;
    use crate::text_to_speech::{ActiveUtterance, Fault, TtsBackend};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// The pause the fixture leaves between two chunks. A fifth of the read
    /// bound, so a raised gap or a lowered bound cannot make the client read a
    /// pause as a server that stopped answering.
    const GAP: Duration = READ_TIMEOUT.checked_div(5).unwrap();

    /// Answers one HTTP request with `status`, then writes each piece of the
    /// body as its own chunk, `GAP` apart, and returns the raw request it read.
    /// Loopback only: nothing leaves this machine in a test.
    fn serve_speech(
        status: &'static str,
        pieces: Vec<Vec<u8>>,
        close_early: bool,
    ) -> (String, std::thread::JoinHandle<String>) {
        serve_typed(status, "audio/pcm", pieces, close_early)
    }

    fn serve_typed(
        status: &'static str,
        content_type: &'static str,
        pieces: Vec<Vec<u8>>,
        close_early: bool,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\n\r\n"
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.flush().unwrap();
            for (at, piece) in pieces.iter().enumerate() {
                if at > 0 {
                    std::thread::sleep(GAP);
                }
                let chunk = [format!("{:x}\r\n", piece.len()).as_bytes(), piece, b"\r\n"].concat();
                // A stopped utterance drops the connection, so a write that
                // fails here is the client leaving, not a fault
                if stream
                    .write_all(&chunk)
                    .and_then(|()| stream.flush())
                    .is_err()
                {
                    break;
                }
            }
            // A body that ends with no terminating chunk is a stream that died
            // mid-utterance, which is the one case no fallback may take over
            if !close_early {
                let _ = stream.write_all(b"0\r\n\r\n").and_then(|()| stream.flush());
            }
            request
        });
        (base_url, handle)
    }

    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut content_length = None;
        let mut header_end = None;
        loop {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if header_end.is_none()
                && let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n")
            {
                header_end = Some(end + 4);
                let head = String::from_utf8_lossy(&buffer[..end]).to_lowercase();
                content_length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok());
            }
            if let (Some(end), Some(length)) = (header_end, content_length)
                && buffer.len() >= end + length
            {
                break;
            }
        }
        String::from_utf8_lossy(&buffer).to_string()
    }

    /// What a fallback was asked to say. `SayBackend` would start a real `say`
    /// process, so the tests use this instead.
    #[derive(Default)]
    struct Spoken(Arc<Mutex<Vec<String>>>);

    struct Nothing;

    impl ActiveUtterance for Nothing {
        fn is_finished(&mut self) -> bool {
            true
        }
        fn stop(&mut self) {}
    }

    impl TtsBackend for Spoken {
        fn start(
            &self,
            text: &str,
            _voice: Option<&str>,
        ) -> std::io::Result<Box<dyn ActiveUtterance>> {
            self.0
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .push(text.to_string());
            Ok(Box::new(Nothing))
        }
    }

    /// A fallback whose sentence ends when the test says so, and what it was
    /// asked to say. `Spoken` finishes at once, which states no ordering.
    #[derive(Default)]
    struct HoldingFallback {
        said: Arc<Mutex<Vec<String>>>,
        ended: Arc<std::sync::atomic::AtomicBool>,
    }

    struct Held(Arc<std::sync::atomic::AtomicBool>);

    impl ActiveUtterance for Held {
        fn is_finished(&mut self) -> bool {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn stop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl TtsBackend for HoldingFallback {
        fn start(
            &self,
            text: &str,
            _voice: Option<&str>,
        ) -> std::io::Result<Box<dyn ActiveUtterance>> {
            lock(&self.said).push(text.to_string());
            Ok(Box::new(Held(Arc::clone(&self.ended))))
        }
    }

    fn table(base_url: String, instructions: &str) -> RemoteTtsConfig {
        RemoteTtsConfig {
            base_url,
            model: "gpt-4o-mini-tts".to_string(),
            voice: "marin".to_string(),
            instructions: instructions.to_string(),
            // The fixture writes bare samples, so the tests that read them ask
            // for the format that says they are bare
            response_format: SpeechFormat::Pcm,
            sample_rate: None,
        }
    }

    struct Built {
        backend: RemoteSpeechBackend,
        faults: std::sync::mpsc::Receiver<Fault>,
        fallback_said: Arc<Mutex<Vec<String>>>,
    }

    fn built(base_url: String, instructions: &str, with_fallback: bool) -> Built {
        build(table(base_url, instructions), with_fallback)
    }

    fn build(remote: RemoteTtsConfig, with_fallback: bool) -> Built {
        keyed(remote, with_fallback, "sk-test")
    }

    fn keyed(remote: RemoteTtsConfig, with_fallback: bool, api_key: &str) -> Built {
        let fallback_said: Arc<Mutex<Vec<String>>> = Arc::default();
        let fallback: Option<Arc<dyn TtsBackend>> = with_fallback
            .then(|| Arc::new(Spoken(Arc::clone(&fallback_said))) as Arc<dyn TtsBackend>);
        let (backend, faults) = assembled(&remote, api_key, fallback, Output::silent());
        Built {
            backend,
            faults,
            fallback_said,
        }
    }

    /// The backend and its fault channel, on whichever fallback and output the
    /// test hands it. A test that watches an utterance end needs an output that
    /// drains, and one that watches the ordering needs a fallback it can hold.
    fn assembled(
        remote: &RemoteTtsConfig,
        api_key: &str,
        fallback: Option<Arc<dyn TtsBackend>>,
        output: Output,
    ) -> (RemoteSpeechBackend, std::sync::mpsc::Receiver<Fault>) {
        let (sender, faults) = std::sync::mpsc::channel();
        let backend = RemoteSpeechBackend::new(
            remote,
            api_key.to_string(),
            1.2,
            fallback,
            Arc::new(output),
            sender,
        )
        .unwrap();
        (backend, faults)
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(
                std::time::Instant::now() < deadline,
                "{what} did not happen within 5s"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn pcm(values: &[i16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn failure(faults: &std::sync::mpsc::Receiver<Fault>) -> String {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match faults.recv_timeout(Duration::from_millis(100)) {
                Ok(Fault::Failed(reason)) => return reason,
                Ok(Fault::Played) => panic!("this utterance must not have played"),
                Err(_) => assert!(
                    std::time::Instant::now() < deadline,
                    "no reason arrived within 5s"
                ),
            }
        }
    }

    #[test]
    fn an_utterance_posts_the_whole_text_the_model_the_voice_the_rate_and_the_instructions() {
        let (base_url, served) = serve_speech("200 OK", vec![pcm(&[0, 1, 2, 3, 4, 5])], false);
        let built = built(base_url, "Calm and even", false);
        let utterance = built.backend.speak("One sentence. And a second.", None);
        wait_until("the reply is read", || utterance.spoken());

        let request = served.join().unwrap();
        assert!(
            request.starts_with("POST /v1/audio/speech HTTP/1.1"),
            "{request}"
        );
        assert!(
            request.contains("authorization: Bearer sk-test")
                || request.contains("Authorization: Bearer sk-test")
        );
        let body = request.split("\r\n\r\n").nth(1).expect("a JSON body");
        let sent: serde_json::Value = serde_json::from_str(body).expect("valid JSON");
        // One request per utterance: a per-sentence split would break the voice
        assert_eq!(sent["input"], "One sentence. And a second.");
        assert_eq!(sent["model"], "gpt-4o-mini-tts");
        assert_eq!(sent["voice"], "marin");
        assert_eq!(sent["response_format"], "pcm");
        assert_eq!(sent["speed"].as_f64().map(|rate| rate as f32), Some(1.2));
        assert_eq!(sent["instructions"], "Calm and even");
    }

    #[test]
    fn an_empty_instruction_is_left_out_and_a_named_voice_wins() {
        let (base_url, served) = serve_speech("200 OK", vec![pcm(&[0, 1, 2, 3, 4, 5])], false);
        let built = built(base_url, "", false);
        let utterance = built.backend.speak("Hello.", Some("cedar"));
        wait_until("the reply is read", || utterance.spoken());

        let request = served.join().unwrap();
        let body = request.split("\r\n\r\n").nth(1).expect("a JSON body");
        let sent: serde_json::Value = serde_json::from_str(body).expect("valid JSON");
        assert!(sent.get("instructions").is_none(), "{sent}");
        assert_eq!(sent["voice"], "cedar");
    }

    #[test]
    fn the_first_chunk_reaches_the_player_before_the_second_is_written() {
        let (base_url, served) =
            serve_speech("200 OK", vec![pcm(&[1; 480]), pcm(&[2; 480])], false);
        let built = built(base_url, "", false);
        let started = std::time::Instant::now();
        let utterance = built.backend.speak("Two chunks.", None);
        wait_until("the first chunk is queued", || utterance.queued() >= 1);
        let first_at = started.elapsed();
        assert!(
            first_at < GAP,
            "the first chunk waited for the second: {first_at:?}"
        );
        wait_until("both chunks are queued", || utterance.queued() == 2);
        served.join().unwrap();
    }

    #[test]
    fn a_stop_mid_stream_ends_the_utterance() {
        let (base_url, served) = serve_speech(
            "200 OK",
            vec![pcm(&[1; 480]), pcm(&[2; 480]), pcm(&[3; 480])],
            false,
        );
        let built = built(base_url, "", false);
        let mut utterance = built.backend.speak("Three chunks.", None);
        wait_until("the first chunk is queued", || utterance.queued() >= 1);
        utterance.stop();
        wait_until("the utterance ends", || utterance.spoken());
        assert_eq!(utterance.queued(), 1, "no chunk may follow a stop");
        let _ = served.join();
    }

    // A fallback started after the stop would speak over the stopped sentence,
    // and no stop could reach it.
    #[test]
    fn a_stop_before_the_first_byte_starts_no_fallback() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let (asked, request_read) = std::sync::mpsc::channel();
        let (release, held) = std::sync::mpsc::channel();
        let served = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            asked.send(()).unwrap();
            let _ = held.recv();
            // The request fails with no answer at all, which is the one case a
            // fallback may take over
            drop(stream);
        });

        let built = built(base_url, "", true);
        let mut utterance = built.backend.speak("Cancel this.", None);
        request_read
            .recv_timeout(Duration::from_secs(5))
            .expect("the request reached the server");
        utterance.stop();
        release.send(()).unwrap();

        wait_until("the request ends", || utterance.spoken());
        assert!(
            built.fallback_said.lock().unwrap().is_empty(),
            "a stopped sentence is not spoken again"
        );
        assert!(
            built.faults.recv_timeout(GAP).is_err(),
            "a stopped sentence reports nothing"
        );
        served.join().unwrap();
    }

    #[test]
    fn a_refused_key_is_named_and_the_fallback_speaks_the_text() {
        let (base_url, served) = serve_speech("401 Unauthorized", vec![], false);
        let built = built(base_url, "", true);
        let mut utterance = built.backend.start("Say this anyway.", None).unwrap();
        assert_eq!(failure(&built.faults), "the remote speaker refused the key");
        wait_until("the fallback is asked", || {
            !built.fallback_said.lock().unwrap().is_empty()
        });
        assert_eq!(
            built.fallback_said.lock().unwrap().as_slice(),
            ["Say this anyway.".to_string()]
        );
        wait_until("the utterance ends", || utterance.is_finished());
        let _ = served.join();
    }

    // A server that answers 200 with an audio type and then drops the
    // connection leaves nothing heard, so the whole sentence is still the
    // fallback's to speak.
    #[test]
    fn a_stream_that_dies_before_the_first_byte_hands_the_text_to_the_fallback() {
        let (base_url, served) = serve_speech("200 OK", vec![], true);
        let built = built(base_url, "", true);
        let mut utterance = built.backend.start("Say this anyway.", None).unwrap();
        let reason = failure(&built.faults);
        assert!(reason.contains("stopped answering"), "{reason}");
        wait_until("the fallback is asked", || {
            !built.fallback_said.lock().unwrap().is_empty()
        });
        assert_eq!(
            built.fallback_said.lock().unwrap().as_slice(),
            ["Say this anyway.".to_string()]
        );
        wait_until("the utterance ends", || utterance.is_finished());
        let _ = served.join();
    }

    // `Output::silent()` is a mixer nobody pulls from, so `is_finished` cannot
    // answer true under it, and every other test here waits on the request
    // instead. The `SpeechPlayer` queue turns on this answer: nothing else
    // starts the next utterance or clears the speaking flag.
    #[test]
    fn a_played_utterance_ends_once_the_device_takes_its_samples() {
        const SAMPLES: usize = 2_400;
        let (base_url, served) = serve_speech("200 OK", vec![pcm(&[1; SAMPLES])], false);
        let (output, mut mixed) = Output::readable();
        let (backend, _faults) = assembled(&table(base_url, ""), "sk-test", None, output);
        let mut utterance = backend.speak("Every word of this.", None);

        wait_until("the samples reach the player", || utterance.queued() > 0);
        assert!(
            !utterance.is_finished(),
            "samples the device has not taken are not spoken yet"
        );

        wait_until("the utterance ends", || {
            // A device pulls the samples; nothing else empties the player.
            mixed.by_ref().take(SAMPLES).for_each(drop);
            utterance.is_finished()
        });
        let _ = served.join();
    }

    // The fallback speaks the sentence the person is waiting for, and the
    // request it replaced ended long before it does. An utterance that answered
    // on the request alone would open the microphone over that sentence.
    #[test]
    fn a_failed_utterance_ends_only_once_its_fallback_ends() {
        let (base_url, served) = serve_speech("401 Unauthorized", vec![], false);
        let fallback = Arc::new(HoldingFallback::default());
        let (backend, faults) = assembled(
            &table(base_url, ""),
            "sk-test",
            Some(Arc::clone(&fallback) as Arc<dyn TtsBackend>),
            // No chunk is ever queued, so this player is empty whatever a
            // device does with it
            Output::silent(),
        );
        let mut utterance = backend.speak("Say this anyway.", None);

        assert_eq!(failure(&faults), "the remote speaker refused the key");
        wait_until("the fallback takes the sentence", || {
            !lock(&fallback.said).is_empty()
        });
        wait_until("the request ends", || utterance.spoken());
        assert!(
            !utterance.is_finished(),
            "the fallback is still speaking the sentence"
        );

        fallback
            .ended
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(
            utterance.is_finished(),
            "the fallback ended, so the utterance has"
        );
        let _ = served.join();
    }

    #[test]
    fn an_unknown_model_or_voice_is_named_as_such() {
        let (base_url, served) = serve_speech("404 Not Found", vec![], false);
        let built = built(base_url, "", false);
        let _utterance = built.backend.start("Hello.", None).unwrap();
        assert_eq!(
            failure(&built.faults),
            "the remote speaker has no such model or voice"
        );
        let _ = served.join();
    }

    #[test]
    fn a_status_with_no_wording_of_its_own_is_reported_by_its_code() {
        let (base_url, served) = serve_speech("503 Service Unavailable", vec![], false);
        let built = built(base_url, "", false);
        let _utterance = built.backend.start("Hello.", None).unwrap();
        assert_eq!(failure(&built.faults), "the remote speaker answered 503");
        let _ = served.join();
    }

    #[test]
    fn a_server_that_is_not_there_names_the_host() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        drop(listener);
        let built = built(base_url, "", false);
        let _utterance = built.backend.start("Hello.", None).unwrap();
        let reason = failure(&built.faults);
        assert!(reason.contains("127.0.0.1"), "{reason}");
        assert!(reason.contains("could not be reached"), "{reason}");
    }

    #[test]
    fn a_server_that_stops_answering_names_the_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let held = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            std::thread::sleep(Duration::from_secs(2));
        });
        let built = built(base_url, "", false);
        let _utterance = built.backend.start("Hello.", None).unwrap();
        let reason = failure(&built.faults);
        assert!(reason.contains("did not answer in time"), "{reason}");
        let _ = held.join();
    }

    // An utterance already half heard must not be spoken again from the start
    // in another voice.
    #[test]
    fn no_fallback_takes_over_after_the_first_chunk() {
        let (base_url, served) = serve_speech("200 OK", vec![pcm(&[1; 480])], true);
        let built = built(base_url, "", true);
        let utterance = built.backend.speak("Half of this.", None);
        wait_until("the first chunk is queued", || utterance.queued() >= 1);
        let played = built.faults.recv_timeout(Duration::from_secs(5));
        assert!(
            matches!(&played, Ok(Fault::Played)),
            "the first chunk was heard: {played:?}"
        );
        let reason = failure(&built.faults);
        assert!(!reason.is_empty(), "the reason is still written down");
        assert!(
            built.fallback_said.lock().unwrap().is_empty(),
            "no fallback may take over mid-utterance"
        );
        let _ = served.join();
    }

    // The first chunk that plays clears the reason the utterance before it left.
    #[test]
    fn an_utterance_that_plays_reports_that_it_played() {
        let (base_url, served) = serve_speech("200 OK", vec![pcm(&[1; 480])], false);
        let built = built(base_url, "", false);
        let utterance = built.backend.speak("Hello.", None);
        wait_until("the reply is read", || utterance.spoken());
        let played = built.faults.try_recv();
        assert!(
            matches!(&played, Ok(Fault::Played)),
            "a chunk that played must say so: {played:?}"
        );
        served.join().unwrap();
    }

    #[test]
    fn a_live_rate_write_reaches_the_next_utterance() {
        let (base_url, served) = serve_speech("200 OK", vec![pcm(&[0, 1, 2, 3, 4, 5])], false);
        let built = built(base_url, "", false);
        let tts = crate::config::TTSConfig {
            speed: 0.8,
            remote: table("http://unused.invalid/v1".to_string(), ""),
            ..Default::default()
        };
        assert_eq!(built.backend.reconfigure(&tts).as_deref(), Some("marin"));

        let utterance = built.backend.speak("Hello.", None);
        wait_until("the reply is read", || utterance.spoken());
        let request = served.join().unwrap();
        let body = request.split("\r\n\r\n").nth(1).expect("a JSON body");
        let sent: serde_json::Value = serde_json::from_str(body).expect("valid JSON");
        assert_eq!(sent["speed"].as_f64().map(|rate| rate as f32), Some(0.8));
        assert_eq!(sent["voice"], "marin");
    }

    /// A minimal 16-bit mono WAV. The chunk sizes are the ones a server that
    /// streams cannot know, so they are left at zero.
    fn wav(rate: u32, values: &[i16]) -> Vec<u8> {
        let mut out = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data\0\0\0\0");
        out.extend_from_slice(&pcm(values));
        out
    }

    fn sent_body(request: &str) -> serde_json::Value {
        let body = request.split("\r\n\r\n").nth(1).expect("a JSON body");
        serde_json::from_str(body).expect("valid JSON")
    }

    #[test]
    fn an_utterance_asks_for_wav_and_names_no_rate_unless_one_is_set() {
        let (base_url, served) =
            serve_typed("200 OK", "audio/wav", vec![wav(24_000, &[16_384])], false);
        let mut remote = table(base_url, "");
        remote.response_format = SpeechFormat::Wav;
        let built = build(remote, false);
        let utterance = built.backend.speak("Hello.", None);
        wait_until("the reply is read", || utterance.spoken());

        let sent = sent_body(&served.join().unwrap());
        assert_eq!(sent["response_format"], "wav");
        assert!(sent.get("sample_rate").is_none(), "{sent}");
    }

    #[test]
    fn a_rate_that_is_set_is_asked_for() {
        let (base_url, served) = serve_speech("200 OK", vec![pcm(&[0, 1, 2, 3, 4, 5])], false);
        let mut remote = table(base_url, "");
        remote.sample_rate = std::num::NonZero::new(22_050);
        let built = build(remote, false);
        let utterance = built.backend.speak("Hello.", None);
        wait_until("the reply is read", || utterance.spoken());

        let sent = sent_body(&served.join().unwrap());
        assert_eq!(sent["sample_rate"], 22_050);
    }

    #[test]
    fn a_wav_answer_and_a_bare_answer_play_the_same_audio() {
        let values: Vec<i16> = (0..480).map(|value| value * 60).collect();

        let (wav_url, wav_served) =
            serve_typed("200 OK", "audio/wav", vec![wav(24_000, &values)], false);
        let mut asks_wav = table(wav_url, "");
        asks_wav.response_format = SpeechFormat::Wav;
        let heard_wav = spoken_by(build(asks_wav, false));
        wav_served.join().unwrap();

        let (pcm_url, pcm_served) = serve_speech("200 OK", vec![pcm(&values)], false);
        let heard_pcm = spoken_by(build(table(pcm_url, ""), false));
        pcm_served.join().unwrap();

        assert_eq!(heard_wav, heard_pcm);
        assert_eq!(heard_wav[0].samples.len(), values.len());
        assert_eq!(heard_wav[0].rate.get(), 24_000);
    }

    fn spoken_by(built: Built) -> Vec<crate::text_to_speech::output::Chunk> {
        let utterance = built.backend.speak("The same sentence.", None);
        wait_until("the reply is read", || utterance.spoken());
        utterance.heard()
    }

    #[test]
    fn an_answer_that_is_not_audio_is_refused_by_its_type() {
        let (base_url, served) = serve_typed(
            "200 OK",
            "application/json",
            vec![br#"{"error":{"message":"no such voice"}}"#.to_vec()],
            false,
        );
        let built = built(base_url, "", false);
        let _utterance = built.backend.start("Hello.", None).unwrap();
        let reason = failure(&built.faults);
        assert!(reason.contains("application/json"), "{reason}");
        let _ = served.join();
    }

    // A header names the shape of samples that never arrive. Nothing was
    // heard, so the reply is still the fallback's to speak, and the reason
    // clears the failure the utterance before it left standing.
    #[test]
    fn an_answer_of_a_header_and_no_samples_hands_the_text_to_the_fallback() {
        let (base_url, served) = serve_typed("200 OK", "audio/wav", vec![wav(24_000, &[])], false);
        let mut remote = table(base_url, "");
        remote.response_format = SpeechFormat::Wav;
        let built = build(remote, true);
        let mut utterance = built.backend.start("Say this anyway.", None).unwrap();
        let reason = failure(&built.faults);
        assert!(reason.contains("sent no audio"), "{reason}");
        wait_until("the fallback is asked", || {
            !built.fallback_said.lock().unwrap().is_empty()
        });
        assert_eq!(
            built.fallback_said.lock().unwrap().as_slice(),
            ["Say this anyway.".to_string()]
        );
        wait_until("the utterance ends", || utterance.is_finished());
        let _ = served.join();
    }

    #[test]
    fn an_answer_that_never_says_what_it_is_is_refused() {
        let (base_url, served) = serve_speech("200 OK", vec![], false);
        let built = built(base_url, "", false);
        let _utterance = built.backend.start("Hello.", None).unwrap();
        let reason = failure(&built.faults);
        assert!(reason.contains("ended"), "{reason}");
        let _ = served.join();
    }

    #[test]
    fn an_answer_with_no_decoder_here_is_named_and_the_fallback_speaks_the_text() {
        let (base_url, served) = serve_typed(
            "200 OK",
            "audio/ogg",
            vec![b"OggS\0\x02\0\0\0\0\0\0\0\0".to_vec()],
            false,
        );
        let built = built(base_url, "", true);
        let _utterance = built.backend.start("Say this anyway.", None).unwrap();
        let reason = failure(&built.faults);
        assert!(reason.contains("Ogg"), "{reason}");
        wait_until("the fallback is asked", || {
            !built.fallback_said.lock().unwrap().is_empty()
        });
        assert_eq!(
            built.fallback_said.lock().unwrap().as_slice(),
            ["Say this anyway.".to_string()]
        );
        let _ = served.join();
    }

    const ACTIONABLE_REFUSAL: &str = r#"{"error":{"message":"The model `canopylabs/orpheus-v1-english` requires terms acceptance. Please have the org admin accept the terms at https://console.groq.com/playground?model=canopylabs%2Forpheus-v1-english","type":"invalid_request_error","code":"model_terms_required"}}"#;

    /// Shaped like a key and issued by nobody.
    const FAKE_KEY: &str = "sk-proj-7Qm4Xb2vR8tL1yWn3cZa";

    /// The reason one refused answer leaves.
    fn refusal(status: &'static str, body: &str, api_key: &str) -> String {
        let (base_url, served) = serve_typed(
            status,
            "application/json",
            vec![body.as_bytes().to_vec()],
            false,
        );
        let built = keyed(table(base_url, ""), false, api_key);
        let _utterance = built.backend.start("Hello.", None).unwrap();
        let reason = failure(&built.faults);
        let _ = served.join();
        reason
    }

    /// The server's own words from one body.
    fn message_of(body: &str) -> Option<String> {
        super::server_message(body, FAKE_KEY)
    }

    #[test]
    fn every_shape_a_server_refuses_in_yields_its_message() {
        for (body, message) in [
            (r#"{"error":{"message":"no such voice"}}"#, "no such voice"),
            (r#"{"error":"no such voice"}"#, "no such voice"),
            (r#"{"detail":{"error":"no such voice"}}"#, "no such voice"),
            (r#"{"detail":"no such voice"}"#, "no such voice"),
            (r#"{"message":"no such voice"}"#, "no such voice"),
        ] {
            assert_eq!(message_of(body).as_deref(), Some(message), "{body}");
        }
    }

    #[test]
    fn a_body_that_says_nothing_yields_no_message() {
        for silent in [
            "",
            "<html><body>502 Bad Gateway</body></html>",
            "{}",
            r#"{"error":{"code":"model_terms_required"}}"#,
            r#"{"error":{"message":"   "}}"#,
            r#"{"message":42}"#,
        ] {
            assert_eq!(message_of(silent), None, "{silent}");
        }
    }

    /// A word long enough that the cut inside the cap lands in the middle of
    /// one, whatever the cap is.
    const WORD: &str = "abcdefghijkl";

    #[test]
    fn a_long_message_is_cut_at_a_word_and_marked() {
        let said = format!("{WORD} ").repeat(40);
        let body = format!(r#"{{"detail":"{said}"}}"#);
        let message = message_of(&body).expect("a message");
        assert!(message.chars().count() <= super::MESSAGE_LIMIT, "{message}");
        assert!(message.ends_with('…'), "{message}");
        for word in message.trim_end_matches('…').split_whitespace() {
            assert_eq!(word, WORD, "{message}");
        }
    }

    // The cut counts characters, so a message in another language is cut at a
    // word too, and never inside a character.
    #[test]
    fn a_long_message_in_another_language_is_cut_at_a_word() {
        const WORD_WITH_AN_ACCENT: &str = "abcdéfghijkl";
        let long = format!("{WORD_WITH_AN_ACCENT} ").repeat(40);
        let message = message_of(&format!(r#"{{"detail":"{long}"}}"#)).expect("a message");
        assert!(message.chars().count() <= super::MESSAGE_LIMIT, "{message}");
        for word in message.trim_end_matches('…').split_whitespace() {
            assert_eq!(word, WORD_WITH_AN_ACCENT, "{message}");
        }
    }

    // Every byte offset the cut can land on is one a server can choose.
    #[test]
    fn a_body_past_the_cap_in_another_language_is_read_without_a_panic() {
        for pad in 0..8 {
            let long = format!("{}{}", "x".repeat(pad), "modèle introuvable ".repeat(600));
            let body = format!(r#"{{"error":{{"message":"{long}"}}}}"#);
            assert_eq!(
                refusal("400 Bad Request", &body, "sk-test"),
                "the remote speaker answered 400",
                "{pad} bytes of padding"
            );
        }
    }

    // A key the server writes as a JSON escape is not in the raw body at all,
    // so only redaction of the message it parsed to can catch it.
    #[test]
    fn a_key_a_server_escapes_into_its_message_is_redacted_too() {
        let escaped = FAKE_KEY.replacen("sk-", r"\u0073k-", 1);
        let body =
            format!(r#"{{"error":{{"message":"the key {escaped} is not for this model"}}}}"#);
        let reason = refusal("400 Bad Request", &body, FAKE_KEY);
        assert!(!reason.contains("7Qm4Xb"), "{reason}");
        assert!(reason.contains("<redacted>"), "{reason}");
        assert!(reason.ends_with("is not for this model"), "{reason}");
    }

    #[test]
    fn a_message_inside_the_cap_is_left_whole() {
        let message = message_of(ACTIONABLE_REFUSAL).expect("a message");
        assert!(message.ends_with("orpheus-v1-english"), "{message}");
    }

    #[test]
    fn a_message_with_no_space_near_the_cut_is_cut_where_the_cap_falls() {
        let long = format!("ab {}", "x".repeat(400));
        let body = format!(r#"{{"detail":"{long}"}}"#);
        let message = message_of(&body).expect("a message");
        assert_eq!(message.chars().count(), super::MESSAGE_LIMIT, "{message}");
    }

    // A reason is one line: the status line and the Voice panel hold one, and a
    // body must not forge a line of its own in the log.
    #[test]
    fn a_message_that_arrives_in_lines_is_read_as_one() {
        let message =
            message_of(r#"{"detail":"line one\nbanshee: forged\r\n\ttabbed  and  spaced"}"#)
                .expect("a message");
        assert!(!message.chars().any(char::is_control), "{message}");
        assert!(!message.contains("  "), "{message}");
        assert!(message.starts_with("line one banshee: forged"), "{message}");
        assert!(message.ends_with("tabbed and spaced"), "{message}");
    }

    // Every shape the message is read from is above; one answer proves the
    // message reaches the reason a person is given.
    #[test]
    fn a_refused_answer_says_what_the_server_said() {
        assert_eq!(
            refusal("400 Bad Request", r#"{"error":"no such voice"}"#, "sk-test"),
            "127.0.0.1 says no such voice"
        );
    }

    #[test]
    fn the_body_that_sent_this_change_names_the_terms_to_accept() {
        assert_eq!(
            refusal("400 Bad Request", ACTIONABLE_REFUSAL, "sk-test"),
            "127.0.0.1 says The model `canopylabs/orpheus-v1-english` requires terms acceptance. \
             Please have the org admin accept the terms at \
             https://console.groq.com/playground?model=canopylabs%2Forpheus-v1-english"
        );
    }

    #[test]
    fn a_body_that_is_not_json_falls_back_to_the_status() {
        assert_eq!(
            refusal(
                "503 Service Unavailable",
                "<html><body>502 Bad Gateway</body></html>",
                "sk-test"
            ),
            "the remote speaker answered 503"
        );
    }

    // A body past the cap is read no further, so the JSON it opened with never
    // closes and the status says what it can. Without the cap the whole body
    // would parse and its message would reach the reason.
    #[test]
    fn a_body_past_the_read_cap_is_not_read_to_the_end() {
        let said = "x".repeat(crate::credentials::BODY_LIMIT as usize * 2);
        let body = format!(r#"{{"error":{{"message":"{said}"}}}}"#);
        assert_eq!(
            refusal("400 Bad Request", &body, "sk-test"),
            "the remote speaker answered 400"
        );
    }

    // A 400 body can echo a key straight into the daemon log.
    #[test]
    fn a_key_the_server_echoes_reaches_neither_the_reason_nor_the_log() {
        let body = format!(
            r#"{{"detail":{{"error":"/audio/speech: Invalid model name passed in model={FAKE_KEY}. Call `/v1/models` to view available models for your key."}}}}"#
        );
        let reason = refusal("400 Bad Request", &body, FAKE_KEY);
        assert!(!reason.contains("sk-proj"), "{reason}");
        assert!(reason.contains("model=<redacted>"), "{reason}");
        assert!(reason.contains("Call `/v1/models`"), "{reason}");
    }

    #[test]
    fn a_refused_key_reads_no_body_at_all() {
        let body = format!(r#"{{"error":{{"message":"Incorrect API key provided: {FAKE_KEY}"}}}}"#);
        for status in ["401 Unauthorized", "403 Forbidden"] {
            let reason = refusal(status, &body, "sk-test");
            assert_eq!(reason, "the remote speaker refused the key");
            assert!(!reason.contains("Incorrect"), "{reason}");
        }
    }

    #[test]
    fn a_body_that_was_never_read_leaves_no_body_in_the_log() {
        assert_eq!(
            super::log_line(reqwest::StatusCode::UNAUTHORIZED, None),
            "banshee: the remote speaker answered 401 Unauthorized"
        );
        assert_eq!(
            super::log_line(reqwest::StatusCode::BAD_REQUEST, Some("{}")),
            "banshee: the remote speaker answered 400 Bad Request: {}"
        );
    }

    #[test]
    fn an_empty_voice_refuses_the_backend_and_names_the_key() {
        let mut remote = table("https://api.openai.com/v1".to_string(), "");
        remote.voice = String::new();
        let built = RemoteSpeechBackend::new(
            &remote,
            "sk-test".to_string(),
            1.0,
            None,
            Arc::new(Output::silent()),
            std::sync::mpsc::channel().0,
        );
        // `expect_err` would ask the backend for a Debug line, and the backend
        // holds the key
        let Err(error) = built else {
            panic!("a speaker with no voice must refuse");
        };
        assert!(error.to_string().contains("tts.remote.voice"), "{error}");
    }
}

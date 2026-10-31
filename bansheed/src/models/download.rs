use std::path::Path;
use std::time::Duration;

use banshee_common::{
    COMPANION_VOICE, DownloadProgress, DownloadState, FileRole, KokoroTTSConfig, POCKET_DIR,
    SileroVADConfig,
    error::BansheeError,
    listening::{self, ListeningModel},
    utils::models_path,
};
use tokio::io::AsyncWriteExt;

use crate::config::{Config, Provider, TTSFallback};
use crate::models::licences::{self, Accepted};

pub fn percent(bytes: u64, total: Option<u64>) -> Option<u64> {
    let total = total?;
    (total > 0).then(|| bytes * 100 / total)
}

// A server is free to ignore `Range` and answer 200 with the whole body, and
// the bytes on disk are then a prefix to overwrite rather than append to
fn accepted_offset(status: reqwest::StatusCode, partial: u64) -> u64 {
    match status {
        reqwest::StatusCode::PARTIAL_CONTENT => partial,
        _ => 0,
    }
}

// With no total there is no percentage to report against. 4 MiB gives a 141 MB
// file about the same number of notifications as the percentage rule would.
const UNKNOWN_TOTAL_STEP: u64 = 4 * 1024 * 1024;

// What the sender compares to decide a notification is worth sending
fn milestone(bytes: u64, total: Option<u64>) -> u64 {
    percent(bytes, total).unwrap_or(bytes / UNKNOWN_TOTAL_STEP)
}

#[derive(Clone)]
pub struct Download {
    /// Shown as "about", because it is upstream's to change and nothing here
    /// checks. A listening model's is its catalogue bytes over 1,000,000.
    pub megabytes: u64,
    pub name: String,
    pub url: String,
    /// Checked before the file is renamed into place. Only a listening model
    /// carries one.
    pub sha256: Option<&'static str>,
    /// The listening model this file is, whose licence may hold it back.
    pub model: Option<&'static ListeningModel>,
}

/// Measured on disk on 2026-08-30.
const KOKORO_MEGABYTES: u64 = 311;
const DETECTOR_MEGABYTES: u64 = 2;
const VOICE_MEGABYTES: u64 = 1;

/// Pinned to revisions so a repo update cannot silently change the model.
pub fn pocket_files() -> Vec<Download> {
    const ONNX_REPO: &str = "https://huggingface.co/KevinAHM/pocket-tts-onnx/resolve/58a6d00cf13d239b6748cb0769f35c580a8f606c/onnx/english_2026-04";
    const KYUTAI_REPO: &str = "https://huggingface.co/kyutai/pocket-tts-without-voice-cloning/resolve/a7056f505a4a8ca72d8b5fb1440071d6f1a35739";
    [
        (ONNX_REPO, "flow_lm_main.onnx", 303),
        (ONNX_REPO, "flow_lm_flow.onnx", 40),
        (ONNX_REPO, "mimi_decoder.onnx", 42),
        (ONNX_REPO, "text_conditioner.onnx", 17),
        (ONNX_REPO, "bundle.json", 1),
        (ONNX_REPO, "bos_before_voice.npy", 1),
        (KYUTAI_REPO, "tokenizer.json", 1),
    ]
    .into_iter()
    .map(|(repo, file, megabytes)| Download {
        megabytes,
        name: format!("{POCKET_DIR}/{file}"),
        url: format!("{repo}/{file}"),
        sha256: None,
        model: None,
    })
    .collect()
}

/// Every file a local voice speaks with: Pocket's for the companion, Kokoro and
/// the voice's own file for any other id.
pub fn voice_files(voice: &str) -> Vec<Download> {
    if voice == COMPANION_VOICE {
        return pocket_files();
    }
    let kokoro = KokoroTTSConfig::new(voice);
    vec![
        Download {
            megabytes: KOKORO_MEGABYTES,
            name: kokoro.model_name,
            url: kokoro.model_url,
            sha256: None,
            model: None,
        },
        Download {
            megabytes: VOICE_MEGABYTES,
            name: kokoro.voice_name,
            url: kokoro.voice_url,
            sha256: None,
            model: None,
        },
    ]
}

/// The local voice speaks under a local speaker, and under a remote one that
/// falls back to it.
fn speaks_locally(config: &Config) -> bool {
    config.tts.provider == Provider::Local || config.tts.fallback == TTSFallback::Local
}

pub fn listening_file(model: &'static ListeningModel) -> Download {
    Download {
        megabytes: model.bytes / 1_000_000,
        name: model.file.to_string(),
        url: model.url(),
        sha256: Some(model.sha256),
        model: Some(model),
    }
}

pub fn wanted(config: &Config) -> Vec<Download> {
    let vad = SileroVADConfig::new(crate::models::VAD_MODEL);
    let mut downloads = Vec::new();
    if let Some(model) = crate::models::stt_model(config) {
        downloads.push(listening_file(model));
    }
    downloads.push(Download {
        megabytes: DETECTOR_MEGABYTES,
        name: vad.model_name,
        url: vad.download_url,
        sha256: None,
        model: None,
    });
    if speaks_locally(config) {
        downloads.extend(voice_files(&config.tts.voice));
    }
    downloads
}

/// Zero once every file is here, which is what a client shows after setup.
pub fn pending_megabytes(wanted: &[Download], dir: &Path) -> u64 {
    missing(wanted, dir)
        .map(|download| download.megabytes)
        .sum()
}

pub fn still_missing(wanted: &[Download], dir: &Path) -> Vec<Download> {
    missing(wanted, dir).cloned().collect()
}

/// The one test for what is still to fetch. A sum over it copies nothing.
fn missing<'a>(wanted: &'a [Download], dir: &'a Path) -> impl Iterator<Item = &'a Download> {
    wanted
        .iter()
        .filter(move |download| !dir.join(&download.name).exists())
}

pub fn models_dir() -> Result<std::path::PathBuf, BansheeError> {
    models_path()
        .ok_or_else(|| BansheeError::Other("Could not find the models directory".to_string()))
}

fn other(error: reqwest::Error) -> BansheeError {
    BansheeError::Other(error.to_string())
}

/// Only the daemon holds the catalogue, so a client that has to tell the
/// speech model from the voice asks this rather than reading the filename.
pub fn role(file: &str) -> FileRole {
    if listening::find_file(file).is_some() {
        FileRole::Speech
    } else if file == crate::models::VAD_MODEL {
        FileRole::Detector
    } else if file == banshee_common::KOKORO_MODEL || Path::new(file).starts_with(POCKET_DIR) {
        FileRole::Engine
    } else {
        FileRole::Voice
    }
}

/// The same fact in the user's words, for a progress line. Derived from the
/// role, so rewording this cannot move what a client routes on.
pub fn label(file: &str) -> &'static str {
    match role(file) {
        FileRole::Speech => "Listening model",
        FileRole::Detector => "Voice detection model",
        FileRole::Engine => "Speech engine",
        FileRole::Voice => "Voice",
    }
}

fn progress(
    model: &str,
    index: usize,
    count: usize,
    bytes: u64,
    total: Option<u64>,
    state: DownloadState,
) -> DownloadProgress {
    DownloadProgress {
        model: model.to_string(),
        label: label(model).to_string(),
        index,
        count,
        bytes,
        total,
        state,
    }
}

async fn ask(
    client: &reqwest::Client,
    url: &str,
    from: u64,
) -> Result<reqwest::Response, BansheeError> {
    let mut request = client.get(url);
    if from > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={from}-"));
    }
    request.send().await.map_err(other)
}

// Renamed into place only once the body is complete, so an interrupted
// download is never mistaken for a model. A partial left behind is resumed.
async fn fetch(
    client: &reqwest::Client,
    download: &Download,
    dir: &Path,
    index: usize,
    count: usize,
    on_progress: &mut impl FnMut(DownloadProgress),
) -> Result<(), BansheeError> {
    let part = dir.join(format!("{}.part", download.name));
    if let Some(folder) = part.parent() {
        tokio::fs::create_dir_all(folder).await?;
    }
    let partial = tokio::fs::metadata(&part)
        .await
        .map(|meta| meta.len())
        .unwrap_or(0);

    let first = ask(client, &download.url, partial).await?;
    // A partial that already holds the whole body asks for a range past the
    // end, which the server refuses. Starting over is the one answer that is
    // right whether the file was complete or overlong.
    let (mut response, partial) = if first.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        let _ = tokio::fs::remove_file(&part).await;
        (ask(client, &download.url, 0).await?, 0)
    } else {
        (first, partial)
    };
    if let Err(error) = response.error_for_status_ref() {
        return Err(other(error));
    }

    let already = accepted_offset(response.status(), partial);
    let total = response.content_length().map(|length| already + length);

    let mut file = if already > 0 {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(&part)
            .await?
    } else {
        tokio::fs::File::create(&part).await?
    };

    let mut sent = already;
    let mut reported = None;
    let mut report =
        |sent, state| on_progress(progress(&download.name, index, count, sent, total, state));
    report(sent, DownloadState::Downloading);

    while let Some(chunk) = response.chunk().await.map_err(other)? {
        file.write_all(&chunk).await?;
        sent += chunk.len() as u64;
        let now = Some(milestone(sent, total));
        if now != reported {
            reported = now;
            report(sent, DownloadState::Downloading);
        }
    }

    file.flush().await?;
    drop(file);
    if let Some(expected) = download.sha256 {
        verify(&part, expected).await?;
    }
    tokio::fs::rename(&part, dir.join(&download.name)).await?;
    report(sent, DownloadState::Done);
    Ok(())
}

/// A file that fails is deleted, since resuming it only appends to bytes that
/// are already wrong.
async fn verify(part: &Path, expected: &str) -> Result<(), BansheeError> {
    let path = part.to_path_buf();
    let actual = tokio::task::spawn_blocking(move || sha256_of(&path))
        .await
        .map_err(|error| BansheeError::Other(error.to_string()))??;
    if actual == expected {
        return Ok(());
    }
    let _ = tokio::fs::remove_file(part).await;
    Err(BansheeError::Other(
        "the download failed its sha256 check against Banshee's record, so it was deleted. If it fails again, report it: the file upstream may have changed."
            .to_string(),
    ))
}

fn sha256_of(path: &Path) -> std::io::Result<String> {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// reqwest sets no timeout of its own, so a connect that never completes holds
/// the run for ever and strands every file behind it. A connect to these hosts
/// measured 6-34 ms, so ten seconds is far past a slow link and still bounded.
///
/// The read timeout is the gap between chunks, not the whole transfer: a total
/// timeout would refuse a 574 MB file on any slow connection. Nothing measured
/// that gap. It trades how slow a link may be against how long a stall hides.
fn build_client() -> Result<reqwest::Client, BansheeError> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(other)
}

/// A listening model whose licence limits commercial use is refused here,
/// before any request, until the person has accepted that licence.
///
/// The partial file of a failed download stays on disk, so the next call
/// continues it rather than starting over.
pub async fn download_all(
    dir: &Path,
    downloads: &[Download],
    accepted: &Accepted,
    on_progress: &mut impl FnMut(DownloadProgress),
) -> Result<(), BansheeError> {
    tokio::fs::create_dir_all(dir).await?;

    let client = build_client()?;
    let mut failures = Vec::new();
    let count = downloads.len();
    for (position, download) in downloads.iter().enumerate() {
        let index = position + 1;
        // One bad file must not strand the rest, and a caller counting terminal
        // notifications is owed one for every name it was given
        let refused = download
            .model
            .and_then(|model| licences::refusal(model, accepted));
        let fetched = match refused {
            Some(reason) => Err(BansheeError::Rejected(reason)),
            None => fetch(&client, download, dir, index, count, on_progress).await,
        };
        if let Err(error) = fetched {
            on_progress(progress(
                &download.name,
                index,
                count,
                0,
                None,
                DownloadState::Failed,
            ));
            failures.push(format!("{}: {error}", download.name));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(BansheeError::Other(failures.join("; ")))
    }
}

#[cfg(test)]
mod size_tests {
    use super::{pending_megabytes, wanted};
    use crate::config::{Config, Provider};

    #[test]
    fn wanted_puts_the_configured_model_first() {
        let mut config = Config::default();
        config.stt.model = "whisper-large-v3-turbo".to_string();
        let turbo = banshee_common::listening::find("whisper-large-v3-turbo").unwrap();

        let first = wanted(&config).remove(0);
        assert_eq!(first.name, turbo.file);
        assert_eq!(first.url, turbo.url());
        assert_eq!(first.sha256, Some(turbo.sha256));
    }

    /// The window shows this instead of summing a file list it does not hold,
    /// so it has to follow the model rather than a fixed total.
    #[test]
    fn the_speech_model_moves_the_first_run() {
        let dir = std::path::Path::new("/no-such-models-dir-9f3a");
        let mut config = Config::default();
        let parakeet = pending_megabytes(&wanted(&config), dir);
        config.stt.model = "whisper-large-v3-turbo".to_string();
        let turbo = pending_megabytes(&wanted(&config), dir);
        let price = |id: &str| {
            super::listening_file(banshee_common::listening::find(id).unwrap()).megabytes
        };
        assert_eq!(
            parakeet - turbo,
            price(banshee_common::listening::DEFAULT_MODEL) - price("whisper-large-v3-turbo")
        );
    }

    #[test]
    fn a_remote_listener_downloads_no_listening_model() {
        let mut config = Config::default();
        config.stt.provider = Provider::Remote;
        let names: Vec<String> = wanted(&config).into_iter().map(|d| d.name).collect();
        assert!(
            !names.iter().any(|name| name.ends_with(".gguf")),
            "{names:?}"
        );
        assert!(names.contains(&crate::models::VAD_MODEL.to_string()));
    }

    // The sum above covers the detector, the engine and a voice as well.
    #[test]
    fn each_missing_file_carries_its_own_price_and_role() {
        let dir = std::path::Path::new("/no-such-models-dir-9f3a");
        let config = Config::default();

        let missing = super::still_missing(&wanted(&config), dir);
        let speech = missing
            .iter()
            .find(|file| super::role(&file.name) == banshee_common::FileRole::Speech)
            .expect("a run with no directory is missing its speech model");

        assert!(
            speech.megabytes > 0 && speech.megabytes < pending_megabytes(&wanted(&config), dir),
            "one file priced {} against a run of {}",
            speech.megabytes,
            pending_megabytes(&wanted(&config), dir)
        );
    }
}

#[cfg(test)]
mod sha256_tests {
    use super::{Download, download_all};
    use crate::models::licences::Accepted;
    use std::io::Write;

    fn nothing_accepted() -> Accepted {
        Accepted::at("/no-such-dir-9f3a/accepted-licences".into())
    }

    const HI_SHA256: &str = "8f434346648f6b96df89dda901c5176b10a6d83961dd3c1ac88b59b2dc327aa4";

    /// Answers one request with two bytes, then stops listening. Loopback only.
    fn serve_two_bytes() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Some(stream) = listener.incoming().next() {
                let mut stream = stream.unwrap();
                crate::test_support::read_request(&mut stream);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi",
                );
            }
        });
        url
    }

    #[tokio::test]
    async fn a_listening_file_failing_its_sha256_is_deleted() {
        let dir = crate::test_support::unique_scratch("sha256");
        let file = Download {
            megabytes: 1,
            name: "model.gguf".to_string(),
            url: serve_two_bytes(),
            sha256: Some("0000000000000000000000000000000000000000000000000000000000000000"),
            model: None,
        };

        let error = download_all(&dir, &[file], &nothing_accepted(), &mut |_| {})
            .await
            .expect_err("two bytes cannot match that hash");
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(error.to_string().contains("model.gguf"), "{error}");
        assert!(error.to_string().contains("sha256"), "{error}");
        assert!(left.is_empty(), "{left:?}");
    }

    /// The hash of `hi`, so the check passes a body that matches.
    #[tokio::test]
    async fn a_listening_file_matching_its_sha256_is_kept() {
        let dir = crate::test_support::unique_scratch("sha256-ok");
        let file = Download {
            megabytes: 1,
            name: "model.gguf".to_string(),
            url: serve_two_bytes(),
            sha256: Some(HI_SHA256),
            model: None,
        };

        download_all(&dir, &[file], &nothing_accepted(), &mut |_| {})
            .await
            .unwrap();
        let body = std::fs::read(dir.join("model.gguf")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(body, b"hi");
    }

    /// The server would answer, so only the licence check can stop the file.
    #[tokio::test]
    async fn a_model_whose_licence_limits_commercial_use_waits_for_acceptance() {
        let dir = crate::test_support::unique_scratch("licence-gate");
        let canary = banshee_common::listening::find("canary-1b").unwrap();
        let file = Download {
            megabytes: 1,
            name: canary.file.to_string(),
            url: serve_two_bytes(),
            sha256: Some(HI_SHA256),
            model: Some(canary),
        };
        let mut accepted = Accepted::at(dir.join("accepted-licences"));

        let refused = download_all(&dir, std::slice::from_ref(&file), &accepted, &mut |_| {}).await;
        let arrived_unaccepted = dir.join(canary.file).exists();
        accepted.add(canary.licence.name).unwrap();
        let after = download_all(&dir, &[file], &accepted, &mut |_| {}).await;
        let arrived = dir.join(canary.file).exists();
        let _ = std::fs::remove_dir_all(&dir);

        let error = refused
            .expect_err("an unaccepted licence refuses the file")
            .to_string();
        assert!(error.contains("CC-BY-NC-4.0"), "{error}");
        assert!(error.contains("--accept-licence"), "{error}");
        assert!(!arrived_unaccepted);
        after.unwrap();
        assert!(arrived);
    }
}

#[cfg(test)]
mod voice_tests {
    use super::{pocket_files, role, wanted};
    use crate::config::{Config, Provider, TTSFallback};
    use banshee_common::{FileRole, KOKORO_MODEL};

    fn names(config: &Config) -> Vec<String> {
        wanted(config).into_iter().map(|d| d.name).collect()
    }

    fn pocket_names() -> Vec<String> {
        pocket_files().into_iter().map(|file| file.name).collect()
    }

    fn speaker_files(config: &Config) -> Vec<String> {
        names(config)
            .into_iter()
            .filter(|name| matches!(role(name), FileRole::Engine | FileRole::Voice))
            .collect()
    }

    #[test]
    fn a_companion_config_wants_the_seven_pocket_files() {
        let config = Config::default();
        assert_eq!(speaker_files(&config), pocket_names());
        assert_eq!(pocket_names().len(), 7);
        for name in pocket_names() {
            assert_eq!(role(&name), FileRole::Engine, "{name}");
        }
    }

    #[test]
    fn a_kokoro_voice_wants_kokoro_and_its_voice() {
        let mut config = Config::default();
        config.tts.voice = "am_adam".to_string();
        assert_eq!(speaker_files(&config), [KOKORO_MODEL, "am_adam.bin"]);
    }

    #[test]
    fn a_remote_speaker_with_no_fallback_wants_no_voice() {
        let mut config = Config::default();
        config.tts.provider = Provider::Remote;
        config.tts.fallback = TTSFallback::None;
        assert_eq!(speaker_files(&config), Vec::<String>::new());
    }

    #[test]
    fn a_remote_speaker_with_a_local_fallback_wants_the_local_voice() {
        let mut config = Config::default();
        config.tts.provider = Provider::Remote;
        config.tts.fallback = TTSFallback::Local;
        assert_eq!(speaker_files(&config), pocket_names());
    }
}

#[cfg(test)]
mod tests {
    use super::{accepted_offset, milestone, percent};
    use reqwest::StatusCode;

    // Appending to the prefix instead would leave a file longer than the model
    #[test]
    fn a_server_that_ignores_range_restarts_the_file() {
        assert_eq!(
            accepted_offset(StatusCode::PARTIAL_CONTENT, 900_000),
            900_000
        );
        assert_eq!(accepted_offset(StatusCode::OK, 900_000), 0);
    }

    #[test]
    fn percent_tracks_the_whole_point_only() {
        assert_eq!(percent(0, Some(200)), Some(0));
        assert_eq!(percent(1, Some(200)), Some(0), "half a point is still zero");
        assert_eq!(percent(2, Some(200)), Some(1));
        assert_eq!(percent(200, Some(200)), Some(100));
    }

    // A byte counter that never moves reads as a stalled download
    #[test]
    fn an_unknown_total_still_advances_on_bytes() {
        assert_eq!(milestone(0, None), 0);
        assert_eq!(milestone(super::UNKNOWN_TOTAL_STEP - 1, None), 0);
        assert_eq!(milestone(super::UNKNOWN_TOTAL_STEP, None), 1);
        assert_eq!(milestone(super::UNKNOWN_TOTAL_STEP * 3, None), 3);
    }

    #[test]
    fn an_unknown_or_empty_total_yields_no_percentage() {
        assert_eq!(percent(50, None), None);
        assert_eq!(percent(50, Some(0)), None, "a zero total must not divide");
    }

    #[test]
    fn a_file_is_labelled_by_what_it_is() {
        use super::label;
        assert_eq!(
            label("parakeet-unified-en-0.6b-Q8_0.gguf"),
            "Listening model"
        );
        assert_eq!(label("silero_vad.onnx"), "Voice detection model");
        assert_eq!(label("af_sky.bin"), "Voice");
        assert_eq!(label("kokoro-v1.0.onnx"), "Speech engine");
    }
}

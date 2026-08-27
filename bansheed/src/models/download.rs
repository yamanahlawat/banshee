use std::path::Path;

use banshee_common::{
    DownloadProgress, DownloadState, KokoroTTSConfig, SileroVADConfig, WhisperConfig,
    error::BansheeError, utils::get_models_path,
};
use tokio::io::AsyncWriteExt;

use crate::config::Config;

/// Whole percentage points only, and `None` when the server sent no length.
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
    pub name: String,
    pub url: String,
}

/// Every file this config needs, and where to fetch it.
pub fn wanted(config: &Config) -> Vec<Download> {
    let [speech, voice_activity] = crate::models::required(config);
    let whisper = WhisperConfig::new(speech);
    let vad = SileroVADConfig::new(voice_activity);
    let kokoro = KokoroTTSConfig::new(&config.tts.voice);
    vec![
        Download {
            name: whisper.model_name,
            url: whisper.download_url,
        },
        Download {
            name: vad.model_name,
            url: vad.download_url,
        },
        Download {
            name: kokoro.model_name,
            url: kokoro.model_url,
        },
        Download {
            name: kokoro.voice_name,
            url: kokoro.voice_url,
        },
    ]
}

/// The files in `wanted` that `dir` does not already hold.
pub fn still_missing(wanted: &[Download], dir: &Path) -> Vec<Download> {
    wanted
        .iter()
        .filter(|download| !dir.join(&download.name).exists())
        .cloned()
        .collect()
}

/// Where the models live, or why they cannot be found.
pub fn models_dir() -> Result<std::path::PathBuf, BansheeError> {
    get_models_path()
        .ok_or_else(|| BansheeError::Other("Could not find the models directory".to_string()))
}

fn other(error: reqwest::Error) -> BansheeError {
    BansheeError::Other(error.to_string())
}

/// What a model file is, for a person reading a progress line.
pub fn label(file: &str) -> &'static str {
    if crate::config::STTPreset::ALL
        .iter()
        .any(|preset| preset.model_name() == file)
    {
        "Speech model"
    } else if file == crate::VAD_MODEL {
        "Voice detection model"
    } else {
        "Voice"
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
    tokio::fs::rename(&part, dir.join(&download.name)).await?;
    report(sent, DownloadState::Done);
    Ok(())
}

/// The partial file of a failed download stays on disk, so the next call
/// continues it rather than starting over.
pub async fn download_all(
    dir: &Path,
    downloads: &[Download],
    on_progress: &mut impl FnMut(DownloadProgress),
) -> Result<(), BansheeError> {
    tokio::fs::create_dir_all(dir).await?;

    let client = reqwest::Client::new();
    let mut failures = Vec::new();
    let count = downloads.len();
    for (position, download) in downloads.iter().enumerate() {
        let index = position + 1;
        // One bad file must not strand the rest, and a caller counting terminal
        // notifications is owed one for every name it was given
        if let Err(error) = fetch(&client, download, dir, index, count, on_progress).await {
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
        assert_eq!(label("ggml-large-v3-turbo-q5_0.bin"), "Speech model");
        assert_eq!(label("silero_vad.onnx"), "Voice detection model");
        assert_eq!(label("af_sky.bin"), "Voice");
        assert_eq!(label("kokoro-v1.0.onnx"), "Voice");
    }

    #[test]
    fn a_progress_report_names_the_file_its_place_and_its_size() {
        use banshee_common::DownloadState;
        let reported = super::progress(
            "ggml-large-v3-turbo-q5_0.bin",
            1,
            3,
            356,
            Some(574),
            DownloadState::Downloading,
        );
        assert_eq!(reported.label, "Speech model");
        assert_eq!(reported.index, 1);
        assert_eq!(reported.count, 3);
        assert_eq!(reported.bytes, 356);
        assert_eq!(reported.total, Some(574));
        assert_eq!(super::percent(reported.bytes, reported.total), Some(62));
    }

    #[test]
    fn an_unknown_length_stays_none() {
        use banshee_common::DownloadState;
        let reported = super::progress(
            "silero_vad.onnx",
            2,
            3,
            10,
            None,
            DownloadState::Downloading,
        );
        assert_eq!(reported.label, "Voice detection model");
        assert_eq!(reported.total, None);
        assert_eq!(super::percent(reported.bytes, reported.total), None);
    }
}

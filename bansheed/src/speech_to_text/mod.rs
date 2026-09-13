pub mod local;
pub mod remote;
pub mod vad;

use banshee_common::error::BansheeError;

use crate::config::{STTConfig, STTPreset, SttProvider};
use crate::state::RecordingError;
use local::whisper::WhisperEngine;
use remote::openai_compatible::RemoteTranscriber;

/// The rate every `Transcriber` reads, and the rate the pipeline resamples the
/// microphone to before the VAD sees a chunk.
pub const SAMPLE_RATE: u32 = 16_000;

/// What language the next transcription reads the audio as, and whether it
/// answers in English whatever was said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speech {
    /// `None` asks the engine to detect it.
    pub language: Option<String>,
    pub translate: bool,
}

impl From<&STTConfig> for Speech {
    /// `auto` is the config's word for detect it, and `None` is the engine's.
    fn from(stt: &STTConfig) -> Self {
        Self {
            language: (stt.language != "auto").then(|| stt.language.clone()),
            translate: stt.translate,
        }
    }
}

/// An English-only build carries `.en` in its name and holds no other language,
/// so asking it for one produces an English-shaped guess at the sounds rather
/// than an error.
pub fn english_only(model_name: &str) -> bool {
    model_name.contains(".en")
}

// One thread owns the engine and calls it, so `Send` without `Sync`
pub trait Transcriber: Send {
    /// `audio` is mono `f32` at `SAMPLE_RATE`.
    fn transcribe(&self, audio: &[f32]) -> Result<String, BansheeError>;
    fn set_vocabulary(&mut self, words: &[String]);
    fn set_speech(&mut self, speech: Speech);
    /// `stt.preset` changed live. Answers the model file now loaded, or `None`
    /// for a provider that loads no file.
    fn reload(&mut self, preset: STTPreset) -> Result<Option<&'static str>, BansheeError>;
    /// What shortens a transcription that ran far slower than realtime, or
    /// `None` where the wait is not this machine's to shorten.
    fn slow_advice(&self) -> Option<&'static str> {
        None
    }
}

pub fn select_transcriber(stt: &STTConfig) -> Result<Box<dyn Transcriber>, RecordingError> {
    match stt.provider {
        SttProvider::Local => {
            let engine = WhisperEngine::new(stt.preset.model_name(), &stt.vocabulary, stt.into())
                .map_err(|e| RecordingError::Model(e.to_string()))?;
            Ok(Box::new(engine))
        }
        SttProvider::Remote => select_remote(stt, crate::credentials::Credentials::load()),
    }
}

fn select_remote(
    stt: &STTConfig,
    credentials: Result<crate::credentials::Credentials, BansheeError>,
) -> Result<Box<dyn Transcriber>, RecordingError> {
    let side = crate::credentials::RemoteKey::Stt;
    let key = credentials
        .map_err(|e| RecordingError::KeyFile(e.to_string()))?
        .key(side)
        .ok_or_else(|| RecordingError::Provider(side.no_key()))?
        .to_string();
    println!("Listening through {}", stt.remote.host());
    let engine = RemoteTranscriber::new(&stt.remote, key, &stt.vocabulary, stt.into())
        .map_err(|e| RecordingError::Provider(e.to_string()))?;
    Ok(Box::new(engine))
}

#[cfg(test)]
mod tests {
    use super::{RecordingError, Speech, english_only, select_remote};

    #[test]
    fn a_key_file_that_does_not_parse_is_a_key_file_fault() {
        let path = std::env::temp_dir().join(format!(
            "banshee-select-transcriber-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "[stt.remote]\napi_key = sk-live-SECRET123\n").unwrap();
        let credentials = crate::credentials::Credentials::read(&path);
        let _ = std::fs::remove_file(&path);

        let stt = crate::config::STTConfig::default();
        let fault = select_remote(&stt, credentials)
            .err()
            .expect("a file that does not parse holds no key");
        assert!(
            matches!(&fault, RecordingError::KeyFile(reason) if reason.contains("does not parse")),
            "{fault}"
        );
    }

    #[test]
    fn a_key_nobody_set_is_a_provider_fault() {
        let stt = crate::config::STTConfig::default();
        let fault = select_remote(&stt, Ok(crate::credentials::Credentials::default()))
            .err()
            .expect("a remote listener without a key cannot start");
        assert!(
            matches!(&fault, RecordingError::Provider(reason) if reason.contains("stt.remote.api_key")),
            "{fault}"
        );
    }

    /// `auto` is the config's word for detect it and `None` is the engine's.
    #[test]
    fn auto_becomes_the_absence_the_engine_reads_as_detect_it() {
        let mut stt = crate::config::STTConfig {
            language: "auto".to_string(),
            ..Default::default()
        };
        assert_eq!(Speech::from(&stt).language, None);
        stt.language = "de".to_string();
        assert_eq!(Speech::from(&stt).language, Some("de".to_string()));
    }

    /// The daemon's `english_only` flag and the window's note on the Fast preset
    /// both rest on this mapping, so it is pinned here.
    #[test]
    fn only_the_fast_preset_is_english_only() {
        use crate::config::STTPreset;
        assert!(english_only(STTPreset::Fast.model_name()));
        assert!(!english_only(STTPreset::Balanced.model_name()));
        assert!(!english_only(STTPreset::Quality.model_name()));
    }
}

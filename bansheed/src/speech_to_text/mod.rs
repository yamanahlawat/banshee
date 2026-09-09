pub mod local;
pub mod vad;

use banshee_common::error::BansheeError;

use crate::config::{STTConfig, SttProvider};
use local::whisper::WhisperEngine;

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
    /// `audio` is mono `f32` at 16 kHz.
    fn transcribe(&self, audio: &[f32]) -> Result<String, BansheeError>;
    fn set_vocabulary(&mut self, words: &[String]);
    fn set_speech(&mut self, speech: Speech);
    /// The `stt.preset` changed. On `Ok(())` the listener records `model` as the
    /// loaded model.
    fn reload(&mut self, model: &'static str) -> Result<(), BansheeError>;
}

pub fn select_transcriber(stt: &STTConfig) -> Result<Box<dyn Transcriber>, BansheeError> {
    match stt.provider {
        SttProvider::Local => {
            let engine = WhisperEngine::new(stt.preset.model_name(), &stt.vocabulary, stt.into())?;
            Ok(Box::new(engine))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Speech, english_only};

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

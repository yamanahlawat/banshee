use banshee_common::error::BansheeError;
use whisper_rs::{FullParams, WhisperContext, WhisperContextParameters};

use crate::config::STTPreset;
use crate::speech_to_text::{Speech, Transcriber, english_only};

/// Measured across widths 1 to 8: a 1.5% spread, and the same words from 2 up.
/// whisper.cpp refuses more than 8, with an error naming nothing.
const BEAM_SIZE: i32 = 5;
const BEAM_PATIENCE: f32 = -1.0;

/// What Whisper says about one segment, for the debug line. Nothing gates on it:
/// `no_speech_prob` read 0.00 across 314 segments of real audio.
#[derive(Debug, Clone, Copy)]
pub struct Confidence {
    pub no_speech_prob: f32,
    /// Decoder confidence: mean ln(p) over the segment's tokens.
    pub avg_logprob: f32,
}

impl Confidence {
    fn of(segment: &whisper_rs::WhisperSegment<'_>) -> Self {
        let n_tokens = segment.n_tokens();
        let mut logprob_sum = 0.0f32;
        for index in 0..n_tokens {
            if let Some(token) = segment.get_token(index) {
                // clamp so a zero probability cannot produce -inf
                logprob_sum += token.token_probability().max(f32::MIN_POSITIVE).ln();
            }
        }
        Self {
            no_speech_prob: segment.no_speech_probability(),
            avg_logprob: if n_tokens > 0 {
                logprob_sum / n_tokens as f32
            } else {
                0.0
            },
        }
    }
}

fn build_initial_prompt(vocabulary: &[String]) -> Option<String> {
    if vocabulary.is_empty() {
        return None;
    }
    Some(vocabulary.join(", "))
}

/// What the model can actually do, given what the config asked for.
fn spoken(english_only: bool, wanted: &Speech) -> Speech {
    if english_only {
        return Speech {
            language: Some("en".to_string()),
            translate: false,
        };
    }
    Speech {
        language: wanted.language.clone(),
        // Whisper only ever translates into English, so asking it to translate
        // English is asking for nothing. It is not free: the translate task
        // reads the vocabulary prompt as a list to continue, and every
        // dictation comes back with the leading comma that continues it.
        translate: wanted.translate && wanted.language.as_deref() != Some("en"),
    }
}

pub struct WhisperEngine {
    context: WhisperContext,
    initial_prompt: Option<String>,
    english_only: bool,
    speech: Speech,
    beam_size: i32,
}

impl WhisperEngine {
    pub fn new(
        model: &'static str,
        vocabulary: &[String],
        speech: Speech,
    ) -> Result<Self, BansheeError> {
        log::info!("Loading Whisper AI...");
        Ok(Self {
            context: Self::open(model)?,
            initial_prompt: build_initial_prompt(vocabulary),
            english_only: english_only(model),
            speech,
            beam_size: BEAM_SIZE,
        })
    }

    fn open(model: &str) -> Result<WhisperContext, BansheeError> {
        let whisper_model_path = crate::models::model_path(model)?;
        let whisper_model_path_str = whisper_model_path.to_str().ok_or_else(|| {
            BansheeError::Other(format!(
                "Failed to convert model path {:?} to string.",
                whisper_model_path
            ))
        })?;

        let mut context_params = WhisperContextParameters::default();
        context_params.flash_attn(true);

        WhisperContext::new_with_params(whisper_model_path_str, context_params).map_err(|e| {
            BansheeError::Other(format!("Failed to initialize Whisper context: {:?}", e))
        })
    }
}

impl WhisperEngine {
    /// One pass of the model over `audio`, with every segment it found.
    fn run(&self, audio: &[f32]) -> Result<whisper_rs::WhisperState, BansheeError> {
        let mut state = self
            .context
            .create_state()
            .map_err(|e| BansheeError::Transcription(e.to_string()))?;

        let mut params = FullParams::new(whisper_rs::SamplingStrategy::BeamSearch {
            beam_size: self.beam_size,
            patience: BEAM_PATIENCE,
        });
        let speech = spoken(self.english_only, &self.speech);
        params.set_language(speech.language.as_deref());
        params.set_translate(speech.translate);
        params.set_temperature(0.0);
        params.set_no_context(true);

        if let Some(prompt) = &self.initial_prompt {
            params.set_initial_prompt(prompt);
        }

        state
            .full(params, audio)
            .map_err(|e| BansheeError::Transcription(e.to_string()))?;
        Ok(state)
    }
}

impl Transcriber for WhisperEngine {
    fn transcribe(&self, audio: &[f32]) -> Result<String, BansheeError> {
        let state = self.run(audio)?;
        let mut transcription = String::new();
        for segment in state.as_iter() {
            // Scoring reads every token through the FFI, so only when the line is on
            if log::log_enabled!(log::Level::Debug) {
                let confidence = Confidence::of(&segment);
                log::debug!(
                    "[{} - {}] (no_speech {:.2}, avg_logprob {:.2}): {segment}",
                    // centiseconds
                    segment.start_timestamp(),
                    segment.end_timestamp(),
                    confidence.no_speech_prob,
                    confidence.avg_logprob,
                );
            }
            // The Display impl replaces invalid UTF-8 with the replacement character
            transcription.push_str(&segment.to_string());
        }
        Ok(transcription.trim().to_string())
    }

    /// The words the next transcription leans on. The model behind them does
    /// not move, so this costs nothing.
    fn set_vocabulary(&mut self, words: &[String]) {
        self.initial_prompt = build_initial_prompt(words);
    }

    /// The language and the translate flag are read per utterance, so neither
    /// moves the model.
    fn set_speech(&mut self, speech: Speech) {
        self.speech = speech;
    }

    /// Puts a different model behind the engine, keeping the words it leans on.
    /// The new context is built before the old one is dropped, so a load that
    /// fails leaves the engine transcribing with what it already had.
    fn reload(&mut self, preset: STTPreset) -> Result<Option<&'static str>, BansheeError> {
        let model = preset.model_name();
        self.context = Self::open(model)?;
        self.english_only = english_only(model);
        Ok(Some(model))
    }

    /// The model runs on this machine, so a smaller one is the whole fix.
    fn slow_advice(&self) -> Option<&'static str> {
        Some("Set [stt] preset = \"fast\" in config.toml, then run banshee setup.")
    }
}

#[cfg(test)]
mod speech_tests {
    use super::spoken;
    use crate::speech_to_text::{Speech, english_only};

    fn wants(language: &str, translate: bool) -> Speech {
        Speech {
            language: Some(language.to_string()),
            translate,
        }
    }

    /// The English-only build holds no other language, so it answers an English
    /// shape to whatever it hears rather than refusing. Ask it for English.
    #[test]
    fn an_english_only_model_reads_english_whatever_is_asked() {
        let got = spoken(english_only("ggml-base.en.bin"), &wants("de", true));
        assert_eq!(got.language.as_deref(), Some("en"));
        assert!(!got.translate, "it holds no language to translate from");
    }

    /// The multilingual builds carry every language and the translate task in
    /// the same weights, so nothing is downgraded.
    #[test]
    fn a_multilingual_model_is_asked_for_what_the_config_says() {
        let wanted = wants("hi", true);
        assert_eq!(
            spoken(english_only("ggml-large-v3-turbo-q5_0.bin"), &wanted),
            wanted
        );
        assert_eq!(
            spoken(english_only("ggml-large-v3-q5_0.bin"), &wanted),
            wanted
        );
    }

    /// Whisper translates into English, so translating English is asking for
    /// nothing, and the task changes what it writes: with a comma-separated
    /// vocabulary prompt every dictation comes back with a leading comma.
    #[test]
    fn translating_english_into_english_is_not_asked_for() {
        let got = spoken(false, &wants("en", true));
        assert!(!got.translate);
    }

    /// The task still matters for every other language, which is the whole
    /// point of it.
    #[test]
    fn translating_another_language_into_english_is() {
        let got = spoken(false, &wants("hi", true));
        assert!(got.translate);
    }

    /// `None` is Whisper's own word for detect it, and a multilingual model
    /// must keep it rather than be pinned to one language.
    #[test]
    fn detection_survives_a_multilingual_model() {
        let detect = Speech {
            language: None,
            translate: false,
        };
        assert_eq!(
            spoken(english_only("ggml-large-v3-turbo-q5_0.bin"), &detect),
            detect
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // whisper.cpp allocates one decoder per beam and refuses more than eight,
    // answering a generic "Error code: -4" that names nothing. Measured by
    // asking for twelve.
    #[test]
    fn the_beam_stays_inside_what_whisper_allocates() {
        assert!(
            (1..=8).contains(&BEAM_SIZE),
            "whisper.cpp refuses a beam over 8, and fails with an error naming nothing"
        );
    }

    #[test]
    fn vocabulary_becomes_prompt() {
        assert_eq!(build_initial_prompt(&[]), None);
        let vocabulary = ["banshee".to_string(), "tokio".to_string()];
        assert_eq!(
            build_initial_prompt(&vocabulary).as_deref(),
            Some("banshee, tokio")
        );
    }
}

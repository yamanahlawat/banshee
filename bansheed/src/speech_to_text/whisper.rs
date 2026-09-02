use banshee_common::{WhisperConfig, error::BansheeError, utils::get_models_path};
use whisper_rs::{FullParams, WhisperContext, WhisperContextParameters};

const NO_SPEECH_PROB_GATE: f32 = 0.6;
const AVG_LOGPROB_GATE: f32 = -1.0;

// Both must fail together: the model doubts speech exists AND doubts its own words
fn is_hallucination(no_speech_prob: f32, avg_logprob: f32) -> bool {
    no_speech_prob > NO_SPEECH_PROB_GATE && avg_logprob < AVG_LOGPROB_GATE
}

fn build_initial_prompt(vocabulary: &[String]) -> Option<String> {
    if vocabulary.is_empty() {
        return None;
    }
    Some(vocabulary.join(", "))
}

/// What language the next transcription reads the audio as, and whether it
/// answers in English whatever was said. Whisper translates in one direction
/// only: any language in, English out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speech {
    /// `None` asks Whisper to detect it.
    pub language: Option<String>,
    pub translate: bool,
}

impl From<&crate::config::STTConfig> for Speech {
    /// `auto` is the config's word for detect it, and `None` is Whisper's.
    fn from(stt: &crate::config::STTConfig) -> Self {
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
}

impl WhisperEngine {
    pub fn new(
        whisper_config: WhisperConfig,
        vocabulary: &[String],
        speech: Speech,
    ) -> Result<Self, BansheeError> {
        let english_only = english_only(&whisper_config.model_name);
        Ok(Self {
            context: Self::open(whisper_config)?,
            initial_prompt: build_initial_prompt(vocabulary),
            english_only,
            speech,
        })
    }

    /// Puts a different model behind the engine, keeping the words it leans on.
    /// The new context is built before the old one is dropped, so a load that
    /// fails leaves the engine transcribing with what it already had.
    pub fn reload(&mut self, whisper_config: WhisperConfig) -> Result<(), BansheeError> {
        let english_only = english_only(&whisper_config.model_name);
        self.context = Self::open(whisper_config)?;
        self.english_only = english_only;
        Ok(())
    }

    /// The language the next transcription reads, and whether it answers in
    /// English. Both are read per utterance, so neither moves the model.
    pub fn set_speech(&mut self, speech: Speech) {
        self.speech = speech;
    }

    fn open(whisper_config: WhisperConfig) -> Result<WhisperContext, BansheeError> {
        let models_path = get_models_path().ok_or_else(|| {
            BansheeError::Other(
                "Could not find home directory. Cannot initialize Whisper engine.".to_string(),
            )
        })?;

        let whisper_model_path = models_path.join(&whisper_config.model_name);

        if !whisper_model_path.exists() {
            return Err(BansheeError::Other(format!(
                "Whisper model not found at {:?}. Cannot initialize Whisper engine.",
                whisper_model_path
            )));
        }

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

    /// The words the next transcription leans on. The model behind them does
    /// not move, so this costs nothing.
    pub fn set_vocabulary(&mut self, words: &[String]) {
        self.initial_prompt = build_initial_prompt(words);
    }

    pub fn transcribe(&self, audio_data: &[f32]) -> Result<String, BansheeError> {
        let mut state = self
            .context
            .create_state()
            .map_err(|e| BansheeError::Transcription(e.to_string()))?;

        let mut params = FullParams::new(whisper_rs::SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: -1.0,
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
            .full(params, audio_data)
            .map_err(|e| BansheeError::Transcription(e.to_string()))?;

        let mut transcription = String::new();

        for segment in state.as_iter() {
            let no_speech_prob = segment.no_speech_probability();

            // Decoder confidence: mean ln(p) over the segment's tokens
            let n_tokens = segment.n_tokens();
            let mut logprob_sum = 0.0f32;
            for i in 0..n_tokens {
                if let Some(token) = segment.get_token(i) {
                    // clamp so a zero probability cannot produce -inf
                    logprob_sum += token.token_probability().max(f32::MIN_POSITIVE).ln();
                }
            }
            let avg_logprob = if n_tokens > 0 {
                logprob_sum / n_tokens as f32
            } else {
                0.0
            };

            println!(
                "[{} - {}] (no_speech {:.2}, avg_logprob {:.2}): {}",
                // note start and end timestamps are in centiseconds
                // (10s of milliseconds)
                segment.start_timestamp(),
                segment.end_timestamp(),
                no_speech_prob,
                avg_logprob,
                // the Display impl for WhisperSegment will replace invalid UTF-8 with the Unicode replacement character
                segment
            );

            if is_hallucination(no_speech_prob, avg_logprob) {
                println!("Discarding segment as likely hallucination");
                continue;
            }

            transcription.push_str(&segment.to_string());
        }

        Ok(transcription.trim().to_string())
    }
}

#[cfg(test)]
mod speech_tests {
    use super::{Speech, english_only, spoken};

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

    /// `auto` is the config's word for detect it and `None` is Whisper's.
    #[test]
    fn auto_becomes_the_absence_whisper_reads_as_detect_it() {
        let mut stt = crate::config::STTConfig {
            language: "auto".to_string(),
            ..Default::default()
        };
        assert_eq!(Speech::from(&stt).language, None);
        stt.language = "de".to_string();
        assert_eq!(Speech::from(&stt).language, Some("de".to_string()));
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

    /// The window decides the same thing from the preset name, so the mapping
    /// the two rules meet at is pinned here: change it and this fails rather
    /// than the language control quietly going dead.
    #[test]
    fn only_the_fast_preset_is_english_only() {
        use crate::config::STTPreset;
        assert!(english_only(STTPreset::Fast.model_name()));
        assert!(!english_only(STTPreset::Balanced.model_name()));
        assert!(!english_only(STTPreset::Quality.model_name()));
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

    #[test]
    fn gate_requires_both_signals_to_fail() {
        assert!(is_hallucination(0.9, -1.5));
        // confident words, model just doubts speech was present
        assert!(!is_hallucination(0.9, -0.3));
        // unconfident words, but the model heard speech
        assert!(!is_hallucination(0.2, -1.5));
        assert!(!is_hallucination(0.2, -0.3));
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

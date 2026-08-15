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

pub struct WhisperEngine {
    context: WhisperContext,
    initial_prompt: Option<String>,
}

impl WhisperEngine {
    pub fn new(whisper_config: WhisperConfig, vocabulary: &[String]) -> Result<Self, BansheeError> {
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

        // Enable flash attention for faster inference if supported by the hardware
        let mut context_params = WhisperContextParameters::default();
        context_params.flash_attn(true);

        let context = WhisperContext::new_with_params(whisper_model_path_str, context_params)
            .map_err(|e| {
                BansheeError::Other(format!("Failed to initialize Whisper context: {:?}", e))
            })?;

        Ok(Self {
            context,
            initial_prompt: build_initial_prompt(vocabulary),
        })
    }

    pub fn transcribe(&self, audio_data: &[f32]) -> Result<String, BansheeError> {
        // Holds the temporary memory for a single transcription
        let mut state = self
            .context
            .create_state()
            .map_err(|e| BansheeError::Transcription(e.to_string()))?;

        // Setup the inference parameters
        // Beam search trades speed for accuracy
        let mut params = FullParams::new(whisper_rs::SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: -1.0,
        });
        params.set_language(Some("en"));
        params.set_temperature(0.0);
        params.set_no_context(true);

        // Bias decoding toward the user's vocabulary
        if let Some(prompt) = &self.initial_prompt {
            params.set_initial_prompt(prompt);
        }

        // Run the inference
        state
            .full(params, audio_data)
            .map_err(|e| BansheeError::Transcription(e.to_string()))?;

        // Get the transcribed text
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

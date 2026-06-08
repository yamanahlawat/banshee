use banshee_common::utils::get_models_path;
use whisper_rs::{FullParams, WhisperContext, WhisperContextParameters};

pub struct WhisperEngine {
    context: WhisperContext,
}

impl WhisperEngine {
    pub fn new() -> Result<Self, String> {
        let models_path = get_models_path().ok_or(
            "Could not find home directory. Cannot initialize Whisper engine.".to_string(),
        )?;

        let whisper_model_path = models_path.join("ggml-base.en.bin");

        if !whisper_model_path.exists() {
            return Err(format!(
                "Whisper model not found at {:?}. Cannot initialize Whisper engine.",
                whisper_model_path,
            ));
        }

        let whisper_model_path_str = whisper_model_path.to_str().ok_or(format!(
            "Failed to convert model path {:?} to string.",
            whisper_model_path
        ))?;

        let context = WhisperContext::new_with_params(
            whisper_model_path_str,
            WhisperContextParameters::default(),
        )
        .map_err(|e| format!("Failed to initialize Whisper context: {:?}", e))?;

        Ok(Self { context })
    }

    pub fn transcribe(&self, audio_data: &[f32]) -> Result<String, String> {
        // Holds the temporary memory for a single transcription
        let mut state = self.context.create_state().map_err(|e| e.to_string())?;

        // Setup the inference parameters
        // Greedy Sampling is the fastest way to transcribe
        let mut params = FullParams::new(whisper_rs::SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: -1.0,
        });
        params.set_language(Some("en"));
        params.set_temperature(0.0);
        params.set_no_context(true);

        // Run the inference
        state.full(params, audio_data).map_err(|e| e.to_string())?;

        // Get the transcribed text
        let mut transcription = String::new();

        for segment in state.as_iter() {
            println!(
                "[{} - {}]: {}",
                // note start and end timestamps are in centiseconds
                // (10s of milliseconds)
                segment.start_timestamp(),
                segment.end_timestamp(),
                // the Display impl for WhisperSegment will replace invalid UTF-8 with the Unicode replacement character
                segment
            );

            transcription.push_str(&segment.to_string());
        }

        Ok(transcription)
    }
}

use banshee_common::{SileroVADConfig, utils::get_models_path};
use ort::session::{Session, builder::GraphOptimizationLevel};

// Silero v5 expects each 512-sample chunk to be prefixed with the last 64
// samples of the previous chunk (the official wrapper concatenates this
// "context" outside the ONNX graph). Without it the model returns ~0 speech
// probability for everything.
const CONTEXT_SIZE: usize = 64;

pub struct VADEngine {
    session: Session,
    state: ndarray::Array3<f32>,
    context: Vec<f32>,
}

impl VADEngine {
    pub fn new(vad_config: SileroVADConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let model_path = get_models_path()
            .ok_or("Could not find home directory. Cannot initialize VAD engine.")?;

        let vad_model_path = model_path.join(&vad_config.model_name);

        if !vad_model_path.exists() {
            return Err(format!(
                "VAD model not found at {:?}. Cannot initialize VAD engine.",
                vad_model_path
            )
            .into());
        }

        let vad_model_path_str = vad_model_path.to_str().ok_or(format!(
            "Failed to convert VAD model path {:?} to string.",
            vad_model_path
        ))?;

        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::All)?
            .with_intra_threads(4)?
            .commit_from_file(vad_model_path_str)?;

        let state = ndarray::Array3::<f32>::zeros((2, 1, 128));
        let context = vec![0.0f32; CONTEXT_SIZE];
        Ok(Self {
            session,
            state,
            context,
        })
    }

    pub fn check_speech(
        &mut self,
        audio_data: &[f32],
        target_sample_rate: u32,
    ) -> Result<f32, Box<dyn std::error::Error>> {
        let mut input = Vec::with_capacity(CONTEXT_SIZE + audio_data.len());
        input.extend_from_slice(&self.context);
        input.extend_from_slice(audio_data);

        let input_tensor = ort::value::Tensor::from_array(ndarray::Array2::from_shape_vec(
            (1, input.len()),
            input,
        )?)?;
        let sample_rate_tensor = ort::value::Tensor::from_array(ndarray::Array1::from_vec(vec![
            target_sample_rate as i64,
        ]))?;

        let state_tensor = ort::value::Tensor::from_array(self.state.clone())?;

        let inputs = ort::inputs![
            "input" => input_tensor,
            "sr" => sample_rate_tensor,
            "state" => state_tensor
        ];

        let outputs = self.session.run(inputs)?;
        let (_, probability_data) = outputs["output"].try_extract_tensor::<f32>()?;

        // Grab the raw float array for the new state
        let (_, state_data) = outputs["stateN"].try_extract_tensor::<f32>()?;

        // Re-build our Array3 from the raw floats!
        self.state = ndarray::Array3::from_shape_vec((2, 1, 128), state_data.to_vec())?;

        // Carry the tail of this chunk as context for the next one
        self.context = audio_data[audio_data.len() - CONTEXT_SIZE..].to_vec();

        Ok(probability_data[0])
    }

    pub fn reset_state(&mut self) {
        self.state = ndarray::Array3::<f32>::zeros((2, 1, 128));
        self.context = vec![0.0f32; CONTEXT_SIZE];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // 3.2s of 16kHz mono 16-bit speech, embedded in the test binary so the
    // tests run on any OS without external files or tools.
    const TEST_WAV: &[u8] = include_bytes!("../../tests/data/vad_speech_16k.wav");

    fn load_test_audio() -> Vec<f32> {
        let mut reader = hound::WavReader::new(Cursor::new(TEST_WAV)).expect("invalid test wav");
        assert_eq!(reader.spec().sample_rate, 16000);
        reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / i16::MAX as f32)
            .collect()
    }

    #[test]
    fn detects_speech_in_tts_audio() {
        let samples = load_test_audio();
        let mut vad = VADEngine::new(SileroVADConfig::new("silero_vad.onnx")).unwrap();

        let mut speech = 0;
        let mut total = 0;
        for chunk in samples.chunks(512) {
            if chunk.len() < 512 {
                continue;
            }
            if vad.check_speech(chunk, 16000).unwrap() > 0.5 {
                speech += 1;
            }
            total += 1;
        }
        let ratio = speech as f32 / total as f32;
        println!(
            "speech detected in {speech}/{total} chunks ({:.0}%)",
            ratio * 100.0
        );
        assert!(
            ratio > 0.5,
            "expected most chunks of TTS speech to be flagged, got {speech}/{total}"
        );
    }

    #[test]
    fn rejects_silence() {
        let silence = vec![0.0f32; 512 * 20];
        let mut vad = VADEngine::new(SileroVADConfig::new("silero_vad.onnx")).unwrap();

        for chunk in silence.chunks(512) {
            let p = vad.check_speech(chunk, 16000).unwrap();
            assert!(p < 0.5, "silence flagged as speech with probability {p}");
        }
    }
}

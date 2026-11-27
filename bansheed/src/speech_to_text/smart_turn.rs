use banshee_common::error::BansheeError;
use ort::session::Session;

use super::log_mel;
use crate::models::onnx_fault;

/// Median check on an M-series Mac, release build: 1 thread 54 ms, 2 threads 32 ms, 4 threads 21 ms, 8 threads 19 ms.
/// On Linux x86_64: 1 thread 26.5 ms, 2 threads 16.9 ms, 4 threads 12.5 ms, 8 threads 11.9 ms.
pub const TURN_THREADS: usize = 8;

/// Judges whether the answer so far sounds finished.
pub trait TurnJudge: Send {
    /// P(complete) for 16 kHz mono audio of any length; the last 8 s are heard.
    fn judge(&mut self, audio: &[f32]) -> Result<f32, BansheeError>;
    fn resident(&self) -> bool;
    fn load(&mut self) -> Result<(), BansheeError>;
    /// Answers whether the model was in memory.
    fn unload(&mut self) -> bool;
}

pub struct SmartTurn {
    model: std::path::PathBuf,
    session: Option<Session>,
}

impl SmartTurn {
    /// Loads nothing: `load` reads the model.
    pub fn new(model: std::path::PathBuf) -> Self {
        Self {
            model,
            session: None,
        }
    }
}

fn run(session: &mut Session, audio: &[f32]) -> Result<f32, BansheeError> {
    let features = log_mel::whisper_features(&log_mel::front_padded(audio));
    let tensor = ort::value::Tensor::from_array(
        ndarray::Array3::from_shape_vec((1, 80, 800), features).map_err(onnx_fault)?,
    )
    .map_err(onnx_fault)?;
    let outputs = session
        .run(ort::inputs!["input_features" => tensor])
        .map_err(onnx_fault)?;
    let (_, probability) = outputs[0].try_extract_tensor::<f32>().map_err(onnx_fault)?;
    Ok(probability[0])
}

impl TurnJudge for SmartTurn {
    fn judge(&mut self, audio: &[f32]) -> Result<f32, BansheeError> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| BansheeError::Other("the turn judge is not loaded".to_string()))?;
        run(session, audio)
    }

    fn resident(&self) -> bool {
        self.session.is_some()
    }

    fn load(&mut self) -> Result<(), BansheeError> {
        if self.session.is_none() {
            self.session = Some(crate::models::onnx_session(&self.model, TURN_THREADS, &[])?);
        }
        Ok(())
    }

    fn unload(&mut self) -> bool {
        self.session.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speech_to_text::vad::test_speech;

    const PYTHON_P: f32 = 0.919_974_86;
    const TOLERANCE: f32 = 0.1;

    fn model() -> Option<std::path::PathBuf> {
        std::env::var_os("BANSHEE_TURN_MODEL").map(std::path::PathBuf::from)
    }

    #[test]
    fn an_unloaded_judge_judges_nothing_until_it_is_loaded() {
        let mut judge = SmartTurn::new(std::path::PathBuf::from("/nonexistent/turn.onnx"));
        assert!(!judge.resident());
        assert!(judge.judge(&test_speech()).is_err());
    }

    #[test]
    fn a_missing_model_does_not_load() {
        let mut judge = SmartTurn::new(std::path::PathBuf::from("/nonexistent/turn.onnx"));
        assert!(judge.load().is_err());
        assert!(!judge.resident());
    }

    #[test]
    fn smart_turn_judges_the_fixture_clip() {
        let Some(path) = model() else { return };
        let mut judge = SmartTurn::new(path);
        judge.load().unwrap();
        assert!(judge.resident());
        let p = judge.judge(&test_speech()).unwrap();
        assert!((p - PYTHON_P).abs() < TOLERANCE);
        assert!(p > 0.2);
        let empty = judge.judge(&[]).unwrap();
        assert!((0.0..=1.0).contains(&empty));
        assert!(judge.unload());
        assert!(!judge.resident());
    }

    #[test]
    #[ignore]
    fn smart_turn_check_time() {
        let Some(path) = model() else { return };
        let audio = test_speech();
        for threads in [1, 2, 4, 8] {
            let mut session = crate::models::onnx_session(&path, threads, &[]).unwrap();
            run(&mut session, &audio).unwrap();
            let mut times: Vec<_> = (0..20)
                .map(|_| {
                    let start = std::time::Instant::now();
                    run(&mut session, &audio).unwrap();
                    start.elapsed()
                })
                .collect();
            times.sort();
            println!("{threads} threads: median {:?}", times[10]);
        }
    }
}

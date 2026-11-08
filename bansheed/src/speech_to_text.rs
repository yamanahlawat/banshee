pub mod local;
mod log_mel;
pub mod remote;
mod smart_turn;
pub mod vad;
mod vocabulary;

pub use smart_turn::{SmartTurn, TurnJudge};

use std::path::Path;

use banshee_common::error::BansheeError;
use banshee_common::listening::ListeningModel;

use crate::config::{Provider, STTConfig};
use crate::state::RecordingError;
use local::transcribe::TranscribeEngine;
use remote::openai_compatible::RemoteTranscriber;

/// The rate every `Transcriber` reads, and the rate the pipeline resamples the
/// microphone to before the VAD sees a chunk.
pub const SAMPLE_RATE: u32 = 16_000;

/// Whether the next transcription answers in English whatever was said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speech {
    pub translate: bool,
}

impl From<&STTConfig> for Speech {
    fn from(stt: &STTConfig) -> Self {
        Self {
            translate: stt.translate,
        }
    }
}

/// Where the listener sends what it hears. Two configs with one route need
/// no new listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Local,
    Remote { base_url: String, model: String },
}

impl Route {
    pub fn of(stt: &STTConfig) -> Route {
        match stt.provider {
            Provider::Local => Route::Local,
            Provider::Remote => Route::Remote {
                base_url: stt.remote.base_url.clone(),
                model: stt.remote.model.clone(),
            },
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Route::Remote { .. })
    }
}

/// What the listener in use can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFacts {
    pub translates: bool,
    pub streams: bool,
    pub takes_vocabulary: bool,
}

impl ModelFacts {
    /// `model` is the listening model in use, or `None` for a transcription
    /// server. A server's model is not this machine's to read, so its answer is
    /// what the endpoint offers: a prompt, and a translations path.
    pub fn of(model: Option<&ListeningModel>) -> Self {
        match model {
            Some(model) => Self {
                translates: model.translates,
                streams: model.streams,
                takes_vocabulary: model.takes_vocabulary,
            },
            None => Self {
                translates: true,
                streams: false,
                takes_vocabulary: true,
            },
        }
    }
}

// One thread owns the engine and calls it, so `Send` without `Sync`
pub trait Transcriber: Send {
    /// `audio` is mono `f32` at `SAMPLE_RATE`.
    fn transcribe(&mut self, audio: &[f32]) -> Result<String, BansheeError>;
    /// `transcribe`, for a listener that sends: a notice on `stop` ends the
    /// request, and nothing more of it leaves.
    fn transcribe_until(
        &mut self,
        audio: &[f32],
        _stop: &tokio::sync::Notify,
    ) -> Result<String, BansheeError> {
        self.transcribe(audio)
    }
    /// Opens a stream for the audio that follows, abandoning any stream still
    /// open. False for a listener that does not stream. The stream gives live
    /// words only: the text kept is a `transcribe` of the whole recording.
    fn begin_stream(&mut self) -> Result<bool, BansheeError> {
        Ok(false)
    }
    /// `pcm_16k` is the next part of the open stream's audio. Answers the
    /// committed text when it changed.
    fn feed(&mut self, _pcm_16k: &[f32]) -> Result<Option<String>, BansheeError> {
        Ok(None)
    }
    /// Abandons the open stream, if there is one.
    fn cancel_stream(&mut self) {}
    fn set_vocabulary(&mut self, words: &[String]);
    fn set_speech(&mut self, speech: Speech);
    /// `stt.model` changed live. Answers the model now loaded, or `None` for a
    /// provider that loads no file.
    fn reload(
        &mut self,
        model: &'static ListeningModel,
    ) -> Result<Option<&'static ListeningModel>, BansheeError>;
    /// What shortens a transcription that ran far slower than realtime, or
    /// `None` where the wait is not this machine's to shorten.
    fn slow_advice(&self) -> Option<&'static str> {
        None
    }
    /// The model is in memory. A listener that loads no file is always ready.
    fn resident(&self) -> bool {
        true
    }
    /// Reads the model into memory when it is not there.
    fn load(&mut self) -> Result<(), BansheeError> {
        Ok(())
    }
    /// Frees the model, abandoning any open stream. Answers whether a model
    /// was in memory.
    fn unload(&mut self) -> bool {
        false
    }
    /// The audio leaves this machine.
    fn sends_out(&self) -> bool {
        false
    }
}

/// A listener, and the listening model it waits for when that file is still
/// to download. A waiting listener holds no model until a reload loads one.
/// `refused` says why a remote listener cannot send yet.
pub struct Listener {
    pub engine: Box<dyn Transcriber>,
    pub awaiting: Option<&'static ListeningModel>,
    pub refused: Option<RecordingError>,
}

impl Listener {
    fn ready(engine: Box<dyn Transcriber>) -> Self {
        Self {
            engine,
            awaiting: None,
            refused: None,
        }
    }
}

/// `credentials` is the key file a remote listener reads at each request.
pub fn select_transcriber(stt: &STTConfig, credentials: &Path) -> Result<Listener, RecordingError> {
    match stt.provider {
        Provider::Local => select_local(
            stt,
            crate::models::download::models_dir()
                .map_err(|e| RecordingError::Model(e.to_string()))?,
        ),
        Provider::Remote => select_remote(stt, credentials),
    }
}

/// The listener a running daemon switches to. A local one loads nothing until
/// its next use, and `awaiting` names its model only when the file is absent.
pub fn select_on_demand(stt: &STTConfig, credentials: &Path) -> Result<Listener, RecordingError> {
    match stt.provider {
        Provider::Local => select_local_on_demand(
            stt,
            crate::models::download::models_dir()
                .map_err(|e| RecordingError::Model(e.to_string()))?,
        ),
        Provider::Remote => select_remote(stt, credentials),
    }
}

/// The catalogue model `stt` names, or the fault a listener reports for a
/// name the catalogue does not hold.
pub fn listening_model_of(
    stt: &STTConfig,
) -> Result<&'static banshee_common::listening::ListeningModel, RecordingError> {
    stt.listening_model()
        .ok_or_else(|| RecordingError::Model(format!("'{}' is not a listening model", stt.model)))
}

fn select_local_on_demand(
    stt: &STTConfig,
    models_dir: std::path::PathBuf,
) -> Result<Listener, RecordingError> {
    let model = listening_model_of(stt)?;
    let here = models_dir.join(model.file).exists();
    Ok(Listener {
        engine: Box::new(TranscribeEngine::awaiting(
            models_dir,
            model,
            stt.into(),
            &stt.vocabulary,
        )),
        awaiting: (!here).then_some(model),
        refused: None,
    })
}

fn select_local(
    stt: &STTConfig,
    models_dir: std::path::PathBuf,
) -> Result<Listener, RecordingError> {
    let model = listening_model_of(stt)?;
    if !models_dir.join(model.file).exists() {
        return select_local_on_demand(stt, models_dir);
    }
    let engine = TranscribeEngine::new(models_dir, model, stt.into(), &stt.vocabulary)
        .map_err(|e| RecordingError::Model(e.to_string()))?;
    Ok(Listener::ready(Box::new(engine)))
}

/// Builds whether or not a key is on file: the key is read at each request,
/// so a key written later is the one the next request sends.
fn select_remote(stt: &STTConfig, credentials: &Path) -> Result<Listener, RecordingError> {
    log::info!("Listening through {}", stt.remote.host());
    let engine = RemoteTranscriber::new(
        &stt.remote,
        credentials.to_path_buf(),
        &stt.vocabulary,
        stt.into(),
    )
    .map_err(|e| RecordingError::Provider(e.to_string()))?;
    Ok(Listener {
        refused: key_refusal(credentials),
        ..Listener::ready(Box::new(engine))
    })
}

/// Why a remote listener cannot send with the key file at `credentials`, or
/// `None` when the file holds its key.
pub fn key_refusal(credentials: &Path) -> Option<RecordingError> {
    let side = crate::credentials::RemoteKey::Stt;
    match crate::credentials::Credentials::key_at(credentials, side) {
        Err(error) => Some(RecordingError::KeyFile(error.to_string())),
        Ok(None) => Some(RecordingError::Provider(side.no_key())),
        Ok(Some(_)) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{ModelFacts, RecordingError};

    #[test]
    fn a_listening_model_not_on_disk_is_awaited_by_a_listener_that_holds_none() {
        let dir = crate::test_support::unique_scratch("select-local");
        let stt = crate::config::STTConfig::default();
        let Ok(listener) = super::select_local(&stt, dir.clone()) else {
            panic!("the pipeline stands without the file");
        };
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            listener.awaiting.map(|model| model.id),
            Some(banshee_common::listening::DEFAULT_MODEL)
        );
    }

    #[test]
    fn a_listening_model_answers_the_facts_its_catalogue_entry_holds() {
        let turbo = banshee_common::listening::find("whisper-large-v3-turbo").unwrap();
        assert_eq!(
            ModelFacts::of(Some(turbo)),
            ModelFacts {
                translates: false,
                streams: false,
                takes_vocabulary: true,
            }
        );
        let parakeet =
            banshee_common::listening::find(banshee_common::listening::DEFAULT_MODEL).unwrap();
        assert_eq!(
            ModelFacts::of(Some(parakeet)),
            ModelFacts {
                translates: false,
                streams: true,
                takes_vocabulary: false,
            }
        );
    }

    #[test]
    fn a_remote_listener_with_no_key_still_builds_and_says_why_it_refuses() {
        let listener = super::select_remote(
            &crate::test_support::remote_stt_default(),
            &crate::test_support::key_file(""),
        )
        .expect("the pipeline stands without a key");
        assert!(matches!(
            &listener.refused,
            Some(RecordingError::Provider(reason)) if reason.contains("stt.remote.api_key")
        ));
    }

    #[test]
    fn a_key_file_that_does_not_parse_is_a_key_file_refusal() {
        let path = crate::test_support::key_file("[stt.remote]\napi_key = sk-live-SECRET123\n");
        let listener = super::select_remote(&crate::test_support::remote_stt_default(), &path)
            .expect("the pipeline stands");
        assert!(matches!(
            &listener.refused,
            Some(RecordingError::KeyFile(reason)) if reason.contains("does not parse")
        ));
    }
}

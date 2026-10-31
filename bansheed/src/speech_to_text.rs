pub mod local;
pub mod remote;
pub mod vad;
mod vocabulary;

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
}

/// A listener, and the listening model it waits for when that file is still
/// to download. A waiting listener holds no model until a reload loads one.
pub struct Listener {
    pub engine: Box<dyn Transcriber>,
    pub awaiting: Option<&'static ListeningModel>,
}

impl Listener {
    fn ready(engine: Box<dyn Transcriber>) -> Self {
        Self {
            engine,
            awaiting: None,
        }
    }
}

pub fn select_transcriber(stt: &STTConfig) -> Result<Listener, RecordingError> {
    match stt.provider {
        Provider::Local => select_local(
            stt,
            crate::models::download::models_dir()
                .map_err(|e| RecordingError::Model(e.to_string()))?,
        ),
        Provider::Remote => {
            select_remote(stt, crate::credentials::Credentials::load()).map(Listener::ready)
        }
    }
}

fn select_local(
    stt: &STTConfig,
    models_dir: std::path::PathBuf,
) -> Result<Listener, RecordingError> {
    let model = stt.listening_model().ok_or_else(|| {
        RecordingError::Model(format!("'{}' is not a listening model", stt.model))
    })?;
    if !models_dir.join(model.file).exists() {
        return Ok(Listener {
            engine: Box::new(TranscribeEngine::awaiting(
                models_dir,
                model,
                stt.into(),
                &stt.vocabulary,
            )),
            awaiting: Some(model),
        });
    }
    let engine = TranscribeEngine::new(models_dir, model, stt.into(), &stt.vocabulary)
        .map_err(|e| RecordingError::Model(e.to_string()))?;
    Ok(Listener::ready(Box::new(engine)))
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
    log::info!("Listening through {}", stt.remote.host());
    let engine = RemoteTranscriber::new(&stt.remote, key, &stt.vocabulary, stt.into())
        .map_err(|e| RecordingError::Provider(e.to_string()))?;
    Ok(Box::new(engine))
}

#[cfg(test)]
mod tests {
    use super::{ModelFacts, RecordingError, select_remote};

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
}

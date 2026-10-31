use std::mem::ManuallyDrop;
use std::path::PathBuf;
use std::ptr::NonNull;

use banshee_common::error::BansheeError;
use banshee_common::listening::{DEFAULT_MODEL, ListeningModel, TURBO_ID};
use transcribe_cpp::{
    Model, ParakeetBufferedStreamOptions, RunOptions, Session, SessionOptions, Stream,
    StreamExtension, StreamOptions, Task,
};

use crate::speech_to_text::{ModelFacts, Speech, Transcriber, vocabulary};

/// Parakeet Unified's stream window, (left, chunk, right) in milliseconds.
/// The file accepts a chunk of 80, 160, 560 or 1040 ms and a right context of
/// 0, 80, 160, 240, 320, 560 or 1040 ms; any other value fails at stream start.
const PARAKEET_WINDOW: (i32, i32, i32) = (5600, 560, 560);

/// The model detects the language itself, so no run names one.
fn run_options(facts: ModelFacts, speech: &Speech, words: &[String]) -> RunOptions {
    let task = if speech.translate && facts.translates {
        Task::Translate
    } else {
        Task::Transcribe
    };
    RunOptions {
        task,
        vocabulary: if facts.takes_vocabulary {
            words.to_vec()
        } else {
            Vec::new()
        },
        ..RunOptions::default()
    }
}

/// The window holds for the Parakeet Unified file alone, so every other model
/// streams with its family's own.
fn stream_options(model: &ListeningModel) -> StreamOptions {
    let (left, chunk, right) = PARAKEET_WINDOW;
    StreamOptions {
        family: (model.id == DEFAULT_MODEL).then_some(StreamExtension::ParakeetBuffered(
            ParakeetBufferedStreamOptions {
                left_ms: Some(left),
                chunk_ms: Some(chunk),
                right_ms: Some(right),
            },
        )),
        ..StreamOptions::default()
    }
}

/// The crate refuses to stream a translation.
fn streams(facts: ModelFacts, options: &RunOptions) -> bool {
    facts.streams && options.task != Task::Translate
}

/// A session and the stream that borrows it. The engine keeps the stream
/// between calls, so the session lives on the heap, where the borrow points.
struct Streaming {
    stream: ManuallyDrop<Stream<'static>>,
    session: NonNull<Session>,
}

// SAFETY: the stream and the session it borrows move together and are used
// through `&mut self` alone, and `Session` is `Send`.
unsafe impl Send for Streaming {}

impl Streaming {
    fn begin(
        session: Session,
        run: &RunOptions,
        options: &StreamOptions,
    ) -> Result<Self, (Session, transcribe_cpp::Error)> {
        let session = NonNull::from(Box::leak(Box::new(session)));
        // SAFETY: the leaked session stays put until `release`, which drops
        // the stream before it frees the session. Nothing else reaches it.
        match unsafe { &mut *session.as_ptr() }.stream(run, options) {
            Ok(stream) => Ok(Self {
                stream: ManuallyDrop::new(stream),
                session,
            }),
            // SAFETY: the stream did not begin, so nothing borrows the session.
            Err(error) => Err((*unsafe { Box::from_raw(session.as_ptr()) }, error)),
        }
    }

    /// Abandons the stream, if it still runs, and hands the session back.
    fn end(self) -> Session {
        let mut this = ManuallyDrop::new(self);
        // SAFETY: `this` never drops, so `release` runs once.
        unsafe { this.release() }
    }

    /// # Safety
    /// Called once, and `self` is not used after.
    unsafe fn release(&mut self) -> Session {
        // SAFETY: the stream drops here, once, before the session it borrows.
        unsafe { ManuallyDrop::drop(&mut self.stream) };
        // SAFETY: nothing borrows the session now, and `begin` leaked it from a box.
        *unsafe { Box::from_raw(self.session.as_ptr()) }
    }
}

impl Drop for Streaming {
    fn drop(&mut self) {
        // SAFETY: drop runs once, and nothing reads `self` after it.
        drop(unsafe { self.release() });
    }
}

/// Parakeet is the fastest model offered, and Whisper turbo the fastest one
/// that hears more than English, so neither has a faster model to point at.
fn slow_advice_for(id: &str) -> Option<&'static str> {
    (id != DEFAULT_MODEL && id != TURBO_ID).then_some(
        "Set [stt] model = \"whisper-large-v3-turbo\" in config.toml, then run banshee setup.",
    )
}

pub struct TranscribeEngine {
    models_dir: PathBuf,
    model: &'static ListeningModel,
    /// The session holds the native model, so dropping it frees the weights.
    /// It is `None` while a stream holds it.
    session: Option<Session>,
    streaming: Option<Streaming>,
    speech: Speech,
    words: Vec<String>,
    /// The words prepared for the corrector, which runs where the model takes none.
    targets: vocabulary::Targets,
}

impl TranscribeEngine {
    pub fn new(
        models_dir: PathBuf,
        model: &'static ListeningModel,
        speech: Speech,
        words: &[String],
    ) -> Result<Self, BansheeError> {
        let session = open(&models_dir, model)?;
        Ok(Self {
            models_dir,
            model,
            session: Some(session),
            streaming: None,
            speech,
            words: words.to_vec(),
            targets: vocabulary::Targets::new(words),
        })
    }

    /// An engine for a model whose file is still to download. It loads
    /// nothing until a `reload`.
    pub fn awaiting(
        models_dir: PathBuf,
        model: &'static ListeningModel,
        speech: Speech,
        words: &[String],
    ) -> Self {
        Self {
            models_dir,
            model,
            session: None,
            streaming: None,
            speech,
            words: words.to_vec(),
            targets: vocabulary::Targets::new(words),
        }
    }

    /// The session with no stream open on it.
    fn idle_session(&mut self) -> Result<Session, BansheeError> {
        self.cancel_stream();
        match self.session.take() {
            Some(session) => Ok(session),
            None => open(&self.models_dir, self.model),
        }
    }

    fn finished(&self, text: &str) -> String {
        let text = text.trim();
        if self.model.takes_vocabulary {
            text.to_string()
        } else {
            self.targets.correct(text)
        }
    }
}

fn failed(error: transcribe_cpp::Error) -> BansheeError {
    BansheeError::Transcription(error.to_string())
}

fn no_stream() -> BansheeError {
    BansheeError::Transcription("no stream is open".to_string())
}

fn open(
    models_dir: &std::path::Path,
    entry: &'static ListeningModel,
) -> Result<Session, BansheeError> {
    let path = crate::models::model_in(models_dir, entry.file)?;
    let fault = |error: transcribe_cpp::Error| {
        BansheeError::Other(format!(
            "the listening model {} did not load: {error}",
            entry.id
        ))
    };
    log::info!("Loading {}...", entry.id);
    let model = Model::load(&path).map_err(fault)?;
    let session = model
        .session_with(&SessionOptions {
            n_threads: crate::models::thread_count() as i32,
            ..SessionOptions::default()
        })
        .map_err(fault)?;
    log::info!("{} runs on {}", entry.id, model.backend());
    Ok(session)
}

impl Transcriber for TranscribeEngine {
    fn transcribe(&mut self, audio: &[f32]) -> Result<String, BansheeError> {
        let options = run_options(ModelFacts::of(Some(self.model)), &self.speech, &self.words);
        let mut session = self.idle_session()?;
        let ran = session.run(audio, &options);
        self.session = Some(session);
        Ok(self.finished(&ran.map_err(failed)?.text))
    }

    fn begin_stream(&mut self) -> Result<bool, BansheeError> {
        let facts = ModelFacts::of(Some(self.model));
        let options = run_options(facts, &self.speech, &self.words);
        if !streams(facts, &options) {
            return Ok(false);
        }
        let session = self.idle_session()?;
        match Streaming::begin(session, &options, &stream_options(self.model)) {
            Ok(streaming) => {
                self.streaming = Some(streaming);
                Ok(true)
            }
            Err((session, error)) => {
                self.session = Some(session);
                Err(failed(error))
            }
        }
    }

    fn feed(&mut self, pcm_16k: &[f32]) -> Result<Option<String>, BansheeError> {
        let streaming = self.streaming.as_mut().ok_or_else(no_stream)?;
        if !streaming
            .stream
            .feed(pcm_16k)
            .map_err(failed)?
            .committed_changed
        {
            return Ok(None);
        }
        let committed = streaming.stream.text().committed;
        Ok(Some(self.finished(&committed)))
    }

    fn cancel_stream(&mut self) {
        if let Some(streaming) = self.streaming.take() {
            self.session = Some(streaming.end());
        }
    }

    fn set_vocabulary(&mut self, words: &[String]) {
        self.words = words.to_vec();
        self.targets = vocabulary::Targets::new(words);
    }

    /// Read per utterance, so the model does not move.
    fn set_speech(&mut self, speech: Speech) {
        self.speech = speech;
    }

    /// The new model loads before the old one drops, so a load that fails
    /// leaves the engine transcribing with what it already had.
    fn reload(
        &mut self,
        model: &'static ListeningModel,
    ) -> Result<Option<&'static ListeningModel>, BansheeError> {
        let session = open(&self.models_dir, model)?;
        // A stream still open holds the old model's session, which a cancel
        // would otherwise hand back over the new one.
        self.streaming = None;
        self.session = Some(session);
        self.model = model;
        Ok(Some(model))
    }

    fn slow_advice(&self) -> Option<&'static str> {
        slow_advice_for(self.model.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSLATES: ModelFacts = ModelFacts {
        translates: true,
        streams: false,
        takes_vocabulary: true,
    };

    #[test]
    fn translation_is_asked_only_of_a_model_that_translates() {
        let wanted = Speech { translate: true };
        assert_eq!(run_options(TRANSLATES, &wanted, &[]).task, Task::Translate);
        let cannot = ModelFacts {
            translates: false,
            ..TRANSLATES
        };
        assert_eq!(run_options(cannot, &wanted, &[]).task, Task::Transcribe);
        let unwanted = Speech { translate: false };
        assert_eq!(
            run_options(TRANSLATES, &unwanted, &[]).task,
            Task::Transcribe
        );
    }

    #[test]
    fn the_corrector_runs_only_without_model_vocabulary() {
        let words = vec!["Svelte".to_string()];
        let engine = |id| {
            let model = banshee_common::listening::find(id).unwrap();
            TranscribeEngine::awaiting(PathBuf::new(), model, Speech { translate: false }, &words)
        };
        assert_eq!(engine(DEFAULT_MODEL).finished(" in Svelt "), "in Svelte");
        assert_eq!(engine(TURBO_ID).finished(" in Svelt "), "in Svelt");
    }

    #[test]
    fn a_model_that_takes_vocabulary_is_handed_the_words() {
        let words = vec!["Banshee".to_string()];
        let speech = Speech { translate: false };
        assert_eq!(run_options(TRANSLATES, &speech, &words).vocabulary, words);
        let without = ModelFacts {
            takes_vocabulary: false,
            ..TRANSLATES
        };
        assert!(run_options(without, &speech, &words).vocabulary.is_empty());
    }

    #[test]
    fn no_run_names_a_language() {
        let options = run_options(TRANSLATES, &Speech { translate: true }, &[]);
        assert_eq!(options.language, None);
        assert_eq!(options.target_language, None);
    }

    #[test]
    fn only_parakeet_unified_streams_in_the_measured_window() {
        let parakeet = banshee_common::listening::find(DEFAULT_MODEL).unwrap();
        let Some(StreamExtension::ParakeetBuffered(window)) = stream_options(parakeet).family
        else {
            panic!("Parakeet Unified streams in the measured window");
        };
        assert_eq!(
            (window.left_ms, window.chunk_ms, window.right_ms),
            (Some(5600), Some(560), Some(560))
        );
        let turbo = banshee_common::listening::find(TURBO_ID).unwrap();
        assert_eq!(stream_options(turbo).family, None);
    }

    #[test]
    fn a_translation_is_never_streamed() {
        let both = ModelFacts {
            streams: true,
            ..TRANSLATES
        };
        let translating = run_options(both, &Speech { translate: true }, &[]);
        assert!(!streams(both, &translating));
        let transcribing = run_options(both, &Speech { translate: false }, &[]);
        assert!(streams(both, &transcribing));
        assert!(
            !streams(TRANSLATES, &transcribing),
            "a model that does not stream"
        );
    }

    #[test]
    fn only_a_model_slower_than_turbo_is_pointed_at_turbo() {
        assert_eq!(slow_advice_for(DEFAULT_MODEL), None);
        assert_eq!(slow_advice_for(TURBO_ID), None);
        let advice = slow_advice_for("whisper-large-v3").expect("turbo is faster");
        assert!(advice.contains(TURBO_ID), "{advice}");
    }

    fn clips_in(dir: &std::path::Path) -> Vec<(PathBuf, Vec<f32>)> {
        let mut wavs: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| Some(entry.ok()?.path()))
            .filter(|path| path.extension().is_some_and(|extension| extension == "wav"))
            .collect();
        wavs.sort();
        assert!(!wavs.is_empty(), "BANSHEE_TCPP_MODELS holds a WAV");
        wavs.into_iter()
            .map(|wav| {
                let mut reader = hound::WavReader::open(&wav).unwrap();
                let spec = reader.spec();
                assert_eq!((spec.channels, spec.sample_rate), (1, 16_000), "{wav:?}");
                let audio = reader
                    .samples::<i16>()
                    .map(|sample| f32::from(sample.unwrap()) / 32_768.0)
                    .collect();
                (wav, audio)
            })
            .collect()
    }

    fn facts_of(model: &Model) -> ModelFacts {
        let capabilities = model.capabilities().unwrap();
        ModelFacts {
            translates: capabilities.supports_translate,
            streams: capabilities.supports_streaming,
            takes_vocabulary: model.supports(transcribe_cpp::Feature::Vocabulary),
        }
    }

    fn clip_in(dir: &std::path::Path) -> Vec<f32> {
        clips_in(dir).swap_remove(0).1
    }

    /// Run with `BANSHEE_TCPP_MODELS` naming a folder that holds the Parakeet
    /// file and one 16 kHz mono Pipecat WAV.
    #[test]
    #[ignore = "needs the Parakeet file and a clip in BANSHEE_TCPP_MODELS"]
    fn parakeet_transcribes_a_pipecat_clip() {
        let dir = PathBuf::from(std::env::var("BANSHEE_TCPP_MODELS").unwrap());
        let parakeet = banshee_common::listening::find(DEFAULT_MODEL).unwrap();
        let file = Model::load(dir.join(parakeet.file)).unwrap();
        assert_eq!(
            facts_of(&file),
            ModelFacts::of(Some(parakeet)),
            "the catalogue and the file disagree"
        );
        assert_eq!(
            file.capabilities().unwrap().languages == ["en"],
            parakeet.english_only()
        );
        drop(file);
        let mut engine =
            TranscribeEngine::new(dir.clone(), parakeet, Speech { translate: false }, &[]).unwrap();

        let audio = clip_in(&dir);
        let text = engine.transcribe(&audio).unwrap();
        assert!(
            text.chars().filter(char::is_ascii_alphabetic).count() > 5,
            "{text:?}"
        );

        engine.session = None;
        assert_eq!(engine.transcribe(&audio).unwrap(), text, "after a drop");
    }

    /// Run with `BANSHEE_TCPP_MODELS` naming a folder that holds the Whisper
    /// turbo file.
    #[test]
    #[ignore = "needs the Whisper turbo file in BANSHEE_TCPP_MODELS"]
    fn whisper_turbo_has_the_facts_its_catalogue_entry_holds() {
        let dir = PathBuf::from(std::env::var("BANSHEE_TCPP_MODELS").unwrap());
        let turbo = banshee_common::listening::find(TURBO_ID).unwrap();
        let file = Model::load(dir.join(turbo.file)).unwrap();
        assert_eq!(
            facts_of(&file),
            ModelFacts::of(Some(turbo)),
            "the catalogue and the file disagree"
        );
        assert_eq!(
            file.capabilities().unwrap().languages == ["en"],
            turbo.english_only()
        );
    }

    /// Run with `BANSHEE_TCPP_MODELS` naming a folder that holds the Parakeet
    /// file and one or more 16 kHz mono Pipecat WAVs.
    #[test]
    #[ignore = "needs the Parakeet file and a clip in BANSHEE_TCPP_MODELS"]
    fn parakeet_streams_a_clip_in_160_ms_feeds() {
        let dir = PathBuf::from(std::env::var("BANSHEE_TCPP_MODELS").unwrap());
        let parakeet = banshee_common::listening::find(DEFAULT_MODEL).unwrap();
        let mut engine =
            TranscribeEngine::new(dir.clone(), parakeet, Speech { translate: false }, &[]).unwrap();
        let clips = clips_in(&dir);
        let mut committed_early = 0;
        for (wav, audio) in &clips {
            assert!(engine.begin_stream().unwrap(), "Parakeet streams");
            let mut committed = Vec::new();
            for feed in audio.chunks(2560) {
                committed.extend(engine.feed(feed).unwrap());
            }
            let typed = engine.transcribe(audio).unwrap();
            eprintln!(
                "{}\n  last words: {:?}\n  typed:      {typed}",
                wav.display(),
                committed.last()
            );
            assert!(
                typed.chars().filter(char::is_ascii_alphabetic).count() > 5,
                "{typed:?}"
            );
            committed_early += usize::from(!committed.is_empty());
        }
        eprintln!(
            "{committed_early} of {} clips showed words while they were fed",
            clips.len()
        );
        assert!(
            committed_early > 0,
            "no clip showed words while it was spoken"
        );

        let audio = &clips[0].1;
        assert!(engine.begin_stream().unwrap());
        engine.feed(&audio[..2560]).unwrap();
        engine.cancel_stream();
        assert!(
            engine.begin_stream().unwrap(),
            "a cancel leaves the model free"
        );
        engine.feed(&audio[..2560]).unwrap();
        assert!(
            engine.begin_stream().unwrap(),
            "a new stream abandons the open one"
        );
        engine.cancel_stream();
        engine.transcribe(audio).unwrap();
    }
}

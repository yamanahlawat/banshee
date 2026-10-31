use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use banshee_common::{COMPANION_VOICE, POCKET_DIR, error::BansheeError};

use crate::config::TTSConfig;
use crate::text_to_speech::output::{CHANNELS, Chunk, Output, SAMPLE_RATE};
use crate::text_to_speech::{
    ActiveUtterance, Fault, Faults, TtsBackend, lock, lock_read, lock_write,
};
use kokoro::{KokoroBackend, KokoroEngine};
use pocket::{PocketBackend, PocketEngine};

pub mod kokoro;
pub mod oov;
pub mod pocket;
pub mod voices;

/// The engine that speaks a voice id. The companion is Pocket's one voice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Pocket,
    Kokoro,
}

impl Engine {
    pub fn of(voice: &str) -> Self {
        if voice == COMPANION_VOICE {
            Engine::Pocket
        } else {
            Engine::Kokoro
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Engine::Pocket => "Pocket",
            Engine::Kokoro => "Kokoro",
        }
    }
}

/// Refuses a voice whose files are not all in `models`, naming the first
/// missing one and `banshee setup`.
fn check_files(models: &Path, voice: &str) -> Result<(), BansheeError> {
    for file in crate::models::download::voice_files(voice) {
        crate::models::model_in(models, &file.name)?;
    }
    Ok(())
}

/// A voice a request names. A Kokoro id must name a voice on disk, or the
/// request could load any `.bin` path as a voice.
fn admit_request(models: &Path, voice: &str) -> Result<(), BansheeError> {
    if Engine::of(voice) == Engine::Kokoro {
        kokoro::installed(models, voice)?;
    }
    check_files(models, voice)
}

/// The one engine that speaks `voice`, loaded from `models`.
fn load_engine(
    models: &Path,
    voice: &str,
    output: Arc<Output>,
    faults: Faults,
) -> Result<Box<dyn TtsBackend>, BansheeError> {
    check_files(models, voice)?;
    Ok(match Engine::of(voice) {
        Engine::Pocket => Box::new(PocketBackend::new(
            PocketEngine::new(&models.join(POCKET_DIR))?,
            output,
            faults,
        )),
        Engine::Kokoro => Box::new(KokoroBackend::new(
            KokoroEngine::new(models, voice)?,
            output,
            faults,
        )),
    })
}

/// The local voice `voice` names, with its engine loaded.
pub fn local_backend(
    models: &Path,
    voice: &str,
    output: Arc<Output>,
    faults: Faults,
) -> Result<Box<dyn TtsBackend>, BansheeError> {
    let backend = load_engine(models, voice, Arc::clone(&output), faults.clone())?;
    Ok(Box::new(LocalBackend {
        models: models.to_path_buf(),
        output,
        faults,
        resident: RwLock::new(Resident {
            engine: Engine::of(voice),
            backend,
        }),
    }))
}

/// The local voice `voice` names, loaded by the first utterance it speaks.
pub fn local_on_first_use(
    models: &Path,
    voice: &str,
    output: Arc<Output>,
    faults: Faults,
) -> OnFirstUse {
    let loaded = models.to_path_buf();
    let (engine_output, loads) = (Arc::clone(&output), faults.clone());
    OnFirstUse::new(
        models,
        voice,
        move |voice: &str| local_backend(&loaded, voice, Arc::clone(&engine_output), loads.clone()),
        output,
        faults,
    )
}

/// `voice` loaded for this one utterance, on a thread of its own, and dropped
/// after it.
fn visit(
    models: &Path,
    output: &Arc<Output>,
    faults: &Faults,
    text: &str,
    voice: &str,
) -> Result<Box<dyn ActiveUtterance>, BansheeError> {
    admit_request(models, voice)?;
    let (models, name) = (models.to_path_buf(), voice.to_string());
    let (output, loads) = (Arc::clone(output), faults.clone());
    Ok(start_once_loaded(
        voice,
        move || load_engine(&models, &name, output, loads).map(Arc::from),
        text,
        Some(voice),
        faults.clone(),
    ))
}

struct Resident {
    engine: Engine,
    backend: Box<dyn TtsBackend>,
}

/// One engine stays loaded. A voice of the other engine replaces it on a live
/// `tts.voice` write, and loads for one utterance when a request names it.
pub struct LocalBackend {
    models: PathBuf,
    output: Arc<Output>,
    faults: Faults,
    resident: RwLock<Resident>,
}

impl TtsBackend for LocalBackend {
    fn start(
        &self,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Box<dyn ActiveUtterance>, BansheeError> {
        let resident = lock_read(&self.resident);
        match voice {
            Some(requested) if Engine::of(requested) != resident.engine => {
                drop(resident);
                visit(&self.models, &self.output, &self.faults, text, requested)
            }
            _ => resident.backend.start(text, voice),
        }
    }

    fn reconfigure(&self, tts: &TTSConfig) -> Option<String> {
        let engine = Engine::of(&tts.voice);
        {
            let resident = lock_read(&self.resident);
            if resident.engine == engine {
                return resident.backend.reconfigure(tts);
            }
        }
        match load_engine(
            &self.models,
            &tts.voice,
            Arc::clone(&self.output),
            self.faults.clone(),
        ) {
            Ok(backend) => {
                *lock_write(&self.resident) = Resident { engine, backend };
                Some(tts.voice.clone())
            }
            Err(error) => {
                log::warn!("The voice {} will not load: {error}", tts.voice);
                None
            }
        }
    }
}

type Load = Arc<dyn Fn(&str) -> Result<Box<dyn TtsBackend>, BansheeError> + Send + Sync>;
type Slot = Arc<Mutex<Option<Arc<dyn TtsBackend>>>>;

/// A local voice loaded by the first utterance it has to speak, and kept from
/// then on. A voice whose files are missing is refused at once; a load that
/// fails is a fault and is tried again by the next utterance. A request that
/// names another voice loads that voice alone for its one utterance, whether
/// or not this voice's files are here.
pub struct OnFirstUse {
    models: PathBuf,
    voice: Arc<Mutex<String>>,
    load: Load,
    loaded: Slot,
    output: Arc<Output>,
    faults: Faults,
}

impl OnFirstUse {
    #[cfg(test)]
    pub(crate) fn faults(&self) -> &Faults {
        &self.faults
    }

    pub fn new(
        models: &Path,
        voice: &str,
        load: impl Fn(&str) -> Result<Box<dyn TtsBackend>, BansheeError> + Send + Sync + 'static,
        output: Arc<Output>,
        faults: Faults,
    ) -> Self {
        Self {
            models: models.to_path_buf(),
            voice: Arc::new(Mutex::new(voice.to_string())),
            load: Arc::new(load),
            loaded: Arc::default(),
            output,
            faults,
        }
    }

    /// Starts the utterance on the voice already kept, or on a voice the
    /// request names other than the one this keeps. Otherwise answers the
    /// configured voice to load, once its files are here.
    fn route(&self, text: &str, voice: Option<&str>) -> Result<Route, BansheeError> {
        let kept = lock(&self.loaded).clone();
        if let Some(backend) = kept {
            return backend.start(text, voice).map(Route::Started);
        }
        let configured = lock(&self.voice).clone();
        if let Some(requested) = voice.filter(|requested| *requested != configured) {
            return visit(&self.models, &self.output, &self.faults, text, requested)
                .map(Route::Started);
        }
        check_files(&self.models, &configured)?;
        Ok(Route::Load(configured))
    }
}

enum Route {
    Started(Box<dyn ActiveUtterance>),
    Load(String),
}

/// Loads `wanted` and keeps it, unless a voice write moved the voice while it
/// loaded: then it speaks this utterance and the next one loads the new voice.
/// The voice it keeps is reported on `loads`.
fn load_kept(
    load: &Load,
    slot: &Slot,
    voice: &Mutex<String>,
    wanted: &str,
    loads: &Faults,
) -> Result<Arc<dyn TtsBackend>, BansheeError> {
    let backend: Arc<dyn TtsBackend> = Arc::from(load(wanted)?);
    let mut slot = lock(slot);
    if let Some(kept) = slot.as_ref() {
        return Ok(Arc::clone(kept));
    }
    if *lock(voice) == wanted {
        *slot = Some(Arc::clone(&backend));
        loads.send(Fault::Loaded);
    }
    Ok(backend)
}

fn not_loaded(name: &str, error: BansheeError) -> BansheeError {
    BansheeError::Other(format!("the voice {name} did not load: {error}"))
}

impl TtsBackend for OnFirstUse {
    fn start(
        &self,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Box<dyn ActiveUtterance>, BansheeError> {
        let configured = match self.route(text, voice)? {
            Route::Started(utterance) => return Ok(utterance),
            Route::Load(configured) => configured,
        };
        let (load, slot, wanted, loads) = (
            Arc::clone(&self.load),
            Arc::clone(&self.loaded),
            Arc::clone(&self.voice),
            self.faults.clone(),
        );
        let name = configured.clone();
        Ok(start_once_loaded(
            &configured,
            move || load_kept(&load, &slot, &wanted, &name, &loads),
            text,
            voice,
            self.faults.clone(),
        ))
    }

    fn start_unlocked(
        &self,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Box<dyn ActiveUtterance>, BansheeError> {
        let configured = match self.route(text, voice)? {
            Route::Started(utterance) => return Ok(utterance),
            Route::Load(configured) => configured,
        };
        load_kept(
            &self.load,
            &self.loaded,
            &self.voice,
            &configured,
            &self.faults,
        )
        .map_err(|error| not_loaded(&configured, error))?
        .start(text, voice)
    }

    fn reconfigure(&self, tts: &TTSConfig) -> Option<String> {
        let slot = lock(&self.loaded);
        if let Some(backend) = slot.clone() {
            drop(slot);
            return backend.reconfigure(tts);
        }
        check_files(&self.models, &tts.voice).ok()?;
        *lock(&self.voice) = tts.voice.clone();
        Some(tts.voice.clone())
    }
}

enum Pending {
    Loading,
    Playing(Box<dyn ActiveUtterance>),
    Ended,
}

/// An utterance whose voice is still loading on a thread of its own.
struct Deferred(Arc<Mutex<Pending>>);

impl ActiveUtterance for Deferred {
    fn is_finished(&mut self) -> bool {
        match &mut *lock(&self.0) {
            Pending::Loading => false,
            Pending::Playing(utterance) => utterance.is_finished(),
            Pending::Ended => true,
        }
    }

    fn stop(&mut self) {
        let mut pending = lock(&self.0);
        if let Pending::Playing(utterance) = &mut *pending {
            utterance.stop();
        }
        *pending = Pending::Ended;
    }
}

/// Ends the utterance when the thread loading its voice panics, so the queue
/// behind it moves on.
struct EndsOnPanic(Arc<Mutex<Pending>>);

impl Drop for EndsOnPanic {
    fn drop(&mut self) {
        if std::thread::panicking() {
            *lock(&self.0) = Pending::Ended;
        }
    }
}

/// Loads a backend and starts `text` on it, on a thread of its own, so no
/// caller holds a lock while an engine loads. A failed load is a fault.
fn start_once_loaded(
    name: &str,
    load: impl FnOnce() -> Result<Arc<dyn TtsBackend>, BansheeError> + Send + 'static,
    text: &str,
    voice: Option<&str>,
    faults: Faults,
) -> Box<dyn ActiveUtterance> {
    let pending = Arc::new(Mutex::new(Pending::Loading));
    let shared = Arc::clone(&pending);
    let (name, text, voice) = (
        name.to_string(),
        text.to_string(),
        voice.map(str::to_string),
    );
    std::thread::spawn(move || {
        let _ends = EndsOnPanic(Arc::clone(&shared));
        let loaded = load().map_err(|error| not_loaded(&name, error));
        let started = loaded.and_then(|backend| {
            if matches!(*lock(&shared), Pending::Ended) {
                return Ok(None);
            }
            backend.start(&text, voice.as_deref()).map(Some)
        });
        let mut pending = lock(&shared);
        match started {
            Ok(Some(mut utterance)) if matches!(*pending, Pending::Ended) => utterance.stop(),
            Ok(Some(utterance)) => *pending = Pending::Playing(utterance),
            Ok(None) => {}
            Err(_) if matches!(*pending, Pending::Ended) => {}
            Err(error) => {
                faults.send(Fault::Failed(error.to_string()));
                *pending = Pending::Ended;
            }
        }
    });
    Box::new(Deferred(pending))
}

/// One chunk of audio, or the fault that ends the utterance. The first chunk
/// that plays clears the last speech error, like the remote speaker's.
pub(crate) fn chunk_or_fault(
    engine: &str,
    synthesized: Result<Vec<f32>, BansheeError>,
    played: &mut bool,
    faults: &Faults,
) -> Option<Chunk> {
    match synthesized {
        Ok(samples) => {
            if !*played {
                *played = true;
                faults.send(Fault::Played);
            }
            Some(Chunk {
                samples,
                rate: SAMPLE_RATE,
                channels: CHANNELS,
            })
        }
        Err(error) => {
            faults.send(Fault::Failed(format!("{engine} synthesis failed: {error}")));
            None
        }
    }
}

#[cfg(test)]
mod tests;

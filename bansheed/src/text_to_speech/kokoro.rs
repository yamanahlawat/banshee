use std::collections::HashSet;
use std::fs;
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};
use std::thread;

use banshee_common::{
    KokoroTTSConfig,
    error::BansheeError,
    utils::{get_models_path, get_oov_log_path},
};
use misaki_rs::lexicon::{Lexicon, PhonemeEntry};
use misaki_rs::{G2P, Language, MToken};
use ort::session::{Session, builder::GraphOptimizationLevel};
use rodio::buffer::SamplesBuffer;
use rodio::mixer::Mixer;
use rodio::{DeviceSinkBuilder, Player};

use super::oov::OovFallback;
use super::{ActiveUtterance, TtsBackend, lock};

const SAMPLE_RATE: std::num::NonZero<u32> = std::num::NonZero::new(24_000).unwrap();
const CHANNELS: std::num::NonZero<u16> = std::num::NonZero::new(1).unwrap();

// Voice files hold one style row per input token count: 510 rows x 256 floats
const STYLE_DIM: usize = 256;
// Model context is 512 with a mandatory pad token at both ends
const MAX_TOKENS: usize = 509;

// Phoneme char -> model token id, from the model repo's tokenizer.json
const VOCAB: &[(char, i64)] = &[
    ('$', 0),
    (';', 1),
    (':', 2),
    (',', 3),
    ('.', 4),
    ('!', 5),
    ('?', 6),
    ('—', 9),
    ('…', 10),
    ('"', 11),
    ('(', 12),
    (')', 13),
    ('“', 14),
    ('”', 15),
    (' ', 16),
    ('̃', 17),
    ('ʣ', 18),
    ('ʥ', 19),
    ('ʦ', 20),
    ('ʨ', 21),
    ('ᵝ', 22),
    ('ꭧ', 23),
    ('A', 24),
    ('I', 25),
    ('O', 31),
    ('Q', 33),
    ('S', 35),
    ('T', 36),
    ('W', 39),
    ('Y', 41),
    ('ᵊ', 42),
    ('a', 43),
    ('b', 44),
    ('c', 45),
    ('d', 46),
    ('e', 47),
    ('f', 48),
    ('h', 50),
    ('i', 51),
    ('j', 52),
    ('k', 53),
    ('l', 54),
    ('m', 55),
    ('n', 56),
    ('o', 57),
    ('p', 58),
    ('q', 59),
    ('r', 60),
    ('s', 61),
    ('t', 62),
    ('u', 63),
    ('v', 64),
    ('w', 65),
    ('x', 66),
    ('y', 67),
    ('z', 68),
    ('ɑ', 69),
    ('ɐ', 70),
    ('ɒ', 71),
    ('æ', 72),
    ('β', 75),
    ('ɔ', 76),
    ('ɕ', 77),
    ('ç', 78),
    ('ɖ', 80),
    ('ð', 81),
    ('ʤ', 82),
    ('ə', 83),
    ('ɚ', 85),
    ('ɛ', 86),
    ('ɜ', 87),
    ('ɟ', 90),
    ('ɡ', 92),
    ('ɥ', 99),
    ('ɨ', 101),
    ('ɪ', 102),
    ('ʝ', 103),
    ('ɯ', 110),
    ('ɰ', 111),
    ('ŋ', 112),
    ('ɳ', 113),
    ('ɲ', 114),
    ('ɴ', 115),
    ('ø', 116),
    ('ɸ', 118),
    ('θ', 119),
    ('œ', 120),
    ('ɹ', 123),
    ('ɾ', 125),
    ('ɻ', 126),
    ('ʁ', 128),
    ('ɽ', 129),
    ('ʂ', 130),
    ('ʃ', 131),
    ('ʈ', 132),
    ('ʧ', 133),
    ('ʊ', 135),
    ('ʋ', 136),
    ('ʌ', 138),
    ('ɣ', 139),
    ('ɤ', 140),
    ('χ', 142),
    ('ʎ', 143),
    ('ʒ', 147),
    ('ʔ', 148),
    ('ˈ', 156),
    ('ˌ', 157),
    ('ː', 158),
    ('ʰ', 162),
    ('ʲ', 164),
    ('↓', 169),
    ('→', 171),
    ('↗', 172),
    ('↘', 173),
    ('ᵻ', 177),
];

fn token_id(c: char) -> Option<i64> {
    VOCAB.iter().find(|(v, _)| *v == c).map(|(_, id)| *id)
}

fn read_voice_file(voice_path: &std::path::Path) -> Result<Vec<f32>, BansheeError> {
    let voice_bytes = fs::read(voice_path).map_err(|e| {
        BansheeError::Other(format!("Failed to read voice file {voice_path:?}: {e}"))
    })?;
    let voice: Vec<f32> = voice_bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect();
    if voice.is_empty() || !voice.len().is_multiple_of(STYLE_DIM) {
        return Err(BansheeError::Other(format!(
            "Voice file {voice_path:?} is not a multiple of {STYLE_DIM} floats"
        )));
    }
    Ok(voice)
}

fn to_io_error(e: BansheeError) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

fn strip_bin_suffix(voice_name: &str) -> &str {
    voice_name.strip_suffix(".bin").unwrap_or(voice_name)
}

fn installed(voice: &str) -> Result<(), BansheeError> {
    if crate::models::installed_voices()
        .iter()
        .any(|id| id == voice)
    {
        Ok(())
    } else {
        Err(BansheeError::Other(format!(
            "Voice {voice} is not installed on this machine."
        )))
    }
}

// Streaming boundary only; the token cap is enforced per window in synthesize
fn sentences(text: &str) -> impl Iterator<Item = &str> {
    text.split_inclusive(['.', '!', '?'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

pub struct KokoroEngine {
    session: Session,
    g2p: G2P,
    voice: Vec<f32>,
    loaded_voice: String,
    speed: f32,
    oov: Option<OovFallback>,
    logged_oov: HashSet<String>,
}

impl KokoroEngine {
    pub fn new(kokoro_config: &KokoroTTSConfig, speed: f32) -> Result<Self, BansheeError> {
        let models_path = get_models_path().ok_or_else(|| {
            BansheeError::Other(
                "Could not find home directory. Cannot initialize Kokoro engine.".to_string(),
            )
        })?;

        let model_path = models_path.join(&kokoro_config.model_name);
        let voice_path = models_path.join(&kokoro_config.voice_name);

        if !model_path.exists() {
            return Err(BansheeError::Other(format!(
                "Kokoro model not found at {model_path:?}. Run 'banshee setup' to download it."
            )));
        }

        // Pre-packed weights cost 32 MB of resident memory and bought no synthesis
        // time: three timed runs each way of a two-sentence utterance all took 1.2 s.
        let session = Session::builder()
            .map_err(|e| BansheeError::Other(e.to_string()))?
            .with_optimization_level(GraphOptimizationLevel::All)
            .map_err(|e| BansheeError::Other(e.to_string()))?
            .with_intra_threads(4)
            .map_err(|e| BansheeError::Other(e.to_string()))?
            .with_config_entry("session.disable_prepacking", "1")
            .map_err(|e| BansheeError::Other(e.to_string()))?
            .commit_from_file(&model_path)
            .map_err(|e| BansheeError::Other(e.to_string()))?;

        let voice = read_voice_file(&voice_path)?;

        let mut g2p = G2P::new(Language::EnglishUS);
        super::pronunciation::install_dictionary(&mut g2p);

        let oov = OovFallback::detect();
        if oov.is_none() {
            eprintln!(
                "espeak-ng not found; unknown words will be spelled out. \
                 Install espeak-ng for better pronunciation (run 'banshee status')."
            );
        }

        Ok(Self {
            session,
            g2p,
            voice,
            loaded_voice: strip_bin_suffix(&kokoro_config.voice_name).to_string(),
            speed,
            oov,
            logged_oov: HashSet::new(),
        })
    }

    pub fn set_voice(&mut self, voice_name: &str) -> Result<(), BansheeError> {
        installed(voice_name)?;
        let models_path = get_models_path().ok_or_else(|| {
            BansheeError::Other(
                "Could not find home directory. Cannot initialize Kokoro engine.".to_string(),
            )
        })?;
        let voice_config = KokoroTTSConfig::new(voice_name);
        let voice_path = models_path.join(&voice_config.voice_name);
        self.voice = read_voice_file(&voice_path)?;
        self.loaded_voice = voice_name.to_string();
        Ok(())
    }

    pub fn ensure_voice(&mut self, voice: &str) -> Result<(), BansheeError> {
        if voice == self.loaded_voice {
            return Ok(());
        }
        self.set_voice(voice)
    }

    pub fn loaded_voice(&self) -> &str {
        &self.loaded_voice
    }

    /// Read into the synthesis tensor for each sentence, so the next one is
    /// spoken at the new rate without the model loading again.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
    }

    #[cfg(test)]
    fn style_fingerprint(&self) -> (usize, Option<f32>) {
        (self.voice.len(), self.voice.first().copied())
    }

    #[cfg(test)]
    fn voice_ptr(&self) -> *const f32 {
        self.voice.as_ptr()
    }

    // Text in, 24kHz mono samples out; empty when nothing is speakable
    pub fn synthesize(&mut self, text: &str) -> Result<Vec<f32>, BansheeError> {
        let mut out = self
            .g2p
            .g2p(text)
            .map_err(|e| BansheeError::Other(format!("G2P failed: {e}")))?;

        // Re-run g2p so misaki reassembles with the inserted pronunciations.
        if self.resolve_oov(&out.1) {
            out = self
                .g2p
                .g2p(text)
                .map_err(|e| BansheeError::Other(format!("G2P failed: {e}")))?;
        }
        let (phonemes, tokens) = out;

        self.note_letter_spelled(&tokens);

        // Unknown phonemes (e.g. the OOV marker) simply drop out here
        let ids: Vec<i64> = phonemes.chars().filter_map(token_id).collect();

        // Windowing guarantees the model's context cap even for
        // terminator-less text that arrives as one giant chunk
        let mut samples = Vec::new();
        for window in ids.chunks(MAX_TOKENS) {
            samples.extend(self.synthesize_window(window)?);
        }
        Ok(samples)
    }

    fn resolve_oov(&mut self, tokens: &[MToken]) -> bool {
        let Some(oov) = &self.oov else { return false };
        let mut resolved = Vec::new();
        for tk in tokens {
            // Skip words with a curated gold entry so espeak never overrides it.
            if is_letter_spelled(tk)
                && !curated(&self.g2p.lexicon, &tk.text)
                && let Some(phonemes) = oov.phonemize(&tk.text)
            {
                // One casing serves the other, so the word is kept as it was spoken.
                resolved.push((tk.text.clone(), phonemes));
            }
        }
        for (word, phonemes) in &resolved {
            self.g2p
                .lexicon
                .golds
                .insert(word.clone(), PhonemeEntry::Simple(phonemes.clone()));
        }
        !resolved.is_empty()
    }

    // Log words still spelled out as table candidates; never changes audio.
    fn note_letter_spelled(&mut self, tokens: &[MToken]) {
        for tk in tokens {
            if is_letter_spelled(tk) && self.logged_oov.insert(tk.text.to_lowercase()) {
                append_oov(&tk.text);
            }
        }
    }

    fn synthesize_window(&mut self, ids: &[i64]) -> Result<Vec<f32>, BansheeError> {
        let style_row = ids.len().min(self.voice.len() / STYLE_DIM - 1);
        let style = self.voice[style_row * STYLE_DIM..][..STYLE_DIM].to_vec();

        let mut input_ids = Vec::with_capacity(ids.len() + 2);
        input_ids.push(0);
        input_ids.extend_from_slice(ids);
        input_ids.push(0);
        let input_len = input_ids.len();

        let input_ids_tensor = ort::value::Tensor::from_array(
            ndarray::Array2::from_shape_vec((1, input_len), input_ids)
                .map_err(|e| BansheeError::Other(e.to_string()))?,
        )
        .map_err(|e| BansheeError::Other(e.to_string()))?;
        let style_tensor = ort::value::Tensor::from_array(
            ndarray::Array2::from_shape_vec((1, STYLE_DIM), style)
                .map_err(|e| BansheeError::Other(e.to_string()))?,
        )
        .map_err(|e| BansheeError::Other(e.to_string()))?;
        let speed_tensor =
            ort::value::Tensor::from_array(ndarray::Array1::from_vec(vec![self.speed]))
                .map_err(|e| BansheeError::Other(e.to_string()))?;

        let outputs = self
            .session
            .run(ort::inputs![
                "input_ids" => input_ids_tensor,
                "style" => style_tensor,
                "speed" => speed_tensor
            ])
            .map_err(|e| BansheeError::Other(e.to_string()))?;

        let (_, samples) = outputs["waveform"]
            .try_extract_tensor::<f32>()
            .map_err(|e| BansheeError::Other(e.to_string()))?;

        Ok(samples.to_vec())
    }
}

fn lock_read<T>(value: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    value.read().unwrap_or_else(|poison| poison.into_inner())
}

/// The voice a live `[tts]` write puts in effect, or `None` when this machine
/// does not hold it. A voice accepted here but missing fails later inside the
/// chunk iterator, where nothing can refuse it and every reply goes silent.
fn voice_to_take(wanted: &str, held: &[String]) -> Option<String> {
    held.iter()
        .any(|id| id == wanted)
        .then(|| wanted.to_string())
}

fn voice_for(requested: Option<&str>, configured: &str) -> String {
    requested.unwrap_or(configured).to_string()
}

pub struct KokoroBackend {
    engine: Arc<Mutex<KokoroEngine>>,
    mixer: Mixer,
    // What an utterance that names no voice speaks in. A live `tts.voice`
    // moves it, so it cannot be fixed at construction.
    configured_voice: RwLock<String>,
}

impl KokoroBackend {
    pub fn new(engine: KokoroEngine) -> Result<Self, BansheeError> {
        let configured_voice = engine.loaded_voice().to_string();
        let sink = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| BansheeError::Other(format!("No audio output device: {e}")))?;
        let mixer = sink.mixer().clone();
        // The !Send sink only has to stay alive, never move; leaking it
        // keeps the output stream open for the daemon's lifetime
        std::mem::forget(sink);

        Ok(Self {
            engine: Arc::new(Mutex::new(engine)),
            mixer,
            configured_voice: RwLock::new(configured_voice),
        })
    }
}

struct KokoroUtterance {
    cancelled: Arc<Mutex<bool>>,
    // is_finished after a panic too, unlike a hand-rolled done flag
    synth: thread::JoinHandle<()>,
    player: Arc<Player>,
}

// One player per utterance: rodio's `append` sleeps until a stopped player
// drains, and a player whose device is gone never drains
fn play(mixer: &Mixer, chunks: impl Iterator<Item = Vec<f32>> + Send + 'static) -> KokoroUtterance {
    let player = Arc::new(Player::connect_new(mixer));
    let cancelled = Arc::new(Mutex::new(false));
    let thread_player = Arc::clone(&player);
    let thread_cancelled = Arc::clone(&cancelled);
    let synth = thread::spawn(move || {
        for samples in chunks {
            if samples.is_empty() {
                continue;
            }
            // Append under the lock so stop() can never race a chunk into a
            // stopped player, where append would sleep
            let guard = lock(&thread_cancelled);
            if *guard {
                break;
            }
            thread_player.append(SamplesBuffer::new(CHANNELS, SAMPLE_RATE, samples));
        }
    });
    KokoroUtterance {
        cancelled,
        synth,
        player,
    }
}

impl TtsBackend for KokoroBackend {
    fn reconfigure(&self, tts: &crate::config::TTSConfig) -> Option<String> {
        let taken = voice_to_take(&tts.voice, &crate::models::installed_voices())?;
        *self
            .configured_voice
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = taken.clone();
        lock(&self.engine).set_speed(tts.speed);
        Some(taken)
    }

    fn start(&self, text: &str, voice: Option<&str>) -> std::io::Result<Box<dyn ActiveUtterance>> {
        if let Some(requested) = voice {
            installed(requested).map_err(to_io_error)?;
        }
        let desired = voice_for(voice, &lock_read(&self.configured_voice));
        let engine = Arc::clone(&self.engine);
        let mut sentences = sentences(text)
            .map(str::to_string)
            .collect::<Vec<_>>()
            .into_iter();
        // Sentence-chunked streaming: the first sentence plays while the
        // rest are still synthesizing
        let chunks = std::iter::from_fn(move || {
            let sentence = sentences.next()?;
            let mut engine = lock(&engine);
            if let Err(e) = engine.ensure_voice(&desired) {
                eprintln!("Kokoro synthesis failed: {e}");
                return None;
            }
            match engine.synthesize(&sentence) {
                Ok(samples) => Some(samples),
                Err(e) => {
                    eprintln!("Kokoro synthesis failed: {e}");
                    None
                }
            }
        });
        Ok(Box::new(play(&self.mixer, chunks)))
    }
}

impl ActiveUtterance for KokoroUtterance {
    fn is_finished(&mut self) -> bool {
        // empty() only drops when the device pulls samples; a dead device keeps
        // an utterance unfinished until stop()
        self.synth.is_finished() && self.player.empty()
    }

    fn stop(&mut self) {
        let mut guard = lock(&self.cancelled);
        *guard = true;
        self.player.stop();
    }
}

// A word misaki can't resolve is spelled letter-by-letter, giving one all-alphabetic
// token whose phonemes are the per-letter names joined by spaces. Resolved words are a
// single contiguous group; hyphenated/numeric tokens aren't all-alphabetic.
// misaki answers the all-caps form from the lowercase entry, so the guard asks the same way.
fn curated(lexicon: &Lexicon, word: &str) -> bool {
    lexicon.in_gold(word) || lexicon.in_gold(&word.to_lowercase())
}

fn is_letter_spelled(tk: &MToken) -> bool {
    let w = tk.text.trim();
    let is_word = w.chars().count() > 1 && w.chars().all(|c| c.is_alphabetic());
    is_word && tk.phonemes.as_deref().is_some_and(|p| p.contains(' '))
}

// Dedup is per run, so a word can repeat across restarts; sort -u at read time
fn append_oov(word: &str) {
    let Some(path) = get_oov_log_path() else {
        return;
    };
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write;
        let _ = writeln!(f, "{word}"); // logging must never break playback
    }
}

#[cfg(test)]
mod tests;

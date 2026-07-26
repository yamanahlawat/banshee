use std::collections::HashSet;
use std::fs;
use std::sync::{Arc, Mutex};
use std::thread;

use banshee_common::{
    KokoroTTSConfig,
    error::BansheeError,
    utils::{get_models_path, get_oov_log_path},
};
use misaki_rs::lexicon::PhonemeEntry;
use misaki_rs::{G2P, Language, MToken};
use ort::session::{Session, builder::GraphOptimizationLevel};
use rodio::buffer::SamplesBuffer;
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

        let session = Session::builder()
            .map_err(|e| BansheeError::Other(e.to_string()))?
            .with_optimization_level(GraphOptimizationLevel::All)
            .map_err(|e| BansheeError::Other(e.to_string()))?
            .with_intra_threads(4)
            .map_err(|e| BansheeError::Other(e.to_string()))?
            .commit_from_file(&model_path)
            .map_err(|e| BansheeError::Other(e.to_string()))?;

        let voice_bytes = fs::read(&voice_path).map_err(|e| {
            BansheeError::Other(format!("Failed to read voice file {voice_path:?}: {e}"))
        })?;
        let voice: Vec<f32> = voice_bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        if voice.is_empty() || !voice.len().is_multiple_of(STYLE_DIM) {
            return Err(BansheeError::Other(format!(
                "Voice file {voice_path:?} is not a multiple of {STYLE_DIM} floats"
            )));
        }

        let mut g2p = G2P::new(Language::EnglishUS);
        super::pronunciation::install_dictionary(&mut g2p);

        let oov = OovFallback::detect();
        if oov.is_none() {
            tracing::info!(
                "espeak-ng not found; unknown words will be spelled out. \
                 Install espeak-ng for better pronunciation (run 'banshee doctor')."
            );
        }

        Ok(Self {
            session,
            g2p,
            voice,
            speed,
            oov,
            logged_oov: HashSet::new(),
        })
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

    // Insert espeak phonemes for each letter-spelled word into misaki's lexicon
    fn resolve_oov(&mut self, tokens: &[MToken]) -> bool {
        let Some(oov) = &self.oov else { return false };
        let mut resolved = Vec::new();
        for tk in tokens {
            // Skip words with a curated gold entry so espeak never overrides it.
            if is_letter_spelled(tk)
                && !self.g2p.lexicon.golds.contains_key(&tk.text)
                && let Some(phonemes) = oov.phonemize(&tk.text)
            {
                // Exact surface: misaki looks up by case.
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

pub struct KokoroBackend {
    engine: Arc<Mutex<KokoroEngine>>,
    player: Arc<Player>,
}

impl KokoroBackend {
    pub fn new(engine: KokoroEngine) -> Result<Self, BansheeError> {
        let sink = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| BansheeError::Other(format!("No audio output device: {e}")))?;
        let player = Player::connect_new(sink.mixer());
        // The !Send sink only has to stay alive, never move; leaking it
        // keeps the output stream open for the daemon's lifetime
        std::mem::forget(sink);

        Ok(Self {
            engine: Arc::new(Mutex::new(engine)),
            player: Arc::new(player),
        })
    }
}

struct KokoroUtterance {
    cancelled: Arc<Mutex<bool>>,
    // is_finished after a panic too, unlike a hand-rolled done flag
    synth: thread::JoinHandle<()>,
    player: Arc<Player>,
}

impl TtsBackend for KokoroBackend {
    fn start(&self, text: &str) -> std::io::Result<Box<dyn ActiveUtterance>> {
        let cancelled = Arc::new(Mutex::new(false));

        let engine = Arc::clone(&self.engine);
        let player = Arc::clone(&self.player);
        let thread_cancelled = Arc::clone(&cancelled);
        let text = text.to_string();

        // Sentence-chunked streaming: the first sentence plays while the
        // rest are still synthesizing
        let synth = thread::spawn(move || {
            for sentence in sentences(&text) {
                let synthesized = lock(&engine).synthesize(sentence);
                let samples = match synthesized {
                    Ok(samples) => samples,
                    Err(e) => {
                        eprintln!("Kokoro synthesis failed: {e}");
                        break;
                    }
                };
                if samples.is_empty() {
                    continue;
                }
                // Append under the lock so stop() can never race a sentence
                // into an already-stopped player
                let guard = lock(&thread_cancelled);
                if *guard {
                    break;
                }
                player.append(SamplesBuffer::new(CHANNELS, SAMPLE_RATE, samples));
            }
        });

        Ok(Box::new(KokoroUtterance {
            cancelled,
            synth,
            player: Arc::clone(&self.player),
        }))
    }
}

impl ActiveUtterance for KokoroUtterance {
    fn is_finished(&mut self) -> bool {
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
fn is_letter_spelled(tk: &MToken) -> bool {
    let w = tk.text.trim();
    let is_word = w.chars().count() > 1 && w.chars().all(|c| c.is_alphabetic());
    is_word && tk.phonemes.as_deref().is_some_and(|p| p.contains(' '))
}

// In-memory dedup resets on restart, so a word may be appended once per run;
// dedup at read time with `sort -u`. Add a persistent index only if that matters.
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
mod tests {
    use super::*;

    #[test]
    fn flags_letter_spelled_words_only() {
        let mut spelled = MToken::new("nginx".into(), "NN".into(), " ".into());
        spelled.phonemes = Some("ˈɛn dʒˈiː ˈaɪ ɛn ɛks".into());
        assert!(is_letter_spelled(&spelled));

        let mut resolved = MToken::new("build".into(), "NN".into(), " ".into());
        resolved.phonemes = Some("bˈɪld".into());
        assert!(!is_letter_spelled(&resolved));

        let mut hyphenated = MToken::new("twenty-one".into(), "CD".into(), " ".into());
        hyphenated.phonemes = Some("twˈɛnti wˈʌn".into());
        assert!(!is_letter_spelled(&hyphenated));
    }

    // Skips unless espeak-ng is installed.
    #[test]
    fn espeak_resolves_a_letter_spelled_word() {
        let Some(oov) = OovFallback::detect() else {
            eprintln!("espeak-ng not installed; skipping");
            return;
        };
        let word = "kustomize"; // not in misaki gold or our tables
        let mut g2p = G2P::new(Language::EnglishUS);
        let spelled = g2p.g2p(word).unwrap().1;
        assert!(
            spelled.iter().any(is_letter_spelled),
            "expected {word} to start out letter-spelled"
        );

        let phonemes = oov.phonemize(word).expect("espeak should phonemize");
        g2p.lexicon
            .golds
            .insert(word.to_string(), PhonemeEntry::Simple(phonemes));

        let after = g2p.g2p(word).unwrap().1;
        assert!(
            !after.iter().any(is_letter_spelled),
            "espeak phonemes should have resolved {word}"
        );
    }

    #[test]
    fn g2p_output_maps_into_vocab() {
        let g2p = G2P::new(Language::EnglishUS);
        let (phonemes, _) = g2p.g2p("Hello, world!").unwrap();
        let ids: Vec<i64> = phonemes.chars().filter_map(token_id).collect();
        assert!(!ids.is_empty(), "no phonemes mapped for: {phonemes}");
        // Most of the phoneme string should map; a low ratio means vocab drift
        assert!(ids.len() * 2 >= phonemes.chars().count());
    }

    // Needs the real model on disk: cargo test kokoro_synthesizes -- --ignored
    #[test]
    #[ignore]
    fn kokoro_synthesizes_audible_speech() {
        let config = KokoroTTSConfig::new("af_heart");
        let mut engine = KokoroEngine::new(&config, 1.0).unwrap();
        let samples = engine.synthesize("Kokoro is alive.").unwrap();
        // ~1s of speech at 24kHz, with actual signal in it
        assert!(samples.len() > 10_000, "too few samples: {}", samples.len());
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.05, "output is near-silence, peak {peak}");
    }

    #[test]
    fn sentences_split_on_terminators() {
        let chunks: Vec<&str> = sentences("Done with the build. Tests pass! Ready?").collect();
        assert_eq!(
            chunks,
            vec!["Done with the build.", "Tests pass!", "Ready?"]
        );
        assert_eq!(
            sentences("no terminator").collect::<Vec<_>>(),
            vec!["no terminator"]
        );
    }
}

use banshee_common::KokoroTTSConfig;
use banshee_common::error::BansheeError;

use crate::config::TTSConfig;
use crate::text_to_speech::local::kokoro;
use crate::text_to_speech::local::kokoro::KokoroEngine;
use crate::text_to_speech::local::oov::OovFallback;
use crate::text_to_speech::{pronunciation, sanitizer};

pub const SAMPLE_RATE: u32 = crate::text_to_speech::output::SAMPLE_RATE.get();

pub const SHORT: &str = "The build finished and every test passed.";

pub const MEDIUM: &str = "The tests passed on the first run. Two warnings remain in the \
    audio module, and both are safe to ignore for now. Shall I open a pull request against \
    the develop branch?";

pub const LONG: &str = "While the daemon waits for the next request it keeps the model in \
    memory, so the first sentence of a reply can start playing almost at once, and while \
    that sentence plays the engine prepares the next one, then the one after that, so the \
    listener hears a steady stream of speech instead of long pauses between the parts of a \
    thought, and when the text arrives as one long run with no full stop at all, as it does \
    here, the engine cuts it into windows that fit the model, speaks each window in turn, \
    and joins the pieces back together, which is exactly the path this passage exists to \
    measure, because a short sentence never reaches that code, and a long paragraph with \
    ordinary punctuation is split into sentences long before any window is needed, so only \
    text like this, a single breathless stretch of words joined by commas and conjunctions, \
    reaches the windowing path and shows how long the engine takes when it has to work \
    through several windows in a row";

pub fn text_path(text: &str) -> Vec<String> {
    let clean = sanitizer::sanitize(text);
    let normalized = pronunciation::normalize(&clean);
    kokoro::sentences(&normalized).map(str::to_string).collect()
}

pub fn english_g2p() -> misaki_rs::G2P {
    kokoro::english_g2p()
}

fn defaults() -> (TTSConfig, KokoroTTSConfig) {
    let tts = TTSConfig::default();
    let kokoro = KokoroTTSConfig::new(&tts.voice);
    (tts, kokoro)
}

pub fn missing_model() -> Option<String> {
    match crate::models::download::models_dir() {
        Ok(dir) => missing_file(&dir),
        Err(error) => Some(error.to_string()),
    }
}

fn missing_file(dir: &std::path::Path) -> Option<String> {
    let (_, config) = defaults();
    crate::models::missing_in(dir, &[&config.model_name, &config.voice_name])
        .into_iter()
        .next()
        .map(|name| format!("{name} is not in the models folder"))
}

pub fn engine() -> Result<KokoroEngine, BansheeError> {
    let (tts, config) = defaults();
    KokoroEngine::new(&config, tts.speed)
}

pub fn run_facts() -> String {
    let (tts, _) = defaults();
    let espeak = if OovFallback::available() {
        "found"
    } else {
        "absent"
    };
    format!(
        "voice {}, speed {}, threads {}, espeak-ng {espeak}",
        tts.voice,
        tts.speed,
        crate::models::kokoro_threads()
    )
}

#[cfg(test)]
mod tests;

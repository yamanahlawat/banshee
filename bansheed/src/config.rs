use std::path::{Path, PathBuf};

use banshee_common::{error::BansheeError, utils::get_config_path};
use serde::{Deserialize, Deserializer, Serialize};

// Every section denies unknown fields: TOML binds a key to whatever table
// precedes it, so a misplaced setting parses fine and silently does nothing.

#[derive(Deserialize, Serialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonConfig {
    pub always_on: bool,
    pub save_history: bool,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            always_on: true,
            save_history: true,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyMode {
    Hold,
    Toggle,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum BargeInMode {
    Stop,
    Duck,
    None,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct AudioCuesConfig {
    pub enabled: bool,
    pub start: Option<PathBuf>,
    pub stop: Option<PathBuf>,
    pub ready: Option<PathBuf>,
    pub error: Option<PathBuf>,
}

impl Default for AudioCuesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            start: None,
            stop: None,
            ready: None,
            error: None,
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct AudioConfig {
    pub input_device: String,
    pub hotkey: crate::binding::Hotkey,
    pub hotkey_mode: HotkeyMode,
    pub barge_in: BargeInMode,
    pub cues: AudioCuesConfig,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input_device: crate::audio::DEFAULT_INPUT_DEVICE.to_string(),
            hotkey: crate::binding::Hotkey::default(),
            hotkey_mode: HotkeyMode::Hold,
            barge_in: BargeInMode::Stop,
            cues: AudioCuesConfig::default(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum STTPreset {
    Fast,
    Balanced,
    Quality,
}

impl STTPreset {
    pub const ALL: [STTPreset; 3] = [STTPreset::Fast, STTPreset::Balanced, STTPreset::Quality];

    pub fn model_name(&self) -> &'static str {
        match self {
            STTPreset::Fast => "ggml-base.en.bin",
            STTPreset::Balanced => "ggml-large-v3-turbo-q5_0.bin",
            STTPreset::Quality => "ggml-large-v3-q5_0.bin",
        }
    }
}

fn language<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    Ok(spoken_or_english(value))
}

/// A language the engine reads, or the config's own word for detect it, which
/// is not in the engine's table. Both sides of the liberal-read strict-write
/// split ask this, so it is asked once.
pub fn known_language(value: &str) -> bool {
    value == "auto" || whisper_rs::get_lang_id(value).is_some()
}

/// A code Whisper does not know, read as English rather than refused. Nothing
/// read this field before, so a config written then can hold anything, and a
/// daemon that exits on it is a daemon launchd restarts for ever. `banshee
/// config set` refuses the same value at the boundary, where a person is there
/// to read why.
fn spoken_or_english(value: String) -> String {
    if known_language(&value) {
        return value;
    }
    eprintln!("banshee: '{value}' is not a language Whisper knows, so English is read instead");
    "en".to_string()
}

// Out of range no probability ever matches, so VAD stops firing with no error
fn probability<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    let value = f32::deserialize(deserializer)?;
    if !(0.0..=1.0).contains(&value) {
        return Err(serde::de::Error::custom(format!(
            "must be between 0.0 and 1.0, got {value}"
        )));
    }
    Ok(value)
}

// Below 0.5 an utterance drags, above 2.0 it slurs. The window's slider offers
// this range, and the file has to refuse what the slider cannot ask for.
fn rate<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    let value = f32::deserialize(deserializer)?;
    if !(0.5..=2.0).contains(&value) {
        return Err(serde::de::Error::custom(format!(
            "must be between 0.5 and 2.0, got {value}"
        )));
    }
    Ok(value)
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct STTConfig {
    pub preset: STTPreset,
    /// A Whisper language code, or `auto` to detect it. The English-only build
    /// holds no other language, so `preset = "fast"` reads English whatever
    /// this says.
    #[serde(deserialize_with = "language")]
    pub language: String,
    pub translate: bool,
    #[serde(deserialize_with = "probability")]
    pub vad_threshold: f32,
    pub vocabulary: Vec<String>,
    // Trailing silence that ends an armed-listening answer
    pub endpoint_silence_ms: u64,
}

impl Default for STTConfig {
    fn default() -> Self {
        Self {
            preset: STTPreset::Balanced,
            language: "en".to_string(),
            translate: false,
            vad_threshold: 0.5,
            vocabulary: vec!["banshee".to_string()],
            endpoint_silence_ms: 2500,
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum TTSFallback {
    System,
    None,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct TTSConfig {
    pub voice: String,
    #[serde(deserialize_with = "rate")]
    pub speed: f32,
    pub fallback: TTSFallback,
}

impl Default for TTSConfig {
    fn default() -> Self {
        Self {
            voice: "af_sky".to_string(),
            speed: 1.2,
            fallback: TTSFallback::System,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub daemon: DaemonConfig,
    pub audio: AudioConfig,
    pub stt: STTConfig,
    pub tts: TTSConfig,
    /// Parsed so a `config.toml` that carries it still loads, and never written
    /// back or reported, so it does not read as a setting.
    #[serde(default, skip_serializing)]
    #[allow(dead_code, reason = "parsed only so an older config still loads")]
    logging: Option<toml::Value>,
}

impl Config {
    pub fn path() -> Result<PathBuf, BansheeError> {
        get_config_path()
            .ok_or_else(|| BansheeError::Other("Failed to get config path".to_string()))
    }

    /// Empty rather than an error when the file is absent, because no file means
    /// every default.
    pub fn read(path: &Path) -> Result<String, BansheeError> {
        if path.exists() {
            Ok(std::fs::read_to_string(path)?)
        } else {
            Ok(String::new())
        }
    }

    pub fn load() -> Result<Self, BansheeError> {
        let contents = Config::read(&Config::path()?)?;
        Ok(toml::from_str(&contents)?)
    }
}

#[cfg(test)]
mod language_tests {
    /// Nothing read this field before, so a config written then can hold any
    /// string. Exiting on one is a daemon launchd restarts for ever.
    #[test]
    fn an_unknown_code_reads_as_english_rather_than_stopping_the_daemon() {
        let config: super::Config =
            toml::from_str("[stt]\nlanguage = \"en-US\"\n").expect("an old config must load");
        assert_eq!(config.stt.language, "en");
    }

    #[test]
    fn a_code_the_engine_knows_is_kept() {
        let config: super::Config = toml::from_str("[stt]\nlanguage = \"hi\"\n").unwrap();
        assert_eq!(config.stt.language, "hi");
    }

    /// `auto` is the config's own word for detect it and is not in the engine's
    /// table, so it has to survive the same check.
    #[test]
    fn auto_survives() {
        let config: super::Config = toml::from_str("[stt]\nlanguage = \"auto\"\n").unwrap();
        assert_eq!(config.stt.language, "auto");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `hotkey_mode` under `[tts]` is valid TOML, so only `deny_unknown_fields`
    // stands between a misplaced key and a silent default
    #[test]
    fn a_key_under_the_wrong_section_is_rejected() {
        let misplaced = "[tts]\nvoice = \"af_sky\"\nhotkey_mode = \"toggle\"\n";
        let error = toml::from_str::<Config>(misplaced)
            .expect_err("a key in the wrong section must not parse");
        assert!(
            error.to_string().contains("hotkey_mode"),
            "the error must name the offending key: {error}"
        );

        let placed = "[audio]\nhotkey_mode = \"toggle\"\n\n[tts]\nvoice = \"af_sky\"\n";
        let config: Config = toml::from_str(placed).expect("the same key parses under [audio]");
        assert!(matches!(config.audio.hotkey_mode, HotkeyMode::Toggle));
    }

    // The listener matches what this field parses, so an unmatchable binding
    // must fail the config load, not sit silent behind a working-looking file
    #[test]
    fn a_hotkey_the_listener_cannot_match_is_refused() {
        let error = toml::from_str::<Config>("[audio]\nhotkey = \"banana\"\n")
            .expect_err("an unknown key name must not parse");
        assert!(
            error.to_string().contains("RightOption"),
            "the error must list the legal names: {error}"
        );

        let config: Config = toml::from_str("[audio]\nhotkey = \"RightOption\"\n").unwrap();
        assert_eq!(
            config.audio.hotkey,
            crate::binding::Hotkey::Modifier(rdev::Key::AltGr)
        );
    }
}

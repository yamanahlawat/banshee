use std::path::{Path, PathBuf};

use banshee_common::{error::BansheeError, utils::get_config_path};
use serde::{Deserialize, Deserializer};

// Every section denies unknown fields: TOML binds a key to whatever table
// precedes it, so a misplaced setting parses fine and silently does nothing.

#[derive(Deserialize, Debug)]
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

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyMode {
    Hold,
    Toggle,
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum BargeInMode {
    Stop,
    Duck,
    None,
}

#[derive(Deserialize, Debug)]
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

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct AudioConfig {
    pub input_device: String,
    pub hotkey: String,
    pub hotkey_mode: HotkeyMode,
    pub barge_in: BargeInMode,
    pub cues: AudioCuesConfig,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input_device: crate::audio::DEFAULT_INPUT_DEVICE.to_string(),
            hotkey: "F5".to_string(),
            hotkey_mode: HotkeyMode::Hold,
            barge_in: BargeInMode::Stop,
            cues: AudioCuesConfig::default(),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum STTPreset {
    Fast,
    Balanced,
    Quality,
}

impl STTPreset {
    pub fn model_name(&self) -> &'static str {
        match self {
            STTPreset::Fast => "ggml-base.en.bin",
            STTPreset::Balanced => "ggml-large-v3-turbo-q5_0.bin",
            STTPreset::Quality => "ggml-large-v3-q5_0.bin",
        }
    }
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

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct STTConfig {
    pub preset: STTPreset,
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

#[derive(Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum TTSFallback {
    System,
    None,
}

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct TTSConfig {
    pub voice: String,
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

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
        }
    }
}

#[derive(Deserialize, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub daemon: DaemonConfig,
    pub audio: AudioConfig,
    pub stt: STTConfig,
    pub tts: TTSConfig,
    pub logging: LoggingConfig,
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
}

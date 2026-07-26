use std::path::PathBuf;

use banshee_common::{error::BansheeError, utils::get_config_path};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(default)]
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

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyMode {
    Hold,
    Toggle,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum BargeInMode {
    Stop,
    Duck,
    None,
}

#[derive(Deserialize)]
#[serde(default)]
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

#[derive(Deserialize)]
#[serde(default)]
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
            input_device: "default".to_string(),
            hotkey: "F5".to_string(),
            hotkey_mode: HotkeyMode::Hold,
            barge_in: BargeInMode::Stop,
            cues: AudioCuesConfig::default(),
        }
    }
}

#[derive(Deserialize)]
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

#[derive(Deserialize)]
#[serde(default)]
pub struct STTConfig {
    pub preset: STTPreset,
    pub language: String,
    pub translate: bool,
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

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TTSFallback {
    System,
    None,
}

#[derive(Deserialize)]
#[serde(default)]
pub struct TTSConfig {
    pub voice: String,
    pub speed: f32,
    pub fallback: TTSFallback,
}

impl Default for TTSConfig {
    fn default() -> Self {
        Self {
            voice: "af_heart".to_string(),
            speed: 1.0,
            fallback: TTSFallback::System,
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
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

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub daemon: DaemonConfig,
    pub audio: AudioConfig,
    pub stt: STTConfig,
    pub tts: TTSConfig,
    pub logging: LoggingConfig,
}

impl Config {
    pub fn load() -> Result<Self, BansheeError> {
        let config_path = get_config_path()
            .ok_or_else(|| BansheeError::Other("Failed to get config path".to_string()))?;
        if !config_path.exists() {
            return Ok(Config::default());
        }
        let config_content = std::fs::read_to_string(&config_path)?;
        let config: Config = toml::from_str(&config_content)?;
        Ok(config)
    }
}

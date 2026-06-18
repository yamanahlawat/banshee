use banshee_common::{error::BansheeError, utils::get_config_path};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(default)]
pub struct Config {
    pub stt_model: String,
    pub sample_rate: u32,
    pub vad_model: String,
    pub vad_threshold: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            stt_model: "ggml-large-v3-turbo-q5_0.bin".to_string(),
            sample_rate: 16000,
            vad_model: "silero_vad.onnx".to_string(),
            vad_threshold: 0.5,
        }
    }
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

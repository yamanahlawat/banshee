use banshee_common::{error::BansheeError, utils::get_config_path};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(default)]
pub struct Config {
    pub stt_model: String,
    pub vad_model: String,
    pub vad_threshold: f32,
    pub save_history: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            stt_model: "ggml-large-v3-turbo-q5_0.bin".to_string(),
            vad_model: "silero_vad.onnx".to_string(),
            vad_threshold: 0.5,
            save_history: true,
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

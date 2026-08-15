pub mod download;

use crate::config::Config;

/// The models the recording pipeline loads at startup, named in one place so a
/// preflight and the daemon cannot disagree about what has to be on disk.
pub fn required(config: &Config) -> [&'static str; 2] {
    [config.stt.preset.model_name(), crate::VAD_MODEL]
}

pub fn missing(config: &Config) -> Vec<&'static str> {
    let Some(dir) = banshee_common::utils::get_models_path() else {
        return Vec::new();
    };
    required(config)
        .into_iter()
        .filter(|name| !dir.join(name).exists())
        .collect()
}

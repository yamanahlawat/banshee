use std::fs;

use banshee_common::{SileroVADConfig, WhisperConfig, utils::get_models_path};

pub async fn download_models(
    whisper_config: WhisperConfig,
    silero_vad_config: SileroVADConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(models_path) = get_models_path() else {
        println!("Could not find home directory. Skipping model download.");
        return Ok(());
    };

    // Download Whisper.cpp model
    let whisper_model_path = models_path.join(&whisper_config.model_name);

    if !whisper_model_path.exists() {
        fs::create_dir_all(&models_path)?;

        println!("Downloading Whisper:{} model...", whisper_config.model_name);
        let response = reqwest::get(&whisper_config.download_url).await?;
        let bytes = response.bytes().await?;
        let model_path = models_path.join(&whisper_config.model_name);
        fs::write(model_path, bytes)?;
        println!("{} downloaded successfully!", whisper_config.model_name);
    } else {
        println!("Whisper.cpp already exists. Skipping download.");
    }

    // Download Silero VAD model
    let silero_vad_model_path = models_path.join(&silero_vad_config.model_name);
    if !silero_vad_model_path.exists() {
        println!("Downloading Silero VAD model...");
        let response = reqwest::get(&silero_vad_config.download_url).await?;
        let bytes = response.bytes().await?;
        let model_path = models_path.join(&silero_vad_config.model_name);
        fs::write(model_path, bytes)?;
        println!("{} downloaded successfully!", silero_vad_config.model_name);
    } else {
        println!("Silero VAD model already exists. Skipping download.");
    }

    Ok(())
}

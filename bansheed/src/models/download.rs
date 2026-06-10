use std::fs;

use banshee_common::{WhisperConfig, utils::get_models_path};

pub async fn download_models(
    whisper_config: WhisperConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(models_path) = get_models_path() else {
        println!("Could not find home directory. Skipping model download.");
        return Ok(());
    };

    let whisper_model_path = models_path.join(&whisper_config.model_name);

    if !whisper_model_path.exists() {
        fs::create_dir_all(&models_path)?;

        println!("Downloading Whisper:{} model...", whisper_config.model_name);
        let response = reqwest::get(&whisper_config.download_url).await?;
        let bytes = response.bytes().await?;
        let model_path = models_path.join(&whisper_config.model_name);
        fs::write(model_path, bytes)?;
        println!("Model downloaded successfully!");
    } else {
        println!("Whisper.cpp already exists. Skipping download.");
    }

    Ok(())
}

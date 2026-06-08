use std::fs;

use banshee_common::utils::get_models_path;

static WHISPER_CPP_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";

pub async fn download_models() -> Result<(), Box<dyn std::error::Error>> {
    let Some(models_path) = get_models_path() else {
        println!("Could not find home directory. Skipping model download.");
        return Ok(());
    };

    let whisper_model_path = models_path.join("ggml-base.en.bin");

    if !whisper_model_path.exists() {
        println!("Models directory not found. Creating one...");
        fs::create_dir_all(&models_path)?;

        println!("Downloading whisper.cpp model...");
        let response = reqwest::get(WHISPER_CPP_URL).await?;
        let bytes = response.bytes().await?;
        let model_path = models_path.join("ggml-base.en.bin");
        fs::write(model_path, bytes)?;
        println!("Model downloaded successfully!");
    } else {
        println!("Whisper.cpp already exists. Skipping download.");
    }

    Ok(())
}

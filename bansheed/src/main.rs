mod api;
mod args;
mod audio;
mod daemon;
mod dictation;
mod hotkey;
mod models;
mod speech_to_text;
mod text_to_speech;

use args::{Cli, CommandType};
use banshee_common::{SileroVADConfig, WhisperConfig};
use clap::Parser;

use crate::speech_to_text::{vad::VADEngine, whisper::WhisperEngine};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let whisper_config = WhisperConfig::new("ggml-large-v3-turbo-q5_0.bin");
    let silero_vad_config = SileroVADConfig::new("silero_vad.onnx");

    match cli.command {
        CommandType::Serve => {
            let Ok((_stream, consumer, sample_rate)) = audio::start_audio_capture() else {
                eprintln!("Failed to start audio capture");
                return;
            };
            println!("Loading Whisper AI...");
            let Ok(speech_to_text_engine) = WhisperEngine::new(whisper_config) else {
                eprintln!("Failed to initialize Whisper engine");
                return;
            };
            let Ok(vad_engine) = VADEngine::new(silero_vad_config) else {
                eprintln!("Failed to initialize VAD engine");
                return;
            };
            hotkey::hotkey_listener(consumer, speech_to_text_engine, vad_engine, sample_rate);
            if let Err(error) = daemon::run().await {
                eprintln!("Daemon crashed {error}")
            }
        }
        CommandType::Setup => {
            println!("Download models offline!");
            let _ = models::download::download_models(whisper_config, silero_vad_config).await;
        }
        CommandType::Status => {
            println!("Querying the running daemon!");
        }
        CommandType::Listen => {
            println!("Getting latest transcription!");
        }
        CommandType::Speak { text } => {
            println!("Telling the daemon to speak {text}");
        }
    }
}

mod api;
mod args;
mod audio;
mod daemon;
mod hotkey;
mod models;
mod speech_to_text;

use args::{Cli, CommandType};
use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        CommandType::Serve => {
            let Ok((_stream, consumer, sample_rate)) = audio::start_audio_capture() else {
                eprintln!("Failed to start audio capture");
                return;
            };
            println!("Loading Whisper AI...");
            let Ok(speech_to_text_engine) = speech_to_text::whisper::WhisperEngine::new() else {
                eprintln!("Failed to initialize Whisper engine");
                return;
            };
            hotkey::hotkey_listener(consumer, speech_to_text_engine, sample_rate);
            if let Err(error) = daemon::run().await {
                eprintln!("Daemon crashed {error}")
            }
        }
        CommandType::Setup => {
            println!("Download models offline!");
            let _ = models::download::download_models().await;
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

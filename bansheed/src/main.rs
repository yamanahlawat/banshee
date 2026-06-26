mod api;
mod args;
mod audio;
mod config;
mod daemon;
mod dictation;
mod hotkey;
mod models;
mod speech_to_text;
mod state;
mod text_to_speech;

use std::sync::Arc;

use args::{Cli, CommandType};
use banshee_common::{SileroVADConfig, WhisperConfig, utils};
use clap::Parser;

use crate::{
    config::Config,
    speech_to_text::{vad::VADEngine, whisper::WhisperEngine},
};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Failed to load config: {error}");
            return;
        }
    };
    let whisper_config = WhisperConfig::new(&config.stt_model);
    let silero_vad_config = SileroVADConfig::new(&config.vad_model);
    let daemon_state = Arc::new(state::DaemonState::new(
        env!("CARGO_PKG_VERSION"),
        config.stt_model,
        config.vad_model,
        config.vad_threshold,
    ));
    match cli.command {
        CommandType::Serve => {
            let audio_capture_state = Arc::clone(&daemon_state);
            let Ok((_stream, consumer, sample_rate)) =
                audio::start_audio_capture(audio_capture_state)
            else {
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

            let hotkey_listener_state = Arc::clone(&daemon_state);
            hotkey::hotkey_listener(
                consumer,
                speech_to_text_engine,
                vad_engine,
                sample_rate,
                hotkey_listener_state,
            );
            if let Err(error) = daemon::run(&daemon_state).await {
                eprintln!("Daemon crashed {error}")
            }
        }
        CommandType::Setup => {
            println!("Download models offline!");
            let _ = models::download::download_models(whisper_config, silero_vad_config).await;
        }
        CommandType::Status => {
            match utils::call_daemon(banshee_common::BANSHEE_STATUS, serde_json::json!({})).await {
                Ok(result) => println!(
                    "{}",
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                ),
                Err(error) => eprintln!("Failed to get daemon status: {error}"),
            }
        }
        CommandType::Listen => {
            match utils::call_daemon(
                banshee_common::BANSHEE_GET_TRANSCRIPTION,
                serde_json::json!({}),
            )
            .await
            {
                Ok(result) => println!("Transcription: {result:?}"),
                Err(error) => eprintln!("Failed to get transcription: {error}"),
            }
        }
        CommandType::Speak { text } => {
            match utils::call_daemon(
                banshee_common::BANSHEE_SPEAK,
                serde_json::json!({ "text": text }),
            )
            .await
            {
                Ok(result) => println!("Speak command result: {result:?}"),
                Err(error) => eprintln!("Failed to send speak command: {error}"),
            }
        }
    }
}

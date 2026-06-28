mod api;
mod args;
mod audio;
mod config;
mod daemon;
mod dictation;
mod history;
mod hotkey;
mod models;
mod speech_to_text;
mod state;
mod text_to_speech;

use std::sync::{Arc, Mutex};

use args::{Cli, CommandType};
use banshee_common::{
    SileroVADConfig, WhisperConfig,
    error::BansheeError,
    utils::{self, get_db_path},
};
use clap::Parser;

use crate::{
    config::Config,
    history::TranscriptionHistory,
    speech_to_text::{vad::VADEngine, whisper::WhisperEngine},
};

#[tokio::main]
async fn main() -> Result<(), BansheeError> {
    let cli = Cli::parse();
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Failed to load config: {error}");
            return Err(error);
        }
    };
    let whisper_config = WhisperConfig::new(&config.stt_model);
    let silero_vad_config = SileroVADConfig::new(&config.vad_model);

    let db_connection = if config.save_history {
        let db_path = get_db_path()
            .ok_or_else(|| BansheeError::Other("Failed to get database path".to_string()))?;
        let Ok(connection) = rusqlite::Connection::open(db_path) else {
            eprintln!("Failed to open database");
            return Err(BansheeError::Other("Failed to open database".to_string()));
        };
        if let Err(error) = TranscriptionHistory::create_table(&connection) {
            eprintln!("Failed to initialize transcription history: {error}");
            return Err(BansheeError::Other(
                "Failed to initialize transcription history".to_string(),
            ));
        }
        Some(Mutex::new(connection))
    } else {
        None
    };

    let daemon_state = Arc::new(state::DaemonState::new(
        env!("CARGO_PKG_VERSION"),
        config.stt_model,
        config.vad_model,
        config.vad_threshold,
        db_connection,
    ));

    match cli.command {
        CommandType::Serve => {
            let audio_capture_state = Arc::clone(&daemon_state);
            let Ok((_stream, consumer, sample_rate)) =
                audio::start_audio_capture(audio_capture_state)
            else {
                return Err(BansheeError::Other(
                    "Failed to start audio capture".to_string(),
                ));
            };
            println!("Loading Whisper AI...");
            let Ok(speech_to_text_engine) = WhisperEngine::new(whisper_config) else {
                return Err(BansheeError::Other(
                    "Failed to initialize Whisper engine".to_string(),
                ));
            };
            let Ok(vad_engine) = VADEngine::new(silero_vad_config) else {
                return Err(BansheeError::Other(
                    "Failed to initialize VAD engine".to_string(),
                ));
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
    Ok(())
}

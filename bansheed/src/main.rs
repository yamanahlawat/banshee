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

const VAD_MODEL: &str = "silero_vad.onnx";

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
    let whisper_config = WhisperConfig::new(config.stt.preset.model_name());
    let silero_vad_config = SileroVADConfig::new(VAD_MODEL);

    let db_connection = if config.daemon.save_history {
        let db_path = get_db_path()
            .ok_or_else(|| BansheeError::Other("Failed to get database path".to_string()))?;
        let connection =
            rusqlite::Connection::open(db_path).map_err(|e| BansheeError::Other(e.to_string()))?;
        TranscriptionHistory::create_table(&connection)
            .map_err(|e| BansheeError::Other(e.to_string()))?;
        Some(Mutex::new(connection))
    } else {
        None
    };

    let daemon_state = Arc::new(state::DaemonState::new(
        env!("CARGO_PKG_VERSION"),
        config.stt.preset.model_name(),
        VAD_MODEL,
        config.stt.vad_threshold,
        db_connection,
    ));

    match cli.command {
        CommandType::Serve => {
            let audio_capture_state = Arc::clone(&daemon_state);
            let (_stream, consumer, sample_rate) = audio::start_audio_capture(audio_capture_state)?;
            println!("Loading Whisper AI...");
            let speech_to_text_engine = WhisperEngine::new(whisper_config)?;
            let vad_engine = VADEngine::new(silero_vad_config)?;
            let cue_sender = audio::cues::start_cue_player(config.audio.cues.enabled);
            let hotkey_listener_state = Arc::clone(&daemon_state);
            hotkey::hotkey_listener(
                consumer,
                speech_to_text_engine,
                vad_engine,
                sample_rate,
                hotkey_listener_state,
                cue_sender,
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
                Ok(result) => println!(
                    "{}",
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                ),
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
                Ok(result) => println!(
                    "{}",
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                ),
                Err(error) => eprintln!("Failed to send speak command: {error}"),
            }
        }
        CommandType::History => {
            match utils::call_daemon(banshee_common::BANSHEE_HISTORY, serde_json::json!({})).await {
                Ok(result) => println!(
                    "{}",
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                ),
                Err(error) => eprintln!("Failed to get history: {error}"),
            }
        }
        CommandType::ClearHistory => {
            match utils::call_daemon(banshee_common::BANSHEE_CLEAR_HISTORY, serde_json::json!({}))
                .await
            {
                Ok(result) => println!(
                    "{}",
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                ),
                Err(error) => eprintln!("Failed to clear history: {error}"),
            }
        }
    }
    Ok(())
}

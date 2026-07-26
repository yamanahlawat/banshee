mod api;
mod args;
mod audio;
mod config;
mod daemon;
mod dictation;
mod doctor;
mod history;
mod hotkey;
mod models;
mod permissions;
mod service;
mod speech_to_text;
mod state;
mod text_to_speech;

use std::sync::{Arc, Mutex};

use args::{Cli, CommandType};
use banshee_common::{
    KokoroTTSConfig, SileroVADConfig, WhisperConfig,
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
    // Only unwrapped by the arms that read it: RPC commands work without a
    // parseable config, and doctor diagnoses a broken one
    let config_result =
        Config::load().inspect_err(|error| eprintln!("Failed to load config: {error}"));

    match cli.command {
        CommandType::Serve => {
            let config = config_result?;
            let (socket_path, listener) = daemon::claim()?;
            permissions::restart_when_granted();
            let db_connection = if config.daemon.save_history {
                let db_path = get_db_path().ok_or_else(|| {
                    BansheeError::Other("Failed to get database path".to_string())
                })?;
                let connection = rusqlite::Connection::open(db_path)
                    .map_err(|e| BansheeError::Other(e.to_string()))?;
                TranscriptionHistory::create_table(&connection)
                    .map_err(|e| BansheeError::Other(e.to_string()))?;
                Some(Mutex::new(connection))
            } else {
                None
            };

            let speech_backend = text_to_speech::select_backend(&config.tts)?;
            let (commands, command_receiver) = std::sync::mpsc::channel();
            let cue_sender = audio::cues::start_cue_player(config.audio.cues.enabled);
            let daemon_state = Arc::new(state::DaemonState::new(
                env!("CARGO_PKG_VERSION"),
                config.stt.preset.model_name(),
                VAD_MODEL,
                config.stt.vad_threshold,
                db_connection,
                text_to_speech::SpeechPlayer::new(speech_backend),
                commands,
                cue_sender.clone(),
                config.audio.barge_in,
            ));

            let audio_capture_state = Arc::clone(&daemon_state);
            let (_stream, consumer, sample_rate) = audio::start_audio_capture(audio_capture_state)?;
            println!("Loading Whisper AI...");
            let speech_to_text_engine = WhisperEngine::new(
                WhisperConfig::new(config.stt.preset.model_name()),
                &config.stt.vocabulary,
            )?;
            let vad_engine = VADEngine::new(SileroVADConfig::new(VAD_MODEL))?;
            let consumer_thread = hotkey::hotkey_listener(
                hotkey::Pipeline {
                    consumer,
                    speech_to_text: speech_to_text_engine,
                    vad: vad_engine,
                    sample_rate,
                    state: Arc::clone(&daemon_state),
                    cues: cue_sender,
                    endpoint_silence_ms: config.stt.endpoint_silence_ms,
                },
                command_receiver,
            );
            let result = daemon::run(&daemon_state, socket_path, listener).await;
            // Drop the Whisper context before atexit runs, on error paths too:
            // ggml's Metal cleanup asserts if buffers are still resident. Waits
            // for an in-flight transcription or ask session, both hard-bounded
            let _ = daemon_state
                .commands()
                .send(state::ConsumerCommand::Shutdown);
            let _ = consumer_thread.join();
            result?;
        }
        CommandType::Stop => {
            match utils::call_daemon(banshee_common::BANSHEE_STOP, serde_json::json!({})).await {
                Ok(_) => println!("Daemon stopped."),
                Err(BansheeError::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    println!("Daemon is not running.")
                }
                Err(error) => eprintln!("Failed to stop daemon: {error}"),
            }
        }
        CommandType::Doctor => {
            if !doctor::run(config_result).await {
                std::process::exit(1);
            }
        }
        CommandType::Setup => {
            let config = config_result?;
            println!("Download models offline!");
            let _ = models::download::download_models(
                WhisperConfig::new(config.stt.preset.model_name()),
                SileroVADConfig::new(VAD_MODEL),
                KokoroTTSConfig::new(&config.tts.voice),
            )
            .await;
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
        CommandType::Record { action } => {
            let (method, params) = match action {
                args::RecordAction::Start { dictate } => (
                    banshee_common::BANSHEE_RECORD_START,
                    serde_json::json!({ "dictate": dictate }),
                ),
                args::RecordAction::Stop => {
                    (banshee_common::BANSHEE_RECORD_STOP, serde_json::json!({}))
                }
            };
            if let Err(error) = utils::call_daemon(method, params).await {
                eprintln!("Failed to send record command: {error}");
            }
        }
        CommandType::Start => {
            service::install()?;
            if !permissions::input_granted() {
                println!();
                println!(
                    "Accessibility is not granted: the hotkey and dictation stay inert until it is."
                );
                println!("Opening System Settings. Grant it and the daemon picks it up by itself.");
                permissions::open_settings();
            }
        }
        CommandType::Service { action } => match action {
            args::ServiceAction::Uninstall => service::uninstall()?,
        },
    }
    Ok(())
}

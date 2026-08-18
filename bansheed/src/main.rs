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
mod readiness;
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
    state::RecordingError,
};

const VAD_MODEL: &str = "silero_vad.onnx";

/// Capture, the models, and the thread that turns audio into text. All of it or
/// none: with any piece missing the daemon cannot transcribe, so they share one
/// error path and one reason for `banshee status` to report.
fn start_recording(
    daemon_state: &Arc<state::DaemonState>,
    config: &Config,
    command_receiver: std::sync::mpsc::Receiver<state::ConsumerCommand>,
    cues: std::sync::mpsc::Sender<audio::cues::Cue>,
) -> Result<(cpal::Stream, std::thread::JoinHandle<()>), RecordingError> {
    // Both failures stringify to BansheeError::Other, so the stage that failed
    // is only knowable here, at the call
    let (stream, consumer, sample_rate) =
        audio::start_audio_capture(Arc::clone(daemon_state), &config.audio.input_device)
            .map_err(|e| RecordingError::Microphone(e.to_string()))?;
    println!("Loading Whisper AI...");
    let speech_to_text = WhisperEngine::new(
        WhisperConfig::new(config.stt.preset.model_name()),
        &config.stt.vocabulary,
    )
    .map_err(|e| RecordingError::Model(e.to_string()))?;
    let vad = VADEngine::new(SileroVADConfig::new(VAD_MODEL))
        .map_err(|e| RecordingError::Model(e.to_string()))?;
    let thread = hotkey::hotkey_listener(
        hotkey::Pipeline {
            consumer,
            speech_to_text,
            vad,
            sample_rate,
            state: Arc::clone(daemon_state),
            cues,
            endpoint_silence_ms: config.stt.endpoint_silence_ms,
        },
        command_receiver,
    );
    Ok((stream, thread))
}

fn daemon_is_down(error: &BansheeError) -> bool {
    match error {
        BansheeError::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
        ),
        // A socket orphaned by an unclean exit accepts the connection then
        // closes it. tokio's nonblocking connect cannot tell; a blocking one can.
        BansheeError::Serde(_) => {
            utils::get_socket_path().is_some_and(|path| !daemon::socket_answers(&path))
        }
        _ => false,
    }
}

#[tokio::main]
async fn main() -> Result<(), BansheeError> {
    let cli = Cli::parse();
    // Unwrapped only by the arms that read it: RPC works without a parseable
    // config, and doctor diagnoses a broken one
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

            // Held as one binding, and past daemon::run: dropping the stream
            // stops capture, and the thread is the only thing left to join
            let recording =
                match start_recording(&daemon_state, &config, command_receiver, cue_sender) {
                    Ok(pair) => Some(pair),
                    // A missing mic or model leaves the daemon useful rather than
                    // exiting, which the supervisor reads as a crash and retries
                    Err(error) => {
                        eprintln!("Recording is unavailable: {error}");
                        eprintln!(
                            "The daemon is up: speak, status, and history still work. \
                             Recording, dictation, and ask_user do not."
                        );
                        eprintln!("Run `banshee doctor` for the fix.");
                        daemon_state.set_recording_error(error);
                        None
                    }
                };
            // After the pipeline, so a press always reaches record_start: with
            // no pipeline it answers with the error cue rather than nothing
            hotkey::start_global_hotkey(Arc::clone(&daemon_state), config.audio.hotkey_mode);
            let result = daemon::run(&daemon_state, socket_path, listener).await;
            // Drop the Whisper context before atexit: ggml's Metal cleanup
            // asserts if buffers are still resident
            let _ = daemon_state
                .commands()
                .send(state::ConsumerCommand::Shutdown);
            if let Some((_stream, consumer_thread)) = recording {
                let _ = consumer_thread.join();
            }
            result?;
        }
        CommandType::Stop => {
            match utils::call_daemon(banshee_common::BANSHEE_STOP, serde_json::json!({})).await {
                Ok(_) => println!("Daemon stopped."),
                Err(error) if daemon_is_down(&error) => println!("Daemon is not running."),
                Err(error) => eprintln!("Failed to stop daemon: {error}"),
            }
        }
        CommandType::Doctor => {
            if !doctor::run(config_result).await {
                std::process::exit(1);
            }
        }
        CommandType::Readiness => {
            let reply =
                match utils::call_daemon(banshee_common::BANSHEE_READINESS, serde_json::json!({}))
                    .await
                {
                    Ok(reply) => reply,
                    Err(error) if daemon_is_down(&error) => {
                        eprintln!("The daemon is not running. Start it with: banshee start");
                        std::process::exit(1);
                    }
                    Err(error) => {
                        eprintln!("Could not reach the daemon: {error}");
                        std::process::exit(1);
                    }
                };
            let listed = reply.get("blockers").cloned().unwrap_or_default();
            let blockers: Vec<banshee_common::Blocker> = match serde_json::from_value(listed) {
                Ok(blockers) => blockers,
                Err(error) => {
                    eprintln!("The daemon sent a reply this build cannot read: {error}");
                    std::process::exit(1);
                }
            };
            if blockers.is_empty() {
                println!("Nothing is blocking recording.");
                return Ok(());
            }
            for blocker in &blockers {
                println!();
                println!("{} is not ready: {}.", blocker.name, blocker.consequence);
                println!("  Fix: {}", blocker.fix);
            }
            std::process::exit(1);
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
            let log = service::install()?;
            println!("Banshee is running, and starts again at login.");

            // The daemon reports these to its log, which nobody reads on a first run
            let mut blocked = false;
            let hotkey_mode = match &config_result {
                Ok(config) => {
                    let missing = models::missing(&models::required(config));
                    if !missing.is_empty() {
                        blocked = true;
                        println!();
                        println!("Models not downloaded yet: {}.", missing.join(", "));
                        println!("It runs without them, but cannot record. Run: banshee setup");
                    }
                    Some(config.audio.hotkey_mode)
                }
                // main already printed why the config would not load
                Err(_) => {
                    blocked = true;
                    None
                }
            };

            // Opens System Settings, so it goes last: read here, then switch
            blocked |= permissions::guide_missing();

            match hotkey_mode {
                Some(mode) if !blocked => {
                    println!();
                    println!("{}", hotkey::usage_hint(mode));
                }
                _ => println!("Logs: {log}"),
            }
        }
        CommandType::Service { action } => match action {
            args::ServiceAction::Uninstall => service::uninstall()?,
        },
    }
    Ok(())
}

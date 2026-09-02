mod api;
mod args;
mod audio;
mod binding;
mod config;
mod connect;
mod daemon;
mod dictation;
mod history;
mod hotkey;
mod models;
mod permissions;
mod readiness;
mod service;
mod settings;
mod speech_to_text;
mod state;
mod status;
#[cfg(test)]
mod test_support;
mod text_to_speech;

use std::io::{IsTerminal, Write};
use std::sync::Arc;

use args::{Cli, CommandType};
use banshee_common::{SileroVADConfig, Voice, WhisperConfig, error::BansheeError, utils};
use clap::Parser;

use crate::{
    config::Config,
    speech_to_text::{vad::VADEngine, whisper::WhisperEngine},
    state::RecordingError,
};

const VAD_MODEL: &str = "silero_vad.onnx";

/// Turns a model failure into the reported error, and drops the device name
/// with the stream this error takes down. `open_capture` writes that name once
/// `play()` succeeds, and every subscriber is told it.
fn model_failure(daemon_state: &state::DaemonState, reason: String) -> RecordingError {
    daemon_state.set_audio_device(None);
    RecordingError::Model(reason)
}

/// What startup built and resolved. `open` and `missing` seed the watchdog, so
/// its binding never reads them back out of `DaemonState`.
struct Recording {
    stream: cpal::Stream,
    thread: std::thread::JoinHandle<()>,
    open: String,
    missing: Option<String>,
}

/// Capture, the models, and the thread that turns audio into text. All of it or
/// none: with any piece missing the daemon cannot transcribe, so they share one
/// error path and one reason for `banshee status` to report.
fn start_recording(
    daemon_state: &Arc<state::DaemonState>,
    config: &Config,
    command_receiver: std::sync::mpsc::Receiver<state::ConsumerCommand>,
    cues: audio::cues::Cues,
) -> Result<Recording, RecordingError> {
    // Startup selects through the same function the watchdog tick uses, so a
    // device that is absent at boot falls back rather than leaving capture dead
    let selection =
        audio::select(&config.audio.input_device).map_err(RecordingError::Microphone)?;
    // Both failures stringify to BansheeError::Other, so the stage that failed
    // is only knowable here, at the call
    let capture = audio::open_capture(Arc::clone(daemon_state), &selection)
        .map_err(|e| RecordingError::Microphone(e.to_string()))?;
    match &selection.missing {
        Some(name) => println!(
            "Capture opened {}, still waiting for {name}",
            selection.open
        ),
        None => println!("Capture opened {}", selection.open),
    }
    println!("Loading Whisper AI...");
    let speech_to_text = WhisperEngine::new(
        WhisperConfig::new(config.stt.preset.model_name()),
        &config.stt.vocabulary,
        (&config.stt).into(),
    )
    .map_err(|e| model_failure(daemon_state, e.to_string()))?;
    let vad = VADEngine::new(SileroVADConfig::new(VAD_MODEL))
        .map_err(|e| model_failure(daemon_state, e.to_string()))?;
    let thread = hotkey::hotkey_listener(
        hotkey::Pipeline {
            source: hotkey::CaptureSource {
                consumer: capture.consumer,
                sample_rate: capture.sample_rate,
            },
            speech_to_text,
            vad,
            state: Arc::clone(daemon_state),
            cues,
            endpoint_silence_ms: config.stt.endpoint_silence_ms,
        },
        command_receiver,
    );
    // Written once the whole pipeline stands. A model failure drops capture, and
    // a substitution recorded with nothing open contradicts the accessor.
    daemon_state.set_missing_device(selection.missing.clone());
    Ok(Recording {
        stream: capture.stream,
        thread,
        open: selection.open,
        missing: selection.missing,
    })
}

fn show_progress(progress: banshee_common::DownloadProgress) {
    // Rewritten in place, so a percentage does not scroll the screen away
    let ending = if progress.state == banshee_common::DownloadState::Downloading {
        '\r'
    } else {
        '\n'
    };
    // Erase to end of line, so a shorter line does not leave the tail of the
    // longer one behind it. A redirected stdout takes neither the erase nor the
    // carriage return: both reach a log file as themselves.
    let (erase, ending) = if std::io::stdout().is_terminal() {
        ("\x1b[K", ending)
    } else {
        ("", '\n')
    };
    print!("{}{erase}{ending}", progress_line(&progress));
    let _ = std::io::stdout().flush();
}

// One writer at a time: the `.part` file that makes resume possible has a
// stable name, so this process must not fetch alongside a daemon already doing it
async fn follow_daemon_download(mut progress: utils::Subscription) -> Result<(), BansheeError> {
    let reply = utils::call_daemon(
        banshee_common::BANSHEE_DOWNLOAD_MODELS,
        serde_json::json!({}),
    )
    .await?;
    // The counter terminates because only one download runs at a time, so every
    // notification on this connection belongs to the batch just asked for
    let mut pending: usize = reply
        .get("downloading")
        .and_then(|names| names.as_array())
        .map_or(0, Vec::len);
    if pending == 0 {
        println!("Everything is already downloaded.");
        return Ok(());
    }

    while pending > 0 {
        let Some(params) = progress
            .next_of(banshee_common::BANSHEE_DOWNLOAD_PROGRESS)
            .await?
        else {
            return Err(BansheeError::Other(
                "The daemon stopped before the download finished".to_string(),
            ));
        };
        let reported: banshee_common::DownloadProgress = serde_json::from_value(params)?;
        let done = reported.state != banshee_common::DownloadState::Downloading;
        show_progress(reported);
        if done {
            pending -= 1;
        }
    }
    Ok(())
}

fn progress_line(progress: &banshee_common::DownloadProgress) -> String {
    use banshee_common::DownloadState;
    match progress.state {
        DownloadState::Done => format!("{} downloaded", progress.model),
        DownloadState::Failed => format!("{} failed", progress.model),
        DownloadState::Downloading => {
            match models::download::percent(progress.bytes, progress.total) {
                // A daemon older than this field set sends no count at all,
                // and a real run always has count >= 1, so 0 marks a message
                // with no place to report
                Some(done) if progress.count > 0 => format!(
                    "{}, {} of {}  {done}%",
                    progress.label, progress.index, progress.count
                ),
                Some(done) => format!("{} {done}%", progress.model),
                // No Content-Length, so there is no bar to draw: count what arrived
                None => format!("{} {} MB", progress.model, progress.bytes / 1_048_576),
            }
        }
    }
}

// Defaulting to empty would report a newer daemon's reply as nothing at all
fn decoded<T: serde::de::DeserializeOwned>(reply: &serde_json::Value, key: &str) -> T {
    match serde_json::from_value(reply.get(key).cloned().unwrap_or_default()) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("The daemon sent a reply this build cannot read: {error}");
            std::process::exit(1);
        }
    }
}

// One object per line, as Waybar requires. The same word lands in three keys
// because Waybar splits them: `text` shows, `alt` picks a format-icon, `class`
// picks CSS. Readiness is left out: blockers are answered once and never
// pushed, so a bar would show them stale.
fn waybar_line(word: &str, device: Option<&str>, missing: Option<&str>) -> String {
    let tooltip = match (device, missing) {
        // Nothing to say about the microphone, so the word stands alone
        (None, None) => format!("Banshee is {word}"),
        _ => format!(
            "Banshee is {word}. Microphone: {}",
            banshee_common::microphone_label(device, missing)
        ),
    };
    serde_json::json!({
        "text": word,
        "alt": word,
        "class": word,
        "tooltip": tooltip,
    })
    .to_string()
}

fn watch_line(waybar: bool, word: &str, device: Option<&str>, missing: Option<&str>) -> String {
    if waybar {
        waybar_line(word, device, missing)
    } else {
        word.to_string()
    }
}

fn state_word(state: &serde_json::Value) -> &'static str {
    match banshee_common::Activity::of(state) {
        banshee_common::Activity::Idle => "idle",
        banshee_common::Activity::Recording => "recording",
        banshee_common::Activity::Speaking => "speaking",
        banshee_common::Activity::Listening => "listening",
    }
}

/// A device can be both, and hiding either label would read as its being false.
fn device_labels(device: &banshee_common::InputDevice, current: Option<&str>) -> String {
    let mut labels = Vec::new();
    if device.default {
        labels.push("system default");
    }
    if current == Some(device.name.as_str()) {
        labels.push("in use");
    }
    labels.join(", ")
}

/// An answer meant for the caller reads on its own; anything else is this
/// process failing to ask.
fn fail(error: &BansheeError) -> ! {
    match error {
        BansheeError::Rejected(_) | BansheeError::Rpc { .. } => {
            eprintln!("{}", error.rpc_message())
        }
        other => eprintln!("Could not reach the daemon: {other}"),
    }
    std::process::exit(1)
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
    // config, and the checklist diagnoses a broken one
    let config_result =
        Config::load().inspect_err(|error| eprintln!("Failed to load config: {error}"));

    match cli.command {
        CommandType::Serve => {
            let config = Arc::new(config_result?);
            let (socket_path, listener) = daemon::claim()?;
            permissions::ask_for_accessibility();
            permissions::restart_when_granted();
            let db_connection = if config.daemon.save_history {
                Some(history::open()?)
            } else {
                None
            };

            let (speech_backend, live_voice) = text_to_speech::select_backend(&config.tts)?;
            let (commands, command_receiver) = std::sync::mpsc::channel();
            let cues = audio::cues::start_cue_player(config.audio.cues.enabled);
            let daemon_state = Arc::new(state::DaemonState::new(
                env!("CARGO_PKG_VERSION"),
                config.stt.preset.model_name(),
                VAD_MODEL,
                config.stt.vad_threshold,
                config.audio.input_device.clone(),
                db_connection,
                text_to_speech::SpeechPlayer::new(speech_backend),
                commands,
                cues.clone(),
                config.audio.barge_in,
            ));

            if let Some(voice) = live_voice {
                daemon_state.set_tts_voice(voice);
            }
            daemon_state.set_config(Arc::clone(&config));

            // The watchdog owns the stream past daemon::run: stopping it stops
            // capture, and the thread is the only thing left to join
            let recording = match start_recording(&daemon_state, &config, command_receiver, cues) {
                Ok(started) => {
                    let watchdog = audio::watchdog::spawn(
                        Arc::clone(&daemon_state),
                        started.stream,
                        started.open,
                        started.missing,
                    );
                    Some((watchdog, started.thread))
                }
                // A missing mic or model leaves the daemon useful rather than
                // exiting, which the supervisor reads as a crash and retries
                Err(error) => {
                    eprintln!("Recording is unavailable: {error}");
                    eprintln!(
                        "The daemon is up: speak, status, and history still work. \
                             Recording, dictation, and ask_user do not."
                    );
                    eprintln!("Run `banshee status` for the fix.");
                    daemon_state.set_recording_error(error);
                    None
                }
            };
            // After the pipeline, so a press always reaches record_start: with
            // no pipeline it answers with the error cue rather than nothing
            hotkey::start_global_hotkey(
                Arc::clone(&daemon_state),
                config.audio.hotkey,
                config.audio.hotkey_mode,
            );
            let result = daemon::run(&daemon_state, socket_path, listener).await;
            if let Some((watchdog, consumer_thread)) = recording {
                // Capture stops first, so no Rebind arrives at a thread that
                // has already left its loop
                watchdog.stop();
                // Drop the Whisper context before atexit: ggml's Metal cleanup
                // asserts if buffers are still resident
                let _ = daemon_state
                    .commands()
                    .send(state::ConsumerCommand::Shutdown);
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
        CommandType::Devices => {
            let (devices, current) = match utils::call_daemon(
                banshee_common::BANSHEE_LIST_INPUT_DEVICES,
                serde_json::json!({}),
            )
            .await
            {
                Ok(reply) => {
                    let devices: Vec<banshee_common::InputDevice> = decoded(&reply, "devices");
                    let current = reply
                        .get("current")
                        .and_then(|name| name.as_str())
                        .map(str::to_string);
                    (devices, current)
                }
                // You need the names before you can start a daemon on the right one
                Err(error) if daemon_is_down(&error) => (audio::input_devices(), None),
                Err(error) => {
                    fail(&error);
                }
            };

            if devices.is_empty() {
                println!("No microphones found.");
                return Ok(());
            }
            let width = devices
                .iter()
                .map(|d| d.name.chars().count())
                .max()
                .unwrap_or(0);
            for device in &devices {
                let labels = device_labels(device, current.as_deref());
                if labels.is_empty() {
                    println!("  {}", device.name);
                } else {
                    println!("  {:width$}  {labels}", device.name);
                }
            }
            println!();
            println!("Record from one with: banshee config set audio.input_device \"<name>\"");
        }
        CommandType::Voices => {
            let (voices, current) = match utils::call_daemon(
                banshee_common::BANSHEE_LIST_VOICES,
                serde_json::json!({}),
            )
            .await
            {
                Ok(reply) => (
                    decoded::<Vec<Voice>>(&reply, "voices"),
                    reply
                        .get("current")
                        .and_then(|voice| voice.as_str())
                        .map(str::to_string),
                ),
                // A voice gets chosen before there is a daemon to ask. These
                // came off the disk, so every one of them is here.
                Err(error) if daemon_is_down(&error) => (
                    models::installed_voices()
                        .iter()
                        .map(|id| text_to_speech::voices::describe(id, true))
                        .collect(),
                    None,
                ),
                Err(error) => fail(&error),
            };

            // The daemon names every voice it can describe, so what this has to
            // print is the ones that are here: `banshee voices` promises that
            // every name it lists works today.
            let held: Vec<&Voice> = voices.iter().filter(|voice| voice.downloaded).collect();
            if held.is_empty() {
                println!("No voices found. Download one with: banshee setup");
                return Ok(());
            }
            for voice in held {
                let marker = if current.as_deref() == Some(voice.id.as_str()) {
                    "*"
                } else {
                    " "
                };
                println!(
                    "{marker} {}  {}  ({})",
                    voice.name, voice.description, voice.id
                );
            }
            println!();
            println!("Speak with one by: banshee config set tts.voice \"<id>\"");
        }
        CommandType::Watch { waybar } => {
            let (mut state, mut changes) =
                match utils::Subscription::open(&[banshee_common::EVENT_STATE]).await {
                    Ok(subscription) => subscription,
                    Err(error) if daemon_is_down(&error) => {
                        eprintln!("Daemon is not running.");
                        std::process::exit(1);
                    }
                    Err(error) => fail(&error),
                };
            // No real line is empty, so the first one always prints
            let mut shown = String::new();
            loop {
                let line = watch_line(
                    waybar,
                    state_word(&state),
                    banshee_common::audio_device(&state),
                    banshee_common::missing_device(&state),
                );
                // The reader sees the line, so the line is what has to differ
                if line != shown {
                    // `banshee watch | head` closes the pipe. That is the reader
                    // having seen enough, not this command failing
                    if writeln!(std::io::stdout(), "{line}").is_err() {
                        return Ok(());
                    }
                    shown = line;
                }
                state = match changes.next_of(banshee_common::BANSHEE_STATE_CHANGED).await {
                    Ok(Some(params)) => params,
                    // There is no other clean end, so a supervisor can read the
                    // exit code as one
                    Ok(None) => {
                        eprintln!("The daemon closed the connection.");
                        std::process::exit(1);
                    }
                    Err(error) => fail(&error),
                };
            }
        }
        CommandType::Config {
            action: args::ConfigAction::Set { key, value },
        } => {
            // So `0.6` arrives as a number and `de` as a string
            let value: serde_json::Value = serde_json::from_str(&value)
                .unwrap_or_else(|_| serde_json::Value::String(value.clone()));
            let assignments = settings::Assignments::from([(key.clone(), value)]);

            let outcome = match utils::call_daemon(
                banshee_common::BANSHEE_CONFIGURE,
                serde_json::json!({ "settings": &assignments, "persist": true }),
            )
            .await
            {
                Ok(reply) => Ok(reply
                    .get("restart_required")
                    .and_then(|keys| keys.as_array())
                    .is_some_and(|keys| !keys.is_empty())),
                // A daemon that is down never writes, so the CLI can be the one writer
                Err(error) if daemon_is_down(&error) => {
                    settings::configure(None, &assignments, true)
                        .map(|outcome| !outcome.restart_required.is_empty())
                }
                Err(error) => Err(error),
            };

            match outcome {
                Ok(restart_required) => {
                    println!("Set {key} in config.toml.");
                    if restart_required {
                        println!("Restart to use it: banshee start");
                    }
                }
                Err(error) => {
                    fail(&error);
                }
            }
        }
        CommandType::Setup => {
            // Subscribed before the download is asked for, so the first
            // notifications are not lost in the gap
            let watching = utils::Subscription::open(&[banshee_common::EVENT_DOWNLOADS]).await;
            match watching {
                Ok((_, subscription)) => {
                    if let Err(error) = follow_daemon_download(subscription).await {
                        fail(&error);
                    }
                }
                Err(error) if daemon_is_down(&error) => {
                    // No daemon, so this process is the only writer there can be
                    let config = config_result?;
                    let dir = models::download::models_dir()?;
                    let missing =
                        models::download::still_missing(&models::download::wanted(&config), &dir);
                    if missing.is_empty() {
                        println!("Everything is already downloaded.");
                        return Ok(());
                    }
                    // Not `fail`: nothing was asked of a daemon here, so the
                    // reason stands on its own
                    if let Err(error) =
                        models::download::download_all(&dir, &missing, &mut show_progress).await
                    {
                        eprintln!("{error}");
                        std::process::exit(1);
                    }
                }
                Err(error) => fail(&error),
            }
        }
        CommandType::Status { json } => {
            if !json {
                if !status::run(config_result).await {
                    std::process::exit(1);
                }
                return Ok(());
            }
            // The same probe the checklist uses, so the two halves of one command
            // cannot disagree about whether a daemon is up
            let reply = match status::probe_daemon().await {
                status::Daemon::Running { status, .. } | status::Daemon::Legacy(status) => status,
                _ => {
                    println!("{}", serde_json::json!({"running": false}));
                    std::process::exit(1);
                }
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&reply).unwrap_or_else(|_| reply.to_string())
            );
            // The checklist's verdict, and absent leaves the code alone
            if reply.get("ready") == Some(&serde_json::Value::Bool(false)) {
                std::process::exit(1);
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
            let log = service::install(service::Agent::Daemon)?;
            println!("Banshee is running, and starts again at login.");

            // The daemon reports these to its log, which nobody reads on a first run
            let mut blocked = false;
            let binding = match &config_result {
                Ok(config) => {
                    let missing = models::missing(&models::required(config));
                    if !missing.is_empty() {
                        blocked = true;
                        println!();
                        println!("Models not downloaded yet: {}.", missing.join(", "));
                        println!("It runs without them, but cannot record. Run: banshee setup");
                    }
                    Some((config.audio.hotkey, config.audio.hotkey_mode))
                }
                // main already printed why the config would not load
                Err(_) => {
                    blocked = true;
                    None
                }
            };

            permissions::grant_note();

            match binding {
                Some((hotkey, mode)) if !blocked => {
                    println!();
                    println!("{}", hotkey::usage_hint(hotkey, mode));
                }
                _ => println!("Logs: {log}"),
            }
        }
        CommandType::Tray { uninstall } => {
            if uninstall {
                if service::uninstall(service::Agent::Tray)? {
                    println!("The menu bar icon no longer starts at login.");
                } else {
                    println!("The menu bar icon was not set to start at login.");
                }
            } else {
                let log = service::install(service::Agent::Tray)?;
                println!("The menu bar icon is running, and comes back at login.");
                println!("Logs: {log}");
            }
        }
        CommandType::Connect { agent, yes } => {
            if let Err(error) = connect::run(agent.map(Into::into), yes) {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        CommandType::Service { action } => match action {
            // Every entry, so none is left behind to fail at the next login
            args::ServiceAction::Uninstall => {
                let mut removed = false;
                for agent in service::Agent::ALL {
                    if service::uninstall(agent)? {
                        println!("The {} no longer starts at login.", agent.name());
                        removed = true;
                    }
                }
                if !removed {
                    println!("Nothing was set to start at login.");
                }
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests;

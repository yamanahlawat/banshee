use banshee_common::utils::get_socket_path;
use banshee_common::{
    BANSHEE_DOWNLOAD_PROGRESS, BANSHEE_STATE_CHANGED, BANSHEE_SUBSCRIBE, DownloadProgress,
    JsonRpcNotification, JsonRpcRequest, SileroVADConfig, WhisperConfig, error::BansheeError,
};
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Mutex, broadcast, watch};

use crate::api::{dispatch, live_state};
use crate::config::Config;
use crate::speech_to_text::{vad::VADEngine, whisper::WhisperEngine};
use crate::state::{ConsumerCommand, DaemonState, RecordingError};
use crate::{audio, history, hotkey, models, permissions, text_to_speech};

// Claimed before model loading, so a lost single-instance race stays cheap
pub fn claim() -> Result<(std::path::PathBuf, UnixListener), io::Error> {
    let socket_path = get_socket_path()
        .ok_or_else(|| io::Error::other("could not find home directory for the socket path"))?;

    if let Some(parent_dir) = socket_path.parent() {
        fs::create_dir_all(parent_dir)?;
    }

    let listener = claim_socket(&socket_path)?;
    // owner-only: the socket is a command channel into the mic and speakers
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    Ok((socket_path, listener))
}

/// Turns a model failure into the reported error, and drops the device name
/// with the stream this error takes down. `open_capture` writes that name once
/// `play()` succeeds, and every subscriber is told it.
pub(crate) fn model_failure(daemon_state: &DaemonState, reason: String) -> RecordingError {
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
    daemon_state: &Arc<DaemonState>,
    config: &Config,
    command_receiver: std::sync::mpsc::Receiver<ConsumerCommand>,
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
    let vad = VADEngine::new(SileroVADConfig::new(models::VAD_MODEL))
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

pub async fn start(config: Config) -> Result<(), BansheeError> {
    let config = Arc::new(config);
    let (socket_path, listener) = claim()?;
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
    let daemon_state = Arc::new(DaemonState::new(
        Arc::clone(&config),
        db_connection,
        text_to_speech::SpeechPlayer::new(speech_backend),
        commands,
        cues.clone(),
    ));

    if let Some(voice) = live_voice {
        daemon_state.set_tts_voice(voice);
    }

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
    let result = run(&daemon_state, socket_path, listener).await;
    if let Some((watchdog, consumer_thread)) = recording {
        // Capture stops first, so no Rebind arrives at a thread that
        // has already left its loop
        watchdog.stop();
        // Drop the Whisper context before atexit: ggml's Metal cleanup
        // asserts if buffers are still resident
        let _ = daemon_state.commands().send(ConsumerCommand::Shutdown);
        let _ = consumer_thread.join();
    }
    result?;
    Ok(())
}

pub async fn run(
    daemon_state: &Arc<DaemonState>,
    socket_path: std::path::PathBuf,
    listener: UnixListener,
) -> Result<(), std::io::Error> {
    println!("Listening on {}", socket_path.display());

    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    // Coarse tick: the ceiling it enforces is measured in minutes
    let mut watchdog = tokio::time::interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            _ = sigint.recv() => break,
            _ = sigterm.recv() => break,
            _ = watchdog.tick() => {
                daemon_state.expire_stuck_recording();
            }
            // Stop RPC: wait a beat so the client task can flush its response
            _ = daemon_state.shutdown().notified() => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                break;
            }
            accepted = listener.accept() => match accepted {
                Ok((stream, _addr)) => {
                    println!("New client connected!");
                    tokio::spawn(serve(stream, Arc::clone(daemon_state)));
                }
                Err(error) => println!("Connection failed, Error: {error}"),
            }
        }
    }

    println!("Shutting down.");
    daemon_state.speech().stop();
    let _ = fs::remove_file(&socket_path);
    Ok(())
}

async fn write_line(
    writer: &mut (impl AsyncWriteExt + Unpin),
    message: &impl serde::Serialize,
) -> io::Result<()> {
    let mut line = serde_json::to_string(message).map_err(io::Error::other)?;
    line.push('\n');
    writer.write_all(line.as_bytes()).await
}

struct Events {
    state: bool,
    downloads: bool,
}

// What a `subscribe` call asked to be sent. Absent means state alone, so a
// client that names no events is unaffected by there being more than one
fn requested_events(params: Option<&serde_json::Value>) -> Events {
    let Some(named) = params
        .and_then(|params| params.get("events"))
        .and_then(|events| events.as_array())
    else {
        return Events {
            state: true,
            downloads: false,
        };
    };
    let asked = |name: &str| named.iter().any(|event| event.as_str() == Some(name));
    Events {
        state: asked(banshee_common::EVENT_STATE),
        downloads: asked(banshee_common::EVENT_DOWNLOADS),
    }
}

async fn push_downloads(
    writer: Arc<Mutex<OwnedWriteHalf>>,
    mut downloads: broadcast::Receiver<DownloadProgress>,
) {
    loop {
        let progress = match downloads.recv().await {
            Ok(progress) => progress,
            // Too far behind to catch up on the ones it missed, but the ones
            // still coming are worth having
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        };
        let Ok(params) = serde_json::to_value(progress) else {
            continue;
        };
        let notification = JsonRpcNotification::new(BANSHEE_DOWNLOAD_PROGRESS, params);
        if write_line(&mut *writer.lock().await, &notification)
            .await
            .is_err()
        {
            break;
        }
    }
}

/// Sends one connection its state changes, until the daemon stops or the client
/// does. `told` is the state that client already has, which a push is judged
/// against: an unchanged one is not worth a line.
async fn push_changes(
    state: Arc<DaemonState>,
    writer: Arc<Mutex<OwnedWriteHalf>>,
    mut recording: watch::Receiver<bool>,
    mut speaking: watch::Receiver<bool>,
    mut transcribing: watch::Receiver<bool>,
    mut devices: watch::Receiver<u64>,
    mut told: serde_json::Value,
) {
    loop {
        // Every arm only wakes the task; the state is read fresh below
        let woken = tokio::select! {
            woken = recording.changed() => woken,
            woken = speaking.changed() => woken,
            woken = transcribing.changed() => woken,
            woken = devices.changed() => woken,
        };
        if woken.is_err() {
            break;
        }
        let now = live_state(&state);
        if now == told {
            continue;
        }
        let notification = JsonRpcNotification::new(BANSHEE_STATE_CHANGED, now);
        if write_line(&mut *writer.lock().await, &notification)
            .await
            .is_err()
        {
            break;
        }
        told = notification.params;
    }
}

/// The subscription lives and dies with the connection.
async fn serve(stream: UnixStream, state: Arc<DaemonState>) {
    let (reader, writer) = stream.into_split();
    // Pushing is a task of its own, because ask_user parks this one inside
    // dispatch for minutes while it holds the microphone open
    let writer = Arc::new(Mutex::new(writer));
    let mut lines = BufReader::new(reader).lines();
    // The handles are the bookkeeping: a later subscribe opens only the kind
    // that has none
    let mut pushing_state: Option<tokio::task::JoinHandle<()>> = None;
    let mut pushing_downloads: Option<tokio::task::JoinHandle<()>> = None;

    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(request) = serde_json::from_str::<JsonRpcRequest>(&line) else {
            continue;
        };
        // Taken before the reply is built: a change landing in between then
        // costs a duplicate push, where the other order would lose it
        let asked = if request.method == BANSHEE_SUBSCRIBE {
            requested_events(request.params.as_ref())
        } else {
            Events {
                state: false,
                downloads: false,
            }
        };
        let opening_state = (asked.state && pushing_state.is_none()).then(|| {
            (
                state.subscribe_recording(),
                state.speech().subscribe_speaking(),
                state.subscribe_transcribing(),
                state.device_changes(),
                live_state(&state),
            )
        });
        let opening_downloads =
            (asked.downloads && pushing_downloads.is_none()).then(|| state.subscribe_downloads());

        let response = dispatch(request, &state).await;
        if write_line(&mut *writer.lock().await, &response)
            .await
            .is_err()
        {
            break;
        }

        if let Some((recording, speaking, transcribing, devices, told)) = opening_state {
            pushing_state = Some(tokio::spawn(push_changes(
                Arc::clone(&state),
                Arc::clone(&writer),
                recording,
                speaking,
                transcribing,
                devices,
                told,
            )));
        }
        if let Some(downloads) = opening_downloads {
            pushing_downloads = Some(tokio::spawn(push_downloads(Arc::clone(&writer), downloads)));
        }
    }

    for task in [pushing_state, pushing_downloads].into_iter().flatten() {
        task.abort();
    }
}

// Probe with std's blocking connect: tokio's nonblocking UDS connect on
// macOS reports success against a dead socket
pub fn socket_answers(socket_path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket_path).is_ok()
}

// The socket file doubles as the single-instance lock
fn claim_socket(socket_path: &Path) -> io::Result<UnixListener> {
    if socket_path.exists() {
        if socket_answers(socket_path) {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "another banshee daemon is already running",
            ));
        }
        // nobody answered: stale socket left by an unclean exit
        println!("Removing stale socket at {}", socket_path.display());
        fs::remove_file(socket_path)?;
    }
    UnixListener::bind(socket_path)
}

#[cfg(test)]
mod tests;

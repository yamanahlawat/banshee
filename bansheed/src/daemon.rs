use banshee_common::utils::socket_path;
use banshee_common::{
    BANSHEE_DOWNLOAD_PROGRESS, BANSHEE_STATE_CHANGED, BANSHEE_SUBSCRIBE, DownloadProgress,
    JsonRpcNotification, JsonRpcRequest, SileroVADConfig, error::BansheeError,
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
use crate::speech_to_text::vad::VADEngine;
use crate::state::{ConsumerCommand, DaemonState, RecordingError};
use crate::{audio, history, hotkey, models, permissions, text_to_speech};

// Claimed before model loading, so a lost single-instance race stays cheap
pub fn claim() -> Result<(std::path::PathBuf, UnixListener, fs::File), BansheeError> {
    let socket_path = socket_path().ok_or_else(|| {
        BansheeError::Other("could not find home directory for the socket path".to_string())
    })?;

    if let Some(parent_dir) = socket_path.parent() {
        fs::create_dir_all(parent_dir).map_err(|e| BansheeError::file(parent_dir, e))?;
    }

    let (listener, lock) = claim_socket(&socket_path)?;
    // owner-only: the socket is a command channel into the mic and speakers
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
        .map_err(|e| BansheeError::file(&socket_path, e))?;
    Ok((socket_path, listener, lock))
}

/// `open_capture` writes the device name once `play()` succeeds, so a name left
/// behind after a failure shows a microphone nothing holds.
pub(crate) fn drop_device_name(daemon_state: &DaemonState) {
    daemon_state.set_audio_device(None);
}

/// What startup built and resolved. `open` and `missing` seed the watchdog, so
/// its binding never reads them back out of `DaemonState`.
struct Recording {
    stream: cpal::Stream,
    thread: std::thread::JoinHandle<()>,
    capture: Arc<hotkey::Capture>,
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
        Some(name) => log::info!(
            "Capture opened {}, still waiting for {name}",
            selection.open
        ),
        None => log::info!("Capture opened {}", selection.open),
    }
    let speech_to_text =
        crate::speech_to_text::select_transcriber(&config.stt).inspect_err(|_| {
            drop_device_name(daemon_state);
        })?;
    let vad = VADEngine::new(SileroVADConfig::new(models::VAD_MODEL)).map_err(|e| {
        drop_device_name(daemon_state);
        RecordingError::Model(e.to_string())
    })?;
    // One capture, held by the consumer thread and by the watchdog that
    // replaces it. A question that is already listening reads it too.
    let shared_capture = Arc::new(hotkey::Capture::new(hotkey::CaptureSource {
        consumer: capture.consumer,
        sample_rate: capture.sample_rate,
    }));
    let thread = hotkey::hotkey_listener(
        hotkey::Pipeline {
            source: Arc::clone(&shared_capture),
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
        capture: shared_capture,
        open: selection.open,
        missing: selection.missing,
    })
}

pub async fn start(config: Config) -> Result<(), BansheeError> {
    let config = Arc::new(config);
    let (socket_path, listener, _lock) = claim()?;
    permissions::ask_for_accessibility();
    let db_connection = if config.daemon.save_history {
        Some(history::open()?)
    } else {
        None
    };

    // Created before the backend, because the backend holds the sender and the
    // drain holds the state the backend must not see
    let (faults, fault_reports) = std::sync::mpsc::channel();
    // One output for every sound the daemon makes. The voice and the cues on
    // one device is the point: two holders drift apart the moment one dies.
    let output = Arc::new(text_to_speech::output::Output::lazy());
    let speech = text_to_speech::select_backend(&config.tts, faults, Arc::clone(&output))?;
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let cues = audio::cues::start_cue_player(config.audio.cues.enabled, output);
    let daemon_state = Arc::new(DaemonState::new(
        Arc::clone(&config),
        db_connection,
        text_to_speech::SpeechPlayer::new(speech.backend),
        speech.speaker,
        commands,
        cues.clone(),
        models::download::models_dir()?,
    ));

    if let Some(reason) = speech.fault {
        daemon_state.set_last_speech_error(Some(reason));
    }

    permissions::restart_when_granted(
        {
            let state = Arc::clone(&daemon_state);
            move || state.is_downloading()
        },
        {
            let state = Arc::clone(&daemon_state);
            move || {
                state.request_restart();
                state.shutdown().notify_one();
            }
        },
    );

    // After the state, which the drain writes
    let draining_state = Arc::clone(&daemon_state);
    let draining_cues = cues.clone();
    std::thread::spawn(move || {
        text_to_speech::drain_faults(draining_state, draining_cues, fault_reports)
    });

    // Off this thread, because the socket is already bound and `run` below is
    // what answers it. The device walk enters Core Audio, which stalls for
    // minutes on a virtual device whose plugin stops answering, and a stall
    // here leaves every client connected to a daemon that says nothing.
    let building = {
        let state = Arc::clone(&daemon_state);
        let config = Arc::clone(&config);
        std::thread::spawn(move || {
            // The watchdog owns the stream past daemon::run: stopping it stops
            // capture, and the thread is the only thing left to join
            match start_recording(&state, &config, command_receiver, cues) {
                Ok(started) => {
                    let watchdog = audio::watchdog::spawn(
                        Arc::clone(&state),
                        started.stream,
                        started.capture,
                        started.open,
                        started.missing,
                    );
                    state.set_pipeline(crate::state::Pipeline::Open);
                    Some((watchdog, started.thread))
                }
                // A missing mic or model leaves the daemon useful rather than
                // exiting, which the supervisor reads as a crash and retries
                Err(error) => {
                    log::error!("Recording is unavailable: {error}");
                    log::info!(
                        "The daemon is up: speak, status, and history still work. \
                             Recording, dictation, and ask_user do not."
                    );
                    log::info!("Run `banshee status` for the fix.");
                    state.set_pipeline(crate::state::Pipeline::Broken(error));
                    None
                }
            }
        })
    };
    // A press that lands before the pipeline stands answers with the error cue:
    // record_start refuses anything but an open pipeline.
    hotkey::start_global_hotkey(
        Arc::clone(&daemon_state),
        config.audio.hotkey,
        config.audio.hotkey_mode,
    );
    let result = run(&daemon_state, socket_path, listener).await;
    // A build still parked in Core Audio cannot be woken, so the process leaves
    // without it rather than never leaving at all.
    if building.is_finished()
        && let Ok(Some((watchdog, consumer_thread))) = building.join()
    {
        // Capture stops first, so no Rebind arrives at a thread that
        // has already left its loop
        watchdog.stop();
        // Drop the Whisper context before atexit: ggml's Metal cleanup
        // asserts if buffers are still resident
        let _ = daemon_state.commands().send(ConsumerCommand::Shutdown);
        let _ = consumer_thread.join();
    }
    result?;
    if daemon_state.restart_wanted() {
        return Err(BansheeError::Other(
            "the Accessibility grant landed; starting again to pick it up".to_string(),
        ));
    }
    Ok(())
}

pub async fn run(
    daemon_state: &Arc<DaemonState>,
    socket_path: std::path::PathBuf,
    listener: UnixListener,
) -> Result<(), std::io::Error> {
    log::info!("Listening on {}", socket_path.display());

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
                    log::debug!("New client connected!");
                    tokio::spawn(serve(stream, Arc::clone(daemon_state)));
                }
                Err(error) => log::warn!("Connection failed, Error: {error}"),
            }
        }
    }

    log::info!("Shutting down.");
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

/// Everything a state subscription waits on.
struct StateWatches {
    recording: watch::Receiver<bool>,
    speaking: watch::Receiver<bool>,
    transcribing: watch::Receiver<bool>,
    loading_model: watch::Receiver<bool>,
    telling: watch::Receiver<bool>,
    devices: watch::Receiver<u64>,
    pipeline: watch::Receiver<crate::state::Pipeline>,
    last_error: watch::Receiver<Option<crate::state::Failed>>,
    last_speech_error: watch::Receiver<Option<String>>,
}

/// Sends one connection its state changes, until the daemon stops or the client
/// does. `told` is the state that client already has, which a push is judged
/// against: an unchanged one is not worth a line.
async fn push_changes(
    state: Arc<DaemonState>,
    writer: Arc<Mutex<OwnedWriteHalf>>,
    mut watches: StateWatches,
    mut told: serde_json::Value,
) {
    loop {
        // Every arm only wakes the task; the state is read fresh below
        let woken = tokio::select! {
            woken = watches.recording.changed() => woken,
            woken = watches.speaking.changed() => woken,
            woken = watches.transcribing.changed() => woken,
            woken = watches.loading_model.changed() => woken,
            woken = watches.telling.changed() => woken,
            woken = watches.devices.changed() => woken,
            woken = watches.pipeline.changed() => woken,
            woken = watches.last_error.changed() => woken,
            woken = watches.last_speech_error.changed() => woken,
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
    // A request that arrived while an earlier call was still running. The
    // connection is read throughout a call, so a pipelined one cannot be lost.
    let mut queued: std::collections::VecDeque<String> = std::collections::VecDeque::new();

    loop {
        let line = match queued.pop_front() {
            Some(line) => line,
            None => match lines.next_line().await {
                Ok(Some(line)) => line,
                _ => break,
            },
        };
        let request = match serde_json::from_str::<JsonRpcRequest>(&line) {
            Ok(request) => request,
            Err(error) => {
                let refusal = banshee_common::JsonRpcResponse::parse_error(&error);
                if write_line(&mut *writer.lock().await, &refusal)
                    .await
                    .is_err()
                {
                    break;
                }
                continue;
            }
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
                StateWatches {
                    recording: state.subscribe_recording(),
                    speaking: state.speech().subscribe_speaking(),
                    transcribing: state.subscribe_transcribing(),
                    loading_model: state.subscribe_loading_model(),
                    telling: state.subscribe_telling(),
                    devices: state.device_changes(),
                    pipeline: state.subscribe_pipeline(),
                    last_error: state.subscribe_last_error(),
                    last_speech_error: state.subscribe_last_speech_error(),
                },
                live_state(&state),
            )
        });
        let opening_downloads =
            (asked.downloads && pushing_downloads.is_none()).then(|| state.subscribe_downloads());

        // Watched while it runs: `ask_user` parks here for minutes holding the
        // microphone, and a client that goes away in the meantime is asking
        // for none of it. Dropping the call is what ends the work.
        let mut call = std::pin::pin!(dispatch(request, &state));
        let answered = loop {
            tokio::select! {
                response = &mut call => break Some(response),
                next = lines.next_line() => match next {
                    Ok(Some(line)) => queued.push_back(line),
                    _ => break None,
                },
            }
        };
        let Some(response) = answered else { break };
        if write_line(&mut *writer.lock().await, &response)
            .await
            .is_err()
        {
            break;
        }

        if let Some((watches, told)) = opening_state {
            pushing_state = Some(tokio::spawn(push_changes(
                Arc::clone(&state),
                Arc::clone(&writer),
                watches,
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

fn claim_socket(socket_path: &Path) -> io::Result<(UnixListener, fs::File)> {
    let lock = fs::File::create(socket_path.with_extension("lock"))?;
    if lock.try_lock().is_err() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "another banshee daemon is already running",
        ));
    }
    if socket_path.exists() {
        if socket_answers(socket_path) {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "another banshee daemon is already running",
            ));
        }
        // nobody answered: stale socket left by an unclean exit
        log::info!("Removing stale socket at {}", socket_path.display());
        fs::remove_file(socket_path)?;
    }
    Ok((UnixListener::bind(socket_path)?, lock))
}

#[cfg(test)]
mod tests;

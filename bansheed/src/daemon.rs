use banshee_common::utils::{banshee_dir, make_owner_only, socket_path};
use banshee_common::{
    BANSHEE_CUE, BANSHEE_DOWNLOAD_PROGRESS, BANSHEE_LEVEL, BANSHEE_STATE_CHANGED,
    BANSHEE_SUBSCRIBE, BANSHEE_WORDS, JsonRpcNotification, JsonRpcRequest, error::BansheeError,
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
use crate::speech_to_text::vad::Detector;
use crate::state::{ConsumerCommand, DaemonState, RecordingError};
use crate::{audio, history, hotkey, models, permissions, text_to_speech};

// Claimed before model loading, so a lost single-instance race stays cheap
pub fn claim() -> Result<(std::path::PathBuf, UnixListener, fs::File), BansheeError> {
    let socket_path = socket_path().ok_or_else(|| {
        BansheeError::Other("could not find home directory for the socket path".to_string())
    })?;

    if let Some(dir) = banshee_dir() {
        // `banshee start`, the tray and a config write can create the directory
        // first, at the default mode.
        make_owner_only(&dir).map_err(|e| BansheeError::file(&dir, e))?;
    }
    if let Some(parent_dir) = socket_path.parent() {
        fs::create_dir_all(parent_dir).map_err(|e| BansheeError::file(parent_dir, e))?;
    }

    let (listener, lock) = claim_socket(&socket_path)?;
    // owner-only: the socket is a command channel into the mic and speakers
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
        .map_err(|e| BansheeError::file(&socket_path, e))?;
    Ok((socket_path, listener, lock))
}

/// What startup built: the thread that turns audio into text, and the capture
/// it reads, which the thread that owns capture fills.
struct Recording {
    thread: std::thread::JoinHandle<()>,
    capture: Arc<hotkey::Capture>,
}

/// The models and the thread that turns audio into text. All of it or none:
/// with any piece missing the daemon cannot transcribe, so they share one
/// error path and one reason for `banshee status` to report. A listening model
/// still to download, and a remote listener with no key, are the exceptions:
/// the rest stands, and the listener serves once the file or the key arrives.
/// The microphone is not part of it: it opens for each use.
fn start_recording(
    daemon_state: &Arc<DaemonState>,
    config: &Config,
    command_receiver: std::sync::mpsc::Receiver<ConsumerCommand>,
    cues: audio::cues::Cues,
) -> Result<Recording, RecordingError> {
    let listener =
        crate::speech_to_text::select_transcriber(&config.stt, daemon_state.credentials_path())?;
    let vad = models::model_path(models::VAD_MODEL)
        .and_then(Detector::open)
        .map_err(|e| RecordingError::Model(e.to_string()))?;
    // Before the consumer exists, so a load a write queued meanwhile lands
    // after this and is not overwritten by it.
    let listening = match (listener.awaiting, listener.refused) {
        (Some(model), _) => {
            log::warn!(
                "Recording waits for the listening model {}: run banshee setup",
                model.id
            );
            crate::state::Listening::Missing(model)
        }
        (None, Some(refused)) => {
            log::warn!("The remote listener cannot send yet: {refused}");
            crate::state::Listening::Refused(refused)
        }
        (None, None) => models::stt_model(config).map_or(
            crate::state::Listening::Remote,
            crate::state::Listening::Loaded,
        ),
    };
    daemon_state.install_listener(crate::speech_to_text::Route::of(&config.stt), listening);
    let credentials = daemon_state.credentials_path().to_path_buf();
    // One capture, read by the consumer thread and filled by the thread that
    // owns the stream. A question that is already listening reads it too.
    let capture = Arc::new(hotkey::Capture::new(hotkey::CaptureSource::unopened()));
    let thread = hotkey::hotkey_listener(
        hotkey::Pipeline {
            source: Arc::clone(&capture),
            speech_to_text: listener.engine,
            vad,
            // An empty path never loads, so no models directory means no turn model
            judge: Box::new(crate::speech_to_text::SmartTurn::new(
                models::download::models_dir()
                    .map(|dir| dir.join(models::TURN_MODEL))
                    .unwrap_or_default(),
            )),
            state: Arc::clone(daemon_state),
            cues,
            used: std::time::Instant::now(),
            build_listener: Box::new(move |stt| {
                crate::speech_to_text::select_on_demand(stt, &credentials)
            }),
            withdrawals_seen: daemon_state.listener_withdrawals(),
        },
        command_receiver,
    );
    Ok(Recording { thread, capture })
}

/// Returns the status the process exits with.
pub async fn start(config: Config) -> Result<u8, BansheeError> {
    let config = Arc::new(config);
    let (socket_path, listener, _lock) = claim()?;
    transcribe_cpp::init_logging();
    for deleted in models::delete_old_whisper_files(&models::download::models_dir()?) {
        log::info!("Deleted {}, which nothing loads now", deleted.display());
    }
    permissions::ask_for_accessibility();
    let history = history::open(config.daemon.keep_audio)?;

    // Created before the backend, because the backend holds the sender and the
    // drain holds the state the backend must not see
    let (faults, fault_reports) = std::sync::mpsc::channel();
    // One output for every sound the daemon makes. The voice and the cues on
    // one device is the point: two holders drift apart the moment one dies.
    let output = Arc::new(text_to_speech::output::Output::lazy());
    std::thread::spawn(text_to_speech::pronunciation::warm);
    let credentials = crate::credentials::Credentials::path()?;
    let models_dir = models::download::models_dir()?;
    let licences = banshee_common::utils::accepted_licences_path().ok_or_else(|| {
        BansheeError::Other("Could not find the accepted licences record".to_string())
    })?;
    let speech = text_to_speech::select_backend(
        &config.tts,
        &models_dir,
        &credentials,
        faults.clone(),
        Arc::clone(&output),
    );
    let build_speaker: text_to_speech::BuildSpeaker = {
        let (credentials, output, models_dir) =
            (credentials.clone(), Arc::clone(&output), models_dir.clone());
        Box::new(move |tts| {
            text_to_speech::select_live(
                tts,
                &models_dir,
                &credentials,
                faults.clone(),
                Arc::clone(&output),
            )
        })
    };
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let cues = audio::cues::start_cue_player(config.feedback_mode(), output);
    let speech_player = text_to_speech::SpeechPlayer::new(speech.backend);
    speech_player.set_builder(build_speaker);
    let daemon_state = Arc::new(DaemonState::new(
        Arc::clone(&config),
        history,
        speech_player,
        speech.speaker,
        commands,
        cues.clone(),
        crate::state::Machine {
            models: models_dir,
            config: Config::path()?,
            credentials,
            licences,
            voiceover: crate::settings::voiceover_on,
            frontmost: crate::frontmost::frontmost,
            #[cfg(target_os = "macos")]
            accessibility: permissions::accessibility_granted,
            #[cfg(target_os = "macos")]
            microphone: permissions::microphone_access,
            #[cfg(target_os = "macos")]
            key_presses: permissions::key_presses_reach_us,
            #[cfg(all(unix, not(target_os = "macos")))]
            typer_blockers: permissions::live_typer_blockers,
        },
    ));
    tokio::spawn(crate::level::sample(Arc::clone(&daemon_state)));
    let voice_keeper = text_to_speech::unload_when_idle(Arc::clone(&daemon_state));
    daemon_state.set_voice_keeper(voice_keeper.thread().clone());

    if let Some(reason) = speech.fault {
        daemon_state.set_last_speech_error(Some(reason));
    }

    permissions::follow_accessibility(Arc::clone(&daemon_state));
    // Before the speak task, so a refusal already there is part of its
    // baseline and is not spoken.
    #[cfg(target_os = "macos")]
    permissions::read_microphone(&daemon_state);
    tokio::spawn(speak_new_problems(
        Arc::clone(&daemon_state),
        StateWatches::of(&daemon_state),
    ));

    // After the state, which the drain writes
    let draining_state = Arc::clone(&daemon_state);
    let draining_cues = cues.clone();
    std::thread::spawn(move || {
        text_to_speech::drain_faults(draining_state, draining_cues, fault_reports)
    });

    // Off this thread, because the socket is already bound and `run` below is
    // what answers it, and loading the models takes seconds.
    let building = {
        let state = Arc::clone(&daemon_state);
        let config = Arc::clone(&config);
        std::thread::spawn(move || {
            match start_recording(&state, &config, command_receiver, cues) {
                Ok(started) => {
                    // Before the owner starts, which may open at once and
                    // must not have its open overwritten
                    state.set_mic(crate::state::Mic::Closed);
                    let watchdog =
                        audio::watchdog::spawn(Arc::clone(&state), started.capture, audio::Cpal);
                    // A download that ended while the listener was built
                    // found no listener waiting for it.
                    crate::settings::reconcile_listening(&state);
                    Some((watchdog, started.thread))
                }
                // A missing model or listener leaves the daemon useful rather
                // than exiting, which the supervisor reads as a crash and retries
                Err(error) => {
                    log::error!("Recording is unavailable: {error}");
                    log::info!(
                        "The daemon is up: speak, status, and history still work. \
                             Recording, dictation, and ask_user do not."
                    );
                    log::info!("Run `banshee status` for the fix.");
                    state.set_mic(crate::state::Mic::Faulted(error.at_start()));
                    None
                }
            }
        })
    };
    // A press that lands before the pipeline stands answers with the error cue:
    // record_start refuses anything but an open pipeline.
    hotkey::start_global_hotkey(Arc::clone(&daemon_state));
    let result = run(&daemon_state, socket_path, listener).await;
    // A build still loading a model cannot be woken, so the process leaves
    // without it rather than never leaving at all. Core Audio, which stalls
    // for minutes on a virtual device whose plugin stops answering, can park
    // only the thread that owns capture, and `watchdog.stop` below joins it.
    if building.is_finished()
        && let Ok(Some((watchdog, consumer_thread))) = building.join()
    {
        // Capture stops first, so no Rebind arrives at a thread that
        // has already left its loop
        watchdog.stop();
        // Drop the listening model before atexit: ggml's Metal cleanup
        // asserts if buffers are still resident
        let _ = daemon_state.commands().send(ConsumerCommand::Shutdown);
        let _ = consumer_thread.join();
    }
    result?;
    Ok(daemon_state.exit_code())
}

pub async fn run(
    daemon_state: &Arc<DaemonState>,
    socket_path: std::path::PathBuf,
    listener: UnixListener,
) -> Result<(), std::io::Error> {
    log::info!("Listening on {}", socket_path.display());

    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    // Coarse tick: the ceiling it enforces is measured in minutes. The first
    // tick fires at once, so the purge also runs at start.
    let mut watchdog = tokio::time::interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            _ = sigint.recv() => break,
            _ = sigterm.recv() => break,
            _ = watchdog.tick() => {
                daemon_state.expire_stuck_recording();
                let state = Arc::clone(daemon_state);
                tokio::task::spawn_blocking(move || state.purge_audio());
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

#[derive(Default)]
struct Events {
    state: bool,
    downloads: bool,
    cues: bool,
    level: bool,
    words: bool,
    draws: bool,
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
            ..Default::default()
        };
    };
    let asked = |name: &str| named.iter().any(|event| event.as_str() == Some(name));
    Events {
        state: asked(banshee_common::EVENT_STATE),
        downloads: asked(banshee_common::EVENT_DOWNLOADS),
        cues: asked(banshee_common::EVENT_CUES),
        level: asked(banshee_common::EVENT_LEVEL),
        words: asked(banshee_common::EVENT_WORDS),
        draws: params
            .and_then(|params| params.get("draws"))
            .and_then(serde_json::Value::as_bool)
            == Some(true),
    }
}

// A receiver that lags skips what it missed, because what still comes is worth having.
async fn push_broadcast<T: serde::Serialize + Clone>(
    writer: Arc<Mutex<OwnedWriteHalf>>,
    mut rx: broadcast::Receiver<T>,
    method: &'static str,
) {
    loop {
        let value = match rx.recv().await {
            Ok(value) => value,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        };
        let Ok(params) = serde_json::to_value(value) else {
            continue;
        };
        let notification = JsonRpcNotification::new(method, params);
        if write_line(&mut *writer.lock().await, &notification)
            .await
            .is_err()
        {
            break;
        }
    }
}

async fn push_level(writer: Arc<Mutex<OwnedWriteHalf>>, mut level: watch::Receiver<f32>) {
    while level.changed().await.is_ok() {
        let now = *level.borrow_and_update();
        // `f64::from` gives the f32 a long binary tail, which the rounding cuts.
        let now = (f64::from(now) * 1000.0).round() / 1000.0;
        let notification =
            JsonRpcNotification::new(BANSHEE_LEVEL, serde_json::json!({"level": now}));
        if write_line(&mut *writer.lock().await, &notification)
            .await
            .is_err()
        {
            break;
        }
    }
}

async fn push_words(writer: Arc<Mutex<OwnedWriteHalf>>, mut words: watch::Receiver<String>) {
    while words.changed().await.is_ok() {
        let text = words.borrow_and_update().clone();
        let notification =
            JsonRpcNotification::new(BANSHEE_WORDS, serde_json::json!({"text": text}));
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
    playing: watch::Receiver<Option<u64>>,
    transcribing: watch::Receiver<bool>,
    loading_model: watch::Receiver<bool>,
    devices: watch::Receiver<u64>,
    pipeline: watch::Receiver<crate::state::Pipeline>,
    last_error: watch::Receiver<Option<String>>,
    last_speech_error: watch::Receiver<Option<String>>,
    feedback: watch::Receiver<crate::config::FeedbackMode>,
    history: watch::Receiver<u64>,
    settings: watch::Receiver<u64>,
}

impl StateWatches {
    fn of(state: &DaemonState) -> Self {
        Self {
            recording: state.subscribe_recording(),
            playing: state.speech().subscribe_playing(),
            transcribing: state.subscribe_transcribing(),
            loading_model: state.subscribe_loading_model(),
            devices: state.device_changes(),
            pipeline: state.subscribe_pipeline(),
            last_error: state.subscribe_last_error(),
            last_speech_error: state.subscribe_last_speech_error(),
            feedback: state.subscribe_feedback(),
            history: state.subscribe_history_changes(),
            settings: state.subscribe_settings_changes(),
        }
    }

    /// Only wakes the caller, which reads the state fresh. An error is the
    /// daemon going away.
    async fn changed(&mut self) -> Result<(), watch::error::RecvError> {
        tokio::select! {
            woken = self.recording.changed() => woken,
            woken = self.playing.changed() => woken,
            woken = self.transcribing.changed() => woken,
            woken = self.loading_model.changed() => woken,
            woken = self.devices.changed() => woken,
            woken = self.pipeline.changed() => woken,
            woken = self.last_error.changed() => woken,
            woken = self.last_speech_error.changed() => woken,
            woken = self.feedback.changed() => woken,
            woken = self.history.changed() => woken,
            woken = self.settings.changed() => woken,
        }
    }
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
    while watches.changed().await.is_ok() {
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

/// Says the first problem that was absent a moment before. What stands once
/// the daemon has started is the baseline, so a start or a login says nothing.
async fn speak_new_problems(state: Arc<DaemonState>, mut watches: StateWatches) {
    if watches
        .pipeline
        .wait_for(|pipeline| !matches!(pipeline, crate::state::Pipeline::Opening))
        .await
        .is_err()
    {
        return;
    }
    let mut before = ids_of(&problems(&state));
    while watches.changed().await.is_ok() {
        let blockers = problems(&state);
        if state.speaks_problems()
            && let Some(blocker) = blockers.iter().find(|b| !before.contains(&b.id))
            && let Err(error) = state.speech().speak(&spoken_problem(blocker), false, None)
        {
            log::warn!("Failed to say the problem {}: {error}", blocker.id);
        }
        before = ids_of(&blockers);
    }
}

/// Kokoro drops `>`, so a menu path is read as steps.
fn spoken_problem(blocker: &banshee_common::Blocker) -> String {
    let fix = blocker.fix.replace(" > ", ", then ");
    format!(
        "{}. {}.",
        blocker.name,
        crate::audio::cues::capitalised(&fix)
    )
}

fn problems(state: &DaemonState) -> Vec<banshee_common::Blocker> {
    crate::readiness::blockers_as_read(state, &state.pipeline())
}

fn ids_of(blockers: &[banshee_common::Blocker]) -> Vec<String> {
    blockers.iter().map(|blocker| blocker.id.clone()).collect()
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
    let mut pushing_cues: Option<tokio::task::JoinHandle<()>> = None;
    let mut pushing_level: Option<tokio::task::JoinHandle<()>> = None;
    let mut pushing_words: Option<tokio::task::JoinHandle<()>> = None;
    let mut drawn: Option<crate::audio::cues::Drawn> = None;
    let mut watching_level: Option<crate::state::LevelWatch> = None;
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
            Events::default()
        };
        let opening_state = (asked.state && pushing_state.is_none())
            .then(|| (StateWatches::of(&state), live_state(&state)));
        let opening_downloads =
            (asked.downloads && pushing_downloads.is_none()).then(|| state.subscribe_downloads());
        let opening_cues =
            (asked.cues && pushing_cues.is_none()).then(|| state.cues().subscribe_signals());
        let opening_level =
            (asked.level && pushing_level.is_none()).then(|| state.subscribe_level());
        let opening_words =
            (asked.words && pushing_words.is_none()).then(|| state.subscribe_words());
        if asked.cues && asked.draws && drawn.is_none() {
            drawn = Some(state.cues().drawn());
        }
        if asked.level && watching_level.is_none() {
            watching_level = Some(state.watch_level());
        }

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
            pushing_downloads = Some(tokio::spawn(push_broadcast(
                Arc::clone(&writer),
                downloads,
                BANSHEE_DOWNLOAD_PROGRESS,
            )));
        }
        if let Some(signals) = opening_cues {
            pushing_cues = Some(tokio::spawn(push_broadcast(
                Arc::clone(&writer),
                signals,
                BANSHEE_CUE,
            )));
        }
        if let Some(level) = opening_level {
            pushing_level = Some(tokio::spawn(push_level(Arc::clone(&writer), level)));
        }
        if let Some(words) = opening_words {
            pushing_words = Some(tokio::spawn(push_words(Arc::clone(&writer), words)));
        }
    }

    for task in [
        pushing_state,
        pushing_downloads,
        pushing_cues,
        pushing_level,
        pushing_words,
    ]
    .into_iter()
    .flatten()
    {
        task.abort();
    }
}

pub fn socket_answers(socket_path: &Path) -> bool {
    banshee_common::utils::listens(socket_path).unwrap_or(false)
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

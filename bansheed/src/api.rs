use std::sync::Arc;
use std::time::Duration;

use banshee_common::error::BansheeError;
use banshee_common::{
    BANSHEE_AGENTS, BANSHEE_ASK_USER, BANSHEE_CLEAR_HISTORY, BANSHEE_CONFIGURE,
    BANSHEE_CONNECT_APPLY, BANSHEE_CONNECT_PLAN, BANSHEE_DOWNLOAD_MODELS,
    BANSHEE_GET_TRANSCRIPTION, BANSHEE_HISTORY, BANSHEE_LIST_INPUT_DEVICES, BANSHEE_LIST_LANGUAGES,
    BANSHEE_LIST_VOICES, BANSHEE_OPEN_PERMISSION, BANSHEE_RECORD_START, BANSHEE_RECORD_STOP,
    BANSHEE_RECORD_TOGGLE, BANSHEE_SPEAK, BANSHEE_STATUS, BANSHEE_STOP, BANSHEE_STOP_SPEAKING,
    BANSHEE_SUBSCRIBE,
};
use banshee_common::{JsonRpcRequest, JsonRpcResponse, rpc_code};

use crate::connect;
use crate::permissions;
use crate::state::{AskCommand, ConsumerCommand, DaemonState, RecordingError, TranscribeTarget};
use crate::text_to_speech::sanitizer::sanitize;
use crate::{readiness, settings};

const MAX_WAIT_MS: u64 = 30_000;
const DEFAULT_ASK_WAIT_MS: u64 = 30_000;
const MAX_ASK_WAIT_MS: u64 = 120_000;
// Budget scales with question length; the per-word figure is a speech-rate
// estimate, not real audio duration.
const PLAYBACK_BASE_MS: u64 = 15_000;
const PLAYBACK_PER_WORD_MS: u64 = 700;
const MAX_PLAYBACK_WAIT_MS: u64 = 120_000;

fn from_error(id: Option<serde_json::Value>, error: BansheeError) -> JsonRpcResponse {
    JsonRpcResponse::error(id, error.rpc_code(), error.rpc_message())
}

/// `Env::from_machine` polls a login shell for up to five seconds, and an
/// apply waits on an agent's own CLI with no timeout. On a worker thread that
/// wait holds every other socket behind it.
async fn off_the_worker(
    id: Option<serde_json::Value>,
    work: impl FnOnce() -> Result<serde_json::Value, BansheeError> + Send + 'static,
) -> JsonRpcResponse {
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok(result)) => JsonRpcResponse::success(id, result),
        Ok(Err(error)) => from_error(id, error),
        Err(_) => JsonRpcResponse::error(
            id,
            rpc_code::INTERNAL,
            "The connect task stopped before it answered.",
        ),
    }
}

fn the_plan(agent: connect::Agent) -> Result<(connect::Env, Vec<connect::Change>), BansheeError> {
    let env = connect::Env::from_machine()?;
    let changes = connect::plan(agent, &env)?;
    Ok((env, changes))
}

fn read_the_agents() -> Result<serde_json::Value, BansheeError> {
    // The window reads this on an Agents panel open, and on a reconnect until
    // one read succeeds. So an agent installed now appears without a restart.
    let env = connect::Env::from_machine_refreshed()?;
    let agents: Vec<_> = connect::Agent::ALL
        .iter()
        .map(|agent| connect::row(*agent, &env))
        .collect();
    Ok(serde_json::json!({"agents": agents}))
}

fn read_the_plan(agent: connect::Agent) -> Result<serde_json::Value, BansheeError> {
    let (_, changes) = the_plan(agent)?;
    Ok(serde_json::json!({
        "changes": changes.iter().map(connect::planned_change).collect::<Vec<_>>(),
    }))
}

fn write_the_plan(agent: connect::Agent) -> Result<serde_json::Value, BansheeError> {
    let (env, changes) = the_plan(agent)?;
    connect::apply_all(&changes, &env.path, |_| {})?;
    Ok(serde_json::json!({"applied": changes.len()}))
}

/// Carries the request id, so a reader can answer -32602 without every handler
/// threading the id into it.
struct Params<'a> {
    values: Option<&'a serde_json::Value>,
    id: Option<serde_json::Value>,
}

impl<'a> Params<'a> {
    fn new(request: &'a JsonRpcRequest) -> Self {
        Self {
            values: request.params.as_ref(),
            id: request.id.clone(),
        }
    }

    fn id(&self) -> Option<serde_json::Value> {
        self.id.clone()
    }

    fn get(&self, key: &str) -> Option<&'a serde_json::Value> {
        self.values.and_then(|p| p.get(key))
    }

    // absent means None; a present value of the wrong type means -32602 naming the field
    fn typed<T>(
        &self,
        key: &str,
        read: impl FnOnce(&'a serde_json::Value) -> Option<T>,
        expected: &str,
    ) -> Result<Option<T>, Box<JsonRpcResponse>> {
        let Some(value) = self.get(key) else {
            return Ok(None);
        };
        read(value).map(Some).ok_or_else(|| {
            Box::new(JsonRpcResponse::error(
                self.id(),
                rpc_code::INVALID_PARAMS,
                format!("'{key}' must be {expected}."),
            ))
        })
    }

    fn str(&self, key: &str) -> Result<&'a str, Box<JsonRpcResponse>> {
        self.get(key).and_then(|v| v.as_str()).ok_or_else(|| {
            Box::new(JsonRpcResponse::error(
                self.id(),
                rpc_code::INVALID_PARAMS,
                format!("'{key}' is required and must be a string."),
            ))
        })
    }

    // absent → default; present but not a u64 → -32602 naming the field
    fn u64(&self, key: &str, default: u64) -> Result<u64, Box<JsonRpcResponse>> {
        match self.get(key) {
            None => Ok(default),
            Some(value) => value.as_u64().ok_or_else(|| {
                Box::new(JsonRpcResponse::error(
                    self.id(),
                    rpc_code::INVALID_PARAMS,
                    format!("'{key}' must be a non-negative integer."),
                ))
            }),
        }
    }

    fn optional_str(&self, key: &str) -> Result<Option<&'a str>, Box<JsonRpcResponse>> {
        self.typed(key, serde_json::Value::as_str, "a string")
    }

    // Absent is false; present and not a boolean is refused, never read as false.
    fn flag(&self, key: &str) -> Result<bool, Box<JsonRpcResponse>> {
        self.typed(key, serde_json::Value::as_bool, "a boolean")
            .map(|value| value.unwrap_or(false))
    }

    fn optional_u64(&self, key: &str) -> Result<Option<u64>, Box<JsonRpcResponse>> {
        self.typed(key, serde_json::Value::as_u64, "a non-negative integer")
    }
}

// The daemon started without a pipeline. The code says which fix applies, so a
// client can prompt for a microphone or re-run setup instead of retrying.
fn unavailable(id: Option<serde_json::Value>, error: &RecordingError) -> JsonRpcResponse {
    let code = match error {
        RecordingError::Microphone(_) => rpc_code::MICROPHONE,
        RecordingError::Model(_) => rpc_code::MODEL,
        RecordingError::Provider(_) => rpc_code::PROVIDER,
        RecordingError::KeyFile(_) => rpc_code::KEY_FILE,
    };
    JsonRpcResponse::error(id, code, format!("Recording is unavailable: {error}"))
}

/// Gives this call's session back on every exit. A client that goes away drops
/// the call before its answer arrives, and the listen ends when the consumer
/// sees the session gone.
struct EndsTheSession<'a> {
    state: &'a DaemonState,
    session: u64,
}

impl Drop for EndsTheSession<'_> {
    fn drop(&mut self) {
        self.state.disarm(self.session);
    }
}

/// `None` while the pipeline is open. A pipeline still being built refuses the
/// same way a broken one does: nothing records either way.
fn not_recording(
    id: Option<serde_json::Value>,
    pipeline: &crate::state::Pipeline,
) -> Option<Box<JsonRpcResponse>> {
    match pipeline {
        crate::state::Pipeline::Open => None,
        crate::state::Pipeline::Opening => Some(Box::new(JsonRpcResponse::error(
            id,
            rpc_code::MICROPHONE,
            "Recording is unavailable: the microphone is still opening",
        ))),
        crate::state::Pipeline::Broken(error) => Some(Box::new(unavailable(id, error))),
    }
}

/// Nothing stops Banshee working. A pipeline still opening raises no blocker,
/// because waiting is nobody's to fix, and it is not ready either: nothing
/// records until the microphone is open.
fn ready(blockers: &[banshee_common::Blocker], pipeline: &crate::state::Pipeline) -> bool {
    blockers.is_empty() && matches!(pipeline, crate::state::Pipeline::Open)
}

pub fn status_payload(daemon_state: &DaemonState) -> serde_json::Value {
    // Read once for every answer below, so one reply cannot report two states
    // of the pipeline.
    let pipeline = daemon_state.pipeline();
    let blockers = readiness::blockers(daemon_state, &pipeline);
    let running = daemon_state.running_config();
    // Read once for the two sides below, so one reply cannot answer from two
    // states of the file. A file that will not parse holds no key either way.
    let credentials = crate::credentials::Credentials::load().ok();
    // The same call the `banshee.state_changed` push answers with, so a reply
    // and a push cannot spell one fact two ways.
    let mut payload = live_state_at(daemon_state, &pipeline);
    let rest = serde_json::json!({
        "running": true,
        "version": daemon_state.version(),
        "stt_model": daemon_state.stt_model(),
        "vad_model": daemon_state.vad_model(),
        "uptime_seconds": daemon_state.uptime().as_secs(),
        "vad_threshold": daemon_state.vad_threshold(),
        "history_enabled": daemon_state.history_enabled(),
        // The window shows this rather than summing a file list it does not
        // hold: only the daemon knows which files are already here.
        "download_megabytes": crate::models::download::pending_megabytes(
            &daemon_state.wanted_downloads(),
            daemon_state.models_dir(),
        ),
        // What is absent, which is not what stops Banshee working: a heavier
        // preset chosen while dictation runs on the loaded one is a file to
        // fetch and no blocker at all. A client pricing one of these must not
        // reach for the sum above, which covers the detector and the voice too.
        "missing_downloads": crate::models::download::still_missing(
            &daemon_state.wanted_downloads(),
            daemon_state.models_dir(),
        )
        .iter()
        .map(|file| {
            serde_json::json!({
                "name": file.name,
                "role": crate::models::download::role(&file.name),
                "megabytes": file.megabytes,
            })
        })
        .collect::<Vec<_>>(),
        // The English-only build reads English whatever `stt.language` says.
        // Read off the model the listener loaded, not the configured preset:
        // a preset applied without persist moves one and not the other.
        "english_only": daemon_state
            .stt_model()
            .is_some_and(crate::speech_to_text::english_only),
        // False where the compositor holds the binding, so the window does not
        // name a key the daemon never listens for.
        "hotkey_listens": crate::hotkey::listens(),
        "bindable_modifiers": crate::binding::bindable_modifiers(),
        // Stated, so no client invents a narrower definition of ready
        "ready": ready(&blockers, &pipeline),
        "blockers": blockers,
        "config": &*daemon_state.config(),
        "pending": daemon_state.pending(),
        // The providers are read at startup, so the running config answers,
        // not the file a `persist` write has already replaced. The key file is
        // read each time: a key set after startup is "present" before the restart.
        "remote": remote_report(
            &running,
            |side| credentials.as_ref().is_some_and(|held| held.key(side).is_some()),
            daemon_state.speaker_started(),
        ),
    });

    let serde_json::Value::Object(rest) = rest else {
        unreachable!("an object literal answers an object")
    };
    payload
        .as_object_mut()
        .expect("live_state answers an object")
        .extend(rest);
    with_key_press_access(payload)
}

/// Where each side sends what it handles, and whether its key is set. The key
/// read is a parameter so a test can answer for one side without a key file.
/// `speaker_started` says whether the speaker the config names is the one
/// running. It is false under a local provider whose Kokoro failed to load,
/// because the system voice speaks then too.
fn remote_report(
    config: &crate::config::Config,
    key_present: impl Fn(crate::credentials::RemoteKey) -> bool,
    speaker_started: bool,
) -> serde_json::Value {
    serde_json::json!({
        "stt": {
            "remote": config.stt.provider.is_remote(),
            "host": config.stt.provider.is_remote().then(|| config.stt.remote.host()),
            "key_present": key_present(crate::credentials::RemoteKey::Stt),
        },
        "tts": {
            "remote": config.tts.provider.is_remote(),
            "host": config.tts.provider.is_remote().then(|| config.tts.remote.host()),
            "speaker_started": speaker_started,
            "key_present": key_present(crate::credentials::RemoteKey::Tts),
        },
    })
}

/// Only the daemon can answer this, so only its reply carries it.
#[cfg(target_os = "macos")]
fn with_key_press_access(mut payload: serde_json::Value) -> serde_json::Value {
    payload["key_press_access"] = crate::permissions::key_presses_reach_us().as_str().into();
    payload
}

#[cfg(not(target_os = "macos"))]
fn with_key_press_access(payload: serde_json::Value) -> serde_json::Value {
    payload
}

/// The `banshee.state_changed` params: what moves without a client touching it.
/// The two device fields and the pipeline move on their own, because the
/// watchdog rebinds while the daemon idles. `vad_threshold` moves at runtime too, but only when a
/// `configure` call asks it to, and that call already answers.
pub fn live_state(daemon_state: &DaemonState) -> serde_json::Value {
    live_state_at(daemon_state, &daemon_state.pipeline())
}

fn live_state_at(
    daemon_state: &DaemonState,
    pipeline: &crate::state::Pipeline,
) -> serde_json::Value {
    let mut live = serde_json::json!({
        "recording": daemon_state.is_recording(),
        "armed": daemon_state.is_armed(),
        "transcribing": daemon_state.is_transcribing(),
        "loading_model": daemon_state.is_loading_model(),
        "telling": daemon_state.is_telling(),
        "speaking": daemon_state.speech().is_speaking(),
        "audio_device": daemon_state.audio_device(),
        "missing_device": daemon_state.missing_device(),
        "last_error": daemon_state.last_error(),
        "last_speech_error": daemon_state.last_speech_error(),
        "pipeline": pipeline.as_str(),
    });
    // Ranked once, in banshee-common, so no client ranks the flags itself.
    live["activity"] = banshee_common::Activity::of(&live).word().into();
    live
}

fn speak(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    let raw_text = match params.str("text") {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let interrupt = match params.flag("interrupt") {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let voice = match params.optional_str("voice") {
        Ok(value) => value,
        Err(response) => return *response,
    };

    let clean_text = sanitize(raw_text);

    match daemon_state.speech().speak(&clean_text, interrupt, voice) {
        Ok(utterance_id) => JsonRpcResponse::success(
            params.id(),
            serde_json::json!({"ok": true, "utterance_id": utterance_id}),
        ),
        Err(error) => from_error(params.id(), error),
    }
}

fn stop_speaking(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    daemon_state.speech().stop();
    JsonRpcResponse::success(params.id(), serde_json::json!({"ok": true}))
}

fn stop(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    daemon_state.shutdown().notify_one();
    JsonRpcResponse::success(params.id(), serde_json::json!({"ok": true}))
}

fn dictate_target(params: &Params<'_>) -> Result<TranscribeTarget, Box<JsonRpcResponse>> {
    let dictate = params.flag("dictate")?;
    let tell = params.flag("tell")?;
    Ok(match (dictate, tell) {
        (true, true) => {
            return Err(Box::new(JsonRpcResponse::error(
                params.id(),
                rpc_code::INVALID_PARAMS,
                "dictate and tell are two destinations. Pass one.",
            )));
        }
        (true, false) => TranscribeTarget::Dictate,
        (false, true) => TranscribeTarget::Tell,
        (false, false) => TranscribeTarget::Mailbox,
    })
}

fn record_start(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    let action = match dictate_target(&params) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    // Checked before the transition, so -32004 keeps meaning "busy"
    if let Some(response) = not_recording(params.id(), &daemon_state.pipeline()) {
        return *response;
    }
    if daemon_state.record_start(action) {
        JsonRpcResponse::success(params.id(), serde_json::json!({"ok": true}))
    } else {
        JsonRpcResponse::error(
            params.id(),
            rpc_code::BUSY,
            "Microphone is busy with another recording or listening session.",
        )
    }
}

fn record_stop(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    daemon_state.record_stop();
    JsonRpcResponse::success(params.id(), serde_json::json!({"ok": true}))
}

fn record_toggle(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    let action = match dictate_target(&params) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    if let Some(response) = not_recording(params.id(), &daemon_state.pipeline()) {
        return *response;
    }
    let recording = daemon_state.record_toggle(action);
    JsonRpcResponse::success(params.id(), serde_json::json!({"recording": recording}))
}

async fn silence_within(daemon_state: &DaemonState, budget: Duration) -> bool {
    let mut speaking = daemon_state.speech().subscribe_speaking();
    tokio::time::timeout(budget, speaking.wait_for(|s| !s))
        .await
        .is_ok()
}

// Echo avoidance by ordering: listen only after playback ends.
// Bounded so a stalled backend cannot hold the mic armed forever
async fn playback_ended(daemon_state: &DaemonState, question: &str) -> bool {
    let words = question.split_whitespace().count() as u64;
    let playback_budget = Duration::from_millis(
        (PLAYBACK_BASE_MS + words * PLAYBACK_PER_WORD_MS).min(MAX_PLAYBACK_WAIT_MS),
    );
    silence_within(daemon_state, playback_budget).await
}

async fn ask_user(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    let question = match params.str("question") {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let timeout_ms = match params.u64("timeout_ms", DEFAULT_ASK_WAIT_MS) {
        Ok(value) => value.min(MAX_ASK_WAIT_MS),
        Err(response) => return *response,
    };

    if let Some(response) = not_recording(params.id(), &daemon_state.pipeline()) {
        return *response;
    }

    // One armed session at a time; the mode is the lock. Armed before the wait
    // below, so a press while Banshee talks holds to answer rather than dictates.
    let Some(session) = daemon_state.arm_for_ask() else {
        return JsonRpcResponse::error(
            params.id(),
            rpc_code::BUSY,
            "Microphone is busy with another recording or listening session.",
        );
    };

    // From here the mode is held, and every way out of this call gives it back:
    // the question is spoken before anyone listens, and a client that dies
    // while it plays would otherwise hold the microphone until a restart.
    let _ends_the_session = EndsTheSession {
        state: daemon_state,
        session,
    };

    // A status outruns any budget short of the stalled-backend bound.
    let settled = silence_within(daemon_state, Duration::from_millis(MAX_PLAYBACK_WAIT_MS)).await;

    // Interrupts only what outran the wait, so a stalled backend costs one budget.
    let clean_question = sanitize(question);
    if let Err(e) = daemon_state.speech().speak(&clean_question, !settled, None) {
        return JsonRpcResponse::error(
            params.id(),
            rpc_code::INTERNAL,
            format!("Failed to speak question: {e}"),
        );
    }

    if !playback_ended(daemon_state, &clean_question).await {
        daemon_state.speech().stop();
        return JsonRpcResponse::error(
            params.id(),
            rpc_code::INTERNAL,
            "Question playback did not finish.",
        );
    }

    let (reply, answer) = tokio::sync::oneshot::channel();
    let command = ConsumerCommand::Ask(AskCommand {
        reply,
        timeout: Duration::from_millis(timeout_ms),
        session,
    });
    if daemon_state.commands().send(command).is_err() {
        return JsonRpcResponse::error(
            params.id(),
            rpc_code::INTERNAL,
            "Audio pipeline is not running.",
        );
    }

    match answer.await {
        Ok(Ok(text)) => JsonRpcResponse::success(params.id(), serde_json::json!({ "text": text })),
        // Distinct from silence, which answers empty text
        Ok(Err(reason)) => JsonRpcResponse::error(
            params.id(),
            rpc_code::LISTENING_FAILED,
            format!("Listening failed: {reason}"),
        ),
        Err(_) => JsonRpcResponse::error(
            params.id(),
            rpc_code::INTERNAL,
            "Listening session ended unexpectedly.",
        ),
    }
}

async fn get_transcription(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    let mut since_id = match params.u64("since_id", 0) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let wait_ms = match params.u64("wait_ms", 0) {
        Ok(value) => value.min(MAX_WAIT_MS),
        Err(response) => return *response,
    };

    // Subscribe before the first read so a push landing in between
    // is still seen by wait_for
    let mut latest_id = daemon_state.subscribe_transcriptions();

    // since_id ahead of the newest id = stale cursor from an older daemon run
    if since_id > *latest_id.borrow() {
        since_id = 0;
    }

    let mut transcriptions = daemon_state.transcriptions_since(since_id);

    if transcriptions.is_empty() && wait_ms > 0 {
        let _ = tokio::time::timeout(
            Duration::from_millis(wait_ms),
            latest_id.wait_for(|id| *id > since_id),
        )
        .await;
        transcriptions = daemon_state.transcriptions_since(since_id);
    }

    JsonRpcResponse::success(
        params.id(),
        serde_json::json!({ "transcriptions": transcriptions }),
    )
}

fn configure(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    let Some(requested) = params.get("settings") else {
        return JsonRpcResponse::error(
            params.id(),
            rpc_code::INVALID_PARAMS,
            "'settings' is required, as in {\"stt.language\": \"de\"}.",
        );
    };
    let assignments: settings::Assignments = match serde_json::from_value(requested.clone()) {
        Ok(assignments) => assignments,
        Err(error) => {
            return JsonRpcResponse::error(
                params.id(),
                rpc_code::INVALID_PARAMS,
                format!("'settings' must map dotted keys to values: {error}"),
            );
        }
    };
    let persist = match params.flag("persist") {
        Ok(value) => value,
        Err(response) => return *response,
    };

    match settings::configure(Some(daemon_state), assignments, persist) {
        Ok(outcome) => JsonRpcResponse::success(
            params.id(),
            serde_json::json!({
                "ok": true,
                "applied": outcome.applied,
                "restart_required": outcome.restart_required,
            }),
        ),
        Err(error) => JsonRpcResponse::error(params.id(), error.rpc_code(), error.rpc_message()),
    }
}

fn download_models(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    let dir = daemon_state.models_dir().to_path_buf();
    let missing = crate::models::download::still_missing(&daemon_state.wanted_downloads(), &dir);
    if missing.is_empty() {
        return JsonRpcResponse::success(
            params.id(),
            serde_json::json!({"ok": true, "downloading": []}),
        );
    }
    let Some(slot) = daemon_state.start_downloading() else {
        return JsonRpcResponse::error(
            params.id(),
            rpc_code::DOWNLOAD_RUNNING,
            "A download is already running.",
        );
    };

    let names: Vec<&str> = missing.iter().map(|d| d.name.as_str()).collect();
    let response = JsonRpcResponse::success(
        params.id(),
        serde_json::json!({"ok": true, "downloading": names}),
    );

    let state = Arc::clone(daemon_state);
    let dir = dir.clone();
    tokio::spawn(async move {
        {
            let mut report = |progress| state.report_download(progress);
            if let Err(error) =
                crate::models::download::download_all(&dir, &missing, &mut report).await
            {
                log::error!("Download failed: {error}");
            }
        }
        // The files these settings were waiting for are here now, so a
        // preset or a voice chosen before its model arrived takes
        // effect rather than waiting for a restart with nothing to do.
        crate::settings::reapply_pending(&state);
        drop(slot);
    });
    response
}

// Every voice this build can name, and whether each is here. A client
// that can fetch one needs the whole list to offer a choice; one that
// cannot filters to the installed ones itself.
fn list_voices(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    let installed = crate::models::installed_voices();
    let mut ids: Vec<String> = crate::text_to_speech::local::voices::catalogue()
        .map(str::to_string)
        .collect();
    for id in &installed {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    let voices: Vec<_> = ids
        .iter()
        .map(|id| crate::text_to_speech::local::voices::describe(id, installed.contains(id)))
        .collect();
    JsonRpcResponse::success(
        params.id(),
        serde_json::json!({ "voices": voices, "current": daemon_state.tts_voice() }),
    )
}

// The engine's own list, so a client offering a choice cannot drift
// from what the engine will accept.
fn list_languages(params: Params<'_>) -> JsonRpcResponse {
    JsonRpcResponse::success(
        params.id(),
        serde_json::json!({ "languages": crate::speech_to_text::local::languages::all() }),
    )
}

fn list_input_devices(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    JsonRpcResponse::success(
        params.id(),
        serde_json::json!({
            "devices": crate::audio::input_devices(),
            "current": daemon_state.audio_device(),
        }),
    )
}

fn status(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    JsonRpcResponse::success(params.id(), status_payload(daemon_state))
}

fn history(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    let limit = match params.optional_u64("limit") {
        Ok(None) => None,
        Ok(Some(limit)) => match u32::try_from(limit) {
            Ok(limit) => Some(limit),
            Err(_) => {
                return JsonRpcResponse::error(
                    params.id(),
                    rpc_code::INVALID_PARAMS,
                    "'limit' must fit in 32 bits.",
                );
            }
        },
        Err(response) => return *response,
    };
    match daemon_state.with_history(|c| crate::history::TranscriptionHistory::list(c, limit)) {
        Some(Ok(history)) => {
            JsonRpcResponse::success(params.id(), serde_json::json!({ "history": history }))
        }
        Some(Err(e)) => JsonRpcResponse::error(
            params.id(),
            rpc_code::INTERNAL,
            format!("Failed to retrieve history: {e}"),
        ),
        None => JsonRpcResponse::error(
            params.id(),
            rpc_code::HISTORY_OFF,
            "History is not enabled.",
        ),
    }
}

fn clear_history(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    match daemon_state.with_history(crate::history::TranscriptionHistory::clear) {
        Some(Ok(())) => JsonRpcResponse::success(params.id(), serde_json::json!({})),
        // Not -32003: that code names history being off, and the
        // listing path already answers -32603 for the same failure.
        Some(Err(e)) => JsonRpcResponse::error(
            params.id(),
            rpc_code::INTERNAL,
            format!("Failed to clear history: {e}"),
        ),
        None => JsonRpcResponse::error(
            params.id(),
            rpc_code::HISTORY_OFF,
            "History is not enabled.",
        ),
    }
}

async fn agents(params: Params<'_>) -> JsonRpcResponse {
    off_the_worker(params.id(), read_the_agents).await
}

async fn connect(
    params: Params<'_>,
    work: fn(connect::Agent) -> Result<serde_json::Value, BansheeError>,
) -> JsonRpcResponse {
    let disconnect = match params.flag("disconnect") {
        Ok(value) => value,
        Err(response) => return *response,
    };
    if disconnect {
        return JsonRpcResponse::error(
            params.id(),
            rpc_code::INVALID_PARAMS,
            "Disconnect is not available yet. Remove Banshee from the agent's config by hand.",
        );
    }
    let slug = match params.str("agent") {
        Ok(slug) => slug,
        Err(response) => return *response,
    };
    let Some(agent) = connect::Agent::ALL
        .into_iter()
        .find(|agent| agent.name() == slug)
    else {
        return JsonRpcResponse::error(
            params.id(),
            rpc_code::INVALID_PARAMS,
            format!("'{slug}' is not a known agent."),
        );
    };
    off_the_worker(params.id(), move || work(agent)).await
}

fn open_permission(params: Params<'_>) -> JsonRpcResponse {
    let id = match params.str("id") {
        Ok(value) => value,
        Err(response) => return *response,
    };
    match permissions::open_pane(id) {
        Ok(()) => JsonRpcResponse::success(params.id(), serde_json::json!({"ok": true})),
        Err(error) => JsonRpcResponse::error(params.id(), error.rpc_code(), error.rpc_message()),
    }
}

pub async fn dispatch(request: JsonRpcRequest, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    let params = Params::new(&request);
    match request.method.as_str() {
        BANSHEE_SPEAK => speak(params, daemon_state),
        BANSHEE_STOP_SPEAKING => stop_speaking(params, daemon_state),
        BANSHEE_STOP => stop(params, daemon_state),
        BANSHEE_RECORD_START => record_start(params, daemon_state),
        BANSHEE_RECORD_STOP => record_stop(params, daemon_state),
        BANSHEE_RECORD_TOGGLE => record_toggle(params, daemon_state),
        BANSHEE_ASK_USER => ask_user(params, daemon_state).await,
        BANSHEE_GET_TRANSCRIPTION => get_transcription(params, daemon_state).await,
        BANSHEE_CONFIGURE => configure(params, daemon_state),
        BANSHEE_DOWNLOAD_MODELS => download_models(params, daemon_state),
        BANSHEE_LIST_VOICES => list_voices(params, daemon_state),
        BANSHEE_LIST_LANGUAGES => list_languages(params),
        BANSHEE_LIST_INPUT_DEVICES => list_input_devices(params, daemon_state),
        // Subscribing answers with the poll, so a client needs no first poll and
        // cannot miss a change in the gap before the pushes start. `daemon.rs`
        // owns the pushing itself, because only it holds the connection.
        BANSHEE_STATUS | BANSHEE_SUBSCRIBE => status(params, daemon_state),
        BANSHEE_HISTORY => history(params, daemon_state),
        BANSHEE_CLEAR_HISTORY => clear_history(params, daemon_state),
        BANSHEE_AGENTS => agents(params).await,
        BANSHEE_CONNECT_PLAN => connect(params, read_the_plan).await,
        BANSHEE_CONNECT_APPLY => connect(params, write_the_plan).await,
        BANSHEE_OPEN_PERMISSION => open_permission(params),
        _ => JsonRpcResponse::error(
            params.id(),
            rpc_code::METHOD_NOT_FOUND,
            format!("Method '{}' not found!", request.method),
        ),
    }
}

#[cfg(test)]
mod tests;

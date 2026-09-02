use std::sync::Arc;
use std::time::Duration;

use banshee_common::error::BansheeError;
use banshee_common::{
    BANSHEE_AGENTS, BANSHEE_ASK_USER, BANSHEE_CLEAR_HISTORY, BANSHEE_CONFIGURE,
    BANSHEE_CONNECT_APPLY, BANSHEE_CONNECT_PLAN, BANSHEE_DOWNLOAD_MODELS,
    BANSHEE_GET_TRANSCRIPTION, BANSHEE_HISTORY, BANSHEE_LIST_INPUT_DEVICES, BANSHEE_LIST_LANGUAGES,
    BANSHEE_LIST_VOICES, BANSHEE_OPEN_PERMISSION, BANSHEE_RECORD_START, BANSHEE_RECORD_STOP,
    BANSHEE_SPEAK, BANSHEE_STATUS, BANSHEE_STOP, BANSHEE_STOP_SPEAKING, BANSHEE_SUBSCRIBE,
};
use banshee_common::{JsonRpcRequest, JsonRpcResponse};

use crate::connect;
use crate::permissions;
use crate::state::{
    AskCommand, ConsumerCommand, DaemonState, RecordingError, RecordingMode, TranscribeTarget,
};
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
        Err(_) => {
            JsonRpcResponse::error(id, -32603, "The connect task stopped before it answered.")
        }
    }
}

fn the_plan(agent: connect::Agent) -> Result<(connect::Env, Vec<connect::Change>), BansheeError> {
    let env = connect::Env::from_machine()?;
    let changes = connect::plan(agent, &env)?;
    Ok((env, changes))
}

fn read_the_agents() -> Result<serde_json::Value, BansheeError> {
    let env = connect::Env::from_machine()?;
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
                -32602,
                format!("'{key}' must be {expected}."),
            ))
        })
    }

    fn str(&self, key: &str) -> Result<&'a str, Box<JsonRpcResponse>> {
        self.get(key).and_then(|v| v.as_str()).ok_or_else(|| {
            Box::new(JsonRpcResponse::error(
                self.id(),
                -32602,
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
                    -32602,
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
        RecordingError::Microphone(_) => -32000,
        RecordingError::Model(_) => -32002,
    };
    JsonRpcResponse::error(id, code, format!("Recording is unavailable: {error}"))
}

pub fn status_payload(daemon_state: &DaemonState) -> serde_json::Value {
    let blockers = readiness::blockers(daemon_state);
    let payload = serde_json::json!({
        "running": true,
        "version": daemon_state.version(),
        "stt_model": daemon_state.stt_model(),
        "vad_model": daemon_state.vad_model(),
        "audio_device": daemon_state.audio_device(),
        "missing_device": daemon_state.missing_device(),
        "recording": daemon_state.is_recording(),
        "armed": daemon_state.is_armed(),
        "transcribing": daemon_state.is_transcribing(),
        "speaking": daemon_state.speech().is_speaking(),
        "uptime_seconds": daemon_state.uptime().as_secs(),
        "vad_threshold": daemon_state.vad_threshold(),
        "history_enabled": daemon_state.history_enabled(),
        // The window shows this rather than summing a file list it does not
        // hold: only the daemon knows which files are already here.
        "download_megabytes": crate::models::download::models_dir()
            .map(|dir| {
                crate::models::download::pending_megabytes(&daemon_state.wanted_downloads(), &dir)
            })
            .unwrap_or(0),
        // The English-only build reads English whatever `stt.language` says.
        // Stated here rather than worked out from the preset name by every
        // client that has to know.
        "english_only": crate::speech_to_text::whisper::english_only(
            daemon_state.config().stt.preset.model_name(),
        ),
        // Stated, so no client invents a narrower definition of ready
        "ready": blockers.is_empty(),
        "blockers": blockers,
        "config": &*daemon_state.config(),
        "pending": daemon_state.pending(),
    });
    with_key_press_access(payload)
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
/// The two device fields move on their own, because the watchdog rebinds while
/// the daemon idles. `vad_threshold` moves at runtime too, but only when a
/// `configure` call asks it to, and that call already answers.
pub fn live_state(daemon_state: &DaemonState) -> serde_json::Value {
    serde_json::json!({
        "recording": daemon_state.is_recording(),
        "armed": daemon_state.is_armed(),
        "transcribing": daemon_state.is_transcribing(),
        "speaking": daemon_state.speech().is_speaking(),
        "audio_device": daemon_state.audio_device(),
        "missing_device": daemon_state.missing_device(),
    })
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
        Err(e) => JsonRpcResponse::error(
            params.id(),
            -32603,
            format!("Failed to start speech playback: {e}"),
        ),
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

fn record_start(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    let dictate = match params.flag("dictate") {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let action = if dictate {
        TranscribeTarget::Dictate
    } else {
        TranscribeTarget::Mailbox
    };
    // Checked before the transition, so -32004 keeps meaning "busy"
    if let Some(reason) = daemon_state.recording_error() {
        return unavailable(params.id(), &reason);
    }
    if daemon_state.record_start(action) {
        JsonRpcResponse::success(params.id(), serde_json::json!({"ok": true}))
    } else {
        JsonRpcResponse::error(
            params.id(),
            -32004,
            "Microphone is busy with another recording or listening session.",
        )
    }
}

fn record_stop(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    daemon_state.record_stop();
    JsonRpcResponse::success(params.id(), serde_json::json!({"ok": true}))
}

// Echo avoidance by ordering: listen only after playback ends.
// Bounded so a stalled backend cannot hold the mic armed forever
async fn playback_ended(daemon_state: &DaemonState, question: &str) -> bool {
    let words = question.split_whitespace().count() as u64;
    let playback_budget = Duration::from_millis(
        (PLAYBACK_BASE_MS + words * PLAYBACK_PER_WORD_MS).min(MAX_PLAYBACK_WAIT_MS),
    );
    let mut speaking = daemon_state.speech().subscribe_speaking();
    tokio::time::timeout(playback_budget, speaking.wait_for(|s| !s))
        .await
        .is_ok()
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

    if let Some(reason) = daemon_state.recording_error() {
        return unavailable(params.id(), &reason);
    }

    // One armed session at a time; the mode is the lock
    if !daemon_state.arm_for_ask() {
        return JsonRpcResponse::error(
            params.id(),
            -32004,
            "Microphone is busy with another recording or listening session.",
        );
    }

    // Interrupt: the question must not queue behind stale status speech
    let clean_question = sanitize(question);
    if let Err(e) = daemon_state.speech().speak(&clean_question, true, None) {
        daemon_state.set_recording_mode(RecordingMode::Idle);
        return JsonRpcResponse::error(
            params.id(),
            -32603,
            format!("Failed to speak question: {e}"),
        );
    }

    if !playback_ended(daemon_state, &clean_question).await {
        daemon_state.speech().stop();
        daemon_state.set_recording_mode(RecordingMode::Idle);
        return JsonRpcResponse::error(params.id(), -32603, "Question playback did not finish.");
    }

    let (reply, answer) = tokio::sync::oneshot::channel();
    let command = ConsumerCommand::Ask(AskCommand {
        reply,
        timeout: Duration::from_millis(timeout_ms),
    });
    if daemon_state.commands().send(command).is_err() {
        daemon_state.set_recording_mode(RecordingMode::Idle);
        return JsonRpcResponse::error(params.id(), -32603, "Audio pipeline is not running.");
    }

    match answer.await {
        Ok(text) => JsonRpcResponse::success(params.id(), serde_json::json!({ "text": text })),
        Err(_) => {
            daemon_state.set_recording_mode(RecordingMode::Idle);
            JsonRpcResponse::error(params.id(), -32603, "Listening session ended unexpectedly.")
        }
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
            -32602,
            "'settings' is required, as in {\"stt.language\": \"de\"}.",
        );
    };
    let assignments: settings::Assignments = match serde_json::from_value(requested.clone()) {
        Ok(assignments) => assignments,
        Err(error) => {
            return JsonRpcResponse::error(
                params.id(),
                -32602,
                format!("'settings' must map dotted keys to values: {error}"),
            );
        }
    };
    let persist = match params.flag("persist") {
        Ok(value) => value,
        Err(response) => return *response,
    };

    match settings::configure(Some(daemon_state), &assignments, persist) {
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
    let dir = match crate::models::download::models_dir() {
        Ok(dir) => dir,
        Err(error) => {
            return JsonRpcResponse::error(params.id(), -32603, error.to_string());
        }
    };
    let missing = crate::models::download::still_missing(&daemon_state.wanted_downloads(), &dir);
    if missing.is_empty() {
        return JsonRpcResponse::success(
            params.id(),
            serde_json::json!({"ok": true, "downloading": []}),
        );
    }
    let Some(slot) = daemon_state.start_downloading() else {
        return JsonRpcResponse::error(params.id(), -32005, "A download is already running.");
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
                eprintln!("Download failed: {error}");
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
    let mut ids: Vec<String> = crate::text_to_speech::voices::catalogue()
        .map(str::to_string)
        .collect();
    for id in &installed {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    let voices: Vec<_> = ids
        .iter()
        .map(|id| crate::text_to_speech::voices::describe(id, installed.contains(id)))
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
        serde_json::json!({ "languages": crate::speech_to_text::languages::all() }),
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
                return JsonRpcResponse::error(params.id(), -32602, "'limit' must fit in 32 bits.");
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
            -32603,
            format!("Failed to retrieve history: {e}"),
        ),
        None => JsonRpcResponse::error(params.id(), -32003, "History is not enabled."),
    }
}

fn clear_history(params: Params<'_>, daemon_state: &Arc<DaemonState>) -> JsonRpcResponse {
    match daemon_state.with_history(crate::history::TranscriptionHistory::clear) {
        Some(Ok(())) => JsonRpcResponse::success(params.id(), serde_json::json!({})),
        // Not -32003: that code names history being off, and the
        // listing path already answers -32603 for the same failure.
        Some(Err(e)) => {
            JsonRpcResponse::error(params.id(), -32603, format!("Failed to clear history: {e}"))
        }
        None => JsonRpcResponse::error(params.id(), -32003, "History is not enabled."),
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
            -32602,
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
            -32602,
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
            -32601,
            format!("Method '{}' not found!", request.method),
        ),
    }
}

#[cfg(test)]
mod tests;

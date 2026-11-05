use std::collections::BTreeMap;
use std::sync::Mutex;

use banshee_common::error::BansheeError;
use serde::Serialize;
use toml_edit::DocumentMut;

use crate::config::{Config, FeedbackMode};
use crate::credentials::RemoteKey;
use crate::models::download::Download;
use crate::speech_to_text::Route;
use crate::state::{DaemonState, Listening};
use crate::text_to_speech::SpeakerSpec;

/// Dotted `section.field` keys, spelled as `config.toml` spells them.
pub type Assignments = BTreeMap<String, serde_json::Value>;

/// The catalogue model a write names, when it writes `stt.model`.
pub fn listening_model_named(
    key: &str,
    value: &serde_json::Value,
) -> Option<&'static banshee_common::listening::ListeningModel> {
    if key != "stt.model" {
        return None;
    }
    value.as_str().and_then(banshee_common::listening::find)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Live {
    VadThreshold,
    EndpointSilence,
    InputDevice,
    BargeIn,
    KeepOpen,
    Feedback,
    SpeakProblems,
    SaveHistory,
    Tts,
    Vocabulary,
    Model,
    Speech,
    UnloadAfter,
    Hotkey,
    Listener,
    Speaker,
}

fn live(key: &str) -> Option<Live> {
    match key {
        "stt.vad_threshold" => Some(Live::VadThreshold),
        "stt.endpoint_silence_ms" => Some(Live::EndpointSilence),
        "audio.input_device" => Some(Live::InputDevice),
        "audio.barge_in" => Some(Live::BargeIn),
        "audio.keep_open" => Some(Live::KeepOpen),
        "feedback.mode" => Some(Live::Feedback),
        "feedback.speak_problems" => Some(Live::SpeakProblems),
        "daemon.save_history" => Some(Live::SaveHistory),
        "tts.voice" => Some(Live::Tts),
        "stt.vocabulary" => Some(Live::Vocabulary),
        // Read per utterance, so it moves no model.
        "stt.translate" => Some(Live::Speech),
        "stt.model" => Some(Live::Model),
        "models.unload_after" => Some(Live::UnloadAfter),
        "audio.hotkey" | "audio.hotkey_mode" => Some(Live::Hotkey),
        "stt.provider" | "stt.remote.base_url" | "stt.remote.model" => Some(Live::Listener),
        "tts.provider"
        | "tts.fallback"
        | "tts.remote.base_url"
        | "tts.remote.model"
        | "tts.remote.voice"
        | "tts.remote.instructions"
        | "tts.remote.response_format"
        | "tts.remote.sample_rate" => Some(Live::Speaker),
        _ => None,
    }
}

pub fn is_hotkey(key: &str) -> bool {
    live(key) == Some(Live::Hotkey)
}

/// Puts one live setting into effect, and answers whether the daemon took it.
/// One arm per `Live` variant and no catch-all, so a variant without an arm
/// does not compile.
fn apply(variant: Live, state: &DaemonState, config: &Config) -> bool {
    match variant {
        Live::VadThreshold => {
            state.set_vad_threshold(config.stt.vad_threshold);
            true
        }
        // Read when the next answer opens; an answer already open keeps its length
        Live::EndpointSilence => {
            state.set_endpoint_silence_ms(config.stt.endpoint_silence_ms);
            true
        }
        // Open capture rebinds on its next tick, and closed capture opens it
        // at the next use
        Live::InputDevice => {
            state.set_wanted_device(config.audio.input_device.clone());
            true
        }
        // Read at every record start, so the next dictation obeys it
        Live::BargeIn => {
            state.set_barge_in(config.audio.barge_in);
            true
        }
        // The thread that owns capture opens or closes it at once
        Live::KeepOpen => {
            state.set_keep_open(config.audio.keep_open);
            true
        }
        // The next cue reads this, so the next dictation obeys it
        Live::Feedback => {
            state.set_feedback_mode(config.feedback_mode());
            true
        }
        // Read when the next problem appears
        Live::SpeakProblems => {
            state.set_speak_problems(config.speak_problems());
            true
        }
        // Read when the next row is kept
        Live::SaveHistory => {
            state.set_save_history(config.daemon.save_history);
            true
        }
        // A voice of the loaded engine reloads nothing; one of the other engine
        // loads it. Silence takes no voice, and says so.
        Live::Tts => state.set_tts(&config.tts),
        // A listener that has gone leaves the words unread, so say so rather
        // than report a prompt nothing holds
        Live::Vocabulary => state.set_vocabulary(config.stt.vocabulary.clone()),
        Live::Model => reconcile(state, config),
        Live::Speech => state.set_speech((&config.stt).into()),
        Live::UnloadAfter => {
            state.set_unload_after(config.models.unload_limit());
            true
        }
        // The listener takes it at the next key event once no key is held. On
        // Wayland the compositor holds the key, and the daemon listens for none.
        Live::Hotkey => {
            state.set_binding(config.audio.hotkey, config.audio.hotkey_mode);
            true
        }
        Live::Listener => switch_listener(state, config),
        Live::Speaker => switch_speaker(state, config),
    }
}

/// Puts the speaker `config` names in place, unless it is the one in force.
fn switch_speaker(state: &DaemonState, config: &Config) -> bool {
    let next = SpeakerSpec::of(&config.tts);
    if next == state.speaker_spec() {
        return true;
    }
    state.switch_speaker(&config.tts, cuts_off(state, &next))
}

/// Whether a reply from a server has to stop: `next` names no server, or no
/// key is on file. A reply on its way to one server finishes there while the
/// config names another.
fn cuts_off(state: &DaemonState, next: &SpeakerSpec) -> bool {
    let keyed =
        crate::credentials::Credentials::present_at(state.credentials_path(), RemoteKey::Tts);
    !(next.provider.is_remote() && keyed)
}

/// Hands the listener the route `config` names. The switch last asked may
/// still wait in the queue behind a dictation, so a route goes unless it is
/// both the one in force and the one last asked, and the listener in force
/// built. Turning a server off stops a request on its way there.
fn switch_listener(state: &DaemonState, config: &Config) -> bool {
    let next = Route::of(&config.stt);
    let in_force = state.listener_route();
    let asked = state.asked_listener_route();
    let unbuilt = matches!(state.listening(), Listening::Failed(..)) && next == Route::Local;
    if next == in_force && next == asked && !unbuilt {
        return true;
    }
    if next == Route::Local && (in_force.is_remote() || asked.is_remote()) {
        state.withdraw_listener(Some(config.stt.clone()));
    }
    state.replace_listener(config.stt.clone())
}

/// Loads the listening model `config` names when the listener serves another
/// and the file is here, and answers whether the setting is in effect. A
/// download is minutes, so a model still to download leaves the key waiting.
/// A listener that serves no model waits for the one named now.
fn reconcile(state: &DaemonState, config: &Config) -> bool {
    let Some(wanted) = crate::models::stt_model(config) else {
        return true;
    };
    let here = is_here(state, wanted);
    let had = state.claim_listening_model(wanted, here);
    if matches!(
        had,
        Listening::Available(model) | Listening::Loading(model) | Listening::Loaded(model)
            if model == wanted
    ) {
        return true;
    }
    if !here {
        if had.serves() {
            log::warn!(
                "the listening model {} is not downloaded yet, so the model is unchanged",
                wanted.id
            );
        }
        return false;
    }
    // The listener switch in the same write builds the local engine, and a
    // listener that holds no file holds no engine to load into.
    if had.is_remote() {
        return true;
    }
    state.load_stt_model(wanted)
}

/// `reconcile` for a download that ended or a start, which change no setting.
/// A listener that serves keeps its model: the write that picked another one
/// already sent its load, or left its key waiting.
fn reconcile_waiting(state: &DaemonState, config: &Config) {
    let Some(wanted) = waited_for(state, config) else {
        return;
    };
    let here = is_here(state, wanted);
    let had = state.claim_listening_model(wanted, here);
    if !had.serves() && !had.is_remote() && here {
        state.load_stt_model(wanted);
    }
}

/// The listening model `config` names, or under a transcription server the
/// one the local listener still waits for.
fn waited_for(
    state: &DaemonState,
    config: &Config,
) -> Option<&'static banshee_common::listening::ListeningModel> {
    crate::models::stt_model(config).or_else(|| match state.listening() {
        Listening::Missing(model) | Listening::Failed(model, _) => Some(model),
        _ => None,
    })
}

fn is_here(state: &DaemonState, model: &banshee_common::listening::ListeningModel) -> bool {
    crate::models::missing_in(state.models_dir(), &[model.file]).is_empty()
}

/// `reconcile_waiting` under the lock a write holds, for the start.
pub fn reconcile_listening(state: &DaemonState) {
    let _writing = writing();
    reconcile_waiting(state, &state.config());
}

/// The daemon serves a task per connection, so two calls can otherwise read
/// the same file before either writes and one setting is lost.
static WRITING: Mutex<()> = Mutex::new(());

fn writing() -> std::sync::MutexGuard<'static, ()> {
    WRITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Default, Debug)]
pub struct Outcome {
    pub applied: Vec<String>,
    pub waiting: Vec<String>,
}

/// The section a dotted path names, or `None` when the file never wrote it.
fn table_along<'a>(
    mut table: &'a mut toml_edit::Table,
    path: &str,
) -> Result<Option<&'a mut toml_edit::Table>, BansheeError> {
    for name in path.split('.') {
        let Some(item) = table.get_mut(name) else {
            return Ok(None);
        };
        table = item
            .as_table_mut()
            .ok_or_else(|| BansheeError::Rejected(format!("[{path}] is not a section")))?;
    }
    Ok(Some(table))
}

/// Edits the document rather than serializing a `Config`, so hand-written
/// comments and layout survive.
fn edit(existing: &str, assignments: &Assignments) -> Result<(String, Config), BansheeError> {
    let mut document: DocumentMut = existing
        .parse()
        .map_err(|error| BansheeError::Other(format!("config.toml does not parse: {error}")))?;

    for (key, value) in assignments {
        // `[stt.remote]` is a section two deep, so only the last segment is a field
        let (path, field) = key.rsplit_once('.').ok_or_else(|| {
            BansheeError::Rejected(format!("'{key}' must name a section, as in stt.model"))
        })?;
        if value.is_null() {
            if banshee_common::settings::find(key).is_none() {
                return Err(not_a_setting(key));
            }
            if let Some(section) = table_along(document.as_table_mut(), path)? {
                section.remove(field);
                if section.is_empty() {
                    section.set_implicit(true);
                }
            }
            continue;
        }
        let mut table = document.as_table_mut();
        for section in path.split('.') {
            let invented = !table.contains_key(section);
            table = table
                .entry(section)
                .or_insert(toml_edit::table())
                .as_table_mut()
                .ok_or_else(|| BansheeError::Rejected(format!("[{path}] is not a section")))?;
            // Suppressing an existing header takes the comment above it too
            if invented {
                table.set_implicit(true);
            }
        }

        // Ids match ignoring case, and the file keeps the catalogue's spelling
        let catalogue_id =
            listening_model_named(key, value).map(|model| serde_json::Value::from(model.id));
        let value = catalogue_id.as_ref().unwrap_or(value);
        let toml_value = value
            .serialize(toml_edit::ser::ValueSerializer::new())
            .map_err(|error| BansheeError::Rejected(format!("'{key}': {error}")))?;
        let mut item = toml_edit::value(toml_value);
        // An insert replaces the key too, and the comment above a line is the key's
        if let Some(replaced_key) = table.key(field).cloned() {
            if let (Some(replaced), Some(value)) = (
                table.get(field).and_then(toml_edit::Item::as_value),
                item.as_value_mut(),
            ) {
                *value.decor_mut() = replaced.decor().clone();
            }
            table.insert_formatted(&replaced_key, item);
        } else {
            table.insert(field, item);
        }
    }

    let rendered = document.to_string();
    // `Config`'s types and `deny_unknown_fields` are the only definition of a legal setting
    let validated: Config = Config::parse(&rendered).map_err(|error| match error {
        // The caller wrote this document, so a value its types refuse is input
        BansheeError::Toml(toml) => BansheeError::Rejected(toml.to_string()),
        refused => refused,
    })?;
    Ok((rendered, validated))
}

/// Only a write that sets a side's server, or turns the side on, is refused:
/// any other write beside an old URL still goes through.
fn refuse_plain_http(config: &Config, assignments: &Assignments) -> Result<(), BansheeError> {
    let sides = [
        (
            RemoteKey::Stt,
            config.stt.provider,
            &config.stt.remote.base_url,
        ),
        (
            RemoteKey::Tts,
            config.tts.provider,
            &config.tts.remote.base_url,
        ),
    ];
    for (side, provider, base_url) in sides {
        let turned_on = provider.is_remote() && assignments.contains_key(side.provider_setting());
        if turned_on || assignments.contains_key(side.base_url_setting()) {
            crate::config::refuse_plain_http_to_the_internet(side, base_url)
                .map_err(BansheeError::Rejected)?;
        }
    }
    Ok(())
}

/// The keys go to the credentials file and never into the TOML, so they leave
/// the map before `edit` sees it. Ordered by side, so a caller writes them in a
/// fixed order.
fn take_api_keys(assignments: &mut Assignments) -> Result<Vec<(RemoteKey, String)>, BansheeError> {
    let mut taken = Vec::new();
    for side in [RemoteKey::Stt, RemoteKey::Tts] {
        match assignments.remove(side.setting()) {
            None => {}
            Some(serde_json::Value::String(key)) => taken.push((side, key)),
            Some(_) => {
                return Err(BansheeError::Rejected(format!(
                    "'{}' takes the key as a string",
                    side.setting()
                )));
            }
        }
    }
    Ok(taken)
}

/// A key that decides what leaves this machine is never held only in memory,
/// so the file always names where Banshee sends.
fn refuse_sending_without_persist(
    assignments: &Assignments,
    persist: bool,
) -> Result<(), BansheeError> {
    if persist {
        return Ok(());
    }
    let sending = assignments
        .keys()
        .find(|key| banshee_common::settings::find(key).is_some_and(|setting| setting.sends_out));
    match sending {
        Some(key) => Err(BansheeError::Rejected(format!(
            "'{key}' decides what leaves this machine, so it needs persist: true"
        ))),
        None => Ok(()),
    }
}

/// Asks again for the live settings that refused while their file was missing,
/// and loads the listening model a pipeline stands without once its file is
/// here. The listening model and the voice both answer no while their file is
/// absent, and a download is the only thing that changes that answer.
pub fn reapply_pending(state: &DaemonState) {
    // The same lock `configure` holds: a write landing between the snapshot and
    // `record_outcome` would be applied from the older config and struck off,
    // leaving the daemon on the old setting with nothing marked waiting. A
    // download runs on its own task, so this is never re-entrant.
    let _writing = writing();

    let config = state.config();
    let waiting: Vec<String> = state.pending();
    let outcome = apply_each(state, &config, waiting.iter());
    // A key that still refuses is left where it was, not moved or cleared, and
    // a key with no live apply is not this function's to answer for.
    state.record_outcome(&outcome.applied, &[]);
    // A waiting `stt.model` reconciled above
    if !waiting.iter().any(|key| key == "stt.model") {
        reconcile_waiting(state, &config);
    }
}

fn apply_each<'a>(
    state: &DaemonState,
    config: &Config,
    keys: impl Iterator<Item = &'a String>,
) -> Outcome {
    outcome_of(keys, |variant| apply(variant, state, config))
}

/// A variant applies once however many of its keys one call carries:
/// `audio.hotkey` beside `audio.hotkey_mode` names `Live::Hotkey` twice.
fn outcome_of<'a>(
    keys: impl Iterator<Item = &'a String>,
    mut apply: impl FnMut(Live) -> bool,
) -> Outcome {
    let mut outcome = Outcome::default();
    let mut done: BTreeMap<Live, bool> = BTreeMap::new();
    for key in keys {
        match live(key) {
            Some(variant) => {
                let honoured = *done.entry(variant).or_insert_with(|| apply(variant));
                if honoured {
                    outcome.applied.push(key.clone());
                } else {
                    outcome.waiting.push(key.clone());
                }
            }
            // Unreachable: `edit` refuses a key with no row, and every row
            // but a secret has a `Live`
            None => outcome.waiting.push(key.clone()),
        }
    }
    outcome
}

pub fn not_a_setting(key: &str) -> BansheeError {
    BansheeError::Rejected(format!("'{key}' is not a setting"))
}

/// Pass no `state` when no daemon is running: nothing to apply live, and no
/// second writer to race with. A running daemon writes the config it names.
pub fn configure(
    state: Option<&DaemonState>,
    assignments: Assignments,
    persist: bool,
) -> Result<Outcome, BansheeError> {
    match state {
        Some(state) => configure_at(
            state.config_path(),
            state.credentials_path(),
            Some(state),
            assignments,
            persist,
        ),
        None => configure_at(
            &Config::path()?,
            &crate::credentials::Credentials::path()?,
            None,
            assignments,
            persist,
        ),
    }
}

/// `path` is the config.toml the write reads and, with `persist`, replaces.
/// `credentials` is the key file a key write replaces.
fn configure_at(
    path: &std::path::Path,
    credentials: &std::path::Path,
    state: Option<&DaemonState>,
    mut assignments: Assignments,
    persist: bool,
) -> Result<Outcome, BansheeError> {
    refuse_sending_without_persist(&assignments, persist)?;

    let keys = take_api_keys(&mut assignments)?;
    let keys = (!keys.is_empty()).then_some(KeyWrite {
        file: credentials,
        keys,
    });

    let writing = writing();
    let existing = Config::read(path)?;
    write_locked(&writing, path, &existing, state, assignments, keys, persist)
}

/// The keys one write stores, and the key file they go to.
struct KeyWrite<'a> {
    file: &'a std::path::Path,
    keys: Vec<(RemoteKey, String)>,
}

/// The rest of a checked write, with `existing` read under the guard. The
/// guard proves the caller holds `WRITING`, which does not re-enter. No `keys`
/// is a write that carries no key.
fn write_locked(
    _writing: &std::sync::MutexGuard<'_, ()>,
    path: &std::path::Path,
    existing: &str,
    state: Option<&DaemonState>,
    assignments: Assignments,
    keys: Option<KeyWrite>,
    persist: bool,
) -> Result<Outcome, BansheeError> {
    let (rendered, config) = edit(existing, &assignments)?;
    refuse_plain_http(&config, &assignments)?;

    // Last of the checks, first of the writes: a key stored before `edit`
    // refused a value beside it would be on disk under an error the caller
    // reads as "nothing was stored"
    // Each side written, and whether its key was removed
    let written: Vec<(RemoteKey, bool)> = match keys {
        Some(KeyWrite { file, keys }) => {
            let held: Vec<_> = keys
                .iter()
                .map(|(side, key)| (*side, key.as_str()))
                .collect();
            crate::credentials::Credentials::set_many_at(file, &held)?;
            keys.into_iter()
                .map(|(side, key)| (side, key.trim().is_empty()))
                .collect()
        }
        None => Vec::new(),
    };

    // The key alone changes nothing in config.toml, so there is nothing to write
    if persist && !assignments.is_empty() {
        banshee_common::utils::write_atomically(path, rendered.as_bytes(), None)?;
    }

    // Saved before the applies: the listener checks a switch against it
    let config = std::sync::Arc::new(config);
    if persist && let Some(state) = state {
        state.set_config(std::sync::Arc::clone(&config));
    }
    let speaker_before = state.map(DaemonState::speaker_spec);
    let mut outcome = match state {
        Some(state) => apply_each(state, &config, assignments.keys()),
        // With no daemon running nothing applies, so every key waits for one
        None => Outcome {
            applied: Vec::new(),
            waiting: assignments.keys().cloned().collect(),
        },
    };
    for (side, removed) in written {
        match (side, state) {
            // Each request reads the key file, so the next dictation sends this key
            (RemoteKey::Stt, Some(state)) => {
                state.listener_key_changed();
                if removed && state.listener_route().is_remote() {
                    state.withdraw_listener(None);
                }
                outcome.applied.push(side.setting().to_string());
            }
            // A speaker built in this write already read the key on file, and
            // the local voice reads none
            (RemoteKey::Tts, Some(state)) => {
                let in_force = state.speaker_spec();
                let local = !config.tts.provider.is_remote() && !in_force.provider.is_remote();
                let rebuilt = local
                    || Some(&in_force) != speaker_before.as_ref()
                    || state.switch_speaker(
                        &config.tts,
                        cuts_off(state, &SpeakerSpec::of(&config.tts)),
                    );
                if rebuilt {
                    outcome.applied.push(side.setting().to_string());
                } else {
                    outcome.waiting.push(side.setting().to_string());
                }
            }
            (_, None) => outcome.waiting.push(side.setting().to_string()),
        }
    }

    if let Some(state) = state {
        state.record_outcome(&outcome.applied, &outcome.waiting);
    }

    Ok(outcome)
}

/// A file chose its feedback when it sets `feedback.mode`.
#[cfg(target_os = "macos")]
fn chose_feedback(text: &str) -> Result<bool, BansheeError> {
    let document: DocumentMut = text
        .parse()
        .map_err(|error| BansheeError::Other(format!("config.toml does not parse: {error}")))?;
    Ok(document
        .get("feedback")
        .and_then(|feedback| feedback.get("mode"))
        .is_some())
}

/// True when VoiceOver is running. Off macOS this build never runs the
/// accessibility API it reads, so it answers false.
///
/// Not `NSWorkspace::isVoiceOverEnabled`: that reads false inside a launchd
/// agent with VoiceOver on.
#[cfg(target_os = "macos")]
pub fn voiceover_on() -> bool {
    use objc2_core_foundation::{
        CFPreferencesAppSynchronize, CFPreferencesGetAppBooleanValue, CFString,
    };

    let domain = CFString::from_static_str("com.apple.universalaccess");
    CFPreferencesAppSynchronize(&domain);
    let key = CFString::from_static_str("voiceOverOnOffKey");
    // SAFETY: a null pointer means the caller does not ask whether the key
    // exists; a missing or malformed key then answers false.
    unsafe { CFPreferencesGetAppBooleanValue(&key, &domain, std::ptr::null_mut()) }
}

#[cfg(not(target_os = "macos"))]
pub fn voiceover_on() -> bool {
    false
}

/// A fetch into a models folder that holds none of the wanted files, with a
/// config at `path` that never chose its feedback, is a new install, so it
/// writes `visual` there. VoiceOver users start on `both`, because nothing is
/// announced while the microphone waits for an answer. Answers the mode it wrote.
///
/// The chip only draws on macOS, so off macOS this leaves the key absent
/// rather than writing a mode with no figure to show it.
#[cfg(target_os = "macos")]
pub fn write_first_feedback(
    path: &std::path::Path,
    state: Option<&DaemonState>,
    wanted: &[Download],
    missing: &[Download],
    voiceover: impl FnOnce() -> bool,
) -> Result<Option<FeedbackMode>, BansheeError> {
    if wanted.is_empty() || missing.len() != wanted.len() {
        return Ok(None);
    }
    // The VoiceOver read takes seconds in some processes, so it runs only for a
    // file that has not chosen, and outside the lock. The check under the lock
    // decides.
    if chose_feedback(&Config::read(path)?)? {
        return Ok(None);
    }
    let mode = if voiceover() {
        FeedbackMode::Both
    } else {
        FeedbackMode::Visual
    };
    let writing = writing();
    let existing = Config::read(path)?;
    if chose_feedback(&existing)? {
        return Ok(None);
    }
    let assignments = [("feedback.mode".to_string(), mode.word().into())].into();
    write_locked(&writing, path, &existing, state, assignments, None, true)?;
    Ok(Some(mode))
}

#[cfg(not(target_os = "macos"))]
pub fn write_first_feedback(
    _path: &std::path::Path,
    _state: Option<&DaemonState>,
    _wanted: &[Download],
    _missing: &[Download],
    _voiceover: impl FnOnce() -> bool,
) -> Result<Option<FeedbackMode>, BansheeError> {
    Ok(None)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod table_tests;

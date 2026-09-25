use std::collections::BTreeMap;
use std::sync::Mutex;

use banshee_common::error::BansheeError;
use serde::Serialize;
use toml_edit::DocumentMut;

use crate::config::{Config, FeedbackMode};
use crate::credentials::RemoteKey;
use crate::models::download::Download;
use crate::state::DaemonState;

/// Dotted `section.field` keys, spelled as `config.toml` spells them.
pub type Assignments = BTreeMap<String, serde_json::Value>;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Live {
    VadThreshold,
    InputDevice,
    BargeIn,
    Feedback,
    SaveHistory,
    Tts,
    Vocabulary,
    Preset,
    Speech,
}

fn live(key: &str) -> Option<Live> {
    match key {
        "stt.vad_threshold" => Some(Live::VadThreshold),
        "audio.input_device" => Some(Live::InputDevice),
        "audio.barge_in" => Some(Live::BargeIn),
        "feedback.mode" | "audio.cues.enabled" => Some(Live::Feedback),
        "daemon.save_history" => Some(Live::SaveHistory),
        // `tts.fallback` is not here: it decides what to do when Kokoro will
        // not load, which is settled once, at startup.
        "tts.voice" | "tts.speed" => Some(Live::Tts),
        "stt.vocabulary" => Some(Live::Vocabulary),
        // Whisper reads both per utterance, so neither moves the model.
        "stt.language" | "stt.translate" => Some(Live::Speech),
        "stt.preset" => Some(Live::Preset),
        _ => None,
    }
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
        // The watchdog reads this on its next tick and rebinds capture
        Live::InputDevice => {
            state.set_wanted_device(config.audio.input_device.clone());
            true
        }
        // Read at every record start, so the next dictation obeys it
        Live::BargeIn => {
            state.set_barge_in(config.audio.barge_in);
            true
        }
        // The next cue reads this, so the next dictation obeys it
        Live::Feedback => {
            state.set_feedback_mode(config.feedback_mode());
            true
        }
        // Opening the file is the whole of the setting, so a failure to open
        // one leaves the key unapplied rather than half applied
        Live::SaveHistory => match history_for(config) {
            Ok(connection) => {
                state.set_history(connection);
                true
            }
            Err(error) => {
                log::error!("Failed to open the history file: {error}");
                false
            }
        },
        // The next utterance reads both, so neither reloads the model. The
        // system fallback takes neither, and says so.
        Live::Tts => state.set_tts(&config.tts),
        // A listener that has gone leaves the words unread, so say so rather
        // than report a prompt nothing holds
        Live::Vocabulary => state.set_vocabulary(config.stt.vocabulary.clone()),
        Live::Preset => apply_preset(state, config),
        Live::Speech => state.set_speech((&config.stt).into()),
    }
}

/// Nothing loads when the model is already behind the engine or still to download; a download is
/// minutes, so the key stays unapplied. A remote listener loads no file, so nothing has to load.
fn apply_preset(state: &DaemonState, config: &Config) -> bool {
    let Some(model) = crate::models::stt_file(config) else {
        return true;
    };
    if state.stt_model() == Some(model) {
        return true;
    }
    let absent = crate::models::missing_in(state.models_dir(), &[model]);
    if !absent.is_empty() {
        log::warn!("{model} is not downloaded yet, so the preset is unchanged");
        return false;
    }
    // The engine that loads this starts on the pending restart, and a listener
    // that holds no file holds no engine to load into.
    if state.stt_model().is_none() {
        return true;
    }
    state.load_stt_model(config.stt.preset)
}

fn history_for(config: &Config) -> Result<Option<rusqlite::Connection>, BansheeError> {
    if !config.daemon.save_history {
        return Ok(None);
    }
    crate::history::open().map(Some)
}

/// The daemon serves a task per connection, so two calls can otherwise read
/// the same file before either writes and one setting is lost.
static WRITING: Mutex<()> = Mutex::new(());

#[derive(Default, Debug)]
pub struct Outcome {
    pub applied: Vec<String>,
    pub restart_required: Vec<String>,
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
        // `[audio.cues]` is a section two deep, so only the last segment is a field
        let (path, field) = key.rsplit_once('.').ok_or_else(|| {
            BansheeError::Rejected(format!("'{key}' must name a section, as in stt.language"))
        })?;
        if value.is_null() {
            if value_at(&Config::default(), key).is_none_or(|default| default.is_object()) {
                return Err(BansheeError::Rejected(format!("'{key}' is not a setting")));
            }
            if let Some(section) = table_along(document.as_table_mut(), path)? {
                section.remove(field);
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

/// A code the engine cannot read is refused here, where a person is there to
/// see why. `Config` reads the same field liberally, because a file written
/// before anything read it can hold anything.
fn refuse_unknown_language(assignments: &Assignments) -> Result<(), BansheeError> {
    match assignments
        .get("stt.language")
        .and_then(|value| value.as_str())
    {
        Some(value) if !crate::config::known_language(value) => {
            Err(BansheeError::Rejected(format!(
                "'{value}' is not a language Whisper knows. Use a code like en, de or hi, or auto"
            )))
        }
        _ => Ok(()),
    }
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

/// Applying one of these without writing it would report success and change nothing.
fn startup_only(assignments: &Assignments) -> Option<&String> {
    assignments.keys().find(|key| live(key).is_none())
}

/// Asks again for the live settings that refused while their file was missing.
/// The preset and the voice both answer no while their model is absent, and a
/// download is the only thing that changes that answer.
pub fn reapply_pending(state: &DaemonState) {
    // The same lock `configure` holds: a write landing between the snapshot and
    // `record_outcome` would be applied from the older config and struck off,
    // leaving the daemon on the old setting with nothing marked waiting. A
    // download runs on its own task, so this is never re-entrant.
    let _writing = WRITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let config = state.config();
    let waiting: Vec<String> = state.pending();
    let outcome = apply_each(state, &config, waiting.iter());
    // A key that still refuses is left where it was, not moved or cleared, and
    // a key with no live apply is not this function's to answer for.
    state.record_outcome(&outcome.applied, &[]);
}

/// The value a dotted key names, read off a serialized `Config` so both sides
/// of a comparison are spelled by the same serializer. `None` for a key no
/// config field answers to.
fn value_at(config: &Config, key: &str) -> Option<serde_json::Value> {
    let mut node = serde_json::to_value(config).ok()?;
    for segment in key.split('.') {
        node = node.get(segment)?.clone();
    }
    Some(node)
}

/// True when the daemon already runs the value written; a key changed and changed back has nothing
/// to wait for.
fn in_effect(key: &str, state: &DaemonState, config: &Config) -> bool {
    let running = state.running_config();
    let was = value_at(&running, key);
    was.is_some() && was == value_at(config, key)
}

/// A variant applies once however many of its keys one call carries: without
/// the memo, `tts.voice` and `tts.speed` together would reconfigure the backend
/// twice with the same pair.
fn apply_each<'a>(
    state: &DaemonState,
    config: &Config,
    keys: impl Iterator<Item = &'a String>,
) -> Outcome {
    let mut outcome = Outcome::default();
    let mut done: BTreeMap<Live, bool> = BTreeMap::new();
    for key in keys {
        match live(key) {
            Some(variant) => {
                let honoured = *done
                    .entry(variant)
                    .or_insert_with(|| apply(variant, state, config));
                if honoured {
                    outcome.applied.push(key.clone());
                } else {
                    outcome.restart_required.push(key.clone());
                }
            }
            None if in_effect(key, state, config) => outcome.applied.push(key.clone()),
            None => outcome.restart_required.push(key.clone()),
        }
    }
    outcome
}

// A null write passes, because it is the only way `config set` removes the key.
fn refuse_retired_keys(assignments: &Assignments) -> Result<(), BansheeError> {
    if assignments
        .get("audio.cues.enabled")
        .is_some_and(|value| !value.is_null())
    {
        return Err(BansheeError::Rejected(
            "'audio.cues.enabled' is replaced by feedback.mode: set it to visual, sound, both or none"
                .to_string(),
        ));
    }
    Ok(())
}

/// Pass no `state` when no daemon is running: nothing to apply live, and no
/// second writer to race with. A running daemon writes the config it names.
pub fn configure(
    state: Option<&DaemonState>,
    assignments: Assignments,
    persist: bool,
) -> Result<Outcome, BansheeError> {
    match state {
        Some(state) => configure_at(state.config_path(), Some(state), assignments, persist),
        None => configure_at(&Config::path()?, None, assignments, persist),
    }
}

/// `path` is the config.toml the write reads and, with `persist`, replaces.
fn configure_at(
    path: &std::path::Path,
    state: Option<&DaemonState>,
    mut assignments: Assignments,
    persist: bool,
) -> Result<Outcome, BansheeError> {
    refuse_retired_keys(&assignments)?;
    refuse_unknown_language(&assignments)?;

    if !persist && let Some(key) = startup_only(&assignments) {
        return Err(BansheeError::Rejected(format!(
            "'{key}' is read when the daemon starts, so it needs persist: true"
        )));
    }

    let api_keys = take_api_keys(&mut assignments)?;

    let writing = WRITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let existing = Config::read(path)?;
    write_locked(
        &writing,
        path,
        &existing,
        state,
        assignments,
        api_keys,
        persist,
    )
}

/// The rest of a checked write, with `existing` read under the guard. The
/// guard proves the caller holds `WRITING`, which does not re-enter.
fn write_locked(
    _writing: &std::sync::MutexGuard<'_, ()>,
    path: &std::path::Path,
    existing: &str,
    state: Option<&DaemonState>,
    assignments: Assignments,
    api_keys: Vec<(RemoteKey, String)>,
    persist: bool,
) -> Result<Outcome, BansheeError> {
    let (rendered, config) = edit(existing, &assignments)?;

    // Last of the checks, first of the writes: a key stored before `edit`
    // refused a value beside it would be on disk under an error the caller
    // reads as "nothing was stored"
    if !api_keys.is_empty() {
        let keys: Vec<_> = api_keys
            .iter()
            .map(|(side, key)| (*side, key.as_str()))
            .collect();
        crate::credentials::Credentials::set_many(&keys)?;
    }

    // The key alone changes nothing in config.toml, so there is nothing to write
    if persist && !assignments.is_empty() {
        banshee_common::utils::write_atomically(path, rendered.as_bytes(), None)?;
    }

    // A live key needs a restart too when no daemon runs, so with no state
    // every key is one.
    let mut outcome = match state {
        Some(state) => apply_each(state, &config, assignments.keys()),
        None => Outcome {
            applied: Vec::new(),
            restart_required: assignments.keys().cloned().collect(),
        },
    };
    for (side, _) in &api_keys {
        outcome.restart_required.push(side.setting().to_string());
    }

    if let Some(state) = state {
        state.record_outcome(&outcome.applied, &outcome.restart_required);
        if persist {
            state.set_config(std::sync::Arc::new(config));
        }
    }

    Ok(outcome)
}

/// A file chose its feedback when it sets `feedback.mode`, or turns the old
/// `audio.cues.enabled` switch off.
#[cfg(target_os = "macos")]
fn chose_feedback(text: &str) -> Result<bool, BansheeError> {
    let document: DocumentMut = text
        .parse()
        .map_err(|error| BansheeError::Other(format!("config.toml does not parse: {error}")))?;
    let mode = document
        .get("feedback")
        .and_then(|feedback| feedback.get("mode"))
        .is_some();
    let cues = document
        .get("audio")
        .and_then(|audio| audio.get("cues"))
        .and_then(|cues| cues.get("enabled"))
        .and_then(toml_edit::Item::as_bool);
    Ok(mode || cues == Some(false))
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
    let writing = WRITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let existing = Config::read(path)?;
    if chose_feedback(&existing)? {
        return Ok(None);
    }
    let assignments = [("feedback.mode".to_string(), mode.word().into())].into();
    write_locked(
        &writing,
        path,
        &existing,
        state,
        assignments,
        Vec::new(),
        true,
    )?;
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

use std::collections::BTreeMap;
use std::sync::Mutex;

use banshee_common::error::BansheeError;
use serde::Serialize;
use toml_edit::DocumentMut;

use crate::config::Config;
use crate::state::DaemonState;

/// Dotted `section.field` keys, spelled as `config.toml` spells them.
pub type Assignments = BTreeMap<String, serde_json::Value>;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Live {
    VadThreshold,
    InputDevice,
    BargeIn,
    Cues,
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
        "audio.cues.enabled" => Some(Live::Cues),
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
        // The player reads this as each cue reaches it
        Live::Cues => {
            state.set_cues_enabled(config.audio.cues.enabled);
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
                eprintln!("Failed to open the history file: {error}");
                false
            }
        },
        // The next utterance reads both, so neither reloads the model. The
        // system fallback takes neither, and says so.
        Live::Tts => state.set_tts(&config.tts),
        // A listener that has gone leaves the words unread, so say so rather
        // than report a prompt nothing holds
        Live::Vocabulary => state.set_vocabulary(config.stt.vocabulary.clone()),
        Live::Preset => apply_preset(state, config.stt.preset.model_name()),
        Live::Speech => state.set_speech((&config.stt).into()),
    }
}

/// Nothing loads when the model is already behind the engine or still to download; a download is
/// minutes, so the key stays unapplied.
fn apply_preset(state: &DaemonState, model: &'static str) -> bool {
    if state.stt_model() == model {
        return true;
    }
    let absent = crate::models::missing(&[model]);
    if !absent.is_empty() {
        eprintln!("banshee: {model} is not downloaded yet, so the preset is unchanged");
        return false;
    }
    state.load_stt_model(model)
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
    let validated: Config =
        toml::from_str(&rendered).map_err(|error| BansheeError::Rejected(error.to_string()))?;
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

/// Pass no `state` when no daemon is running: nothing to apply live, and no
/// second writer to race with.
pub fn configure(
    state: Option<&DaemonState>,
    assignments: &Assignments,
    persist: bool,
) -> Result<Outcome, BansheeError> {
    if !persist && let Some(key) = startup_only(assignments) {
        return Err(BansheeError::Rejected(format!(
            "'{key}' is read when the daemon starts, so it needs persist: true"
        )));
    }

    refuse_unknown_language(assignments)?;

    let _writing = WRITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let path = Config::path()?;
    let existing = Config::read(&path)?;

    let (rendered, config) = edit(&existing, assignments)?;

    if persist {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // A partial write would truncate a file the user hand-edits, and a shared
        // staging name would let two processes interleave their bytes
        let staged = path.with_extension(format!("toml.{}", std::process::id()));
        std::fs::write(&staged, &rendered)?;
        std::fs::rename(&staged, &path)?;
    }

    // A live key needs a restart too when no daemon runs, so with no state
    // every key is one.
    let outcome = match state {
        Some(state) => apply_each(state, &config, assignments.keys()),
        None => Outcome {
            applied: Vec::new(),
            restart_required: assignments.keys().cloned().collect(),
        },
    };

    if let Some(state) = state {
        state.record_outcome(&outcome.applied, &outcome.restart_required);
        if persist {
            state.set_config(std::sync::Arc::new(config));
        }
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests;

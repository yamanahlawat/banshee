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

/// Loads a different Whisper model, unless one is already behind the engine or
/// the file is still to download. A model still to download is minutes, not the
/// seconds a swap takes, so this leaves it unapplied rather than promise a
/// preset nothing loaded.
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

/// The history file the config now asks for, or `None` when it asks for none.
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
mod tests {
    use super::{Assignments, edit, startup_only};

    fn assignments(pairs: &[(&str, serde_json::Value)]) -> Assignments {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn a_write_keeps_the_comments_around_it() {
        let existing = "# how strict speech detection is\n[stt]\nvad_threshold = 0.5 # tune me\nlanguage = \"en\"\n";
        let (rendered, config) =
            edit(existing, &assignments(&[("stt.vad_threshold", 0.8.into())])).unwrap();

        assert!(
            rendered.contains("# how strict speech detection is"),
            "the leading comment must survive: {rendered}"
        );
        assert!(
            rendered.contains("# tune me"),
            "the trailing comment must survive: {rendered}"
        );
        assert!(
            rendered.contains("language = \"en\""),
            "an untouched key must survive: {rendered}"
        );
        assert_eq!(config.stt.vad_threshold, 0.8);
    }

    #[test]
    fn a_write_keeps_the_comment_above_the_key_it_changes() {
        let existing = "[stt]\nvad_threshold = 0.5\n\n# the language spoken\nlanguage = \"en\"\n\ntranslate = false\n";
        let (rendered, _) = edit(existing, &assignments(&[("stt.language", "de".into())])).unwrap();

        assert!(
            rendered.contains("# the language spoken\nlanguage = \"de\""),
            "the comment above the key must stay above it: {rendered}"
        );
        assert!(
            rendered.contains("\n\n# the language spoken"),
            "the blank line separating it must survive: {rendered}"
        );
    }

    #[test]
    fn a_setting_two_sections_deep_is_reachable() {
        let (rendered, config) =
            edit("", &assignments(&[("audio.cues.enabled", false.into())])).unwrap();
        assert!(
            rendered.contains("[audio.cues]"),
            "the key must land under its own section, not quoted under [audio]: {rendered}"
        );
        assert!(
            !rendered.contains("[audio]"),
            "a parent invented only to hold the subtable needs no header: {rendered}"
        );
        assert!(!config.audio.cues.enabled);
    }

    // An [audio] that holds keys of its own keeps its header either way, so the
    // section here is empty: only then does suppressing it lose the comment.
    #[test]
    fn a_nested_write_keeps_a_section_that_was_already_written() {
        let existing = "# audio settings, see docs\n[audio]\n\n[stt]\nvad_threshold = 0.5\n";
        let (rendered, _) = edit(
            existing,
            &assignments(&[("audio.cues.enabled", false.into())]),
        )
        .unwrap();
        assert!(
            rendered.contains("# audio settings, see docs"),
            "the comment above the section must survive: {rendered}"
        );
        assert!(
            rendered.contains("[audio]"),
            "a section the user wrote must keep its header: {rendered}"
        );
        assert!(rendered.contains("[audio.cues]"), "{rendered}");
    }

    /// Whisper reads the language and the task per utterance, so a write moves
    /// the next dictation and no model reloads.
    #[test]
    fn the_spoken_language_applies_without_a_restart() {
        assert_eq!(
            startup_only(&assignments(&[("stt.language", "de".into())])),
            None
        );
        assert_eq!(
            startup_only(&assignments(&[("stt.translate", true.into())])),
            None
        );
    }

    /// A code the engine does not know is refused where a person is there to
    /// read why. The config's own reader falls back to English instead, so a
    /// file written before the field was read cannot stop the daemon.
    #[test]
    fn a_language_the_engine_does_not_know_is_refused_at_the_boundary() {
        let error = super::configure(
            None,
            &assignments(&[("stt.language", "klingon".into())]),
            false,
        )
        .expect_err("an unknown code must not be written");
        assert!(error.to_string().contains("klingon"), "{error}");
        assert!(error.to_string().contains("auto"), "{error}");
    }

    #[test]
    fn a_language_the_engine_knows_is_written() {
        super::configure(None, &assignments(&[("stt.language", "de".into())]), false)
            .expect("de is a language whisper knows");
    }

    #[test]
    fn a_startup_setting_needs_a_write_to_mean_anything() {
        assert_eq!(
            startup_only(&assignments(&[("audio.hotkey", "F6".into())])),
            Some(&"audio.hotkey".to_string())
        );
        assert_eq!(
            startup_only(&assignments(&[("stt.vad_threshold", 0.6.into())])),
            None,
            "the daemon rereads this one, so applying it without a write does change something"
        );
    }

    // The weaker sibling tests only ask whether a key is in the live set. This
    // one asks whether the write reached the daemon that is running.
    #[test]
    fn a_barge_in_write_reaches_the_running_daemon() {
        let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
        assert!(matches!(state.barge_in(), crate::config::BargeInMode::Stop));

        let outcome = super::configure(
            Some(&state),
            &assignments(&[("audio.barge_in", "none".into())]),
            false,
        )
        .expect("a known key and a legal value must apply");

        assert!(
            matches!(state.barge_in(), crate::config::BargeInMode::None),
            "the next dictation must obey the new value, not the old one"
        );
        assert_eq!(outcome.applied, vec!["audio.barge_in".to_string()]);
        assert!(outcome.restart_required.is_empty());
    }

    #[test]
    fn a_cues_write_reaches_the_running_daemon() {
        let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
        assert!(!state.cues_enabled());

        let outcome = super::configure(
            Some(&state),
            &assignments(&[("audio.cues.enabled", true.into())]),
            false,
        )
        .expect("a known key and a legal value must apply");

        assert!(
            state.cues_enabled(),
            "the next cue must be heard, without a restart"
        );
        assert_eq!(outcome.applied, vec!["audio.cues.enabled".to_string()]);
        assert!(outcome.restart_required.is_empty());
    }

    // Only the off direction runs here: turning history on opens the real file
    // under the home directory, which a test must not create. The on direction
    // is verified against a running daemon.
    #[test]
    fn a_save_history_write_reaches_the_running_daemon() {
        let state = crate::test_support::daemon_state_with_history(&["one"]);
        assert!(state.history_enabled());

        let outcome = super::configure(
            Some(&state),
            &assignments(&[("daemon.save_history", false.into())]),
            false,
        )
        .expect("a known key and a legal value must apply");

        assert!(
            !state.history_enabled(),
            "the next dictation must not be kept, without a restart"
        );
        assert_eq!(outcome.applied, vec!["daemon.save_history".to_string()]);
        assert!(outcome.restart_required.is_empty());
    }

    #[test]
    fn a_voice_write_reaches_the_running_daemon() {
        let (state, spoken) = crate::test_support::daemon_state_recording_tts();
        assert_eq!(state.tts_voice(), None);

        let outcome = super::configure(
            Some(&state),
            &assignments(&[("tts.voice", "am_adam".into())]),
            false,
        )
        .expect("a known key and a legal value must apply");

        assert_eq!(
            spoken
                .lock()
                .unwrap()
                .last()
                .map(|(voice, _)| voice.clone()),
            Some("am_adam".to_string()),
            "the next utterance must be spoken in the new voice"
        );
        assert_eq!(
            state.tts_voice().as_deref(),
            Some("am_adam"),
            "the window marks the current voice from this"
        );
        assert_eq!(outcome.applied, vec!["tts.voice".to_string()]);
        assert!(outcome.restart_required.is_empty());
    }

    #[test]
    fn a_speed_write_reaches_the_running_daemon() {
        let (state, spoken) = crate::test_support::daemon_state_recording_tts();

        let outcome = super::configure(
            Some(&state),
            &assignments(&[("tts.speed", 1.5.into())]),
            false,
        )
        .expect("a known key and a legal value must apply");

        assert_eq!(
            spoken.lock().unwrap().last().map(|(_, speed)| *speed),
            Some(1.5),
            "the next utterance must be spoken at the new rate"
        );
        assert_eq!(outcome.applied, vec!["tts.speed".to_string()]);
    }

    // A backend that cannot take the voice must not be reported as speaking in
    // it, or the window marks a voice nothing will ever use.
    #[test]
    fn a_backend_that_refuses_the_voice_is_not_reported_as_applied() {
        let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);

        let outcome = super::configure(
            Some(&state),
            &assignments(&[("tts.voice", "am_adam".into())]),
            false,
        )
        .expect("a known key and a legal value must reach the backend");

        assert!(
            outcome.applied.is_empty(),
            "the null backend takes no voice"
        );
        assert_eq!(outcome.restart_required, vec!["tts.voice".to_string()]);
        assert_eq!(
            state.tts_voice(),
            None,
            "no voice is in use, so none is named"
        );
    }

    #[test]
    fn the_voice_and_the_speed_together_reach_the_backend_once() {
        let (state, spoken) = crate::test_support::daemon_state_recording_tts();

        let outcome = super::configure(
            Some(&state),
            &assignments(&[("tts.voice", "am_adam".into()), ("tts.speed", 1.5.into())]),
            false,
        )
        .expect("two known keys and legal values must apply");

        assert_eq!(
            spoken.lock().unwrap().len(),
            1,
            "one write, one reconfigure"
        );
        assert_eq!(outcome.applied.len(), 2, "both keys are in effect");
    }

    #[test]
    fn a_vocabulary_write_reaches_the_listener_that_holds_the_engine() {
        let (commands, taken) = std::sync::mpsc::channel();
        let state = crate::test_support::daemon_state(commands);

        let outcome = super::configure(
            Some(&state),
            &assignments(&[("stt.vocabulary", vec!["banshee", "tokio"].into())]),
            false,
        )
        .expect("a known key and a legal value must apply");

        match taken.try_recv() {
            Ok(crate::state::ConsumerCommand::Retune(prompt)) => {
                assert_eq!(prompt, vec!["banshee".to_string(), "tokio".to_string()]);
            }
            other => panic!("the listener was handed no prompt: {:?}", other.is_ok()),
        }
        assert_eq!(outcome.applied, vec!["stt.vocabulary".to_string()]);
    }

    #[test]
    fn a_preset_write_reaches_the_listener_that_holds_the_engine() {
        let (commands, taken) = std::sync::mpsc::channel();
        let state = crate::test_support::daemon_state(commands);

        let outcome = super::configure(
            Some(&state),
            &assignments(&[("stt.preset", "balanced".into())]),
            false,
        )
        .expect("a known key and a legal value must apply");

        // The model is the one the daemon already runs on a real machine, so
        // this asserts what the arm decided rather than what is on this disk.
        match (taken.try_recv(), outcome.applied.is_empty()) {
            (Ok(crate::state::ConsumerCommand::Reload(model)), false) => {
                assert_eq!(model, "ggml-large-v3-turbo-q5_0.bin");
            }
            (Err(_), true) => {
                assert_eq!(outcome.restart_required, vec!["stt.preset".to_string()]);
            }
            _ => panic!("the arm handed over a model and reported nothing, or the reverse"),
        }
    }

    // Every click on the segmented control writes the key, the active one
    // included. Without the guard each of those costs a multi-second load and
    // holds two models at once.
    #[test]
    fn a_preset_already_behind_the_engine_loads_nothing() {
        let (commands, taken) = std::sync::mpsc::channel();
        let state = crate::test_support::daemon_state(commands);
        state.set_stt_model("ggml-large-v3-turbo-q5_0.bin");

        let outcome = super::configure(
            Some(&state),
            &assignments(&[("stt.preset", "balanced".into())]),
            false,
        )
        .expect("a known key and a legal value must apply");

        assert!(taken.try_recv().is_err(), "the model was already loaded");
        assert_eq!(outcome.applied, vec!["stt.preset".to_string()]);
    }

    #[test]
    fn the_tts_fallback_still_needs_a_restart() {
        assert_eq!(
            startup_only(&assignments(&[("tts.fallback", "system".into())])),
            Some(&"tts.fallback".to_string())
        );
    }

    #[test]
    fn the_input_device_no_longer_needs_a_restart() {
        assert_eq!(
            startup_only(&assignments(&[("audio.input_device", "yeti".into())])),
            None,
            "the watchdog rebuilds capture, so this key applies live now"
        );
    }

    #[test]
    fn both_live_keys_together_still_need_no_restart() {
        assert_eq!(
            startup_only(&assignments(&[
                ("audio.input_device", "yeti".into()),
                ("stt.vad_threshold", 0.6.into()),
            ])),
            None
        );
    }

    #[test]
    fn a_missing_section_is_created() {
        let (rendered, config) =
            edit("", &assignments(&[("tts.voice", "af_bella".into())])).unwrap();
        assert!(rendered.contains("[tts]"), "{rendered}");
        assert_eq!(config.tts.voice, "af_bella");
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let error = edit("", &assignments(&[("stt.nonesuch", 1.into())])).unwrap_err();
        assert!(
            error.to_string().contains("nonesuch"),
            "the error must name the key: {error}"
        );
    }

    #[test]
    fn an_unknown_section_is_refused() {
        let error = edit("", &assignments(&[("nonesuch.voice", "x".into())])).unwrap_err();
        assert!(
            error.to_string().contains("nonesuch"),
            "the error must name the section: {error}"
        );
    }

    #[test]
    fn a_key_without_a_section_is_refused() {
        let error = edit("", &assignments(&[("voice", "af_sky".into())])).unwrap_err();
        assert!(
            error.to_string().contains("stt.language"),
            "the error must show the dotted form: {error}"
        );
    }

    #[test]
    fn a_value_of_the_wrong_type_is_refused() {
        let error = edit("", &assignments(&[("stt.translate", "yes".into())])).unwrap_err();
        assert!(
            error.to_string().contains("translate"),
            "the error must name the key: {error}"
        );
    }

    /// A download is the only thing that turns a refusal into a yes, so a
    /// setting chosen before its file arrives has nothing else to wait for.
    #[test]
    fn a_setting_that_was_waiting_on_a_file_applies_once_the_download_ends() {
        let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
        // `audio.cues.enabled` is live and needs no file, so it stands in for a
        // key whose apply succeeds the moment it is asked again.
        state.record_outcome(&[], &["audio.cues.enabled".to_string()]);
        assert_eq!(state.pending(), vec!["audio.cues.enabled".to_string()]);

        super::reapply_pending(&state);
        assert!(
            state.pending().is_empty(),
            "a key that applies must stop waiting"
        );
    }

    /// A model that is still absent answers no again, and the key keeps its
    /// place rather than being cleared as though it had worked.
    #[test]
    fn a_setting_whose_file_is_still_missing_keeps_waiting() {
        let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
        state.record_outcome(&[], &["stt.preset".to_string()]);

        super::reapply_pending(&state);
        assert_eq!(state.pending(), vec!["stt.preset".to_string()]);
    }

    /// Nothing else in `pending` is a live key, and a restart-only key has no
    /// apply to run.
    #[test]
    fn a_restart_only_key_is_left_alone() {
        let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
        state.record_outcome(&[], &["audio.hotkey".to_string()]);

        super::reapply_pending(&state);
        assert_eq!(state.pending(), vec!["audio.hotkey".to_string()]);
    }

    #[test]
    fn a_key_that_needs_a_restart_becomes_pending_and_a_live_one_does_not() {
        let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
        state.record_outcome(
            &["stt.vad_threshold".to_string()],
            &["audio.cues.enabled".to_string()],
        );
        assert_eq!(state.pending(), vec!["audio.cues.enabled".to_string()]);

        state.record_outcome(&["audio.cues.enabled".to_string()], &[]);
        assert!(state.pending().is_empty());
    }

    #[test]
    fn a_value_outside_its_range_is_refused() {
        let error = edit("", &assignments(&[("stt.vad_threshold", 5.0.into())])).unwrap_err();
        assert!(
            error.to_string().contains("0.0 and 1.0"),
            "the error must state the range: {error}"
        );
    }
}

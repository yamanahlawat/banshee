use std::collections::BTreeMap;
use std::sync::Mutex;

use banshee_common::error::BansheeError;
use serde::Serialize;
use toml_edit::DocumentMut;

use crate::config::Config;
use crate::state::DaemonState;

/// Dotted `section.field` keys, spelled as `config.toml` spells them.
pub type Assignments = BTreeMap<String, serde_json::Value>;

enum Live {
    VadThreshold,
    InputDevice,
    BargeIn,
}

fn live(key: &str) -> Option<Live> {
    match key {
        "stt.vad_threshold" => Some(Live::VadThreshold),
        "audio.input_device" => Some(Live::InputDevice),
        "audio.barge_in" => Some(Live::BargeIn),
        _ => None,
    }
}

/// The daemon serves a task per connection, so two calls can otherwise read
/// the same file before either writes and one setting is lost.
static WRITING: Mutex<()> = Mutex::new(());

#[derive(Default)]
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

/// Applying one of these without writing it would report success and change nothing.
fn startup_only(assignments: &Assignments) -> Option<&String> {
    assignments.keys().find(|key| live(key).is_none())
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

    let mut outcome = Outcome::default();
    for key in assignments.keys() {
        // One arm per `Live` variant and no catch-all over them, so a variant
        // without an arm does not compile
        match (live(key), state) {
            (Some(Live::VadThreshold), Some(state)) => {
                state.set_vad_threshold(config.stt.vad_threshold);
                outcome.applied.push(key.clone());
            }
            // The watchdog reads this on its next tick and rebinds capture
            (Some(Live::InputDevice), Some(state)) => {
                state.set_wanted_device(config.audio.input_device.clone());
                outcome.applied.push(key.clone());
            }
            // Read at every record start, so the next dictation obeys it
            (Some(Live::BargeIn), Some(state)) => {
                state.set_barge_in(config.audio.barge_in);
                outcome.applied.push(key.clone());
            }
            // A live key needs a restart too when no daemon runs
            (Some(Live::VadThreshold) | Some(Live::InputDevice) | Some(Live::BargeIn), None)
            | (None, _) => outcome.restart_required.push(key.clone()),
        }
    }

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

    #[test]
    fn a_startup_setting_needs_a_write_to_mean_anything() {
        assert_eq!(
            startup_only(&assignments(&[("stt.language", "de".into())])),
            Some(&"stt.language".to_string())
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

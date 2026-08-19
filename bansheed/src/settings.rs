use std::collections::BTreeMap;

use banshee_common::{error::BansheeError, utils::get_config_path};
use serde::Serialize;
use toml_edit::DocumentMut;

use crate::config::Config;
use crate::state::DaemonState;

/// Dotted `section.field` keys, spelled as `config.toml` spells them.
pub type Assignments = BTreeMap<String, serde_json::Value>;

/// The only setting the running daemon rereads.
const LIVE: &str = "stt.vad_threshold";

#[derive(Default)]
pub struct Outcome {
    pub applied: Vec<String>,
    pub restart_required: Vec<String>,
}

/// A caller fault, as against the daemon failing to read or write the file.
fn rejected(message: String) -> BansheeError {
    BansheeError::Rpc {
        code: -32602,
        message,
    }
}

/// Edits the document rather than serializing a `Config`, so hand-written
/// comments and layout survive.
fn edit(existing: &str, assignments: &Assignments) -> Result<(String, Config), BansheeError> {
    let mut document: DocumentMut = existing
        .parse()
        .map_err(|error| BansheeError::Other(format!("config.toml does not parse: {error}")))?;

    for (key, value) in assignments {
        // `[audio.cues]` is a section two deep, so only the last segment is a field
        let (path, field) = key
            .rsplit_once('.')
            .ok_or_else(|| rejected(format!("'{key}' must name a section, as in stt.language")))?;
        let mut table = document.as_table_mut();
        for section in path.split('.') {
            table = table
                .entry(section)
                .or_insert(toml_edit::table())
                .as_table_mut()
                .ok_or_else(|| rejected(format!("[{path}] is not a section")))?;
            // A parent written only to hold a subtable needs no header of its own
            table.set_implicit(true);
        }

        let toml_value = value
            .serialize(toml_edit::ser::ValueSerializer::new())
            .map_err(|error| rejected(format!("'{key}': {error}")))?;
        let mut item = toml_edit::value(toml_value);
        // An insert replaces the key too, and the comment above a line is the key's
        match (
            table.key(field).cloned(),
            table.get(field).and_then(toml_edit::Item::as_value),
        ) {
            (Some(replaced_key), replaced_value) => {
                if let (Some(replaced_value), Some(value)) = (replaced_value, item.as_value_mut()) {
                    *value.decor_mut() = replaced_value.decor().clone();
                }
                table.insert_formatted(&replaced_key, item);
            }
            (None, _) => {
                table.insert(field, item);
            }
        }
    }

    let rendered = document.to_string();
    // `Config`'s types and `deny_unknown_fields` are the only definition of a legal setting
    let validated: Config =
        toml::from_str(&rendered).map_err(|error| rejected(error.to_string()))?;
    Ok((rendered, validated))
}

/// Applying one of these without writing it would report success and change nothing.
fn startup_only(assignments: &Assignments) -> Option<&String> {
    assignments.keys().find(|key| key.as_str() != LIVE)
}

/// Pass no `state` when no daemon is running: nothing to apply live, and no
/// second writer to race with.
pub fn configure(
    state: Option<&DaemonState>,
    assignments: &Assignments,
    persist: bool,
) -> Result<Outcome, BansheeError> {
    if !persist && let Some(key) = startup_only(assignments) {
        return Err(rejected(format!(
            "'{key}' is read when the daemon starts, so it needs persist: true"
        )));
    }

    let path = get_config_path()
        .ok_or_else(|| BansheeError::Other("Failed to get config path".to_string()))?;
    let existing = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };

    let (rendered, config) = edit(&existing, assignments)?;

    if persist {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // A partial write would truncate a file the user hand-edits
        let staged = path.with_extension("toml.new");
        std::fs::write(&staged, &rendered)?;
        std::fs::rename(&staged, &path)?;
    }

    let mut outcome = Outcome::default();
    for key in assignments.keys() {
        match (key.as_str(), state) {
            (LIVE, Some(state)) => {
                state.set_vad_threshold(config.stt.vad_threshold);
                outcome.applied.push(key.clone());
            }
            _ => outcome.restart_required.push(key.clone()),
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
        assert!(!config.audio.cues.enabled);
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
    fn a_value_outside_its_range_is_refused() {
        let error = edit("", &assignments(&[("stt.vad_threshold", 5.0.into())])).unwrap_err();
        assert!(
            error.to_string().contains("0.0 and 1.0"),
            "the error must state the range: {error}"
        );
    }
}

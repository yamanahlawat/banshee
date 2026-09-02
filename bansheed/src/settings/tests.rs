use super::{Assignments, edit, startup_only};
use crate::config::Config;

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
    let (rendered, config) = edit("", &assignments(&[("tts.voice", "af_bella".into())])).unwrap();
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

/// The window turns `restart_required` into "it takes effect when Banshee
/// restarts". A binding written with the value the daemon already runs makes
/// that notice untrue.
#[test]
fn a_restart_only_key_set_to_the_value_already_running_needs_no_restart() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let running: Config =
        toml::from_str("[audio]\nhotkey = \"RightCommand\"\n").expect("a legal binding");
    state.set_config(std::sync::Arc::new(running));
    let next: Config =
        toml::from_str("[audio]\nhotkey = \"RightCommand\"\n").expect("a legal binding");

    let keys = vec!["audio.hotkey".to_string()];
    let outcome = super::apply_each(&state, &next, keys.iter());

    assert!(
        outcome.restart_required.is_empty(),
        "nothing changed, so nothing waits: {:?}",
        outcome.restart_required
    );
    assert_eq!(outcome.applied, keys);
}

/// The other half of the same rule: a binding that really is different has
/// to wait, or the window would promise a key that does nothing yet.
#[test]
fn a_restart_only_key_set_to_a_different_value_still_waits() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let running: Config =
        toml::from_str("[audio]\nhotkey = \"RightCommand\"\n").expect("a legal binding");
    state.set_config(std::sync::Arc::new(running));
    let next: Config =
        toml::from_str("[audio]\nhotkey = \"LeftCommand\"\n").expect("a legal binding");

    let keys = vec!["audio.hotkey".to_string()];
    let outcome = super::apply_each(&state, &next, keys.iter());

    assert_eq!(outcome.restart_required, keys);
    assert!(outcome.applied.is_empty());
}

#[test]
fn a_value_outside_its_range_is_refused() {
    let error = edit("", &assignments(&[("stt.vad_threshold", 5.0.into())])).unwrap_err();
    assert!(
        error.to_string().contains("0.0 and 1.0"),
        "the error must state the range: {error}"
    );
}

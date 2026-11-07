use super::{Assignments, Outcome, edit, write_first_feedback};
use crate::config::Config;
use crate::config::FeedbackMode;
use crate::credentials::RemoteKey;
use crate::models::download::Download;
use crate::speech_to_text::Route;
use crate::state::{ConsumerCommand, Listening, RecordingError};
use banshee_common::error::BansheeError;

/// A write against a config.toml of this test's own, so whose machine runs the
/// suite decides nothing.
pub(super) fn configure_in_scratch(
    state: Option<&crate::state::DaemonState>,
    assignments: Assignments,
    persist: bool,
) -> Result<Outcome, BansheeError> {
    let dir = crate::test_support::unique_scratch("settings");
    let credentials = match state {
        Some(state) => state.credentials_path().to_path_buf(),
        None => dir.join("credentials.toml"),
    };
    super::configure_at(
        &dir.join("config.toml"),
        &credentials,
        state,
        assignments,
        persist,
    )
}

/// The same write, over a config.toml that already holds `existing`.
fn configure_over(existing: &str, assignments: Assignments) -> Result<Outcome, BansheeError> {
    let dir = crate::test_support::unique_scratch("settings");
    let path = dir.join("config.toml");
    std::fs::write(&path, existing).unwrap();
    super::configure_at(
        &path,
        &dir.join("credentials.toml"),
        None,
        assignments,
        true,
    )
}

fn assignments(pairs: &[(&str, serde_json::Value)]) -> Assignments {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

#[test]
fn a_model_id_is_written_as_the_catalogue_spells_it() {
    let (rendered, config) =
        edit("", &assignments(&[("stt.model", "qwen3-asr-0.6b".into())])).unwrap();
    assert!(
        rendered.contains("model = \"Qwen3-ASR-0.6B\""),
        "{rendered}"
    );
    assert_eq!(config.stt.model, "Qwen3-ASR-0.6B");
}

#[test]
fn a_write_keeps_the_comments_around_it() {
    let existing = "# how strict speech detection is\n[stt]\nvad_threshold = 0.5 # tune me\ntranslate = false\n";
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
        rendered.contains("translate = false"),
        "an untouched key must survive: {rendered}"
    );
    assert_eq!(config.stt.vad_threshold, 0.8);
}

#[test]
fn a_write_keeps_the_comment_above_the_key_it_changes() {
    let existing = "[stt]\nvad_threshold = 0.5\n\n# the model that listens\nmodel = \"parakeet-unified-en-0.6b\"\n\ntranslate = false\n";
    let (rendered, _) = edit(
        existing,
        &assignments(&[("stt.model", "whisper-large-v3-turbo".into())]),
    )
    .unwrap();

    assert!(
        rendered.contains("# the model that listens\nmodel = \"whisper-large-v3-turbo\""),
        "the comment above the key must stay above it: {rendered}"
    );
    assert!(
        rendered.contains("\n\n# the model that listens"),
        "the blank line separating it must survive: {rendered}"
    );
}

#[test]
fn a_setting_two_sections_deep_is_reachable() {
    let (rendered, config) = edit(
        "",
        &assignments(&[("stt.remote.base_url", "https://listener.example/v1".into())]),
    )
    .unwrap();
    assert!(
        rendered.contains("[stt.remote]"),
        "the key must land under its own section, not quoted under [stt]: {rendered}"
    );
    assert!(
        !rendered.contains("[stt]"),
        "a parent invented only to hold the subtable needs no header: {rendered}"
    );
    assert_eq!(config.stt.remote.base_url, "https://listener.example/v1");
}

// An [stt] that holds keys of its own keeps its header either way, so the
// section here is empty: only then does suppressing it lose the comment.
#[test]
fn a_nested_write_keeps_a_section_that_was_already_written() {
    let existing = "# listener settings, see docs\n[stt]\n\n[tts]\nvoice = \"af_sky\"\n";
    let (rendered, _) = edit(
        existing,
        &assignments(&[("stt.remote.base_url", "https://listener.example/v1".into())]),
    )
    .unwrap();
    assert!(
        rendered.contains("# listener settings, see docs"),
        "the comment above the section must survive: {rendered}"
    );
    assert!(
        rendered.contains("[stt]"),
        "a section the user wrote must keep its header: {rendered}"
    );
    assert!(rendered.contains("[stt.remote]"), "{rendered}");
}

#[test]
fn a_model_banshee_does_not_know_is_refused_at_the_boundary() {
    let error = configure_in_scratch(None, assignments(&[("stt.model", "nope".into())]), false)
        .expect_err("an unknown model must not be written");
    assert!(error.to_string().contains("nope"), "{error}");
}

#[test]
fn plain_http_to_the_internet_is_not_written() {
    let error = configure_in_scratch(
        None,
        assignments(&[("tts.remote.base_url", "http://api.example.com/v1".into())]),
        true,
    )
    .expect_err("plain http to the internet must not be written");
    assert!(error.to_string().contains("needs https"), "{error}");
}

const OLD_PLAIN_HTTP_LISTENER: &str =
    "[stt]\nprovider = \"remote\"\n\n[stt.remote]\nbase_url = \"http://api.example.com/v1\"\n";

#[test]
fn switching_to_a_remote_side_with_plain_http_to_the_internet_is_not_written() {
    let error = configure_over(
        OLD_PLAIN_HTTP_LISTENER,
        assignments(&[("stt.provider", "remote".into())]),
    )
    .expect_err("a side switched on over plain http to the internet must not be written");
    assert!(error.to_string().contains("needs https"), "{error}");
}

#[test]
fn switching_a_side_to_local_over_an_old_plain_http_url_is_written() {
    configure_over(
        OLD_PLAIN_HTTP_LISTENER,
        assignments(&[("stt.provider", "local".into())]),
    )
    .expect("turning the side off is not refused");
}

#[test]
fn an_unrelated_write_beside_an_old_plain_http_url_is_written() {
    configure_over(
        OLD_PLAIN_HTTP_LISTENER,
        assignments(&[("stt.translate", true.into())]),
    )
    .expect("a write that touches no remote side is not refused");
}

// The weaker sibling tests only ask whether a key is in the live set. This
// one asks whether the write reached the daemon that is running.
#[test]
fn a_barge_in_write_reaches_the_running_daemon() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    assert!(matches!(state.barge_in(), crate::config::BargeInMode::Stop));

    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("audio.barge_in", "none".into())]),
        false,
    )
    .expect("a known key and a legal value must apply");

    assert!(
        matches!(state.barge_in(), crate::config::BargeInMode::None),
        "the next dictation must obey the new value, not the old one"
    );
    assert_eq!(outcome.applied, vec!["audio.barge_in".to_string()]);
    assert!(outcome.waiting.is_empty());
}

// The thread that owns capture is woken by the write and opens or closes at
// once.
#[test]
fn keep_open_reaches_the_running_daemon_both_ways() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    assert!(!state.keep_open(), "capture opens for each use by default");

    for keep in [true, false] {
        let outcome = configure_in_scratch(
            Some(&state),
            assignments(&[("audio.keep_open", keep.into())]),
            false,
        )
        .expect("a known key and a legal value must apply");
        assert_eq!(state.keep_open(), keep);
        assert_eq!(state.wants_mic(), keep);
        assert_eq!(outcome.applied, vec!["audio.keep_open".to_string()]);
        assert!(outcome.waiting.is_empty());
    }
}

#[test]
fn a_keep_open_that_is_not_true_or_false_is_refused_by_name() {
    let error = configure_in_scratch(
        None,
        assignments(&[("audio.keep_open", "yes".into())]),
        false,
    )
    .expect_err("keep_open is a switch");
    assert!(error.to_string().contains("keep_open"), "{error}");
}

#[test]
fn unload_after_reaches_the_running_daemon_and_zero_keeps_the_models() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    assert_eq!(
        state.unload_after(),
        Some(std::time::Duration::from_secs(300)),
        "five idle minutes by default"
    );

    for (minutes, limit) in [(1, Some(std::time::Duration::from_secs(60))), (0, None)] {
        let outcome = configure_in_scratch(
            Some(&state),
            assignments(&[("models.unload_after", minutes.into())]),
            false,
        )
        .expect("a whole number of minutes is a legal value");
        assert_eq!(state.unload_after(), limit, "{minutes}");
        assert_eq!(outcome.applied, vec!["models.unload_after".to_string()]);
        assert!(
            taken
                .try_iter()
                .any(|command| matches!(command, crate::state::ConsumerCommand::UnloadAfter)),
            "{minutes}: the listener measures its idle wait again"
        );
    }
}

#[test]
fn an_unload_after_that_is_not_whole_minutes_is_refused_by_name() {
    for wrong in [serde_json::Value::from("five"), (-1).into(), 1.5.into()] {
        let error = configure_in_scratch(
            None,
            assignments(&[("models.unload_after", wrong.clone())]),
            false,
        )
        .expect_err("unload_after is whole minutes");
        assert!(
            error.to_string().contains("unload_after"),
            "{wrong}: {error}"
        );
    }
}

#[test]
fn a_feedback_write_reaches_the_running_daemon() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    assert_eq!(state.feedback_mode(), FeedbackMode::Off);

    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("feedback.mode", "visual".into())]),
        false,
    )
    .expect("a known key and a legal value must apply");

    assert_eq!(state.feedback_mode(), FeedbackMode::Visual);
    assert_eq!(outcome.applied, vec!["feedback.mode".to_string()]);
    assert!(outcome.waiting.is_empty());
}

#[test]
fn a_speak_problems_write_reaches_the_running_daemon() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    assert!(state.speaks_problems(), "the fixture must start elsewhere");

    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("feedback.speak_problems", false.into())]),
        false,
    )
    .expect("a known key and a legal value must apply");

    assert!(!state.speaks_problems());
    assert_eq!(outcome.applied, vec!["feedback.speak_problems".to_string()]);
    assert!(outcome.waiting.is_empty());
}

#[test]
fn removing_the_feedback_mode_falls_back_to_both() {
    let (_, config) = edit(
        "[feedback]\nmode = \"sound\"\n",
        &assignments(&[("feedback.mode", serde_json::Value::Null)]),
    )
    .unwrap();
    assert_eq!(config.feedback_mode(), FeedbackMode::Both);
}

#[test]
fn a_save_history_write_reaches_the_running_daemon() {
    let state = crate::test_support::daemon_state_with_history(&["one"]);
    assert!(state.history_enabled());

    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("daemon.save_history", false.into())]),
        true,
    )
    .expect("a known key and a legal value must apply");

    assert!(
        !state.history_enabled(),
        "the next dictation must not be kept"
    );
    assert_eq!(state.history(None, None).unwrap().len(), 1);
    assert_eq!(outcome.applied, vec!["daemon.save_history".to_string()]);
    assert!(outcome.waiting.is_empty());
}

#[test]
fn a_keep_audio_write_reaches_the_running_daemon() {
    let state = crate::test_support::daemon_state_with_history(&[]);
    assert_eq!(state.keep_audio(), crate::config::KeepAudio::SevenDays);

    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("daemon.keep_audio", "1d".into())]),
        true,
    )
    .expect("a known key and a legal value must apply");

    assert_eq!(state.keep_audio(), crate::config::KeepAudio::OneDay);
    assert_eq!(outcome.applied, vec!["daemon.keep_audio".to_string()]);
    assert!(outcome.waiting.is_empty());
}

#[test]
fn an_unpersisted_save_history_write_still_stops_the_next_row() {
    let state = crate::test_support::daemon_state_with_history(&[]);

    configure_in_scratch(
        Some(&state),
        assignments(&[("daemon.save_history", false.into())]),
        false,
    )
    .unwrap();

    assert!(!state.history_enabled());
    state.keep(&crate::history::NewEntry::dictation("dropped"));
    assert!(state.history(None, None).unwrap().is_empty());
}

#[test]
fn a_voice_write_reaches_the_running_daemon() {
    let (state, spoken) = crate::test_support::daemon_state_recording_tts();
    assert_eq!(state.tts_voice(), None);

    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("tts.voice", "am_adam".into())]),
        false,
    )
    .expect("a known key and a legal value must apply");

    assert_eq!(
        spoken.lock().unwrap().last().cloned(),
        Some("am_adam".to_string()),
        "the next utterance must be spoken in the new voice"
    );
    assert_eq!(
        state.tts_voice().as_deref(),
        Some("am_adam"),
        "the window marks the current voice from this"
    );
    assert_eq!(outcome.applied, vec!["tts.voice".to_string()]);
    assert!(outcome.waiting.is_empty());
}

#[test]
fn a_persisted_write_moves_settings_changes() {
    let (state, _spoken) = crate::test_support::daemon_state_recording_tts();
    let before = state.settings_changes();

    configure_in_scratch(
        Some(&state),
        assignments(&[("tts.voice", "am_adam".into())]),
        true,
    )
    .unwrap();

    assert_eq!(state.settings_changes(), before + 1);
}

#[test]
fn an_unpersisted_write_leaves_settings_changes() {
    let (state, _spoken) = crate::test_support::daemon_state_recording_tts();
    let before = state.settings_changes();

    configure_in_scratch(
        Some(&state),
        assignments(&[("tts.voice", "am_adam".into())]),
        false,
    )
    .unwrap();

    assert_eq!(state.settings_changes(), before);
}

// A backend that cannot take the voice must not be reported as speaking in
// it, or the window marks a voice nothing will ever use.
#[test]
fn a_backend_that_refuses_the_voice_is_not_reported_as_applied() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);

    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("tts.voice", "am_adam".into())]),
        false,
    )
    .expect("a known key and a legal value must reach the backend");

    assert!(
        outcome.applied.is_empty(),
        "the null backend takes no voice"
    );
    assert_eq!(outcome.waiting, vec!["tts.voice".to_string()]);
    assert_eq!(
        state.tts_voice(),
        None,
        "no voice is in use, so none is named"
    );
}

#[test]
fn two_keys_of_one_setting_apply_it_once() {
    let keys = ["audio.hotkey".to_string(), "audio.hotkey_mode".to_string()];
    let mut applies = Vec::new();

    let outcome = super::outcome_of(keys.iter(), |variant| {
        applies.push(variant);
        true
    });

    assert_eq!(applies.len(), 1, "one call, one apply: {}", applies.len());
    assert_eq!(outcome.applied, keys, "both keys are in effect");
}

#[test]
fn a_vocabulary_write_reaches_the_listener_that_holds_the_engine() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);

    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("stt.vocabulary", vec!["banshee", "tokio"].into())]),
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
fn a_listening_model_that_arrives_is_loaded_into_the_waiting_pipeline() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let parakeet =
        banshee_common::listening::find(banshee_common::listening::DEFAULT_MODEL).unwrap();

    super::reapply_pending(&state);
    assert!(taken.try_recv().is_err(), "nothing waits for a model");

    state.set_listening(crate::state::Listening::Missing(parakeet));
    super::reapply_pending(&state);
    assert!(taken.try_recv().is_err(), "the file is not here yet");

    pretend_downloaded(&state, parakeet.file);
    super::reapply_pending(&state);
    match taken.try_recv() {
        Ok(crate::state::ConsumerCommand::Reload(model)) => assert_eq!(model, parakeet),
        _ => panic!("the model that arrived is loaded"),
    }
}

#[test]
fn a_second_download_that_ends_while_the_model_loads_queues_no_second_load() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let parakeet =
        banshee_common::listening::find(banshee_common::listening::DEFAULT_MODEL).unwrap();
    state.set_listening(crate::state::Listening::Missing(parakeet));
    pretend_downloaded(&state, parakeet.file);

    super::reapply_pending(&state);
    super::reapply_pending(&state);

    assert_eq!(reloads(&taken), vec![parakeet], "one file, one load");
    assert_eq!(
        state.listening(),
        crate::state::Listening::Loading(parakeet)
    );
    assert_eq!(
        state.pipeline(),
        crate::state::Pipeline::Open,
        "a press waits for the load"
    );
}

#[test]
fn a_transcription_server_chosen_while_the_model_downloads_is_queued_ahead_of_the_load() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let parakeet =
        banshee_common::listening::find(banshee_common::listening::DEFAULT_MODEL).unwrap();
    state.set_listening(crate::state::Listening::Missing(parakeet));

    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("stt.provider", "remote".into())]),
        true,
    )
    .expect("a transcription server is a legal choice");
    assert!(outcome.waiting.is_empty());

    pretend_downloaded(&state, parakeet.file);
    super::reapply_pending(&state);

    let order: Vec<&str> = taken
        .try_iter()
        .map(|command| match command {
            ConsumerCommand::Replace(_) => "replace",
            ConsumerCommand::Reload(_) => "reload",
            _ => "other",
        })
        .collect();
    assert_eq!(
        order,
        ["replace", "reload"],
        "the remote listener is in place before the load reaches it"
    );
}

/// The route of every listener `taken` holds a `Replace` for, in order.
fn replaces(taken: &std::sync::mpsc::Receiver<ConsumerCommand>) -> Vec<Route> {
    taken
        .try_iter()
        .filter_map(|command| match command {
            ConsumerCommand::Replace(stt) => Some(Route::of(&stt)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_switch_to_a_server_reaches_the_listener_that_holds_the_engine() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("stt.provider", "remote".into())]),
        true,
    )
    .unwrap();
    assert_eq!(
        replaces(&taken),
        [Route::of(&crate::test_support::remote_stt_default())]
    );
    assert_eq!(outcome.applied, vec!["stt.provider".to_string()]);
    assert_eq!(
        state.listener_withdrawals(),
        0,
        "turning a server on withdraws nothing"
    );
}

#[test]
fn a_write_that_leaves_the_route_alone_sends_nothing() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("stt.remote.model", "whisper-2".into())]),
        true,
    )
    .unwrap();
    assert!(replaces(&taken).is_empty());
    assert_eq!(outcome.applied, vec!["stt.remote.model".to_string()]);
}

#[test]
fn turning_the_server_off_withdraws_the_request_in_flight() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    state.install_listener(
        Route::of(&crate::test_support::remote_stt_default()),
        Listening::Remote,
    );
    configure_in_scratch(
        Some(&state),
        assignments(&[("stt.provider", "local".into())]),
        true,
    )
    .unwrap();
    assert_eq!(state.listener_withdrawals(), 1);
    assert_eq!(
        state.take_withdrawn_listener().map(|stt| stt.provider),
        Some(crate::config::Provider::Local)
    );
    assert_eq!(replaces(&taken), [Route::Local]);
}

#[test]
fn a_server_turned_off_before_its_switch_landed_is_turned_off_again() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    for provider in ["remote", "local"] {
        configure_in_scratch(
            Some(&state),
            assignments(&[("stt.provider", provider.into())]),
            true,
        )
        .unwrap();
    }
    assert_eq!(
        replaces(&taken),
        [
            Route::of(&crate::test_support::remote_stt_default()),
            Route::Local
        ],
        "the switch to the server still waits in the queue"
    );
    assert_eq!(state.listener_withdrawals(), 1);
}

#[test]
fn writing_local_again_over_a_listener_that_would_not_build_rebuilds_it() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let parakeet =
        banshee_common::listening::find(banshee_common::listening::DEFAULT_MODEL).unwrap();
    state.install_listener(
        Route::Local,
        Listening::Failed(
            parakeet,
            "a model would not load: the models folder is gone".into(),
        ),
    );
    configure_in_scratch(
        Some(&state),
        assignments(&[("stt.provider", "local".into())]),
        true,
    )
    .unwrap();
    assert_eq!(replaces(&taken), [Route::Local]);
}

#[test]
fn a_switch_that_does_not_parse_reaches_no_listener() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let refused = configure_in_scratch(
        Some(&state),
        assignments(&[
            ("stt.provider", "remote".into()),
            ("stt.remote.base_url", "not a url".into()),
        ]),
        true,
    );
    assert!(refused.is_err());
    assert!(replaces(&taken).is_empty());
    assert_eq!(state.listener_withdrawals(), 0);
}

#[test]
fn removing_the_listener_key_withdraws_the_request_in_flight() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    state.install_listener(
        Route::of(&crate::test_support::remote_stt_default()),
        Listening::Remote,
    );
    configure_in_scratch(
        Some(&state),
        assignments(&[("stt.remote.api_key", "".into())]),
        true,
    )
    .unwrap();
    assert_eq!(state.listener_withdrawals(), 1);
    assert!(state.take_withdrawn_listener().is_none());
    assert!(matches!(state.listening(), Listening::Refused(_)));
}

#[test]
fn turning_the_server_off_and_removing_its_key_in_one_write_goes_local() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    state.install_listener(
        Route::of(&crate::test_support::remote_stt_default()),
        Listening::Remote,
    );
    configure_in_scratch(
        Some(&state),
        assignments(&[
            ("stt.provider", "local".into()),
            ("stt.remote.api_key", "".into()),
        ]),
        true,
    )
    .unwrap();
    assert_eq!(
        state.take_withdrawn_listener().map(|stt| stt.provider),
        Some(crate::config::Provider::Local),
        "the turn-off is what the listener acts on"
    );
}

#[test]
fn a_new_listener_key_lets_the_request_in_flight_finish() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    state.install_listener(
        Route::of(&crate::test_support::remote_stt_default()),
        Listening::Remote,
    );
    configure_in_scratch(
        Some(&state),
        assignments(&[("stt.remote.api_key", "sk-test".into())]),
        true,
    )
    .unwrap();
    assert_eq!(state.listener_withdrawals(), 0);
}

#[test]
fn a_listening_model_that_arrived_while_the_daemon_started_is_loaded() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let parakeet =
        banshee_common::listening::find(banshee_common::listening::DEFAULT_MODEL).unwrap();
    state.set_listening(crate::state::Listening::Missing(parakeet));
    pretend_downloaded(&state, parakeet.file);

    super::reconcile_listening(&state);

    match taken.try_recv() {
        Ok(crate::state::ConsumerCommand::Reload(model)) => assert_eq!(model, parakeet),
        _ => panic!("the model whose file is here is loaded"),
    }
}

#[test]
fn a_failed_load_is_tried_again_by_a_write_of_the_same_model() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let parakeet =
        banshee_common::listening::find(banshee_common::listening::DEFAULT_MODEL).unwrap();
    state.set_listening(crate::state::Listening::Missing(parakeet));
    pretend_downloaded(&state, parakeet.file);
    state.listening_model_failed(parakeet, "the file is damaged".to_string());

    assert!(super::reconcile(&state, &Config::default()));

    match taken.try_recv() {
        Ok(crate::state::ConsumerCommand::Reload(model)) => assert_eq!(model, parakeet),
        _ => panic!("nothing listens, so the model loads again"),
    }
}

fn reloads(
    taken: &std::sync::mpsc::Receiver<crate::state::ConsumerCommand>,
) -> Vec<&'static banshee_common::listening::ListeningModel> {
    taken
        .try_iter()
        .filter_map(|command| match command {
            crate::state::ConsumerCommand::Reload(model) => Some(model),
            _ => None,
        })
        .collect()
}

#[test]
fn a_downloaded_model_picked_while_another_serves_loads_once_whatever_downloads_end() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    pretend_downloaded(&state, turbo().file);

    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("stt.model", TURBO.into())]),
        true,
    )
    .expect("a known model is a legal write");
    assert_eq!(outcome.applied, vec!["stt.model".to_string()]);
    super::reapply_pending(&state);
    super::reconcile_listening(&state);

    assert_eq!(reloads(&taken), vec![turbo()], "one pick, one load");
}

#[test]
fn a_download_that_ends_while_a_model_serves_loads_nothing() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let mut moved = Config::default();
    moved.stt.model = TURBO.to_string();
    state.set_config(std::sync::Arc::new(moved));
    pretend_downloaded(&state, turbo().file);
    state.listening_model_failed(turbo(), "the file is damaged".to_string());

    super::reapply_pending(&state);

    assert!(
        reloads(&taken).is_empty(),
        "the serving model listens on, and no write asked for turbo again"
    );
}

#[test]
fn a_model_the_config_no_longer_names_is_not_loaded_when_it_arrives() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let parakeet =
        banshee_common::listening::find(banshee_common::listening::DEFAULT_MODEL).unwrap();
    state.set_listening(crate::state::Listening::Missing(parakeet));
    let mut moved = Config::default();
    moved.stt.model = TURBO.to_string();
    state.set_config(std::sync::Arc::new(moved));

    pretend_downloaded(&state, parakeet.file);
    super::reapply_pending(&state);
    assert!(
        taken.try_recv().is_err(),
        "the config chose turbo, so loading Parakeet is stale"
    );
}

#[test]
fn a_model_chosen_while_none_listens_is_the_one_awaited() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let parakeet =
        banshee_common::listening::find(banshee_common::listening::DEFAULT_MODEL).unwrap();
    state.set_listening(crate::state::Listening::Missing(parakeet));
    state.listening_model_failed(parakeet, "the file is damaged".to_string());

    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("stt.model", TURBO.into())]),
        true,
    )
    .expect("a known model is a legal write");
    assert_eq!(outcome.waiting, vec!["stt.model".to_string()]);
    assert_eq!(
        state.pipeline(),
        crate::state::Pipeline::Broken(crate::state::RecordingError::NoListeningModel(TURBO)),
        "the old failure is not this model's"
    );
    assert!(taken.try_recv().is_err(), "turbo is not here yet");

    pretend_downloaded(&state, turbo().file);
    super::reapply_pending(&state);
    assert_eq!(
        reloads(&taken),
        vec![turbo()],
        "turbo loads once when it arrives"
    );
}

/// Puts an empty stand-in for `name` in the state's model directory, so the
/// model reads as downloaded.
fn pretend_downloaded(state: &crate::state::DaemonState, name: &str) {
    std::fs::create_dir_all(state.models_dir()).unwrap();
    std::fs::write(state.models_dir().join(name), b"").unwrap();
}

const TURBO: &str = "whisper-large-v3-turbo";

fn turbo() -> &'static banshee_common::listening::ListeningModel {
    banshee_common::listening::find(TURBO).unwrap()
}

#[test]
fn a_model_write_under_a_refused_remote_listener_loads_nothing() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    state.set_listening(Listening::Refused(RecordingError::Provider(
        "no key".into(),
    )));
    pretend_downloaded(&state, turbo().file);
    let mut file = Config::default();
    file.stt.model = TURBO.to_string();
    assert!(super::reconcile(&state, &file));
    assert!(
        taken.try_recv().is_err(),
        "a refused remote listener holds no engine to load into"
    );
    assert!(matches!(state.listening(), Listening::Refused(_)));
}

// These ask `reconcile` directly, against the model directory the state
// holds, which starts empty.
#[test]
fn a_model_write_reaches_the_listener_that_holds_the_engine() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let mut wanted = Config::default();
    wanted.stt.model = TURBO.to_string();
    pretend_downloaded(&state, turbo().file);

    let applied = super::reconcile(&state, &wanted);

    assert!(applied, "a downloaded model loads live");
    match taken.try_recv() {
        Ok(crate::state::ConsumerCommand::Reload(model)) => assert_eq!(model, turbo()),
        other => panic!("the listener was handed no model: {:?}", other.is_ok()),
    }
}

// Every click on a model writes the key, the active one included. Without the
// guard each of those costs a load and holds two models at once.
#[test]
fn a_model_already_behind_the_engine_loads_nothing() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    state.listening_model_loaded(Some(turbo()));
    let mut file = Config::default();
    file.stt.model = TURBO.to_string();

    assert!(super::reconcile(&state, &file));
    assert!(taken.try_recv().is_err(), "the model was already loaded");
}

#[test]
fn a_model_write_under_a_remote_listener_is_applied_and_loads_nothing() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let mut file = Config::default();
    file.stt.provider = crate::config::Provider::Remote;
    file.stt.model = TURBO.to_string();

    assert!(super::reconcile(&state, &file));
    assert!(
        taken.try_recv().is_err(),
        "nothing is loaded for a remote listener"
    );
}

/// The file can name a local listener while a remote one is still in force.
/// The listener switch in the same write builds the local engine.
#[test]
fn a_model_written_for_a_listener_that_is_not_running_yet_loads_nothing() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    state.install_listener(
        Route::Remote {
            base_url: "https://api.openai.com/v1".into(),
            model: "whisper-1".into(),
        },
        Listening::Remote,
    );
    let mut file = Config::default();
    file.stt.model = TURBO.to_string();

    let applied = super::reconcile(&state, &file);

    assert!(
        taken.try_recv().is_err(),
        "a remote listener holds no engine to load into"
    );
    assert!(
        !applied,
        "a model that is not downloaded leaves the key waiting"
    );
}

#[test]
fn a_provider_write_is_kept_by_the_file() {
    let (rendered, config) = edit(
        "[stt]\ntranslate = false\n",
        &assignments(&[("stt.provider", "local".into())]),
    )
    .unwrap();
    assert!(
        rendered.contains("provider = \"local\""),
        "the key must land under [stt]: {rendered}"
    );
    assert_eq!(config.stt.provider, crate::config::Provider::Local);
}

/// Both providers written with the values in force move nothing.
#[test]
fn a_provider_set_to_the_value_already_running_waits_for_nothing() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let keys = vec!["stt.provider".to_string(), "tts.provider".to_string()];
    let outcome = super::apply_each(&state, &Config::default(), keys.iter());

    assert!(
        outcome.waiting.is_empty(),
        "nothing changed, so nothing waits: {:?}",
        outcome.waiting
    );
    assert_eq!(outcome.applied, keys);
    assert!(replaces(&taken).is_empty(), "the listener in force stays");
}

#[test]
fn a_missing_section_is_created() {
    let (rendered, config) = edit("", &assignments(&[("tts.voice", "af_bella".into())])).unwrap();
    assert!(rendered.contains("[tts]"), "{rendered}");
    assert_eq!(config.tts.voice, "af_bella");
}

#[test]
fn a_null_clears_the_key_back_to_its_default() {
    let existing = "[tts.remote]\nresponse_format = \"pcm\"\nsample_rate = 22050\n";
    let (rendered, config) = edit(
        existing,
        &assignments(&[("tts.remote.sample_rate", serde_json::Value::Null)]),
    )
    .unwrap();
    assert!(!rendered.contains("sample_rate"), "{rendered}");
    assert!(rendered.contains("response_format"), "{rendered}");
    assert_eq!(config.tts.remote.sample_rate, None);
}

#[test]
fn a_null_for_a_key_never_written_changes_nothing() {
    let (rendered, config) = edit(
        "",
        &assignments(&[("tts.remote.sample_rate", serde_json::Value::Null)]),
    )
    .unwrap();
    assert_eq!(rendered, "");
    assert_eq!(config.tts.remote.sample_rate, None);
}

#[test]
fn a_null_for_an_unknown_key_is_refused() {
    let error = edit(
        "",
        &assignments(&[("stt.nonesuch", serde_json::Value::Null)]),
    )
    .unwrap_err();
    assert!(error.to_string().contains("nonesuch"), "{error}");
}

#[test]
fn a_null_for_a_section_is_refused() {
    let existing = "[tts.remote]\nvoice = \"marin\"\n";
    let error = edit(
        existing,
        &assignments(&[("tts.remote", serde_json::Value::Null)]),
    )
    .unwrap_err();
    assert!(error.to_string().contains("tts.remote"), "{error}");
}

#[test]
fn a_null_inside_an_inline_table_is_refused_as_a_write_would_be() {
    let existing = "[tts]\nremote = { response_format = \"pcm\", sample_rate = 22050 }\n";
    let error = edit(
        existing,
        &assignments(&[("tts.remote.sample_rate", serde_json::Value::Null)]),
    )
    .unwrap_err();
    assert!(error.to_string().contains("is not a section"), "{error}");
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
        error.to_string().contains("stt.model"),
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
    // `audio.barge_in` is live and needs no file, so it stands in for a
    // key whose apply succeeds the moment it is asked again.
    state.record_outcome(&[], &["audio.barge_in".to_string()]);
    assert_eq!(state.pending(), vec!["audio.barge_in".to_string()]);

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
    assert_ne!(
        state.config().stt.model,
        TURBO,
        "the fixture must ask for a model the daemon does not run"
    );
    let mut next = Config::default();
    next.stt.model = TURBO.to_string();
    state.set_config(std::sync::Arc::new(next));
    state.record_outcome(&[], &["stt.model".to_string()]);

    super::reapply_pending(&state);
    assert_eq!(state.pending(), vec!["stt.model".to_string()]);
}

#[test]
fn a_waiting_key_becomes_pending_and_an_applied_one_does_not() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    state.record_outcome(
        &["stt.vad_threshold".to_string()],
        &["audio.barge_in".to_string()],
    );
    assert_eq!(state.pending(), vec!["audio.barge_in".to_string()]);

    state.record_outcome(&["audio.barge_in".to_string()], &[]);
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

#[test]
fn the_api_keys_are_split_off_and_never_rendered() {
    let mut rest = assignments(&[
        ("stt.remote.api_key", "sk-listener".into()),
        ("tts.remote.api_key", "sk-speaker".into()),
        ("stt.remote.model", "whisper-1".into()),
    ]);
    let keys = super::take_api_keys(&mut rest).unwrap();
    assert_eq!(
        keys,
        vec![
            (RemoteKey::Stt, "sk-listener".to_string()),
            (RemoteKey::Tts, "sk-speaker".to_string()),
        ]
    );
    assert!(!rest.contains_key("stt.remote.api_key"));
    assert!(!rest.contains_key("tts.remote.api_key"));

    let (rendered, _) = edit("[stt]\nprovider = \"remote\"\n", &rest).unwrap();
    assert!(
        !rendered.contains("sk-listener") && !rendered.contains("sk-speaker"),
        "neither key may be rendered: {rendered}"
    );
    assert!(rendered.contains("model = \"whisper-1\""));
}

#[test]
fn one_side_alone_splits_off_alone() {
    let mut only_speaker = assignments(&[("tts.remote.api_key", "sk-speaker".into())]);
    assert_eq!(
        super::take_api_keys(&mut only_speaker).unwrap(),
        vec![(RemoteKey::Tts, "sk-speaker".to_string())]
    );
    let mut neither = assignments(&[("tts.voice", "af_sky".into())]);
    assert!(super::take_api_keys(&mut neither).unwrap().is_empty());
    assert!(neither.contains_key("tts.voice"));
}

// A file hand-edited to hold a key renders back into the document `edit`
// validates
#[test]
fn a_key_already_in_the_file_is_refused_without_the_reply_carrying_it() {
    let error = edit(
        "[tts.remote]\napi_key = \"sk-test\"\n",
        &assignments(&[("tts.remote.model", "tts-1".into())]),
    )
    .expect_err("the key must not validate from config.toml");
    assert!(
        error
            .to_string()
            .contains("banshee config set tts.remote.api_key"),
        "{error}"
    );
    assert!(
        !error.to_string().contains("sk-test"),
        "the reply must not echo the key: {error}"
    );
}

#[test]
fn a_base_url_without_a_scheme_is_refused_and_the_message_names_the_key() {
    for key in ["stt.remote.base_url", "tts.remote.base_url"] {
        let error = edit("", &assignments(&[(key, "localhost:8080/v1".into())]))
            .expect_err("a base_url with no scheme must not parse");
        assert!(
            error
                .to_string()
                .contains(&format!("{key} needs an http or https URL with a host")),
            "{error}"
        );
    }
}

#[test]
fn a_base_url_with_a_scheme_and_a_host_is_accepted() {
    for key in ["stt.remote.base_url", "tts.remote.base_url"] {
        let (_, config) = edit(
            "",
            &assignments(&[(key, "http://localhost:8080/v1".into())]),
        )
        .unwrap_or_else(|error| panic!("a legal base_url must parse: {error}"));
        let base_url = if key.starts_with("stt") {
            &config.stt.remote.base_url
        } else {
            &config.tts.remote.base_url
        };
        assert_eq!(base_url, "http://localhost:8080/v1");
    }
}

#[test]
fn a_key_that_is_not_a_string_is_refused() {
    let error = super::take_api_keys(&mut assignments(&[("tts.remote.api_key", 42.into())]))
        .expect_err("a number is not a key");
    assert!(error.to_string().contains("tts.remote.api_key"));
}

#[test]
fn the_listener_key_applies_at_once() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    state.set_listening(Listening::Refused(RecordingError::Provider(
        "no key".into(),
    )));
    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("stt.remote.api_key", "sk-test".into())]),
        true,
    )
    .unwrap();
    assert_eq!(outcome.applied, vec!["stt.remote.api_key".to_string()]);
    assert!(outcome.waiting.is_empty());
    assert_eq!(
        state.listening(),
        Listening::Remote,
        "the key ends the refusal"
    );
    let held = crate::credentials::Credentials::read(state.credentials_path()).unwrap();
    assert_eq!(
        held.key(crate::credentials::RemoteKey::Stt),
        Some("sk-test")
    );
}

#[test]
fn a_key_written_without_persist_is_still_refused() {
    let error = configure_in_scratch(
        None,
        assignments(&[("stt.remote.api_key", "sk-x".into())]),
        false,
    )
    .expect_err("a key decides what leaves the machine");
    assert!(error.to_string().contains("persist"), "{error}");
}

/// A scratch folder's `config.toml`, holding `text` unless it is empty, and
/// two `Download`s a test can call `write_first_feedback` as wanted or missing.
fn scratch_config(text: &str) -> (std::path::PathBuf, Vec<Download>) {
    let dir = crate::test_support::unique_scratch("settings");
    let path = dir.join("config.toml");
    if !text.is_empty() {
        std::fs::write(&path, text).unwrap();
    }
    let wanted = vec![
        Download {
            megabytes: 1,
            name: "one.bin".to_string(),
            url: "https://example.test/one".to_string(),
            sha256: None,
            model: None,
        },
        Download {
            megabytes: 1,
            name: "two.bin".to_string(),
            url: "https://example.test/two".to_string(),
            sha256: None,
            model: None,
        },
    ];
    (path, wanted)
}

#[test]
fn a_fetch_into_an_empty_folder_chooses_the_chip() {
    let (path, wanted) = scratch_config("");
    let chosen = write_first_feedback(&path, None, &wanted, &wanted, || false).unwrap();
    assert_eq!(chosen, Some(FeedbackMode::Visual));
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("mode = \"visual\"")
    );
}

#[test]
fn voiceover_starts_on_both() {
    let (path, wanted) = scratch_config("");
    let chosen = write_first_feedback(&path, None, &wanted, &wanted, || true).unwrap();
    assert_eq!(chosen, Some(FeedbackMode::Both));
}

#[test]
fn a_folder_that_holds_a_model_is_no_new_install() {
    let (path, wanted) = scratch_config("");
    let chosen = write_first_feedback(&path, None, &wanted, &wanted[1..], || {
        panic!("a folder that holds a model never reads VoiceOver")
    })
    .unwrap();
    assert_eq!(chosen, None);
    assert!(!path.exists(), "nothing is written");
}

#[test]
fn a_file_that_chose_keeps_its_choice() {
    let text = "[feedback]\nmode = \"sound\"\n";
    let (path, wanted) = scratch_config(text);
    assert_eq!(
        write_first_feedback(&path, None, &wanted, &wanted, || {
            panic!("an install that chose never reads VoiceOver")
        })
        .unwrap(),
        None
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn the_first_write_keeps_the_comments() {
    let text = "# my settings\n[audio]\nhotkey = \"F5\" # the key\n";
    let (path, wanted) = scratch_config(text);
    write_first_feedback(&path, None, &wanted, &wanted, || false).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("# my settings") && written.contains("# the key"));
}

#[test]
fn the_first_write_goes_through_a_symlinked_config() {
    let (target, wanted) = scratch_config("");
    let link = target.with_file_name("linked-config.toml");
    std::fs::write(&target, "").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    write_first_feedback(&link, None, &wanted, &wanted, || false).unwrap();
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(std::fs::read_to_string(&target).unwrap().contains("visual"));
}

#[test]
fn a_config_that_never_chose_a_mode_is_a_new_install() {
    let text = "[tts]\nvoice = \"af_sky\"\n";
    let (path, wanted) = scratch_config(text);
    assert_eq!(
        write_first_feedback(&path, None, &wanted, &wanted, || false).unwrap(),
        Some(FeedbackMode::Visual)
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.starts_with(text), "{written}");
}

#[test]
fn a_config_of_blank_lines_and_comments_is_a_new_install() {
    let (path, wanted) = scratch_config("\n  \n# my settings\n");
    let chosen = write_first_feedback(&path, None, &wanted, &wanted, || false).unwrap();
    assert_eq!(chosen, Some(FeedbackMode::Visual));
}

#[test]
fn a_mode_set_while_the_first_write_waits_wins() {
    let (path, wanted) = scratch_config("");
    let writing = super::WRITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let first = {
        let path = path.clone();
        std::thread::spawn(move || write_first_feedback(&path, None, &wanted, &wanted, || false))
    };
    // The locked check passes at any delay. The wait gives a check outside the
    // lock time to read the file before the write below.
    std::thread::sleep(std::time::Duration::from_millis(100));
    let chosen = "[feedback]\nmode = \"sound\"\n";
    std::fs::write(&path, chosen).unwrap();
    drop(writing);

    assert_eq!(first.join().unwrap().unwrap(), None);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), chosen);
}

#[test]
fn removing_the_last_key_of_a_section_leaves_no_header() {
    let dir = crate::test_support::unique_scratch("settings");
    let path = dir.join("config.toml");
    for value in [serde_json::Value::from(3), serde_json::Value::Null] {
        super::configure_at(
            &path,
            &dir.join("credentials.toml"),
            None,
            assignments(&[("models.unload_after", value)]),
            true,
        )
        .unwrap();
    }
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(!written.contains("[models]"), "{written}");
}

#[test]
fn a_quiet_length_write_reaches_the_running_daemon() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("stt.endpoint_silence_ms", 1000.into())]),
        false,
    )
    .expect("a known key and a legal value must apply");
    assert_eq!(state.endpoint_silence_ms(), 1000);
    assert_eq!(outcome.applied, vec!["stt.endpoint_silence_ms".to_string()]);
    assert!(outcome.waiting.is_empty());
}

#[test]
fn a_hotkey_write_reaches_the_running_daemon() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let (_, _, before) = state.binding();
    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[
            ("audio.hotkey", "F6".into()),
            ("audio.hotkey_mode", "toggle".into()),
        ]),
        false,
    )
    .unwrap();
    let (hotkey, mode, after) = state.binding();
    assert_eq!(hotkey, crate::binding::hotkey("F6").unwrap());
    assert_eq!(mode, crate::config::HotkeyMode::Toggle);
    assert_eq!(after, before + 1, "two keys of one binding move it once");
    assert!(outcome.waiting.is_empty());
}

/// Records the provider of every speaker the daemon builds. Each one speaks
/// into a list nobody reads.
fn counting_builds(
    state: &crate::state::DaemonState,
) -> std::sync::Arc<std::sync::Mutex<Vec<crate::config::Provider>>> {
    let built: std::sync::Arc<std::sync::Mutex<Vec<crate::config::Provider>>> =
        std::sync::Arc::default();
    let seen = std::sync::Arc::clone(&built);
    state.speech().set_builder(Box::new(move |tts| {
        seen.lock().unwrap().push(tts.provider);
        crate::test_support::speaking_selection(
            crate::test_support::SpokenLines::default(),
            crate::text_to_speech::Speaker::Configured(tts.voice.clone()),
        )
    }));
    built
}

#[test]
fn a_speaker_switch_reaches_the_running_daemon() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let built = counting_builds(&state);
    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[
            ("tts.provider", "remote".into()),
            ("tts.remote.voice", "alloy".into()),
        ]),
        true,
    )
    .unwrap();
    assert_eq!(*built.lock().unwrap(), [crate::config::Provider::Remote]);
    assert_eq!(
        state.speaker_spec().provider,
        crate::config::Provider::Remote
    );
    assert!(outcome.waiting.is_empty());
}

#[test]
fn both_sides_and_both_keys_in_one_write_rebuild_each_side_once() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    let built = counting_builds(&state);
    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[
            ("stt.provider", "remote".into()),
            ("tts.provider", "remote".into()),
            ("tts.remote.voice", "alloy".into()),
            ("stt.remote.api_key", "sk-a".into()),
            ("tts.remote.api_key", "sk-b".into()),
        ]),
        true,
    )
    .unwrap();
    assert_eq!(replaces(&taken).len(), 1);
    assert_eq!(built.lock().unwrap().len(), 1);
    assert!(outcome.waiting.is_empty());
}

#[test]
fn a_speaker_key_alone_rebuilds_the_speaker_once() {
    let mut config = Config::default();
    config.tts.provider = crate::config::Provider::Remote;
    let state = crate::test_support::daemon_state_speaking(
        config,
        crate::text_to_speech::Speaker::Fallback,
        std::sync::mpsc::channel().0,
    );
    let built = counting_builds(&state);
    configure_in_scratch(
        Some(&state),
        assignments(&[("tts.remote.api_key", "sk-b".into())]),
        true,
    )
    .unwrap();
    assert_eq!(built.lock().unwrap().len(), 1);
}

#[test]
fn the_fallback_applies_at_once() {
    let mut config = Config::default();
    config.tts.provider = crate::config::Provider::Remote;
    config.tts.remote.base_url = "https://a.example.test/v1".to_string();
    config.tts.remote.voice = "alloy".to_string();
    let state = crate::test_support::daemon_state_speaking(
        config,
        crate::text_to_speech::Speaker::Fallback,
        std::sync::mpsc::channel().0,
    );
    let built = counting_builds(&state);
    let outcome = configure_beside(
        &state,
        "https://a.example.test/v1",
        assignments(&[("tts.fallback", "none".into())]),
    );
    assert_eq!(built.lock().unwrap().len(), 1);
    assert_eq!(
        state.speaker_spec().fallback,
        crate::config::TTSFallback::None
    );
    assert!(outcome.waiting.is_empty());
}

/// A daemon whose speaker in force is the server at `base_url` with a key on
/// file, holding each reply for 30 s, with a builder whose speakers record
/// what they say.
fn speaking_through_a_server(
    base_url: &str,
) -> (
    crate::test_support::HoldingSpeech,
    crate::test_support::SpokenLines,
) {
    let speaking = speaking_under_a_server(
        base_url,
        crate::text_to_speech::Speaker::Configured("alloy".to_string()),
    );
    crate::test_support::write_key(
        speaking.0.state.credentials_path(),
        crate::credentials::RemoteKey::Tts,
        "sk-on-file",
    );
    speaking
}

/// `speaking_through_a_server` for a daemon that built `speaker`, with no key
/// on file.
fn speaking_under_a_server(
    base_url: &str,
    speaker: crate::text_to_speech::Speaker,
) -> (
    crate::test_support::HoldingSpeech,
    crate::test_support::SpokenLines,
) {
    let mut config = Config::default();
    config.tts.provider = crate::config::Provider::Remote;
    config.tts.remote.base_url = base_url.to_string();
    config.tts.remote.voice = "alloy".to_string();
    let sends_to = speaker.started().then(|| base_url.to_string());
    let held = crate::test_support::daemon_state_holding_speech_under(
        config,
        speaker,
        std::time::Duration::from_secs(30),
        sends_to,
    );
    let spoken_here = building_here(&held.state);
    held.state.speech().speak("the reply", false, None).unwrap();
    (held, spoken_here)
}

/// Sets a builder whose speakers keep what they say and send nothing out. A
/// server with no voice builds the stand-in, as the real build does.
fn building_here(state: &crate::state::DaemonState) -> crate::test_support::SpokenLines {
    let spoken_here = crate::test_support::SpokenLines::default();
    let lines = spoken_here.clone();
    state.speech().set_builder(Box::new(move |tts| {
        let standing_in = tts.provider.is_remote() && tts.remote.voice.is_empty();
        crate::test_support::speaking_selection(
            lines.clone(),
            if standing_in {
                crate::text_to_speech::Speaker::Fallback
            } else {
                crate::text_to_speech::Speaker::Configured(tts.remote.voice.clone())
            },
        )
    }));
    spoken_here
}

/// The scratch config.toml the write goes to, already naming the server.
fn configure_beside(
    state: &crate::state::DaemonState,
    base_url: &str,
    assignments: Assignments,
) -> Outcome {
    let dir = crate::test_support::unique_scratch("settings");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        format!("[tts]\nprovider = \"remote\"\n\n[tts.remote]\nbase_url = \"{base_url}\"\nvoice = \"alloy\"\n"),
    )
    .unwrap();
    super::configure_at(
        &path,
        state.credentials_path(),
        Some(state),
        assignments,
        true,
    )
    .unwrap()
}

/// Each row is the writes made while a reply plays on server A, each beside
/// the server config.toml names at that point, and whether the reply stops
/// and is spoken here.
#[test]
fn a_reply_on_a_server_stops_only_when_no_keyed_server_takes_the_next_line() {
    const A: &str = "https://a.example.test/v1";
    const B: &str = "https://b.example.test/v1";
    let rows = [
        (
            "the key removed",
            vec![(A, assignments(&[("tts.remote.api_key", "".into())]))],
            true,
        ),
        (
            "the server turned off",
            vec![(A, assignments(&[("tts.provider", "local".into())]))],
            true,
        ),
        (
            "a second server, then turned off",
            vec![
                (A, assignments(&[("tts.remote.base_url", B.into())])),
                (B, assignments(&[("tts.provider", "local".into())])),
            ],
            true,
        ),
        (
            "a new key",
            vec![(A, assignments(&[("tts.remote.api_key", "sk-new".into())]))],
            false,
        ),
    ];
    for (writes, steps, cut) in rows {
        let (held, spoken_here) = speaking_through_a_server(A);
        for (beside, step) in steps {
            configure_beside(&held.state, beside, step);
        }
        assert_eq!(
            held.cut_short.load(std::sync::atomic::Ordering::SeqCst),
            cut,
            "{writes}"
        );
        let expected: &[&str] = if cut { &["the reply"] } else { &[] };
        assert_eq!(*spoken_here.lock().unwrap(), expected, "{writes}");
    }
}

#[test]
fn another_speaker_server_lets_the_reply_finish_on_the_first() {
    let (held, spoken_here) = speaking_through_a_server("https://a.example.test/v1");
    configure_beside(
        &held.state,
        "https://a.example.test/v1",
        assignments(&[("tts.remote.base_url", "https://b.example.test/v1".into())]),
    );
    assert!(!held.cut_short.load(std::sync::atomic::Ordering::SeqCst));
    assert!(spoken_here.lock().unwrap().is_empty());
    held.state.speech().stop();
    held.state
        .speech()
        .speak("the next reply", false, None)
        .unwrap();
    assert_eq!(
        *spoken_here.lock().unwrap(),
        ["the next reply"],
        "the next reply goes to the second server"
    );
}

#[test]
fn a_reply_on_a_server_stops_at_the_turn_off_after_a_build_that_did_not_start() {
    let (held, spoken_here) = speaking_through_a_server("https://a.example.test/v1");
    configure_beside(
        &held.state,
        "https://a.example.test/v1",
        assignments(&[("tts.remote.voice", "".into())]),
    );
    assert!(!held.state.speaker_started(), "the stand-in speaks now");
    configure_beside(
        &held.state,
        "https://a.example.test/v1",
        assignments(&[("tts.provider", "local".into())]),
    );
    assert!(held.cut_short.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(*spoken_here.lock().unwrap(), ["the reply"]);
}

#[test]
fn a_local_reply_plays_on_when_a_server_is_turned_on_and_off() {
    let held = crate::test_support::daemon_state_holding_speech_under(
        Config::default(),
        crate::text_to_speech::Speaker::Configured(banshee_common::COMPANION_VOICE.to_string()),
        std::time::Duration::from_secs(30),
        None,
    );
    let spoken_here = building_here(&held.state);
    held.state
        .speech()
        .speak("the local reply", false, None)
        .unwrap();
    configure_beside(
        &held.state,
        "https://a.example.test/v1",
        assignments(&[("tts.remote.api_key", "sk-on-file".into())]),
    );
    configure_beside(
        &held.state,
        "https://a.example.test/v1",
        assignments(&[("tts.provider", "local".into())]),
    );
    assert!(!held.cut_short.load(std::sync::atomic::Ordering::SeqCst));
    assert!(spoken_here.lock().unwrap().is_empty());
}

#[test]
fn a_speaker_key_under_the_local_voice_rebuilds_nothing() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    state.set_last_speech_error(Some("the voice did not load".to_string()));
    let built = counting_builds(&state);
    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("tts.remote.api_key", "sk-b".into())]),
        true,
    )
    .unwrap();
    assert!(built.lock().unwrap().is_empty());
    assert!(!state.speaker_started());
    assert_eq!(
        state.last_speech_error().as_deref(),
        Some("the voice did not load")
    );
    assert_eq!(outcome.applied, vec!["tts.remote.api_key".to_string()]);
}

#[test]
fn a_remote_field_under_the_local_voice_rebuilds_nothing() {
    for (key, value) in [("tts.remote.voice", "marin"), ("tts.fallback", "none")] {
        let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
        state.set_last_speech_error(Some("the voice did not load".to_string()));
        let built = counting_builds(&state);
        let outcome =
            configure_in_scratch(Some(&state), assignments(&[(key, value.into())]), true).unwrap();
        assert!(built.lock().unwrap().is_empty(), "{key}");
        assert!(!state.speaker_started(), "{key}");
        assert_eq!(
            state.last_speech_error().as_deref(),
            Some("the voice did not load"),
            "{key}"
        );
        assert_eq!(outcome.applied, vec![key.to_string()]);
    }
}

#[test]
fn another_voice_on_the_same_server_lets_the_reply_finish() {
    let (held, spoken_here) = speaking_through_a_server("https://a.example.test/v1");
    configure_beside(
        &held.state,
        "https://a.example.test/v1",
        assignments(&[("tts.remote.voice", "marin".into())]),
    );
    assert!(!held.cut_short.load(std::sync::atomic::Ordering::SeqCst));
    assert!(spoken_here.lock().unwrap().is_empty());
    assert_eq!(held.state.tts_voice().as_deref(), Some("marin"));
}

#[test]
fn a_speaker_key_beside_the_server_in_force_rebuilds_the_speaker() {
    let mut config = Config::default();
    config.tts.provider = crate::config::Provider::Remote;
    config.tts.remote.base_url = "https://a.example.test/v1".to_string();
    config.tts.remote.voice = "alloy".to_string();
    let state = crate::test_support::daemon_state_speaking(
        config,
        crate::text_to_speech::Speaker::Fallback,
        std::sync::mpsc::channel().0,
    );
    let built = counting_builds(&state);
    configure_beside(
        &state,
        "https://a.example.test/v1",
        assignments(&[
            ("tts.remote.voice", "alloy".into()),
            ("tts.remote.api_key", "sk-b".into()),
        ]),
    );
    assert_eq!(*built.lock().unwrap(), [crate::config::Provider::Remote]);
    assert!(
        state.speaker_started(),
        "the server the key opens speaks now"
    );
}

#[test]
fn a_reply_from_the_voice_standing_in_finishes_when_the_server_goes() {
    let (held, spoken_here) = speaking_under_a_server(
        "https://a.example.test/v1",
        crate::text_to_speech::Speaker::Fallback,
    );
    configure_beside(
        &held.state,
        "https://a.example.test/v1",
        assignments(&[("tts.provider", "local".into())]),
    );
    assert!(!held.cut_short.load(std::sync::atomic::Ordering::SeqCst));
    assert!(spoken_here.lock().unwrap().is_empty());
}

/// Lines to server A that play until the test sets `ends`.
struct UntilTold {
    spoken: crate::test_support::SpokenLines,
    ends: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl crate::text_to_speech::TtsBackend for UntilTold {
    fn start(
        &self,
        text: &str,
        _voice: Option<&str>,
    ) -> Result<Box<dyn crate::text_to_speech::ActiveUtterance>, BansheeError> {
        self.spoken.lock().unwrap().push(text.to_string());
        Ok(Box::new(crate::test_support::UntilEnds(
            std::sync::Arc::clone(&self.ends),
        )))
    }

    fn sends_to(&self) -> Option<&str> {
        Some("https://a.example.test/v1")
    }
}

/// A daemon speaking through server A with a key on file, switching to
/// server B on a thread. The switch's build waits for the gate.
struct SwitchingToB {
    state: std::sync::Arc<crate::state::DaemonState>,
    spoken_on_a: crate::test_support::SpokenLines,
    spoken_on_b: crate::test_support::SpokenLines,
    ends_on_a: std::sync::Arc<std::sync::atomic::AtomicBool>,
    building: std::sync::mpsc::Receiver<()>,
    gate: std::sync::mpsc::Sender<()>,
}

impl SwitchingToB {
    fn new() -> Self {
        let mut config = Config::default();
        config.tts.provider = crate::config::Provider::Remote;
        config.tts.remote.base_url = "https://a.example.test/v1".to_string();
        config.tts.remote.voice = "alloy".to_string();
        let spoken_on_a = crate::test_support::SpokenLines::default();
        let ends_on_a = std::sync::Arc::default();
        let state = crate::test_support::daemon_state_speaking_on(
            config,
            crate::text_to_speech::Speaker::Configured("alloy".to_string()),
            crate::text_to_speech::SpeechPlayer::new(Box::new(UntilTold {
                spoken: spoken_on_a.clone(),
                ends: std::sync::Arc::clone(&ends_on_a),
            })),
        );
        crate::test_support::write_key(
            state.credentials_path(),
            crate::credentials::RemoteKey::Tts,
            "sk-on-file",
        );
        let spoken_on_b = crate::test_support::SpokenLines::default();
        let lines = spoken_on_b.clone();
        let (build, building, gate) = crate::test_support::gated_builder(move |tts| {
            crate::test_support::speaking_selection(
                lines.clone(),
                crate::text_to_speech::Speaker::Configured(tts.remote.voice.clone()),
            )
        });
        state.speech().set_builder(build);
        Self {
            state,
            spoken_on_a,
            spoken_on_b,
            ends_on_a,
            building,
            gate,
        }
    }

    /// Writes server B on a thread, and returns once the build has begun.
    fn switch(&self) -> std::thread::JoinHandle<()> {
        let state = std::sync::Arc::clone(&self.state);
        let writing = std::thread::spawn(move || {
            configure_beside(
                &state,
                "https://a.example.test/v1",
                assignments(&[("tts.remote.base_url", "https://b.example.test/v1".into())]),
            );
        });
        self.building
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the switch never began its build");
        writing
    }

    fn finish(self, writing: std::thread::JoinHandle<()>) -> Self {
        self.gate.send(()).unwrap();
        writing.join().unwrap();
        self
    }
}

#[test]
fn a_line_queued_behind_one_that_ends_during_a_switch_starts_on_the_new_server() {
    let switching = SwitchingToB::new();
    let speech = std::sync::Arc::clone(switching.state.speech());
    speech.speak("first", false, None).unwrap();
    speech.speak("second", false, None).unwrap();
    let writing = switching.switch();
    switching
        .ends_on_a
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let playing = speech.subscribe_playing();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while playing.borrow().is_some() {
        assert!(std::time::Instant::now() < deadline, "first never ended");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(*switching.spoken_on_a.lock().unwrap(), ["first"]);
    let done = switching.finish(writing);
    assert_eq!(*done.spoken_on_b.lock().unwrap(), ["second"]);
}

#[test]
fn a_line_sent_during_a_switch_starts_on_the_new_server() {
    let switching = SwitchingToB::new();
    let writing = switching.switch();
    switching.state.speech().speak("new", false, None).unwrap();
    assert!(switching.spoken_on_a.lock().unwrap().is_empty());
    let done = switching.finish(writing);
    assert_eq!(*done.spoken_on_b.lock().unwrap(), ["new"]);
}

#[test]
fn a_write_the_daemon_cannot_apply_yet_is_waiting() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let outcome = configure_in_scratch(
        Some(&state),
        assignments(&[("stt.model", TURBO.into())]),
        true,
    )
    .unwrap();
    assert_eq!(
        outcome.waiting,
        vec!["stt.model".to_string()],
        "the scratch models folder holds no turbo"
    );
    assert!(outcome.applied.is_empty());
}

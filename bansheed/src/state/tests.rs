#[test]
fn the_models_wanted_follow_the_preset_the_config_names() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    let mut config = Config::default();
    config.stt.preset = crate::config::STTPreset::Fast;
    state.set_config(std::sync::Arc::new(config));
    let named = |state: &DaemonState| {
        state
            .wanted_downloads()
            .iter()
            .map(|d| d.name.clone())
            .collect::<Vec<_>>()
    };
    assert!(named(&state).contains(&"ggml-base.en.bin".to_string()));

    let mut config = Config::default();
    config.stt.preset = crate::config::STTPreset::Quality;
    state.set_config(std::sync::Arc::new(config));

    let after = named(&state);
    assert!(after.contains(&"ggml-large-v3-q5_0.bin".to_string()));
    assert!(
        !after.contains(&"ggml-base.en.bin".to_string()),
        "the window must stop asking for the model the daemon left behind"
    );
}

#[test]
fn a_preset_change_hands_the_listener_the_model_to_load() {
    let (commands, taken) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);

    assert!(state.load_stt_model("ggml-large-v3-q5_0.bin"));

    match taken.try_recv() {
        Ok(ConsumerCommand::Reload(model)) => assert_eq!(model, "ggml-large-v3-q5_0.bin"),
        _ => panic!("the listener was handed no model"),
    }
}

// A listener that has gone cannot load anything, and saying otherwise would
// report a preset in effect that nothing runs.
#[test]
fn a_preset_change_with_no_listener_is_not_reported_as_taken() {
    let (commands, gone) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    drop(gone);

    assert!(!state.load_stt_model("ggml-large-v3-q5_0.bin"));
}

#[test]
fn the_model_reported_is_the_one_the_listener_loaded() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    assert_eq!(state.stt_model(), "stt");

    state.set_stt_model("ggml-large-v3-q5_0.bin");

    assert_eq!(state.stt_model(), "ggml-large-v3-q5_0.bin");
}

// The off direction alone would pass a `set_history` that always stores
// nothing, so this asks whether a reopened file is reached.
#[test]
fn history_turned_back_on_is_written_to_again() {
    let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
    assert!(!state.history_enabled(), "this state starts with none");

    state.set_history(Some(crate::test_support::seeded_history(&["kept"])));

    assert!(state.history_enabled());
    let rows = state
        .with_history(|c| crate::history::TranscriptionHistory::list(c, None))
        .expect("history is on, so the job must run")
        .expect("the table must be readable");
    assert_eq!(rows.len(), 1, "the reopened file must be the one read");
}

use super::*;

/// A client offers the command behind a Copy button, so `command` has to
/// be the command the sentence names, not a near miss.
#[test]
fn a_recording_error_names_the_same_command_its_sentence_does() {
    for error in [
        RecordingError::Microphone(String::new()),
        RecordingError::Model(String::new()),
    ] {
        let command = error.command().expect("both faults name a command");
        assert!(
            error.fix().ends_with(command),
            "`{}` does not end with `{command}`",
            error.fix()
        );
    }
}

// A watchdog rescans after a fault, but `start_recording` returning Err
// spawns none, so that one microphone fault needs a restart.
#[test]
fn a_microphone_fix_names_the_restart_that_some_faults_need() {
    let fix = RecordingError::Microphone("no device".to_string()).fix();
    assert!(fix.contains("connect the microphone"), "{fix}");
    assert!(fix.contains("[audio] input_device"), "{fix}");
    assert!(
        fix.contains("banshee start"),
        "a fault at startup recovers only on a restart: {fix}"
    );
    // The renderer prints "fix: {}" and adds no period of its own
    assert!(!fix.ends_with('.'), "{fix}");
}

// Hands back the receiver: dropping it discards whatever record_stop queues
fn test_state_with_commands() -> (DaemonState, std::sync::mpsc::Receiver<ConsumerCommand>) {
    let (commands, requests) = std::sync::mpsc::channel();
    let state = DaemonState::new(
        "0.0.0",
        "stt",
        "vad",
        0.5,
        "default".to_string(),
        None,
        crate::text_to_speech::SpeechPlayer::default(),
        commands,
        crate::audio::cues::Cues::silent(),
        BargeInMode::Stop,
    );
    (state, requests)
}

fn test_state() -> DaemonState {
    test_state_with_commands().0
}

// A deaf daemon must refuse the press rather than open a session that no
// consumer thread exists to drain.
#[test]
fn record_start_is_refused_without_a_pipeline() {
    let (state, transcribe_requests) = test_state_with_commands();
    state.set_recording_error(RecordingError::Microphone("no device".to_string()));

    assert!(!state.record_start(TranscribeTarget::Mailbox));
    assert_eq!(state.recording_mode(), RecordingMode::Idle);
    // Nothing may reach the consumer: there is nothing on the other end
    assert!(transcribe_requests.try_recv().is_err());

    // record_stop stays a no-op, so a release keybind cannot wedge the mode
    state.record_stop();
    assert_eq!(state.recording_mode(), RecordingMode::Idle);
}

#[test]
fn recording_error_keeps_the_cause_it_was_given() {
    let state = test_state();
    assert!(state.recording_error().is_none());
    // An armed session is available while nothing is wrong
    assert!(state.arm_for_ask());
    state.set_recording_mode(RecordingMode::Idle);

    state.set_recording_error(RecordingError::Model("missing file".to_string()));
    assert!(matches!(
        state.recording_error(),
        Some(RecordingError::Model(_))
    ));
    // The same gate record_start uses, so ask_user cannot arm a deaf mic
    assert!(!state.arm_for_ask());
    assert_eq!(state.recording_mode(), RecordingMode::Idle);
}

#[test]
fn audio_device_refreshes() {
    let (state, _requests) = test_state_with_commands();
    assert_eq!(state.audio_device(), None);

    state.set_audio_device(Some("OnePlus Buds 3".to_string()));
    assert_eq!(state.audio_device().as_deref(), Some("OnePlus Buds 3"));

    // A rebind depends on a second write landing.
    state.set_audio_device(Some("MacBook Pro Microphone".to_string()));
    assert_eq!(
        state.audio_device().as_deref(),
        Some("MacBook Pro Microphone")
    );

    state.set_audio_device(None);
    assert_eq!(state.audio_device(), None);
}

#[test]
fn missing_device_names_what_the_config_waits_for() {
    let (state, _requests) = test_state_with_commands();
    assert_eq!(state.missing_device(), None);

    state.set_missing_device(Some("yeti".to_string()));
    assert_eq!(state.missing_device().as_deref(), Some("yeti"));

    // A substitute does not block recording, so no blocker appears with it
    assert!(state.recording_error().is_none());
    assert!(state.record_start(TranscribeTarget::Mailbox));
    state.record_stop();

    // The named device returns, so nothing is missing any more
    state.set_missing_device(None);
    assert_eq!(state.missing_device(), None);
}

// A panic here kills capture silently, and a device name has no invariant
// a panic can break, so a poisoned lock must still answer
#[test]
fn the_wanted_device_survives_a_poisoned_lock() {
    let state = Arc::new(test_state());
    let holder = Arc::clone(&state);
    let poisoning = std::thread::spawn(move || {
        let _held = holder.wanted_device.lock().unwrap();
        panic!("the writer died holding the lock");
    });
    assert!(poisoning.join().is_err());

    assert_eq!(state.wanted_device(), "default");
    state.set_wanted_device("yeti".to_string());
    assert_eq!(state.wanted_device(), "yeti");
}

// While a named device stays absent the watchdog rewrites the same name
// every rescan, which is every 5 seconds. Each rewrite that counts as a
// change wakes the push task of every subscriber.
#[test]
fn a_rewritten_device_name_is_not_a_change() {
    let (state, _requests) = test_state_with_commands();
    let mut changes = state.device_changes();
    assert_eq!(*changes.borrow_and_update(), 0);

    state.set_missing_device(Some("yeti".to_string()));
    state.set_missing_device(Some("yeti".to_string()));
    state.set_missing_device(Some("yeti".to_string()));
    assert!(
        changes.has_changed().unwrap(),
        "the first write went unheard"
    );
    assert_eq!(*changes.borrow_and_update(), 1, "a rewrite counted");
    assert!(!changes.has_changed().unwrap());

    state.set_audio_device(Some("MacBook Pro Microphone".to_string()));
    state.set_audio_device(Some("MacBook Pro Microphone".to_string()));
    assert!(
        changes.has_changed().unwrap(),
        "the open device went unheard"
    );
    assert_eq!(*changes.borrow_and_update(), 2, "a rewrite counted");

    // One counter for the whole picture, so either field moving wakes the push
    state.set_missing_device(None);
    assert!(
        changes.has_changed().unwrap(),
        "the device returning went unheard"
    );
    assert_eq!(*changes.borrow_and_update(), 3);
}

#[test]
fn recording_error_clears_when_capture_recovers() {
    let (state, _requests) = test_state_with_commands();
    state.set_recording_error(RecordingError::Microphone("gone".to_string()));
    assert!(state.recording_error().is_some());

    state.clear_recording_error();
    assert!(state.recording_error().is_none());

    // A second fault after a recovery must still report
    state.set_recording_error(RecordingError::Microphone("gone again".to_string()));
    assert!(matches!(
        state.recording_error(),
        Some(RecordingError::Microphone(_))
    ));
}

// A wrong branch here drops the utterance or leaves the mic open
#[test]
fn a_toggle_stops_the_session_a_toggle_started() {
    let (state, requests) = test_state_with_commands();

    assert!(
        state.record_toggle(TranscribeTarget::Dictate),
        "idle: starts"
    );
    assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);

    assert!(
        !state.record_toggle(TranscribeTarget::Dictate),
        "in flight: stops"
    );
    assert_eq!(state.recording_mode(), RecordingMode::Idle);
    assert!(matches!(
        requests.try_recv(),
        Ok(ConsumerCommand::Transcribe(TranscribeTarget::Dictate))
    ));

    // A manual override counts as in flight and ends as a stop
    state.set_recording_mode(RecordingMode::ArmedHold);
    assert!(!state.record_toggle(TranscribeTarget::Dictate));
    assert_eq!(state.recording_mode(), RecordingMode::Armed);
}

// A wrong routing here turns typed-with-the-modifier noise into dictation
#[test]
fn cancel_discards_the_session_instead_of_routing_it() {
    let (state, requests) = test_state_with_commands();

    assert!(state.record_start(TranscribeTarget::Dictate));
    state.record_cancel();
    assert_eq!(state.recording_mode(), RecordingMode::Idle);
    assert!(matches!(requests.try_recv(), Ok(ConsumerCommand::Discard)));
    assert!(requests.try_recv().is_err(), "nothing may be transcribed");

    // Nothing in flight, so a cancel is a no-op, like record_stop
    state.record_cancel();
    assert_eq!(state.recording_mode(), RecordingMode::Idle);
    assert!(requests.try_recv().is_err());

    // A manual override returns to armed and keeps its audio
    state.set_recording_mode(RecordingMode::ArmedHold);
    state.record_cancel();
    assert_eq!(state.recording_mode(), RecordingMode::Armed);
    assert!(
        requests.try_recv().is_err(),
        "the armed session owns the ring"
    );
}

#[test]
fn watchdog_releases_a_push_to_talk_that_never_stopped() {
    let (state, transcribe_requests) = test_state_with_commands();

    assert!(state.record_start(TranscribeTarget::Mailbox));
    assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);

    // Nowhere near the ceiling yet: the mic stays open
    assert!(!state.expire_stuck_recording());
    assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);

    // Bring the deadline forward instead of waiting out MAX_PUSH_TO_TALK
    state
        .push_to_talk_deadline
        .store(0, std::sync::atomic::Ordering::Relaxed);

    // Past it, the mic comes back and the utterance is still transcribed
    assert!(state.expire_stuck_recording());
    assert_eq!(state.recording_mode(), RecordingMode::Idle);
    assert!(matches!(
        transcribe_requests.try_recv(),
        Ok(ConsumerCommand::Transcribe(TranscribeTarget::Mailbox))
    ));

    // And a fresh start is accepted rather than refused as busy
    assert!(state.record_start(TranscribeTarget::Mailbox));
}

#[test]
fn watchdog_leaves_armed_listening_alone() {
    let state = test_state();
    // ask_user sessions run their own timeouts; the watchdog must not
    // yank the microphone out from under one
    state.set_recording_mode(RecordingMode::Armed);
    state
        .push_to_talk_deadline
        .store(0, std::sync::atomic::Ordering::Relaxed);
    assert!(!state.expire_stuck_recording());
    assert_eq!(state.recording_mode(), RecordingMode::Armed);
}

// Three writers move the mic: the hotkey and the record RPCs through
// try_transition, the consumer thread through a direct write.
#[test]
fn every_path_that_moves_the_mic_publishes_it() {
    let (state, _requests) = test_state_with_commands();
    let mut recording = state.subscribe_recording();
    assert!(!*recording.borrow_and_update());

    assert!(state.record_start(TranscribeTarget::Mailbox));
    assert!(
        recording.has_changed().unwrap(),
        "record_start went unheard"
    );
    assert!(*recording.borrow_and_update());

    state.record_stop();
    assert!(recording.has_changed().unwrap(), "record_stop went unheard");
    assert!(!*recording.borrow_and_update());

    state.set_recording_mode(RecordingMode::Armed);
    assert!(
        recording.has_changed().unwrap(),
        "a direct write went unheard"
    );
    assert!(*recording.borrow_and_update());
}

#[test]
fn the_voice_reported_is_the_one_the_daemon_loaded() {
    let state = test_state();
    assert_eq!(state.tts_voice(), None);
    state.set_tts_voice("af_sky".to_string());
    assert_eq!(state.tts_voice().as_deref(), Some("af_sky"));

    state.set_tts_voice("am_adam".to_string());

    assert_eq!(
        state.tts_voice().as_deref(),
        Some("am_adam"),
        "a live voice change must move the voice the window marks as current"
    );
}

#[test]
fn recording_mode_roundtrips_and_derives_is_recording() {
    let state = test_state();
    assert_eq!(state.recording_mode(), RecordingMode::Idle);
    assert!(!state.is_recording());
    for mode in [RecordingMode::PushToTalk, RecordingMode::Armed] {
        state.set_recording_mode(mode);
        assert_eq!(state.recording_mode(), mode);
        assert!(state.is_recording());
    }
}

#[test]
fn ring_evicts_oldest_and_filters_by_cursor() {
    let state = test_state();
    for i in 1..=20 {
        state.push_transcription(format!("utterance {i}"));
    }

    let all = state.transcriptions_since(0);
    assert_eq!(all.len(), TRANSCRIPTION_RING_CAPACITY);
    assert_eq!(all.first().unwrap().id, 5);
    assert_eq!(all.last().unwrap().id, 20);

    let newer = state.transcriptions_since(18);
    assert_eq!(newer.len(), 2);
    assert_eq!(newer[0].text, "utterance 19");

    assert!(state.transcriptions_since(20).is_empty());
    assert_eq!(*state.subscribe_transcriptions().borrow(), 20);
}

#[test]
fn capture_is_stalled_until_the_callback_stamps_it() {
    let (state, _requests) = test_state_with_commands();
    // Nothing has captured yet, so a watchdog must not call this healthy
    assert!(state.capture_is_stalled());

    state.mark_capture_alive();
    assert!(!state.capture_is_stalled());
}

#[test]
fn the_silence_limit_clears_every_measured_callback_rate() {
    // Measured: 55 callbacks per second on Bluetooth, 93 on the built in mic.
    // The slowest gives the longest gap, so it sets the headroom.
    let slowest_gap = Duration::from_millis(1000 / 55);
    assert!(
        CAPTURE_SILENCE_LIMIT >= slowest_gap * 20,
        "the limit must keep well clear of a healthy gap, or a busy \
             machine trips it"
    );
}

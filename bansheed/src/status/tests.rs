use super::{BansheeError, microphone_line, report_probe};

const DAEMON_HOLDS: &str = "daemon has the microphone";

#[test]
fn the_microphone_line_leads_with_who_holds_the_device() {
    assert_eq!(
        microphone_line(DAEMON_HOLDS, Some("MacBook Pro Microphone"), Some("yeti")),
        "daemon has the microphone: MacBook Pro Microphone (waiting for \"yeti\")"
    );
}

#[test]
fn a_daemon_with_no_stream_open_fails_and_names_the_download() {
    let models = crate::models::blockers(&["no-such-model-9f3a.bin"]);

    assert!(!super::report_open(
        &serde_json::json!({ "audio_device": null }),
        &models
    ));
    assert_eq!(super::open_fix(&models), "run: banshee setup");
}

#[test]
fn nothing_open_and_no_model_blocker_falls_back_to_the_microphone_fix() {
    assert_eq!(super::open_fix(&[]), super::MICROPHONE_FIX);
}

#[test]
fn a_model_failure_always_leaves_a_model_blocker_to_borrow_the_fix_from() {
    let (commands, _drain) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    state.set_pipeline(crate::state::Pipeline::Broken(
        crate::state::RecordingError::Model("missing file.".to_string()),
    ));

    let blockers = crate::readiness::blockers(&state, &state.pipeline());
    let model = blockers
        .iter()
        .find(|blocker| blocker.kind == banshee_common::BlockerKind::Model)
        .expect("a model failure leaves a model blocker");

    assert_eq!(super::open_fix(&blockers), model.fix);
}

#[test]
fn an_unreadable_key_file_is_a_recording_fault_the_checklist_names() {
    let (commands, _drain) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    state.set_pipeline(crate::state::Pipeline::Broken(
        crate::state::RecordingError::KeyFile("credentials.toml does not parse".to_string()),
    ));

    let daemon = super::Daemon::Running {
        status: serde_json::json!({ "audio_device": "MacBook Pro Microphone" }),
        blockers: crate::readiness::blockers(&state, &state.pipeline()),
    };

    assert!(!super::check_recording(&daemon, ""));
}

#[test]
fn a_daemon_that_names_its_device_passes() {
    assert!(super::report_open(
        &serde_json::json!({ "audio_device": "MacBook Pro Microphone" }),
        &[]
    ));
}

// No daemon holds the device, so the checklist opens it. It selects the way
// capture does, so a substitute is a working machine, not a broken one.
#[test]
fn a_probed_substitute_passes_and_names_what_it_waits_for() {
    assert!(report_probe(Ok((
        "MacBook Pro Microphone".to_string(),
        Some("oneplus".to_string())
    ))));
}

#[test]
fn a_microphone_that_will_not_open_fails_the_checklist() {
    assert!(!report_probe(Err(
        "no input device is available".to_string()
    )));
}

#[test]
fn the_last_error_line_names_no_producer() {
    assert_eq!(
        super::last_error_line("opencode exited exit status: 1"),
        "the last attempt failed: opencode exited exit status: 1"
    );
}

// The checklist names the host either way, because that is the server the
// config asks for. Only the daemon says whether text reaches it.
#[test]
fn the_speech_note_says_text_stays_here_until_the_speaker_starts() {
    assert_eq!(
        super::speech_line("api.openai.com", true),
        "text goes to api.openai.com for speaking"
    );
    assert_eq!(
        super::speech_line("api.openai.com", false),
        "the speaker on api.openai.com did not start, so text stays on this machine"
    );
}

// Each speaker keeps its voice in its own table, and the settings line has
// room for one. `tts.voice` is Kokoro's, and a remote server has never heard
// of it.
#[test]
fn the_settings_line_names_the_voice_of_the_speaker_in_force() {
    let mut config = crate::config::Config::default();
    config.tts.voice = "af_sky".to_string();
    config.tts.remote.voice = "marin".to_string();

    assert_eq!(
        super::settings_voice(&config, true),
        config.tts.remote.voice
    );
    assert_eq!(super::settings_voice(&config, false), config.tts.voice);
}

#[test]
fn the_tell_line_says_whether_the_agent_is_scoped() {
    use crate::tell::Headless;

    assert_eq!(
        super::tell_line(Ok(Headless::ClaudeCode)),
        "tell runs claude, and it gets the folders in tell.paths, and its own run directory"
    );
    assert_eq!(
        super::tell_line(Ok(Headless::OpenCode)),
        "tell runs opencode, which takes no folder list, so it can edit any file"
    );
}

#[test]
fn the_checklist_names_none_as_the_mode_with_no_cue() {
    use crate::config::FeedbackMode;
    let none = super::silent_tell_line(true, FeedbackMode::Off).expect("none must be named");
    assert!(
        none.contains("makes no sound") && none.contains("feedback.mode"),
        "the line must say what is lost and which key restores it: {none}"
    );
    for heard in [FeedbackMode::Sound, FeedbackMode::Both] {
        assert_eq!(
            super::silent_tell_line(true, heard),
            None,
            "{heard:?} sounds a tell"
        );
    }
    assert_eq!(
        super::silent_tell_line(false, FeedbackMode::Off),
        None,
        "no agent can fail, and the line above already says so"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn visual_names_the_figure_on_macos() {
    use crate::config::FeedbackMode;
    let visual = super::silent_tell_line(true, FeedbackMode::Visual).expect("visual must be named");
    assert!(
        visual.contains("makes no sound") && visual.contains("figure"),
        "the line must say the figure stands in for the sound: {visual}"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn visual_sounds_a_tell_with_no_figure_to_show_it() {
    use crate::config::FeedbackMode;
    assert_eq!(
        super::silent_tell_line(true, FeedbackMode::Visual),
        None,
        "with no figure, visual plays every sound"
    );
}

#[test]
fn a_tell_line_with_no_agent_carries_the_reason_whole() {
    let refused = BansheeError::Rejected("no connected agent. Run: banshee connect claude".into());

    assert_eq!(
        super::tell_line(Err(refused)),
        "tell has no agent: no connected agent. Run: banshee connect claude"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_key_press_note_states_what_the_daemon_measured() {
    use super::{Access, key_press_line};

    assert_eq!(
        key_press_line(Access::Granted),
        "the daemon can receive key presses"
    );
    assert_eq!(
        key_press_line(Access::Denied),
        "the daemon cannot receive key presses"
    );
    assert_eq!(
        key_press_line(Access::Undetermined),
        "macOS has not decided whether the daemon can receive key presses"
    );
    for access in [Access::Granted, Access::Denied, Access::Undetermined] {
        assert!(
            !key_press_line(access).contains("System Settings"),
            "the note names no pane: a person cannot grant this one"
        );
    }
}

// The walk hands back a path to run, so a file it cannot run is not an
// answer: a bare name let the kernel skip one.
#[test]
fn resolve_skips_a_file_it_cannot_run() {
    let root = std::env::temp_dir().join(format!("banshee-resolve-{}", std::process::id()));
    let (early, late) = (root.join("early"), root.join("late"));
    std::fs::create_dir_all(&early).unwrap();
    std::fs::create_dir_all(&late).unwrap();
    std::fs::write(early.join("claude"), "not a program").unwrap();
    crate::test_support::write_executable(&late.join("claude"), "#!/bin/sh\n");

    let path = std::ffi::OsString::from(format!("{}:{}", early.display(), late.display()));
    assert_eq!(super::resolve("claude", &path), Some(late.join("claude")));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_daemon_that_is_not_running_fails_the_checklist() {
    assert!(!super::report_daemon(&super::Daemon::Missing));
    assert!(!super::report_daemon(&super::Daemon::Stale));
}

use super::{Daemon, classify};

#[test]
fn a_reply_without_blockers_answers_nothing_the_checklist_can_read() {
    let reply = serde_json::json!({"running": true, "version": "0.7.0"});
    assert!(matches!(classify(reply), Daemon::Silent(_)));
}

// A field that is present but unreadable is this build failing to parse a
// daemon, which no restart of an older one explains.
#[test]
fn a_reply_with_unreadable_blockers_is_not_an_older_daemon() {
    let reply = serde_json::json!({"blockers": [{"kind": "moonbeam"}]});
    assert!(matches!(classify(reply), Daemon::Silent(_)));
}

#[test]
fn a_reply_with_blockers_carries_them_decoded() {
    let reply = serde_json::json!({"blockers": [{
        "kind": "model", "id": "m.bin", "name": "m.bin",
        "consequence": "nothing works", "fix": "run: banshee setup",
    }]});
    let Daemon::Running { blockers, .. } = classify(reply) else {
        panic!("a decodable blockers field must not read as an older daemon");
    };
    assert_eq!(blockers.len(), 1);
}

#[test]
fn an_empty_blockers_list_is_not_the_same_as_no_field() {
    let reply = serde_json::json!({"blockers": []});
    assert!(
        matches!(classify(reply), Daemon::Running { .. }),
        "a daemon reporting nothing wrong is not a daemon that reported nothing"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn an_install_inside_a_bundle_is_named_as_one() {
    let bundled = std::path::Path::new("/Users/x/Applications/Banshee.app/Contents/MacOS/banshee");
    let loose = std::path::Path::new("/Users/x/.cargo/bin/banshee");

    assert_eq!(super::install_shape(bundled), "Banshee.app");
    assert_eq!(super::install_shape(loose), "a loose binary");
}

// The socket answers from `claim()`, before the device is open. Calling that
// gap a missing microphone sends the reader after a fault that is not there.
#[test]
fn a_pipeline_still_opening_is_not_a_missing_microphone() {
    let opening = serde_json::json!({ "pipeline": "opening" });
    assert!(super::still_opening(&opening).is_some());

    for settled in ["open", "broken"] {
        let status = serde_json::json!({ "pipeline": settled });
        assert!(
            super::still_opening(&status).is_none(),
            "{settled} is answered by the lines below, not by a wait"
        );
    }
}

#[test]
fn a_daemon_still_opening_its_microphone_is_healthy() {
    let daemon = super::Daemon::Running {
        status: serde_json::json!({ "pipeline": "opening" }),
        blockers: Vec::new(),
    };
    assert!(
        super::check_recording(&daemon, "default"),
        "waiting is not a failed check"
    );
}

// A clean `kill` leaves the same file a crash does, so the word states more
// than the file can carry.
#[test]
fn a_socket_left_behind_is_not_called_a_crash() {
    let (line, fix) = super::absence(&Daemon::Stale).expect("a stale socket is an absence");
    assert!(!line.contains("crash"), "{line}");
    assert!(fix.contains("banshee start"), "{fix}");
}

// The daemon binds its socket before it can answer, so silence is usually a
// start in progress. Restarting it first only starts that wait again.
#[test]
fn a_daemon_that_does_not_answer_is_offered_the_retry_before_the_restart() {
    let reason = "nothing within 2s".to_string();
    let (line, fix) = super::absence(&Daemon::Silent(reason)).expect("silence is an absence");
    assert!(line.contains("nothing within 2s"), "{line}");
    assert!(fix.contains("again"), "{fix}");
}

#[test]
fn a_daemon_that_answers_is_no_absence() {
    let running = Daemon::Running {
        status: serde_json::json!({}),
        blockers: Vec::new(),
    };
    assert!(super::absence(&running).is_none());
}

#[test]
fn a_speaker_that_shares_the_microphone_device_says_what_it_costs() {
    assert_eq!(
        super::shared_device("OnePlus Buds 3", Some(("OnePlus Buds 3".to_string(), 16_000))),
        Some(
            "the default speaker is this microphone's own device, and it plays at 16000 Hz while Banshee listens"
                .to_string()
        )
    );
}

#[test]
fn a_speaker_of_its_own_costs_the_microphone_nothing() {
    assert_eq!(
        super::shared_device(
            "MacBook Pro Microphone",
            Some(("MacBook Pro Speakers".to_string(), 48_000))
        ),
        None
    );
}

// A device that records and plays without dropping its rate has nothing to
// report, and a line that fires anyway is noise on every headset that works.
#[test]
fn one_device_at_full_rate_is_not_worth_a_line() {
    assert_eq!(
        super::shared_device(
            "Studio Display",
            Some(("Studio Display".to_string(), 48_000))
        ),
        None
    );
}

#[test]
fn nothing_is_said_when_no_speaker_answers() {
    assert_eq!(super::shared_device("Buds", None), None);
}

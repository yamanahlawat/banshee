use super::{Line, check_remote_side, microphone_lines, report_probe};
use crate::credentials::{Credentials, RemoteKey};

const BUILT_IN: &str = "MacBook Pro Microphone";

fn microphone(status: serde_json::Value, blockers: &[banshee_common::Blocker]) -> Vec<Line> {
    microphone_lines(&status, blockers, None)
}

fn pass(text: &str) -> Line {
    Line::Pass(text.to_string())
}

fn note(text: &str) -> Line {
    Line::Note(text.to_string())
}

// An idle daemon has nothing open and nothing wrong: the next use opens the
// device it names.
#[test]
fn an_idle_microphone_passes_naming_the_device_the_next_use_opens() {
    assert_eq!(
        microphone(
            serde_json::json!({ "audio_device": BUILT_IN, "microphone_open": false }),
            &[]
        ),
        [pass(
            "microphone: MacBook Pro Microphone, opens when you speak"
        )]
    );
    assert_eq!(
        microphone(
            serde_json::json!({
                "audio_device": BUILT_IN,
                "missing_device": "yeti",
                "microphone_open": false,
            }),
            &[]
        ),
        [pass(
            "microphone: MacBook Pro Microphone (waiting for \"yeti\"), opens when you speak"
        )]
    );
}

#[test]
fn an_open_microphone_says_so_and_keep_open_says_why_it_stays() {
    let open = |keep_open: bool| {
        microphone(
            serde_json::json!({
                "audio_device": BUILT_IN,
                "microphone_open": true,
                "config": { "audio": { "keep_open": keep_open } },
            }),
            &[],
        )
    };
    assert_eq!(
        open(false),
        [pass("microphone: MacBook Pro Microphone, open")]
    );
    assert_eq!(
        open(true),
        [
            pass("microphone: MacBook Pro Microphone, open"),
            note("audio.keep_open is on, so the microphone stays open between uses"),
        ]
    );
}

#[test]
fn a_model_loading_is_named_beside_the_microphone() {
    assert_eq!(
        microphone(
            serde_json::json!({
                "audio_device": BUILT_IN,
                "microphone_open": false,
                "loading_model": true,
            }),
            &[]
        ),
        [
            pass("microphone: MacBook Pro Microphone, opens when you speak"),
            note("loading the listening model"),
        ]
    );
}

#[test]
fn a_remote_listener_loading_names_the_detector_it_loads() {
    assert_eq!(
        microphone(
            serde_json::json!({
                "audio_device": BUILT_IN,
                "microphone_open": false,
                "loading_model": true,
                "remote": { "stt": { "remote": true } },
            }),
            &[]
        )[1],
        note("loading the voice detection model")
    );
}

#[test]
fn no_device_to_open_fails_with_the_fix_and_borrows_a_model_blockers() {
    let models = crate::models::blockers(&["no-such-model-9f3a.bin"]);
    assert_eq!(
        microphone(serde_json::json!({ "audio_device": null }), &[]),
        [Line::Fail(
            "no microphone is available to open".to_string(),
            super::MICROPHONE_FIX.to_string()
        )]
    );
    assert_eq!(
        microphone(serde_json::json!({ "audio_device": null }), &models),
        [Line::Fail(
            "no microphone is available to open".to_string(),
            "run: banshee setup".to_string()
        )]
    );
}

// The grant check fails with the fix, so the microphone line says only what
// the device waits on, and never that it opens when you speak.
#[test]
fn a_refused_microphone_is_not_reported_as_one_that_opens() {
    assert_eq!(
        microphone(
            serde_json::json!({
                "audio_device": BUILT_IN,
                "microphone_open": false,
                "microphone_refused": true,
            }),
            &[]
        ),
        [note(
            "microphone: MacBook Pro Microphone, not allowed by macOS"
        )]
    );
}

#[test]
fn a_model_failure_always_leaves_a_model_blocker_to_borrow_the_fix_from() {
    let (commands, _drain) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state(commands);
    state.set_mic(crate::state::Mic::Faulted(
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
    state.set_mic(crate::state::Mic::Faulted(
        crate::state::RecordingError::KeyFile("credentials.toml does not parse".to_string()),
    ));

    let daemon = super::Daemon::Running {
        status: serde_json::json!({ "audio_device": "MacBook Pro Microphone" }),
        blockers: crate::readiness::blockers(&state, &state.pipeline()),
    };

    assert!(!super::check_recording(&daemon, ""));
}

// No daemon holds the device, so the checklist opens it. It selects the way
// capture does, so a substitute is a working machine, not a broken one.
#[test]
fn a_probed_substitute_passes_and_a_microphone_that_will_not_open_fails() {
    let cases = [
        (
            "a substitute naming what it waits for",
            Ok((
                "MacBook Pro Microphone".to_string(),
                Some("oneplus".to_string()),
            )),
            true,
        ),
        (
            "no input device",
            Err("no input device is available".to_string()),
            false,
        ),
    ];

    for (name, probed, passes) in cases {
        assert_eq!(report_probe(probed), passes, "{name}");
    }
}

#[test]
fn a_removed_section_is_named_on_the_status_line() {
    let dir = crate::test_support::unique_scratch("status-config");
    let path = dir.join("config.toml");
    std::fs::write(&path, "[tell]\nagent = \"claude\"\n").unwrap();

    let error = crate::config::Config::load_from(&path).expect_err("[tell] is refused");
    let line = super::invalid_config_line(&error);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(line.contains("`tell`"), "{line}");
}

#[test]
fn the_fix_for_a_refused_config_keeps_the_rest_of_the_file() {
    let fix = super::invalid_config_fix("/home/me/.banshee/config.toml");
    assert!(fix.contains("/home/me/.banshee/config.toml"), "{fix}");
    assert!(!fix.contains("delete it"), "{fix}");
}

#[test]
fn translate_on_a_model_that_cannot_translate_is_named() {
    let mut config = crate::config::Config::default();
    assert_eq!(super::translate_line(&config), None, "translate is off");
    config.stt.translate = true;
    let line = super::translate_line(&config).expect("Parakeet does not translate");
    assert!(line.contains("stt.translate"), "{line}");
    assert!(line.contains("parakeet-unified-en-0.6b"), "{line}");
    config.stt.provider = crate::config::Provider::Remote;
    assert_eq!(
        super::translate_line(&config),
        None,
        "a transcription server is asked to translate"
    );
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
// room for one. `tts.voice` is the local voice's, and a remote server has
// never heard of it.
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
fn the_local_voice_line_names_the_voice_and_its_engine() {
    let models = crate::test_support::companion_models_with(&["af_sky"]);

    let (companion, fix) = super::local_voice_check("companion", &models);
    assert_eq!(fix, None, "{companion}");
    assert!(
        companion.contains("companion") && companion.contains("Pocket"),
        "{companion}"
    );
    let (sky, fix) = super::local_voice_check("af_sky", &models);
    assert_eq!(fix, None, "{sky}");
    assert!(sky.contains("af_sky") && sky.contains("Kokoro"), "{sky}");
}

#[test]
fn a_missing_companion_says_run_banshee_setup() {
    let models = crate::test_support::unique_scratch("status-models");

    let (line, fix) = super::local_voice_check("companion", &models);
    assert_eq!(fix, Some("run: banshee setup"), "{line}");
    assert!(line.contains("companion"), "{line}");
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

// A field that is present but unreadable is this build failing to parse a
// daemon, which no restart of an older one explains.
#[test]
fn classify_reads_the_blockers_field_by_what_it_holds() {
    let decodable = serde_json::json!({"blockers": [{
        "kind": "model", "id": "m.bin", "name": "m.bin",
        "consequence": "nothing works", "fix": "run: banshee setup",
    }]});
    let cases = [
        (
            "no blockers field",
            serde_json::json!({"running": true, "version": "0.7.0"}),
            None,
            "an older daemon answers nothing the checklist can read",
        ),
        (
            "unreadable blockers",
            serde_json::json!({"blockers": [{"kind": "moonbeam"}]}),
            None,
            "an unreadable field is not an older daemon",
        ),
        (
            "decodable blockers",
            decodable,
            Some(1),
            "a decodable blockers field must not read as an older daemon",
        ),
        (
            "an empty list is not the same as no field",
            serde_json::json!({"blockers": []}),
            Some(0),
            "a daemon reporting nothing wrong is not a daemon that reported nothing",
        ),
    ];

    for (name, reply, decoded, why) in cases {
        match (classify(reply), decoded) {
            (Daemon::Silent(_), None) => {}
            (Daemon::Running { blockers, .. }, Some(count)) => {
                assert_eq!(blockers.len(), count, "{name}: {why}")
            }
            _ => panic!("{name}: {why}"),
        }
    }
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
// than the file can carry. The daemon binds its socket before it can answer,
// so silence is usually a start in progress, and restarting it first only
// starts that wait again.
#[test]
fn an_absence_is_named_without_a_guess_and_a_running_daemon_is_none() {
    let running = Daemon::Running {
        status: serde_json::json!({}),
        blockers: Vec::new(),
    };
    let cases = [
        ("a stale socket", Daemon::Stale, Some(("", "banshee start"))),
        (
            "no answer",
            Daemon::Silent("nothing within 2s".to_string()),
            Some(("nothing within 2s", "again")),
        ),
        ("a daemon that answers", running, None),
    ];

    for (name, daemon, want) in cases {
        match (super::absence(&daemon), want) {
            (Some((line, fix)), Some((line_has, fix_has))) => {
                assert!(!line.contains("crash"), "{name}: {line}");
                assert!(line.contains(line_has), "{name}: {line}");
                assert!(fix.contains(fix_has), "{name}: {fix}");
            }
            (None, None) => {}
            _ => panic!("{name}"),
        }
    }
}

// A device that records and plays without dropping its rate has nothing to
// report, and a line that fires anyway is noise on every headset that works.
#[test]
fn a_shared_device_is_reported_only_when_the_speaker_drops_its_rate() {
    let cases = [
        (
            "a shared device at a low rate",
            "OnePlus Buds 3",
            Some(("OnePlus Buds 3", 16_000)),
            Some(
                "the default speaker is this microphone's own device, and it plays at 16000 Hz while Banshee listens",
            ),
        ),
        (
            "a speaker of its own",
            "MacBook Pro Microphone",
            Some(("MacBook Pro Speakers", 48_000)),
            None,
        ),
        (
            "one device at full rate",
            "Studio Display",
            Some(("Studio Display", 48_000)),
            None,
        ),
        ("no speaker answers", "Buds", None, None),
    ];

    for (name, microphone, speaker, line) in cases {
        let speaker = speaker.map(|(device, rate)| (device.to_string(), rate));
        assert_eq!(
            super::shared_device(microphone, speaker),
            line.map(str::to_string),
            "{name}"
        );
    }
}

#[test]
fn plain_http_to_the_internet_fails_its_side() {
    let credentials = Credentials {
        stt_api_key: Some("sk-test".to_string()),
        ..Credentials::default()
    };
    let side = |base_url| check_remote_side(RemoteKey::Stt, base_url, Ok(&credentials), None);
    assert!(!side("http://api.example.com/v1"));
    assert!(side("https://api.example.com/v1"));
}

#[test]
fn a_closed_microphone_gets_no_speaker_note() {
    let status = serde_json::json!({ "audio_device": BUILT_IN, "microphone_open": false });
    let lines = microphone_lines(&status, &[], Some((BUILT_IN.to_string(), 16_000)));
    assert_eq!(
        lines,
        [pass(
            "microphone: MacBook Pro Microphone, opens when you speak"
        )]
    );
}

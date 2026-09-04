use super::{microphone_line, report_probe};

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
    state.set_recording_error(crate::state::RecordingError::Model(
        "missing file.".to_string(),
    ));

    assert_ne!(
        super::open_fix(&crate::readiness::blockers(&state)),
        super::MICROPHONE_FIX
    );
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
    use std::os::unix::fs::PermissionsExt;

    let root = std::env::temp_dir().join(format!("banshee-resolve-{}", std::process::id()));
    let (early, late) = (root.join("early"), root.join("late"));
    std::fs::create_dir_all(&early).unwrap();
    std::fs::create_dir_all(&late).unwrap();
    std::fs::write(early.join("claude"), "not a program").unwrap();
    std::fs::write(late.join("claude"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(late.join("claude"), std::fs::Permissions::from_mode(0o755)).unwrap();

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
fn a_reply_without_blockers_is_an_older_daemon() {
    let reply = serde_json::json!({"running": true, "version": "0.7.0"});
    assert!(matches!(classify(reply), Daemon::Legacy(_)));
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

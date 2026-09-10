use super::{progress_line, state_word, watch_line, waybar_line};
use banshee_common::InputDevice;

#[test]
fn a_message_from_a_daemon_that_predates_these_fields_still_shows_progress() {
    let old_format = serde_json::json!({
        "model": "silero_vad.onnx",
        "bytes": 356,
        "total": 574,
        "state": "downloading",
    });
    let reported: banshee_common::DownloadProgress =
        serde_json::from_value(old_format).expect("old-format messages must still deserialize");

    let line = progress_line(&reported);
    assert!(!line.contains("of 0"), "{line}");
    assert!(!line.contains("  "), "{line}");
    assert_eq!(line, "silero_vad.onnx 62%");

    let new_format = banshee_common::DownloadProgress {
        model: "silero_vad.onnx".to_string(),
        label: "Voice detection model".to_string(),
        index: 1,
        count: 3,
        bytes: 356,
        total: Some(574),
        state: banshee_common::DownloadState::Downloading,
    };
    let line = progress_line(&new_format);
    assert!(line.contains("Voice detection model"), "{line}");
    assert!(line.contains("1 of 3"), "{line}");
}

#[test]
fn a_waybar_line_is_one_parseable_object() {
    let line = waybar_line("recording", Some("Blue Yeti"), None);
    assert!(
        !line.contains('\n'),
        "Waybar reads one object per line: {line}"
    );

    let parsed: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
    assert_eq!(parsed["text"], "recording");
    assert_eq!(parsed["alt"], "recording", "format-icons keys on alt");
    assert_eq!(parsed["class"], "recording", "CSS keys on class");
    assert!(
        parsed["tooltip"].as_str().unwrap().contains("Blue Yeti"),
        "{parsed}"
    );
}

// A device name is whatever the hardware calls itself, so it has to be
// escaped rather than pasted into the line
#[test]
fn a_quote_in_the_device_name_does_not_break_the_line() {
    let line = waybar_line("idle", Some("Bob\"s \\ Mic"), None);
    let parsed: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
    assert!(
        parsed["tooltip"]
            .as_str()
            .unwrap()
            .contains("Bob\"s \\ Mic")
    );
}

#[test]
fn an_unknown_device_is_left_out_rather_than_named_empty() {
    let line = waybar_line("idle", None, None);
    let parsed: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
    let tooltip = parsed["tooltip"].as_str().unwrap();
    assert!(
        !tooltip.contains("Microphone"),
        "no device means none is named: {tooltip}"
    );
    assert!(tooltip.contains("idle"), "{tooltip}");
}

// A bar reader has the tooltip only, so a substitution has to show there or
// the bar names a device that is gone
#[test]
fn a_waybar_tooltip_names_what_the_config_still_waits_for() {
    let line = waybar_line("idle", None, Some("Yeti Nano"));
    let parsed: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
    let tooltip = parsed["tooltip"].as_str().unwrap();
    assert!(tooltip.contains("Not open"), "{tooltip}");
    assert!(
        !tooltip.contains("No microphone"),
        "a closed stream is not an absent device: {tooltip}"
    );
    assert!(tooltip.contains("\"Yeti Nano\""), "{tooltip}");
}

// The reader sees the line, so the dedupe compares lines. A plain reader
// cannot see the device, so the same word twice would be noise.
#[test]
fn a_device_change_alone_moves_the_line_only_where_the_device_shows() {
    let plain_bound = watch_line(false, "idle", Some("Yeti Nano"), None);
    let plain_substituted = watch_line(
        false,
        "idle",
        Some("MacBook Pro Microphone"),
        Some("Yeti Nano"),
    );
    assert_eq!(
        plain_bound, plain_substituted,
        "plain mode prints the word alone"
    );

    let waybar_bound = watch_line(true, "idle", Some("Yeti Nano"), None);
    let waybar_substituted = watch_line(
        true,
        "idle",
        Some("MacBook Pro Microphone"),
        Some("Yeti Nano"),
    );
    assert_ne!(
        waybar_bound, waybar_substituted,
        "the tooltip carries the device"
    );

    // The loop starts from an empty line, which must print
    assert!(!plain_bound.is_empty());
    assert!(!waybar_bound.is_empty());
}

#[test]
fn the_microphone_outranks_the_speaker_in_one_word() {
    let state =
        |recording, speaking| serde_json::json!({"recording": recording, "speaking": speaking});
    assert_eq!(state_word(&state(false, false)), "idle");
    assert_eq!(state_word(&state(true, false)), "recording");
    assert_eq!(state_word(&state(false, true)), "speaking");
    // Both at once, when barge-in is off: the mic is what the user waits on
    assert_eq!(state_word(&state(true, true)), "recording");
}

// An older daemon says less than this build reads
#[test]
fn a_state_missing_its_fields_reads_as_idle() {
    assert_eq!(state_word(&serde_json::json!({})), "idle");
}

fn device(name: &str, default: bool) -> InputDevice {
    InputDevice {
        name: name.to_string(),
        default,
    }
}

#[test]
fn the_recording_device_carries_both_labels_when_it_is_also_the_preference() {
    assert_eq!(
        super::device_labels(&device("Blue Yeti", true), Some("Blue Yeti")),
        "system default, in use"
    );
}

#[test]
fn a_device_the_daemon_passed_over_keeps_its_preference_label() {
    assert_eq!(
        super::device_labels(&device("Built-in", true), Some("Blue Yeti")),
        "system default"
    );
    assert_eq!(
        super::device_labels(&device("Blue Yeti", false), Some("Blue Yeti")),
        "in use"
    );
}

#[test]
fn a_device_nothing_points_at_carries_no_label() {
    assert_eq!(super::device_labels(&device("BlackHole", false), None), "");
}

#[test]
fn no_daemon_means_no_in_use_label_even_for_the_preference() {
    assert_eq!(
        super::device_labels(&device("Built-in", true), None),
        "system default",
        "a device nobody opened must not read as recording"
    );
}

// The prompt cannot say "remove it", so an empty answer must not be read as one
#[test]
fn nothing_typed_at_the_key_prompt_leaves_the_key_on_file_alone() {
    assert_eq!(super::key_change(None, || Ok(String::new())).unwrap(), None);
}

// Removal is the argument a person can only mean on purpose
#[test]
fn an_empty_argument_removes_the_key() {
    assert_eq!(
        super::key_change(Some(String::new()), || unreachable!()).unwrap(),
        Some(String::new())
    );
}

// A coercion through serde_json turns an all-digit token into a number and
// strips the quotes off a quoted one
#[test]
fn a_key_is_taken_as_it_was_typed() {
    assert_eq!(
        super::key_change(None, || Ok("12345".to_string())).unwrap(),
        Some("12345".to_string())
    );
    assert_eq!(
        super::key_change(Some("\"sk-test\"".to_string()), || unreachable!()).unwrap(),
        Some("\"sk-test\"".to_string())
    );
}

// The prompt names the side, so a person answering two in a row knows which
// one is being asked for
#[test]
fn each_side_has_its_own_key_prompt() {
    use crate::credentials::RemoteKey;
    assert_eq!(
        super::key_prompt(RemoteKey::Stt),
        "Key for the remote listener (not shown): "
    );
    assert_eq!(
        super::key_prompt(RemoteKey::Tts),
        "Key for the remote speaker (not shown): "
    );
}

// The speaker is worth turning on only once it has a voice: the endpoint has no
// default one, so an empty voice would refuse the backend at every startup.
#[test]
fn the_speaker_is_only_switched_on_once_it_has_a_voice() {
    assert!(super::speaker_sends_text_out("marin"));
    assert!(!super::speaker_sends_text_out(""));
}

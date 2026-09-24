use super::{failure_line, progress_line, state_word, watch_line, waybar_line};
use banshee_common::InputDevice;
use banshee_common::error::BansheeError;

#[test]
fn a_download_line_names_the_file_and_its_place_in_the_run() {
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

#[test]
fn a_failed_download_ends_the_wait_and_is_named_in_the_error() {
    use banshee_common::{DownloadProgress, DownloadState};
    let report = |model: &str, state: DownloadState| DownloadProgress {
        model: model.to_string(),
        label: model.to_string(),
        index: 1,
        count: 2,
        bytes: 0,
        total: None,
        state,
    };
    let mut pending = 2;
    let mut failed = Vec::new();

    super::note_progress(
        &report("a.bin", DownloadState::Downloading),
        &mut pending,
        &mut failed,
    );
    assert_eq!((pending, failed.len()), (2, 0));
    super::note_progress(
        &report("a.bin", DownloadState::Done),
        &mut pending,
        &mut failed,
    );
    assert_eq!((pending, failed.len()), (1, 0));
    super::note_progress(
        &report("b.onnx", DownloadState::Failed),
        &mut pending,
        &mut failed,
    );
    assert_eq!(
        (pending, failed.as_slice()),
        (0, &["b.onnx".to_string()][..])
    );

    let error = super::downloads_settled(&failed).expect_err("a failed model is an error");
    assert!(error.to_string().contains("b.onnx"), "{error}");
    assert!(error.to_string().contains("banshee setup"), "{error}");
    assert!(super::downloads_settled(&[]).is_ok());
}

#[test]
fn a_rejection_reads_as_the_daemon_wrote_it() {
    let error = BansheeError::Rejected("say what to tell it, or pass --undo".into());
    assert_eq!(
        failure_line(&error),
        "say what to tell it, or pass --undo",
        "the caller's own mistake needs no wrapper and no Debug form"
    );
}

#[test]
fn a_socket_failure_names_the_daemon_as_the_thing_not_reached() {
    let refused = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
    let line = failure_line(&BansheeError::Io(refused));
    assert!(
        line.starts_with("Could not reach the daemon"),
        "an io failure on the socket is the daemon being away: {line}"
    );
}

#[test]
fn a_config_failure_does_not_blame_the_daemon() {
    let broken = toml::from_str::<toml::Table>("stt = ").unwrap_err();
    let line = failure_line(&BansheeError::Toml(broken));
    assert!(
        !line.contains("daemon"),
        "config.toml failing to parse says nothing about the daemon: {line}"
    );
}

#[test]
fn an_io_failure_away_from_the_socket_is_not_blamed_on_the_daemon() {
    let taken = std::io::Error::new(
        std::io::ErrorKind::AddrInUse,
        "another banshee daemon is already running",
    );
    let line = failure_line(&BansheeError::Io(taken));
    assert_eq!(
        line, "another banshee daemon is already running",
        "only a socket the CLI could not reach is the daemon being away"
    );
}

#[test]
fn a_missing_model_is_named_beside_the_command_that_fetches_it() {
    let note = super::missing_models_note(&["silero_vad.onnx".to_string()])
        .expect("a missing model has something to say");
    assert!(note.contains("silero_vad.onnx"), "{note}");
    assert!(
        note.contains("banshee setup"),
        "the line must name the command that downloads: {note}"
    );
}

#[test]
fn nothing_is_said_when_every_model_is_on_disk() {
    assert_eq!(super::missing_models_note(&[]), None);
}

#[test]
fn a_file_that_arrived_through_a_running_daemon_names_the_restart() {
    let note = super::restart_note(1).expect("a file that arrived has something to say");
    assert!(
        note.contains("banshee start"),
        "the line must name the command that reloads: {note}"
    );
}

#[test]
fn a_run_that_fetched_nothing_asks_for_no_restart() {
    assert_eq!(super::restart_note(0), None);
}

// Both halves of what an orphaned socket answers with: nothing at all, and a
// line that is not a reply. A fault that misses either one leaves `banshee
// stop` reporting a failure where a daemon is simply not running.
#[test]
fn an_empty_reply_and_an_unparsable_one_are_both_worth_probing_the_socket_for() {
    assert!(super::may_be_an_orphaned_socket(&BansheeError::NoAnswer));
    assert!(super::may_be_an_orphaned_socket(&BansheeError::Serde(
        serde_json::from_str::<serde_json::Value>("{").expect_err("not json")
    )));
}

#[test]
fn a_daemon_that_answered_is_never_read_as_an_orphaned_socket() {
    assert!(!super::may_be_an_orphaned_socket(&BansheeError::Rpc {
        code: -32004,
        message: "busy".to_string(),
    }));
    assert!(!super::may_be_an_orphaned_socket(&BansheeError::Other(
        "anything else".to_string()
    )));
}

const SETTLE: std::time::Duration = std::time::Duration::from_millis(200);

#[tokio::test]
async fn watch_ends_when_its_reader_closes_the_pipe() {
    let (reader, writer) = std::io::pipe().unwrap();
    let gone = tokio::spawn(super::reader_gone(writer));
    drop(reader);
    tokio::time::timeout(SETTLE, gone)
        .await
        .expect("a closed reader must end the watch without a write")
        .unwrap();
}

#[tokio::test]
async fn watch_ends_when_its_reader_closes_while_it_waits() {
    let (reader, writer) = std::io::pipe().unwrap();
    let gone = tokio::spawn(super::reader_gone(writer));
    tokio::time::sleep(SETTLE).await;
    drop(reader);
    tokio::time::timeout(SETTLE, gone)
        .await
        .expect("a reader that closes later must still end the watch")
        .unwrap();
}

#[tokio::test]
async fn watch_keeps_running_while_its_reader_is_open() {
    let (_reader, writer) = std::io::pipe().unwrap();
    let gone = tokio::spawn(super::reader_gone(writer));
    tokio::time::sleep(SETTLE).await;
    assert!(
        !gone.is_finished(),
        "a reader that stays must keep the watch"
    );
    gone.abort();
}

#[tokio::test]
async fn watch_into_a_file_never_reads_as_a_closed_reader() {
    let dir = crate::test_support::scratch("watch-into-a-file");
    let file = std::fs::File::create(dir.join("states")).unwrap();
    let gone = tokio::spawn(super::reader_gone(file));
    tokio::time::sleep(SETTLE).await;
    assert!(!gone.is_finished(), "a file has no reader to lose");
    gone.abort();
}

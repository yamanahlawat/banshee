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
fn a_device_carries_a_label_for_each_thing_that_points_at_it() {
    let cases = [
        (
            "both",
            device("Blue Yeti", true),
            Some("Blue Yeti"),
            "system default, in use",
        ),
        (
            "the daemon passed it over",
            device("Built-in", true),
            Some("Blue Yeti"),
            "system default",
        ),
        (
            "only in use",
            device("Blue Yeti", false),
            Some("Blue Yeti"),
            "in use",
        ),
        ("nothing points at it", device("BlackHole", false), None, ""),
        (
            "no daemon: a device nobody opened must not read as recording",
            device("Built-in", true),
            None,
            "system default",
        ),
    ];

    for (name, device, current, labels) in cases {
        assert_eq!(super::device_labels(&device, current), labels, "{name}");
    }
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

    let error =
        super::downloads_settled(&failed, "banshee setup").expect_err("a failed model is an error");
    assert!(error.to_string().contains("b.onnx"), "{error}");
    assert!(error.to_string().contains("run: banshee setup"), "{error}");
    assert!(super::downloads_settled(&[], "banshee setup").is_ok());
}

#[test]
fn a_rejection_reads_as_the_daemon_wrote_it() {
    let error = BansheeError::Rejected("'stt.model' needs a value".into());
    assert_eq!(
        failure_line(&error),
        "'stt.model' needs a value",
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
fn a_missing_listening_model_is_named_by_its_id() {
    let note = super::missing_models_note(&["parakeet-unified-en-0.6b-Q8_0.gguf".to_string()])
        .expect("a missing model has something to say");
    assert_eq!(
        note,
        "Recording waits on the listening model parakeet-unified-en-0.6b: run banshee setup"
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
    assert_eq!(
        super::missing_models_note(&[]),
        None,
        "nothing is said when every model is on disk"
    );
}

#[tokio::test]
async fn a_config_that_does_not_load_starts_nothing() {
    let mut registered = false;
    let refused = super::start_with(
        Err(BansheeError::Rejected(
            "Delete `preset` under [stt]: Banshee listens with stt.model now.".to_string(),
        )),
        || {
            registered = true;
            Ok(String::new())
        },
    )
    .await
    .expect_err("a config the daemon refuses is no start");
    assert!(!registered, "nothing may start a daemon that exits at once");
    let refused = refused.to_string();
    assert!(refused.contains("`preset` under [stt]"), "{refused}");
    assert!(refused.contains("banshee start"), "{refused}");
}

#[test]
fn a_listening_model_not_downloaded_is_fetched_and_never_restarted_into() {
    let dir = crate::test_support::unique_scratch("cli-model");
    let turbo = serde_json::json!("whisper-large-v3-turbo");
    let line = super::restart_line("stt.model", &turbo, &dir);
    assert!(line.contains("banshee setup"), "{line}");
    assert!(line.contains("whisper-large-v3-turbo"), "{line}");
    assert!(!line.contains("banshee start"), "{line}");
    assert!(
        !line.contains("the model it has"),
        "a daemon waiting for its first model has none: {line}"
    );

    std::fs::write(dir.join("whisper-large-v3-turbo-Q5_K_M.gguf"), b"").unwrap();
    let line = super::restart_line("stt.model", &turbo, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(line, "Restart to use it: banshee start");
    assert_eq!(
        super::restart_line("audio.hotkey", &serde_json::json!("F13"), &dir),
        "Restart to use it: banshee start"
    );
}

#[test]
fn a_null_write_says_the_key_was_removed() {
    assert_eq!(
        super::written_line("stt.preset", &serde_json::Value::Null),
        "Removed stt.preset from config.toml."
    );
    assert_eq!(
        super::written_line("stt.vad_threshold", &serde_json::json!(0.6)),
        "Set stt.vad_threshold in config.toml."
    );
}

#[test]
fn only_the_detector_that_arrived_through_a_running_daemon_names_the_restart() {
    let note =
        super::restart_note(&[crate::models::VAD_MODEL]).expect("the detector loads only at start");
    assert!(
        note.contains("banshee start"),
        "the line must name the command that reloads: {note}"
    );
    let parakeet = banshee_common::listening::SUGGESTED[0].file;
    assert_eq!(
        super::restart_note(&[parakeet]),
        None,
        "the daemon loads a listening model once its file arrives"
    );
    assert_eq!(super::restart_note(&[]), None, "nothing arrived");
}

#[test]
fn a_voice_that_arrived_asks_for_no_restart() {
    let companion: Vec<String> =
        crate::models::download::voice_files(banshee_common::COMPANION_VOICE)
            .into_iter()
            .map(|file| file.name)
            .collect();
    let names: Vec<&str> = companion.iter().map(String::as_str).collect();
    assert_eq!(super::restart_note(&names), None, "the next reply loads it");
    assert_eq!(super::restart_note(&["af_sky.bin"]), None);
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

#[test]
fn only_cues_level_and_words_can_be_watched_as_events() {
    let names = vec!["cues".to_string(), "level".to_string(), "words".to_string()];
    assert_eq!(
        super::watched_events(&names).unwrap(),
        vec!["cues", "level", "words"]
    );
    assert!(super::watched_events(&["downloads".to_string()]).is_err());
}

#[test]
fn an_event_prints_as_one_json_line() {
    let pushed = banshee_common::JsonRpcNotification::new(
        banshee_common::BANSHEE_CUE,
        serde_json::json!({"cue": "arm"}),
    );
    let line = super::event_line(&pushed);
    assert!(!line.contains('\n'));
    let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(parsed["method"], "banshee.cue");
    assert_eq!(parsed["params"]["cue"], "arm");
}

#[test]
fn a_non_commercial_model_needs_the_licence_accepted() {
    let dir = crate::test_support::unique_scratch("setup-licence");
    let mut accepted = crate::models::licences::Accepted::at(dir.join("accepted-licences"));

    let refused = super::model_to_fetch("canary-1b", false, &mut accepted);
    let accepting = super::model_to_fetch("canary-1b", true, &mut accepted).map(|model| model.id);
    let later = super::model_to_fetch("canary-1b", false, &mut accepted).map(|model| model.id);
    let _ = std::fs::remove_dir_all(&dir);

    let error = refused
        .expect_err("an unaccepted licence refuses")
        .to_string();
    assert!(error.contains("CC-BY-NC-4.0"), "{error}");
    assert!(error.contains("non-commercial purposes only"), "{error}");
    assert!(
        error.contains("https://creativecommons.org/licenses/by-nc/4.0/"),
        "{error}"
    );
    assert!(
        error.contains("banshee setup --model canary-1b --accept-licence"),
        "{error}"
    );
    assert_eq!(accepting.unwrap(), "canary-1b");
    assert_eq!(later.unwrap(), "canary-1b", "acceptance is recorded once");
}

#[test]
fn an_unknown_model_id_names_the_id() {
    let mut accepted =
        crate::models::licences::Accepted::at("/no-such-dir-9f3a/accepted-licences".into());
    let error = super::model_to_fetch("whisper-tiny-en", false, &mut accepted)
        .expect_err("an id the catalogue lacks is refused")
        .to_string();
    assert!(error.contains("'whisper-tiny-en'"), "{error}");
    assert!(
        error.contains(banshee_common::listening::DEFAULT_MODEL),
        "{error}"
    );
    assert!(error.contains("whisper-large-v3-turbo"), "{error}");
    assert!(error.contains("whisper-tiny.en"), "the closest id: {error}");
    assert!(error.contains("banshee setup --list"), "{error}");
}

#[test]
fn a_model_id_in_any_case_fetches_the_catalogue_id() {
    let mut accepted =
        crate::models::licences::Accepted::at("/no-such-dir-9f3a/accepted-licences".into());
    let model = super::model_to_fetch("QWEN3-asr-0.6b", false, &mut accepted).unwrap();
    assert_eq!(model.id, "Qwen3-ASR-0.6B");
}

#[test]
fn the_list_has_a_row_per_model_with_its_size_languages_streaming_and_licence() {
    let lines = super::catalogue_lines();
    assert_eq!(lines.len(), banshee_common::listening::CATALOGUE.len() + 1);
    let parakeet = lines
        .iter()
        .find(|line| line.starts_with("parakeet-unified-en-0.6b "))
        .expect("the default model is listed");
    for column in [
        "731 MB",
        "English",
        "yes",
        "NVIDIA Open Model License (2025-10-24)",
    ] {
        assert!(parakeet.contains(column), "{parakeet}");
    }
    let voxtral = lines
        .iter()
        .find(|line| line.starts_with("Voxtral-Small-24B-2507 "))
        .expect("the largest model is listed");
    let heard = banshee_common::listening::find("Voxtral-Small-24B-2507")
        .unwrap()
        .languages
        .len();
    assert!(heard > 3, "this row stands for a count");
    for column in ["GB", &format!("{heard} languages"), "no", "Apache-2.0"] {
        assert!(voxtral.contains(column), "{voxtral}");
    }
    let moonshine = lines
        .iter()
        .find(|line| line.starts_with("moonshine-tiny-ja "))
        .expect("a Moonshine build is listed");
    assert!(moonshine.contains("Powered by Moonshine AI"), "{moonshine}");
}

#[test]
fn up_to_three_languages_print_their_codes() {
    assert_eq!(super::languages_heard(&["en"]), "English");
    assert_eq!(super::languages_heard(&["zh", "en", "ja"]), "zh, en, ja");
    assert_eq!(super::languages_heard(&["a", "b", "c", "d"]), "4 languages");
}

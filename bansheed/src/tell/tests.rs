use super::*;

fn saved(agent: &str, at: u64) -> Session {
    Session {
        agent: agent.to_string(),
        id: "ses_one".to_string(),
        at,
    }
}

#[test]
fn a_command_inside_the_window_continues_the_thread() {
    let session = saved("opencode", 1_000);
    assert_eq!(
        resume(Some(&session), "opencode", 1_000 + 9 * 60, span(10)),
        Some("ses_one".to_string())
    );
}

#[test]
fn a_command_past_the_window_starts_a_new_thread() {
    let session = saved("opencode", 1_000);
    assert_eq!(
        resume(Some(&session), "opencode", 1_000 + 11 * 60, span(10)),
        None
    );
}

#[test]
fn a_different_agent_does_not_inherit_the_thread() {
    let session = saved("opencode", 1_000);
    assert_eq!(resume(Some(&session), "claude", 1_000 + 60, span(10)), None);
}

#[test]
fn no_saved_thread_starts_a_new_one() {
    assert_eq!(resume(None, "opencode", 1_000, span(10)), None);
}

#[test]
fn a_clock_that_went_backwards_starts_a_new_thread() {
    let session = saved("opencode", 2_000);
    assert_eq!(resume(Some(&session), "opencode", 1_000, span(10)), None);
}

#[test]
fn start_over_is_a_reset_whatever_its_case_or_spacing() {
    assert!(is_reset("start over"));
    assert!(is_reset("  Start over.  "));
    assert!(is_reset("START OVER"));
}

#[test]
fn start_over_inside_a_longer_command_is_not_a_reset() {
    assert!(!is_reset("start over from the blue theme"));
    assert!(!is_reset("make it start over"));
}

#[test]
fn the_slug_comes_from_connect_rather_than_a_second_list() {
    assert_eq!(
        Headless::ClaudeCode.name(),
        crate::connect::Agent::ClaudeCode.name()
    );
    assert_eq!(
        Headless::OpenCode.name(),
        crate::connect::Agent::OpenCode.name()
    );
}

#[test]
fn only_claude_is_scoped_to_the_watched_folders() {
    assert!(Headless::ClaudeCode.scoped());
    assert!(!Headless::OpenCode.scoped());
}

#[test]
fn the_configured_agent_wins() {
    let chosen = agent_for(
        Some("claude"),
        Some("opencode"),
        &[Headless::ClaudeCode, Headless::OpenCode],
    );
    assert_eq!(chosen.unwrap(), Headless::ClaudeCode);
}

#[test]
fn omarchys_default_wins_when_nothing_is_configured() {
    let chosen = agent_for(
        None,
        Some("opencode"),
        &[Headless::ClaudeCode, Headless::OpenCode],
    );
    assert_eq!(chosen.unwrap(), Headless::OpenCode);
}

#[test]
fn an_omarchy_default_with_no_headless_mode_falls_through() {
    let chosen = agent_for(None, Some("gemini"), &[Headless::ClaudeCode]);
    assert_eq!(chosen.unwrap(), Headless::ClaudeCode);
}

#[test]
fn an_omarchy_default_that_is_not_ready_falls_through() {
    let chosen = agent_for(None, Some("opencode"), &[Headless::ClaudeCode]);
    assert_eq!(chosen.unwrap(), Headless::ClaudeCode);
}

#[test]
fn no_ready_agent_names_what_to_do() {
    let error = agent_for(None, None, &[]).unwrap_err();
    assert!(
        error.to_string().contains("banshee connect"),
        "the failure must name the fix: {error}"
    );
}

#[test]
fn a_configured_agent_that_is_not_ready_is_an_error_rather_than_a_fallback() {
    // A pinned agent is a decision. Another agent run in its place hides it.
    let error = agent_for(Some("claude"), Some("opencode"), &[Headless::OpenCode]).unwrap_err();
    assert!(
        error.to_string().contains("claude"),
        "the failure must name the pinned agent: {error}"
    );
}

#[test]
fn a_configured_agent_with_no_headless_entry_names_the_ones_that_have_one() {
    let error = agent_for(Some("gemini"), None, &[Headless::OpenCode]).unwrap_err();
    let text = error.to_string();
    assert!(
        text.contains("gemini") && text.contains("opencode"),
        "{text}"
    );
}

use std::path::{Path, PathBuf};

fn run_dir() -> &'static Path {
    Path::new("/home/x/.banshee/tell")
}

fn one_folder() -> Vec<PathBuf> {
    vec![PathBuf::from("/home/x/.config/hypr")]
}

#[test]
fn opencode_runs_headless_with_auto() {
    // --auto is not a convenience. Measured: without it a write outside --dir
    // prints "permission requested: external_directory ...; auto-rejecting".
    assert_eq!(
        argv_for(
            Headless::OpenCode,
            "make the gaps bigger",
            None,
            run_dir(),
            &one_folder()
        ),
        vec![
            "run",
            "--dir",
            "/home/x/.banshee/tell",
            "--auto",
            "--format",
            "json",
            "--",
            "make the gaps bigger",
        ]
    );
}

#[test]
fn opencode_is_never_given_a_folder_list() {
    // Measured: an opencode.json permission block makes the run hang instead of
    // asking. --auto is all OpenCode has, and it takes no folders.
    let command = argv_for(Headless::OpenCode, "hi", None, run_dir(), &one_folder());
    assert!(!command.iter().any(|part| part.contains("hypr")));
}

#[test]
fn opencode_resumes_by_session_id() {
    let command = argv_for(
        Headless::OpenCode,
        "a bit more",
        Some("ses_one"),
        run_dir(),
        &one_folder(),
    );
    assert!(
        command
            .windows(2)
            .any(|pair| pair == ["--session", "ses_one"])
    );
}

#[test]
fn claude_is_allowed_to_speak_and_to_reach_the_folders() {
    // Measured: without --allowedTools a run answers with
    // "permission_denials":[{"tool_name":"mcp__banshee__speak_status"}] and the
    // user hears nothing. Without --add-dir it cannot write the config at all.
    assert_eq!(
        argv_for(
            Headless::ClaudeCode,
            "make the gaps bigger",
            None,
            run_dir(),
            &one_folder()
        ),
        vec![
            "--print",
            "--output-format",
            "json",
            "--permission-mode",
            "acceptEdits",
            "--allowedTools",
            "mcp__banshee__speak_status,mcp__banshee__ask_user",
            "--add-dir",
            "/home/x/.config/hypr",
            "--",
            "make the gaps bigger",
        ]
    );
}

#[test]
fn claude_takes_one_add_dir_flag_per_folder() {
    let folders = vec![
        PathBuf::from("/home/x/.config/hypr"),
        PathBuf::from("/home/x/.config/omarchy"),
    ];
    let command = argv_for(Headless::ClaudeCode, "hi", None, run_dir(), &folders);
    let count = command.iter().filter(|part| *part == "--add-dir").count();
    assert_eq!(count, 2);
}

#[test]
fn claude_resumes_by_session_id() {
    let command = argv_for(
        Headless::ClaudeCode,
        "a bit more",
        Some("07247d4f"),
        run_dir(),
        &one_folder(),
    );
    assert!(
        command
            .windows(2)
            .any(|pair| pair == ["--resume", "07247d4f"])
    );
}

#[test]
fn a_command_that_starts_with_a_hyphen_stays_a_message() {
    for agent in Headless::ALL {
        let command = argv_for(agent, "-brighter please", None, run_dir(), &one_folder());
        let last_two = &command[command.len() - 2..];
        assert_eq!(last_two, ["--", "-brighter please"]);
    }
}

const OPENCODE_OUT: &str = include_str!("../../tests/data/opencode-run.ndjson");
const CLAUDE_OUT: &str = include_str!("../../tests/data/claude-print.json");

#[test]
fn opencodes_session_id_comes_from_any_line() {
    assert_eq!(
        session_id(Headless::OpenCode, OPENCODE_OUT),
        Some("ses_f5f519f33ffe21kJeR0PSobNHE".to_string())
    );
}

#[test]
fn claudes_session_id_comes_from_the_one_object() {
    assert_eq!(
        session_id(Headless::ClaudeCode, CLAUDE_OUT),
        Some("07247d4f-3e15-4d9c-a5bd-8403a2b5d229".to_string())
    );
}

#[test]
fn the_reply_text_is_read_back_for_the_terminal() {
    assert_eq!(
        reply(Headless::OpenCode, OPENCODE_OUT).as_deref(),
        Some("The gap is now 10.")
    );
    assert_eq!(
        reply(Headless::ClaudeCode, CLAUDE_OUT).as_deref(),
        Some("The gap is now 10.")
    );
}

#[test]
fn output_that_is_not_json_yields_nothing_rather_than_a_failure() {
    // An agent that dies early prints a message, not JSON. The run already
    // failed; a parse error on top of it would hide the reason.
    assert_eq!(session_id(Headless::OpenCode, "command not found"), None);
    assert_eq!(session_id(Headless::ClaudeCode, ""), None);
    assert_eq!(reply(Headless::OpenCode, "not json"), None);
}

#[test]
fn a_run_with_several_text_parts_keeps_the_last_one() {
    let two = format!(
        "{}\n{}",
        r#"{"type":"text","sessionID":"ses_a","part":{"type":"text","text":"first"}}"#,
        r#"{"type":"text","sessionID":"ses_a","part":{"type":"text","text":"second"}}"#
    );
    assert_eq!(reply(Headless::OpenCode, &two).as_deref(), Some("second"));
}

#[test]
fn a_refused_tool_is_visible_in_the_output() {
    let denied = r#"{"result":"done","session_id":"a","permission_denials":[{"tool_name":"mcp__banshee__speak_status"}]}"#;
    assert!(
        denied_tools(Headless::ClaudeCode, denied)
            .contains(&"mcp__banshee__speak_status".to_string())
    );
    assert!(denied_tools(Headless::ClaudeCode, CLAUDE_OUT).is_empty());
}

#[test]
fn the_saved_thread_survives_a_write_and_a_read() {
    let dir = crate::test_support::scratch("tell-session");
    assert_eq!(read_session(&dir), None);
    let session = Session {
        agent: "opencode".to_string(),
        id: "ses_one".to_string(),
        at: 1_789_402_180,
    };
    write_session(&dir, &session).unwrap();
    assert_eq!(read_session(&dir), Some(session));
}

#[test]
fn a_corrupt_session_file_reads_as_no_thread_rather_than_a_failure() {
    let dir = crate::test_support::scratch("tell-corrupt");
    std::fs::write(dir.join("session.json"), "{not json").unwrap();
    assert_eq!(read_session(&dir), None);
}

#[test]
fn the_default_config_names_the_omarchy_skills_own_folders() {
    let config = TellConfig::default();
    assert_eq!(config.thread_timeout_min, 10);
    assert_eq!(config.run_timeout_min, 5);
    assert_eq!(config.snapshots, 10);
    assert_eq!(
        config.paths,
        vec![
            "~/.config/hypr",
            "~/.config/omarchy",
            "~/.config/alacritty",
            "~/.config/foot",
            "~/.config/kitty",
            "~/.config/ghostty",
        ]
    );
}

#[test]
fn a_tell_section_in_config_toml_parses() {
    let config =
        crate::config::Config::parse("[tell]\nagent = \"claude\"\nthread_timeout_min = 30\n")
            .unwrap();
    assert_eq!(config.tell.agent, "claude");
    assert_eq!(config.tell.thread_timeout_min, 30);
    assert_eq!(config.tell.snapshots, 10);
}

#[test]
fn a_config_with_no_tell_section_still_loads() {
    let config = crate::config::Config::parse("[audio]\n").unwrap();
    assert_eq!(config.tell.thread_timeout_min, 10);
}

/// Every `RunLock` shares one process-wide atomic, so the tests that take one
/// run one at a time. Two of them in parallel would refuse each other and go
/// red on a rule neither of them tests.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A failed test panics while it holds the lock, and a poisoned mutex must not
/// turn one red test into every later one.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|held| held.into_inner())
}

#[test]
fn one_run_at_a_time_and_the_lock_frees_when_it_ends() {
    let _serial = serial();
    // Every scenario lives here: two test functions racing for the lock would
    // be flaky.
    let long = std::time::Duration::from_secs(600);
    let dir = crate::test_support::scratch("tell-lock");
    {
        let _first = RunLock::take(&dir, long).expect("the first run takes the lock");
        assert!(
            RunLock::take(&dir, long).is_none(),
            "a second run must be refused"
        );

        // undo is the command a user reaches for when something is already
        // going wrong, so it must be refused too, not just a second run.
        let config = TellConfig::default();
        let error = undo_in(&dir, &config).unwrap_err();
        assert_eq!(
            error.to_string(),
            "a command is already running. Try again once it finishes."
        );
    }
    assert!(
        RunLock::take(&dir, long).is_some(),
        "the lock must free on drop"
    );

    let stale_dir = crate::test_support::scratch("tell-lock-stale");
    std::fs::write(
        stale_dir.join("run.lock"),
        (now_seconds() - 1_000).to_string(),
    )
    .unwrap();
    let short = std::time::Duration::from_secs(60);
    assert!(
        RunLock::take(&stale_dir, short).is_some(),
        "a lock older than the deadline must be taken over"
    );

    let garbled_dir = crate::test_support::scratch("tell-lock-garbled");
    std::fs::write(garbled_dir.join("run.lock"), "not a timestamp").unwrap();
    assert!(
        RunLock::take(&garbled_dir, long).is_some(),
        "a garbled lock file must be treated as stale"
    );

    let token_dir = crate::test_support::scratch("tell-lock-token");
    let taken = RunLock::take(&token_dir, long).expect("the lock is free");
    std::fs::write(token_dir.join("run.lock"), "999999999 424242").unwrap();
    drop(taken);
    assert_eq!(
        std::fs::read_to_string(token_dir.join("run.lock")).unwrap(),
        "999999999 424242",
        "a lock file carrying someone else's token must survive this drop"
    );
}

#[test]
fn a_lock_younger_than_the_deadline_is_not_stale() {
    assert!(!stale(1_000, 1_030, std::time::Duration::from_secs(60)));
}

#[test]
fn a_lock_older_than_the_deadline_is_stale() {
    assert!(stale(1_000, 2_000, std::time::Duration::from_secs(60)));
}

#[test]
fn a_child_past_its_deadline_is_killed_rather_than_waited_on() {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let sleep = crate::status::resolve("sleep", &path).expect("sleep must be on PATH to test this");
    let start = std::time::Instant::now();
    let ran = run_bounded(
        &sleep,
        &["5".to_string()],
        &std::env::temp_dir(),
        &path,
        std::time::Duration::from_millis(200),
    )
    .unwrap();
    assert!(matches!(ran, Ran::TimedOut));
    assert!(
        start.elapsed() < std::time::Duration::from_secs(2),
        "the wait must not run out the child's own sleep"
    );
}

#[test]
fn a_child_that_finishes_in_time_is_read_normally() {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let echo = crate::status::resolve("echo", &path).expect("echo must be on PATH to test this");
    let ran = run_bounded(
        &echo,
        &["hello".to_string()],
        &std::env::temp_dir(),
        &path,
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    let Ran::Finished {
        output,
        stdout_lost,
    } = ran
    else {
        panic!("a command that finishes in time must not read as timed out");
    };
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    assert!(
        !stdout_lost,
        "a pipe that closed must not read as a lost one"
    );
}

#[test]
fn a_descendant_holding_the_pipe_does_not_hold_the_call_open() {
    // A shell that backgrounds a long sleep and exits stands in for
    // `opencode run` and the local server it leaves behind: the immediate
    // child is gone almost at once, but the backgrounded sleep inherits the
    // piped stdout and stderr and holds them open for a hundred seconds.
    let path = std::env::var_os("PATH").unwrap_or_default();
    let sh = crate::status::resolve("sh", &path).expect("sh must be on PATH to test this");
    let start = std::time::Instant::now();
    let ran = run_bounded(
        &sh,
        &[
            "-c".to_string(),
            // The echo proves the loss: the child did write, and the bytes sit
            // in a pipe the backgrounded sleep still holds open.
            "echo written-but-unread; sleep 100 & exit 0".to_string(),
        ],
        &std::env::temp_dir(),
        &path,
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    let Ran::Finished {
        output,
        stdout_lost,
    } = ran
    else {
        panic!("a child that exits must not read as timed out");
    };
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "a descendant holding the pipe must not hold the call open"
    );
    assert!(
        output.stdout.is_empty(),
        "the read gave up, so nothing arrived"
    );
    assert!(
        stdout_lost,
        "empty output here must not read as a quiet child: the reply and the \
         session id were both in there"
    );
}

#[test]
fn a_fresh_thread_names_the_agent_and_a_resumed_one_does_not() {
    assert_eq!(
        opening_announcement(Headless::ClaudeCode, None),
        Some("Running claude. It can edit only the folders in tell.paths.".to_string())
    );
    assert_eq!(
        opening_announcement(Headless::OpenCode, None),
        Some("Running opencode. It can edit anything: OpenCode takes no folder list.".to_string())
    );
    assert_eq!(
        opening_announcement(Headless::ClaudeCode, Some("ses_one")),
        None
    );
}

#[test]
fn a_tilde_path_expands_against_home() {
    let home = Path::new("/home/x");
    assert_eq!(expand("~/.config/hypr", home), home.join(".config/hypr"));
    assert_eq!(expand("/etc/thing", home), Path::new("/etc/thing"));
}

#[test]
fn a_snapshot_copies_every_named_folder() {
    let root = crate::test_support::scratch("tell-snapshot");
    let hypr = root.join("hypr");
    std::fs::create_dir_all(hypr.join("nested")).unwrap();
    std::fs::write(hypr.join("looknfeel.lua"), "gaps = 5\n").unwrap();
    std::fs::write(hypr.join("nested/input.lua"), "kb = us\n").unwrap();

    let into = root.join("snapshots");
    let made = snapshot(std::slice::from_ref(&hypr), &into, 1_789_402_180).unwrap();

    assert_eq!(made, into.join("1789402180"));
    assert_eq!(
        std::fs::read_to_string(made.join("hypr/looknfeel.lua")).unwrap(),
        "gaps = 5\n"
    );
    assert_eq!(
        std::fs::read_to_string(made.join("hypr/nested/input.lua")).unwrap(),
        "kb = us\n"
    );
}

#[test]
fn a_folder_that_is_not_there_is_skipped_rather_than_a_failure() {
    let root = crate::test_support::scratch("tell-missing");
    let into = root.join("snapshots");
    let made = snapshot(&[root.join("ghostty")], &into, 1).unwrap();
    assert!(made.is_dir());
    assert!(!made.join("ghostty").exists());
}

#[test]
fn prune_keeps_the_newest_and_deletes_the_rest() {
    let snapshots = crate::test_support::scratch("tell-prune");
    for name in ["100", "200", "300", "400"] {
        std::fs::create_dir_all(snapshots.join(name)).unwrap();
    }
    prune(&snapshots, 2).unwrap();
    let mut left: Vec<String> = std::fs::read_dir(&snapshots)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    left.sort();
    assert_eq!(left, vec!["300".to_string(), "400".to_string()]);
}

#[test]
fn prune_sorts_by_number_rather_than_by_name() {
    // "1000" sorts before "900" as text, and after it as a time.
    let snapshots = crate::test_support::scratch("tell-prune-order");
    for name in ["900", "1000"] {
        std::fs::create_dir_all(snapshots.join(name)).unwrap();
    }
    prune(&snapshots, 1).unwrap();
    let left: Vec<String> = std::fs::read_dir(&snapshots)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(left, vec!["1000".to_string()]);
}

#[test]
fn a_snapshots_setting_of_zero_still_keeps_one() {
    let config = TellConfig {
        snapshots: 0,
        ..TellConfig::default()
    };
    assert_eq!(config.keep(), 1);
}

#[test]
fn a_snapshot_copies_a_symlink_as_a_symlink() {
    let root = crate::test_support::scratch("tell-symlink");
    let source = root.join("source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("file.txt"), "content\n").unwrap();
    std::fs::create_dir_all(source.join("subdir")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("file.txt", source.join("link.txt")).unwrap();

    let into = root.join("snapshots");
    let made = snapshot(std::slice::from_ref(&source), &into, 1).unwrap();

    let link_path = made.join("source/link.txt");
    assert!(std::fs::symlink_metadata(&link_path).unwrap().is_symlink());
    assert_eq!(
        std::fs::read_to_string(made.join("source/file.txt")).unwrap(),
        "content\n"
    );
}

#[test]
fn a_snapshot_does_not_follow_symlink_loops() {
    let root = crate::test_support::scratch("tell-symlink-loop");
    let source = root.join("source");
    std::fs::create_dir_all(&source).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(".", source.join("loop")).unwrap();

    let into = root.join("snapshots");
    let made = snapshot(std::slice::from_ref(&source), &into, 1).unwrap();

    let loop_path = made.join("source/loop");
    assert!(std::fs::symlink_metadata(&loop_path).unwrap().is_symlink());
}

#[test]
fn copying_a_symlink_twice_into_the_same_destination_succeeds() {
    let root = crate::test_support::scratch("tell-copy-twice");
    let source = root.join("source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("file.txt"), "content\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("file.txt", source.join("link.txt")).unwrap();

    let dest = root.join("dest");
    copy_tree(&source, &dest).unwrap();
    copy_tree(&source, &dest).unwrap();

    assert!(
        std::fs::symlink_metadata(dest.join("link.txt"))
            .unwrap()
            .is_symlink()
    );
}

#[test]
fn the_newest_snapshot_is_the_highest_number() {
    let snapshots = crate::test_support::scratch("tell-newest");
    for name in ["900", "1000", "950"] {
        std::fs::create_dir_all(snapshots.join(name)).unwrap();
    }
    assert_eq!(newest(&snapshots), Some(snapshots.join("1000")));
}

#[test]
fn a_stray_file_that_parses_as_a_number_does_not_outrank_a_real_snapshot() {
    let snapshots = crate::test_support::scratch("tell-newest-stray-file");
    std::fs::create_dir_all(snapshots.join("100")).unwrap();
    std::fs::write(snapshots.join("999999"), "not a snapshot").unwrap();
    assert_eq!(newest(&snapshots), Some(snapshots.join("100")));
}

#[test]
fn no_snapshot_yet_is_none_rather_than_an_error() {
    let snapshots = crate::test_support::scratch("tell-newest-empty");
    assert_eq!(newest(&snapshots), None);
}

#[test]
fn a_restore_puts_the_files_back_and_names_the_folders() {
    let root = crate::test_support::scratch("tell-restore");
    let hypr = root.join("hypr");
    std::fs::create_dir_all(&hypr).unwrap();
    std::fs::write(hypr.join("looknfeel.lua"), "gaps = 5\n").unwrap();

    let into = root.join("snapshots");
    snapshot(std::slice::from_ref(&hypr), &into, 100).unwrap();

    // The agent changes one file and adds another.
    std::fs::write(hypr.join("looknfeel.lua"), "gaps = 40\n").unwrap();
    std::fs::write(hypr.join("stray.lua"), "oops\n").unwrap();

    let named = restore(&newest(&into).unwrap(), std::slice::from_ref(&hypr));

    assert_eq!(
        std::fs::read_to_string(hypr.join("looknfeel.lua")).unwrap(),
        "gaps = 5\n"
    );
    assert!(
        !hypr.join("stray.lua").exists(),
        "a restore must remove a file the agent added, or the config keeps it"
    );
    assert_eq!(named.done, vec![hypr.display().to_string()]);
    assert!(named.failed.is_empty());
}

#[test]
fn a_restore_leaves_no_temporary_folder_behind() {
    let root = crate::test_support::scratch("tell-restore-safe");
    let hypr = root.join("hypr");
    std::fs::create_dir_all(&hypr).unwrap();
    std::fs::write(hypr.join("a.lua"), "one\n").unwrap();
    let into = root.join("snapshots");
    snapshot(std::slice::from_ref(&hypr), &into, 100).unwrap();

    restore(&newest(&into).unwrap(), std::slice::from_ref(&hypr));

    assert!(hypr.is_dir(), "the folder must exist after a restore");
    let leftovers: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .filter(|name| name.contains("banshee-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary folders left behind: {leftovers:?}"
    );
}

#[test]
fn a_folder_with_no_copy_in_the_snapshot_is_left_alone() {
    let root = crate::test_support::scratch("tell-restore-absent");
    let ghostty = root.join("ghostty");
    std::fs::create_dir_all(&ghostty).unwrap();
    std::fs::write(ghostty.join("new.toml"), "made later\n").unwrap();
    let into = root.join("snapshots");
    std::fs::create_dir_all(into.join("100")).unwrap();

    let named = restore(&into.join("100"), std::slice::from_ref(&ghostty));

    assert!(ghostty.join("new.toml").exists());
    assert!(named.done.is_empty());
    assert!(named.failed.is_empty());
}

#[test]
fn a_symlinked_target_is_refused_and_named_rather_than_replaced() {
    let root = crate::test_support::scratch("tell-restore-symlinked-target");
    let real = root.join("real-hypr");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("looknfeel.lua"), "gaps = 1\n").unwrap();

    let hypr = root.join("hypr");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &hypr).unwrap();

    let into = root.join("snapshots");
    snapshot(std::slice::from_ref(&hypr), &into, 100).unwrap();
    std::fs::write(real.join("looknfeel.lua"), "gaps = 40\n").unwrap();

    let result = restore(&newest(&into).unwrap(), std::slice::from_ref(&hypr));

    assert!(result.done.is_empty());
    assert_eq!(result.failed.len(), 1);
    assert_eq!(result.failed[0].0, hypr.display().to_string());
    assert!(
        std::fs::symlink_metadata(&hypr).unwrap().is_symlink(),
        "the link itself must survive a refusal"
    );
    assert_eq!(
        std::fs::read_to_string(real.join("looknfeel.lua")).unwrap(),
        "gaps = 40\n",
        "a refusal must not touch the file the link points to either"
    );
}

#[test]
fn a_failure_on_one_folder_does_not_cost_the_record_of_the_ones_already_restored() {
    let root = crate::test_support::scratch("tell-restore-partial");
    let a = root.join("a");
    std::fs::create_dir_all(&a).unwrap();
    let hypr = a.join("hypr");
    std::fs::create_dir_all(&hypr).unwrap();
    std::fs::write(hypr.join("looknfeel.lua"), "gaps = 5\n").unwrap();

    let b = root.join("b");
    std::fs::create_dir_all(&b).unwrap();
    let ghostty = b.join("ghostty");
    std::fs::create_dir_all(&ghostty).unwrap();
    std::fs::write(ghostty.join("config.toml"), "one\n").unwrap();

    let into = root.join("snapshots");
    snapshot(&[hypr.clone(), ghostty.clone()], &into, 100).unwrap();
    std::fs::write(hypr.join("looknfeel.lua"), "gaps = 40\n").unwrap();

    // b has no write permission, so the staged copy for ghostty cannot be
    // created there: restoring it must fail without touching hypr's own,
    // already-succeeded, restore.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&b).unwrap().permissions();
        perms.set_mode(0o555);
        std::fs::set_permissions(&b, perms).unwrap();
    }

    let result = restore(&newest(&into).unwrap(), &[hypr.clone(), ghostty.clone()]);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&b).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&b, perms).unwrap();
    }

    assert_eq!(result.done, vec![hypr.display().to_string()]);
    assert_eq!(
        std::fs::read_to_string(hypr.join("looknfeel.lua")).unwrap(),
        "gaps = 5\n",
        "the folder that succeeded must still be restored"
    );
    assert_eq!(result.failed.len(), 1);
    assert_eq!(result.failed[0].0, ghostty.display().to_string());
}

#[test]
fn describe_names_both_the_kept_and_the_lost_when_a_restore_is_partial() {
    let restored = Restored {
        done: vec!["/home/x/.config/hypr".to_string()],
        failed: vec![(
            "/home/x/.config/ghostty".to_string(),
            "is a symlink; put the copy back by hand".to_string(),
        )],
    };
    assert_eq!(
        describe(&restored),
        "Put back: /home/x/.config/hypr. Could not put back: \
         /home/x/.config/ghostty (is a symlink; put the copy back by hand)."
    );
}

#[test]
fn describe_says_nothing_changed_when_the_snapshot_held_no_watched_folder() {
    assert_eq!(
        describe(&Restored::default()),
        "The snapshot held none of the watched folders. Nothing changed."
    );
}

#[test]
fn a_restore_that_put_nothing_back_is_a_failure_rather_than_a_success() {
    assert!(failed_outright(&Restored {
        done: Vec::new(),
        failed: vec![("/home/x/.config/hypr".to_string(), "denied".to_string())],
    }));
}

#[test]
fn a_restore_that_put_one_folder_back_stays_a_success() {
    assert!(!failed_outright(&Restored {
        done: vec!["/home/x/.config/hypr".to_string()],
        failed: vec![("/home/x/.config/ghostty".to_string(), "denied".to_string())],
    }));
    assert!(!failed_outright(&Restored::default()));
}

/// Builds `<dir>/snapshots/<at>/<name>` holding one file, and answers with the
/// folder the restore would write back to.
fn snapshot_holding(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let copy = dir.join("snapshots").join("1000").join(name);
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::write(copy.join("looknfeel.lua"), contents).unwrap();
    dir.join(name)
}

/// The watched folders as absolute paths, so `expand` leaves them alone and no
/// test can reach the real `~/.config`.
fn config_watching(targets: &[&Path]) -> TellConfig {
    TellConfig {
        paths: targets.iter().map(|p| p.display().to_string()).collect(),
        ..TellConfig::default()
    }
}

#[test]
fn an_undo_that_restored_nothing_fails_while_the_sentence_stays_the_same() {
    let _serial = serial();
    let dir = crate::test_support::scratch("tell-undo-all-failed");
    let target = snapshot_holding(&dir, "hypr", "gaps = 5\n");
    // A symlinked folder is refused rather than replaced, so this restore has
    // one failure and nothing put back.
    std::os::unix::fs::symlink(dir.join("elsewhere"), &target).unwrap();

    let error = undo_in(&dir, &config_watching(&[&target])).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "Could not put back: {} (is a symlink; put the copy back by hand)",
            target.display()
        ),
        "the words the user hears must not change, only the exit code"
    );
}

#[test]
fn an_undo_that_restored_one_folder_succeeds() {
    let _serial = serial();
    let dir = crate::test_support::scratch("tell-undo-partial");
    let kept = snapshot_holding(&dir, "hypr", "gaps = 5\n");
    let lost = snapshot_holding(&dir, "ghostty", "font = 12\n");
    std::fs::create_dir_all(&kept).unwrap();
    std::os::unix::fs::symlink(dir.join("elsewhere"), &lost).unwrap();

    let sentence = undo_in(&dir, &config_watching(&[&kept, &lost])).unwrap();
    assert!(
        sentence.starts_with(&format!("Put back: {}", kept.display())),
        "a partial restore stays a success: {sentence}"
    );
    assert_eq!(
        std::fs::read_to_string(kept.join("looknfeel.lua")).unwrap(),
        "gaps = 5\n"
    );
}

#[test]
fn a_run_timeout_of_a_lifetime_is_clamped_rather_than_overflowing() {
    let config = TellConfig {
        run_timeout_min: u64::MAX,
        ..TellConfig::default()
    };
    assert_eq!(run_deadline(&config), MAX_SPAN);
    let _lock_deadline = run_deadline(&config) + PRE_SPAWN_MARGIN;
    let _poll_deadline = Instant::now() + run_deadline(&config);
}

#[test]
fn a_run_timeout_under_the_ceiling_is_the_one_configured() {
    let config = TellConfig {
        run_timeout_min: 5,
        ..TellConfig::default()
    };
    assert_eq!(run_deadline(&config), Duration::from_secs(300));
}

#[test]
fn a_thread_timeout_of_a_lifetime_is_clamped_rather_than_overflowing() {
    let config = TellConfig {
        thread_timeout_min: u64::MAX,
        ..TellConfig::default()
    };
    assert_eq!(thread_window(&config), MAX_SPAN);
    let session = saved("opencode", 1_000);
    assert_eq!(
        resume(
            Some(&session),
            "opencode",
            1_000 + MAX_SPAN.as_secs(),
            thread_window(&config)
        ),
        Some("ses_one".to_string()),
        "a thread as old as the ceiling is still inside it"
    );
    assert_eq!(
        resume(
            Some(&session),
            "opencode",
            1_000 + MAX_SPAN.as_secs() + 1,
            thread_window(&config)
        ),
        None,
        "the ceiling ends the thread whatever the config asked for"
    );
}

#[test]
fn a_thread_timeout_under_the_ceiling_is_the_one_configured() {
    let config = TellConfig {
        thread_timeout_min: 30,
        ..TellConfig::default()
    };
    assert_eq!(thread_window(&config), Duration::from_secs(1_800));
}

#[test]
fn a_lost_stdout_keeps_the_thread_the_run_asked_to_resume() {
    assert_eq!(
        thread_to_save(None, Some("ses_one"), true),
        Some("ses_one".to_string()),
        "the run knows the thread it asked for, and \"a bit more\" needs it"
    );
}

#[test]
fn a_lost_stdout_on_a_fresh_thread_saves_nothing() {
    assert_eq!(thread_to_save(None, None, true), None);
}

#[test]
fn an_id_in_the_output_wins_over_the_one_the_run_asked_for() {
    assert_eq!(
        thread_to_save(Some("ses_two".to_string()), Some("ses_one"), true),
        Some("ses_two".to_string())
    );
}

#[test]
fn a_read_that_finished_saves_only_what_the_output_named() {
    // Without the `stdout_lost` guard a resumed run with no id in its output
    // would keep writing the same thread back for ever.
    assert_eq!(thread_to_save(None, Some("ses_one"), false), None);
}

#[test]
fn a_lost_reply_says_whether_the_thread_survived() {
    assert_eq!(
        lost_output_warning(Headless::OpenCode, true),
        "opencode finished, but its output did not arrive in time. Its reply is lost. \
         The thread is kept."
    );
    assert!(
        lost_output_warning(Headless::OpenCode, false).ends_with("starts a new thread."),
        "a lost thread must not read as a kept one"
    );
}

#[test]
fn a_refused_tool_becomes_a_line_the_user_gets() {
    assert_eq!(
        denied_warning(
            Headless::ClaudeCode,
            &["mcp__banshee__speak_status".to_string()]
        ),
        Some(
            "claude was refused these tools, so it may have worked in silence: \
             mcp__banshee__speak_status"
                .to_string()
        )
    );
    assert_eq!(denied_warning(Headless::ClaudeCode, &[]), None);
}

#[test]
fn the_user_must_hear_the_warnings_but_not_the_reply() {
    let told = Told {
        reply: Some("The gap is five.".to_string()),
        warnings: vec![
            "It was refused a tool.".to_string(),
            "Its output did not arrive in time.".to_string(),
        ],
    };
    assert_eq!(
        told.must_hear().collect::<Vec<_>>(),
        vec![
            "It was refused a tool.",
            "Its output did not arrive in time.",
        ],
        "the agent spoke its own reply, so speaking it again says it twice"
    );
}

/// What one run said and did, in the order it happened.
fn ordered(agent: Headless, resume_id: Option<&str>) -> Vec<String> {
    let steps = std::sync::Mutex::new(Vec::new());
    announce_then_start(
        &|line| steps.lock().unwrap().push(format!("said: {line}")),
        agent,
        resume_id,
        || steps.lock().unwrap().push("started the agent".to_string()),
    );
    steps.into_inner().unwrap()
}

#[test]
fn the_scope_reaches_the_user_before_the_agent_starts() {
    assert_eq!(
        ordered(Headless::OpenCode, None),
        vec![
            "said: Running opencode. It can edit anything: OpenCode takes no folder list.",
            "started the agent",
        ]
    );
}

#[test]
fn a_resumed_thread_starts_the_agent_and_says_nothing() {
    assert_eq!(
        ordered(Headless::ClaudeCode, Some("ses_one")),
        vec!["started the agent"],
        "the scope is stated on the first turn, and the run must still happen"
    );
}

#[test]
fn a_run_with_nothing_to_report_asks_for_no_speech() {
    let told = Told {
        reply: Some("Done.".to_string()),
        ..Told::default()
    };
    assert_eq!(told.must_hear().count(), 0);
}

#[test]
fn a_bounded_run_closes_stdin_rather_than_inheriting_it() {
    // `omarchy_default` runs through here for this and for the deadline.
    let path = std::env::var_os("PATH").unwrap_or_default();
    let sh = crate::status::resolve("sh", &path).expect("sh must be on PATH to test this");
    // `cat` answers only when stdin closes, so a run that inherits a terminal
    // would sit here until the bound killed it.
    let start = Instant::now();
    let ran = run_bounded(
        &sh,
        &["-c".to_string(), "cat; echo read-to-the-end".to_string()],
        &std::env::temp_dir(),
        &path,
        Duration::from_secs(5),
    )
    .unwrap();
    let Ran::Finished { output, .. } = ran else {
        panic!("a closed stdin must let the child finish");
    };
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "read-to-the-end"
    );
    assert!(start.elapsed() < Duration::from_secs(5));
}

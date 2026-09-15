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
fn show_me_asks_for_the_screen_whatever_its_case_or_spacing() {
    assert!(is_show("show me"));
    assert!(is_show("  Show me.  "));
    assert!(is_show("SHOW ME"));
}

#[test]
fn show_me_inside_a_longer_command_still_reaches_the_agent() {
    assert!(!is_show("show me a list of themes"));
    assert!(!is_show("show me the current gap size"));
    assert!(!is_show("start over"));
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

fn run_dir() -> PathBuf {
    agent_dir(Path::new("/home/x/.banshee/tell"))
}

fn one_folder() -> Vec<PathBuf> {
    vec![PathBuf::from("/home/x/.config/hypr")]
}

#[test]
fn nothing_banshee_keeps_lives_inside_the_agents_directory() {
    // An agent that lists its own working directory must not find the
    // snapshots there. It would edit a copy, not the real config.
    let state = Path::new("/home/x/.banshee/tell");
    let run_in = agent_dir(state);
    for kept in [
        snapshots_dir(state),
        state.join("session.json"),
        state.join("run.lock"),
    ] {
        assert!(
            !kept.starts_with(&run_in),
            "{} sits inside the agent's directory",
            kept.display()
        );
    }
}

#[test]
fn the_snapshot_store_keeps_the_path_earlier_runs_wrote_to() {
    assert_eq!(
        snapshots_dir(Path::new("/home/x/.banshee/tell")),
        Path::new("/home/x/.banshee/tell/snapshots"),
        "a moved store would strand every snapshot the user already has"
    );
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
            &run_dir(),
            &one_folder()
        ),
        vec![
            "run",
            "--dir",
            "/home/x/.banshee/tell/run",
            "--auto",
            "--format",
            "json",
            "--",
            "make the gaps bigger",
        ]
    );
}

#[test]
fn opencode_resumes_by_session_id() {
    let command = argv_for(
        Headless::OpenCode,
        "a bit more",
        Some("ses_one"),
        &run_dir(),
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
            &run_dir(),
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
    let command = argv_for(Headless::ClaudeCode, "hi", None, &run_dir(), &folders);
    let count = command.iter().filter(|part| *part == "--add-dir").count();
    assert_eq!(count, 2);
}

#[test]
fn claude_resumes_by_session_id() {
    let command = argv_for(
        Headless::ClaudeCode,
        "a bit more",
        Some("07247d4f"),
        &run_dir(),
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
        let command = argv_for(agent, "-brighter please", None, &run_dir(), &one_folder());
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
    assert!(denied_tools(denied).contains(&"mcp__banshee__speak_status".to_string()));
    assert!(denied_tools(CLAUDE_OUT).is_empty());
    // Measured on opencode 1.18.31: the string "denials" is absent from the
    // binary, where "sessionID" appears 2143 times.
    assert!(denied_tools(OPENCODE_OUT).is_empty());
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
fn a_reset_that_cannot_remove_the_thread_is_an_error_rather_than_a_confirmation() {
    let dir = crate::test_support::scratch("tell-reset-denied");
    write_session(
        &dir,
        &Session {
            agent: "opencode".to_string(),
            id: "ses_one".to_string(),
            at: now_seconds(),
        },
    )
    .unwrap();
    // An unlink asks the parent directory for write permission, not the file.
    let denied = read_only(&dir);
    let answer = clear_thread(&dir);
    writable(&dir);

    assert!(
        denied,
        "the directory must refuse the removal for this to measure anything"
    );
    let error = answer.unwrap_err();
    assert!(
        error
            .to_string()
            .contains(&dir.join("session.json").display().to_string()),
        "a user who cannot see the screen needs the path: {error}"
    );
    assert!(
        read_session(&dir).is_some(),
        "the thread is still there, so the answer must not say it is cleared"
    );
}

#[test]
fn a_reset_with_no_saved_thread_still_clears() {
    let dir = crate::test_support::scratch("tell-reset-empty");
    assert_eq!(
        clear_thread(&dir).unwrap().reply.as_deref(),
        Some("Thread cleared."),
        "a fresh daemon has no file to remove, and that is not a failure"
    );
}

#[test]
fn a_corrupt_session_file_reads_as_no_thread_rather_than_a_failure() {
    let dir = crate::test_support::scratch("tell-corrupt");
    std::fs::write(dir.join("session.json"), "{not json").unwrap();
    assert_eq!(read_session(&dir), None);
}

/// Takes write permission off `dir`, and answers whether the removal it guards
/// now fails. A test that runs as root gets `false`: root ignores the mode.
fn read_only(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(dir).unwrap().permissions();
    perms.set_mode(0o555);
    std::fs::set_permissions(dir, perms).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join("banshee-permission-probe"))
        .is_err()
}

fn writable(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(dir).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(dir, perms).unwrap();
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

/// Every `RunLock` shares one process-wide atomic, so these tests run one at a
/// time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|held| held.into_inner())
}

#[test]
fn one_run_at_a_time_and_the_lock_frees_when_it_ends() {
    let _serial = serial();
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

    let short = std::time::Duration::from_secs(60);
    let young_dir = crate::test_support::scratch("tell-lock-young");
    std::fs::write(
        young_dir.join("run.lock"),
        format!("{} 424242", now_seconds() - 30),
    )
    .unwrap();
    assert!(
        RunLock::take(&young_dir, short).is_none(),
        "a lock younger than the deadline is a live run"
    );

    let stale_dir = crate::test_support::scratch("tell-lock-stale");
    std::fs::write(
        stale_dir.join("run.lock"),
        (now_seconds() - 1_000).to_string(),
    )
    .unwrap();
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
fn a_takeover_that_finds_a_live_lock_puts_it_back_rather_than_stealing_it() {
    let dir = crate::test_support::scratch("tell-lock-live");
    let path = dir.join("run.lock");
    let live = format!("{} 424242", now_seconds());
    std::fs::write(&path, &live).unwrap();

    assert!(!claim_stale(&path, std::time::Duration::from_secs(60)));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        live,
        "a process a step behind must not remove a fresh lock"
    );
}

#[test]
fn one_caller_takes_over_a_dead_lock_and_the_next_one_finds_nothing_to_move() {
    let dir = crate::test_support::scratch("tell-lock-takeover");
    let path = dir.join("run.lock");
    std::fs::write(&path, format!("{} 424242", now_seconds() - 1_000)).unwrap();
    let short = std::time::Duration::from_secs(60);

    assert!(claim_stale(&path, short), "a dead lock is taken over");
    assert!(
        !claim_stale(&path, short),
        "the second caller finds no file to move"
    );
    assert!(!path.exists());
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
    // A shell that backgrounds a long sleep and exits stands in for `opencode
    // run` and the server it leaves behind. The child is gone at once. The
    // sleep inherits the piped stdout and stderr and holds them for a hundred
    // seconds.
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
        Some(
            "Running claude. It gets the folders in tell.paths, and its own run directory."
                .to_string()
        )
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

    let copy = made.join(snapshot_key(&hypr));
    assert_eq!(made, into.join("1789402180"));
    assert_eq!(
        std::fs::read_to_string(copy.join("looknfeel.lua")).unwrap(),
        "gaps = 5\n"
    );
    assert_eq!(
        std::fs::read_to_string(copy.join("nested/input.lua")).unwrap(),
        "kb = us\n"
    );
}

#[test]
fn a_folder_that_is_not_there_is_skipped_rather_than_a_failure() {
    let root = crate::test_support::scratch("tell-missing");
    let into = root.join("snapshots");
    let ghostty = root.join("ghostty");
    let made = snapshot(std::slice::from_ref(&ghostty), &into, 1).unwrap();
    assert!(made.is_dir());
    assert!(!made.join(snapshot_key(&ghostty)).exists());
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
fn a_stray_file_that_parses_as_a_number_does_not_wedge_prune() {
    let snapshots = crate::test_support::scratch("tell-prune-stray-file");
    for name in ["100", "200"] {
        std::fs::create_dir_all(snapshots.join(name)).unwrap();
    }
    std::fs::write(snapshots.join("50"), "not a snapshot").unwrap();

    prune(&snapshots, 1).unwrap();

    assert!(
        snapshots.join("50").is_file(),
        "a file prune cannot judge must be left rather than deleted"
    );
    assert!(
        snapshots.join("200").is_dir(),
        "the newest snapshot must stay"
    );
    assert!(!snapshots.join("100").exists());
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
fn two_watched_folders_with_one_name_keep_their_own_copies() {
    let root = crate::test_support::scratch("tell-snapshot-one-name");
    let config = root.join(".config").join("omarchy");
    let share = root.join(".local").join("share").join("omarchy");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(&share).unwrap();
    std::fs::write(config.join("theme"), "tokyo-night\n").unwrap();
    std::fs::write(share.join("theme"), "catppuccin\n").unwrap();

    let into = root.join("snapshots");
    let made = snapshot(&[config.clone(), share.clone()], &into, 100).unwrap();
    std::fs::write(config.join("theme"), "changed\n").unwrap();
    std::fs::write(share.join("theme"), "changed\n").unwrap();

    let result = restore(&made, &[config.clone(), share.clone()]);

    assert!(result.failed.is_empty(), "{:?}", result.failed);
    assert_eq!(
        std::fs::read_to_string(config.join("theme")).unwrap(),
        "tokyo-night\n",
        "one folder must not get the other folder's files"
    );
    assert_eq!(
        std::fs::read_to_string(share.join("theme")).unwrap(),
        "catppuccin\n"
    );
}

#[test]
fn an_older_snapshot_that_fits_two_folders_is_refused_rather_than_guessed() {
    let root = crate::test_support::scratch("tell-restore-older-ambiguous");
    let config = root.join(".config").join("omarchy");
    let share = root.join(".local").join("share").join("omarchy");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(&share).unwrap();
    std::fs::write(config.join("theme"), "tokyo-night\n").unwrap();
    std::fs::write(share.join("theme"), "catppuccin\n").unwrap();

    // A snapshot taken before Banshee keyed a copy on the whole path.
    let older = root.join("snapshots").join("100");
    std::fs::create_dir_all(older.join("omarchy")).unwrap();
    std::fs::write(older.join("omarchy").join("theme"), "the merge\n").unwrap();

    let result = restore(&older, &[config.clone(), share.clone()]);

    assert!(result.done.is_empty(), "{:?}", result.done);
    assert_eq!(result.failed.len(), 2, "{:?}", result.failed);
    assert_eq!(
        std::fs::read_to_string(config.join("theme")).unwrap(),
        "tokyo-night\n",
        "a copy that fits two folders must reach neither"
    );
    assert_eq!(
        std::fs::read_to_string(share.join("theme")).unwrap(),
        "catppuccin\n"
    );
}

#[test]
fn a_snapshot_from_before_the_whole_path_key_still_restores() {
    let root = crate::test_support::scratch("tell-restore-older");
    let hypr = root.join("hypr");
    std::fs::create_dir_all(&hypr).unwrap();
    let older = root.join("snapshots").join("100");
    std::fs::create_dir_all(older.join("hypr")).unwrap();
    std::fs::write(older.join("hypr").join("looknfeel.lua"), "gaps = 5\n").unwrap();
    std::fs::write(hypr.join("looknfeel.lua"), "gaps = 40\n").unwrap();

    let result = restore(&older, std::slice::from_ref(&hypr));

    assert_eq!(result.done, vec![hypr.display().to_string()]);
    assert_eq!(
        std::fs::read_to_string(hypr.join("looknfeel.lua")).unwrap(),
        "gaps = 5\n"
    );
}

#[test]
fn a_second_snapshot_in_the_same_second_gets_a_directory_of_its_own() {
    let root = crate::test_support::scratch("tell-snapshot-same-second");
    let hypr = root.join("hypr");
    std::fs::create_dir_all(&hypr).unwrap();
    std::fs::write(hypr.join("looknfeel.lua"), "gaps = 5\n").unwrap();

    let into = root.join("snapshots");
    let first = snapshot(std::slice::from_ref(&hypr), &into, 100).unwrap();
    std::fs::write(hypr.join("looknfeel.lua"), "gaps = 40\n").unwrap();
    let second = snapshot(std::slice::from_ref(&hypr), &into, 100).unwrap();

    assert_ne!(first, second);
    assert_eq!(
        std::fs::read_to_string(first.join(snapshot_key(&hypr)).join("looknfeel.lua")).unwrap(),
        "gaps = 5\n",
        "the second run must not write over the state the first one copied"
    );
    assert_eq!(newest(&into), Some(second));
}

#[test]
fn a_clock_that_went_backwards_does_not_bury_the_newest_snapshot() {
    let root = crate::test_support::scratch("tell-snapshot-backwards");
    let hypr = root.join("hypr");
    std::fs::create_dir_all(&hypr).unwrap();
    std::fs::write(hypr.join("looknfeel.lua"), "gaps = 5\n").unwrap();

    let into = root.join("snapshots");
    let first = snapshot(std::slice::from_ref(&hypr), &into, 1_000).unwrap();
    let second = snapshot(std::slice::from_ref(&hypr), &into, 100).unwrap();

    assert_eq!(newest(&into), Some(second.clone()));
    prune(&into, 1).unwrap();
    assert!(
        second.is_dir(),
        "prune must not delete the snapshot it just took"
    );
    assert!(!first.exists());
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

    let copy = made.join(snapshot_key(&source));
    assert!(
        std::fs::symlink_metadata(copy.join("link.txt"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(copy.join("file.txt")).unwrap(),
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

    let loop_path = made.join(snapshot_key(&source)).join("loop");
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
    // created there. The failure must not touch hypr, which restored already.
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

/// A snapshot from before Banshee keyed a copy on the whole path: the copy sits
/// under the basename alone. It answers with the folder a restore writes to.
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
fn a_timeout_names_the_deadline_that_ran_rather_than_the_configured_one() {
    let config = TellConfig {
        run_timeout_min: 100_000,
        ..TellConfig::default()
    };
    let sentence = timed_out(Headless::OpenCode, run_deadline(&config));
    assert!(
        sentence.contains("1440 minutes"),
        "the run had the ceiling, not the configured value: {sentence}"
    );
    assert!(
        !sentence.contains("Raise tell.run_timeout_min"),
        "a clamped run cannot be given longer: {sentence}"
    );
}

#[test]
fn a_timeout_under_the_ceiling_says_how_to_give_the_agent_longer() {
    let config = TellConfig {
        run_timeout_min: 5,
        ..TellConfig::default()
    };
    let sentence = timed_out(Headless::ClaudeCode, run_deadline(&config));
    assert!(
        sentence.contains("5 minutes") && sentence.contains("Raise tell.run_timeout_min"),
        "{sentence}"
    );
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
fn the_screen_opens_on_the_stored_thread_and_its_own_agent() {
    let session = saved("opencode", 1_000);
    assert_eq!(
        thread_to_show(Some(&session), 1_000 + 60, span(10)),
        Ok((Headless::OpenCode, "ses_one".to_string()))
    );
}

#[test]
fn a_run_that_never_happened_is_said_rather_than_shown() {
    let answer = thread_to_show(None, 1_000, span(10)).unwrap_err();
    assert!(
        answer.contains("Nothing has run yet"),
        "an empty agent staring at the user says nothing: {answer}"
    );
}

#[test]
fn a_thread_past_the_window_is_said_rather_than_shown() {
    let session = saved("opencode", 1_000);
    let answer = thread_to_show(Some(&session), 1_000 + 11 * 60, span(10)).unwrap_err();
    assert!(answer.contains("timed out"), "{answer}");
}

#[test]
fn a_thread_from_an_agent_banshee_cannot_open_names_that_agent() {
    let session = saved("codex", 1_000);
    let answer = thread_to_show(Some(&session), 1_000 + 60, span(10)).unwrap_err();
    assert!(answer.contains("codex"), "{answer}");
}

#[test]
fn the_screen_resumes_the_thread_with_the_root_command() {
    let dir = Path::new("/home/ada/.banshee/tell");
    assert_eq!(
        show_line(
            Headless::OpenCode,
            Path::new("/usr/bin/opencode"),
            dir,
            "ses_one"
        ),
        "cd '/home/ada/.banshee/tell' && '/usr/bin/opencode' --session 'ses_one'"
    );
    assert_eq!(
        show_line(
            Headless::ClaudeCode,
            Path::new("/usr/bin/claude"),
            dir,
            "abc-123"
        ),
        "cd '/home/ada/.banshee/tell' && '/usr/bin/claude' --resume 'abc-123'"
    );
}

#[test]
fn a_home_holding_a_space_or_a_quote_stays_one_word() {
    assert_eq!(
        show_line(
            Headless::OpenCode,
            Path::new("/home/ada's box/bin/opencode"),
            Path::new("/home/ada's box/tell"),
            "ses_one"
        ),
        "cd '/home/ada'\\''s box/tell' && '/home/ada'\\''s box/bin/opencode' --session 'ses_one'"
    );
}

fn executable(dir: &Path, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let file = dir.join(name);
    std::fs::write(&file, body).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn only_path(dir: &Path) -> std::ffi::OsString {
    std::ffi::OsString::from(dir.display().to_string())
}

#[test]
fn omarchys_floating_terminal_wins_over_a_plain_one() {
    let dir = crate::test_support::scratch("tell-terminal-omarchy");
    executable(&dir, "kitty", "#!/bin/sh\n");
    executable(
        &dir,
        "omarchy-launch-floating-terminal-with-presentation",
        "#!/bin/sh\n",
    );
    let (program, words) = terminal(&only_path(&dir)).expect("a terminal must be found");
    assert_eq!(
        program,
        dir.join("omarchy-launch-floating-terminal-with-presentation")
    );
    assert!(
        words.is_empty(),
        "the launcher takes the command as its own arguments"
    );
}

#[test]
fn a_machine_without_omarchy_still_gets_a_screen() {
    let dir = crate::test_support::scratch("tell-terminal-plain");
    executable(&dir, "kitty", "#!/bin/sh\n");
    let (program, words) = terminal(&only_path(&dir)).expect("a terminal must be found");
    assert_eq!(program, dir.join("kitty"));
    assert_eq!(words.to_vec(), vec!["sh", "-c"]);
}

#[test]
fn no_terminal_on_path_is_none_rather_than_a_guess() {
    let dir = crate::test_support::scratch("tell-terminal-none");
    assert!(terminal(&only_path(&dir)).is_none());
}

fn recorded(file: &Path) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = std::fs::read_to_string(file) {
            return text.lines().map(str::to_string).collect();
        }
        assert!(Instant::now() < deadline, "the terminal never ran");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Opens the thread with `dir` as the whole PATH, and keeps the lines `notify`
/// printed. A sibling test can hold the new file open, so the spawn is retried.
fn shown(dir: &Path) -> (Result<Told, BansheeError>, Vec<String>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let said = std::cell::RefCell::new(Vec::new());
        let told = show(
            dir,
            &agent_dir(dir),
            &TellConfig::default(),
            &only_path(dir),
            &|line| {
                said.borrow_mut().push(line.to_string());
            },
        );
        let busy = matches!(&told, Err(BansheeError::Io(error))
            if error.kind() == std::io::ErrorKind::ExecutableFileBusy);
        if !busy || Instant::now() >= deadline {
            return (told, said.into_inner());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A terminal that writes down its arguments, and a thread to open.
fn with_a_thread(name: &str) -> PathBuf {
    let dir = crate::test_support::scratch(name);
    executable(
        &dir,
        "kitty",
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$0.args\"\n",
    );
    write_session(
        &dir,
        &Session {
            agent: "opencode".to_string(),
            id: "ses_one".to_string(),
            at: now_seconds(),
        },
    )
    .unwrap();
    dir
}

#[test]
fn show_hands_the_thread_to_the_terminal_it_found() {
    let dir = with_a_thread("tell-show-spawn");
    executable(&dir, "opencode", "#!/bin/sh\n");

    shown(&dir).0.unwrap();
    assert_eq!(
        recorded(&dir.join("kitty.args")),
        vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "cd '{}' && '{}' --session 'ses_one'",
                dir.join("run").display(),
                dir.join("opencode").display()
            ),
        ],
        "the terminal gets the resolved binary: its PATH is not the daemon's"
    );
}

#[test]
fn the_screen_reads_the_thread_from_banshees_directory_and_opens_the_agents() {
    let dir = with_a_thread("tell-show-dirs");
    executable(&dir, "opencode", "#!/bin/sh\n");

    // The directory below is spelled out. One derived from `agent_dir` would
    // agree with it however `agent_dir` is written.
    shown(&dir).0.unwrap();
    let line = recorded(&dir.join("kitty.args")).pop().unwrap();
    assert!(
        line.starts_with(&format!("cd '{}' ", dir.join("run").display())),
        "an agent on a screen writes where it is started, so it starts where \
         the headless one does: {line}"
    );
}

#[test]
fn show_says_what_it_opens_rather_than_returning_it() {
    let dir = with_a_thread("tell-show-said");
    executable(&dir, "opencode", "#!/bin/sh\n");

    let (told, said) = shown(&dir);
    assert_eq!(said, vec!["Opening the thread in opencode.".to_string()]);
    assert_eq!(
        told.unwrap(),
        Told::default(),
        "a returned reply never reaches the hotkey user"
    );
}

#[test]
fn show_refuses_an_agent_that_left_the_path() {
    let dir = with_a_thread("tell-show-gone");

    let (told, said) = shown(&dir);
    let error = told.unwrap_err();
    assert!(
        error.to_string().contains("opencode is not on PATH"),
        "{error}"
    );
    assert!(said.is_empty(), "{said:?}");
    assert!(
        !dir.join("kitty.args").exists(),
        "a terminal that can only print `command not found` must not open"
    );
}

#[test]
fn show_opens_nothing_when_there_is_no_thread() {
    let dir = crate::test_support::scratch("tell-show-empty");
    executable(
        &dir,
        "kitty",
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$0.args\"\n",
    );
    let error = shown(&dir).0.unwrap_err();
    assert!(error.to_string().contains("Nothing has run yet"), "{error}");
    assert!(
        !dir.join("kitty.args").exists(),
        "an empty agent must not be opened"
    );
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
        Warning::LostOutput(
            "opencode finished, but its output did not arrive in time. Its reply is lost. \
             The thread is kept."
                .to_string()
        )
    );
    assert!(
        lost_output_warning(Headless::OpenCode, false)
            .text()
            .ends_with("starts a new thread."),
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
        Some(Warning::DeniedTools(
            "claude was refused these tools, so it may have worked in silence: \
             mcp__banshee__speak_status"
                .to_string()
        ))
    );
    assert_eq!(denied_warning(Headless::ClaudeCode, &[]), None);
}

/// What one run printed and did, in the order it happened.
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

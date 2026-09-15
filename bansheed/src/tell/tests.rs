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
        resume(Some(&session), "opencode", 1_000 + 9 * 60, 10),
        Some("ses_one".to_string())
    );
}

#[test]
fn a_command_past_the_window_starts_a_new_thread() {
    let session = saved("opencode", 1_000);
    assert_eq!(
        resume(Some(&session), "opencode", 1_000 + 11 * 60, 10),
        None
    );
}

#[test]
fn a_different_agent_does_not_inherit_the_thread() {
    let session = saved("opencode", 1_000);
    assert_eq!(resume(Some(&session), "claude", 1_000 + 60, 10), None);
}

#[test]
fn no_saved_thread_starts_a_new_one() {
    assert_eq!(resume(None, "opencode", 1_000, 10), None);
}

#[test]
fn a_clock_that_went_backwards_starts_a_new_thread() {
    let session = saved("opencode", 2_000);
    assert_eq!(resume(Some(&session), "opencode", 1_000, 10), None);
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
    // Measured: OpenCode's only working permission flag is --auto, which allows
    // any edit anywhere. Nothing narrower runs without a hang.
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
    // Measured on both agents: the `--` separator keeps it out of the flags.
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
    // The one failure that breaks the loop in silence. A reader of a failed run
    // must be able to find it.
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
    // A half-written file must not stop the next command. A lost thread costs
    // one repeated sentence; a refused run costs the feature.
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
    // Untouched keys keep their defaults.
    assert_eq!(config.tell.snapshots, 10);
}

#[test]
fn a_config_with_no_tell_section_still_loads() {
    let config = crate::config::Config::parse("[audio]\n").unwrap();
    assert_eq!(config.tell.thread_timeout_min, 10);
}

#[test]
fn one_run_at_a_time_and_the_lock_frees_when_it_ends() {
    // Every scenario here shares one process-wide atomic, so they all live in
    // this one test: two test functions racing for the lock would be flaky.
    let long = std::time::Duration::from_secs(600);

    // Two agents editing the same files at once leave a config neither wrote,
    // and the second run's snapshot holds the first run's half-done work.
    let dir = crate::test_support::scratch("tell-lock");
    {
        let _first = RunLock::take(&dir, long).expect("the first run takes the lock");
        assert!(
            RunLock::take(&dir, long).is_none(),
            "a second run must be refused"
        );
    }
    assert!(
        RunLock::take(&dir, long).is_some(),
        "the lock must free on drop"
    );

    // A lock file older than the run's own deadline belonged to a run that
    // was killed, not one still working. Refusing forever would wedge the
    // feature past any recovery.
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

    // A half-written or corrupt lock file must not wedge the feature either.
    let garbled_dir = crate::test_support::scratch("tell-lock-garbled");
    std::fs::write(garbled_dir.join("run.lock"), "not a timestamp").unwrap();
    assert!(
        RunLock::take(&garbled_dir, long).is_some(),
        "a garbled lock file must be treated as stale"
    );

    // A spurious takeover (the margin `run()` adds still exceeded, say) means
    // the file now names a different holder by the time this one drops.
    // Deleting it then would delete a live lock rather than a dead one.
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
    let Ran::Finished(output) = ran else {
        panic!("a command that finishes in time must not read as timed out");
    };
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
}

#[test]
fn a_descendant_holding_the_pipe_does_not_hold_the_call_open() {
    // `opencode run` starts a local server, so a child that exits while a
    // descendant keeps running (and keeps the pipe open) is the case this
    // guards, not an edge case. A shell that backgrounds a long sleep and
    // exits stands in for it: the immediate child is gone almost at once,
    // but the backgrounded sleep inherits the piped stdout and stderr and
    // holds them open for a hundred seconds.
    let path = std::env::var_os("PATH").unwrap_or_default();
    let sh = crate::status::resolve("sh", &path).expect("sh must be on PATH to test this");
    let start = std::time::Instant::now();
    let ran = run_bounded(
        &sh,
        &["-c".to_string(), "sleep 100 & exit 0".to_string()],
        &std::env::temp_dir(),
        &path,
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    assert!(matches!(ran, Ran::Finished(_)));
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "a descendant holding the pipe must not hold the call open"
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

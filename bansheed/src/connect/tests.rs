use super::*;
use std::path::PathBuf;

pub(super) fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("banshee-connect-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub(super) fn env_at(home: &std::path::Path) -> Env {
    Env {
        home: home.to_path_buf(),
        claude_config_dir: home.join(".claude"),
        banshee: PathBuf::from("/opt/banshee/bin/banshee"),
        shim: Some(PathBuf::from("/opt/banshee/bin/banshee-mcp-shim")),
        path: OsString::from("/usr/local/bin"),
        on_path: Vec::new(),
        claude_shim: None,
    }
}

/// Where a machine would have a binary, and where `found` records it.
fn found_at(binary: &str) -> String {
    format!("/usr/local/bin/{binary}")
}

/// Records a binary as found, so a plan can carry the path detection kept.
fn found(env: &mut Env, binary: &'static str) {
    env.on_path.push((binary, PathBuf::from(found_at(binary))));
}

/// A file write runs no program, so the PATH is not part of what it does.
fn apply_write(change: &Change) -> Result<(), BansheeError> {
    apply(change, OsStr::new(""))
}

fn installed_env(home: &std::path::Path, agent: Agent) -> Env {
    let mut env = env_at(home);
    match agent.signal() {
        Signal::OnPath(binary) => found(&mut env, binary),
        Signal::HomeDir(dir) => std::fs::create_dir_all(home.join(dir)).unwrap(),
    }
    env
}

const SHIM: &str = "/opt/banshee/bin/banshee-mcp-shim";

#[test]
fn detection_searches_the_resolved_path_not_the_process_path() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("resolved-path");
    // Executable, because an installed CLI is: a file it cannot run is not one
    std::fs::write(dir.join("codex"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(dir.join("codex"), std::fs::Permissions::from_mode(0o755)).unwrap();

    let env = Env::with_shell_path(Some(OsString::from(&dir))).unwrap();
    let expected = dir.join("codex");
    assert_eq!(env.program(Agent::Codex), Some(expected.as_path()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_banner_before_the_markers_is_not_part_of_the_path() {
    let output = format!("Welcome to zsh\n{PATH_START}/usr/bin:/bin{PATH_END}\n");
    assert_eq!(extract_path(&output), Some(OsString::from("/usr/bin:/bin")));
}

#[test]
fn a_missing_start_marker_yields_none() {
    let output = format!("/usr/bin:/bin{PATH_END}");
    assert_eq!(extract_path(&output), None);
}

#[test]
fn a_missing_end_marker_yields_none() {
    let output = format!("{PATH_START}/usr/bin:/bin");
    assert_eq!(extract_path(&output), None);
}

#[test]
fn an_empty_path_does_not_search_the_working_directory() {
    assert!(
        std::path::Path::new("Cargo.toml").is_file(),
        "test assumes the crate root as the working directory"
    );
    assert!(crate::status::resolve("Cargo.toml", &OsString::new()).is_none());
}

#[test]
fn a_resolver_that_fails_falls_back_to_the_process_path() {
    assert_eq!(
        with_fallback(None),
        std::env::var_os("PATH").unwrap_or_default()
    );
}

#[test]
fn an_empty_resolved_path_falls_back_to_the_process_path() {
    assert_eq!(
        with_fallback(Some(OsString::new())),
        std::env::var_os("PATH").unwrap_or_default()
    );
}

#[test]
fn a_row_reports_absent_installed_and_connected_apart() {
    let home = scratch("rows");
    let env = env_at(&home);
    let row = row(Agent::Cursor, &env);
    assert_eq!(row.id, "cursor");
    assert_eq!(row.name, "Cursor");
    assert_eq!(row.presence, "absent");
    assert!(row.note.contains("Not installed"));
}

#[test]
fn an_installed_agent_reads_found_before_it_is_connected_and_connected_after() {
    let home = scratch("rows-connected");
    let env = installed_env(&home, Agent::Cursor);
    assert_eq!(row(Agent::Cursor, &env).presence, "found");

    for change in plan(Agent::Cursor, &env).unwrap() {
        apply(&change, &env.path).unwrap();
    }
    assert_eq!(row(Agent::Cursor, &env).presence, "connected");
}

#[test]
fn every_agent_has_a_display_name_that_is_not_its_slug() {
    for agent in Agent::ALL {
        assert_ne!(agent.display_name(), agent.name(), "{agent:?}");
    }
    assert_eq!(Agent::ClaudeCode.display_name(), "Claude Code");
}

#[test]
fn a_file_change_carries_its_path_and_a_command_does_not() {
    let home = scratch("planned");
    let env = installed_env(&home, Agent::Cursor);
    let planned: Vec<_> = plan(Agent::Cursor, &env)
        .unwrap()
        .iter()
        .map(planned_change)
        .collect();
    assert!(planned.iter().any(|change| change.path.is_some()));
    for change in &planned {
        assert!(!change.diff.is_empty());
    }
}

#[test]
fn cursor_is_installed_when_its_dir_exists_and_the_clis_when_on_path() {
    let home = scratch("detect-new");
    let mut env = env_at(&home);
    assert_eq!(
        detect(Agent::Cursor, &env),
        Presence::NotInstalled {
            looked_for: "~/.cursor/".into()
        }
    );
    assert_eq!(
        detect(Agent::Codex, &env),
        Presence::NotInstalled {
            looked_for: "codex on PATH".into()
        }
    );
    assert_eq!(
        detect(Agent::Antigravity, &env),
        Presence::NotInstalled {
            looked_for: "agy on PATH".into()
        }
    );
    std::fs::create_dir_all(home.join(".cursor")).unwrap();
    found(&mut env, "codex");
    found(&mut env, "agy");
    assert_eq!(detect(Agent::Cursor, &env), Presence::Installed);
    assert_eq!(detect(Agent::Codex, &env), Presence::Installed);
    assert_eq!(detect(Agent::Antigravity, &env), Presence::Installed);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn mcp_server_is_added_to_a_missing_file() {
    let after = with_mcp_server(None, "mcp.json", Path::new(SHIM))
        .unwrap()
        .expect("a change");
    let value: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(
        value,
        serde_json::json!({ "mcpServers": { "banshee": { "command": SHIM } } })
    );
    assert!(after.ends_with("}\n"));
}

#[test]
fn mcp_server_keeps_other_servers_and_key_order() {
    let before = "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"other\": { \"command\": \"x\" },\n  },\n}\n";
    let after = with_mcp_server(Some(before), "settings.json", Path::new(SHIM))
        .unwrap()
        .expect("a change");
    let value: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(value["theme"], "dark");
    assert_eq!(value["mcpServers"]["other"]["command"], "x");
    assert_eq!(value["mcpServers"]["banshee"]["command"], SHIM);
    let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["theme", "mcpServers"]);
}

#[test]
fn mcp_server_that_reaches_the_shim_means_no_change() {
    let absolute = format!(r#"{{"mcpServers":{{"banshee":{{"command":"{SHIM}"}}}}}}"#);
    assert_eq!(
        with_mcp_server(Some(&absolute), "mcp.json", Path::new(SHIM)).unwrap(),
        None
    );
    let bare = r#"{"mcpServers":{"banshee":{"command":"banshee-mcp-shim"}}}"#;
    assert!(
        with_mcp_server(Some(bare), "mcp.json", Path::new(SHIM))
            .unwrap()
            .is_some(),
        "a bare name never reaches the shim"
    );
}

#[test]
fn mcp_server_errors_name_the_file() {
    let error =
        with_mcp_server(Some("[1]"), "mcp.json", Path::new(SHIM)).expect_err("not an object");
    assert!(error.to_string().starts_with("mcp.json "), "{error}");
}

#[test]
fn codex_server_is_added_to_a_missing_file() {
    let after = with_codex_server(None, Path::new(SHIM))
        .unwrap()
        .expect("a change");
    assert_eq!(
        after,
        format!("[mcp_servers.banshee]\ncommand = \"{SHIM}\"\n")
    );
}

#[test]
fn codex_server_keeps_comments_and_other_tables() {
    let before =
        "# my codex config\nmodel = \"o3\" # keep\n\n[mcp_servers.other]\ncommand = \"x\"\n";
    let after = with_codex_server(Some(before), Path::new(SHIM))
        .unwrap()
        .expect("a change");
    assert!(
        after.starts_with("# my codex config\nmodel = \"o3\" # keep\n"),
        "{after}"
    );
    assert!(
        after.contains("[mcp_servers.other]\ncommand = \"x\"\n"),
        "{after}"
    );
    assert!(
        after.contains(&format!("[mcp_servers.banshee]\ncommand = \"{SHIM}\"\n")),
        "{after}"
    );
}

#[test]
fn codex_server_that_reaches_the_shim_means_no_change() {
    let absolute = format!("[mcp_servers.banshee]\ncommand = \"{SHIM}\"\n");
    assert_eq!(
        with_codex_server(Some(&absolute), Path::new(SHIM)).unwrap(),
        None
    );
    let bare = "[mcp_servers.banshee]\ncommand = \"banshee-mcp-shim\"\n";
    let after = with_codex_server(Some(bare), Path::new(SHIM))
        .unwrap()
        .expect("a bare name never reaches the shim");
    assert!(after.contains(&format!("command = \"{SHIM}\"")), "{after}");
    let stale = "[mcp_servers.banshee]\ncommand = \"/old/banshee-mcp-shim\"\nargs = []\n";
    let after = with_codex_server(Some(stale), Path::new(SHIM))
        .unwrap()
        .expect("a change");
    assert!(after.contains(&format!("command = \"{SHIM}\"")), "{after}");
    assert!(
        after.contains("args = []"),
        "other keys of the entry survive: {after}"
    );
}

#[test]
fn codex_refuses_an_mcp_servers_that_is_not_a_table() {
    for before in [
        "mcp_servers = { banshee = { command = \"x\" } }\n",
        "[[mcp_servers]]\ncommand = \"x\"\n",
        "[mcp_servers]\nbanshee = { command = \"x\" }\n",
    ] {
        let error = with_codex_server(Some(before), Path::new(SHIM)).expect_err(before);
        assert!(
            matches!(error, BansheeError::Rejected(_)),
            "{before}: {error}"
        );
    }
}

#[test]
fn mcp_server_refuses_an_mcp_servers_that_is_not_an_object() {
    let error = with_mcp_server(Some(r#"{"mcpServers":[1]}"#), "mcp.json", Path::new(SHIM))
        .expect_err("a list is not a server map");
    assert!(matches!(error, BansheeError::Rejected(_)), "{error}");
    assert_eq!(error.to_string(), "mcp.json mcpServers is not an object");
}

#[test]
fn codex_errors_name_the_file() {
    let error = with_codex_server(Some("not = = toml"), Path::new(SHIM)).expect_err("bad toml");
    assert!(error.to_string().starts_with("config.toml "), "{error}");
}

#[test]
fn the_new_agents_plan_their_own_files_and_need_the_shim() {
    let home = scratch("plan-new");
    let mut env = env_at(&home);
    std::fs::create_dir_all(home.join(".cursor")).unwrap();
    found(&mut env, "codex");
    found(&mut env, "agy");
    for (agent, file) in [
        (Agent::Cursor, ".cursor/mcp.json"),
        (Agent::Codex, ".codex/config.toml"),
        (Agent::Antigravity, ".gemini/config/mcp_config.json"),
    ] {
        match &plan(agent, &env).unwrap()[..] {
            [
                Change::WriteFile {
                    path,
                    before: None,
                    executable: false,
                    ..
                },
            ] => {
                assert_eq!(path, &home.join(file));
            }
            other => panic!("{agent:?}: {other:?}"),
        }
    }
    env.shim = None;
    for agent in [Agent::Cursor, Agent::Codex, Agent::Antigravity] {
        assert!(plan(agent, &env).is_err(), "{agent:?} must need the shim");
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_bare_shim_name_never_reaches_the_shim_even_when_registered() {
    let home = scratch("claude-bare-registered");
    let mut env = env_at(&home);
    found(&mut env, "claude");
    env.claude_shim = Some("banshee-mcp-shim".into());
    let script = home.join(".claude/hooks/banshee-speak-check.sh");
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();
    std::fs::write(&script, hook_script(&env.banshee)).unwrap();
    std::fs::write(
        home.join(".claude/settings.json"),
        format!(
            r#"{{"hooks":{{"Stop":[{{"hooks":[{{"type":"command","command":"bash '{}'"}}]}}]}}}}"#,
            script.display()
        ),
    )
    .unwrap();
    let changes = plan(Agent::ClaudeCode, &env).unwrap();
    assert_eq!(
        changes,
        vec![
            Change::Run {
                argv: [
                    &found_at("claude"),
                    "mcp",
                    "remove",
                    "--scope",
                    "user",
                    "banshee"
                ]
                .map(String::from)
                .to_vec()
            },
            Change::Run {
                argv: [
                    &found_at("claude"),
                    "mcp",
                    "add",
                    "--scope",
                    "user",
                    "banshee",
                    "--",
                    "/opt/banshee/bin/banshee-mcp-shim"
                ]
                .map(String::from)
                .to_vec()
            },
        ],
        "the working hook is untouched; only the shim registration is reissued"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_bare_shim_name_with_no_hook_set_up_is_still_reissued() {
    let home = scratch("claude-bare-no-hook");
    let mut env = env_at(&home);
    found(&mut env, "claude");
    env.claude_shim = Some("banshee-mcp-shim".into());
    let changes = plan(Agent::ClaudeCode, &env).unwrap();
    assert!(
        matches!(&changes[0], Change::Run { argv } if argv[2] == "remove"),
        "{changes:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn opencode_bare_shim_name_never_means_no_change() {
    let before =
        r#"{"mcp":{"banshee":{"type":"local","enabled":true,"command":["banshee-mcp-shim"]}}}"#;
    let shim = std::path::Path::new("/opt/banshee/bin/banshee-mcp-shim");
    assert!(
        with_opencode_server(Some(before), shim).unwrap().is_some(),
        "a bare name never reaches the shim"
    );
}

#[test]
fn claude_is_installed_when_on_path() {
    let home = scratch("detect-claude");
    let mut env = env_at(&home);
    assert_eq!(
        detect(Agent::ClaudeCode, &env),
        Presence::NotInstalled {
            looked_for: "claude on PATH".into()
        }
    );
    found(&mut env, "claude");
    assert_eq!(detect(Agent::ClaudeCode, &env), Presence::Installed);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn opencode_is_installed_when_its_config_dir_exists() {
    let home = scratch("detect-opencode");
    let env = env_at(&home);
    assert_eq!(
        detect(Agent::OpenCode, &env),
        Presence::NotInstalled {
            looked_for: "~/.config/opencode/".into()
        }
    );
    std::fs::create_dir_all(home.join(".config/opencode")).unwrap();
    assert_eq!(detect(Agent::OpenCode, &env), Presence::Installed);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn pi_is_installed_when_its_agent_dir_exists() {
    let home = scratch("detect-pi");
    let env = env_at(&home);
    assert_eq!(
        detect(Agent::Pi, &env),
        Presence::NotInstalled {
            looked_for: "~/.pi/agent/".into()
        }
    );
    std::fs::create_dir_all(home.join(".pi/agent")).unwrap();
    assert_eq!(detect(Agent::Pi, &env), Presence::Installed);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn every_agent_has_a_cli_name() {
    let names: Vec<&str> = Agent::ALL.iter().map(|a| a.name()).collect();
    assert_eq!(
        names,
        ["antigravity", "claude", "codex", "cursor", "opencode", "pi"]
    );
}

#[test]
fn pi_plan_writes_the_extension_when_absent() {
    let home = scratch("pi-absent");
    std::fs::create_dir_all(home.join(".pi/agent")).unwrap();
    let changes = plan(Agent::Pi, &env_at(&home)).unwrap();
    assert_eq!(
        changes,
        vec![Change::WriteFile {
            path: home.join(".pi/agent/extensions/banshee.ts"),
            before: None,
            after: PI_EXTENSION.to_string(),
            executable: false,
        }]
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn pi_plan_shows_the_old_text_when_it_differs() {
    let home = scratch("pi-stale");
    let path = home.join(".pi/agent/extensions/banshee.ts");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "// old\n").unwrap();
    let changes = plan(Agent::Pi, &env_at(&home)).unwrap();
    assert_eq!(changes.len(), 1);
    match &changes[0] {
        Change::WriteFile { before, .. } => assert_eq!(before.as_deref(), Some("// old\n")),
        other => panic!("expected a file write, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn pi_plan_is_empty_when_already_connected() {
    let home = scratch("pi-connected");
    let path = home.join(".pi/agent/extensions/banshee.ts");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, PI_EXTENSION).unwrap();
    assert!(plan(Agent::Pi, &env_at(&home)).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_hook_is_added_to_settings_with_no_hooks() {
    let before = "{\n  \"model\": \"opus\",\n  \"theme\": \"dark\"\n}\n";
    let after = with_stop_hook(Some(before), "bash '/x/banshee-speak-check.sh'")
        .unwrap()
        .expect("a change");
    let value: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(value["model"], "opus");
    assert_eq!(value["theme"], "dark");
    assert_eq!(
        value["hooks"]["Stop"][0],
        serde_json::json!({ "hooks": [{
                "type": "command",
                "command": "bash '/x/banshee-speak-check.sh'",
                "timeout": 15,
                "statusMessage": "Checking you spoke",
            }] })
    );
    // Key order and indentation survive, so the diff shows only the addition
    assert!(
        after.starts_with("{\n  \"model\": \"opus\",\n  \"theme\": \"dark\",\n  \"hooks\""),
        "{after}"
    );
    assert!(after.ends_with("}\n"));
}

#[test]
fn a_hook_is_appended_after_other_stop_hooks() {
    let before = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo hi"}]}]}}"#;
    let after = with_stop_hook(Some(before), "bash '/x/banshee-speak-check.sh'")
        .unwrap()
        .expect("a change");
    let value: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(value["hooks"]["Stop"][0]["hooks"][0]["command"], "echo hi");
    assert_eq!(
        value["hooks"]["Stop"][1]["hooks"][0]["command"],
        "bash '/x/banshee-speak-check.sh'"
    );
}

#[test]
fn a_hook_already_present_at_any_path_means_no_change() {
    let before = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"bash '/elsewhere/hooks/banshee-speak-check.sh'"}]}]}}"#;
    assert_eq!(
        with_stop_hook(Some(before), "bash '/x/banshee-speak-check.sh'").unwrap(),
        None
    );
}

#[test]
fn a_missing_settings_file_gets_only_the_hook() {
    let after = with_stop_hook(None, "bash '/x/banshee-speak-check.sh'")
        .unwrap()
        .expect("a change");
    let value: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 1);
    assert_eq!(
        value["hooks"]["Stop"][0]["hooks"][0]["command"],
        "bash '/x/banshee-speak-check.sh'"
    );
}

#[test]
fn unreadable_settings_are_an_error_not_a_rewrite() {
    let error = with_stop_hook(Some("{ not json"), "x").expect_err("must not guess");
    assert!(error.to_string().contains("settings.json"), "{error}");
}

#[test]
fn the_hook_script_names_the_installed_binary() {
    let script = hook_script(std::path::Path::new("/opt/banshee/bin/banshee"));
    assert!(
        script.contains("banshee=\"${BANSHEE_BIN:-/opt/banshee/bin/banshee}\""),
        "{script}"
    );
    assert!(!script.contains("@BANSHEE_BIN@"));
}

#[test]
fn claude_plan_adds_the_server_the_script_and_the_hook() {
    let home = scratch("claude-fresh");
    let mut env = env_at(&home);
    found(&mut env, "claude");
    let changes = plan(Agent::ClaudeCode, &env).unwrap();
    assert_eq!(changes.len(), 3, "{changes:?}");
    assert_eq!(
        changes[0],
        Change::Run {
            argv: [
                &found_at("claude"),
                "mcp",
                "add",
                "--scope",
                "user",
                "banshee",
                "--",
                "/opt/banshee/bin/banshee-mcp-shim"
            ]
            .map(String::from)
            .to_vec()
        }
    );
    match &changes[1] {
        Change::WriteFile {
            path,
            before,
            executable,
            ..
        } => {
            assert_eq!(path, &home.join(".claude/hooks/banshee-speak-check.sh"));
            assert_eq!(*before, None);
            assert!(executable);
        }
        other => panic!("{other:?}"),
    }
    match &changes[2] {
        Change::WriteFile {
            path,
            after,
            executable,
            ..
        } => {
            assert_eq!(path, &home.join(".claude/settings.json"));
            assert!(!executable);
            let expected = format!(
                "bash '{}'",
                home.join(".claude/hooks/banshee-speak-check.sh").display()
            );
            assert!(after.contains(&expected), "{after}");
        }
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn claude_plan_skips_the_server_when_claude_already_has_it() {
    let home = scratch("claude-has-mcp");
    let mut env = env_at(&home);
    found(&mut env, "claude");
    env.claude_shim = Some("/opt/banshee/bin/banshee-mcp-shim".into());
    let changes = plan(Agent::ClaudeCode, &env).unwrap();
    assert!(
        !changes.iter().any(|c| matches!(c, Change::Run { .. })),
        "{changes:?}"
    );
    assert_eq!(changes.len(), 2);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn claude_plan_leaves_a_working_hook_alone_wherever_its_script_lives() {
    let home = scratch("claude-hook-elsewhere");
    let mut env = env_at(&home);
    found(&mut env, "claude");
    env.claude_shim = Some("/opt/banshee/bin/banshee-mcp-shim".into());
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::create_dir_all(home.join("elsewhere")).unwrap();
    std::fs::write(home.join("elsewhere/banshee-speak-check.sh"), "# theirs\n").unwrap();
    std::fs::write(
            home.join(".claude/settings.json"),
            format!(
                r#"{{"hooks":{{"Stop":[{{"hooks":[{{"type":"command","command":"bash '{}/elsewhere/banshee-speak-check.sh'"}}]}}]}}}}"#,
                home.display()
            ),
        )
        .unwrap();
    assert!(plan(Agent::ClaudeCode, &env).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn claude_plan_repairs_a_registered_script_that_is_missing() {
    let home = scratch("claude-hook-missing");
    let mut env = env_at(&home);
    found(&mut env, "claude");
    env.claude_shim = Some("/opt/banshee/bin/banshee-mcp-shim".into());
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    let registered = home.join("gone/banshee-speak-check.sh");
    std::fs::write(
        home.join(".claude/settings.json"),
        format!(
            r#"{{"hooks":{{"Stop":[{{"hooks":[{{"type":"command","command":"bash '{}'"}}]}}]}}}}"#,
            registered.display()
        ),
    )
    .unwrap();
    match &plan(Agent::ClaudeCode, &env).unwrap()[..] {
        [
            Change::WriteFile {
                path,
                before: None,
                executable: true,
                ..
            },
        ] => {
            assert_eq!(path, &registered)
        }
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_hook_in_settings_local_counts_as_registered() {
    let home = scratch("claude-hook-local");
    let mut env = env_at(&home);
    found(&mut env, "claude");
    env.claude_shim = Some("/opt/banshee/bin/banshee-mcp-shim".into());
    let script = home.join(".claude/hooks/banshee-speak-check.sh");
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();
    std::fs::write(&script, hook_script(&env.banshee)).unwrap();
    std::fs::write(
        home.join(".claude/settings.local.json"),
        format!(
            r#"{{"hooks":{{"Stop":[{{"hooks":[{{"type":"command","command":"bash '{}'"}}]}}]}}}}"#,
            script.display()
        ),
    )
    .unwrap();
    assert!(plan(Agent::ClaudeCode, &env).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn an_absolute_command_through_a_symlink_reaches_the_shim() {
    let home = scratch("shim-symlink");
    let real = home.join("app/banshee-mcp-shim");
    std::fs::create_dir_all(home.join("app")).unwrap();
    std::fs::create_dir_all(home.join("bin")).unwrap();
    std::fs::write(&real, "").unwrap();
    let link = home.join("bin/banshee-mcp-shim");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(reaches_shim(Some(&link.display().to_string()), &real));
    assert!(!reaches_shim(Some("/nowhere/banshee-mcp-shim"), &real));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_bare_name_never_reaches_the_shim() {
    assert!(!reaches_shim(Some(SHIM_NAME), Path::new(SHIM)));
}

#[test]
fn an_absolute_path_to_the_shim_still_reaches_it() {
    assert!(reaches_shim(Some(SHIM), Path::new(SHIM)));

    let home = scratch("shim-still-reaches");
    let real = home.join("app/banshee-mcp-shim");
    std::fs::create_dir_all(home.join("app")).unwrap();
    std::fs::create_dir_all(home.join("bin")).unwrap();
    std::fs::write(&real, "").unwrap();
    let link = home.join("bin/banshee-mcp-shim");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(reaches_shim(Some(&link.display().to_string()), &real));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn json_hosts_keep_the_entry_other_keys() {
    let before = r#"{"mcpServers":{"banshee":{"command":"/old/shim","env":{"KEEP":"me"}}}}"#;
    let after = with_mcp_server(Some(before), "mcp.json", Path::new(SHIM))
        .unwrap()
        .expect("a change");
    let value: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(value["mcpServers"]["banshee"]["command"], SHIM);
    assert_eq!(value["mcpServers"]["banshee"]["env"]["KEEP"], "me");

    let before = r#"{"mcp":{"banshee":{"type":"local","command":["/old/shim"],"timeout":20000}}}"#;
    let after = with_opencode_server(Some(before), Path::new(SHIM))
        .unwrap()
        .expect("a change");
    let value: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(value["mcp"]["banshee"]["command"][0], SHIM);
    assert_eq!(value["mcp"]["banshee"]["timeout"], 20000);
}

#[test]
fn opencode_entry_with_no_enabled_key_counts_as_enabled() {
    let before = format!(r#"{{"mcp":{{"banshee":{{"type":"local","command":["{SHIM}"]}}}}}}"#);
    assert_eq!(
        with_opencode_server(Some(&before), Path::new(SHIM)).unwrap(),
        None
    );
    let disabled = format!(
        r#"{{"mcp":{{"banshee":{{"type":"local","enabled":false,"command":["{SHIM}"]}}}}}}"#
    );
    let after = with_opencode_server(Some(&disabled), Path::new(SHIM))
        .unwrap()
        .expect("a change");
    let value: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(value["mcp"]["banshee"]["enabled"], true);
}

#[test]
fn claude_plan_honours_the_config_dir() {
    let home = scratch("claude-config-dir");
    let mut env = env_at(&home);
    found(&mut env, "claude");
    env.claude_shim = Some("/opt/banshee/bin/banshee-mcp-shim".into());
    env.claude_config_dir = home.join(".claude-work");
    let changes = plan(Agent::ClaudeCode, &env).unwrap();
    let paths: Vec<&PathBuf> = changes
        .iter()
        .filter_map(|c| match c {
            Change::WriteFile { path, .. } => Some(path),
            Change::Run { .. } => None,
        })
        .collect();
    assert_eq!(
        paths,
        [
            &home.join(".claude-work/hooks/banshee-speak-check.sh"),
            &home.join(".claude-work/settings.json")
        ]
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn opencode_server_is_added_to_a_jsonc_file_with_comments_and_trailing_commas() {
    let before = "// top\n{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  /* keep */ \"plugin\": [\n    \"a\",\n  ],\n  \"mcp\": {\n    \"other\": { \"type\": \"remote\", \"url\": \"https://x\", \"enabled\": false },\n  },\n}\n";
    let after = with_opencode_server(
        Some(before),
        std::path::Path::new("/opt/banshee/bin/banshee-mcp-shim"),
    )
    .unwrap()
    .expect("a change");
    let value: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(value["$schema"], "https://opencode.ai/config.json");
    assert_eq!(value["plugin"][0], "a");
    assert_eq!(value["mcp"]["other"]["type"], "remote");
    assert_eq!(value["mcp"]["banshee"]["type"], "local");
    assert_eq!(value["mcp"]["banshee"]["enabled"], true);
    assert_eq!(
        value["mcp"]["banshee"]["command"][0],
        "/opt/banshee/bin/banshee-mcp-shim"
    );
    let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["$schema", "plugin", "mcp"], "key order must survive");
}

#[test]
fn opencode_server_is_updated_when_it_points_elsewhere() {
    let before =
        r#"{"mcp":{"banshee":{"type":"local","enabled":true,"command":["banshee-mcp-shim"]}}}"#;
    let after = with_opencode_server(
        Some(before),
        std::path::Path::new("/opt/banshee/bin/banshee-mcp-shim"),
    )
    .unwrap()
    .expect("a change");
    let value: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(
        value["mcp"]["banshee"]["command"][0],
        "/opt/banshee/bin/banshee-mcp-shim"
    );
}

#[test]
fn opencode_server_already_pointing_at_the_shim_means_no_change() {
    let before = r#"{"mcp":{"banshee":{"type":"local","enabled":true,"command":["/opt/banshee/bin/banshee-mcp-shim"]}}}"#;
    assert_eq!(
        with_opencode_server(
            Some(before),
            std::path::Path::new("/opt/banshee/bin/banshee-mcp-shim")
        )
        .unwrap(),
        None
    );
}

#[test]
fn a_missing_opencode_config_gets_only_the_server() {
    let after = with_opencode_server(
        None,
        std::path::Path::new("/opt/banshee/bin/banshee-mcp-shim"),
    )
    .unwrap()
    .expect("a change");
    let value: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 1);
    assert_eq!(
        value["mcp"]["banshee"]["command"][0],
        "/opt/banshee/bin/banshee-mcp-shim"
    );
}

#[test]
fn a_command_renders_as_one_quoted_line() {
    let change = Change::Run {
        argv: [
            "claude",
            "mcp",
            "add",
            "--",
            "/Applications/My Tools/banshee-mcp-shim",
        ]
        .map(String::from)
        .to_vec(),
    };
    assert_eq!(
        render(&change),
        "$ claude mcp add -- '/Applications/My Tools/banshee-mcp-shim'\n"
    );
}

#[test]
fn a_new_file_renders_as_a_line_count() {
    let change = Change::WriteFile {
        path: PathBuf::from("/h/.pi/agent/extensions/banshee.ts"),
        before: None,
        after: "a\nb\nc\n".into(),
        executable: false,
    };
    assert_eq!(
        render(&change),
        "new file /h/.pi/agent/extensions/banshee.ts, 3 lines\n"
    );
}

#[test]
fn a_changed_file_renders_as_a_unified_diff() {
    let change = Change::WriteFile {
        path: PathBuf::from("/h/settings.json"),
        before: Some("{\n  \"a\": 1\n}\n".into()),
        after: "{\n  \"a\": 1,\n  \"b\": 2\n}\n".into(),
        executable: false,
    };
    let text = render(&change);
    assert!(
        text.starts_with("--- /h/settings.json\n+++ /h/settings.json\n"),
        "{text}"
    );
    assert!(text.contains("+  \"b\": 2\n"), "{text}");
    assert!(
        text.contains("-  \"a\": 1\n") && text.contains("+  \"a\": 1,\n"),
        "{text}"
    );
    // "a": 1 gains a trailing comma, so it is a real change (removed then
    // re-added); only the braces around it are truly unchanged.
    assert!(
        !text.contains("-{\n"),
        "unchanged line shown as removed: {text}"
    );
    assert!(
        !text.contains("-}\n"),
        "unchanged line shown as removed: {text}"
    );
}

#[test]
fn applying_a_file_write_creates_parents_and_sets_the_mode() {
    let home = scratch("apply-write");
    let path = home.join("deep/er/hook.sh");
    apply_write(&Change::WriteFile {
        path: path.clone(),
        before: None,
        after: "#!/bin/sh\n".into(),
        executable: true,
    })
    .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "#!/bin/sh\n");
    assert_eq!(
        std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
        1,
        "no staging file left"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o111,
        0o111
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_failing_command_is_an_error() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("failing-command");
    let script = dir.join("fails");
    std::fs::write(&script, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let error = apply(
        &Change::Run {
            argv: vec![script.display().to_string()],
        },
        OsStr::new("/usr/bin:/bin"),
    )
    .expect_err("the script exits 1");
    assert!(error.to_string().contains("fails"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

// The daemon runs under a supervisor, which hands it four system
// directories on PATH. A plan that names a program the daemon has to
// resolve itself cannot run there, so a plan resolves it first.
#[test]
fn every_program_a_plan_runs_is_absolute() {
    let home = scratch("planned-programs");
    for agent in Agent::ALL {
        let mut env = installed_env(&home, agent);
        // A stale registration, so the plan carries every command it can
        env.claude_shim = Some("/elsewhere/banshee-mcp-shim".to_string());
        for change in plan(agent, &env).expect("a plan") {
            let Change::Run { argv } = change else {
                continue;
            };
            let program = std::path::Path::new(&argv[0]);
            assert!(
                program.is_absolute(),
                "{agent:?} plans to run {program:?}, which the daemon resolves on its own PATH"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&home);
}

// A resolved program is not enough: an agent CLI is often a script, and its
// interpreter is found on the PATH the child is handed.
#[test]
fn a_command_runs_with_the_path_it_was_given() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("run-path");
    let script = dir.join("writes-path");
    let seen = dir.join("seen");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf '%s' \"$PATH\" > '{}'\n", seen.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    apply(
        &Change::Run {
            argv: vec![script.display().to_string()],
        },
        OsStr::new("/handed/to/the/child"),
    )
    .unwrap();

    assert_eq!(
        std::fs::read_to_string(&seen).unwrap(),
        "/handed/to/the/child"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn claude_shim_is_read_from_the_config_dir() {
    let home = scratch("claude-shim-read");
    let file = home.join(".claude.json");
    assert_eq!(registered_claude_shim(&file), None);
    std::fs::write(
        &file,
        r#"{"mcpServers":{"banshee":{"type":"stdio","command":"/x/banshee-mcp-shim","args":[]}}}"#,
    )
    .unwrap();
    assert_eq!(
        registered_claude_shim(&file).as_deref(),
        Some("/x/banshee-mcp-shim")
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn plans_that_need_the_shim_refuse_without_it() {
    let home = scratch("no-shim");
    std::fs::create_dir_all(home.join(".pi/agent")).unwrap();
    let mut env = env_at(&home);
    env.shim = None;
    let error = plan(Agent::ClaudeCode, &env).expect_err("claude needs the shim");
    assert!(matches!(error, BansheeError::Rejected(_)), "{error:?}");
    assert_eq!(
        error.to_string(),
        "banshee-mcp-shim is not beside /opt/banshee/bin/banshee; reinstall so they ship together"
    );
    assert!(plan(Agent::OpenCode, &env).is_err());
    assert_eq!(plan(Agent::Pi, &env).unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn claude_plan_reissues_the_server_when_the_registered_command_differs() {
    let home = scratch("claude-stale-shim");
    let mut env = env_at(&home);
    found(&mut env, "claude");
    env.claude_shim = Some("/old/bin/banshee-mcp-shim".into());
    let changes = plan(Agent::ClaudeCode, &env).unwrap();
    assert_eq!(
        changes[0],
        Change::Run {
            argv: [
                &found_at("claude"),
                "mcp",
                "remove",
                "--scope",
                "user",
                "banshee"
            ]
            .map(String::from)
            .to_vec()
        }
    );
    assert_eq!(
        changes[1],
        Change::Run {
            argv: [
                &found_at("claude"),
                "mcp",
                "add",
                "--scope",
                "user",
                "banshee",
                "--",
                "/opt/banshee/bin/banshee-mcp-shim"
            ]
            .map(String::from)
            .to_vec()
        }
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn claude_plan_repairs_a_missing_script_at_the_canonical_path() {
    let home = scratch("claude-script-gone");
    let mut env = env_at(&home);
    found(&mut env, "claude");
    env.claude_shim = Some("/opt/banshee/bin/banshee-mcp-shim".into());
    let script_path = home.join(".claude/hooks/banshee-speak-check.sh");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(
        home.join(".claude/settings.json"),
        format!(
            r#"{{"hooks":{{"Stop":[{{"hooks":[{{"type":"command","command":"bash '{}'"}}]}}]}}}}"#,
            script_path.display()
        ),
    )
    .unwrap();
    let changes = plan(Agent::ClaudeCode, &env).unwrap();
    match &changes[..] {
        [
            Change::WriteFile {
                path,
                before,
                after,
                executable,
            },
        ] => {
            assert_eq!(path, &script_path);
            assert_eq!(*before, None);
            assert_eq!(after, &hook_script(&env.banshee));
            assert!(executable);
        }
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn opencode_plan_targets_the_jsonc_file() {
    let home = scratch("opencode-plan");
    std::fs::create_dir_all(home.join(".config/opencode")).unwrap();
    let changes = plan(Agent::OpenCode, &env_at(&home)).unwrap();
    match &changes[..] {
        [
            Change::WriteFile {
                path,
                before,
                executable,
                ..
            },
        ] => {
            assert_eq!(path, &home.join(".config/opencode/opencode.jsonc"));
            assert_eq!(*before, None);
            assert!(!executable);
        }
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&home);
}

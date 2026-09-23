use super::*;
use banshee_common::TurnVerdict::{Pass, Speak};

const TABLE: &str = "    1     0 /sbin/launchd
37777     1 /Applications/WezTerm.app/Contents/MacOS/wezterm-gui
90683 37777 -zsh
53325 90683 claude
53377 53325 banshee-mcp-shim
53505 53325 /bin/sh -c \"$0\" turn-end claude || exit 0 /Applications/Banshee.app/Contents/MacOS/banshee
53506 53505 /Applications/Banshee.app/Contents/MacOS/banshee turn-end claude
53617 90683 codex
53674 53617 /Users/someone/Banshee 0.15/banshee-mcp-shim
54185 53617 /Applications/Banshee.app/Contents/MacOS/banshee turn-end codex
54265 90683 agy
54293 54265 banshee-mcp-shi
54358 54265 /usr/local/bin/banshee turn-end antigravity
";

#[test]
fn rows_keep_arguments_with_spaces_whole() {
    let rows = process_rows(TABLE);
    let shim = rows.iter().find(|row| row.pid == 53674).unwrap();
    assert_eq!(shim.ppid, 53617);
    assert_eq!(shim.args, "/Users/someone/Banshee 0.15/banshee-mcp-shim");
}

#[test]
fn the_agent_is_the_nearest_ancestor_that_started_a_shim() {
    let rows = process_rows(TABLE);
    assert_eq!(
        agent_pid(&rows, 53506),
        Some(53325),
        "through sh -c, to a shim run by its bare name"
    );
    assert_eq!(agent_pid(&rows, 53505), Some(53325), "a direct child");
    assert_eq!(
        agent_pid(&rows, 54185),
        Some(53617),
        "a shim path with a space"
    );
}

#[test]
fn a_cut_process_name_is_not_a_shim() {
    assert_eq!(agent_pid(&process_rows(TABLE), 54358), None);
}

#[test]
fn a_process_outside_the_table_has_no_agent() {
    assert_eq!(agent_pid(&process_rows(TABLE), 99999), None);
}

#[test]
fn a_stop_is_not_a_plain_end_when_its_payload_says_so() {
    for agent in [GatedAgent::Claude, GatedAgent::Codex] {
        assert!(not_a_plain_end(
            agent,
            &serde_json::json!({"stop_hook_active": true})
        ));
        assert!(!not_a_plain_end(
            agent,
            &serde_json::json!({"stop_hook_active": false})
        ));
        assert!(!not_a_plain_end(agent, &serde_json::Value::Null));
    }
    let antigravity = |error: &str| serde_json::json!({"terminationReason": "NO_TOOL_CALL", "error": error, "executionNum": 0});
    assert!(not_a_plain_end(
        GatedAgent::Antigravity,
        &antigravity("quota exceeded")
    ));
    assert!(
        !not_a_plain_end(GatedAgent::Antigravity, &antigravity("")),
        "agy's plain end, as measured"
    );
    assert!(!not_a_plain_end(
        GatedAgent::Antigravity,
        &serde_json::Value::Null
    ));
}

#[test]
fn each_agent_hears_its_own_answer() {
    let parsed = |agent, verdict| {
        answer(agent, verdict).map(|line| serde_json::from_str::<serde_json::Value>(&line).unwrap())
    };
    assert_eq!(
        parsed(GatedAgent::Claude, Speak).unwrap()["decision"],
        "block"
    );
    assert_eq!(
        parsed(GatedAgent::Codex, Speak).unwrap()["decision"],
        "block"
    );
    assert_eq!(
        parsed(GatedAgent::Antigravity, Speak).unwrap()["decision"],
        "continue"
    );
    for agent in [
        GatedAgent::Claude,
        GatedAgent::Codex,
        GatedAgent::Antigravity,
    ] {
        assert_eq!(
            answer(agent, Pass),
            None,
            "{agent:?}: a pass prints nothing"
        );
    }
}

#[test]
fn only_codex_is_told_where_its_hidden_tools_are() {
    let reason = |agent| {
        let line = answer(agent, Speak).unwrap();
        serde_json::from_str::<serde_json::Value>(&line).unwrap()["reason"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(reason(GatedAgent::Codex).contains("ALL_TOOLS"));
    assert!(!reason(GatedAgent::Claude).contains("ALL_TOOLS"));
    assert!(reason(GatedAgent::Claude).contains("speak_status"));
}

struct Stalled;

impl std::io::Read for Stalled {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        std::thread::sleep(Duration::from_secs(30));
        Ok(0)
    }
}

#[test]
fn stdin_that_stays_open_is_given_up_on() {
    let started = std::time::Instant::now();
    assert_eq!(
        read_payload(Stalled, Duration::from_millis(50)),
        serde_json::Value::Null
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_payload_that_is_not_json_is_empty() {
    assert_eq!(
        read_payload(&b"not json"[..], STDIN_WAIT),
        serde_json::Value::Null
    );
    assert_eq!(
        read_payload(&br#"{"stop_hook_active":true}"#[..], STDIN_WAIT)["stop_hook_active"],
        true
    );
}

const PLAIN: serde_json::Value = serde_json::Value::Null;

#[tokio::test]
async fn a_daemon_that_does_not_answer_passes() {
    let silent = async |_: u32, _: bool| {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(serde_json::json!({"verdict": "speak"}))
    };
    let started = std::time::Instant::now();
    assert_eq!(
        verdict(
            GatedAgent::Claude,
            &PLAIN,
            Some(1),
            silent,
            Duration::from_millis(50)
        )
        .await,
        Pass
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn every_failure_passes() {
    let refusing = async |_: u32, _: bool| Err(banshee_common::error::BansheeError::NoAnswer);
    let garbled = async |_: u32, _: bool| Ok(serde_json::json!({"verdict": "maybe"}));
    let speaking = async |_: u32, _: bool| Ok(serde_json::json!({"verdict": "speak"}));
    let claude = GatedAgent::Claude;
    assert_eq!(
        verdict(claude, &PLAIN, Some(1), refusing, DAEMON_WAIT).await,
        Pass
    );
    assert_eq!(
        verdict(claude, &PLAIN, Some(1), garbled, DAEMON_WAIT).await,
        Pass
    );
    assert_eq!(
        verdict(claude, &PLAIN, None, speaking, DAEMON_WAIT).await,
        Pass,
        "no agent found"
    );
    assert_eq!(
        verdict(claude, &PLAIN, Some(1), speaking, DAEMON_WAIT).await,
        Speak
    );
}

#[tokio::test]
async fn a_repeated_stop_tells_the_daemon_and_passes() {
    let repeated = [
        (
            GatedAgent::Claude,
            serde_json::json!({"stop_hook_active": true}),
        ),
        (
            GatedAgent::Codex,
            serde_json::json!({"stop_hook_active": true}),
        ),
        (
            GatedAgent::Antigravity,
            serde_json::json!({"terminationReason": "NO_TOOL_CALL", "error": "quota exceeded"}),
        ),
    ];
    for (agent, payload) in repeated {
        let asked = std::sync::Mutex::new(Vec::new());
        let speaking = async |agent_pid: u32, repeat: bool| {
            asked.lock().unwrap().push((agent_pid, repeat));
            Ok(serde_json::json!({"verdict": "speak"}))
        };
        assert_eq!(
            verdict(agent, &payload, Some(1), speaking, DAEMON_WAIT).await,
            Pass,
            "{agent:?}"
        );
        assert_eq!(*asked.lock().unwrap(), [(1, true)], "{agent:?}");
    }
}

#[tokio::test]
async fn a_plain_stop_is_not_a_repeat() {
    let asked = std::sync::Mutex::new(Vec::new());
    let speaking = async |agent_pid: u32, repeat: bool| {
        asked.lock().unwrap().push((agent_pid, repeat));
        Ok(serde_json::json!({"verdict": "speak"}))
    };
    let plain = serde_json::json!({"stop_hook_active": false});
    assert_eq!(
        verdict(GatedAgent::Codex, &plain, Some(1), speaking, DAEMON_WAIT).await,
        Speak
    );
    assert_eq!(*asked.lock().unwrap(), [(1, false)]);
}

#[test]
fn each_hook_word_parses_as_its_agent() {
    use clap::{Parser, ValueEnum};
    for &agent in GatedAgent::value_variants() {
        let cli = crate::args::Cli::try_parse_from(["banshee", "turn-end", agent.name()])
            .unwrap_or_else(|error| panic!("{agent:?}: {error}"));
        assert!(
            matches!(cli.command, crate::args::CommandType::TurnEnd { agent: parsed } if parsed == agent),
            "{agent:?}: {:?}",
            cli.command
        );
    }
}

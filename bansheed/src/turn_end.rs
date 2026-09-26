use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::time::Duration;

use banshee_common::error::BansheeError;
use banshee_common::{BANSHEE_TURN_ENDED, TurnVerdict};

use crate::connect::SHIM_NAME;

// About sixty times the slowest stdin close measured, 0.017 s over seven stops.
const STDIN_WAIT: Duration = Duration::from_secs(1);
// About seven times the slowest `banshee status` round trip measured on this machine.
const DAEMON_WAIT: Duration = Duration::from_secs(2);

const REMINDER: &str = "You are about to end this turn without saying anything aloud. \
    The user is working eyes-free and is not reading the screen. Speak your status now with \
    the banshee speak_status tool, or ask_user if you need an answer from them, then finish. \
    Keep written output for what must be read: code, paths, commands, tables.";

const HIDDEN_TOOLS: &str = " If Banshee's tools are not in your tool list, they are in \
    ALL_TOOLS inside exec: call speak_status from there.";

/// An agent whose Stop hook runs `banshee turn-end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum GatedAgent {
    Claude,
    Codex,
    Antigravity,
}

impl GatedAgent {
    /// The word `banshee turn-end` takes for this agent.
    pub fn name(self) -> &'static str {
        match self {
            GatedAgent::Claude => "claude",
            GatedAgent::Codex => "codex",
            GatedAgent::Antigravity => "antigravity",
        }
    }
}

/// One line of `ps -A -ww -o pid=,ppid=,args=`.
#[derive(Debug)]
pub struct ProcessRow {
    pub pid: u32,
    pub ppid: u32,
    pub args: String,
}

/// The rows of a `ps -A -ww -o pid=,ppid=,args=` table. A line that does not start
/// with two numbers is skipped.
pub fn process_rows(table: &str) -> Vec<ProcessRow> {
    table
        .lines()
        .filter_map(|line| {
            let (pid, rest) = line.trim_start().split_once(char::is_whitespace)?;
            let (ppid, args) = rest.trim_start().split_once(char::is_whitespace)?;
            Some(ProcessRow {
                pid: pid.parse().ok()?,
                ppid: ppid.parse().ok()?,
                args: args.trim().to_string(),
            })
        })
        .collect()
}

fn is_shim(args: &str) -> bool {
    args == SHIM_NAME || args.ends_with(&format!("/{SHIM_NAME}"))
}

/// The nearest ancestor of `me` that started a Banshee shim: the agent a hook runs for.
pub fn agent_pid(rows: &[ProcessRow], me: u32) -> Option<u32> {
    let shim_parents: HashSet<u32> = rows
        .iter()
        .filter(|row| is_shim(&row.args))
        .map(|row| row.ppid)
        .collect();
    let parents: HashMap<u32, u32> = rows.iter().map(|row| (row.pid, row.ppid)).collect();
    let mut current = me;
    // A table read while processes exit can hold a loop through a reused PID.
    for _ in 0..rows.len() {
        let parent = *parents.get(&current)?;
        if shim_parents.contains(&parent) {
            return Some(parent);
        }
        current = parent;
    }
    None
}

pub fn not_a_plain_end(agent: GatedAgent, payload: &serde_json::Value) -> bool {
    match agent {
        GatedAgent::Claude | GatedAgent::Codex => payload["stop_hook_active"] == true,
        // `terminationReason` is `NO_TOOL_CALL` on agy's plain end, whatever its docs say.
        GatedAgent::Antigravity => payload["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty()),
    }
}

/// What the agent's hook prints for this verdict. `None` prints nothing.
pub fn answer(agent: GatedAgent, verdict: TurnVerdict) -> Option<String> {
    if verdict == TurnVerdict::Pass {
        return None;
    }
    let (decision, reason) = match agent {
        GatedAgent::Claude => ("block", REMINDER.to_string()),
        GatedAgent::Codex => ("block", format!("{REMINDER}{HIDDEN_TOOLS}")),
        GatedAgent::Antigravity => ("continue", REMINDER.to_string()),
    };
    Some(serde_json::json!({"decision": decision, "reason": reason}).to_string())
}

/// The hook's payload, or `Null` when it is not JSON or does not end within `wait`.
pub fn read_payload(mut input: impl Read + Send + 'static, wait: Duration) -> serde_json::Value {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = input.read_to_string(&mut text);
        let _ = sender.send(text);
    });
    receiver
        .recv_timeout(wait)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// The daemon's verdict for `agent_pid` on this stop. A stop that is not a plain
/// end still goes to the daemon, as a repeat, and is `Pass`. Any failure is `Pass`.
pub async fn verdict(
    agent: GatedAgent,
    payload: &serde_json::Value,
    agent_pid: Option<u32>,
    ask: impl AsyncFn(u32, bool) -> Result<serde_json::Value, BansheeError>,
    wait: Duration,
) -> TurnVerdict {
    let Some(agent_pid) = agent_pid else {
        return TurnVerdict::Pass;
    };
    let repeat = not_a_plain_end(agent, payload);
    match tokio::time::timeout(wait, ask(agent_pid, repeat)).await {
        Ok(Ok(result)) if !repeat => {
            serde_json::from_value(result["verdict"].clone()).unwrap_or(TurnVerdict::Pass)
        }
        _ => TurnVerdict::Pass,
    }
}

fn this_agent() -> Option<u32> {
    let table = std::process::Command::new("/bin/ps")
        .args(["-A", "-ww", "-o", "pid=,ppid=,args="])
        .output()
        .ok()?;
    agent_pid(
        &process_rows(&String::from_utf8_lossy(&table.stdout)),
        std::process::id(),
    )
}

/// Answers `agent`'s Stop hook on stdout, and prints nothing on any failure.
pub async fn run(agent: GatedAgent) {
    let payload = read_payload(std::io::stdin(), STDIN_WAIT);
    let ask = async |agent_pid: u32, repeat: bool| {
        banshee_common::utils::call_daemon(
            BANSHEE_TURN_ENDED,
            serde_json::json!({ "agent_pid": agent_pid, "repeat": repeat }),
        )
        .await
    };
    let verdict = verdict(agent, &payload, this_agent(), ask, DAEMON_WAIT).await;
    if let Some(line) = answer(agent, verdict) {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests;

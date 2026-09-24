use std::path::Path;

use banshee_common::error::BansheeError;

use super::{command_words, hook_script_path, malformed, parse_settings, pretty_json, shell_word};
use crate::turn_end::GatedAgent;

fn script_end(agent: GatedAgent) -> String {
    format!(" turn-end {} || exit 0", agent.name())
}

/// The Stop hook command for `agent`. It runs the `banshee` on PATH when
/// `banshee` is not an executable file. Any failure, no binary at all included,
/// exits 0, which every agent reads as letting the turn end.
pub(super) fn turn_end_command(agent: GatedAgent, banshee: &Path) -> String {
    let script = format!(
        "b=\"$0\"; [ -x \"$b\" ] || b=$(command -v banshee) || exit 0; \"$b\"{}",
        script_end(agent)
    );
    format!(
        "/bin/sh -c {} {}",
        shell_word(&script),
        shell_word(&banshee.display().to_string())
    )
}

/// Whether `command` is a Banshee turn-end hook for `agent`, at any binary
/// path, whatever the script runs before `turn-end`.
pub(super) fn is_turn_end_command(command: &str, agent: GatedAgent) -> bool {
    matches!(
        command_words(command).as_slice(),
        [shell, flag, body, _banshee]
            if shell == "/bin/sh" && flag == "-c" && body.ends_with(&script_end(agent))
    )
}

fn is_banshee_hook(command: &str, agent: GatedAgent) -> bool {
    is_turn_end_command(command, agent) || hook_script_path(command).is_some()
}

/// The commands under `hooks.Stop[].hooks[]`, the shape Claude Code and Codex share.
pub(super) fn stop_hook_commands(root: &serde_json::Value) -> impl Iterator<Item = &str> {
    root["hooks"]["Stop"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|group| group["hooks"].as_array().into_iter().flatten())
        .filter_map(|hook| hook["command"].as_str())
}

/// Whether a settings file in the `hooks.Stop[].hooks[]` shape holds Banshee's
/// hook, as the script entry or as the turn-end command. A file that does not
/// parse holds none.
pub(super) fn holds_banshee_hook(settings: Option<&str>, agent: GatedAgent) -> bool {
    let Some(root) = settings.and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
    else {
        return false;
    };
    stop_hook_commands(&root).any(|command| is_banshee_hook(command, agent))
}

/// `before` with Banshee's `hooks.Stop[].hooks[]` entry set to `command`, the
/// shape Claude Code and Codex share. `None` when it already is.
pub(super) fn with_turn_end(
    before: Option<&str>,
    file: &str,
    command: &str,
    agent: GatedAgent,
) -> Result<Option<String>, BansheeError> {
    let mut root: serde_json::Value = parse_settings(before, file)?;
    let stop = root
        .as_object_mut()
        .ok_or_else(|| malformed(file, "is not a JSON object"))?
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| malformed(file, "hooks is not an object"))?
        .entry("Stop")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| malformed(file, "hooks.Stop is not a list"))?;
    let existing = stop
        .iter_mut()
        .flat_map(|group| {
            group
                .get_mut("hooks")
                .and_then(serde_json::Value::as_array_mut)
                .into_iter()
                .flatten()
        })
        .find(|hook| {
            hook["command"]
                .as_str()
                .is_some_and(|c| is_banshee_hook(c, agent))
        });
    match existing {
        Some(hook) if hook["command"] == command => return Ok(None),
        Some(hook) => hook["command"] = serde_json::json!(command),
        None => stop.push(serde_json::json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": 15,
                "statusMessage": "Checking you spoke",
            }]
        })),
    }
    Ok(Some(pretty_json(&root)?))
}

/// `before` with Antigravity's `banshee` hook holding `command` under `Stop`.
/// `None` when it already does.
pub(super) fn with_antigravity_turn_end(
    before: Option<&str>,
    command: &str,
) -> Result<Option<String>, BansheeError> {
    const FILE: &str = "hooks.json";
    let mut root: serde_json::Value = parse_settings(before, FILE)?;
    let stop = root
        .as_object_mut()
        .ok_or_else(|| malformed(FILE, "is not a JSON object"))?
        .entry("banshee")
        .or_insert_with(|| serde_json::json!({"enabled": true}))
        .as_object_mut()
        .ok_or_else(|| malformed(FILE, "banshee is not an object"))?
        .entry("Stop")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| malformed(FILE, "banshee.Stop is not a list"))?;
    let existing = stop.iter_mut().find(|handler| {
        handler["command"]
            .as_str()
            .is_some_and(|c| is_turn_end_command(c, GatedAgent::Antigravity))
    });
    match existing {
        Some(handler) if handler["command"] == command => return Ok(None),
        Some(handler) => handler["command"] = serde_json::json!(command),
        None => stop.push(serde_json::json!({
            "type": "command",
            "command": command,
            "timeout": 15,
        })),
    }
    Ok(Some(pretty_json(&root)?))
}

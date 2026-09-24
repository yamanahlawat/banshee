use banshee_common::error::BansheeError;
use banshee_common::{AgentRow, PlannedChange};
use std::ffi::{OsStr, OsString};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

mod hooks;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Antigravity,
    ClaudeCode,
    Codex,
    Cursor,
    OpenCode,
    Pi,
}

impl Agent {
    pub const ALL: [Agent; 6] = [
        Agent::Antigravity,
        Agent::ClaudeCode,
        Agent::Codex,
        Agent::Cursor,
        Agent::OpenCode,
        Agent::Pi,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Agent::Antigravity => "antigravity",
            Agent::ClaudeCode => "claude",
            Agent::Codex => "codex",
            Agent::Cursor => "cursor",
            Agent::OpenCode => "opencode",
            Agent::Pi => "pi",
        }
    }

    /// What a person calls the tool. `name()` stays the slug the CLI takes.
    pub fn display_name(self) -> &'static str {
        match self {
            Agent::Antigravity => "Antigravity",
            Agent::ClaudeCode => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Cursor => "Cursor",
            Agent::OpenCode => "OpenCode",
            Agent::Pi => "Pi",
        }
    }

    /// What the user must still do after connecting, for an agent that needs a step.
    pub fn connect_note(self) -> Option<&'static str> {
        match self {
            Agent::Codex => {
                Some("Codex runs this hook only after you trust it: open Codex and run /hooks.")
            }
            _ => None,
        }
    }

    fn signal(self) -> Signal {
        match self {
            Agent::Antigravity => Signal::OnPath("agy"),
            Agent::ClaudeCode => Signal::OnPath("claude"),
            Agent::Codex => Signal::OnPath("codex"),
            Agent::Cursor => Signal::HomeDir(".cursor"),
            Agent::OpenCode => Signal::HomeDir(".config/opencode"),
            Agent::Pi => Signal::HomeDir(".pi/agent"),
        }
    }
}

/// What tells `detect` that an agent is installed.
enum Signal {
    OnPath(&'static str),
    HomeDir(&'static str),
}

impl From<crate::args::AgentName> for Agent {
    fn from(name: crate::args::AgentName) -> Agent {
        match name {
            crate::args::AgentName::Antigravity => Agent::Antigravity,
            crate::args::AgentName::Claude => Agent::ClaudeCode,
            crate::args::AgentName::Codex => Agent::Codex,
            crate::args::AgentName::Cursor => Agent::Cursor,
            crate::args::AgentName::Opencode => Agent::OpenCode,
            crate::args::AgentName::Pi => Agent::Pi,
        }
    }
}

/// Everything `detect` and `plan` read from the machine, so tests can point
/// them at a scratch directory.
pub struct Env {
    pub home: PathBuf,
    pub claude_config_dir: PathBuf,
    pub banshee: PathBuf,
    /// None when the shim does not ship beside `banshee`. Only Pi works then.
    pub shim: Option<PathBuf>,
    /// The PATH detection searched, which is also the PATH a command runs with.
    /// A resolved program still needs it: an agent CLI is often a script whose
    /// interpreter the daemon's own PATH does not hold.
    pub path: OsString,
    /// Where each agent binary was found. A plan that runs one carries this
    /// path: the daemon cannot resolve a name against its own PATH.
    pub on_path: Vec<(&'static str, PathBuf)>,
    /// The command Claude Code has registered for the banshee MCP server.
    pub claude_shim: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Presence {
    NotInstalled { looked_for: String },
    Installed,
}

/// One edit to another tool's config, shown before it is applied.
#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Run {
        argv: Vec<String>,
    },
    WriteFile {
        path: PathBuf,
        before: Option<String>,
        after: String,
    },
    RemoveFile {
        path: PathBuf,
        before: String,
    },
}

pub fn detect(agent: Agent, env: &Env) -> Presence {
    let (present, looked_for) = match agent.signal() {
        Signal::OnPath(binary) => (env.program(agent).is_some(), format!("{binary} on PATH")),
        Signal::HomeDir(dir) => (env.home.join(dir).is_dir(), format!("~/{dir}/")),
    };
    if present {
        Presence::Installed
    } else {
        Presence::NotInstalled { looked_for }
    }
}

pub fn row(agent: Agent, env: &Env) -> AgentRow {
    let (presence, note) = match detect(agent, env) {
        Presence::NotInstalled { looked_for } => {
            ("absent", format!("Not installed. Looked for {looked_for}"))
        }
        Presence::Installed => match plan(agent, env) {
            Err(error) => ("found", format!("Installed, but the plan failed: {error}")),
            Ok(changes) if changes.is_empty() => ("connected", "Connected".to_string()),
            Ok(_) => ("found", "Installed, not connected".to_string()),
        },
    };
    AgentRow {
        id: agent.name().to_string(),
        name: agent.display_name().to_string(),
        presence: presence.to_string(),
        note,
    }
}

pub fn planned_change(change: &Change) -> PlannedChange {
    PlannedChange {
        path: match change {
            Change::WriteFile { path, .. } | Change::RemoveFile { path, .. } => {
                Some(path.display().to_string())
            }
            Change::Run { .. } => None,
        },
        diff: render(change),
    }
}

const PI_EXTENSION: &str = include_str!("../../integrations/pi/banshee.ts");

fn require_shim(env: &Env) -> Result<&Path, BansheeError> {
    env.shim.as_deref().ok_or_else(|| {
        BansheeError::Rejected(format!(
            "banshee-mcp-shim is not beside {}; reinstall so they ship together",
            env.banshee.display()
        ))
    })
}

pub(crate) const SHIM_NAME: &str = "banshee-mcp-shim";

fn reaches_shim(registered: Option<&str>, shim: &Path) -> bool {
    let Some(command) = registered else {
        return false;
    };
    if command == SHIM_NAME {
        return false;
    }
    let command = Path::new(command);
    command == shim || same_file(command, shim)
}

// A registered symlink and the canonical shim are one file
fn same_file(a: &Path, b: &Path) -> bool {
    matches!(
        (std::fs::canonicalize(a), std::fs::canonicalize(b)),
        (Ok(a), Ok(b)) if a == b
    )
}

fn plan_claude(env: &Env) -> Result<Vec<Change>, BansheeError> {
    let shim_path = require_shim(env)?;
    let shim = shim_path.display().to_string();
    let mut changes = Vec::new();
    if !reaches_shim(env.claude_shim.as_deref(), shim_path) {
        let claude = env
            .program(Agent::ClaudeCode)
            .ok_or_else(|| {
                BansheeError::Rejected("claude is not on PATH; nothing to connect".into())
            })?
            .display()
            .to_string();
        // `claude mcp add` refuses a name it already holds, so a stale command
        // has to go first
        if env.claude_shim.is_some() {
            changes.push(Change::Run {
                argv: vec![
                    claude.clone(),
                    "mcp".into(),
                    "remove".into(),
                    "--scope".into(),
                    "user".into(),
                    "banshee".into(),
                ],
            });
        }
        changes.push(Change::Run {
            argv: vec![
                claude,
                "mcp".into(),
                "add".into(),
                "--scope".into(),
                "user".into(),
                "banshee".into(),
                "--".into(),
                shim,
            ],
        });
    }

    let command = hooks::turn_end_command(crate::turn_end::GatedAgent::Claude, &env.banshee);
    let settings_path = env.claude_config_dir.join("settings.json");
    let local_path = env.claude_config_dir.join("settings.local.json");
    let settings_text = read_if_present(&settings_path)?;
    let local_text = read_if_present(&local_path)?;
    // Claude Code merges both files, so the file that already holds Banshee's hook keeps it
    let (path, before) = if !hooks::holds_banshee_hook(
        settings_text.as_deref(),
        crate::turn_end::GatedAgent::Claude,
    ) && hooks::holds_banshee_hook(
        local_text.as_deref(),
        crate::turn_end::GatedAgent::Claude,
    ) {
        (local_path, local_text)
    } else {
        (settings_path, settings_text)
    };
    changes.extend(claude_hook(path, before, &command)?);
    Ok(changes)
}

/// Sets Banshee's Stop hook to `command` at `path`, whose text the plan
/// already read as `before`, and removes the `banshee-speak-check.sh` script
/// that the file's Stop hook names.
fn claude_hook(
    path: PathBuf,
    before: Option<String>,
    command: &str,
) -> Result<Vec<Change>, BansheeError> {
    let file = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("settings.json")
        .to_string();
    let retired = old_script(before.as_deref(), &file)?;
    let mut changes = rewrite_from(path, before, |before| {
        hooks::with_turn_end(before, &file, command, crate::turn_end::GatedAgent::Claude)
    })?;
    changes.extend(retired);
    Ok(changes)
}

/// The removal of the script that the Stop hook in `file` runs, whatever the script holds.
fn old_script(settings: Option<&str>, file: &str) -> Result<Option<Change>, BansheeError> {
    let root = parse_settings(settings, file)?;
    let Some(script) = hooks::stop_hook_commands(&root).find_map(hook_script_path) else {
        return Ok(None);
    };
    Ok(read_if_present(&script)?.map(|before| Change::RemoveFile {
        path: script,
        before,
    }))
}

/// The command's words, with a quoted one kept whole and its quotes dropped.
/// Outside quotes, a backslash keeps the next character as it is.
fn command_words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut characters = command.chars();
    while let Some(character) = characters.next() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => word.push(character),
            (None, '\\') => word.extend(characters.next()),
            (None, '\'' | '"') => quote = Some(character),
            (None, _) if character.is_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            (None, _) => word.push(character),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

fn hook_script_path(command: &str) -> Option<PathBuf> {
    command_words(command)
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.file_name() == Some(OsStr::new(HOOK_SCRIPT_NAME)))
}

pub fn plan(agent: Agent, env: &Env) -> Result<Vec<Change>, BansheeError> {
    match agent {
        Agent::ClaudeCode => plan_claude(env),
        Agent::Codex => {
            let shim = require_shim(env)?;
            let gate = crate::turn_end::GatedAgent::Codex;
            let command = hooks::turn_end_command(gate, &env.banshee);
            let mut changes = rewrite(env.home.join(".codex/config.toml"), |before| {
                with_codex_server(before, shim)
            })?;
            changes.extend(rewrite(env.home.join(".codex/hooks.json"), |before| {
                hooks::with_turn_end(before, "hooks.json", &command, gate)
            })?);
            Ok(changes)
        }
        Agent::Cursor => {
            let shim = require_shim(env)?;
            rewrite(env.home.join(".cursor/mcp.json"), |before| {
                with_mcp_server(before, "mcp.json", shim)
            })
        }
        // The IDE, the `agy` CLI and the SDK share this one file
        Agent::Antigravity => {
            let shim = require_shim(env)?;
            let gate = crate::turn_end::GatedAgent::Antigravity;
            let command = hooks::turn_end_command(gate, &env.banshee);
            let mut changes = rewrite(env.home.join(".gemini/config/mcp_config.json"), |before| {
                with_mcp_server(before, "mcp_config.json", shim)
            })?;
            changes.extend(rewrite(
                env.home.join(".gemini/config/hooks.json"),
                |before| hooks::with_antigravity_turn_end(before, &command),
            )?);
            Ok(changes)
        }
        Agent::OpenCode => {
            let shim = require_shim(env)?;
            rewrite(env.home.join(".config/opencode/opencode.jsonc"), |before| {
                with_opencode_server(before, shim)
            })
        }
        Agent::Pi => Ok(write_if_changed(
            env.home.join(".pi/agent/extensions/banshee.ts"),
            PI_EXTENSION,
        )?
        .into_iter()
        .collect()),
    }
}

fn read_if_present(path: &Path) -> Result<Option<String>, BansheeError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

// json5 reads the comments and trailing commas that serde_json refuses; the rewrite is plain JSON
fn with_opencode_server(config: Option<&str>, shim: &Path) -> Result<Option<String>, BansheeError> {
    let mut root: serde_json::Value = match config {
        Some(text) => json5::from_str(text)
            .map_err(|error| malformed("opencode.jsonc", &format!("could not be read: {error}")))?,
        None => serde_json::json!({}),
    };
    let entry = root
        .as_object_mut()
        .ok_or_else(|| malformed("opencode.jsonc", "is not a JSON object"))?
        .entry("mcp")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| malformed("opencode.jsonc", "mcp is not an object"))?
        .entry("banshee")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| malformed("opencode.jsonc", "mcp.banshee is not an object"))?;
    let command = entry
        .get("command")
        .and_then(serde_json::Value::as_array)
        .filter(|argv| argv.len() == 1)
        .and_then(|argv| argv[0].as_str());
    // OpenCode treats a missing `enabled` as true
    let enabled = entry.get("enabled") != Some(&serde_json::Value::Bool(false));
    if enabled
        && entry.get("type") == Some(&serde_json::json!("local"))
        && reaches_shim(command, shim)
    {
        return Ok(None);
    }
    entry.insert("type".into(), serde_json::json!("local"));
    entry.insert("enabled".into(), serde_json::json!(true));
    entry.insert(
        "command".into(),
        serde_json::json!([shim.display().to_string()]),
    );
    Ok(Some(pretty_json(&root)?))
}

fn malformed(file: &str, what: &str) -> BansheeError {
    BansheeError::Rejected(format!("{file} {what}"))
}

fn pretty_json(root: &serde_json::Value) -> Result<String, BansheeError> {
    let mut text = serde_json::to_string_pretty(root)?;
    text.push('\n');
    Ok(text)
}

fn rewrite(
    path: PathBuf,
    edit: impl FnOnce(Option<&str>) -> Result<Option<String>, BansheeError>,
) -> Result<Vec<Change>, BansheeError> {
    let before = read_if_present(&path)?;
    rewrite_from(path, before, edit)
}

/// `rewrite`, for a file already read as `before`.
fn rewrite_from(
    path: PathBuf,
    before: Option<String>,
    edit: impl FnOnce(Option<&str>) -> Result<Option<String>, BansheeError>,
) -> Result<Vec<Change>, BansheeError> {
    Ok(edit(before.as_deref())?
        .map(|after| Change::WriteFile {
            path,
            before,
            after,
        })
        .into_iter()
        .collect())
}

fn with_mcp_server(
    config: Option<&str>,
    file: &str,
    shim: &Path,
) -> Result<Option<String>, BansheeError> {
    let mut root: serde_json::Value = match config {
        Some(text) => json5::from_str(text)
            .map_err(|error| malformed(file, &format!("could not be read: {error}")))?,
        None => serde_json::json!({}),
    };
    let entry = root
        .as_object_mut()
        .ok_or_else(|| malformed(file, "is not a JSON object"))?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| malformed(file, "mcpServers is not an object"))?
        .entry("banshee")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| malformed(file, "mcpServers.banshee is not an object"))?;
    let command = entry.get("command").and_then(serde_json::Value::as_str);
    if reaches_shim(command, shim) {
        return Ok(None);
    }
    entry.insert(
        "command".into(),
        serde_json::json!(shim.display().to_string()),
    );
    Ok(Some(pretty_json(&root)?))
}

// toml_edit keeps the user's comments
fn with_codex_server(config: Option<&str>, shim: &Path) -> Result<Option<String>, BansheeError> {
    let mut document: toml_edit::DocumentMut = config
        .unwrap_or_default()
        .parse()
        .map_err(|error| malformed("config.toml", &format!("could not be read: {error}")))?;
    let servers = document
        .as_table_mut()
        .entry("mcp_servers")
        .or_insert_with(|| {
            let mut table = toml_edit::Table::new();
            table.set_implicit(true);
            toml_edit::Item::Table(table)
        })
        .as_table_mut()
        .ok_or_else(|| malformed("config.toml", "mcp_servers is not a table"))?;
    let banshee = servers
        .entry("banshee")
        .or_insert(toml_edit::table())
        .as_table_mut()
        .ok_or_else(|| malformed("config.toml", "mcp_servers.banshee is not a table"))?;
    let command = banshee.get("command").and_then(toml_edit::Item::as_str);
    if reaches_shim(command, shim) {
        return Ok(None);
    }
    banshee["command"] = toml_edit::value(shim.display().to_string());
    Ok(Some(document.to_string()))
}

fn write_if_changed(path: PathBuf, after: &str) -> Result<Option<Change>, BansheeError> {
    let before = read_if_present(&path)?;
    if before.as_deref() == Some(after) {
        return Ok(None);
    }
    Ok(Some(Change::WriteFile {
        path,
        before,
        after: after.to_string(),
    }))
}

pub(crate) const HOOK_SCRIPT_NAME: &str = "banshee-speak-check.sh";

fn parse_settings(settings: Option<&str>, file: &str) -> Result<serde_json::Value, BansheeError> {
    match settings {
        Some(text) => serde_json::from_str(text)
            .map_err(|error| malformed(file, &format!("is not valid JSON: {error}"))),
        None => Ok(serde_json::json!({})),
    }
}

pub fn render(change: &Change) -> String {
    match change {
        Change::Run { argv } => {
            let words: Vec<String> = argv.iter().map(|word| shell_word(word)).collect();
            format!("$ {}\n", words.join(" "))
        }
        Change::WriteFile {
            path,
            before,
            after,
        } => {
            let name = path.display().to_string();
            let old_name = if before.is_some() { &name } else { "/dev/null" };
            similar::TextDiff::from_lines(before.as_deref().unwrap_or(""), after.as_str())
                .unified_diff()
                .header(old_name, &name)
                .to_string()
        }
        Change::RemoveFile { path, .. } => format!("remove {}\n", path.display()),
    }
}

// POSIX single quotes, which a shell and `command_words` both read back as the one word
fn shell_word(word: &str) -> String {
    let plain = word
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./=:".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

pub fn apply(change: &Change, path: &OsStr) -> Result<(), BansheeError> {
    match change {
        Change::Run { argv } => {
            let Some(program) = argv.first() else {
                return Err(BansheeError::Other("a command with no program".into()));
            };
            let status = std::process::Command::new(program)
                .args(&argv[1..])
                .env("PATH", path)
                .status()?;
            if status.success() {
                Ok(())
            } else {
                Err(BansheeError::Other(format!(
                    "{program} exited with {status}"
                )))
            }
        }
        Change::WriteFile {
            path,
            before,
            after,
        } => {
            if read_if_present(path)? != *before {
                return Err(BansheeError::Rejected(format!(
                    "{} changed after the plan was made. Nothing was written to it; run the command again.",
                    path.display()
                )));
            }
            banshee_common::utils::write_atomically(path, after.as_bytes(), None)?;
            Ok(())
        }
        Change::RemoveFile { path, before } => {
            if read_if_present(path)?.as_deref() != Some(before.as_str()) {
                return Err(BansheeError::Rejected(format!(
                    "{} changed after the plan was made. Nothing was removed; run the command again.",
                    path.display()
                )));
            }
            std::fs::remove_file(path)?;
            Ok(())
        }
    }
}

pub fn confirm(prompt: &str) -> Result<bool, BansheeError> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

// Claude Code owns this file: connect only reads it, and unreadable means unregistered
fn registered_claude_shim(global_config: &Path) -> Option<String> {
    let text = std::fs::read_to_string(global_config).ok()?;
    let root: serde_json::Value = serde_json::from_str(&text).ok()?;
    root.get("mcpServers")?
        .get("banshee")?
        .get("command")?
        .as_str()
        .map(String::from)
}

/// An empty entry resolves against the working directory, which is never what
/// a PATH lookup means here.
pub(crate) fn path_dirs(path: &OsStr) -> impl Iterator<Item = PathBuf> + '_ {
    std::env::split_paths(path).filter(|dir| !dir.as_os_str().is_empty())
}

fn path_holds(path: &OsStr, dir: &Path) -> bool {
    path_dirs(path).any(|held| held == dir)
}

// The native installer for Claude Code and Codex writes the binary here. It
// puts the directory in the interactive rc file. One name serves macOS and
// Linux, so this needs no platform split.
fn with_local_bin(path: OsString, home: &Path) -> OsString {
    let local_bin = home.join(".local/bin");
    if path_holds(&path, &local_bin) {
        return path;
    }
    std::env::join_paths(path_dirs(&path).chain(std::iter::once(local_bin))).unwrap_or(path)
}

const PATH_START: &str = "__BANSHEE_PATH_START__";
const PATH_END: &str = "__BANSHEE_PATH_END__";

/// The text before the first `start`, between it and the next `end`, and after.
pub(crate) fn split_between<'a>(
    text: &'a str,
    start: &str,
    end: &str,
) -> Option<(&'a str, &'a str, &'a str)> {
    let (before, rest) = text.split_once(start)?;
    let (inside, after) = rest.split_once(end)?;
    Some((before, inside, after))
}

fn extract_path(output: &str) -> Option<OsString> {
    split_between(output, PATH_START, PATH_END).map(|(_, path, _)| OsString::from(path))
}

// A profile that blocks would otherwise hold the first caller forever, and
// every later caller behind the OnceLock. An interactive profile measured
// 1.25 s on one machine, so 5 s leaves room for a slower one. The poll trades
// wake-ups against how late the probe reads the exit status.
const SHELL_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
const SHELL_POLL: std::time::Duration = std::time::Duration::from_millis(50);

enum Probe {
    Path(OsString),
    /// One sentence for the log. Nothing else reads it.
    Failed(String),
}

// A marker can straddle two reads, so a search covers everything read so far.
const READ_CHUNK: usize = 256;

fn holds_marker(bytes: &[u8], marker: &str) -> bool {
    bytes
        .windows(marker.len())
        .any(|window| window == marker.as_bytes())
}

// A shell that fills the 64 KiB pipe buffer blocks, so the read runs on its
// own thread. An rc file's daemon holds the pipe open for its own life, so
// `marker` ends the read too.
fn drain(
    pipe: Option<impl std::io::Read + Send + 'static>,
    marker: Option<&'static str>,
) -> std::sync::mpsc::Receiver<String> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let mut buffer = [0u8; READ_CHUNK];
            while let Ok(read) = pipe.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..read]);
                if marker.is_some_and(|marker| holds_marker(&bytes, marker)) {
                    break;
                }
            }
        }
        let _ = send.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    receive
}

fn remaining(deadline: std::time::Instant) -> std::time::Duration {
    deadline.saturating_duration_since(std::time::Instant::now())
}

/// The exit status, or `None` when the shell still runs at the deadline.
fn exit_status(child: &mut std::process::Child, deadline: std::time::Instant) -> Option<String> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.to_string()),
            Err(error) => return Some(error.to_string()),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(SHELL_POLL);
            }
        }
    }
}

fn reap(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn probe(shell: &OsStr, flags: &[&str], wait: std::time::Duration) -> Probe {
    let deadline = std::time::Instant::now() + wait;
    let command = format!(r#"printf '{PATH_START}%s{PATH_END}' "$PATH""#);
    let started = std::process::Command::new(shell)
        .args(flags)
        .arg("-c")
        .arg(&command)
        // An rc file that reads stdin holds `banshee connect` at the terminal.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let flags = flags.join(" ");
    let mut child = match started {
        Ok(child) => child,
        Err(error) => {
            return Probe::Failed(format!("{shell:?} {flags} did not start: {error}"));
        }
    };

    let printed = drain(child.stdout.take(), Some(PATH_END));
    let complained = drain(child.stderr.take(), None);
    // The status belongs to the last rc command, so a failed rc file still
    // leaves the PATH good.
    let answer = printed
        .recv_timeout(remaining(deadline))
        .unwrap_or_default();
    let found = extract_path(&answer);
    let ended = match found {
        Some(_) => None,
        None => exit_status(&mut child, deadline),
    };
    reap(&mut child);
    if let Some(path) = found {
        return Probe::Path(path);
    }

    let complained = complained
        .recv_timeout(remaining(deadline))
        .unwrap_or_default();
    Probe::Failed(format!(
        "{shell:?} {flags} reported no PATH: {}. {}",
        ended.unwrap_or_else(|| format!("no answer in {wait:?}")),
        complained.trim()
    ))
}

// An interactive shell reads the rc file, where an npm or nvm install puts
// its directory. A login shell reads none.
fn shell_path(shell: &OsStr, wait: std::time::Duration) -> Option<OsString> {
    for flags in [["-i", "-l"].as_slice(), ["-l"].as_slice()] {
        match probe(shell, flags, wait) {
            Probe::Path(path) => return Some(path),
            Probe::Failed(why) => log::warn!("{why}"),
        }
    }
    None
}

fn login_shell_path() -> Option<OsString> {
    shell_path(&std::env::var_os("SHELL")?, SHELL_WAIT)
}

struct SearchPath(std::sync::Mutex<Option<OsString>>);

impl SearchPath {
    const fn new() -> SearchPath {
        SearchPath(std::sync::Mutex::new(None))
    }

    /// The PATH in hand, or `probe`'s answer when nothing is in hand yet.
    fn get(&self, probe: impl FnOnce() -> OsString) -> OsString {
        if let Some(path) = self.held().clone() {
            return path;
        }
        // No probe runs under the lock. A reader that waited behind another
        // reader's shell would be the stall this cache exists to prevent. A
        // race probes twice instead, and the first answer stored wins.
        let probed = probe();
        self.held().get_or_insert(probed).clone()
    }

    /// Puts a PATH in hand, and answers what readers now get.
    fn replace(&self, path: OsString) -> OsString {
        *self.held() = Some(path.clone());
        path
    }

    // A panic under the lock leaves either no PATH or a good one. So the
    // daemon keeps resolving instead of dying with the poison.
    fn held(&self) -> std::sync::MutexGuard<'_, Option<OsString>> {
        self.0.lock().unwrap_or_else(|held| held.into_inner())
    }
}

static SEARCH_PATH: SearchPath = SearchPath::new();

/// A shell that printed nothing between the markers reported no PATH.
fn answered(shell_path: Option<OsString>) -> Option<OsString> {
    shell_path.filter(|path| !path.is_empty())
}

fn searchable(path: OsString) -> OsString {
    // `tell show`, dictation, the status report and the readiness check all
    // search this PATH. None of them builds an `Env`, so the call belongs here.
    let path = match crate::service::home_dir() {
        Ok(home) => with_local_bin(path, &home),
        Err(_) => path,
    };
    log::debug!("agents are searched for on PATH {path:?}");
    path
}

/// What a shell reported. `None` when none answered.
fn probed_path() -> Option<OsString> {
    answered(login_shell_path()).map(searchable)
}

/// What to search when no shell answers.
fn fallback_path() -> OsString {
    searchable(std::env::var_os("PATH").unwrap_or_default())
}

/// A probe that answers nothing leaves the PATH in hand.
fn refreshed(
    cache: &SearchPath,
    probe: impl FnOnce() -> Option<OsString>,
    fallback: impl FnOnce() -> OsString,
) -> OsString {
    match probe() {
        Some(path) => cache.replace(path),
        // The fallback holds almost nothing under a service manager. Writing
        // it over a working PATH would take detection, dictation and `tell`
        // down until a later probe happened to answer.
        None => cache.get(fallback),
    }
}

pub(crate) fn resolved_path() -> OsString {
    SEARCH_PATH.get(|| probed_path().unwrap_or_else(fallback_path))
}

/// Asks the shell again, so an agent installed since the daemon started
/// appears.
pub(crate) fn refreshed_path() -> OsString {
    refreshed(&SEARCH_PATH, probed_path, fallback_path)
}

impl Env {
    pub fn from_machine() -> Result<Env, BansheeError> {
        Env::with_path(resolved_path())
    }

    pub fn from_machine_refreshed() -> Result<Env, BansheeError> {
        Env::with_path(refreshed_path())
    }

    #[cfg(test)]
    fn with_shell_path(shell_path: Option<OsString>) -> Result<Env, BansheeError> {
        let path = answered(shell_path).map(searchable);
        Env::with_path(path.unwrap_or_else(fallback_path))
    }

    /// Where detection found this agent's binary. `None` for an agent found by
    /// a directory, and for one that is not installed.
    pub fn program(&self, agent: Agent) -> Option<&Path> {
        let Signal::OnPath(binary) = agent.signal() else {
            return None;
        };
        self.on_path
            .iter()
            .find(|(name, _)| *name == binary)
            .map(|(_, program)| program.as_path())
    }

    fn with_path(path: OsString) -> Result<Env, BansheeError> {
        let home = crate::service::home_dir()?;
        let config_dir_override = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
        // Claude Code keeps user-scope servers in ~/.claude.json unless the variable moves them
        let claude_global = match &config_dir_override {
            Some(dir) => dir.join(".claude.json"),
            None => home.join(".claude.json"),
        };
        let claude_config_dir = config_dir_override.unwrap_or_else(|| home.join(".claude"));
        let exe = std::env::current_exe()?;
        let banshee = std::fs::canonicalize(&exe)?;
        let shim = banshee_common::utils::sibling(&exe, SHIM_NAME).ok();
        // Starting Claude Code makes it rewrite its own config, so detection never spawns an agent
        let on_path = Agent::ALL
            .iter()
            .filter_map(|agent| match agent.signal() {
                Signal::OnPath(binary) => Some(binary),
                Signal::HomeDir(_) => None,
            })
            .filter_map(|binary| {
                crate::status::resolve(binary, &path).map(|program| (binary, program))
            })
            .collect();
        Ok(Env {
            home,
            claude_config_dir,
            banshee,
            shim,
            path,
            on_path,
            claude_shim: registered_claude_shim(&claude_global),
        })
    }
}

/// Applies in order and names what is left when one fails.
pub fn apply_all(
    changes: &[Change],
    path: &OsStr,
    mut written: impl FnMut(&Change),
) -> Result<(), BansheeError> {
    for (done, change) in changes.iter().enumerate() {
        if let Err(error) = apply(change, path) {
            let left: String = changes[done..].iter().map(render).collect();
            return Err(BansheeError::Rejected(format!(
                "{error}\n{done} of {} changes applied. Still to apply:\n{left}",
                changes.len()
            )));
        }
        written(change);
    }
    Ok(())
}

pub fn run(agent: Option<Agent>, yes: bool) -> Result<(), BansheeError> {
    let env = Env::from_machine()?;
    let Some(agent) = agent else {
        return list(&env);
    };
    if let Presence::NotInstalled { looked_for } = detect(agent, &env) {
        return Err(BansheeError::Rejected(format!(
            "{} is not installed here (looked for {looked_for})",
            agent.name()
        )));
    }
    let changes = plan(agent, &env)?;
    if changes.is_empty() {
        println!("{} is already connected.", agent.name());
        return Ok(());
    }
    let mut done = format!(
        "{} is connected. Restart it to pick up the change.",
        agent.name()
    );
    if let Some(note) = agent.connect_note() {
        done.push('\n');
        done.push_str(note);
    }
    apply_plan(&changes, &env.path, yes, &done)
}

/// Shows every change, asks unless `yes`, applies them in order, then prints
/// `done`. A declined confirmation writes nothing and returns `Rejected`.
pub fn apply_plan(
    changes: &[Change],
    path: &OsStr,
    yes: bool,
    done: &str,
) -> Result<(), BansheeError> {
    for change in changes {
        print!("{}", render(change));
        println!();
    }
    if !yes && !confirm("Apply? [y/N] ")? {
        return Err(BansheeError::Rejected("Nothing written.".into()));
    }
    apply_all(changes, path, |change| match change {
        Change::WriteFile { path, .. } => println!("wrote {}", path.display()),
        Change::RemoveFile { path, .. } => println!("removed {}", path.display()),
        Change::Run { .. } => {}
    })?;
    println!("{done}");
    Ok(())
}

fn list(env: &Env) -> Result<(), BansheeError> {
    let width = Agent::ALL
        .iter()
        .map(|agent| agent.name().len())
        .max()
        .unwrap_or(0);
    for agent in Agent::ALL {
        let row = row(agent, env);
        println!("{:<width$} {:<10} {}", row.id, row.presence, row.note);
    }
    println!();
    println!("Connect one with: banshee connect <agent>");
    Ok(())
}

#[cfg(test)]
mod tests;

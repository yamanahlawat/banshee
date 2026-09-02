use banshee_common::error::BansheeError;
use banshee_common::{AgentRow, PlannedChange};
use std::ffi::{OsStr, OsString};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

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
        executable: bool,
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
            Change::WriteFile { path, .. } => Some(path.display().to_string()),
            Change::Run { .. } => None,
        },
        diff: render(change),
    }
}

const PI_EXTENSION: &str = include_str!("../../integrations/pi/banshee.ts");

const HOOK_SCRIPT: &str = include_str!("../../integrations/claude-code/banshee-speak-check.sh");

// A hook does not get the login shell's PATH, so the script carries the path
fn hook_script(banshee: &Path) -> String {
    HOOK_SCRIPT.replace("@BANSHEE_BIN@", &banshee.display().to_string())
}

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

    let script_path = env.claude_config_dir.join("hooks").join(HOOK_SCRIPT_NAME);
    let script = hook_script(&env.banshee);
    match registered_stop_hook(&env.claude_config_dir)?
        .as_deref()
        .and_then(hook_script_path)
    {
        None => {
            changes.extend(write_if_changed(script_path.clone(), &script, true)?);
            let command = format!("bash '{}'", script_path.display());
            changes.extend(rewrite(
                env.claude_config_dir.join("settings.json"),
                |settings| with_stop_hook(settings, &command),
            )?);
        }
        Some(registered) if registered == script_path || !registered.exists() => {
            changes.extend(write_if_changed(registered, &script, true)?);
        }
        // A working hook of the user's own, at a path of their own
        Some(_) => {}
    }
    Ok(changes)
}

// Claude Code merges hooks from both files, so a hook in either one counts
fn registered_stop_hook(claude_config_dir: &Path) -> Result<Option<String>, BansheeError> {
    for file in ["settings.json", "settings.local.json"] {
        let settings = read_if_present(&claude_config_dir.join(file))?;
        if let Some(command) = stop_hook_command(&parse_settings(settings.as_deref())?) {
            return Ok(Some(command));
        }
    }
    Ok(None)
}

fn hook_script_path(command: &str) -> Option<PathBuf> {
    command
        .split_whitespace()
        .find(|word| word.contains(HOOK_SCRIPT_NAME))
        .map(|word| PathBuf::from(word.trim_matches(|c| c == '\'' || c == '"')))
}

pub fn plan(agent: Agent, env: &Env) -> Result<Vec<Change>, BansheeError> {
    match agent {
        Agent::ClaudeCode => plan_claude(env),
        Agent::Codex => {
            let shim = require_shim(env)?;
            rewrite(env.home.join(".codex/config.toml"), |before| {
                with_codex_server(before, shim)
            })
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
            rewrite(env.home.join(".gemini/config/mcp_config.json"), |before| {
                with_mcp_server(before, "mcp_config.json", shim)
            })
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
            false,
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
    Ok(edit(before.as_deref())?
        .map(|after| Change::WriteFile {
            path,
            before,
            after,
            executable: false,
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

fn write_if_changed(
    path: PathBuf,
    after: &str,
    executable: bool,
) -> Result<Option<Change>, BansheeError> {
    let before = read_if_present(&path)?;
    if before.as_deref() == Some(after) {
        return Ok(None);
    }
    Ok(Some(Change::WriteFile {
        path,
        before,
        after: after.to_string(),
        executable,
    }))
}

pub(crate) const HOOK_SCRIPT_NAME: &str = "banshee-speak-check.sh";

fn parse_settings(settings: Option<&str>) -> Result<serde_json::Value, BansheeError> {
    match settings {
        Some(text) => serde_json::from_str(text)
            .map_err(|error| malformed("settings.json", &format!("is not valid JSON: {error}"))),
        None => Ok(serde_json::json!({})),
    }
}

fn stop_hook_command(root: &serde_json::Value) -> Option<String> {
    root["hooks"]["Stop"]
        .as_array()?
        .iter()
        .flat_map(|group| group["hooks"].as_array().into_iter().flatten())
        .filter_map(|hook| hook["command"].as_str())
        .find(|command| command.contains(HOOK_SCRIPT_NAME))
        .map(String::from)
}

// Whole file in, whole file out, so key order matches what Claude Code writes
fn with_stop_hook(settings: Option<&str>, command: &str) -> Result<Option<String>, BansheeError> {
    let mut root = parse_settings(settings)?;
    if stop_hook_command(&root).is_some() {
        return Ok(None);
    }
    let object = root
        .as_object_mut()
        .ok_or_else(|| malformed("settings.json", "is not a JSON object"))?;
    let stop = object
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| malformed("settings.json", "hooks is not an object"))?
        .entry("Stop")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| malformed("settings.json", "hooks.Stop is not a list"))?;
    stop.push(serde_json::json!({
        "hooks": [{
            "type": "command",
            "command": command,
            "timeout": 15,
            "statusMessage": "Checking you spoke",
        }]
    }));
    Ok(Some(pretty_json(&root)?))
}

pub fn render(change: &Change) -> String {
    match change {
        Change::Run { argv } => {
            let words: Vec<String> = argv.iter().map(|word| shell_word(word)).collect();
            format!("$ {}\n", words.join(" "))
        }
        Change::WriteFile {
            path,
            before: None,
            after,
            ..
        } => {
            format!(
                "new file {}, {} lines\n",
                path.display(),
                after.lines().count()
            )
        }
        Change::WriteFile {
            path,
            before: Some(before),
            after,
            ..
        } => {
            let name = path.display().to_string();
            similar::TextDiff::from_lines(before.as_str(), after.as_str())
                .unified_diff()
                .header(&name, &name)
                .to_string()
        }
    }
}

// Single quotes are enough for a path; the command is shown, never re-parsed
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
            after,
            executable,
            ..
        } => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // A partial write would truncate a file another tool owns
            let staged = path.with_extension(format!("banshee.{}", std::process::id()));
            std::fs::write(&staged, after)?;
            if *executable {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
            }
            std::fs::rename(&staged, path)?;
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

const PATH_START: &str = "__BANSHEE_PATH_START__";
const PATH_END: &str = "__BANSHEE_PATH_END__";

fn extract_path(output: &str) -> Option<OsString> {
    let after_start = output.split_once(PATH_START)?.1;
    let path = after_start.split_once(PATH_END)?.0;
    Some(OsString::from(path))
}

// A login profile that blocks would otherwise hold the first caller forever,
// and every later caller behind the OnceLock. Nothing measured either number.
// The wait trades how slow a profile may be against how long a hung one stalls
// agent detection. The poll trades wake-ups against how late the kill lands.
const SHELL_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
const SHELL_POLL: std::time::Duration = std::time::Duration::from_millis(50);

fn login_shell_path() -> Option<OsString> {
    use std::io::Read;

    let shell = std::env::var_os("SHELL")?;
    let command = format!(r#"printf '{PATH_START}%s{PATH_END}' "$PATH""#);
    let mut child = std::process::Command::new(&shell)
        .arg("-lc")
        .arg(&command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let deadline = std::time::Instant::now() + SHELL_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(SHELL_POLL);
            }
        }
    }

    let mut printed = String::new();
    child.stdout.take()?.read_to_string(&mut printed).ok()?;
    extract_path(&printed)
}

fn with_fallback(shell_path: Option<OsString>) -> OsString {
    match shell_path {
        Some(path) if !path.is_empty() => path,
        _ => std::env::var_os("PATH").unwrap_or_default(),
    }
}

pub(crate) fn resolved_path() -> OsString {
    static RESOLVED: std::sync::OnceLock<OsString> = std::sync::OnceLock::new();
    RESOLVED
        .get_or_init(|| with_fallback(login_shell_path()))
        .clone()
}

impl Env {
    pub fn from_machine() -> Result<Env, BansheeError> {
        Env::with_path(resolved_path())
    }

    #[cfg(test)]
    fn with_shell_path(shell_path: Option<OsString>) -> Result<Env, BansheeError> {
        Env::with_path(with_fallback(shell_path))
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
    for change in &changes {
        print!("{}", render(change));
        println!();
    }
    if !yes && !confirm("Apply? [y/N] ")? {
        return Err(BansheeError::Rejected("Nothing written.".into()));
    }
    apply_all(&changes, &env.path, |change| {
        if let Change::WriteFile { path, .. } = change {
            println!("wrote {}", path.display());
        }
    })?;
    println!(
        "{} is connected. Restart it to pick up the change.",
        agent.name()
    );
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

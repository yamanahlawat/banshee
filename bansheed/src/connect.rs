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
    /// The agent binaries found on PATH.
    pub on_path: Vec<&'static str>,
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
        Signal::OnPath(binary) => (env.on_path.contains(&binary), format!("{binary} on PATH")),
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
        // `claude mcp add` refuses a name it already holds, so a stale command
        // has to go first
        if env.claude_shim.is_some() {
            changes.push(Change::Run {
                argv: ["claude", "mcp", "remove", "--scope", "user", "banshee"]
                    .map(String::from)
                    .to_vec(),
            });
        }
        changes.push(Change::Run {
            argv: ["claude", "mcp", "add", "--scope", "user", "banshee", "--"]
                .into_iter()
                .map(String::from)
                .chain(std::iter::once(shim))
                .collect(),
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

pub fn apply(change: &Change) -> Result<(), BansheeError> {
    match change {
        Change::Run { argv } => {
            let Some(program) = argv.first() else {
                return Err(BansheeError::Other("a command with no program".into()));
            };
            let status = std::process::Command::new(program)
                .args(&argv[1..])
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
        let shim = crate::service::sibling(&exe, SHIM_NAME).ok();
        // Starting Claude Code makes it rewrite its own config, so detection never spawns an agent
        let on_path = Agent::ALL
            .iter()
            .filter_map(|agent| match agent.signal() {
                Signal::OnPath(binary) => Some(binary),
                Signal::HomeDir(_) => None,
            })
            .filter(|binary| crate::status::on_path(binary, &path))
            .collect();
        Ok(Env {
            home,
            claude_config_dir,
            banshee,
            shim,
            on_path,
            claude_shim: registered_claude_shim(&claude_global),
        })
    }
}

/// Applies in order and names what is left when one fails.
pub fn apply_all(changes: &[Change], mut written: impl FnMut(&Change)) -> Result<(), BansheeError> {
    for (done, change) in changes.iter().enumerate() {
        if let Err(error) = apply(change) {
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
    apply_all(&changes, |change| {
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
mod tests {
    use super::*;
    use std::path::PathBuf;

    pub(super) fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("banshee-connect-{name}-{}", std::process::id()));
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
            on_path: Vec::new(),
            claude_shim: None,
        }
    }

    fn installed_env(home: &std::path::Path, agent: Agent) -> Env {
        let mut env = env_at(home);
        match agent.signal() {
            Signal::OnPath(binary) => env.on_path.push(binary),
            Signal::HomeDir(dir) => std::fs::create_dir_all(home.join(dir)).unwrap(),
        }
        env
    }

    const SHIM: &str = "/opt/banshee/bin/banshee-mcp-shim";

    #[test]
    fn detection_searches_the_resolved_path_not_the_process_path() {
        let dir = scratch("resolved-path");
        std::fs::write(dir.join("codex"), "").unwrap();

        let env = Env::with_shell_path(Some(OsString::from(&dir))).unwrap();
        assert_eq!(env.on_path, vec!["codex"]);
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
        assert!(!crate::status::on_path("Cargo.toml", &OsString::new()));
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
            apply(&change).unwrap();
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
        env.on_path.push("codex");
        env.on_path.push("agy");
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
        env.on_path.push("codex");
        env.on_path.push("agy");
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
        env.on_path.push("claude");
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
                    argv: ["claude", "mcp", "remove", "--scope", "user", "banshee"]
                        .map(String::from)
                        .to_vec()
                },
                Change::Run {
                    argv: [
                        "claude",
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
        env.on_path.push("claude");
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
        env.on_path.push("claude");
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
        env.on_path.push("claude");
        let changes = plan(Agent::ClaudeCode, &env).unwrap();
        assert_eq!(changes.len(), 3, "{changes:?}");
        assert_eq!(
            changes[0],
            Change::Run {
                argv: [
                    "claude",
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
        env.on_path.push("claude");
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
        env.on_path.push("claude");
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
        env.on_path.push("claude");
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
        env.on_path.push("claude");
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

        let before =
            r#"{"mcp":{"banshee":{"type":"local","command":["/old/shim"],"timeout":20000}}}"#;
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
        env.on_path.push("claude");
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
        apply(&Change::WriteFile {
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
        let error = apply(&Change::Run {
            argv: vec!["false".into()],
        })
        .expect_err("false exits 1");
        assert!(error.to_string().contains("false"), "{error}");
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
        env.on_path.push("claude");
        env.claude_shim = Some("/old/bin/banshee-mcp-shim".into());
        let changes = plan(Agent::ClaudeCode, &env).unwrap();
        assert_eq!(
            changes[0],
            Change::Run {
                argv: ["claude", "mcp", "remove", "--scope", "user", "banshee"]
                    .map(String::from)
                    .to_vec()
            }
        );
        assert_eq!(
            changes[1],
            Change::Run {
                argv: [
                    "claude",
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
        env.on_path.push("claude");
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
}

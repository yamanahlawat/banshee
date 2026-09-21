//! Sends what you say to a coding agent, which changes the desktop.
//!
//! Banshee adds no prompt. The agent's own skills carry the desktop knowledge,
//! so a second copy here would be a second version to keep correct.

use crate::connect::Agent;
use banshee_common::error::BansheeError;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub use crate::config::TellConfig;

/// The thread Banshee last opened with an agent. `at` is Unix seconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub agent: String,
    pub id: String,
    pub at: u64,
}

/// The session id to continue, or `None` to start a new thread. A saved thread
/// from a different agent never carries over: the id means nothing to it.
pub fn resume(saved: Option<&Session>, agent: &str, now: u64, window: Duration) -> Option<String> {
    let session = saved?;
    if session.agent != agent {
        return None;
    }
    // A clock that went backwards reads as an expired thread, not an endless one.
    let elapsed = now.checked_sub(session.at)?;
    (elapsed <= window.as_secs()).then(|| session.id.clone())
}

/// Whether the words are the phrase that ends the thread. Banshee matches it
/// itself: an agent asked to forget cannot prove that it did.
pub fn is_reset(words: &str) -> bool {
    is_exactly(words, "start over")
}

/// Whether the words ask for the thread on a screen. Banshee matches it
/// itself: the agent is headless, so it has nowhere to put what it wrote.
pub fn is_show(words: &str) -> bool {
    is_exactly(words, "show me")
}

/// The phrase and nothing more. A longer sentence that starts the same way is a
/// command for the agent: "show me a list of themes" is one.
fn is_exactly(words: &str, phrase: &str) -> bool {
    let trimmed = words.trim().trim_end_matches(['.', '!']).trim();
    trimmed.eq_ignore_ascii_case(phrase)
}

/// The agents with a headless mode Banshee has measured. `tell` offers no
/// other `connect::Agent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Headless {
    ClaudeCode,
    OpenCode,
}

impl Headless {
    pub const ALL: [Headless; 2] = [Headless::ClaudeCode, Headless::OpenCode];

    pub fn agent(self) -> Agent {
        match self {
            Headless::ClaudeCode => Agent::ClaudeCode,
            Headless::OpenCode => Agent::OpenCode,
        }
    }

    pub fn name(self) -> &'static str {
        self.agent().name()
    }

    /// The binary to run. `connect` finds OpenCode by a directory, so it holds
    /// no binary name to reuse.
    pub fn binary(self) -> &'static str {
        match self {
            Headless::ClaudeCode => "claude",
            Headless::OpenCode => "opencode",
        }
    }

    /// The flag that continues a thread. Measured: the same flag on the root
    /// command opens the agent's own screen on that thread, where `run` and
    /// `--print` keep it headless.
    pub fn resume_flag(self) -> &'static str {
        match self {
            Headless::ClaudeCode => "--resume",
            Headless::OpenCode => "--session",
        }
    }

    /// Whether the folder list reaches the agent. Measured: Claude Code takes
    /// `--add-dir`, and OpenCode has no flag that takes one.
    pub fn scoped(self) -> bool {
        matches!(self, Headless::ClaudeCode)
    }

    pub fn from_name(name: &str) -> Option<Headless> {
        Headless::ALL.into_iter().find(|agent| agent.name() == name)
    }
}

fn known() -> String {
    Headless::ALL.map(Headless::name).join(" or ")
}

/// Order: the pinned agent, then Omarchy's default, then the first ready one.
pub fn agent_for(
    configured: Option<&str>,
    omarchy: Option<&str>,
    ready: &[Headless],
) -> Result<Headless, BansheeError> {
    if let Some(name) = configured.filter(|name| !name.is_empty()) {
        let agent = Headless::from_name(name).ok_or_else(|| {
            BansheeError::Rejected(format!(
                "tell.agent is {name}, which has no headless mode Banshee has measured. \
                 Use {}, or clear the setting.",
                known()
            ))
        })?;
        return if ready.contains(&agent) {
            Ok(agent)
        } else {
            Err(BansheeError::Rejected(format!(
                "tell.agent is {name}, and it is not connected and runnable. \
                 Run: banshee connect {name}"
            )))
        };
    }
    let from_omarchy = omarchy
        .and_then(Headless::from_name)
        .filter(|agent| ready.contains(agent));
    from_omarchy
        .or_else(|| ready.first().copied())
        .ok_or_else(|| {
            BansheeError::Rejected(format!(
                "no connected agent has a headless mode Banshee has measured. \
                 Run: banshee connect claude, or banshee connect opencode. \
                 Banshee knows {}.",
                known()
            ))
        })
}

/// The agent a command would run. `dir` is the directory the Omarchy probe
/// runs in, and it must exist, or the probe cannot spawn.
pub fn resolved_agent(
    config: &TellConfig,
    env: &crate::connect::Env,
    dir: &Path,
) -> Result<Headless, BansheeError> {
    agent_for(
        Some(config.agent.as_str()),
        omarchy_default(&env.path, dir).as_deref(),
        &ready(env),
    )
}

/// The arguments for one headless run.
///
/// `--` closes the flags, so a command that starts with a hyphen stays a
/// message. Measured on both agents.
pub fn argv_for(
    agent: Headless,
    words: &str,
    resume: Option<&str>,
    dir: &Path,
    allow: &[PathBuf],
) -> Vec<String> {
    let mut argv: Vec<String> = match agent {
        Headless::OpenCode => vec![
            "run".into(),
            "--dir".into(),
            dir.display().to_string(),
            // Without this, a write outside --dir is auto-rejected. It cannot
            // be narrowed: an opencode.json permission block hangs the run.
            "--auto".into(),
            "--format".into(),
            "json".into(),
        ],
        Headless::ClaudeCode => {
            let mut claude: Vec<String> = vec![
                "--print".into(),
                "--output-format".into(),
                "json".into(),
                "--permission-mode".into(),
                "acceptEdits".into(),
                // Without this the agent edits the config and stays silent.
                "--allowedTools".into(),
                "mcp__banshee__speak_status,mcp__banshee__ask_user".into(),
            ];
            // Every watched folder sits outside the run directory, so each one
            // needs a name here or the tools refuse it.
            for folder in allow {
                claude.push("--add-dir".into());
                claude.push(folder.display().to_string());
            }
            claude
        }
    };
    if let Some(id) = resume {
        argv.push(agent.resume_flag().into());
        argv.push(id.into());
    }
    argv.push("--".into());
    argv.push(words.into());
    argv
}

/// The thread the agent just used. OpenCode names it on every NDJSON line;
/// Claude Code names it once in a single object.
pub fn session_id(agent: Headless, stdout: &str) -> Option<String> {
    let key = match agent {
        Headless::OpenCode => "sessionID",
        Headless::ClaudeCode => "session_id",
    };
    objects(stdout).find_map(|value| value.get(key)?.as_str().map(str::to_string))
}

/// What the agent wrote, for the terminal. Output holding nothing readable is
/// not a failure.
pub fn reply(agent: Headless, stdout: &str) -> Option<String> {
    match agent {
        Headless::OpenCode => objects(stdout)
            .filter(|value| value.get("type").and_then(|kind| kind.as_str()) == Some("text"))
            .filter_map(|value| value.get("part")?.get("text")?.as_str().map(str::to_string))
            .next_back(),
        Headless::ClaudeCode => {
            objects(stdout).find_map(|value| value.get("result")?.as_str().map(str::to_string))
        }
    }
}

/// Tools the agent asked for and did not get. A refused `speak_status` is the
/// one failure that leaves the user with silence and no error.
pub fn denied_tools(stdout: &str) -> Vec<String> {
    objects(stdout)
        .filter_map(|value| value.get("permission_denials")?.as_array().cloned())
        .flatten()
        .filter_map(|denial| denial.get("tool_name")?.as_str().map(str::to_string))
        .collect()
}

/// Every line that parses as a JSON object. One object and NDJSON both fit, and
/// a line of plain text is skipped rather than fails the run.
fn objects(stdout: &str) -> impl DoubleEndedIterator<Item = serde_json::Value> + '_ {
    stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok())
}

static RUNNING: AtomicBool = AtomicBool::new(false);

/// One run at a time. The atomic guards two calls inside one daemon. The lock
/// file guards two `banshee tell` processes, which share no memory.
pub struct RunLock {
    path: PathBuf,
    /// What this holder wrote into the file. `Drop` compares it again. After a
    /// takeover the file names another holder, and a blind delete would remove
    /// a live lock.
    token: String,
}

impl RunLock {
    /// `stale_after` is the run's own deadline. A lock file older than that
    /// belonged to a killed process, so this takes it over.
    pub fn take(dir: &Path, stale_after: Duration) -> Option<RunLock> {
        if RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return None;
        }
        let path = dir.join("run.lock");
        let token = format!("{} {}", now_seconds(), std::process::id());
        if acquire_file_lock(&path, stale_after, &token) {
            Some(RunLock { path, token })
        } else {
            RUNNING.store(false, Ordering::Release);
            None
        }
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        if std::fs::read_to_string(&self.path).ok().as_deref() == Some(self.token.as_str()) {
            let _ = std::fs::remove_file(&self.path);
        }
        RUNNING.store(false, Ordering::Release);
    }
}

/// Creates `path` as the lock. `create_new` is atomic on POSIX, so the
/// creation is the lock.
fn acquire_file_lock(path: &Path, stale_after: Duration, token: &str) -> bool {
    if write_lock_file(path, token) {
        return true;
    }
    lock_is_stale(path, stale_after)
        && claim_stale(path, stale_after)
        && write_lock_file(path, token)
}

/// Moves a dead lock out of the way, and answers whether this caller moved it.
/// One caller can rename a given file, so two callers that both read it as
/// stale cannot both take it over.
///
/// std has no atomic "delete this file if it is still that one", so the second
/// check reads the file after the move. A caller that moved a live lock puts it
/// straight back.
fn claim_stale(path: &Path, stale_after: Duration) -> bool {
    let aside = with_suffix(path, &format!(".taken-{}", std::process::id()));
    if std::fs::rename(path, &aside).is_err() {
        return false;
    }
    if lock_is_stale(&aside, stale_after) {
        let _ = std::fs::remove_file(&aside);
        return true;
    }
    let _ = std::fs::rename(&aside, path);
    false
}

fn write_lock_file(path: &Path, token: &str) -> bool {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .is_ok_and(|mut file| write!(file, "{token}").is_ok())
}

/// A lock file is dead once its start time is older than `stale_after`.
/// Garbled or unreadable content reads as dead too: an unreadable lock must
/// not wedge the feature for ever either.
fn lock_is_stale(path: &Path, stale_after: Duration) -> bool {
    match std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse::<u64>().ok())
    {
        Some(started) => stale(started, now_seconds(), stale_after),
        None => true,
    }
}

fn stale(started: u64, now: u64, stale_after: Duration) -> bool {
    now.saturating_sub(started) > stale_after.as_secs()
}

/// `~` is the only expansion: a person writes the config file, not a shell.
pub fn expand(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(path),
    }
}

/// The name a folder's copy takes inside a snapshot. The whole path is kept,
/// so two watched folders with one basename cannot share a copy. `/` becomes
/// `%`, and a `%` in the path becomes `%25`, so no two paths give one name.
fn snapshot_key(source: &Path) -> String {
    source
        .to_string_lossy()
        .replace('%', "%25")
        .replace('/', "%")
}

/// The snapshot's own directory, created here. The name is `at`, raised past
/// every snapshot already in the store. A backward clock step must not give the
/// newest snapshot the lowest number. Two runs in one second must not share a
/// directory.
fn make_dir(into: &Path, at: u64) -> Result<PathBuf, BansheeError> {
    std::fs::create_dir_all(into)?;
    let highest = taken(into)?.into_iter().map(|(at, _)| at).max();
    let mut name = highest
        .filter(|top| *top >= at)
        .map_or(at, |top| top.saturating_add(1));
    loop {
        let made = into.join(name.to_string());
        match std::fs::create_dir(&made) {
            Ok(()) => return Ok(made),
            // A stray file can hold the name too, so this asks the filesystem
            // rather than the listing above.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => match name.checked_add(1) {
                Some(next) => name = next,
                None => return Err(e.into()),
            },
            Err(e) => return Err(e.into()),
        }
    }
}

/// Copies each folder into a directory of its own under `<into>`. A folder that
/// is not there is skipped rather than failed: the config may name a folder the
/// machine does not hold.
pub fn snapshot(paths: &[PathBuf], into: &Path, at: u64) -> Result<PathBuf, BansheeError> {
    let made = make_dir(into, at)?;
    for source in paths {
        // `/` and `..` name no folder a restore could put back.
        if !source.is_dir() || source.file_name().is_none() {
            continue;
        }
        if let Err(e) = copy_tree(source, &made.join(snapshot_key(source))) {
            // Only whole snapshots exist, so `--undo` cannot pick a
            // half-written one.
            let _ = std::fs::remove_dir_all(&made);
            return Err(e);
        }
    }
    Ok(made)
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), BansheeError> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            // Following the link would copy what it points at, and a dotfile
            // tree that links back on itself would loop for ever.
            let link_target = std::fs::read_link(entry.path())?;
            // std::os::unix::fs::symlink refuses an existing path, where
            // std::fs::copy below overwrites one silently. restore's staged
            // directory is only best-effort removed first, so a leftover
            // from an earlier attempt must not fail this one.
            let _ = std::fs::remove_file(&target);
            std::os::unix::fs::symlink(link_target, target)?;
        } else if file_type.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// The snapshot directories, each with the number its name holds. An entry that
/// is not a directory is not a snapshot. A stray file would outrank every real
/// snapshot here, and `remove_dir_all` fails on one, which wedges every later
/// run.
fn taken(snapshots: &Path) -> std::io::Result<Vec<(u64, PathBuf)>> {
    Ok(std::fs::read_dir(snapshots)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let at = entry.file_name().to_string_lossy().parse::<u64>().ok()?;
            Some((at, entry.path()))
        })
        .collect())
}

/// The snapshot to restore from.
pub fn newest(snapshots: &Path) -> Option<PathBuf> {
    taken(snapshots)
        .ok()?
        .into_iter()
        .max_by_key(|(at, _)| *at)
        .map(|(_, path)| path)
}

/// What a restore did with every watched folder: the ones it put back, and
/// the ones it could not, each paired with why not.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Restored {
    pub done: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// Puts each watched folder back.
///
/// A folder the snapshot does not hold is left alone. It did not exist when the
/// copy ran, and removal would take work the user did since.
///
/// A folder that is itself a symlink is refused, not replaced. A plain
/// directory in its place would stop every later edit from reaching the
/// dotfiles repo it points at.
///
/// A fault on one folder keeps the record of the folders already put back.
pub fn restore(from: &Path, paths: &[PathBuf]) -> Restored {
    let mut result = Restored::default();
    for target in paths {
        let copy = from.join(snapshot_key(target));
        if !copy.is_dir() {
            continue;
        }
        if std::fs::symlink_metadata(target).is_ok_and(|meta| meta.is_symlink()) {
            result.failed.push((
                target.display().to_string(),
                "is a symlink; put the copy back by hand".into(),
            ));
            continue;
        }
        match restore_one(&copy, target) {
            Ok(()) => result.done.push(target.display().to_string()),
            Err(e) => result
                .failed
                .push((target.display().to_string(), e.to_string())),
        }
    }
    result
}

/// Stages the copy beside the target and swaps with two renames, so a fault
/// never leaves the user with no config.
fn restore_one(copy: &Path, target: &Path) -> Result<(), BansheeError> {
    swap_in(copy, target, |from, to| std::fs::rename(from, to))
}

/// `rename` is taken rather than called, because no filesystem operation makes
/// the second rename fail while the first one works. A test gives its own.
fn swap_in(
    copy: &Path,
    target: &Path,
    rename: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), BansheeError> {
    let staged = with_suffix(target, ".banshee-restoring");
    let replaced = with_suffix(target, ".banshee-replaced");
    let _ = std::fs::remove_dir_all(&staged);
    let _ = std::fs::remove_dir_all(&replaced);
    copy_tree(copy, &staged)?;
    let mut moved_aside = false;
    if target.exists() {
        rename(target, &replaced).map_err(|e| BansheeError::file(target, e))?;
        moved_aside = true;
    }
    if let Err(e) = rename(&staged, target) {
        if moved_aside {
            return Err(put_back(&replaced, target, e));
        }
        return Err(BansheeError::file(target, e));
    }
    let _ = std::fs::remove_dir_all(&replaced);
    Ok(())
}

/// Without this the watched folder has nothing in it at all.
fn put_back(replaced: &Path, target: &Path, cause: std::io::Error) -> BansheeError {
    if std::fs::rename(replaced, target).is_ok() {
        return BansheeError::file(target, cause);
    }
    // Nothing else tells the user where the config went.
    BansheeError::Rejected(format!(
        "{}: {cause}. The config is now at {}. Move it back by hand.",
        target.display(),
        replaced.display()
    ))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// Puts the newest snapshot back.
pub fn undo(config: &TellConfig) -> Result<String, BansheeError> {
    undo_in(&state_dir()?, config)
}

/// `state` is Banshee's own directory, the one `run` locks and snapshots into.
fn undo_in(state: &Path, config: &TellConfig) -> Result<String, BansheeError> {
    // The margin matches `run`'s own, so undo never judges a still-active run
    // stale and steps on the folders it is mid-edit in.
    let Some(_lock) = RunLock::take(state, run_deadline(config) + PRE_SPAWN_MARGIN) else {
        return Err(BansheeError::Rejected(
            "a command is already running. Try again once it finishes.".into(),
        ));
    };
    let snapshots = snapshots_dir(state);
    let from = newest(&snapshots).ok_or_else(|| {
        BansheeError::Rejected("no snapshot to restore. Nothing has run yet.".into())
    })?;
    let home = crate::service::home_dir()?;
    let watched: Vec<PathBuf> = config.paths.iter().map(|p| expand(p, &home)).collect();
    let restored = restore(&from, &watched);
    let sentence = describe(&restored);
    if failed_outright(&restored) {
        return Err(BansheeError::Rejected(sentence));
    }
    Ok(sentence)
}

fn failed_outright(restored: &Restored) -> bool {
    restored.done.is_empty() && !restored.failed.is_empty()
}

/// The sentence `undo` prints. A partial restore still says what changed: the
/// user cannot see the screen.
fn describe(restored: &Restored) -> String {
    let done =
        (!restored.done.is_empty()).then(|| format!("Put back: {}", restored.done.join(", ")));
    let failed = (!restored.failed.is_empty()).then(|| {
        format!(
            "Could not put back: {}",
            restored
                .failed
                .iter()
                .map(|(name, why)| format!("{name} ({why})"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    match (done, failed) {
        (Some(done), Some(failed)) => format!("{done}. {failed}."),
        (Some(done), None) => done,
        (None, Some(failed)) => failed,
        (None, None) => "The snapshot held none of the watched folders. Nothing changed.".into(),
    }
}

/// Keeps the `keep` newest snapshots.
pub fn prune(snapshots: &Path, keep: usize) -> Result<(), BansheeError> {
    let mut made = taken(snapshots)?;
    made.sort_by_key(|(at, _)| *at);
    let extra = made.len().saturating_sub(keep);
    for (_, path) in made.into_iter().take(extra) {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

/// Banshee's own directory for `tell`.
pub fn state_dir() -> Result<PathBuf, BansheeError> {
    let dir = crate::service::home_dir()?.join(".banshee").join("tell");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The agent's working directory. Each agent reads a settings file from the
/// directory it runs in, so the spawn gets one of its own rather than a project.
///
/// It is not `state` itself. The agent may create, edit and delete anything
/// inside its own directory. Nothing Banshee keeps may live there: an agent
/// that lists the directory would find the snapshots and edit a copy.
pub fn agent_dir(state: &Path) -> PathBuf {
    state.join("run")
}

fn snapshots_dir(state: &Path) -> PathBuf {
    state.join("snapshots")
}

/// `None` for a missing or unreadable file. A lost thread costs one repeated
/// sentence; a refused run costs the feature.
pub fn read_session(dir: &Path) -> Option<Session> {
    let text = std::fs::read_to_string(dir.join("session.json")).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_session(dir: &Path, session: &Session) -> Result<(), BansheeError> {
    std::fs::write(dir.join("session.json"), serde_json::to_string(session)?)?;
    Ok(())
}

/// Clears the saved thread. A file that is not there is the ordinary case: no
/// thread has been saved yet, or the last one expired.
///
/// Any other fault leaves the thread in place, so the answer is an error. The
/// cue for a run that worked would otherwise tell the user their next command
/// starts fresh, and it would not.
fn clear_thread(state: &Path) -> Result<Told, BansheeError> {
    let file = state.join("session.json");
    if let Err(e) = std::fs::remove_file(&file)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(BansheeError::Rejected(format!(
            "the thread is still there. Banshee could not remove {}: {e}",
            file.display()
        )));
    }
    Ok(Told {
        reply: Some("Thread cleared.".to_string()),
        ..Told::default()
    })
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

/// Installed, planned with no changes left, and with a binary on PATH. This is
/// the same test `connect::row` calls "connected".
pub fn ready(env: &crate::connect::Env) -> Vec<Headless> {
    Headless::ALL
        .into_iter()
        .filter(|agent| {
            let connected = matches!(
                crate::connect::detect(agent.agent(), env),
                crate::connect::Presence::Installed
            ) && crate::connect::plan(agent.agent(), env)
                .is_ok_and(|changes| changes.is_empty());
            connected && crate::status::resolve(agent.binary(), &env.path).is_some()
        })
        .collect()
}

/// A stated allowance, not a measurement, for a command that only prints a
/// name. It runs inside the run lock, before the module's only other deadline.
/// A hang here leaves the user with no cue.
const OMARCHY_BOUND: Duration = Duration::from_secs(5);

/// What `omarchy-default-agent` prints, or `None` where the command is absent,
/// hangs, or prints nothing. The command exits 0 and prints nothing when
/// Omarchy has no default.
fn omarchy_default(path: &std::ffi::OsStr, dir: &Path) -> Option<String> {
    let program = crate::status::resolve("omarchy-default-agent", path)?;
    let Ran::Finished { output, .. } = run_bounded(&program, &[], dir, path, OMARCHY_BOUND).ok()?
    else {
        return None;
    };
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// What `notify` carries before the first spawn. `None` on a resumed thread:
/// the scope was stated on the turn that opened it.
fn opening_announcement(agent: Headless, resume_id: Option<&str>) -> Option<String> {
    resume_id.is_none().then(|| {
        if agent.scoped() {
            format!(
                "Running {}. It gets the folders in tell.paths, and its own run directory.",
                agent.name()
            )
        } else {
            format!(
                "Running {}. It can edit anything: OpenCode takes no folder list.",
                agent.name()
            )
        }
    })
}

enum Ran {
    /// `stdout_lost` says the read of stdout gave up before the bytes came. An
    /// empty `output.stdout` then means the pipe stayed open, not that the
    /// child stayed quiet. The session id and the reply are in there.
    Finished {
        output: std::process::Output,
        stdout_lost: bool,
    },
    TimedOut,
}

/// A stated allowance, not a measurement, for a reader thread to finish once
/// its pipe should be closed. `opencode run` starts a local server, so a
/// descendant often outlives the child and holds the pipe open.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Runs the child bounded by `timeout`. Stdout and stderr each drain on their
/// own thread, so a full pipe buffer cannot deadlock the wait.
///
/// The wait polls `try_wait` rather than blocks on `wait`. A child stuck on a
/// network call or a hung tool is killed, not held on to for ever.
fn run_bounded(
    program: &Path,
    argv: &[String],
    dir: &Path,
    env_path: &std::ffi::OsStr,
    timeout: Duration,
) -> std::io::Result<Ran> {
    let mut child = std::process::Command::new(program)
        .args(argv)
        .current_dir(dir)
        // The login shell PATH, not the daemon's: an agent CLI is often a
        // script whose interpreter the daemon's PATH does not hold.
        .env("PATH", env_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout_rx = drain(child.stdout.take().expect("stdout was piped"));
    let stderr_rx = drain(child.stderr.take().expect("stderr was piped"));

    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            let (stdout, stderr, stdout_lost) = collect_both(stdout_rx, stderr_rx);
            return Ok(Ran::Finished {
                output: std::process::Output {
                    status,
                    stdout,
                    stderr,
                },
                stdout_lost,
            });
        }
        if Instant::now() >= deadline {
            // A descendant the child left running can still hold the pipe's
            // write end open. A kill of the child does not close it, so this
            // waits on the child and never on the pipes.
            let _ = child.kill();
            let _ = child.wait();
            return Ok(Ran::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Reads a pipe to the end on its own thread, then sends the bytes. The caller
/// gets a channel, not a `JoinHandle`.
///
/// A descendant can hold the write end open after the child is gone, so the
/// send may never happen. Every read of this channel is bounded by
/// `DRAIN_GRACE`, and an orphaned reader is never joined.
fn drain(mut pipe: impl Read + Send + 'static) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx
}

/// Collects both channels concurrently, so two `DRAIN_GRACE` waits cost one
/// `DRAIN_GRACE`, not two. Stderr carries no lost flag of its own: it only
/// decorates the message of a run that already failed.
fn collect_both(
    stdout_rx: std::sync::mpsc::Receiver<Vec<u8>>,
    stderr_rx: std::sync::mpsc::Receiver<Vec<u8>>,
) -> (Vec<u8>, Vec<u8>, bool) {
    // This join cannot hang: the thread's own wait is bounded by DRAIN_GRACE.
    let stderr_thread = std::thread::spawn(move || collect(stderr_rx).0);
    let (stdout, stdout_lost) = collect(stdout_rx);
    let stderr = stderr_thread.join().unwrap_or_default();
    (stdout, stderr, stdout_lost)
}

/// The bytes, and whether the read gave up before they came.
fn collect(rx: std::sync::mpsc::Receiver<Vec<u8>>) -> (Vec<u8>, bool) {
    match rx.recv_timeout(DRAIN_GRACE) {
        Ok(bytes) => (bytes, false),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (Vec::new(), true),
        // The reader ended without sending, so the pipe held nothing.
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => (Vec::new(), false),
    }
}

/// A stated allowance, not a measurement, for the checks `run` makes before it
/// writes the lock file. `omarchy_default`, `ready` and `agent_for` each spawn
/// or probe outside this process with no bound of their own.
///
/// Without this margin a slow prefix could hold the lock past
/// `run_timeout_min`. A second command would then judge a live lock stale.
const PRE_SPAWN_MARGIN: Duration = Duration::from_secs(30);

/// The ceiling on every configured span, stated and not measured. A day is past
/// any run a person waits through.
///
/// Without it a large value overflows three things: the `Duration` the lock
/// adds its margin to, the `Instant` the poll compares against, and the seconds
/// `resume` counts. All three panic, and the hotkey path runs in a thread where
/// a panic reaches nobody.
const MAX_SPAN: Duration = Duration::from_secs(24 * 60 * 60);

fn span(minutes: u64) -> Duration {
    Duration::from_secs(minutes.saturating_mul(60)).min(MAX_SPAN)
}

fn run_deadline(config: &TellConfig) -> Duration {
    span(config.run_minutes())
}

fn thread_window(config: &TellConfig) -> Duration {
    span(config.thread_minutes())
}

/// What one run answers with once it ends. The scope is not here: it has to
/// arrive before the run, so `notify` carries it instead.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Told {
    /// What to print once the command ends.
    pub reply: Option<String>,
    pub warnings: Vec<Warning>,
}

/// What went wrong in a run that still exited 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    DeniedTools(String),
    LostOutput(String),
}

impl Warning {
    pub fn text(&self) -> &str {
        match self {
            Warning::DeniedTools(line) | Warning::LostOutput(line) => line,
        }
    }

    /// Whether the user perceives nothing at all. A refused `speak_status`
    /// sounds like a run that worked, so only that kind needs a cue.
    ///
    /// A lost reply still reaches the user: the agent spoke over MCP while it
    /// ran.
    pub fn leaves_the_user_with_silence(&self) -> bool {
        matches!(self, Warning::DeniedTools(_))
    }
}

/// Prints what the agent may edit, then starts it. The line goes out before the
/// spawn, so a `banshee tell` user can still press Ctrl-C.
fn announce_then_start<T>(
    notify: &dyn Fn(&str),
    agent: Headless,
    resume_id: Option<&str>,
    start: impl FnOnce() -> T,
) -> T {
    if let Some(line) = opening_announcement(agent, resume_id) {
        notify(&line);
    }
    start()
}

/// The terminals Banshee opens a thread in, each with the words that put a
/// command inside it. Omarchy's own launcher comes first: it floats the window
/// and wraps the run. The rest are the terminals `tell.paths` already names, so
/// a machine without Omarchy still gets a screen.
const TERMINALS: [(&str, &[&str]); 5] = [
    ("omarchy-launch-floating-terminal-with-presentation", &[]),
    ("alacritty", &["-e", "sh", "-c"]),
    ("ghostty", &["-e", "sh", "-c"]),
    ("foot", &["sh", "-c"]),
    ("kitty", &["sh", "-c"]),
];

fn terminal(path: &std::ffi::OsStr) -> Option<(PathBuf, &'static [&'static str])> {
    TERMINALS
        .iter()
        .find_map(|(name, words)| Some((crate::status::resolve(name, path)?, *words)))
}

/// The command the terminal runs. It carries the directory and the binary
/// itself. Omarchy's launcher hands the line to a systemd unit, which starts it
/// elsewhere with a PATH the daemon never chose.
fn show_line(agent: Headless, binary: &Path, dir: &Path, id: &str) -> String {
    format!(
        "cd {} && {} {} {}",
        quoted(&dir.display().to_string()),
        quoted(&binary.display().to_string()),
        agent.resume_flag(),
        quoted(id)
    )
}

/// One shell word. A home directory may hold a space, and every launcher hands
/// the line to a shell.
pub(crate) fn quoted(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

fn thread_to_show(
    saved: Option<&Session>,
    now: u64,
    window: Duration,
) -> Result<(Headless, String), String> {
    let Some(session) = saved else {
        return Err("There is no thread to show. Nothing has run yet.".into());
    };
    let Some(agent) = Headless::from_name(&session.agent) else {
        return Err(format!(
            "The last thread belongs to {}, which Banshee cannot open.",
            session.agent
        ));
    };
    let Some(id) = resume(saved, &session.agent, now, window) else {
        return Err("The last thread has timed out. Say something to start a new one.".into());
    };
    Ok((agent, id))
}

/// Opens the stored thread in a terminal, and runs no agent. The hotkey path
/// drops the reply, so this is the only way that path shows what the agent
/// wrote.
///
/// The thread comes from `state` and the terminal opens in `run_in`: the agent
/// on that screen writes wherever it is started, exactly as the headless one
/// does.
fn show(
    state: &Path,
    run_in: &Path,
    config: &TellConfig,
    path: &std::ffi::OsStr,
    notify: &dyn Fn(&str),
) -> Result<Told, BansheeError> {
    let (agent, id) = thread_to_show(
        read_session(state).as_ref(),
        now_seconds(),
        thread_window(config),
    )
    .map_err(BansheeError::Rejected)?;
    let binary = crate::status::resolve(agent.binary(), path)
        .ok_or_else(|| BansheeError::Rejected(format!("{} is not on PATH", agent.binary())))?;
    let (program, words) = terminal(path).ok_or_else(|| {
        BansheeError::Rejected(
            "no terminal Banshee knows is on PATH, so it has nowhere to open the thread.".into(),
        )
    })?;
    let mut argv: Vec<String> = words.iter().map(|word| (*word).to_string()).collect();
    argv.push(show_line(agent, &binary, run_in, &id));
    // Sent before the spawn, and not returned as a reply: the window is
    // detached, so a launcher that dies after exec reaches nobody.
    notify(&format!("Opening the thread in {}.", agent.name()));
    let mut child = std::process::Command::new(&program)
        .args(&argv)
        .env("PATH", path)
        .stdin(Stdio::null())
        .spawn()?;
    // Reaped off this thread: the window outlives the command, and a child
    // nobody waits on stays in the daemon's process table for its whole life.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(Told::default())
}

/// Runs one command. `notify` fires with the scope before the agent starts.
pub fn run(words: &str, config: &TellConfig, notify: &dyn Fn(&str)) -> Result<Told, BansheeError> {
    let state = state_dir()?;
    let run_in = agent_dir(&state);
    let Some(_lock) = RunLock::take(&state, run_deadline(config) + PRE_SPAWN_MARGIN) else {
        return Err(BansheeError::Rejected(
            "a command is already running. Wait for it to finish.".into(),
        ));
    };
    // Taken with the lock held. A reset racing a live run must not be undone
    // when that run writes a fresh session back.
    if is_reset(words) {
        return clear_thread(&state);
    }
    std::fs::create_dir_all(&run_in)?;
    if is_show(words) {
        return show(
            &state,
            &run_in,
            config,
            &crate::connect::resolved_path(),
            notify,
        );
    }

    let env = crate::connect::Env::from_machine()?;
    let agent = resolved_agent(config, &env, &state)?;
    let program = crate::status::resolve(agent.binary(), &env.path)
        .ok_or_else(|| BansheeError::Rejected(format!("{} is not on PATH", agent.binary())))?;

    let home = crate::service::home_dir()?;
    let watched: Vec<PathBuf> = config.paths.iter().map(|p| expand(p, &home)).collect();
    let present: Vec<PathBuf> = watched.iter().filter(|p| p.is_dir()).cloned().collect();

    let snapshots = snapshots_dir(&state);
    snapshot(&present, &snapshots, now_seconds())?;
    prune(&snapshots, config.keep())?;

    let saved = read_session(&state);
    let resume_id = resume(
        saved.as_ref(),
        agent.name(),
        now_seconds(),
        thread_window(config),
    );
    let argv = argv_for(agent, words, resume_id.as_deref(), &run_in, &present);
    let ran = announce_then_start(notify, agent, resume_id.as_deref(), || {
        run_bounded(&program, &argv, &run_in, &env.path, run_deadline(config))
    })?;
    let Ran::Finished {
        output,
        stdout_lost,
    } = ran
    else {
        return Err(BansheeError::Rejected(timed_out(
            agent,
            run_deadline(config),
        )));
    };
    let stdout = String::from_utf8_lossy(&output.stdout);

    let thread = thread_to_save(
        session_id(agent, &stdout),
        resume_id.as_deref(),
        stdout_lost,
    );
    if let Some(id) = &thread {
        write_session(
            &state,
            &Session {
                agent: agent.name().to_string(),
                id: id.clone(),
                at: now_seconds(),
            },
        )?;
    }
    let mut warnings: Vec<Warning> = denied_warning(agent, &denied_tools(&stdout))
        .into_iter()
        .collect();
    if stdout_lost {
        warnings.push(lost_output_warning(agent, thread.is_some()));
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BansheeError::Other(
            std::iter::once(format!(
                "{} exited {}: {stderr}",
                agent.name(),
                output.status
            ))
            .chain(warnings.iter().map(|warning| warning.text().to_string()))
            .collect::<Vec<_>>()
            .join(" "),
        ));
    }
    Ok(Told {
        reply: Some(
            reply(agent, &stdout)
                .unwrap_or_else(|| format!("{} finished and wrote nothing.", agent.name())),
        ),
        warnings,
    })
}

/// The sentence a run that ran out of time answers with. The minutes are the
/// deadline that ran, which `span` may have cut below the configured value.
fn timed_out(agent: Headless, deadline: Duration) -> String {
    let minutes = deadline.as_secs() / 60;
    if deadline >= MAX_SPAN {
        return format!(
            "{} did not answer within {minutes} minutes, which is Banshee's ceiling. \
             tell.run_timeout_min cannot go higher.",
            agent.name()
        );
    }
    format!(
        "{} did not answer within {minutes} minutes. Raise tell.run_timeout_min to give it longer.",
        agent.name()
    )
}

/// The thread to save. A lost stdout carries no id away. A resumed run still
/// knows which thread it asked for, and "a bit more" needs it.
fn thread_to_save(
    found: Option<String>,
    resume_id: Option<&str>,
    stdout_lost: bool,
) -> Option<String> {
    match found {
        Some(id) => Some(id),
        None if stdout_lost => resume_id.map(str::to_string),
        None => None,
    }
}

/// What the user reads when the read of stdout gave up. `kept` says whether
/// the thread survived, because the next command behaves differently.
fn lost_output_warning(agent: Headless, kept: bool) -> Warning {
    let thread = if kept {
        "The thread is kept."
    } else {
        "The next command starts a new thread."
    };
    Warning::LostOutput(format!(
        "{} finished, but its output did not arrive in time. Its reply is lost. {thread}",
        agent.name()
    ))
}

fn denied_warning(agent: Headless, denied: &[String]) -> Option<Warning> {
    (!denied.is_empty()).then(|| {
        Warning::DeniedTools(format!(
            "{} was refused these tools, so it may have worked in silence: {}",
            agent.name(),
            denied.join(", ")
        ))
    })
}

#[cfg(test)]
mod tests;

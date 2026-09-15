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

    /// The `connect` agent this is, so the slug and the connected check both
    /// come from one place.
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
    /// no binary name for it and this is the only place one exists.
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
pub fn denied_tools(agent: Headless, stdout: &str) -> Vec<String> {
    if !matches!(agent, Headless::ClaudeCode) {
        return Vec::new();
    }
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

/// One run at a time. Two agents editing the same files at once leave a config
/// neither of them wrote, and the second run's snapshot holds the first run's
/// half-done work. The atomic guards two calls inside the daemon, which stays
/// up; the lock file guards two `banshee tell` invocations, which share no
/// memory to guard with.
pub struct RunLock {
    path: PathBuf,
    /// What this holder wrote into the file. Compared again in `Drop`: after a
    /// spurious takeover the file names a different holder, and deleting it
    /// then would delete a live lock rather than a dead one.
    token: String,
}

impl RunLock {
    /// `stale_after` is the run's own deadline: a lock file older than the
    /// time a run is allowed to take belonged to a process that was killed,
    /// not one still working, so taking it over is correct rather than
    /// wedging the feature for good.
    pub fn take(dir: &Path, stale_after: Duration) -> Option<RunLock> {
        if RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return None;
        }
        let path = dir.join("run.lock");
        // The pid alongside the start time is enough to tell two holders
        // apart; a distributed lock is not the bar this clears.
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
/// creation itself is the lock rather than a check a second caller could race.
fn acquire_file_lock(path: &Path, stale_after: Duration, token: &str) -> bool {
    if write_lock_file(path, token) {
        return true;
    }
    if lock_is_stale(path, stale_after) {
        let _ = std::fs::remove_file(path);
        return write_lock_file(path, token);
    }
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

/// Copies each folder into `<into>/<at>/<folder name>`. A folder that is not
/// there is skipped: most machines have only two or three of the six.
pub fn snapshot(paths: &[PathBuf], into: &Path, at: u64) -> Result<PathBuf, BansheeError> {
    let made = into.join(at.to_string());
    std::fs::create_dir_all(&made)?;
    for source in paths {
        if !source.is_dir() {
            continue;
        }
        let Some(name) = source.file_name() else {
            continue;
        };
        if let Err(e) = copy_tree(source, &made.join(name)) {
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

/// The most recent snapshot. Names are Unix seconds, so the newest is the
/// highest number. A stray file that parses as a number is not a snapshot: it
/// would outrank every real one and leave nothing to restore.
pub fn newest(snapshots: &Path) -> Option<PathBuf> {
    std::fs::read_dir(snapshots)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let at = entry.file_name().to_string_lossy().parse::<u64>().ok()?;
            Some((at, entry.path()))
        })
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

/// Puts each folder back, and reports the ones it replaced and the ones it
/// could not.
///
/// A folder the snapshot does not hold is left alone: it did not exist when
/// the copy ran, and removal would take work the user did since. A folder that
/// is itself a symlink is refused rather than replaced: a plain directory in
/// its place would silently stop every later edit from reaching wherever it
/// points (a dotfiles repo, say). A fault on one folder does not cost the
/// record of the folders already put back.
pub fn restore(from: &Path, paths: &[PathBuf]) -> Restored {
    let mut result = Restored::default();
    for target in paths {
        let Some(name) = target.file_name() else {
            continue;
        };
        let copy = from.join(name);
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
    let staged = with_suffix(target, ".banshee-restoring");
    let replaced = with_suffix(target, ".banshee-replaced");
    let _ = std::fs::remove_dir_all(&staged);
    let _ = std::fs::remove_dir_all(&replaced);
    copy_tree(copy, &staged)?;
    if target.exists() {
        std::fs::rename(target, &replaced)?;
    }
    std::fs::rename(&staged, target)?;
    let _ = std::fs::remove_dir_all(&replaced);
    Ok(())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// Puts the newest snapshot back, and reports the folders it replaced and the
/// ones it could not.
pub fn undo(config: &TellConfig) -> Result<String, BansheeError> {
    undo_in(&dir()?, config)
}

/// `dir` is `tell`'s own working directory, the one `run` locks and snapshots
/// into.
fn undo_in(dir: &Path, config: &TellConfig) -> Result<String, BansheeError> {
    // The margin matches `run`'s own, so undo never judges a still-active run
    // stale and steps on the folders it is mid-edit in.
    let Some(_lock) = RunLock::take(dir, run_deadline(config) + PRE_SPAWN_MARGIN) else {
        return Err(BansheeError::Rejected(
            "a command is already running. Try again once it finishes.".into(),
        ));
    };
    let snapshots = dir.join("snapshots");
    let from = newest(&snapshots).ok_or_else(|| {
        BansheeError::Rejected("no snapshot to restore. Nothing has run yet.".into())
    })?;
    let home = crate::service::home_dir()?;
    let watched: Vec<PathBuf> = config.paths.iter().map(|p| expand(p, &home)).collect();
    let restored = restore(&from, &watched);
    let sentence = describe(&restored);
    if failed_outright(&restored) {
        // Rejected, not Other: Other prints "Internal error:" in front of the
        // text, and this sentence is the one the user reads.
        return Err(BansheeError::Rejected(sentence));
    }
    Ok(sentence)
}

/// A restore that put nothing back and failed. A partial restore stays a
/// success: it did put folders back.
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

/// Keeps the `keep` newest snapshots. Names are Unix seconds, so they sort as
/// numbers, not as text.
pub fn prune(snapshots: &Path, keep: usize) -> Result<(), BansheeError> {
    let mut made: Vec<(u64, PathBuf)> = std::fs::read_dir(snapshots)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let at = entry.file_name().to_string_lossy().parse::<u64>().ok()?;
            Some((at, entry.path()))
        })
        .collect();
    made.sort_by_key(|(at, _)| *at);
    let extra = made.len().saturating_sub(keep);
    for (_, path) in made.into_iter().take(extra) {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

/// The agent's working directory. Each agent reads a settings file from the
/// directory it runs in, so the spawn gets one of its own rather than a project.
pub fn dir() -> Result<PathBuf, BansheeError> {
    let dir = crate::service::home_dir()?.join(".banshee").join("tell");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
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

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

/// Connected and runnable: connected the way `connect::row` answers it, plus a
/// binary on PATH.
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
/// name. This runs inside the run lock and before the only other deadline in
/// the module, so a hang here leaves the user with no cue at all.
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

/// What running the child produced: what it wrote, or that it ran out of time.
enum Ran {
    /// `stdout_lost` says the read of stdout gave up before the bytes came, so
    /// an empty `output.stdout` means the pipe was still held open rather than
    /// that the child stayed quiet. The session id and the reply are in there.
    Finished {
        output: std::process::Output,
        stdout_lost: bool,
    },
    TimedOut,
}

/// A stated allowance, not a measurement, for a reader thread to finish once
/// its pipe should already be closed. `opencode run` starts a local server, so
/// a descendant surviving the child and holding its end of the pipe open is
/// the likely case rather than the rare one.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Runs the child bounded by `timeout`. Stdout and stderr each drain on their
/// own thread as the child writes, so a full pipe buffer cannot deadlock the
/// wait. The wait itself polls `try_wait` rather than blocking on `wait`, so a
/// child stuck on a network call or a hung tool is killed rather than held on
/// to for ever.
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
            // A descendant the child left running (a server it started, say)
            // can still hold the pipe's write end open, and killing the child
            // does not close it. So this waits on the child, never the pipes.
            let _ = child.kill();
            let _ = child.wait();
            return Ok(Ran::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Reads a pipe to the end on its own thread and sends the bytes once done,
/// rather than a `JoinHandle` the caller would join. A descendant the child
/// left behind can hold the pipe's write end open long after the child
/// itself is gone, so the send may never happen; every read of this channel
/// is bounded by `DRAIN_GRACE` instead, and an orphaned reader is left to end
/// on its own, whenever that is, rather than joined.
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
/// writes the lock file: `omarchy_default`, `ready` and `agent_for` each spawn
/// or probe outside this process with no bound of their own. Without this
/// margin, a slow prefix could hold the lock longer than `run_timeout_min`,
/// and a second command would then judge a live lock stale and start beside it.
const PRE_SPAWN_MARGIN: Duration = Duration::from_secs(30);

/// The ceiling on every configured span. A day is past any run a person waits
/// through and past any thread they still hold in mind, and the ceiling is
/// stated, not measured. Without it a large configured value overflows the
/// `Duration` the lock adds its margin to, the `Instant` the poll compares
/// against, and the minutes `resume` turns into seconds. All three panic, and
/// the hotkey path runs in a thread of its own, where a panic leaves the user
/// waiting for a result that never comes.
const MAX_SPAN: Duration = Duration::from_secs(24 * 60 * 60);

/// Configured minutes, bounded.
fn span(minutes: u64) -> Duration {
    Duration::from_secs(minutes.saturating_mul(60)).min(MAX_SPAN)
}

/// How long one agent run may take.
fn run_deadline(config: &TellConfig) -> Duration {
    span(config.run_timeout_min)
}

/// How long a saved thread stays resumable.
fn thread_window(config: &TellConfig) -> Duration {
    span(config.thread_timeout_min)
}

/// What one run answers with once it ends. The scope is not here: it has to
/// arrive before the run, so `notify` carries it instead.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Told {
    /// What to print once the command ends. A command that only opened a
    /// screen has none: it wrote its line through `notify` before the spawn.
    pub reply: Option<String>,
    /// What went wrong without failing the run.
    pub warnings: Vec<String>,
}

/// Names what the agent may edit, then starts it. Said first, not last: only
/// while the run is open can the user still press Ctrl-C.
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

/// The first terminal on PATH, and how it takes a command.
fn terminal(path: &std::ffi::OsStr) -> Option<(PathBuf, &'static [&'static str])> {
    TERMINALS
        .iter()
        .find_map(|(name, words)| Some((crate::status::resolve(name, path)?, *words)))
}

/// The command the terminal runs. It carries the directory and the binary
/// itself: Omarchy's launcher hands the line to a systemd unit, which starts it
/// somewhere else, with a PATH the daemon never chose.
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
fn quoted(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// The thread a screen can be opened on, or why there is none.
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

/// Opens the stored thread in a terminal, and runs no agent. The headless agent
/// writes to the journal, which nobody reads, so this is the one way what it
/// wrote reaches the user.
fn show(
    dir: &Path,
    config: &TellConfig,
    path: &std::ffi::OsStr,
    notify: &dyn Fn(&str),
) -> Result<Told, BansheeError> {
    let (agent, id) = thread_to_show(
        read_session(dir).as_ref(),
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
    argv.push(show_line(agent, &binary, dir, &id));
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

/// Runs one command. It prints nothing, and `notify` fires with the scope
/// before the agent starts.
pub fn run(words: &str, config: &TellConfig, notify: &dyn Fn(&str)) -> Result<Told, BansheeError> {
    let dir = dir()?;
    let Some(_lock) = RunLock::take(&dir, run_deadline(config) + PRE_SPAWN_MARGIN) else {
        return Err(BansheeError::Rejected(
            "a command is already running. Wait for it to finish.".into(),
        ));
    };
    // Taken with the lock held: a reset racing an in-flight run must not be
    // undone by that run writing a fresh session back once it finishes.
    if is_reset(words) {
        let _ = std::fs::remove_file(dir.join("session.json"));
        return Ok(Told {
            reply: Some("Thread cleared.".to_string()),
            ..Told::default()
        });
    }
    if is_show(words) {
        return show(&dir, config, &crate::connect::resolved_path(), notify);
    }

    let env = crate::connect::Env::from_machine()?;
    let agent = resolved_agent(config, &env, &dir)?;
    let program = crate::status::resolve(agent.binary(), &env.path)
        .ok_or_else(|| BansheeError::Rejected(format!("{} is not on PATH", agent.binary())))?;

    let home = crate::service::home_dir()?;
    let watched: Vec<PathBuf> = config.paths.iter().map(|p| expand(p, &home)).collect();
    let present: Vec<PathBuf> = watched.iter().filter(|p| p.is_dir()).cloned().collect();

    let snapshots = dir.join("snapshots");
    snapshot(&present, &snapshots, now_seconds())?;
    prune(&snapshots, config.keep())?;

    let saved = read_session(&dir);
    let resume_id = resume(
        saved.as_ref(),
        agent.name(),
        now_seconds(),
        thread_window(config),
    );
    let argv = argv_for(agent, words, resume_id.as_deref(), &dir, &present);
    // An agent CLI is often a script whose interpreter the daemon's own PATH
    // does not hold.
    let ran = announce_then_start(notify, agent, resume_id.as_deref(), || {
        run_bounded(&program, &argv, &dir, &env.path, run_deadline(config))
    })?;
    let Ran::Finished {
        output,
        stdout_lost,
    } = ran
    else {
        return Err(BansheeError::Rejected(format!(
            "{} did not answer within {} minutes. Raise tell.run_timeout_min to give it longer.",
            agent.name(),
            config.run_timeout_min
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
            &dir,
            &Session {
                agent: agent.name().to_string(),
                id: id.clone(),
                at: now_seconds(),
            },
        )?;
    }
    let mut warnings: Vec<String> = denied_warning(agent, &denied_tools(agent, &stdout))
        .into_iter()
        .collect();
    if stdout_lost {
        warnings.push(lost_output_warning(agent, thread.is_some()));
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        // A failed run answers with no `Told`, and a refused tool still says
        // why the agent stayed silent.
        return Err(BansheeError::Other(
            std::iter::once(format!(
                "{} exited {}: {stderr}",
                agent.name(),
                output.status
            ))
            .chain(warnings)
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

/// The thread to save. A lost stdout carries no id away with it, but a resumed
/// run still knows which thread it asked for, and "a bit more" needs it.
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
fn lost_output_warning(agent: Headless, kept: bool) -> String {
    let thread = if kept {
        "The thread is kept."
    } else {
        "The next command starts a new thread."
    };
    format!(
        "{} finished, but its output did not arrive in time. Its reply is lost. {thread}",
        agent.name()
    )
}

/// What the user reads about the tools `denied_tools` found.
fn denied_warning(agent: Headless, denied: &[String]) -> Option<String> {
    (!denied.is_empty()).then(|| {
        format!(
            "{} was refused these tools, so it may have worked in silence: {}",
            agent.name(),
            denied.join(", ")
        )
    })
}

#[cfg(test)]
mod tests;

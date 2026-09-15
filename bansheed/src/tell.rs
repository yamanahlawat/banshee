//! Sends what you say to a coding agent, which changes the desktop.
//!
//! Banshee adds no prompt. The agent's own skills carry the desktop knowledge,
//! so a second copy here would be a second version to keep correct.

use crate::connect::Agent;
use banshee_common::error::BansheeError;
use serde::{Deserialize, Serialize};

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
pub fn resume(saved: Option<&Session>, agent: &str, now: u64, timeout_min: u64) -> Option<String> {
    let session = saved?;
    if session.agent != agent {
        return None;
    }
    // A clock that went backwards reads as an expired thread rather than an
    // endless one, so `checked_sub` decides instead of a subtraction that wraps.
    let elapsed = now.checked_sub(session.at)?;
    (elapsed <= timeout_min * 60).then(|| session.id.clone())
}

/// Ends the thread and spawns nothing. Banshee matches the phrase itself,
/// because an agent asked to forget cannot prove that it did.
pub fn is_reset(words: &str) -> bool {
    let trimmed = words.trim().trim_end_matches(['.', '!']).trim();
    trimmed.eq_ignore_ascii_case("start over")
}

/// The agents with a headless mode Banshee has measured. Every other
/// `connect::Agent` has no entry, and `tell` does not offer it.
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

    /// Whether the folder list reaches the agent. Measured: Claude Code takes
    /// `--add-dir`; OpenCode's only working flag is `--auto`, which allows any
    /// edit anywhere.
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

use std::path::{Path, PathBuf};

/// The arguments for one headless run. Every flag is measured. The plan's
/// Global Constraints say what breaks without each one.
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
        let flag = match agent {
            Headless::OpenCode => "--session",
            Headless::ClaudeCode => "--resume",
        };
        argv.push(flag.into());
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

/// What the agent wrote. The user already heard it speak, so this serves the
/// terminal, and a run that printed nothing readable is not a failure.
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

use std::io::Read;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static RUNNING: AtomicBool = AtomicBool::new(false);

/// One run at a time: the atomic guards two calls inside this one process
/// (the hotkey path lives in the daemon, which stays up), and the lock file
/// guards two separate `banshee tell` invocations, which share no memory to
/// guard with. Two agents editing the same files at once leave a config
/// neither of them wrote, and the second run's snapshot holds the first run's
/// half-done work.
pub struct RunLock {
    path: PathBuf,
    /// What this holder wrote into the file. Compared again in `Drop`: a
    /// spurious takeover (the margin `run()` adds still exceeded, or a clock
    /// oddity) means the file now names a different holder, and deleting it
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

/// Pure, so the rule is tested directly rather than only through a file.
fn stale(started: u64, now: u64, stale_after: Duration) -> bool {
    now.saturating_sub(started) > stale_after.as_secs()
}

/// `~` is the only expansion. A person writes the config file, and a person
/// writes `~`.
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
            // Clean up the partial snapshot so only whole snapshots exist, and
            // Task 7's `--undo` cannot pick a half-written one.
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
            // Recreate the symlink as-is, rather than following it. A restore then
            // writes back what the user actually had. A symlink loop or a dotfile
            // tree that links back on itself cannot crash the restore.
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
/// highest number. A stray file that happens to parse as a number is not a
/// snapshot: without `is_dir`, it would win here and every folder would then
/// find no copy to restore, which reads as a misleading success.
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
/// The copy lands beside the folder first and the swap is two renames, so a
/// fault never leaves the user with no config. A folder the snapshot does not
/// hold is left alone: it did not exist when the copy ran, and removal would
/// take work the user did since. A folder that is itself a symlink is refused
/// rather than replaced: swapping it for a plain directory would silently
/// stop every later edit from reaching wherever it points (a dotfiles repo,
/// say). A fault on one folder does not cost the record of the folders
/// already put back: the loop carries on rather than aborting on the first
/// error.
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

/// The staged-copy-then-two-renames swap for one folder.
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

/// `dir` is `tell`'s own working directory, the same one `run` locks and
/// snapshots into. Kept apart from `undo` so a test can pass a scratch
/// directory instead of the real one.
fn undo_in(dir: &Path, config: &TellConfig) -> Result<String, BansheeError> {
    // Restoring folders while an agent is mid-edit is exactly the corruption
    // this lock exists to prevent, and undo is the command a user reaches for
    // when something is already going wrong. The margin matches `run`'s own,
    // so undo never judges a still-active run stale early and steps on it.
    let run_deadline = Duration::from_secs(config.run_timeout_min.saturating_mul(60));
    let Some(_lock) = RunLock::take(dir, run_deadline + PRE_SPAWN_MARGIN) else {
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
    Ok(describe(&restore(&from, &watched)))
}

/// The sentence `undo` prints. A user who cannot see the screen needs to hear
/// what changed even when part of the restore did not go through, not only
/// when all of it did or none of it did.
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

/// Connected and runnable. `connect` answers connected the way `connect::row`
/// does at `connect.rs:137`. The binary is resolved apart from that, because
/// `connect` finds OpenCode by a directory and holds no binary name for it.
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

/// What `omarchy-default-agent` prints, or `None` where the command is absent.
/// The command exits 0 and prints nothing when Omarchy has no default.
fn omarchy_default(path: &std::ffi::OsStr) -> Option<String> {
    let output = std::process::Command::new("omarchy-default-agent")
        .env("PATH", path)
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// What to print before the first spawn with a new agent. `None` on a resumed
/// thread: the design asks for this only on the first turn.
fn opening_announcement(agent: Headless, resume_id: Option<&str>) -> Option<String> {
    resume_id.is_none().then(|| {
        if agent.scoped() {
            format!(
                "Running {}. It can edit only the folders in tell.paths.",
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
    Finished(std::process::Output),
    TimedOut,
}

/// A stated allowance, not a measurement, for a reader thread to finish once
/// its pipe should already be closed. `opencode run` starts a local server, so
/// a descendant surviving the child and holding its end of the pipe open is
/// the likely case rather than the rare one; a reader is never joined without
/// this bound.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Runs the child bounded by `timeout`. Stdout and stderr each drain on their
/// own thread as the child writes, so a full pipe buffer cannot deadlock the
/// wait. The wait itself polls `try_wait` rather than blocking on `wait`, so a
/// child stuck on a network call or a hung tool is killed instead of held on
/// to forever: the user cannot see the terminal, so a hang must not read as
/// success.
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
            let (stdout, stderr) = collect_both(stdout_rx, stderr_rx);
            return Ok(Ran::Finished(std::process::Output {
                status,
                stdout,
                stderr,
            }));
        }
        if Instant::now() >= deadline {
            // The child itself is killed here, but a descendant it left
            // running (a server it started, say) can still hold the pipe's
            // write end open. Killing this child alone would not close it,
            // so this waits on the child only, never on the pipes.
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
/// `DRAIN_GRACE`, not two.
fn collect_both(
    stdout_rx: std::sync::mpsc::Receiver<Vec<u8>>,
    stderr_rx: std::sync::mpsc::Receiver<Vec<u8>>,
) -> (Vec<u8>, Vec<u8>) {
    // This join cannot hang: the thread's own wait is bounded by DRAIN_GRACE.
    let stderr_thread = std::thread::spawn(move || collect(stderr_rx));
    let stdout = collect(stdout_rx);
    let stderr = stderr_thread.join().unwrap_or_default();
    (stdout, stderr)
}

fn collect(rx: std::sync::mpsc::Receiver<Vec<u8>>) -> Vec<u8> {
    rx.recv_timeout(DRAIN_GRACE).unwrap_or_default()
}

/// A stated allowance, not a measurement, for the checks `run` makes before it
/// writes the lock file: `omarchy_default`, `ready` and `agent_for` each spawn
/// or probe outside this process with no bound of their own. Without this
/// margin, a slow prefix could hold the lock longer than `run_timeout_min`,
/// and a second command would then judge a live lock stale and start beside it.
const PRE_SPAWN_MARGIN: Duration = Duration::from_secs(30);

/// Runs one command. Answers with what the agent wrote, for the terminal. The
/// user already heard it: the agent speaks through Banshee's own MCP server.
pub fn run(words: &str, config: &TellConfig) -> Result<Option<String>, BansheeError> {
    let dir = dir()?;
    let run_deadline = Duration::from_secs(config.run_timeout_min.saturating_mul(60));
    let Some(_lock) = RunLock::take(&dir, run_deadline + PRE_SPAWN_MARGIN) else {
        return Err(BansheeError::Rejected(
            "a command is already running. Wait for it to finish.".into(),
        ));
    };
    // Taken with the lock held: a reset racing an in-flight run must not be
    // undone by that run writing a fresh session back once it finishes.
    if is_reset(words) {
        let _ = std::fs::remove_file(dir.join("session.json"));
        return Ok(Some("Thread cleared.".to_string()));
    }

    let env = crate::connect::Env::from_machine()?;
    let agent = agent_for(
        Some(config.agent.as_str()),
        omarchy_default(&env.path).as_deref(),
        &ready(&env),
    )?;
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
        config.thread_timeout_min,
    );
    if let Some(line) = opening_announcement(agent, resume_id.as_deref()) {
        println!("{line}");
    }
    let argv = argv_for(agent, words, resume_id.as_deref(), &dir, &present);

    // An agent CLI is often a script whose interpreter the daemon's own PATH
    // does not hold. `connect` learned this first.
    let output = match run_bounded(&program, &argv, &dir, &env.path, run_deadline)? {
        Ran::Finished(output) => output,
        Ran::TimedOut => {
            return Err(BansheeError::Rejected(format!(
                "{} did not answer within {} minutes. Raise tell.run_timeout_min to give it \
                 longer.",
                agent.name(),
                config.run_timeout_min
            )));
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);

    if let Some(id) = session_id(agent, &stdout) {
        write_session(
            &dir,
            &Session {
                agent: agent.name().to_string(),
                id,
                at: now_seconds(),
            },
        )?;
    }
    let denied = denied_tools(agent, &stdout);
    if !denied.is_empty() {
        eprintln!(
            "{} was refused these tools, so it may have worked in silence: {}",
            agent.name(),
            denied.join(", ")
        );
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BansheeError::Other(format!(
            "{} exited {}: {stderr}",
            agent.name(),
            output.status
        )));
    }
    Ok(reply(agent, &stdout))
}

#[cfg(test)]
mod tests;

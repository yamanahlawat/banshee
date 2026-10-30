//! The app's login jobs, through `SMAppService`.
//!
//! `banshee-app --login-items <register|unregister|status> [daemon|tray|all]`
//! reads each agent's status first:
//!
//! - `status` calls nothing more.
//! - `register` calls nothing more unless the agent is `not-registered` or
//!   `not-found`. Only the user's switch in Login Items changes
//!   `requires-approval`.
//! - `unregister` on an agent `not-registered` or `not-found` calls nothing more.
//! - Otherwise it calls `register` or `unregister`, then reads the status again.
//!
//! For each agent that succeeds, stdout gets one line, `<label> <word>`, with
//! the word from `Registration::word`. For each agent that fails, stderr gets
//! `<label>: <error>` and stdout gets nothing. The agents run in order, daemon
//! then tray, and a failure does not stop the next one. So on a failure with
//! `all`, stdout holds the lines for the agents that succeeded.
//!
//! The exit code is 0 when every agent succeeded, 1 when any failed, and 2 when
//! the arguments are not a request.

use banshee_common::utils::{DAEMON_AGENT, Registration, TRAY_AGENT};
use objc2::rc::{Retained, autoreleasepool};
use objc2_foundation::{NSError, NSString};
use objc2_service_management::{SMAppService, SMAppServiceStatus};

const USAGE: &str =
    "usage: banshee-app --login-items <register|unregister|status> [daemon|tray|all]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Register,
    Unregister,
    Status,
}

/// What the words after `--login-items` ask for, and the labels they name.
fn request(args: &[String]) -> Result<(Action, &'static [&'static str]), String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let (action, target) = match words.as_slice() {
        [action] => (*action, "all"),
        [action, target] => (*action, *target),
        _ => return Err(USAGE.into()),
    };
    let action = match action {
        "register" => Action::Register,
        "unregister" => Action::Unregister,
        "status" => Action::Status,
        _ => return Err(USAGE.into()),
    };
    let labels: &'static [&'static str] = match target {
        "all" => &[DAEMON_AGENT, TRAY_AGENT],
        "daemon" => &[DAEMON_AGENT],
        "tray" => &[TRAY_AGENT],
        _ => return Err(USAGE.into()),
    };
    Ok((action, labels))
}

fn needs_the_api(action: Action, now: Registration) -> bool {
    match action {
        Action::Status => false,
        Action::Register => !now.is_registered(),
        Action::Unregister => now.is_registered(),
    }
}

/// Carries out a `--login-items` request as the module doc describes, and
/// returns the process's exit code.
pub fn run(args: &[String]) -> i32 {
    let (action, labels) = match request(args) {
        Ok(request) => request,
        Err(usage) => {
            eprintln!("{usage}");
            return 2;
        }
    };
    let mut code = 0;
    for &label in labels {
        match settle(action, label) {
            Ok(state) => println!("{label} {}", state.word()),
            Err(error) => {
                eprintln!("{label}: {error}");
                code = 1;
            }
        }
    }
    code
}

fn settle(action: Action, label: &str) -> Result<Registration, String> {
    let now = registration(label)?;
    if !needs_the_api(action, now) {
        return Ok(now);
    }
    if action == Action::Register {
        register(label)?;
    } else {
        unregister(label)?;
    }
    registration(label)
}

/// The agent whose plist ships as `Contents/Library/LaunchAgents/{label}.plist`.
fn agent(label: &str) -> Retained<SMAppService> {
    let plist = NSString::from_str(&format!("{label}.plist"));
    // SAFETY: the plist name is a live NSString, and the call only builds a
    // handle; macOS reads the bundle when the handle is used.
    unsafe { SMAppService::agentServiceWithPlistName(&plist) }
}

fn describe(error: &NSError) -> String {
    format!(
        "{} ({} {})",
        error.localizedDescription(),
        error.domain(),
        error.code()
    )
}

fn register(label: &str) -> Result<(), String> {
    autoreleasepool(|_| {
        // SAFETY: the handle is a live SMAppService, and the call takes no other input.
        unsafe { agent(label).registerAndReturnError() }.map_err(|error| describe(&error))
    })
}

fn unregister(label: &str) -> Result<(), String> {
    autoreleasepool(|_| {
        // SAFETY: the handle is a live SMAppService, and the call takes no other input.
        unsafe { agent(label).unregisterAndReturnError() }.map_err(|error| describe(&error))
    })
}

fn registration(label: &str) -> Result<Registration, String> {
    // SAFETY: the handle is a live SMAppService, and the call only reads.
    let status = autoreleasepool(|_| unsafe { agent(label).status() });
    match status {
        SMAppServiceStatus::Enabled => Ok(Registration::Enabled),
        SMAppServiceStatus::RequiresApproval => Ok(Registration::RequiresApproval),
        SMAppServiceStatus::NotRegistered => Ok(Registration::NotRegistered),
        SMAppServiceStatus::NotFound => Ok(Registration::NotFound),
        SMAppServiceStatus(other) => Err(format!(
            "SMAppService reported status {other}, which macOS does not document"
        )),
    }
}

/// What the window does so the menu bar icon runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayStep {
    Leave,
    /// `SMAppService` has no call that starts a stopped agent.
    Reregister,
    Register,
}

fn tray_step(now: Registration, running: bool) -> TrayStep {
    match (running, now) {
        (true, _) => TrayStep::Leave,
        (false, Registration::Enabled) => TrayStep::Reregister,
        (false, Registration::RequiresApproval) => TrayStep::Leave,
        (false, _) => TrayStep::Register,
    }
}

pub fn keep_the_tray_running() -> Result<(), String> {
    let running = banshee_common::utils::tray_lock_path().is_some_and(|lock| held(&lock));
    match tray_step(registration(TRAY_AGENT)?, running) {
        TrayStep::Leave => Ok(()),
        TrayStep::Reregister => {
            unregister(TRAY_AGENT)?;
            register(TRAY_AGENT)
        }
        TrayStep::Register => register(TRAY_AGENT),
    }
}

/// Whether another open file holds an exclusive `flock` on `lock`.
/// A tray claiming the lock in the instant this holds it exits 1, and launchd relaunches it.
fn held(lock: &std::path::Path) -> bool {
    std::fs::File::open(lock)
        .is_ok_and(|file| matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn an_unbundled_binary_reads_as_not_found() {
        assert_eq!(registration(DAEMON_AGENT), Ok(Registration::NotFound));
    }

    #[test]
    fn a_request_names_its_action_and_agents() {
        assert_eq!(
            request(&words("status")),
            Ok((Action::Status, &[DAEMON_AGENT, TRAY_AGENT][..]))
        );
        assert_eq!(
            request(&words("register tray")),
            Ok((Action::Register, &[TRAY_AGENT][..]))
        );
        assert_eq!(
            request(&words("unregister daemon")),
            Ok((Action::Unregister, &[DAEMON_AGENT][..]))
        );
        for wrong in ["", "start", "status speaker", "status daemon tray"] {
            assert!(
                request(&words(wrong)).is_err(),
                "{wrong:?} is not a request"
            );
        }
    }

    #[test]
    fn only_a_change_of_state_calls_the_api() {
        use Registration::*;
        let cases = [
            (Action::Register, Enabled, false),
            (Action::Register, RequiresApproval, false),
            (Action::Register, NotRegistered, true),
            (Action::Register, NotFound, true),
            (Action::Unregister, Enabled, true),
            (Action::Unregister, RequiresApproval, true),
            (Action::Unregister, NotRegistered, false),
            (Action::Unregister, NotFound, false),
        ];
        for (action, now, expected) in cases {
            assert_eq!(
                needs_the_api(action, now),
                expected,
                "{action:?} on {now:?}"
            );
        }
        for now in [Enabled, RequiresApproval, NotRegistered, NotFound] {
            assert!(!needs_the_api(Action::Status, now), "status on {now:?}");
        }
    }

    #[test]
    fn a_lock_reads_as_held_only_while_another_file_holds_it() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("tray.lock");
        assert!(!held(&lock), "a missing lock is nobody's");

        let holder = std::fs::File::create(&lock).unwrap();
        assert!(!held(&lock), "a lock file nobody locked is free");
        holder.lock().unwrap();
        assert!(held(&lock));
        drop(holder);
        assert!(!held(&lock), "the lock goes with the file that held it");
    }

    #[test]
    fn the_window_starts_only_a_tray_that_is_not_running() {
        use Registration::*;
        let cases = [
            (Enabled, true, TrayStep::Leave),
            // A tray started by hand holds the lock with no registration.
            (NotRegistered, true, TrayStep::Leave),
            (Enabled, false, TrayStep::Reregister),
            (NotRegistered, false, TrayStep::Register),
            (NotFound, false, TrayStep::Register),
            (RequiresApproval, false, TrayStep::Leave),
        ];
        for (now, running, expected) in cases {
            assert_eq!(
                tray_step(now, running),
                expected,
                "{now:?}, running: {running}"
            );
        }
    }
}

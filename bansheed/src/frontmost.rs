//! Which app is in front, as a process id. Nothing else about it is read.

/// A process id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct App(pub i32);

impl App {
    #[cfg(any(unix, test))]
    fn known(pid: i32) -> Option<App> {
        (pid > 0).then_some(App(pid))
    }
}

/// What `banshee frontmost-pid` printed, read as an app.
#[cfg(any(target_os = "macos", test))]
fn pid_of(printed: &str) -> Option<App> {
    App::known(printed.trim().parse().ok()?)
}

/// An unknown read on either side never counts as a move.
pub fn moved(at_stop: Option<App>, now: Option<App>) -> bool {
    matches!((at_stop, now), (Some(then), Some(now)) if then != now)
}

// NSWorkspace moves its frontmost app only on a run loop, and the daemon's
// threads run none. A fresh process reads it once.
#[cfg(target_os = "macos")]
pub fn frontmost() -> Option<App> {
    let mut command = std::process::Command::new(std::env::current_exe().ok()?);
    command.arg("frontmost-pid").env_clear();
    pid_of(std::str::from_utf8(&printed_within_cap(&mut command)?).ok()?)
}

// A ceiling on a hung child, not a tuned value.
#[cfg(unix)]
const CAP: std::time::Duration = std::time::Duration::from_millis(250);
#[cfg(unix)]
const POLL: std::time::Duration = std::time::Duration::from_millis(1);

/// What `command` printed, when it exits with success within `CAP`. A child
/// still running then is killed and reaped.
#[cfg(unix)]
fn printed_within_cap(command: &mut std::process::Command) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::process::Stdio;

    use crate::connect::exit_by;

    let deadline = std::time::Instant::now() + CAP;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let Some(Ok(status)) = exit_by(deadline, POLL, || child.try_wait()) else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let mut printed = Vec::new();
    child.stdout.take()?.read_to_end(&mut printed).ok()?;
    status.success().then_some(printed)
}

/// What `banshee frontmost-pid` does in the child that `frontmost` starts.
#[cfg(target_os = "macos")]
pub fn print_pid() -> std::process::ExitCode {
    use objc2_app_kit::NSWorkspace;

    let pid = objc2::rc::autoreleasepool(|_| {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|app| app.processIdentifier())
    });
    match pid {
        Some(pid) => {
            println!("{pid}");
            std::process::ExitCode::SUCCESS
        }
        None => std::process::ExitCode::FAILURE,
    }
}

// X11 and other compositors answer `None`: no read for them was measured.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn frontmost() -> Option<App> {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    // A stop holds the push-to-talk lock through this read, and the login
    // shell's PATH probe can take seconds. Until something else resolves that
    // PATH, the app is unknown.
    let path = crate::connect::resolved_path_in_hand()?;
    let hyprctl = crate::status::resolve("hyprctl", &path)?;
    let mut command = std::process::Command::new(hyprctl);
    command.args(["activewindow", "-j"]);
    hyprland_app(&printed_within_cap(&mut command)?)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn hyprland_app(json: &[u8]) -> Option<App> {
    #[derive(serde::Deserialize)]
    struct Active {
        pid: Option<i32>,
    }
    let active: Active = serde_json::from_slice(json).ok()?;
    App::known(active.pid?)
}

#[cfg(not(unix))]
pub fn frontmost() -> Option<App> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_two_known_and_different_apps_count_as_moved() {
        assert!(moved(Some(App(1)), Some(App(2))));
        assert!(!moved(Some(App(1)), Some(App(1))));
        assert!(!moved(None, Some(App(1))));
        assert!(!moved(Some(App(1)), None));
        assert!(!moved(None, None));
    }

    #[test]
    fn the_child_prints_a_known_pid_or_nothing_counts() {
        assert_eq!(pid_of(""), None);
        assert_eq!(pid_of("-1\n"), None);
        assert_eq!(pid_of("0"), None);
        assert_eq!(pid_of("abc"), None);
        assert_eq!(pid_of("4242\n"), Some(App(4242)));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn hyprland_gives_the_pid_and_nothing_else() {
        assert_eq!(
            hyprland_app(br#"{"address":"0x5a1","pid":4242,"class":"kitty","title":"x"}"#),
            Some(App(4242))
        );
        assert_eq!(hyprland_app(br#"{"pid":-1}"#), None);
        assert_eq!(hyprland_app(b"not json"), None);
        assert_eq!(hyprland_app(b"{}"), None, "an empty workspace");
        assert_eq!(hyprland_app(br#"{"pid":null,"class":null}"#), None);
    }
}

// macOS gates the hotkey and dictation behind Accessibility; the grant applies
// only to processes started after it. No equivalent elsewhere, so always true.

use std::thread;
use std::time::Duration;

// How often we look for a grant that landed while we were running.
const POLL: Duration = Duration::from_secs(2);

#[cfg(target_os = "macos")]
pub fn input_granted() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> u8;
    }
    // Boolean in C is an unsigned char, so u8 instead of bool keeps the call sound
    unsafe { AXIsProcessTrusted() != 0 }
}

#[cfg(not(target_os = "macos"))]
pub fn input_granted() -> bool {
    true
}

/// What macOS reports for a grant it has been asked about.
#[cfg(target_os = "macos")]
pub enum Access {
    Granted,
    Denied,
    /// Never decided. The first event tap turns this into granted or denied.
    Undetermined,
}

/// Input Monitoring, which `rdev`'s event tap needs on top of Accessibility.
/// Its absence is the only one that leaves no trace: events are withheld, so
/// there is no hotkey, no earcon, no error, and nothing in the log.
#[cfg(target_os = "macos")]
pub fn hotkey_events_granted() -> Access {
    // IOHIDRequestType, in header order: PostEvent is 0, ListenEvent is 1
    const LISTEN_EVENT: i32 = 1;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOHIDCheckAccess(request: i32) -> i32;
    }
    // IOHIDAccessType, in header order
    match unsafe { IOHIDCheckAccess(LISTEN_EVENT) } {
        0 => Access::Granted,
        1 => Access::Denied,
        _ => Access::Undetermined,
    }
}

// Straight to the pane, rather than describing where it lives.
#[cfg(target_os = "macos")]
pub fn open_settings() {
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .status();
}

#[cfg(not(target_os = "macos"))]
pub fn open_settings() {}

/// Exit once the missing grant lands, so the supervisor restarts us with it.
/// launchd re-runs on nonzero exit (`KeepAlive`/`SuccessfulExit`) and so does
/// systemd (`Restart=on-failure`), and the next start reclaims the stale socket.
pub fn restart_when_granted() {
    if input_granted() {
        return;
    }
    tracing::warn!("accessibility not granted; hotkey and dictation are inert until it is");
    thread::spawn(|| {
        loop {
            thread::sleep(POLL);
            if input_granted() {
                tracing::info!("accessibility granted, restarting to pick it up");
                std::process::exit(1);
            }
        }
    });
}

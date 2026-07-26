// macOS gates the hotkey and dictation behind Accessibility, and the grant only
// applies to processes started after it. Other platforms have no equivalent, so
// the check is vacuously true there.

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

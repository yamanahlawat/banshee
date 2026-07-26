use arboard::Clipboard;
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};
use std::{error::Error, thread, time::Duration};

// Let the clipboard settle before we paste. On the ownership-based platforms
// this is also what gives the owner thread time to publish.
const PASTE_SETTLE: Duration = Duration::from_millis(50);

// enigo pastes through X11, so a wayland session has to shell out instead.
// Checked at call time rather than at startup: the daemon can be launched
// from a session manager that sets neither variable until later.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland")
        || std::env::var("WAYLAND_DISPLAY").is_ok()
}

pub fn type_text(text: &str) -> Result<(), Box<dyn Error>> {
    #[cfg(all(unix, not(target_os = "macos")))]
    if is_wayland() {
        return type_text_wayland(text);
    }

    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| {
        format!(
            "Accessibility permissions missing, Please grant them in settings! {}",
            e
        )
    })?;
    let mut clipboard = Clipboard::new().map_err(|e| {
        format!(
            "Clipboard access failed! Please grant permission in System Settings: {}",
            e
        )
    })?;

    let old_clipboard = clipboard.get_text().ok();

    stage(clipboard, text, old_clipboard)?;

    let modifier = if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    };

    thread::sleep(PASTE_SETTLE);

    enigo.key(modifier, Press)?;
    enigo.key(Key::Unicode('v'), Click)?;
    enigo.key(modifier, Release)?;

    Ok(())
}

// wtype drives the wlroots virtual-keyboard protocol (Hyprland, Sway); ydotool
// goes through uinput and needs its daemon, so it is only the second choice.
// There is no clipboard-and-paste fallback: synthesising Ctrl+V would need one
// of these same two tools, so it could never work when both are missing.
#[cfg(all(unix, not(target_os = "macos")))]
pub const WAYLAND_TYPERS: [(&str, &[&str]); 2] = [("wtype", &["--"]), ("ydotool", &["type", "--"])];

#[cfg(all(unix, not(target_os = "macos")))]
fn type_text_wayland(text: &str) -> Result<(), Box<dyn Error>> {
    use std::process::Command;

    let mut attempts = Vec::new();
    for (binary, args) in WAYLAND_TYPERS {
        // `--` keeps a transcription that opens with a dash from being read as
        // a flag; without it wtype exits 1 on "Missing argument to -foo".
        match Command::new(binary).args(args).arg(text).output() {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                attempts.push(format!(
                    "{binary} exited with {}: {}",
                    output.status,
                    stderr.trim()
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                attempts.push(format!("{binary} is not installed"));
            }
            Err(e) => attempts.push(format!("{binary} failed to start: {e}")),
        }
    }

    // Never report success here: the caller plays the ready cue on Ok, which
    // would claim the text landed in an app that never received it.
    Err(format!(
        "could not type into the focused window on wayland ({}); \
         install wtype or ydotool. the transcription is still in `banshee history`",
        attempts.join("; ")
    )
    .into())
}

// Put the dictated text on the clipboard and arrange to put the old text back.
// The two implementations differ because the platforms disagree on who owns
// clipboard contents: macOS and Windows hand it to a system service that
// outlives us, X11 and Wayland keep it in a live process.

// Restore only after the app has consumed the paste.
#[cfg(not(all(unix, not(target_os = "macos"))))]
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(500);

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn stage(mut clipboard: Clipboard, text: &str, old: Option<String>) -> Result<(), Box<dyn Error>> {
    clipboard.set_text(text.to_string())?;

    // Restore off the hot path so the ready cue fires without waiting.
    if let Some(old) = old {
        let dictated = text.to_string();
        thread::spawn(move || {
            thread::sleep(CLIPBOARD_RESTORE_DELAY);
            let Ok(mut clipboard) = Clipboard::new() else {
                return;
            };
            // Anything the user copied during the delay outranks the restore
            if clipboard
                .get_text()
                .is_ok_and(|current| current == dictated)
            {
                let _ = clipboard.set_text(old);
            }
        });
    }

    Ok(())
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    use super::*;

    // Both tools take the text after `--`. Without it, wtype reads a leading
    // dash as a flag and fails with "Missing argument to -foo", losing any
    // transcription that happens to start with one.
    #[test]
    fn every_wayland_typer_terminates_its_options() {
        for (binary, args) in WAYLAND_TYPERS {
            assert_eq!(
                args.last(),
                Some(&"--"),
                "{binary} must take the text after `--`"
            );
        }
    }

    #[test]
    fn failing_to_type_is_reported_as_an_error() {
        // No wayland typer resolves under a scrubbed PATH, so this exercises
        // the both-missing path without depending on the host's tools
        let path = std::env::var_os("PATH");
        // SAFETY: single-threaded test, restored before returning
        unsafe { std::env::set_var("PATH", "") };
        let result = type_text_wayland("hello");
        if let Some(path) = path {
            unsafe { std::env::set_var("PATH", path) };
        }

        let error = result.expect_err("missing typers must not report success");
        let message = error.to_string();
        assert!(message.contains("wtype"), "unhelpful error: {message}");
        assert!(message.contains("ydotool"), "unhelpful error: {message}");
    }
}

// The caller's handle only ever read; ownership belongs to the thread below.
#[cfg(all(unix, not(target_os = "macos")))]
fn stage(_clipboard: Clipboard, text: &str, old: Option<String>) -> Result<(), Box<dyn Error>> {
    use arboard::SetExtLinux;
    use std::time::Instant;

    // The target app fetches the selection asynchronously after Ctrl+V, so we
    // have to still own it well after the keystroke.
    const PASTE_WINDOW: Duration = Duration::from_secs(2);
    // How long the restored text stays served. A daemon can afford to sit on it.
    const RESTORE_HOLD: Duration = Duration::from_secs(120);
    // wait_until never says why it returned, so a clearly early return is read
    // as another app having taken the clipboard. Drop this once arboard reports
    // the reason.
    const SLACK: Duration = Duration::from_millis(250);

    let dictated = text.to_string();
    thread::spawn(move || {
        let Ok(mut clipboard) = Clipboard::new() else {
            return;
        };
        let started = Instant::now();
        // Keeps dictated speech out of clipboard manager history
        let published = clipboard
            .set()
            .exclude_from_history()
            .wait_until(started + PASTE_WINDOW)
            .text(dictated);
        if published.is_err() || started.elapsed() + SLACK < PASTE_WINDOW {
            // Errored, or someone else took the clipboard; either way it is no
            // longer ours to put back.
            return;
        }
        if let Some(old) = old {
            let _ = clipboard
                .set()
                .wait_until(Instant::now() + RESTORE_HOLD)
                .text(old);
        }
    });

    Ok(())
}

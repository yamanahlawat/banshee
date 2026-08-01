use arboard::Clipboard;
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};
use std::{error::Error, thread, time::Duration};

// Let the clipboard settle before we paste.
const PASTE_SETTLE: Duration = Duration::from_millis(50);

// enigo pastes through X11, so wayland shells out instead. Checked at call
// time: a session manager may set neither variable until later.
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

    // Key::Unicode would resolve through Text Input Services, which is
    // main-thread-only and aborts when called from here.
    #[cfg(target_os = "macos")]
    let (modifier, paste_key) = (Key::Meta, Key::Other(0x09));
    #[cfg(not(target_os = "macos"))]
    let (modifier, paste_key) = (Key::Control, Key::Unicode('v'));

    thread::sleep(PASTE_SETTLE);

    enigo.key(modifier, Press)?;
    enigo.key(paste_key, Click)?;
    enigo.key(modifier, Release)?;

    Ok(())
}

// wtype first: ydotool needs its own daemon and uinput access.
#[cfg(all(unix, not(target_os = "macos")))]
pub const WAYLAND_TYPERS: [(&str, &[&str]); 2] = [("wtype", &["--"]), ("ydotool", &["type", "--"])];

#[cfg(all(unix, not(target_os = "macos")))]
fn type_text_wayland(text: &str) -> Result<(), Box<dyn Error>> {
    use std::process::Command;

    let mut attempts = Vec::new();
    for (binary, args) in WAYLAND_TYPERS {
        // `--` stops a leading dash being read as a flag
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

    // Never Ok here: the caller plays the ready cue on Ok.
    Err(format!(
        "could not type into the focused window on wayland ({}); \
         install wtype or ydotool. the transcription is still in `banshee history`",
        attempts.join("; ")
    )
    .into())
}

// Stage the text, then restore the old clipboard. Two impls: macOS and Windows
// hand contents to a system service, X11 and Wayland keep them in a live process.

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

    // Without `--`, wtype reads a leading dash as a flag and drops the text.
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
        // A scrubbed PATH resolves no typer, so this hits the both-missing path
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

    // The app fetches the selection asynchronously, well after the keystroke.
    const PASTE_WINDOW: Duration = Duration::from_secs(2);
    // How long the restored text stays served.
    const RESTORE_HOLD: Duration = Duration::from_secs(120);
    // wait_until never says why it returned; an early return means someone
    // else took the clipboard.
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
            // No longer ours to put back
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

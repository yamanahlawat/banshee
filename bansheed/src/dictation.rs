use arboard::Clipboard;
#[cfg(not(target_os = "macos"))]
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};
use std::{error::Error, thread, time::Duration};

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

    // Before the clipboard is staged, because a denied grant makes the paste a
    // silent no-op that nothing downstream can report
    #[cfg(target_os = "macos")]
    ensure_accessibility()?;

    let mut clipboard = Clipboard::new().map_err(|e| {
        format!(
            "Clipboard access failed! Please grant permission in System Settings: {}",
            e
        )
    })?;

    let old_clipboard = clipboard.get_text().ok();

    stage(clipboard, text, old_clipboard)?;

    thread::sleep(PASTE_SETTLE);

    send_paste()?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_accessibility() -> Result<(), Box<dyn Error>> {
    use crate::permissions::{ACCESSIBILITY, accessibility_granted};

    if !accessibility_granted() {
        return Err(format!(
            "Accessibility permission missing! To type, {}",
            ACCESSIBILITY.fix
        )
        .into());
    }
    Ok(())
}

// The modifier is a flag on the keystroke, never a key event of its own: a
// synthetic Command press desynchronises the system's modifier state, and the
// next press of that modifier then arrives as a release with no press.
#[cfg(target_os = "macos")]
fn send_paste() -> Result<(), Box<dyn Error>> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    // A macOS virtual keycode, not ASCII
    const KEY_V: u16 = 0x09;

    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| "Could not open an event source to paste with")?;
    for key_down in [true, false] {
        let event = CGEvent::new_keyboard_event(source.clone(), KEY_V, key_down)
            .map_err(|_| "Could not build the paste keystroke")?;
        event.set_flags(CGEventFlags::CGEventFlagCommand);
        event.post(CGEventTapLocation::HID);
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn send_paste() -> Result<(), Box<dyn Error>> {
    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| format!("Could not reach the display to type with! {e}"))?;
    enigo.key(Key::Control, Press)?;
    enigo.key(Key::Unicode('v'), Click)?;
    enigo.key(Key::Control, Release)?;
    Ok(())
}

// wtype first: ydotool needs its own daemon and uinput access.
#[cfg(all(unix, not(target_os = "macos")))]
pub const WAYLAND_TYPERS: [(&str, &[&str]); 2] = [("wtype", &["--"]), ("ydotool", &["type", "--"])];

// One search, so the spawn, the checklist and the blocker cannot disagree
// about what the daemon can run.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn resolve_wayland_typers(
    path: &std::ffi::OsStr,
) -> Vec<(std::path::PathBuf, &'static [&'static str])> {
    WAYLAND_TYPERS
        .into_iter()
        .filter_map(|(binary, args)| crate::status::resolve(binary, path).map(|bin| (bin, args)))
        .collect()
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn resolve_wayland_typer(
    path: &std::ffi::OsStr,
) -> Option<(std::path::PathBuf, &'static [&'static str])> {
    resolve_wayland_typers(path).into_iter().next()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn no_typer_error() -> Box<dyn Error> {
    // Never Ok here: the caller plays the ready cue on Ok.
    "could not type into the focused window on wayland (no typer on PATH); \
     install 'wtype' (or 'ydotool'). the transcription is still in `banshee history`"
        .into()
}

// GNOME/Mutter denies wtype the virtual-keyboard protocol it needs, so the
// first typer resolved is not always the one that can run.
#[cfg(all(unix, not(target_os = "macos")))]
fn type_with(
    resolved: &[(std::path::PathBuf, &'static [&'static str])],
    text: &str,
) -> Result<(), Box<dyn Error>> {
    use std::process::Command;

    let mut attempts = Vec::new();
    for (binary, args) in resolved {
        // `--` stops a leading dash being read as a flag
        match Command::new(binary).args(*args).arg(text).output() {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                attempts.push(format!(
                    "{} exited with {}: {}",
                    binary.display(),
                    output.status,
                    stderr.trim()
                ));
            }
            Err(e) => attempts.push(format!("{} failed to start: {e}", binary.display())),
        }
    }

    if attempts.is_empty() {
        return Err(no_typer_error());
    }
    Err(format!(
        "could not type into the focused window on wayland ({}); \
         install 'wtype' (or 'ydotool'). the transcription is still in `banshee history`",
        attempts.join("; ")
    )
    .into())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn type_text_wayland(text: &str) -> Result<(), Box<dyn Error>> {
    let path = crate::connect::resolved_path();
    type_with(&resolve_wayland_typers(&path), text)
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
    use std::ffi::OsStr;

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
    fn no_typer_installed_is_reported_with_both_tool_names() {
        // The resolver reads the login shell's PATH, not this process's, so
        // scrubbing an env var here would prove nothing.
        let message = no_typer_error().to_string();
        assert!(message.contains("wtype"), "unhelpful error: {message}");
        assert!(message.contains("ydotool"), "unhelpful error: {message}");
    }

    #[test]
    fn resolve_wayland_typer_finds_nothing_on_an_empty_path() {
        assert!(resolve_wayland_typer(OsStr::new("")).is_none());
    }

    #[test]
    fn resolve_wayland_typer_returns_the_absolute_path_and_its_args() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("banshee-typer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wtype = dir.join("wtype");
        std::fs::write(&wtype, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&wtype, std::fs::Permissions::from_mode(0o755)).unwrap();

        let found = resolve_wayland_typer(dir.as_os_str());

        let _ = std::fs::remove_dir_all(&dir);

        let (binary, args) = found.expect("a directory holding wtype must resolve it");
        assert_eq!(binary, wtype);
        assert_eq!(args, &["--"]);
    }

    // A fresh directory per script, so concurrent tests never share a path.
    fn write_script(name: &str, exit_code: u8, stderr: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "banshee-typer-{}-{name}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join(name);
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho '{stderr}' >&2\nexit {exit_code}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[test]
    fn a_typer_that_fails_at_runtime_falls_through_to_the_next() {
        let first = write_script("first", 1, "no virtual keyboard protocol");
        let second = write_script("second", 0, "");

        let resolved = [(first.clone(), WAYLAND_TYPERS[0].1), (second.clone(), WAYLAND_TYPERS[1].1)];
        let result = type_with(&resolved, "hello");

        let _ = std::fs::remove_dir_all(first.parent().unwrap());
        let _ = std::fs::remove_dir_all(second.parent().unwrap());

        assert!(
            result.is_ok(),
            "a working second typer must recover from a failing first one: {result:?}"
        );
    }

    #[test]
    fn an_error_when_every_typer_fails_names_every_attempt() {
        let first = write_script("alpha", 1, "boom-alpha");
        let second = write_script("beta", 1, "boom-beta");

        let resolved = [(first.clone(), WAYLAND_TYPERS[0].1), (second.clone(), WAYLAND_TYPERS[1].1)];
        let message = type_with(&resolved, "hello")
            .expect_err("two failing typers must not report success")
            .to_string();

        let _ = std::fs::remove_dir_all(first.parent().unwrap());
        let _ = std::fs::remove_dir_all(second.parent().unwrap());

        assert!(
            message.contains("boom-alpha") && message.contains("boom-beta"),
            "every attempt must be named: {message}"
        );
    }
}

// The caller's handle only ever read; ownership belongs to the thread below.
#[cfg(all(unix, not(target_os = "macos")))]
fn stage(_clipboard: Clipboard, text: &str, old: Option<String>) -> Result<(), Box<dyn Error>> {
    use arboard::SetExtLinux;
    use std::time::Instant;

    // The app fetches the selection asynchronously, well after the keystroke.
    const PASTE_WINDOW: Duration = Duration::from_secs(2);
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

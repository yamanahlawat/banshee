use arboard::Clipboard;
use banshee_common::error::BansheeError;
#[cfg(not(target_os = "macos"))]
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};
#[cfg(target_os = "macos")]
use objc2::{ClassType, msg_send, rc::Retained};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSArray, NSString};
use std::{thread, time::Duration};

use crate::frontmost::{self, App};

const PASTE_SETTLE: Duration = Duration::from_millis(50);

// enigo pastes through X11, so wayland shells out instead.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland")
        || std::env::var("WAYLAND_DISPLAY").is_ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivered {
    Typed,
    Copied,
}

pub fn deliver_text(text: &str, at_stop: Option<App>) -> Result<Delivered, BansheeError> {
    let moved = at_stop.is_some() && frontmost::moved(at_stop, frontmost::frontmost());
    deliver_with(text, moved, type_text, copy_text)
}

fn deliver_with(
    text: &str,
    moved: bool,
    type_words: impl FnOnce(&str) -> Result<(), BansheeError>,
    copy_words: impl FnOnce(&str) -> Result<(), BansheeError>,
) -> Result<Delivered, BansheeError> {
    if moved {
        match copy_words(text) {
            Ok(()) => {
                log::info!("The app changed after the recording stopped, so the words were copied");
                return Ok(Delivered::Copied);
            }
            Err(e) => log::warn!("Could not copy the dictation, so it is typed: {e}"),
        }
        return type_words(text).map(|()| Delivered::Typed);
    }
    let why = match type_words(text) {
        Ok(()) => return Ok(Delivered::Typed),
        Err(why) => why,
    };
    match copy_words(text) {
        Ok(()) => {
            log::warn!("Copied the dictation, since it could not be typed: {why}");
            Ok(Delivered::Copied)
        }
        Err(e) => {
            log::warn!("Could not copy the dictation either: {e}");
            Err(why)
        }
    }
}

fn type_text(text: &str) -> Result<(), BansheeError> {
    #[cfg(all(unix, not(target_os = "macos")))]
    if is_wayland() {
        return type_text_wayland(text);
    }

    // Before the clipboard is staged, because a denied grant makes the paste a
    // silent no-op that nothing downstream can report
    #[cfg(target_os = "macos")]
    ensure_accessibility()?;

    let mut clipboard = Clipboard::new().map_err(|e| {
        BansheeError::Other(format!(
            "Clipboard access failed! Please grant permission in System Settings: {e}"
        ))
    })?;

    let old_clipboard = clipboard.get_text().ok();

    let restore = stage(clipboard, text, old_clipboard)?;

    thread::sleep(PASTE_SETTLE);

    send_paste()?;
    restore.after_paste();

    Ok(())
}

// Plain, with no marker: a person pastes this copy on purpose.
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn copy_text(text: &str) -> Result<(), BansheeError> {
    Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text))
        .map_err(|e| BansheeError::Other(format!("could not copy the dictation: {e}")))
}

// X11 and Wayland serve the clipboard from the process that set it, so the
// thread holds the text until anything else takes the clipboard. Clipboard
// history keeps this copy, unlike a staged paste.
#[cfg(all(unix, not(target_os = "macos")))]
fn copy_text(text: &str) -> Result<(), BansheeError> {
    use arboard::SetExtLinux;

    let failed =
        |e: arboard::Error| BansheeError::Other(format!("could not copy the dictation: {e}"));
    let mut clipboard = Clipboard::new().map_err(failed)?;
    clipboard.set().text(text).map_err(failed)?;
    let text = text.to_string();
    thread::spawn(move || {
        let _ = clipboard.set().wait().text(text);
    });
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_accessibility() -> Result<(), BansheeError> {
    use crate::permissions::{ACCESSIBILITY, accessibility_granted};

    if !accessibility_granted() {
        return Err(BansheeError::Other(format!(
            "Accessibility permission missing! To type, {}",
            ACCESSIBILITY.fix
        )));
    }
    Ok(())
}

// The modifier is a flag on the keystroke, never a key event of its own: a
// synthetic Command press desynchronises the system's modifier state, and the
// next press of that modifier then arrives as a release with no press.
#[cfg(target_os = "macos")]
fn send_paste() -> Result<(), BansheeError> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    // A macOS virtual keycode, not ASCII
    const KEY_V: u16 = 0x09;

    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| BansheeError::Other("Could not open an event source to paste with".into()))?;
    for key_down in [true, false] {
        let event = CGEvent::new_keyboard_event(source.clone(), KEY_V, key_down)
            .map_err(|_| BansheeError::Other("Could not build the paste keystroke".into()))?;
        event.set_flags(CGEventFlags::CGEventFlagCommand);
        event.post(CGEventTapLocation::HID);
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn send_paste() -> Result<(), BansheeError> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| {
        BansheeError::Other(format!("Could not reach the display to type with! {e}"))
    })?;
    let key = |enigo: &mut Enigo, key, direction| {
        enigo
            .key(key, direction)
            .map_err(|e| BansheeError::Other(e.to_string()))
    };
    key(&mut enigo, Key::Control, Press)?;
    key(&mut enigo, Key::Unicode('v'), Click)?;
    key(&mut enigo, Key::Control, Release)?;
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
fn no_typer_error() -> BansheeError {
    // Never Ok here: the caller plays the ready cue on Ok.
    BansheeError::Other(
        "could not type into the focused window on wayland (no typer on PATH); \
         install 'wtype' (or 'ydotool'). the transcription is still in `banshee history`"
            .into(),
    )
}

// GNOME/Mutter denies wtype the virtual-keyboard protocol it needs, so the
// first typer resolved is not always the one that can run.
#[cfg(all(unix, not(target_os = "macos")))]
fn type_with(
    resolved: &[(std::path::PathBuf, &'static [&'static str])],
    text: &str,
) -> Result<(), BansheeError> {
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
    Err(BansheeError::Other(format!(
        "could not type into the focused window on wayland ({}); \
         install 'wtype' (or 'ydotool'). the transcription is still in `banshee history`",
        attempts.join("; ")
    )))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn type_text_wayland(text: &str) -> Result<(), BansheeError> {
    let path = crate::connect::resolved_path();
    type_with(&resolve_wayland_typers(&path), text)
}

// Stage the text, then put the old clipboard back. Two impls: macOS and
// Windows hand contents to a system service and restore once the paste is
// sent. X11 and Wayland keep them in a live process, which restores once the
// paste window ends, whether or not the paste went out.

// Restore only after the app has consumed the paste.
#[cfg(not(all(unix, not(target_os = "macos"))))]
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(500);

#[cfg(not(all(unix, not(target_os = "macos"))))]
struct Restore {
    old: Option<String>,
    dictated: String,
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
impl Restore {
    // Off the hot path so the ready cue fires without waiting.
    fn after_paste(self) {
        let Some(old) = self.old else {
            return;
        };
        let dictated = self.dictated;
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
}

#[cfg(windows)]
fn stage(
    mut clipboard: Clipboard,
    text: &str,
    old: Option<String>,
) -> Result<Restore, BansheeError> {
    clipboard
        .set_text(text.to_string())
        .map_err(|e| BansheeError::Other(e.to_string()))?;
    Ok(Restore {
        old,
        dictated: text.to_string(),
    })
}

#[cfg(target_os = "macos")]
const TRANSIENT_TYPE: &str = "org.nspasteboard.TransientType";
#[cfg(target_os = "macos")]
const AUTO_GENERATED_TYPE: &str = "org.nspasteboard.AutoGeneratedType";

// Every type is declared in one change, before any data lands, so a clipboard
// manager that polls mid-write never sees the text without its markers.
#[cfg(target_os = "macos")]
fn write_transient(board: &NSPasteboard, text: &str) -> Result<(), BansheeError> {
    // SAFETY: a framework constant, valid for the life of the process.
    let string_type = unsafe { NSPasteboardTypeString };
    let transient = NSString::from_str(TRANSIENT_TYPE);
    let auto_generated = NSString::from_str(AUTO_GENERATED_TYPE);
    let types = NSArray::from_slice(&[string_type, &*transient, &*auto_generated]);
    // SAFETY: a nil owner is allowed, and the types are NSStrings.
    unsafe { board.declareTypes_owner(&types, None) };

    let empty = NSString::from_str("");
    let filled = board.setString_forType(&NSString::from_str(text), string_type)
        && board.setString_forType(&empty, &transient)
        && board.setString_forType(&empty, &auto_generated);
    if filled {
        Ok(())
    } else {
        Err(BansheeError::Other(
            "the pasteboard refused the dictated text".into(),
        ))
    }
}

#[cfg(target_os = "macos")]
fn stage(_clipboard: Clipboard, text: &str, old: Option<String>) -> Result<Restore, BansheeError> {
    // generalPasteboard can be nil under launchd, which the binding does not allow for.
    let board: Option<Retained<NSPasteboard>> =
        // SAFETY: a class method with no arguments, returning an object or nil.
        unsafe { msg_send![NSPasteboard::class(), generalPasteboard] };
    let board =
        board.ok_or_else(|| BansheeError::Other("the general pasteboard is unavailable".into()))?;
    write_transient(&board, text)?;
    Ok(Restore {
        old,
        dictated: text.to_string(),
    })
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn typed_words_are_never_copied() {
        let copied = Cell::new(false);
        let delivered = deliver_with(
            "hi",
            false,
            |_| Ok(()),
            |_| {
                copied.set(true);
                Ok(())
            },
        );
        assert_eq!(delivered.unwrap(), Delivered::Typed);
        assert!(!copied.get());
    }

    #[test]
    fn words_that_cannot_be_typed_are_copied_whole() {
        let copied = RefCell::new(String::new());
        let delivered = deliver_with(
            "hi there",
            false,
            |_| Err(BansheeError::Other("no accessibility grant".into())),
            |words| {
                copied.replace(words.to_string());
                Ok(())
            },
        );
        assert_eq!(delivered.unwrap(), Delivered::Copied);
        assert_eq!(copied.into_inner(), "hi there");
    }

    #[test]
    fn a_copy_that_fails_too_reports_why_typing_failed() {
        let delivered = deliver_with(
            "hi",
            false,
            |_| Err(BansheeError::Other("no typer on PATH".into())),
            |_| Err(BansheeError::Other("no clipboard".into())),
        );
        let message = delivered.unwrap_err().to_string();
        assert!(message.contains("no typer on PATH"), "{message}");
    }

    #[test]
    fn a_changed_app_copies_and_never_types() {
        let typed = Cell::new(false);
        let delivered = deliver_with(
            "hi",
            true,
            |_| {
                typed.set(true);
                Ok(())
            },
            |_| Ok(()),
        );
        assert_eq!(delivered.unwrap(), Delivered::Copied);
        assert!(!typed.get());
    }

    #[test]
    fn a_changed_app_whose_copy_fails_types_as_today() {
        let delivered = deliver_with(
            "hi",
            true,
            |_| Ok(()),
            |_| Err(BansheeError::Other("no clipboard".into())),
        );
        assert_eq!(delivered.unwrap(), Delivered::Typed);
    }

    #[test]
    fn a_changed_app_with_no_clipboard_and_no_typer_fails() {
        let copies = Cell::new(0);
        let delivered = deliver_with(
            "hi",
            true,
            |_| Err(BansheeError::Other("no typer".into())),
            |_| {
                copies.set(copies.get() + 1);
                Err(BansheeError::Other("no clipboard".into()))
            },
        );
        assert!(delivered.is_err());
        assert_eq!(copies.get(), 1);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;

    #[test]
    fn a_staged_dictation_is_transient_and_auto_generated_and_never_concealed() {
        let board = NSPasteboard::pasteboardWithUniqueName();
        write_transient(&board, "hello there").unwrap();
        let types: Vec<String> = board
            .types()
            .unwrap()
            .iter()
            .map(|t| t.to_string())
            .collect();
        assert!(
            types.iter().any(|t| t == "org.nspasteboard.TransientType"),
            "{types:?}"
        );
        assert!(
            types
                .iter()
                .any(|t| t == "org.nspasteboard.AutoGeneratedType"),
            "{types:?}"
        );
        assert!(
            !types.iter().any(|t| t == "org.nspasteboard.ConcealedType"),
            "{types:?}"
        );
        // SAFETY: a framework constant, valid for the life of the process.
        let string_type = unsafe { NSPasteboardTypeString };
        let text = board.stringForType(string_type).unwrap();
        assert_eq!(text.to_string(), "hello there");
        // SAFETY: releaseGlobally takes no arguments and returns nothing.
        let _: () = unsafe { msg_send![&*board, releaseGlobally] };
    }
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
        let dir = std::env::temp_dir().join(format!("banshee-typer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wtype = dir.join("wtype");
        crate::test_support::write_executable(&wtype, "#!/bin/sh\n");

        let found = resolve_wayland_typer(dir.as_os_str());

        let _ = std::fs::remove_dir_all(&dir);

        let (binary, args) = found.expect("a directory holding wtype must resolve it");
        assert_eq!(binary, wtype);
        assert_eq!(args, &["--"]);
    }

    // A fresh directory per script, so concurrent tests never share a path.
    fn write_script(name: &str, exit_code: u8, stderr: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "banshee-typer-{}-{name}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join(name);
        crate::test_support::write_executable(
            &script,
            &format!("#!/bin/sh\necho '{stderr}' >&2\nexit {exit_code}\n"),
        );
        script
    }

    #[test]
    fn a_typer_that_fails_at_runtime_falls_through_to_the_next() {
        let first = write_script("first", 1, "no virtual keyboard protocol");
        let second = write_script("second", 0, "");

        let resolved = [
            (first.clone(), WAYLAND_TYPERS[0].1),
            (second.clone(), WAYLAND_TYPERS[1].1),
        ];
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

        let resolved = [
            (first.clone(), WAYLAND_TYPERS[0].1),
            (second.clone(), WAYLAND_TYPERS[1].1),
        ];
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

// Nothing to do after the paste: the hold thread in `stage` puts the old text
// back, and a copy's write ends that thread early.
#[cfg(all(unix, not(target_os = "macos")))]
struct Restore;

#[cfg(all(unix, not(target_os = "macos")))]
impl Restore {
    fn after_paste(self) {}
}

// The caller's handle only ever read; ownership belongs to the thread below.
#[cfg(all(unix, not(target_os = "macos")))]
fn stage(_clipboard: Clipboard, text: &str, old: Option<String>) -> Result<Restore, BansheeError> {
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

    Ok(Restore)
}

use arboard::Clipboard;
use banshee_common::error::BansheeError;
#[cfg(not(target_os = "macos"))]
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};
#[cfg(target_os = "macos")]
use objc2::{ClassType, msg_send, rc::Retained, runtime::ProtocolObject};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeString, NSPasteboardWriting};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSArray, NSData, NSString};
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

    let restore = stage(text)?;

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

// Stage the text, then put the old clipboard back. macOS restores the saved
// items and Windows restores the saved text, both once the paste is sent.
// X11 and Wayland keep the contents in a live process, which restores once
// the paste window ends, whether or not the paste went out.

// Restore only after the app has consumed the paste.
#[cfg(not(all(unix, not(target_os = "macos"))))]
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(500);

#[cfg(windows)]
struct Restore {
    old: Option<String>,
    dictated: String,
}

#[cfg(windows)]
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

#[cfg(not(target_os = "macos"))]
fn clipboard_and_old_text() -> Result<(Clipboard, Option<String>), BansheeError> {
    let mut clipboard = Clipboard::new()
        .map_err(|e| BansheeError::Other(format!("could not open the clipboard: {e}")))?;
    let old = clipboard.get_text().ok();
    Ok((clipboard, old))
}

#[cfg(windows)]
fn stage(text: &str) -> Result<Restore, BansheeError> {
    let (mut clipboard, old) = clipboard_and_old_text()?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| BansheeError::Other(e.to_string()))?;
    Ok(Restore {
        old,
        dictated: text.to_string(),
    })
}

// Items from pasteboardItems go stale at the next change, so the bytes are
// copied out here and fresh items are built from them on restore.
#[cfg(target_os = "macos")]
type Item = Vec<(String, Vec<u8>)>;

#[cfg(target_os = "macos")]
struct Saved(Vec<Item>);

// None means the pasteboard did not answer, or held items and gave no data for
// any of them. Only a board with no items at all is empty.
#[cfg(target_os = "macos")]
fn save(board: &NSPasteboard) -> Option<Saved> {
    let started = std::time::Instant::now();
    let items = board.pasteboardItems()?;
    let mut skipped = 0usize;
    let mut saved = Vec::new();
    for item in items.iter() {
        let mut kept = Vec::new();
        for kind in item.types().iter() {
            match item.dataForType(&kind) {
                Some(data) => kept.push((kind.to_string(), data.to_vec())),
                None => skipped += 1,
            }
        }
        if !kept.is_empty() {
            saved.push(kept);
        }
    }
    if saved.is_empty() && skipped > 0 {
        return None;
    }
    log::debug!(
        "Saved {} clipboard items in {:?}, skipped {skipped} types",
        saved.len(),
        started.elapsed()
    );
    Some(Saved(saved))
}

// One writeObjects lands every item and type together.
#[cfg(target_os = "macos")]
fn write_items(board: &NSPasteboard, items: Vec<Item>) -> bool {
    let fresh: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = items
        .into_iter()
        .map(|kinds| {
            let item = NSPasteboardItem::new();
            for (kind, bytes) in kinds {
                item.setData_forType(&NSData::from_vec(bytes), &NSString::from_str(&kind));
            }
            ProtocolObject::from_retained(item)
        })
        .collect();
    board.writeObjects(&NSArray::from_retained_slice(&fresh))
}

#[cfg(target_os = "macos")]
const TRANSIENT_TYPE: &str = "org.nspasteboard.TransientType";
#[cfg(target_os = "macos")]
const AUTO_GENERATED_TYPE: &str = "org.nspasteboard.AutoGeneratedType";

// Every type is declared in one change, before any data lands, so a clipboard
// manager that polls mid-write never sees the text without its markers.
#[cfg(target_os = "macos")]
fn write_transient(board: &NSPasteboard, text: &str) -> Result<isize, BansheeError> {
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
        Ok(board.changeCount())
    } else {
        Err(BansheeError::Other(
            "the pasteboard refused the dictated text".into(),
        ))
    }
}

#[cfg(target_os = "macos")]
struct Restore {
    saved: Option<Saved>,
    staged: isize,
}

#[cfg(target_os = "macos")]
#[derive(Debug, PartialEq, Eq)]
enum Restored {
    Put,
    Cleared,
    Skipped,
    Refused,
}

// A changed count means the user copied something, even the same words, and
// that copy wins.
#[cfg(target_os = "macos")]
fn restore_onto(board: &NSPasteboard, restore: Restore) -> Restored {
    let Some(Saved(items)) = restore.saved else {
        return Restored::Skipped;
    };
    if board.changeCount() != restore.staged {
        return Restored::Skipped;
    }
    put_back(board, items)
}

#[cfg(target_os = "macos")]
fn put_back(board: &NSPasteboard, items: Vec<Item>) -> Restored {
    board.clearContents();
    if items.is_empty() {
        return Restored::Cleared;
    }
    if write_items(board, items) {
        Restored::Put
    } else {
        log::warn!("The pasteboard refused the saved clipboard items");
        Restored::Refused
    }
}

#[cfg(target_os = "macos")]
fn stage_on(board: &NSPasteboard, text: &str) -> Result<Restore, BansheeError> {
    let saved = save(board);
    match write_transient(board, text) {
        Ok(staged) => Ok(Restore { saved, staged }),
        Err(e) => {
            if let Some(Saved(items)) = saved {
                put_back(board, items);
            }
            Err(e)
        }
    }
}

#[cfg(target_os = "macos")]
fn general_board() -> Result<Retained<NSPasteboard>, BansheeError> {
    // generalPasteboard can be nil under launchd, which the binding does not allow for.
    let board: Option<Retained<NSPasteboard>> =
        // SAFETY: a class method with no arguments, returning an object or nil.
        unsafe { msg_send![NSPasteboard::class(), generalPasteboard] };
    board.ok_or_else(|| BansheeError::Other("the general pasteboard is unavailable".into()))
}

#[cfg(target_os = "macos")]
fn stage(text: &str) -> Result<Restore, BansheeError> {
    // The consumer thread runs no run loop to drain what the reads autorelease.
    objc2::rc::autoreleasepool(|_| {
        let board = general_board()?;
        stage_on(&board, text)
    })
}

#[cfg(target_os = "macos")]
impl Restore {
    // Off the hot path so the ready cue fires without waiting.
    fn after_paste(self) {
        thread::spawn(move || {
            thread::sleep(CLIPBOARD_RESTORE_DELAY);
            objc2::rc::autoreleasepool(|_| {
                if let Ok(board) = general_board() {
                    restore_onto(&board, self);
                }
            });
        });
    }
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
    use objc2::AnyThread;
    use objc2_app_kit::NSPasteboardItemDataProvider;
    use objc2_foundation::NSObjectProtocol;

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
        assert_eq!(text_of(&board).as_deref(), Some("hello there"));
        release(&board);
    }

    const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";

    fn owned(items: &[&[(&str, &[u8])]]) -> Vec<Item> {
        items
            .iter()
            .map(|types| {
                types
                    .iter()
                    .map(|(t, b)| (t.to_string(), b.to_vec()))
                    .collect()
            })
            .collect()
    }

    fn board_with(items: &[&[(&str, &[u8])]]) -> Retained<NSPasteboard> {
        let board = NSPasteboard::pasteboardWithUniqueName();
        board.clearContents();
        assert!(write_items(&board, owned(items)));
        board
    }

    fn items_of(board: &NSPasteboard) -> Vec<Item> {
        let mut items = save(board).expect("pasteboardItems answered nil").0;
        for item in &mut items {
            item.sort();
        }
        items
    }

    fn release(board: &NSPasteboard) {
        // SAFETY: releaseGlobally takes no arguments and returns nothing.
        let _: () = unsafe { msg_send![board, releaseGlobally] };
    }

    fn text_of(board: &NSPasteboard) -> Option<String> {
        // SAFETY: a framework constant, valid for the life of the process.
        let string_type = unsafe { NSPasteboardTypeString };
        board.stringForType(string_type).map(|s| s.to_string())
    }

    fn copy_as_user(board: &NSPasteboard, text: &str) {
        board.clearContents();
        // SAFETY: a framework constant, valid for the life of the process.
        let string_type = unsafe { NSPasteboardTypeString };
        assert!(board.setString_forType(&NSString::from_str(text), string_type));
    }

    #[test]
    fn an_image_and_its_text_come_back_byte_for_byte() {
        let before: &[&[(&str, &[u8])]] = &[
            &[
                ("public.png", b"\x89PNG not decoded"),
                ("public.utf8-plain-text", b"caption"),
            ],
            &[("public.file-url", b"file:///tmp/a.txt")],
        ];
        let board = board_with(before);
        let expected = items_of(&board);
        let mut literal = owned(before);
        for item in &mut literal {
            item.sort();
        }
        assert_eq!(expected, literal);

        let restore = stage_on(&board, "dictated words").unwrap();
        assert_eq!(text_of(&board).as_deref(), Some("dictated words"));
        assert_eq!(restore_onto(&board, restore), Restored::Put);
        assert_eq!(items_of(&board), expected);
        release(&board);
    }

    #[test]
    fn a_copy_during_the_window_wins() {
        let board = board_with(&[&[("public.utf8-plain-text", b"old")]]);
        let restore = stage_on(&board, "dictated words").unwrap();
        copy_as_user(&board, "mine");
        assert_eq!(restore_onto(&board, restore), Restored::Skipped);
        assert_eq!(text_of(&board).as_deref(), Some("mine"));
        release(&board);
    }

    #[test]
    fn the_same_words_copied_again_still_win() {
        let board = board_with(&[&[("public.utf8-plain-text", b"old")]]);
        let restore = stage_on(&board, "same").unwrap();
        copy_as_user(&board, "same");
        assert_eq!(restore_onto(&board, restore), Restored::Skipped);
        assert_eq!(text_of(&board).as_deref(), Some("same"));
        release(&board);
    }

    #[test]
    fn a_concealed_copy_comes_back_with_its_marker() {
        let board = board_with(&[&[
            ("public.utf8-plain-text", b"hunter2"),
            (CONCEALED_TYPE, b""),
        ]]);
        let expected = items_of(&board);
        assert!(expected[0].iter().any(|(t, _)| t == CONCEALED_TYPE));
        let restore = stage_on(&board, "dictated words").unwrap();
        assert_eq!(restore_onto(&board, restore), Restored::Put);
        assert_eq!(items_of(&board), expected);
        release(&board);
    }

    #[test]
    fn an_empty_board_is_empty_after_the_paste() {
        let board = NSPasteboard::pasteboardWithUniqueName();
        board.clearContents();
        let restore = stage_on(&board, "dictated words").unwrap();
        assert_eq!(restore_onto(&board, restore), Restored::Cleared);
        assert!(board.pasteboardItems().unwrap().is_empty());
        release(&board);
    }

    objc2::define_class!(
        #[unsafe(super(objc2_foundation::NSObject))]
        #[name = "BansheeTestPngProvider"]
        struct PngProvider;

        unsafe impl NSObjectProtocol for PngProvider {}

        unsafe impl NSPasteboardItemDataProvider for PngProvider {
            #[unsafe(method(pasteboard:item:provideDataForType:))]
            fn provide(
                &self,
                _board: Option<&NSPasteboard>,
                item: &NSPasteboardItem,
                kind: &NSString,
            ) {
                item.setData_forType(&NSData::with_bytes(b"promised png"), kind);
            }
        }
    );

    #[test]
    fn a_promised_type_is_resolved_at_save() {
        let provider: Retained<PngProvider> =
            // SAFETY: init on a fresh allocation of a class with no ivars.
            unsafe { msg_send![PngProvider::alloc(), init] };
        let board = board_with_promise(ProtocolObject::from_ref(&*provider), None);
        let saved = save(&board).unwrap().0;
        assert_eq!(
            saved,
            vec![vec![("public.png".to_string(), b"promised png".to_vec())]]
        );
        release(&board);
    }

    objc2::define_class!(
        #[unsafe(super(objc2_foundation::NSObject))]
        #[name = "BansheeTestEmptyProvider"]
        struct EmptyProvider;

        unsafe impl NSObjectProtocol for EmptyProvider {}

        unsafe impl NSPasteboardItemDataProvider for EmptyProvider {
            #[unsafe(method(pasteboard:item:provideDataForType:))]
            fn provide(
                &self,
                _board: Option<&NSPasteboard>,
                _item: &NSPasteboardItem,
                _kind: &NSString,
            ) {
            }
        }
    );

    fn board_with_promise(
        provider: &ProtocolObject<dyn NSPasteboardItemDataProvider>,
        plain: Option<(&str, &[u8])>,
    ) -> Retained<NSPasteboard> {
        let board = NSPasteboard::pasteboardWithUniqueName();
        board.clearContents();
        let item = NSPasteboardItem::new();
        if let Some((kind, bytes)) = plain {
            assert!(item.setData_forType(&NSData::with_bytes(bytes), &NSString::from_str(kind)));
        }
        let kinds = NSArray::from_retained_slice(&[NSString::from_str("public.png")]);
        assert!(item.setDataProvider_forTypes(provider, &kinds));
        assert!(board.writeObjects(&NSArray::from_retained_slice(&[
            ProtocolObject::from_retained(item)
        ])));
        board
    }

    fn board_with_empty_promise(plain: Option<(&str, &[u8])>) -> Retained<NSPasteboard> {
        let provider: Retained<EmptyProvider> =
            // SAFETY: init on a fresh allocation of a class with no ivars.
            unsafe { msg_send![EmptyProvider::alloc(), init] };
        board_with_promise(ProtocolObject::from_ref(&*provider), plain)
    }

    #[test]
    fn a_type_with_no_data_is_dropped_and_the_rest_come_back() {
        let board = board_with_empty_promise(Some(("public.utf8-plain-text", b"caption")));
        let saved = save(&board).unwrap().0;
        assert_eq!(
            saved,
            vec![vec![(
                "public.utf8-plain-text".to_string(),
                b"caption".to_vec()
            )]]
        );
        release(&board);
    }

    #[test]
    fn an_item_whose_only_type_gives_no_data_leaves_the_save_unknown() {
        let board = board_with_empty_promise(None);
        assert!(save(&board).is_none());
        release(&board);
    }

    #[test]
    fn an_unknown_save_never_touches_the_board() {
        let board = board_with(&[&[("public.utf8-plain-text", b"old")]]);
        write_transient(&board, "dictated words").unwrap();
        let restore = Restore {
            saved: None,
            staged: board.changeCount(),
        };
        assert_eq!(restore_onto(&board, restore), Restored::Skipped);
        assert_eq!(text_of(&board).as_deref(), Some("dictated words"));
        release(&board);
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

// The thread serves the selection from its own handle; this one only reads
// the old text.
#[cfg(all(unix, not(target_os = "macos")))]
fn stage(text: &str) -> Result<Restore, BansheeError> {
    use arboard::SetExtLinux;
    use std::time::Instant;

    // The app fetches the selection asynchronously, well after the keystroke.
    const PASTE_WINDOW: Duration = Duration::from_secs(2);
    const RESTORE_HOLD: Duration = Duration::from_secs(120);
    // wait_until never says why it returned; an early return means someone
    // else took the clipboard.
    const SLACK: Duration = Duration::from_millis(250);

    let (_clipboard, old) = clipboard_and_old_text()?;
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

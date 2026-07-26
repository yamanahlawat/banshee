use arboard::Clipboard;
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};
use std::{error::Error, thread, time::Duration};

// Let the clipboard settle before we paste. On the ownership-based platforms
// this is also what gives the owner thread time to publish.
const PASTE_SETTLE: Duration = Duration::from_millis(50);

pub fn type_text(text: &str) -> Result<(), Box<dyn Error>> {
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

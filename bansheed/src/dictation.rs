use arboard::Clipboard;
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};
use std::{error::Error, thread::sleep, time::Duration};

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

    let old_clipboard_content = clipboard.get_text().ok();

    clipboard.set_text(text.to_string())?;

    let modifier = if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    };

    sleep(Duration::from_millis(50));

    enigo.key(modifier, Press)?;
    enigo.key(Key::Unicode('v'), Click)?;
    enigo.key(modifier, Release)?;

    sleep(Duration::from_millis(50));

    if let Some(content) = old_clipboard_content {
        clipboard.set_text(content)?;
    }

    Ok(())
}

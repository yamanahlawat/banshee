use enigo::{Enigo, Keyboard, Settings};
use std::error::Error;

pub fn type_text(text: &str) -> Result<(), Box<dyn Error>> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| {
        format!(
            "Accessibility permissions missing! Please grant permission in System Settings: {}",
            e
        )
    })?;
    enigo.text(text)?;

    Ok(())
}

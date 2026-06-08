use std::sync::Mutex;

// A global mailbox that can hold a String
pub static TRANSCRIPTION_MAILBOX: Mutex<Option<String>> = Mutex::new(None);

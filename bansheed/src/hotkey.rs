use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use tokio::sync::mpsc;

use rdev::{EventType, Key, listen};

pub static IS_RECORDING: AtomicBool = AtomicBool::new(false);

pub fn hotkey_listener() {
    // Create a mail tube to send messages from rdev to tokio
    let (sender, mut receiver) = mpsc::channel::<()>(32);

    // Spawn a heavy thread for the global hotkey listener
    thread::spawn(|| {
        if let Err(error) = listen(move |event| {
            match event.event_type {
                EventType::KeyPress(Key::F5) => {
                    println!("F5 hotkey detected! Waking up tokio runtime...");
                    IS_RECORDING.store(true, Ordering::SeqCst);
                }
                EventType::KeyRelease(Key::F5) => {
                    println!("F5 hotkey released");
                    IS_RECORDING.store(false, Ordering::SeqCst);
                    let _ = sender.blocking_send(());
                }
                _ => (), // Ignore mouse movements
            }
        }) {
            println!("Error: {:?}", error);
        }
    });

    // Spawn an async task that waits for the mail
    tokio::spawn(async move {
        while let Some(_) = receiver.recv().await {
            println!("Tokio received the hotkey stop signal!");
        }
    });
}

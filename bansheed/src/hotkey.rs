use ringbuf::traits::Consumer;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use tokio::sync::mpsc;

use rdev::{EventType, Key, listen};

use crate::audio::utils::resample_audio;
use crate::dictation::type_text;
use crate::speech_to_text::mailbox::TRANSCRIPTION_MAILBOX;
use crate::speech_to_text::whisper::WhisperEngine;

pub static IS_RECORDING: AtomicBool = AtomicBool::new(false);

pub enum HotKeyAction {
    Mailbox,
    Dictate,
}

pub fn hotkey_listener(
    mut consumer: impl Consumer<Item = f32> + Send + 'static,
    speech_to_text_engine: WhisperEngine,
    sample_rate: u32,
) {
    // Create a mail tube to send messages from rdev to tokio
    let (sender, mut receiver) = mpsc::channel::<HotKeyAction>(32);

    // Spawn a heavy thread for the global hotkey listener
    thread::spawn(|| {
        // Track the state of the shift key
        let mut shift_key_pressed = false;

        // Track what action are we recording for
        let mut current_action = HotKeyAction::Mailbox;

        if let Err(error) = listen(move |event| {
            match event.event_type {
                // Track when shift goes down or up
                EventType::KeyPress(Key::ShiftLeft) | EventType::KeyPress(Key::ShiftRight) => {
                    shift_key_pressed = true;
                }
                EventType::KeyRelease(Key::ShiftLeft) | EventType::KeyRelease(Key::ShiftRight) => {
                    shift_key_pressed = false;
                }

                // Track F5
                EventType::KeyPress(Key::F5) => {
                    if !IS_RECORDING.swap(true, Ordering::Relaxed) {
                        current_action = if shift_key_pressed {
                            HotKeyAction::Dictate
                        } else {
                            HotKeyAction::Mailbox
                        };
                        println!("F5 hotkey detected! Recording Audio...");
                    }
                }
                EventType::KeyRelease(Key::F5) => {
                    println!("F5 hotkey released");
                    IS_RECORDING.store(false, Ordering::Relaxed);
                    // Send whichever action we locked in when the key is pressed
                    let action_to_send = match current_action {
                        HotKeyAction::Mailbox => HotKeyAction::Mailbox,
                        HotKeyAction::Dictate => HotKeyAction::Dictate,
                    };
                    let _ = sender.blocking_send(action_to_send);
                }
                _ => (), // Ignore mouse movements
            }
        }) {
            println!("Error: {:?}", error);
        }
    });

    // Spawn an async task that waits for the mail
    tokio::spawn(async move {
        while let Some(action) = receiver.recv().await {
            println!("Tokio received the stop signal!");

            // pull all the floats out of the ring buffer into a standard Vec
            let mut audio_data = Vec::new();
            audio_data.extend(consumer.pop_iter());

            println!("Downsampling audio from {sample_rate} Hz to 16000 Hz...");

            // Downsample the audio by taking every nth sample
            let final_data = resample_audio(&audio_data, sample_rate, 16000);

            // Resample the audio data if necessary
            println!("Transcribing...");
            match speech_to_text_engine.transcribe(&final_data) {
                Ok(transcription) => {
                    println!("Transcription: {transcription}");
                    match action {
                        HotKeyAction::Mailbox => {
                            if let Ok(mut mailbox) = TRANSCRIPTION_MAILBOX.lock() {
                                *mailbox = Some(transcription);
                            }
                        }
                        HotKeyAction::Dictate => {
                            println!("Dictating: {}", transcription);
                            if let Err(e) = type_text(&transcription) {
                                eprintln!("Failed to type text: {:?}", e);
                            }
                        }
                    }
                }
                Err(error) => eprintln!("Transcription failed: {error}"),
            }
        }
    });
}

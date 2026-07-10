use ringbuf::traits::Consumer;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Instant;

use rdev::{EventType, Key, listen};

use crate::audio::cues::Cue;
use crate::audio::utils::resample_audio;
use crate::dictation::type_text;
use crate::speech_to_text::vad::VADEngine;
use crate::speech_to_text::whisper::WhisperEngine;
use crate::state::DaemonState;

#[derive(Clone, Copy)]
pub enum HotKeyAction {
    Mailbox,
    Dictate,
}

const TARGET_SAMPLE_RATE: u32 = 16000;

pub fn hotkey_listener(
    mut consumer: impl Consumer<Item = f32> + Send + 'static,
    speech_to_text_engine: WhisperEngine,
    mut vad_engine: VADEngine,
    sample_rate: u32,
    daemon_state: Arc<DaemonState>,
    cues: mpsc::Sender<Cue>,
) {
    // Create a channel to communicate between the hotkey listener and the audio processing thread
    let (sender, receiver) = mpsc::channel();

    let task_state = Arc::clone(&daemon_state);
    let hotkey_cues = cues.clone();

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
                    if !daemon_state.is_recording() {
                        current_action = if shift_key_pressed {
                            HotKeyAction::Dictate
                        } else {
                            HotKeyAction::Mailbox
                        };
                        daemon_state.set_recording(true);
                        let _ = hotkey_cues.send(Cue::RecordStart);
                        println!("F5 hotkey detected! Recording Audio...");
                    }
                }
                EventType::KeyRelease(Key::F5) => {
                    println!("F5 hotkey released");
                    daemon_state.set_recording(false);
                    let _ = hotkey_cues.send(Cue::RecordStop);
                    // Send whichever action we locked in when the key is pressed
                    let _ = sender.send(current_action);
                }
                _ => (), // Ignore mouse movements
            }
        }) {
            println!("Error: {:?}", error);
        }
    });

    // Spawn a thread to handle the audio processing and transcription
    thread::spawn(move || {
        while let Ok(action) = receiver.recv() {
            // pull all the floats out of the ring buffer into a standard Vec
            let mut audio_data = Vec::new();
            audio_data.extend(consumer.pop_iter());

            println!("Downsampling audio from {sample_rate} Hz to {TARGET_SAMPLE_RATE} Hz...");

            // Downsample the audio by taking every nth sample
            let final_data = match resample_audio(&audio_data, sample_rate, TARGET_SAMPLE_RATE) {
                Ok(data) => data,
                Err(e) => {
                    eprintln!("Error: {e}");
                    let _ = cues.send(Cue::Error);
                    continue;
                }
            };

            let max_amplitude = final_data.iter().cloned().fold(0.0f32, f32::max);
            let min_amplitude = final_data.iter().cloned().fold(0.0f32, f32::min);
            println!("Audio range: [{min_amplitude}, {max_amplitude}]");

            println!("Total audio samples after resampling: {}", final_data.len());

            // Chunk the audio into 512 sample frames and run VAD on each frame until we find speech or run out of audio
            let mut speech_chunks = 0;
            let mut total_chunks = 0;

            // Reset the VAD state before processing new audio
            vad_engine.reset_state();
            let vad_threshold = task_state.vad_threshold();

            for chunk in final_data.chunks(512) {
                if chunk.len() < 512 {
                    continue; // Ignore the last chunk if it's smaller than 512 samples
                }
                match vad_engine.check_speech(chunk, TARGET_SAMPLE_RATE) {
                    Ok(probability) => {
                        if probability > vad_threshold {
                            speech_chunks += 1;
                        }
                    }
                    Err(e) => {
                        eprintln!("VAD error: {e}");
                        continue;
                    }
                };
                total_chunks += 1;
            }

            println!("VAD detected speech in {speech_chunks} out of {total_chunks} chunks.");

            let speech_ratio = if total_chunks > 0 {
                speech_chunks as f32 / total_chunks as f32
            } else {
                0.0
            };

            if speech_ratio < 0.1 {
                println!(
                    "Only detected speech in {:.2}% of the audio. Skipping transcription.",
                    speech_ratio * 100.0
                );
                let _ = cues.send(Cue::Error);
                continue;
            }

            if speech_chunks < 2 {
                println!("No speech detected in the audio. Skipping transcription.");
                let _ = cues.send(Cue::Error);
                continue;
            }

            println!("Transcribing...");
            let transcribe_started = Instant::now();
            match speech_to_text_engine.transcribe(&final_data) {
                Ok(transcription) => {
                    println!(
                        "Transcribed {:.1}s of audio in {:.2}s",
                        final_data.len() as f32 / TARGET_SAMPLE_RATE as f32,
                        transcribe_started.elapsed().as_secs_f32()
                    );
                    println!("Transcription: {transcription}");

                    // Whisper can return nothing for noise; skip before it reaches the ring or clipboard
                    if transcription.is_empty() {
                        println!("Empty transcription. Skipping.");
                        let _ = cues.send(Cue::Error);
                        continue;
                    }

                    let _ = cues.send(Cue::Ready);

                    if let Some(db) = task_state.db_connection()
                        && let Ok(connection) = db.lock()
                        && let Err(e) = crate::history::TranscriptionHistory::insert(
                            &connection,
                            &transcription,
                        )
                    {
                        eprintln!("Failed to insert transcription into database: {e}");
                    }

                    match action {
                        HotKeyAction::Mailbox => {
                            task_state.push_transcription(transcription);
                        }
                        HotKeyAction::Dictate => {
                            println!("Dictating: {}", transcription);
                            if let Err(e) = type_text(&transcription) {
                                eprintln!("Failed to type text: {:?}", e);
                            }
                        }
                    }
                }
                Err(error) => {
                    eprintln!("Transcription failed: {error}");
                    let _ = cues.send(Cue::Error);
                }
            }
        }
    });
}

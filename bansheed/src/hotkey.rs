use ringbuf::traits::Consumer;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rdev::{EventType, Key, listen};

use crate::audio::cues::Cue;
use crate::audio::utils::{StreamingResampler, resample_audio};
use crate::config::BargeInMode;
use crate::dictation::type_text;
use crate::speech_to_text::vad::VADEngine;
use crate::speech_to_text::whisper::WhisperEngine;
use crate::state::{AskCommand, ConsumerCommand, DaemonState, HotKeyAction, RecordingMode};

const TARGET_SAMPLE_RATE: u32 = 16000;
const VAD_CHUNK: usize = 512;
const CHUNK_MS: u64 = (VAD_CHUNK * 1000) as u64 / TARGET_SAMPLE_RATE as u64;
const ONSET_CHUNKS: usize = 12; // ~384 ms of consecutive speech confirms onset
const PREROLL_CHUNKS: usize = 8; // keep ~256 ms before onset for Whisper
const ARMED_POLL: Duration = Duration::from_millis(30);
// Covers the 190 ms arm cue plus playback latency
const CUE_SETTLE: Duration = Duration::from_millis(250);
// Ceiling on one answer past the onset timeout; nothing may hang the session
const MAX_ANSWER: Duration = Duration::from_secs(60);

// Everything the audio consumer thread owns
pub struct Pipeline<C: Consumer<Item = f32>> {
    pub consumer: C,
    pub speech_to_text: WhisperEngine,
    pub vad: VADEngine,
    pub sample_rate: u32,
    pub state: Arc<DaemonState>,
    pub cues: mpsc::Sender<Cue>,
    pub endpoint_silence_ms: u64,
}

enum Phase {
    Waiting { speech_run: usize },
    InSpeech { silence_run: usize, start: usize },
    // The user is holding F5; the release ends the answer
    Manual { start: usize },
}

pub fn hotkey_listener<C>(
    pipeline: Pipeline<C>,
    barge_in: BargeInMode,
    commands: mpsc::Receiver<ConsumerCommand>,
) where
    C: Consumer<Item = f32> + Send + 'static,
{
    let key_state = Arc::clone(&pipeline.state);
    let key_cues = pipeline.cues.clone();

    // Spawn a heavy thread for the global hotkey listener
    thread::spawn(move || {
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
                    if key_state.try_transition(RecordingMode::Armed, RecordingMode::ArmedHold) {
                        // Manual override of an armed session: hold to answer
                        if matches!(barge_in, BargeInMode::Stop) {
                            key_state.speech().stop();
                        }
                        let _ = key_cues.send(Cue::RecordStart);
                    } else if key_state
                        .try_transition(RecordingMode::Idle, RecordingMode::PushToTalk)
                    {
                        // Silence the daemon's own voice before the mic opens
                        if matches!(barge_in, BargeInMode::Stop) {
                            key_state.speech().stop();
                        }
                        current_action = if shift_key_pressed {
                            HotKeyAction::Dictate
                        } else {
                            HotKeyAction::Mailbox
                        };
                        let _ = key_cues.send(Cue::RecordStart);
                        println!("F5 hotkey detected! Recording Audio...");
                    }
                }
                EventType::KeyRelease(Key::F5) => {
                    if key_state.try_transition(RecordingMode::PushToTalk, RecordingMode::Idle) {
                        println!("F5 hotkey released");
                        let _ = key_cues.send(Cue::RecordStop);
                        // Send whichever action we locked in when the key is pressed
                        let _ = key_state
                            .commands()
                            .send(ConsumerCommand::Transcribe(current_action));
                    } else if key_state
                        .try_transition(RecordingMode::ArmedHold, RecordingMode::Armed)
                    {
                        let _ = key_cues.send(Cue::RecordStop);
                    }
                }
                _ => (), // Ignore mouse movements
            }
        }) {
            println!("Error: {:?}", error);
        }
    });

    // Spawn a thread to handle the audio processing and transcription
    thread::spawn(move || {
        let mut pipeline = pipeline;
        while let Ok(command) = commands.recv() {
            match command {
                ConsumerCommand::Transcribe(action) => pipeline.transcribe_utterance(action),
                ConsumerCommand::Ask(ask) => pipeline.ask(ask),
            }
        }
    });
}

impl<C: Consumer<Item = f32>> Pipeline<C> {
    // Push-to-talk: F5 was released, the ring holds the whole utterance
    fn transcribe_utterance(&mut self, action: HotKeyAction) {
        // pull all the floats out of the ring buffer into a standard Vec
        let mut audio_data = Vec::new();
        audio_data.extend(self.consumer.pop_iter());

        println!(
            "Downsampling audio from {} Hz to {TARGET_SAMPLE_RATE} Hz...",
            self.sample_rate
        );

        // Downsample the audio by taking every nth sample
        let final_data = match resample_audio(&audio_data, self.sample_rate, TARGET_SAMPLE_RATE) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("Error: {e}");
                let _ = self.cues.send(Cue::Error);
                return;
            }
        };

        let (min_amplitude, max_amplitude) = final_data
            .iter()
            .fold((0.0f32, 0.0f32), |(min, max), &sample| {
                (min.min(sample), max.max(sample))
            });
        println!("Audio range: [{min_amplitude}, {max_amplitude}]");

        println!("Total audio samples after resampling: {}", final_data.len());

        // Chunk the audio into 512 sample frames and run VAD on each frame until we find speech or run out of audio
        let mut speech_chunks = 0;
        let mut total_chunks = 0;

        // Reset the VAD state before processing new audio
        self.vad.reset_state();
        let vad_threshold = self.state.vad_threshold();

        for chunk in final_data.chunks(VAD_CHUNK) {
            if chunk.len() < VAD_CHUNK {
                continue; // Ignore the last chunk if it's smaller than 512 samples
            }
            match self.vad.check_speech(chunk, TARGET_SAMPLE_RATE) {
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
            let _ = self.cues.send(Cue::Error);
            return;
        }

        if speech_chunks < 2 {
            println!("No speech detected in the audio. Skipping transcription.");
            let _ = self.cues.send(Cue::Error);
            return;
        }

        println!("Transcribing...");
        let transcribe_started = Instant::now();
        match self.speech_to_text.transcribe(&final_data) {
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
                    let _ = self.cues.send(Cue::Error);
                    return;
                }

                save_history(&self.state, &transcription);

                // Ready only after the utterance is actually delivered
                match action {
                    HotKeyAction::Mailbox => {
                        self.state.push_transcription(transcription);
                        let _ = self.cues.send(Cue::Ready);
                    }
                    HotKeyAction::Dictate => {
                        println!("Dictating: {}", transcription);
                        match type_text(&transcription) {
                            Ok(_) => {
                                let _ = self.cues.send(Cue::Ready);
                            }
                            Err(e) => {
                                eprintln!("Failed to type text: {:?}", e);
                                let _ = self.cues.send(Cue::Error);
                            }
                        }
                    }
                }
            }
            Err(error) => {
                eprintln!("Transcription failed: {error}");
                let _ = self.cues.send(Cue::Error);
            }
        }
    }

    // Armed listening for ask_user: the mode is already Armed and the
    // question has finished playing when this command arrives
    fn ask(&mut self, ask: AskCommand) {
        // The ring holds echo captured while the question played
        self.drain();
        let _ = self.cues.send(Cue::Arm);
        // Let the arm cue leave the speaker before the VAD listens
        thread::sleep(CUE_SETTLE);
        self.drain();

        let answer_audio = self.listen_for_answer(ask.timeout);

        // Close the mic before the slow transcription; every exit disarms
        self.state.set_recording_mode(RecordingMode::Idle);
        let _ = self.cues.send(Cue::Disarm);

        let text = match answer_audio {
            Some(audio) => match self.speech_to_text.transcribe(&audio) {
                Ok(text) => {
                    println!("Answer: {text}");
                    if !text.is_empty() {
                        save_history(&self.state, &text);
                    }
                    text
                }
                Err(e) => {
                    eprintln!("Transcription failed: {e}");
                    let _ = self.cues.send(Cue::Error);
                    String::new()
                }
            },
            None => {
                let _ = self.cues.send(Cue::Error);
                String::new()
            }
        };
        let _ = ask.reply.send(text);
    }

    // Online endpointing: confirm speech onset, then end on trailing silence.
    // Returns the answer audio at 16 kHz, or None on timeout or error.
    fn listen_for_answer(&mut self, timeout: Duration) -> Option<Vec<f32>> {
        let mut resampler = match StreamingResampler::new(self.sample_rate, TARGET_SAMPLE_RATE) {
            Ok(resampler) => resampler,
            Err(e) => {
                eprintln!("Failed to create resampler: {e}");
                return None;
            }
        };
        self.vad.reset_state();
        let vad_threshold = self.state.vad_threshold();
        let endpoint_chunks = (self.endpoint_silence_ms / CHUNK_MS).max(1) as usize;
        let deadline = Instant::now() + timeout;
        let hard_deadline = deadline + MAX_ANSWER;

        let mut audio: Vec<f32> = Vec::new();
        // Reused across polls so the loop allocates nothing in steady state
        let mut batch: Vec<f32> = Vec::new();
        let mut processed = 0;
        let mut phase = Phase::Waiting { speech_run: 0 };
        let mut suppressed = false;

        loop {
            thread::sleep(ARMED_POLL);

            match self.state.recording_mode() {
                RecordingMode::ArmedHold => {
                    if !matches!(phase, Phase::Manual { .. }) {
                        // The hold replaces whatever endpointing had collected
                        phase = Phase::Manual { start: audio.len() };
                    }
                }
                RecordingMode::Armed => {
                    if let Phase::Manual { start } = phase {
                        // F5 release ends the manual answer
                        audio.drain(..start);
                        return Some(audio);
                    }
                }
                // The session was closed from outside
                _ => return None,
            }

            // Checked before suppression so stuck speech cannot hang the session
            if Instant::now() >= hard_deadline {
                return match phase {
                    Phase::InSpeech { start, .. } | Phase::Manual { start } => {
                        audio.drain(..start);
                        Some(audio)
                    }
                    Phase::Waiting { .. } => None,
                };
            }

            // Half-duplex: drop capture while the daemon itself is speaking
            if self.state.speech().is_speaking() {
                self.drain();
                suppressed = true;
                continue;
            }
            if suppressed {
                suppressed = false;
                self.vad.reset_state();
                // Drop the pre-gap partial window so no spliced frame reaches the VAD
                resampler.reset();
                match &mut phase {
                    Phase::Waiting { speech_run } => *speech_run = 0,
                    Phase::InSpeech { silence_run, .. } => *silence_run = 0,
                    Phase::Manual { .. } => {}
                }
            }

            batch.clear();
            batch.extend(self.consumer.pop_iter());
            if let Err(e) = resampler.push(&batch, &mut audio) {
                eprintln!("Resampling failed: {e}");
                return None;
            }

            // Manual capture needs no VAD; the release is the endpoint
            while !matches!(phase, Phase::Manual { .. }) && audio.len() - processed >= VAD_CHUNK {
                let chunk = &audio[processed..processed + VAD_CHUNK];
                processed += VAD_CHUNK;
                let is_speech = match self.vad.check_speech(chunk, TARGET_SAMPLE_RATE) {
                    Ok(probability) => probability > vad_threshold,
                    Err(e) => {
                        eprintln!("VAD error: {e}");
                        false
                    }
                };
                match &mut phase {
                    Phase::Waiting { speech_run } => {
                        *speech_run = if is_speech { *speech_run + 1 } else { 0 };
                        if *speech_run >= ONSET_CHUNKS {
                            let start = processed
                                .saturating_sub((ONSET_CHUNKS + PREROLL_CHUNKS) * VAD_CHUNK);
                            phase = Phase::InSpeech {
                                silence_run: 0,
                                start,
                            };
                        }
                    }
                    Phase::InSpeech { silence_run, .. } => {
                        *silence_run = if is_speech { 0 } else { *silence_run + 1 };
                    }
                    Phase::Manual { .. } => {}
                }
            }

            if let Phase::InSpeech { silence_run, start } = phase
                && silence_run >= endpoint_chunks
            {
                audio.drain(..start);
                return Some(audio);
            }
            if matches!(phase, Phase::Waiting { .. }) && Instant::now() >= deadline {
                return None;
            }
        }
    }

    fn drain(&mut self) {
        self.consumer.pop_iter().for_each(drop);
    }
}

fn save_history(state: &DaemonState, transcription: &str) {
    if let Some(db) = state.db_connection()
        && let Ok(connection) = db.lock()
        && let Err(e) = crate::history::TranscriptionHistory::insert(&connection, transcription)
    {
        eprintln!("Failed to insert transcription into database: {e}");
    }
}

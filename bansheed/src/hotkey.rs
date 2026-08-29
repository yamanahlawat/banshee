use ringbuf::HeapCons;
use ringbuf::traits::Consumer;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rdev::listen;

use crate::audio::cues::{Cue, Cues};
use crate::audio::utils::{StreamingResampler, resample_audio};
use crate::binding::{Hotkey, HotkeyAction, HotkeyTracker};
use crate::config::HotkeyMode;
use crate::dictation::type_text;
use crate::speech_to_text::vad::VADEngine;
use crate::speech_to_text::whisper::WhisperEngine;
use crate::state::{AskCommand, ConsumerCommand, DaemonState, RecordingMode, TranscribeTarget};

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
// Past this ratio of wall time to audio length, the model is too heavy
const SLOW_TRANSCRIBE_FACTOR: f32 = 2.0;

/// They change together on a rebind, so they travel together.
pub struct CaptureSource {
    pub consumer: HeapCons<f32>,
    pub sample_rate: u32,
}

impl CaptureSource {
    pub fn drain(&mut self) -> Vec<f32> {
        self.consumer.pop_iter().collect()
    }

    pub fn discard(&mut self) {
        self.consumer.pop_iter().for_each(drop);
    }
}

// Everything the audio consumer thread owns
pub struct Pipeline {
    pub source: CaptureSource,
    pub speech_to_text: WhisperEngine,
    pub vad: VADEngine,
    pub state: Arc<DaemonState>,
    pub cues: Cues,
    pub endpoint_silence_ms: u64,
}

enum Phase {
    Waiting { speech_run: usize },
    InSpeech { silence_run: usize, start: usize },
    // The user is holding the hotkey; the release ends the answer
    Manual { start: usize },
}

pub fn hotkey_listener(
    pipeline: Pipeline,
    commands: mpsc::Receiver<ConsumerCommand>,
) -> thread::JoinHandle<()> {
    // The handle lets shutdown join it, dropping the Whisper context before atexit
    thread::spawn(move || {
        let mut pipeline = pipeline;
        while let Ok(command) = commands.recv() {
            match command {
                ConsumerCommand::Transcribe(action) => pipeline.transcribe_utterance(action),
                // A session opened while this command sat in the queue owns
                // the ring now: the discard skips, the cancelled lead-in stays.
                ConsumerCommand::Discard => {
                    if !pipeline.state.is_recording() {
                        pipeline.source.discard();
                    }
                }
                ConsumerCommand::Ask(ask) => pipeline.ask(ask),
                // The old ring dies with the stream that filled it. Anything
                // still in it came from a device that is gone.
                ConsumerCommand::Rebind {
                    consumer,
                    sample_rate,
                } => {
                    pipeline.source = CaptureSource {
                        consumer,
                        sample_rate,
                    };
                }
                ConsumerCommand::Shutdown => break,
            }
        }
    })
}

// Said by the daemon at startup and by the checklist, so the two cannot drift apart.
#[cfg(all(unix, not(target_os = "macos")))]
pub const WAYLAND_HOTKEY_HINT: &str = "the global hotkey needs X11. Bind \
     `banshee record start` on press and `banshee record stop` on release in \
     your compositor instead";

/// How to dictate, for the binding and mode in effect.
pub fn usage_hint(hotkey: Hotkey, hotkey_mode: HotkeyMode) -> String {
    #[cfg(all(unix, not(target_os = "macos")))]
    if crate::dictation::is_wayland() {
        return format!("{WAYLAND_HOTKEY_HINT}.");
    }
    let press = match hotkey_mode {
        HotkeyMode::Toggle => format!("Tap {hotkey} and speak, then tap it again to stop."),
        HotkeyMode::Hold => format!("Hold {hotkey} and speak, then release to stop."),
    };
    format!(
        "{press} The text lands in whatever app has focus.\n\
         Shift + {hotkey} sends it to `banshee listen` instead of typing it."
    )
}

// Registered by the daemon, not by the pipeline: a press must still reach
// record_start when recording is unavailable, or it answers with silence.
// rdev needs X11's XRecord, which wayland does not serve: listen either errors
// or attaches to Xwayland and never sees a key. Say so instead of looking broken.
pub fn start_global_hotkey(key_state: Arc<DaemonState>, hotkey: Hotkey, hotkey_mode: HotkeyMode) {
    #[cfg(all(unix, not(target_os = "macos")))]
    if crate::dictation::is_wayland() {
        println!("Wayland session: {WAYLAND_HOTKEY_HINT}.");
        return;
    }

    // Spawn a heavy thread for the global hotkey listener
    thread::spawn(move || {
        let mut tracker = HotkeyTracker::new(hotkey, hotkey_mode);
        // Whether this listener's own start opened the session in flight.
        // Cancel discards audio, so it must never touch a session another
        // caller opened while our start was refused as busy.
        let mut owned = false;
        if let Err(error) = listen(move |event| {
            // The daemon's own paste presses modifier keys on this same
            // event stream; they must not re-enter the tracker
            if key_state.is_typing() {
                return;
            }
            match tracker.on_event(&event.event_type) {
                Some(HotkeyAction::Start(target)) => owned = key_state.record_start(target),
                Some(HotkeyAction::Toggle(target)) => owned = key_state.record_toggle(target),
                Some(HotkeyAction::Stop) => key_state.record_stop(),
                Some(HotkeyAction::Cancel) if owned => key_state.record_cancel(),
                Some(HotkeyAction::Cancel) | None => {}
            }
        }) {
            // Names the capability that is gone, not just the error type
            eprintln!(
                "Global hotkey listener stopped: {error:?}. `banshee record start` \
                 and `banshee record stop` still work."
            );
        }
    });
}

impl Pipeline {
    // Push-to-talk ended: the ring holds the whole utterance
    fn transcribe_utterance(&mut self, action: TranscribeTarget) {
        // pull all the floats out of the ring buffer into a standard Vec
        let audio_data = self.source.drain();

        println!(
            "Downsampling audio from {} Hz to {TARGET_SAMPLE_RATE} Hz...",
            self.source.sample_rate
        );

        // Downsample the audio by taking every nth sample
        let final_data =
            match resample_audio(&audio_data, self.source.sample_rate, TARGET_SAMPLE_RATE) {
                Ok(data) => data,
                Err(e) => {
                    eprintln!("Error: {e}");
                    self.cues.send(Cue::Error);
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
            self.cues.send(Cue::Error);
            return;
        }

        if speech_chunks < 2 {
            println!("No speech detected in the audio. Skipping transcription.");
            self.cues.send(Cue::Error);
            return;
        }

        println!("Transcribing...");
        let transcribe_started = Instant::now();
        self.state.set_transcribing(true);
        let transcribed = self.speech_to_text.transcribe(&final_data);
        // A client refetches its history when `transcribing` falls, so the
        // row has to be stored before the flag drops.
        store(&self.state, transcribed.as_deref().ok());
        self.state.set_transcribing(false);
        match transcribed {
            Ok(transcription) => {
                let audio_secs = final_data.len() as f32 / TARGET_SAMPLE_RATE as f32;
                let elapsed = transcribe_started.elapsed().as_secs_f32();
                println!("Transcribed {audio_secs:.1}s of audio in {elapsed:.2}s");
                // A slow CPU reads as a dead microphone rather than a slow one
                let slowdown = elapsed / audio_secs.max(0.001);
                if slowdown > SLOW_TRANSCRIBE_FACTOR {
                    println!(
                        "Transcription ran {slowdown:.0}x slower than realtime on this \
                         machine. Set [stt] preset = \"fast\" in config.toml, then run \
                         banshee setup."
                    );
                }
                println!("Transcription: {transcription}");

                // Whisper can return nothing for noise; skip before it reaches the ring or clipboard
                if transcription.is_empty() {
                    println!("Empty transcription. Skipping.");
                    self.cues.send(Cue::Error);
                    return;
                }

                // Ready only after the utterance is actually delivered
                match action {
                    TranscribeTarget::Mailbox => {
                        self.state.push_transcription(transcription);
                        self.cues.send(Cue::Ready);
                    }
                    TranscribeTarget::Dictate => {
                        println!("Dictating: {}", transcription);
                        self.state.set_typing(true);
                        let typed = type_text(&transcription);
                        self.state.set_typing(false);
                        match typed {
                            Ok(_) => {
                                self.cues.send(Cue::Ready);
                            }
                            Err(e) => {
                                eprintln!("Failed to type text: {:?}", e);
                                self.cues.send(Cue::Error);
                            }
                        }
                    }
                }
            }
            Err(error) => {
                eprintln!("Transcription failed: {error}");
                self.cues.send(Cue::Error);
            }
        }
    }

    // Armed listening: the mode is Armed and the question has finished playing
    fn ask(&mut self, ask: AskCommand) {
        // The ring holds echo captured while the question played
        self.source.discard();
        self.cues.send(Cue::Arm);
        // Let the arm cue leave the speaker before the VAD listens
        thread::sleep(CUE_SETTLE);
        self.source.discard();

        let answer_audio = self.listen_for_answer(ask.timeout);

        // Close the mic before the slow transcription; every exit disarms
        self.state.set_recording_mode(RecordingMode::Idle);
        self.cues.send(Cue::Disarm);

        let text = match answer_audio {
            Some(audio) => {
                self.state.set_transcribing(true);
                let transcribed = self.speech_to_text.transcribe(&audio);
                store(&self.state, transcribed.as_deref().ok());
                self.state.set_transcribing(false);
                match transcribed {
                    Ok(text) => {
                        println!("Answer: {text}");
                        text
                    }
                    Err(e) => {
                        eprintln!("Transcription failed: {e}");
                        self.cues.send(Cue::Error);
                        String::new()
                    }
                }
            }
            None => {
                self.cues.send(Cue::Error);
                String::new()
            }
        };
        let _ = ask.reply.send(text);
    }

    // Online endpointing: confirm speech onset, then end on trailing silence.
    // Returns the answer audio at 16 kHz, or None on timeout or error.
    fn listen_for_answer(&mut self, timeout: Duration) -> Option<Vec<f32>> {
        let mut resampler =
            match StreamingResampler::new(self.source.sample_rate, TARGET_SAMPLE_RATE) {
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
                        // The hotkey release ends the manual answer
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
                self.source.discard();
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
            batch.extend(self.source.consumer.pop_iter());
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
}

/// Stores an utterance worth keeping. Whisper answers an empty string for
/// noise, and that is not a dictation.
fn store(state: &DaemonState, transcription: Option<&str>) {
    if let Some(text) = transcription
        && !text.is_empty()
    {
        save_history(state, text);
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

#[cfg(test)]
mod hint_tests {
    use super::*;
    use crate::binding::hotkey;

    #[test]
    fn the_hint_matches_the_mode_in_effect() {
        let toggle = usage_hint(Hotkey::default(), HotkeyMode::Toggle);
        assert!(toggle.contains("again"), "toggle must say to press twice");
        assert!(!toggle.contains("release"), "toggle must not say release");

        let hold = usage_hint(Hotkey::default(), HotkeyMode::Hold);
        assert!(hold.contains("release"), "hold must say to release");
        assert!(!hold.contains("again"), "hold must not say to press twice");
    }

    // A non-default key, so a regression to a constant goes red
    #[test]
    fn the_hint_names_the_key_the_listener_matches() {
        let rebound = hotkey("F6").unwrap();
        for mode in [HotkeyMode::Toggle, HotkeyMode::Hold] {
            let hint = usage_hint(rebound, mode);
            assert!(hint.contains("F6"), "the bound key must be named: {hint}");
            assert!(
                !hint.contains("RightOption"),
                "the default must not leak in: {hint}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::traits::{Producer, Split};

    fn source_holding(
        samples: &[f32],
        sample_rate: u32,
    ) -> (CaptureSource, ringbuf::HeapProd<f32>) {
        let (mut producer, consumer) = ringbuf::HeapRb::<f32>::new(64).split();
        producer.push_slice(samples);
        (
            CaptureSource {
                consumer,
                sample_rate,
            },
            producer,
        )
    }

    #[test]
    fn a_swapped_source_reads_the_new_ring_and_the_new_rate() {
        let (mut source, _old_producer) = source_holding(&[1.0, 2.0], 16000);
        assert_eq!(source.drain(), vec![1.0, 2.0]);
        assert_eq!(source.sample_rate, 16000);

        // A headset at 16 kHz gives way to the built in mic at 48 kHz
        let (replacement, _new_producer) = source_holding(&[7.0], 48000);
        source = replacement;

        assert_eq!(source.drain(), vec![7.0]);
        assert_eq!(
            source.sample_rate, 48000,
            "a stale rate resamples by the wrong ratio and distorts silently"
        );
    }

    #[test]
    fn discarding_a_source_empties_it() {
        let (mut source, _producer) = source_holding(&[1.0, 2.0, 3.0], 16000);
        source.discard();
        assert!(source.drain().is_empty());
    }
}

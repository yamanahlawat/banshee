use std::{
    sync::atomic::{AtomicBool, AtomicU32},
    time::Instant,
};

pub struct DaemonState {
    version: &'static str,
    stt_model: String,
    vad_model: String,
    vad_threshold: AtomicU32,
    audio_device: Option<String>,
    recording: AtomicBool,
    speaking: AtomicBool,
    started_at: Instant,
}

impl DaemonState {
    pub fn new(
        version: &'static str,
        stt_model: String,
        vad_model: String,
        initial_vad_threshold: f32,
        audio_device: Option<String>,
    ) -> Self {
        Self {
            version,
            stt_model,
            vad_model,
            vad_threshold: AtomicU32::new(initial_vad_threshold.to_bits()),
            audio_device,
            recording: AtomicBool::new(false),
            speaking: AtomicBool::new(false),
            started_at: Instant::now(),
        }
    }

    pub fn is_recording(&self) -> bool {
        self.recording.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn set_recording(&self, value: bool) {
        self.recording
            .store(value, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_speaking(&self) -> bool {
        self.speaking.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn set_speaking(&self, value: bool) {
        self.speaking
            .store(value, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn uptime(&self) -> std::time::Duration {
        self.started_at.elapsed()
    }

    pub fn version(&self) -> &'static str {
        self.version
    }

    pub fn stt_model(&self) -> &str {
        &self.stt_model
    }

    pub fn vad_model(&self) -> &str {
        &self.vad_model
    }

    pub fn audio_device(&self) -> Option<&str> {
        self.audio_device.as_deref()
    }

    pub fn set_vad_threshold(&self, threshold: f32) {
        self.vad_threshold
            .store(threshold.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    pub fn vad_threshold(&self) -> f32 {
        let bits = self
            .vad_threshold
            .load(std::sync::atomic::Ordering::Relaxed);
        f32::from_bits(bits)
    }
}

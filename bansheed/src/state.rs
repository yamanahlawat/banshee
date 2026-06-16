use std::{sync::atomic::AtomicBool, time::Instant};

pub struct DaemonState {
    version: &'static str,
    stt_model: String,
    vad_model: String,
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
        audio_device: Option<String>,
    ) -> Self {
        Self {
            version,
            stt_model,
            vad_model,
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
}

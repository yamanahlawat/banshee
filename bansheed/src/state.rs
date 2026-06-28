use std::{
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32},
    },
    time::Instant,
};

pub struct DaemonState {
    version: &'static str,
    stt_model: String,
    vad_model: String,
    vad_threshold: AtomicU32,
    audio_device: OnceLock<String>,
    recording: AtomicBool,
    started_at: Instant,
    connection: Option<Mutex<rusqlite::Connection>>,
}

impl DaemonState {
    pub fn new(
        version: &'static str,
        stt_model: String,
        vad_model: String,
        initial_vad_threshold: f32,
        connection: Option<Mutex<rusqlite::Connection>>,
    ) -> Self {
        Self {
            version,
            stt_model,
            vad_model,
            vad_threshold: AtomicU32::new(initial_vad_threshold.to_bits()),
            audio_device: OnceLock::new(),
            recording: AtomicBool::new(false),
            started_at: Instant::now(),
            connection,
        }
    }

    pub fn is_recording(&self) -> bool {
        self.recording.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_recording(&self, value: bool) {
        self.recording
            .store(value, std::sync::atomic::Ordering::Relaxed);
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
        self.audio_device.get().map(String::as_str)
    }

    pub fn set_audio_device(&self, device_name: String) {
        let _ = self.audio_device.set(device_name);
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

    pub fn connection(&self) -> Option<&Mutex<rusqlite::Connection>> {
        self.connection.as_ref()
    }
}

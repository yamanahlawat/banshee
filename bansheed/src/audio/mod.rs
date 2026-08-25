pub mod cues;
pub mod utils;
use banshee_common::{InputDevice, error::BansheeError};

use std::sync::Arc;

use cpal::{
    Stream,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use ringbuf::{
    HeapRb,
    traits::{Consumer, Producer, Split},
};

use crate::state::DaemonState;

pub const RING_SECS: usize = 120; // 120 seconds of audio in the ring buffer

// The name [audio] input_device carries to mean "whatever the OS is set to"
pub const DEFAULT_INPUT_DEVICE: &str = "default";

/// Names only: opening each device to prove it records would steal the
/// microphone from a running daemon.
pub fn input_devices() -> Vec<InputDevice> {
    let host = cpal::default_host();
    let preferred = host
        .default_input_device()
        .and_then(|device| device.description().ok())
        .map(|description| description.name().to_string());
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    devices
        .filter_map(|device| device.description().ok())
        .map(|description| {
            let name = description.name().to_string();
            InputDevice {
                default: Some(&name) == preferred.as_ref(),
                name,
            }
        })
        .collect()
}

// Substring match, so a config can say "yeti" not "Blue Yeti Stereo Microphone"
fn find_input_device(host: &cpal::Host, wanted: &str) -> Result<cpal::Device, BansheeError> {
    let wanted_lower = wanted.to_lowercase();
    for device in host
        .input_devices()
        .map_err(|e| BansheeError::Other(e.to_string()))?
    {
        let Ok(description) = device.description() else {
            continue;
        };
        if description.name().to_lowercase().contains(&wanted_lower) {
            return Ok(device);
        }
    }
    let available: Vec<String> = input_devices().into_iter().map(|d| d.name).collect();
    let available = if available.is_empty() {
        "none".to_string()
    } else {
        available.join(", ")
    };
    Err(BansheeError::Other(format!(
        "no input device matching \"{wanted}\"; available: {available}"
    )))
}

// Anything but "default" is explicit and must fail loudly rather than fall back
// to the wrong microphone. Shared with the checklist, so it sees what capture opens.
pub fn resolve_input_device(input_device: &str) -> Result<cpal::Device, BansheeError> {
    let host = cpal::default_host();
    if input_device == DEFAULT_INPUT_DEVICE {
        host.default_input_device()
            .ok_or(BansheeError::NoAudioDevice)
    } else {
        find_input_device(&host, input_device)
    }
}

/// Whether the configured preference still appears in the OS input list.
/// Portable: no CoreAudio property listeners (cpal's disconnect path does not
/// fire on Bluetooth object destruction — see #47 maintainer measurements).
pub fn configured_input_still_listed(input_device: &str) -> bool {
    if input_device == DEFAULT_INPUT_DEVICE {
        // "default" tracks whatever the OS currently prefers; absence of *any*
        // input device is the only list-level failure mode.
        return !input_devices().is_empty();
    }
    let wanted = input_device.to_lowercase();
    input_devices()
        .into_iter()
        .any(|d| d.name.to_lowercase().contains(&wanted))
}

/// Run the #47 health checks and fail closed if the bound input is gone.
pub fn poll_input_health(daemon_state: &DaemonState, configured_input: &str) {
    let listed = configured_input_still_listed(configured_input);
    if daemon_state.should_mark_input_lost(listed) {
        let detail = if !listed {
            format!("input device no longer listed ({configured_input})")
        } else {
            "input stream stopped delivering audio callbacks".to_string()
        };
        daemon_state.mark_input_lost(detail);
    }
}

// The checklist opens through this too, so a green tick and a working daemon
// cannot drift. A device that will not describe itself stays unknown.
fn open_input(
    input_device: &str,
) -> Result<(cpal::Device, Option<String>, cpal::SupportedStreamConfig), BansheeError> {
    let device = resolve_input_device(input_device)?;
    let name = device.description().map(|d| d.name().to_string()).ok();
    let config = device
        .default_input_config()
        .map_err(|e| BansheeError::Other(e.to_string()))?;
    Ok((device, name, config))
}

// Shared so the probe fails wherever capture would, down to the sample format.
fn build_and_play<D>(
    device: &cpal::Device,
    config: cpal::SupportedStreamConfig,
    data: D,
) -> Result<Stream, BansheeError>
where
    D: FnMut(&[f32], &cpal::InputCallbackInfo) + Send + 'static,
{
    let stream = device
        .build_input_stream(
            &config.into(),
            data,
            |error| eprintln!("Audio Error: {error}"),
            None,
        )
        .map_err(|e| BansheeError::Other(e.to_string()))?;
    stream
        .play()
        .map_err(|e| BansheeError::Other(e.to_string()))?;
    Ok(stream)
}

/// Enumeration is not proof: a device can list itself and still fail
/// `hw_params` when opened. Returns the microphone it opened.
pub fn probe_input_device(input_device: &str) -> Result<Option<String>, BansheeError> {
    let (device, name, config) = open_input(input_device)?;
    // Dropped at once: opening and starting it is the whole proof
    drop(build_and_play(&device, config, |_, _| {})?);
    Ok(name)
}

// `use<>`: edition 2024 would otherwise capture input_device's lifetime and
// stop the consumer crossing into the audio thread, which needs 'static.
pub fn start_audio_capture(
    daemon_state: Arc<DaemonState>,
    input_device: &str,
) -> Result<(Stream, impl Consumer<Item = f32> + use<>, u32), BansheeError> {
    let (device, name, config) = open_input(input_device)?;
    println!("Default config {:?}", config);

    let sample_rate = config.sample_rate();
    let channels = config.channels();

    let ring_capacity = sample_rate as usize * RING_SECS;
    let (mut producer, consumer) = HeapRb::<f32>::new(ring_capacity).split();

    let capture_state = Arc::clone(&daemon_state);
    // Runs on the real-time audio thread, so it must not allocate: downmixing
    // through an iterator keeps the mono copy out of the heap
    let stream = build_and_play(&device, config, move |data: &[f32], _| {
        // Always tick: a dead BT device stops callbacks entirely (#47). Silence
        // is not what we see — we see nothing, and that is the detect signal.
        capture_state.note_input_callback();
        if capture_state.is_recording() {
            if channels > 1 {
                producer.push_iter(
                    data.chunks(channels as usize)
                        .map(|frame| frame.iter().sum::<f32>() / channels as f32),
                );
            } else {
                producer.push_slice(data);
            }
        }
    })?;

    // Set after play() succeeds, so status never names a mic that failed to open
    if let Some(name) = name {
        println!("Using microphone {name}");
        daemon_state.set_audio_device(name);
    }
    Ok((stream, consumer, sample_rate))
}

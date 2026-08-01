pub mod cues;
pub mod utils;
use banshee_common::error::BansheeError;

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

// Substring match, so a config can say "yeti" not "Blue Yeti Stereo Microphone"
fn find_input_device(host: &cpal::Host, wanted: &str) -> Result<cpal::Device, BansheeError> {
    let wanted_lower = wanted.to_lowercase();
    let mut available = Vec::new();
    for device in host
        .input_devices()
        .map_err(|e| BansheeError::Other(e.to_string()))?
    {
        let Ok(description) = device.description() else {
            continue;
        };
        let name = description.name().to_string();
        if name.to_lowercase().contains(&wanted_lower) {
            return Ok(device);
        }
        available.push(name);
    }
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
// to the wrong microphone. Shared with doctor so it sees what capture opens.
pub fn resolve_input_device(input_device: &str) -> Result<cpal::Device, BansheeError> {
    // Ask cpal to give us the default OS audio API
    let host = cpal::default_host();
    if input_device == DEFAULT_INPUT_DEVICE {
        host.default_input_device()
            .ok_or(BansheeError::NoAudioDevice)
    } else {
        find_input_device(&host, input_device)
    }
}

// `use<>`: edition 2024 would otherwise capture input_device's lifetime and
// stop the consumer crossing into the audio thread, which needs 'static.
pub fn start_audio_capture(
    daemon_state: Arc<DaemonState>,
    input_device: &str,
) -> Result<(Stream, impl Consumer<Item = f32> + use<>, u32), BansheeError> {
    let device = resolve_input_device(input_device)?;

    if let Ok(description) = device.description() {
        let name = description.name();
        daemon_state.set_audio_device(name.to_string());
        println!("Using microphone {name}");
    }

    // Get the default config
    let config = device
        .default_input_config()
        .map_err(|e| BansheeError::Other(e.to_string()))?;
    println!("Default config {:?}", config);

    let sample_rate = config.sample_rate();
    let channels = config.channels();

    // Create a ring buffer with 120 sec capacity
    let ring_capacity = sample_rate as usize * RING_SECS;
    let (mut producer, consumer) = HeapRb::<f32>::new(ring_capacity).split();

    let stream = device
        .build_input_stream(
            &config.into(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if daemon_state.is_recording() {
                    let mono_data: Vec<f32> = if channels > 1 {
                        data.chunks(channels as usize)
                            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                            .collect()
                    } else {
                        data.to_vec()
                    };
                    producer.push_slice(&mono_data);
                };
            },
            |error| eprintln!("Audio Error: {error}"),
            None,
        )
        .map_err(|e| BansheeError::Other(e.to_string()))?;
    stream
        .play()
        .map_err(|e| BansheeError::Other(e.to_string()))?;
    Ok((stream, consumer, sample_rate))
}

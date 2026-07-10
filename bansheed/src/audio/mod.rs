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

const RING_SECS: usize = 120; // 120 seconds of audio in the ring buffer

pub fn start_audio_capture(
    daemon_state: Arc<DaemonState>,
) -> Result<(Stream, impl Consumer<Item = f32>, u32), BansheeError> {
    // Ask cpal to give us the default OS audio API
    let host = cpal::default_host();

    // Find the default microphone
    let device = host
        .default_input_device()
        .ok_or(BansheeError::NoAudioDevice)?;

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

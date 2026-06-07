use cpal::{
    Stream,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use ringbuf::{
    HeapRb,
    traits::{Consumer, Producer, Split},
};
use std::sync::atomic::Ordering;

use crate::hotkey::IS_RECORDING;

pub fn start_audio_capture()
-> Result<(Stream, impl Consumer<Item = f32>), Box<dyn std::error::Error>> {
    // Ask cpal to give us the default OS audio API
    let host = cpal::default_host();

    // Create a ring buffer with 30 sec capacity
    let (mut producer, consumer) = HeapRb::<f32>::new(48000 * 30).split();

    // Find the default microphone
    let device = host
        .default_input_device()
        .ok_or("Error: No input device(microphone) was found!")?;

    if let Ok(description) = device.description() {
        let name = description.name();
        println!("Using microphone {name}");
    }

    // Get the default config
    let config = device.default_input_config()?;
    println!("Default config {:?}", config);

    let stream = device.build_input_stream(
        &config.into(),
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            if IS_RECORDING.load(Ordering::Relaxed) {
                producer.push_slice(data);
            };
        },
        |error| eprintln!("Audio Error: {error}"),
        None,
    )?;
    stream.play()?;
    Ok((stream, consumer))
}

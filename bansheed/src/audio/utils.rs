use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

pub fn resample_audio(
    audio_data: &[f32],
    original_sample_rate: u32,
    target_sample_rate: u32,
) -> Vec<f32> {
    let final_audio = if original_sample_rate == target_sample_rate {
        audio_data.to_vec()
    } else {
        println!("Resampling audio from {original_sample_rate} Hz to {target_sample_rate} Hz...");
        // Setup the input adapter (1 channel, so frames = length)
        let nbr_input_frames = audio_data.len();

        let input_adapter = InterleavedSlice::new(&audio_data, 1, nbr_input_frames).unwrap();

        // Setup the output buffer and adapter
        // Calculate how big the output will be (e.g 1/3 the size), plus some padding
        let out_capacity = (audio_data.len() as f64 * target_sample_rate as f64
            / original_sample_rate as f64)
            .ceil() as usize
            + 2048;
        let mut output = vec![0.0f32; out_capacity];
        let mut output_adapter = InterleavedSlice::new_mut(&mut output, 1, out_capacity).unwrap();

        // Create the FFT resampler
        let mut resampler = Fft::<f32>::new(
            original_sample_rate as usize,
            target_sample_rate as usize,
            1024, // chunk size
            1,    // Sub chunks
            1,    // Channels (mono)
            FixedSync::Both,
        )
        .expect("Failed to create resampler");

        // Process the entire audio in one go
        let (_frames_read, frames_written) = resampler
            .process_all_into_buffer(&input_adapter, &mut output_adapter, nbr_input_frames, None)
            .expect("Resampling failed");

        // Truncate the output to the actual number of frames written
        output.truncate(frames_written);
        output
    };
    final_audio
}

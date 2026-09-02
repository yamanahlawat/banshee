use audioadapter_buffers::direct::InterleavedSlice;
use banshee_common::error::BansheeError;
use rubato::{Fft, FixedSync, Resampler};

// One persistent Fft fed fixed windows keeps chunk boundaries continuous;
// per-batch one-shot resampling would glitch at every seam
pub struct StreamingResampler {
    resampler: Option<Fft<f32>>,
    carry: Vec<f32>,
    scratch: Vec<f32>,
}

impl StreamingResampler {
    pub fn new(original_sample_rate: u32, target_sample_rate: u32) -> Result<Self, BansheeError> {
        let resampler = if original_sample_rate == target_sample_rate {
            None
        } else {
            Some(
                Fft::<f32>::new(
                    original_sample_rate as usize,
                    target_sample_rate as usize,
                    1024,
                    1,
                    1,
                    FixedSync::Input,
                )
                .map_err(|e| BansheeError::Other(format!("Failed to create resampler: {e}")))?,
            )
        };
        Ok(Self {
            resampler,
            carry: Vec::new(),
            scratch: Vec::new(),
        })
    }

    // For a capture gap: the carried window must not bridge it
    pub fn reset(&mut self) {
        self.carry.clear();
        if let Some(resampler) = self.resampler.as_mut() {
            resampler.reset();
        }
    }

    // A partial window waits for the next push
    pub fn push(&mut self, samples: &[f32], out: &mut Vec<f32>) -> Result<(), BansheeError> {
        let Some(resampler) = self.resampler.as_mut() else {
            out.extend_from_slice(samples);
            return Ok(());
        };
        self.carry.extend_from_slice(samples);
        let window = resampler.input_frames_next();
        let mut consumed = 0;
        while self.carry.len() - consumed >= window {
            let input = InterleavedSlice::new(&self.carry[consumed..consumed + window], 1, window)
                .map_err(|e| BansheeError::Other(format!("Failed to create input adapter: {e}")))?;
            let needed = resampler.output_frames_next();
            self.scratch.resize(needed, 0.0);
            let mut output =
                InterleavedSlice::new_mut(&mut self.scratch, 1, needed).map_err(|e| {
                    BansheeError::Other(format!("Failed to create output adapter: {e}"))
                })?;
            let (frames_in, frames_out) = resampler
                .process_into_buffer(&input, &mut output, None)
                .map_err(|e| BansheeError::Other(format!("Failed to resample audio: {e}")))?;
            consumed += frames_in;
            out.extend_from_slice(&self.scratch[..frames_out]);
        }
        self.carry.drain(..consumed);
        Ok(())
    }
}

pub fn resample_audio(
    audio_data: &[f32],
    original_sample_rate: u32,
    target_sample_rate: u32,
) -> Result<Vec<f32>, BansheeError> {
    let final_audio = if original_sample_rate == target_sample_rate {
        audio_data.to_vec()
    } else {
        println!("Resampling audio from {original_sample_rate} Hz to {target_sample_rate} Hz...");
        let nbr_input_frames = audio_data.len();

        let input_adapter = InterleavedSlice::new(audio_data, 1, nbr_input_frames)
            .map_err(|e| BansheeError::Other(format!("Failed to create input adapter: {e}")))?;

        let out_capacity = (audio_data.len() as f64 * target_sample_rate as f64
            / original_sample_rate as f64)
            .ceil() as usize
            + 2048;
        let mut output = vec![0.0f32; out_capacity];
        let mut output_adapter = InterleavedSlice::new_mut(&mut output, 1, out_capacity)
            .map_err(|e| BansheeError::Other(format!("Failed to create output adapter: {e}")))?;

        let mut resampler = Fft::<f32>::new(
            original_sample_rate as usize,
            target_sample_rate as usize,
            1024, // chunk size
            1,    // Sub chunks
            1,    // Channels (mono)
            FixedSync::Both,
        )
        .map_err(|e| BansheeError::Other(format!("Failed to create resampler: {e}")))?;

        let (_frames_read, frames_written) = resampler
            .process_all_into_buffer(&input_adapter, &mut output_adapter, nbr_input_frames, None)
            .map_err(|e| BansheeError::Other(format!("Failed to resample audio: {e}")))?;

        output.truncate(frames_written);
        output
    };
    Ok(final_audio)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_resampler_converges_on_the_expected_length() {
        let input: Vec<f32> = (0..48000).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut resampler = StreamingResampler::new(48000, 16000).unwrap();
        let mut out = Vec::new();
        // Odd batch size so windows straddle push boundaries
        for batch in input.chunks(1447) {
            resampler.push(batch, &mut out).unwrap();
        }
        // One second in, about a third out, minus resampler delay and carry
        assert!(
            (14000..=16000).contains(&out.len()),
            "unexpected output length {}",
            out.len()
        );
    }

    #[test]
    fn streaming_resampler_passes_through_matching_rates() {
        let mut resampler = StreamingResampler::new(16000, 16000).unwrap();
        let mut out = Vec::new();
        resampler.push(&[0.1, 0.2, 0.3], &mut out).unwrap();
        assert_eq!(out, vec![0.1, 0.2, 0.3]);
    }
}

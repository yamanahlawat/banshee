use audioadapter_buffers::direct::InterleavedSlice;
use banshee_common::error::BansheeError;
use rubato::{Fft, FixedSync, Resampler, WindowFunction};

/// One 16-bit sample as the mixer takes it. The scale is `32_768` while
/// `pcm16_wav` encodes by `i16::MAX`, so that `+1.0` cannot overflow; the
/// asymmetry is deliberate and lives in one place.
pub fn from_pcm16(sample: i16) -> f32 {
    f32::from(sample) / 32_768.0
}

/// Unmeasured. Shared so both resamplers cut on the same seams.
const WINDOW: usize = 1024;

/// Margin on the predicted output length. Measured at zero frames needed, and
/// `slack_is_never_needed` fails if that changes.
const OUTPUT_SLACK: usize = 2 * WINDOW;

// One persistent Fft fed fixed windows keeps chunk boundaries continuous;
// per-batch one-shot resampling would glitch at every seam
pub struct StreamingResampler {
    resampler: Option<Fft<f32>>,
    carry: Vec<f32>,
    scratch: Vec<f32>,
    /// Input frames pushed and output frames written since the last reset.
    taken: usize,
    given: usize,
}

impl StreamingResampler {
    pub fn new(original_sample_rate: u32, target_sample_rate: u32) -> Result<Self, BansheeError> {
        let resampler = if original_sample_rate == target_sample_rate {
            None
        } else {
            Some(
                Fft::<f32>::new_custom(
                    original_sample_rate as usize,
                    target_sample_rate as usize,
                    WINDOW,
                    1,
                    1,
                    WindowFunction::BlackmanHarris2,
                    FixedSync::Input,
                )
                .map_err(|e| BansheeError::Other(format!("Failed to create resampler: {e}")))?,
            )
        };
        Ok(Self {
            resampler,
            carry: Vec::new(),
            scratch: Vec::new(),
            taken: 0,
            given: 0,
        })
    }

    // For a capture gap: the carried window must not bridge it
    pub fn reset(&mut self) {
        self.carry.clear();
        self.taken = 0;
        self.given = 0;
        if let Some(resampler) = self.resampler.as_mut() {
            resampler.reset();
        }
    }

    // A partial window waits for the next push
    pub fn push(&mut self, samples: &[f32], out: &mut Vec<f32>) -> Result<(), BansheeError> {
        if self.resampler.is_none() {
            out.extend_from_slice(samples);
            return Ok(());
        }
        self.taken += samples.len();
        self.carry.extend_from_slice(samples);
        self.convert(out)
    }

    /// Pads the last partial window with silence and drains what the filter
    /// still holds, so `out` covers every frame pushed, as `resample_audio`
    /// does. `out` keeps one filter delay, `output_delay` frames, at the start.
    pub fn finish(&mut self, out: &mut Vec<f32>) -> Result<(), BansheeError> {
        let Some(resampler) = self.resampler.as_ref() else {
            return Ok(());
        };
        let window = resampler.input_frames_next();
        let wanted = resampler.output_delay()
            + (self.taken as f64 * resampler.resample_ratio()).ceil() as usize;
        while self.given < wanted {
            let padded = self.carry.len().div_ceil(window).max(1) * window;
            self.carry.resize(padded, 0.0);
            self.convert(out)?;
        }
        out.truncate(out.len() - (self.given - wanted));
        self.given = wanted;
        self.carry.clear();
        Ok(())
    }

    fn convert(&mut self, out: &mut Vec<f32>) -> Result<(), BansheeError> {
        let Some(resampler) = self.resampler.as_mut() else {
            return Ok(());
        };
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
            self.given += frames_out;
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
        log::debug!(
            "Resampling audio from {original_sample_rate} Hz to {target_sample_rate} Hz..."
        );
        let nbr_input_frames = audio_data.len();

        let input_adapter = InterleavedSlice::new(audio_data, 1, nbr_input_frames)
            .map_err(|e| BansheeError::Other(format!("Failed to create input adapter: {e}")))?;

        let out_capacity = (audio_data.len() as f64 * target_sample_rate as f64
            / original_sample_rate as f64)
            .ceil() as usize
            + OUTPUT_SLACK;
        let mut output = vec![0.0f32; out_capacity];
        let mut output_adapter = InterleavedSlice::new_mut(&mut output, 1, out_capacity)
            .map_err(|e| BansheeError::Other(format!("Failed to create output adapter: {e}")))?;

        let mut resampler = Fft::<f32>::new_custom(
            original_sample_rate as usize,
            target_sample_rate as usize,
            WINDOW,
            1, // Sub chunks
            1, // Channels (mono)
            WindowFunction::BlackmanHarris2,
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

    fn streamed_and_finished(input: &[f32], from: u32) -> Vec<f32> {
        let mut resampler = StreamingResampler::new(from, 16000).unwrap();
        let delay = resampler.resampler.as_ref().unwrap().output_delay();
        let mut out = Vec::new();
        // Odd batch size so windows straddle push boundaries
        for batch in input.chunks(1447) {
            resampler.push(batch, &mut out).unwrap();
        }
        resampler.finish(&mut out).unwrap();
        out.drain(..delay);
        out
    }

    #[test]
    fn a_finished_stream_matches_the_whole_resample() {
        for (from, frames) in [(48_000u32, 48_000usize), (44_100, 30_001)] {
            let input: Vec<f32> = (0..frames).map(|i| (i as f32 * 0.01).sin()).collect();
            let whole = resample_audio(&input, from, 16000).unwrap();
            assert_eq!(
                streamed_and_finished(&input, from),
                whole,
                "{frames} frames at {from} Hz"
            );
        }
    }

    #[test]
    fn a_finished_stream_shorter_than_a_window_has_the_whole_length() {
        let input: Vec<f32> = (0..700).map(|i| (i as f32 * 0.01).sin()).collect();
        let whole = resample_audio(&input, 48_000, 16000).unwrap();
        assert_eq!(streamed_and_finished(&input, 48_000).len(), whole.len());
    }

    #[test]
    fn finishing_a_matching_rate_adds_nothing() {
        let mut resampler = StreamingResampler::new(16000, 16000).unwrap();
        let mut out = Vec::new();
        resampler.push(&[0.1, 0.2, 0.3], &mut out).unwrap();
        resampler.finish(&mut out).unwrap();
        assert_eq!(out, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn streaming_resampler_passes_through_matching_rates() {
        let mut resampler = StreamingResampler::new(16000, 16000).unwrap();
        let mut out = Vec::new();
        resampler.push(&[0.1, 0.2, 0.3], &mut out).unwrap();
        assert_eq!(out, vec![0.1, 0.2, 0.3]);
    }
}

#[cfg(test)]
mod slack_tests {
    use super::*;

    /// Fails the day the reserve stops being a margin and starts carrying load.
    #[test]
    fn slack_is_never_needed() {
        let mut worst = 0i64;
        for (from, to) in [
            (48_000u32, 16_000u32),
            (44_100, 16_000),
            (96_000, 16_000),
            (16_000, 16_000),
            (22_050, 16_000),
            (8_000, 16_000),
        ] {
            for seconds in [0.02f64, 0.1, 1.0, 3.7] {
                let frames = (from as f64 * seconds) as usize;
                let input: Vec<f32> = (0..frames).map(|i| (i as f32 * 0.01).sin()).collect();
                let predicted = (frames as f64 * f64::from(to) / f64::from(from)).ceil() as usize;
                let written = resample_audio(&input, from, to).expect("a resample").len();
                worst = worst.max(written as i64 - predicted as i64);
            }
        }
        assert_eq!(
            worst, 0,
            "the resampler wrote past the predicted length, so the reserve is load-bearing now"
        );
    }
}

use std::f64::consts::PI;
use std::sync::OnceLock;

use realfft::RealFftPlanner;

pub const WINDOW: usize = 128_000;

const SAMPLE_RATE: f64 = 16_000.0;
const N_FFT: usize = 400;
const HOP: usize = 160;
const BINS: usize = N_FFT / 2 + 1;
const MELS: usize = 80;
const FRAMES: usize = 800;

/// The last `WINDOW` samples of `audio`, with zeros in front when it is shorter.
pub fn front_padded(audio: &[f32]) -> Vec<f32> {
    let mut window = vec![0.0; WINDOW.saturating_sub(audio.len())];
    window.extend_from_slice(&audio[audio.len().saturating_sub(WINDOW)..]);
    window
}

/// Whisper log-mel features of exactly `WINDOW` samples, mel-major: `out[m * 800 + t]`.
pub fn whisper_features(window: &[f32]) -> Vec<f32> {
    assert_eq!(
        window.len(),
        WINDOW,
        "the window is exactly {WINDOW} samples"
    );
    let filters = mel_filters();
    let hann = hann();

    let mean = window.iter().map(|&x| f64::from(x)).sum::<f64>() / WINDOW as f64;
    let variance = window
        .iter()
        .map(|&x| (f64::from(x) - mean).powi(2))
        .sum::<f64>()
        / WINDOW as f64;
    let scale = (variance + 1e-7).sqrt();
    let normalised: Vec<f32> = window
        .iter()
        .map(|&x| ((f64::from(x) - mean) / scale) as f32)
        .collect();

    let pad = N_FFT / 2;
    let mut padded = Vec::with_capacity(WINDOW + 2 * pad);
    padded.extend((1..=pad).rev().map(|i| normalised[i]));
    padded.extend_from_slice(&normalised);
    padded.extend((1..=pad).map(|i| normalised[WINDOW - 1 - i]));

    let fft = RealFftPlanner::<f32>::new().plan_fft_forward(N_FFT);
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();
    let mut power = [0.0f32; BINS];
    let mut log_mel = vec![0.0f32; MELS * FRAMES];

    for t in 0..FRAMES {
        let frame = &padded[t * HOP..t * HOP + N_FFT];
        for ((slot, &x), &w) in input.iter_mut().zip(frame).zip(&hann) {
            *slot = x * w;
        }
        fft.process(&mut input, &mut spectrum)
            .expect("buffers come from the plan");
        for (p, c) in power.iter_mut().zip(&spectrum) {
            *p = c.norm_sqr();
        }
        for (m, filter) in filters.as_chunks::<BINS>().0.iter().enumerate() {
            let energy: f32 = filter.iter().zip(&power).map(|(f, p)| f * p).sum();
            log_mel[m * FRAMES + t] = energy.max(1e-10).log10();
        }
    }

    let floor = log_mel.iter().copied().fold(f32::MIN, f32::max) - 8.0;
    for x in &mut log_mel {
        *x = (x.max(floor) + 4.0) / 4.0;
    }
    log_mel
}

fn hann() -> Vec<f32> {
    (0..N_FFT)
        .map(|i| (0.5 - 0.5 * (2.0 * PI * i as f64 / N_FFT as f64).cos()) as f32)
        .collect()
}

/// 80 Slaney-normalised triangle filters over the 201 FFT bins, filter-major.
fn mel_filters() -> &'static [f32] {
    static FILTERS: OnceLock<Vec<f32>> = OnceLock::new();
    FILTERS.get_or_init(|| {
        let bin_hz: Vec<f64> = (0..BINS)
            .map(|k| k as f64 * SAMPLE_RATE / N_FFT as f64)
            .collect();
        let top = hz_to_mel(SAMPLE_RATE / 2.0);
        let edges: Vec<f64> = (0..MELS + 2)
            .map(|i| mel_to_hz(top * i as f64 / (MELS + 1) as f64))
            .collect();
        let mut filters = Vec::with_capacity(MELS * BINS);
        for m in 0..MELS {
            let (lo, mid, hi) = (edges[m], edges[m + 1], edges[m + 2]);
            let norm = 2.0 / (hi - lo);
            filters.extend(bin_hz.iter().map(|&f| {
                let rise = (f - lo) / (mid - lo);
                let fall = (hi - f) / (hi - mid);
                (rise.min(fall).max(0.0) * norm) as f32
            }));
        }
        filters
    })
}

const LOG_FROM_HZ: f64 = 1000.0;
const LOG_FROM_MEL: f64 = 15.0;

fn log_step() -> f64 {
    6.4f64.ln() / 27.0
}

fn hz_to_mel(hz: f64) -> f64 {
    if hz >= LOG_FROM_HZ {
        LOG_FROM_MEL + (hz / LOG_FROM_HZ).ln() / log_step()
    } else {
        3.0 * hz / 200.0
    }
}

fn mel_to_hz(mel: f64) -> f64 {
    if mel >= LOG_FROM_MEL {
        LOG_FROM_HZ * ((mel - LOG_FROM_MEL) * log_step()).exp()
    } else {
        200.0 * mel / 3.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOLERANCE: f32 = 1e-4;

    fn fixture() -> Vec<f32> {
        include_bytes!("../../tests/data/smart_turn_features.f32")
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    }

    #[test]
    fn features_match_the_whisper_extractor() {
        let expected = fixture();
        assert_eq!(expected.len(), 80 * 800);
        let got = whisper_features(&front_padded(&crate::speech_to_text::vad::test_speech()));
        assert_eq!(got.len(), 80 * 800);
        let worst = got
            .iter()
            .zip(&expected)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(worst < TOLERANCE, "worst difference {worst}");
    }

    #[test]
    fn front_padding_keeps_the_latest_audio_at_the_end() {
        let audio: Vec<f32> = (0..10).map(|i| i as f32).collect();
        let padded = front_padded(&audio);
        assert_eq!(padded.len(), WINDOW);
        assert_eq!(&padded[WINDOW - 10..], &audio[..]);
        assert!(padded[..WINDOW - 10].iter().all(|&x| x == 0.0));
        let long: Vec<f32> = (0..WINDOW + 5).map(|i| i as f32).collect();
        assert_eq!(front_padded(&long)[0], 5.0);
    }
}

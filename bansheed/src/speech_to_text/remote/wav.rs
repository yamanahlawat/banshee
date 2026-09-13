/// A 16-bit mono PCM WAV around the pipeline's `f32` samples: 44 bytes of
/// header and one conversion.
pub fn pcm16_wav(audio: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (audio.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in audio {
        let scaled = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&scaled.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::pcm16_wav;
    use std::io::Cursor;

    #[test]
    fn the_header_says_mono_16_bit_at_the_given_rate() {
        let bytes = pcm16_wav(&[0.0, 0.5, -0.5], 16_000);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[4..8], &(36u32 + 6).to_le_bytes());
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(bytes.len(), 44 + 3 * 2);
        let reader = hound::WavReader::new(Cursor::new(&bytes)).expect("a reader must accept it");
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(spec.sample_format, hound::SampleFormat::Int);
    }

    #[test]
    fn samples_scale_to_the_16_bit_range_and_clip_at_the_edges() {
        let bytes = pcm16_wav(&[1.0, -1.0, 0.0, 2.0, -2.0], 16_000);
        let samples: Vec<i16> = hound::WavReader::new(Cursor::new(&bytes))
            .unwrap()
            .samples::<i16>()
            .map(Result::unwrap)
            .collect();
        assert_eq!(samples, vec![i16::MAX, -i16::MAX, 0, i16::MAX, -i16::MAX]);
    }
}

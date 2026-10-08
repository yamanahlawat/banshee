//! What the speech server sent. The type it declared names the format, and a
//! type that names none leaves the format the request asked for. Raw samples
//! carry no header, so no byte of theirs is read as anything else.

use std::num::NonZero;

use crate::config::SpeechFormat;
use crate::text_to_speech::output::Chunk;

/// OpenAI answers `pcm` at this rate and says so nowhere in the stream, so
/// headerless samples are read at it until `tts.remote.sample_rate` names
/// another.
const ASSUMED_RATE: NonZero<u32> = NonZero::new(24_000).unwrap();
const MONO: NonZero<u16> = NonZero::new(1).unwrap();

/// `RIFF`, the size and `WAVE`. A WAV answer is not read on fewer bytes, so a
/// magic split across two reads is not misread.
const RIFF_HEADER: usize = 12;

/// Unmeasured. A ceiling on a WAV header that never reaches its samples, far
/// past any header this reads.
const HEADER_BOUND: usize = 64 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Shape {
    rate: NonZero<u32>,
    channels: NonZero<u16>,
}

/// One answer, read as samples from its first byte under PCM, or from the end
/// of its header under WAV.
pub(crate) struct Arrival {
    header: Vec<u8>,
    /// None while a WAV header is still arriving.
    shape: Option<Shape>,
    /// The byte a read ended on, half a sample short.
    carry: Option<u8>,
}

impl Arrival {
    pub(crate) fn new(format: SpeechFormat, sample_rate: Option<NonZero<u32>>) -> Self {
        let shape = match format {
            SpeechFormat::Pcm => Some(Shape {
                rate: sample_rate.unwrap_or(ASSUMED_RATE),
                channels: MONO,
            }),
            SpeechFormat::Wav => None,
        };
        Self {
            header: Vec::new(),
            shape,
            carry: None,
        }
    }

    /// Adds one read. Answers the samples it completes, or nothing while the
    /// WAV header is incomplete or a whole sample is still short a byte.
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<Option<Chunk>, String> {
        let (shape, samples) = match self.shape {
            Some(shape) => (shape, self.samples(bytes)),
            None => {
                self.header.extend_from_slice(bytes);
                match wave(&self.header)? {
                    None if self.header.len() >= HEADER_BOUND => {
                        return Err(format!(
                            "the remote speaker sent {} KiB of WAV header and never reached its samples",
                            HEADER_BOUND / 1024
                        ));
                    }
                    None => return Ok(None),
                    Some((shape, samples_at)) => {
                        self.shape = Some(shape);
                        let read = std::mem::take(&mut self.header);
                        (shape, self.samples(&read[samples_at..]))
                    }
                }
            }
        };
        Ok((!samples.is_empty()).then_some(Chunk {
            samples,
            rate: shape.rate,
            channels: shape.channels,
        }))
    }

    /// 16-bit signed little-endian samples, the first of them completed by the
    /// byte the read before ended on. A trailing odd byte waits for the read
    /// after this one.
    fn samples(&mut self, bytes: &[u8]) -> Vec<f32> {
        let Some((first, rest)) = bytes.split_first() else {
            return Vec::new();
        };
        let mut samples = Vec::with_capacity(bytes.len() / 2 + 1);
        let paired = match self.carry.take() {
            Some(held) => {
                samples.push(sample([held, *first]));
                rest
            }
            None => bytes,
        };
        let (pairs, tail) = paired.as_chunks::<2>();
        samples.extend(pairs.iter().copied().map(sample));
        self.carry = tail.first().copied();
        samples
    }

    /// An answer that ended before its samples began is refused.
    pub(crate) fn ended(&self) -> Result<(), String> {
        match self.shape {
            Some(_) => Ok(()),
            None => Err("the remote speaker's answer ended before its samples began".to_string()),
        }
    }
}

fn sample(pair: [u8; 2]) -> f32 {
    crate::audio::utils::from_pcm16(i16::from_le_bytes(pair))
}

/// The shape of the samples and the offset they start at, or None while the
/// header is still arriving. A `LIST` chunk can sit before `data`, so a fixed
/// 44-byte skip reads its text as samples.
fn wave(bytes: &[u8]) -> Result<Option<(Shape, usize)>, String> {
    if bytes.len() < RIFF_HEADER {
        return Ok(None);
    }
    if !bytes.starts_with(b"RIFF") || &bytes[8..12] != b"WAVE" {
        return Err("the remote speaker's WAV answer does not open with RIFF and WAVE".to_string());
    }
    let mut at = RIFF_HEADER;
    let mut shape = None;
    loop {
        let Some(head) = bytes.get(at..at + 8) else {
            return Ok(None);
        };
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let body = at + 8;
        match &head[..4] {
            // The size fields lie: LocalAI writes 0xFFFFFFFF, and a server that
            // streams cannot know the length it already sent. The samples run
            // to the end of the stream instead.
            b"data" => {
                return match shape {
                    Some(shape) => Ok(Some((shape, body))),
                    None => Err(
                        "the remote speaker answered with a WAVE file whose data comes before its fmt chunk"
                            .to_string(),
                    ),
                };
            }
            // The declared size is the only bound on a fmt chunk: reading 16
            // bytes out of a shorter one reads the chunk after it
            b"fmt " if size < 16 => {
                return Err(format!(
                    "the remote speaker answered with a WAVE file whose fmt chunk is {size} bytes"
                ));
            }
            b"fmt " => match bytes.get(body..body + 16) {
                None => return Ok(None),
                Some(fmt) => shape = Some(read_fmt(fmt)?),
            },
            _ => {}
        }
        // A chunk body is padded to an even length. The size is the server's
        // number, so the walk saturates rather than wraps.
        at = body.saturating_add(size).saturating_add(size % 2);
    }
}

/// The first 16 bytes of a `fmt ` chunk.
fn read_fmt(fmt: &[u8]) -> Result<Shape, String> {
    let format = u16::from_le_bytes([fmt[0], fmt[1]]);
    let channels = u16::from_le_bytes([fmt[2], fmt[3]]);
    let rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
    let bits = u16::from_le_bytes([fmt[14], fmt[15]]);
    let named = match format {
        1 if bits == 16 => None,
        1 => Some(format!("a {bits}-bit WAVE file")),
        3 => Some("an IEEE float WAVE file".to_string()),
        0xFFFE => Some("an extensible WAVE file".to_string()),
        other => Some(format!("a WAVE file in format {other}")),
    };
    if let Some(named) = named {
        return Err(format!(
            "the remote speaker answered with {named}, and Banshee reads 16-bit"
        ));
    }
    match (NonZero::new(rate), NonZero::new(channels)) {
        (Some(rate), Some(channels)) => Ok(Shape { rate, channels }),
        _ => Err(
            "the remote speaker answered with a WAVE file whose fmt chunk names no rate or no channel"
                .to_string(),
        ),
    }
}

/// How much of a declared type a reason carries. Nothing measured this number.
/// It trades how much of the server's own text the reason keeps against how
/// long a line a person has to read.
const TYPE_BOUND: usize = 60;

/// The format the declared type names. A type that names none leaves the
/// format asked for, and so does no type at all: RFC 9110 8.3 reads a missing
/// type as application/octet-stream.
pub(crate) fn declared(
    content_type: Option<&str>,
    asked: SpeechFormat,
) -> Result<SpeechFormat, String> {
    let Some(kind) = content_type
        .and_then(|value| value.split(';').next())
        .map(|kind| kind.trim().to_ascii_lowercase())
        .filter(|kind| !kind.is_empty())
    else {
        return Ok(asked);
    };
    match kind.as_str() {
        "audio/wav" | "audio/wave" | "audio/x-wav" => Ok(SpeechFormat::Wav),
        "audio/pcm" => Ok(SpeechFormat::Pcm),
        "audio/mpeg" | "audio/mp3" | "audio/x-mpeg" | "audio/aac" | "audio/x-aac"
        | "audio/flac" | "audio/x-flac" | "audio/opus" | "audio/ogg" | "audio/webm"
        | "audio/mp4" => Err(format!(
            "the remote speaker answered with {kind}, which Banshee cannot play"
        )),
        // Azure OpenAI sends application/octet-stream
        _ if kind.starts_with("audio/") || kind == "application/octet-stream" => Ok(asked),
        _ => {
            let named: String = kind.chars().take(TYPE_BOUND).collect();
            Err(format!(
                "the remote speaker answered with {named}, not audio"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Arrival, TYPE_BOUND, declared};
    use crate::config::SpeechFormat;
    use crate::text_to_speech::output::Chunk;

    fn le(values: &[i16]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        sized_chunk(id, body.len() as u32, body)
    }

    /// A chunk whose declared size is whatever the caller says, because a
    /// server in the wild writes sizes it cannot know while it streams.
    fn sized_chunk(id: &[u8; 4], size: u32, body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(body);
        if !body.len().is_multiple_of(2) {
            out.push(0);
        }
        out
    }

    fn fmt_body(format: u16, channels: u16, rate: u32, bits: u16) -> Vec<u8> {
        let block_align = channels * bits / 8;
        let byte_rate = rate * u32::from(block_align);
        [
            &format.to_le_bytes()[..],
            &channels.to_le_bytes(),
            &rate.to_le_bytes(),
            &byte_rate.to_le_bytes(),
            &block_align.to_le_bytes(),
            &bits.to_le_bytes(),
        ]
        .concat()
    }

    fn riff(parts: &[Vec<u8>]) -> Vec<u8> {
        let body = parts.concat();
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32 + 4).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(&body);
        out
    }

    fn wav(rate: u32, channels: u16, values: &[i16]) -> Vec<u8> {
        riff(&[
            chunk(b"fmt ", &fmt_body(1, channels, rate, 16)),
            chunk(b"data", &le(values)),
        ])
    }

    /// Feeds every piece in turn, then ends the stream.
    fn read(asked: SpeechFormat, pieces: &[&[u8]]) -> Result<Vec<Chunk>, String> {
        let mut arrival = Arrival::new(asked, None);
        let mut heard = Vec::new();
        for piece in pieces {
            if let Some(chunk) = arrival.feed(piece)? {
                heard.push(chunk);
            }
        }
        arrival.ended()?;
        Ok(heard)
    }

    fn played(chunks: &[Chunk]) -> Vec<f32> {
        chunks
            .iter()
            .flat_map(|chunk| chunk.samples.clone())
            .collect()
    }

    fn refusal(bytes: &[u8]) -> String {
        match read(SpeechFormat::Wav, &[bytes]) {
            Err(reason) => reason,
            Ok(chunks) => panic!("these bytes must be refused, not played: {chunks:?}"),
        }
    }

    #[test]
    fn a_wav_says_its_rate_its_channels_and_its_samples() {
        let bytes = wav(24_000, 1, &[0, 16_384, -16_384]);
        let heard = read(SpeechFormat::Wav, &[&bytes]).unwrap();
        assert_eq!(played(&heard), vec![0.0, 0.5, -0.5]);
        assert_eq!(heard[0].rate.get(), 24_000);
        assert_eq!(heard[0].channels.get(), 1);
    }

    // A fixed 44-byte skip would read this chunk's text as samples.
    #[test]
    fn a_list_chunk_before_the_data_is_walked_over() {
        let bytes = riff(&[
            chunk(b"fmt ", &fmt_body(1, 1, 24_000, 16)),
            chunk(b"LIST", b"INFOISFTLavf61"),
            chunk(b"data", &le(&[16_384])),
        ]);
        let heard = read(SpeechFormat::Wav, &[&bytes]).unwrap();
        assert_eq!(played(&heard), vec![0.5]);
    }

    #[test]
    fn a_header_split_across_reads_is_read() {
        let bytes = wav(22_050, 1, &[16_384, -16_384]);
        // Across the magic, across `WAVE`, and across the data chunk's header
        for cuts in [vec![6], vec![3, 9], vec![20, 40]] {
            let mut pieces = Vec::new();
            let mut rest = bytes.as_slice();
            let mut taken = 0;
            for cut in &cuts {
                let (piece, tail) = rest.split_at(cut - taken);
                pieces.push(piece);
                rest = tail;
                taken = *cut;
            }
            pieces.push(rest);
            let heard = read(SpeechFormat::Wav, &pieces).unwrap();
            assert_eq!(played(&heard), vec![0.5, -0.5], "cut at {cuts:?}");
            assert_eq!(heard[0].rate.get(), 22_050, "cut at {cuts:?}");
        }
    }

    // LocalAI writes 0xFFFFFFFF, and a streamed WAV cannot know its length, so
    // the only honest length is the end of the stream.
    #[test]
    fn a_data_size_that_lies_is_ignored_and_every_sample_is_read() {
        for size in [0, u32::MAX] {
            let bytes = riff(&[
                chunk(b"fmt ", &fmt_body(1, 1, 24_000, 16)),
                sized_chunk(b"data", size, &le(&[16_384, -16_384, 16_384])),
            ]);
            let heard = read(SpeechFormat::Wav, &[&bytes]).unwrap();
            assert_eq!(played(&heard), vec![0.5, -0.5, 0.5], "size {size}");
        }
    }

    #[test]
    fn a_wav_that_is_not_16_bit_pcm_is_refused_and_names_its_own_case() {
        let cases = [
            (fmt_body(1, 1, 24_000, 8), "8-bit"),
            (fmt_body(1, 1, 24_000, 24), "24-bit"),
            (fmt_body(3, 1, 24_000, 32), "IEEE float"),
            (fmt_body(0xFFFE, 1, 24_000, 16), "extensible"),
        ];
        for (fmt, named) in cases {
            let bytes = riff(&[chunk(b"fmt ", &fmt), chunk(b"data", &le(&[1]))]);
            let reason = refusal(&bytes);
            assert!(reason.contains(named), "{reason}");
        }
    }

    #[test]
    fn a_wav_answer_that_does_not_open_with_riff_and_wave_is_refused() {
        let samples = le(&[16_384, -16_384, 0, 4_096, 8_192, 0]);
        let not_wave: &[u8] = b"RIFF\x24\0\0\0AVI LIST";
        for bytes in [samples.as_slice(), not_wave] {
            let reason = refusal(bytes);
            assert!(reason.contains("RIFF and WAVE"), "{reason}");
        }
    }

    // OmniVoice opens quiet replies on `ff ff`, which is also how an MP3 frame
    // starts.
    #[test]
    fn pcm_plays_whatever_its_bytes_look_like() {
        let openings: [&[u8]; 5] = [
            &le(&[-1, -2, -3, 0, 2, -1]),
            b"\xff\xfb\x90\xc4\x00\x00\x14\x6d\xe0\xee\x07\xa4",
            b"ID3\x04\0\0\0\0\0\x23TSSE",
            b"OggS\0\x02\0\0\0\0\0\0",
            br#"{"error":{"message":"no"}}"#,
        ];
        for bytes in openings {
            let heard = read(SpeechFormat::Pcm, &[bytes])
                .unwrap_or_else(|reason| panic!("{bytes:x?}: {reason}"));
            assert_eq!(played(&heard).len(), bytes.len() / 2, "{bytes:x?}");
        }
    }

    // A WAVE header that a lying content length cut in half is not audio.
    #[test]
    fn a_stream_that_ends_mid_header_is_refused() {
        let bytes = wav(24_000, 1, &[16_384]);
        let reason = refusal(&bytes[..20]);
        assert!(reason.contains("ended"), "{reason}");
    }

    #[test]
    fn a_header_that_never_ends_is_refused_at_the_bound() {
        let mut bytes = riff(&[chunk(b"fmt ", &fmt_body(1, 1, 24_000, 16))]);
        while bytes.len() < 64 * 1024 {
            bytes.extend_from_slice(&chunk(b"junk", b""));
        }
        let reason = refusal(&bytes);
        assert!(reason.contains("64 KiB"), "{reason}");
    }

    // Nothing says what the samples are until the fmt chunk does.
    #[test]
    fn a_wav_whose_data_comes_first_is_refused() {
        let bytes = riff(&[
            chunk(b"data", &le(&[16_384])),
            chunk(b"fmt ", &fmt_body(1, 1, 24_000, 16)),
        ]);
        let reason = refusal(&bytes);
        assert!(reason.contains("data comes before"), "{reason}");
    }

    // A chunk body of odd length is padded to an even one, and the pad byte
    // belongs to no chunk.
    #[test]
    fn an_odd_chunk_body_is_padded_before_the_chunk_after_it() {
        let odd = b"INFOISFTLavf6";
        assert!(!odd.len().is_multiple_of(2), "the LIST body is odd");
        let bytes = riff(&[
            chunk(b"fmt ", &fmt_body(1, 1, 24_000, 16)),
            chunk(b"LIST", odd),
            chunk(b"data", &le(&[16_384, -16_384])),
        ]);
        let heard = read(SpeechFormat::Wav, &[&bytes]).unwrap();
        assert_eq!(played(&heard), vec![0.5, -0.5]);
    }

    // M7 in the review: a fmt chunk shorter than its 16 bytes.
    #[test]
    fn a_fmt_chunk_too_short_to_read_is_refused() {
        let bytes = riff(&[
            sized_chunk(b"fmt ", 8, &fmt_body(1, 1, 24_000, 16)),
            chunk(b"data", &le(&[16_384])),
        ]);
        let reason = refusal(&bytes);
        assert!(reason.contains("8 bytes"), "{reason}");
    }

    #[test]
    fn a_stereo_wav_reports_two_channels() {
        let bytes = wav(48_000, 2, &[16_384, -16_384]);
        let heard = read(SpeechFormat::Wav, &[&bytes]).unwrap();
        assert_eq!(heard[0].channels.get(), 2);
        assert_eq!(heard[0].rate.get(), 48_000);
    }

    #[test]
    fn headerless_samples_take_the_rate_the_config_names() {
        let mut arrival = Arrival::new(SpeechFormat::Pcm, std::num::NonZero::new(16_000));
        let chunk = arrival.feed(&le(&[0, 1, 2, 3, 4, 5])).unwrap().unwrap();
        assert_eq!(chunk.rate.get(), 16_000);
    }

    // 16-bit samples cross a read boundary, so a trailing odd byte has to wait
    // rather than shift every sample after it by one byte.
    #[test]
    fn an_odd_trailing_byte_waits_for_the_next_chunk() {
        let mut arrival = Arrival::new(SpeechFormat::Pcm, None);
        let first = arrival.feed(&[
            0x00, 0x40, 0x00, 0x40, 0x00, 0x40, 0x00, 0x40, 0x00, 0x40, 0x00, 0x40, 0x00,
        ]);
        assert_eq!(first.unwrap().unwrap().samples, vec![0.5; 6]);
        let second = arrival.feed(&[0xc0]).unwrap().unwrap();
        assert_eq!(second.samples, vec![-0.5]);
    }

    #[test]
    fn a_declared_type_that_is_not_audio_is_refused_by_name() {
        let reason = declared(Some("application/json; charset=utf-8"), SpeechFormat::Pcm)
            .expect_err("a JSON answer is not audio");
        assert!(reason.contains("application/json"), "{reason}");
    }

    // tts.ai and audio.cpp answer a `pcm` request with a WAV file, and say so.
    #[test]
    fn the_declared_type_names_the_format_whatever_was_asked_for() {
        for asked in [SpeechFormat::Pcm, SpeechFormat::Wav] {
            for kind in [
                "audio/wav",
                "AUDIO/WAV; charset=binary",
                "audio/wave",
                "audio/x-wav",
            ] {
                assert_eq!(declared(Some(kind), asked), Ok(SpeechFormat::Wav), "{kind}");
            }
            assert_eq!(declared(Some("audio/pcm"), asked), Ok(SpeechFormat::Pcm));
        }
    }

    // RFC 9110 8.3 reads a missing type as application/octet-stream, which
    // names no format, and audio.cpp streams its samples under it.
    #[test]
    fn a_type_that_names_no_format_leaves_the_one_asked_for() {
        for asked in [SpeechFormat::Pcm, SpeechFormat::Wav] {
            for kind in [None, Some(""), Some("  "), Some("application/octet-stream")] {
                assert_eq!(declared(kind, asked), Ok(asked), "{kind:?}");
            }
        }
    }

    // The types Kokoro-FastAPI declares for its other formats.
    #[test]
    fn a_format_with_no_decoder_here_is_refused_by_its_type() {
        for kind in ["audio/mpeg", "audio/aac", "audio/flac", "audio/opus"] {
            for asked in [SpeechFormat::Pcm, SpeechFormat::Wav] {
                let reason = declared(Some(kind), asked).expect_err("no decoder here");
                assert!(reason.contains(kind), "{kind}: {reason}");
            }
        }
    }

    // The header is the server's text, and the reason it lands in is read by a
    // person.
    #[test]
    fn a_type_longer_than_a_reason_is_cut() {
        let shouted = "text/".to_string() + &"long".repeat(500);
        let reason = declared(Some(&shouted), SpeechFormat::Pcm).expect_err("text is not audio");
        let named = reason
            .strip_prefix("the remote speaker answered with ")
            .and_then(|rest| rest.strip_suffix(", not audio"))
            .expect("the reason names the type");
        assert_eq!(named.chars().count(), TYPE_BOUND, "{reason}");
        assert!(named.starts_with("text/longlong"), "{reason}");
    }
}

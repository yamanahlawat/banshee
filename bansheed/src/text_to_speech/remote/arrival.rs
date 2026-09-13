//! What the speech server actually sent. Raw samples are the one thing no byte
//! can prove, so they are read only when the request asked for them.

use std::num::NonZero;

use crate::config::SpeechFormat;
use crate::text_to_speech::output::Chunk;

/// OpenAI answers `pcm` at this rate and says so nowhere in the stream, so
/// headerless samples are read at it until `tts.remote.sample_rate` names
/// another.
const ASSUMED_RATE: NonZero<u32> = NonZero::new(24_000).unwrap();
const MONO: NonZero<u16> = NonZero::new(1).unwrap();

/// `RIFF`, the size and `WAVE`. Nothing is identified on fewer bytes than the
/// longest signature, so a magic split across two reads is not misread.
const ENOUGH: usize = 12;

/// The bound stops a server that never declares itself from filling memory.
const HEADER_BOUND: usize = 64 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Shape {
    rate: NonZero<u32>,
    channels: NonZero<u16>,
}

#[derive(PartialEq, Eq, Debug)]
enum Verdict {
    /// The caller feeds the next read and asks again.
    Undecided,
    /// 16-bit little-endian samples in this shape.
    Samples {
        shape: Shape,
        samples_at: usize,
    },
    Refused(String),
}

/// One answer identifies what arrived, then turns every byte after the header
/// into samples.
pub(crate) struct Arrival {
    asked: SpeechFormat,
    assumed: NonZero<u32>,
    unidentified: Vec<u8>,
    shape: Option<Shape>,
    /// The byte a read ended on, half a sample short.
    carry: Option<u8>,
}

impl Arrival {
    pub(crate) fn new(asked: SpeechFormat, sample_rate: Option<NonZero<u32>>) -> Self {
        Self {
            asked,
            assumed: sample_rate.unwrap_or(ASSUMED_RATE),
            unidentified: Vec::new(),
            shape: None,
            carry: None,
        }
    }

    /// Adds one read. Answers the samples it completes, or nothing while the
    /// answer is unidentified or a whole sample is still short a byte.
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<Option<Chunk>, String> {
        let (shape, samples) = match self.shape {
            Some(shape) => (shape, self.samples(bytes)),
            None => {
                self.unidentified.extend_from_slice(bytes);
                match identify(&self.unidentified, self.asked, self.assumed) {
                    Verdict::Refused(reason) => return Err(reason),
                    Verdict::Undecided if self.unidentified.len() >= HEADER_BOUND => {
                        return Err(format!(
                            "the remote speaker sent {} KiB and never said what the audio is",
                            HEADER_BOUND / 1024
                        ));
                    }
                    Verdict::Undecided => return Ok(None),
                    Verdict::Samples { shape, samples_at } => {
                        self.shape = Some(shape);
                        let identified = std::mem::take(&mut self.unidentified);
                        (shape, self.samples(&identified[samples_at..]))
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

    /// An answer that never said what it was is refused: nothing identified
    /// it, so nothing may play it.
    pub(crate) fn ended(&self) -> Result<(), String> {
        match self.shape {
            Some(_) => Ok(()),
            None => Err(
                "the remote speaker's answer ended before it said what the audio is".to_string(),
            ),
        }
    }
}

fn sample(pair: [u8; 2]) -> f32 {
    f32::from(i16::from_le_bytes(pair)) / 32_768.0
}

fn refused(what: &str, asked: SpeechFormat) -> Verdict {
    Verdict::Refused(format!(
        "the remote speaker answered with {what}, not the {asked} it was asked for"
    ))
}

/// What the leading bytes say the answer is.
fn identify(bytes: &[u8], asked: SpeechFormat, assumed: NonZero<u32>) -> Verdict {
    if bytes.len() < ENOUGH {
        return Verdict::Undecided;
    }
    if bytes.starts_with(b"RIFF") {
        return match &bytes[8..12] {
            b"WAVE" => wave(bytes),
            _ => refused("a RIFF file that is not WAVE", asked),
        };
    }
    if let Some(container) = container(bytes) {
        return refused(container, asked);
    }
    if let Some(short) = short_signature(bytes) {
        return refused(short, asked);
    }
    match asked {
        SpeechFormat::Pcm => Verdict::Samples {
            shape: Shape {
                rate: assumed,
                channels: MONO,
            },
            samples_at: 0,
        },
        SpeechFormat::Wav => refused("something else", asked),
    }
}

/// The four-byte signatures. Each one is two samples that no speech starts
/// with.
fn container(bytes: &[u8]) -> Option<&'static str> {
    match &bytes[..4] {
        b"OggS" => Some("Ogg audio"),
        b"fLaC" => Some("FLAC audio"),
        _ if &bytes[4..8] == b"ftyp" => Some("MP4 or AAC audio"),
        _ => None,
    }
}

/// The signatures under four bytes. The sync word matches 32 first samples,
/// -7937 to -1 in steps of 256, and -1 is near silence, which is where a
/// stream opens. It stays, because a collision costs one utterance and the
/// other mistake plays a whole reply as noise. The one-byte ones are weaker
/// still, so a document has to read as one as well.
fn short_signature(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"ID3") {
        return Some("MP3 audio");
    }
    // The eleven sync bits an MP3 frame opens with
    if let [0xFF, second, ..] = bytes
        && second & 0xE0 == 0xE0
    {
        return Some("MP3 audio");
    }
    let document = bytes.trim_ascii_start();
    // A short window, because a page in another language carries its first
    // non-ASCII byte early
    let window = &document[..document.len().min(ENOUGH)];
    match document {
        [b'{' | b'[', ..] if text(window) => Some("JSON"),
        [b'<', ..] if text(window) => Some("HTML or XML"),
        _ => None,
    }
}

/// Every byte reads as printable ASCII, or as the whitespace a document is
/// laid out with.
fn text(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .all(|byte| matches!(byte, 0x20..=0x7E | b'\t' | b'\n' | b'\r'))
}

/// Walks the chunks from offset 12. A `LIST` chunk can sit before `data`, so a
/// fixed 44-byte skip reads its text as samples.
fn wave(bytes: &[u8]) -> Verdict {
    let mut at = ENOUGH;
    let mut shape = None;
    loop {
        let Some(head) = bytes.get(at..at + 8) else {
            return Verdict::Undecided;
        };
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let body = at + 8;
        match &head[..4] {
            // The size fields lie: LocalAI writes 0xFFFFFFFF, and a server that
            // streams cannot know the length it already sent. The samples run
            // to the end of the stream instead.
            b"data" => {
                return match shape {
                    Some(shape) => Verdict::Samples { shape, samples_at: body },
                    None => Verdict::Refused(
                        "the remote speaker answered with a WAVE file whose data comes before its fmt chunk"
                            .to_string(),
                    ),
                };
            }
            // The declared size is the only bound on a fmt chunk: reading 16
            // bytes out of a shorter one reads the chunk after it
            b"fmt " if size < 16 => {
                return Verdict::Refused(format!(
                    "the remote speaker answered with a WAVE file whose fmt chunk is {size} bytes"
                ));
            }
            b"fmt " => match bytes.get(body..body + 16) {
                None => return Verdict::Undecided,
                Some(fmt) => match read_fmt(fmt) {
                    Ok(read) => shape = Some(read),
                    Err(reason) => return Verdict::Refused(reason),
                },
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

/// The type the server declared. It corroborates the bytes and catches a 200
/// carrying an error body, and it is never the decision on its own. A server
/// that declared nothing leaves the bytes to say what arrived: RFC 9110 8.3
/// lets a recipient read a missing type as application/octet-stream, which is
/// a type this takes.
pub(crate) fn declared(content_type: Option<&str>) -> Result<(), String> {
    let Some(kind) = content_type
        .and_then(|value| value.split(';').next())
        .map(|kind| kind.trim().to_ascii_lowercase())
        .filter(|kind| !kind.is_empty())
    else {
        return Ok(());
    };
    // Azure OpenAI sends the second one
    if kind.starts_with("audio/") || kind == "application/octet-stream" {
        return Ok(());
    }
    let named: String = kind.chars().take(TYPE_BOUND).collect();
    Err(format!(
        "the remote speaker answered with {named}, not audio"
    ))
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

    fn refusal(asked: SpeechFormat, bytes: &[u8]) -> String {
        match read(asked, &[bytes]) {
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
            let reason = refusal(SpeechFormat::Wav, &bytes);
            assert!(reason.contains(named), "{reason}");
        }
    }

    #[test]
    fn every_format_with_no_decoder_here_is_refused_by_name() {
        let cases: [(&[u8], &str); 7] = [
            (b"OggS\0\x02\0\0\0\0\0\0", "Ogg"),
            (b"fLaC\0\0\0\x22\x12\x00\x12\x00", "FLAC"),
            (b"ID3\x04\0\0\0\0\0\x23TSSE", "MP3"),
            (b"\xff\xfb\x90\x64\0\0\0\0\0\0\0\0", "MP3"),
            (b"\0\0\0\x20ftypM4A \0\0\x02\0", "MP4 or AAC"),
            (b"{\"error\":{\"message\":\"no\"}}", "JSON"),
            (b"<!DOCTYPE html><html><head>", "HTML or XML"),
        ];
        for (bytes, named) in cases {
            let reason = refusal(SpeechFormat::Wav, bytes);
            assert!(reason.contains(named), "{named}: {reason}");
        }
    }

    // Those four bytes alone are two legal samples, so `WAVE` has to be there
    // as well.
    #[test]
    fn riff_without_wave_is_refused() {
        let reason = refusal(SpeechFormat::Wav, b"RIFF\x24\0\0\0AVI LIST");
        assert!(reason.contains("RIFF"), "{reason}");
        assert!(reason.contains("WAVE"), "{reason}");
    }

    #[test]
    fn unrecognised_bytes_play_only_when_pcm_was_asked_for() {
        let bytes = le(&[16_384, -16_384, 0, 4_096, 8_192, 0]);
        let heard = read(SpeechFormat::Pcm, &[&bytes]).unwrap();
        assert_eq!(heard[0].rate.get(), 24_000, "the assumed rate");
        assert_eq!(heard[0].channels.get(), 1);
        assert_eq!(played(&heard).len(), 6);

        let reason = refusal(SpeechFormat::Wav, &bytes);
        assert!(reason.contains("wav"), "{reason}");
    }

    // Speech opens with silence, which every encoder writes as zero bytes, so
    // an MP3 that arrives where samples were asked for is an MP3, and not a
    // loud first sample.
    #[test]
    fn an_mp3_is_refused_whatever_was_asked_for() {
        let sync = le(&[-1_025, 25_708, 0, 0, 0, 0]);
        assert_eq!(&sync[..2], b"\xff\xfb", "the bytes read as a sync word");
        for asked in [SpeechFormat::Wav, SpeechFormat::Pcm] {
            assert!(refusal(asked, &sync).contains("MP3"), "{asked}");
            let tagged = b"ID3\x04\0\0\0\0\0\x23TSSE";
            assert!(refusal(asked, tagged).contains("MP3"), "{asked}");
        }
    }

    // The 0xFF the sync word opens with is also the low byte of 128 samples,
    // and only 32 of them carry the sync bits. The other 127 play.
    #[test]
    fn a_sample_that_opens_with_ff_and_no_sync_bits_plays() {
        let bytes = le(&[0x00FF, 0x10FF, -32_768, 0, 0, 0]);
        assert_eq!(bytes[0], 0xFF, "the low byte is the sync word's");
        assert_eq!(bytes[1] & 0xE0, 0x00, "and the bits after it are not");
        let heard = read(SpeechFormat::Pcm, &[&bytes]).unwrap();
        assert_eq!(played(&heard).len(), 6);
    }

    // One leading byte is weak evidence, so the bytes after it have to read as
    // text as well. Samples that open with 0x7B are samples.
    #[test]
    fn a_one_byte_signature_needs_the_text_that_follows_it() {
        let error_page = br#"{"error":{"message":"no such voice"}}"#;
        // A document is laid out, so the brace is not always the first byte
        let indented = b"\n\t  <!DOCTYPE html><html><head>";
        for asked in [SpeechFormat::Wav, SpeechFormat::Pcm] {
            assert!(refusal(asked, error_page).contains("JSON"), "{asked}");
            assert!(refusal(asked, indented).contains("HTML"), "{asked}");
        }

        let samples = le(&[0x007B, 0x003C, -32_768, 4_096, 0, 0]);
        assert_eq!(samples[0], b'{');
        assert_eq!(samples[2], b'<');
        let heard = read(SpeechFormat::Pcm, &[&samples]).unwrap();
        assert_eq!(played(&heard).len(), 6, "these are samples, not a document");
    }

    // A WAVE header that a lying content length cut in half is not audio.
    #[test]
    fn a_stream_that_ends_mid_header_is_refused() {
        let bytes = wav(24_000, 1, &[16_384]);
        let reason = refusal(SpeechFormat::Wav, &bytes[..20]);
        assert!(reason.contains("ended"), "{reason}");
    }

    #[test]
    fn a_header_that_never_ends_is_refused_at_the_bound() {
        let mut bytes = riff(&[chunk(b"fmt ", &fmt_body(1, 1, 24_000, 16))]);
        while bytes.len() < 64 * 1024 {
            bytes.extend_from_slice(&chunk(b"junk", b""));
        }
        let reason = refusal(SpeechFormat::Wav, &bytes);
        assert!(reason.contains("64 KiB"), "{reason}");
    }

    // Nothing says what the samples are until the fmt chunk does.
    #[test]
    fn a_wav_whose_data_comes_first_is_refused() {
        let bytes = riff(&[
            chunk(b"data", &le(&[16_384])),
            chunk(b"fmt ", &fmt_body(1, 1, 24_000, 16)),
        ]);
        let reason = refusal(SpeechFormat::Wav, &bytes);
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
        let reason = refusal(SpeechFormat::Wav, &bytes);
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
        let reason = declared(Some("application/json; charset=utf-8"))
            .expect_err("a JSON answer is not audio");
        assert!(reason.contains("application/json"), "{reason}");
    }

    #[test]
    fn the_types_a_speech_server_declares_are_taken() {
        for kind in [
            "audio/wav",
            "audio/pcm",
            "audio/L16; rate=24000",
            "application/octet-stream",
            "AUDIO/WAV",
        ] {
            assert!(declared(Some(kind)).is_ok(), "{kind}");
        }
    }

    // Absence is not evidence. RFC 9110 8.3 reads a missing type as
    // application/octet-stream, which this function takes, so the bytes decide.
    #[test]
    fn an_answer_with_no_declared_type_leaves_the_bytes_to_say() {
        assert!(declared(None).is_ok());
        assert!(declared(Some("")).is_ok());
        assert!(declared(Some("  ")).is_ok());
    }

    // The header is the server's text, and the reason it lands in is read by a
    // person.
    #[test]
    fn a_type_longer_than_a_reason_is_cut() {
        let shouted = "text/".to_string() + &"long".repeat(500);
        let reason = declared(Some(&shouted)).expect_err("text is not audio");
        let named = reason
            .strip_prefix("the remote speaker answered with ")
            .and_then(|rest| rest.strip_suffix(", not audio"))
            .expect("the reason names the type");
        assert_eq!(named.chars().count(), TYPE_BOUND, "{reason}");
        assert!(named.starts_with("text/longlong"), "{reason}");
    }
}

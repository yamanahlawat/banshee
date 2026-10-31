use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use banshee_common::{COMPANION_VOICE, error::BansheeError};
use ort::session::{Session, SessionInputValue};
use ort::value::{DynValue, PrimitiveTensorElementType, Tensor};
use serde::Deserialize;
use tokenizers::Tokenizer;

use super::chunk_or_fault;
use crate::text_to_speech::output::{Chunk, Output};
use crate::text_to_speech::{ActiveUtterance, Faults, TtsBackend, lock};

/// The `mimi_encoder.onnx` embedding of the companion's reference clip, [119, 1024] f32.
const COMPANION: &[u8] = include_bytes!("../../../assets/companion.npy");

const LATENT_DIM: usize = 32;
const CONDITIONING_DIM: usize = 1024;
const FRAME_RATE: f64 = 12.5;
const DECODING_STEPS: usize = 4;
const TEMPERATURE: f32 = 0.3;
const EOS_THRESHOLD: f32 = -4.0;
const MIN_FRAMES_BEFORE_EOS: usize = 6;
const FIRST_CHUNK_FRAMES: usize = 2;
const CHUNK_FRAMES: usize = 12;
const SEED: u64 = 0x5eed;

fn fault(error: impl std::fmt::Display) -> BansheeError {
    BansheeError::Other(error.to_string())
}

#[derive(Deserialize)]
struct Bundle {
    flow_lm_state_manifest: Vec<StateEntry>,
    mimi_state_manifest: Vec<StateEntry>,
    max_token_per_chunk: usize,
}

fn read_bundle(json: &[u8]) -> Result<Bundle, BansheeError> {
    serde_json::from_slice(json).map_err(|error| fault(format!("bundle.json: {error}")))
}

/// One state tensor a model takes in and hands back after each run.
#[derive(Deserialize)]
struct StateEntry {
    #[serde(rename = "input_name")]
    input: String,
    #[serde(rename = "output_name")]
    output: String,
    dtype: Dtype,
    fill: Fill,
    shape: Vec<i64>,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Dtype {
    Float32,
    Int64,
    Bool,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Fill {
    Nan,
    Empty,
    Zeros,
    Ones,
}

impl StateEntry {
    fn initial(&self) -> Result<DynValue, BansheeError> {
        let len = usize::try_from(self.shape.iter().product::<i64>()).map_err(fault)?;
        let shape = self.shape.clone();
        match self.dtype {
            Dtype::Float32 => {
                let value = match self.fill {
                    Fill::Nan => f32::NAN,
                    Fill::Ones => 1.0,
                    Fill::Empty | Fill::Zeros => 0.0,
                };
                tensor(shape, vec![value; len])
            }
            Dtype::Int64 => tensor(shape, vec![i64::from(self.fill == Fill::Ones); len]),
            Dtype::Bool => tensor(shape, vec![self.fill == Fill::Ones; len]),
        }
    }
}

/// A model's state tensors: the input each takes, and where a run hands each back.
struct States {
    entries: Vec<StateEntry>,
    outputs: Vec<usize>,
}

impl States {
    fn new(entries: Vec<StateEntry>, model: &Model) -> Result<Self, BansheeError> {
        let outputs = entries
            .iter()
            .map(|entry| model.output_index(&entry.output))
            .collect::<Result<_, _>>()?;
        Ok(Self { entries, outputs })
    }

    fn initial(&self) -> Result<Vec<DynValue>, BansheeError> {
        self.entries.iter().map(StateEntry::initial).collect()
    }

    fn inputs<'v>(
        &'v self,
        state: &'v [DynValue],
    ) -> impl Iterator<Item = (&'v str, SessionInputValue<'v>)> {
        self.entries
            .iter()
            .zip(state)
            .map(|(entry, value)| (entry.input.as_str(), value.into()))
    }

    /// The state a run hands back, out of its outputs in session order.
    fn handed_back(&self, outputs: Vec<DynValue>) -> Result<Vec<DynValue>, BansheeError> {
        let mut outputs: Vec<Option<DynValue>> = outputs.into_iter().map(Some).collect();
        self.outputs
            .iter()
            .map(|&index| {
                outputs
                    .get_mut(index)
                    .and_then(Option::take)
                    .ok_or_else(|| fault(format!("the model gave no output {index}")))
            })
            .collect()
    }
}

fn tensor<T: PrimitiveTensorElementType + std::fmt::Debug + Clone + 'static>(
    shape: Vec<i64>,
    data: Vec<T>,
) -> Result<DynValue, BansheeError> {
    Tensor::from_array((shape, data))
        .map(Tensor::into_dyn)
        .map_err(fault)
}

fn floats(value: &DynValue) -> Result<Vec<f32>, BansheeError> {
    let (_, data) = value.try_extract_tensor::<f32>().map_err(fault)?;
    Ok(data.to_vec())
}

fn output(outputs: &[DynValue], index: usize) -> Result<&DynValue, BansheeError> {
    outputs
        .get(index)
        .ok_or_else(|| fault(format!("the model gave no output {index}")))
}

/// The shape of a version 1.0 npy file and its little-endian f32 values.
fn read_npy(bytes: &[u8]) -> Result<(Vec<usize>, Vec<f32>), BansheeError> {
    let malformed = |what: &str| fault(format!("not an npy 1.0 file of f32: {what}"));
    let rest = bytes
        .strip_prefix(b"\x93NUMPY\x01\x00")
        .ok_or_else(|| malformed("no npy 1.0 magic"))?;
    let (length, rest) = rest
        .split_first_chunk::<2>()
        .ok_or_else(|| malformed("no header length"))?;
    let (header, data) = rest
        .split_at_checked(usize::from(u16::from_le_bytes(*length)))
        .ok_or_else(|| malformed("the header runs past the end"))?;
    let shape = std::str::from_utf8(header)
        .ok()
        .and_then(|header| header.split_once("'shape': (")?.1.split_once(')'))
        .ok_or_else(|| malformed("no shape"))?
        .0
        .split(',')
        .map(str::trim)
        .filter(|dim| !dim.is_empty())
        .map(str::parse)
        .collect::<Result<Vec<usize>, _>>()
        .map_err(|_| malformed("a shape that is not numbers"))?;
    let (values, tail) = data.as_chunks::<4>();
    if !tail.is_empty() || values.len() != shape.iter().product::<usize>() {
        return Err(malformed("data that does not fill the shape"));
    }
    Ok((
        shape,
        values
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes))
            .collect(),
    ))
}

fn token_ids(tokenizer: &Tokenizer, text: &str) -> Result<Vec<u32>, BansheeError> {
    let encoding = tokenizer.encode(text, false).map_err(fault)?;
    Ok(encoding.get_ids().to_vec())
}

/// The tokenizer falls back to bytes, so "…" or an emoji still yields ids; the
/// model reads those as noise.
fn speakable(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

/// The reference's `_prepare_text_prompt`, or `None` for blank text. The
/// bundle's `remove_semicolons` and `pad_with_spaces_for_short_inputs` are false.
fn prepare(text: &str) -> Option<String> {
    let text = text.trim().replace(['\n', '\r'], " ").replace("  ", " ");
    let mut chars = text.chars();
    let first = chars.next()?;
    let mut prepared: String = first.to_uppercase().chain(chars).collect();
    if prepared
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
    {
        prepared.push('.');
    }
    Some(prepared)
}

/// The reference's `_find_boundary_indices`: a segment ends after each run of
/// end tokens.
fn boundaries(tokens: &[u32], ends: &HashSet<u32>) -> Vec<usize> {
    let mut indices = vec![0];
    let mut after_an_end = false;
    for (index, token) in tokens.iter().enumerate() {
        if ends.contains(token) {
            after_an_end = true;
        } else {
            if after_an_end {
                indices.push(index);
            }
            after_an_end = false;
        }
    }
    indices.push(tokens.len());
    indices
}

/// The reference's packing: segments join while their token counts fit the limit.
fn pack(segments: Vec<(usize, String)>, limit: usize) -> Vec<(usize, String)> {
    let mut packed = Vec::new();
    let mut current = (0, String::new());
    for (count, text) in segments {
        if current.1.is_empty() {
            current = (count, text);
        } else if current.0 + count > limit {
            let full = std::mem::replace(&mut current, (count, text));
            packed.push((full.0, full.1.trim().to_string()));
        } else {
            current.1.push(' ');
            current.1.push_str(&text);
            current.0 += count;
        }
    }
    if !current.1.is_empty() {
        packed.push((current.0, current.1.trim().to_string()));
    }
    packed
}

/// The reference's `_split_into_best_sentences`, which cuts a reply into the
/// pieces the model speaks one at a time.
struct Splitter {
    tokenizer: Tokenizer,
    limit: usize,
    sentence_ends: HashSet<u32>,
    clause_ends: HashSet<u32>,
}

impl Splitter {
    fn new(tokenizer: Tokenizer, limit: usize) -> Result<Self, BansheeError> {
        // The reference drops the first id, the word-start marker the tokenizer prepends.
        let ends = |marks: &str| -> Result<HashSet<u32>, BansheeError> {
            Ok(token_ids(&tokenizer, marks)?.into_iter().skip(1).collect())
        };
        let sentence_ends = ends(".!...?")?;
        let clause_ends = ends(",;:")?;
        Ok(Self {
            tokenizer,
            limit,
            sentence_ends,
            clause_ends,
        })
    }

    fn split(&self, text: &str) -> Result<Vec<String>, BansheeError> {
        let Some(prepared) = prepare(text) else {
            return Ok(Vec::new());
        };
        let tokens = token_ids(&self.tokenizer, prepared.trim())?;
        let mut refined = Vec::new();
        for (count, segment) in self.segments(&tokens, &self.sentence_ends)? {
            if count <= self.limit {
                refined.push((count, segment));
                continue;
            }
            let clauses = self.segments(
                &token_ids(&self.tokenizer, segment.trim())?,
                &self.clause_ends,
            )?;
            if clauses.len() > 1 {
                refined.extend(clauses);
            } else {
                refined.push((count, segment));
            }
        }
        let mut within = Vec::new();
        for (count, segment) in refined {
            if count <= self.limit {
                within.push((count, segment));
            } else {
                within.extend(self.between_words(&segment)?);
            }
        }
        Ok(pack(within, self.limit)
            .into_iter()
            .map(|(_, chunk)| chunk)
            .collect())
    }

    fn segments(
        &self,
        tokens: &[u32],
        ends: &HashSet<u32>,
    ) -> Result<Vec<(usize, String)>, BansheeError> {
        boundaries(tokens, ends)
            .windows(2)
            .map(|pair| {
                let piece = &tokens[pair[0]..pair[1]];
                let text = self.tokenizer.decode(piece, true).map_err(fault)?;
                Ok((piece.len(), text))
            })
            .collect()
    }

    /// The reference keeps a segment over the limit whole, and the flow model's
    /// cache overflows from 166 tokens. This packs its words within the limit.
    fn between_words(&self, segment: &str) -> Result<Vec<(usize, String)>, BansheeError> {
        let words = segment
            .split_whitespace()
            .map(|word| Ok((token_ids(&self.tokenizer, word)?.len(), word.to_string())))
            .collect::<Result<Vec<_>, BansheeError>>()?;
        Ok(pack(words, self.limit))
    }
}

/// When a sentence stops: a few frames after the first EOS the model reports,
/// or at a cap that scales with its tokens.
struct Ending {
    cap: usize,
    after_eos: usize,
    eos_at: Option<usize>,
}

impl Ending {
    fn new(tokens: usize, words: usize) -> Self {
        Self {
            cap: ((tokens as f64 / 3.0 + 2.0) * FRAME_RATE).ceil() as usize,
            after_eos: if words <= 4 { 3 } else { 1 } + 2,
            eos_at: None,
        }
    }

    fn ends(&mut self, frame: usize, eos_logit: f32) -> bool {
        if self.eos_at.is_none() && frame >= MIN_FRAMES_BEFORE_EOS && eos_logit > EOS_THRESHOLD {
            self.eos_at = Some(frame);
        }
        self.eos_at
            .is_some_and(|eos_at| frame >= eos_at + self.after_eos)
    }
}

/// splitmix64 with a Box-Muller normal.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn uniform(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    fn normal(&mut self) -> f32 {
        let (u1, u2) = (self.uniform(), self.uniform());
        ((-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32
    }
}

struct Model {
    session: Session,
    #[cfg(test)]
    runs: usize,
}

impl Model {
    fn load(path: &Path, threads: usize) -> Result<Self, BansheeError> {
        Ok(Self {
            session: crate::models::onnx_session(path, threads, &[])?,
            #[cfg(test)]
            runs: 0,
        })
    }

    /// Where the output `name` comes in what `run` answers.
    fn output_index(&self, name: &str) -> Result<usize, BansheeError> {
        self.session
            .outputs()
            .iter()
            .position(|output| output.name() == name)
            .ok_or_else(|| fault(format!("the model has no output {name}")))
    }

    /// Every output, in session order.
    fn run(
        &mut self,
        inputs: Vec<(&str, SessionInputValue<'_>)>,
    ) -> Result<Vec<DynValue>, BansheeError> {
        #[cfg(test)]
        {
            self.runs += 1;
        }
        let outputs = self.session.run(inputs).map_err(fault)?;
        Ok(outputs.into_iter().map(|(_, value)| value).collect())
    }
}

/// One `flow_lm_main` run, every output in session order.
fn step_flow_lm(
    flow_lm_main: &mut Model,
    states: &States,
    sequence: &DynValue,
    text: &DynValue,
    state: &[DynValue],
) -> Result<Vec<DynValue>, BansheeError> {
    let mut inputs: Vec<(&str, SessionInputValue)> = vec![
        ("sequence", sequence.into()),
        ("text_embeddings", text.into()),
    ];
    inputs.extend(states.inputs(state));
    flow_lm_main.run(inputs)
}

/// One sentence partway through synthesis.
struct Sentence {
    state: Vec<DynValue>,
    decoder_state: Option<Vec<DynValue>>,
    latent: DynValue,
    undecoded: Vec<f32>,
    frame: usize,
    ending: Ending,
    finished: bool,
}

pub struct PocketEngine {
    splitter: Splitter,
    text_conditioner: Model,
    flow_lm_main: Model,
    flow_lm_flow: Model,
    decoder: Model,
    flow_states: States,
    decoder_states: States,
    voice: Vec<DynValue>,
    decoder_start: Vec<DynValue>,
    no_sequence: DynValue,
    no_text: DynValue,
    /// The `s` and `t` of each Euler step, which no frame changes.
    flow_steps: Vec<(DynValue, DynValue)>,
    rng: Rng,
}

impl PocketEngine {
    pub fn new(dir: &Path) -> Result<Self, BansheeError> {
        let read = |file: &str| {
            let path = dir.join(file);
            std::fs::read(&path).map_err(|error| BansheeError::file(&path, error))
        };
        let Bundle {
            flow_lm_state_manifest,
            mimi_state_manifest,
            max_token_per_chunk,
        } = read_bundle(&read("bundle.json")?)?;
        let tokenizer = Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|error| fault(format!("tokenizer.json: {error}")))?;
        let splitter = Splitter::new(tokenizer, max_token_per_chunk)?;

        let threads = crate::models::thread_count();
        let model = |file: &str| Model::load(&dir.join(file), threads);
        let text_conditioner = model("text_conditioner.onnx")?;
        let mut flow_lm_main = model("flow_lm_main.onnx")?;
        let flow_lm_flow = model("flow_lm_flow.onnx")?;
        let decoder = model("mimi_decoder.onnx")?;

        let decoder_states = {
            let taken: HashSet<&str> = decoder
                .session
                .inputs()
                .iter()
                .map(|input| input.name())
                .collect();
            let entries = mimi_state_manifest
                .into_iter()
                .filter(|entry| taken.contains(entry.input.as_str()))
                .collect();
            States::new(entries, &decoder)?
        };
        let decoder_start = decoder_states.initial()?;
        let flow_states = States::new(flow_lm_state_manifest, &flow_lm_main)?;

        let no_sequence = tensor(vec![1, 0, LATENT_DIM as i64], Vec::<f32>::new())?;
        let no_text = tensor(vec![1, 0, CONDITIONING_DIM as i64], Vec::<f32>::new())?;

        let (_, mut prompt) = read_npy(&read("bos_before_voice.npy")?)?;
        prompt.extend(read_npy(COMPANION)?.1);
        let prompt_frames = (prompt.len() / CONDITIONING_DIM) as i64;
        let prompt = tensor(vec![1, prompt_frames, CONDITIONING_DIM as i64], prompt)?;
        let voice = flow_states.handed_back(step_flow_lm(
            &mut flow_lm_main,
            &flow_states,
            &no_sequence,
            &prompt,
            &flow_states.initial()?,
        )?)?;

        let dt = 1.0 / DECODING_STEPS as f32;
        let flow_steps = (0..DECODING_STEPS)
            .map(|step| {
                let s = step as f32 * dt;
                Ok((
                    tensor(vec![1, 1], vec![s])?,
                    tensor(vec![1, 1], vec![s + dt])?,
                ))
            })
            .collect::<Result<_, BansheeError>>()?;

        Ok(Self {
            splitter,
            text_conditioner,
            flow_lm_main,
            flow_lm_flow,
            decoder,
            flow_states,
            decoder_states,
            voice,
            decoder_start,
            no_sequence,
            no_text,
            flow_steps,
            rng: Rng(SEED),
        })
    }

    /// The sentence after its text prompt pass, or `None` when it holds no words.
    fn begin(&mut self, text: &str) -> Result<Option<Sentence>, BansheeError> {
        let Some(prepared) = prepare(text).filter(|prepared| speakable(prepared)) else {
            return Ok(None);
        };
        let ids: Vec<i64> = token_ids(&self.splitter.tokenizer, &prepared)?
            .into_iter()
            .map(i64::from)
            .collect();
        let tokens = ids.len();
        let ids = tensor(vec![1, tokens as i64], ids)?;
        let outputs = self
            .text_conditioner
            .run(vec![("token_ids", (&ids).into())])?;
        let embeddings = floats(output(&outputs, 0)?)?;
        let frames = (embeddings.len() / CONDITIONING_DIM) as i64;
        let embeddings = tensor(vec![1, frames, CONDITIONING_DIM as i64], embeddings)?;

        let state = self.flow_states.handed_back(step_flow_lm(
            &mut self.flow_lm_main,
            &self.flow_states,
            &self.no_sequence,
            &embeddings,
            &self.voice,
        )?)?;
        Ok(Some(Sentence {
            state,
            decoder_state: None,
            latent: tensor(vec![1, 1, LATENT_DIM as i64], vec![f32::NAN; LATENT_DIM])?,
            undecoded: Vec::new(),
            frame: 0,
            ending: Ending::new(tokens, prepared.split_whitespace().count()),
            finished: false,
        }))
    }

    /// The sentence's next 24 kHz mono chunk, or `None` once it is all spoken.
    fn next_chunk(&mut self, sentence: &mut Sentence) -> Result<Option<Vec<f32>>, BansheeError> {
        while !sentence.finished {
            if sentence.frame >= sentence.ending.cap {
                sentence.finished = true;
                break;
            }
            let outputs = step_flow_lm(
                &mut self.flow_lm_main,
                &self.flow_states,
                &sentence.latent,
                &self.no_text,
                &sentence.state,
            )?;
            let (_, eos_logit) = output(&outputs, 1)?
                .try_extract_tensor::<f32>()
                .map_err(fault)?;
            let eos_logit = *eos_logit
                .first()
                .ok_or_else(|| fault("flow_lm_main gave an empty EOS logit"))?;
            if sentence.ending.ends(sentence.frame, eos_logit) {
                sentence.finished = true;
                break;
            }
            let latent = self.sample_latent(output(&outputs, 0)?)?;
            sentence.state = self.flow_states.handed_back(outputs)?;
            sentence.undecoded.extend_from_slice(&latent);
            sentence.latent = tensor(vec![1, 1, LATENT_DIM as i64], latent)?;
            sentence.frame += 1;

            let chunk_frames = if sentence.decoder_state.is_none() {
                FIRST_CHUNK_FRAMES
            } else {
                CHUNK_FRAMES
            };
            if sentence.undecoded.len() / LATENT_DIM >= chunk_frames {
                return self.decode(sentence).map(Some);
            }
        }
        if sentence.undecoded.is_empty() {
            return Ok(None);
        }
        self.decode(sentence).map(Some)
    }

    /// Euler steps of the flow from Gaussian noise to the next latent frame.
    fn sample_latent(&mut self, condition: &DynValue) -> Result<Vec<f32>, BansheeError> {
        let spread = TEMPERATURE.sqrt();
        let mut x: Vec<f32> = (0..LATENT_DIM)
            .map(|_| self.rng.normal() * spread)
            .collect();
        let dt = 1.0 / DECODING_STEPS as f32;
        for (s, t) in &self.flow_steps {
            let x_tensor = tensor(vec![1, LATENT_DIM as i64], x.clone())?;
            let outputs = self.flow_lm_flow.run(vec![
                ("c", condition.into()),
                ("s", s.into()),
                ("t", t.into()),
                ("x", (&x_tensor).into()),
            ])?;
            let flow = floats(output(&outputs, 0)?)?;
            for (xi, fi) in x.iter_mut().zip(&flow) {
                *xi += fi * dt;
            }
        }
        Ok(x)
    }

    fn decode(&mut self, sentence: &mut Sentence) -> Result<Vec<f32>, BansheeError> {
        let latents = std::mem::take(&mut sentence.undecoded);
        let frames = (latents.len() / LATENT_DIM) as i64;
        let latent = tensor(vec![1, frames, LATENT_DIM as i64], latents)?;
        let state = sentence
            .decoder_state
            .as_deref()
            .unwrap_or(&self.decoder_start);
        let mut inputs: Vec<(&str, SessionInputValue)> = vec![("latent", (&latent).into())];
        inputs.extend(self.decoder_states.inputs(state));
        let outputs = self.decoder.run(inputs)?;
        let audio = floats(output(&outputs, 0)?)?;
        sentence.decoder_state = Some(self.decoder_states.handed_back(outputs)?);
        Ok(audio)
    }

    #[cfg(test)]
    fn runs(&self) -> usize {
        [
            &self.text_conditioner,
            &self.flow_lm_main,
            &self.flow_lm_flow,
            &self.decoder,
        ]
        .iter()
        .map(|model| model.runs)
        .sum()
    }
}

fn refuse_other_voices(voice: Option<&str>) -> Result<(), BansheeError> {
    match voice {
        Some(other) if other != COMPANION_VOICE => Err(BansheeError::Rejected(format!(
            "Voice {other} is not the {COMPANION_VOICE} voice, the only one Pocket speaks."
        ))),
        _ => Ok(()),
    }
}

/// Every chunk of every sentence, synthesized only when the player pulls it,
/// so a stop ends synthesis at the next chunk boundary.
fn chunks(
    engine: Arc<Mutex<PocketEngine>>,
    text: &str,
    faults: Faults,
) -> impl Iterator<Item = Chunk> + Send + 'static {
    let text = text.to_string();
    let mut sentences = None;
    let mut speaking = None;
    let mut played = false;
    std::iter::from_fn(move || {
        let samples =
            next_samples(&mut lock(&engine), &text, &mut sentences, &mut speaking).transpose()?;
        chunk_or_fault("Pocket", samples, &mut played, &faults)
    })
}

fn next_samples(
    engine: &mut PocketEngine,
    text: &str,
    sentences: &mut Option<std::vec::IntoIter<String>>,
    speaking: &mut Option<Sentence>,
) -> Result<Option<Vec<f32>>, BansheeError> {
    let sentences = match sentences {
        Some(sentences) => sentences,
        None => sentences.insert(engine.splitter.split(text)?.into_iter()),
    };
    loop {
        if let Some(sentence) = speaking
            && let Some(samples) = engine.next_chunk(sentence)?
        {
            return Ok(Some(samples));
        }
        let Some(text) = sentences.next() else {
            return Ok(None);
        };
        *speaking = engine.begin(&text)?;
    }
}

pub struct PocketBackend {
    engine: Arc<Mutex<PocketEngine>>,
    output: Arc<Output>,
    faults: Faults,
}

impl PocketBackend {
    pub fn new(engine: PocketEngine, output: Arc<Output>, faults: Faults) -> Self {
        Self {
            engine: Arc::new(Mutex::new(engine)),
            output,
            faults,
        }
    }
}

impl TtsBackend for PocketBackend {
    fn reconfigure(&self, tts: &crate::config::TTSConfig) -> Option<String> {
        (tts.voice == COMPANION_VOICE).then(|| COMPANION_VOICE.to_string())
    }

    fn start(
        &self,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Box<dyn ActiveUtterance>, BansheeError> {
        refuse_other_voices(voice)?;
        let chunks = chunks(Arc::clone(&self.engine), text, self.faults.clone());
        Ok(Box::new(self.output.play(chunks, self.faults.clone())?))
    }
}

#[cfg(test)]
mod tests;

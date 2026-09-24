// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Chatterbox Turbo: Resemble AI's open voice, run through ONNX Runtime.
//!
//! sherpa-onnx has no such model, so this one is driven here directly,
//! the way Resemble's own reference script drives its ONNX export. Four
//! networks: a speech encoder that turns a few seconds of someone talking
//! into what the voice sounds like, a text embedding, a language model
//! that writes speech tokens one at a time with a key-value cache behind
//! it, and a decoder that turns the tokens into sound. The text can carry
//! tags the model was taught - `[laugh]`, `[chuckle]`, `[sigh]` - and it
//! reads them as what they say.
//!
//! The bundle is the quantised export, nine files and a little over a
//! gigabyte, fetched one by one into `<app data>/tts-models/<id>/onnx/`
//! under the names the graphs expect their weights beside them as. Each
//! file lands in a `.part` and is renamed whole, so a torn download is
//! never mistaken for a model.
//!
//! Every voice is a recording: there are no speakers built in. Resemble's
//! Python stamps its output with a watermark; the ONNX path carries no
//! such step, and this one adds none.
//!
//! The one thing the export is particular about, and the thing that turns
//! a read into moaning when it is got wrong: the embedding graph splits
//! whatever ids it is given into two runs. Everything but the last two goes
//! through the *text* table; the last two go through the *speech* table,
//! with `<|endoftext|>` (50256) swapped for the start-of-speech token
//! (6561) on the way. The tokenizer's post-processor appends two
//! `<|endoftext|>` to every text, so the whole of its output - terminators
//! included - is embedded in one call, and the graph itself turns the two
//! terminators into two start-of-speech marks. A single written token,
//! embedded on its own, has no first run and is speech. Nothing else may be
//! appended: a start token added by hand is a third mark the model never
//! saw in training, and a text encoded without its terminators sends its
//! last word through the speech table as noise.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use ort::session::Session;
use ort::value::Tensor;

use crate::DownloadProgress;

/// The bundle id, as the settings panel and a request name it.
pub const BUNDLE_ID: &str = "chatterbox-turbo-q8";

/// What the model speaks at.
pub const SAMPLE_RATE: u32 = 24_000;
/// Speech tokens a second: how long a token is in sound.
const TOKENS_PER_SECOND: f32 = 25.0;
/// The token the language model starts writing after. Never fed by hand:
/// the embedding graph makes it from the text's `<|endoftext|>` pair.
const START_SPEECH_TOKEN: i64 = 6561;
/// GPT-2's `<|endoftext|>`, which the tokenizer's post-processor appends
/// twice to every text and the embedding graph reads as the start of
/// speech.
const TEXT_END_TOKEN: i64 = 50256;
/// The token that means it has finished.
const STOP_SPEECH_TOKEN: i64 = 6562;
/// A token of silence, three of which end every utterance.
const SILENCE_TOKEN: i64 = 4299;
/// The language model's shape: attention heads, their width, its depth.
const KV_HEADS: usize = 16;
const HEAD_DIM: usize = 64;
const LAYERS: usize = 24;
/// Resemble's reference setting: a token the model has written is that
/// much less likely to be written again.
const REPETITION_PENALTY: f32 = 1.2;
/// Resemble's own sampling, as `ChatterboxTurboTTS.generate` defaults it:
/// the logits softened by this temperature, the thousand likeliest kept,
/// and the smallest set of those that covers `TOP_P` drawn from. The ONNX
/// reference script takes the argmax instead, which is what a language
/// model does when it wants to repeat itself; the sampled read is the one
/// people compare against.
const TEMPERATURE: f32 = 0.8;
const TOP_K: usize = 1000;
const TOP_P: f32 = 0.95;
/// The most tokens one chunk of text may take: forty seconds of speech.
const MAX_NEW_TOKENS: usize = 1024;
/// How far past its expected length a chunk may run before it is cut off:
/// a read that has said its words stops on its own well inside this, and
/// one that has not is not going to.
const RUNAWAY: usize = 4;
/// The least recording a voice can be taken from. Resemble's own loader
/// refuses less with "Audio prompt must be longer than 5 seconds": under
/// it the speaker encoder has nothing to hold and the model reads in
/// nobody's voice, at length.
pub const MIN_REFERENCE_SECONDS: f32 = 5.0;
/// Where a recording's loudness is brought to before it is heard, as
/// Resemble's loader does with pyloudnorm. Measured here as RMS, which for
/// speech sits within a decibel or two of the integrated loudness.
const TARGET_LUFS: f32 = -27.0;
/// A chunk of text longer than this is read as two.
const CHUNK_CHARS: usize = 250;
/// Between chunks.
const GAP_SECONDS: f32 = 0.25;

/// One file of the bundle, as the mirror and the table name it.
struct KnownModel {
    /// What the file is called on the mirror: the bundle's name and the
    /// file's own, so two exports never collide on one flat release.
    id: &'static str,
    /// What the file is called on disk, which is what the graphs expect
    /// of their weights.
    local: &'static str,
    upstream: &'static str,
    bytes: u64,
    sha256: &'static str,
}

/// The nine files, largest last so a torn download costs the least.
const FILES: &[KnownModel] = &[
    KnownModel {
        id: "chatterbox-turbo-tokenizer.json",
        local: "tokenizer.json",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/d21799bd0354adb85e348b8a0442a8405110a2cf/tokenizer.json",
        bytes: 3_562_272,
        sha256: "3f04e34bea22f9144d1a19151154095bc9ce0430bf421304f5797e716288a906",
    },
    KnownModel {
        id: "chatterbox-turbo-embed_tokens_quantized.onnx",
        local: "onnx/embed_tokens_quantized.onnx",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/d21799bd0354adb85e348b8a0442a8405110a2cf/onnx/embed_tokens_quantized.onnx",
        bytes: 2_887,
        sha256: "0efe1bc01c2c48a98425a74444fd9887924d887f922c2722a6ec961ebb9e1db6",
    },
    KnownModel {
        id: "chatterbox-turbo-language_model_quantized.onnx",
        local: "onnx/language_model_quantized.onnx",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/d21799bd0354adb85e348b8a0442a8405110a2cf/onnx/language_model_quantized.onnx",
        bytes: 279_670,
        sha256: "0b40581277e30b7034331ec8c3ad47ed71d321f015b387a95221e54e2fcbfde8",
    },
    KnownModel {
        id: "chatterbox-turbo-speech_encoder_quantized.onnx",
        local: "onnx/speech_encoder_quantized.onnx",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/d21799bd0354adb85e348b8a0442a8405110a2cf/onnx/speech_encoder_quantized.onnx",
        bytes: 1_205_728,
        sha256: "5b6f15870a43cf97892df86fc550a0ef4763522d527cde72b2a4316f80a34de4",
    },
    KnownModel {
        id: "chatterbox-turbo-conditional_decoder_quantized.onnx",
        local: "onnx/conditional_decoder_quantized.onnx",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/d21799bd0354adb85e348b8a0442a8405110a2cf/onnx/conditional_decoder_quantized.onnx",
        bytes: 2_202_035,
        sha256: "2af3b150196d9d559cd3c91e03da80eb27a466032369dc2b57ea729cddad3ebb",
    },
    KnownModel {
        id: "chatterbox-turbo-embed_tokens_quantized.onnx_data",
        local: "onnx/embed_tokens_quantized.onnx_data",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/d21799bd0354adb85e348b8a0442a8405110a2cf/onnx/embed_tokens_quantized.onnx_data",
        bytes: 67_297_376,
        sha256: "9025d04c124899823124b1d7bb7069b1f535fb8a6c2d88f97520eb6fecced986",
    },
    KnownModel {
        id: "chatterbox-turbo-conditional_decoder_quantized.onnx_data",
        local: "onnx/conditional_decoder_quantized.onnx_data",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/d21799bd0354adb85e348b8a0442a8405110a2cf/onnx/conditional_decoder_quantized.onnx_data",
        bytes: 326_548_688,
        sha256: "4918ca09e05e41d2b4aa1ace6201d1cd911ffc58a42801002bab177d495cfe0a",
    },
    KnownModel {
        id: "chatterbox-turbo-speech_encoder_quantized.onnx_data",
        local: "onnx/speech_encoder_quantized.onnx_data",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/d21799bd0354adb85e348b8a0442a8405110a2cf/onnx/speech_encoder_quantized.onnx_data",
        bytes: 354_676_576,
        sha256: "d59861fb55e806fbeee731da9d4f8ff819fb5735de5d15e262d902594ee4dbb6",
    },
    KnownModel {
        id: "chatterbox-turbo-language_model_quantized.onnx_data",
        local: "onnx/language_model_quantized.onnx_data",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/d21799bd0354adb85e348b8a0442a8405110a2cf/onnx/language_model_quantized.onnx_data",
        bytes: 367_962_860,
        sha256: "ec9945df36cb5d131d46688f2609fd715fbfcb0b8ee9681af5c84118de2d55a2",
    },
];

/// The whole bundle's size, for the settings row.
pub const BUNDLE_BYTES: u64 = 1_123_738_092;

/// Whether every file of the bundle is in `dir`, whole.
pub fn installed(dir: &Path) -> bool {
    FILES.iter().all(|file| {
        std::fs::metadata(dir.join(file.local)).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
    })
}

/// Fetches whatever of the bundle is not in `dir` yet, file by file, the
/// mirror first and Resemble's upstream second, reporting the bytes of
/// the whole as one download. Blocks: run it on its own thread.
pub fn download(
    dir: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(DownloadProgress),
) -> Result<(), String> {
    let total: u64 = FILES.iter().map(|file| file.bytes).sum();
    let mut before: u64 = 0;
    for file in FILES {
        let target = dir.join(file.local);
        if std::fs::metadata(&target).is_ok_and(|meta| meta.is_file() && meta.len() > 0) {
            before += file.bytes;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        let partial = target.with_extension(match target.extension().and_then(|e| e.to_str()) {
            Some(extension) => format!("{extension}.part"),
            None => "part".to_owned(),
        });
        let so_far = before;
        crate::fetch_model(
            file.id,
            file.upstream,
            &partial,
            BUNDLE_ID,
            file.bytes,
            file.sha256,
            cancel,
            &mut |report: DownloadProgress| {
                progress(DownloadProgress {
                    id: BUNDLE_ID.to_owned(),
                    received: so_far + report.received.min(file.bytes),
                    total,
                    unpacking: false,
                    done: false,
                });
            },
        )?;
        std::fs::rename(&partial, &target)
            .map_err(|error| format!("could not finish {}: {error}", target.display()))?;
        before += file.bytes;
        if cancel.load(Ordering::Relaxed) {
            return Err("download cancelled".to_owned());
        }
    }
    progress(DownloadProgress {
        id: BUNDLE_ID.to_owned(),
        received: total,
        total,
        unpacking: false,
        done: true,
    });
    Ok(())
}

/// The four networks and the tokenizer, loaded once.
pub struct Engine {
    tokenizer: tokenizers::Tokenizer,
    speech_encoder: Mutex<Session>,
    embed_tokens: Mutex<Session>,
    language_model: Mutex<Session>,
    decoder: Mutex<Session>,
}

/// What the speech encoder heard in a recording: everything the other
/// networks need to speak in that voice.
struct Voice {
    /// `[1, T, 1024]`, prepended to the text's embedding.
    features: (Vec<usize>, Vec<f32>),
    /// The recording's own speech tokens, which the output continues.
    prompt: Vec<i64>,
    /// `[1, 192]`.
    embedding: (Vec<usize>, Vec<f32>),
    /// `[1, F, 80]`.
    spectrum: (Vec<usize>, Vec<f32>),
}

/// One network, opened for the CPU or, when asked and where the build has
/// it, for CoreML: the Mac's GPU and Neural Engine. CoreML takes the parts
/// of a graph it knows and hands the rest back to the CPU, and a graph it
/// cannot take at all still runs - slower, not broken - so asking for it
/// is safe; the log says which it got.
fn session(path: &Path, accelerated: bool) -> Result<Session, String> {
    let threads = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .clamp(1, 8);
    let mut builder = Session::builder()
        .map_err(|error| format!("onnx runtime: {error}"))?
        .with_intra_threads(threads)
        .map_err(|error| format!("onnx runtime: {error}"))?;
    #[cfg(target_os = "macos")]
    if accelerated {
        use ort::ep::coreml::{ComputeUnits, CoreML};
        builder = builder
            .with_execution_providers([CoreML::default()
                .with_compute_units(ComputeUnits::All)
                .with_subgraphs(true)
                .build()])
            .map_err(|error| format!("onnx runtime: coreml: {error}"))?;
        log::info!(
            "chatterbox: {} asked to run on CoreML",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
    }
    #[cfg(not(target_os = "macos"))]
    let _ = accelerated;
    builder
        .commit_from_file(path)
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// One named input, as a session takes it.
type Fed = (
    std::borrow::Cow<'static, str>,
    ort::session::SessionInputValue<'static>,
);

fn feed(name: impl Into<std::borrow::Cow<'static, str>>, value: ort::value::Value) -> Fed {
    (name.into(), value.into())
}

fn tensor_f32(dims: Vec<usize>, data: Vec<f32>) -> Result<ort::value::Value, String> {
    Tensor::from_array((dims, data))
        .map(|tensor| tensor.into_dyn())
        .map_err(|error| format!("chatterbox input: {error}"))
}

fn tensor_i64(dims: Vec<usize>, data: Vec<i64>) -> Result<ort::value::Value, String> {
    Tensor::from_array((dims, data))
        .map(|tensor| tensor.into_dyn())
        .map_err(|error| format!("chatterbox input: {error}"))
}

fn take_f32(
    outputs: &ort::session::SessionOutputs<'_>,
    name: &str,
) -> Result<(Vec<usize>, Vec<f32>), String> {
    let value = outputs
        .get(name)
        .ok_or_else(|| format!("chatterbox: the model has no output {name:?}"))?;
    let (shape, data) = value
        .try_extract_tensor::<f32>()
        .map_err(|error| format!("chatterbox output {name}: {error}"))?;
    Ok((
        shape.iter().map(|&d| d.max(0) as usize).collect(),
        data.to_vec(),
    ))
}

fn take_i64(outputs: &ort::session::SessionOutputs<'_>, name: &str) -> Result<Vec<i64>, String> {
    let value = outputs
        .get(name)
        .ok_or_else(|| format!("chatterbox: the model has no output {name:?}"))?;
    let (_, data) = value
        .try_extract_tensor::<i64>()
        .map_err(|error| format!("chatterbox output {name}: {error}"))?;
    Ok(data.to_vec())
}

impl Engine {
    /// Loads the bundle in `dir`, for the accelerator or the CPU.
    pub fn load(dir: &Path, accelerated: bool) -> Result<Engine, String> {
        if !installed(dir) {
            return Err("the Chatterbox bundle is incomplete - re-download it".to_owned());
        }
        let started = std::time::Instant::now();
        log::info!(
            "chatterbox: loading the bundle in {} for the {}",
            dir.display(),
            if accelerated { "accelerator" } else { "CPU" }
        );
        let tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|error| format!("chatterbox tokenizer: {error}"))?;
        let onnx = dir.join("onnx");
        let open = |name: &str| session(&onnx.join(name), accelerated);
        let engine = Engine {
            tokenizer,
            speech_encoder: Mutex::new(open("speech_encoder_quantized.onnx")?),
            embed_tokens: Mutex::new(open("embed_tokens_quantized.onnx")?),
            language_model: Mutex::new(open("language_model_quantized.onnx")?),
            decoder: Mutex::new(open("conditional_decoder_quantized.onnx")?),
        };
        log::info!(
            "chatterbox: four networks loaded in {:.1}s",
            started.elapsed().as_secs_f32()
        );
        Ok(engine)
    }

    /// Reads `text` in the voice of `reference`, mono samples at
    /// [`SAMPLE_RATE`]. Long text is read a chunk at a time with a short
    /// gap between; `progress` is told `0..=1` as tokens are written, and
    /// `cancel` stops it at the next token.
    pub fn speak(
        &self,
        text: &str,
        reference: &[f32],
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(f32),
    ) -> Result<Vec<f32>, String> {
        let started = std::time::Instant::now();
        log::info!(
            "chatterbox: reading {} characters in the voice of a {:.1}s recording",
            text.chars().count(),
            reference.len() as f32 / SAMPLE_RATE as f32
        );
        let voice = self.hear(reference)?;
        // The text as the model was trained to see it: capitalised, one
        // space between words, plain punctuation, and a full stop at the
        // end, which is what tells it to stop.
        let text = punc_norm(text);
        let chunks = chunks(&text);
        if chunks.is_empty() {
            return Err("nothing to say: the text is empty".to_owned());
        }
        log::info!("chatterbox: {} chunk(s) to read", chunks.len());
        let expected: usize = chunks.iter().map(|chunk| expected_tokens(chunk)).sum();
        let mut written = 0usize;
        let mut samples = Vec::new();
        for (index, chunk) in chunks.iter().enumerate() {
            if index > 0 {
                samples.extend(std::iter::repeat_n(
                    0.0f32,
                    (SAMPLE_RATE as f32 * GAP_SECONDS) as usize,
                ));
            }
            log::debug!(
                "chatterbox: chunk {}/{}: {:?}",
                index + 1,
                chunks.len(),
                chunk.chars().take(80).collect::<String>()
            );
            let writing = std::time::Instant::now();
            let tokens = self.write(chunk, &voice, cancel, &mut |count| {
                progress(((written + count) as f32 / expected.max(1) as f32).min(0.95));
            })?;
            log::info!(
                "chatterbox: chunk {}/{} wrote {} speech tokens ({:.1}s of speech) in {:.1}s",
                index + 1,
                chunks.len(),
                tokens.len(),
                tokens.len() as f32 / TOKENS_PER_SECOND,
                writing.elapsed().as_secs_f32()
            );
            written += tokens.len();
            let decoding = std::time::Instant::now();
            let wave = self.decode(&voice, &tokens)?;
            log::info!(
                "chatterbox: chunk {}/{} decoded to {:.2}s of audio in {:.1}s",
                index + 1,
                chunks.len(),
                wave.len() as f32 / SAMPLE_RATE as f32,
                decoding.elapsed().as_secs_f32()
            );
            samples.extend(wave);
        }
        log::info!(
            "chatterbox: {:.2}s of audio in {:.1}s",
            samples.len() as f32 / SAMPLE_RATE as f32,
            started.elapsed().as_secs_f32()
        );
        progress(1.0);
        Ok(samples)
    }

    /// The speech encoder over the recording, brought to the loudness the
    /// model was trained at.
    fn hear(&self, reference: &[f32]) -> Result<Voice, String> {
        let seconds = reference.len() as f32 / SAMPLE_RATE as f32;
        if seconds < MIN_REFERENCE_SECONDS {
            return Err(format!(
                "the recording is {seconds:.1}s of sound and Chatterbox needs at least \
                 {MIN_REFERENCE_SECONDS:.0}s of clear speech to take a voice from - pick a longer \
                 sample, or start it earlier"
            ));
        }
        let (reference, gain_db) = norm_loudness(reference);
        log::info!("chatterbox: the recording brought to {TARGET_LUFS} LUFS ({gain_db:+.1} dB)");
        let mut encoder = self
            .speech_encoder
            .lock()
            .map_err(|_| "chatterbox: encoder poisoned")?;
        let audio = tensor_f32(vec![1, reference.len()], reference)?;
        let outputs = encoder
            .run(vec![feed("audio_values", audio)])
            .map_err(|error| format!("chatterbox speech encoder: {error}"))?;
        let voice = Voice {
            features: take_f32(&outputs, "audio_features")?,
            prompt: take_i64(&outputs, "audio_tokens")?,
            embedding: take_f32(&outputs, "speaker_embeddings")?,
            spectrum: take_f32(&outputs, "speaker_features")?,
        };
        log::debug!(
            "chatterbox: heard the voice: features {:?}, {} prompt tokens, embedding {:?}, spectrum {:?}",
            voice.features.0,
            voice.prompt.len(),
            voice.embedding.0,
            voice.spectrum.0
        );
        Ok(voice)
    }

    /// The language model over one chunk: the speech tokens it writes,
    /// without the start and stop tokens.
    fn write(
        &self,
        chunk: &str,
        voice: &Voice,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(usize),
    ) -> Result<Vec<i64>, String> {
        // With the post-processor's two `<|endoftext|>` on the end: they are
        // what the embedding graph turns into the start of speech, and a
        // text without them is read wrong from its last word on - see the
        // module's note.
        let encoding = self
            .tokenizer
            .encode(chunk, true)
            .map_err(|error| format!("chatterbox tokenizer: {error}"))?;
        let text_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| i64::from(id)).collect();
        if text_ids.iter().all(|&id| id == TEXT_END_TOKEN) {
            return Ok(Vec::new());
        }
        log::debug!(
            "chatterbox: {} text tokens, ending {:?}",
            text_ids.len(),
            &text_ids[text_ids.len().saturating_sub(3)..]
        );
        let mut embed = self
            .embed_tokens
            .lock()
            .map_err(|_| "chatterbox: embedding poisoned")?;
        let mut model = self
            .language_model
            .lock()
            .map_err(|_| "chatterbox: model poisoned")?;

        // The first step reads the voice and the whole text together - the
        // text's own terminators becoming the start of speech inside the
        // embedding graph, so nothing is appended here; every later step
        // reads the one token just written, with the cache.
        let embed_ids =
            |embed: &mut Session, ids: &[i64]| -> Result<(Vec<usize>, Vec<f32>), String> {
                let outputs = embed
                    .run(vec![feed(
                        "input_ids",
                        tensor_i64(vec![1, ids.len()], ids.to_vec())?,
                    )])
                    .map_err(|error| format!("chatterbox embedding: {error}"))?;
                take_f32(&outputs, "inputs_embeds")
            };
        let (_, text_embeds) = embed_ids(&mut embed, &text_ids)?;
        let width = voice.features.0[2];
        let mut embeds = voice.features.1.clone();
        embeds.extend_from_slice(&text_embeds);
        let mut seq_len = embeds.len() / width;
        // The prompt: the recording's tokens and the text's, which every
        // step's mask covers along with one token per step before it.
        let prompt_len = seq_len;
        let mut position = 0usize;

        let mut cache: Vec<Option<ort::value::Value>> = (0..LAYERS * 2).map(|_| None).collect();
        let mut generated: Vec<i64> = vec![START_SPEECH_TOKEN];
        let mut rng = Rng::seeded();
        // Cut off well past where this chunk should have finished, so a
        // read that has lost its way costs seconds and not the forty the
        // model would otherwise fill.
        let limit = MAX_NEW_TOKENS.min(expected_tokens(chunk) * RUNAWAY + 64);
        for step in 0..limit {
            if cancel.load(Ordering::Relaxed) {
                return Err("speech generation cancelled".to_owned());
            }
            let total_len = prompt_len + step;
            let mut inputs: Vec<Fed> = Vec::with_capacity(3 + LAYERS * 2);
            inputs.push(feed(
                "inputs_embeds",
                tensor_f32(vec![1, seq_len, width], std::mem::take(&mut embeds))?,
            ));
            inputs.push(feed(
                "attention_mask",
                tensor_i64(vec![1, total_len], vec![1; total_len])?,
            ));
            inputs.push(feed(
                "position_ids",
                tensor_i64(
                    vec![1, seq_len],
                    (position..position + seq_len).map(|p| p as i64).collect(),
                )?,
            ));
            for layer in 0..LAYERS {
                for (slot, kind) in ["key", "value"].iter().enumerate() {
                    let value = match cache[layer * 2 + slot].take() {
                        Some(value) => value,
                        None => tensor_f32(vec![1, KV_HEADS, 0, HEAD_DIM], Vec::new())?,
                    };
                    inputs.push(feed(format!("past_key_values.{layer}.{kind}"), value));
                }
            }
            let mut outputs = model
                .run(inputs)
                .map_err(|error| format!("chatterbox language model: {error}"))?;
            let (logits_shape, logits) = take_f32(&outputs, "logits")?;
            let vocab = logits_shape[2];
            let last = &logits[(logits_shape[1] - 1) * vocab..];
            let next = sample_token(
                last,
                &generated,
                REPETITION_PENALTY,
                TEMPERATURE,
                TOP_K,
                TOP_P,
                &mut rng,
            );
            for layer in 0..LAYERS {
                for (slot, kind) in ["key", "value"].iter().enumerate() {
                    let name = format!("present.{layer}.{kind}");
                    cache[layer * 2 + slot] =
                        Some(outputs.remove(name.as_str()).ok_or_else(|| {
                            format!("chatterbox: the model has no output {name:?}")
                        })?);
                }
            }
            generated.push(next);
            progress(step + 1);
            if next == STOP_SPEECH_TOKEN {
                log::debug!("chatterbox: stop token after {} speech tokens", step);
                break;
            }
            if step + 1 == limit {
                log::warn!(
                    "chatterbox: no stop token in {limit} tokens for {} characters - the read \
                     ran out and was cut; the recording or the text may be one the model \
                     cannot follow",
                    chunk.chars().count()
                );
            }
            position += seq_len;
            let (_, next_embed) = embed_ids(&mut embed, &[next])?;
            embeds = next_embed;
            seq_len = 1;
        }
        Ok(spoken(&generated))
    }

    /// The decoder over the recording's tokens, the written ones, and a
    /// beat of silence: sound.
    fn decode(&self, voice: &Voice, tokens: &[i64]) -> Result<Vec<f32>, String> {
        let all = with_prompt(&voice.prompt, tokens);
        let mut decoder = self
            .decoder
            .lock()
            .map_err(|_| "chatterbox: decoder poisoned")?;
        let outputs = decoder
            .run(vec![
                feed("speech_tokens", tensor_i64(vec![1, all.len()], all)?),
                feed(
                    "speaker_embeddings",
                    tensor_f32(voice.embedding.0.clone(), voice.embedding.1.clone())?,
                ),
                feed(
                    "speaker_features",
                    tensor_f32(voice.spectrum.0.clone(), voice.spectrum.1.clone())?,
                ),
            ])
            .map_err(|error| format!("chatterbox decoder: {error}"))?;
        let (_, wave) = take_f32(&outputs, "waveform")?;
        Ok(wave)
    }
}

/// A small, fast generator for the draw a sample takes: xorshift64*, seeded
/// from the clock and the process so two reads of one text differ, the way
/// two takes do.
pub struct Rng(u64);

impl Rng {
    /// Seeded from the clock and the process id.
    pub fn seeded() -> Rng {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Rng::from_seed(nanos ^ (u64::from(std::process::id()) << 32))
    }

    /// Seeded by hand, for a draw that has to repeat.
    pub fn from_seed(seed: u64) -> Rng {
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        let word = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (word >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// The next token, drawn the way Resemble's `generate` draws it: the
/// repetition penalty first, then the logits over `temperature`, the
/// `top_k` likeliest kept, and one drawn from the smallest set of those
/// whose probability adds up to `top_p`. A temperature of zero is the
/// argmax, as [`next_token`] takes it.
pub fn sample_token(
    logits: &[f32],
    written: &[i64],
    penalty: f32,
    temperature: f32,
    top_k: usize,
    top_p: f32,
    rng: &mut Rng,
) -> i64 {
    if temperature <= 0.0 || logits.is_empty() {
        return next_token(logits, written, penalty);
    }
    let mut scored: Vec<(usize, f32)> = logits
        .iter()
        .enumerate()
        .map(|(index, &score)| {
            let score = if written.contains(&(index as i64)) {
                if score < 0.0 {
                    score * penalty
                } else {
                    score / penalty
                }
            } else {
                score
            };
            (
                index,
                if score.is_finite() {
                    score
                } else {
                    f32::NEG_INFINITY
                },
            )
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(top_k.max(1));
    let top = scored[0].1;
    if !top.is_finite() {
        return scored[0].0 as i64;
    }
    // Softmax over the kept, off the top score so the exponents stay in
    // range; then the nucleus.
    let weights: Vec<f32> = scored
        .iter()
        .map(|(_, score)| ((score - top) / temperature).exp())
        .collect();
    let total: f32 = weights.iter().sum();
    let cutoff = top_p.clamp(0.0, 1.0) * total;
    let mut kept = 0;
    let mut covered = 0.0;
    for weight in &weights {
        kept += 1;
        covered += weight;
        if covered >= cutoff {
            break;
        }
    }
    let mut draw = rng.unit() * covered;
    for (index, weight) in weights.iter().take(kept).enumerate() {
        draw -= weight;
        if draw <= 0.0 {
            return scored[index].0 as i64;
        }
    }
    scored[kept - 1].0 as i64
}

/// The recording brought to [`TARGET_LUFS`], and the gain it took in
/// decibels. RMS stands in for the integrated loudness Resemble measures;
/// a silent recording is left as it is rather than amplified into noise.
pub fn norm_loudness(samples: &[f32]) -> (Vec<f32>, f32) {
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt();
    if rms <= 1e-6 {
        return (samples.to_vec(), 0.0);
    }
    let loudness = 20.0 * rms.log10();
    let gain_db = TARGET_LUFS - loudness;
    let gain = 10f32.powf(gain_db / 20.0);
    if !gain.is_finite() || gain <= 0.0 {
        return (samples.to_vec(), 0.0);
    }
    (
        samples
            .iter()
            .map(|s| (s * gain).clamp(-1.0, 1.0))
            .collect(),
        gain_db,
    )
}

/// The text as the model saw its training data, ported from Resemble's
/// `punc_norm`: a capital to start, one space between words, the
/// punctuation a keyboard has in place of the kind a word processor
/// makes, and something to end on - a full stop when there is nothing
/// else, which is what tells the model the sentence is over.
pub fn punc_norm(text: &str) -> String {
    let mut text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return text;
    }
    let mut chars = text.chars();
    if let Some(first) = chars.next()
        && first.is_lowercase()
    {
        text = first.to_uppercase().collect::<String>() + chars.as_str();
    }
    for (from, to) in [
        ("…", ", "),
        (":", ","),
        ("—", "-"),
        ("–", "-"),
        (" ,", ","),
        ("“", "\""),
        ("”", "\""),
        ("‘", "'"),
        ("’", "'"),
    ] {
        text = text.replace(from, to);
    }
    // An ellipsis before a space became ", " before a space: one space.
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if ['.', '!', '?', '-', ',']
        .iter()
        .any(|end| text.ends_with(*end))
    {
        text
    } else {
        text + "."
    }
}

/// The most likely next token, with every token already written made
/// [`REPETITION_PENALTY`] times less likely: a score below zero is
/// multiplied by the penalty, one above divided by it, as Resemble's
/// reference does.
pub fn next_token(logits: &[f32], written: &[i64], penalty: f32) -> i64 {
    let mut best = 0usize;
    let mut best_score = f32::NEG_INFINITY;
    for (index, &score) in logits.iter().enumerate() {
        let score = if written.contains(&(index as i64)) {
            if score < 0.0 {
                score * penalty
            } else {
                score / penalty
            }
        } else {
            score
        };
        if score > best_score {
            best_score = score;
            best = index;
        }
    }
    best as i64
}

/// The speech in a written sequence: without the start token, and
/// without the stop token when it ended on one.
pub fn spoken(generated: &[i64]) -> Vec<i64> {
    let body = generated.get(1..).unwrap_or(&[]);
    match body.split_last() {
        Some((&last, rest)) if last == STOP_SPEECH_TOKEN => rest.to_vec(),
        _ => body.to_vec(),
    }
}

/// What the decoder reads: the recording's own tokens, then the written
/// ones, then three of silence so the sound ends rather than stops.
pub fn with_prompt(prompt: &[i64], tokens: &[i64]) -> Vec<i64> {
    let mut all = Vec::with_capacity(prompt.len() + tokens.len() + 3);
    all.extend_from_slice(prompt);
    all.extend_from_slice(tokens);
    all.extend_from_slice(&[SILENCE_TOKEN; 3]);
    all
}

/// `text` as the chunks it is read in: sentences, joined up to
/// [`CHUNK_CHARS`] each, so the model never writes more at once than it
/// keeps straight and a long script still reads as one.
pub fn chunks(text: &str) -> Vec<String> {
    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        // A full stop ends a sentence when a space follows, so "2.5" does
        // not; a line break ends one whatever follows.
        let ends = c == '\n'
            || (matches!(c, '.' | '!' | '?')
                && chars.peek().is_none_or(|next| next.is_whitespace()));
        if ends {
            sentences.push(std::mem::take(&mut current));
        }
    }
    sentences.push(current);
    let mut chunks: Vec<String> = Vec::new();
    for sentence in sentences {
        // One space between words, whatever the script's own spacing.
        let sentence = sentence.split_whitespace().collect::<Vec<_>>().join(" ");
        if sentence.is_empty() {
            continue;
        }
        let sentence = sentence.as_str();
        match chunks.last_mut() {
            Some(last) if last.chars().count() + 1 + sentence.chars().count() <= CHUNK_CHARS => {
                last.push(' ');
                last.push_str(sentence);
            }
            _ => chunks.push(sentence.to_owned()),
        }
    }
    chunks
}

/// About how many speech tokens `chunk` takes, for the readout: fifteen
/// characters a second is ordinary narration.
fn expected_tokens(chunk: &str) -> usize {
    ((chunk.chars().count() as f32 / 15.0) * TOKENS_PER_SECOND) as usize + 10
}

/// `samples` as a 16-bit mono WAV file.
pub fn wav_bytes(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// Where the bundle lives under the models folder.
pub fn bundle_dir(models: &Path) -> PathBuf {
    models.join(BUNDLE_ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UPSTREAM: &str = "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/d21799bd0354adb85e348b8a0442a8405110a2cf/";

    #[test]
    fn the_table_names_nine_distinct_files_with_the_bundle_in_front() {
        assert_eq!(FILES.len(), 9);
        let mut ids: Vec<&str> = FILES.iter().map(|file| file.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 9, "an id serves two files");
        assert_eq!(
            FILES.iter().map(|file| file.bytes).sum::<u64>(),
            BUNDLE_BYTES
        );
        for file in FILES {
            assert!(file.id.starts_with("chatterbox-turbo-"), "{}", file.id);
            assert!(file.upstream.starts_with(UPSTREAM), "{}", file.upstream);
            assert!(
                file.upstream
                    .ends_with(file.local.rsplit('/').next().unwrap())
            );
            assert!(file.bytes > 0);
            assert!(!file.local.starts_with('/'));
        }
        // Graphs before their weights: the small files land first.
        let graphs = FILES
            .iter()
            .position(|f| f.local.ends_with(".onnx"))
            .unwrap();
        let data = FILES
            .iter()
            .position(|f| f.local.ends_with(".onnx_data"))
            .unwrap();
        assert!(graphs < data);
        // Nothing is installed in an empty folder, and nothing panics.
        let dir = std::env::temp_dir().join(format!("concat-chatterbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!installed(&dir));
        std::fs::create_dir_all(dir.join("onnx")).expect("mkdir");
        std::fs::write(dir.join("tokenizer.json"), b"{}").expect("write");
        assert!(!installed(&dir), "one file is not the bundle");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_penalty_turns_a_written_token_away_and_the_best_is_picked() {
        // Unwritten: the largest wins.
        assert_eq!(next_token(&[0.1, 0.9, 0.5], &[], 1.2), 1);
        // Written and positive: divided, so a close second overtakes it.
        assert_eq!(next_token(&[0.8, 0.9, 0.5], &[1], 1.2), 0);
        // Written and negative: multiplied further down.
        assert_eq!(next_token(&[-0.5, -0.55, -1.0], &[0], 1.2), 1);
        // A penalty of one changes nothing.
        assert_eq!(next_token(&[0.8, 0.9, 0.5], &[1], 1.0), 1);
        // Ties go to the first; an empty vocabulary gives token 0 rather
        // than panicking.
        assert_eq!(next_token(&[0.3, 0.3], &[], 1.2), 0);
        assert_eq!(next_token(&[], &[], 1.2), 0);
        assert_eq!(next_token(&[f32::NAN, 0.1], &[], 1.2), 1, "NaN never wins");
    }

    /// A draw at temperature zero is the argmax; a draw with the nucleus
    /// shut to nothing is the argmax too; and a draw with it open lands on
    /// one of the kept tokens and never on one the penalty has buried.
    #[test]
    fn a_sample_stays_inside_the_nucleus() {
        let logits = [0.1, 5.0, 4.9, -3.0, 0.0];
        let mut rng = Rng::from_seed(7);
        assert_eq!(
            sample_token(&logits, &[], 1.2, 0.0, 1000, 0.95, &mut rng),
            1
        );
        assert_eq!(sample_token(&logits, &[], 1.2, 0.8, 1000, 0.0, &mut rng), 1);
        for _ in 0..200 {
            let token = sample_token(&logits, &[], 1.2, 0.8, 1000, 0.95, &mut rng);
            assert!(token == 1 || token == 2, "{token}");
        }
        // Two tokens' worth of nucleus, and both get drawn over time.
        let mut seen = [false; 5];
        for _ in 0..500 {
            seen[sample_token(&logits, &[], 1.2, 0.8, 1000, 0.95, &mut rng) as usize] = true;
        }
        assert!(seen[1] && seen[2]);
        // top_k of one is the argmax however the dice fall.
        for _ in 0..50 {
            assert_eq!(sample_token(&logits, &[], 1.2, 0.8, 1, 0.95, &mut rng), 1);
        }
        // Nothing to draw from does not panic.
        assert_eq!(sample_token(&[], &[], 1.2, 0.8, 1000, 0.95, &mut rng), 0);
        assert_eq!(
            sample_token(&[f32::NAN, f32::NAN], &[], 1.2, 0.8, 1000, 0.95, &mut rng),
            0
        );
    }

    /// The generator gives numbers in `[0, 1)`, different ones, and the same
    /// ones again from the same seed.
    #[test]
    fn the_generator_is_uniform_and_repeatable() {
        let mut a = Rng::from_seed(42);
        let mut b = Rng::from_seed(42);
        let mut distinct = std::collections::HashSet::new();
        for _ in 0..1000 {
            let x = a.unit();
            assert!((0.0..1.0).contains(&x), "{x}");
            assert_eq!(x, b.unit());
            distinct.insert(x.to_bits());
        }
        assert!(distinct.len() > 900);
        assert_ne!(Rng::from_seed(0).unit(), 0.0, "a zero seed is not stuck");
    }

    /// A recording is brought to the target loudness whether it came in
    /// loud or quiet, silence is left alone, and nothing clips.
    #[test]
    fn a_recording_is_brought_to_the_target_loudness() {
        let tone = |amplitude: f32| -> Vec<f32> {
            (0..24_000)
                .map(|i| amplitude * (i as f32 * 0.05).sin())
                .collect()
        };
        for amplitude in [0.9, 0.05, 0.002] {
            let (out, gain_db) = norm_loudness(&tone(amplitude));
            let rms = (out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt();
            let lufs = 20.0 * rms.log10();
            assert!(
                (lufs - TARGET_LUFS).abs() < 0.5,
                "{amplitude}: {lufs} after {gain_db} dB"
            );
            assert!(out.iter().all(|s| (-1.0..=1.0).contains(s)));
        }
        let (silent, gain_db) = norm_loudness(&[0.0; 100]);
        assert!(silent.iter().all(|&s| s == 0.0) && gain_db == 0.0);
    }

    /// Resemble's text cleanup, ported: a capital, single spaces, plain
    /// punctuation, and an end.
    #[test]
    fn the_text_is_normalised_the_way_resemble_does_it() {
        assert_eq!(punc_norm("hello   world"), "Hello world.");
        assert_eq!(punc_norm("Already ends!"), "Already ends!");
        assert_eq!(punc_norm("a list: one… two"), "A list, one, two.");
        assert_eq!(
            punc_norm("“quoted” — and ‘this’"),
            "\"quoted\" - and 'this'."
        );
        assert_eq!(punc_norm("trailing ,"), "Trailing,");
        assert_eq!(punc_norm("   "), "");
        assert_eq!(punc_norm("ünïcode start"), "Ünïcode start.");
    }

    #[test]
    fn the_start_and_stop_are_stripped_and_the_prompt_and_silence_added() {
        assert_eq!(
            spoken(&[START_SPEECH_TOKEN, 5, 6, STOP_SPEECH_TOKEN]),
            vec![5, 6]
        );
        assert_eq!(
            spoken(&[START_SPEECH_TOKEN, 5, 6]),
            vec![5, 6],
            "ran out without a stop"
        );
        assert_eq!(
            spoken(&[START_SPEECH_TOKEN, STOP_SPEECH_TOKEN]),
            Vec::<i64>::new()
        );
        assert_eq!(spoken(&[START_SPEECH_TOKEN]), Vec::<i64>::new());
        assert_eq!(spoken(&[]), Vec::<i64>::new());
        assert_eq!(
            with_prompt(&[1, 2], &[5, 6]),
            vec![1, 2, 5, 6, SILENCE_TOKEN, SILENCE_TOKEN, SILENCE_TOKEN]
        );
        assert_eq!(with_prompt(&[], &[]), vec![SILENCE_TOKEN; 3]);
    }

    #[test]
    fn text_is_read_a_few_sentences_at_a_time() {
        assert!(chunks("").is_empty());
        assert!(chunks("   \n ").is_empty());
        assert_eq!(chunks("Hello there."), vec!["Hello there."]);
        assert_eq!(chunks("One. Two! Three?"), vec!["One. Two! Three?"]);
        assert_eq!(
            chunks("Version 2.5 is out. Yes."),
            vec!["Version 2.5 is out. Yes."],
            "a dot inside a number is not an end"
        );
        assert_eq!(
            chunks("First line\nSecond line"),
            vec!["First line Second line"]
        );
        let long = "This sentence is exactly fifty characters long, ok. ".repeat(10);
        let parts = chunks(&long);
        assert!(parts.len() >= 2, "{parts:?}");
        for part in &parts {
            assert!(
                part.chars().count() <= CHUNK_CHARS,
                "{}",
                part.chars().count()
            );
        }
        assert_eq!(
            parts.join(" ").split_whitespace().count(),
            long.split_whitespace().count()
        );
        // One sentence longer than a chunk is still one chunk: it is not cut mid-word.
        let run = "word ".repeat(80);
        assert_eq!(chunks(&run).len(), 1);
        assert!(expected_tokens("Hello there, how are you today?") > 10);
    }

    #[test]
    fn a_wav_is_forty_four_bytes_of_header_and_the_samples() {
        let wav = wav_bytes(&[0.0, 1.0, -1.0, 2.0], 24_000);
        assert_eq!(wav.len(), 44 + 8);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 24_000);
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 8);
        assert_eq!(i16::from_le_bytes(wav[44..46].try_into().unwrap()), 0);
        assert_eq!(i16::from_le_bytes(wav[46..48].try_into().unwrap()), 32767);
        assert_eq!(i16::from_le_bytes(wav[48..50].try_into().unwrap()), -32767);
        assert_eq!(
            i16::from_le_bytes(wav[50..52].try_into().unwrap()),
            32767,
            "clamped"
        );
        assert_eq!(wav_bytes(&[], 8_000).len(), 44);
    }
}

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
//! Every voice is a recording: there are no speakers built in. The sheet
//! offers the selected clip's own. Resemble's Python stamps its output
//! with a watermark; the ONNX path carries no such step, and this one
//! adds none.

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
/// The token the language model starts writing after.
const START_SPEECH_TOKEN: i64 = 6561;
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
/// The most tokens one chunk of text may take: forty seconds of speech.
const MAX_NEW_TOKENS: usize = 1024;
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

/// Where every file comes from; the test holds the table to it.
#[cfg(test)]
const UPSTREAM: &str = "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/main/";

/// The nine files, largest last so a torn download costs the least.
const FILES: &[KnownModel] = &[
    KnownModel {
        id: "chatterbox-turbo-tokenizer.json",
        local: "tokenizer.json",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/main/tokenizer.json",
        bytes: 3_562_272,
        sha256: "",
    },
    KnownModel {
        id: "chatterbox-turbo-embed_tokens_quantized.onnx",
        local: "onnx/embed_tokens_quantized.onnx",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/main/onnx/embed_tokens_quantized.onnx",
        bytes: 2_887,
        sha256: "",
    },
    KnownModel {
        id: "chatterbox-turbo-language_model_quantized.onnx",
        local: "onnx/language_model_quantized.onnx",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/main/onnx/language_model_quantized.onnx",
        bytes: 279_670,
        sha256: "",
    },
    KnownModel {
        id: "chatterbox-turbo-speech_encoder_quantized.onnx",
        local: "onnx/speech_encoder_quantized.onnx",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/main/onnx/speech_encoder_quantized.onnx",
        bytes: 1_205_728,
        sha256: "",
    },
    KnownModel {
        id: "chatterbox-turbo-conditional_decoder_quantized.onnx",
        local: "onnx/conditional_decoder_quantized.onnx",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/main/onnx/conditional_decoder_quantized.onnx",
        bytes: 2_202_035,
        sha256: "",
    },
    KnownModel {
        id: "chatterbox-turbo-embed_tokens_quantized.onnx_data",
        local: "onnx/embed_tokens_quantized.onnx_data",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/main/onnx/embed_tokens_quantized.onnx_data",
        bytes: 67_297_376,
        sha256: "",
    },
    KnownModel {
        id: "chatterbox-turbo-conditional_decoder_quantized.onnx_data",
        local: "onnx/conditional_decoder_quantized.onnx_data",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/main/onnx/conditional_decoder_quantized.onnx_data",
        bytes: 326_548_688,
        sha256: "",
    },
    KnownModel {
        id: "chatterbox-turbo-speech_encoder_quantized.onnx_data",
        local: "onnx/speech_encoder_quantized.onnx_data",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/main/onnx/speech_encoder_quantized.onnx_data",
        bytes: 354_676_576,
        sha256: "",
    },
    KnownModel {
        id: "chatterbox-turbo-language_model_quantized.onnx_data",
        local: "onnx/language_model_quantized.onnx_data",
        upstream: "https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/main/onnx/language_model_quantized.onnx_data",
        bytes: 367_962_860,
        sha256: "",
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

fn session(path: &Path) -> Result<Session, String> {
    let threads = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .clamp(1, 8);
    Session::builder()
        .map_err(|error| format!("onnx runtime: {error}"))?
        .with_intra_threads(threads)
        .map_err(|error| format!("onnx runtime: {error}"))?
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
    /// Loads the bundle in `dir`.
    pub fn load(dir: &Path) -> Result<Engine, String> {
        if !installed(dir) {
            return Err("the Chatterbox bundle is incomplete - re-download it".to_owned());
        }
        let tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|error| format!("chatterbox tokenizer: {error}"))?;
        let onnx = dir.join("onnx");
        Ok(Engine {
            tokenizer,
            speech_encoder: Mutex::new(session(&onnx.join("speech_encoder_quantized.onnx"))?),
            embed_tokens: Mutex::new(session(&onnx.join("embed_tokens_quantized.onnx"))?),
            language_model: Mutex::new(session(&onnx.join("language_model_quantized.onnx"))?),
            decoder: Mutex::new(session(&onnx.join("conditional_decoder_quantized.onnx"))?),
        })
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
        let voice = self.hear(reference)?;
        let chunks = chunks(text);
        if chunks.is_empty() {
            return Err("nothing to say: the text is empty".to_owned());
        }
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
            let tokens = self.write(chunk, &voice, cancel, &mut |count| {
                progress(((written + count) as f32 / expected.max(1) as f32).min(0.95));
            })?;
            written += tokens.len();
            let wave = self.decode(&voice, &tokens)?;
            samples.extend(wave);
        }
        progress(1.0);
        Ok(samples)
    }

    /// The speech encoder over the recording.
    fn hear(&self, reference: &[f32]) -> Result<Voice, String> {
        if reference.len() < SAMPLE_RATE as usize / 2 {
            return Err("too little sound to take a voice from".to_owned());
        }
        let mut encoder = self
            .speech_encoder
            .lock()
            .map_err(|_| "chatterbox: encoder poisoned")?;
        let audio = tensor_f32(vec![1, reference.len()], reference.to_vec())?;
        let outputs = encoder
            .run(vec![feed("audio_values", audio)])
            .map_err(|error| format!("chatterbox speech encoder: {error}"))?;
        Ok(Voice {
            features: take_f32(&outputs, "audio_features")?,
            prompt: take_i64(&outputs, "audio_tokens")?,
            embedding: take_f32(&outputs, "speaker_embeddings")?,
            spectrum: take_f32(&outputs, "speaker_features")?,
        })
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
        let encoding = self
            .tokenizer
            .encode(chunk, false)
            .map_err(|error| format!("chatterbox tokenizer: {error}"))?;
        let text_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| i64::from(id)).collect();
        if text_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut embed = self
            .embed_tokens
            .lock()
            .map_err(|_| "chatterbox: embedding poisoned")?;
        let mut model = self
            .language_model
            .lock()
            .map_err(|_| "chatterbox: model poisoned")?;

        // The first step reads the voice, the text and the start token
        // together; every later step reads one token with the cache.
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
        let (_, start_embed) = embed_ids(&mut embed, &[START_SPEECH_TOKEN])?;
        let width = voice.features.0[2];
        let mut embeds = voice.features.1.clone();
        embeds.extend_from_slice(&text_embeds);
        embeds.extend_from_slice(&start_embed);
        let mut seq_len = embeds.len() / width;
        let mut total_len = seq_len;
        let mut position = 0usize;

        let mut cache: Vec<Option<ort::value::Value>> = (0..LAYERS * 2).map(|_| None).collect();
        let mut generated: Vec<i64> = vec![START_SPEECH_TOKEN];
        for step in 0..MAX_NEW_TOKENS {
            if cancel.load(Ordering::Relaxed) {
                return Err("speech generation cancelled".to_owned());
            }
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
            let next = next_token(last, &generated, REPETITION_PENALTY);
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
                break;
            }
            position += seq_len;
            let (_, next_embed) = embed_ids(&mut embed, &[next])?;
            embeds = next_embed;
            seq_len = 1;
            total_len += 1;
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

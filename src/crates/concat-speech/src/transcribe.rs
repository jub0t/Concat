// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Whisper transcription: audio in, timed caption segments out.
//!
//! whisper.cpp runs in-process through `whisper-rs`, the way Kokoro runs
//! through sherpa-onnx: no binary to ship or find, and cancellation is a
//! callback returning `true`. On macOS the Metal build is used, so the
//! encoder runs on the GPU.
//!
//! Two pieces:
//!
//! 1. **Models.** Downloaded on demand from Hugging Face into the app data
//!    folder as plain `.bin` files, streamed with progress, written to a
//!    `.part` and renamed - a torn download must never look usable. A loaded
//!    model is cached until another is asked for, because loading one takes
//!    seconds.
//! 2. **Transcription.** The engine's audio decoder hands over the clip's
//!    window as 16 kHz mono floats - the input whisper wants - and the
//!    segments come back relative to the window.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use concat_host::{AppDirs, SingleFlight};
use concat_media::{AudioDecoder, AudioOptions, SampleFormat};
use serde::{Deserialize, Serialize};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperVadContext,
    WhisperVadContextParams, WhisperVadParams,
};

use crate::{DownloadProgress, JobStage};

/// One downloadable Whisper model. Sizes are approximate, for progress bars
/// when the server declines to send a Content-Length.
struct KnownModel {
    id: &'static str,
    label: &'static str,
    /// One line for the settings row: what this size trades away.
    blurb: &'static str,
    approx_bytes: u64,
    /// What the finished file must hash to; a download with nothing to
    /// check against is refused. See [`concat_host::models`].
    sha256: &'static str,
    english_only: bool,
}

/// The models the settings panel offers, fastest first.
///
/// All from the official `ggerganov/whisper.cpp` conversions. Nothing bigger
/// than `small`: `medium` is 1.5 GB and minutes-per-minute on CPU, which is
/// the opposite of what this feature is for.
const KNOWN_MODELS: &[KnownModel] = &[
    KnownModel {
        id: "tiny.en",
        label: "Tiny (English)",
        blurb: "Fastest draft. Rough on names and punctuation.",
        approx_bytes: 77_704_715,
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        english_only: true,
    },
    KnownModel {
        id: "tiny",
        label: "Tiny (Multilingual)",
        blurb: "Fastest draft, any language.",
        approx_bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        english_only: false,
    },
    KnownModel {
        id: "base.en",
        label: "Base (English)",
        blurb: "The sweet spot: solid captions at ~10x realtime.",
        approx_bytes: 147_964_211,
        sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        english_only: true,
    },
    KnownModel {
        id: "base",
        label: "Base (Multilingual)",
        blurb: "Solid captions, any language.",
        approx_bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        english_only: false,
    },
    KnownModel {
        id: "small.en",
        label: "Small (English)",
        blurb: "Noticeably better wording; a few times slower.",
        approx_bytes: 487_614_201,
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        english_only: true,
    },
    KnownModel {
        id: "small",
        label: "Small (Multilingual)",
        blurb: "Best quality offered, any language.",
        approx_bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        english_only: false,
    },
];

fn known(id: &str) -> Option<&'static KnownModel> {
    KNOWN_MODELS.iter().find(|model| model.id == id)
}

/// The file a model arrives as, and what it is called on the mirror.
fn model_archive(id: &str) -> String {
    format!("ggml-{id}.bin")
}

/// The whisper.cpp commit the table's digests were taken at: what the
/// fallback fetches is pinned to it, so `main` moving on changes nothing.
const UPSTREAM_COMMIT: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";

/// Where the mirror was filled from, and the second place a download tries.
fn model_upstream(id: &str) -> String {
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/{UPSTREAM_COMMIT}/ggml-{id}.bin")
}

/// Where downloaded models live: `<app data>/whisper-models/ggml-<id>.bin`.
fn models_dir(dirs: &AppDirs) -> Result<PathBuf, String> {
    let dir = dirs.data.join("whisper-models");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    Ok(dir)
}

/// whisper's abort callback: `user_data` is the run's cancel flag, and a
/// true here stops the computation where it stands. See `transcribe`.
unsafe extern "C" fn abort_when_set(user_data: *mut std::ffi::c_void) -> bool {
    // SAFETY: the caller passed `Arc::as_ptr` of an `AtomicBool` it keeps
    // alive for as long as whisper may call this.
    unsafe { (*(user_data as *const AtomicBool)).load(Ordering::Relaxed) }
}

/// Makes sure whisper has a CPU to run on, and names that CPU.
///
/// A build linked whisper statically - every one but the x86_64 Windows and
/// Linux bundles - has its CPU backend from the start. Those two link it as
/// shared libraries, with the kernels in one module per x86_64 generation
/// (ggml-cpu-sandybridge, -haswell, -icelake, ...), and ggml registers
/// nothing until it is told where to look: it then scores every module it
/// finds against the CPU and loads the best one. That is how the same
/// bundle runs AVX2, FMA and F16C on a Haswell or a Ryzen - eight times as
/// fast as the AVX-only build on a Ryzen 5 2600 (#388) - and still runs on
/// a Sandy Bridge (#193, #318).
///
/// The modules are beside the program on Windows and in its lib/ folder on
/// Linux; a `cargo test` binary is one folder further down, in deps/.
/// Only those folders are searched - not the working directory, which ggml
/// would otherwise also look in. ggml logs which module it chose.
///
/// Called once per process; later calls answer from the first.
pub fn cpu_backend() -> Result<String, String> {
    use whisper_rs::whisper_rs_sys as sys;

    fn cpu() -> Option<String> {
        // SAFETY: a lookup in ggml's registry of devices, which ggml
        // guards itself; the description is a C string ggml owns for the
        // life of the process.
        unsafe {
            let device = sys::ggml_backend_dev_by_type(
                sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_CPU,
            );
            if device.is_null() {
                return None;
            }
            let description = sys::ggml_backend_dev_description(device);
            Some(if description.is_null() {
                "CPU".to_owned()
            } else {
                std::ffi::CStr::from_ptr(description)
                    .to_string_lossy()
                    .into_owned()
            })
        }
    }

    static LOADED: std::sync::OnceLock<Result<String, String>> = std::sync::OnceLock::new();
    LOADED
        .get_or_init(|| {
            if let Some(cpu) = cpu() {
                return Ok(cpu);
            }
            let program = std::env::current_exe()
                .map_err(|error| format!("could not find the program's folder: {error}"))?;
            let beside = program.parent().unwrap_or(std::path::Path::new("."));
            let mut folders = vec![beside.to_path_buf(), beside.join("lib")];
            if beside.file_name().is_some_and(|name| name == "deps")
                && let Some(profile) = beside.parent()
            {
                folders.push(profile.to_path_buf());
            }
            for folder in &folders {
                // ggml reads the path as UTF-8 on every platform.
                let Some(text) = folder.to_str() else {
                    continue;
                };
                let Ok(path) = std::ffi::CString::new(text) else {
                    continue;
                };
                // SAFETY: a NUL-terminated path that outlives the call.
                unsafe { sys::ggml_backend_load_all_from_path(path.as_ptr()) };
                if let Some(cpu) = cpu() {
                    log::info!(
                        "whisper: CPU kernels loaded from {}: {cpu}",
                        folder.display()
                    );
                    return Ok(cpu);
                }
            }
            Err(format!(
                "whisper's CPU kernels (ggml-cpu-*) are missing from {} - reinstall Concat",
                folders[0].display()
            ))
        })
        .clone()
}

fn model_file(dirs: &AppDirs, id: &str) -> Result<PathBuf, String> {
    // Ids come from our own table; anything else is a bug, not input.
    known(id).ok_or_else(|| format!("unknown model {id:?}"))?;
    Ok(models_dir(dirs)?.join(format!("ggml-{id}.bin")))
}

/// Silero VAD v6.2.0 in whisper.cpp's format, from ggml-org/whisper-vad
/// (MIT; sha256 2aa269b7...6987). It tells whisper where the speech is, so
/// whisper hears only that and times each caption from where its words
/// begin. Without it a caption started where the one before had ended:
/// the silence between them went to the next line, which came up seconds
/// before anyone spoke (#438). Under a megabyte, so it ships inside the
/// app rather than as a second download.
const VAD_MODEL: &[u8] = include_bytes!("../assets/ggml-silero-v6.2.0.bin");

/// The spans of `samples` (16 kHz mono) that are speech, in seconds, by
/// Silero VAD. Empty when the model cannot be had or will not run: the
/// captions then keep whisper's own times, as they always did.
fn speech_spans(dirs: &AppDirs, samples: &[f32], threads: usize) -> Vec<(f64, f64)> {
    let spans = vad_model(dirs).and_then(|path| {
        let path = path
            .to_str()
            .ok_or_else(|| format!("{} is not a path whisper can open", path.display()))?
            .to_owned();
        let mut context_params = WhisperVadContextParams::new();
        context_params.set_n_threads(threads as i32);
        context_params.set_use_gpu(false);
        let mut context = WhisperVadContext::new(&path, context_params)
            .map_err(|error| format!("could not load {path}: {error}"))?;
        let segments = context
            .segments_from_samples(WhisperVadParams::new(), samples)
            .map_err(|error| format!("voice detection failed: {error}"))?;
        // Centiseconds, like whisper's own timestamps.
        Ok(segments
            .map(|span| (f64::from(span.start) / 100.0, f64::from(span.end) / 100.0))
            .collect())
    });
    spans.unwrap_or_else(|error: String| {
        log::warn!("transcribe: captions keep whisper's times: {error}");
        Vec::new()
    })
}

/// A caption's span drawn in to the speech it covers. Whisper starts a
/// caption where the one before it ended, so the silence between two lines
/// went to the second, which came up seconds before it was said (#438); the
/// voice detector knows where the words begin and end. A caption over no
/// detected speech - a whisper too quiet for the detector, say - keeps the
/// times whisper gave it.
fn on_speech(start: f64, end: f64, speech: &[(f64, f64)]) -> (f64, f64) {
    let mut covered = speech
        .iter()
        .filter(|(from, to)| *from < end && *to > start);
    let Some(&(first, mut last)) = covered.next() else {
        return (start, end);
    };
    if let Some(&(_, to)) = covered.next_back() {
        last = to;
    }
    (start.max(first), end.min(last))
}

/// The VAD model as a file whisper can open, written beside the
/// transcription models the first time it is wanted.
fn vad_model(dirs: &AppDirs) -> Result<PathBuf, String> {
    let path = models_dir(dirs)?.join("ggml-silero-v6.2.0.bin");
    let whole = std::fs::metadata(&path).is_ok_and(|meta| meta.len() == VAD_MODEL.len() as u64);
    if !whole {
        // Named for this process, so two windows writing it at once do
        // not write into one file; whichever renames last wins, and both
        // wrote the same bytes.
        let part = path.with_extension(format!("part-{}", std::process::id()));
        std::fs::write(&part, VAD_MODEL)
            .and_then(|()| std::fs::rename(&part, &path))
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    }
    Ok(path)
}

/// One model, as the settings panel shows it.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    /// The model id, e.g. "base.en".
    pub id: String,
    /// Display name.
    pub label: String,
    /// One line on what this size trades away.
    pub blurb: String,
    /// Whether the model only understands English.
    pub english_only: bool,
    /// Approximate download size in bytes, for display.
    pub size_bytes: u64,
    /// Whether the file is on disk.
    pub downloaded: bool,
}

/// What the settings panel shows.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TranscriberStatus {
    /// Where models are stored, so the settings panel can say so.
    pub models_dir: String,
    /// Every model the panel offers.
    pub models: Vec<ModelStatus>,
}

/// One caption, in seconds relative to the transcribed window's start.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    /// Seconds from the window's start.
    pub start: f64,
    /// Seconds from the window's start.
    pub end: f64,
    /// What was said.
    pub text: String,
}

/// What to transcribe.
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeRequest {
    /// The media file to transcribe.
    pub path: String,
    /// Which of the file's audio streams, by index; `None` is the first -
    /// the same choice the clip's playback makes, so the words come from
    /// the track that is heard.
    pub audio_stream: Option<u32>,
    /// Seconds into the file where the clip's source window begins.
    pub source_start: f64,
    /// How much source the clip covers, in seconds (`duration * speed`).
    pub window: f64,
    /// Which model to use, e.g. "base.en".
    pub model_id: String,
}

struct LoadedModel {
    id: String,
    context: WhisperContext,
}

/// The transcriber: the one transcription and the one model download that
/// can run at a time, and the model kept loaded between runs.
pub struct Transcriber {
    gate: Arc<SingleFlight>,
    downloads: Arc<SingleFlight>,
    /// Held for the whole transcription, which is what lets
    /// [`Transcriber::delete_model`] refuse to delete a model mid-use.
    model: Mutex<Option<LoadedModel>>,
}

impl Default for Transcriber {
    fn default() -> Self {
        Self::new()
    }
}

impl Transcriber {
    /// An idle transcriber, loading nothing until asked to transcribe.
    pub fn new() -> Self {
        Self {
            gate: Arc::new(SingleFlight::new()),
            downloads: Arc::new(SingleFlight::new()),
            model: Mutex::new(None),
        }
    }

    /// Per-model state, for the settings panel.
    pub fn status(dirs: &AppDirs) -> Result<TranscriberStatus, String> {
        let dir = models_dir(dirs)?;
        Ok(TranscriberStatus {
            models_dir: dir.to_string_lossy().into_owned(),
            models: KNOWN_MODELS
                .iter()
                .map(|model| ModelStatus {
                    id: model.id.to_owned(),
                    label: model.label.to_owned(),
                    blurb: model.blurb.to_owned(),
                    english_only: model.english_only,
                    size_bytes: model.approx_bytes,
                    downloaded: dir.join(format!("ggml-{}.bin", model.id)).is_file(),
                })
                .collect(),
        })
    }

    /// Streams one model from Concat's mirror into the models folder. Blocks
    /// for the whole download: run it on its own thread.
    ///
    /// Written to `<file>.part` and renamed at the end: a download killed
    /// halfway must never be mistaken for a model, because whisper would fail
    /// on it with a message that blames the model rather than the download.
    pub fn download_model(
        &self,
        dirs: &AppDirs,
        id: &str,
        mut progress: impl FnMut(DownloadProgress),
    ) -> Result<(), String> {
        let job = self.downloads.begin("model download")?;
        let destination = model_file(dirs, id)?;
        if destination.is_file() {
            return Ok(());
        }
        let estimate = known(id).map(|model| model.approx_bytes).unwrap_or(0);
        let partial = destination.with_extension("bin.part");
        let (received, total) = crate::fetch_model(
            &model_archive(id),
            &model_upstream(id),
            &partial,
            id,
            estimate,
            known(id).map(|model| model.sha256).unwrap_or(""),
            job.cancel_flag(),
            &mut progress,
        )?;
        std::fs::rename(&partial, &destination)
            .map_err(|error| format!("could not finish {}: {error}", destination.display()))?;
        progress(DownloadProgress {
            id: id.to_owned(),
            received,
            total,
            unpacking: false,
            done: true,
        });
        Ok(())
    }

    /// What model `id` is called as a file, and every address a download
    /// of it tries, in order: for a person whose network lets their
    /// browser through and not Concat, who can fetch the first in the
    /// browser and hand the file to [`concat_host::models::supply`] under
    /// that name. `None` for a model Concat does not know.
    pub fn model_sources(id: &str) -> Option<(String, Vec<String>)> {
        known(id)?;
        let file = model_archive(id);
        let sources = concat_host::models::sources(&file, &model_upstream(id));
        Some((file, sources))
    }

    /// Asks the running download to stop. Idle is a harmless no-op.
    pub fn cancel_download(&self) {
        self.downloads.cancel();
    }

    /// Removes a downloaded model file, unless a transcription is using it.
    pub fn delete_model(&self, dirs: &AppDirs, id: &str) -> Result<(), String> {
        let file = model_file(dirs, id)?;
        let mut loaded = self.model.try_lock().map_err(|_| {
            "a transcription is running - wait for it or cancel it first".to_owned()
        })?;
        if loaded.as_ref().is_some_and(|model| model.id == id) {
            *loaded = None;
        }
        std::fs::remove_file(&file)
            .map_err(|error| format!("could not remove {}: {error}", file.display()))
    }

    /// The model still loaded from the last run, which the next run on it
    /// does not load again. None while a run holds it.
    pub fn loaded_model(&self) -> Option<String> {
        let loaded = self.model.try_lock().ok()?;
        loaded.as_ref().map(|model| model.id.clone())
    }

    /// Transcribes one clip's audio window into caption segments. Blocks
    /// for the whole run: run it on its own thread. `progress` is told each
    /// stage as it begins, and in whisper's work each window it starts:
    /// whisper reads thirty seconds at a time, so `next` is a window on.
    ///
    /// Timestamps come back relative to the window: 0 is `source_start`. The
    /// caller owns the clip, so the map onto the timeline (divide by speed,
    /// add the clip's start) happens there, next to the numbers it needs.
    pub fn transcribe(
        &self,
        dirs: &AppDirs,
        request: &TranscribeRequest,
        mut progress: impl FnMut(JobStage) + Send + 'static,
    ) -> Result<Vec<Segment>, String> {
        let job = self.gate.begin("transcription")?;
        let cancel = job.cancel_handle();

        let model_path = model_file(dirs, &request.model_id)?;
        if !model_path.is_file() {
            return Err(format!(
                "model {} is not downloaded - see Settings > Transcriber",
                request.model_id
            ));
        }
        // NaN is refused along with zero and negative windows.
        if request.window.is_nan() || request.window <= 0.0 {
            return Err("nothing to transcribe: the clip covers no time".to_owned());
        }

        // The window, as whisper wants it: 16 kHz mono floats.
        progress(JobStage::Reading);
        let mut decoder = AudioDecoder::open(
            &request.path,
            &AudioOptions {
                start: Some(request.source_start),
                duration: Some(request.window),
                filters: Vec::new(),
                rate: 16_000,
                channels: 1,
                format: SampleFormat::F32,
                stream: request.audio_stream.map(|index| index as usize),
            },
        )
        .map_err(|error| error.to_string())?;
        let samples = decoder.collect_f32().map_err(|error| error.to_string())?;
        if samples.is_empty() {
            return Err("nothing to transcribe: the clip has no audio".to_owned());
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("transcription cancelled".to_owned());
        }

        let mut loaded = self
            .model
            .lock()
            .map_err(|_| "transcriber state poisoned".to_owned())?;
        if loaded.as_ref().map(|model| model.id.as_str()) != Some(request.model_id.as_str()) {
            // Load before overwriting: a failed load keeps the old model.
            progress(JobStage::Loading);
            cpu_backend()?;
            let context =
                WhisperContext::new_with_params(&model_path, WhisperContextParameters::default())
                    .map_err(|error| format!("could not load the model: {error}"))?;
            *loaded = Some(LoadedModel {
                id: request.model_id.clone(),
                context,
            });
        }
        let context = &loaded.as_ref().expect("model loaded above").context;
        let mut state = context
            .create_state()
            .map_err(|error| format!("could not start whisper: {error}"))?;

        // Two hardware threads are left to the rest of the app: playback,
        // the window and the decoders. On every one of a four-core
        // machine's eight, whisper starved them, and the preview stuttered
        // and its sound broke up for as long as a run went on (#388).
        let threads = std::thread::available_parallelism()
            .map(|count| count.get().saturating_sub(2).clamp(1, 8))
            .unwrap_or(4);
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(threads as i32);
        // The language is whisper's to hear, when it can hear more than
        // one: a multilingual model detects it from the first window. An
        // English-only model asked for "auto" still runs the detection,
        // over a vocabulary it was never trained on, and "hears" Thai at
        // one percent - so it is simply told English.
        params.set_language(Some(if context.is_multilingual() {
            "auto"
        } else {
            "en"
        }));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        // The raw hooks, not `set_abort_callback_safe`. whisper-rs 0.16's
        // safe one hands whisper a pointer to a boxed trait object with a
        // trampoline typed for the closure itself, so whisper reads the
        // box's two words as the closure and the answer is whatever byte
        // sits there: true, as a rule, and every encode was aborted before
        // it began - "failed to encode", error -6, on the first clip. The
        // flag itself is what whisper is given here, and it outlives the
        // call: `cancel` is held to the end of this function.
        // SAFETY: `abort_when_set` reads `user_data` as the `AtomicBool`
        // inside `cancel`, which lives until `full` has returned.
        unsafe {
            params.set_abort_callback(Some(abort_when_set));
            params.set_abort_callback_user_data(Arc::as_ptr(&cancel) as *mut std::ffi::c_void);
        }
        // Whisper calls back as it starts each window, with how far into
        // the sound it is; the window it starts reads thirty seconds more.
        let window = (30.0 / request.window).min(1.0) as f32;
        progress(JobStage::Working {
            done: 0.0,
            next: window,
        });
        params.set_progress_callback_safe(move |percent: i32| {
            let done = (percent as f32 / 100.0).clamp(0.0, 1.0);
            progress(JobStage::Working {
                done,
                next: (done + window).min(1.0),
            });
        });

        // Where the speech is, for timing the captions; see `VAD_MODEL`.
        // Without it the captions come out as whisper timed them.
        let speech = speech_spans(dirs, &samples, threads);
        let run = state.full(params, &samples);

        // An abort ends whisper's run as an error of its own - "failed to
        // encode", -6 - so a Cancel read as a failure, and the person was
        // told transcription failed when they had stopped it (#388).
        if let Err(error) = run {
            if cancel.load(Ordering::Relaxed) {
                return Err("transcription cancelled".to_owned());
            }
            return Err(format!("transcription failed: {error}"));
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("transcription cancelled".to_owned());
        }

        Ok(state
            .as_iter()
            .filter_map(|segment| {
                // Whisper's timestamps are centiseconds.
                let start = segment.start_timestamp() as f64 / 100.0;
                let end = segment.end_timestamp() as f64 / 100.0;
                let (start, end) = on_speech(start, end, &speech);
                let text = segment.to_str_lossy().ok()?.trim().to_owned();
                keep_segment(start, end, text)
            })
            .collect())
    }

    /// Asks the running transcription to stop at its next check.
    pub fn cancel(&self) {
        self.gate.cancel();
    }

    /// Whether a transcription is running.
    pub fn is_busy(&self) -> bool {
        self.gate.is_busy()
    }
}

/// Whisper marks silence and music with bracketed stage directions
/// ("[BLANK_AUDIO]", "(upbeat music)"). Those are not captions, and neither
/// is an empty or degenerate span.
fn keep_segment(start: f64, end: f64, text: String) -> Option<Segment> {
    if text.is_empty() || (text.starts_with('[') && text.ends_with(']')) {
        return None;
    }
    (end > start).then_some(Segment { start, end, text })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prints the captions a downloaded model makes of a file, for timing
    /// them by hand against the speech: `CONCAT_CAPTION_FILE=talk.wav
    /// CONCAT_CAPTION_MODEL=base.en cargo test -p concat-speech --
    /// --ignored --nocapture captions_of_a_file`.
    #[test]
    #[ignore = "needs a downloaded model and a file to caption"]
    fn captions_of_a_file() {
        let path = std::env::var("CONCAT_CAPTION_FILE").expect("CONCAT_CAPTION_FILE");
        let model_id = std::env::var("CONCAT_CAPTION_MODEL").unwrap_or("base.en".to_owned());
        let dirs = AppDirs::locate().expect("app dirs");
        // Past the end is fine: the decoder stops where the sound does.
        let window = 3600.0;
        let segments = Transcriber::new()
            .transcribe(
                &dirs,
                &TranscribeRequest {
                    path,
                    audio_stream: None,
                    source_start: 0.0,
                    window,
                    model_id,
                },
                |_| {},
            )
            .expect("transcribes");
        for segment in segments {
            println!(
                "{:7.2} {:7.2}  {}",
                segment.start, segment.end, segment.text
            );
        }
    }

    /// The reporter's clip in #438: five lines, each after a silence.
    /// Whisper started each where the last had ended; the speech says
    /// where it starts.
    #[test]
    fn a_caption_starts_and_ends_with_the_speech_it_covers() {
        let speech = [(3.07, 4.93), (14.08, 16.48), (25.09, 27.52)];
        assert_eq!(on_speech(0.0, 7.0, &speech), (3.07, 4.93));
        assert_eq!(on_speech(7.0, 18.0, &speech), (14.08, 16.48));
        // A line whisper split in two keeps its split.
        assert_eq!(on_speech(25.0, 26.0, &speech), (25.09, 26.0));
        assert_eq!(on_speech(26.0, 30.0, &speech), (26.0, 27.52));
        // Two spans under one caption: from the first to the last.
        assert_eq!(on_speech(0.0, 20.0, &speech), (3.07, 16.48));
        // Over no detected speech, or with no detector: whisper's times.
        assert_eq!(on_speech(40.0, 45.0, &speech), (40.0, 45.0));
        assert_eq!(on_speech(0.0, 7.0, &[]), (0.0, 7.0));
    }

    #[test]
    fn stage_directions_and_degenerate_spans_are_not_captions() {
        assert_eq!(
            keep_segment(0.0, 2.5, " Hello there.".trim().to_owned()).map(|s| s.text),
            Some("Hello there.".to_owned())
        );
        assert!(keep_segment(2.5, 2.5, "degenerate".to_owned()).is_none());
        assert!(keep_segment(3.0, 4.0, "[BLANK_AUDIO]".to_owned()).is_none());
        assert!(keep_segment(3.0, 4.0, String::new()).is_none());
    }

    #[test]
    fn every_known_model_is_asked_of_the_mirror_before_upstream() {
        for model in KNOWN_MODELS {
            let file = model_archive(model.id);
            assert_eq!(file, format!("ggml-{}.bin", model.id));
            assert!(model_upstream(model.id).ends_with(&file));
            let sources = concat_host::models::sources(&file, &model_upstream(model.id));
            let (mirror, upstream) = (&sources[0], &sources[1]);
            assert!(mirror.contains(concat_host::models::RELEASE));
            assert!(mirror.ends_with(&file));
            assert_eq!(*upstream, model_upstream(model.id));
            assert!(model.approx_bytes > 0);
        }
    }

    #[test]
    fn status_lists_every_model_undownloaded_in_an_empty_data_dir() {
        let scratch =
            std::env::temp_dir().join(format!("concat-whisper-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let status = Transcriber::status(&AppDirs::under(&scratch)).expect("status");
        assert_eq!(status.models.len(), KNOWN_MODELS.len());
        assert!(status.models.iter().all(|model| !model.downloaded));
        let _ = std::fs::remove_dir_all(&scratch);
    }
}

#[cfg(test)]
mod real_audio {
    use super::*;

    /// The whole path through whisper on a real file with a downloaded
    /// model - the thing the unit tests cannot cover, and what the abort
    /// hook broke. Ignored: it wants the app's model directory and a file
    /// named in CONCAT_TRANSCRIBE_AUDIO.
    ///
    ///   CONCAT_TRANSCRIBE_AUDIO=speech.wav cargo test -p concat-speech -- --ignored real_audio
    #[test]
    #[ignore = "needs a downloaded tiny.en and CONCAT_TRANSCRIBE_AUDIO"]
    fn transcribes_a_file_with_the_installed_model() {
        let Ok(audio) = std::env::var("CONCAT_TRANSCRIBE_AUDIO") else {
            eprintln!("CONCAT_TRANSCRIBE_AUDIO not set; nothing to transcribe");
            return;
        };
        let dirs = AppDirs::locate().expect("the app's directories");
        let transcriber = Transcriber::new();
        let request = TranscribeRequest {
            path: audio,
            audio_stream: None,
            source_start: 0.0,
            window: 30.0,
            model_id: "tiny.en".to_owned(),
        };
        let segments = transcriber
            .transcribe(&dirs, &request, |stage| eprintln!("{stage:?}"))
            .expect("transcription");
        for segment in &segments {
            eprintln!("{:.2}-{:.2} {}", segment.start, segment.end, segment.text);
        }
        assert!(!segments.is_empty(), "no words heard");
    }
}

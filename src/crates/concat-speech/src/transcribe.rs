// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Whisper transcription: audio in, timed caption segments out.
//!
//! whisper.cpp runs in-process through `whisper-rs`: no binary to ship or
//! find, and cancellation is a
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
//!    window as 16 kHz mono floats - the input whisper wants. Silero finds
//!    the speech in it, and whisper hears only that, laid end to end: the
//!    silence and the music between are where it made words up. The
//!    language is the one asked for, or the one heard over the speech,
//!    and it is fixed for the whole run. The segments come back relative
//!    to the window, with what whisper heard but did not mean - a line it
//!    had no confidence in over what it took for silence, a phrase looped
//!    - left out.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use concat_host::{AppDirs, SingleFlight};
use concat_media::{AudioDecoder, AudioOptions, SampleFormat};
use serde::{Deserialize, Serialize};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperSegment,
    WhisperState, WhisperVadContext, WhisperVadContextParams, WhisperVadParams,
};

/// Every language whisper's multilingual models know, by the code whisper
/// takes and the name the language calls itself - the way Settings lists
/// the interface's languages, so a person finds their own without reading
/// someone else's, and no list of a hundred names waits on 26
/// translations. In the order a reader of each script expects: the Latin
/// names alphabetically with their accents set aside, then each other
/// script together, alphabetically within it.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("af", "Afrikaans"),
    ("az", "Azərbaycanca"),
    ("id", "Bahasa Indonesia"),
    ("ms", "Bahasa Melayu"),
    ("jw", "Basa Jawa"),
    ("su", "Basa Sunda"),
    ("bs", "Bosanski"),
    ("br", "Brezhoneg"),
    ("ca", "Català"),
    ("cs", "Čeština"),
    ("sn", "ChiShona"),
    ("cy", "Cymraeg"),
    ("da", "Dansk"),
    ("de", "Deutsch"),
    ("et", "Eesti"),
    ("en", "English"),
    ("es", "Español"),
    ("eu", "Euskara"),
    ("fo", "Føroyskt"),
    ("fr", "Français"),
    ("gl", "Galego"),
    ("ha", "Hausa"),
    ("hr", "Hrvatski"),
    ("is", "Íslenska"),
    ("it", "Italiano"),
    ("sw", "Kiswahili"),
    ("ht", "Kreyòl ayisyen"),
    ("la", "Latina"),
    ("lv", "Latviešu"),
    ("lb", "Lëtzebuergesch"),
    ("lt", "Lietuvių"),
    ("ln", "Lingála"),
    ("hu", "Magyar"),
    ("mg", "Malagasy"),
    ("mt", "Malti"),
    ("mi", "Māori"),
    ("nl", "Nederlands"),
    ("no", "Norsk bokmål"),
    ("nn", "Norsk nynorsk"),
    ("oc", "Occitan"),
    ("haw", "ʻŌlelo Hawaiʻi"),
    ("uz", "Oʻzbekcha"),
    ("pl", "Polski"),
    ("pt", "Português"),
    ("ro", "Română"),
    ("sq", "Shqip"),
    ("sk", "Slovenčina"),
    ("sl", "Slovenščina"),
    ("so", "Soomaali"),
    ("fi", "Suomi"),
    ("sv", "Svenska"),
    ("tl", "Tagalog"),
    ("vi", "Tiếng Việt"),
    ("tr", "Türkçe"),
    ("tk", "Türkmençe"),
    ("yo", "Yorùbá"),
    ("el", "Ελληνικά"),
    ("ba", "Башҡортса"),
    ("be", "Беларуская"),
    ("bg", "Български"),
    ("kk", "Қазақ тілі"),
    ("mk", "Македонски"),
    ("mn", "Монгол"),
    ("ru", "Русский"),
    ("sr", "Српски"),
    ("tt", "Татарча"),
    ("tg", "Тоҷикӣ"),
    ("uk", "Українська"),
    ("hy", "Հայերեն"),
    ("yi", "ייִדיש"),
    ("he", "עברית"),
    ("ur", "اردو"),
    ("ar", "العربية"),
    ("sd", "سنڌي"),
    ("fa", "فارسی"),
    ("ps", "پښتو"),
    ("ne", "नेपाली"),
    ("mr", "मराठी"),
    ("sa", "संस्कृतम्"),
    ("hi", "हिन्दी"),
    ("as", "অসমীয়া"),
    ("bn", "বাংলা"),
    ("pa", "ਪੰਜਾਬੀ"),
    ("gu", "ગુજરાતી"),
    ("ta", "தமிழ்"),
    ("te", "తెలుగు"),
    ("kn", "ಕನ್ನಡ"),
    ("ml", "മലയാളം"),
    ("si", "සිංහල"),
    ("th", "ไทย"),
    ("lo", "ລາວ"),
    ("bo", "བོད་སྐད"),
    ("my", "မြန်မာ"),
    ("ka", "ქართული"),
    ("ko", "한국어"),
    ("am", "አማርኛ"),
    ("km", "ខ្មែរ"),
    ("zh", "中文"),
    ("ja", "日本語"),
    ("yue", "粵語"),
];

/// Whether model `id` hears only English: its language is not a choice.
pub fn english_only(id: &str) -> bool {
    known(id).is_some_and(|model| model.english_only)
}

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
/// Silero VAD. `None` when the model cannot be had or will not run: whisper
/// then hears the whole of the sound and the captions keep its own times,
/// as they always did.
///
/// Silero is run here, by itself, rather than through whisper's own VAD
/// switch (`FullParams::enable_vad`). In whisper.cpp 1.8.3 that switch is
/// read only by `whisper_full` and `whisper_full_parallel`, which work on
/// the context's built-in state; whisper-rs runs `whisper_full_with_state`
/// on a state of its own, which ignores the switch altogether, and its
/// contexts are made without a built-in state, which `whisper_vad` writes
/// through regardless - a null pointer. Cutting the speech out here works
/// on every path, and keeps the spans for timing the captions as well.
fn speech_spans(dirs: &AppDirs, samples: &[f32], threads: usize) -> Option<Vec<(f64, f64)>> {
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
    spans
        .map_err(|error: String| {
            log::warn!("transcribe: whisper hears all of the sound, and times it: {error}");
        })
        .ok()
}

/// What whisper is given to hear: the speech of the sound, cut out and
/// laid end to end, and where each piece of it came from - so a time in
/// what whisper heard can be told as a time in the sound.
#[derive(Debug)]
struct SpeechReel<'a> {
    samples: Cow<'a, [f32]>,
    pieces: Vec<Piece>,
}

/// One stretch of the sound in the reel: `len` samples, `at` into the reel
/// and `from` into the sound.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Piece {
    at: usize,
    from: usize,
    len: usize,
}

/// Sound kept before a span of speech, and after it. Silero's edges are
/// tight, and it misses a quiet start outright: in the measurement in the
/// tests the opening "Ini adalah" of a sentence came a second and a half
/// ahead of where Silero heard speech begin, and without it whisper lost
/// the words - and, conditioned on its own unpunctuated first line, the
/// full stops of every line after. Real sound, not silence laid in: a
/// stretch of digital zero between pieces did the same.
const LEAD_IN: f64 = 1.5;
const TAIL: f64 = 0.5;

impl<'a> SpeechReel<'a> {
    /// The whole of the sound, as it is.
    fn whole(samples: &'a [f32]) -> Self {
        SpeechReel {
            pieces: vec![Piece {
                at: 0,
                from: 0,
                len: samples.len(),
            }],
            samples: Cow::Borrowed(samples),
        }
    }

    /// The `speech` of `samples` with [`LEAD_IN`] before each span and
    /// [`TAIL`] after it, end to end. Spans whose padding meets are one
    /// piece with the pause between kept as it was said, so a pause of
    /// two seconds or more is cut to two seconds of its own sound, and a
    /// shorter one is not cut at all.
    fn cut(samples: &[f32], speech: &[(f64, f64)]) -> SpeechReel<'static> {
        let rate = f64::from(SAMPLE_RATE);
        let to_sample = |seconds: f64| ((seconds.max(0.0) * rate) as usize).min(samples.len());
        let mut stretches: Vec<(usize, usize)> = Vec::new();
        for &(start, end) in speech {
            let (from, to) = (to_sample(start - LEAD_IN), to_sample(end + TAIL));
            if to <= from {
                continue;
            }
            match stretches.last_mut() {
                Some(last) if from <= last.1 => last.1 = last.1.max(to),
                _ => stretches.push((from, to)),
            }
        }
        let mut reel = Vec::new();
        let mut pieces = Vec::with_capacity(stretches.len());
        for (from, to) in stretches {
            pieces.push(Piece {
                at: reel.len(),
                from,
                len: to - from,
            });
            reel.extend_from_slice(&samples[from..to]);
        }
        SpeechReel {
            samples: Cow::Owned(reel),
            pieces,
        }
    }

    /// Seconds into the reel as seconds into the sound. Past the end is
    /// the end of the last piece.
    fn to_source(&self, seconds: f64) -> f64 {
        let rate = f64::from(SAMPLE_RATE);
        let at = (seconds.max(0.0) * rate).round() as usize;
        self.pieces
            .iter()
            .find(|piece| at <= piece.at + piece.len)
            .or(self.pieces.last())
            .map_or(seconds, |piece| {
                (piece.from + at.clamp(piece.at, piece.at + piece.len) - piece.at) as f64 / rate
            })
    }

    fn seconds(&self) -> f64 {
        self.samples.len() as f64 / f64::from(SAMPLE_RATE)
    }
}

/// The rate whisper hears at.
const SAMPLE_RATE: u32 = 16_000;

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
    /// The language spoken, by whisper's code ("id", "de", ...; see
    /// [`LANGUAGES`]). `None`, "" or "auto" is whisper's to hear. An
    /// English-only model hears English whatever this says.
    #[serde(default)]
    pub language: Option<String>,
}

struct LoadedModel {
    id: String,
    /// Loaded for the GPU; the measurement in the tests also runs one on
    /// the CPU, which is what most machines that run Concat have.
    gpu: bool,
    context: WhisperContext,
}

/// How a run decodes. [`Recipe::SHIPPED`] is what the app runs. The
/// measurement in the tests also runs [`Recipe::BEFORE`], how Concat
/// decoded until the Indonesian report - greedy, the language heard from
/// the file's first thirty seconds, every second of the sound - so what
/// each change is worth can be measured again and not taken on trust.
#[derive(Clone, Copy, Debug)]
struct Recipe {
    /// Candidates beam search keeps per step; 1 is greedy decoding.
    beam: i32,
    /// Greedy decoding's candidates when a window is read again at a
    /// higher temperature; see `run`.
    best_of: i32,
    /// Whisper hears only the speech Silero found, laid end to end.
    speech_only: bool,
    /// The language is heard over the speech before the run, and fixed.
    detect_over_speech: bool,
    /// Lines whisper made up of silence, and its loops, left out.
    prune: bool,
    /// The model on the GPU, where the build has one.
    gpu: bool,
}

impl Recipe {
    const SHIPPED: Recipe = Recipe {
        beam: 1,
        best_of: 5,
        speech_only: true,
        detect_over_speech: true,
        prune: true,
        gpu: true,
    };

    #[cfg(test)]
    const BEFORE: Recipe = Recipe {
        beam: 1,
        best_of: 1,
        speech_only: false,
        detect_over_speech: false,
        prune: false,
        gpu: true,
    };
}

// Greedy decoding, not beam search. Beam search five wide - whisper's
// default for it, and what OpenAI's figures were taken with - was
// measured against greedy on an Indonesian reel (the measurement in the
// tests, FLEURS read speech among music and silence, base and small, on
// the Mac's GPU and its CPU): it took 1.2 to 1.9 times as long, its word
// error rate came out a point either side of greedy's, and in three runs
// of eight it left a whole sentence out, which greedy never did. Greedy
// with whisper's fallback reads an unsure window again at higher
// temperatures, five candidates at a time (`best_of`), which is where
// beam search's care pays off, and only where it is needed.

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
        progress: impl FnMut(JobStage) + Send + 'static,
    ) -> Result<Vec<Segment>, String> {
        self.run(dirs, request, Recipe::SHIPPED, progress)
    }

    fn run(
        &self,
        dirs: &AppDirs,
        request: &TranscribeRequest,
        recipe: Recipe,
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
        let asked = asked_language(request.language.as_deref())?;

        // The window, as whisper wants it: 16 kHz mono floats.
        progress(JobStage::Reading);
        let mut decoder = AudioDecoder::open(
            &request.path,
            &AudioOptions {
                start: Some(request.source_start),
                duration: Some(request.window),
                filters: Vec::new(),
                rate: SAMPLE_RATE,
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

        // Two hardware threads are left to the rest of the app: playback,
        // the window and the decoders. On every one of a four-core
        // machine's eight, whisper starved them, and the preview stuttered
        // and its sound broke up for as long as a run went on (#388).
        let threads = std::thread::available_parallelism()
            .map(|count| count.get().saturating_sub(2).clamp(1, 8))
            .unwrap_or(4);

        // Where the speech is; see `VAD_MODEL`. Whisper hears only that,
        // and the captions are timed by it. A clip in which Silero hears no
        // speech at all has nothing to caption: whisper given music or
        // silence alone is where "thanks for watching" came from.
        let speech = speech_spans(dirs, &samples, threads);
        let reel = match &speech {
            Some(spans) if recipe.speech_only => SpeechReel::cut(&samples, spans),
            _ => SpeechReel::whole(&samples),
        };
        if reel.samples.is_empty() {
            log::info!(
                "transcribe: no speech heard in {:.1} s of sound",
                samples.len() as f64 / f64::from(SAMPLE_RATE)
            );
            return Ok(Vec::new());
        }
        let speech = speech.unwrap_or_default();
        if recipe.speech_only {
            log::info!(
                "transcribe: {:.1} s of speech in {} piece(s), of {:.1} s of sound",
                reel.seconds(),
                reel.pieces.len(),
                samples.len() as f64 / f64::from(SAMPLE_RATE)
            );
        }

        let mut loaded = self
            .model
            .lock()
            .map_err(|_| "transcriber state poisoned".to_owned())?;
        if loaded
            .as_ref()
            .is_none_or(|model| model.id != request.model_id || model.gpu != recipe.gpu)
        {
            // Load before overwriting: a failed load keeps the old model.
            progress(JobStage::Loading);
            cpu_backend()?;
            let mut context_params = WhisperContextParameters::default();
            if !recipe.gpu {
                context_params.use_gpu(false);
            }
            let context = WhisperContext::new_with_params(&model_path, context_params)
                .map_err(|error| format!("could not load the model: {error}"))?;
            *loaded = Some(LoadedModel {
                id: request.model_id.clone(),
                gpu: recipe.gpu,
                context,
            });
        }
        let context = &loaded.as_ref().expect("model loaded above").context;
        let mut state = context
            .create_state()
            .map_err(|error| format!("could not start whisper: {error}"))?;

        // Whisper calls back as it starts each window, with how far into
        // the sound it is; the window it starts reads thirty seconds more.
        let window = (30.0 / reel.seconds()).min(1.0) as f32;
        progress(JobStage::Working {
            done: 0.0,
            next: window,
        });

        // The language. An English-only model asked for "auto" still runs
        // the detection, over a vocabulary it was never trained on, and
        // "hears" Thai at one percent - so it is simply told English. A
        // multilingual one is told the language asked for, or hears it
        // over the speech here; left to itself, whisper hears it once from
        // the first thirty seconds of whatever it is given, which in a
        // video is as often the intro music as anyone talking.
        let language: &'static str = if !context.is_multilingual() {
            "en"
        } else if let Some(code) = asked {
            code
        } else if recipe.detect_over_speech {
            let code = detect_language(&mut state, &reel.samples, threads)?;
            if cancel.load(Ordering::Relaxed) {
                return Err("transcription cancelled".to_owned());
            }
            code
        } else {
            "auto"
        };

        let strategy = if recipe.beam > 1 {
            SamplingStrategy::BeamSearch {
                beam_size: recipe.beam,
                // Whisper's default: no patience factor.
                patience: -1.0,
            }
        } else {
            SamplingStrategy::Greedy {
                best_of: recipe.best_of,
            }
        };
        let mut params = FullParams::new(strategy);
        params.set_n_threads(threads as i32);
        params.set_language(Some(language));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        // Non-speech tokens are left to whisper: "[Music]", "(tepuk
        // tangan)" and "♪" are how it says what it hears is not words,
        // and `keep_segment` drops those lines. Suppressed
        // (`set_suppress_nst`), they leave it nothing to say over music
        // but words: in the measurement it captioned a music intro
        // "KEMBATAN PIRAMIDANIYA DALAM GELAP".
        //
        // The temperature fallback is whisper's own and kept: a window
        // whose best reading is unsure (log probability under -1) or loops
        // (compression past 2.4) is read again a little less greedily,
        // and a window it takes for silence (no-speech over 0.6) with an
        // unsure reading is dropped. These are whisper's defaults, set
        // here so they hold if whisper's change.
        params.set_temperature(0.0);
        params.set_temperature_inc(0.2);
        params.set_logprob_thold(LOGPROB_THOLD);
        params.set_no_speech_thold(NO_SPEECH_THOLD);
        params.set_entropy_thold(2.4);
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
        params.set_progress_callback_safe(move |percent: i32| {
            let done = (percent as f32 / 100.0).clamp(0.0, 1.0);
            progress(JobStage::Working {
                done,
                next: (done + window).min(1.0),
            });
        });

        let run = state.full(params, &reel.samples);

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

        let eot = context.token_eot();
        let lines = state.as_iter().filter_map(|segment| {
            if recipe.prune && made_up(&segment, eot) {
                log::debug!(
                    "transcribe: dropped as made up of silence: {:?}",
                    segment.to_str_lossy().unwrap_or_default()
                );
                return None;
            }
            // Whisper's timestamps are centiseconds, in what it heard.
            let start = reel.to_source(segment.start_timestamp() as f64 / 100.0);
            let end = reel.to_source(segment.end_timestamp() as f64 / 100.0);
            let (start, end) = on_speech(start, end, &speech);
            let mut text = segment.to_str_lossy().ok()?.trim().to_owned();
            if recipe.prune {
                text = unloop(&text);
            }
            keep_segment(start, end, text)
        });
        let lines: Vec<Segment> = lines.collect();
        Ok(if recipe.prune {
            drop_repeats(lines)
        } else {
            lines
        })
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
/// ("[BLANK_AUDIO]", "(upbeat music)") or with notes ("♪"). Those are not
/// captions, and neither is a line with no letter or digit in it, nor an
/// empty or degenerate span.
fn keep_segment(start: f64, end: f64, text: String) -> Option<Segment> {
    let direction = (text.starts_with('[') && text.ends_with(']'))
        || (text.starts_with('(') && text.ends_with(')'));
    if direction || !text.chars().any(char::is_alphanumeric) {
        return None;
    }
    (end > start).then_some(Segment { start, end, text })
}

/// Whisper's own lines for what is not speech: a window it gives better
/// than even odds of being silence, read with an average token log
/// probability under -1, is silence it has put words to. OpenAI's numbers,
/// and whisper.cpp's defaults.
const NO_SPEECH_THOLD: f32 = 0.6;
const LOGPROB_THOLD: f32 = -1.0;

/// The language asked for, as the code in [`LANGUAGES`]; `None` for none,
/// "" or "auto", which leave it to be heard. A code whisper does not know
/// is refused rather than quietly heard as something else.
fn asked_language(code: Option<&str>) -> Result<Option<&'static str>, String> {
    let code = code.map(str::trim).unwrap_or_default();
    if code.is_empty() || code.eq_ignore_ascii_case("auto") {
        return Ok(None);
    }
    LANGUAGES
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(code))
        .map(|(known, _)| Some(*known))
        .ok_or_else(|| format!("whisper does not know the language {code:?}"))
}

/// How many thirty-second stretches of the speech its language is heard
/// over: the first, and as many as two more after it, their verdicts
/// added. One stretch is a few sentences, and on a pair of languages as
/// close as Indonesian and Malay a few sentences tip either way.
const LANGUAGE_PROBES: usize = 3;
/// Speech a stretch after the first needs, in seconds, to be heard at all.
const LANGUAGE_PROBE_MIN: f64 = 10.0;

/// The language of `samples` - the speech alone, see [`SpeechReel`] - by
/// whisper's own detector over up to [`LANGUAGE_PROBES`] stretches of it.
/// Each stretch costs one pass of the encoder, a window's worth of
/// whisper's work less the decoding.
fn detect_language(
    state: &mut WhisperState,
    samples: &[f32],
    threads: usize,
) -> Result<&'static str, String> {
    state
        .pcm_to_mel(samples, threads)
        .map_err(|error| format!("could not hear the language: {error}"))?;
    let seconds = samples.len() as f64 / f64::from(SAMPLE_RATE);
    let mut sum: Vec<f32> = Vec::new();
    let mut probes = 0;
    for probe in 0..LANGUAGE_PROBES {
        let offset = probe as f64 * 30.0;
        if probe > 0 && seconds - offset < LANGUAGE_PROBE_MIN {
            break;
        }
        let (_, odds) = state
            .lang_detect((offset * 1000.0) as usize, threads)
            .map_err(|error| format!("could not hear the language: {error}"))?;
        if sum.is_empty() {
            sum = odds;
        } else {
            sum.iter_mut()
                .zip(odds)
                .for_each(|(sum, odds)| *sum += odds);
        }
        probes += 1;
    }
    let mut ranked: Vec<(i32, f32)> = sum
        .iter()
        .enumerate()
        .map(|(id, odds)| (id as i32, odds / probes as f32))
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let name = |id: i32| whisper_rs::get_lang_str(id).unwrap_or("?");
    log::info!(
        "transcribe: heard {} (p = {:.3}) over {probes} stretch(es) of speech; then {} ({:.3}), {} ({:.3})",
        name(ranked[0].0),
        ranked[0].1,
        name(ranked[1].0),
        ranked[1].1,
        name(ranked[2].0),
        ranked[2].1,
    );
    whisper_rs::get_lang_str(ranked[0].0).ok_or_else(|| {
        format!(
            "whisper heard language {}, which it cannot name",
            ranked[0].0
        )
    })
}

/// A line whisper made up of silence: one from a window it took for
/// silence (see [`NO_SPEECH_THOLD`]) that it was unsure of besides.
/// Whisper drops such a window whole when the window's reading as a whole
/// is unsure; this takes the same test to each line of a window it kept,
/// where a sure sentence can be followed by an invented one.
fn made_up(segment: &WhisperSegment, eot: whisper_rs::WhisperTokenId) -> bool {
    if segment.no_speech_probability() <= NO_SPEECH_THOLD {
        return false;
    }
    let (sum, count) = (0..segment.n_tokens())
        .filter_map(|index| segment.get_token(index))
        .map(|token| token.token_data())
        // Timestamps and the other special tokens sit past end-of-text.
        .filter(|data| data.id < eot)
        .fold((0.0, 0), |(sum, count), data| (sum + data.plog, count + 1));
    count == 0 || sum / (count as f32) < LOGPROB_THOLD
}

/// The longest run of words [`unloop`] looks for a repeat of.
const LOOP_UNIT: usize = 8;
/// Words a run of repeats must cover before it is taken for a loop: "no,
/// no, no" is said; six words of it is whisper stuck.
const LOOP_WORDS: usize = 6;

/// A line with whisper's loops taken out: a phrase said three times or
/// more in a row, across six words or more, is said once - the last time,
/// so the line keeps its closing stop. Whisper caught in a loop repeats a
/// phrase until the window runs out ("terima kasih terima kasih ...").
/// Words are compared without case or punctuation. A script without
/// spaces between words is left as it is.
fn unloop(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let plain: Vec<String> = words
        .iter()
        .map(|word| {
            word.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect()
        })
        .collect();
    let mut kept: Vec<&str> = Vec::with_capacity(words.len());
    let mut at = 0;
    while at < words.len() {
        // The unit whose repeats from here cover the most words.
        let mut best: Option<(usize, usize)> = None;
        for unit in 1..=LOOP_UNIT.min(words.len() - at) {
            let mut times = 1;
            while at + (times + 1) * unit <= words.len()
                && (0..unit).all(|k| plain[at + k] == plain[at + times * unit + k])
            {
                times += 1;
            }
            let covers = times * unit;
            if times >= 3
                && covers >= LOOP_WORDS
                && best.is_none_or(|(unit, times)| covers > unit * times)
            {
                best = Some((unit, times));
            }
        }
        match best {
            Some((unit, times)) => {
                let last = at + (times - 1) * unit;
                kept.extend_from_slice(&words[last..last + unit]);
                at += times * unit;
            }
            None => {
                kept.push(words[at]);
                at += 1;
            }
        }
    }
    let unlooped = kept.join(" ");
    if kept.len() < words.len() {
        log::debug!("transcribe: a loop taken out of {text:?}");
        unlooped
    } else {
        // Untouched, and with its own spacing.
        text.to_owned()
    }
}

/// The lines without a loop that runs from one line to the next: the
/// same words three lines running or more are kept once, the first time,
/// at its own time. Twice in a row is left alone - "Yes." "Yes." is said.
fn drop_repeats(lines: Vec<Segment>) -> Vec<Segment> {
    let plain = |text: &str| -> String {
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let mut out: Vec<Segment> = Vec::with_capacity(lines.len());
    let mut run = 0;
    for line in lines {
        let same = out
            .last()
            .is_some_and(|last| plain(&last.text) == plain(&line.text));
        run = if same { run + 1 } else { 1 };
        if run < 3 {
            out.push(line);
        } else if run == 3 {
            // The second was the loop's first repeat, not a line said twice.
            out.pop();
        }
    }
    out
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
                    language: std::env::var("CONCAT_CAPTION_LANGUAGE").ok(),
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
        assert!(keep_segment(3.0, 4.0, "(musik)".to_owned()).is_none());
        assert!(keep_segment(3.0, 4.0, "♪ ♪".to_owned()).is_none());
        assert!(keep_segment(3.0, 4.0, "Lalu (sambil tertawa) pergi.".to_owned()).is_some());
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

    /// The list is every language whisper knows, each once, under the
    /// code whisper takes.
    #[test]
    fn every_whisper_language_is_offered_once() {
        let mut codes: Vec<&str> = LANGUAGES.iter().map(|(code, _)| *code).collect();
        for code in &codes {
            assert!(
                whisper_rs::get_lang_id(code).is_some(),
                "whisper does not know {code}"
            );
        }
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), LANGUAGES.len());
        assert_eq!(codes.len() as i32, whisper_rs::get_lang_max_id() + 1);
        assert!(LANGUAGES.iter().all(|(_, name)| !name.trim().is_empty()));
    }

    #[test]
    fn a_language_is_asked_for_by_its_code_or_left_to_be_heard() {
        assert_eq!(asked_language(None), Ok(None));
        assert_eq!(asked_language(Some("")), Ok(None));
        assert_eq!(asked_language(Some("auto")), Ok(None));
        assert_eq!(asked_language(Some("id")), Ok(Some("id")));
        assert_eq!(asked_language(Some(" ID ")), Ok(Some("id")));
        assert!(asked_language(Some("klingon")).is_err());
        assert!(english_only("base.en"));
        assert!(!english_only("small"));
    }

    /// Twenty seconds of sound with speech at 2-3, 4-5 and 12-13: the
    /// first two are a short pause apart and stay one piece with the pause
    /// in it; the long silence before the third is cut to the lead-in and
    /// tail around it, of the sound's own samples.
    #[test]
    fn the_reel_is_the_speech_and_its_times_are_the_sound_s() {
        let rate = f64::from(SAMPLE_RATE);
        let at = |seconds: f64| (seconds * rate) as usize;
        let samples: Vec<f32> = (0..20 * SAMPLE_RATE).map(|n| n as f32).collect();
        let reel = SpeechReel::cut(&samples, &[(2.0, 3.0), (4.0, 5.0), (12.0, 13.0)]);
        let first = Piece {
            at: 0,
            from: at(2.0 - LEAD_IN),
            len: at(5.0 + TAIL) - at(2.0 - LEAD_IN),
        };
        let second = Piece {
            at: first.len,
            from: at(12.0 - LEAD_IN),
            len: at(13.0 + TAIL) - at(12.0 - LEAD_IN),
        };
        assert_eq!(reel.pieces, [first, second]);
        assert_eq!(reel.samples.len(), first.len + second.len);
        assert_eq!(reel.samples[0], samples[first.from]);
        assert_eq!(reel.samples[second.at], samples[second.from]);
        // Times in the reel are the sound's.
        let close = |a: f64, b: f64| (a - b).abs() < 1e-3;
        assert!(close(reel.to_source(0.0), 2.0 - LEAD_IN));
        assert!(close(reel.to_source(1.0), 3.0 - LEAD_IN));
        let into_second = second.at as f64 / rate + 0.5;
        assert!(close(reel.to_source(into_second), 12.0 - LEAD_IN + 0.5));
        // Past the end is the end of the last piece.
        assert!(close(reel.to_source(99.0), 13.0 + TAIL));
        // Speech at the very start is padded only as far as the start.
        let early = SpeechReel::cut(&samples, &[(0.2, 1.0)]);
        assert_eq!(early.pieces[0].from, 0);
        // No speech, no pieces; the whole is the sound as it is.
        assert!(SpeechReel::cut(&samples, &[]).pieces.is_empty());
        let whole = SpeechReel::whole(&samples);
        assert!(close(whole.to_source(4.2), 4.2));
        assert!(close(whole.seconds(), 20.0));
    }

    #[test]
    fn a_loop_inside_a_line_is_said_once() {
        assert_eq!(
            unloop("Terima kasih, terima kasih, terima kasih, terima kasih."),
            "terima kasih."
        );
        assert_eq!(
            unloop("Jadi ya ya ya ya ya ya ya kita mulai."),
            "Jadi ya kita mulai."
        );
        // Said, not looped: short repeats, and a phrase twice.
        assert_eq!(unloop("No, no, no!"), "No, no, no!");
        assert_eq!(
            unloop("Satu dua tiga, satu dua tiga."),
            "Satu dua tiga, satu dua tiga."
        );
        assert_eq!(unloop("  Spacing  kept "), "  Spacing  kept ");
        assert_eq!(unloop(""), "");
    }

    #[test]
    fn a_line_repeated_three_times_running_is_kept_once() {
        let line = |start: f64, text: &str| Segment {
            start,
            end: start + 1.0,
            text: text.to_owned(),
        };
        let kept = drop_repeats(vec![
            line(0.0, "Yes."),
            line(1.0, "yes"),
            line(2.0, "Go on."),
            line(3.0, "Thanks for watching!"),
            line(4.0, "Thanks for watching!"),
            line(5.0, "Thanks for watching!"),
            line(6.0, "Thanks for watching."),
            line(7.0, "Bye."),
        ]);
        let text: Vec<&str> = kept.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            text,
            ["Yes.", "yes", "Go on.", "Thanks for watching!", "Bye."]
        );
        assert_eq!(kept[3].start, 3.0);
    }
}

/// How the recipe measures, on a recording with a known transcript:
/// word error rate and real-time factor, before and after, per model.
///
///   CONCAT_WER_AUDIO=reel.wav CONCAT_WER_REFERENCE=reel.txt \
///   CONCAT_WER_DATA=<a folder with whisper-models/ in it> \
///   CONCAT_WER_MODELS=base,small CONCAT_WER_CPU=1 \
///   cargo test --profile quick -p concat-speech -- --ignored --nocapture word_error_rate
///
/// The reference is plain text; case and punctuation are not counted.
/// CONCAT_WER_DATA defaults to the app's own data folder, CONCAT_WER_MODELS
/// to base and small, and CONCAT_WER_CPU=1 adds the runs on the CPU.
#[cfg(test)]
mod accuracy {
    use super::*;
    use std::time::Instant;

    /// The words of a text as they are counted: lower case, letters and
    /// digits, everything else a space.
    fn words(text: &str) -> Vec<String> {
        text.chars()
            .map(|c| if c.is_alphanumeric() { c } else { ' ' })
            .flat_map(char::to_lowercase)
            .collect::<String>()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }

    /// Word error rate: the edits from `hypothesis` to `reference`, over
    /// the reference's words.
    fn wer(reference: &[String], hypothesis: &[String]) -> f64 {
        let mut row: Vec<usize> = (0..=hypothesis.len()).collect();
        for (i, said) in reference.iter().enumerate() {
            let mut diagonal = row[0];
            row[0] = i + 1;
            for (j, heard) in hypothesis.iter().enumerate() {
                let substitute = diagonal + usize::from(said != heard);
                diagonal = row[j + 1];
                row[j + 1] = substitute.min(row[j] + 1).min(row[j + 1] + 1);
            }
        }
        row[hypothesis.len()] as f64 / reference.len().max(1) as f64
    }

    #[test]
    fn word_error_rate_counts_edits_over_the_reference() {
        let r = words("Agama utama di Moldova adalah Kristen Ortodoks.");
        assert_eq!(wer(&r, &r), 0.0);
        let h = words("agama utama di moldova ialah kristen");
        assert!((wer(&r, &h) - 2.0 / 7.0).abs() < 1e-9);
    }

    /// The log, on stderr, so the language heard shows beside the numbers.
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::max_level()
        }
        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) && record.target().starts_with("concat_speech") {
                eprintln!("  [{}] {}", record.level(), record.args());
            }
        }
        fn flush(&self) {}
    }

    #[test]
    #[ignore = "needs downloaded models and CONCAT_WER_AUDIO / CONCAT_WER_REFERENCE"]
    fn word_error_rate() {
        // CONCAT_WER_DEBUG=1 adds the lines dropped and the loops cut.
        let level = if std::env::var("CONCAT_WER_DEBUG").is_ok_and(|value| value == "1") {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        };
        let _ = log::set_logger(&Stderr).map(|()| log::set_max_level(level));
        let audio = std::env::var("CONCAT_WER_AUDIO").expect("CONCAT_WER_AUDIO");
        let reference = std::fs::read_to_string(
            std::env::var("CONCAT_WER_REFERENCE").expect("CONCAT_WER_REFERENCE"),
        )
        .expect("the reference");
        let reference = words(&reference);
        let dirs = match std::env::var("CONCAT_WER_DATA") {
            Ok(root) => AppDirs::under(std::path::Path::new(&root)),
            Err(_) => AppDirs::locate().expect("app dirs"),
        };
        let models = std::env::var("CONCAT_WER_MODELS").unwrap_or("base,small".to_owned());
        let language = std::env::var("CONCAT_WER_LANGUAGE").ok();
        let mut devices = vec![true];
        if std::env::var("CONCAT_WER_CPU").is_ok_and(|value| value == "1") {
            devices.push(false);
        }
        let recipes = [
            ("before", Recipe::BEFORE),
            ("after", Recipe::SHIPPED),
            (
                "after, beam 5",
                Recipe {
                    beam: 5,
                    ..Recipe::SHIPPED
                },
            ),
        ];
        // CONCAT_WER_RECIPE="beam=1,speech=1,detect=0,prune=1" runs that
        // one beside the rest, for telling what each part is worth.
        let mut recipes = recipes.to_vec();
        if let Ok(spec) = std::env::var("CONCAT_WER_RECIPE") {
            let mut recipe = Recipe::SHIPPED;
            for (key, value) in spec.split(',').filter_map(|part| part.split_once('=')) {
                let on = value.trim() == "1";
                match key.trim() {
                    "beam" => recipe.beam = value.trim().parse().expect("a beam width"),
                    "speech" => recipe.speech_only = on,
                    "detect" => recipe.detect_over_speech = on,
                    "prune" => recipe.prune = on,
                    other => panic!("no part of a recipe is called {other}"),
                }
            }
            // CONCAT_WER_ONLY=1 runs it alone.
            if std::env::var("CONCAT_WER_ONLY").is_ok_and(|value| value == "1") {
                recipes.clear();
            }
            recipes.push(("asked", recipe));
        }
        let mut table = Vec::new();
        for model in models.split(',').map(str::trim) {
            for &gpu in &devices {
                let transcriber = Transcriber::new();
                let mut runs: Vec<(&str, Recipe, Option<String>)> = recipes
                    .iter()
                    .map(|(name, recipe)| (*name, Recipe { gpu, ..*recipe }, None))
                    .collect();
                // The language picked rather than heard, as the sheet
                // allows.
                if let Some(code) = &language {
                    runs.push((
                        "after, language picked",
                        Recipe {
                            gpu,
                            ..Recipe::SHIPPED
                        },
                        Some(code.clone()),
                    ));
                }
                for (name, recipe, language) in runs {
                    eprintln!("{model} / {} / {name}", if gpu { "GPU" } else { "CPU" });
                    let request = TranscribeRequest {
                        path: audio.clone(),
                        audio_stream: None,
                        source_start: 0.0,
                        window: 3600.0,
                        model_id: model.to_owned(),
                        language,
                    };
                    // The model's load is timed apart from the work: it is
                    // paid once per session, not per run.
                    let loading = Arc::new(Mutex::new(None::<(Instant, Option<Instant>)>));
                    let seen = Arc::clone(&loading);
                    let began = Instant::now();
                    let lines = transcriber
                        .run(&dirs, &request, recipe, move |stage| {
                            let mut seen = seen.lock().unwrap();
                            match stage {
                                JobStage::Loading => *seen = Some((Instant::now(), None)),
                                JobStage::Working { .. } => {
                                    if let Some((_, end @ None)) = seen.as_mut() {
                                        *end = Some(Instant::now());
                                    }
                                }
                                JobStage::Reading => {}
                            }
                        })
                        .expect("transcribes");
                    let load = loading
                        .lock()
                        .unwrap()
                        .and_then(|(from, to)| to.map(|to| to - from))
                        .unwrap_or_default();
                    let seconds = (began.elapsed() - load).as_secs_f64();
                    let text: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
                    let text = text.join(" ");
                    let rate = wer(&reference, &words(&text));
                    let sound = AudioDecoder::open(
                        &audio,
                        &AudioOptions {
                            start: None,
                            duration: None,
                            filters: Vec::new(),
                            rate: SAMPLE_RATE,
                            channels: 1,
                            format: SampleFormat::F32,
                            stream: None,
                        },
                    )
                    .and_then(|mut decoder| decoder.collect_f32())
                    .map(|samples| samples.len() as f64 / f64::from(SAMPLE_RATE))
                    .unwrap_or(1.0);
                    for line in &lines {
                        eprintln!("  {:7.2} {:7.2}  {}", line.start, line.end, line.text);
                    }
                    table.push(format!(
                        "{model:6} {:4} {name:24} WER {:5.1}%  {:6.1} s  RTF {:.3}  {} lines",
                        if gpu { "GPU" } else { "CPU" },
                        rate * 100.0,
                        seconds,
                        seconds / sound,
                        lines.len()
                    ));
                }
            }
        }
        eprintln!("\n{} reference words", reference.len());
        for row in table {
            eprintln!("{row}");
        }
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
            language: None,
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

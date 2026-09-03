// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

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
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use concat_host::{AppDirs, SingleFlight};
use concat_media::{AudioDecoder, AudioOptions, SampleFormat};
use serde::{Deserialize, Serialize};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::DownloadProgress;

/// One downloadable Whisper model. Sizes are approximate, for progress bars
/// when the server declines to send a Content-Length.
struct KnownModel {
    id: &'static str,
    label: &'static str,
    /// One line for the settings row: what this size trades away.
    blurb: &'static str,
    approx_bytes: u64,
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
        approx_bytes: 77_700_000,
        english_only: true,
    },
    KnownModel {
        id: "tiny",
        label: "Tiny (Multilingual)",
        blurb: "Fastest draft, any language.",
        approx_bytes: 77_700_000,
        english_only: false,
    },
    KnownModel {
        id: "base.en",
        label: "Base (English)",
        blurb: "The sweet spot: solid captions at ~10x realtime.",
        approx_bytes: 147_400_000,
        english_only: true,
    },
    KnownModel {
        id: "base",
        label: "Base (Multilingual)",
        blurb: "Solid captions, any language.",
        approx_bytes: 147_500_000,
        english_only: false,
    },
    KnownModel {
        id: "small.en",
        label: "Small (English)",
        blurb: "Noticeably better wording; a few times slower.",
        approx_bytes: 487_600_000,
        english_only: true,
    },
    KnownModel {
        id: "small",
        label: "Small (Multilingual)",
        blurb: "Best quality offered, any language.",
        approx_bytes: 487_600_000,
        english_only: false,
    },
];

fn known(id: &str) -> Option<&'static KnownModel> {
    KNOWN_MODELS.iter().find(|model| model.id == id)
}

fn model_url(id: &str) -> String {
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{id}.bin")
}

/// Where downloaded models live: `<app data>/whisper-models/ggml-<id>.bin`.
fn models_dir(dirs: &AppDirs) -> Result<PathBuf, String> {
    let dir = dirs.data.join("whisper-models");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    Ok(dir)
}

fn model_file(dirs: &AppDirs, id: &str) -> Result<PathBuf, String> {
    // Ids come from our own table; anything else is a bug, not input.
    known(id).ok_or_else(|| format!("unknown model {id:?}"))?;
    Ok(models_dir(dirs)?.join(format!("ggml-{id}.bin")))
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
    /// Seconds into the file where the clip's source window begins.
    pub source_start: f64,
    /// How much source the clip covers, in seconds (`duration * speed`).
    pub window: f64,
    /// Whisper language code, or "auto".
    pub language: String,
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

    /// Streams one model from Hugging Face into the models folder. Blocks
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
        let (received, total) = crate::download_to(
            &model_url(id),
            &partial,
            id,
            estimate,
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

    /// Transcribes one clip's audio window into caption segments. Blocks
    /// for the whole run: run it on its own thread. `progress` is called
    /// with a 0..100 percentage as whisper works.
    ///
    /// Timestamps come back relative to the window: 0 is `source_start`. The
    /// caller owns the clip, so the map onto the timeline (divide by speed,
    /// add the clip's start) happens there, next to the numbers it needs.
    pub fn transcribe(
        &self,
        dirs: &AppDirs,
        request: &TranscribeRequest,
        progress: impl FnMut(i32) + Send + 'static,
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
        let mut decoder = AudioDecoder::open(
            &request.path,
            &AudioOptions {
                start: Some(request.source_start),
                duration: Some(request.window),
                filters: Vec::new(),
                rate: 16_000,
                channels: 1,
                format: SampleFormat::F32,
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

        let threads = std::thread::available_parallelism()
            .map(|count| count.get().min(8))
            .unwrap_or(4);
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(threads as i32);
        params.set_language(Some(if request.language.is_empty() {
            "auto"
        } else {
            &request.language
        }));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        let abort = Arc::clone(&cancel);
        params.set_abort_callback_safe(move || abort.load(Ordering::Relaxed));
        params.set_progress_callback_safe(progress);

        state
            .full(params, &samples)
            .map_err(|error| format!("transcription failed: {error}"))?;
        if cancel.load(Ordering::Relaxed) {
            return Err("transcription cancelled".to_owned());
        }

        Ok(state
            .as_iter()
            .filter_map(|segment| {
                // Whisper's timestamps are centiseconds.
                let start = segment.start_timestamp() as f64 / 100.0;
                let end = segment.end_timestamp() as f64 / 100.0;
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
    fn every_known_model_has_a_url_and_file_name() {
        for model in KNOWN_MODELS {
            assert!(model_url(model.id).ends_with(&format!("ggml-{}.bin", model.id)));
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

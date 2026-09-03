// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Kokoro text-to-speech: text in, a narration WAV in the project folder out.
//!
//! The same local-first shape as transcription, with one difference:
//! whisper runs as a child process, Kokoro runs in-process through
//! sherpa-onnx (the official k2-fsa bindings, statically linked).
//! In-process because there is no `kokoro-cli` worth shipping, and because
//! sherpa's progress callback gives us cancellation anyway - returning
//! `false` from it stops synthesis mid-sentence.
//!
//! Three pieces:
//!
//! 1. **Models.** Kokoro ships as a tar.bz2 bundle (the network, the voice
//!    bank, espeak-ng data, lexicons), downloaded on demand from the
//!    sherpa-onnx releases into `<app data>/tts-models/<id>/`. Streamed to a
//!    `.part`, unpacked through a staging folder and renamed - a torn
//!    download or unpack must never look usable.
//! 2. **Voices.** Kokoro is one model with many built-in speakers, addressed
//!    by integer id. The table below names the ones we offer - the English
//!    and Chinese speakers, the languages this model's lexicons actually
//!    cover.
//! 3. **Synthesis.** The engine loads once and is cached until the model
//!    changes or is deleted; each request writes a WAV into the project's
//!    `audio/` folder (not `cache/` - cache is regenerable, narration the
//!    user placed on the timeline is not) and returns its path for the
//!    caller to import like any other media file.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use concat_host::{AppDirs, SingleFlight, projects};
use serde::{Deserialize, Serialize};
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig,
    OfflineTtsModelConfig,
};

use crate::DownloadProgress;

/// One downloadable Kokoro bundle. Sizes are approximate, for progress bars
/// when the server declines to send a Content-Length.
struct KnownModel {
    id: &'static str,
    label: &'static str,
    /// One line for the settings row: what this size trades away.
    blurb: &'static str,
    approx_bytes: u64,
}

/// The models the settings panel offers, smallest first.
///
/// Both are Kokoro v1.0 multi-language (English + Chinese) from the official
/// sherpa-onnx conversions; they differ only in quantisation. The int8 build
/// is the default recommendation - a third of the download for a difference
/// most ears never find.
const KNOWN_MODELS: &[KnownModel] = &[
    KnownModel {
        id: "kokoro-int8-multi-lang-v1_0",
        label: "Kokoro (compact)",
        blurb: "The recommended build: same voices, a third of the download.",
        approx_bytes: 131_839_838,
    },
    KnownModel {
        id: "kokoro-multi-lang-v1_0",
        label: "Kokoro (full precision)",
        blurb: "Bit-perfect weights for the skeptical; rarely audibly better.",
        approx_bytes: 349_418_188,
    },
];

/// The speakers we offer, by Kokoro v1.0 speaker id.
///
/// The model bundles 53 voices across nine languages, but its lexicons (and
/// sherpa's text frontend) only genuinely cover English and Chinese, so only
/// those are listed. The name encodes accent and gender - `af` American
/// female, `bm` British male, `zf` Chinese female - which the caller decodes
/// for display rather than us shipping 36 label strings.
const VOICES: &[(i32, &str)] = &[
    (0, "af_alloy"),
    (1, "af_aoede"),
    (2, "af_bella"),
    (3, "af_heart"),
    (4, "af_jessica"),
    (5, "af_kore"),
    (6, "af_nicole"),
    (7, "af_nova"),
    (8, "af_river"),
    (9, "af_sarah"),
    (10, "af_sky"),
    (11, "am_adam"),
    (12, "am_echo"),
    (13, "am_eric"),
    (14, "am_fenrir"),
    (15, "am_liam"),
    (16, "am_michael"),
    (17, "am_onyx"),
    (18, "am_puck"),
    (19, "am_santa"),
    (20, "bf_alice"),
    (21, "bf_emma"),
    (22, "bf_isabella"),
    (23, "bf_lily"),
    (24, "bm_daniel"),
    (25, "bm_fable"),
    (26, "bm_george"),
    (27, "bm_lewis"),
    (45, "zf_xiaobei"),
    (46, "zf_xiaoni"),
    (47, "zf_xiaoxiao"),
    (48, "zf_xiaoyi"),
    (49, "zm_yunjian"),
    (50, "zm_yunxi"),
    (51, "zm_yunxia"),
    (52, "zm_yunyang"),
];

fn known(id: &str) -> Option<&'static KnownModel> {
    KNOWN_MODELS.iter().find(|model| model.id == id)
}

fn model_url(id: &str) -> String {
    format!("https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/{id}.tar.bz2")
}

/// Where downloaded models live: `<app data>/tts-models/<id>/`.
fn models_dir(dirs: &AppDirs) -> Result<PathBuf, String> {
    let dir = dirs.data.join("tts-models");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    Ok(dir)
}

fn model_dir(dirs: &AppDirs, id: &str) -> Result<PathBuf, String> {
    // Ids come from our own table; anything else is a bug, not input.
    known(id).ok_or_else(|| format!("unknown model {id:?}"))?;
    Ok(models_dir(dirs)?.join(id))
}

/// The `.onnx` network inside an unpacked bundle. Located by scanning rather
/// than by name because the archives disagree - `model.onnx` in the full
/// build, `model.int8.onnx` in the quantised one.
fn onnx_file(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        (path.extension().is_some_and(|ext| ext == "onnx")).then_some(path)
    })
}

/// A bundle counts as downloaded once its network is in place. The rename at
/// the end of unpacking makes this atomic: no folder, or a complete one.
fn model_downloaded(dirs: &AppDirs, id: &str) -> bool {
    model_dir(dirs, id).is_ok_and(|dir| onnx_file(&dir).is_some())
}

/// One model, as the settings panel shows it.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    /// The bundle id.
    pub id: String,
    /// Display name.
    pub label: String,
    /// One line on what this size trades away.
    pub blurb: String,
    /// Approximate download size in bytes, for display.
    pub size_bytes: u64,
    /// Whether the bundle is unpacked on disk.
    pub downloaded: bool,
}

/// One speaker.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VoiceInfo {
    /// Kokoro speaker id, what synthesis wants back.
    pub id: i32,
    /// The upstream voice name, e.g. "af_heart"; the caller decodes the
    /// accent/gender prefix and title-cases the rest.
    pub name: String,
}

/// What the settings panel and the speech dialog show.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TtsStatus {
    /// Where models are stored, so the settings panel can say so.
    pub models_dir: String,
    /// Every model the panel offers.
    pub models: Vec<ModelStatus>,
    /// Every speaker on offer.
    pub voices: Vec<VoiceInfo>,
}

/// What to say, and where the file goes.
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SpeakRequest {
    /// Which model to use, e.g. "kokoro-int8-multi-lang-v1_0".
    pub model_id: String,
    /// Kokoro speaker id, from the voices table.
    pub voice: i32,
    /// What to say.
    pub text: String,
    /// Speaking rate; 1.0 is the voice's natural pace.
    pub speed: f32,
    /// The project folder the WAV should land in.
    pub project: String,
}

/// What synthesis produced.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SpeakResult {
    /// Absolute path of the written WAV, ready for the media import path.
    pub path: String,
    /// Seconds of audio, so the caller can say something useful.
    pub duration: f64,
}

struct CachedEngine {
    model_id: String,
    tts: OfflineTts,
}

/// The one synthesis and the one download that can run at a time, plus the
/// loaded engine.
///
/// The engine is cached because loading means parsing a 100-300 MB network;
/// doing that per sentence would make the feature feel broken. The mutex is
/// held for the whole synthesis, which is what lets [`Speech::delete_model`]
/// use `try_lock` to refuse deleting a model mid-use instead of yanking
/// mapped files out from under the session.
pub struct Speech {
    gate: Arc<SingleFlight>,
    downloads: Arc<SingleFlight>,
    engine: Mutex<Option<CachedEngine>>,
}

impl Default for Speech {
    fn default() -> Self {
        Self::new()
    }
}

impl Speech {
    /// An idle speech engine, loading nothing until asked to speak.
    pub fn new() -> Self {
        Self {
            gate: Arc::new(SingleFlight::new()),
            downloads: Arc::new(SingleFlight::new()),
            engine: Mutex::new(None),
        }
    }

    /// Models and voices, for the settings panel and the speech dialog.
    pub fn status(dirs: &AppDirs) -> Result<TtsStatus, String> {
        let dir = models_dir(dirs)?;
        Ok(TtsStatus {
            models_dir: dir.to_string_lossy().into_owned(),
            models: KNOWN_MODELS
                .iter()
                .map(|model| ModelStatus {
                    id: model.id.to_owned(),
                    label: model.label.to_owned(),
                    blurb: model.blurb.to_owned(),
                    size_bytes: model.approx_bytes,
                    downloaded: model_downloaded(dirs, model.id),
                })
                .collect(),
            voices: VOICES
                .iter()
                .map(|(id, name)| VoiceInfo {
                    id: *id,
                    name: (*name).to_owned(),
                })
                .collect(),
        })
    }

    /// Streams one bundle from the sherpa-onnx releases and unpacks it.
    /// Blocks for the whole download: run it on its own thread.
    ///
    /// The archive lands in a `.part`, unpacks into a `.staging-<id>` folder,
    /// and only the final rename puts `<id>/` in place - killed at any
    /// earlier point, nothing is left that could be mistaken for a model.
    pub fn download_model(
        &self,
        dirs: &AppDirs,
        id: &str,
        mut progress: impl FnMut(DownloadProgress),
    ) -> Result<(), String> {
        let job = self.downloads.begin("voice model download")?;
        let cancel = job.cancel_flag();
        let destination = model_dir(dirs, id)?;
        if onnx_file(&destination).is_some() {
            return Ok(());
        }
        let parent = models_dir(dirs)?;
        let estimate = known(id).map(|model| model.approx_bytes).unwrap_or(0);

        let archive = parent.join(format!("{id}.tar.bz2.part"));
        let staging = parent.join(format!(".staging-{id}"));
        let result = (|| {
            let (received, total) = crate::download_to(
                &model_url(id),
                &archive,
                id,
                estimate,
                cancel,
                &mut progress,
            )?;

            // Unpacking a 130 MB bz2 takes long enough to deserve its own
            // phase on the progress bar.
            progress(DownloadProgress {
                id: id.to_owned(),
                received,
                total,
                unpacking: true,
                done: false,
            });

            let _ = std::fs::remove_dir_all(&staging);
            std::fs::create_dir_all(&staging)
                .map_err(|error| format!("could not create {}: {error}", staging.display()))?;

            let file = std::fs::File::open(&archive)
                .map_err(|error| format!("could not reopen the download: {error}"))?;
            let tar = bzip2::read::BzDecoder::new(std::io::BufReader::new(file));
            let mut entries = tar::Archive::new(tar);
            for entry in entries
                .entries()
                .map_err(|error| format!("could not read the archive: {error}"))?
            {
                if cancel.load(Ordering::Relaxed) {
                    return Err("download cancelled".to_owned());
                }
                // `unpack_in` refuses paths that escape the staging folder,
                // so a hostile archive can drop files nowhere else.
                entry
                    .and_then(|mut entry| entry.unpack_in(&staging))
                    .map_err(|error| format!("could not unpack the archive: {error}"))?;
            }

            let unpacked = staging.join(id);
            if onnx_file(&unpacked).is_none() {
                return Err("the archive did not contain the expected model".to_owned());
            }
            // A leftover folder from an older torn unpack would fail the
            // rename; it holds no model (checked above), so it goes.
            let _ = std::fs::remove_dir_all(&destination);
            std::fs::rename(&unpacked, &destination)
                .map_err(|error| format!("could not finish {}: {error}", destination.display()))?;

            progress(DownloadProgress {
                id: id.to_owned(),
                received,
                total,
                unpacking: false,
                done: true,
            });
            Ok(())
        })();

        // The archive and staging folder are dead weight whether the unpack
        // finished or failed; only `<id>/` matters now.
        let _ = std::fs::remove_file(&archive);
        let _ = std::fs::remove_dir_all(&staging);
        result
    }

    /// Asks the running download to stop. Idle is a harmless no-op.
    pub fn cancel_download(&self) {
        self.downloads.cancel();
    }

    /// Removes a downloaded model folder, unless the engine is speaking from it.
    pub fn delete_model(&self, dirs: &AppDirs, id: &str) -> Result<(), String> {
        let dir = model_dir(dirs, id)?;
        let mut engine = self
            .engine
            .try_lock()
            .map_err(|_| "speech is being generated - wait for it or cancel it first".to_owned())?;
        if engine.as_ref().is_some_and(|cached| cached.model_id == id) {
            *engine = None;
        }
        std::fs::remove_dir_all(&dir)
            .map_err(|error| format!("could not remove {}: {error}", dir.display()))
    }

    /// Synthesizes one narration clip into `<project>/audio/`. Blocks for
    /// the whole synthesis: run it on its own thread. `progress` is called
    /// with a 0..1 fraction as sentences complete.
    pub fn speak(
        &self,
        dirs: &AppDirs,
        request: &SpeakRequest,
        progress: impl FnMut(f32) + Send + 'static,
    ) -> Result<SpeakResult, String> {
        let job = self.gate.begin("speech generation")?;
        let cancel = job.cancel_handle();

        let text = request.text.trim();
        if text.is_empty() {
            return Err("nothing to say: the text is empty".to_owned());
        }
        if !VOICES.iter().any(|(id, _)| *id == request.voice) {
            return Err(format!("unknown voice {}", request.voice));
        }
        let speed = if request.speed.is_finite() {
            request.speed.clamp(0.5, 2.0)
        } else {
            1.0
        };

        let dir = model_dir(dirs, &request.model_id)?;
        if onnx_file(&dir).is_none() {
            return Err(format!(
                "model {} is not downloaded - see Settings > Speech",
                request.model_id
            ));
        }

        // The WAV goes in the project so it travels (and dies) with it - and
        // in `audio/`, not `cache/`, because a clip on the timeline points at
        // it: cache is for things that can be regenerated.
        let root = Path::new(&request.project);
        if !projects::is_project(root) {
            return Err(format!("{} is not a project folder", request.project));
        }
        let out_dir = root.join("audio");
        std::fs::create_dir_all(&out_dir)
            .map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;

        let mut engine = self
            .engine
            .lock()
            .map_err(|_| "speech state poisoned".to_owned())?;
        if engine.as_ref().map(|cached| cached.model_id.as_str()) != Some(request.model_id.as_str())
        {
            // Load before overwriting: a failed load keeps the old engine.
            let tts = load_engine(&dir)?;
            *engine = Some(CachedEngine {
                model_id: request.model_id.clone(),
                tts,
            });
        }
        let tts = &engine.as_ref().expect("engine cached above").tts;

        let progress_cancel = Arc::clone(&cancel);
        let mut progress = progress;
        let mut last_fraction = -1.0f32;
        let audio = tts
            .generate_with_config(
                text,
                &GenerationConfig {
                    sid: request.voice,
                    speed,
                    ..Default::default()
                },
                Some(move |_samples: &[f32], fraction: f32| -> bool {
                    if progress_cancel.load(Ordering::Relaxed) {
                        return false;
                    }
                    // The engine reports once per sentence; only meaningful
                    // movement is worth a redraw.
                    if fraction - last_fraction >= 0.01 {
                        last_fraction = fraction;
                        progress(fraction);
                    }
                    true
                }),
            )
            .ok_or_else(|| "speech generation failed".to_owned())?;

        // A cancelled run still returns the samples made so far; the user
        // asked for none of them.
        if cancel.load(Ordering::Relaxed) {
            return Err("speech generation cancelled".to_owned());
        }
        if audio.samples().is_empty() {
            return Err("the engine produced no audio for this text".to_owned());
        }

        // Wall-clock millis plus process id: unique enough for files created
        // by one human clicking a button, and stable for the media bin name.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0);
        let voice_name = VOICES
            .iter()
            .find(|(id, _)| *id == request.voice)
            .map(|(_, name)| *name)
            .unwrap_or("voice");
        let file = out_dir.join(format!("speech-{voice_name}-{stamp}.wav"));

        if !audio.save(&file.to_string_lossy()) {
            return Err(format!("could not write {}", file.display()));
        }

        let duration = audio.samples().len() as f64 / f64::from(audio.sample_rate().max(1));
        Ok(SpeakResult {
            path: file.to_string_lossy().into_owned(),
            duration,
        })
    }

    /// Asks the running synthesis to stop at the next sentence boundary.
    pub fn cancel(&self) {
        self.gate.cancel();
    }

    /// Whether a synthesis is running.
    pub fn is_busy(&self) -> bool {
        self.gate.is_busy()
    }
}

/// Builds the engine for one model folder.
///
/// The lexicon list mirrors the official sherpa-onnx invocation for this
/// bundle: US English and Chinese, joined by commas, each only if present.
fn load_engine(dir: &Path) -> Result<OfflineTts, String> {
    let network = onnx_file(dir)
        .ok_or_else(|| "the model folder has no .onnx network - re-download it".to_owned())?;
    let existing = |name: &str| {
        let path = dir.join(name);
        path.is_file().then(|| path.to_string_lossy().into_owned())
    };
    let lexicon: Vec<String> = ["lexicon-us-en.txt", "lexicon-zh.txt"]
        .iter()
        .filter_map(|name| existing(name))
        .collect();

    let threads = std::thread::available_parallelism()
        .map(|count| count.get().min(8))
        .unwrap_or(4) as i32;

    let config = OfflineTtsConfig {
        model: OfflineTtsModelConfig {
            kokoro: OfflineTtsKokoroModelConfig {
                model: Some(network.to_string_lossy().into_owned()),
                voices: existing("voices.bin"),
                tokens: existing("tokens.txt"),
                data_dir: {
                    let data = dir.join("espeak-ng-data");
                    data.is_dir().then(|| data.to_string_lossy().into_owned())
                },
                dict_dir: {
                    let dict = dir.join("dict");
                    dict.is_dir().then(|| dict.to_string_lossy().into_owned())
                },
                lexicon: (!lexicon.is_empty()).then(|| lexicon.join(",")),
                ..Default::default()
            },
            num_threads: threads,
            ..Default::default()
        },
        ..Default::default()
    };

    OfflineTts::create(&config)
        .ok_or_else(|| "the speech engine failed to load - try re-downloading the model".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_model_has_a_url_and_size() {
        for model in KNOWN_MODELS {
            assert!(model_url(model.id).ends_with(&format!("{}.tar.bz2", model.id)));
            assert!(model.approx_bytes > 0);
        }
    }

    #[test]
    fn voice_ids_are_unique_and_names_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for (id, name) in VOICES {
            assert!(seen.insert(*id), "duplicate voice id {id}");
            // The caller decodes accent and gender from the prefix; a name
            // that breaks the pattern would render as gibberish.
            let (prefix, rest) = name.split_once('_').expect("prefix_name shape");
            assert!(
                matches!(prefix, "af" | "am" | "bf" | "bm" | "zf" | "zm"),
                "{name}"
            );
            assert!(!rest.is_empty());
        }
    }

    #[test]
    fn finds_the_onnx_network_by_extension() {
        let scratch = std::env::temp_dir().join(format!("concat-tts-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch dir");
        assert!(onnx_file(&scratch).is_none());
        std::fs::write(scratch.join("model.int8.onnx"), b"x").expect("writes");
        assert!(onnx_file(&scratch).is_some());
        let _ = std::fs::remove_dir_all(&scratch);
    }
}

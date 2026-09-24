// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Text-to-speech: text in, a narration WAV in the project folder out.
//!
//! The same local-first shape as transcription, with one difference:
//! whisper runs as a child process, the voices run in-process through
//! sherpa-onnx (the official k2-fsa bindings, statically linked).
//! In-process because there is no `kokoro-cli` worth shipping, and because
//! sherpa's progress callback gives us cancellation anyway - returning
//! `false` from it stops synthesis mid-sentence.
//!
//! Three families of model, one front. **Kokoro** is one network with
//! many built-in speakers, addressed by integer id: the English and
//! Chinese ones its lexicons cover are the voices it offers. **Pocket
//! TTS** has no speakers of its own: every voice is a few seconds of
//! someone talking, which it reads in. Its bundle carries two such
//! samples, and the sheet adds a third voice that is the sound of
//! whatever clip is selected - the narrator's own voice, from footage
//! already in the project. **Chatterbox Turbo** reads in a recording the
//! same way, closer to a studio voice, at a gigabyte and a wait; it runs
//! through ONNX Runtime directly ([`crate::chatterbox`]) rather than
//! sherpa, on the desktop only.
//!
//! Three pieces:
//!
//! 1. **Models.** Each ships as a tar.bz2 bundle from the sherpa-onnx
//!    releases, downloaded on demand into `<app data>/tts-models/<id>/`.
//!    Streamed to a `.part`, unpacked through a staging folder and renamed
//!    - a torn download or unpack must never look usable.
//! 2. **Voices.** One table per family, below. A voice id says which
//!    family it belongs to, and the sheet shows the voices of the model it
//!    has chosen.
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
    OfflineTtsModelConfig, OfflineTtsPocketModelConfig,
};

use crate::DownloadProgress;

/// Which network a bundle is, and so which voices it reads with and how
/// the engine is configured for it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    /// Kokoro v1.0: built-in speakers by id.
    #[default]
    Kokoro,
    /// Kyutai's Pocket TTS: a voice is a reference recording.
    Pocket,
    /// Resemble's Chatterbox Turbo: a voice is a reference recording,
    /// and the engine is ONNX Runtime rather than sherpa.
    Chatterbox,
}

/// The family a model id belongs to, by the bundle's name.
pub fn family_of(id: &str) -> Family {
    if id.starts_with("sherpa-onnx-pocket-tts") {
        Family::Pocket
    } else if id.starts_with("chatterbox") {
        Family::Chatterbox
    } else {
        Family::Kokoro
    }
}

/// One downloadable bundle. Sizes are approximate, for progress bars when
/// the server declines to send a Content-Length.
struct KnownModel {
    id: &'static str,
    label: &'static str,
    /// One line for the settings row: what this size trades away.
    blurb: &'static str,
    approx_bytes: u64,
    /// What the finished archive must hash to. Empty until the mirror has
    /// been filled once and reported what it holds; see
    /// [`concat_host::models`].
    sha256: &'static str,
}

/// The models the settings panel offers, smallest first.
///
/// The Kokoro pair are v1.0 multi-language (English + Chinese) from the
/// official sherpa-onnx conversions; they differ only in quantisation. The
/// int8 build is the default recommendation - a third of the download for
/// a difference most ears never find. Pocket TTS is the sherpa-onnx int8
/// conversion of Kyutai's 100M model: English, and it reads in any voice
/// it is given a few seconds of.
const KNOWN_MODELS: &[KnownModel] = &[
    #[cfg(feature = "chatterbox")]
    KnownModel {
        id: crate::chatterbox::BUNDLE_ID,
        label: "Chatterbox Turbo (studio)",
        blurb: "The closest to a studio voice; reads in Pocket's recordings or any sample of your own. English, slow, a gigabyte.",
        approx_bytes: crate::chatterbox::BUNDLE_BYTES,
        // Nine files, each checked against its own digest in
        // `chatterbox::FILES`; this row never reaches `verify`.
        sha256: "",
    },
    KnownModel {
        id: POCKET_ID,
        label: "Pocket TTS (cloning)",
        blurb: "Reads in any voice, including one from a clip on the timeline. English.",
        approx_bytes: 98_336_520,
        sha256: "2f3b88823cbbb9bf0b2477ec8ae7b3fec417b3a87b6bb5f256dba66f2ad967cb",
    },
    KnownModel {
        id: "kokoro-int8-multi-lang-v1_0",
        label: "Kokoro (compact)",
        blurb: "The recommended build: same voices, a third of the download.",
        approx_bytes: 132_303_094,
        sha256: "4c3052abaa60943a341f193888cf6abd68787dae6ab8ae5c925a706caa247e4e",
    },
    KnownModel {
        id: "kokoro-multi-lang-v1_0",
        label: "Kokoro (full precision)",
        blurb: "Bit-perfect weights for the skeptical; rarely audibly better.",
        approx_bytes: 349_906_910,
        sha256: "c5f7e2d2caf082bc1d20fb70334a61d99d20b484500aad32e7cf84c128ea3298",
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

/// The voices Pocket TTS offers, by our own id, out of Kokoro's range.
///
/// Two are recordings the bundle carries, named here by the file beside
/// the network; the third is the clone: no file of its own, since the
/// recording is the clip the sheet was given, decoded at request time.
pub const POCKET_VOICES: &[(i32, &str, Option<&str>)] = &[
    (1000, "pocket_bria", Some("test_wavs/bria.wav")),
    (1001, "pocket_loona", Some("test_wavs/loona.wav")),
    (POCKET_CLONE, "pocket_clone", None),
];

/// The voice that is a recording of the caller's choosing - a clip on the
/// timeline, a file in the bin - handed over as the request's reference.
pub const POCKET_CLONE: i32 = 1099;

/// The Pocket bundle, which is also where the stock recordings live.
pub const POCKET_ID: &str = "sherpa-onnx-pocket-tts-int8-2026-01-26";

/// Chatterbox's voices. The model has no speakers of its own and ships no
/// recording of anyone's under a licence this build may carry, so its
/// stock voice is Pocket's Bria recording, read from the Pocket bundle:
/// offered only while that bundle is on disk, and named as Chatterbox's
/// so a voice id still says which family reads it. Pocket's other
/// recording, Loona, is a second long, and Chatterbox needs five to take
/// a voice from - see `chatterbox::MIN_REFERENCE_SECONDS` - so it is not
/// offered here. The other voice is a recording of the caller's choosing,
/// as Pocket's is.
pub const CHATTERBOX_VOICES: &[(i32, &str, Option<&str>)] = &[
    (2000, "chatterbox_bria", Some("test_wavs/bria.wav")),
    (CHATTERBOX_CLONE, "chatterbox_clone", None),
];

/// A recording of the caller's choosing, read by Chatterbox.
pub const CHATTERBOX_CLONE: i32 = 2099;

/// Whether a voice is a recording of the caller's choosing, in any family.
pub fn is_clone(voice: i32) -> bool {
    voice == POCKET_CLONE || voice == CHATTERBOX_CLONE
}

/// Pocket TTS reads the voice from this many seconds of a recording at
/// most; more costs time and adds nothing.
pub const REFERENCE_SECONDS: f64 = 10.0;

/// Which family a voice id belongs to.
pub fn voice_family(voice: i32) -> Option<Family> {
    if VOICES.iter().any(|(id, _)| *id == voice) {
        Some(Family::Kokoro)
    } else if POCKET_VOICES.iter().any(|(id, _, _)| *id == voice) {
        Some(Family::Pocket)
    } else if CHATTERBOX_VOICES.iter().any(|(id, _, _)| *id == voice) {
        Some(Family::Chatterbox)
    } else {
        None
    }
}

/// The upstream name of a voice, in either family.
fn voice_name(voice: i32) -> Option<&'static str> {
    VOICES
        .iter()
        .find(|(id, _)| *id == voice)
        .map(|(_, name)| *name)
        .or_else(|| {
            POCKET_VOICES
                .iter()
                .find(|(id, _, _)| *id == voice)
                .map(|(_, name, _)| *name)
        })
        .or_else(|| {
            CHATTERBOX_VOICES
                .iter()
                .find(|(id, _, _)| *id == voice)
                .map(|(_, name, _)| *name)
        })
}

fn known(id: &str) -> Option<&'static KnownModel> {
    KNOWN_MODELS.iter().find(|model| model.id == id)
}

/// The archive a bundle arrives as, and what it is called on the mirror.
fn model_archive(id: &str) -> String {
    format!("{id}.tar.bz2")
}

/// Where the mirror was filled from, and the second place a download tries.
fn model_upstream(id: &str) -> String {
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
/// Chatterbox arrives as files rather than an archive, and counts once
/// every one of them is there.
fn model_downloaded(dirs: &AppDirs, id: &str) -> bool {
    let Ok(dir) = model_dir(dirs, id) else {
        return false;
    };
    match family_of(id) {
        #[cfg(feature = "chatterbox")]
        Family::Chatterbox => crate::chatterbox::installed(&dir),
        _ => onnx_file(&dir).is_some(),
    }
}

/// One model, as the settings panel shows it.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    /// The bundle id.
    pub id: String,
    /// Which family, and so which voices it reads with.
    pub family: Family,
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
    /// The voice id, what synthesis wants back: Kokoro's speaker id, or
    /// one of [`POCKET_VOICES`].
    pub id: i32,
    /// The upstream voice name, e.g. "af_heart"; the caller decodes the
    /// accent/gender prefix and title-cases the rest. Pocket voices are
    /// `pocket_<name>`.
    pub name: String,
    /// The family whose models read with this voice.
    pub family: Family,
}

/// A recording to read in the voice of: the clip the sheet was given.
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    /// The media file.
    pub path: String,
    /// Seconds into the file the voice is heard from.
    #[serde(default)]
    pub start: f64,
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
    /// The voice, from the voices table; it must belong to the model's
    /// family.
    pub voice: i32,
    /// For [`POCKET_CLONE`]: the recording to read in the voice of.
    #[serde(default)]
    pub reference: Option<Reference>,
    /// What to say.
    pub text: String,
    /// Speaking rate; 1.0 is the voice's natural pace. Kokoro and Pocket
    /// honour it; Chatterbox reads at its own.
    pub speed: f32,
    /// The breath between sentences, as sherpa scales it: 0 runs them
    /// together, 1 is a long pause, 0.2 is the engine's own. Absent is
    /// the engine's own. Kokoro and Pocket.
    #[serde(default)]
    pub pauses: Option<f32>,
    /// Pocket's flow-matching steps: more is a cleaner voice and a longer
    /// wait, 5 is the engine's own. Absent is the engine's own; the other
    /// families have no such dial and ignore it.
    #[serde(default)]
    pub steps: Option<i32>,
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

/// A loaded model: sherpa's engine for the families it runs, or
/// Chatterbox's own.
enum Engine {
    Sherpa(OfflineTts),
    #[cfg(feature = "chatterbox")]
    Chatterbox(Box<crate::chatterbox::Engine>),
}

struct CachedEngine {
    model_id: String,
    /// Loaded for the accelerator, or for the CPU; a change of mind loads
    /// the model again.
    accelerated: bool,
    tts: Engine,
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
                    family: family_of(model.id),
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
                    family: Family::Kokoro,
                })
                .chain(POCKET_VOICES.iter().map(|(id, name, _)| VoiceInfo {
                    id: *id,
                    name: (*name).to_owned(),
                    family: Family::Pocket,
                }))
                // Chatterbox's stock voices are Pocket's recordings, and are
                // only offered while the bundle holding them is here.
                .chain(
                    CHATTERBOX_VOICES
                        .iter()
                        .filter(|(_, _, file)| file.is_none() || model_downloaded(dirs, POCKET_ID))
                        .map(|(id, name, _)| VoiceInfo {
                            id: *id,
                            name: (*name).to_owned(),
                            family: Family::Chatterbox,
                        }),
                )
                .collect(),
        })
    }

    /// Streams one Kokoro bundle from Concat's mirror and unpacks it.
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
        #[cfg(feature = "chatterbox")]
        if family_of(id) == Family::Chatterbox {
            return crate::chatterbox::download(&destination, cancel, &mut progress);
        }
        if onnx_file(&destination).is_some() {
            return Ok(());
        }
        let parent = models_dir(dirs)?;
        let estimate = known(id).map(|model| model.approx_bytes).unwrap_or(0);

        let archive = parent.join(format!("{id}.tar.bz2.part"));
        let staging = parent.join(format!(".staging-{id}"));
        let result = (|| {
            let (received, total) = crate::fetch_model(
                &model_archive(id),
                &model_upstream(id),
                &archive,
                id,
                estimate,
                known(id).map(|model| model.sha256).unwrap_or(""),
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
        let started = std::time::Instant::now();

        let text = request.text.trim();
        if text.is_empty() {
            return Err("nothing to say: the text is empty".to_owned());
        }
        let family = family_of(&request.model_id);
        log::info!(
            "tts: {} reading {} characters with voice {} ({}), speed {:.2}, pauses {:?}, steps {:?}{}",
            request.model_id,
            text.chars().count(),
            request.voice,
            voice_name(request.voice).unwrap_or("?"),
            request.speed,
            request.pauses,
            request.steps,
            request
                .reference
                .as_ref()
                .map(|reference| format!(
                    ", voice from {} at {:.1}s",
                    reference.path, reference.start
                ))
                .unwrap_or_default()
        );
        match voice_family(request.voice) {
            Some(owner) if owner == family => {}
            Some(_) => {
                return Err(format!(
                    "voice {} does not belong to {}",
                    request.voice, request.model_id
                ));
            }
            None => return Err(format!("unknown voice {}", request.voice)),
        }
        let speed = if request.speed.is_finite() {
            request.speed.clamp(0.5, 2.0)
        } else {
            1.0
        };
        let defaults = GenerationConfig::default();
        let silence_scale = request
            .pauses
            .filter(|pauses| pauses.is_finite())
            .map_or(defaults.silence_scale, |pauses| pauses.clamp(0.0, 1.0));
        let num_steps = request
            .steps
            .map_or(defaults.num_steps, |steps| steps.clamp(1, 16));

        let dir = model_dir(dirs, &request.model_id)?;
        if !model_downloaded(dirs, &request.model_id) {
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
        let accelerated = crate::accelerated();
        let stale = engine.as_ref().is_none_or(|cached| {
            cached.model_id != request.model_id || cached.accelerated != accelerated
        });
        if stale {
            // Load before overwriting: a failed load keeps the old engine.
            let loading = std::time::Instant::now();
            log::info!(
                "tts: loading {} from {} for the {}",
                request.model_id,
                dir.display(),
                if accelerated { "accelerator" } else { "CPU" }
            );
            let tts = match family {
                Family::Kokoro => Engine::Sherpa(load_kokoro(&dir, accelerated)?),
                Family::Pocket => Engine::Sherpa(load_pocket(&dir, accelerated)?),
                #[cfg(feature = "chatterbox")]
                Family::Chatterbox => Engine::Chatterbox(Box::new(
                    crate::chatterbox::Engine::load(&dir, accelerated)?,
                )),
                #[cfg(not(feature = "chatterbox"))]
                Family::Chatterbox => {
                    return Err("Chatterbox is not part of this build".to_owned());
                }
            };
            log::info!(
                "tts: {} loaded in {:.1}s",
                request.model_id,
                loading.elapsed().as_secs_f32()
            );
            *engine = Some(CachedEngine {
                model_id: request.model_id.clone(),
                accelerated,
                tts,
            });
        }
        let tts = &engine.as_ref().expect("engine cached above").tts;

        // Wall-clock millis plus process id: unique enough for files created
        // by one human clicking a button, and stable for the media bin name.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0);
        let voice_name = voice_name(request.voice).unwrap_or("voice");
        let file = out_dir.join(format!("speech-{voice_name}-{stamp}.wav"));

        // Chatterbox is its own loop: the recording in, the samples out,
        // and the file written here. Sherpa's families go on below.
        // A named voice's recording lives in the Pocket bundle, whichever
        // model reads it; a chosen recording needs no bundle at all.
        let recordings = if is_clone(request.voice) {
            dir.clone()
        } else {
            recordings_dir(dirs)?
        };

        #[cfg(feature = "chatterbox")]
        let tts = match tts {
            Engine::Chatterbox(chatterbox) => {
                let samples = reference_samples(&recordings, request)?;
                let mut progress = progress;
                let spoken = chatterbox.speak(text, &samples, &cancel, &mut |fraction| {
                    progress(fraction);
                })?;
                if spoken.is_empty() {
                    return Err("the engine produced no audio for this text".to_owned());
                }
                let rate = crate::chatterbox::SAMPLE_RATE;
                std::fs::write(&file, crate::chatterbox::wav_bytes(&spoken, rate))
                    .map_err(|error| format!("could not write {}: {error}", file.display()))?;
                let duration = spoken.len() as f64 / f64::from(rate);
                log::info!(
                    "tts: wrote {} ({duration:.2}s) in {:.1}s",
                    file.display(),
                    started.elapsed().as_secs_f32()
                );
                return Ok(SpeakResult {
                    path: file.to_string_lossy().into_owned(),
                    duration,
                });
            }
            Engine::Sherpa(tts) => tts,
        };
        #[cfg(not(feature = "chatterbox"))]
        let Engine::Sherpa(tts) = tts;

        // Kokoro picks its speaker by id; Pocket reads the voice out of a
        // recording, decoded here to what the model listens at.
        let generation = match family {
            Family::Kokoro => GenerationConfig {
                sid: request.voice,
                speed,
                silence_scale,
                ..Default::default()
            },
            _ => {
                let samples = reference_samples(&recordings, request)?;
                GenerationConfig {
                    speed,
                    silence_scale,
                    num_steps,
                    reference_audio: Some(samples),
                    reference_sample_rate: REFERENCE_RATE as i32,
                    extra: Some(
                        [(
                            "max_reference_audio_len".to_owned(),
                            serde_json::json!(REFERENCE_SECONDS),
                        )]
                        .into_iter()
                        .collect(),
                    ),
                    ..Default::default()
                }
            }
        };

        let progress_cancel = Arc::clone(&cancel);
        let mut progress = progress;
        let mut last_fraction = -1.0f32;
        let audio = tts
            .generate_with_config(
                text,
                &generation,
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

        if !audio.save(&file.to_string_lossy()) {
            return Err(format!("could not write {}", file.display()));
        }

        let duration = audio.samples().len() as f64 / f64::from(audio.sample_rate().max(1));
        log::info!(
            "tts: wrote {} ({duration:.2}s at {} Hz) in {:.1}s",
            file.display(),
            audio.sample_rate(),
            started.elapsed().as_secs_f32()
        );
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

/// What Pocket TTS listens to a voice at: the rate of its own codec, and
/// the rate its bundled recordings are in.
const REFERENCE_RATE: u32 = 24_000;

/// Where the stock recordings are: the Pocket bundle. An error naming the
/// fix when it is not on disk, since a Chatterbox voice borrowed from it
/// can be asked for without Pocket ever having been downloaded.
fn recordings_dir(dirs: &AppDirs) -> Result<PathBuf, String> {
    if !model_downloaded(dirs, POCKET_ID) {
        return Err(
            "the stock voices are Pocket TTS's recordings - download Pocket TTS in Settings > \
             Speech, or read in a sample voice of your own"
                .to_owned(),
        );
    }
    model_dir(dirs, POCKET_ID)
}

/// The recording a request reads the voice from, as mono samples at
/// [`REFERENCE_RATE`]: one of the stock recordings in `dir` for a named
/// voice, or the request's reference for a chosen one, cut to
/// [`REFERENCE_SECONDS`] from where the voice is heard.
fn reference_samples(dir: &Path, request: &SpeakRequest) -> Result<Vec<f32>, String> {
    let (path, start) = if is_clone(request.voice) {
        let reference = request.reference.as_ref().ok_or_else(|| {
            "no recording to take the voice from - pick a sample with a voice in it".to_owned()
        })?;
        (
            PathBuf::from(&reference.path),
            if reference.start.is_finite() {
                reference.start.max(0.0)
            } else {
                0.0
            },
        )
    } else {
        let file = POCKET_VOICES
            .iter()
            .chain(CHATTERBOX_VOICES.iter())
            .find(|(id, _, _)| *id == request.voice)
            .and_then(|(_, _, file)| *file)
            .ok_or_else(|| format!("voice {} has no recording", request.voice))?;
        (dir.join(file), 0.0)
    };
    let options = concat_media::AudioOptions {
        start: (start > 0.0).then_some(start),
        duration: Some(REFERENCE_SECONDS),
        rate: REFERENCE_RATE,
        channels: 1,
        format: concat_media::SampleFormat::F32,
        ..Default::default()
    };
    let mut decoder = concat_media::AudioDecoder::open(&path, &options)
        .map_err(|error| format!("could not read the voice from {}: {error}", path.display()))?;
    let samples = decoder
        .collect_f32()
        .map_err(|error| format!("could not read the voice from {}: {error}", path.display()))?;
    // Half a second is the least a voice can be told from; under that the
    // model reads in nobody's.
    if samples.len() < REFERENCE_RATE as usize / 2 {
        return Err(format!(
            "too little sound in {} to take a voice from",
            path.display()
        ));
    }
    let peak = samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    log::info!(
        "tts: voice from {} at {start:.1}s: {:.1}s of sound, peak {peak:.2}",
        path.display(),
        samples.len() as f32 / REFERENCE_RATE as f32
    );
    if peak < 0.01 {
        log::warn!("tts: the voice recording is near silent - the read will be in nobody's voice");
    }
    Ok(samples)
}

/// The file in `dir` whose name starts with `prefix` and ends `.onnx`:
/// `lm_main.onnx` in a full bundle, `lm_main.int8.onnx` in a quantised one.
fn onnx_starting(dir: &Path, prefix: &str) -> Option<String> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "onnx")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(prefix))
        })
        .collect();
    found.sort();
    found
        .first()
        .map(|path| path.to_string_lossy().into_owned())
}

/// Builds the engine for a Pocket TTS bundle: the five networks and the
/// two tables the sherpa-onnx example names.
fn load_pocket(dir: &Path, accelerated: bool) -> Result<OfflineTts, String> {
    let part = |prefix: &str| {
        onnx_starting(dir, prefix)
            .ok_or_else(|| format!("the model folder has no {prefix} network - re-download it"))
    };
    let table = |name: &str| {
        let path = dir.join(name);
        path.is_file()
            .then(|| path.to_string_lossy().into_owned())
            .ok_or_else(|| format!("the model folder has no {name} - re-download it"))
    };
    let config = OfflineTtsConfig {
        model: OfflineTtsModelConfig {
            pocket: OfflineTtsPocketModelConfig {
                lm_flow: Some(part("lm_flow")?),
                lm_main: Some(part("lm_main")?),
                encoder: Some(part("encoder")?),
                decoder: Some(part("decoder")?),
                text_conditioner: Some(part("text_conditioner")?),
                vocab_json: Some(table("vocab.json")?),
                token_scores_json: Some(table("token_scores.json")?),
                // The voices offered plus a few clones, before the oldest
                // embedding is worked out again.
                voice_embedding_cache_capacity: 8,
            },
            num_threads: threads(),
            provider: Some(provider(accelerated).to_owned()),
            ..Default::default()
        },
        ..Default::default()
    };
    OfflineTts::create(&config)
        .ok_or_else(|| "the speech engine failed to load - try re-downloading the model".to_owned())
}

/// The ONNX Runtime provider sherpa is asked for: CoreML on a Mac that
/// wants the accelerator, the CPU otherwise. sherpa falls back to the CPU
/// itself, with a line in the log, where its runtime was built without
/// the one asked for.
fn provider(accelerated: bool) -> &'static str {
    if accelerated && cfg!(target_os = "macos") {
        "coreml"
    } else {
        "cpu"
    }
}

/// The threads the engine may use: every core up to eight.
fn threads() -> i32 {
    std::thread::available_parallelism()
        .map(|count| count.get().min(8))
        .unwrap_or(4) as i32
}

/// Builds the engine for a Kokoro bundle.
///
/// The lexicon list mirrors the official sherpa-onnx invocation for this
/// bundle: US English and Chinese, joined by commas, each only if present.
fn load_kokoro(dir: &Path, accelerated: bool) -> Result<OfflineTts, String> {
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
            num_threads: threads(),
            provider: Some(provider(accelerated).to_owned()),
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
    fn a_voice_belongs_to_one_family_and_a_model_says_which_it_reads_with() {
        assert_eq!(family_of("kokoro-int8-multi-lang-v1_0"), Family::Kokoro);
        assert_eq!(family_of("kokoro-multi-lang-v1_0"), Family::Kokoro);
        assert_eq!(
            family_of("sherpa-onnx-pocket-tts-int8-2026-01-26"),
            Family::Pocket
        );
        assert_eq!(voice_family(3), Some(Family::Kokoro));
        assert_eq!(voice_family(52), Some(Family::Kokoro));
        assert_eq!(voice_family(1000), Some(Family::Pocket));
        assert_eq!(voice_family(POCKET_CLONE), Some(Family::Pocket));
        assert_eq!(voice_family(CHATTERBOX_CLONE), Some(Family::Chatterbox));
        assert_eq!(family_of("chatterbox-turbo-q8"), Family::Chatterbox);
        assert!(is_clone(POCKET_CLONE) && is_clone(CHATTERBOX_CLONE) && !is_clone(3));
        assert_eq!(voice_family(53), None);
        assert_eq!(voice_family(-1), None);
        // No id serves two families, and every voice has a name.
        for (id, _) in VOICES {
            assert!(!POCKET_VOICES.iter().any(|(other, _, _)| other == id));
            assert!(voice_name(*id).is_some());
        }
        for (id, name, _) in POCKET_VOICES {
            assert_eq!(voice_name(*id), Some(*name));
            assert!(!CHATTERBOX_VOICES.iter().any(|(other, _, _)| other == id));
        }
        for (id, name, _) in CHATTERBOX_VOICES {
            assert_eq!(voice_name(*id), Some(*name));
            assert_eq!(voice_family(*id), Some(Family::Chatterbox));
        }
        // The stock Chatterbox voice is Pocket's long recording by another
        // name; Pocket's one-second one is not offered to it.
        assert_eq!(CHATTERBOX_VOICES[0].2, POCKET_VOICES[0].2);
        assert!(
            !CHATTERBOX_VOICES
                .iter()
                .any(|(_, _, file)| *file == POCKET_VOICES[1].2)
        );
        assert_eq!(voice_name(999), None);
        for model in KNOWN_MODELS {
            assert!(known(model.id).is_some());
        }
    }

    #[test]
    fn the_clone_voice_needs_a_recording_and_a_named_voice_has_one() {
        let dir = std::env::temp_dir().join(format!("concat-pocket-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let request = |voice: i32, reference: Option<Reference>| SpeakRequest {
            model_id: "sherpa-onnx-pocket-tts-int8-2026-01-26".to_owned(),
            voice,
            reference,
            text: "hello".to_owned(),
            speed: 1.0,
            pauses: None,
            steps: None,
            project: String::new(),
        };
        let error = reference_samples(&dir, &request(POCKET_CLONE, None)).expect_err("no clip");
        assert!(error.contains("pick a sample"), "{error}");
        // A recording that is not there names itself.
        let error = reference_samples(&dir, &request(1000, None)).expect_err("no file");
        assert!(error.contains("bria.wav"), "{error}");
        let error = reference_samples(
            &dir,
            &request(
                POCKET_CLONE,
                Some(Reference {
                    path: dir.join("nobody.mp4").to_string_lossy().into_owned(),
                    start: f64::NAN,
                }),
            ),
        )
        .expect_err("no file");
        assert!(error.contains("nobody.mp4"), "{error}");
        // A voice with no recording in the table is refused, not read.
        let error = reference_samples(&dir, &request(1098, None)).expect_err("no such voice");
        assert!(error.contains("1098"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bundles_networks_are_found_by_their_prefix_whatever_the_quantisation() {
        let dir = std::env::temp_dir().join(format!("concat-pocket-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        for name in [
            "lm_main.int8.onnx",
            "lm_flow.onnx",
            "decoder.int8.onnx",
            "text_conditioner.onnx",
            "vocab.json",
        ] {
            std::fs::write(dir.join(name), b"x").expect("write");
        }
        let found = |prefix: &str| onnx_starting(&dir, prefix).unwrap_or_default();
        assert!(found("lm_main").ends_with("lm_main.int8.onnx"));
        assert!(found("lm_flow").ends_with("lm_flow.onnx"));
        assert!(found("decoder").ends_with("decoder.int8.onnx"));
        assert!(found("text_conditioner").ends_with("text_conditioner.onnx"));
        assert_eq!(
            onnx_starting(&dir, "encoder"),
            None,
            "decoder is not encoder"
        );
        assert_eq!(
            onnx_starting(&dir, "vocab"),
            None,
            "a table is not a network"
        );
        assert_eq!(onnx_starting(&dir.join("nowhere"), "lm_main"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_known_model_is_asked_of_the_mirror_before_upstream() {
        for model in KNOWN_MODELS {
            let file = model_archive(model.id);
            assert_eq!(file, format!("{}.tar.bz2", model.id));
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

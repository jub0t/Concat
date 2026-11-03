// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Text-to-speech: text in, a narration WAV in the project folder out.
//!
//! Two families of voice, one front. **Chatterbox Turbo**, on the desktop,
//! reads in the voice of a recording - a stock one Concat carries, or the
//! sound of whatever clip is selected, the narrator's own voice from
//! footage already in the project - closer to a studio voice, at a
//! gigabyte and a wait. It runs through ONNX Runtime ([`crate::chatterbox`]).
//! **The phone's voice**, on Android, is the phone's own text-to-speech
//! engine - Google's, or whichever the phone has - with nothing to
//! download: the platform crate hands it over through [`set_system_voice`].
//!
//! Three pieces:
//!
//! 1. **Models.** Chatterbox's files are downloaded on demand into
//!    `<app data>/tts-models/<id>/`, streamed to a `.part` and renamed, so
//!    a torn download never looks usable. The phone's voice is the phone's.
//! 2. **Voices.** One table per family, below. A voice id says which
//!    family it belongs to, and the sheet shows the voices of the model it
//!    has chosen.
//! 3. **Synthesis.** Chatterbox loads once and is cached until the model
//!    changes or is deleted; each request writes a WAV into the project's
//!    `audio/` folder (not `cache/` - cache is regenerable, narration the
//!    user placed on the timeline is not) and returns its path for the
//!    caller to import like any other media file.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};

use concat_host::{AppDirs, SingleFlight, projects};
use serde::{Deserialize, Serialize};

use crate::{DownloadProgress, JobStage};

/// Which voice engine a model is, and so which voices it reads with.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    /// Resemble's Chatterbox Turbo: a voice is a reference recording.
    #[default]
    Chatterbox,
    /// The phone's own text-to-speech engine, in the phone's voice.
    System,
}

/// The phone's voice, as a model id: there is nothing to download, and the
/// settings show it as a model like any other.
pub const SYSTEM_ID: &str = "system-voice";

/// The family a model id belongs to.
pub fn family_of(id: &str) -> Family {
    if id == SYSTEM_ID {
        Family::System
    } else {
        Family::Chatterbox
    }
}

/// The phone's text-to-speech engine, as the platform crate hands it over:
/// says `text` into a WAV at the path, at a rate where 1 is the voice's
/// own, and returns once the file is written. Android's, set at start.
pub type SystemVoice = dyn Fn(&str, &Path, f32) -> Result<(), String> + Send + Sync;

static SYSTEM_VOICE: OnceLock<Box<SystemVoice>> = OnceLock::new();

/// Hands the phone's voice over. The first call wins; later ones are
/// ignored.
pub fn set_system_voice(voice: Box<SystemVoice>) {
    let _ = SYSTEM_VOICE.set(voice);
}

/// Whether this build has a phone's voice to offer.
fn system_voice() -> Option<&'static SystemVoice> {
    SYSTEM_VOICE.get().map(Box::as_ref)
}

/// The most the phone's engine reads in one go: Android's
/// `getMaxSpeechInputLength` is 4000 characters, and a little under keeps
/// clear of how it counts. Longer text is read in pieces and joined.
const SYSTEM_PIECE: usize = 3_900;

/// One model the settings offer. Sizes are approximate, for progress bars
/// when the server declines to send a Content-Length.
struct KnownModel {
    id: &'static str,
    label: &'static str,
    /// One line for the settings row: what it is for.
    blurb: &'static str,
    approx_bytes: u64,
}

/// The models the settings panel offers: the ones this build has.
const KNOWN_MODELS: &[KnownModel] = &[
    #[cfg(feature = "chatterbox")]
    KnownModel {
        id: crate::chatterbox::BUNDLE_ID,
        label: "Chatterbox Turbo (studio)",
        blurb: "The closest to a studio voice; reads in a stock voice or any sample of your own. English, slow, a gigabyte.",
        approx_bytes: crate::chatterbox::BUNDLE_BYTES,
    },
    KnownModel {
        id: SYSTEM_ID,
        label: "Phone voice",
        blurb: "Your phone's own voice: instant, in its language, with nothing to download.",
        approx_bytes: 0,
    },
];

/// Chatterbox's voices. The model has no speakers of its own: its stock
/// voices are recordings Concat carries (`voices/`, all LibriVox readings
/// their readers dedicated to the public domain), and the last is a
/// recording of the caller's choosing.
///
/// A name's prefix is what the voice list says under it, as
/// `speech::voice_label` reads it: the accent (`a` American, `b` British)
/// and the gender (`f`, `m`). `x` is no accent, for a reader whose accent
/// no source states. 2000 was Bria until November 2026, and stays the
/// default.
pub const CHATTERBOX_VOICES: &[(i32, &str, Option<&str>)] = &[
    (2000, "af_elizabeth", Some("elizabeth-klett.flac")),
    (2001, "am_mark", Some("mark-f-smith.flac")),
    (2002, "bf_ruth", Some("ruth-golding.flac")),
    (2003, "xm_bob", Some("bob-neufeld.flac")),
    (CHATTERBOX_CLONE, "chatterbox_clone", None),
];

/// A recording of the caller's choosing, read by Chatterbox.
pub const CHATTERBOX_CLONE: i32 = 2099;

/// The phone's voice: whichever its engine is set to.
pub const SYSTEM_VOICES: &[(i32, &str)] = &[(3000, "system_default")];

/// The stock recordings Concat carries, by file name.
#[cfg(feature = "chatterbox")]
const RECORDINGS: &[(&str, &[u8])] = &[
    (
        "elizabeth-klett.flac",
        include_bytes!("../voices/elizabeth-klett.flac"),
    ),
    (
        "mark-f-smith.flac",
        include_bytes!("../voices/mark-f-smith.flac"),
    ),
    (
        "ruth-golding.flac",
        include_bytes!("../voices/ruth-golding.flac"),
    ),
    (
        "bob-neufeld.flac",
        include_bytes!("../voices/bob-neufeld.flac"),
    ),
];

/// Whether a voice is a recording of the caller's choosing.
pub fn is_clone(voice: i32) -> bool {
    voice == CHATTERBOX_CLONE
}

/// Chatterbox reads the voice from this many seconds of a recording at
/// most; more costs time and adds nothing.
pub const REFERENCE_SECONDS: f64 = 10.0;

/// Which family a voice id belongs to.
pub fn voice_family(voice: i32) -> Option<Family> {
    if CHATTERBOX_VOICES.iter().any(|(id, _, _)| *id == voice) {
        Some(Family::Chatterbox)
    } else if SYSTEM_VOICES.iter().any(|(id, _)| *id == voice) {
        Some(Family::System)
    } else {
        None
    }
}

/// The name of a voice, in either family.
fn voice_name(voice: i32) -> Option<&'static str> {
    CHATTERBOX_VOICES
        .iter()
        .find(|(id, _, _)| *id == voice)
        .map(|(_, name, _)| *name)
        .or_else(|| {
            SYSTEM_VOICES
                .iter()
                .find(|(id, _)| *id == voice)
                .map(|(_, name)| *name)
        })
}

fn known(id: &str) -> Option<&'static KnownModel> {
    KNOWN_MODELS.iter().find(|model| model.id == id)
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

/// Whether a model is ready to read with: every one of Chatterbox's files
/// in place, or a phone with a voice to offer.
fn model_downloaded(dirs: &AppDirs, id: &str) -> bool {
    match family_of(id) {
        Family::System => system_voice().is_some(),
        #[cfg(feature = "chatterbox")]
        Family::Chatterbox => {
            model_dir(dirs, id).is_ok_and(|dir| crate::chatterbox::installed(&dir))
        }
        #[cfg(not(feature = "chatterbox"))]
        Family::Chatterbox => {
            let _ = dirs;
            false
        }
    }
}

/// Whether this build offers a model at all: the phone's voice only where
/// a phone handed one over.
fn offered(id: &str) -> bool {
    family_of(id) != Family::System || system_voice().is_some()
}

/// One model, as the settings panel shows it.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    /// The model id.
    pub id: String,
    /// Which family, and so which voices it reads with.
    pub family: Family,
    /// Display name.
    pub label: String,
    /// One line on what it is for.
    pub blurb: String,
    /// Approximate download size in bytes, for display; 0 for the phone's
    /// voice, which downloads nothing.
    pub size_bytes: u64,
    /// Whether it is ready to read with.
    pub downloaded: bool,
}

/// One speaker.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VoiceInfo {
    /// The voice id, what synthesis wants back: one of
    /// [`CHATTERBOX_VOICES`] or [`SYSTEM_VOICES`].
    pub id: i32,
    /// The voice's name, `chatterbox_<name>` or `system_default`.
    pub name: String,
    /// The family whose models read with this voice.
    pub family: Family,
    /// A picture of the voice, as a path to an image file, for the voice
    /// picker's avatar: none of the stock voices has one, and the picker
    /// draws a letter in a colour instead. For a voice someone adds later
    /// with a picture of their own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
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
    /// Which model to use, e.g. "chatterbox-turbo-q8".
    pub model_id: String,
    /// The voice, from the voices table; it must belong to the model's
    /// family.
    pub voice: i32,
    /// For [`CHATTERBOX_CLONE`]: the recording to read in the voice of.
    #[serde(default)]
    pub reference: Option<Reference>,
    /// What to say.
    pub text: String,
    /// Speaking rate; 1.0 is the voice's natural pace. The phone's voice
    /// honours it; Chatterbox reads at its own.
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

/// A loaded Chatterbox.
#[cfg(feature = "chatterbox")]
struct CachedEngine {
    model_id: String,
    tts: Box<crate::chatterbox::Engine>,
    /// Whether it was loaded for the accelerator rather than the CPU.
    accelerated: bool,
}

/// Why a read failed, as far as the accelerator is concerned: the runtime
/// failed running one of the networks, which the accelerator may be to
/// blame for and the CPU may well run - DirectML refuses some of
/// Chatterbox's quantised attention (#291) - or something else, which the
/// CPU would stop at too: the text, the recording, a cancel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The runtime failed running a network.
    Runtime(String),
    /// Anything else.
    Other(String),
}

impl From<String> for ReadError {
    fn from(message: String) -> Self {
        ReadError::Other(message)
    }
}

impl From<&str> for ReadError {
    fn from(message: &str) -> Self {
        ReadError::Other(message.to_owned())
    }
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Runtime(message) | ReadError::Other(message) => f.write_str(message),
        }
    }
}

/// The accelerator the runtime is asked for here, by the name its log
/// lines use.
#[cfg_attr(not(feature = "chatterbox"), allow(dead_code))]
fn accelerator() -> &'static str {
    if cfg!(target_os = "macos") {
        "CoreML"
    } else {
        "the accelerator"
    }
}

/// `read` on `engine`, and when an engine loaded for the accelerator fails
/// it in the runtime, `read` once more on the engine `on_cpu` loads for the
/// CPU. A model that loads for the accelerator can still be refused there a
/// network at a time, mid-read, and loading alone never found that out
/// (#291); slower is better than no voice. Any other failure is handed
/// back as it is, and nothing is loaded.
///
/// The CPU engine comes back beside the result, whether or not the second
/// read went through, for the caller to keep in place of the one that
/// failed: an accelerator that refused a network once will refuse it again.
#[cfg_attr(not(feature = "chatterbox"), allow(dead_code))]
fn read_or_on_cpu<E, T>(
    engine: &E,
    accelerated: bool,
    model: &str,
    mut read: impl FnMut(&E) -> Result<T, ReadError>,
    on_cpu: impl FnOnce() -> Result<E, String>,
) -> (Result<T, String>, Option<E>) {
    match read(engine) {
        Ok(done) => (Ok(done), None),
        Err(ReadError::Runtime(error)) if accelerated => {
            log::warn!(
                "tts: {model} failed on {} ({error}); loading it for the CPU and reading again",
                accelerator()
            );
            match on_cpu() {
                Ok(cpu) => {
                    let again = read(&cpu).map_err(|error| error.to_string());
                    (again, Some(cpu))
                }
                Err(load) => (
                    Err(format!(
                        "{error}; and it would not load for the CPU either: {load}"
                    )),
                    None,
                ),
            }
        }
        Err(error) => (Err(error.to_string()), None),
    }
}

/// The one synthesis and the one download that can run at a time, plus the
/// loaded engine.
///
/// The engine is cached because loading means parsing a gigabyte of
/// networks; doing that per read would make the feature feel broken. The
/// mutex is held for the whole synthesis, which is what lets
/// [`Speech::delete_model`] use `try_lock` to refuse deleting a model
/// mid-use instead of yanking mapped files out from under the session.
pub struct Speech {
    gate: Arc<SingleFlight>,
    downloads: Arc<SingleFlight>,
    #[cfg(feature = "chatterbox")]
    engine: Mutex<Option<CachedEngine>>,
    #[cfg(not(feature = "chatterbox"))]
    engine: Mutex<Option<String>>,
    /// The models the accelerator failed to run this session: loaded for
    /// the CPU from then on, rather than tried there and failed again each
    /// time one is loaded.
    #[cfg_attr(not(feature = "chatterbox"), allow(dead_code))]
    on_cpu: Mutex<std::collections::HashSet<String>>,
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
            on_cpu: Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// Models and voices, for the settings panel and the speech dialog.
    pub fn status(dirs: &AppDirs) -> Result<TtsStatus, String> {
        let dir = models_dir(dirs)?;
        Ok(TtsStatus {
            models_dir: dir.to_string_lossy().into_owned(),
            models: KNOWN_MODELS
                .iter()
                .filter(|model| offered(model.id))
                .map(|model| ModelStatus {
                    id: model.id.to_owned(),
                    family: family_of(model.id),
                    label: model.label.to_owned(),
                    blurb: model.blurb.to_owned(),
                    size_bytes: model.approx_bytes,
                    downloaded: model_downloaded(dirs, model.id),
                })
                .collect(),
            voices: CHATTERBOX_VOICES
                .iter()
                .filter(|_| {
                    KNOWN_MODELS
                        .iter()
                        .any(|model| family_of(model.id) == Family::Chatterbox)
                })
                .map(|(id, name, _)| VoiceInfo {
                    id: *id,
                    name: (*name).to_owned(),
                    family: Family::Chatterbox,
                    avatar: None,
                })
                .chain(
                    SYSTEM_VOICES
                        .iter()
                        .filter(|_| offered(SYSTEM_ID))
                        .map(|(id, name)| VoiceInfo {
                            id: *id,
                            name: (*name).to_owned(),
                            family: Family::System,
                            avatar: None,
                        }),
                )
                .collect(),
        })
    }

    /// Downloads model `id` from Concat's mirror. Blocks for the whole
    /// download: run it on its own thread. The phone's voice has nothing
    /// to download.
    pub fn download_model(
        &self,
        dirs: &AppDirs,
        id: &str,
        mut progress: impl FnMut(DownloadProgress),
    ) -> Result<(), String> {
        let job = self.downloads.begin("voice model download")?;
        let cancel = job.cancel_flag();
        let destination = model_dir(dirs, id)?;
        match family_of(id) {
            Family::System => Ok(()),
            #[cfg(feature = "chatterbox")]
            Family::Chatterbox => crate::chatterbox::download(&destination, cancel, &mut progress),
            #[cfg(not(feature = "chatterbox"))]
            Family::Chatterbox => {
                let _ = (destination, cancel, &mut progress);
                Err("Chatterbox is not part of this build".to_owned())
            }
        }
    }

    /// What model `id` is called as a file, and every address a download
    /// of it tries, in order; see [`crate::Transcriber::model_sources`].
    /// `None` for every voice model: Chatterbox is several files rather
    /// than one, and the phone's voice is no file at all.
    pub fn model_sources(id: &str) -> Option<(String, Vec<String>)> {
        let _ = known(id)?;
        None
    }

    /// Asks the running download to stop. Idle is a harmless no-op.
    pub fn cancel_download(&self) {
        self.downloads.cancel();
    }

    /// Removes a downloaded model folder, unless the engine is speaking from
    /// it. The phone's voice is the phone's, and is not Concat's to remove.
    pub fn delete_model(&self, dirs: &AppDirs, id: &str) -> Result<(), String> {
        if family_of(id) == Family::System {
            return Err("the phone's voice is part of the phone".to_owned());
        }
        let dir = model_dir(dirs, id)?;
        let mut engine = self
            .engine
            .try_lock()
            .map_err(|_| "speech is being generated - wait for it or cancel it first".to_owned())?;
        #[cfg(feature = "chatterbox")]
        if engine.as_ref().is_some_and(|cached| cached.model_id == id) {
            *engine = None;
        }
        #[cfg(not(feature = "chatterbox"))]
        let _ = &mut engine;
        std::fs::remove_dir_all(&dir)
            .map_err(|error| format!("could not remove {}: {error}", dir.display()))
    }

    /// The model still loaded from the last read, which the next read
    /// with it does not load again. None while a read holds it.
    pub fn loaded_model(&self) -> Option<String> {
        let engine = self.engine.try_lock().ok()?;
        #[cfg(feature = "chatterbox")]
        return engine.as_ref().map(|cached| cached.model_id.clone());
        #[cfg(not(feature = "chatterbox"))]
        engine.clone()
    }

    /// Synthesizes one narration clip into `<project>/audio/`. Blocks for
    /// the whole synthesis: run it on its own thread. `progress` is told
    /// each stage as it begins, and in the reading itself how far it is:
    /// Chatterbox's as it writes, the phone's as each piece is done.
    pub fn speak(
        &self,
        dirs: &AppDirs,
        request: &SpeakRequest,
        mut progress: impl FnMut(JobStage) + Send + 'static,
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
            "tts: {} reading {} characters with voice {} ({}), speed {:.2}{}",
            request.model_id,
            text.chars().count(),
            request.voice,
            voice_name(request.voice).unwrap_or("?"),
            request.speed,
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

        // Wall-clock millis plus process id: unique enough for files created
        // by one human clicking a button, and stable for the media bin name.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0);
        let voice_name = voice_name(request.voice).unwrap_or("voice");
        let file = out_dir.join(format!("speech-{voice_name}-{stamp}.wav"));

        if family == Family::System {
            let voice = system_voice().ok_or("this device has no voice of its own")?;
            let duration = speak_on_the_phone(voice, text, speed, &file, &cancel, &mut progress)?;
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

        #[cfg(not(feature = "chatterbox"))]
        {
            let _ = (dir, speed, cancel, &mut progress);
            Err("Chatterbox is not part of this build".to_owned())
        }
        #[cfg(feature = "chatterbox")]
        {
            let _ = speed;
            let mut engine = self
                .engine
                .lock()
                .map_err(|_| "speech state poisoned".to_owned())?;
            let stale = engine
                .as_ref()
                .is_none_or(|cached| cached.model_id != request.model_id);
            if stale {
                // Load before overwriting: a failed load keeps the old engine.
                progress(JobStage::Loading);
                let loading = std::time::Instant::now();
                let load = |accelerated: bool| -> Result<Box<crate::chatterbox::Engine>, String> {
                    log::info!(
                        "tts: loading {} from {} for the {}",
                        request.model_id,
                        dir.display(),
                        if accelerated { "accelerator" } else { "CPU" }
                    );
                    Ok(Box::new(crate::chatterbox::Engine::load(
                        &dir,
                        accelerated,
                    )?))
                };
                // The accelerator where there is one, and the CPU when a
                // model will not load for it: slower is better than not at
                // all, and nobody should have to find a setting to get the
                // voice back. A model the accelerator failed to run this
                // session goes straight to the CPU.
                let refused = self
                    .on_cpu
                    .lock()
                    .is_ok_and(|models| models.contains(&request.model_id));
                let (tts, accelerated) = match load(crate::accelerated() && !refused) {
                    Ok(tts) => (tts, crate::accelerated() && !refused),
                    Err(error) if crate::accelerated() && !refused => {
                        log::warn!(
                            "tts: {} would not load on the accelerator ({error}); loading it for the CPU",
                            request.model_id
                        );
                        (load(false)?, false)
                    }
                    Err(error) => return Err(error),
                };
                log::info!(
                    "tts: {} loaded in {:.1}s",
                    request.model_id,
                    loading.elapsed().as_secs_f32()
                );
                *engine = Some(CachedEngine {
                    model_id: request.model_id.clone(),
                    tts,
                    accelerated,
                });
            }
            let cached = engine.as_ref().expect("engine cached above");
            let (chatterbox, accelerated) = (&cached.tts, cached.accelerated);

            // A named voice's recording is one Concat carries; a chosen
            // recording is the request's own.
            let recordings = if is_clone(request.voice) {
                dir.clone()
            } else {
                recordings_dir(dirs)?
            };
            progress(JobStage::Reading);
            let samples = reference_samples(&recordings, request)?;
            progress(JobStage::Working {
                done: 0.0,
                next: CHATTERBOX_STEP,
            });
            let (spoken, on_cpu) = read_or_on_cpu(
                chatterbox.as_ref(),
                accelerated,
                &request.model_id,
                |chatterbox| {
                    chatterbox.speak(text, &samples, &cancel, &mut |done| {
                        progress(JobStage::Working {
                            done,
                            next: (done + CHATTERBOX_STEP).min(1.0),
                        });
                    })
                },
                || crate::chatterbox::Engine::load(&dir, false),
            );
            // The CPU engine stays for the session, and the model is
            // loaded for the CPU from now on.
            if let Some(cpu) = on_cpu {
                if let Ok(mut models) = self.on_cpu.lock() {
                    models.insert(request.model_id.clone());
                }
                *engine = Some(CachedEngine {
                    model_id: request.model_id.clone(),
                    tts: Box::new(cpu),
                    accelerated: false,
                });
            }
            let spoken = spoken?;
            if spoken.is_empty() {
                return Err("the engine produced no audio for this text".to_owned());
            }
            let rate = crate::chatterbox::SAMPLE_RATE;
            std::fs::write(&file, wav_bytes(&spoken, rate))
                .map_err(|error| format!("could not write {}: {error}", file.display()))?;
            let duration = spoken.len() as f64 / f64::from(rate);
            log::info!(
                "tts: wrote {} ({duration:.2}s) in {:.1}s",
                file.display(),
                started.elapsed().as_secs_f32()
            );
            Ok(SpeakResult {
                path: file.to_string_lossy().into_owned(),
                duration,
            })
        }
    }

    /// Asks the running synthesis to stop.
    pub fn cancel(&self) {
        self.gate.cancel();
    }

    /// Whether a synthesis is running.
    pub fn is_busy(&self) -> bool {
        self.gate.is_busy()
    }
}

/// What Chatterbox listens to a voice at, and the rate of the recordings
/// Concat carries.
#[cfg_attr(not(feature = "chatterbox"), allow(dead_code))]
const REFERENCE_RATE: u32 = 24_000;

/// Where the stock recordings are: written out of the app beside the
/// models the first time one is asked for, so the decoder reads them as it
/// reads any file.
#[cfg(feature = "chatterbox")]
fn recordings_dir(dirs: &AppDirs) -> Result<PathBuf, String> {
    let dir = models_dir(dirs)?.join("voices");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    for (name, bytes) in RECORDINGS {
        let path = dir.join(name);
        if std::fs::metadata(&path).is_ok_and(|meta| meta.len() == bytes.len() as u64) {
            continue;
        }
        std::fs::write(&path, bytes)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    }
    Ok(dir)
}

/// How far ahead of Chatterbox's last report its next is taken to be: it
/// reports every few tokens, so a short way.
#[cfg(feature = "chatterbox")]
const CHATTERBOX_STEP: f32 = 0.02;

/// `text` in pieces of at most `limit` characters, each ending at a
/// sentence where one fits and at a space where none does, so the phone's
/// engine is never handed more than it reads at once.
fn pieces(text: &str, limit: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text.trim();
    while rest.chars().count() > limit {
        let cut = rest
            .char_indices()
            .nth(limit)
            .map_or(rest.len(), |(index, _)| index);
        let window = &rest[..cut];
        let end = window
            .rfind(['.', '!', '?', '。', '！', '？', '\n'])
            .map(|index| index + window[index..].chars().next().map_or(1, char::len_utf8))
            .or_else(|| window.rfind(' '))
            .filter(|end| *end > 0)
            .unwrap_or(cut);
        out.push(rest[..end].trim());
        rest = rest[end..].trim_start();
    }
    if !rest.is_empty() {
        out.push(rest);
    }
    out.retain(|piece| !piece.is_empty());
    out
}

/// Reads `text` on the phone's engine into `file`, a piece at a time, and
/// joins the pieces: each is written to a file of its own, decoded at the
/// rate of the first, and the samples written as one WAV. Returns the
/// seconds written.
fn speak_on_the_phone(
    voice: &SystemVoice,
    text: &str,
    speed: f32,
    file: &Path,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(JobStage),
) -> Result<f64, String> {
    let pieces = pieces(text, SYSTEM_PIECE);
    let count = pieces.len().max(1) as f32;
    let mut samples: Vec<f32> = Vec::new();
    let mut rate = 0u32;
    for (index, piece) in pieces.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("speech generation cancelled".to_owned());
        }
        progress(JobStage::Working {
            done: index as f32 / count,
            next: (index + 1) as f32 / count,
        });
        let part = file.with_extension(format!("part{index}.wav"));
        let read = voice(piece, &part, speed).and_then(|()| {
            if rate == 0 {
                rate = concat_media::probe(&part)
                    .ok()
                    .and_then(|info| info.audio.map(|audio| audio.sample_rate))
                    .filter(|rate| *rate > 0)
                    .unwrap_or(24_000);
            }
            let options = concat_media::AudioOptions {
                rate,
                channels: 1,
                format: concat_media::SampleFormat::F32,
                ..Default::default()
            };
            concat_media::AudioDecoder::open(&part, &options)
                .and_then(|mut decoder| decoder.collect_f32())
                .map_err(|error| format!("could not read the phone's voice back: {error}"))
        });
        let _ = std::fs::remove_file(&part);
        samples.extend(read?);
    }
    if samples.is_empty() {
        return Err("the phone's voice produced no audio for this text".to_owned());
    }
    std::fs::write(file, wav_bytes(&samples, rate))
        .map_err(|error| format!("could not write {}: {error}", file.display()))?;
    Ok(samples.len() as f64 / f64::from(rate))
}

/// Mono samples as a 16-bit PCM WAV.
pub(crate) fn wav_bytes(samples: &[f32], rate: u32) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// The recording a request reads the voice from, as mono samples at
/// [`REFERENCE_RATE`]: one of the stock recordings in `dir` for a named
/// voice, or the request's reference for a chosen one, cut to
/// [`REFERENCE_SECONDS`] from where the voice is heard.
#[cfg_attr(not(feature = "chatterbox"), allow(dead_code))]
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
        let file = CHATTERBOX_VOICES
            .iter()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Text longer than the phone reads at once is cut at sentences where
    /// one fits and at spaces where none does, never mid-word, and nothing
    /// is lost but the space between pieces.
    #[test]
    fn long_text_is_read_in_pieces_at_sentences() {
        assert_eq!(pieces("Hello there.", 100), ["Hello there."]);
        assert_eq!(
            pieces("One two. Three four. Five six.", 20),
            ["One two. Three four.", "Five six."]
        );
        assert_eq!(
            pieces("alpha beta gamma delta", 11),
            ["alpha beta", "gamma delta"]
        );
        assert_eq!(
            pieces("你好。再见！你好。", 4),
            ["你好。", "再见！", "你好。"]
        );
        assert_eq!(pieces("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert!(pieces("   ", 10).is_empty());
        let long = "Word. ".repeat(2_000);
        let cut = pieces(&long, SYSTEM_PIECE);
        assert!(
            cut.iter()
                .all(|piece| piece.chars().count() <= SYSTEM_PIECE)
        );
        assert_eq!(cut.concat().matches("Word.").count(), 2_000);
    }

    /// A read the accelerator's runtime fails is read again once on the
    /// CPU, and the CPU engine comes back to be kept: DirectML refused
    /// Chatterbox's attention mid-read, after it had loaded (#291).
    #[test]
    fn a_read_the_accelerator_fails_is_read_again_on_the_cpu() {
        let read = |engine: &&str| match *engine {
            "gpu" => Err(ReadError::Runtime(
                "MultiHeadAttention: E_INVALIDARG".to_owned(),
            )),
            _ => Ok(format!("read on the {engine}")),
        };
        let (result, kept) = read_or_on_cpu(&"gpu", true, "chatterbox", read, || Ok("cpu"));
        assert_eq!(result.as_deref(), Ok("read on the cpu"));
        assert_eq!(kept, Some("cpu"));

        // Read once on the CPU, however it goes.
        let mut reads = 0;
        let (result, kept) = read_or_on_cpu(
            &"gpu",
            true,
            "chatterbox",
            |_: &&str| {
                reads += 1;
                Err::<(), _>(ReadError::Runtime("still refused".to_owned()))
            },
            || Ok("cpu"),
        );
        assert_eq!(reads, 2);
        assert_eq!(result, Err("still refused".to_owned()));
        assert_eq!(kept, Some("cpu"), "the CPU engine is kept all the same");

        // A model that will not load for the CPU says both.
        let (result, kept) = read_or_on_cpu(&"gpu", true, "chatterbox", read, || {
            Err("no memory".to_owned())
        });
        let error = result.expect_err("nothing read");
        assert!(
            error.contains("E_INVALIDARG") && error.contains("no memory"),
            "{error}"
        );
        assert!(kept.is_none());
    }

    /// Nothing is loaded again for a failure the CPU would meet too, nor
    /// for an engine on the CPU already.
    #[test]
    fn only_the_accelerators_failures_are_read_again() {
        let never = || -> Result<&str, String> { panic!("loaded for the CPU") };
        let (result, kept) = read_or_on_cpu(
            &"gpu",
            true,
            "chatterbox",
            |_: &&str| Err::<(), _>(ReadError::from("speech generation cancelled")),
            never,
        );
        assert_eq!(result, Err("speech generation cancelled".to_owned()));
        assert!(kept.is_none());
        let (result, _) = read_or_on_cpu(
            &"cpu",
            false,
            "chatterbox",
            |_: &&str| Err::<(), _>(ReadError::Runtime("refused".to_owned())),
            never,
        );
        assert_eq!(result, Err("refused".to_owned()));
    }

    #[test]
    fn a_voice_belongs_to_one_family_and_a_model_says_which_it_reads_with() {
        assert_eq!(family_of("chatterbox-turbo-q8"), Family::Chatterbox);
        assert_eq!(family_of(SYSTEM_ID), Family::System);
        assert_eq!(voice_family(2000), Some(Family::Chatterbox));
        assert_eq!(voice_family(CHATTERBOX_CLONE), Some(Family::Chatterbox));
        assert_eq!(voice_family(3000), Some(Family::System));
        assert!(is_clone(CHATTERBOX_CLONE) && !is_clone(2000) && !is_clone(3000));
        assert_eq!(voice_family(3), None);
        assert_eq!(voice_family(-1), None);
        for (id, name, _) in CHATTERBOX_VOICES {
            assert_eq!(voice_name(*id), Some(*name));
            assert!(!SYSTEM_VOICES.iter().any(|(other, _)| other == id));
        }
        for (id, name) in SYSTEM_VOICES {
            assert_eq!(voice_name(*id), Some(*name));
        }
        assert_eq!(voice_name(999), None);
        for model in KNOWN_MODELS {
            assert!(known(model.id).is_some());
        }
    }

    /// The phone's voice is offered only where a phone handed one over,
    /// and is never Concat's to download or remove.
    #[test]
    fn the_phones_voice_is_offered_only_with_a_phone() {
        assert!(
            !offered(SYSTEM_ID),
            "no phone has handed a voice over in a test"
        );
        assert!(offered("chatterbox-turbo-q8"));
        let speech = Speech::new();
        let dirs = AppDirs {
            config: std::env::temp_dir().join(format!("concat-tts-system-{}", std::process::id())),
            data: std::env::temp_dir().join(format!("concat-tts-system-{}", std::process::id())),
        };
        assert!(speech.delete_model(&dirs, SYSTEM_ID).is_err());
        assert_eq!(Speech::model_sources(SYSTEM_ID), None);
        let _ = std::fs::remove_dir_all(&dirs.data);
    }

    #[test]
    fn the_clone_voice_needs_a_recording_and_a_named_voice_has_one() {
        let dir = std::env::temp_dir().join(format!("concat-voices-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let request = |voice: i32, reference: Option<Reference>| SpeakRequest {
            model_id: "chatterbox-turbo-q8".to_owned(),
            voice,
            reference,
            text: "hello".to_owned(),
            speed: 1.0,
            project: String::new(),
        };
        let error = reference_samples(&dir, &request(CHATTERBOX_CLONE, None)).expect_err("no clip");
        assert!(error.contains("pick a sample"), "{error}");
        // A recording that is not there names itself.
        let error = reference_samples(&dir, &request(2000, None)).expect_err("no file");
        assert!(error.contains("elizabeth-klett.flac"), "{error}");
        let error = reference_samples(
            &dir,
            &request(
                CHATTERBOX_CLONE,
                Some(Reference {
                    path: dir.join("nobody.mp4").to_string_lossy().into_owned(),
                    start: f64::NAN,
                }),
            ),
        )
        .expect_err("no file");
        assert!(error.contains("nobody.mp4"), "{error}");
        // A voice with no recording in the table is refused, not read.
        let error = reference_samples(&dir, &request(2098, None)).expect_err("no such voice");
        assert!(error.contains("2098"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every stock voice Concat carries is written out beside the models and
    /// reads as ten seconds of someone talking.
    #[cfg(feature = "chatterbox")]
    #[test]
    fn the_stock_voices_are_carried_and_read() {
        let dirs = AppDirs {
            config: std::env::temp_dir().join(format!("concat-tts-stock-{}", std::process::id())),
            data: std::env::temp_dir().join(format!("concat-tts-stock-{}", std::process::id())),
        };
        let dir = recordings_dir(&dirs).expect("written");
        let stock: Vec<(i32, &str)> = CHATTERBOX_VOICES
            .iter()
            .filter_map(|(id, _, file)| file.map(|file| (*id, file)))
            .collect();
        assert!(stock.len() >= 4, "{stock:?}");
        for (voice, file) in stock {
            assert!(
                RECORDINGS.iter().any(|(name, _)| *name == file),
                "{file} is carried"
            );
            let request = SpeakRequest {
                model_id: "chatterbox-turbo-q8".to_owned(),
                voice,
                reference: None,
                text: "hello".to_owned(),
                speed: 1.0,
                project: String::new(),
            };
            let samples = reference_samples(&dir, &request).expect("reads");
            let seconds = samples.len() as f64 / f64::from(REFERENCE_RATE);
            assert!(
                (seconds - REFERENCE_SECONDS).abs() < 0.1,
                "{file}: {seconds}"
            );
            assert!(
                samples.iter().any(|sample| sample.abs() > 0.1),
                "someone is talking in {file}"
            );
        }
        let _ = std::fs::remove_dir_all(&dirs.data);
    }

    #[test]
    fn a_wav_says_its_rate_and_length() {
        let wav = wav_bytes(&[0.0, 0.5, -0.5], 22_050);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 22_050);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 6);
        assert_eq!(wav.len(), 50);
    }
}

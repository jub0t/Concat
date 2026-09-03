// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Whisper.cpp transcription: audio in, timed caption segments out.
//!
//! The same shape as every other media operation in this app: spawn a child
//! process, feed it a file, read a file back, kill it to cancel. Whisper gets
//! that treatment for the reasons FFmpeg does (see
//! `engine/docs/decisions/0002`): no linkage, no unsafe, and cancellation is
//! `Child::kill`.
//!
//! Three pieces:
//!
//! 1. **The binary.** `whisper-cli` ships inside the app, staged at build
//!    like FFmpeg; the release guard refuses to build without it. The search
//!    below still honours a remembered override first and falls back to the
//!    app data folder, beside the executable, then PATH - for dev builds and
//!    for anyone who insists on their own copy.
//! 2. **Models.** Downloaded on demand from Hugging Face into the app data
//!    folder as plain `.bin` files, streamed with progress events, written to
//!    a `.part` and renamed - a torn download must never look usable.
//! 3. **Transcription.** FFmpeg (the bundled one) extracts the clip's window
//!    as 16 kHz mono WAV - the input whisper wants - then `whisper-cli`
//!    writes JSON, which is parsed into segments relative to the window.
//!
//! This lives in the host rather than `concat-media` for now: the engine has
//! no caption concept, and model downloads certainly do not belong there. If
//! the engine grows an audio-analysis seam, the runner half graduates.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::jobs::SingleFlight;

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
fn models_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("could not locate the data directory: {error}"))?
        .join("whisper-models");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    Ok(dir)
}

fn model_file(app: &tauri::AppHandle, id: &str) -> Result<PathBuf, String> {
    // Ids come from our own table; anything else is a bug, not input.
    known(id).ok_or_else(|| format!("unknown model {id:?}"))?;
    Ok(models_dir(app)?.join(format!("ggml-{id}.bin")))
}

/// The remembered settings: today just where `whisper-cli` is.
fn config_file(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("could not locate the config directory: {error}"))?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    Ok(dir.join("transcriber.json"))
}

fn remembered_binary(app: &tauri::AppHandle) -> Option<PathBuf> {
    let file = config_file(app).ok()?;
    let text = std::fs::read_to_string(file).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let path = PathBuf::from(value.get("binary")?.as_str()?);
    path.is_file().then_some(path)
}

/// Finds `whisper-cli`, checking the remembered choice first, then the app
/// data folder, beside the executable, and finally PATH. `main` is accepted
/// too - the name the tool shipped under before it was renamed.
fn find_binary(app: &tauri::AppHandle) -> Option<PathBuf> {
    if let Some(path) = remembered_binary(app) {
        return Some(path);
    }

    let names: &[&str] = if cfg!(windows) {
        &["whisper-cli.exe", "whisper-main.exe", "main.exe"]
    } else {
        &["whisper-cli", "whisper-main"]
    };

    let mut directories: Vec<PathBuf> = Vec::new();
    if let Ok(data) = app.path().app_data_dir() {
        directories.push(data.join("whisper"));
    }
    if let Ok(resources) = app.path().resource_dir() {
        directories.push(resources.join("whisper"));
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            directories.push(parent.to_path_buf());
        }
    }

    for directory in &directories {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // PATH last, and only the canonical name: a bare `main` on PATH could be
    // absolutely anything.
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(if cfg!(windows) { "whisper-cli.exe" } else { "whisper-cli" });
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    id: String,
    label: String,
    blurb: String,
    english_only: bool,
    /// Approximate download size in bytes, for display.
    size_bytes: u64,
    downloaded: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriberStatus {
    /// Where `whisper-cli` was found, or null when it was not.
    binary: Option<String>,
    /// True when the binary in use is the copy shipped inside the app.
    bundled: bool,
    /// Where models are stored, so the settings panel can say so.
    models_dir: String,
    models: Vec<ModelStatus>,
}

/// What the settings panel shows: binary location and per-model state.
#[tauri::command]
pub fn transcriber_status(app: tauri::AppHandle) -> Result<TranscriberStatus, String> {
    let dir = models_dir(&app)?;
    let binary = find_binary(&app);
    let bundled = match (&binary, app.path().resource_dir()) {
        (Some(path), Ok(resources)) => path.starts_with(&resources),
        _ => false,
    };
    Ok(TranscriberStatus {
        binary: binary.map(|path| path.to_string_lossy().into_owned()),
        bundled,
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

/// Remembers a user-chosen `whisper-cli` after checking it exists.
#[tauri::command]
pub fn set_transcriber_binary(app: tauri::AppHandle, path: String) -> Result<TranscriberStatus, String> {
    if !Path::new(&path).is_file() {
        return Err(format!("{path} is not a file"));
    }
    let file = config_file(&app)?;
    let document = serde_json::json!({ "binary": path });
    std::fs::write(&file, serde_json::to_string_pretty(&document).unwrap_or_default())
        .map_err(|error| format!("could not write {}: {error}", file.display()))?;
    transcriber_status(app)
}

/// Progress for one model download, emitted as `transcriber://download`.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    id: String,
    received: u64,
    /// Content-Length when the server sent one, the table estimate otherwise.
    total: u64,
    done: bool,
}

/// The slot for the one model download that can run at a time. Enforced by
/// `SingleFlight`: a second download is refused rather than sharing (and
/// resetting) the first one's cancel flag.
pub struct DownloadState(pub Arc<SingleFlight>);

/// Streams one model from Hugging Face into the models folder.
///
/// Written to `<file>.part` and renamed at the end: a download killed halfway
/// must never be mistaken for a model, because whisper would fail on it with
/// a message that blames the model rather than the download.
#[tauri::command]
pub async fn download_transcriber_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, DownloadState>,
    id: String,
) -> Result<(), String> {
    let job = state.0.begin("model download")?;

    tauri::async_runtime::spawn_blocking(move || {
        let cancel = job.cancel_flag();
        let destination = model_file(&app, &id)?;
        if destination.is_file() {
            return Ok(());
        }
        let estimate = known(&id).map(|model| model.approx_bytes).unwrap_or(0);

        // A read timeout, or a stalled connection blocks in `read` forever
        // with the cancel flag unreachable - the flag is only checked between
        // reads. Thirty seconds without a byte means the download is dead.
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(20))
            .timeout_read(std::time::Duration::from_secs(30))
            .build();
        let response = agent
            .get(&model_url(&id))
            .call()
            .map_err(|error| format!("download failed: {error}"))?;
        let total = response
            .header("Content-Length")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(estimate);

        let partial = destination.with_extension("bin.part");
        let mut file = std::fs::File::create(&partial)
            .map_err(|error| format!("could not create {}: {error}", partial.display()))?;

        let mut reader = response.into_reader();
        let mut buffer = [0u8; 64 * 1024];
        let mut received: u64 = 0;
        let mut last_report: u64 = 0;

        loop {
            if cancel.load(Ordering::Relaxed) {
                drop(file);
                let _ = std::fs::remove_file(&partial);
                return Err("download cancelled".to_owned());
            }
            let read = reader
                .read(&mut buffer)
                .map_err(|error| format!("download interrupted: {error}"))?;
            if read == 0 {
                break;
            }
            std::io::Write::write_all(&mut file, &buffer[..read])
                .map_err(|error| format!("could not write model: {error}"))?;
            received += read as u64;

            // Every 2 MB, not every chunk: the event crosses the IPC bridge.
            if received - last_report >= 2 * 1024 * 1024 {
                last_report = received;
                let _ = app.emit(
                    "transcriber://download",
                    DownloadProgress { id: id.clone(), received, total, done: false },
                );
            }
        }

        drop(file);
        std::fs::rename(&partial, &destination)
            .map_err(|error| format!("could not finish {}: {error}", destination.display()))?;
        let _ = app.emit(
            "transcriber://download",
            DownloadProgress { id: id.clone(), received, total: received.max(total), done: true },
        );
        Ok(())
    })
    .await
    .map_err(|error| format!("download task failed: {error}"))?
}

/// Asks the running download to stop. Idle is a harmless no-op.
#[tauri::command]
pub fn cancel_model_download(state: tauri::State<'_, DownloadState>) {
    state.0.cancel();
}

/// Removes a downloaded model file.
#[tauri::command]
pub fn delete_transcriber_model(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let file = model_file(&app, &id)?;
    std::fs::remove_file(&file)
        .map_err(|error| format!("could not remove {}: {error}", file.display()))
}

/// One caption, in seconds relative to the transcribed window's start.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    start: f64,
    end: f64,
    text: String,
}

/// The one transcription that can run at a time: a gate that refuses a
/// second concurrent run, and the child being run so a cancel can kill it
/// mid-transcription. Without the gate, a second `transcribe_clip` used to
/// overwrite the child slot - orphaning the first child unkilled and unwaited
/// while both wait loops polled the second.
pub struct TranscribeState {
    gate: Arc<SingleFlight>,
    child: Arc<Mutex<Option<Child>>>,
}

impl TranscribeState {
    pub fn new() -> Self {
        Self {
            gate: Arc::new(SingleFlight::new()),
            child: Arc::new(Mutex::new(None)),
        }
    }
}

/// Runs `command`, parking the child in `slot` so `cancel_transcribe` can
/// reach it, and waits for it to finish.
fn run_killable(
    mut command: Command,
    slot: &Arc<Mutex<Option<Child>>>,
    what: &str,
) -> Result<(), String> {
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not run {what}: {error}"))?;

    *slot.lock().expect("transcribe slot poisoned") = Some(child);

    // Wait without holding the lock, or cancel could never take it to kill.
    let mut stderr_tail = String::new();
    let status = loop {
        let mut guard = slot.lock().expect("transcribe slot poisoned");
        let Some(child) = guard.as_mut() else {
            return Err("transcription cancelled".to_owned());
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = stderr.read_to_string(&mut stderr_tail);
                }
                *guard = None;
                break status;
            }
            Ok(None) => {}
            Err(error) => {
                *guard = None;
                return Err(format!("{what} failed: {error}"));
            }
        }
        drop(guard);
        std::thread::sleep(std::time::Duration::from_millis(60));
    };

    if status.success() {
        Ok(())
    } else {
        let tail: String = stderr_tail.lines().rev().take(3).collect::<Vec<_>>().join(" / ");
        Err(format!("{what} exited with {status}: {tail}"))
    }
}

/// Extracts a window of `source` as 16 kHz mono WAV - the input whisper wants.
fn extract_wav(
    source: &str,
    start: f64,
    duration: f64,
    destination: &Path,
    slot: &Arc<Mutex<Option<Child>>>,
) -> Result<(), String> {
    let mut command = concat_media::command(concat_media::ffmpeg());
    command
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y"])
        .args(["-ss", &format!("{start:.6}")])
        .args(["-t", &format!("{duration:.6}")])
        .args(["-i", source])
        .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(destination);
    run_killable(command, slot, "ffmpeg")
}

#[derive(serde::Deserialize)]
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

/// Transcribes one clip's audio window into caption segments.
///
/// Timestamps come back relative to the window: 0 is `source_start`. The
/// frontend owns the clip, so the map onto the timeline (divide by speed, add
/// the clip's start) happens there, next to the numbers it needs.
#[tauri::command]
pub async fn transcribe_clip(
    app: tauri::AppHandle,
    state: tauri::State<'_, TranscribeState>,
    request: TranscribeRequest,
) -> Result<Vec<Segment>, String> {
    let job = state.gate.begin("transcription")?;
    let slot = Arc::clone(&state.child);
    tauri::async_runtime::spawn_blocking(move || {
        let result = run_transcription(&app, &slot, &request);
        drop(job);
        result
    })
    .await
    .map_err(|error| format!("transcribe task failed: {error}"))?
}

/// Kills the running transcription's child process, if any.
#[tauri::command]
pub fn cancel_transcribe(state: tauri::State<'_, TranscribeState>) {
    let mut guard = state.child.lock().expect("transcribe slot poisoned");
    if let Some(mut child) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn run_transcription(
    app: &tauri::AppHandle,
    slot: &Arc<Mutex<Option<Child>>>,
    request: &TranscribeRequest,
) -> Result<Vec<Segment>, String> {
    let binary = find_binary(app).ok_or_else(|| {
        "whisper-cli was not found - point Settings > Transcriber at it".to_owned()
    })?;
    let model = model_file(app, &request.model_id)?;
    if !model.is_file() {
        return Err(format!(
            "model {} is not downloaded - see Settings > Transcriber",
            request.model_id
        ));
    }
    // NaN is refused along with zero and negative windows.
    if request.window.is_nan() || request.window <= 0.0 {
        return Err("nothing to transcribe: the clip covers no time".to_owned());
    }

    // Unique scratch names, so two clips transcribed back to back (or a
    // crashed run's leftovers) can never collide.
    // Wall-clock millis alone collide when two runs start in the same
    // instant; the process id and a counter make the name unique without
    // reaching for a random source.
    static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = format!(
        "{}-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0),
        std::process::id(),
        RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    );
    let scratch = std::env::temp_dir();
    let wav = scratch.join(format!("wolfcut-transcribe-{nonce}.wav"));
    let out_base = scratch.join(format!("wolfcut-transcribe-{nonce}"));
    let json = scratch.join(format!("wolfcut-transcribe-{nonce}.json"));

    let result = (|| {
        extract_wav(&request.path, request.source_start, request.window, &wav, slot)?;

        let threads = std::thread::available_parallelism()
            .map(|count| count.get().min(8))
            .unwrap_or(4);

        let mut command = concat_media::command(&binary);
        command
            .arg("-m")
            .arg(&model)
            .arg("-f")
            .arg(&wav)
            .args(["-l", &request.language])
            .args(["-t", &threads.to_string()])
            // JSON to a file: stdout is progress noise, the file is the data.
            .arg("-oj")
            .arg("-of")
            .arg(&out_base)
            .args(["-np"]);
        run_killable(command, slot, "whisper-cli")?;

        let text = std::fs::read_to_string(&json)
            .map_err(|error| format!("whisper wrote no output: {error}"))?;
        parse_whisper_json(&text)
    })();

    // Scratch files are meaningless the moment we have segments (or gave up).
    let _ = std::fs::remove_file(&wav);
    let _ = std::fs::remove_file(&json);

    result
}

/// Pulls segments out of whisper-cli's `-oj` JSON.
///
/// Only `offsets` (integer milliseconds) are read. The pretty `timestamps`
/// strings next to them are for subtitle files, and parsing clocks out of
/// strings when the integers sit right there would be self-harm.
fn parse_whisper_json(text: &str) -> Result<Vec<Segment>, String> {
    let root: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("whisper output was not json: {error}"))?;
    let entries = root
        .get("transcription")
        .and_then(|value| value.as_array())
        .ok_or_else(|| "whisper output had no `transcription` array".to_owned())?;

    Ok(entries
        .iter()
        .filter_map(|entry| {
            let offsets = entry.get("offsets")?;
            let start = offsets.get("from")?.as_u64()? as f64 / 1000.0;
            let end = offsets.get("to")?.as_u64()? as f64 / 1000.0;
            let text = entry.get("text")?.as_str()?.trim().to_owned();
            // Whisper marks silence and music with bracketed stage directions
            // ("[BLANK_AUDIO]", "(upbeat music)"). Those are not captions.
            if text.is_empty() || (text.starts_with('[') && text.ends_with(']')) {
                return None;
            }
            (end > start).then_some(Segment { start, end, text })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_whisper_segments() {
        let json = r#"{"transcription":[
            {"offsets":{"from":0,"to":2500},"text":" Hello there."},
            {"offsets":{"from":2500,"to":2500},"text":"degenerate"},
            {"offsets":{"from":3000,"to":4000},"text":"[BLANK_AUDIO]"},
            {"offsets":{"from":4000,"to":6000},"text":" Second line "}
        ]}"#;
        let segments = parse_whisper_json(json).expect("parses");
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "Hello there.");
        assert_eq!(segments[0].start, 0.0);
        assert_eq!(segments[0].end, 2.5);
        assert_eq!(segments[1].text, "Second line");
    }

    #[test]
    fn refuses_output_with_no_transcription() {
        assert!(parse_whisper_json("{}").is_err());
        assert!(parse_whisper_json("not json").is_err());
    }

    #[test]
    fn every_known_model_has_a_url_and_file_name() {
        for model in KNOWN_MODELS {
            assert!(model_url(model.id).ends_with(&format!("ggml-{}.bin", model.id)));
            assert!(model.approx_bytes > 0);
        }
    }
}

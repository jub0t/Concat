// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Tauri host for the WolfCut editor.
//!
//! Deliberately thin. Every command here does two things: call into the engine
//! crates, and convert the result to something `serde` can put on the wire.
//! No editing logic lives on this side of the bridge - if a command starts
//! making decisions about the edit, that logic belongs in `wolfcut-core` or
//! `wolfcut-render` where it can be unit-tested without a window.

// Public so the integration tests can drive a real export; see tests/.
pub mod export;
mod editor_api;
mod jobs;
mod playback;
mod projects;
mod templates;
mod transcribe;
mod tts;

use serde::Serialize;

/// A video stream, as the UI sees it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoStreamInfo {
    index: u32,
    codec: String,
    width: u32,
    height: u32,
    /// Decimal fps, for display only.
    frame_rate: f64,
    /// The exact fraction the engine actually works in, e.g. "30000/1001".
    frame_rate_fraction: String,
}

/// An audio stream, as the UI sees it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStreamInfo {
    index: u32,
    codec: String,
    sample_rate: u32,
    channels: u32,
}

/// What `probe_media` hands back.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSummary {
    path: String,
    duration: Option<f64>,
    /// "video", "audio" or "image".
    kind: &'static str,
    video: Option<VideoStreamInfo>,
    audio: Option<AudioStreamInfo>,
}

/// Extensions WolfCut is willing to treat as stills.
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "avif", "heic", "heif", "gif",
];

/// Decides whether a file is footage, sound or a still.
///
/// Extension first, because that is what the user means by "a png", and
/// ffprobe is genuinely ambiguous here: a PNG presents as a one-frame video
/// stream, usually with `r_frame_rate` of 25/1 invented by the demuxer.
///
/// The duration check is what separates a still from an animation. An animated
/// GIF or WebP reports a duration; a single image does not. It is a heuristic,
/// and a deliberately conservative one - misreading an animation as a still
/// shows its first frame rather than failing.
fn classify(info: &wolfcut_media::MediaInfo) -> &'static str {
    if info.video.is_none() {
        return "audio";
    }

    let extension = info
        .path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if IMAGE_EXTENSIONS.contains(&extension.as_str()) && info.duration.is_none() {
        "image"
    } else {
        "video"
    }
}

impl From<wolfcut_media::MediaInfo> for MediaSummary {
    fn from(info: wolfcut_media::MediaInfo) -> Self {
        Self {
            kind: classify(&info),
            path: info.path.to_string_lossy().into_owned(),
            // Exact rational time stops at this boundary: JSON has no
            // fractions, and the UI only ever displays these numbers.
            duration: info.duration.map(|duration| duration.as_f64()),
            video: info.video.map(|video| VideoStreamInfo {
                index: video.index,
                codec: video.codec,
                width: video.width,
                height: video.height,
                frame_rate: video.frame_rate.fps().as_f64(),
                frame_rate_fraction: format!(
                    "{}/{}",
                    video.frame_rate.fps().numerator(),
                    video.frame_rate.fps().denominator()
                ),
            }),
            audio: info.audio.map(|audio| AudioStreamInfo {
                index: audio.index,
                codec: audio.codec,
                sample_rate: audio.sample_rate,
                channels: audio.channels,
            }),
        }
    }
}

/// Reports what is inside a media file.
///
/// Async because ffprobe on a slow or network volume takes real time, and a
/// synchronous command runs on the main thread - the one place a stall is a
/// frozen window.
///
/// A successful probe also admits the file to the asset protocol scope, so
/// the preview's media elements can play it. This is where user intent is
/// expressed - importing a file IS asking the app to show it - and it is the
/// whole grant: the static scope is empty, so the webview can only ever
/// reach files that probed as media. See `grant_asset` for why.
#[tauri::command]
async fn probe_media(app: tauri::AppHandle, path: String) -> Result<MediaSummary, String> {
    let summary = tauri::async_runtime::spawn_blocking(move || {
        wolfcut_media::probe(&path).map(MediaSummary::from).map_err(describe)
    })
    .await
    .map_err(|error| format!("probe task failed: {error}"))??;
    grant_asset(&app, &summary.path);
    Ok(summary)
}

/// Admits one file to the asset protocol scope.
///
/// The static scope used to be `**` - the whole filesystem - which made any
/// script that ran in the webview a disk-wide read primitive. Now the scope
/// starts empty and grows one file at a time, only at the two places the
/// user expresses intent: importing media (a successful probe) and opening a
/// project whose document lists it. A path that never probed as media is
/// unreachable however the page is compromised.
pub(crate) fn grant_asset(app: &tauri::AppHandle, path: &str) {
    use tauri::Manager;
    // Failure means the preview cannot show this one file - the import
    // itself still stands, and the error names the path for the console.
    if let Err(error) = app.asset_protocol_scope().allow_file(path) {
        eprintln!("wolfcut: could not scope {path}: {error}");
    }
}

/// The version of the app the UI is talking to. Also a liveness check on the
/// IPC bridge at startup.
#[tauri::command]
fn engine_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The most `read_media_bytes` will hand the webview at once.
///
/// Its remaining callers decode still images and register fonts - assets
/// that are megabytes, not gigabytes. The cap is what keeps the command
/// from quietly growing back into a whole-disk read primitive now that
/// waveform peaks (the one legitimate whole-file consumer) decode in the
/// engine instead.
const MEDIA_READ_CAP: u64 = 64 * 1024 * 1024;

/// Reads a whole file and returns it as raw bytes.
///
/// Two callers: still-image decode in `lib/assets.ts`, and custom font
/// registration. `tauri::ipc::Response` puts the bytes on the wire as an
/// ArrayBuffer rather than a JSON array of numbers, which for a few
/// megabytes is the difference between instant and unusable.
///
/// It loads the entire file into memory, so it is not a general-purpose
/// reader and must not become one: anything past [`MEDIA_READ_CAP`] is
/// refused.
#[tauri::command]
async fn read_media_bytes(path: String) -> Result<tauri::ipc::Response, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let size = std::fs::metadata(&path)
            .map_err(|error| format!("could not read {path}: {error}"))?
            .len();
        if size > MEDIA_READ_CAP {
            return Err(format!(
                "refusing to read {path}: {size} bytes is over the {MEDIA_READ_CAP} byte limit"
            ));
        }
        std::fs::read(&path).map_err(|error| format!("could not read {path}: {error}"))
    })
    .await
    .map_err(|error| format!("read task failed: {error}"))?
    .map(tauri::ipc::Response::new)
}

/// Resolution of the cached waveform.
///
/// 200 buckets per second is roughly two buckets per pixel at the default
/// timeline zoom, which is enough that the drawn shape does not visibly
/// change as you zoom in a step or two, without storing the whole decoded
/// file.
const PEAKS_BUCKETS_PER_SECOND: u32 = 200;

/// Waveform peaks for one media file: engine-decoded, project-cached.
///
/// The engine streams FFmpeg's decode into min/max buckets, so neither the
/// file nor its samples are ever resident - and nothing crosses the IPC
/// boundary but the buckets themselves. The result is cached in the
/// project's `cache/` folder under a key derived from the path, and served
/// from there on every later call; `project: None` (an unsaved session)
/// just skips the cache.
#[tauri::command]
async fn extract_peaks(path: String, project: Option<String>) -> Result<tauri::ipc::Response, String> {
    tauri::async_runtime::spawn_blocking(move || peaks_bytes(&path, project.as_deref()))
        .await
        .map_err(|error| format!("peaks task failed: {error}"))?
        .map(tauri::ipc::Response::new)
}

fn peaks_bytes(path: &str, project: Option<&str>) -> Result<Vec<u8>, String> {
    let cached = project.and_then(|project| artwork_file(project, &peaks_key(path)).ok());
    if let Some(file) = &cached {
        if let Ok(bytes) = std::fs::read(file) {
            // A corrupt entry falls through to regeneration rather than
            // being served - the frontend has no second request to make.
            if plausible_peaks(&bytes) {
                return Ok(bytes);
            }
        }
    }

    let peaks = wolfcut_media::peaks::extract(std::path::Path::new(path), PEAKS_BUCKETS_PER_SECOND)
        .map_err(describe)?;
    let bytes = peaks.encode();

    // Best-effort, like every artwork write: a failed cache write only
    // means decoding again next launch.
    if let Some(file) = &cached {
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(file, &bytes);
    }
    Ok(bytes)
}

/// The cache filename for one file's peaks.
///
/// FNV-1a 64 over the path, like the audio cache's `decode_key` and for the
/// same reason: these keys name files that outlive the process, and
/// `DefaultHasher` is free to change between Rust releases. The bucket rate
/// rides in the name so a resolution change regenerates instead of serving
/// yesterday's shape.
fn peaks_key(path: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in path.as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}-b{PEAKS_BUCKETS_PER_SECOND}.peaks")
}

/// Whether bytes have the shape `Peaks::encode` writes:
/// `[rate f32][count u32][min f32 x count][max f32 x count]`.
fn plausible_peaks(bytes: &[u8]) -> bool {
    if bytes.len() < 8 {
        return false;
    }
    let count = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    bytes.len() == 8 + count * 8
}

/// Where one artwork file lives inside a project's cache.
///
/// The cache sits in the project folder so it travels with the project and
/// vanishes with it. The key is confined to a single flat filename - anything
/// that could walk out of the folder is refused rather than sanitised,
/// because the only caller is our own frontend and a strange key is a bug.
fn artwork_file(project: &str, key: &str) -> Result<std::path::PathBuf, String> {
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        || key.starts_with('.')
    {
        return Err(format!("refusing artwork key {key:?}"));
    }
    let root = std::path::Path::new(project);
    // A real project, not merely a directory: without the manifest check,
    // this command was a write primitive into any folder on disk that could
    // grow a `cache/` child. The manifest is what makes a folder ours.
    if !projects::manifest_path(root).is_file() {
        return Err(format!("{project} is not a project folder"));
    }
    Ok(root.join("cache").join(key))
}

/// Returns one cached artwork file, or an error the frontend treats as a miss.
#[tauri::command]
async fn read_artwork(project: String, key: String) -> Result<tauri::ipc::Response, String> {
    let file = artwork_file(&project, &key)?;
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read(&file).map_err(|error| format!("no cached artwork {key}: {error}"))
    })
    .await
    .map_err(|error| format!("artwork task failed: {error}"))?
    .map(tauri::ipc::Response::new)
}

/// Stores one artwork file in the project's cache. Best-effort: the caller
/// fires and forgets, and a failed write only means regenerating next launch.
#[tauri::command]
async fn write_artwork(project: String, key: String, bytes: Vec<u8>) -> Result<(), String> {
    let file = artwork_file(&project, &key)?;
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        std::fs::write(&file, bytes).map_err(|error| format!("could not write {key}: {error}"))
    })
    .await
    .map_err(|error| format!("artwork task failed: {error}"))?
}

/// Renders a strip of evenly spaced frames from a video as one JPEG.
///
/// One image rather than N, because the timeline draws the frames as slices of
/// a single texture - that is one decode and one cached bitmap instead of
/// twenty-four of each.
///
/// `fps=count/duration` is what spaces the frames evenly, and `tile` lays them
/// out side by side. If the container reports no duration there is nothing to
/// space frames across, so this refuses rather than guessing.
#[tauri::command]
async fn extract_filmstrip(path: String, count: u32, height: u32) -> Result<tauri::ipc::Response, String> {
    let bytes = tauri::async_runtime::spawn_blocking(move || filmstrip(&path, count, height))
        .await
        .map_err(|error| format!("filmstrip task failed: {error}"))??;

    Ok(tauri::ipc::Response::new(bytes))
}

fn filmstrip(path: &str, count: u32, height: u32) -> Result<Vec<u8>, String> {
    let count = count.clamp(1, 60);
    let height = height.clamp(16, 240);

    let info = wolfcut_media::probe(path).map_err(describe)?;
    let duration = info
        .duration
        .map(|duration| duration.as_f64())
        .filter(|seconds| *seconds > 0.0)
        .ok_or_else(|| format!("{path} reports no duration"))?;

    let output = wolfcut_media::command(wolfcut_media::ffmpeg())
        .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
        .args(["-i", path])
        .args([
            "-vf",
            // scale=-2 keeps the aspect ratio and an even width, which the
            // JPEG encoder requires.
            // lanczos because the default bilinear leaves 4K sources visibly
            // mushy at thumbnail sizes, and the strip is rendered once but
            // looked at constantly.
            &format!(
                "fps={:.6},scale=-2:{height}:flags=lanczos,tile={count}x1",
                f64::from(count) / duration
            ),
        ])
        .args(["-frames:v", "1", "-q:v", "3", "-f", "mjpeg", "pipe:1"])
        .output()
        .map_err(|error| format!("could not run ffmpeg: {error}"))?;

    if !output.status.success() {
        return Err(format!("ffmpeg exited with {} for {path}", output.status));
    }
    if output.stdout.is_empty() {
        return Err(format!("ffmpeg produced no filmstrip for {path}"));
    }

    Ok(output.stdout)
}

/// A small poster frame for one project, for the launch screen's recents.
///
/// Grabbed from the earliest visible clip of the project's active timeline
/// and cached as `cache/preview.jpg` in the project folder; the cache is
/// fresh as long as it is newer than `wolfcut.json`, so an edited project gets
/// a new poster on its next appearance and an untouched one costs a stat.
#[tauri::command]
async fn project_preview(path: String) -> Result<tauri::ipc::Response, String> {
    tauri::async_runtime::spawn_blocking(move || poster_frame(&path))
        .await
        .map_err(|error| format!("preview task failed: {error}"))?
        .map(tauri::ipc::Response::new)
}

fn poster_frame(project: &str) -> Result<Vec<u8>, String> {
    let root = std::path::Path::new(project);
    let manifest = projects::manifest_path(root);
    let cached = root.join("cache").join("preview.jpg");

    let fresh = match (std::fs::metadata(&cached), std::fs::metadata(&manifest)) {
        (Ok(cache), Ok(source)) => match (cache.modified(), source.modified()) {
            (Ok(cache), Ok(source)) => cache >= source,
            _ => false,
        },
        _ => false,
    };
    if fresh {
        if let Ok(bytes) = std::fs::read(&cached) {
            return Ok(bytes);
        }
    }

    let text = std::fs::read_to_string(&manifest)
        .map_err(|error| format!("could not read {}: {error}", manifest.display()))?;
    let document: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("not a project: {error}"))?;

    // Typed access through the engine's own reader, not hand-parsed JSON -
    // a schema change breaks this at compile time now, not silently at the
    // next launch screen.
    let Some(project) = wolfcut_project::from_document(&document) else {
        return Err("the project has no timeline to preview".to_owned());
    };
    let timeline = project
        .timelines
        .iter()
        .find(|timeline| timeline.id == project.active_timeline_id)
        .or_else(|| project.timelines.first());
    let Some(timeline) = timeline else {
        return Err("the project has no timeline to preview".to_owned());
    };

    // The earliest clip with a picture is the poster - exactly the frame
    // the user last saw open.
    use wolfcut_project::model::ClipKind;
    let mut poster: Option<(f64, String, f64, bool)> = None;
    for clip in &timeline.clips {
        if clip.kind != ClipKind::Video && clip.kind != ClipKind::Image {
            continue;
        }
        if poster.as_ref().is_some_and(|(best, ..)| *best <= clip.start) {
            continue;
        }
        let Some(media) = project.media.iter().find(|item| item.id == clip.media_id) else {
            continue;
        };
        poster = Some((
            clip.start,
            media.path.clone(),
            clip.source_start,
            clip.kind == ClipKind::Image,
        ));
    }

    let Some((_, media_path, source_start, is_still)) = poster else {
        return Err("nothing on the timeline to preview".to_owned());
    };

    let mut command = wolfcut_media::command(wolfcut_media::ffmpeg());
    command.args(["-hide_banner", "-nostdin", "-loglevel", "error"]);
    // Seeking a still means seeking a one-frame stream to nowhere.
    if !is_still && source_start > 0.0 {
        command.args(["-ss", &format!("{source_start:.3}")]);
    }
    let output = command
        .args(["-i", &media_path])
        .args(["-frames:v", "1", "-vf", "scale=480:-2", "-q:v", "4", "-f", "mjpeg", "pipe:1"])
        .output()
        .map_err(|error| format!("could not run ffmpeg: {error}"))?;

    if !output.status.success() || output.stdout.is_empty() {
        return Err(format!("no poster frame for {media_path}"));
    }

    // Best effort: a failed cache write only means regenerating next launch.
    if let Some(parent) = cached.parent() {
        let _ = std::fs::create_dir_all(parent);
        let _ = std::fs::write(&cached, &output.stdout);
    }
    Ok(output.stdout)
}

/// The audible clip set, replaced wholesale whenever the timeline changes.
///
/// Decoding, mixing and the playback clock all live in the engine - see
/// `playback.rs`. The UI describes what should be audible and follows the
/// engine's `transport` position events.
#[tauri::command]
fn audio_set_clips(
    state: tauri::State<'_, std::sync::Arc<playback::Playback>>,
    project: String,
    clips: Vec<playback::ClipSpec>,
) {
    state.set_clips(std::path::PathBuf::from(project), clips);
}

#[tauri::command]
fn transport_play(state: tauri::State<'_, std::sync::Arc<playback::Playback>>, position: f64) {
    state.play(position);
}

#[tauri::command]
fn transport_pause(state: tauri::State<'_, std::sync::Arc<playback::Playback>>) {
    state.pause();
}

#[tauri::command]
fn transport_seek(state: tauri::State<'_, std::sync::Arc<playback::Playback>>, position: f64) {
    state.seek(position);
}

/// Creates a project folder, writes its manifest and records it as recent.
#[tauri::command]
async fn create_project(
    app: tauri::AppHandle,
    location: String,
    name: String,
    width: u32,
    height: u32,
    rate_num: i64,
    rate_den: i64,
) -> Result<projects::ProjectInfo, String> {
    let config = config_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let project = projects::create(&location, &name, width, height, rate_num, rate_den)?;
        // A project that cannot be added to the recents list has still been
        // created, so this failure is reported but not fatal.
        if let Err(error) = projects::remember(&config, &project) {
            eprintln!("wolfcut: {error}");
        }
        Ok(project)
    })
    .await
    .map_err(|error| format!("create_project task failed: {error}"))?
}

/// Reads an existing project and moves it to the front of the recents list.
#[tauri::command]
async fn open_project(app: tauri::AppHandle, path: String) -> Result<projects::ProjectInfo, String> {
    let config = config_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let project = projects::open(&path)?;
        if let Err(error) = projects::remember(&config, &project) {
            eprintln!("wolfcut: {error}");
        }
        Ok(project)
    })
    .await
    .map_err(|error| format!("open_project task failed: {error}"))?
}

/// The recents list, most recent first, with vanished folders left out.
#[tauri::command]
async fn recent_projects(app: tauri::AppHandle) -> Result<Vec<projects::ProjectInfo>, String> {
    let config = config_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || projects::list(&config))
        .await
        .map_err(|error| format!("recents task failed: {error}"))
}

/// Removes a project from the recents list. The folder itself is left alone.
#[tauri::command]
async fn forget_project(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let config = config_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || projects::forget(&config, &path))
        .await
        .map_err(|error| format!("forget task failed: {error}"))?
}

/// The template library, for the gallery.
#[tauri::command]
async fn template_list(app: tauri::AppHandle) -> Result<Vec<templates::TemplateInfo>, String> {
    let config = config_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || templates::list(&config))
        .await
        .map_err(|error| format!("template list task failed: {error}"))
}

/// Packs the open project into a new template bundle.
#[tauri::command]
async fn template_save(
    app: tauri::AppHandle,
    state: tauri::State<'_, editor_api::EditorState>,
    name: String,
) -> Result<templates::TemplateInfo, String> {
    let (document, project_path, settings) = editor_api::session_snapshot(&state)?;
    let config = config_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        templates::save(&config, &document, &settings, &project_path, &name)
    })
    .await
    .map_err(|error| format!("template save task failed: {error}"))?
}

/// Unpacks a template into a fresh project with every slot filled, and
/// records it as recent - the caller opens it like any other project.
#[tauri::command]
async fn template_instantiate(
    app: tauri::AppHandle,
    template: String,
    location: String,
    name: String,
    fills: Vec<templates::SlotFill>,
) -> Result<projects::ProjectInfo, String> {
    let config = config_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let project = templates::instantiate(&template, &location, &name, fills)?;
        if let Err(error) = projects::remember(&config, &project) {
            eprintln!("wolfcut: {error}");
        }
        Ok(project)
    })
    .await
    .map_err(|error| format!("template task failed: {error}"))?
}

/// Removes one template bundle for good.
#[tauri::command]
async fn template_delete(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let config = config_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || templates::delete(&config, &path))
        .await
        .map_err(|error| format!("template delete task failed: {error}"))?
}

/// A template's poster, or an error the UI treats as "no art".
#[tauri::command]
async fn template_poster(path: String) -> Result<tauri::ipc::Response, String> {
    tauri::async_runtime::spawn_blocking(move || templates::poster(&path))
        .await
        .map_err(|error| format!("poster task failed: {error}"))?
        .map(tauri::ipc::Response::new)
}

fn config_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .map_err(|error| format!("could not locate the config directory: {error}"))
}

/// The slot for the one export that can run at a time - enforced, not
/// documented: a second `export_project` while one runs is refused instead of
/// racing the first for its temp files and cancel flag.
struct ExportState(std::sync::Arc<jobs::SingleFlight>);

/// The reader pool behind the paused monitor's true frames. One pool for the
/// app's lifetime: its whole value is what stays warm between scrubs.
struct PoolState(std::sync::Arc<std::sync::Mutex<wolfcut_media::ReaderPool>>);

/// What the UI sends for one true frame: an instant and a resolution. The
/// clips come from the engine's own session - the UI no longer serialises
/// its whole clip list per scrub frame (see engine decision 0009).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewSpec {
    /// The timeline instant to composite, in seconds.
    time: f64,
    /// Preview frame width in pixels.
    width: u32,
    /// Preview frame height in pixels.
    height: u32,
}

/// The engine-composited frame at one instant, as raw RGBA bytes.
///
/// Serialised through the pool's mutex on a blocking thread: one scrub at a
/// time, in order, off the main thread. The UI debounces and drops stale
/// responses, so a slow decode never wedges anything but itself.
#[tauri::command]
async fn preview_frame(
    state: tauri::State<'_, PoolState>,
    editor: tauri::State<'_, editor_api::EditorState>,
    request: PreviewSpec,
) -> Result<tauri::ipc::Response, String> {
    let (clips, settings) = editor_api::flattened_clips(&editor)?;
    let request = export::PreviewFrameRequest {
        time: request.time,
        width: request.width,
        height: request.height,
        rate_num: settings.rate_num,
        rate_den: settings.rate_den,
        clips,
    };
    let pool = std::sync::Arc::clone(&state.0);
    tauri::async_runtime::spawn_blocking(move || {
        let mut pool = pool.lock().map_err(|_| "reader pool poisoned".to_owned())?;
        export::preview_frame(&mut pool, &request)
    })
    .await
    .map_err(|error| format!("preview task failed: {error}"))?
    // The UI quietly keeps its approximation on error, which is right for
    // the monitor and useless for debugging - so the reason lands here.
    .inspect_err(|error| eprintln!("wolfcut: preview_frame: {error}"))
    .map(tauri::ipc::Response::new)
}

/// Decode-ahead for the playback stream.
///
/// Warms the pool for the next few frame instants after `request.time`, so
/// the following `preview_frame` pulls are cache hits instead of decode
/// waits. Fire-and-forget from the UI: a source that will not decode fails
/// the pull too, and the pull is the path that reports it.
#[tauri::command]
async fn preview_prefetch(
    state: tauri::State<'_, PoolState>,
    editor: tauri::State<'_, editor_api::EditorState>,
    request: PreviewSpec,
    frames: u32,
) -> Result<(), String> {
    let (clips, settings) = editor_api::flattened_clips(&editor)?;
    let request = export::PreviewFrameRequest {
        time: request.time,
        width: request.width,
        height: request.height,
        rate_num: settings.rate_num,
        rate_den: settings.rate_den,
        clips,
    };
    let pool = std::sync::Arc::clone(&state.0);
    tauri::async_runtime::spawn_blocking(move || {
        if let Ok(mut pool) = pool.lock() {
            // Clamped so a confused caller cannot park the pool's mutex on a
            // long decode march the presenter then queues behind.
            export::preview_prefetch(&mut pool, &request, frames.min(8));
        }
    })
    .await
    .map_err(|error| format!("prefetch task failed: {error}"))
}

/// Renders the timeline to a file.
///
/// Runs on a blocking thread and reports progress through the
/// `export://progress` event, because a two-minute export must not freeze the
/// window and gives the UI nothing to show if it says nothing until it is done.
/// What the UI sends to start an export: the destination, the quality, and
/// its rasterised titles rejoining as image clips. The timeline itself is
/// deliberately absent - the engine flattens its own session, so the pixels
/// rendered are the model's, never a frontend's copy of it.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportSpec {
    /// The file to write.
    output: String,
    /// Constant rate factor; lower is better quality and a bigger file.
    crf: u8,
    /// The x264 speed/size preset name, e.g. "medium".
    preset: String,
    /// Rasterised text clips - fonts and layout live in the webview, so
    /// titles arrive as images the flattener cannot produce itself.
    #[serde(default)]
    titles: Vec<export::ExportClip>,
}

#[tauri::command]
async fn export_project(
    app: tauri::AppHandle,
    state: tauri::State<'_, ExportState>,
    editor: tauri::State<'_, editor_api::EditorState>,
    request: ExportSpec,
) -> Result<String, String> {
    let (mut clips, settings) = editor_api::flattened_clips(&editor)?;
    clips.extend(request.titles);
    let request = export::ExportRequest {
        output: request.output,
        width: settings.width,
        height: settings.height,
        rate_num: settings.rate_num,
        rate_den: settings.rate_den,
        crf: request.crf,
        preset: request.preset,
        clips,
    };
    let job = state.0.begin("export")?;
    tauri::async_runtime::spawn_blocking(move || export::run(&app, request, job.cancel_flag()))
        .await
        .map_err(|error| format!("export task failed: {error}"))?
}

/// Asks the running export to stop at the next frame. Idle is a harmless no-op.
#[tauri::command]
fn cancel_export(state: tauri::State<'_, ExportState>) {
    state.0.cancel();
}

/// Flattens an error and its causes into one line.
///
/// `Display` on a `thiserror` enum prints only the outermost message, and the
/// useful half - what FFmpeg or the OS actually said - is in the source chain.
fn describe(error: wolfcut_media::Error) -> String {
    use std::error::Error;

    let mut message = error.to_string();
    let mut cause = error.source();
    while let Some(current) = cause {
        message.push_str(&format!(": {current}"));
        cause = current.source();
    }
    message
}

/// Points the engine at the bundled FFmpeg.
///
/// WolfCut ships its own copy so that a fresh install works without the user
/// having installed anything - "is FFmpeg on PATH?" is not a question anyone
/// should have to answer to open a video file.
///
/// Searched in order: the bundled resource directory, then beside the
/// executable (which is where a `cargo run` build finds it), then nothing -
/// leaving the engine on its `PATH` default, which is what keeps development
/// working without copying 170 MB into every build directory.
///
/// Both binaries must be present to switch. A bundled decoder paired with
/// whatever `ffprobe` happens to be on PATH is a version mismatch waiting to
/// happen, and it would be an intermittent one.
fn use_bundled_ffmpeg(app: &tauri::App) {
    use tauri::Manager;

    let suffix = if cfg!(windows) { ".exe" } else { "" };

    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(resources) = app.path().resource_dir() {
        candidates.push(resources.join("ffmpeg"));
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join("ffmpeg"));
            candidates.push(directory.to_path_buf());
        }
    }

    for directory in candidates {
        let ffmpeg = directory.join(format!("ffmpeg{suffix}"));
        let ffprobe = directory.join(format!("ffprobe{suffix}"));

        if ffmpeg.is_file() && ffprobe.is_file() {
            wolfcut_media::set_binaries(ffmpeg, ffprobe);
            return;
        }
    }
}

/// Builds and runs the editor window.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // Resolved here rather than before the builder, because finding
            // the resource directory needs the app to exist.
            use tauri::Manager;
            use_bundled_ffmpeg(app);
            app.manage(playback::Playback::start(app.handle().clone()));
            app.manage(ExportState(std::sync::Arc::new(jobs::SingleFlight::new())));
            app.manage(transcribe::DownloadState(std::sync::Arc::new(
                jobs::SingleFlight::new(),
            )));
            app.manage(transcribe::TranscribeState::new());
            app.manage(tts::TtsDownloadState(std::sync::Arc::new(
                jobs::SingleFlight::new(),
            )));
            app.manage(tts::TtsState::new());
            app.manage(editor_api::EditorState(std::sync::Mutex::new(None)));
            app.manage(PoolState(std::sync::Arc::new(std::sync::Mutex::new(
                wolfcut_media::ReaderPool::with_defaults(),
            ))));
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            probe_media,
            engine_version,
            read_media_bytes,
            extract_peaks,
            audio_set_clips,
            transport_play,
            transport_pause,
            transport_seek,
            extract_filmstrip,
            read_artwork,
            write_artwork,
            create_project,
            open_project,
            project_preview,
            recent_projects,
            forget_project,
            export_project,
            cancel_export,
            preview_frame,
            preview_prefetch,
            template_list,
            template_save,
            template_instantiate,
            template_delete,
            template_poster,
            editor_api::editor_open,
            editor_api::editor_apply,
            editor_api::editor_undo,
            editor_api::editor_redo,
            editor_api::editor_state,
            editor_api::editor_save,
            editor_api::editor_close,
            transcribe::transcriber_status,
            transcribe::set_transcriber_binary,
            transcribe::download_transcriber_model,
            transcribe::cancel_model_download,
            transcribe::delete_transcriber_model,
            transcribe::transcribe_clip,
            transcribe::cancel_transcribe,
            tts::tts_status,
            tts::download_tts_model,
            tts::cancel_tts_model_download,
            tts::delete_tts_model,
            tts::speak_text,
            tts::cancel_speak
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

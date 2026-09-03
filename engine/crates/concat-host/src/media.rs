// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Media as the window sees it: what a file is, its waveform, its
//! filmstrip, a project's poster - and the caches beside the project that
//! keep each of those from being computed twice.

use std::path::{Path, PathBuf};

use concat_core::frame::Frame;
use concat_core::time::Rational;
use concat_media::{DecodeOptions, Decoder, FrameSource, SeekableSource};
use serde::Serialize;

use crate::projects;

/// A video stream, as the UI sees it.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VideoStreamInfo {
    /// Stream index within the file.
    pub index: u32,
    /// Codec short name.
    pub codec: String,
    /// Displayed width in pixels.
    pub width: u32,
    /// Displayed height in pixels.
    pub height: u32,
    /// Decimal fps, for display only.
    pub frame_rate: f64,
    /// The exact fraction the engine actually works in, e.g. "30000/1001".
    pub frame_rate_fraction: String,
}

/// An audio stream, as the UI sees it.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AudioStreamInfo {
    /// Stream index within the file.
    pub index: u32,
    /// Codec short name.
    pub codec: String,
    /// Samples per second.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u32,
}

/// What [`probe`] hands back.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MediaSummary {
    /// The file, as given.
    pub path: String,
    /// Container duration in seconds, when the container states one.
    pub duration: Option<f64>,
    /// What the file is.
    pub kind: concat_project::model::MediaKind,
    /// First video stream, if any.
    pub video: Option<VideoStreamInfo>,
    /// First audio stream, if any.
    pub audio: Option<AudioStreamInfo>,
}

impl MediaSummary {
    /// The bin entry this file becomes, named after its basename.
    pub fn to_new_media(&self) -> concat_project::commands::NewMedia {
        concat_project::commands::NewMedia {
            path: self.path.clone(),
            name: Path::new(&self.path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.path.clone()),
            duration: self.duration,
            kind: self.kind,
            width: self.video.as_ref().map(|video| video.width),
            height: self.video.as_ref().map(|video| video.height),
            frame_rate: self.video.as_ref().map(|video| video.frame_rate),
            frame_rate_fraction: self
                .video
                .as_ref()
                .map(|video| video.frame_rate_fraction.clone()),
            video_codec: self.video.as_ref().map(|video| video.codec.clone()),
            audio_codec: self.audio.as_ref().map(|audio| audio.codec.clone()),
            has_audio: self.audio.is_some(),
        }
    }
}

/// Extensions Concat is willing to treat as stills.
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "avif", "heic", "heif", "gif",
];

/// Decides whether a file is footage, sound or a still.
///
/// Extension first, because that is what the user means by "a png", and the
/// demuxer is genuinely ambiguous here: a PNG presents as a one-frame video
/// stream, usually with a frame rate of 25/1 invented by the demuxer.
///
/// The duration check is what separates a still from an animation. An animated
/// GIF or WebP reports a duration; a single image does not. It is a heuristic,
/// and a deliberately conservative one - misreading an animation as a still
/// shows its first frame rather than failing.
fn classify(info: &concat_media::MediaInfo) -> concat_project::model::MediaKind {
    use concat_project::model::MediaKind;
    if info.video.is_none() {
        return MediaKind::Audio;
    }

    let extension = info
        .path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if IMAGE_EXTENSIONS.contains(&extension.as_str()) && info.duration.is_none() {
        MediaKind::Image
    } else {
        MediaKind::Video
    }
}

impl From<concat_media::MediaInfo> for MediaSummary {
    fn from(info: concat_media::MediaInfo) -> Self {
        Self {
            kind: classify(&info),
            path: info.path.to_string_lossy().into_owned(),
            // Exact rational time stops at this boundary: the document is
            // f64 seconds, and the UI only ever displays these numbers.
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

/// Reports what is inside a media file. Opening a file on a slow or network
/// volume takes real time, so call this off the UI thread.
pub fn probe(path: &str) -> Result<MediaSummary, String> {
    concat_media::probe(path)
        .map(MediaSummary::from)
        .map_err(describe)
}

/// The most [`read_bytes`] will hand back at once.
///
/// Its callers decode still images and register fonts - assets that are
/// megabytes, not gigabytes. The cap is what keeps the function from quietly
/// growing into a whole-disk read primitive.
pub const MEDIA_READ_CAP: u64 = 64 * 1024 * 1024;

/// Reads a whole file, refusing anything past [`MEDIA_READ_CAP`].
pub fn read_bytes(path: &str) -> Result<Vec<u8>, String> {
    let size = std::fs::metadata(path)
        .map_err(|error| format!("could not read {path}: {error}"))?
        .len();
    if size > MEDIA_READ_CAP {
        return Err(format!(
            "refusing to read {path}: {size} bytes is over the {MEDIA_READ_CAP} byte limit"
        ));
    }
    std::fs::read(path).map_err(|error| format!("could not read {path}: {error}"))
}

/// Resolution of the cached waveform.
///
/// 200 buckets per second is roughly two buckets per pixel at the default
/// timeline zoom, which is enough that the drawn shape does not visibly
/// change as you zoom in a step or two, without storing the whole decoded
/// file.
pub const PEAKS_BUCKETS_PER_SECOND: u32 = 200;

/// Waveform peaks for one media file: engine-decoded, project-cached.
///
/// The engine streams the decode into min/max buckets, so neither the file
/// nor its samples are ever resident. The result is cached in the project's
/// `cache/` folder under a key derived from the path, and served from there
/// on every later call; `project: None` (an unsaved session) just skips the
/// cache.
pub fn peaks(path: &str, project: Option<&str>) -> Result<concat_media::peaks::Peaks, String> {
    use concat_media::peaks::Peaks;

    let cached = project.and_then(|project| artwork_file(project, &peaks_key(path)).ok());
    if let Some(file) = &cached
        && let Ok(bytes) = std::fs::read(file)
        && let Some(peaks) = Peaks::decode(&bytes)
    {
        // A corrupt entry falls through to regeneration rather than being
        // served.
        return Ok(peaks);
    }

    let peaks = concat_media::peaks::extract(Path::new(path), PEAKS_BUCKETS_PER_SECOND)
        .map_err(describe)?;

    // Best-effort, like every artwork write: a failed cache write only
    // means decoding again next launch.
    if let Some(file) = &cached {
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(file, peaks.encode());
    }
    Ok(peaks)
}

/// The cache filename for one file's peaks.
///
/// FNV-1a 64 over the path, like the audio cache's `decode_key` and for the
/// same reason: these keys name files that outlive the process, and
/// `DefaultHasher` is free to change between Rust releases. The bucket rate
/// rides in the name so a resolution change regenerates instead of serving
/// yesterday's shape.
fn peaks_key(path: &str) -> String {
    format!(
        "{:016x}-b{PEAKS_BUCKETS_PER_SECOND}.peaks",
        fnv1a(path.as_bytes())
    )
}

/// FNV-1a 64, for cache keys that must survive toolchain upgrades.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Where one artwork file lives inside a project's cache.
///
/// The cache sits in the project folder so it travels with the project and
/// vanishes with it. The key is confined to a single flat filename - anything
/// that could walk out of the folder is refused rather than sanitised,
/// because the only caller is our own window and a strange key is a bug.
pub fn artwork_file(project: &str, key: &str) -> Result<PathBuf, String> {
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        || key.starts_with('.')
    {
        return Err(format!("refusing artwork key {key:?}"));
    }
    let root = Path::new(project);
    // A real project, not merely a directory: the manifest is what makes a
    // folder ours to write a `cache/` into.
    if !projects::is_project(root) {
        return Err(format!("{project} is not a project folder"));
    }
    Ok(root.join("cache").join(key))
}

/// Returns one cached artwork file, or an error the caller treats as a miss.
pub fn read_artwork(project: &str, key: &str) -> Result<Vec<u8>, String> {
    let file = artwork_file(project, key)?;
    std::fs::read(&file).map_err(|error| format!("no cached artwork {key}: {error}"))
}

/// Stores one artwork file in the project's cache. Best-effort in spirit: a
/// failed write only means regenerating next launch.
pub fn write_artwork(project: &str, key: &str, bytes: &[u8]) -> Result<(), String> {
    let file = artwork_file(project, key)?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    std::fs::write(&file, bytes).map_err(|error| format!("could not write {key}: {error}"))
}

/// Renders a strip of evenly spaced frames from a video as one picture.
///
/// One image rather than N, because the timeline draws the frames as slices
/// of a single texture - that is one texture upload instead of twenty-four.
/// Each frame is sought directly, so the cost is `count` seeks, not a decode
/// of the whole file. If the container reports no duration there is nothing
/// to space frames across, so this refuses rather than guessing.
pub fn filmstrip(path: &str, count: u32, height: u32) -> Result<Frame, String> {
    let count = count.clamp(1, 60);
    let height = height.clamp(16, 240);

    let info = concat_media::probe(path).map_err(describe)?;
    let video = info.require_video().map_err(describe)?;
    let duration = info
        .duration
        .map(|duration| duration.as_f64())
        .filter(|seconds| *seconds > 0.0)
        .ok_or_else(|| format!("{path} reports no duration"))?;
    // Aspect-correct and even, which is what the scaler is happiest with.
    let width = ((f64::from(height) * f64::from(video.width) / f64::from(video.height)).round()
        as u32)
        .max(2)
        & !1;

    let mut decoder = Decoder::open(path, &DecodeOptions::default().scaled_to(width, height))
        .map_err(describe)?;
    let mut strip = Frame::black(width * count, height);
    let mut last: Option<Frame> = None;
    for index in 0..count {
        // Sample the middle of each slice, so the first frame is not always
        // the file's own first (often black) frame.
        let at = duration * (f64::from(index) + 0.5) / f64::from(count);
        let time = Rational::approximate(at).unwrap_or(Rational::ZERO);
        let frame = match decoder.seek(time).and_then(|()| decoder.next_frame()) {
            Ok(Some(frame)) => Some(frame),
            // Past the last frame, or a stretch that will not decode: repeat
            // the last picture rather than leaving a hole.
            _ => None,
        };
        let frame = frame.or_else(|| last.take());
        if let Some(frame) = &frame {
            strip.blit(frame, index * width, 0);
        }
        last = frame;
    }
    Ok(strip)
}

/// A small poster frame for one project, as a JPEG, for the launch screen's
/// recents.
///
/// Grabbed from the earliest visible clip of the project's active timeline
/// and cached as `cache/preview.jpg` in the project folder; the cache is
/// fresh as long as it is newer than the manifest, so an edited project gets
/// a new poster on its next appearance and an untouched one costs a stat.
pub fn poster_frame(project: &str) -> Result<Vec<u8>, String> {
    let root = Path::new(project);
    let manifest = projects::manifest_path(root);
    let cached = root.join("cache").join("preview.jpg");

    let fresh = match (std::fs::metadata(&cached), std::fs::metadata(&manifest)) {
        (Ok(cache), Ok(source)) => match (cache.modified(), source.modified()) {
            (Ok(cache), Ok(source)) => cache >= source,
            _ => false,
        },
        _ => false,
    };
    if fresh && let Ok(bytes) = std::fs::read(&cached) {
        return Ok(bytes);
    }

    let (media_path, source_start, is_still) = poster_source(project)?;
    let frame = still_at(&media_path, if is_still { 0.0 } else { source_start }, 480)?;
    let bytes = concat_media::jpeg(&frame, 4).map_err(describe)?;

    // Best effort: a failed cache write only means regenerating next launch.
    if let Some(parent) = cached.parent() {
        let _ = std::fs::create_dir_all(parent);
        let _ = std::fs::write(&cached, &bytes);
    }
    Ok(bytes)
}

/// Which frame of which file is a project's poster: the earliest clip with
/// a picture on the active timeline - exactly the frame the user last saw
/// open.
fn poster_source(project: &str) -> Result<(String, f64, bool), String> {
    let manifest = projects::manifest_path(Path::new(project));
    let text = std::fs::read_to_string(&manifest)
        .map_err(|error| format!("could not read {}: {error}", manifest.display()))?;
    let document: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("not a project: {error}"))?;

    // Typed access through the engine's own reader, not hand-parsed JSON -
    // a schema change breaks this at compile time now, not silently at the
    // next launch screen.
    let Some(project) = concat_project::from_document(&document) else {
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

    use concat_project::model::ClipKind;
    let mut poster: Option<(f64, String, f64, bool)> = None;
    for clip in &timeline.clips {
        if clip.kind != ClipKind::Video && clip.kind != ClipKind::Image {
            continue;
        }
        if poster
            .as_ref()
            .is_some_and(|(best, ..)| *best <= clip.start)
        {
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
    poster
        .map(|(_, path, source_start, still)| (path, source_start, still))
        .ok_or_else(|| "nothing on the timeline to preview".to_owned())
}

/// One frame of `path` at `seconds`, scaled to `width` across with the
/// height following the picture's shape.
pub fn still_at(path: &str, seconds: f64, width: u32) -> Result<Frame, String> {
    let info = concat_media::probe(path).map_err(describe)?;
    let video = info.require_video().map_err(describe)?;
    let height = ((f64::from(width) * f64::from(video.height) / f64::from(video.width)).round()
        as u32)
        .max(2)
        & !1;
    let mut options = DecodeOptions::default().scaled_to(width, height);
    if seconds > 0.0 {
        options = options.starting_at(Rational::approximate(seconds).unwrap_or(Rational::ZERO));
    }
    let mut decoder = Decoder::open(path, &options).map_err(describe)?;
    match decoder.next_frame().map_err(describe)? {
        Some(frame) => Ok(frame),
        // Past the end: the file's first frame beats nothing.
        None => {
            decoder.seek(Rational::ZERO).map_err(describe)?;
            decoder
                .next_frame()
                .map_err(describe)?
                .ok_or_else(|| format!("no frame to show for {path}"))
        }
    }
}

/// Flattens an error and its causes into one line.
///
/// `Display` on a `thiserror` enum prints only the outermost message, and the
/// useful half - what FFmpeg or the OS actually said - is in the source chain.
pub fn describe(error: concat_media::Error) -> String {
    use std::error::Error;

    let mut message = error.to_string();
    let mut cause = error.source();
    while let Some(current) = cause {
        message.push_str(&format!(": {current}"));
        cause = current.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artwork_keys_cannot_leave_the_cache() {
        let scratch =
            std::env::temp_dir().join(format!("concat-artwork-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch dir");
        let project = scratch.to_string_lossy().into_owned();

        // Not a project yet: refused however good the key.
        assert!(artwork_file(&project, "ok.jpg").is_err());
        std::fs::write(scratch.join("concat.json"), b"{}").expect("writes");
        assert!(artwork_file(&project, "ok.jpg").is_ok());
        for bad in ["", "../x", ".hidden", "a/b", "a\\b"] {
            assert!(
                artwork_file(&project, bad).is_err(),
                "{bad:?} must be refused"
            );
        }

        write_artwork(&project, "poster.jpg", b"jpeg").expect("writes");
        assert_eq!(
            read_artwork(&project, "poster.jpg").expect("reads"),
            b"jpeg"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn cache_keys_are_stable() {
        // Pinned: a changed hash would orphan every project's caches.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(
            peaks_key("/a.mp4"),
            format!("{:016x}-b200.peaks", fnv1a(b"/a.mp4"))
        );
    }

    #[test]
    fn a_filmstrip_of_a_synthetic_video_has_the_asked_for_shape() {
        use concat_core::time::FrameRate;
        use concat_media::{EncodeOptions, Encoder, FrameSink};

        let path =
            std::env::temp_dir().join(format!("concat-filmstrip-test-{}.mp4", std::process::id()));
        let mut encoder =
            Encoder::create(&path, 64, 32, FrameRate::THIRTY, &EncodeOptions::default())
                .expect("encodes");
        for index in 0..60u32 {
            let mut frame = Frame::black(64, 32);
            frame.fill([(index * 4).min(255) as u8, 60, 60, 255]);
            encoder.write_frame(&frame).expect("writes");
        }
        encoder.finish().expect("finishes");

        let strip = filmstrip(&path.to_string_lossy(), 4, 32).expect("strips");
        assert_eq!((strip.width(), strip.height()), (4 * 64, 32));
        // Later slices come from later in the file: the red ramps up.
        let first = strip.pixel(32, 16).expect("in bounds")[0];
        let last = strip.pixel(3 * 64 + 32, 16).expect("in bounds")[0];
        assert!(
            last > first,
            "strip is not in time order: {first} then {last}"
        );

        let poster = still_at(&path.to_string_lossy(), 1.0, 32).expect("still");
        assert_eq!(poster.width(), 32);
        let _ = std::fs::remove_file(&path);
    }
}

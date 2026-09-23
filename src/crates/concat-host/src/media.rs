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
    /// The levels the stream says its numbers span, "limited" or "full",
    /// or absent when the file says nothing - which a player then takes
    /// for limited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_range: Option<String>,
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
    /// The name the file gives the stream, or empty.
    pub title: String,
    /// The stream's language tag, or empty.
    pub language: String,
}

impl AudioStreamInfo {
    fn from_stream(audio: concat_media::AudioStream) -> Self {
        Self {
            index: audio.index,
            codec: audio.codec,
            sample_rate: audio.sample_rate,
            channels: audio.channels,
            title: audio.title,
            language: audio.language,
        }
    }

    fn to_track(&self) -> concat_project::model::AudioTrack {
        concat_project::model::AudioTrack {
            index: self.index,
            codec: self.codec.clone(),
            channels: self.channels,
            sample_rate: self.sample_rate,
            title: self.title.clone(),
            language: self.language.clone(),
        }
    }
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
    /// The first audio stream in file order, if any: what a clip plays
    /// unless it names another.
    pub audio: Option<AudioStreamInfo>,
    /// Every audio stream, in file order.
    pub audio_tracks: Vec<AudioStreamInfo>,
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
            audio_tracks: self
                .audio_tracks
                .iter()
                .map(AudioStreamInfo::to_track)
                .collect(),
            origin: None,
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
/// GIF or WebP reports a duration of many frames; a single image reports none,
/// or - through FFmpeg's `image2` demuxer, which is how a JPEG is read - the
/// length of the one frame it invented a rate for: 0.04 s at 25 fps. Counting
/// only "no duration" made every JPEG a 0.04-second video. It is a heuristic,
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

    // Up to a frame and a half, so a rate the demuxer rounded still counts.
    let one_frame = info.video.as_ref().map_or(0.1, |video| {
        video.frame_rate.frame_duration().as_f64() * 1.5
    });
    let single_image = info
        .duration
        .is_none_or(|duration| duration.as_f64() <= one_frame);
    if IMAGE_EXTENSIONS.contains(&extension.as_str()) && single_image {
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
                color_range: video.color_range.map(|range| range.name().to_owned()),
            }),
            audio: info.audio.map(AudioStreamInfo::from_stream),
            audio_tracks: info
                .audio_streams
                .into_iter()
                .map(AudioStreamInfo::from_stream)
                .collect(),
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
/// A thousand a second: a bucket is a millisecond, forty-eight samples,
/// which is what a clip a screen wide at the closest zoom needs, and the
/// pyramid folds it down for every wider view. See `concat_media::Pyramid`.
pub const PEAKS_BUCKETS_PER_SECOND: u32 = 1000;

/// Waveform peaks for one media file: engine-decoded, project-cached.
///
/// `stream` is which of the file's audio streams, by index, or `None` for
/// the first - the same choice a clip makes. The engine streams the decode
/// into min/max buckets, so neither the file nor its samples are ever
/// resident. The result is cached in the project's `cache/` folder under a
/// key derived from the path and the stream, and served from there on every
/// later call; `project: None` (an unsaved session) just skips the cache.
pub fn peaks(
    path: &str,
    stream: Option<u32>,
    project: Option<&str>,
) -> Result<concat_media::peaks::Peaks, String> {
    use concat_media::peaks::Peaks;

    let cached = project.and_then(|project| artwork_file(project, &peaks_key(path, stream)).ok());
    if let Some(file) = &cached
        && let Ok(bytes) = std::fs::read(file)
        && let Some(peaks) = Peaks::decode(&bytes)
    {
        // A corrupt entry falls through to regeneration rather than being
        // served.
        return Ok(peaks);
    }

    let peaks = concat_media::peaks::extract(
        Path::new(path),
        PEAKS_BUCKETS_PER_SECOND,
        stream.map(|index| index as usize),
    )
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
/// yesterday's shape. A named stream rides in it too; the default stream's
/// name is unchanged from before streams were named, so every cache written
/// until then is still served.
fn peaks_key(path: &str, stream: Option<u32>) -> String {
    let stream = stream.map(|index| format!("-s{index}")).unwrap_or_default();
    format!(
        "{:016x}{stream}-b{PEAKS_BUCKETS_PER_SECOND}.peaks",
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

/// A file with this many frames or fewer gets its filmstrip from one pass
/// through it rather than from seeks.
///
/// Seeking lands on a keyframe and, for the exact frame, walks from there;
/// on long-GOP footage that walk is hundreds of full-size decodes per tile,
/// and twenty-four tiles of it per file is what pinned every core when a
/// folder of clips came in at once (#52). Under this many frames a straight
/// read is cheaper than the seeks would be, and exact. Over it the tiles are
/// too far apart for the walk to be worth it, and each is the keyframe at
/// or before its instant - one decode per tile, near enough for a
/// thumbnail. Three thousand is a hundred seconds at thirty a second: a
/// few seconds of one core for 1080p, and the last length at which a
/// single keyframe could plausibly cover several tiles.
const STRIP_SEQUENTIAL_FRAMES: f64 = 3000.0;

/// Renders a strip of evenly spaced frames from a video as one picture.
///
/// One image rather than N, because the timeline draws the frames as slices
/// of a single texture - that is one texture upload instead of twenty-four.
/// A short file is read once through; a long one is sought tile by tile,
/// to its keyframes - see [`STRIP_SEQUENTIAL_FRAMES`]. If the container
/// reports no duration there is nothing to space frames across, so this
/// refuses rather than guessing.
pub fn filmstrip(path: &str, count: u32, height: u32) -> Result<Frame, String> {
    filmstrip_between(path, 0.0, f64::INFINITY, count, height)
}

/// The same strip, sampled across one stretch of the footage - `from` to
/// `to`, in seconds - rather than the whole of it.
///
/// This is what a clip cut down to a few seconds of a long file tiles its
/// body with: the file's own strip has one frame per twenty-fourth of the
/// footage, and a cut shorter than that is that one frame repeated to the
/// end of the clip. The window is held inside the file, and never shorter
/// than a frame per tile, which is the finest the file can show.
///
/// No tile is ever left black. A tile whose instant would not decode - a
/// seek the index refuses, a duration the container overstated - takes the
/// nearest picture before it, or the first one after when there is none
/// before. The strip was a black canvas the frames were painted onto, and
/// a cut that landed wholly on a hole was a clip with a black body.
pub fn filmstrip_between(
    path: &str,
    from: f64,
    to: f64,
    count: u32,
    height: u32,
) -> Result<Frame, String> {
    let count = count.clamp(1, 60);
    let height = height.clamp(16, 240);

    let info = concat_media::probe(path).map_err(describe)?;
    let video = info.require_video().map_err(describe)?;
    let duration = info
        .duration
        .map(|duration| duration.as_f64())
        .filter(|seconds| *seconds > 0.0)
        .ok_or_else(|| format!("{path} reports no duration"))?;
    let fps = video.frame_rate.fps().as_f64().max(1.0);
    // Aspect-correct and even, which is what the scaler is happiest with.
    let width = ((f64::from(height) * f64::from(video.width) / f64::from(video.height)).round()
        as u32)
        .max(2)
        & !1;

    // The window, inside the file and at least a frame per tile long.
    let least = (f64::from(count) / fps).min(duration);
    let mut from = from.clamp(0.0, duration);
    let mut to = to.clamp(from, duration);
    if to - from < least {
        to = (from + least).min(duration);
        from = (to - least).max(0.0);
    }
    let span = to - from;

    // Sample the middle of each slice, so the first frame is not always the
    // file's own first (often black) frame.
    let instant = |index: usize| from + span * (index as f64 + 0.5) / f64::from(count);
    let mut tiles: Vec<Option<Frame>> = (0..count).map(|_| None).collect();

    if span * fps <= STRIP_SEQUENTIAL_FRAMES {
        read_through(path, from, fps, &instant, width, height, &mut tiles)?;
    } else {
        seek_each(path, &instant, width, height, &mut tiles)?;
        if tiles.iter().all(Option::is_none) {
            // An index that will not seek at all: read the file instead, as
            // far as the tiles reach.
            read_through(path, from, fps, &instant, width, height, &mut tiles)?;
        }
    }

    let first = tiles
        .iter()
        .position(Option::is_some)
        .ok_or_else(|| format!("no frame to show for {path}"))?;
    let mut strip = Frame::black(width * count, height);
    let mut shown = first;
    for index in 0..tiles.len() {
        if tiles[index].is_some() {
            shown = index;
        }
        if let Some(frame) = &tiles[shown] {
            strip.blit(frame, index as u32 * width, 0);
        }
    }
    Ok(strip)
}

/// One pass through the footage from `from`, each tile taking the first
/// frame at or past its instant. Exact, and cheap while the pass is short.
fn read_through(
    path: &str,
    from: f64,
    fps: f64,
    instant: &dyn Fn(usize) -> f64,
    width: u32,
    height: u32,
    tiles: &mut [Option<Frame>],
) -> Result<(), String> {
    let mut options = DecodeOptions::default().scaled_to(width, height);
    if from > 0.0 {
        options = options.starting_at(Rational::approximate(from).unwrap_or(Rational::ZERO));
    }
    let mut decoder = Decoder::open(path, &options).map_err(describe)?;
    let count = tiles.len();
    let mut index = 0;
    let mut produced = 0u64;
    let mut last: Option<Frame> = None;
    while index < count {
        let Ok(Some(frame)) = decoder.next_frame() else {
            break;
        };
        // Where this frame is: its own stamp, or its count when the
        // container stamps nothing.
        let at = decoder
            .position()
            .map(|position| position.as_f64())
            .unwrap_or(from + produced as f64 / fps);
        produced += 1;
        // Every tile whose instant this frame has reached takes it; a
        // file with fewer frames than tiles hands one frame to several.
        while index < count && at >= instant(index) {
            tiles[index] = Some(frame.clone());
            index += 1;
        }
        last = Some(frame);
    }
    // Past the last frame - a duration the container overstated - the last
    // picture stands for what is left; the caller spreads it.
    if index < count
        && let Some(frame) = last
    {
        tiles[index] = Some(frame);
    }
    Ok(())
}

/// One seek per tile, to the keyframe at or before its instant. Near
/// enough for a thumbnail, and one decode each however long the file.
fn seek_each(
    path: &str,
    instant: &dyn Fn(usize) -> f64,
    width: u32,
    height: u32,
    tiles: &mut [Option<Frame>],
) -> Result<(), String> {
    let mut decoder = Decoder::open(
        path,
        &DecodeOptions::default()
            .scaled_to(width, height)
            .nearest_keyframes(),
    )
    .map_err(describe)?;
    for (index, tile) in tiles.iter_mut().enumerate() {
        let time = Rational::approximate(instant(index)).unwrap_or(Rational::ZERO);
        if let Ok(Some(frame)) = decoder.seek(time).and_then(|()| decoder.next_frame()) {
            *tile = Some(frame);
        }
    }
    Ok(())
}

/// The fraction of the footage a cell at `level` spans.
pub fn window_span(level: u32) -> f64 {
    1.0 / f64::from(1u32 << level)
}

/// Where cell `cell` at `level` begins, as a fraction of the footage.
pub fn window_start(level: u32, cell: u32) -> f64 {
    f64::from(cell) / f64::from(1u32 << (level + 1))
}

/// The cell a cut wants its frames sampled across, or `None` when the
/// file's own strip already shows it as more than a few frames.
///
/// `start` and `span` are the cut as fractions of the footage, `duration`
/// the footage in seconds. The level is the finest whose half-cell still
/// holds the cut, so the cut covers at least half the strip it is drawn
/// from and at least twelve of its frames; it stops at the level where a
/// cell is a second long, under which the file has no more pictures to
/// give, and at [`WINDOW_LEVELS`] regardless. Cuts over a quarter of the
/// footage get no cell: the file's strip has six or more frames across them.
pub fn strip_window(start: f64, span: f64, duration: f64) -> Option<(u32, u32)> {
    if span.is_nan() || span <= 0.0 || duration.is_nan() || duration <= 1.0 {
        return None;
    }
    let finest = (duration.log2().floor() as i64).min(i64::from(WINDOW_LEVELS));
    let level = ((1.0 / span).log2().floor() as i64 - 1).min(finest);
    if level < 1 {
        return None;
    }
    let level = level as u32;
    let steps = 1u32 << (level + 1);
    // The last cell is the one that ends at the end of the footage.
    let cell = ((start.clamp(0.0, 1.0) * f64::from(steps)).floor() as u32).min(steps - 2);
    Some((level, cell))
}

/// The finest cell grid: 1/65536 of the footage, where a cut is a frame or
/// two of even the longest file.
pub const WINDOW_LEVELS: u32 = 16;

#[cfg(test)]
mod window_tests {
    use super::*;

    /// The cell a cut is given holds the whole cut, at every level.
    #[test]
    fn a_cut_fits_in_its_cell() {
        let duration = 3600.0;
        let mut span = 0.25;
        while span > 1.0 / 200_000.0 {
            let mut start = 0.0;
            while start + span <= 1.0 {
                let (level, cell) = strip_window(start, span, duration).expect("a cell");
                let from = window_start(level, cell);
                let to = from + window_span(level);
                assert!(
                    from <= start && start + span <= to + 1e-12,
                    "{start} {span} at {level}/{cell}"
                );
                assert!(
                    to <= 1.0 + 1e-12,
                    "cell {level}/{cell} runs past the footage"
                );
                // and, short of the cap, the cut is at least a quarter of
                // it: the cell is the finest whose half still holds the cut
                let finest = duration.log2().floor() as u32;
                assert!(level == finest || span * 4.0 >= window_span(level) - 1e-12);
                start += span * 0.37;
            }
            span *= 0.7;
        }
    }

    #[test]
    fn wide_cuts_and_short_files_keep_the_files_own_strip() {
        assert_eq!(strip_window(0.0, 0.5, 3600.0), None);
        assert_eq!(strip_window(0.1, 0.26, 3600.0), None);
        assert_eq!(strip_window(0.1, 0.01, 0.5), None);
        assert_eq!(strip_window(0.1, 0.0, 3600.0), None);
        assert_eq!(strip_window(0.1, f64::NAN, 3600.0), None);
    }

    #[test]
    fn the_grid_stops_at_a_second_and_at_the_finest_level() {
        // 10s of footage: a cell no shorter than a second is level 3
        assert_eq!(strip_window(0.0, 1e-6, 10.0), Some((3, 0)));
        // and a day of footage stops at WINDOW_LEVELS
        assert_eq!(
            strip_window(0.0, 1e-9, 86400.0).map(|(level, _)| level),
            Some(WINDOW_LEVELS)
        );
    }
}

/// Where a project's poster is cached. Under a name of its own rather than
/// the old `preview.jpg`, because a poster made before the black check
/// below could be a black frame, and this way one is never read back.
pub fn poster_cache(project: &str) -> PathBuf {
    Path::new(project).join("cache").join("poster.jpg")
}

/// A small poster frame for one project, as a JPEG, for the launch screen's
/// recents.
///
/// Grabbed from the earliest clips with a picture on the project's active
/// timeline, and never black: a video's first frame is so often a black
/// leader or the foot of a fade that the launch screen was a column of
/// dead squares. Several moments of each clip are tried, earliest clip
/// first, and the first frame with light in it is the poster; when every
/// one is dark the project has no poster, and the screen shows its film
/// mark instead of a black square. Cached in the project folder, fresh as
/// long as it is newer than the manifest, so an edited project gets a new
/// poster on its next appearance and an untouched one costs a stat.
pub fn poster_frame(project: &str) -> Result<Vec<u8>, String> {
    let root = Path::new(project);
    let manifest = projects::manifest_path(root);
    let cached = poster_cache(project);

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

    let mut failure = None;
    let mut frame = None;
    for (media_path, seconds) in poster_moments(project)? {
        match still_at(&media_path, seconds, 480) {
            Ok(candidate) if !is_dead(&candidate) => {
                frame = Some(candidate);
                break;
            }
            Ok(_) => failure = Some(format!("{media_path} at {seconds:.2}s is black")),
            Err(error) => failure = Some(error),
        }
    }
    let Some(frame) = frame else {
        return Err(failure.unwrap_or_else(|| "nothing on the timeline to preview".to_owned()));
    };
    let bytes = concat_media::jpeg(&frame, 4).map_err(describe)?;

    // Best effort: a failed cache write only means regenerating next launch.
    if let Some(parent) = cached.parent() {
        let _ = std::fs::create_dir_all(parent);
        let _ = std::fs::write(&cached, &bytes);
        // The poster from before the black check, no longer read.
        let _ = std::fs::remove_file(parent.join("preview.jpg"));
    }
    Ok(bytes)
}

/// Below this mean brightness, out of 255, a frame is a black one: a
/// leader, the foot of a fade, a lens cap. A night scene is well above it.
const DEAD_BELOW: u32 = 10;

/// Whether a frame is black, or as near as makes no poster. The mean of
/// each pixel's brightest channel, over every fourth pixel: enough of the
/// picture to tell a leader from a dark scene, and cheap on a 480-wide
/// still.
pub fn is_dead(frame: &Frame) -> bool {
    let pixels = frame.pixels();
    if pixels.len() < 4 {
        return true;
    }
    let mut total = 0u64;
    let mut count = 0u64;
    for pixel in pixels.chunks_exact(4).step_by(4) {
        total += u64::from(pixel[0].max(pixel[1]).max(pixel[2]));
        count += 1;
    }
    count == 0 || total / count < u64::from(DEAD_BELOW)
}

/// The moments to try for a project's poster, in order: for each clip
/// with a picture on the active timeline, earliest first, its in-point,
/// then a second in (or a quarter of the way, on a short clip), then its
/// middle. A still is one moment. Capped, so a timeline of black clips
/// does not cost a launch a hundred decodes.
fn poster_moments(project: &str) -> Result<Vec<(String, f64)>, String> {
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
    let mut clips: Vec<_> = timeline
        .clips
        .iter()
        .filter(|clip| clip.kind == ClipKind::Video || clip.kind == ClipKind::Image)
        .collect();
    clips.sort_by(|a, b| a.start.total_cmp(&b.start));

    const MOST: usize = 12;
    let mut moments = Vec::new();
    for clip in clips {
        let Some(media) = project.media.iter().find(|item| item.id == clip.media_id) else {
            continue;
        };
        if clip.kind == ClipKind::Image {
            moments.push((media.path.clone(), 0.0));
        } else {
            // The source the clip covers, at its speed: the moments are
            // inside what the timeline shows, not past it.
            let span = (clip.duration * clip.speed.max(0.0)).max(0.0);
            let head = clip.source_start.max(0.0);
            let mut at = vec![head];
            if span > 0.0 {
                at.push(head + span.min(4.0) * 0.25);
                at.push(head + span * 0.5);
            }
            at.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
            moments.extend(at.into_iter().map(|seconds| (media.path.clone(), seconds)));
        }
        if moments.len() >= MOST {
            moments.truncate(MOST);
            break;
        }
    }
    if moments.is_empty() {
        return Err("nothing on the timeline to preview".to_owned());
    }
    Ok(moments)
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

    fn probed(
        path: &str,
        duration: Option<concat_core::time::Rational>,
    ) -> concat_media::MediaInfo {
        concat_media::MediaInfo {
            path: std::path::PathBuf::from(path),
            duration,
            video: Some(concat_media::VideoStream {
                index: 0,
                codec: "mjpeg".to_owned(),
                width: 1440,
                height: 1080,
                frame_rate: concat_core::time::FrameRate::from_int(25),
                color_range: None,
            }),
            audio: None,
            audio_streams: Vec::new(),
        }
    }

    /// FFmpeg's `image2` demuxer, which reads JPEGs, states the one frame's
    /// length as the file's duration; a JPEG is still a still. A GIF that
    /// runs for seconds is not.
    #[test]
    fn a_picture_that_lasts_one_frame_is_a_still() {
        use concat_core::time::Rational;
        use concat_project::model::MediaKind;
        assert_eq!(classify(&probed("/a.png", None)), MediaKind::Image);
        assert_eq!(
            classify(&probed("/a.jpg", Some(Rational::new(1, 25)))),
            MediaKind::Image
        );
        assert_eq!(
            classify(&probed("/a.gif", Some(Rational::new(3, 1)))),
            MediaKind::Video
        );
        assert_eq!(
            classify(&probed("/a.mp4", Some(Rational::new(1, 25)))),
            MediaKind::Video
        );
    }

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
            peaks_key("/a.mp4", None),
            format!("{:016x}-b1000.peaks", fnv1a(b"/a.mp4"))
        );
        assert_eq!(
            peaks_key("/a.mp4", Some(2)),
            format!("{:016x}-s2-b1000.peaks", fnv1a(b"/a.mp4"))
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
        // Later slices come from later in the file: the red ramps up, tile
        // by tile - a two-second file is read straight through, so every
        // tile is its own frame even though the file has one keyframe.
        let reds: Vec<u8> = (0..4)
            .map(|tile| strip.pixel(tile * 64 + 32, 16).expect("in bounds")[0])
            .collect();
        assert!(
            reds.windows(2).all(|pair| pair[1] > pair[0]),
            "strip is not in time order: {reds:?}"
        );

        let poster = still_at(&path.to_string_lossy(), 1.0, 32).expect("still");
        assert_eq!(poster.width(), 32);
        let _ = std::fs::remove_file(&path);
    }

    /// A black frame is dead, a dark scene is not, and a frame with a
    /// bright corner on a black ground is not either.
    #[test]
    fn a_dead_frame_is_one_with_no_light_in_it() {
        assert!(is_dead(&Frame::black(64, 32)));
        let mut leader = Frame::black(64, 32);
        leader.fill([6, 6, 6, 255]);
        assert!(is_dead(&leader), "a leader is never quite zero");
        let mut night = Frame::black(64, 32);
        night.fill([0, 0, 24, 255]);
        assert!(!is_dead(&night), "a night scene has light in it");
        let mut corner = Frame::black(64, 32);
        for y in 0..16 {
            for x in 0..32 {
                corner.pixels_mut()[((y * 64 + x) * 4)..((y * 64 + x) * 4 + 3)]
                    .copy_from_slice(&[200, 200, 200]);
            }
        }
        assert!(!is_dead(&corner), "a quarter of the frame lit is a picture");
        assert!(is_dead(&Frame::black(0, 0)), "nothing at all is dead");
    }

    /// A project whose first clip opens on a black leader gets a poster
    /// from further in, not the leader; a project of black alone gets no
    /// poster rather than a black one, and nothing black is cached.
    #[test]
    fn a_poster_is_never_a_black_frame() {
        use concat_core::time::FrameRate;
        use concat_media::{EncodeOptions, Encoder, FrameSink};
        use concat_project::{Command, DocumentSettings, Editor, commands::NewMedia};

        let scratch =
            std::env::temp_dir().join(format!("concat-poster-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch dir");

        // Two seconds black, then two seconds of red.
        let leader = scratch.join("leader.mp4");
        let mut encoder = Encoder::create(
            &leader,
            64,
            32,
            FrameRate::THIRTY,
            &EncodeOptions::default(),
        )
        .expect("encodes");
        for index in 0..120u32 {
            let mut frame = Frame::black(64, 32);
            if index >= 60 {
                frame.fill([220, 30, 30, 255]);
            }
            encoder.write_frame(&frame).expect("writes");
        }
        encoder.finish().expect("finishes");
        // And four seconds of nothing.
        let dark = scratch.join("dark.mp4");
        let mut encoder =
            Encoder::create(&dark, 64, 32, FrameRate::THIRTY, &EncodeOptions::default())
                .expect("encodes");
        for _ in 0..120u32 {
            encoder.write_frame(&Frame::black(64, 32)).expect("writes");
        }
        encoder.finish().expect("finishes");

        let project_with = |name: &str, file: &Path| -> String {
            let mut editor = Editor::new();
            let media_id = editor
                .apply(Command::AddMedia {
                    item: NewMedia {
                        path: file.to_string_lossy().into_owned(),
                        name: "clip.mp4".to_owned(),
                        duration: Some(4.0),
                        kind: concat_project::model::MediaKind::Video,
                        width: Some(64),
                        height: Some(32),
                        frame_rate: Some(30.0),
                        frame_rate_fraction: Some("30/1".to_owned()),
                        video_codec: Some("h264".to_owned()),
                        audio_codec: None,
                        has_audio: false,
                        audio_tracks: Vec::new(),
                        origin: None,
                    },
                })
                .expect("adds")
                .created_id
                .expect("id");
            editor
                .apply(Command::AddClipAtFirstFree {
                    media_id,
                    start: 0.0,
                })
                .expect("places");
            let settings = DocumentSettings {
                name: name.to_owned(),
                width: 64,
                height: 32,
                rate_num: 30,
                rate_den: 1,
            };
            let document = concat_project::to_document(&settings, editor.project());
            let root = scratch.join(name);
            projects::save(&root.to_string_lossy(), &document).expect("saves");
            root.to_string_lossy().into_owned()
        };

        let lit = project_with("lit", &leader);
        let bytes = poster_frame(&lit).expect("a poster from past the leader");
        let poster = still_at(&poster_cache(&lit).to_string_lossy(), 0.0, 64).expect("reads back");
        assert!(!is_dead(&poster), "the cached poster has light in it");
        let middle = poster.pixel(32, 16).expect("in bounds");
        assert!(
            middle[0] > 150 && middle[1] < 90,
            "red, from past the leader: {middle:?}"
        );
        assert!(!bytes.is_empty());

        let black = project_with("black", &dark);
        let refused = poster_frame(&black).expect_err("no poster for a black project");
        assert!(refused.contains("black"), "{refused}");
        assert!(
            !poster_cache(&black).is_file(),
            "and nothing black was cached"
        );

        let _ = std::fs::remove_dir_all(&scratch);
    }
}

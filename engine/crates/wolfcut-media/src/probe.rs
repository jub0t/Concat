//! Asking `ffprobe` what is inside a file.

use std::path::{Path, PathBuf};
use std::process::Command;

use wolfcut_core::time::{FrameRate, Rational};
use serde_json::Value;

use crate::error::{Error, Result};

/// Name used in error messages; see [`crate::binaries::ffprobe`].
const FFPROBE: &str = "ffprobe";

/// What a video stream looks like.
#[derive(Clone, Debug)]
pub struct VideoStream {
    /// Stream index within the file.
    pub index: u32,
    /// Codec short name, for example `h264`.
    pub codec: String,
    /// Coded width in pixels.
    pub width: u32,
    /// Coded height in pixels.
    pub height: u32,
    /// Average frame rate, exact.
    pub frame_rate: FrameRate,
}

/// What an audio stream looks like.
#[derive(Clone, Debug)]
pub struct AudioStream {
    /// Stream index within the file.
    pub index: u32,
    /// Codec short name, for example `aac`.
    pub codec: String,
    /// Samples per second.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u32,
}

/// A summary of one media file.
///
/// WolfCut only cares about the first video and first audio stream. Multi-stream
/// files exist, but nothing in the editor addresses them yet, and inventing an
/// API for a case we do not handle would be worse than not having one.
#[derive(Clone, Debug)]
pub struct MediaInfo {
    /// The file this describes.
    pub path: PathBuf,
    /// Container duration, when the container bothers to state one.
    pub duration: Option<Rational>,
    /// First video stream, if any.
    pub video: Option<VideoStream>,
    /// First audio stream, if any.
    pub audio: Option<AudioStream>,
}

impl MediaInfo {
    /// The video stream, or [`Error::NoVideoStream`] if the file has none.
    pub fn require_video(&self) -> Result<&VideoStream> {
        self.video.as_ref().ok_or_else(|| Error::NoVideoStream { path: self.path.clone() })
    }
}

/// Runs `ffprobe` and summarises what it found.
pub fn probe(path: impl AsRef<Path>) -> Result<MediaInfo> {
    let path = path.as_ref();
    let output = Command::new(crate::binaries::ffprobe())
        .args(["-v", "error", "-show_streams", "-show_format", "-of", "json"])
        .arg(path)
        .output()
        .map_err(|source| Error::Spawn { program: FFPROBE, source })?;

    if !output.status.success() {
        return Err(Error::Exited {
            program: FFPROBE,
            path: path.to_path_buf(),
            status: output.status,
            stderr: crate::process::summarize(&output.stderr),
        });
    }

    let root: Value = serde_json::from_slice(&output.stdout).map_err(|error| Error::Probe {
        path: path.to_path_buf(),
        detail: format!("output was not valid json: {error}"),
    })?;

    let streams = root.get("streams").and_then(Value::as_array).ok_or_else(|| Error::Probe {
        path: path.to_path_buf(),
        detail: "no `streams` array".to_owned(),
    })?;

    Ok(MediaInfo {
        path: path.to_path_buf(),
        duration: root
            .get("format")
            .and_then(|format| format.get("duration"))
            .and_then(Value::as_str)
            .and_then(Rational::parse),
        video: streams.iter().find_map(|stream| parse_video(stream, path)).transpose()?,
        audio: streams.iter().find_map(parse_audio),
    })
}

/// Returns `None` for non-video streams, `Some(Err(..))` for a video stream we
/// cannot make sense of. Skipping a malformed video stream silently would show
/// up much later as a blank preview.
fn parse_video(stream: &Value, path: &Path) -> Option<Result<VideoStream>> {
    if stream.get("codec_type").and_then(Value::as_str) != Some("video") {
        return None;
    }

    let missing = |field: &str| Error::Probe {
        path: path.to_path_buf(),
        detail: format!("video stream has no usable `{field}`"),
    };

    Some((|| {
        let mut width = u32_field(stream, "width").ok_or_else(|| missing("width"))?;
        let mut height = u32_field(stream, "height").ok_or_else(|| missing("height"))?;

        // Phones write a sideways-coded frame plus a Display Matrix (or the
        // older `rotate` tag) telling players to turn it 90/270 on playback.
        // Swap to display dimensions now so nothing downstream has to know
        // about container rotation to get the aspect ratio right.
        if matches!(rotation_field(stream), 90 | 270) {
            std::mem::swap(&mut width, &mut height);
        }

        // `avg_frame_rate` is 0/0 for streams with no constant rate, in which
        // case `r_frame_rate` carries the best guess FFmpeg has.
        let frame_rate = ["avg_frame_rate", "r_frame_rate"]
            .iter()
            .filter_map(|field| stream.get(field).and_then(Value::as_str))
            .find_map(Rational::parse)
            .filter(|rate| !rate.is_zero() && !rate.is_negative())
            .ok_or_else(|| missing("frame rate"))?;

        Ok(VideoStream {
            index: u32_field(stream, "index").unwrap_or(0),
            codec: string_field(stream, "codec_name"),
            width,
            height,
            frame_rate: FrameRate::new(frame_rate),
        })
    })())
}

/// Normalised rotation in degrees clockwise (0, 90, 180 or 270) from the
/// stream's Display Matrix side data, falling back to the older `rotate` tag
/// some containers use instead.
fn rotation_field(stream: &Value) -> i32 {
    let raw = stream
        .get("side_data_list")
        .and_then(Value::as_array)
        .and_then(|entries| entries.iter().find_map(|entry| entry.get("rotation")))
        .and_then(Value::as_f64)
        .map(|degrees| degrees.round() as i32)
        .or_else(|| {
            stream
                .get("tags")
                .and_then(|tags| tags.get("rotate"))
                .and_then(Value::as_str)
                .and_then(|text| text.parse().ok())
        })
        .unwrap_or(0);
    ((raw % 360) + 360) % 360
}

fn parse_audio(stream: &Value) -> Option<AudioStream> {
    if stream.get("codec_type").and_then(Value::as_str) != Some("audio") {
        return None;
    }
    Some(AudioStream {
        index: u32_field(stream, "index").unwrap_or(0),
        codec: string_field(stream, "codec_name"),
        sample_rate: u32_field(stream, "sample_rate").unwrap_or(0),
        channels: u32_field(stream, "channels").unwrap_or(0),
    })
}

/// ffprobe is inconsistent about quoting numbers, so accept both forms.
fn u32_field(stream: &Value, field: &str) -> Option<u32> {
    let value = stream.get(field)?;
    match value {
        Value::Number(number) => number.as_u64().and_then(|n| u32::try_from(n).ok()),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn string_field(stream: &Value, field: &str) -> String {
    stream.get(field).and_then(Value::as_str).unwrap_or("unknown").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(text: &str) -> Value {
        serde_json::from_str(text).expect("test json is valid")
    }

    #[test]
    fn reads_a_video_stream() {
        let stream = json(
            r#"{"codec_type":"video","index":0,"codec_name":"h264",
                "width":1920,"height":1080,"avg_frame_rate":"30000/1001"}"#,
        );
        let video = parse_video(&stream, Path::new("a.mp4")).expect("is video").expect("parses");
        assert_eq!((video.width, video.height), (1920, 1080));
        assert_eq!(video.frame_rate, FrameRate::NTSC_30);
        assert_eq!(video.codec, "h264");
    }

    #[test]
    fn falls_back_when_avg_frame_rate_is_zero() {
        let stream = json(
            r#"{"codec_type":"video","width":640,"height":480,
                "avg_frame_rate":"0/0","r_frame_rate":"25/1"}"#,
        );
        let video = parse_video(&stream, Path::new("a.mp4")).expect("is video").expect("parses");
        assert_eq!(video.frame_rate, FrameRate::PAL);
    }

    #[test]
    fn accepts_numbers_quoted_or_not() {
        let quoted = json(r#"{"width":"640"}"#);
        let bare = json(r#"{"width":640}"#);
        assert_eq!(u32_field(&quoted, "width"), Some(640));
        assert_eq!(u32_field(&bare, "width"), Some(640));
    }

    #[test]
    fn a_video_stream_with_no_usable_rate_is_an_error_not_a_skip() {
        let stream = json(r#"{"codec_type":"video","width":64,"height":64,"avg_frame_rate":"0/0"}"#);
        let result = parse_video(&stream, Path::new("a.mp4")).expect("is video");
        assert!(matches!(result, Err(Error::Probe { .. })));
    }

    #[test]
    fn swaps_dimensions_for_a_sideways_coded_phone_video() {
        let stream = json(
            r#"{"codec_type":"video","width":1920,"height":1080,"avg_frame_rate":"30/1",
                "side_data_list":[{"side_data_type":"Display Matrix","rotation":-90}]}"#,
        );
        let video = parse_video(&stream, Path::new("a.mp4")).expect("is video").expect("parses");
        assert_eq!((video.width, video.height), (1080, 1920));
    }

    #[test]
    fn keeps_dimensions_for_an_upright_or_upside_down_video() {
        let stream = json(
            r#"{"codec_type":"video","width":1920,"height":1080,"avg_frame_rate":"30/1",
                "side_data_list":[{"side_data_type":"Display Matrix","rotation":180}]}"#,
        );
        let video = parse_video(&stream, Path::new("a.mp4")).expect("is video").expect("parses");
        assert_eq!((video.width, video.height), (1920, 1080));
    }

    #[test]
    fn falls_back_to_the_legacy_rotate_tag() {
        let stream = json(
            r#"{"codec_type":"video","width":1920,"height":1080,"avg_frame_rate":"30/1",
                "tags":{"rotate":"90"}}"#,
        );
        let video = parse_video(&stream, Path::new("a.mp4")).expect("is video").expect("parses");
        assert_eq!((video.width, video.height), (1080, 1920));
    }

    #[test]
    fn ignores_non_video_streams() {
        assert!(parse_video(&json(r#"{"codec_type":"audio"}"#), Path::new("a.mp4")).is_none());
    }

    #[test]
    fn reads_an_audio_stream() {
        let stream = json(
            r#"{"codec_type":"audio","index":1,"codec_name":"aac",
                "sample_rate":"48000","channels":2}"#,
        );
        let audio = parse_audio(&stream).expect("is audio");
        assert_eq!((audio.sample_rate, audio.channels), (48000, 2));
        assert_eq!(audio.codec, "aac");
    }
}

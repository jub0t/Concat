// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Asking `ffprobe` what is inside a file.

use std::path::{Path, PathBuf};

use concat_core::time::{FrameRate, Rational};
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
    /// Displayed width in pixels: the coded width, swapped with the height
    /// when the stream carries a quarter-turn display rotation.
    pub width: u32,
    /// Displayed height in pixels; see [`VideoStream::width`].
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
/// Concat only cares about the first video and first audio stream. Multi-stream
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
    let output = crate::process::command(crate::binaries::ffprobe())
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
        let width = u32_field(stream, "width").ok_or_else(|| missing("width"))?;
        let height = u32_field(stream, "height").ok_or_else(|| missing("height"))?;

        // Portrait phone footage is commonly coded sideways, with a display
        // rotation every player applies before showing the frame - as does
        // our own decoder, which leaves ffmpeg's autorotate on. What the rest
        // of the app must see are the displayed dimensions, so a quarter turn
        // swaps them here; ignoring it treats a portrait clip as landscape.
        let (width, height) = if display_rotation(stream).rem_euclid(180) == 90 {
            (height, width)
        } else {
            (width, height)
        };

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

/// Display rotation in degrees: a Display Matrix in the stream's side data
/// (how phones record portrait video), or the legacy `rotate` tag older
/// files carry. Zero when neither says anything.
fn display_rotation(stream: &Value) -> i64 {
    stream
        .get("side_data_list")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|entry| i64_field(entry, "rotation"))
        .or_else(|| stream.get("tags").and_then(|tags| i64_field(tags, "rotate")))
        .unwrap_or(0)
}

/// Like [`u32_field`], but signed, and tolerant of the float form some
/// ffprobe builds use for the display matrix angle.
fn i64_field(value: &Value, field: &str) -> Option<i64> {
    match value.get(field)? {
        Value::Number(number) => {
            number.as_i64().or_else(|| number.as_f64().map(|angle| angle.round() as i64))
        }
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
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
    fn a_quarter_turn_display_rotation_swaps_the_dimensions() {
        // iPhone portrait HEVC: coded sideways, display matrix says -90.
        let stream = json(
            r#"{"codec_type":"video","width":1920,"height":1080,"avg_frame_rate":"30/1",
                "side_data_list":[{"side_data_type":"Display Matrix","rotation":-90}]}"#,
        );
        let video = parse_video(&stream, Path::new("a.mp4")).expect("is video").expect("parses");
        assert_eq!((video.width, video.height), (1080, 1920));
    }

    #[test]
    fn a_half_turn_rotation_keeps_the_dimensions() {
        let stream = json(
            r#"{"codec_type":"video","width":1920,"height":1080,"avg_frame_rate":"30/1",
                "side_data_list":[{"side_data_type":"Display Matrix","rotation":180}]}"#,
        );
        let video = parse_video(&stream, Path::new("a.mp4")).expect("is video").expect("parses");
        assert_eq!((video.width, video.height), (1920, 1080));
    }

    #[test]
    fn the_legacy_rotate_tag_counts_too() {
        let stream = json(
            r#"{"codec_type":"video","width":640,"height":480,"avg_frame_rate":"30/1",
                "tags":{"rotate":"90"}}"#,
        );
        let video = parse_video(&stream, Path::new("a.mp4")).expect("is video").expect("parses");
        assert_eq!((video.width, video.height), (480, 640));
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

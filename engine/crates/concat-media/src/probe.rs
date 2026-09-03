// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Asking libavformat what is inside a file.

use std::path::{Path, PathBuf};

use concat_core::time::{FrameRate, Rational};
use ffmpeg_the_third as ffmpeg;

use crate::error::{Error, Result};
use crate::ffi;

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
        self.video.as_ref().ok_or_else(|| Error::NoVideoStream {
            path: self.path.clone(),
        })
    }
}

/// Opens the file and summarises what it found.
pub fn probe(path: impl AsRef<Path>) -> Result<MediaInfo> {
    ffi::init();
    let path = path.as_ref();
    let input = ffmpeg::format::input(path).map_err(|error| ffi::fail("open", path, error))?;

    let duration = input.duration();
    let duration =
        (duration > 0).then(|| Rational::new(duration, i64::from(ffmpeg::sys::AV_TIME_BASE)));

    let video = match input.streams().best(ffmpeg::media::Type::Video) {
        Some(stream) => Some(video_stream(&stream, path)?),
        None => None,
    };
    let audio = input
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .map(|stream| {
            let parameters = stream.parameters();
            AudioStream {
                index: stream.index() as u32,
                codec: parameters.id().name().to_owned(),
                sample_rate: parameters.sample_rate(),
                channels: parameters.ch_layout().channels(),
            }
        });

    Ok(MediaInfo {
        path: path.to_path_buf(),
        duration,
        video,
        audio,
    })
}

fn video_stream(stream: &ffmpeg::format::stream::Stream<'_>, path: &Path) -> Result<VideoStream> {
    let parameters = stream.parameters();
    let (width, height) = (parameters.width(), parameters.height());
    if width == 0 || height == 0 {
        return Err(Error::Probe {
            path: path.to_path_buf(),
            detail: "video stream has no usable size".to_owned(),
        });
    }
    let (width, height) = ffi::displayed(width, height, ffi::rotation(stream));

    let frame_rate = pick_rate(rational(stream.avg_frame_rate()), rational(stream.rate()))
        .ok_or_else(|| Error::Probe {
            path: path.to_path_buf(),
            detail: "video stream has no usable frame rate".to_owned(),
        })?;

    Ok(VideoStream {
        index: stream.index() as u32,
        codec: parameters.id().name().to_owned(),
        width,
        height,
        frame_rate: FrameRate::new(frame_rate),
    })
}

fn rational(value: ffmpeg::Rational) -> Option<Rational> {
    (value.denominator() != 0)
        .then(|| Rational::new(i64::from(value.numerator()), i64::from(value.denominator())))
}

/// `avg_frame_rate` is 0/0 for streams with no constant rate, in which case
/// `r_frame_rate` carries the best guess FFmpeg has.
fn pick_rate(average: Option<Rational>, guess: Option<Rational>) -> Option<Rational> {
    [average, guess]
        .into_iter()
        .flatten()
        .find(|rate| !rate.is_zero() && !rate.is_negative())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_when_avg_frame_rate_is_zero() {
        let zero = Some(Rational::ZERO);
        let pal = Some(Rational::from_int(25));
        assert_eq!(pick_rate(zero, pal), pal);
        assert_eq!(pick_rate(None, pal), pal);
        assert_eq!(
            pick_rate(Some(Rational::new(30000, 1001)), pal),
            Some(Rational::new(30000, 1001))
        );
    }

    #[test]
    fn no_usable_rate_is_none() {
        assert_eq!(pick_rate(Some(Rational::ZERO), Some(Rational::ZERO)), None);
        assert_eq!(pick_rate(None, None), None);
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        assert!(probe("does-not-exist.mp4").is_err());
    }
}

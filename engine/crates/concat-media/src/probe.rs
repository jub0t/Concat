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
    /// Stream index within the file - what a clip names to play this one.
    pub index: u32,
    /// Codec short name, for example `aac`.
    pub codec: String,
    /// Samples per second.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u32,
    /// What the file calls the stream, when it says: the `title` a recorder
    /// writes per track ("Desktop Audio", "Mic/Aux"). Empty when it does not.
    pub title: String,
    /// The stream's language tag, for example `eng`; empty when unstated.
    pub language: String,
}

/// A summary of one media file.
///
/// One video stream is all the editor addresses. Audio streams are listed in
/// full: a screen recording often carries the desktop's sound and the
/// microphone as two streams, and which one a clip plays is the clip's to
/// say. `audio` is the first of them in file order - the one a clip plays
/// unless it names another - and not libavformat's "best", which weighs
/// bitrate and frame counts and on such a recording tends to land on the
/// second track, so the microphone played and the desktop was never heard.
#[derive(Clone, Debug)]
pub struct MediaInfo {
    /// The file this describes.
    pub path: PathBuf,
    /// Container duration, when the container bothers to state one.
    pub duration: Option<Rational>,
    /// First video stream, if any.
    pub video: Option<VideoStream>,
    /// The first audio stream in file order, if any: the default.
    pub audio: Option<AudioStream>,
    /// Every audio stream, in file order. Empty for a file without sound.
    pub audio_streams: Vec<AudioStream>,
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
    let audio_streams: Vec<AudioStream> = input
        .streams()
        .filter(is_audio)
        .map(|stream| {
            let parameters = stream.parameters();
            let tag = |name: &str| {
                stream
                    .metadata()
                    .get(name)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_default()
            };
            AudioStream {
                index: stream.index() as u32,
                codec: parameters.id().name().to_owned(),
                sample_rate: parameters.sample_rate(),
                channels: parameters.ch_layout().channels(),
                title: tag("title"),
                language: tag("language"),
            }
        })
        .collect();
    let audio = audio_streams.first().cloned();

    Ok(MediaInfo {
        path: path.to_path_buf(),
        duration,
        video,
        audio,
        audio_streams,
    })
}

fn is_audio(stream: &ffmpeg::format::stream::Stream<'_>) -> bool {
    stream.parameters().medium() == ffmpeg::media::Type::Audio
}

/// The audio stream a caller means: `wanted`, when it names an audio stream
/// the file has, else the first audio stream in file order. `None` for a
/// file with no sound at all.
///
/// Every reader of samples picks its stream here - the waveform, playback,
/// the mix, the transcriber - so a clip that names a track hears the same
/// one everywhere, and a clip that names none hears the same default the
/// probe reported as `audio`. A named stream the file no longer has (the
/// file was replaced) degrades to that default rather than to silence.
pub(crate) fn audio_stream_index(
    input: &ffmpeg::format::context::Input,
    wanted: Option<usize>,
) -> Option<usize> {
    if let Some(index) = wanted
        && input.stream(index).as_ref().is_some_and(is_audio)
    {
        return Some(index);
    }
    input.streams().find(is_audio).map(|stream| stream.index())
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

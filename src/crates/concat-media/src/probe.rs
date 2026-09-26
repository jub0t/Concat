// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Asking libavformat what is inside a file.

use std::path::{Path, PathBuf};

use concat_core::frame::Signal;
use concat_core::time::{FrameRate, Rational};
use ffmpeg_the_third as ffmpeg;

use crate::decode::ColorRange;
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
    /// The levels the stream says its numbers span, or `None` where it
    /// says nothing - which a player then takes for video range. What a
    /// person compares the picture against when it looks washed out.
    pub color_range: Option<ColorRange>,
    /// What the stream's tags say its picture is: HLG or PQ by its
    /// transfer, a wide SDR by its primaries, SDR where they say nothing.
    pub signal: Signal,
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

    // SAFETY: the parameters are live for as long as `stream` is, and the
    // range, the transfer and the primaries are plain fields libavformat
    // filled from the container.
    let (range, transfer, primaries) = unsafe {
        let raw = &*parameters.as_ptr();
        (raw.color_range, raw.color_trc, raw.color_primaries)
    };
    let signal = {
        use ffmpeg::color::{Primaries, TransferCharacteristic as Transfer};
        match (Transfer::from(transfer), Primaries::from(primaries)) {
            (Transfer::ARIB_STD_B67, _) => Signal::Hlg,
            (Transfer::SMPTE2084, _) => Signal::Pq,
            (_, Primaries::BT2020) => Signal::SdrWide,
            _ => Signal::Sdr,
        }
    };
    Ok(VideoStream {
        index: stream.index() as u32,
        codec: parameters.id().name().to_owned(),
        width,
        height,
        frame_rate: FrameRate::new(frame_rate),
        color_range: ColorRange::from_ffmpeg(ffmpeg::color::Range::from(range)),
        signal,
    })
}

fn rational(value: ffmpeg::Rational) -> Option<Rational> {
    (value.denominator() != 0)
        .then(|| Rational::new(i64::from(value.numerator()), i64::from(value.denominator())))
}

/// The rate a stream's frames are indexed by.
///
/// `avg_frame_rate` is the frame count over the duration, and on a short
/// or oddly-closed file it is off by a fraction - ninety frames over
/// eighty-nine thirtieths is 30.34 - which puts every frame on a boundary
/// a third of a frame early and leaves holes where an index is skipped.
/// `r_frame_rate` is the rate the timestamps are on. So a constant-rate
/// stream, where the two agree within two percent, takes the base rate
/// exactly; a stream with no constant rate takes the average, since its
/// base rate is whatever the timestamps' common denominator happens to
/// be and can be absurd. `avg_frame_rate` is 0/0 for such a stream with
/// no average either, in which case the base rate is the best guess left.
fn pick_rate(average: Option<Rational>, guess: Option<Rational>) -> Option<Rational> {
    let usable =
        |rate: &Option<Rational>| rate.filter(|rate| !rate.is_zero() && !rate.is_negative());
    match (usable(&average), usable(&guess)) {
        (Some(average), Some(base)) => {
            let (a, b) = (average.as_f64(), base.as_f64());
            let close = ((a - b) / b).abs() <= 0.02;
            Some(if close { base } else { average })
        }
        (Some(average), None) => Some(average),
        (None, base) => base,
    }
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

    /// A short constant-rate file whose average is skewed by its length
    /// takes the base rate exactly; a stream whose average is nothing
    /// like its base rate is variable, and keeps the average.
    #[test]
    fn a_constant_rate_stream_takes_its_base_rate_exactly() {
        let skewed = Some(Rational::new(2700, 89));
        let thirty = Some(Rational::from_int(30));
        assert_eq!(pick_rate(skewed, thirty), thirty);
        let ntsc = Some(Rational::new(30000, 1001));
        assert_eq!(pick_rate(Some(Rational::new(29970, 1000)), ntsc), ntsc);
        // Variable: the base rate is the timestamps' denominator, not a rate.
        let variable = Some(Rational::new(24, 1));
        assert_eq!(
            pick_rate(variable, Some(Rational::from_int(1000))),
            variable
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

    /// The probe reads what a file's tags say its picture is: HLG, PQ, or
    /// SDR where it is BT.709 - the fact a timeline turns HDR by.
    #[test]
    fn the_probe_reads_a_files_hdr_tags() {
        use crate::encode::{EncodeOptions, Encoder, FrameSink, RateMode, VideoCodec};
        use concat_core::frame::Frame;
        let dir = std::env::temp_dir().join(format!("concat-probe-hdr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        let options = EncodeOptions {
            codec: VideoCodec::Hevc,
            preset: "ultrafast".to_owned(),
            crf: 28,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 0,
            ten_bit: false,
            color_range: crate::ColorRange::Limited,
            hardware: false,
            threads: 0,
        };
        let rate = FrameRate::from_int(25);
        for (name, hdr, want) in [
            ("sdr.mp4", None, Signal::Sdr),
            ("hlg.mp4", Some(false), Signal::Hlg),
            ("pq.mp4", Some(true), Signal::Pq),
        ] {
            let path = dir.join(name);
            let mut encoder = match hdr {
                None => Encoder::create(&path, 64, 64, rate, &options),
                Some(pq) => Encoder::create_mislabelled_hdr(&path, 64, 64, rate, &options, pq),
            }
            .expect("encodes");
            for _ in 0..3 {
                encoder.write_frame(&Frame::black(64, 64)).expect("writes");
            }
            encoder.finish().expect("finishes");
            let info = probe(&path).expect("probes");
            assert_eq!(info.video.expect("a picture").signal, want, "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

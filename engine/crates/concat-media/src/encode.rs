// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Writing RGBA frames back out to a file.

use std::path::{Path, PathBuf};

use concat_core::frame::Frame;
use concat_core::time::FrameRate;
use ffmpeg_the_third as ffmpeg;
use ffmpeg_the_third::codec::encoder;
use ffmpeg_the_third::format::{self, Pixel};
use ffmpeg_the_third::software::scaling;
use ffmpeg_the_third::util::frame::video::Video;

use crate::error::{Error, Result};
use crate::ffi;

/// Anything that accepts finished frames.
///
/// The mirror of [`FrameSource`](crate::decode::FrameSource): render code
/// writes to this trait, so an export target, a preview window and a test spy
/// are interchangeable.
pub trait FrameSink {
    /// Accepts one frame. Frames must all be the size the sink was opened with.
    fn write_frame(&mut self, frame: &Frame) -> Result<()>;

    /// Flushes and closes. Always call this - a dropped sink produces a
    /// truncated file, because the encoder never got to write its trailer.
    fn finish(&mut self) -> Result<()>;
}

/// Encoder settings.
#[derive(Clone, Debug)]
pub struct EncodeOptions {
    /// Video codec, as FFmpeg names it.
    pub codec: String,
    /// x264-style speed/size tradeoff.
    pub preset: String,
    /// Constant rate factor. Lower is better quality and a bigger file.
    pub crf: u8,
    /// Output pixel format. `yuv420p` is what players actually accept.
    pub pixel_format: String,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            codec: "libx264".to_owned(),
            preset: "medium".to_owned(),
            crf: 18,
            pixel_format: "yuv420p".to_owned(),
        }
    }
}

/// The output pixel formats an encoder can be asked for, by FFmpeg's names.
fn pixel_format(name: &str) -> Option<Pixel> {
    Some(match name {
        "yuv420p" => Pixel::YUV420P,
        "yuv422p" => Pixel::YUV422P,
        "yuv444p" => Pixel::YUV444P,
        "nv12" => Pixel::NV12,
        "rgba" => Pixel::RGBA,
        _ => return None,
    })
}

/// Encodes through libavcodec into a container libavformat writes.
pub struct Encoder {
    path: PathBuf,
    output: format::context::Output,
    encoder: encoder::video::Encoder,
    scaler: scaling::Context,
    /// The encoder's time base, and the stream's after the header was
    /// written - the muxer is free to pick its own.
    encoder_time_base: ffmpeg::Rational,
    stream_time_base: ffmpeg::Rational,
    width: u32,
    height: u32,
    written: u64,
    finished: bool,
}

impl Encoder {
    /// Opens `path` for writing, overwriting anything already there.
    pub fn create(
        path: impl AsRef<Path>,
        width: u32,
        height: u32,
        frame_rate: FrameRate,
        options: &EncodeOptions,
    ) -> Result<Self> {
        ffi::init();
        let path = path.as_ref();
        let fps = frame_rate.fps();
        let rate = ffmpeg::Rational::new(fps.numerator() as i32, fps.denominator() as i32);
        let time_base = rate.invert();

        let codec = encoder::find_by_name(&options.codec).ok_or_else(|| Error::Missing {
            what: "encoder",
            name: options.codec.clone(),
        })?;
        let pixel_format = pixel_format(&options.pixel_format).ok_or_else(|| Error::Missing {
            what: "pixel format",
            name: options.pixel_format.clone(),
        })?;

        let mut output =
            ffmpeg::format::output(path).map_err(|error| ffi::fail("create", path, error))?;
        let global_header = output
            .format()
            .flags()
            .contains(format::Flags::GLOBAL_HEADER);

        let mut video = ffmpeg::codec::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(|error| ffi::fail("video encoder", path, error))?;
        video.set_width(width);
        video.set_height(height);
        video.set_format(pixel_format);
        video.set_time_base(time_base);
        video.set_frame_rate(Some(rate));
        if global_header {
            video.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        }
        let encoder = video
            .open_with(ffmpeg::dict! {
                "preset" => options.preset.as_str(),
                "crf" => &options.crf.to_string(),
            })
            .map_err(|error| ffi::fail("open encoder", path, error))?;

        {
            let mut stream = output
                .add_stream(codec)
                .map_err(|error| ffi::fail("add stream", path, error))?;
            stream.copy_parameters_from_context(&encoder);
            stream.set_time_base(time_base);
        }
        output
            .write_header_with(ffmpeg::dict! { "movflags" => "+faststart" })
            .map_err(|error| ffi::fail("write header", path, error))?;
        let stream_time_base = output
            .stream(0)
            .map(|stream| stream.time_base())
            .unwrap_or(time_base);

        let scaler = scaling::Context::get(
            Pixel::RGBA,
            width,
            height,
            pixel_format,
            width,
            height,
            scaling::Flags::BILINEAR,
        )
        .map_err(|error| ffi::fail("scaler", path, error))?;

        Ok(Self {
            path: path.to_path_buf(),
            output,
            encoder,
            scaler,
            encoder_time_base: time_base,
            stream_time_base,
            width,
            height,
            written: 0,
            finished: false,
        })
    }

    /// How many frames have been accepted so far.
    pub const fn written(&self) -> u64 {
        self.written
    }

    /// Writes every packet the encoder has ready.
    fn drain(&mut self) -> Result<()> {
        loop {
            let mut packet = ffmpeg::Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(0);
                    packet.rescale_ts(self.encoder_time_base, self.stream_time_base);
                    packet
                        .write_interleaved(&mut self.output)
                        .map_err(|error| ffi::fail("write packet", &self.path, error))?;
                }
                Err(ffmpeg::Error::Eof) => return Ok(()),
                Err(error) if ffi::is_again(&error) => return Ok(()),
                Err(error) => return Err(ffi::fail("encode", &self.path, error)),
            }
        }
    }
}

impl FrameSink for Encoder {
    fn write_frame(&mut self, frame: &Frame) -> Result<()> {
        if frame.width() != self.width || frame.height() != self.height {
            return Err(Error::FrameSizeMismatch {
                want_width: self.width,
                want_height: self.height,
                got_width: frame.width(),
                got_height: frame.height(),
            });
        }
        if self.finished {
            return Err(Error::Io {
                path: self.path.clone(),
                source: std::io::Error::other("encoder was already finished"),
            });
        }

        let mut rgba = Video::new(Pixel::RGBA, self.width, self.height);
        {
            let stride = rgba.stride(0);
            let row = self.width as usize * 4;
            let data = rgba.data_mut(0);
            for (y, source) in frame.pixels().chunks_exact(row).enumerate() {
                data[y * stride..y * stride + row].copy_from_slice(source);
            }
        }
        let mut converted = Video::empty();
        self.scaler
            .run(&rgba, &mut converted)
            .map_err(|error| ffi::fail("convert", &self.path, error))?;
        converted.set_pts(Some(self.written as i64));

        self.encoder
            .send_frame(&converted)
            .map_err(|error| ffi::fail("encode", &self.path, error))?;
        self.written += 1;
        self.drain()
    }

    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.encoder
            .send_eof()
            .map_err(|error| ffi::fail("encode", &self.path, error))?;
        self.drain()?;
        self.output
            .write_trailer()
            .map_err(|error| ffi::fail("write trailer", &self.path, error))
    }
}

/// Encodes one frame as a JPEG, for posters and thumbnails written to disk.
///
/// `quality` is the JPEG quantiser scale, 2 (best) to 31 (worst) - the
/// `-q:v` of the command line.
pub fn jpeg(frame: &Frame, quality: u8) -> Result<Vec<u8>> {
    ffi::init();
    let path = Path::new("<jpeg>");
    let codec = encoder::find_by_name("mjpeg").ok_or_else(|| Error::Missing {
        what: "encoder",
        name: "mjpeg".to_owned(),
    })?;
    let mut video = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|error| ffi::fail("jpeg encoder", path, error))?;
    video.set_width(frame.width());
    video.set_height(frame.height());
    video.set_format(Pixel::YUVJ420P);
    video.set_time_base(ffmpeg::Rational::new(1, 25));
    let quality = i32::from(quality.clamp(2, 31));
    video.set_qmin(quality);
    video.set_qmax(quality);
    let mut encoder = video
        .open()
        .map_err(|error| ffi::fail("open jpeg encoder", path, error))?;

    let mut rgba = Video::new(Pixel::RGBA, frame.width(), frame.height());
    {
        let stride = rgba.stride(0);
        let row = frame.width() as usize * 4;
        let data = rgba.data_mut(0);
        for (y, source) in frame.pixels().chunks_exact(row).enumerate() {
            data[y * stride..y * stride + row].copy_from_slice(source);
        }
    }
    let mut scaler = scaling::Context::get(
        Pixel::RGBA,
        frame.width(),
        frame.height(),
        Pixel::YUVJ420P,
        frame.width(),
        frame.height(),
        scaling::Flags::BILINEAR,
    )
    .map_err(|error| ffi::fail("scaler", path, error))?;
    let mut converted = Video::empty();
    scaler
        .run(&rgba, &mut converted)
        .map_err(|error| ffi::fail("convert", path, error))?;
    converted.set_pts(Some(0));

    encoder
        .send_frame(&converted)
        .map_err(|error| ffi::fail("encode", path, error))?;
    encoder
        .send_eof()
        .map_err(|error| ffi::fail("encode", path, error))?;
    let mut bytes = Vec::new();
    loop {
        let mut packet = ffmpeg::Packet::empty();
        match encoder.receive_packet(&mut packet) {
            Ok(()) => bytes.extend_from_slice(packet.data().unwrap_or(&[])),
            Err(ffmpeg::Error::Eof) => break,
            Err(error) if ffi::is_again(&error) => break,
            Err(error) => return Err(ffi::fail("encode", path, error)),
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_becomes_a_jpeg() {
        let mut frame = Frame::black(32, 32);
        frame.fill([200, 40, 40, 255]);
        let bytes = jpeg(&frame, 4).expect("encodes");
        assert!(bytes.starts_with(&[0xFF, 0xD8]), "a JPEG starts with SOI");
    }

    #[test]
    fn defaults_are_a_playable_h264_file() {
        let options = EncodeOptions::default();
        assert_eq!(options.codec, "libx264");
        assert_eq!(
            options.pixel_format, "yuv420p",
            "yuv444 will not play in browsers"
        );
    }

    #[test]
    fn a_wrong_sized_frame_is_rejected() {
        let path = std::env::temp_dir().join("concat-encode-size-test.mp4");
        let mut encoder =
            Encoder::create(&path, 64, 64, FrameRate::THIRTY, &EncodeOptions::default())
                .expect("the linked FFmpeg encodes h264");

        let wrong = Frame::black(32, 32);
        assert!(matches!(
            encoder.write_frame(&wrong),
            Err(Error::FrameSizeMismatch { got_width: 32, .. })
        ));
        encoder.write_frame(&Frame::black(64, 64)).expect("writes");
        encoder.finish().expect("finishes");
        assert!(std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 0));
        let _ = std::fs::remove_file(&path);
    }
}

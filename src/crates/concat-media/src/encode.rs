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

/// The codecs an export can be asked for, by the name of the standard, not
/// of an encoder: which encoder makes it is the platform's business - see
/// [`VideoCodec::encoders`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum VideoCodec {
    /// H.264 / AVC: plays everywhere, the largest files.
    #[default]
    H264,
    /// H.265 / HEVC: about half the size of H.264 at the same quality; the
    /// phones' and cameras' own format, and the one Apple's hardware
    /// encodes.
    Hevc,
    /// AV1: smaller again, royalty-free, the web's; slower to encode in
    /// software and not every old device plays it.
    Av1,
}

impl VideoCodec {
    /// Every codec, in the order a menu lists them.
    pub const ALL: [VideoCodec; 3] = [VideoCodec::H264, VideoCodec::Hevc, VideoCodec::Av1];

    /// The name a document or a request stores.
    pub fn name(self) -> &'static str {
        match self {
            VideoCodec::H264 => "h264",
            VideoCodec::Hevc => "hevc",
            VideoCodec::Av1 => "av1",
        }
    }

    /// The codec a stored name means, or `None` for one nobody stores.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "h264" | "h.264" | "avc" => Some(VideoCodec::H264),
            "hevc" | "h265" | "h.265" => Some(VideoCodec::Hevc),
            "av1" => Some(VideoCodec::Av1),
            _ => None,
        }
    }

    /// What the standard is called on a label.
    pub fn label(self) -> &'static str {
        match self {
            VideoCodec::H264 => "H.264",
            VideoCodec::Hevc => "HEVC",
            VideoCodec::Av1 => "AV1",
        }
    }

    /// The FFmpeg encoders that make this codec, best first. With
    /// `hardware`, the platform's own encoder leads where there is one
    /// worth leading with: VideoToolbox for HEVC on macOS, which is many
    /// times faster than x265 and, at these rates, as good to look at.
    /// H.264 stays with x264 everywhere: the hardware H.264 encoders
    /// spend noticeably more bits for the same picture, and H.264 is the
    /// choice made for compatibility, not speed.
    pub fn encoders(self, hardware: bool) -> &'static [&'static str] {
        match self {
            VideoCodec::H264 => &["libx264"],
            VideoCodec::Hevc if hardware && cfg!(target_os = "macos") => {
                &["hevc_videotoolbox", "libx265"]
            }
            VideoCodec::Hevc => &["libx265"],
            VideoCodec::Av1 => &["libsvtav1", "libaom-av1"],
        }
    }

    /// Whether the linked FFmpeg carries an encoder for it.
    pub fn available(self) -> bool {
        ffi::init();
        self.encoders(true)
            .iter()
            .any(|name| encoder::find_by_name(name).is_some())
    }

    /// Bytes per second relative to H.264 at the same quality, for a size
    /// estimate: the rule of thumb the codecs are chosen by.
    pub fn size_factor(self) -> f32 {
        match self {
            VideoCodec::H264 => 1.0,
            VideoCodec::Hevc => 0.6,
            VideoCodec::Av1 => 0.5,
        }
    }
}

/// How the encoder is told what rate to hold. VBR leaves the bitrate free
/// and asks for a quality (the CRF); CBR pins one target, at the cost of
/// some quality in the busy seconds. VBR is what every export used to be,
/// and stays the default.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RateMode {
    /// Quality first: the encoder picks a bitrate per frame. `bitrate_kbps`
    /// is unused.
    #[default]
    Vbr,
    /// Size first: `bitrate_kbps` is the target and the encoder keeps to it.
    /// Only the soft encoders support it; hardware encoders fall back to
    /// VBR because their rate control differs.
    Cbr,
}

impl RateMode {
    /// The name a document or a request stores.
    pub fn name(self) -> &'static str {
        match self {
            RateMode::Vbr => "vbr",
            RateMode::Cbr => "cbr",
        }
    }

    /// Every mode, in the order a menu lists them.
    pub const ALL: [RateMode; 2] = [RateMode::Vbr, RateMode::Cbr];
}

/// Encoder settings.
#[derive(Clone, Debug)]
pub struct EncodeOptions {
    /// What to encode to.
    pub codec: VideoCodec,
    /// x264's speed/size tradeoff, by x264's names; the other encoders
    /// are handed their own equivalent.
    pub preset: String,
    /// Constant rate factor on x264's scale, lower being better quality
    /// and a bigger file; the other encoders are handed their own
    /// equivalent.
    pub crf: u8,
    /// VBR (the CRF carries the quality) or CBR (the bitrate is the
    /// target). Default VBR, so an unchanged export means an unchanged
    /// file.
    pub rate_mode: RateMode,
    /// Target bitrate in kilobits per second, used when `rate_mode` is
    /// CBR. Zero means "not set" and the encoder falls back to VBR.
    pub bitrate_kbps: u32,
    /// Ten bits a channel rather than eight: no banding in a sky or a
    /// gradient, at a few percent more file. H.264 at ten bits plays on
    /// less than HEVC or AV1 at ten bits do.
    pub ten_bit: bool,
    /// Let the platform's hardware encoder lead where there is one; see
    /// [`VideoCodec::encoders`].
    pub hardware: bool,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            codec: VideoCodec::H264,
            preset: "medium".to_owned(),
            crf: 18,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 0,
            ten_bit: false,
            hardware: true,
        }
    }
}

/// The x264 presets in speed order, which is also how SVT-AV1 numbers its
/// own: 13 is the fastest there and 0 the slowest.
const X264_PRESETS: [&str; 10] = [
    "ultrafast",
    "superfast",
    "veryfast",
    "faster",
    "fast",
    "medium",
    "slow",
    "slower",
    "veryslow",
    "placebo",
];

/// SVT-AV1's preset for an x264 one: the same place on its own scale.
fn svt_preset(preset: &str) -> u8 {
    const SVT: [u8; 10] = [12, 11, 10, 9, 8, 6, 4, 3, 2, 1];
    X264_PRESETS
        .iter()
        .position(|name| *name == preset)
        .map_or(6, |index| SVT[index])
}

/// VideoToolbox's quality, 1 to 100 with 100 the best, for an x264 CRF:
/// 16 lands at 65 and 26 at 43, which is where the files come out about
/// the size x264 makes them.
fn videotoolbox_quality(crf: u8) -> u8 {
    (100.0 - f32::from(crf) * 2.2).round().clamp(1.0, 100.0) as u8
}

/// A four-character code as the muxer stores it.
const fn fourcc(tag: [u8; 4]) -> u32 {
    u32::from_le_bytes(tag)
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
    /// Which FFmpeg encoder took the job.
    encoder_name: &'static str,
}

impl Encoder {
    /// Opens `path` for writing, overwriting anything already there.
    ///
    /// The file is tagged BT.709, full stop: primaries, transfer and
    /// matrix, video range. Every frame this is given is an sRGB picture,
    /// and a file that does not say what it holds is shown however each
    /// player guesses - the washed-out-on-the-phone complaint. The RGB to
    /// YUV conversion uses the matching coefficients, so the tag is true.
    pub fn create(
        path: impl AsRef<Path>,
        width: u32,
        height: u32,
        frame_rate: FrameRate,
        options: &EncodeOptions,
    ) -> Result<Self> {
        use ffmpeg::color::{Primaries, Space, TransferCharacteristic};
        Self::create_tagged(
            path,
            width,
            height,
            frame_rate,
            options,
            (
                Primaries::BT709,
                TransferCharacteristic::BT709,
                Space::BT709,
            ),
        )
    }

    /// [`Encoder::create`] with the colour the file is tagged as. Crate
    /// private, and only the tests use it for anything but BT.709: the
    /// frames are sRGB whatever the tag says, so any other tag is a lie
    /// - which is exactly what a test of the decoder's HDR path needs.
    pub(crate) fn create_tagged(
        path: impl AsRef<Path>,
        width: u32,
        height: u32,
        frame_rate: FrameRate,
        options: &EncodeOptions,
        tags: (
            ffmpeg::color::Primaries,
            ffmpeg::color::TransferCharacteristic,
            ffmpeg::color::Space,
        ),
    ) -> Result<Self> {
        ffi::init();
        let path = path.as_ref();
        let fps = frame_rate.fps();
        let rate = ffmpeg::Rational::new(fps.numerator() as i32, fps.denominator() as i32);
        let time_base = rate.invert();

        // The first encoder the linked FFmpeg has for the codec.
        let (encoder_name, codec) = options
            .codec
            .encoders(options.hardware)
            .iter()
            .find_map(|name| encoder::find_by_name(name).map(|codec| (*name, codec)))
            .ok_or_else(|| Error::Missing {
                what: "encoder",
                name: options.codec.label().to_owned(),
            })?;
        let videotoolbox = encoder_name.ends_with("_videotoolbox");
        // VideoToolbox takes its pictures planar-chroma; the software
        // encoders take the planar 4:2:0 they all read.
        let pixel_format = match (videotoolbox, options.ten_bit) {
            (true, true) => Pixel::P010LE,
            (true, false) => Pixel::NV12,
            (false, true) => Pixel::YUV420P10LE,
            (false, false) => Pixel::YUV420P,
        };

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
        let (primaries, transfer, matrix) = tags;
        video.set_colorspace(matrix);
        video.set_color_range(ffmpeg::color::Range::MPEG);
        // SAFETY: `video` owns a live AVCodecContext; primaries and
        // transfer have no setter in the bindings, and both are plain
        // fields the encoder reads at open.
        unsafe {
            let context = video.as_mut_ptr();
            (*context).color_primaries = primaries.into();
            (*context).color_trc = transfer.into();
        }
        if global_header {
            video.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        }

        let crf = options.crf.to_string();
        let cbr = options.rate_mode == RateMode::Cbr && options.bitrate_kbps > 0;
        let bitrate = format!("{}k", options.bitrate_kbps);
        // One second of VBV, not two. With a looser buffer x264 can stay
        // well under b:v on calm content and never emit the padding that
        // makes a CBR a CBR; `bufsize == bitrate` is where it starts
        // holding the target.
        let bufsize = format!("{}k", options.bitrate_kbps);
        let settings = match encoder_name {
            // libx264 / libx265 both take -b:v, -minrate and -maxrate for
            // CBR; VBR is the CRF that already shipped.
            // Real CBR needs `nal-hrd=cbr` on x264, and `strict-cbr=1` on
            // libx265. Without it x264 caps at maxrate but does not pad the
            // output to b:v on content that does not need the bitrate, so an
            // "8000k CBR" export of a calm clip comes out at whatever the
            // content costs, not 8000k.
            "libx264" if cbr => ffmpeg::dict! {
                "preset" => options.preset.as_str(),
                "b:v" => bitrate.as_str(),
                "minrate" => bitrate.as_str(),
                "maxrate" => bitrate.as_str(),
                "bufsize" => bufsize.as_str(),
                "x264-params" => "nal-hrd=cbr:force-cfr=1",
            },
            "libx265" if cbr => ffmpeg::dict! {
                "preset" => options.preset.as_str(),
                "b:v" => bitrate.as_str(),
                "minrate" => bitrate.as_str(),
                "maxrate" => bitrate.as_str(),
                "bufsize" => bufsize.as_str(),
                "x265-params" => "strict-cbr=1",
            },
            "libx264" | "libx265" => ffmpeg::dict! {
                "preset" => options.preset.as_str(),
                "crf" => crf.as_str(),
            },
            "libsvtav1" => ffmpeg::dict! {
                "preset" => &svt_preset(&options.preset).to_string(),
                // AV1's CRF runs to 63 and reads a few steps coarser than
                // x264's; eight on is where the pictures match.
                "crf" => &options.crf.saturating_add(8).min(63).to_string(),
            },
            "libaom-av1" => ffmpeg::dict! {
                "crf" => &options.crf.saturating_add(8).min(63).to_string(),
                "cpu-used" => "6",
            },
            "hevc_videotoolbox" => ffmpeg::dict! {
                "q:v" => &videotoolbox_quality(options.crf).to_string(),
                "profile" => if options.ten_bit { "main10" } else { "main" },
                // A machine without the hardware still gets a file.
                "allow_sw" => "1",
            },
            _ => ffmpeg::dict! {},
        };
        let encoder = video
            .open_with(settings)
            .map_err(|error| ffi::fail("open encoder", path, error))?;

        {
            let mut stream = output
                .add_stream(codec)
                .map_err(|error| ffi::fail("add stream", path, error))?;
            stream.copy_parameters_from_context(&encoder);
            stream.set_time_base(time_base);
            if options.codec == VideoCodec::Hevc {
                // `hvc1`, not FFmpeg's default `hev1`: the tag Apple's
                // players and QuickTime need to open an HEVC file at all.
                // SAFETY: the stream is live and its parameters were just
                // copied; the tag is a plain field the muxer reads at the
                // header.
                unsafe {
                    (*(*stream.as_mut_ptr()).codecpar).codec_tag = fourcc(*b"hvc1");
                }
            }
        }
        output
            .write_header_with(ffmpeg::dict! { "movflags" => "+faststart" })
            .map_err(|error| ffi::fail("write header", path, error))?;
        let stream_time_base = output
            .stream(0)
            .map(|stream| stream.time_base())
            .unwrap_or(time_base);

        let mut scaler = scaling::Context::get(
            Pixel::RGBA,
            width,
            height,
            pixel_format,
            width,
            height,
            scaling::Flags::BILINEAR,
        )
        .map_err(|error| ffi::fail("scaler", path, error))?;
        // BT.709 coefficients from full-range RGB to video-range YUV,
        // rather than swscale's BT.601 default: the file says 709, so the
        // numbers in it are 709.
        // SAFETY: `scaler` owns a live SwsContext; the coefficient tables
        // are static and the call only sets fields on the context.
        unsafe {
            let coefficients = ffmpeg::sys::sws_getCoefficients(ffmpeg::sys::SWS_CS_ITU709);
            ffmpeg::sys::sws_setColorspaceDetails(
                scaler.as_mut_ptr(),
                coefficients,
                1,
                coefficients,
                0,
                0,
                1 << 16,
                1 << 16,
            );
        }

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
            encoder_name,
        })
    }

    /// Which FFmpeg encoder is doing the work, e.g. "hevc_videotoolbox".
    pub fn encoder_name(&self) -> &'static str {
        self.encoder_name
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
        assert_eq!(options.codec, VideoCodec::H264);
        assert!(!options.ten_bit, "eight-bit H.264 is what plays everywhere");
        assert_eq!(VideoCodec::parse("H.265"), Some(VideoCodec::Hevc));
        assert_eq!(VideoCodec::parse("mpeg2"), None);
        for codec in VideoCodec::ALL {
            assert_eq!(VideoCodec::parse(codec.name()), Some(codec));
        }
    }

    #[test]
    fn presets_and_quality_map_onto_the_other_encoders() {
        assert_eq!(svt_preset("medium"), 6);
        assert_eq!(svt_preset("veryfast"), 10);
        assert_eq!(svt_preset("nonsense"), 6);
        assert_eq!(videotoolbox_quality(16), 65);
        assert_eq!(videotoolbox_quality(26), 43);
        assert_eq!(fourcc(*b"hvc1"), 0x3163_7668);
    }

    /// The tags a file carries, read back through libavformat.
    fn tags_of(
        path: &Path,
    ) -> (
        String,
        u32,
        ffmpeg::color::Primaries,
        ffmpeg::color::TransferCharacteristic,
        ffmpeg::color::Space,
        Pixel,
    ) {
        let input = ffmpeg::format::input(path).expect("opens");
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .expect("video");
        let parameters = stream.parameters();
        // SAFETY: the parameters are live for as long as `input` is, and
        // the tag is a plain field.
        let tag = unsafe { (*parameters.as_ptr()).codec_tag };
        let context = ffmpeg::codec::Context::from_parameters(parameters).expect("context");
        let decoder = context.decoder().video().expect("decoder");
        (
            decoder.id().name().to_owned(),
            tag,
            decoder.color_primaries(),
            decoder.color_transfer_characteristic(),
            decoder.color_space(),
            decoder.format(),
        )
    }

    /// Every codec the linked FFmpeg has, at eight and ten bits, makes a
    /// file that says what it is: the codec, BT.709 all the way through,
    /// the bit depth asked for, and `hvc1` on HEVC.
    #[test]
    fn every_available_codec_writes_a_tagged_file() {
        for codec in VideoCodec::ALL {
            if !codec.available() {
                eprintln!("{} not in the linked FFmpeg; skipped", codec.label());
                continue;
            }
            for ten_bit in [false, true] {
                let path = std::env::temp_dir().join(format!(
                    "concat-encode-{}-{}.mp4",
                    codec.name(),
                    ten_bit
                ));
                let options = EncodeOptions {
                    codec,
                    preset: "ultrafast".to_owned(),
                    crf: 24,
                    rate_mode: RateMode::Vbr,
                    bitrate_kbps: 0,
                    ten_bit,
                    hardware: true,
                };
                let mut encoder = Encoder::create(&path, 64, 64, FrameRate::THIRTY, &options)
                    .unwrap_or_else(|error| panic!("{} {ten_bit}: {error}", codec.label()));
                let mut frame = Frame::black(64, 64);
                frame.fill([200, 40, 40, 255]);
                for _ in 0..4 {
                    encoder.write_frame(&frame).expect("writes");
                }
                encoder.finish().expect("finishes");

                let (name, tag, primaries, transfer, space, format) = tags_of(&path);
                let _ = std::fs::remove_file(&path);
                assert_eq!(name, codec.name(), "the stream's codec");
                assert_eq!(primaries, ffmpeg::color::Primaries::BT709);
                assert_eq!(transfer, ffmpeg::color::TransferCharacteristic::BT709);
                assert_eq!(space, ffmpeg::color::Space::BT709);
                let deep = matches!(format, Pixel::YUV420P10LE | Pixel::P010LE);
                assert_eq!(deep, ten_bit, "{} bit depth ({format:?})", codec.label());
                if codec == VideoCodec::Hevc {
                    assert_eq!(tag, fourcc(*b"hvc1"), "HEVC in MP4 is tagged hvc1");
                }
            }
        }
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

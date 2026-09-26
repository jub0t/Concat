// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Writing RGBA frames back out to a file.

use std::path::{Path, PathBuf};

use concat_core::frame::{Depth, Frame, Signal};
use concat_core::time::FrameRate;
use ffmpeg_the_third as ffmpeg;
use ffmpeg_the_third::codec::encoder;
use ffmpeg_the_third::format::{self, Pixel};
use ffmpeg_the_third::software::scaling;
use ffmpeg_the_third::util::frame::video::Video;

use crate::decode::ColorRange;
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
    /// The levels the file is written in and tagged with: video range,
    /// 16-235, which every player and YouTube expect, or full range,
    /// 0-255, for screen content bound for a PC player that reads the
    /// tag. The RGB to YUV conversion follows the choice, so the tag is
    /// true either way. Video range unless told otherwise.
    /// https://github.com/jub0t/Concat/issues/103
    pub color_range: ColorRange,
    /// Let the platform's hardware encoder lead where there is one; see
    /// [`VideoCodec::encoders`].
    pub hardware: bool,
    /// Threads the encoder may use, or zero to let it count the cores.
    /// Zero for an export; a few for a proxy written while the editor is
    /// in use.
    pub threads: u16,
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
            color_range: ColorRange::Limited,
            hardware: true,
            threads: 0,
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
    /// The range every converted frame is stamped with, matching the
    /// stream's tag and the scaler's conversion.
    color_range: ffmpeg::color::Range,
    /// The frames are sixteen bits a channel, Rec. 2020 with the transfer
    /// applied (an HDR file), rather than eight-bit sRGB.
    deep: bool,
    /// A PQ file's light, measured as it is written, for the metadata the
    /// file carries once it is finished.
    light: Option<LightLevel>,
}

/// The mastering display an HDR10 file names: a P3 display of 1000 nits
/// over 0.0001, the reference most HDR is graded on. FFmpeg's
/// `AVMasteringDisplayMetadata`, whose header the bindings leave out.
#[repr(C)]
struct MasteringDisplay {
    /// CIE 1931 xy of the red, green and blue primaries.
    display_primaries: [[ffmpeg::sys::AVRational; 2]; 3],
    white_point: [ffmpeg::sys::AVRational; 2],
    min_luminance: ffmpeg::sys::AVRational,
    max_luminance: ffmpeg::sys::AVRational,
    has_primaries: std::os::raw::c_int,
    has_luminance: std::os::raw::c_int,
}

impl MasteringDisplay {
    fn p3_1000() -> Self {
        let xy = |x: i32, y: i32| {
            [
                ffmpeg::sys::AVRational {
                    num: x,
                    den: 50_000,
                },
                ffmpeg::sys::AVRational {
                    num: y,
                    den: 50_000,
                },
            ]
        };
        MasteringDisplay {
            display_primaries: [xy(34_000, 16_000), xy(13_250, 34_500), xy(7_500, 3_000)],
            white_point: xy(15_635, 16_450),
            min_luminance: ffmpeg::sys::AVRational {
                num: 1,
                den: 10_000,
            },
            max_luminance: ffmpeg::sys::AVRational { num: 1_000, den: 1 },
            has_primaries: 1,
            has_luminance: 1,
        }
    }
}

/// The same display as x265 and SVT-AV1 spell it.
const X265_MASTER_DISPLAY: &str =
    "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1)";
const SVT_MASTER_DISPLAY: &str =
    "G(0.265,0.690)B(0.150,0.060)R(0.680,0.320)WP(0.3127,0.3290)L(1000,0.0001)";

/// FFmpeg's `AVContentLightMetadata`: the brightest pixel and the brightest
/// frame's average, in nits.
#[repr(C)]
struct ContentLight {
    max_cll: std::os::raw::c_uint,
    max_fall: std::os::raw::c_uint,
}

/// MaxCLL and MaxFALL as a PQ file is written: every pixel's brightest
/// channel in nits, through a table of the sixteen-bit signal.
struct LightLevel {
    nits: Box<[f32]>,
    max_cll: f32,
    max_fall: f32,
}

impl LightLevel {
    fn new() -> Self {
        // SMPTE ST 2084's EOTF, a signal to nits.
        let (m1, m2) = (0.159_301_76_f64, 78.843_75_f64);
        let (c1, c2, c3) = (0.835_937_5_f64, 18.851_562_5_f64, 18.687_5_f64);
        let nits = (0..=u16::MAX)
            .map(|code| {
                let e = (f64::from(code) / 65_535.0).powf(1.0 / m2);
                let y = ((e - c1).max(0.0) / (c2 - c3 * e)).powf(1.0 / m1);
                (y * 10_000.0) as f32
            })
            .collect();
        LightLevel {
            nits,
            max_cll: 0.0,
            max_fall: 0.0,
        }
    }

    /// One frame's pixels, RGBA sixteen bits a channel, little-endian.
    fn measure(&mut self, pixels: &[u8]) {
        let mut sum = 0.0f64;
        let mut count = 0usize;
        for pixel in pixels.chunks_exact(8) {
            let channel = |at: usize| u16::from_le_bytes([pixel[at], pixel[at + 1]]);
            let brightest = channel(0).max(channel(2)).max(channel(4));
            let nits = self.nits[usize::from(brightest)];
            self.max_cll = self.max_cll.max(nits);
            sum += f64::from(nits);
            count += 1;
        }
        if count > 0 {
            self.max_fall = self.max_fall.max((sum / count as f64) as f32);
        }
    }
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

    /// [`Encoder::create`] tagged as HDR - BT.2020, 10-bit, HLG or PQ -
    /// over frames that are sRGB pictures. The tag is a lie, and the name
    /// says so: it exists for the perf harness, which times the decoder's
    /// HDR path, and the path is chosen by the tag, not the pixels. Nothing
    /// that writes a file a person will watch may call it; real HDR export
    /// is a phase of the HDR plan, not this.
    #[doc(hidden)]
    pub fn create_mislabelled_hdr(
        path: impl AsRef<Path>,
        width: u32,
        height: u32,
        frame_rate: FrameRate,
        options: &EncodeOptions,
        pq: bool,
    ) -> Result<Self> {
        use ffmpeg::color::{Primaries, Space, TransferCharacteristic};
        let transfer = if pq {
            TransferCharacteristic::SMPTE2084
        } else {
            TransferCharacteristic::ARIB_STD_B67
        };
        let options = EncodeOptions {
            ten_bit: true,
            ..options.clone()
        };
        Self::create_tagged(
            path,
            width,
            height,
            frame_rate,
            &options,
            (Primaries::BT2020, transfer, Space::BT2020NCL),
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
        Self::open(path, width, height, frame_rate, options, tags, None)
    }

    /// Opens `path` for an HDR file: BT.2020, ten bits, HLG or PQ as
    /// `signal` says, written from frames that are that already - sixteen
    /// bits a channel, Rec. 2020, the transfer applied, which is what the
    /// compositor's HDR resolve hands back. A PQ file also names the
    /// display it was mastered on (a P3 display of 1000 nits) and, once it
    /// is finished, the brightest pixel and frame in it. H.264 is refused:
    /// the players that matter do not take HDR in it.
    pub fn create_hdr(
        path: impl AsRef<Path>,
        width: u32,
        height: u32,
        frame_rate: FrameRate,
        options: &EncodeOptions,
        signal: Signal,
    ) -> Result<Self> {
        use ffmpeg::color::{Primaries, Space, TransferCharacteristic};
        let path = path.as_ref();
        let transfer = match signal {
            Signal::Hlg => TransferCharacteristic::ARIB_STD_B67,
            Signal::Pq => TransferCharacteristic::SMPTE2084,
            Signal::Sdr | Signal::SdrWide => {
                return Err(Error::Ffi {
                    operation: "open an HDR encoder",
                    path: path.to_path_buf(),
                    detail: "an HDR file is HLG or PQ".to_owned(),
                });
            }
        };
        if options.codec == VideoCodec::H264 {
            return Err(Error::Ffi {
                operation: "open an HDR encoder",
                path: path.to_path_buf(),
                detail: "H.264 does not carry HDR here: choose HEVC or AV1".to_owned(),
            });
        }
        let options = EncodeOptions {
            ten_bit: true,
            ..options.clone()
        };
        Self::open(
            path,
            width,
            height,
            frame_rate,
            &options,
            (Primaries::BT2020, transfer, Space::BT2020NCL),
            Some(signal),
        )
    }

    /// The one way in: `tags` on the file, and `hdr` the signal of deep
    /// frames when the file is HDR (see [`Encoder::create_hdr`]), else the
    /// frames are eight-bit sRGB.
    fn open(
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
        hdr: Option<Signal>,
    ) -> Result<Self> {
        ffi::init();
        let pq = hdr == Some(Signal::Pq);
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
        // The range the frames will be converted to below, so the tag is
        // true; VideoToolbox reads it to pick its full- or video-range
        // pixel format, the software encoders write it into the stream.
        video.set_color_range(options.color_range.as_ffmpeg());
        // SAFETY: `video` owns a live AVCodecContext; primaries and
        // transfer have no setter in the bindings, and all three are plain
        // fields the encoder reads at open.
        unsafe {
            let context = video.as_mut_ptr();
            (*context).color_primaries = primaries.into();
            (*context).color_trc = transfer.into();
            if options.threads > 0 {
                (*context).thread_count = i32::from(options.threads);
            }
        }
        if global_header {
            video.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        }
        if pq {
            // The display it was mastered on, where an encoder that writes
            // it into the stream reads it.
            // SAFETY: `video` owns a live AVCodecContext; the side data is
            // allocated by FFmpeg at the struct's size and filled here.
            unsafe {
                let context = video.as_mut_ptr();
                let entry = ffmpeg::sys::av_frame_side_data_new(
                    &mut (*context).decoded_side_data,
                    &mut (*context).nb_decoded_side_data,
                    ffmpeg::sys::AVFrameSideDataType::MASTERING_DISPLAY_METADATA,
                    std::mem::size_of::<MasteringDisplay>(),
                    0,
                );
                if !entry.is_null() {
                    std::ptr::write(
                        (*entry).data.cast::<MasteringDisplay>(),
                        MasteringDisplay::p3_1000(),
                    );
                }
            }
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
            // libx264 / libx265 both take a bitrate, -minrate and -maxrate
            // for CBR; VBR is the CRF that already shipped. The bitrate is
            // `b`, the codec option's own name: `b:v` is the command line's
            // spelling with a stream specifier, which the library does not
            // know, so it was dropped - x264 then ran at its CRF under the
            // cap, and x265 refused strict-cbr without a bitrate.
            // Real CBR needs `nal-hrd=cbr` on x264, and `strict-cbr=1` on
            // libx265. Without it x264 caps at maxrate but does not pad the
            // output to b:v on content that does not need the bitrate, so an
            // "8000k CBR" export of a calm clip comes out at whatever the
            // content costs, not 8000k.
            "libx264" if cbr => ffmpeg::dict! {
                "preset" => options.preset.as_str(),
                "b" => bitrate.as_str(),
                "minrate" => bitrate.as_str(),
                "maxrate" => bitrate.as_str(),
                "bufsize" => bufsize.as_str(),
                "x264-params" => "nal-hrd=cbr:force-cfr=1",
            },
            "libx265" if cbr => ffmpeg::dict! {
                "preset" => options.preset.as_str(),
                "b" => bitrate.as_str(),
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
        // HDR's own words to the software encoders, besides the tags they
        // read off the context: headers with every keyframe, and a PQ
        // file's mastering display.
        let mut settings = settings;
        if hdr.is_some() {
            match encoder_name {
                "libx265" => {
                    let mut params = settings
                        .get("x265-params")
                        .map(str::to_owned)
                        .unwrap_or_default();
                    for param in [
                        Some("repeat-headers=1".to_owned()),
                        pq.then(|| format!("hdr10=1:master-display={X265_MASTER_DISPLAY}")),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if !params.is_empty() {
                            params.push(':');
                        }
                        params.push_str(&param);
                    }
                    settings.set("x265-params", &params);
                }
                "libsvtav1" if pq => {
                    settings.set(
                        "svtav1-params",
                        format!("mastering-display={SVT_MASTER_DISPLAY}"),
                    );
                }
                _ => {}
            }
        }
        let encoder = video
            .open_with(settings)
            .map_err(|error| ffi::fail("open encoder", path, error))?;

        {
            let mut stream = output
                .add_stream(codec)
                .map_err(|error| ffi::fail("add stream", path, error))?;
            stream.copy_parameters_from_context(&encoder);
            stream.set_time_base(time_base);
            if pq {
                // The same display in the container, for the players that
                // read it there ('mdcv').
                // SAFETY: the stream is live and its parameters were just
                // copied; FFmpeg allocates the entry at the struct's size.
                unsafe {
                    let parameters = (*stream.as_mut_ptr()).codecpar;
                    let entry = ffmpeg::sys::av_packet_side_data_new(
                        &mut (*parameters).coded_side_data,
                        &mut (*parameters).nb_coded_side_data,
                        ffmpeg::sys::AVPacketSideDataType::MASTERING_DISPLAY_METADATA,
                        std::mem::size_of::<MasteringDisplay>(),
                        0,
                    );
                    if !entry.is_null() {
                        std::ptr::write(
                            (*entry).data.cast::<MasteringDisplay>(),
                            MasteringDisplay::p3_1000(),
                        );
                    }
                }
            }
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

        let deep = hdr.is_some();
        let mut scaler = scaling::Context::get(
            if deep { Pixel::RGBA64LE } else { Pixel::RGBA },
            width,
            height,
            pixel_format,
            width,
            height,
            scaling::Flags::BILINEAR,
        )
        .map_err(|error| ffi::fail("scaler", path, error))?;
        // BT.709 coefficients from full-range RGB to YUV in the range the
        // file is tagged with, rather than swscale's BT.601 default: the
        // file says 709, so the numbers in it are 709, and it says which
        // range, so the numbers span that range.
        // SAFETY: `scaler` owns a live SwsContext; the coefficient tables
        // are static and the call only sets fields on the context.
        unsafe {
            let coefficients = ffmpeg::sys::sws_getCoefficients(if deep {
                ffmpeg::sys::SWS_CS_BT2020
            } else {
                ffmpeg::sys::SWS_CS_ITU709
            });
            ffmpeg::sys::sws_setColorspaceDetails(
                scaler.as_mut_ptr(),
                coefficients,
                1,
                coefficients,
                i32::from(options.color_range == ColorRange::Full),
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
            color_range: options.color_range.as_ffmpeg(),
            deep,
            light: pq.then(LightLevel::new),
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
                    // One frame, in the encoder's own time base - unset,
                    // this defaults to zero, and a muxer that never hears a
                    // packet's duration falls back to inferring the stream's
                    // total duration from the PTS span alone, which comes up
                    // one frame short: the last packet has no next one to
                    // measure to. Set here so `rescale_ts` carries it over
                    // with the timestamps, and the file's own duration
                    // matches what was actually encoded.
                    packet.set_duration(1);
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

        let deep = frame.depth() == Depth::Sixteen;
        if deep != self.deep {
            return Err(Error::Io {
                path: self.path.clone(),
                source: std::io::Error::other(if self.deep {
                    "an HDR file takes sixteen-bit frames"
                } else {
                    "an SDR file takes eight-bit frames"
                }),
            });
        }
        let (pixel, bytes) = if deep {
            (Pixel::RGBA64LE, 8)
        } else {
            (Pixel::RGBA, 4)
        };
        let mut rgba = Video::new(pixel, self.width, self.height);
        {
            let stride = rgba.stride(0);
            let row = self.width as usize * bytes;
            let data = rgba.data_mut(0);
            for (y, source) in frame.pixels().chunks_exact(row).enumerate() {
                data[y * stride..y * stride + row].copy_from_slice(source);
            }
        }
        if let Some(light) = &mut self.light {
            light.measure(frame.pixels());
        }
        let mut converted = Video::empty();
        self.scaler
            .run(&rgba, &mut converted)
            .map_err(|error| ffi::fail("convert", &self.path, error))?;
        converted.set_pts(Some(self.written as i64));
        // The frame says what its numbers span, the same as the stream
        // does: a hardware encoder reads it off the frame.
        // SAFETY: `converted` is a live frame the scaler just filled; the
        // range is a plain field the encoder reads with the picture.
        unsafe {
            (*converted.as_mut_ptr()).color_range = self.color_range.into();
        }

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
        if let Some(light) = &self.light {
            // The light the file holds, measured as it was written, in the
            // container ('clli'), which the muxer writes with the index.
            // SAFETY: the stream is live; FFmpeg allocates the entry at the
            // struct's size.
            unsafe {
                if let Some(mut stream) = self.output.stream_mut(0) {
                    let parameters = (*stream.as_mut_ptr()).codecpar;
                    let entry = ffmpeg::sys::av_packet_side_data_new(
                        &mut (*parameters).coded_side_data,
                        &mut (*parameters).nb_coded_side_data,
                        ffmpeg::sys::AVPacketSideDataType::CONTENT_LIGHT_LEVEL,
                        std::mem::size_of::<ContentLight>(),
                        0,
                    );
                    if !entry.is_null() {
                        std::ptr::write(
                            (*entry).data.cast::<ContentLight>(),
                            ContentLight {
                                max_cll: light.max_cll.round() as u32,
                                max_fall: light.max_fall.round() as u32,
                            },
                        );
                    }
                }
            }
        }
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

    /// A constant-bitrate export opens with each software encoder and comes
    /// out near the rate asked for. Noise, so the content costs more than
    /// the target and the encoder has to hold it rather than coast under.
    #[test]
    fn a_cbr_export_opens_and_holds_its_rate() {
        let dir =
            std::env::temp_dir().join(format!("concat-media-cbr-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        const KBPS: u32 = 2_000;
        const FRAMES: u32 = 60;
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            let path = dir.join(format!("{}.mp4", codec.name()));
            let options = EncodeOptions {
                codec,
                preset: "ultrafast".to_owned(),
                rate_mode: RateMode::Cbr,
                bitrate_kbps: KBPS,
                hardware: false,
                ..EncodeOptions::default()
            };
            let mut encoder = Encoder::create(&path, 320, 180, FrameRate::THIRTY, &options)
                .unwrap_or_else(|error| panic!("{codec:?} CBR opens: {error}"));
            let mut frame = Frame::black(320, 180);
            let mut state = 0x2545_F491_4F6C_DD1D_u64;
            for _ in 0..FRAMES {
                for byte in frame.pixels_mut() {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    *byte = state as u8;
                }
                encoder.write_frame(&frame).expect("writes");
            }
            encoder.finish().expect("finishes");
            let kbps = std::fs::metadata(&path).expect("written").len() as f64 * 8.0
                / 1000.0
                / (f64::from(FRAMES) / 30.0);
            assert!(
                (f64::from(KBPS) * 0.7..=f64::from(KBPS) * 1.3).contains(&kbps),
                "{codec:?} CBR at {KBPS} kb/s came out at {kbps:.0} kb/s"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every written frame decodes back, at a frame rate an even number of
    /// frames does not land on a round second at (25 fps, 6 seconds: every
    /// packet but the last infers its duration from the one after it, so a
    /// packet's duration was never set here and the file's own reported
    /// length came up one frame short - which is exactly the gap a strict
    /// reader, not just this crate's own decoder, counts by).
    #[test]
    fn every_written_frame_decodes_back_even_at_a_rate_with_no_last_neighbour() {
        let dir =
            std::env::temp_dir().join(format!("concat-media-encode-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let path = dir.join("150-at-25fps.mp4");

        let options = EncodeOptions {
            codec: VideoCodec::H264,
            preset: "ultrafast".to_owned(),
            crf: 16,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 0,
            ten_bit: false,
            color_range: ColorRange::Limited,
            hardware: false,
            threads: 0,
        };
        let rate = FrameRate::new(concat_core::time::Rational::new(25, 1));
        const FRAMES: u32 = 150;
        {
            let mut encoder = Encoder::create(&path, 64, 64, rate, &options).expect("encodes");
            let mut frame = Frame::black(64, 64);
            for i in 0..FRAMES {
                frame.fill([(i % 256) as u8, 0, 0, 255]);
                encoder.write_frame(&frame).expect("writes");
            }
            encoder.finish().expect("finishes");
        }

        use crate::decode::{DecodeOptions, Decoder, FrameSource};
        let mut decoder =
            Decoder::open(&path, &DecodeOptions::default()).expect("opens what was just written");
        let mut count = 0;
        while decoder.next_frame().expect("decodes").is_some() {
            count += 1;
        }
        assert_eq!(count, FRAMES, "every encoded frame should decode back");

        let _ = std::fs::remove_dir_all(&dir);
    }

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
        ffmpeg::color::Range,
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
            decoder.color_range(),
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
                    color_range: ColorRange::Limited,
                    hardware: true,
                    threads: 0,
                };
                let mut encoder = Encoder::create(&path, 64, 64, FrameRate::THIRTY, &options)
                    .unwrap_or_else(|error| panic!("{} {ten_bit}: {error}", codec.label()));
                let mut frame = Frame::black(64, 64);
                frame.fill([200, 40, 40, 255]);
                for _ in 0..4 {
                    encoder.write_frame(&frame).expect("writes");
                }
                encoder.finish().expect("finishes");

                let (name, tag, primaries, transfer, space, format, range) = tags_of(&path);
                let _ = std::fs::remove_file(&path);
                assert_eq!(name, codec.name(), "the stream's codec");
                assert_eq!(primaries, ffmpeg::color::Primaries::BT709);
                assert_eq!(transfer, ffmpeg::color::TransferCharacteristic::BT709);
                assert_eq!(space, ffmpeg::color::Space::BT709);
                assert_eq!(range, ffmpeg::color::Range::MPEG, "video range by default");
                let deep = matches!(format, Pixel::YUV420P10LE | Pixel::P010LE);
                assert_eq!(deep, ten_bit, "{} bit depth ({format:?})", codec.label());
                if codec == VideoCodec::Hevc {
                    assert_eq!(tag, fourcc(*b"hvc1"), "HEVC in MP4 is tagged hvc1");
                }
            }
        }
    }

    /// An HDR file is what it says it is: ten bits, BT.2020 with HLG or PQ,
    /// written from deep frames whose levels come back as they went in -
    /// HLG's reference white at 75 % of the signal, PQ's at 58 % - and an
    /// eight-bit frame is refused. A PQ file names its mastering display
    /// and the light it holds in the container; H.264 is refused outright.
    #[test]
    fn an_hdr_file_is_tagged_and_keeps_its_levels() {
        use crate::decode::{DecodeOptions, Decoder, FrameSource};
        let options = |codec| EncodeOptions {
            codec,
            preset: "ultrafast".to_owned(),
            crf: 16,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 0,
            ten_bit: false,
            color_range: ColorRange::Limited,
            hardware: true,
            threads: 0,
        };
        let refused = std::env::temp_dir().join("concat-encode-hdr-h264.mp4");
        assert!(
            Encoder::create_hdr(
                &refused,
                64,
                64,
                FrameRate::THIRTY,
                &options(VideoCodec::H264),
                Signal::Hlg
            )
            .is_err()
        );
        for codec in [VideoCodec::Hevc, VideoCodec::Av1] {
            if !codec.available() {
                eprintln!("{} not in the linked FFmpeg; skipped", codec.label());
                continue;
            }
            for (signal, level) in [(Signal::Hlg, 0.75f64), (Signal::Pq, 0.58)] {
                let path = std::env::temp_dir()
                    .join(format!("concat-encode-hdr-{}-{signal:?}.mp4", codec.name()));
                let mut encoder =
                    Encoder::create_hdr(&path, 64, 64, FrameRate::THIRTY, &options(codec), signal)
                        .unwrap_or_else(|error| panic!("{} {signal:?}: {error}", codec.label()));
                let value = (level * 65_535.0).round() as u16;
                let pixel: Vec<u8> = [value, value, value, u16::MAX]
                    .iter()
                    .flat_map(|channel| channel.to_le_bytes())
                    .collect();
                let frame = Frame::from_rgba64(64, 64, pixel.repeat(64 * 64), signal)
                    .expect("a deep frame");
                for _ in 0..4 {
                    encoder.write_frame(&frame).expect("writes");
                }
                assert!(
                    encoder.write_frame(&Frame::black(64, 64)).is_err(),
                    "an HDR file takes deep frames only"
                );
                encoder.finish().expect("finishes");

                let (_, _, primaries, transfer, space, format, _) = tags_of(&path);
                assert_eq!(primaries, ffmpeg::color::Primaries::BT2020);
                assert_eq!(
                    transfer,
                    match signal {
                        Signal::Hlg => ffmpeg::color::TransferCharacteristic::ARIB_STD_B67,
                        _ => ffmpeg::color::TransferCharacteristic::SMPTE2084,
                    }
                );
                assert_eq!(space, ffmpeg::color::Space::BT2020NCL);
                assert!(
                    matches!(format, Pixel::YUV420P10LE | Pixel::P010LE),
                    "{format:?}"
                );

                let mut decoder = Decoder::open(&path, &DecodeOptions::default().deep(true))
                    .expect("opens what was just written");
                let back = decoder.next_frame().expect("decodes").expect("a frame");
                assert_eq!(back.signal(), signal);
                assert_eq!(back.depth(), Depth::Sixteen);
                let got =
                    f64::from(u16::from_le_bytes([back.pixels()[0], back.pixels()[1]])) / 65_535.0;
                assert!(
                    (got - level).abs() < 0.01,
                    "{} {signal:?}: {level} came back {got}",
                    codec.label()
                );

                if signal == Signal::Pq {
                    let input = ffmpeg::format::input(&path).expect("opens");
                    let stream = input.stream(0).expect("a stream");
                    // SAFETY: the parameters live as long as `input`; the
                    // side data is read, not kept.
                    unsafe {
                        let parameters = stream.parameters().as_ptr();
                        let found = |kind| {
                            ffmpeg::sys::av_packet_side_data_get(
                                (*parameters).coded_side_data,
                                (*parameters).nb_coded_side_data,
                                kind,
                            )
                        };
                        let display =
                            found(ffmpeg::sys::AVPacketSideDataType::MASTERING_DISPLAY_METADATA);
                        assert!(
                            !display.is_null(),
                            "{}: the mastering display",
                            codec.label()
                        );
                        let light = found(ffmpeg::sys::AVPacketSideDataType::CONTENT_LIGHT_LEVEL);
                        assert!(!light.is_null(), "{}: the light level", codec.label());
                        let light = &*(*light).data.cast::<ContentLight>();
                        assert!(
                            (190..=215).contains(&light.max_cll),
                            "MaxCLL of a 203-nit grey: {}",
                            light.max_cll
                        );
                    }
                }
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    /// A file written full range says so, and its levels come back as
    /// they went in - as do a video-range file's. The decoder reads the
    /// tag the encoder wrote, so a black that comes back black in both
    /// is a conversion that matched its tag in both.
    /// https://github.com/jub0t/Concat/issues/103
    #[test]
    fn each_range_is_tagged_and_keeps_its_levels() {
        use crate::decode::{DecodeOptions, Decoder, FrameSource};
        const GREYS: [u8; 4] = [0, 64, 128, 255];
        for range in ColorRange::ALL {
            let path =
                std::env::temp_dir().join(format!("concat-encode-range-{}.mp4", range.name()));
            let options = EncodeOptions {
                preset: "ultrafast".to_owned(),
                // Near-lossless, so a level is a level and not a quantiser's
                // guess at one; software, so the numbers are swscale's own.
                crf: 1,
                color_range: range,
                hardware: false,
                ..EncodeOptions::default()
            };
            let mut encoder = Encoder::create(&path, 64, 64, FrameRate::THIRTY, &options)
                .expect("the linked FFmpeg encodes h264");
            for grey in GREYS {
                let mut frame = Frame::black(64, 64);
                frame.fill([grey, grey, grey, 255]);
                for _ in 0..3 {
                    encoder.write_frame(&frame).expect("writes");
                }
            }
            encoder.finish().expect("finishes");

            let (.., tagged) = tags_of(&path);
            assert_eq!(
                tagged,
                range.as_ffmpeg(),
                "{}: the file says its range",
                range.name()
            );

            let mut decoder =
                Decoder::open(&path, &DecodeOptions::default().in_software()).expect("opens");
            let mut frames = Vec::new();
            while let Some(frame) = decoder.next_frame().expect("decodes") {
                frames.push(frame);
            }
            let _ = std::fs::remove_file(&path);
            assert_eq!(
                frames.len(),
                GREYS.len() * 3,
                "{}: every frame",
                range.name()
            );
            for (index, grey) in GREYS.iter().enumerate() {
                let [r, g, b, _] = frames[index * 3 + 1].pixel(32, 32).expect("inside");
                for channel in [r, g, b] {
                    assert!(
                        channel.abs_diff(*grey) <= 4,
                        "{}: grey {grey} came back as {channel}",
                        range.name()
                    );
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

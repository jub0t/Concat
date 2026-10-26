// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Pulling RGBA frames out of a file.
//!
//! One decoder does everything: it seeks to a start point and discards up to it frame-accurately,
//! turns the picture the way its rotation asks, scales, runs the clip's
//! effect chain through libavfilter, paces frames to a requested output rate
//! by duplicating and dropping, repeats a still forever, and stops after a
//! frame budget. Every frame comes with its real presentation timestamp.
//!
//! The codec runs on the platform's video hardware when the reader's
//! [`HwPolicy`] and the process-wide preference say so, and on the CPU
//! otherwise; see [`crate::hardware`]. A frame decoded on the device is
//! copied back before the filtergraph sees it, so everything from there on
//! is the same path, and a device that fails at any point hands the reader
//! to software from the frame it was at. The frames that come out are the
//! same either way, in count, size and timing; only the pixels may differ
//! by the rounding of a different colour conversion.
//!
//! ## On the unsafe in here
//!
//! The wrapper crate owns every pointer, and the one raw write here is the
//! threading setting the wrapper has no accessor for. What this module
//! owns is the order things happen in.

use std::path::{Path, PathBuf};

use concat_core::frame::{Frame, Signal};
use concat_core::time::{FrameRate, Rational};
use ffmpeg_the_third as ffmpeg;
use ffmpeg_the_third::codec::decoder;
use ffmpeg_the_third::filter;
use ffmpeg_the_third::format::{self, Pixel};
use ffmpeg_the_third::util::frame::video::Video;

use crate::error::{Error, Result};
use crate::feed;
use crate::ffi;
use crate::hardware::{self, Device, HwDevice, HwPolicy, Seat};

/// How [`Decoder::open_codec`] opens the codec.
enum Codec<'a> {
    /// libavcodec's own decoder for the stream, on the CPU.
    Software,
    /// libavcodec's own decoder, with this hwaccel device attached.
    On(&'a Device),
    /// A decoder by name in place of the built-in one: the platform's
    /// decoder wrapped as a codec, MediaCodec's.
    Named(&'static str),
}

/// Where a reader's codec runs when it is not plain software.
enum OnChip {
    /// A hwaccel: frames arrive in the device's memory, in this format,
    /// and are copied back.
    Accel(HwDevice, Pixel),
    /// The platform's own decoder behind a codec of its own: frames arrive
    /// in memory already. Holds its seat on the chip for as long as it
    /// lasts; see [`hardware::MEDIACODEC_READERS`].
    Wrapped { device: HwDevice, _seat: Seat },
}

impl OnChip {
    fn device(&self) -> HwDevice {
        match self {
            OnChip::Accel(device, _) | OnChip::Wrapped { device, .. } => *device,
        }
    }
}

/// Anything that yields frames in order.
///
/// The engine is written against this rather than against [`Decoder`], so
/// that a GPU-decode backend can be dropped in later, and so that tests can
/// feed synthetic frames without touching the disk.
pub trait FrameSource {
    /// Width of the frames this source produces.
    fn width(&self) -> u32;

    /// Height of the frames this source produces.
    fn height(&self) -> u32;

    /// The next frame, or `Ok(None)` at the end of the stream.
    fn next_frame(&mut self) -> Result<Option<Frame>>;

    /// When the frame just returned is meant to be shown, in the media's
    /// own clock. `None` when the container did not say.
    fn position(&self) -> Option<Rational> {
        None
    }
}

/// A source that can jump to an arbitrary point.
pub trait SeekableSource: FrameSource {
    /// Moves to `to`, or the nearest decodable point before it.
    ///
    /// The next [`FrameSource::next_frame`] returns the first frame at or
    /// after that point.
    fn seek(&mut self, to: Rational) -> Result<()>;
}

/// How to open a decoder.
#[derive(Clone, Debug, Default)]
pub struct DecodeOptions {
    /// Seek here before the first frame.
    pub start: Option<Rational>,
    /// Stop after this many frames.
    pub max_frames: Option<u64>,
    /// Scale to this size. Defaults to the file's own displayed dimensions.
    pub size: Option<(u32, u32)>,
    /// Pace output to this frame rate, duplicating or dropping source frames
    /// so that exactly one comes out per output frame instant.
    pub frame_rate: Option<FrameRate>,
    /// Repeat the last frame forever.
    ///
    /// A still image is a one-frame stream: decode it normally and you get a
    /// single frame and then end-of-stream, which on a timeline means the
    /// picture vanishes after 1/30th of a second. Looping makes it behave like
    /// footage of arbitrary length, and the caller stops pulling when the clip
    /// ends.
    pub looping: bool,
    /// Extra FFmpeg video filters, applied after any scaling. Empty for none.
    ///
    /// This is how video effects reach the pixels: the same "one FFmpeg
    /// string" design the audio filters use, so there is no second effect
    /// implementation to drift from. A final scale back to the requested
    /// size follows the chain, so an effect that changes the frame size
    /// cannot change what the caller receives.
    pub filter_chain: Option<String>,
    /// A filter chain applied *before* the fit scale, in the source's own
    /// pixels: where a crop lives, since a crop changes what the fit is of.
    pub pre_chain: Option<String>,
    /// Decode keyframes only, and let a seek land on the keyframe at or
    /// before its target rather than walking forward to the exact frame.
    ///
    /// For a thumbnail or a filmstrip tile, the exact frame is not worth
    /// what it costs: reaching it means decoding every frame from the
    /// keyframe before it at full resolution, up to a whole group of
    /// pictures - hundreds of frames on a screen recording - to keep one.
    /// With this set the codec skips everything that is not a keyframe and a
    /// seek returns the first picture it lands on, so a tile costs one
    /// decoded frame. Never for anything a person will watch or export: the
    /// frame is near the instant, not at it.
    pub keyframes_only: bool,
    /// Threads the codec may use, or zero to let it count the cores.
    ///
    /// Zero for anything a person is waiting on: a scrub wants its frame
    /// on every core. A few for a proxy being written in the background,
    /// where the whole point is that the machine stays usable meanwhile.
    pub threads: u16,
    /// Whether to decode on the platform's video hardware. The default
    /// follows the process-wide preference, [`crate::set_hardware_decode`],
    /// so the window's toggle reaches every reader without being passed
    /// through each one; a reader can insist on software or on a device.
    /// A device that fails is never an error: the reader carries on in
    /// software. See [`crate::hardware`].
    pub hardware: HwPolicy,
    /// What the picture's levels are taken to be, over whatever the file
    /// says: for a file tagged wrong, or tagged nothing and holding the
    /// other range, which is how a screen recording ends up grey where
    /// it should be black. `None` reads the file's tag and, where there
    /// is none, takes video range the way a player does. See
    /// [`ColorRange`].
    pub color_range: Option<ColorRange>,
    /// Keep a wide or HDR source's own signal, at sixteen bits a channel,
    /// rather than tone-map it to eight-bit Rec. 709 on the CPU: for a
    /// caller whose frames go straight to the GPU, which converts them as it
    /// uploads. Ignored for a narrow source, and where a chain or a
    /// pre-chain runs, since those are FFmpeg filters on eight bits. See
    /// [`Frame::signal`].
    pub deep: bool,
}

impl DecodeOptions {
    /// Seeks to `start` before decoding.
    pub fn starting_at(mut self, start: Rational) -> Self {
        self.start = Some(start);
        self
    }

    /// Stops after `count` frames.
    pub fn limited_to(mut self, count: u64) -> Self {
        self.max_frames = Some(count);
        self
    }

    /// Scales output to `width` by `height`.
    pub fn scaled_to(mut self, width: u32, height: u32) -> Self {
        self.size = Some((width, height));
        self
    }

    /// Paces output to `rate`.
    pub fn at_rate(mut self, rate: FrameRate) -> Self {
        self.frame_rate = Some(rate);
        self
    }

    /// Repeats the last frame forever. See [`DecodeOptions::looping`].
    pub fn repeating(mut self) -> Self {
        self.looping = true;
        self
    }

    /// Applies `chain` before the fit scale. See [`DecodeOptions::pre_chain`].
    pub fn prefiltered(mut self, chain: impl Into<String>) -> Self {
        let chain = chain.into();
        self.pre_chain = (!chain.is_empty()).then_some(chain);
        self
    }

    /// Applies `chain` after scaling. See [`DecodeOptions::filter_chain`].
    pub fn filtered(mut self, chain: impl Into<String>) -> Self {
        let chain = chain.into();
        self.filter_chain = (!chain.is_empty()).then_some(chain);
        self
    }

    /// Decodes keyframes only. See [`DecodeOptions::keyframes_only`].
    pub fn nearest_keyframes(mut self) -> Self {
        self.keyframes_only = true;
        self
    }

    /// Limits the codec to `threads`. See [`DecodeOptions::threads`].
    pub fn threaded(mut self, threads: u16) -> Self {
        self.threads = threads;
        self
    }

    /// Decodes on `device`, whatever the preference says, and in software
    /// if the device will not. See [`DecodeOptions::hardware`].
    pub fn accelerated(mut self, device: HwDevice) -> Self {
        self.hardware = HwPolicy::Device(device);
        self
    }

    /// Decodes in software, whatever the preference says.
    pub fn in_software(mut self) -> Self {
        self.hardware = HwPolicy::Software;
        self
    }

    /// Keeps a wide or HDR source deep; see [`DecodeOptions::deep`].
    pub fn deep(mut self, deep: bool) -> Self {
        self.deep = deep;
        self
    }

    /// Reads the picture as `range`, whatever the file says; `None` goes
    /// back to the file's tag. See [`DecodeOptions::color_range`].
    pub fn in_range(mut self, range: Option<ColorRange>) -> Self {
        self.color_range = range;
        self
    }
}

/// The levels a picture's numbers span: video range, black at 16 and
/// white at 235 of 255, which is what broadcast, cameras and every
/// player expect; or full range, 0 to 255, which screen recorders and
/// some phones write. A file tagged the wrong one, or tagged nothing and
/// holding the other, shows grey where it should show black or clips
/// its shadows and highlights: the washed-out or crushed picture.
/// https://github.com/jub0t/Concat/issues/103
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ColorRange {
    /// 16-235: "tv", "MPEG", "video" or "limited" range. The default
    /// everywhere, and what an export writes unless told otherwise.
    #[default]
    Limited,
    /// 0-255: "pc", "JPEG" or "full" range.
    Full,
}

impl ColorRange {
    /// Every range, in the order a menu lists them.
    pub const ALL: [ColorRange; 2] = [ColorRange::Limited, ColorRange::Full];

    /// The range a stream is taken to span when its file says nothing,
    /// read off what the codec and its pixels are by convention - the
    /// call Resolve's "Auto" makes. JPEG and the picture formats are full
    /// range by definition, as is anything RGB rather than YCbCr: there
    /// is no matrix for RGB to be limited under. Every YCbCr video codec
    /// is limited unless tagged otherwise, which is what a player assumes
    /// and what a camera writes. A screen recording holding full-range
    /// pixels and saying nothing is the one case this cannot know, and is
    /// what the override is for.
    /// https://github.com/jub0t/Concat/issues/103
    pub fn implied(codec: ffmpeg::codec::Id, pixel: Pixel) -> ColorRange {
        use ffmpeg::codec::Id;
        let picture = matches!(
            codec,
            Id::MJPEG | Id::PNG | Id::APNG | Id::BMP | Id::GIF | Id::TIFF | Id::WEBP
        );
        // The `j` formats are libavcodec's own way of saying full range;
        // a hardware decoder hands the same stream back as plain NV12,
        // which is why the codec is asked as well as the pixels.
        let full_pixels = matches!(
            pixel,
            Pixel::YUVJ420P
                | Pixel::YUVJ422P
                | Pixel::YUVJ444P
                | Pixel::YUVJ440P
                | Pixel::YUVJ411P
                | Pixel::RGB24
                | Pixel::BGR24
                | Pixel::RGBA
                | Pixel::BGRA
                | Pixel::ARGB
                | Pixel::ABGR
                | Pixel::ZRGB
                | Pixel::ZBGR
                | Pixel::RGB8
                | Pixel::BGR8
                | Pixel::PAL8
                | Pixel::RGB48BE
                | Pixel::RGB48LE
                | Pixel::RGBA64BE
                | Pixel::RGBA64LE
                | Pixel::RGB565BE
                | Pixel::RGB565LE
                | Pixel::RGB555BE
                | Pixel::RGB555LE
                | Pixel::GBRP
                | Pixel::GBRAP
                | Pixel::GBRP10BE
                | Pixel::GBRP10LE
                | Pixel::GBRP12BE
                | Pixel::GBRP12LE
                | Pixel::GBRP16BE
                | Pixel::GBRP16LE
                | Pixel::GBRAP10BE
                | Pixel::GBRAP10LE
                | Pixel::GBRAP12BE
                | Pixel::GBRAP12LE
                | Pixel::GBRAP16BE
                | Pixel::GBRAP16LE
                | Pixel::X2RGB10LE
                | Pixel::X2RGB10BE
                | Pixel::X2BGR10LE
                | Pixel::X2BGR10BE
        );
        if picture || full_pixels {
            ColorRange::Full
        } else {
            ColorRange::Limited
        }
    }

    /// The name a document or a request stores.
    pub fn name(self) -> &'static str {
        match self {
            ColorRange::Limited => "limited",
            ColorRange::Full => "full",
        }
    }

    /// The range a stored name means, or `None` for one nobody stores.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "limited" | "tv" | "video" | "mpeg" => Some(ColorRange::Limited),
            "full" | "pc" | "jpeg" => Some(ColorRange::Full),
            _ => None,
        }
    }

    /// What the standard is called on a label.
    pub fn label(self) -> &'static str {
        match self {
            ColorRange::Limited => "Limited",
            ColorRange::Full => "Full",
        }
    }

    /// libavcodec's spelling.
    pub fn as_ffmpeg(self) -> ffmpeg::color::Range {
        match self {
            ColorRange::Limited => ffmpeg::color::Range::MPEG,
            ColorRange::Full => ffmpeg::color::Range::JPEG,
        }
    }

    /// libavcodec's spelling read back: `None` for a stream that says
    /// nothing, or something neither of these.
    pub fn from_ffmpeg(range: ffmpeg::color::Range) -> Option<Self> {
        match range {
            ffmpeg::color::Range::MPEG => Some(ColorRange::Limited),
            ffmpeg::color::Range::JPEG => Some(ColorRange::Full),
            _ => None,
        }
    }

    /// The `scale` filter's word for it.
    fn scale_name(self) -> &'static str {
        match self {
            ColorRange::Limited => "tv",
            ColorRange::Full => "pc",
        }
    }
}

/// How a stream says its colour is encoded: the tags in its container,
/// as libavcodec reads them. Unspecified where it says nothing, which is
/// most SD and a lot of HD, and then it is taken for BT.709.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ColorSignal {
    /// The gamut.
    pub primaries: ffmpeg::color::Primaries,
    /// The transfer curve: BT.709 for SDR, PQ or HLG for HDR.
    pub transfer: ffmpeg::color::TransferCharacteristic,
    /// The YCbCr matrix.
    pub matrix: ffmpeg::color::Space,
    /// Video (16-235) or full range.
    pub range: ffmpeg::color::Range,
    /// `range` is a reading of the codec rather than the file's tag or
    /// the caller's word: the file said nothing, and
    /// [`ColorRange::implied`] answered for it. The frames then carry no
    /// tag either, so a full reading has to be told to the fit by name.
    pub implied: bool,
}

impl ColorSignal {
    /// Whether the picture is outside BT.709 as the engine works in it:
    /// an HDR transfer, or the BT.2020 gamut. Both need converting on
    /// the way in, or they arrive flat, dim and the wrong colour - the
    /// 10-bit PQ file from a phone, decoded as if it were a 709 one.
    pub fn is_wide(&self) -> bool {
        use ffmpeg::color::{Primaries, TransferCharacteristic as Transfer};
        matches!(self.transfer, Transfer::SMPTE2084 | Transfer::ARIB_STD_B67)
            || self.primaries == Primaries::BT2020
    }

    /// Whether the transfer is a high dynamic range one: PQ or HLG.
    pub fn is_hdr(&self) -> bool {
        use ffmpeg::color::TransferCharacteristic as Transfer;
        matches!(self.transfer, Transfer::SMPTE2084 | Transfer::ARIB_STD_B67)
    }

    /// The `scale` filter's arguments that bring a wide picture into
    /// BT.709: the source's tags named where it has them - `auto` reads
    /// the frames' own where it does not - and BT.709 out, tone-mapped
    /// perceptually rather than clipped. swscale has done this itself
    /// since FFmpeg 7.1, so it costs no library the bundles lack.
    fn bt709_args(self) -> String {
        use ffmpeg::color::{Primaries, Range, Space, TransferCharacteristic as Transfer};
        let primaries = match self.primaries {
            Primaries::BT2020 => "bt2020",
            Primaries::BT709 => "bt709",
            Primaries::SMPTE432 => "smpte432",
            _ => "auto",
        };
        let transfer = match self.transfer {
            Transfer::SMPTE2084 => "smpte2084",
            Transfer::ARIB_STD_B67 => "arib-std-b67",
            Transfer::BT2020_10 => "bt2020-10",
            Transfer::BT2020_12 => "bt2020-12",
            Transfer::BT709 => "bt709",
            _ => "auto",
        };
        let matrix = match self.matrix {
            Space::BT2020NCL => "bt2020nc",
            Space::BT2020CL => "bt2020c",
            Space::BT709 => "bt709",
            _ => "auto",
        };
        let range = match self.range {
            Range::MPEG => "tv",
            Range::JPEG => "pc",
            _ => "auto",
        };
        format!(
            ":in_primaries={primaries}:in_transfer={transfer}:in_color_matrix={matrix}\
             :in_range={range}:out_primaries=bt709:out_transfer=bt709\
             :out_color_matrix=bt709:out_range=tv:intent=perceptual"
        )
    }

    /// What a deep frame of this source says it holds, when the source is
    /// one the compositor converts on the GPU: Rec. 2020 primaries, with
    /// PQ, HLG or a gamma. `None` for anything else - a P3 source, say -
    /// which keeps the eight-bit conversion here.
    pub fn deep_signal(&self) -> Option<Signal> {
        use ffmpeg::color::{Primaries, TransferCharacteristic as Transfer};
        match self.transfer {
            Transfer::SMPTE2084 => Some(Signal::Pq),
            Transfer::ARIB_STD_B67 => Some(Signal::Hlg),
            _ if self.primaries == Primaries::BT2020 => Some(Signal::SdrWide),
            _ => None,
        }
    }

    /// The `scale` filter's arguments for a deep frame: the source's own
    /// matrix and range, to RGB and nothing more - its primaries and
    /// transfer are left as they are, for the GPU to convert.
    fn native_args(self) -> String {
        use ffmpeg::color::{Range, Space};
        let matrix = match self.matrix {
            Space::BT2020CL => "bt2020c",
            Space::BT709 => "bt709",
            _ => "bt2020nc",
        };
        let range = match self.range {
            Range::JPEG => "pc",
            _ => "tv",
        };
        format!(":in_color_matrix={matrix}:in_range={range}")
    }
}

/// The signal a deep frame is decoded in, when `options` asks for one and
/// the source is one the compositor converts: nothing in the decoder's
/// graph may need eight bits, so no chain and no pre-chain.
fn deep_output(options: &DecodeOptions, color: Option<&ColorSignal>) -> Option<Signal> {
    if !options.deep || options.filter_chain.is_some() || options.pre_chain.is_some() {
        return None;
    }
    color?.deep_signal()
}

/// The filtergraph between the decoder and the caller, as one string.
///
/// Rotation first, so everything downstream sees the picture the way a
/// player would; then scale-to-fit, which is also where a wide or HDR
/// picture is brought into BT.709; then the effect chain at output
/// resolution (cheaper than filtering the source size, and parameters mean
/// the same thing at every export size); then the guard scale that pins the
/// frame size the caller was promised; then RGBA.
fn video_filter(
    rotation: i64,
    options: &DecodeOptions,
    width: u32,
    height: u32,
    color: Option<&ColorSignal>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(turn) = ffi::rotation_filters(rotation) {
        parts.push(turn.to_owned());
    }
    if let Some(pre) = &options.pre_chain {
        parts.push(pre.clone());
    }
    // A wide or HDR picture is converted whole, its range among the terms.
    // A BT.709 one is left to swscale's reading of the frames' own tag,
    // unless the caller has said what the levels really are: then the
    // range is named and the tag ignored, which is the whole of the fix
    // for a file that lies about it. A range read off the codec in place
    // of a tag is named only when it is full: the frames carry no tag for
    // swscale to read, and untagged already means limited to it.
    let deep = deep_output(options, color);
    let convert = match (color, options.color_range) {
        (Some(signal), _) if deep.is_some() => signal.native_args(),
        (Some(signal), _) if signal.is_wide() => signal.bt709_args(),
        (_, Some(range)) => format!(":in_range={}", range.scale_name()),
        (Some(signal), None) if signal.implied && signal.range == ffmpeg::color::Range::JPEG => {
            ":in_range=pc".to_owned()
        }
        _ => String::new(),
    };
    parts.push(format!("scale={width}:{height}:flags=bilinear{convert}"));
    if let Some(chain) = &options.filter_chain {
        parts.push(chain.clone());
        parts.push(format!("scale={width}:{height}:flags=bilinear"));
    }
    parts.push(
        if deep.is_some() {
            "format=rgba64le"
        } else {
            "format=rgba"
        }
        .to_owned(),
    );
    parts.join(",")
}

/// The libvpx decoder that reads this stream's WebM alpha layer, when it
/// has one the built-in decoder would drop.
///
/// A WebM track keeps its alpha in a second plane and marks itself
/// `alpha_mode=1`; libavcodec's own VP8 and VP9 decoders decode only the
/// colour plane, and a hardware decoder drops the layer too, so the picture
/// arrives opaque. libvpx reads both planes. See
/// <https://github.com/jub0t/Concat/issues/429>.
fn alpha_decoder(stream: &ffmpeg::format::stream::Stream<'_>) -> Option<&'static str> {
    let alpha = stream
        .metadata()
        .get("alpha_mode")
        .is_some_and(|mode| mode.trim() == "1");
    if !alpha {
        return None;
    }
    match stream.parameters().id() {
        ffmpeg::codec::Id::VP8 => Some("libvpx"),
        ffmpeg::codec::Id::VP9 => Some("libvpx-vp9"),
        _ => None,
    }
}

/// Where a container seek was asked to land, until the first picture after
/// it shows whether it did.
#[derive(Clone, Copy, Debug)]
struct Landing {
    /// The instant the caller wants, measured from the stream's first frame.
    target: Rational,
    /// The instant the container was last asked for: the target itself, or
    /// a point before it once the container has overshot.
    asked: Rational,
}

/// One decoded picture, before the filtergraph.
struct Source {
    frame: Video,
    pts: Option<Rational>,
}

/// Decodes a file through libavcodec and libavfilter.
pub struct Decoder {
    path: PathBuf,
    input: format::context::Input,
    stream: usize,
    time_base: ffmpeg::Rational,
    /// Where the file's timestamps start; every timestamp read has this
    /// taken off and every seek has it put back, so the file reads from
    /// its own first frame, in step with its sound. See `ffi::origin`.
    start: Rational,
    decoder: decoder::Video,
    rotation: i64,
    options: DecodeOptions,
    width: u32,
    height: u32,
    /// Built on the first frame, once its format and size are known;
    /// rebuilt if a later frame differs.
    graph: Option<(Pixel, u32, u32, filter::Graph)>,
    /// Frames before this instant are decoded and discarded - the exact
    /// half of a seek, after the container landed on a keyframe.
    discard_before: Option<Rational>,
    /// Frames at or before this instant are discarded too: where a reader
    /// that moved to software mid-file picks up, after the frame the
    /// device last gave it.
    discard_through: Option<Rational>,
    /// The seek whose landing is still to be checked; see
    /// [`Decoder::retreat`].
    landing: Option<Landing>,
    /// The last picture short of a seek's target, kept back in case none
    /// lands on the target itself; see [`Decoder::next_source`].
    short: Option<Video>,
    /// A picture already out of the codec, to be handed on next: the one
    /// past the target that sent the kept-back picture out first.
    stashed: Option<Video>,
    /// Where the codec runs when it is not on the CPU, or `None` in
    /// software, which is also where a failed device ends.
    hardware: Option<OnChip>,
    /// A packet the codec refused while full, to send again; see
    /// [`crate::feed`].
    held: feed::Held<ffmpeg::Packet>,
    /// Whether the codec has been handed anything since it opened or was
    /// last flushed. A codec with nothing in it is not flushed: MediaCodec
    /// flushed before its first output has lost the stream's parameter
    /// sets and must be handed them again, which a seek straight after
    /// opening - every reader opened at a start point - would do.
    fed: bool,
    /// The last picture's timestamp, for picking up in software after it.
    last_pts: Option<Rational>,
    /// The output frame instant the pacer is at, when pacing.
    tick: u64,
    origin: Rational,
    /// The source frame the pacer is showing, and the one after it.
    current: Option<Source>,
    pending: Option<Source>,
    source_done: bool,
    produced: u64,
    position: Option<Rational>,
    /// What the stream says its colour is.
    color: ColorSignal,
}

impl Decoder {
    /// Opens `path` for decoding.
    pub fn open(path: impl AsRef<Path>, options: &DecodeOptions) -> Result<Self> {
        ffi::init();
        // No slot rule on the chain, unlike the audio mix's: this graph is
        // the clip's own, in to out, and a chain that branches and rejoins
        // with labels - a bloom is a split, a blur and a screen blend - is
        // the shape half the catalogue's effects take. Held to the mix's
        // rule, every one of them failed to open on the CPU renderer. A
        // chain that is not a graph fails at the parse, with its error.

        // An SVG is read as its drawing, which FFmpeg can open; see svg.rs.
        let drawn = crate::svg::readable(path.as_ref())?;
        let path = drawn.as_path();
        let input = ffmpeg::format::input(path).map_err(|error| ffi::fail("open", path, error))?;
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or_else(|| Error::NoVideoStream {
                path: path.to_path_buf(),
            })?;
        let stream_index = stream.index();
        let time_base = stream.time_base();
        let start = ffi::origin(&input);
        let rotation = ffi::rotation(&stream);
        // Wide pixels are shown square: see `ffi::sample_aspect`.
        let aspect = ffi::sample_aspect(&stream);
        let coded = {
            let parameters = stream.parameters();
            (parameters.width(), parameters.height())
        };

        // The device the policy asks for this stream, if the process has
        // one; see `hardware::hardware_wins` for which streams it wants. A
        // codec that will not open on it opens in software instead, and the
        // reader is none the wiser past a line in the log.
        let (codec, format) = {
            let parameters = stream.parameters();
            // SAFETY: a live stream's codec parameters; for a video stream
            // `format` holds an AVPixelFormat.
            let raw = unsafe { (*parameters.as_ptr()).format };
            (
                parameters.id(),
                Pixel::from(ffmpeg::sys::AVPixelFormat(raw)),
            )
        };
        let software = || Self::open_codec(path, &stream, options, Codec::Software);
        // A WebM alpha layer is a plane the built-in VP8 and VP9 decoders
        // do not read, and no hwaccel hands it back either: the picture
        // would arrive opaque and cover the lanes below. libvpx reads it,
        // in software, so a track that says it has one goes there and to
        // no device.
        // https://github.com/jub0t/Concat/issues/429
        let (decoder, accelerated) = match (
            alpha_decoder(&stream),
            options.hardware.device_for(codec, format),
        ) {
            (Some(name), _) => match Self::open_codec(path, &stream, options, Codec::Named(name)) {
                Ok((decoder, _)) => (decoder, None),
                Err(error) => {
                    // A build without libvpx still plays the picture, with
                    // the alpha layer dropped rather than the file refused.
                    log::warn!("{name} does not open here: {error}");
                    software()?
                }
            },
            // MediaCodec: its own decoder, by name, when a seat is free.
            // Not for a keyframes-only reader: the wrapper ignores
            // `skip_frame`, so a filmstrip tile would decode its whole
            // group of pictures instead of one frame, and its pipeline
            // holds the first picture back until several packets are in -
            // a one-frame grab is the chip's worst case and the CPU's best.
            (None, Some(device @ HwDevice::MediaCodec)) => {
                let wrapped = hardware::mediacodec_decoder(codec, format)
                    .filter(|_| !options.keyframes_only)
                    .filter(|&name| {
                        hardware::mediacodec_works(name, || {
                            first_picture_within(path, name, hardware::MEDIACODEC_FIRST_PICTURE)
                        })
                    })
                    .and_then(|name| hardware::mediacodec_seat().map(|seat| (name, seat)));
                match wrapped {
                    Some((name, seat)) => {
                        match Self::open_codec(path, &stream, options, Codec::Named(name)) {
                            Ok((decoder, _)) => {
                                hardware::note_wrapped(device, name);
                                (
                                    decoder,
                                    Some(OnChip::Wrapped {
                                        device,
                                        _seat: seat,
                                    }),
                                )
                            }
                            Err(error) => {
                                // Most often the phone's sessions are all
                                // taken, by this app or another.
                                hardware::note_fallback(
                                    device,
                                    "open the decoder",
                                    &error.to_string(),
                                );
                                software()?
                            }
                        }
                    }
                    None => software()?,
                }
            }
            (None, Some(device)) => match hardware::device(device) {
                Some(device) => {
                    match Self::open_codec(path, &stream, options, Codec::On(&device)) {
                        Ok(opened) => opened,
                        Err(error) => {
                            hardware::note_fallback(
                                device.kind(),
                                "open the decoder",
                                &error.to_string(),
                            );
                            software()?
                        }
                    }
                }
                None => software()?,
            },
            (None, None) => software()?,
        };
        // The caller's word over the file's, where the caller has one: the
        // file's tag is what the override exists to correct. Where neither
        // says, the codec's convention does; see `ColorRange::implied`.
        // Read off the stream, not the opened codec: MediaCodec's wrapper
        // writes the chip's own idea of the colour into the codec as it
        // starts - limited range on a stream that says nothing, NV12 for
        // the pixels - which is not what the file says.
        // SAFETY: a live stream's codec parameters; plain enum fields.
        let (tagged, primaries, transfer, matrix) = unsafe {
            let raw = stream.parameters().as_ptr();
            (
                ffmpeg::color::Range::from((*raw).color_range),
                ffmpeg::color::Primaries::from((*raw).color_primaries),
                ffmpeg::color::TransferCharacteristic::from((*raw).color_trc),
                ffmpeg::color::Space::from((*raw).color_space),
            )
        };
        let implied = options.color_range.is_none() && tagged == ffmpeg::color::Range::Unspecified;
        let color = ColorSignal {
            primaries,
            transfer,
            matrix,
            range: match options.color_range {
                Some(range) => range.as_ffmpeg(),
                None if implied => ColorRange::implied(codec, format).as_ffmpeg(),
                None => tagged,
            },
            implied,
        };

        let (width, height) = match options.size {
            Some(size) => size,
            None => ffi::displayed(coded.0, coded.1, aspect, rotation),
        };
        if width == 0 || height == 0 {
            return Err(Error::Probe {
                path: path.to_path_buf(),
                detail: "video stream has no usable size".to_owned(),
            });
        }

        let mut this = Self {
            path: path.to_path_buf(),
            input,
            stream: stream_index,
            time_base,
            start,
            decoder,
            rotation,
            options: options.clone(),
            width,
            height,
            graph: None,
            discard_before: None,
            discard_through: None,
            landing: None,
            short: None,
            stashed: None,
            hardware: accelerated,
            held: None,
            fed: false,
            last_pts: None,
            tick: 0,
            origin: Rational::ZERO,
            current: None,
            pending: None,
            source_done: false,
            produced: 0,
            position: None,
            color,
        };
        if let Some(start) = options.start
            && !start.is_zero()
        {
            this.jump(start)?;
        }
        Ok(this)
    }

    /// Opens the stream's codec as `codec` says. Gives back the opened
    /// decoder and, when it is on a hwaccel device, where.
    fn open_codec(
        path: &Path,
        stream: &ffmpeg::format::stream::Stream<'_>,
        options: &DecodeOptions,
        codec: Codec<'_>,
    ) -> Result<(decoder::Video, Option<OnChip>)> {
        let mut context = ffmpeg::codec::Context::from_parameters(stream.parameters())
            .map_err(|error| ffi::fail("codec parameters", path, error))?;
        // Every core. libavcodec's API opens a decoder on one thread unless
        // told otherwise (the `ffmpeg` tool turns threads on for itself), and
        // a seek decodes every frame from the keyframe before it: on one
        // thread a 4K H.264 file decoded at about 87 frames a second, and a
        // scrub waited 400 ms for its frame. Frame threads for that walk,
        // slice threads for files cut into slices; zero lets the codec
        // count, and a caller working in the background says how many.
        // SAFETY: the context is not opened yet, which is when threading is
        // set, and both fields are plain integers.
        unsafe {
            let raw = context.as_mut_ptr();
            (*raw).thread_type = ffmpeg::sys::FF_THREAD_FRAME | ffmpeg::sys::FF_THREAD_SLICE;
            (*raw).thread_count = i32::from(options.threads);
        }
        let (accelerated, named) = match codec {
            Codec::Software => (None, None),
            Codec::On(device) => (
                device
                    .attach(&mut context)
                    .map(|format| OnChip::Accel(device.kind(), format)),
                None,
            ),
            Codec::Named(name) => {
                let found = decoder::find_by_name(name).ok_or_else(|| Error::Missing {
                    what: "decoder",
                    name: name.to_owned(),
                })?;
                // MediaCodec counts time in microseconds; told the stream's
                // clock, the wrapper converts each packet's timestamp there
                // and every frame's back, exactly for any clock coarser than
                // a microsecond, which is every one a container uses.
                // SAFETY: the context is not opened yet; a plain rational.
                unsafe {
                    (*context.as_mut_ptr()).pkt_timebase = stream.time_base().into();
                }
                (None, Some(found))
            }
        };
        let opened = match named {
            Some(found) => context
                .decoder()
                .open_as(found)
                .and_then(|opened| opened.video()),
            None => context.decoder().video(),
        };
        let mut decoder = opened.map_err(|error| ffi::fail("open decoder", path, error))?;
        if options.keyframes_only {
            decoder.skip_frame(ffmpeg::Discard::NonKey);
        }
        Ok((decoder, accelerated))
    }

    /// How many frames have been produced so far.
    pub const fn produced(&self) -> u64 {
        self.produced
    }

    /// The device the codec is decoding on, or `None` in software: also
    /// the answer once a device has failed and the reader has moved on
    /// without it, or once the codec has shown, with its first frame, that
    /// it declined the device for this stream.
    pub fn hardware(&self) -> Option<HwDevice> {
        self.hardware.as_ref().map(OnChip::device)
    }

    /// Reopens the codec in software and carries on from `from`: the frame
    /// at that instant is the next one out, or the one after it with
    /// `after`. The pacer's state is kept, so what comes out is what would
    /// have come out of the device. `None` starts over from the origin,
    /// which is the best a stream with no timestamps allows.
    fn resume_in_software(&mut self, from: Option<Rational>, after: bool) -> Result<()> {
        let decoder = {
            let stream = self
                .input
                .stream(self.stream)
                .ok_or_else(|| Error::NoVideoStream {
                    path: self.path.clone(),
                })?;
            Self::open_codec(&self.path, &stream, &self.options, Codec::Software)?.0
        };
        let at = from.unwrap_or(self.origin);
        self.seek_to(at)?;
        self.decoder = decoder;
        // The old codec, and its seat on the chip, go now.
        self.hardware = None;
        self.held = None;
        self.fed = false;
        self.short = None;
        self.stashed = None;
        if !self.options.keyframes_only {
            if after {
                self.discard_through = Some(at);
            } else {
                self.discard_before = Some(self.discard_before.map_or(at, |before| before.max(at)));
            }
        }
        Ok(())
    }

    /// The container seek to `target`, in `AV_TIME_BASE` ticks: the keyframe
    /// at or before it, never one after, so the frames short of the target
    /// can be decoded and dropped. The MP4s VLC's stream output writes leave
    /// their first picture out of the keyframe table, and FFmpeg then finds
    /// no keyframe before any target short of the second one; those seeks
    /// go to 0 instead, which their edit list - the picture starts 14 ms
    /// in - still lets land on the first picture. Without it the monitor
    /// stayed black for those seconds and the media thumbnail never came.
    fn seek_input(&mut self, target: i64) -> Result<()> {
        self.input
            .seek(target, ..=target)
            .or_else(|error| {
                log::debug!(
                    "{}: no keyframe at or before {target}, reading from the top ({error})",
                    self.path.display()
                );
                self.input.seek(0, ..=0).map_err(|_| error)
            })
            .map_err(|error| ffi::fail("seek", &self.path, error))
    }

    /// Back to the first packet of the file.
    ///
    /// A stream whose clock may jump - MPEG-TS and MPEG-PS, which say so
    /// with `AVFMT_TS_DISCONT` - goes back by byte. Its first timestamp is
    /// 1.4 s or more in, and a seek to it by time is a search over the
    /// file's timestamps that, asked for the very first one (or for a
    /// microsecond short of it, which is where rounding `start` down to
    /// whole microseconds puts the request), finds nothing at or before it
    /// and leaves the demuxer at the end of the file: no picture at all.
    /// Everything else seeks by time to its own first timestamp.
    fn seek_top(&mut self) -> Result<()> {
        // SAFETY: the input is open for the decoder's lifetime, so its
        // format pointer is valid; `flags` is a plain integer.
        let by_byte = unsafe {
            let format = (*self.input.as_ptr()).iformat;
            !format.is_null() && {
                let flags = (*format).flags;
                flags & ffmpeg::sys::AVFMT_TS_DISCONT != 0
                    && flags & ffmpeg::sys::AVFMT_NO_BYTE_SEEK == 0
            }
        };
        if by_byte {
            // SAFETY: an open input; a byte seek to offset 0 of every
            // stream.
            let done = unsafe {
                ffmpeg::sys::av_seek_frame(
                    self.input.as_mut_ptr(),
                    -1,
                    0,
                    ffmpeg::sys::AVSEEK_FLAG_BYTE,
                )
            };
            if done >= 0 {
                return Ok(());
            }
        }
        self.seek_input(ffi::av_ticks(self.start))
    }

    /// The container seek to `at`, measured from the stream's first frame,
    /// with its landing left for the first picture after it to check. The
    /// start of the stream is the top of the file, which needs no check.
    fn seek_to(&mut self, at: Rational) -> Result<()> {
        if at <= Rational::ZERO {
            self.landing = None;
            return self.seek_top();
        }
        self.landing = Some(Landing {
            target: at,
            asked: at,
        });
        self.seek_input(ffi::av_ticks(at + self.start))
    }

    /// Seeks again, further back, after the container landed past the
    /// picture `landing` wanted - or past the end of the picture stream.
    ///
    /// A seek asks for the keyframe at or before the target, and MP4, MOV
    /// and Matroska keep an index that answers exactly. MPEG-TS and MPEG-PS
    /// keep none: libavformat searches the file's timestamps and lands a
    /// group of pictures late, on the next keyframe, or - with one keyframe
    /// every few seconds - at the end of the file. A camera's MTS showed
    /// the wrong moment, or no picture at all, wherever a scrub or a
    /// clip's in-point sought to, and an MPEG-2 .mpg started twelve frames
    /// in (#457). The first picture after a seek tells: one timestamped
    /// after the target means the picture on screen at the target was
    /// skipped. The retry asks for a point a second before the target, then
    /// four, then sixteen and so on, and for the top of the file once that
    /// passes the start, which always lands; the frames short of the target
    /// are decoded and dropped as after any seek.
    fn retreat(&mut self, landing: Landing) -> Result<()> {
        let short = landing.target - landing.asked;
        let step = if short.is_zero() {
            Rational::from_int(1)
        } else {
            short * Rational::from_int(4)
        };
        let back = landing.target - step;
        log::debug!(
            "{}: a seek for {:.3} s landed past it; seeking from {:.3} s",
            self.path.display(),
            landing.target.as_f64(),
            back.as_f64().max(0.0)
        );
        if back <= Rational::ZERO {
            self.landing = None;
            self.seek_top()?;
        } else {
            self.landing = Some(Landing {
                target: landing.target,
                asked: back,
            });
            self.seek_input(ffi::av_ticks(back + self.start))?;
        }
        if self.fed {
            self.decoder.flush();
            self.fed = false;
        }
        self.held = None;
        self.short = None;
        self.stashed = None;
        Ok(())
    }

    /// One tick of the stream's clock, and never more than a millisecond:
    /// how far past an instant a frame meant for it can be stamped by a
    /// container that rounds - Matroska, WebM, FLV and ASF to whole
    /// milliseconds - without being taken for the frame after it.
    fn slack(&self) -> Rational {
        ffi::seconds(1, self.time_base)
            .unwrap_or(Rational::ZERO)
            .min(Rational::new(1, 1000))
    }

    /// The container seek, and the state reset that goes with it.
    fn jump(&mut self, to: Rational) -> Result<()> {
        self.seek_to(to)?;
        // Every picture and packet the codec holds is from before the
        // seek: out, and so is the packet it refused. For MediaCodec this
        // is the codec's own flush, which it needs before a jump.
        if self.fed {
            self.decoder.flush();
            self.fed = false;
        }
        self.held = None;
        self.short = None;
        self.stashed = None;
        // Keyframes only: the picture the container landed on is the one
        // wanted, and discarding up to the target would throw it away and
        // wait for the *next* keyframe, a whole group of pictures late.
        self.discard_before = (!self.options.keyframes_only).then_some(to);
        self.discard_through = None;
        self.last_pts = None;
        self.origin = to;
        self.tick = 0;
        self.current = None;
        self.pending = None;
        self.source_done = false;
        self.position = None;
        Ok(())
    }

    /// The next picture out of the codec, or `None` at the end of the file.
    /// After a seek the first is the picture on screen at the target: the
    /// one stamped there, or the last one before it when none is - frames
    /// before that never come out of here. A frame from a hwaccel device is
    /// copied back to memory before it does.
    fn next_source(&mut self) -> Result<Option<Source>> {
        loop {
            // Packets for other streams are skipped; the end of the file
            // drains the codec so its last few frames come out at all; a
            // packet the codec refuses while full is kept and sent again.
            let decoded = match self.stashed.take() {
                Some(frame) => Ok(Some(frame)),
                None => {
                    self.fed = true;
                    let read = || feed::read_packet(&mut self.input, self.stream);
                    feed::next_frame(&mut self.decoder, &mut self.held, read)
                }
            };
            let mut frame = match decoded {
                Ok(Some(frame)) => frame,
                // Nothing at all after a seek: the container landed past
                // the end of the picture stream, not on the keyframe asked
                // for.
                Ok(None) => match (self.landing.take(), self.short.take()) {
                    (Some(landing), _) => {
                        self.retreat(landing)?;
                        continue;
                    }
                    // The picture ended short of the target: its last frame
                    // is the one on screen there.
                    (None, Some(short)) => {
                        self.discard_before = None;
                        short
                    }
                    (None, None) => return Ok(None),
                },
                Err(feed::Stall::Read(error)) => return Err(ffi::fail("read", &self.path, error)),
                Err(stall) if self.hardware.is_some() => {
                    // The device choked on a stream it had accepted.
                    // Software takes over after the last frame the reader
                    // was given.
                    let device = self.hardware.take().expect("checked").device();
                    let (stage, detail) = match &stall {
                        feed::Stall::Send(error) => ("take a packet", error.to_string()),
                        feed::Stall::Receive(error) => ("decode a frame", error.to_string()),
                        _ => ("keep decoding", "it stopped taking packets".to_owned()),
                    };
                    hardware::note_fallback(device, stage, &detail);
                    self.resume_in_software(self.last_pts, self.last_pts.is_some())?;
                    continue;
                }
                Err(stall) => return Err(feed::fail(stall, &self.path)),
            };
            let stamp = |frame: &Video| {
                frame
                    .timestamp()
                    .and_then(|ticks| ffi::seconds(ticks, self.time_base))
                    .map(|pts| pts - self.start)
            };
            let mut pts = stamp(&frame);
            // The first picture after a seek is the keyframe it landed on.
            // One later than the target - by more than a tick of the
            // stream's clock, which a container that rounds to milliseconds
            // can put a frame past its true instant, and never more than a
            // millisecond, which a clock as coarse as the frames would be -
            // means the picture on screen at the target was skipped.
            if let Some(landing) = self.landing.take()
                && pts.is_some_and(|pts| pts > landing.target + self.slack())
            {
                self.retreat(landing)?;
                continue;
            }
            // A frame short of a seek's target is let go before it is copied
            // off the chip: a seek decodes from the keyframe before it, up
            // to a second or more of frames, and a full-size copy back of
            // each one it throws away made every scrub on a Windows GPU
            // decoder wait for all of them (#472). The last of them is kept
            // back, uncopied: when the next frame is past the target, no
            // frame sits on it - a variable-rate recording that holds a
            // still screen for seconds, a phone clip whose frames drift off
            // the grid - and the picture on screen there is the one before.
            // Handing on the next instead showed, after any seek into such
            // a pause, the picture from after it, seconds early (#457).
            if let (Some(before), Some(at)) = (self.discard_before, pts) {
                if at < before {
                    self.short = Some(frame);
                    continue;
                }
                if at > before + self.slack()
                    && let Some(short) = self.short.take()
                {
                    self.stashed = Some(frame);
                    frame = short;
                    pts = stamp(&frame);
                }
                self.short = None;
            }
            if let (Some(through), Some(pts)) = (self.discard_through, pts)
                && pts <= through
            {
                continue;
            }
            if let Some(&OnChip::Accel(device, format)) = self.hardware.as_ref() {
                if frame.format() == format {
                    frame = match hardware::download(&frame) {
                        Ok(copy) => copy,
                        Err(error) => {
                            hardware::note_fallback(
                                device,
                                "copy a frame back",
                                &error.to_string(),
                            );
                            self.resume_in_software(pts, false)?;
                            continue;
                        }
                    };
                } else {
                    // The codec set the device up for this stream and the
                    // device said no, so the codec went on in software by
                    // itself; the frames are fine.
                    hardware::note_fallback(
                        device,
                        "take this stream",
                        "the codec decoded it in software",
                    );
                    self.hardware = None;
                }
            }
            // FFmpeg's ProRes decoder reads the picture's matrix into the
            // stream and leaves every frame it makes untagged, and swscale
            // then reads BT.601 into what is a BT.709 picture: a ProRes
            // 4444 overlay's reds came back orange. Its frames are given
            // the stream's matrix where they say none. Only ProRes: the
            // other decoders tag their frames as their streams are tagged.
            if self.decoder.id() == ffmpeg::codec::Id::PRORES
                && self.color.matrix != ffmpeg::color::Space::Unspecified
            {
                // SAFETY: a frame this reader owns; the matrix is a plain
                // field the filtergraph reads with the picture.
                unsafe {
                    let raw = frame.as_mut_ptr();
                    if (*raw).colorspace == ffmpeg::sys::AVColorSpace::UNSPECIFIED {
                        (*raw).colorspace = self.color.matrix.into();
                    }
                }
            }
            self.discard_before = None;
            self.discard_through = None;
            self.last_pts = pts;
            return Ok(Some(Source { frame, pts }));
        }
    }

    /// Runs one decoded picture through the graph and copies the RGBA out.
    fn convert(&mut self, source: &Video) -> Result<Frame> {
        let key = (source.format(), source.width(), source.height());
        if self
            .graph
            .as_ref()
            .is_none_or(|(format, width, height, _)| (*format, *width, *height) != key)
        {
            let graph = self.build_graph(key.0, key.1, key.2)?;
            self.graph = Some((key.0, key.1, key.2, graph));
        }
        let (.., graph) = self.graph.as_mut().expect("just built");

        {
            let mut context = graph.get("in").expect("the graph has an input");
            context
                .source()
                .add(source)
                .map_err(|error| ffi::fail("filter", &self.path, error))?;
        }
        let mut filtered = Video::empty();
        {
            let mut context = graph.get("out").expect("the graph has an output");
            context
                .sink()
                .frame(&mut filtered)
                .map_err(|error| ffi::fail("filter output", &self.path, error))?;
        }

        // The sink's picture is RGBA already - sixteen bits a channel for a
        // deep frame - and only its row padding differs from the packed
        // buffer a Frame is.
        let deep = deep_output(&self.options, Some(&self.color));
        let width = filtered.width();
        let height = filtered.height();
        let row = width as usize * if deep.is_some() { 8 } else { 4 };
        let stride = filtered.stride(0);
        let data = filtered.data(0);
        let mut pixels = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            pixels.extend_from_slice(&data[y * stride..y * stride + row]);
        }
        let frame = match deep {
            Some(signal) => Frame::from_rgba64(width, height, pixels, signal),
            None => Frame::from_rgba(width, height, pixels),
        };
        frame.ok_or_else(|| Error::Probe {
            path: self.path.clone(),
            detail: "the filtergraph produced a frame of the wrong size".to_owned(),
        })
    }

    fn build_graph(&self, format: Pixel, width: u32, height: u32) -> Result<filter::Graph> {
        let mut graph = filter::Graph::new();
        let args = format!(
            "video_size={width}x{height}:pix_fmt={}:time_base={}/{}:pixel_aspect=1/1",
            Into::<ffmpeg::sys::AVPixelFormat>::into(format).0,
            self.time_base.numerator(),
            self.time_base.denominator()
        );
        let missing = |name: &str| Error::Missing {
            what: "filter",
            name: name.to_owned(),
        };
        graph
            .add(
                &filter::find("buffer").ok_or_else(|| missing("buffer"))?,
                "in",
                &args,
            )
            .map_err(|error| ffi::fail("buffer source", &self.path, error))?;
        graph
            .add(
                &filter::find("buffersink").ok_or_else(|| missing("buffersink"))?,
                "out",
                "",
            )
            .map_err(|error| ffi::fail("buffer sink", &self.path, error))?;
        let spec = video_filter(
            self.rotation,
            &self.options,
            self.width,
            self.height,
            Some(&self.color),
        );
        graph
            .output("in", 0)
            .and_then(|parser| parser.input("out", 0))
            .and_then(|parser| parser.parse(&spec))
            .map_err(|error| ffi::fail("filter graph", &self.path, error))?;
        graph
            .validate()
            .map_err(|error| ffi::fail("filter graph", &self.path, error))?;
        Ok(graph)
    }

    /// Without pacing: every source frame, in order.
    fn next_unpaced(&mut self) -> Result<Option<Frame>> {
        let Some(source) = self.next_source()? else {
            if self.options.looping
                && let Some(last) = self.current.as_ref()
            {
                self.position = last.pts;
                let frame = share(&last.frame);
                return Ok(Some(self.convert(&frame)?));
            }
            return Ok(None);
        };
        self.position = source.pts;
        // The buffer source takes the picture it is handed, and a repeating
        // decoder needs that picture again after the end: it converts a
        // reference and keeps the original, as the paced path does.
        let frame = if self.options.looping {
            let reference = share(&source.frame);
            self.convert(&reference)?
        } else {
            self.convert(&source.frame)?
        };
        self.current = Some(source);
        Ok(Some(frame))
    }

    /// With pacing: the source frame on screen at the next output instant.
    fn next_paced(&mut self, rate: FrameRate) -> Result<Option<Frame>> {
        let target = self.origin + rate.frame_duration() * Rational::from_int(self.tick as i64);

        // Advance until `pending` is the first frame after the target, so
        // `current` is the one on screen at it. A frame with no timestamp
        // counts as the next one in line. A frame within a tick of the
        // stream's clock past the target is the one meant for it: Matroska,
        // WebM, FLV and ASF stamp in whole milliseconds, so the 30 fps
        // frame due at 1.4667 s says 1.467, and taken at its word every
        // exported frame from MKV, WebM, FLV or WMV footage was the one
        // before the picture the preview showed (#457). Never more than a
        // quarter of an output frame, so a clock as coarse as the frames
        // themselves does not pull the next one in early.
        let tick = ffi::seconds(1, self.time_base).unwrap_or(Rational::ZERO);
        let due = target + tick.min(rate.frame_duration() / Rational::from_int(4));
        while !self.source_done
            && self
                .pending
                .as_ref()
                .is_none_or(|next| next.pts.is_none_or(|pts| pts <= due))
        {
            if let Some(next) = self.pending.take() {
                self.current = Some(next);
            }
            match self.next_source()? {
                Some(source) => self.pending = Some(source),
                None => self.source_done = true,
            }
        }
        if self.current.is_none() {
            // Nothing at or before the target: the first frame of the file
            // sits after it, and shows from the first instant, as it would
            // through any player.
            self.current = self.pending.take();
        }
        let Some(current) = self.current.as_ref() else {
            return Ok(None);
        };
        if self.source_done && self.pending.is_none() && !self.options.looping {
            // Past the last frame. It has already shown once at its own
            // instant; the file is over. Its stamp is read with the same
            // tick of slack as above: a last frame stamped late would
            // otherwise be held for one output frame more than it lasts.
            if let Some(pts) = current.pts
                && pts + rate.frame_duration() <= due
            {
                return Ok(None);
            }
        }
        self.position = Some(target);
        self.tick += 1;
        let frame = share(&current.frame);
        Ok(Some(self.convert(&frame)?))
    }
}

impl Decoder {
    /// What the stream says its colour is; see [`ColorSignal`].
    pub fn color(&self) -> ColorSignal {
        self.color
    }
}

impl FrameSource for Decoder {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn position(&self) -> Option<Rational> {
        self.position
    }

    fn next_frame(&mut self) -> Result<Option<Frame>> {
        if self
            .options
            .max_frames
            .is_some_and(|limit| self.produced >= limit)
        {
            return Ok(None);
        }
        let frame = match self.options.frame_rate {
            Some(rate) => self.next_paced(rate)?,
            None => self.next_unpaced()?,
        };
        if frame.is_some() {
            self.produced += 1;
        }
        Ok(frame)
    }
}

/// Whether MediaCodec's decoder `name` gives the first picture of
/// `path`'s video within `wait` - [`hardware::MEDIACODEC_FIRST_PICTURE`] for a
/// reader; see [`hardware::Verdicts`] for why a decoder is asked before it
/// is trusted.
///
/// The try runs on a thread of its own, with its own demuxer and codec,
/// because a decoder that wedges never returns control to the caller; a
/// wedged try is left behind on its thread, and the reader never waits on
/// it past the deadline. A try that answers late still has its say, for
/// the readers that come after.
fn first_picture_within(path: &Path, name: &'static str, wait: std::time::Duration) -> bool {
    let path = path.to_path_buf();
    let (answer, answered) = std::sync::mpsc::channel();
    let tried = std::thread::Builder::new()
        .name("mediacodec-first-picture".to_owned())
        .spawn(move || {
            let attempt = || -> Option<()> {
                let mut input = ffmpeg::format::input(&path).ok()?;
                let (mut codec, index) = {
                    let stream = input.streams().best(ffmpeg::media::Type::Video)?;
                    let (codec, _) = Decoder::open_codec(
                        &path,
                        &stream,
                        &DecodeOptions::default(),
                        Codec::Named(name),
                    )
                    .ok()?;
                    (codec, stream.index())
                };
                let mut held = None;
                let read = || feed::read_packet(&mut input, index);
                feed::next_frame(&mut codec, &mut held, read)
                    .ok()?
                    .map(|_| ())
            };
            let works = attempt().is_some();
            let _ = answer.send(works);
            // Late or not, what it found is the verdict from now on: a
            // decoder that was only slow to start is not written off for
            // the life of the process.
            hardware::mediacodec_answered(name, works);
        });
    tried.is_ok() && answered.recv_timeout(wait).unwrap_or(false)
}

/// A second reference to `frame`'s picture: the same buffers, counted,
/// so a graph that takes the reference (`buffersrc` does) takes nothing
/// from the frame's owner. `Video::clone` copies the pixels - fifty
/// megabytes a frame at 8K - and the pacer clones a frame for every output
/// frame it repeats (audit 2026-09-23, #16). A frame with nothing to share
/// is copied the old way.
fn share(frame: &Video) -> Video {
    let mut shared = Video::empty();
    // SAFETY: both pointers are the wrappers' own valid AVFrames for the
    // duration of the call; av_frame_ref adds references to the source's
    // buffers and copies its properties into the empty destination, and
    // touches nothing else. On failure the destination is left unref'd.
    let result = unsafe { ffmpeg::sys::av_frame_ref(shared.as_mut_ptr(), frame.as_ptr()) };
    if result < 0 { frame.clone() } else { shared }
}

impl SeekableSource for Decoder {
    fn seek(&mut self, to: Rational) -> Result<()> {
        self.jump(to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::RateMode;

    /// A stream that starts late - MPEG-TS starts at 1.4 s by default, and
    /// the MTS files cameras write likewise - is read from its own first
    /// frame, and a seek lands where the caller meant.
    #[test]
    fn a_stream_that_starts_late_is_read_from_its_first_frame() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/starts-at-1.4s.ts");
        let mut decoder = Decoder::open(path, &DecodeOptions::default()).expect("opens");
        let first = decoder.next_frame().expect("decodes").expect("a frame");
        assert_eq!((first.width(), first.height()), (64, 64));
        let at = decoder.position().expect("a position").as_f64();
        assert!(at < 0.05, "the first frame is at {at}, not 1.4 s in");
        decoder.seek(Rational::new(1, 2)).expect("seeks");
        let _ = decoder.next_frame().expect("decodes").expect("a frame");
        let at = decoder.position().expect("a position").as_f64();
        assert!(
            (0.45..0.65).contains(&at),
            "after a seek to 0.5 s the frame is at {at}"
        );
    }

    /// A picture coded with wide pixels - HDV and AVCHD's 1440 across, DV's
    /// 720 - is probed and decoded at the width a player shows it, not
    /// squeezed into a pillarbox. The fixture codes 48 by 32 with pixels a
    /// third wider than tall, its left third white: shown 64 by 32, the
    /// white still a third of the width (#457).
    #[test]
    fn a_picture_of_wide_pixels_is_decoded_at_its_shown_width() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/wide-pixels-hdv.m2ts");
        let video = crate::probe(path)
            .expect("probes")
            .video
            .expect("a picture");
        assert_eq!((video.width, video.height), (64, 32));
        let mut decoder = Decoder::open(path, &DecodeOptions::default()).expect("opens");
        assert_eq!((decoder.width(), decoder.height()), (64, 32));
        let frame = decoder.next_frame().expect("decodes").expect("a frame");
        assert_eq!((frame.width(), frame.height()), (64, 32));
        let row: Vec<u8> = frame.pixels()[..64 * 4]
            .chunks_exact(4)
            .map(|pixel| pixel[0])
            .collect();
        let white = row.iter().filter(|&&level| level > 128).count();
        assert!(
            (19..=24).contains(&white),
            "{white} of 64 columns are white, not a third"
        );
    }

    /// A variable-rate recording that holds a still screen has no frame
    /// for the length of the pause. A seek into it lands on the picture on
    /// screen there - the last one before the pause - and not on the first
    /// one after it, seconds early. The fixture is white to 0.3 s, then has
    /// no frame until black starts at 2 s (#457).
    #[test]
    fn a_seek_into_a_still_screen_shows_the_picture_held_there() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/vfr-still-screen.mp4");
        let brightness = |frame: &Frame| frame.pixels()[0];
        let at = Rational::from_int(1);
        let options = DecodeOptions::default().in_software();
        let mut opened = Decoder::open(path, &options.clone().starting_at(at)).expect("opens");
        let held = opened.next_frame().expect("decodes").expect("a frame");
        assert!(
            brightness(&held) > 200,
            "the pause shows the white before it"
        );
        assert_eq!(opened.position(), Some(Rational::new(8, 30)));
        let next = opened.next_frame().expect("decodes").expect("a frame");
        assert!(brightness(&next) < 50, "and the black after it comes next");
        assert_eq!(opened.position(), Some(Rational::from_int(2)));

        let mut paced = Decoder::open(
            path,
            &options.clone().starting_at(at).at_rate(FrameRate::THIRTY),
        )
        .expect("opens paced");
        let frames = drain(&mut paced);
        assert_eq!(frames.len(), 39, "1 s to 2.3 s at 30 fps");
        let white = frames
            .iter()
            .take_while(|(_, frame)| brightness(frame) > 200);
        assert_eq!(white.count(), 30, "white until the black starts at 2 s");

        // Past the last frame, the last frame is the one on screen.
        let mut late =
            Decoder::open(path, &options.starting_at(Rational::from_int(5))).expect("opens");
        let last = late.next_frame().expect("decodes").expect("the last frame");
        assert!(brightness(&last) < 50);
        assert_eq!(late.position(), Some(Rational::new(68, 30)));
    }

    /// A WebM alpha layer is a second plane the built-in decoders do not
    /// read; without it the picture arrives opaque and covers the lanes
    /// below. Both flavours go through libvpx and keep it. See
    /// <https://github.com/jub0t/Concat/issues/429>.
    #[test]
    fn a_webm_alpha_layer_survives_the_decoder() {
        for fixture in ["alpha-vp8.webm", "alpha-vp9.webm"] {
            let path = format!("{}/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
            let options = DecodeOptions {
                hardware: HwPolicy::Software,
                ..DecodeOptions::default()
            };
            let mut decoder = Decoder::open(&path, &options).expect("opens");
            let frame = decoder.next_frame().expect("decodes").expect("a frame");
            let alphas: Vec<u8> = frame
                .pixels()
                .chunks_exact(4)
                .map(|pixel| pixel[3])
                .collect();
            assert!(alphas.contains(&255), "{fixture}: the square is opaque");
            assert!(
                alphas.iter().filter(|&&alpha| alpha < 8).count() > alphas.len() / 2,
                "{fixture}: the ground is clear"
            );
        }
    }

    /// VLC's stream output leaves an MP4's first picture out of its keyframe
    /// table, behind an edit list that starts the picture 14 ms in: a seek
    /// short of the second keyframe, 2 s in here, lands where the caller
    /// meant instead of failing.
    #[test]
    fn a_seek_before_the_second_keyframe_of_a_vlc_mp4_lands() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/vlc-first-picture-not-a-keyframe.mp4"
        );
        let mut decoder = Decoder::open(path, &DecodeOptions::default()).expect("opens");
        decoder.seek(Rational::from_int(1)).expect("seeks");
        let _ = decoder.next_frame().expect("decodes").expect("a frame");
        let at = decoder.position().expect("a position").as_f64();
        assert!(
            (0.95..1.15).contains(&at),
            "after a seek to 1 s the frame is at {at}"
        );
    }

    #[test]
    fn options_build_up() {
        let options = DecodeOptions::default()
            .starting_at(Rational::from_int(2))
            .limited_to(10)
            .scaled_to(640, 360);
        assert_eq!(options.start, Some(Rational::from_int(2)));
        assert_eq!(options.max_frames, Some(10));
        assert_eq!(options.size, Some((640, 360)));
    }

    #[test]
    fn a_filter_chain_is_fenced_by_the_guard_scale() {
        let options = DecodeOptions::default()
            .scaled_to(640, 360)
            .filtered("hue=s=0");
        assert_eq!(
            video_filter(0, &options, 640, 360, None),
            "scale=640:360:flags=bilinear,hue=s=0,scale=640:360:flags=bilinear,format=rgba",
        );
    }

    #[test]
    fn rotation_comes_first() {
        let options = DecodeOptions::default();
        assert!(video_filter(90, &options, 1080, 1920, None).starts_with("transpose=clock,"));
        assert!(video_filter(0, &options, 1920, 1080, None).starts_with("scale="));
    }

    #[test]
    fn a_wide_picture_is_brought_into_bt709_and_a_709_one_is_left_alone() {
        use ffmpeg::color::{Primaries, Range, Space, TransferCharacteristic as Transfer};
        let options = DecodeOptions::default();
        let sdr = ColorSignal {
            primaries: Primaries::BT709,
            transfer: Transfer::BT709,
            matrix: Space::BT709,
            range: Range::MPEG,
            implied: false,
        };
        assert!(!sdr.is_wide());
        assert_eq!(
            video_filter(0, &options, 640, 360, Some(&sdr)),
            "scale=640:360:flags=bilinear,format=rgba"
        );
        let hdr = ColorSignal {
            primaries: Primaries::BT2020,
            transfer: Transfer::SMPTE2084,
            matrix: Space::BT2020NCL,
            range: Range::MPEG,
            implied: false,
        };
        assert!(hdr.is_wide() && hdr.is_hdr());
        let spec = video_filter(0, &options, 640, 360, Some(&hdr));
        assert!(spec.starts_with("scale=640:360:flags=bilinear:in_primaries=bt2020:in_transfer=smpte2084:in_color_matrix=bt2020nc:in_range=tv:out_primaries=bt709"), "{spec}");
        assert!(spec.contains("intent=perceptual,format=rgba"), "{spec}");
        // wide gamut with an SDR curve converts too, and the untagged
        // half of it is left to the frames
        let wide = ColorSignal {
            primaries: Primaries::BT2020,
            transfer: Transfer::Unspecified,
            matrix: Space::Unspecified,
            range: Range::Unspecified,
            implied: false,
        };
        assert!(wide.is_wide() && !wide.is_hdr());
        assert!(
            video_filter(0, &options, 64, 64, Some(&wide))
                .contains(":in_transfer=auto:in_color_matrix=auto:in_range=auto:")
        );
    }

    /// With no tag and no override, the codec's convention names the
    /// range: JPEG, the picture formats and anything RGB are full, and a
    /// YCbCr video codec is limited - even one a hardware decoder has
    /// handed back as plain NV12.
    #[test]
    fn an_untagged_stream_takes_its_codecs_range() {
        use ffmpeg::codec::Id;
        assert_eq!(
            ColorRange::implied(Id::MJPEG, Pixel::YUVJ420P),
            ColorRange::Full
        );
        assert_eq!(
            ColorRange::implied(Id::MJPEG, Pixel::NV12),
            ColorRange::Full
        );
        assert_eq!(
            ColorRange::implied(Id::H264, Pixel::YUVJ420P),
            ColorRange::Full
        );
        assert_eq!(ColorRange::implied(Id::PNG, Pixel::RGBA), ColorRange::Full);
        assert_eq!(
            ColorRange::implied(Id::RAWVIDEO, Pixel::BGRA),
            ColorRange::Full
        );
        assert_eq!(
            ColorRange::implied(Id::H264, Pixel::YUV420P),
            ColorRange::Limited
        );
        assert_eq!(
            ColorRange::implied(Id::HEVC, Pixel::YUV420P10LE),
            ColorRange::Limited
        );
        assert_eq!(
            ColorRange::implied(Id::RAWVIDEO, Pixel::NV12),
            ColorRange::Limited
        );
        assert_eq!(
            ColorRange::implied(Id::None, Pixel::None),
            ColorRange::Limited
        );
    }

    /// An implied full range is named to the fit, because the frames
    /// carry no tag to read; an implied limited one is left unsaid, since
    /// unsaid already means limited; a tagged one is the frames' to say;
    /// and an override still beats the reading.
    #[test]
    fn an_implied_full_range_reaches_the_filter() {
        use ffmpeg::color::{Primaries, Range, Space, TransferCharacteristic as Transfer};
        let options = DecodeOptions::default().scaled_to(640, 360);
        let signal = |range, implied| ColorSignal {
            primaries: Primaries::BT709,
            transfer: Transfer::BT709,
            matrix: Space::BT709,
            range,
            implied,
        };
        assert_eq!(
            video_filter(0, &options, 640, 360, Some(&signal(Range::JPEG, true))),
            "scale=640:360:flags=bilinear:in_range=pc,format=rgba"
        );
        assert_eq!(
            video_filter(0, &options, 640, 360, Some(&signal(Range::MPEG, true))),
            "scale=640:360:flags=bilinear,format=rgba"
        );
        assert_eq!(
            video_filter(0, &options, 640, 360, Some(&signal(Range::JPEG, false))),
            "scale=640:360:flags=bilinear,format=rgba"
        );
        let limited = options.clone().in_range(Some(ColorRange::Limited));
        assert_eq!(
            video_filter(0, &limited, 640, 360, Some(&signal(Range::JPEG, true))),
            "scale=640:360:flags=bilinear:in_range=tv,format=rgba"
        );
    }

    /// A range the caller names is the range the picture is read as: on
    /// an SDR file it is the one term added to the fit, on a wide one it
    /// replaces the file's tag among the conversion's terms, and with no
    /// override nothing is said and the frames' own tag decides.
    /// https://github.com/jub0t/Concat/issues/103
    #[test]
    fn a_named_range_overrides_the_files_tag() {
        use ffmpeg::color::{Primaries, Range, Space, TransferCharacteristic as Transfer};
        let sdr = ColorSignal {
            primaries: Primaries::BT709,
            transfer: Transfer::BT709,
            matrix: Space::BT709,
            range: Range::MPEG,
            implied: false,
        };
        let full = DecodeOptions::default()
            .scaled_to(640, 360)
            .in_range(Some(ColorRange::Full));
        assert_eq!(
            video_filter(0, &full, 640, 360, Some(&sdr)),
            "scale=640:360:flags=bilinear:in_range=pc,format=rgba"
        );
        let limited = DecodeOptions::default()
            .scaled_to(640, 360)
            .in_range(Some(ColorRange::Limited));
        assert_eq!(
            video_filter(0, &limited, 640, 360, Some(&sdr)),
            "scale=640:360:flags=bilinear:in_range=tv,format=rgba"
        );
        let untold = DecodeOptions::default().scaled_to(640, 360);
        assert_eq!(
            video_filter(0, &untold, 640, 360, Some(&sdr)),
            "scale=640:360:flags=bilinear,format=rgba",
            "with no override the filter says nothing about range"
        );
        // On a wide picture the override has already replaced the signal's
        // range by the time the filter is built; the filter reads the
        // signal it is given.
        let hdr_full = ColorSignal {
            primaries: Primaries::BT2020,
            transfer: Transfer::SMPTE2084,
            matrix: Space::BT2020NCL,
            range: ColorRange::Full.as_ffmpeg(),
            implied: false,
        };
        assert!(
            video_filter(0, &full, 640, 360, Some(&hdr_full)).contains(":in_range=pc:"),
            "a wide picture carries the range among its conversion terms"
        );
        assert_eq!(ColorRange::parse("PC"), Some(ColorRange::Full));
        assert_eq!(ColorRange::parse("video"), Some(ColorRange::Limited));
        assert_eq!(ColorRange::parse("wide"), None);
        assert_eq!(ColorRange::from_ffmpeg(Range::JPEG), Some(ColorRange::Full));
        assert_eq!(ColorRange::from_ffmpeg(Range::Unspecified), None);
        for range in ColorRange::ALL {
            assert_eq!(ColorRange::parse(range.name()), Some(range));
            assert_eq!(ColorRange::from_ffmpeg(range.as_ffmpeg()), Some(range));
        }
    }

    /// A PQ-tagged ten-bit HEVC file, made with the encoder's own machinery,
    /// decodes through the tone-map to frames that are neither the black
    /// nor the blown-out white a curve mismatch gives.
    #[test]
    fn an_hdr_file_decodes_through_the_tone_map() {
        use crate::encode::{EncodeOptions, Encoder, FrameSink, VideoCodec};
        use ffmpeg::color::{Primaries, Space, TransferCharacteristic as Transfer};
        if !VideoCodec::Hevc.available() {
            eprintln!("no HEVC encoder in the linked FFmpeg; skipped");
            return;
        }
        let path = std::env::temp_dir().join("concat-decode-hdr-test.mp4");
        let options = EncodeOptions {
            codec: VideoCodec::Hevc,
            preset: "ultrafast".to_owned(),
            crf: 20,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 0,
            ten_bit: true,
            color_range: ColorRange::Limited,
            hardware: false,
            threads: 0,
            software_only: false,
        };
        let mut encoder = Encoder::create_tagged(
            &path,
            64,
            64,
            FrameRate::THIRTY,
            &options,
            (Primaries::BT2020, Transfer::SMPTE2084, Space::BT2020NCL),
        )
        .expect("an HEVC encoder");
        let mut frame = Frame::black(64, 64);
        frame.fill([180, 180, 180, 255]);
        for _ in 0..3 {
            encoder.write_frame(&frame).expect("writes");
        }
        encoder.finish().expect("finishes");

        let mut decoder = Decoder::open(&path, &DecodeOptions::default()).expect("opens");
        assert!(decoder.color().is_hdr(), "{:?}", decoder.color());
        let decoded = decoder.next_frame().expect("decodes").expect("a frame");
        let luma = decoded.pixels()[0];
        assert!(luma > 8 && luma < 250, "tone-mapped grey came out {luma}");

        // Asked deep, the same file comes out in its own signal at sixteen
        // bits a channel, its grey where it was written - about 180 of 255
        // of full scale - rather than tone-mapped.
        let mut deep = Decoder::open(&path, &DecodeOptions::default().deep(true)).expect("opens");
        let frame = deep.next_frame().expect("decodes").expect("a frame");
        assert_eq!(frame.depth(), concat_core::frame::Depth::Sixteen);
        assert_eq!(frame.signal(), concat_core::frame::Signal::Pq);
        assert_eq!(frame.pixels().len(), 64 * 64 * 8);
        let channel = |at: usize| u16::from_le_bytes([frame.pixels()[at], frame.pixels()[at + 1]]);
        let (r, g, b) = (channel(0), channel(2), channel(4));
        let written = 180.0 / 255.0 * 65535.0;
        for value in [r, g, b] {
            assert!(
                (f64::from(value) - written).abs() < 65535.0 * 0.03,
                "the deep grey came out {r}, {g}, {b}"
            );
        }

        // A chain runs on eight bits, so it keeps the tone-mapped frame.
        let mut chained = Decoder::open(
            &path,
            &DecodeOptions::default().deep(true).filtered("negate"),
        )
        .expect("opens");
        let frame = chained.next_frame().expect("decodes").expect("a frame");
        assert_eq!(frame.depth(), concat_core::frame::Depth::Eight);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_empty_chain_is_no_chain() {
        assert!(DecodeOptions::default().filtered("").filter_chain.is_none());
    }

    /// A chain that branches and rejoins with labels is the clip's own
    /// graph, and opens: the shape a bloom takes. A chain that is not a
    /// graph is an error at the parse, and never a panic.
    #[test]
    fn a_branching_chain_is_the_clips_own_graph_and_a_broken_one_is_an_error() {
        use crate::{EncodeOptions, Encoder, FrameSink};
        let path = std::env::temp_dir().join("concat-decode-chain-test.mp4");
        let mut encoder = Encoder::create(
            &path,
            64,
            64,
            FrameRate::THIRTY,
            &EncodeOptions {
                preset: "ultrafast".to_owned(),
                ..EncodeOptions::default()
            },
        )
        .expect("the linked FFmpeg encodes h264");
        for _ in 0..4 {
            encoder.write_frame(&Frame::black(64, 64)).expect("writes");
        }
        encoder.finish().expect("finishes");

        let bloom = "split[a][b];[b]gblur=sigma=2[c];[a][c]blend=all_mode=screen";
        let mut decoder = Decoder::open(
            &path,
            &DecodeOptions::default().scaled_to(64, 64).filtered(bloom),
        )
        .expect("a branching chain opens");
        let frame = decoder.next_frame().expect("decodes through the graph");
        assert!(frame.is_some(), "the bloom gives a frame back");

        let mut broken = Decoder::open(
            &path,
            &DecodeOptions::default()
                .scaled_to(64, 64)
                .filtered("nosuchfilter=1"),
        )
        .expect("the file opens; the graph is built on the first frame");
        assert!(broken.next_frame().is_err(), "a chain that is not a graph");
        let _ = std::fs::remove_file(&path);
    }

    /// Frame threads hand frames back a few late; a seek must still land
    /// on exactly the frame it asked for, not one the threads had queued.
    #[test]
    fn a_seek_lands_on_the_same_frame_as_decoding_up_to_it() {
        use crate::{EncodeOptions, Encoder, FrameSink};
        let path = std::env::temp_dir().join("concat-decode-threads-test.mp4");
        let mut encoder = Encoder::create(
            &path,
            64,
            64,
            FrameRate::THIRTY,
            &EncodeOptions {
                preset: "ultrafast".to_owned(),
                ..EncodeOptions::default()
            },
        )
        .expect("the linked FFmpeg encodes h264");
        for index in 0..30u8 {
            // Every frame its own flat grey, so frames are told apart.
            let pixels = [index * 8, index * 8, index * 8, 255].repeat(64 * 64);
            let frame = Frame::from_rgba(64, 64, pixels).expect("a frame");
            encoder.write_frame(&frame).expect("writes");
        }
        encoder.finish().expect("finishes");

        let options = DecodeOptions::default().scaled_to(64, 64);
        let mut walked = Decoder::open(&path, &options).expect("opens");
        let mut fifteenth = None;
        for _ in 0..=15 {
            fifteenth = walked.next_frame().expect("decodes");
        }
        let mut sought = Decoder::open(&path, &options.clone().starting_at(Rational::new(1, 2)))
            .expect("opens at half a second");
        let landed = sought.next_frame().expect("decodes").expect("a frame");
        let _ = std::fs::remove_file(&path);

        let fifteenth = fifteenth.expect("sixteen frames");
        let difference = fifteenth
            .pixels()
            .iter()
            .zip(landed.pixels())
            .map(|(a, b)| u32::from(a.abs_diff(*b)))
            .max()
            .unwrap_or(0);
        assert!(
            difference <= 2,
            "the seek landed {difference} levels from frame 15"
        );
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        assert!(Decoder::open("does-not-exist.mp4", &DecodeOptions::default()).is_err());
    }

    /// An MPEG-TS seek lands on the picture asked for. The container has no
    /// index, and FFmpeg's search over its timestamps lands past the
    /// keyframe wanted - here, with the only keyframe at the top, past the
    /// end of the file - and the stream's first timestamp is 1.4 s in, so
    /// a seek to its very start found nothing either. A camera's MTS showed
    /// no picture anywhere in the paused preview, and an MPEG-2 .mpg began
    /// a group of pictures late (#457).
    #[test]
    fn a_transport_stream_seek_lands_on_the_picture_asked_for() {
        let path = grey_steps_in("transport", "ts");
        let options = DecodeOptions::default().scaled_to(64, 64).in_software();
        let walked = drain(&mut Decoder::open(&path, &options).expect("opens"));
        assert_eq!(walked.len(), 30, "every frame from the top");
        let mut reused = Decoder::open(&path, &options).expect("opens");
        for index in [15usize, 27, 0, 1, 15] {
            let at = Rational::new(index as i64, 30);
            let mut opened =
                Decoder::open(&path, &options.clone().starting_at(at)).expect("opens there");
            reused.seek(at).expect("seeks");
            for (how, decoder) in [("opened at", &mut opened), ("sought to", &mut reused)] {
                let landed = decoder
                    .next_frame()
                    .expect("decodes")
                    .unwrap_or_else(|| panic!("{how} frame {index}: no picture"));
                assert_eq!(decoder.position(), Some(at), "{how} frame {index}");
                let difference = worst_difference(&walked[index].1, &landed);
                assert!(
                    difference <= 2,
                    "{how} frame {index}: {difference} levels off the picture there"
                );
            }
        }
        let _ = std::fs::remove_file(&path);
    }

    /// Matroska stamps frames in whole milliseconds, so most 30 fps frames
    /// say a fraction of a millisecond past their true instant - 67 ms for
    /// the one due at 66.7. The paced reader the export draws from still
    /// gives each output frame its own source frame, not the one before
    /// (#457).
    #[test]
    fn a_paced_read_of_millisecond_stamps_keeps_every_frame_in_its_place() {
        let path = grey_steps_in("milliseconds", "mkv");
        let options = DecodeOptions::default().scaled_to(64, 64).in_software();
        let walked = drain(&mut Decoder::open(&path, &options).expect("opens"));
        let paced = drain(
            &mut Decoder::open(&path, &options.clone().at_rate(FrameRate::THIRTY))
                .expect("opens paced"),
        );
        let _ = std::fs::remove_file(&path);
        assert_eq!(walked.len(), 30, "every frame");
        assert_eq!(paced.len(), 30, "a frame for every output instant");
        for (index, ((_, want), (_, got))) in walked.iter().zip(&paced).enumerate() {
            let difference = worst_difference(want, got);
            assert!(
                difference <= 2,
                "output frame {index} is {difference} levels off source frame {index}"
            );
        }
    }

    /// A thirty-frame H.264 file whose every frame is its own flat grey,
    /// for telling frames apart.
    fn grey_steps(name: &str) -> PathBuf {
        grey_steps_in(name, "mp4")
    }

    /// [`grey_steps`] in the container `extension` names.
    fn grey_steps_in(name: &str, extension: &str) -> PathBuf {
        use crate::{EncodeOptions, Encoder, FrameSink};
        let path = std::env::temp_dir().join(format!(
            "concat-decode-{name}-{}.{extension}",
            std::process::id()
        ));
        let mut encoder = Encoder::create(
            &path,
            64,
            64,
            FrameRate::THIRTY,
            &EncodeOptions {
                preset: "ultrafast".to_owned(),
                ..EncodeOptions::default()
            },
        )
        .expect("the linked FFmpeg encodes h264");
        for index in 0..30u8 {
            let pixels = [index * 8, index * 8, index * 8, 255].repeat(64 * 64);
            let frame = Frame::from_rgba(64, 64, pixels).expect("a frame");
            encoder.write_frame(&frame).expect("writes");
        }
        encoder.finish().expect("finishes");
        path
    }

    /// Every frame and its instant, to the end.
    fn drain(decoder: &mut Decoder) -> Vec<(Option<Rational>, Frame)> {
        let mut frames = Vec::new();
        while let Some(frame) = decoder.next_frame().expect("decodes") {
            frames.push((decoder.position(), frame));
        }
        frames
    }

    /// The largest difference between two frames, over every channel.
    fn worst_difference(a: &Frame, b: &Frame) -> u32 {
        a.pixels()
            .iter()
            .zip(b.pixels())
            .map(|(a, b)| u32::from(a.abs_diff(*b)))
            .max()
            .unwrap_or(0)
    }

    /// Insisting on software gives software, whatever the preference, and
    /// the frames are the frames.
    #[test]
    fn software_is_software_whatever_the_preference() {
        let path = grey_steps("software");
        let mut decoder =
            Decoder::open(&path, &DecodeOptions::default().in_software()).expect("opens");
        assert_eq!(decoder.hardware(), None);
        let frames = drain(&mut decoder);
        let _ = std::fs::remove_file(&path);
        assert_eq!(frames.len(), 30);
        assert_eq!(decoder.hardware(), None);
    }

    /// The device decodes the same frames at the same instants as the CPU:
    /// the same count, the same timestamps, and pixels within the rounding
    /// of a different colour conversion. A seek on the device lands where
    /// a seek on the CPU does. On a machine with the device, that is; on
    /// one without, the reader opens in software and the file still
    /// decodes whole.
    #[test]
    fn hardware_decode_matches_software_in_count_and_timing() {
        let path = grey_steps("hardware");
        let Some(device) = HwDevice::platform_default() else {
            return;
        };
        let software = DecodeOptions::default().scaled_to(64, 64).in_software();
        let hardware = DecodeOptions::default()
            .scaled_to(64, 64)
            .accelerated(device);

        let mut cpu = Decoder::open(&path, &software).expect("opens in software");
        let cpu_frames = drain(&mut cpu);
        let mut gpu = Decoder::open(&path, &hardware).expect("opens on the device");
        let gpu_frames = drain(&mut gpu);
        if cfg!(target_os = "macos") {
            assert_eq!(
                gpu.hardware(),
                Some(HwDevice::VideoToolbox),
                "a Mac decodes H.264 on VideoToolbox"
            );
        }

        assert_eq!(
            gpu_frames.len(),
            cpu_frames.len(),
            "the same number of frames"
        );
        for (index, ((cpu_at, cpu_frame), (gpu_at, gpu_frame))) in
            cpu_frames.iter().zip(&gpu_frames).enumerate()
        {
            assert_eq!(gpu_at, cpu_at, "frame {index} at the same instant");
            assert_eq!(
                (gpu_frame.width(), gpu_frame.height()),
                (cpu_frame.width(), cpu_frame.height())
            );
            let difference = worst_difference(cpu_frame, gpu_frame);
            assert!(
                difference <= 8,
                "frame {index} differs by {difference} levels between the device and the CPU"
            );
        }

        // A seek on the device lands on the same frame as one on the CPU.
        let mut sought = Decoder::open(&path, &hardware.clone().starting_at(Rational::new(1, 2)))
            .expect("opens at half a second on the device");
        let landed = sought.next_frame().expect("decodes").expect("a frame");
        let _ = std::fs::remove_file(&path);
        let (_, fifteenth) = &cpu_frames[15];
        let difference = worst_difference(fifteenth, &landed);
        assert!(
            difference <= 8,
            "the seek landed {difference} levels from frame 15"
        );
    }

    /// On a phone or the emulator, with a real file in
    /// `CONCAT_DEVICE_SAMPLE`: MediaCodec decodes it to the frames the CPU
    /// does - every frame of the first `CONCAT_DEVICE_FRAMES` (default 90)
    /// at the same instant, within the rounding of a colour conversion -
    /// and a seek forward, back and to the very start lands on the frame
    /// the CPU's seek lands on. Prints how long each took, which on the
    /// emulator measures its software MediaCodec and nothing a phone
    /// does. With `CONCAT_DEVICE_OUT`, writes the chip's frame at each
    /// seek there as a JPEG to look at.
    ///
    /// Built with `cargo test -p concat-media --no-run --target
    /// aarch64-linux-android`, pushed with the file to /data/local/tmp,
    /// and run there with `--ignored mediacodec`.
    #[test]
    #[ignore = "needs Android and a sample in CONCAT_DEVICE_SAMPLE"]
    fn mediacodec_decodes_a_real_file_as_the_cpu_does() {
        use std::time::Instant;
        let Some(path) = std::env::var_os("CONCAT_DEVICE_SAMPLE").map(PathBuf::from) else {
            return;
        };
        let count: usize = std::env::var("CONCAT_DEVICE_FRAMES")
            .ok()
            .and_then(|count| count.parse().ok())
            .unwrap_or(90);
        let out = std::env::var_os("CONCAT_DEVICE_OUT").map(PathBuf::from);
        let chip = DecodeOptions::default().accelerated(HwDevice::MediaCodec);
        let cpu = DecodeOptions::default().in_software();

        let take = |options: &DecodeOptions| {
            let started = Instant::now();
            let mut decoder = Decoder::open(&path, options).expect("opens");
            let mut frames = Vec::new();
            while frames.len() < count {
                let Some(frame) = decoder.next_frame().expect("decodes") else {
                    break;
                };
                frames.push((decoder.position(), frame));
            }
            (decoder.hardware(), frames, started.elapsed())
        };
        // How long the platform's decoder takes to its first picture, the
        // number the first-picture deadline is measured against.
        {
            let started = Instant::now();
            let name = hardware::mediacodec_decoder(
                ffmpeg::format::input(&path)
                    .expect("opens")
                    .streams()
                    .best(ffmpeg::media::Type::Video)
                    .expect("a video stream")
                    .parameters()
                    .id(),
                Pixel::YUV420P,
            )
            .expect("a codec MediaCodec takes");
            let answered = first_picture_within(&path, name, std::time::Duration::from_secs(60));
            eprintln!(
                "{name}: first picture {} in {:?}",
                if answered { "came" } else { "did not come" },
                started.elapsed()
            );
        }
        let (on_chip, chip_frames, chip_took) = take(&chip);
        let (_, cpu_frames, cpu_took) = take(&cpu);
        assert_eq!(
            on_chip,
            Some(HwDevice::MediaCodec),
            "still on the chip at the end"
        );
        eprintln!(
            "{} frames of {}x{}: MediaCodec {chip_took:?}, CPU {cpu_took:?}",
            chip_frames.len(),
            chip_frames[0].1.width(),
            chip_frames[0].1.height()
        );
        assert_eq!(chip_frames.len(), cpu_frames.len());
        for (index, ((chip_at, chip_frame), (cpu_at, cpu_frame))) in
            chip_frames.iter().zip(&cpu_frames).enumerate()
        {
            assert_eq!(chip_at, cpu_at, "frame {index} at the same instant");
            let difference = worst_difference(chip_frame, cpu_frame);
            assert!(difference <= 8, "frame {index} differs by {difference}");
        }

        // Seeks on one open reader each, the way the pool's warm readers
        // take them: forward, back, forward again, and to the start.
        let mut chip_reader = Decoder::open(&path, &chip).expect("opens");
        let mut cpu_reader = Decoder::open(&path, &cpu).expect("opens");
        for (step, to) in [(7, 3), (1, 1), (5, 2), (0, 1), (11, 4)]
            .into_iter()
            .enumerate()
        {
            let to = Rational::new(to.0, to.1);
            let started = Instant::now();
            chip_reader.seek(to).expect("seeks");
            let landed = chip_reader.next_frame().expect("decodes").expect("a frame");
            let took = started.elapsed();
            cpu_reader.seek(to).expect("seeks");
            let wanted = cpu_reader.next_frame().expect("decodes").expect("a frame");
            assert_eq!(
                chip_reader.position(),
                cpu_reader.position(),
                "seek to {to:?}"
            );
            let difference = worst_difference(&landed, &wanted);
            eprintln!(
                "seek to {to:?} landed at {:?} in {took:?}, {difference} levels off",
                chip_reader.position()
            );
            assert!(
                difference <= 8,
                "the seek to {to:?} landed {difference} levels off"
            );
            if let Some(out) = &out {
                let jpeg = crate::encode::jpeg(&landed, 90).expect("encodes");
                std::fs::write(out.join(format!("seek-{step}.jpg")), jpeg).expect("writes");
            }
        }
        assert_eq!(chip_reader.hardware(), Some(HwDevice::MediaCodec));
    }

    /// The pacer sits on top of the device exactly as it does the CPU: a
    /// file paced to twice its rate gives the same frames either way.
    #[test]
    fn a_paced_hardware_decode_matches_a_paced_software_one() {
        let path = grey_steps("paced");
        let Some(device) = HwDevice::platform_default() else {
            return;
        };
        let rate = FrameRate::new(Rational::from_int(60));
        let software = DecodeOptions::default()
            .scaled_to(64, 64)
            .at_rate(rate)
            .in_software();
        let hardware = DecodeOptions::default()
            .scaled_to(64, 64)
            .at_rate(rate)
            .accelerated(device);
        let cpu_frames = drain(&mut Decoder::open(&path, &software).expect("opens"));
        let gpu_frames = drain(&mut Decoder::open(&path, &hardware).expect("opens"));
        let _ = std::fs::remove_file(&path);
        assert!(
            cpu_frames.len() >= 58,
            "thirty frames at sixty a second: {}",
            cpu_frames.len()
        );
        assert_eq!(gpu_frames.len(), cpu_frames.len());
        for (index, ((cpu_at, cpu_frame), (gpu_at, gpu_frame))) in
            cpu_frames.iter().zip(&gpu_frames).enumerate()
        {
            assert_eq!(gpu_at, cpu_at, "frame {index} at the same instant");
            assert!(worst_difference(cpu_frame, gpu_frame) <= 8, "frame {index}");
        }
    }

    /// A codec the device has no decoder for - a JPEG still, here - opens
    /// in software straight away, with the device asked for, and decodes.
    #[test]
    fn a_codec_the_device_lacks_decodes_in_software() {
        let path =
            std::env::temp_dir().join(format!("concat-decode-jpeg-{}.jpg", std::process::id()));
        let mut frame = Frame::black(64, 64);
        frame.fill([200, 40, 40, 255]);
        let bytes = crate::encode::jpeg(&frame, 90).expect("a jpeg");
        std::fs::write(&path, bytes).expect("writes the still");
        let device = HwDevice::platform_default().unwrap_or(HwDevice::VideoToolbox);
        let mut decoder = Decoder::open(&path, &DecodeOptions::default().accelerated(device))
            .expect("the still opens whatever the device says");
        assert_eq!(
            decoder.hardware(),
            None,
            "a JPEG is not the device's to decode"
        );
        let decoded = decoder.next_frame().expect("decodes").expect("the picture");
        let _ = std::fs::remove_file(&path);
        assert_eq!((decoded.width(), decoded.height()), (64, 64));
        let red = decoded.pixels()[0];
        assert!(red > 150, "the picture came through: red {red}");
    }

    /// A device the linked FFmpeg does not carry is asked for and quietly
    /// not used: the file decodes in software.
    #[test]
    fn a_device_the_library_lacks_means_software() {
        let path = grey_steps("absent");
        let absent = HwDevice::ALL
            .into_iter()
            .find(|device| !device.linked())
            .unwrap_or(HwDevice::Vaapi);
        let mut decoder =
            Decoder::open(&path, &DecodeOptions::default().accelerated(absent)).expect("opens");
        let frames = drain(&mut decoder);
        let _ = std::fs::remove_file(&path);
        assert_eq!(frames.len(), 30);
        assert_ne!(decoder.hardware(), Some(absent));
    }
}

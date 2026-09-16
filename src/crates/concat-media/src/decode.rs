// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Pulling RGBA frames out of a file.
//!
//! One decoder does everything: it seeks to a start point and discards up to it frame-accurately,
//! turns the picture the way its rotation asks, scales, runs the clip's
//! effect chain through libavfilter, paces frames to a requested output rate
//! by duplicating and dropping, repeats a still forever, and stops after a
//! frame budget. Every frame comes with its real presentation timestamp.
//!
//! ## On the unsafe in here
//!
//! There is none: the wrapper crate owns every pointer. What this module
//! owns is the order things happen in.

use std::path::{Path, PathBuf};

use concat_core::frame::Frame;
use concat_core::time::{FrameRate, Rational};
use ffmpeg_the_third as ffmpeg;
use ffmpeg_the_third::codec::decoder;
use ffmpeg_the_third::filter;
use ffmpeg_the_third::format::{self, Pixel};
use ffmpeg_the_third::util::frame::video::Video;

use crate::error::{Error, Result};
use crate::ffi;

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
    let convert = color
        .filter(|signal| signal.is_wide())
        .map(|signal| signal.bt709_args())
        .unwrap_or_default();
    parts.push(format!("scale={width}:{height}:flags=bilinear{convert}"));
    if let Some(chain) = &options.filter_chain {
        parts.push(chain.clone());
        parts.push(format!("scale={width}:{height}:flags=bilinear"));
    }
    parts.push("format=rgba".to_owned());
    parts.join(",")
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
        // The same slot-escape rule the audio graph enforces: a chain with
        // `;` or `[..]` is no longer a filter applied to this clip.
        if let Some(chain) = &options.filter_chain {
            crate::audio::validate_chain(chain)?;
        }

        let path = path.as_ref();
        let input = ffmpeg::format::input(path).map_err(|error| ffi::fail("open", path, error))?;
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or_else(|| Error::NoVideoStream {
                path: path.to_path_buf(),
            })?;
        let stream_index = stream.index();
        let time_base = stream.time_base();
        let rotation = ffi::rotation(&stream);
        let parameters = stream.parameters();
        let coded = (parameters.width(), parameters.height());

        let context = ffmpeg::codec::Context::from_parameters(parameters)
            .map_err(|error| ffi::fail("codec parameters", path, error))?;
        let mut decoder = context
            .decoder()
            .video()
            .map_err(|error| ffi::fail("open decoder", path, error))?;
        if options.keyframes_only {
            decoder.skip_frame(ffmpeg::Discard::NonKey);
        }
        let color = ColorSignal {
            primaries: decoder.color_primaries(),
            transfer: decoder.color_transfer_characteristic(),
            matrix: decoder.color_space(),
            range: decoder.color_range(),
        };

        let (width, height) = match options.size {
            Some(size) => size,
            None => ffi::displayed(coded.0, coded.1, rotation),
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
            decoder,
            rotation,
            options: options.clone(),
            width,
            height,
            graph: None,
            discard_before: None,
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

    /// How many frames have been produced so far.
    pub const fn produced(&self) -> u64 {
        self.produced
    }

    /// The container seek, and the state reset that goes with it.
    fn jump(&mut self, to: Rational) -> Result<()> {
        let target = ffi::av_ticks(to);
        self.input
            .seek(target, ..=target)
            .map_err(|error| ffi::fail("seek", &self.path, error))?;
        self.decoder.flush();
        // Keyframes only: the picture the container landed on is the one
        // wanted, and discarding up to the target would throw it away and
        // wait for the *next* keyframe, a whole group of pictures late.
        self.discard_before = (!self.options.keyframes_only).then_some(to);
        self.origin = to;
        self.tick = 0;
        self.current = None;
        self.pending = None;
        self.source_done = false;
        self.position = None;
        Ok(())
    }

    /// The next picture out of the codec, or `None` at the end of the file.
    /// Frames before the discard point never come out of here.
    fn next_source(&mut self) -> Result<Option<Source>> {
        loop {
            let mut frame = Video::empty();
            match self.decoder.receive_frame(&mut frame) {
                Ok(()) => {
                    let pts = frame
                        .timestamp()
                        .and_then(|ticks| ffi::seconds(ticks, self.time_base));
                    if let (Some(before), Some(pts)) = (self.discard_before, pts)
                        && pts < before
                    {
                        continue;
                    }
                    self.discard_before = None;
                    return Ok(Some(Source { frame, pts }));
                }
                Err(ffmpeg::Error::Eof) => return Ok(None),
                Err(error) if ffi::is_again(&error) => {}
                Err(error) => return Err(ffi::fail("decode", &self.path, error)),
            }

            // The codec wants another packet. Packets for other streams are
            // skipped; the end of the file flushes the codec so its last few
            // frames come out at all.
            loop {
                let mut packet = ffmpeg::Packet::empty();
                match packet.read(&mut self.input) {
                    Ok(()) => {
                        if packet.stream() != self.stream {
                            continue;
                        }
                        match self.decoder.send_packet(&packet) {
                            Ok(()) => break,
                            Err(error) if ffi::is_again(&error) => break,
                            Err(error) => return Err(ffi::fail("send packet", &self.path, error)),
                        }
                    }
                    Err(ffmpeg::Error::Eof) => {
                        let _ = self.decoder.send_eof();
                        break;
                    }
                    Err(error) => return Err(ffi::fail("read", &self.path, error)),
                }
            }
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

        // The sink's picture is RGBA already; only its row padding differs
        // from the packed buffer a Frame is.
        let width = filtered.width();
        let height = filtered.height();
        let row = width as usize * 4;
        let stride = filtered.stride(0);
        let data = filtered.data(0);
        let mut pixels = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            pixels.extend_from_slice(&data[y * stride..y * stride + row]);
        }
        Frame::from_rgba(width, height, pixels).ok_or_else(|| Error::Probe {
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
                let frame = last.frame.clone();
                return Ok(Some(self.convert(&frame)?));
            }
            return Ok(None);
        };
        self.position = source.pts;
        // The buffer source takes the picture it is handed, and a repeating
        // decoder needs that picture again after the end: it converts a
        // reference and keeps the original, as the paced path does.
        let frame = if self.options.looping {
            let reference = source.frame.clone();
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
        // counts as the next one in line.
        while !self.source_done
            && self
                .pending
                .as_ref()
                .is_none_or(|next| next.pts.is_none_or(|pts| pts <= target))
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
            // instant; the file is over.
            if let Some(pts) = current.pts
                && pts + rate.frame_duration() <= target
            {
                return Ok(None);
            }
        }
        self.position = Some(target);
        self.tick += 1;
        let frame = current.frame.clone();
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

impl SeekableSource for Decoder {
    fn seek(&mut self, to: Rational) -> Result<()> {
        self.jump(to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        assert!(wide.is_wide() && !wide.is_hdr());
        assert!(
            video_filter(0, &options, 64, 64, Some(&wide))
                .contains(":in_transfer=auto:in_color_matrix=auto:in_range=auto:")
        );
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
            ten_bit: true,
            hardware: false,
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
        let _ = std::fs::remove_file(&path);
        let luma = decoded.pixels()[0];
        assert!(luma > 8 && luma < 250, "tone-mapped grey came out {luma}");
    }

    #[test]
    fn an_empty_chain_is_no_chain() {
        assert!(DecodeOptions::default().filtered("").filter_chain.is_none());
    }

    #[test]
    fn a_chain_that_escapes_its_slot_is_refused_before_anything_opens() {
        for bad in ["hue=s=0;movie=x", "split[a][b]", "a\nb"] {
            let result = Decoder::open(
                "irrelevant.mp4",
                &DecodeOptions::default().scaled_to(64, 64).filtered(bad),
            );
            assert!(
                matches!(result, Err(Error::InvalidFilterChain { .. })),
                "{bad:?} should have been refused",
            );
        }
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        assert!(Decoder::open("does-not-exist.mp4", &DecodeOptions::default()).is_err());
    }
}

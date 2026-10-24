// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Writing RGBA frames back out to a file.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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

    /// The FFmpeg encoders that make this codec, best first; an export
    /// takes the first that is linked and opens.
    ///
    /// A phone has its platform's hardware and nothing else: its FFmpeg is
    /// built LGPL with MediaCodec or VideoToolbox and none of the GPL
    /// encoders (scripts/ffmpeg-mobile.sh), and its cores are no match for
    /// its chip anyway.
    ///
    /// On a desktop the software encoders lead, and the machine's own
    /// comes after them for an FFmpeg built without them. With `hardware`,
    /// the GPU's leads for HEVC and AV1: VideoToolbox on macOS, and on
    /// Windows and Linux NVIDIA's NVENC, Intel's Quick Sync and AMD's AMF,
    /// in that order - many times faster than x265 and, at these rates, as
    /// good to look at. Whichever of those the machine has no chip for
    /// does not open, and the export takes the next. H.264 stays with x264
    /// wherever it is linked: the hardware H.264 encoders spend noticeably
    /// more bits for the same picture, and H.264 is the choice made for
    /// compatibility, not speed. Media Foundation, Windows' own, is last:
    /// it runs on whatever the machine has, and says little about how.
    pub fn encoders(self, hardware: bool) -> &'static [&'static str] {
        let macos = cfg!(target_os = "macos");
        let windows = cfg!(target_os = "windows");
        if cfg!(target_os = "android") {
            return match self {
                VideoCodec::H264 => &["h264_mediacodec"],
                VideoCodec::Hevc => &["hevc_mediacodec"],
                VideoCodec::Av1 => &["av1_mediacodec"],
            };
        }
        if cfg!(target_os = "ios") {
            return match self {
                VideoCodec::H264 => &["h264_videotoolbox"],
                VideoCodec::Hevc => &["hevc_videotoolbox"],
                VideoCodec::Av1 => &[],
            };
        }
        match self {
            VideoCodec::H264 if macos => &["libx264", "h264_videotoolbox"],
            VideoCodec::H264 if windows => {
                &["libx264", "h264_nvenc", "h264_qsv", "h264_amf", "h264_mf"]
            }
            VideoCodec::H264 => &["libx264", "h264_nvenc", "h264_qsv", "h264_amf"],
            VideoCodec::Hevc if hardware && macos => &["hevc_videotoolbox", "libx265"],
            VideoCodec::Hevc if macos => &["libx265", "hevc_videotoolbox"],
            VideoCodec::Hevc if hardware && windows => {
                &["hevc_nvenc", "hevc_qsv", "hevc_amf", "libx265", "hevc_mf"]
            }
            VideoCodec::Hevc if windows => {
                &["libx265", "hevc_nvenc", "hevc_qsv", "hevc_amf", "hevc_mf"]
            }
            VideoCodec::Hevc if hardware => &["hevc_nvenc", "hevc_qsv", "hevc_amf", "libx265"],
            VideoCodec::Hevc => &["libx265", "hevc_nvenc", "hevc_qsv", "hevc_amf"],
            VideoCodec::Av1 if macos => &["libsvtav1", "libaom-av1"],
            VideoCodec::Av1 if hardware => {
                &["av1_nvenc", "av1_qsv", "av1_amf", "libsvtav1", "libaom-av1"]
            }
            VideoCodec::Av1 => &["libsvtav1", "libaom-av1", "av1_nvenc", "av1_qsv", "av1_amf"],
        }
    }

    /// The encoders for it the linked FFmpeg carries, best first, less any
    /// the environment has switched off (see [`switched_on`]).
    fn linked(self, hardware: bool, ten_bit: bool) -> impl Iterator<Item = &'static str> {
        ffi::init();
        self.encoders(hardware)
            .iter()
            .copied()
            .filter(move |name| !ten_bit || takes_ten_bits(name))
            .filter(|name| switched_on(name, |key| std::env::var_os(key)))
            .filter(|name| encoder::find_by_name(name).is_some())
    }

    /// Whether the linked FFmpeg carries an encoder for it.
    pub fn available(self) -> bool {
        self.linked(true, false).next().is_some()
    }

    /// Whether an export in it would open here: an encoder for it is
    /// linked and, where that is the platform's hardware, the chip takes
    /// it. Linked is not enough on a phone: its FFmpeg carries MediaCodec's
    /// encoder for every codec, and few phones have an AV1 one.
    ///
    /// The export sheet asks this on its first publish, before the window
    /// shows, so it opens as little as it can: a linked software encoder
    /// answers on its own, and nothing is opened at all on a desktop. See
    /// [`encodable_among`] for the rest. A hardware encoder that is asked
    /// is opened once, at 720p, and the answer kept for the process.
    pub fn encodable(self) -> bool {
        static KNOWN: [OnceLock<bool>; 3] = [const { OnceLock::new() }; 3];
        let slot = VideoCodec::ALL
            .iter()
            .position(|codec| *codec == self)
            .expect("every codec is in ALL");
        *KNOWN[slot].get_or_init(|| {
            encodable_among(self.linked(true, false), |name| opens_here(self, name))
        })
    }

    /// Whether it can be written at ten bits here: some linked encoder for
    /// it takes ten-bit pictures. Not on a phone, whose encoders FFmpeg
    /// feeds eight bits only.
    pub fn ten_bit_available(self) -> bool {
        self.linked(true, true).next().is_some()
    }

    /// Whether the encoder an export would pick first is the platform's
    /// hardware rather than the CPU: the first linked one that is software
    /// or opens here. A desktop FFmpeg links NVENC, Quick Sync and AMF
    /// whatever GPU is in the box, so linked alone would say hardware on a
    /// machine that encodes on the CPU.
    ///
    /// The export sheet asks this on every publish, on the UI thread, so a
    /// vendor's encoder is never opened for it: only an export opens one,
    /// and what that export found is the answer from then on. Until then
    /// the sheet says nothing about hardware. See [`leads_with_hardware`].
    pub fn hardware_encoded(self, hardware: bool, ten_bit: bool) -> bool {
        leads_with_hardware(self.linked(hardware, ten_bit), probed, |name| {
            opens_here(self, name)
        })
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
    /// A file's: `bitrate_kbps` is the average the encoder aims for, and a
    /// busy passage may reach half again as much, so the size is known
    /// before the render and a calm passage is not padded to the target.
    /// What every editor's bitrate setting means, and what the export
    /// sheet asks for.
    Average,
}

impl RateMode {
    /// The name a document or a request stores.
    pub fn name(self) -> &'static str {
        match self {
            RateMode::Vbr => "vbr",
            RateMode::Cbr => "cbr",
            RateMode::Average => "average",
        }
    }

    /// Every mode, in the order a menu lists them.
    pub const ALL: [RateMode; 3] = [RateMode::Vbr, RateMode::Cbr, RateMode::Average];
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
    /// The CPU and nothing else: no hardware encoder is opened, and on a
    /// phone its chip is passed over for Android's own software component.
    /// For an export going again after the hardware encoder stopped taking
    /// frames part way through (see [`Error::Stalled`]); off otherwise.
    pub software_only: bool,
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
            software_only: false,
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

/// Which kind of encoder an FFmpeg encoder is: what it takes its pictures
/// in, and what it is steered by.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Family {
    /// x264, x265, SVT-AV1, libaom: planar 4:2:0, steered by a CRF.
    Software,
    /// Apple's: semi-planar 4:2:0, steered by a quality.
    VideoToolbox,
    /// Android's: semi-planar 4:2:0 at eight bits, steered by a bitrate.
    MediaCodec,
    /// Windows': semi-planar 4:2:0 at eight bits, steered by a bitrate.
    MediaFoundation,
    /// NVIDIA's NVENC: semi-planar 4:2:0, steered by a constant quality.
    Nvenc,
    /// Intel's Quick Sync: semi-planar 4:2:0, steered by a quality (ICQ).
    Qsv,
    /// AMD's AMF: semi-planar 4:2:0 at eight bits, steered by a QP.
    Amf,
}

impl Family {
    fn of(encoder_name: &str) -> Self {
        if encoder_name.ends_with("_videotoolbox") {
            Family::VideoToolbox
        } else if encoder_name.ends_with("_mediacodec") {
            Family::MediaCodec
        } else if encoder_name.ends_with("_mf") {
            Family::MediaFoundation
        } else if encoder_name.ends_with("_nvenc") {
            Family::Nvenc
        } else if encoder_name.ends_with("_qsv") {
            Family::Qsv
        } else if encoder_name.ends_with("_amf") {
            Family::Amf
        } else {
            Family::Software
        }
    }

    /// The pixel format the family's encoders take. Semi-planar for the
    /// hardware, which is the layout every one of them reads - MediaCodec
    /// also names planar, and a good share of phones refuse it.
    fn pixel_format(self, ten_bit: bool) -> Pixel {
        match (self, ten_bit) {
            (Family::Software, false) => Pixel::YUV420P,
            (Family::Software, true) => Pixel::YUV420P10LE,
            (_, false) => Pixel::NV12,
            (_, true) => Pixel::P010LE,
        }
    }

    /// Whether the family's encoders are told a bitrate rather than a
    /// quality, in VBR as in CBR.
    fn steered_by_bitrate(self) -> bool {
        matches!(self, Family::MediaCodec | Family::MediaFoundation)
    }

    /// Whether this is the platform's hardware rather than the CPU.
    fn hardware(self) -> bool {
        self != Family::Software
    }

    /// Whether opening one loads a GPU vendor's driver into the process:
    /// NVENC's libcuda, Quick Sync's oneVPL runtime, AMF's runtime. A
    /// broken driver takes the process down from there rather than
    /// returning an error - libcuda's context creation segfaults on some
    /// Linux laptops with two GPUs (#309) - and no in-process probe can
    /// catch that, so these are opened only by an export that has reached
    /// them in its list, never to answer a question about the machine.
    /// VideoToolbox, MediaCodec and Media Foundation are the operating
    /// system's own and answer with an error.
    fn vendor_driver(self) -> bool {
        matches!(self, Family::Nvenc | Family::Qsv | Family::Amf)
    }
}

/// Whether the environment leaves `encoder_name` on, given `var` to read
/// it. `CONCAT_HARDWARE_ENCODE=0` switches every hardware encoder off on a
/// desktop, for a machine whose driver crashes when one is opened; a
/// phone has nothing else, so it is not switched off there. NVENC is also
/// off when `CUDA_VISIBLE_DEVICES` hides every CUDA device - set but
/// empty, or leading with `-1` - since it then cannot open, and on the
/// laptops where libcuda crashes that is the usual way to say so.
fn switched_on(encoder_name: &str, var: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    let family = Family::of(encoder_name);
    if !family.hardware() || cfg!(any(target_os = "android", target_os = "ios")) {
        return true;
    }
    if var("CONCAT_HARDWARE_ENCODE").is_some_and(|value| value.to_string_lossy().trim() == "0") {
        return false;
    }
    if family == Family::Nvenc
        && let Some(devices) = var("CUDA_VISIBLE_DEVICES")
    {
        let devices = devices.to_string_lossy();
        let first = devices.split(',').next().unwrap_or("").trim();
        if first.is_empty() || first == "-1" {
            return false;
        }
    }
    true
}

/// [`VideoCodec::encodable`] over `names`, the linked encoders best first,
/// with `opens` to ask one. A software encoder anywhere in the list means
/// yes without opening anything: the export falls through to it if the
/// hardware ahead does not open. Without one - a phone, or a desktop
/// FFmpeg built without them - a system encoder is opened to find out,
/// and a vendor's is taken as linked, so the question never loads a
/// driver (see [`Family::vendor_driver`]); an export it does not open for
/// says so when it is tried.
fn encodable_among(
    names: impl Iterator<Item = &'static str>,
    mut opens: impl FnMut(&'static str) -> bool,
) -> bool {
    let names: Vec<&'static str> = names.collect();
    if names.iter().any(|name| !Family::of(name).hardware()) {
        return true;
    }
    names
        .into_iter()
        .any(|name| Family::of(name).vendor_driver() || opens(name))
}

/// [`VideoCodec::hardware_encoded`] over `names`, the linked encoders best
/// first: walking them as an export would, whether the one it stops at is
/// hardware. A system encoder is asked with `opens`; a vendor's is never
/// opened, and counts as open only if `known` says an export opened it
/// (see [`Family::vendor_driver`]).
fn leads_with_hardware(
    names: impl Iterator<Item = &'static str>,
    known: impl Fn(&str) -> Option<bool>,
    mut opens: impl FnMut(&'static str) -> bool,
) -> bool {
    for name in names {
        let family = Family::of(name);
        if !family.hardware() {
            return false;
        }
        let opened = if family.vendor_driver() {
            known(name).unwrap_or(false)
        } else {
            opens(name)
        };
        if opened {
            return true;
        }
    }
    false
}

/// Whether `encoder_name` opens for an eight-bit 720p30 file of `codec`:
/// the question a phone's chip answers only by being asked.
fn opens(codec: VideoCodec, encoder_name: &'static str) -> bool {
    let Some(found) = encoder::find_by_name(encoder_name) else {
        return false;
    };
    let options = EncodeOptions {
        codec,
        ..EncodeOptions::default()
    };
    let tags = (
        ffmpeg::color::Primaries::BT709,
        ffmpeg::color::TransferCharacteristic::BT709,
        ffmpeg::color::Space::BT709,
    );
    // Nothing is written: the path only names the file in an error.
    let result = open_video(
        encoder_name,
        found,
        Path::new("probe.mp4"),
        (1280, 720),
        ffmpeg::Rational::new(30, 1),
        &options,
        tags,
        None,
        true,
        Attempt::AsAsked,
    );
    if let Err(error) = &result {
        log::info!("{encoder_name} does not open here: {error}");
    }
    result.is_ok()
}

/// Whether each hardware encoder opens here, as a probe found or an export
/// that opened it showed: the answer is the machine's, and each question
/// opens an encoder.
fn opened_before() -> &'static std::sync::Mutex<std::collections::HashMap<&'static str, bool>> {
    static KNOWN: OnceLock<std::sync::Mutex<std::collections::HashMap<&'static str, bool>>> =
        OnceLock::new();
    KNOWN.get_or_init(Default::default)
}

/// Whether `encoder_name` opens here, or `None` if nothing has found out.
fn probed(encoder_name: &str) -> Option<bool> {
    opened_before()
        .lock()
        .ok()
        .and_then(|map| map.get(encoder_name).copied())
}

/// Remembers whether `encoder_name` opened.
fn remember(encoder_name: &'static str, opened: bool) {
    if let Ok(mut map) = opened_before().lock() {
        map.insert(encoder_name, opened);
    }
}

/// [`opens`], asked once per encoder and remembered for the process.
fn opens_here(codec: VideoCodec, encoder_name: &'static str) -> bool {
    if let Some(answer) = probed(encoder_name) {
        return answer;
    }
    let answer = opens(codec, encoder_name);
    remember(encoder_name, answer);
    answer
}

/// Whether the encoder takes ten-bit pictures. FFmpeg's MediaCodec and
/// Media Foundation encoders are fed eight bits only here; VideoToolbox
/// and AMF make ten-bit HEVC but not ten-bit H.264, and NVENC and Quick
/// Sync make ten-bit HEVC and AV1.
fn takes_ten_bits(encoder_name: &str) -> bool {
    match Family::of(encoder_name) {
        Family::Software => true,
        Family::VideoToolbox | Family::Amf => encoder_name.starts_with("hevc_"),
        Family::Nvenc | Family::Qsv => {
            encoder_name.starts_with("hevc_") || encoder_name.starts_with("av1_")
        }
        Family::MediaCodec | Family::MediaFoundation => false,
    }
}

/// The bitrate in kbps a CRF comes to, for an encoder that is steered by a
/// bitrate alone: the export sheet's rule for its size estimate - 16 Mb/s
/// for H.264 at 1080p30 and CRF 16, halving about every five steps -
/// scaled by the pixels, the rate and the codec's size factor.
fn implied_kbps(
    codec: VideoCodec,
    crf: u8,
    width: u32,
    height: u32,
    rate: ffmpeg::Rational,
) -> u32 {
    let fps = f64::from(rate.numerator()) / f64::from(rate.denominator().max(1));
    let pixels = f64::from(width) * f64::from(height) / (1920.0 * 1080.0);
    let mbps = 16.0
        * 2f64.powf((16.0 - f64::from(crf)) / 5.0)
        * pixels
        * (fps / 30.0)
        * f64::from(codec.size_factor());
    (mbps * 1000.0).round().clamp(500.0, 200_000.0) as u32
}

/// One encoder's context, set up and opened for the file: the frame's
/// size, rate and tags, and the settings `encoder_name` is steered by.
/// Returns the encoder and the pixel format it takes its pictures in.
/// How an encoder is asked to open. A phone's chip turns away an export
/// it cannot do with nothing past FFmpeg's "Generic error in an external
/// library" (#491) - a bitrate past its ceiling, a rate it was not built
/// for, colour keys an older component does not know - and a phone has no
/// other encoder linked. So a MediaCodec encoder that will not open as
/// asked is asked plainly, and then Android's own software encoder is
/// asked by name: slow, but on every phone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Attempt {
    /// Everything the export asked for.
    AsAsked,
    /// The chip's own rate control, a bitrate it can hold, a declared rate
    /// of 30 at most and no colour keys.
    Plain,
    /// One of Android's own components, by name, asked plainly.
    Component(&'static str),
}

/// The ways to ask `encoder_name`, in order.
fn attempts(encoder_name: &str) -> &'static [Attempt] {
    use Attempt::*;
    match encoder_name {
        // c2.android.* since Android 10; OMX.google.* before it.
        "h264_mediacodec" => &[
            AsAsked,
            Plain,
            Component("c2.android.avc.encoder"),
            Component("OMX.google.h264.encoder"),
        ],
        "hevc_mediacodec" => &[AsAsked, Plain, Component("c2.android.hevc.encoder")],
        "av1_mediacodec" => &[AsAsked, Plain, Component("c2.android.av1.encoder")],
        _ => &[AsAsked],
    }
}

impl Attempt {
    /// Whether `encoder_name` asked this way encodes on the CPU: one of
    /// Android's own components, or an encoder that is software already.
    fn in_software(self, encoder_name: &str) -> bool {
        match self {
            Attempt::Component(_) => true,
            Attempt::AsAsked | Attempt::Plain => !Family::of(encoder_name).hardware(),
        }
    }
}

/// The ways to ask `encoder_name`, in order, that `options` allows: every
/// one, or with [`EncodeOptions::software_only`] those on the CPU alone - on
/// a phone straight to Android's software component, past a chip that
/// stopped part way through the last try, and on a desktop the software
/// encoders, with NVENC, Quick Sync, AMF and the rest passed over.
fn allowed_attempts(
    encoder_name: &'static str,
    options: &EncodeOptions,
) -> impl Iterator<Item = Attempt> {
    let software_only = options.software_only;
    attempts(encoder_name)
        .iter()
        .copied()
        .filter(move |attempt| !software_only || attempt.in_software(encoder_name))
}

/// The most a plain ask puts on a phone's chip, in kb/s: room for a clean
/// 1080p, and twice that above it.
fn plain_kbps(width: u32, height: u32) -> u32 {
    if u64::from(width) * u64::from(height) <= 1920 * 1088 {
        16_000
    } else {
        32_000
    }
}

fn open_video(
    encoder_name: &'static str,
    codec: ffmpeg::Codec,
    path: &Path,
    (width, height): (u32, u32),
    rate: ffmpeg::Rational,
    options: &EncodeOptions,
    tags: (
        ffmpeg::color::Primaries,
        ffmpeg::color::TransferCharacteristic,
        ffmpeg::color::Space,
    ),
    hdr: Option<Signal>,
    global_header: bool,
    attempt: Attempt,
) -> Result<(encoder::video::Encoder, Pixel)> {
    let pq = hdr == Some(Signal::Pq);
    let family = Family::of(encoder_name);
    let plain = attempt != Attempt::AsAsked;
    let pixel_format = family.pixel_format(options.ten_bit);
    let cbr = options.rate_mode == RateMode::Cbr && options.bitrate_kbps > 0;
    let average = options.rate_mode == RateMode::Average && options.bitrate_kbps > 0;
    let mut video = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|error| ffi::fail("video encoder", path, error))?;
    video.set_width(width);
    video.set_height(height);
    video.set_format(pixel_format);
    video.set_time_base(rate.invert());
    let fps = f64::from(rate.numerator()) / f64::from(rate.denominator().max(1));
    // Asked plainly, a chip is told 30 at most: the rate is what it sizes
    // its work by and may refuse past, while the frames' times come from
    // the time base and are untouched.
    video.set_frame_rate(Some(if plain && fps > 30.5 {
        ffmpeg::Rational::new(30, 1)
    } else {
        rate
    }));
    let (primaries, transfer, matrix) = tags;
    // Asked plainly, no colour keys at all: an older codec2 component
    // refuses some it does not know, and untagged 8-bit video is read as
    // the limited-range Rec. 709 it is.
    if !plain {
        video.set_colorspace(matrix);
        // The range the frames will be converted to below, so the tag is
        // true; VideoToolbox reads it to pick its full- or video-range
        // pixel format, the software encoders write it into the stream.
        video.set_color_range(options.color_range.as_ffmpeg());
    }
    // SAFETY: `video` owns a live AVCodecContext; primaries and
    // transfer have no setter in the bindings, and all three are plain
    // fields the encoder reads at open.
    unsafe {
        let context = video.as_mut_ptr();
        if !plain {
            (*context).color_primaries = primaries.into();
            (*context).color_trc = transfer.into();
        }
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

    // The encoders steered by a bitrate alone take one whatever the mode:
    // the target in CBR, and in VBR the rate the CRF would come to.
    // Asked plainly, no more than a phone's chip is sure to hold.
    let ceiling = |kbps: u32| {
        if plain {
            kbps.min(plain_kbps(width, height))
        } else {
            kbps
        }
    };
    if family.steered_by_bitrate() {
        let kbps = if cbr || average {
            options.bitrate_kbps
        } else {
            implied_kbps(options.codec, options.crf, width, height, rate)
        };
        video.set_bit_rate(ceiling(kbps) as usize * 1000);
        // Two seconds between keyframes, where MediaCodec's own default
        // is one per second rounded from a GOP of twelve frames.
        video.set_gop((fps.round().max(1.0) * 2.0) as u32);
    }
    // An average, on the context as well as in each encoder's own words:
    // the AV1 encoders read the target and the ceiling from there.
    if average {
        let kbps = ceiling(options.bitrate_kbps);
        video.set_bit_rate(kbps as usize * 1000);
        video.set_max_bit_rate(kbps as usize * 1500);
    }
    let crf = options.crf.to_string();
    let bitrate = format!("{}k", options.bitrate_kbps);
    // The average's ceiling, half again the target, and a buffer of two
    // seconds' worth: room for a cut to a busy shot without the encoder
    // starving the frames after it.
    let peak = format!("{}k", options.bitrate_kbps * 3 / 2);
    let window = format!("{}k", options.bitrate_kbps * 2);
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
        "libx264" | "libx265" if average => ffmpeg::dict! {
            "preset" => options.preset.as_str(),
            "b" => bitrate.as_str(),
            "maxrate" => peak.as_str(),
            "bufsize" => window.as_str(),
        },
        "libx264" | "libx265" => ffmpeg::dict! {
            "preset" => options.preset.as_str(),
            "crf" => crf.as_str(),
        },
        // The context carries the target and the ceiling (above); the
        // encoder runs its own VBR to them.
        "libsvtav1" if average => ffmpeg::dict! {
            "preset" => &svt_preset(&options.preset).to_string(),
        },
        "libaom-av1" if average => ffmpeg::dict! {
            "cpu-used" => "6",
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
        // VideoToolbox steers by the context's bitrate when it is given no
        // quality.
        "hevc_videotoolbox" | "h264_videotoolbox" if average => ffmpeg::dict! {
            "profile" => match (encoder_name, options.ten_bit) {
                ("h264_videotoolbox", _) => "high",
                (_, true) => "main10",
                (_, false) => "main",
            },
            "allow_sw" => "1",
        },
        "hevc_videotoolbox" | "h264_videotoolbox" => ffmpeg::dict! {
            "q:v" => &videotoolbox_quality(options.crf).to_string(),
            "profile" => match (encoder_name, options.ten_bit) {
                ("h264_videotoolbox", _) => "high",
                (_, true) => "main10",
                (_, false) => "main",
            },
            // A machine without the hardware still gets a file.
            "allow_sw" => "1",
        },
        // A phone's chip. The NDK's MediaCodec, not the Java one: no
        // JavaVM is handed to FFmpeg, and the NDK needs none.
        _ if family == Family::MediaCodec => match attempt {
            Attempt::AsAsked => ffmpeg::dict! {
                "bitrate_mode" => if cbr { "cbr" } else { "vbr" },
            },
            // The chip's own rate control, whatever it is.
            Attempt::Plain => ffmpeg::Dictionary::new(),
            Attempt::Component(component) => ffmpeg::dict! {
                "codec_name" => component,
            },
        },
        // Windows' Media Foundation, whatever the machine has behind it.
        _ if family == Family::MediaFoundation => ffmpeg::dict! {
            "rate_control" => if cbr { "cbr" } else { "u_vbr" },
        },
        // The GPUs' own. Each is told the CRF as its own constant quality
        // in VBR - NVENC's CQ, Quick Sync's ICQ and AMF's QP all run on
        // the same 0-51 scale as x264's CRF - and the target in CBR.
        _ if family == Family::Nvenc && cbr => ffmpeg::dict! {
            "preset" => "p5",
            "rc" => "cbr",
            "b" => bitrate.as_str(),
            "maxrate" => bitrate.as_str(),
            "bufsize" => bufsize.as_str(),
        },
        _ if family == Family::Nvenc && average => ffmpeg::dict! {
            "preset" => "p5",
            "rc" => "vbr",
            "b" => bitrate.as_str(),
            "maxrate" => peak.as_str(),
            "bufsize" => window.as_str(),
        },
        _ if family == Family::Nvenc => ffmpeg::dict! {
            "preset" => "p5",
            "rc" => "vbr",
            "cq" => &options.crf.min(51).to_string(),
            // No ceiling but the quality's.
            "b" => "0",
        },
        _ if family == Family::Qsv && cbr => ffmpeg::dict! {
            "preset" => "medium",
            "b" => bitrate.as_str(),
            "maxrate" => bitrate.as_str(),
            "bufsize" => bufsize.as_str(),
        },
        _ if family == Family::Qsv && average => ffmpeg::dict! {
            "preset" => "medium",
            "b" => bitrate.as_str(),
            "maxrate" => peak.as_str(),
            "bufsize" => window.as_str(),
        },
        _ if family == Family::Qsv => ffmpeg::dict! {
            "preset" => "medium",
            "global_quality" => &options.crf.clamp(1, 51).to_string(),
        },
        _ if family == Family::Amf && cbr => ffmpeg::dict! {
            "quality" => "quality",
            "rc" => "cbr",
            "b" => bitrate.as_str(),
        },
        _ if family == Family::Amf && average => ffmpeg::dict! {
            "quality" => "quality",
            "rc" => "vbr_peak",
            "b" => bitrate.as_str(),
            "maxrate" => peak.as_str(),
        },
        _ if family == Family::Amf => ffmpeg::dict! {
            "quality" => "quality",
            "rc" => "cqp",
            "qp_i" => &options.crf.min(51).to_string(),
            "qp_p" => &options.crf.min(51).to_string(),
            "qp_b" => &options.crf.min(51).to_string(),
        },
        _ => ffmpeg::dict! {},
    };
    // HDR's own words to the software encoders, besides the tags they
    // read off the context: headers with every keyframe, and a PQ
    // file's mastering display.
    let mut settings = settings;
    // AMF writes ten-bit pictures under the Main profile unless told, and
    // a Main stream with ten-bit samples is one D3D11 will not decode.
    if encoder_name == "hevc_amf" && options.ten_bit {
        settings.set("profile", "main10");
    }
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
    Ok((encoder, pixel_format))
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
    /// Whether it is the platform's hardware doing the work. Not the same
    /// as the name's family: `h264_mediacodec` asked for Android's own
    /// component by name encodes on the CPU.
    hardware: bool,
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

        let mut output =
            ffmpeg::format::output(path).map_err(|error| ffi::fail("create", path, error))?;
        let global_header = output
            .format()
            .flags()
            .contains(format::Flags::GLOBAL_HEADER);

        // Every encoder the linked FFmpeg has for the codec, best first,
        // and among them the first that opens. Being linked is not being
        // able: a phone's FFmpeg carries MediaCodec's encoders for every
        // codec, and the chip it runs on has a few of them.
        let candidates: Vec<(&'static str, ffmpeg::Codec)> = options
            .codec
            .linked(options.hardware, options.ten_bit)
            .filter_map(|name| encoder::find_by_name(name).map(|codec| (name, codec)))
            .collect();
        if candidates.is_empty() {
            return Err(Error::Missing {
                what: "encoder",
                name: if options.ten_bit {
                    format!("{} at ten bits", options.codec.label())
                } else {
                    options.codec.label().to_owned()
                },
            });
        }
        let mut failure = None;
        let mut opened = None;
        'encoders: for (name, codec) in candidates {
            for attempt in allowed_attempts(name, options) {
                match open_video(
                    name,
                    codec,
                    path,
                    (width, height),
                    rate,
                    options,
                    tags,
                    hdr,
                    global_header,
                    attempt,
                ) {
                    Ok((encoder, pixel_format)) => {
                        // Opening proves the chip, for the export sheet's
                        // label. A refusal is not remembered: it may be
                        // this export's size or depth, not the machine.
                        if attempt == Attempt::AsAsked && Family::of(name).hardware() {
                            remember(name, true);
                        }
                        if attempt != Attempt::AsAsked {
                            log::warn!("{name} opened only when asked {attempt:?}");
                        }
                        let hardware = !attempt.in_software(name);
                        opened = Some((name, codec, encoder, pixel_format, hardware));
                        break 'encoders;
                    }
                    Err(error) => {
                        log::warn!("{name} would not open {attempt:?}: {error}");
                        failure = Some(error);
                    }
                }
            }
        }
        let Some((encoder_name, codec, encoder, pixel_format, hardware)) = opened else {
            // Nothing was even tried when the CPU alone was asked for and
            // every linked encoder is hardware.
            return Err(failure.unwrap_or_else(|| Error::Missing {
                what: "software encoder",
                name: options.codec.label().to_owned(),
            }));
        };

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
            hardware,
            color_range: options.color_range.as_ffmpeg(),
            deep,
            light: pq.then(LightLevel::new),
        })
    }

    /// Which FFmpeg encoder is doing the work, e.g. "hevc_videotoolbox".
    pub fn encoder_name(&self) -> &'static str {
        self.encoder_name
    }

    /// Whether the platform's hardware is doing the work rather than the
    /// CPU - false for a phone's export on Android's own software
    /// component, whatever the encoder's name.
    pub fn on_hardware(&self) -> bool {
        self.hardware
    }

    /// How many frames have been accepted so far.
    pub const fn written(&self) -> u64 {
        self.written
    }

    /// Hands the encoder a frame, or the end of the stream with `None`, as
    /// FFmpeg's send-and-receive contract asks: an encoder that answers
    /// "try again" has finished packets waiting to be read, and takes the
    /// same frame once they are. Android's MediaCodec encoders say so
    /// whenever their input buffers are full, which on a phone is most
    /// exports; treated as a failure, it ended every H.264 export there
    /// a fraction of a second in. An encoder that keeps saying it for
    /// [`ENCODER_PATIENCE`] is stuck, and says so as [`Error::Stalled`]:
    /// an export whose hardware it was goes again on the CPU.
    fn send(&mut self, frame: Option<&Video>) -> Result<()> {
        let sent = patiently(
            self,
            |this| match frame {
                Some(frame) => this.encoder.send_frame(frame),
                None => this.encoder.send_eof(),
            },
            ffi::is_again,
            Self::drain,
            ENCODER_PATIENCE,
        )?;
        sent.map_err(|refused| match refused {
            Refused::Error(error) => ffi::fail("encode", &self.path, error),
            Refused::Stalled => self.stalled(format!(
                "the encoder took no {} for {} seconds",
                if frame.is_some() {
                    "frame"
                } else {
                    "end of the stream"
                },
                ENCODER_PATIENCE.as_secs()
            )),
        })
    }

    /// The failure of an encoder that stopped, saying `detail` and whether
    /// it was the hardware that stopped.
    fn stalled(&self, detail: String) -> Error {
        Error::Stalled {
            operation: "encode",
            path: self.path.clone(),
            detail,
            hardware: self.hardware,
        }
    }

    /// Reads out what the encoder still holds once it has been told the
    /// stream ended, until it says it has handed back the last packet.
    /// FFmpeg's contract has no "try again" here, but its MediaCodec
    /// wrapper waits a few milliseconds for each output buffer and says
    /// "try again" when none has come yet, frames still inside it; stopping
    /// there cut the end off a phone's export with no error. An encoder
    /// that hands back nothing for [`ENCODER_PATIENCE`] is stuck, and the
    /// export fails saying so rather than writing a file short of its end.
    fn flush(&mut self) -> Result<()> {
        if until_ended(self, Self::read_out, ENCODER_PATIENCE)? {
            return Ok(());
        }
        Err(self.stalled(format!(
            "the encoder held its last frames for {} seconds",
            ENCODER_PATIENCE.as_secs()
        )))
    }

    /// Writes every packet the encoder has ready.
    fn drain(&mut self) -> Result<()> {
        self.read_out().map(|_| ())
    }

    /// Writes every packet the encoder has ready, and says whether that was
    /// its last or it has nothing more for now.
    fn read_out(&mut self) -> Result<Drained> {
        let mut packets = 0;
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
                    packets += 1;
                }
                Err(ffmpeg::Error::Eof) => return Ok(Drained::Ended),
                Err(error) if ffi::is_again(&error) => return Ok(Drained::Again { packets }),
                Err(error) => return Err(ffi::fail("encode", &self.path, error)),
            }
        }
    }
}

/// How long an encoder may keep answering "try again" to one frame, its
/// finished packets read out between asks, before the export calls it
/// stuck. A phone's MediaCodec encoder frees an input buffer within a few
/// milliseconds of its output being read; ten seconds is a stall.
const ENCODER_PATIENCE: std::time::Duration = std::time::Duration::from_secs(10);
/// The pause between two asks, so a busy hardware encoder is waited for
/// rather than spun on.
const ENCODER_WAIT: std::time::Duration = std::time::Duration::from_millis(2);

/// Why [`patiently`] gave up on a send.
#[derive(Debug, PartialEq)]
enum Refused<E> {
    /// The encoder refused it for a reason of its own.
    Error(E),
    /// The encoder kept saying "try again" for the whole of the patience.
    Stalled,
}

/// `send` until it is taken: on an error `again` calls a "try again",
/// `drain` reads out what the encoder has finished and `send` is asked
/// once more, for up to `patience`. Any other error ends it at once. A
/// failure to drain is the outer error; the send's own outcome the inner.
fn patiently<S, E>(
    state: &mut S,
    mut send: impl FnMut(&mut S) -> std::result::Result<(), E>,
    again: impl Fn(&E) -> bool,
    mut drain: impl FnMut(&mut S) -> Result<()>,
    patience: std::time::Duration,
) -> Result<std::result::Result<(), Refused<E>>> {
    let started = std::time::Instant::now();
    loop {
        match send(state) {
            Ok(()) => return Ok(Ok(())),
            Err(error) if !again(&error) => return Ok(Err(Refused::Error(error))),
            Err(_) if started.elapsed() >= patience => return Ok(Err(Refused::Stalled)),
            Err(_) => {
                drain(state)?;
                std::thread::sleep(ENCODER_WAIT);
            }
        }
    }
}

/// What reading an encoder's packets out came to.
#[derive(Debug, PartialEq)]
enum Drained {
    /// It has handed back its last packet.
    Ended,
    /// It has nothing more for now; `packets` came out before it said so.
    Again { packets: usize },
}

/// `read_out` until the encoder says it has handed back its last packet,
/// pausing [`ENCODER_WAIT`] between reads that end in "try again". The
/// patience runs from the last packet, so an encoder still handing frames
/// back is waited on however long its tail is; one that hands back nothing
/// for `patience` gives `false`.
fn until_ended<S>(
    state: &mut S,
    mut read_out: impl FnMut(&mut S) -> Result<Drained>,
    patience: std::time::Duration,
) -> Result<bool> {
    let mut last = std::time::Instant::now();
    loop {
        match read_out(state)? {
            Drained::Ended => return Ok(true),
            Drained::Again { packets } if packets > 0 => last = std::time::Instant::now(),
            Drained::Again { .. } if last.elapsed() >= patience => return Ok(false),
            Drained::Again { .. } => {}
        }
        std::thread::sleep(ENCODER_WAIT);
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

        self.send(Some(&converted))?;
        self.written += 1;
        self.drain()
    }

    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.send(None)?;
        self.flush()?;
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

    /// A "try again" is not a failure: the finished packets are read out
    /// and the same frame is sent again, as often as it takes (#286: every
    /// H.264 export on an Android phone ended there). Any other error ends
    /// the send at once, and an encoder that never takes the frame is
    /// called stuck rather than waited on for ever.
    #[test]
    fn a_frame_the_encoder_is_not_ready_for_is_sent_again() {
        #[derive(Default)]
        struct Fake {
            refusals: u32,
            sends: u32,
            drains: u32,
        }
        let again = |error: &&str| *error == "again";
        let patience = std::time::Duration::from_secs(5);

        let mut busy = Fake {
            refusals: 3,
            ..Fake::default()
        };
        let sent = patiently(
            &mut busy,
            |fake| {
                fake.sends += 1;
                if fake.refusals > 0 {
                    fake.refusals -= 1;
                    Err("again")
                } else {
                    Ok(())
                }
            },
            again,
            |fake| {
                fake.drains += 1;
                Ok(())
            },
            patience,
        )
        .unwrap();
        assert_eq!(sent, Ok(()));
        assert_eq!((busy.sends, busy.drains), (4, 3));

        let mut broken = Fake::default();
        let sent = patiently(
            &mut broken,
            |fake| {
                fake.sends += 1;
                Err("invalid")
            },
            again,
            |fake| {
                fake.drains += 1;
                Ok(())
            },
            patience,
        )
        .unwrap();
        assert_eq!(sent, Err(Refused::Error("invalid")));
        assert_eq!((broken.sends, broken.drains), (1, 0));

        let mut stuck = Fake::default();
        let sent = patiently(
            &mut stuck,
            |_| Err("again"),
            again,
            |_| Ok(()),
            std::time::Duration::ZERO,
        )
        .unwrap();
        assert_eq!(sent, Err(Refused::Stalled));
    }

    /// After the end of the stream, an encoder that says "try again" with
    /// frames still inside it (MediaCodec's) is read again until it says it
    /// has handed back the last, and one that hands back nothing for the
    /// whole of the patience is called stuck rather than taken as done.
    #[test]
    fn the_last_frames_are_waited_for() {
        let patience = std::time::Duration::from_secs(5);

        // Two empty reads, one with the tail, two more empty, then the end.
        let mut reads = vec![
            Drained::Ended,
            Drained::Again { packets: 0 },
            Drained::Again { packets: 0 },
            Drained::Again { packets: 3 },
            Drained::Again { packets: 0 },
            Drained::Again { packets: 0 },
        ];
        let mut asked = 0;
        let ended = until_ended(
            &mut reads,
            |reads| {
                asked += 1;
                Ok(reads.pop().expect("read past the end"))
            },
            patience,
        )
        .unwrap();
        assert!(ended);
        assert_eq!(asked, 6);

        let ended = until_ended(
            &mut (),
            |_| Ok(Drained::Again { packets: 0 }),
            std::time::Duration::ZERO,
        )
        .unwrap();
        assert!(!ended);

        let mut failing = ();
        let failed = until_ended(
            &mut failing,
            |_| {
                Err(Error::Ffi {
                    operation: "encode",
                    path: std::path::PathBuf::new(),
                    detail: "broken".into(),
                })
            },
            patience,
        );
        assert!(failed.is_err());
    }

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

    /// An average-bitrate export, the export sheet's: busy pictures come
    /// out near the target, so the size the sheet promised is the size
    /// written, and calm ones well under it, not padded up to it as CBR
    /// pads.
    #[test]
    fn an_average_export_keeps_near_its_target_and_does_not_pad() {
        let dir =
            std::env::temp_dir().join(format!("concat-media-abr-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        const KBPS: u32 = 2_000;
        const FRAMES: u32 = 60;
        let seconds = f64::from(FRAMES) / 30.0;
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            let options = EncodeOptions {
                codec,
                preset: "ultrafast".to_owned(),
                rate_mode: RateMode::Average,
                bitrate_kbps: KBPS,
                hardware: false,
                ..EncodeOptions::default()
            };
            let rate_of = |noisy: bool| {
                let path = dir.join(format!("{}-{noisy}.mp4", codec.name()));
                let mut encoder = Encoder::create(&path, 320, 180, FrameRate::THIRTY, &options)
                    .unwrap_or_else(|error| panic!("{codec:?} average opens: {error}"));
                let mut frame = Frame::black(320, 180);
                let mut state = 0x2545_F491_4F6C_DD1D_u64;
                for _ in 0..FRAMES {
                    if noisy {
                        for byte in frame.pixels_mut() {
                            state ^= state << 13;
                            state ^= state >> 7;
                            state ^= state << 17;
                            *byte = state as u8;
                        }
                    }
                    encoder.write_frame(&frame).expect("writes");
                }
                encoder.finish().expect("finishes");
                std::fs::metadata(&path).expect("written").len() as f64 * 8.0 / 1000.0 / seconds
            };
            let busy = rate_of(true);
            assert!(
                (f64::from(KBPS) * 0.6..=f64::from(KBPS) * 1.6).contains(&busy),
                "{codec:?} averaging {KBPS} kb/s came out at {busy:.0} kb/s on noise"
            );
            let calm = rate_of(false);
            assert!(
                calm < f64::from(KBPS) * 0.5,
                "{codec:?} averaging {KBPS} kb/s padded a black picture to {calm:.0} kb/s"
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
            software_only: false,
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

    /// An encoder steered by a bitrate alone is handed the rate the export
    /// sheet estimates the CRF at: 16 Mb/s for H.264 at 1080p30 and CRF 16,
    /// about a quarter of that at CRF 26, and less again for HEVC.
    #[test]
    fn a_bitrate_steered_encoder_gets_what_the_crf_comes_to() {
        let thirty = ffmpeg::Rational::new(30, 1);
        assert_eq!(
            implied_kbps(VideoCodec::H264, 16, 1920, 1080, thirty),
            16_000
        );
        assert_eq!(
            implied_kbps(VideoCodec::H264, 26, 1920, 1080, thirty),
            4_000
        );
        assert_eq!(
            implied_kbps(VideoCodec::Hevc, 16, 1920, 1080, thirty),
            9_600
        );
        let sixty = ffmpeg::Rational::new(60, 1);
        assert_eq!(
            implied_kbps(VideoCodec::H264, 16, 3840, 2160, sixty),
            128_000
        );
        // A thumbnail-sized file still gets a rate a chip will take.
        assert_eq!(implied_kbps(VideoCodec::H264, 26, 64, 64, thirty), 500);
    }

    /// Each kind of encoder is handed the pictures it reads, and ten bits
    /// only where it takes them.
    /// A phone's encoder that will not open as asked is asked plainly,
    /// then Android's own software one is asked by name; every other
    /// encoder is asked once, as before (#491).
    #[test]
    fn a_phone_encoder_is_asked_again_before_the_export_fails() {
        assert_eq!(
            attempts("h264_mediacodec"),
            [
                Attempt::AsAsked,
                Attempt::Plain,
                Attempt::Component("c2.android.avc.encoder"),
                Attempt::Component("OMX.google.h264.encoder"),
            ]
        );
        assert_eq!(attempts("hevc_mediacodec")[1], Attempt::Plain);
        assert_eq!(attempts("libx264"), [Attempt::AsAsked]);
        assert_eq!(attempts("h264_videotoolbox"), [Attempt::AsAsked]);
        assert_eq!(plain_kbps(1920, 1080), 16_000);
        assert_eq!(plain_kbps(1080, 1920), 16_000, "upright is the same size");
        assert_eq!(plain_kbps(3840, 2160), 32_000);
    }

    /// An export going again on the CPU after a phone's chip stopped part
    /// way through asks Android's own software components only, never the
    /// chip again; on a desktop, the software encoders only (#491).
    #[test]
    fn a_stalled_chip_is_passed_over_for_the_cpu() {
        let cpu = EncodeOptions {
            software_only: true,
            ..EncodeOptions::default()
        };
        let asked = |name: &'static str, options: &EncodeOptions| -> Vec<Attempt> {
            allowed_attempts(name, options).collect()
        };
        assert_eq!(
            asked("h264_mediacodec", &cpu),
            [
                Attempt::Component("c2.android.avc.encoder"),
                Attempt::Component("OMX.google.h264.encoder"),
            ]
        );
        assert_eq!(
            asked("hevc_mediacodec", &cpu),
            [Attempt::Component("c2.android.hevc.encoder")]
        );
        assert_eq!(asked("libx264", &cpu), [Attempt::AsAsked]);
        assert_eq!(asked("libsvtav1", &cpu), [Attempt::AsAsked]);
        for hardware in ["hevc_nvenc", "h264_videotoolbox", "av1_qsv", "h264_mf"] {
            assert!(asked(hardware, &cpu).is_empty(), "{hardware} is hardware");
        }
        // Not asked for, every way is tried, as before.
        let mut every = EncodeOptions::default();
        assert_eq!(
            asked("h264_mediacodec", &every),
            attempts("h264_mediacodec")
        );
        every.hardware = false;
        assert_eq!(asked("hevc_nvenc", &every), [Attempt::AsAsked]);
        // What each way comes to: the chip, or the CPU.
        assert!(!Attempt::AsAsked.in_software("h264_mediacodec"));
        assert!(!Attempt::Plain.in_software("h264_mediacodec"));
        assert!(Attempt::Component("c2.android.avc.encoder").in_software("h264_mediacodec"));
        assert!(Attempt::AsAsked.in_software("libx265"));
    }

    /// Only the hardware stopping is one an export goes again for: the CPU
    /// stopping, or anything else failing, would fail again.
    #[test]
    fn a_hardware_stall_is_told_apart() {
        let stall = |hardware| Error::Stalled {
            operation: "encode",
            path: PathBuf::from("out.mp4"),
            detail: "the encoder took no frame for 10 seconds".to_owned(),
            hardware,
        };
        assert!(stall(true).stalled_on_hardware());
        assert!(!stall(false).stalled_on_hardware());
        let other = Error::Ffi {
            operation: "encode",
            path: PathBuf::from("out.mp4"),
            detail: "Generic error in an external library".to_owned(),
        };
        assert!(!other.stalled_on_hardware());
        // Said as every other failure to encode is, so the words a person
        // reads are the same ones they reported.
        assert_eq!(
            stall(true).to_string(),
            "encode failed for out.mp4: the encoder took no frame for 10 seconds"
        );
    }

    /// Asked for the CPU alone, an export opens on a software encoder and
    /// says so, and still writes a file.
    #[test]
    fn a_software_only_encoder_opens_on_the_cpu() {
        let dir =
            std::env::temp_dir().join(format!("concat-media-cpu-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let path = dir.join("cpu.mp4");
        let options = EncodeOptions {
            preset: "ultrafast".to_owned(),
            software_only: true,
            ..EncodeOptions::default()
        };
        let mut encoder = Encoder::create(&path, 64, 64, FrameRate::THIRTY, &options)
            .expect("a software H.264 encoder is linked");
        assert!(!encoder.on_hardware(), "{}", encoder.encoder_name());
        assert_eq!(Family::of(encoder.encoder_name()), Family::Software);
        for _ in 0..5 {
            encoder.write_frame(&Frame::black(64, 64)).expect("writes");
        }
        encoder.finish().expect("finishes");
        assert!(std::fs::metadata(&path).expect("written").len() > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_encoder_family_takes_its_own_pictures() {
        assert_eq!(Family::of("libx264").pixel_format(false), Pixel::YUV420P);
        assert_eq!(Family::of("libx265").pixel_format(true), Pixel::YUV420P10LE);
        assert_eq!(
            Family::of("hevc_videotoolbox").pixel_format(true),
            Pixel::P010LE
        );
        assert_eq!(
            Family::of("h264_mediacodec").pixel_format(false),
            Pixel::NV12
        );
        assert!(Family::of("h264_mediacodec").steered_by_bitrate());
        assert!(!Family::of("libx264").steered_by_bitrate());
        assert!(takes_ten_bits("libx265") && takes_ten_bits("hevc_videotoolbox"));
        for eight in [
            "h264_videotoolbox",
            "hevc_mediacodec",
            "h264_mediacodec",
            "hevc_mf",
            "h264_nvenc",
            "h264_amf",
            "av1_amf",
        ] {
            assert!(!takes_ten_bits(eight), "{eight}");
        }
        assert!(takes_ten_bits("hevc_nvenc") && takes_ten_bits("av1_qsv"));
        assert!(takes_ten_bits("hevc_amf"));
        for gpu in ["h264_nvenc", "hevc_qsv", "av1_amf"] {
            assert!(Family::of(gpu).hardware(), "{gpu}");
            assert_eq!(Family::of(gpu).pixel_format(false), Pixel::NV12, "{gpu}");
            assert!(!Family::of(gpu).steered_by_bitrate(), "{gpu}");
        }
    }

    /// Every codec has an encoder listed for every platform but AV1 on
    /// iOS, and a phone lists its hardware alone, whatever `hardware` says:
    /// its FFmpeg has nothing else.
    #[test]
    fn a_phone_is_asked_for_its_own_hardware() {
        for codec in VideoCodec::ALL {
            for hardware in [false, true] {
                let names = codec.encoders(hardware);
                if cfg!(target_os = "android") {
                    assert!(names.iter().all(|name| name.ends_with("_mediacodec")));
                } else if cfg!(target_os = "ios") {
                    assert!(names.iter().all(|name| name.ends_with("_videotoolbox")));
                } else {
                    assert!(!names.is_empty(), "{codec:?}");
                }
            }
            // A desktop has a software encoder behind any hardware one, so
            // what is linked is what can be encoded.
            if !cfg!(any(target_os = "android", target_os = "ios")) {
                assert_eq!(codec.encodable(), codec.available(), "{codec:?}");
            }
        }
    }

    /// The export sheet's first question opens nothing while a software
    /// encoder is linked, wherever it sits in the list: on Linux the HEVC
    /// list with hardware leads with NVENC, and opening that to answer
    /// crashed in libcuda before the window showed (#309).
    #[test]
    fn a_linked_software_encoder_answers_without_opening_hardware() {
        let mut asked = Vec::new();
        let names = ["hevc_nvenc", "hevc_qsv", "hevc_amf", "libx265"].into_iter();
        assert!(encodable_among(names, |name| {
            asked.push(name);
            false
        }));
        assert!(asked.is_empty(), "opened {asked:?}");

        // A phone has only its system's encoders, and they are asked.
        let mut asked = Vec::new();
        assert!(!encodable_among(["av1_mediacodec"].into_iter(), |name| {
            asked.push(name);
            false
        }));
        assert_eq!(asked, ["av1_mediacodec"]);

        // A vendor's is taken as linked, never opened, even with nothing
        // behind it.
        assert!(encodable_among(["hevc_nvenc"].into_iter(), |name| panic!(
            "opened {name}"
        )));
    }

    /// The label opens no vendor's encoder: an NVENC an export has not
    /// opened is not counted, one it has is, and the walk stops at
    /// software as the export would.
    #[test]
    fn the_hardware_label_opens_no_vendor_driver() {
        let names = || ["hevc_nvenc", "hevc_qsv", "libx265"].into_iter();
        let never = |name: &'static str| -> bool { panic!("opened {name}") };
        assert!(!leads_with_hardware(names(), |_| None, never));
        assert!(leads_with_hardware(
            names(),
            |name| (name == "hevc_qsv").then_some(true),
            never
        ));
        assert!(!leads_with_hardware(
            ["libx265", "hevc_nvenc"].into_iter(),
            |_| Some(true),
            never
        ));
        // A system encoder is still asked.
        assert!(leads_with_hardware(
            ["hevc_videotoolbox", "libx265"].into_iter(),
            |_| None,
            |name| name == "hevc_videotoolbox"
        ));
    }

    /// `CONCAT_HARDWARE_ENCODE=0` leaves a desktop the CPU's encoders, and
    /// a `CUDA_VISIBLE_DEVICES` that hides every device takes NVENC alone
    /// out.
    #[test]
    fn the_environment_switches_hardware_encoders_off() {
        fn env<'a>(
            pairs: &'a [(&'a str, &'a str)],
        ) -> impl Fn(&str) -> Option<std::ffi::OsString> + 'a {
            move |key| {
                pairs
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| std::ffi::OsString::from(value))
            }
        }
        let desktop = !cfg!(any(target_os = "android", target_os = "ios"));
        assert!(switched_on("hevc_nvenc", env(&[])));
        assert!(switched_on(
            "libx265",
            env(&[("CONCAT_HARDWARE_ENCODE", "0")])
        ));
        assert_eq!(
            switched_on("hevc_qsv", env(&[("CONCAT_HARDWARE_ENCODE", "0")])),
            !desktop
        );
        assert!(switched_on(
            "hevc_qsv",
            env(&[("CONCAT_HARDWARE_ENCODE", "1")])
        ));
        for hidden in ["", "-1", " -1,0"] {
            let vars = [("CUDA_VISIBLE_DEVICES", hidden)];
            assert_eq!(switched_on("av1_nvenc", env(&vars)), !desktop, "{hidden:?}");
            assert!(switched_on("av1_qsv", env(&vars)), "{hidden:?}");
        }
        assert!(switched_on(
            "av1_nvenc",
            env(&[("CUDA_VISIBLE_DEVICES", "0")])
        ));
    }

    /// On a Mac, the fallback H.264 encoder for an FFmpeg without x264
    /// opens with the settings it is handed, and the iPhone's H.264 is the
    /// same encoder.
    #[test]
    fn the_hardware_h264_encoder_opens() {
        ffi::init();
        let Some(codec) = encoder::find_by_name("h264_videotoolbox") else {
            eprintln!("h264_videotoolbox not in the linked FFmpeg; skipped");
            return;
        };
        let path = std::env::temp_dir().join("concat-encode-h264-videotoolbox.mp4");
        let tags = (
            ffmpeg::color::Primaries::BT709,
            ffmpeg::color::TransferCharacteristic::BT709,
            ffmpeg::color::Space::BT709,
        );
        let options = EncodeOptions {
            codec: VideoCodec::H264,
            crf: 20,
            ..EncodeOptions::default()
        };
        let (_, format) = open_video(
            "h264_videotoolbox",
            codec,
            &path,
            (1280, 720),
            ffmpeg::Rational::new(30, 1),
            &options,
            tags,
            None,
            true,
            Attempt::AsAsked,
        )
        .expect("opens");
        assert_eq!(format, Pixel::NV12);
    }

    /// On a Radeon, ten-bit HEVC is AMF's, and its file says Main 10 - the
    /// profile a hardware decoder checks before it takes ten-bit samples.
    #[test]
    fn the_amd_hevc_encoder_writes_main_ten() {
        let path = std::env::temp_dir().join("concat-encode-hevc-amf.mp4");
        let options = EncodeOptions {
            codec: VideoCodec::Hevc,
            crf: 20,
            ten_bit: true,
            hardware: true,
            ..EncodeOptions::default()
        };
        let mut encoder =
            Encoder::create(&path, 1280, 720, FrameRate::THIRTY, &options).expect("creates");
        if encoder.encoder_name() != "hevc_amf" {
            eprintln!("{} chosen, not AMF; skipped", encoder.encoder_name());
            let _ = std::fs::remove_file(&path);
            return;
        }
        let frame = Frame::black(1280, 720);
        for _ in 0..4 {
            encoder.write_frame(&frame).expect("writes");
        }
        encoder.finish().expect("finishes");
        drop(encoder);

        let input = ffmpeg::format::input(&path).expect("opens");
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .expect("video");
        // SAFETY: the parameters are live for as long as `input` is, and
        // the profile is a plain field.
        let profile = unsafe { (*stream.parameters().as_ptr()).profile };
        drop(input);
        let (_, _, _, _, _, format, _) = tags_of(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(format, Pixel::YUV420P10LE);
        assert_eq!(profile, ffmpeg::sys::AV_PROFILE_HEVC_MAIN_10);
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
                    software_only: false,
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
            software_only: false,
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

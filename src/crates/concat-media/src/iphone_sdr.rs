// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The iPhone Dolby Vision 8.4 input transform for SDR timelines.

use std::path::Path;
use std::ptr::{self, NonNull};
use std::sync::OnceLock;

use ffmpeg::color::{Primaries, Range, Space, TransferCharacteristic as Transfer};
use ffmpeg::filter::Graph;
use ffmpeg::format::Pixel;
use ffmpeg::sys;
use ffmpeg::util::frame::side_data::Type;
use ffmpeg::util::frame::video::Video;
use ffmpeg_the_third as ffmpeg;

use crate::decode::{ColorRange, ColorSignal};
use crate::error::{Error, Result};
use crate::ffi;

/// Lets the pool keep using proxies when this build cannot apply the profile.
pub(crate) fn available() -> bool {
    if std::env::var("CONCAT_GPU").is_ok_and(|value| value.trim().eq_ignore_ascii_case("software"))
        || VULKAN.get().is_some_and(std::result::Result::is_err)
    {
        return false;
    }
    static HAS_FILTER: OnceLock<bool> = OnceLock::new();
    *HAS_FILTER.get_or_init(|| {
        ffi::init();
        ffmpeg::filter::find("libplacebo").is_some()
    })
}

pub(crate) fn is_iphone_dolby_vision_stream(stream: &ffmpeg::format::stream::Stream<'_>) -> bool {
    let parameters = stream.parameters();
    if parameters.id() != ffmpeg::codec::Id::HEVC {
        return false;
    }
    // SAFETY: the stream owns its parameters and their coded side data.
    let record = unsafe {
        let raw = &*parameters.as_ptr();
        if Primaries::from(raw.color_primaries) != Primaries::BT2020
            || Transfer::from(raw.color_trc) != Transfer::ARIB_STD_B67
        {
            return false;
        }
        sys::av_packet_side_data_get(
            raw.coded_side_data,
            raw.nb_coded_side_data,
            sys::AVPacketSideDataType::DOVI_CONF,
        )
        .as_ref()
    };
    record.is_some_and(|record| {
        // These first eight bytes of AVDOVIDecoderConfigurationRecord are
        // public fields in dovi_meta.h. Its full size can grow with FFmpeg,
        // and the bindings do not include this header.
        // SAFETY: the entry belongs to the stream and supplies its size.
        record.size >= 8
            && !record.data.is_null()
            && unsafe { *record.data.add(2) == 8 && *record.data.add(7) == 4 }
    })
}

#[derive(Default)]
pub(crate) struct Profile {
    graph: Option<(Input, Graph)>,
    next_pts: i64,
    warned_about_metadata: bool,
}

#[derive(PartialEq, Eq)]
struct Input {
    pixel: Pixel,
    width: u32,
    height: u32,
    aspect: ffmpeg::Rational,
    space: Space,
    range: Range,
}

impl Profile {
    pub(crate) fn convert(
        &mut self,
        path: &Path,
        source: &Video,
        rotation: i64,
        color: ColorSignal,
        range_override: Option<ColorRange>,
    ) -> Result<Option<Video>> {
        if source.side_data(Type::DOVI_METADATA).is_none()
            || !matches!(
                source.color_primaries(),
                Primaries::BT2020 | Primaries::Unspecified
            )
            || !matches!(
                source.color_transfer_characteristic(),
                Transfer::ARIB_STD_B67 | Transfer::Unspecified
            )
        {
            if !self.warned_about_metadata {
                log::warn!(
                    "{}: a frame lacks compatible Dolby Vision metadata, using standard colour conversion for it",
                    path.display()
                );
                self.warned_about_metadata = true;
            }
            return Ok(None);
        }

        let mut frame = Video::empty();
        // SAFETY: both frames are live and the destination is empty. This
        // takes a pixel buffer reference and copies the Dolby Vision side data.
        let ret = unsafe { sys::av_frame_ref(frame.as_mut_ptr(), source.as_ptr()) };
        check(ret).map_err(|error| ffi::fail("Dolby Vision frame reference", path, error))?;
        if frame.color_space() == Space::Unspecified {
            frame.set_color_space(color.matrix);
        }
        if let Some(range) = range_override {
            frame.set_color_range(range.as_ffmpeg());
        } else if frame.color_range() == Range::Unspecified {
            frame.set_color_range(color.range);
        }
        if frame.color_primaries() == Primaries::Unspecified {
            frame.set_color_primaries(color.primaries);
        }
        if frame.color_transfer_characteristic() == Transfer::Unspecified {
            frame.set_color_transfer_characteristic(color.transfer);
        }

        let input = Input {
            pixel: frame.format(),
            width: frame.width(),
            height: frame.height(),
            aspect: frame.aspect_ratio(),
            space: frame.color_space(),
            range: frame.color_range(),
        };
        if self
            .graph
            .as_ref()
            .is_none_or(|(current, _)| *current != input)
        {
            self.graph = Some((input, build(path, &frame, rotation)?));
            self.next_pts = 0;
            log::debug!(
                "{}: iPhone Dolby Vision 8.4 SDR profile active",
                path.display()
            );
        }
        let (_, graph) = self.graph.as_mut().expect("just built");
        let mut input = graph.get("in").expect("profile input");
        // libplacebo requires a later frame even with frame mixing off.
        // Give it a reference to this picture between its output ticks.
        // Only even input ticks produce output. This makes every caller
        // frame independently renderable, including a seek or a repeat.
        frame.set_pts(Some(self.next_pts));
        // SAFETY: the frame and input context are live. write_frame keeps
        // the reference with us so we can supply the same buffer twice.
        let ret = unsafe {
            (*frame.as_mut_ptr()).duration = 1;
            sys::av_buffersrc_write_frame(input.as_mut_ptr(), frame.as_ptr())
        };
        check(ret).map_err(|error| ffi::fail("Dolby Vision filter input", path, error))?;
        frame.set_pts(Some(self.next_pts + 1));
        self.next_pts += 2;
        input
            .source()
            .add(&frame)
            .map_err(|error| ffi::fail("Dolby Vision filter input", path, error))?;
        graph
            .get("out")
            .expect("profile output")
            .sink()
            .frame(&mut frame)
            .map_err(|error| ffi::fail("Dolby Vision filter output", path, error))?;
        frame.set_pts(source.pts());
        // SAFETY: both frames are live, and only the destination is modified.
        unsafe { (*frame.as_mut_ptr()).duration = (*source.as_ptr()).duration };
        Ok(Some(frame))
    }
}

fn check(ret: i32) -> std::result::Result<(), ffmpeg::Error> {
    if ret < 0 {
        return Err(ffmpeg::Error::from(ret));
    }
    Ok(())
}

fn build(path: &Path, frame: &Video, rotation: i64) -> Result<Graph> {
    ffi::begin();
    let fail = |error| ffi::fail_said("iPhone Dolby Vision 8.4 filter graph", path, error);
    let missing = |name: &str| Error::Missing {
        what: "filter",
        name: name.to_owned(),
    };
    let libplacebo = ffmpeg::filter::find("libplacebo").ok_or_else(|| missing("libplacebo"))?;
    let vulkan = VULKAN
        .get_or_init(Vulkan::open)
        .as_ref()
        .map_err(|error| fail(*error))?;
    let mut graph = Graph::new();
    let aspect = frame.aspect_ratio();
    let args = format!(
        "video_size={}x{}:pix_fmt={}:time_base=1/60:frame_rate=60:pixel_aspect={}:colorspace={}:range={}",
        frame.width(),
        frame.height(),
        Into::<sys::AVPixelFormat>::into(frame.format()).0,
        if aspect.numerator() > 0 && aspect.denominator() > 0 {
            aspect
        } else {
            ffmpeg::Rational(1, 1)
        },
        Into::<sys::AVColorSpace>::into(frame.color_space()).0,
        Into::<sys::AVColorRange>::into(frame.color_range()).0,
    );
    let mut input = graph
        .add(
            &ffmpeg::filter::find("buffer").ok_or_else(|| missing("buffer"))?,
            "in",
            &args,
        )
        .map_err(fail)?;
    // SAFETY: the context lives for this graph's lifetime.
    let input = unsafe { input.as_mut_ptr() };
    graph
        .add(
            &ffmpeg::filter::find("buffersink").ok_or_else(|| missing("buffersink"))?,
            "out",
            "",
        )
        .map_err(fail)?;
    // SAFETY: the live graph owns the filter context; its device reference
    // must be set before init, when libplacebo imports the Vulkan device.
    let profile = unsafe {
        let context = NonNull::new(sys::avfilter_graph_alloc_filter(
            graph.as_mut_ptr(),
            libplacebo.as_ptr(),
            c"profile".as_ptr(),
        ))
        .ok_or_else(|| {
            fail(ffmpeg::Error::Other {
                errno: libc::ENOMEM,
            })
        })?;
        (*context.as_ptr()).hw_device_ctx = sys::av_buffer_ref(vulkan.0.as_ptr());
        if (*context.as_ptr()).hw_device_ctx.is_null() {
            return Err(fail(ffmpeg::Error::Other {
                errno: libc::ENOMEM,
            }));
        }
        // The BAT tone maps the autorotated image at its native size.
        // Disable stateful peak analysis for seek/streaming agreement.
        check(sys::avfilter_init_str(
            context.as_ptr(),
            c"apply_dolbyvision=1:tonemapping=spline:gamut_mode=perceptual:colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=tv:format=yuv420p:peak_detect=0:fps=30"
                .as_ptr(),
        ))
        .map_err(fail)?;
        context.as_ptr()
    };
    if let Some(rotation) = ffi::rotation_filters(rotation) {
        graph
            .output("in", 0)
            .and_then(|parser| parser.input("profile", 0))
            .and_then(|parser| parser.parse(&format!("[in]{rotation}[profile]")))
            .map_err(fail)?;
    } else {
        // SAFETY: these filter contexts share the live graph and have
        // one video output and one video input respectively.
        check(unsafe { sys::avfilter_link(input, 0, profile, 0) }).map_err(fail)?;
    }
    // Constrain the output link to software YUV420P, since a Vulkan
    // device also lets libplacebo negotiate a hardware frame.
    graph
        .output("profile", 0)
        .and_then(|parser| parser.input("out", 0))
        .and_then(|parser| parser.parse("[profile]format=yuv420p[out]"))
        .map_err(fail)?;
    graph.validate().map_err(fail)?;
    Ok(graph)
}

static VULKAN: OnceLock<std::result::Result<Vulkan, ffmpeg::Error>> = OnceLock::new();

struct Vulkan(NonNull<sys::AVBufferRef>);

// SAFETY: graphs only take separate AVBufferRefs, which FFmpeg counts
// atomically. libplacebo imports FFmpeg's Vulkan queue lock callbacks for
// sharing the device across filter graphs. The owner outlives all graphs.
unsafe impl Send for Vulkan {}
unsafe impl Sync for Vulkan {}

impl Vulkan {
    fn open() -> std::result::Result<Self, ffmpeg::Error> {
        let mut device = ptr::null_mut();
        // SAFETY: valid local out-pointer. Null device and options match
        // the BAT's "-init_hw_device vulkan".
        check(unsafe {
            sys::av_hwdevice_ctx_create(
                &mut device,
                sys::AVHWDeviceType::VULKAN,
                ptr::null(),
                ptr::null_mut(),
                0,
            )
        })?;
        NonNull::new(device)
            .map(Self)
            .ok_or(ffmpeg::Error::InvalidData)
    }
}

impl Drop for Vulkan {
    fn drop(&mut self) {
        let mut device = self.0.as_ptr();
        // SAFETY: releases the Vulkan owner's AVBufferRef.
        unsafe { sys::av_buffer_unref(&mut device) };
    }
}

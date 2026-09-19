// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Decoding on the platform's own video hardware.
//!
//! Every platform the app ships on has a fixed-function video decoder that
//! turns H.264 and HEVC into pictures at a fraction of what the CPU spends,
//! and libavcodec drives each of them through one door: a device context
//! from `av_hwdevice_ctx_create`, handed to the codec before it opens, and
//! a `get_format` callback that picks the device's pixel format when the
//! codec offers it. The frames then come out of the codec in the device's
//! memory, and `av_hwframe_transfer_data` copies each one back into system
//! memory, where the filtergraph in [`crate::decode`] takes over exactly as
//! it does for a software frame.
//!
//! ## What decides
//!
//! A reader's [`DecodeOptions`](crate::DecodeOptions) carries an
//! [`HwPolicy`]. The default, [`HwPolicy::Preference`], follows one
//! process-wide setting, [`set_hardware_decode`], so the window's toggle
//! reaches every reader the pool, the preview, the export and the
//! thumbnails open without each of them being told. A reader can insist on
//! software or on a named device instead, which is what the tests do.
//!
//! The device is the platform's own: VideoToolbox on macOS and iOS, D3D11VA
//! on Windows, MediaCodec on Android. VAAPI on Linux is never chosen by
//! default and only ever by name, because on an Intel i915 it has hung the
//! GPU for every process on the machine until a reboot (issue #70).
//!
//! ## What happens when it does not work
//!
//! Everything falls back to software, and nothing a person sees changes but
//! the speed. A codec the device has no decoder for goes straight to
//! software with no message, since that is most codecs and a warning per
//! file would be noise. A device that cannot be created, a codec that will
//! not open with it, or a frame that will not copy back is a real failure,
//! logged once per process, and the reader carries on in software from the
//! frame it was at.
//!
//! ## What this is not, yet
//!
//! The frame is copied back to the CPU. On macOS the decoded picture lives
//! in an IOSurface the GPU could sample directly, and the compositor's wgpu
//! device could take it without the copy; that zero-copy path is the
//! follow-up to this module, and needs the frame to stay a hardware frame
//! all the way to the render.

use std::collections::HashMap;
use std::ptr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, Once};

use ffmpeg_the_third as ffmpeg;
use ffmpeg_the_third::codec::decoder;
use ffmpeg_the_third::format::Pixel;
use ffmpeg_the_third::sys;
use ffmpeg_the_third::util::frame::video::Video;

/// A platform's video decode hardware, as libavcodec names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HwDevice {
    /// Apple's, on macOS and iOS.
    VideoToolbox,
    /// Direct3D 11 video acceleration, on Windows.
    D3d11va,
    /// Android's.
    MediaCodec,
    /// The Linux video acceleration API. Never the default; see the module
    /// doc and issue #70.
    Vaapi,
}

impl HwDevice {
    /// Every device, in the order a menu would list them.
    pub const ALL: [HwDevice; 4] = [
        HwDevice::VideoToolbox,
        HwDevice::D3d11va,
        HwDevice::MediaCodec,
        HwDevice::Vaapi,
    ];

    /// The device this binary's platform has, or `None` where there is no
    /// device worth choosing unasked: Linux, whose VAAPI is opt-in only.
    pub fn platform_default() -> Option<Self> {
        if cfg!(any(target_os = "macos", target_os = "ios")) {
            Some(HwDevice::VideoToolbox)
        } else if cfg!(target_os = "windows") {
            Some(HwDevice::D3d11va)
        } else if cfg!(target_os = "android") {
            Some(HwDevice::MediaCodec)
        } else {
            None
        }
    }

    /// What the device is called on a label.
    pub fn label(self) -> &'static str {
        match self {
            HwDevice::VideoToolbox => "VideoToolbox",
            HwDevice::D3d11va => "Direct3D 11",
            HwDevice::MediaCodec => "MediaCodec",
            HwDevice::Vaapi => "VAAPI",
        }
    }

    /// What libavcodec calls the device: the name `-hwaccel` takes.
    pub fn name(self) -> &'static str {
        match self {
            HwDevice::VideoToolbox => "videotoolbox",
            HwDevice::D3d11va => "d3d11va",
            HwDevice::MediaCodec => "mediacodec",
            HwDevice::Vaapi => "vaapi",
        }
    }

    /// Whether the linked FFmpeg was built with this device at all. A
    /// device that is not here cannot be created, whatever the platform.
    pub fn linked(self) -> bool {
        crate::ffi::init();
        let wanted = self.av_type();
        let mut kind = sys::AVHWDeviceType::NONE;
        loop {
            // SAFETY: iterating a static table; NONE ends it.
            kind = unsafe { sys::av_hwdevice_iterate_types(kind) };
            if kind == sys::AVHWDeviceType::NONE {
                return false;
            }
            if kind == wanted {
                return true;
            }
        }
    }

    fn av_type(self) -> sys::AVHWDeviceType {
        match self {
            HwDevice::VideoToolbox => sys::AVHWDeviceType::VIDEOTOOLBOX,
            HwDevice::D3d11va => sys::AVHWDeviceType::D3D11VA,
            HwDevice::MediaCodec => sys::AVHWDeviceType::MEDIACODEC,
            HwDevice::Vaapi => sys::AVHWDeviceType::VAAPI,
        }
    }
}

/// How a reader chooses between the hardware and the CPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HwPolicy {
    /// Follow the process-wide preference; see [`set_hardware_decode`].
    /// The default, and what every reader the app opens uses.
    #[default]
    Preference,
    /// Software, whatever the preference says.
    Software,
    /// This device, whatever the preference says; software if it fails.
    Device(HwDevice),
}

impl HwPolicy {
    /// The device this policy asks for right now, or `None` for software.
    pub fn device(self) -> Option<HwDevice> {
        match self {
            HwPolicy::Preference => hardware_device(),
            HwPolicy::Software => None,
            HwPolicy::Device(device) => Some(device),
        }
    }
}

/// The process-wide preference: `0` for software, else one past the
/// device's index in [`HwDevice::ALL`]. An atomic rather than a lock
/// because every `Decoder::open` reads it.
static PREFERENCE: AtomicU8 = AtomicU8::new(0);

/// Turns hardware decode on or off for every reader opened from now on
/// that follows the preference, which is all of them unless they say
/// otherwise. On means the platform's own device, so on Linux it means
/// nothing until [`set_hardware_device`] names one. Readers already open
/// keep whatever they opened with.
pub fn set_hardware_decode(on: bool) {
    set_hardware_device(if on {
        HwDevice::platform_default()
    } else {
        None
    });
}

/// Whether the preference is currently for a device.
pub fn hardware_decode() -> bool {
    hardware_device().is_some()
}

/// Names the device readers that follow the preference should use, or
/// `None` for software. The way to opt into VAAPI.
pub fn set_hardware_device(device: Option<HwDevice>) {
    let code = device.map_or(0, |device| {
        HwDevice::ALL
            .iter()
            .position(|candidate| *candidate == device)
            .map_or(0, |index| index as u8 + 1)
    });
    PREFERENCE.store(code, Ordering::Relaxed);
}

/// The device the preference names, or `None` for software.
pub fn hardware_device() -> Option<HwDevice> {
    match PREFERENCE.load(Ordering::Relaxed) {
        0 => None,
        code => HwDevice::ALL.get(usize::from(code) - 1).copied(),
    }
}

/// One created device context: a reference to libavutil's
/// `AVHWDeviceContext`, shared by every reader on the device.
pub(crate) struct Device {
    kind: HwDevice,
    context: *mut sys::AVBufferRef,
}

// SAFETY: an AVBufferRef's count is atomic and the device context behind
// it is immutable once created; libavcodec itself shares one device across
// its decoding threads. The reference is only ever cloned or dropped here
// and read by the codec.
unsafe impl Send for Device {}
unsafe impl Sync for Device {}

impl Device {
    fn create(kind: HwDevice) -> Result<Self, ffmpeg::Error> {
        let mut context: *mut sys::AVBufferRef = ptr::null_mut();
        // SAFETY: the out-pointer is a valid local; a null device name and
        // no options ask for the platform's default device, which is the
        // only one a laptop has.
        let ret = unsafe {
            sys::av_hwdevice_ctx_create(
                &mut context,
                kind.av_type(),
                ptr::null(),
                ptr::null_mut(),
                0,
            )
        };
        if ret < 0 || context.is_null() {
            return Err(ffmpeg::Error::from(ret));
        }
        Ok(Self { kind, context })
    }

    /// Which device this is.
    pub(crate) fn kind(&self) -> HwDevice {
        self.kind
    }

    /// The pixel format `codec` decodes to on this device, if it can
    /// decode on it at all.
    fn pixel_format(&self, codec: &ffmpeg::Codec) -> Option<Pixel> {
        hardware_format(codec.as_ptr(), self.kind.av_type()).map(Pixel::from)
    }

    /// Hands the codec context this device, before it opens. Returns the
    /// device's pixel format when the codec can decode on it, and `None`,
    /// having touched nothing, when it cannot: that codec is a software
    /// one whatever the preference.
    pub(crate) fn attach(&self, context: &mut ffmpeg::codec::Context) -> Option<Pixel> {
        let codec = decoder::find(context.id())?;
        let format = self.pixel_format(&codec)?;
        // SAFETY: the context is not opened yet. The codec takes its own
        // reference to the device and releases it when it closes.
        unsafe {
            let raw = context.as_mut_ptr();
            (*raw).hw_device_ctx = sys::av_buffer_ref(self.context);
            if (*raw).hw_device_ctx.is_null() {
                return None;
            }
            (*raw).get_format = Some(pick_format);
        }
        Some(format)
    }
}

impl Clone for Device {
    fn clone(&self) -> Self {
        // SAFETY: a new reference to a live buffer; null only when out of
        // memory, which is not a case this process survives anyway.
        let context = unsafe { sys::av_buffer_ref(self.context) };
        assert!(!context.is_null(), "out of memory cloning a device context");
        Self {
            kind: self.kind,
            context,
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: our own reference, released once.
        unsafe { sys::av_buffer_unref(&mut self.context) };
    }
}

/// The devices created so far, one per kind, and the kinds that could not
/// be: a device that failed once is not tried again, and not logged again.
static DEVICES: Mutex<Option<HashMap<HwDevice, Option<Device>>>> = Mutex::new(None);

/// The process's device of this kind, created on first use, or `None`
/// when the platform has none to give.
pub(crate) fn device(kind: HwDevice) -> Option<Device> {
    crate::ffi::init();
    let mut guard = DEVICES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let devices = guard.get_or_insert_with(HashMap::new);
    devices
        .entry(kind)
        .or_insert_with(|| match Device::create(kind) {
            Ok(device) => {
                log::info!("video decode on {}", kind.label());
                Some(device)
            }
            Err(error) => {
                log::warn!(
                    "{} is not available, decoding in software: {error}",
                    kind.label()
                );
                None
            }
        })
        .clone()
}

/// The pixel format `codec` decodes to on a device of type `kind`, from
/// its table of hardware configurations, or `None` when the table has no
/// entry for the device.
fn hardware_format(
    codec: *const sys::AVCodec,
    kind: sys::AVHWDeviceType,
) -> Option<sys::AVPixelFormat> {
    if codec.is_null() {
        return None;
    }
    let mut index = 0;
    loop {
        // SAFETY: the codec is a static libavcodec table entry, and the
        // config table is walked by index until a null ends it.
        let config = unsafe { sys::avcodec_get_hw_config(codec, index) };
        if config.is_null() {
            return None;
        }
        // SAFETY: non-null, and static like the codec.
        let config = unsafe { &*config };
        if config.device_type == kind
            && config.methods & sys::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX.0 as libc::c_int != 0
        {
            return Some(config.pix_fmt);
        }
        index += 1;
    }
}

/// The codec's `get_format`: the device's pixel format when the codec
/// offers it, else what libavcodec would have picked on its own. The
/// codec calls this again, without the device's format, when setting the
/// device up for the stream failed, and that second call is how a stream
/// the hardware refuses ends up decoding in software with no help from us.
unsafe extern "C" fn pick_format(
    context: *mut sys::AVCodecContext,
    formats: *const sys::AVPixelFormat,
) -> sys::AVPixelFormat {
    // SAFETY: libavcodec hands a live context and a NONE-terminated list.
    unsafe {
        let device = (*context).hw_device_ctx;
        let wanted = if device.is_null() {
            None
        } else {
            let kind = (*((*device).data as *const sys::AVHWDeviceContext)).type_;
            hardware_format((*context).codec, kind)
        };
        if let Some(wanted) = wanted {
            let mut cursor = formats;
            while *cursor != sys::AVPixelFormat::NONE {
                if *cursor == wanted {
                    return wanted;
                }
                cursor = cursor.add(1);
            }
        }
        sys::avcodec_default_get_format(context, formats)
    }
}

/// Copies a frame out of the device's memory into a fresh software frame,
/// in whatever format the device hands back (NV12, for the most part), with
/// its timestamp and colour tags intact.
pub(crate) fn download(frame: &Video) -> Result<Video, ffmpeg::Error> {
    let mut copy = Video::empty();
    // SAFETY: both frames are live; the destination is empty, so the
    // transfer allocates it in the device's first software format.
    unsafe {
        let ret = sys::av_hwframe_transfer_data(copy.as_mut_ptr(), frame.as_ptr(), 0);
        if ret < 0 {
            return Err(ffmpeg::Error::from(ret));
        }
        let ret = sys::av_frame_copy_props(copy.as_mut_ptr(), frame.as_ptr());
        if ret < 0 {
            return Err(ffmpeg::Error::from(ret));
        }
    }
    Ok(copy)
}

/// Writes down, once per process, that a reader gave up on the hardware
/// and why. Once, because the reason is the same for every file after the
/// first and the log is a person's, not a profiler's.
pub(crate) fn note_fallback(device: HwDevice, stage: &str, detail: &str) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        log::warn!(
            "{} could not {stage}; decoding in software from here on: {detail}",
            device.label()
        );
    });
}

/// The name libavutil gives a device type.
#[cfg(test)]
fn type_name(kind: HwDevice) -> String {
    // SAFETY: a pointer to a static string, or null for an unknown type.
    let raw = unsafe { sys::av_hwdevice_get_type_name(kind.av_type()) };
    if raw.is_null() {
        return kind.name().to_owned();
    }
    // SAFETY: checked non-null; a NUL-terminated static string.
    unsafe { std::ffi::CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_platform_default_is_never_vaapi() {
        assert_ne!(HwDevice::platform_default(), Some(HwDevice::Vaapi));
        if cfg!(target_os = "macos") {
            assert_eq!(HwDevice::platform_default(), Some(HwDevice::VideoToolbox));
        }
    }

    #[test]
    fn the_preference_round_trips() {
        // Other tests in this process read the preference through their
        // own readers, so this one only sets what it found.
        let before = hardware_device();
        set_hardware_device(Some(HwDevice::Vaapi));
        assert_eq!(hardware_device(), Some(HwDevice::Vaapi));
        assert_eq!(HwPolicy::Preference.device(), Some(HwDevice::Vaapi));
        assert_eq!(HwPolicy::Software.device(), None);
        assert_eq!(
            HwPolicy::Device(HwDevice::D3d11va).device(),
            Some(HwDevice::D3d11va)
        );
        set_hardware_device(before);
        assert_eq!(hardware_device(), before);
    }

    #[test]
    fn the_names_match_libavutil() {
        for device in HwDevice::ALL {
            assert_eq!(type_name(device), device.name(), "{device:?}");
        }
    }

    #[test]
    fn a_device_the_library_lacks_is_not_created() {
        // The linked FFmpeg on a Mac has VideoToolbox and nothing else;
        // asking for a device it lacks is a clean None, once.
        let absent = HwDevice::ALL.into_iter().find(|device| !device.linked());
        if let Some(absent) = absent {
            assert!(device(absent).is_none());
            assert!(device(absent).is_none());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn videotoolbox_decodes_h264_and_not_jpeg() {
        assert!(HwDevice::VideoToolbox.linked());
        let device = device(HwDevice::VideoToolbox).expect("a Mac has VideoToolbox");
        assert_eq!(device.kind(), HwDevice::VideoToolbox);
        let h264 = decoder::find(ffmpeg::codec::Id::H264).expect("an H.264 decoder");
        assert_eq!(device.pixel_format(&h264), Some(Pixel::VIDEOTOOLBOX));
        let jpeg = decoder::find(ffmpeg::codec::Id::MJPEG).expect("a JPEG decoder");
        assert_eq!(device.pixel_format(&jpeg), None);
    }
}

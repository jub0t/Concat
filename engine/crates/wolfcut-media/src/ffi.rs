// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Linked FFmpeg, as opposed to spawned FFmpeg.
//!
//! This backend exists for the two things a pipe fundamentally cannot do:
//! report a frame's real presentation timestamp, and seek to an exact point.
//! Both are needed for playback and scrubbing; neither matters for probing or
//! export, which is why the subprocess backend is not going anywhere.
//!
//! Only compiled with the `ffi` feature. See
//! `docs/decisions/0002-ffmpeg-over-a-pipe.md`.
//!
//! ## On the unsafe in here
//!
//! Every raw pointer this module owns is allocated in [`FfiDecoder::open`] and
//! freed in exactly one place, [`FfiDecoder::drop`]. Nothing escapes: the only
//! thing callers ever receive is an owned [`Frame`] of copied pixels. That
//! containment is what keeps the `unsafe` auditable - it is a handful of
//! functions, not a lifetime that leaks across the crate.

use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr;

use wolfcut_core::frame::Frame;
use wolfcut_core::time::Rational;
use rusty_ffmpeg::ffi;

use crate::decode::{FrameSource, SeekableSource};
use crate::error::{Error, Result};

/// The version of FFmpeg this binary is linked against.
///
/// The first thing worth checking when something behaves oddly: it proves
/// which library actually loaded, which is not always the one you expect on a
/// machine with several FFmpeg installs.
pub fn linked_version() -> String {
    // SAFETY: av_version_info returns a pointer to a static NUL-terminated
    // string compiled into the library. Valid for the process lifetime.
    let raw = unsafe { ffi::av_version_info() };
    if raw.is_null() {
        return "unknown".to_owned();
    }
    // SAFETY: checked non-null; contract is a NUL-terminated C string.
    unsafe { CStr::from_ptr(raw) }.to_string_lossy().into_owned()
}

/// Major version of the linked `libavcodec`.
pub fn avcodec_major() -> u32 {
    // SAFETY: pure accessor returning a packed integer, no pointers involved.
    unsafe { ffi::avcodec_version() >> 16 }
}

/// Turns a libav error code into the message FFmpeg itself would print.
fn describe(code: i32) -> String {
    let mut buffer = [0i8; ffi::AV_ERROR_MAX_STRING_SIZE as usize];
    // SAFETY: av_strerror writes at most `len` bytes into the buffer and
    // NUL-terminates. The buffer outlives the call.
    let written = unsafe { ffi::av_strerror(code, buffer.as_mut_ptr(), buffer.len()) };
    if written < 0 {
        return format!("error {code}");
    }
    // SAFETY: av_strerror NUL-terminates on success.
    unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy().into_owned()
}

/// `AVERROR(EAGAIN)`: the decoder wants more input before it can produce output.
///
/// Spelled out rather than taken from the bindings because `EAGAIN` comes from
/// the C library's `errno.h`, which is not part of the FFmpeg headers and is
/// therefore not always present in generated bindings. The value is *not* the
/// same everywhere: 11 on Linux and on Windows FFmpeg builds, 35 on macOS,
/// iOS and the BSDs - discovered the honest way, as "Resource temporarily
/// unavailable" errors from a perfectly healthy decoder.
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
const AVERROR_EAGAIN: i32 = -35;
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
)))]
const AVERROR_EAGAIN: i32 = -11;

/// A decoder that links FFmpeg rather than spawning it.
///
/// Produces frames as RGBA8 scaled to a fixed size, alongside the real
/// presentation timestamp of each.
pub struct FfiDecoder {
    path: PathBuf,
    format: *mut ffi::AVFormatContext,
    codec: *mut ffi::AVCodecContext,
    scaler: *mut ffi::SwsContext,
    decoded: *mut ffi::AVFrame,
    packet: *mut ffi::AVPacket,
    stream: i32,
    /// Ticks per second for this stream's timestamps.
    time_base: ffi::AVRational,
    width: u32,
    height: u32,
    position: Option<Rational>,
    finished: bool,
}

// SAFETY: every pointer here is exclusively owned by this struct - none are
// shared, and `&mut self` is required to touch any of them. FFmpeg contexts
// may be used from any single thread, just not concurrently, which is exactly
// what Rust's borrow rules already enforce for the owner.
unsafe impl Send for FfiDecoder {}

impl FfiDecoder {
    /// Opens `path`, producing frames scaled to `width` by `height`.
    pub fn open(path: impl AsRef<Path>, width: u32, height: u32) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let fail = |operation: &'static str, code: i32| Error::Ffi {
            operation,
            path: path.clone(),
            detail: describe(code),
        };

        let c_path = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| Error::Ffi {
            operation: "path conversion",
            path: path.clone(),
            detail: "path contains an interior NUL byte".to_owned(),
        })?;

        // Built up in pieces, and every early return from here on has to undo
        // what came before it - hence `Guard`, which owns the half-built state
        // until construction succeeds.
        let mut guard = Guard::default();

        unsafe {
            // SAFETY: `format` is null, so avformat_open_input allocates it and
            // writes the pointer back. On failure it leaves it null.
            let code = ffi::avformat_open_input(
                &mut guard.format,
                c_path.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
            );
            if code < 0 {
                return Err(fail("avformat_open_input", code));
            }

            // SAFETY: format is open; this reads packets to fill in stream info.
            let code = ffi::avformat_find_stream_info(guard.format, ptr::null_mut());
            if code < 0 {
                return Err(fail("avformat_find_stream_info", code));
            }

            let mut decoder: *const ffi::AVCodec = ptr::null();
            // SAFETY: format is open; decoder is written on success.
            let stream = ffi::av_find_best_stream(
                guard.format,
                ffi::AVMEDIA_TYPE_VIDEO,
                -1,
                -1,
                &mut decoder,
                0,
            );
            if stream < 0 {
                return Err(Error::NoVideoStream { path: path.clone() });
            }

            // SAFETY: av_find_best_stream returned a valid index into streams.
            let stream_ptr = *(*guard.format).streams.offset(stream as isize);
            let time_base = (*stream_ptr).time_base;

            // SAFETY: decoder was written by av_find_best_stream.
            guard.codec = ffi::avcodec_alloc_context3(decoder);
            if guard.codec.is_null() {
                return Err(fail("avcodec_alloc_context3", 0));
            }

            // SAFETY: both pointers are valid and the codec context is fresh.
            let code = ffi::avcodec_parameters_to_context(guard.codec, (*stream_ptr).codecpar);
            if code < 0 {
                return Err(fail("avcodec_parameters_to_context", code));
            }

            // SAFETY: context is populated from the stream's parameters.
            let code = ffi::avcodec_open2(guard.codec, decoder, ptr::null_mut());
            if code < 0 {
                return Err(fail("avcodec_open2", code));
            }

            let source_width = (*guard.codec).width;
            let source_height = (*guard.codec).height;
            let source_format = (*guard.codec).pix_fmt;

            // SAFETY: dimensions and format come from the opened codec.
            guard.scaler = ffi::sws_getContext(
                source_width,
                source_height,
                source_format,
                width as i32,
                height as i32,
                ffi::AV_PIX_FMT_RGBA,
                ffi::SWS_BILINEAR as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            if guard.scaler.is_null() {
                return Err(fail("sws_getContext", 0));
            }

            // SAFETY: plain allocations, checked for null.
            guard.decoded = ffi::av_frame_alloc();
            guard.packet = ffi::av_packet_alloc();
            if guard.decoded.is_null() || guard.packet.is_null() {
                return Err(fail("av_frame_alloc", 0));
            }

            Ok(Self {
                path,
                format: guard.take_format(),
                codec: guard.take_codec(),
                scaler: guard.take_scaler(),
                decoded: guard.take_decoded(),
                packet: guard.take_packet(),
                stream,
                time_base,
                width,
                height,
                position: None,
                finished: false,
            })
        }
    }

    /// Converts a stream timestamp into seconds, exactly.
    ///
    /// This is the whole point of the linked backend: `time_base` is a
    /// rational, the timestamp is an integer count of those ticks, and
    /// `wolfcut-core` speaks rationals - so the value survives with no rounding
    /// anywhere along the way.
    ///
    /// `None` when the container's numbers cannot form a representable value -
    /// a degenerate time base, or a product that overflows. Both come straight
    /// from an arbitrary file, so they must degrade to "position unknown"
    /// rather than panic the process.
    fn to_seconds(&self, ticks: i64) -> Option<Rational> {
        if self.time_base.den == 0 {
            return None;
        }
        Rational::checked_new(
            i128::from(ticks) * i128::from(self.time_base.num),
            i128::from(self.time_base.den),
        )
    }

    /// Scales the decoded frame into a fresh RGBA buffer.
    fn convert(&mut self) -> Frame {
        let stride = self.width as usize * 4;
        let mut pixels = vec![0u8; stride * self.height as usize];

        let destination: [*mut u8; 4] = [pixels.as_mut_ptr(), ptr::null_mut(), ptr::null_mut(), ptr::null_mut()];
        let strides: [i32; 4] = [stride as i32, 0, 0, 0];

        // SAFETY: `decoded` holds a complete frame from the codec, the scaler
        // was built for exactly that source format and this destination size,
        // and `pixels` is stride * height bytes - the size sws_scale writes.
        unsafe {
            ffi::sws_scale(
                self.scaler,
                (*self.decoded).data.as_ptr() as *const *const u8,
                (*self.decoded).linesize.as_ptr(),
                0,
                (*self.decoded).height,
                destination.as_ptr(),
                strides.as_ptr(),
            );
        }

        Frame::from_rgba(self.width, self.height, pixels)
            .expect("buffer is sized from the same width and height")
    }
}

impl FrameSource for FfiDecoder {
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
        if self.finished {
            return Ok(None);
        }

        loop {
            // SAFETY: codec is open; `decoded` is a valid, reusable frame.
            let code = unsafe { ffi::avcodec_receive_frame(self.codec, self.decoded) };

            if code == 0 {
                // SAFETY: receive_frame succeeded, so the frame is populated.
                let ticks = unsafe { (*self.decoded).best_effort_timestamp };
                self.position =
                    (ticks != ffi::AV_NOPTS_VALUE).then(|| self.to_seconds(ticks)).flatten();
                return Ok(Some(self.convert()));
            }

            if code == ffi::AVERROR_EOF {
                self.finished = true;
                return Ok(None);
            }

            if code != AVERROR_EAGAIN {
                return Err(Error::Ffi {
                    operation: "avcodec_receive_frame",
                    path: self.path.clone(),
                    detail: describe(code),
                });
            }

            // EAGAIN: the decoder needs another packet before it can produce
            // anything. Packets for other streams are skipped, not decoded.
            loop {
                // SAFETY: format is open; av_read_frame fills the packet, and
                // we unref it below whatever happens.
                let code = unsafe { ffi::av_read_frame(self.format, self.packet) };

                if code == ffi::AVERROR_EOF {
                    // Flush: a null packet tells the decoder to emit whatever
                    // it is still holding, which is how the final few frames
                    // of a file come out at all.
                    // SAFETY: passing null is the documented flush signal.
                    unsafe { ffi::avcodec_send_packet(self.codec, ptr::null()) };
                    break;
                }
                if code < 0 {
                    return Err(Error::Ffi {
                        operation: "av_read_frame",
                        path: self.path.clone(),
                        detail: describe(code),
                    });
                }

                // SAFETY: packet is populated by av_read_frame.
                let belongs = unsafe { (*self.packet).stream_index } == self.stream;
                if belongs {
                    // SAFETY: sending a valid packet to an open decoder.
                    let sent = unsafe { ffi::avcodec_send_packet(self.codec, self.packet) };
                    // SAFETY: releases the packet's buffer for reuse.
                    unsafe { ffi::av_packet_unref(self.packet) };

                    if sent < 0 && sent != AVERROR_EAGAIN {
                        return Err(Error::Ffi {
                            operation: "avcodec_send_packet",
                            path: self.path.clone(),
                            detail: describe(sent),
                        });
                    }
                    break;
                }

                // SAFETY: not our stream, so drop it and read the next.
                unsafe { ffi::av_packet_unref(self.packet) };
            }
        }
    }
}

impl SeekableSource for FfiDecoder {
    fn seek(&mut self, to: Rational) -> Result<()> {
        // Convert seconds back into this stream's tick count. The time base
        // comes from the file, so a degenerate one is an error, not a panic.
        if self.time_base.num == 0 || self.time_base.den == 0 {
            return Err(Error::Ffi {
                operation: "seek time base",
                path: self.path.clone(),
                detail: format!(
                    "stream has a degenerate time base {}/{}",
                    self.time_base.num, self.time_base.den
                ),
            });
        }
        let ticks_per_second = Rational::new(self.time_base.den as i64, self.time_base.num as i64);
        let target = (to * ticks_per_second).floor();

        // BACKWARD lands on the keyframe at or before the target. Decoding
        // forward from there is what makes the seek frame-accurate rather
        // than keyframe-accurate - which is precisely what the pipe cannot do.
        // SAFETY: format is open and the stream index came from it.
        let code = unsafe {
            ffi::av_seek_frame(self.format, self.stream, target, ffi::AVSEEK_FLAG_BACKWARD as i32)
        };
        if code < 0 {
            return Err(Error::Ffi {
                operation: "av_seek_frame",
                path: self.path.clone(),
                detail: describe(code),
            });
        }

        // The decoder is still holding frames from before the jump.
        // SAFETY: codec is open.
        unsafe { ffi::avcodec_flush_buffers(self.codec) };

        self.finished = false;
        self.position = None;
        Ok(())
    }
}

impl Drop for FfiDecoder {
    fn drop(&mut self) {
        // SAFETY: each pointer was allocated in `open` and is freed exactly
        // once, here. The struct cannot be used after this.
        unsafe {
            ffi::av_packet_free(&mut self.packet);
            ffi::av_frame_free(&mut self.decoded);
            ffi::sws_freeContext(self.scaler);
            ffi::avcodec_free_context(&mut self.codec);
            ffi::avformat_close_input(&mut self.format);
        }
    }
}

/// Owns half-built state so that an early return from `open` still frees it.
///
/// Without this, every `?` between the first allocation and the last would
/// leak whatever had been built up to that point - and there are six of them.
#[derive(Default)]
struct Guard {
    format: *mut ffi::AVFormatContext,
    codec: *mut ffi::AVCodecContext,
    scaler: *mut ffi::SwsContext,
    decoded: *mut ffi::AVFrame,
    packet: *mut ffi::AVPacket,
}

macro_rules! take {
    ($name:ident, $field:ident, $ty:ty) => {
        fn $name(&mut self) -> $ty {
            std::mem::replace(&mut self.$field, ptr::null_mut())
        }
    };
}

impl Guard {
    take!(take_format, format, *mut ffi::AVFormatContext);
    take!(take_codec, codec, *mut ffi::AVCodecContext);
    take!(take_scaler, scaler, *mut ffi::SwsContext);
    take!(take_decoded, decoded, *mut ffi::AVFrame);
    take!(take_packet, packet, *mut ffi::AVPacket);
}

impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: each free is a no-op on null, and a successful `open` has
        // already taken every pointer out, leaving all fields null.
        unsafe {
            if !self.packet.is_null() {
                ffi::av_packet_free(&mut self.packet);
            }
            if !self.decoded.is_null() {
                ffi::av_frame_free(&mut self.decoded);
            }
            if !self.scaler.is_null() {
                ffi::sws_freeContext(self.scaler);
            }
            if !self.codec.is_null() {
                ffi::avcodec_free_context(&mut self.codec);
            }
            if !self.format.is_null() {
                ffi::avformat_close_input(&mut self.format);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_against_the_expected_ffmpeg() {
        assert!(!linked_version().is_empty(), "av_version_info returned nothing");

        // The bindings are generated for avcodec 62 (FFmpeg 8.x). A different
        // major version means the struct layouts we compiled against do not
        // match the library that loaded - which corrupts silently rather than
        // failing, so it is worth an explicit check.
        assert_eq!(avcodec_major(), 62, "linked libavcodec mismatch");
    }

    #[test]
    fn describes_error_codes() {
        // Whatever the wording, it should not be the numeric fallback.
        assert!(!describe(ffi::AVERROR_EOF).starts_with("error "));
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        assert!(FfiDecoder::open("does-not-exist.mp4", 64, 64).is_err());
    }
}

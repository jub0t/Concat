// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The small things every linked-FFmpeg module needs: one-time
//! initialisation, error conversion, timestamp arithmetic and the display
//! rotation a stream carries.

use std::ffi::CStr;
use std::path::Path;
use std::sync::Once;

use concat_core::time::Rational;
use ffmpeg_the_third as ffmpeg;
use libc::{c_char, c_int, c_void};

use crate::error::Error;

/// Makes sure the libraries are initialised and quiet. Cheap after the
/// first call; every entry point calls it.
pub fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = ffmpeg::init();
        // Warnings and up are formatted; where they go is `relay`'s
        // business, and by default that is nowhere a person sees.
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Warning);
        // SAFETY: `relay` has the signature av_log_set_callback wants and
        // is `extern "C"`; it stays valid for the life of the process.
        unsafe { ffmpeg::sys::av_log_set_callback(Some(relay)) };
    });
}

/// The `va_list` a log callback is handed, spelt as the bindings spell it
/// in a parameter. On x86_64's System V ABI - Linux, and Intel Macs - C's
/// `va_list` is an array of one `__va_list_tag`, and an array in a
/// parameter position decays to a pointer, so there bindgen's `va_list`
/// alias (the array) and the callback's parameter (the pointer) are two
/// types, and `relay` must be declared with the second to be the callback
/// at all. Everywhere else the alias is the parameter's type too.
#[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
type VaList = *mut ffmpeg::sys::__va_list_tag;
#[cfg(not(all(target_arch = "x86_64", not(target_os = "windows"))))]
type VaList = ffmpeg::sys::va_list;

/// FFmpeg's log, through the `log` facade instead of standard error.
///
/// Left to itself FFmpeg prints on stderr, which a packaged GUI build has
/// wired to nowhere and a terminal run has wired to the person's screen:
/// a seek into long-GOP H.264 prints "mmco: unref short failure" once per
/// reference frame it cannot find, at FFmpeg's *error* level, twenty lines
/// for a scrub, about a picture that then decodes fine. Those are not
/// errors anyone acts on; the failures that matter come back through
/// `Result`s and are described precisely there. So a panic or a fatal
/// line is an error, and everything else is debug - off the console and
/// out of the file by default, and in the file the one time somebody
/// raises the level to read what FFmpeg saw.
unsafe extern "C" fn relay(
    context: *mut c_void,
    level: c_int,
    format: *const c_char,
    args: VaList,
) {
    // The level set in `init` is applied by FFmpeg's *default* callback,
    // not before the callback is called: a callback of our own is handed
    // every message there is, the per-packet debug and trace lines of a
    // decoder included, thousands a second under a scrub. They are
    // dropped here before anything is formatted, as the default drops
    // them - the first version of this relay formatted them all, and
    // scrubbing crawled.
    // SAFETY: reads a global integer.
    if level > unsafe { ffmpeg::sys::av_log_get_level() } {
        return;
    }
    let mut line = [0 as c_char; 1024];
    let mut print_prefix: c_int = 1;
    // SAFETY: the buffer is ours and its length is passed with it; the
    // context, format and arguments are what FFmpeg handed this callback
    // for exactly this call, and the formatter consumes the va_list once.
    let written = unsafe {
        ffmpeg::sys::av_log_format_line2(
            context,
            level,
            format,
            args,
            line.as_mut_ptr(),
            line.len() as c_int,
            &mut print_prefix,
        )
    };
    if written <= 0 {
        return;
    }
    line[line.len() - 1] = 0;
    // SAFETY: NUL-terminated by the formatter, and by the line above if the
    // message was longer than the buffer.
    let text = unsafe { CStr::from_ptr(line.as_ptr()) }.to_string_lossy();
    let text = text.trim_end();
    if text.is_empty() {
        return;
    }
    if level <= ffmpeg::sys::AV_LOG_FATAL {
        log::error!(target: "ffmpeg", "{text}");
    } else {
        log::debug!(target: "ffmpeg", "{text}");
    }
}

/// The version of FFmpeg this binary is linked against.
///
/// The first thing worth checking when something behaves oddly: it proves
/// which library actually loaded, which is not always the one you expect on a
/// machine with several FFmpeg installs.
pub fn linked_version() -> String {
    // SAFETY: av_version_info returns a pointer to a static NUL-terminated
    // string compiled into the library. Valid for the process lifetime.
    let raw = unsafe { ffmpeg::sys::av_version_info() };
    if raw.is_null() {
        return "unknown".to_owned();
    }
    // SAFETY: checked non-null; contract is a NUL-terminated C string.
    unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned()
}

/// Wraps a libav error with the operation and the file it happened to.
pub(crate) fn fail(operation: &'static str, path: &Path, error: ffmpeg::Error) -> Error {
    Error::Ffi {
        operation,
        path: path.to_path_buf(),
        detail: error.to_string(),
    }
}

/// `AVERROR(EAGAIN)`: the codec or filter wants more input before it can
/// produce output.
pub(crate) fn is_again(error: &ffmpeg::Error) -> bool {
    matches!(error, ffmpeg::Error::Other { errno } if *errno == libc::EAGAIN)
}

/// Converts a stream timestamp into seconds, exactly.
///
/// `time_base` is a rational, the timestamp is an integer count of those
/// ticks, and `concat-core` speaks rationals - so the value survives with no
/// rounding anywhere along the way. `None` when the container's numbers
/// cannot form a representable value - a degenerate time base, or a product
/// that overflows. Both come straight from an arbitrary file, so they must
/// degrade to "position unknown" rather than panic the process.
pub(crate) fn seconds(ticks: i64, time_base: ffmpeg::Rational) -> Option<Rational> {
    if time_base.denominator() == 0 {
        return None;
    }
    Rational::checked_new(
        i128::from(ticks) * i128::from(time_base.numerator()),
        i128::from(time_base.denominator()),
    )
}

/// Where a stream's timestamps start, in seconds: what its frames are
/// measured from. Zero for a stream that starts at zero or does not say;
/// 1.4 s for MPEG-TS by default and the MTS files cameras write, which
/// read as 1.4 s into the picture unless this is taken off every
/// timestamp and put back on every seek (audit 2026-09-23, #9).
pub(crate) fn start_of(stream: &ffmpeg::format::stream::Stream<'_>) -> Rational {
    let ticks = stream.start_time();
    if ticks == ffmpeg::sys::AV_NOPTS_VALUE || ticks <= 0 {
        return Rational::from_int(0);
    }
    seconds(ticks, stream.time_base()).unwrap_or_else(|| Rational::from_int(0))
}

/// Seconds in `AV_TIME_BASE` units, which is what container-level seeks take.
pub(crate) fn av_ticks(seconds: Rational) -> i64 {
    let scaled = seconds * Rational::from_int(i64::from(ffmpeg::sys::AV_TIME_BASE));
    scaled.floor()
}

/// The rotation a stream asks players to apply before showing its frames,
/// in whole degrees clockwise, normalised to `0..360`.
///
/// Phones record portrait video sideways and store a display matrix saying
/// so; older files carry a `rotate` metadata tag instead. Both are read here
/// so the decoder can turn the picture the way every player does, and the
/// probe can report the dimensions as displayed.
pub(crate) fn rotation(stream: &ffmpeg::format::stream::Stream<'_>) -> i64 {
    // The display matrix moved from the stream to its codec parameters in
    // FFmpeg 7.0, which is the oldest this crate builds against; the wrapper
    // has no accessor for that field yet, so it is read directly.
    // SAFETY: the stream pointer is valid for the stream's lifetime, and
    // `coded_side_data` is an array of `nb_coded_side_data` entries owned by
    // the codec parameters; a display matrix entry holds nine `i32`s.
    let from_matrix = unsafe {
        let parameters = (*stream.as_ptr()).codecpar;
        let count = (*parameters).nb_coded_side_data;
        let entries = (*parameters).coded_side_data;
        let mut found = None;
        for index in 0..count {
            let entry = entries.offset(index as isize);
            if (*entry).type_ == ffmpeg::sys::AVPacketSideDataType::DISPLAYMATRIX
                && (*entry).size >= 9 * 4
            {
                // Players rotate by the negative of what the matrix encodes;
                // `get_rotation` in ffmpeg's own tools does exactly this.
                let angle = -ffmpeg::sys::av_display_rotation_get((*entry).data as *const i32);
                found = Some(angle);
                break;
            }
        }
        found
    };
    let degrees = from_matrix.or_else(|| {
        stream
            .metadata()
            .get("rotate")
            .and_then(|value| value.parse::<f64>().ok())
    });
    degrees.map_or(0, |angle| (angle.round() as i64).rem_euclid(360))
}

/// The filters that turn a decoded frame the way its rotation asks - the
/// same three shapes ffmpeg's autorotate inserts - or nothing at all.
pub(crate) fn rotation_filters(degrees: i64) -> Option<&'static str> {
    match degrees {
        90 => Some("transpose=clock"),
        180 => Some("hflip,vflip"),
        270 => Some("transpose=cclock"),
        _ => None,
    }
}

/// The displayed size of a picture coded at `width` by `height` with the
/// given rotation: a quarter turn swaps the two.
pub(crate) fn displayed(width: u32, height: u32, degrees: i64) -> (u32, u32) {
    if degrees.rem_euclid(180) == 90 {
        (height, width)
    } else {
        (width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_against_something() {
        init();
        assert!(!linked_version().is_empty());
    }

    #[test]
    fn a_quarter_turn_swaps_the_dimensions_and_a_half_turn_does_not() {
        assert_eq!(displayed(1920, 1080, 90), (1080, 1920));
        assert_eq!(displayed(1920, 1080, 270), (1080, 1920));
        assert_eq!(displayed(1920, 1080, 180), (1920, 1080));
        assert_eq!(displayed(1920, 1080, 0), (1920, 1080));
    }

    #[test]
    fn rotation_filters_match_autorotate() {
        assert_eq!(rotation_filters(0), None);
        assert_eq!(rotation_filters(90), Some("transpose=clock"));
        assert_eq!(rotation_filters(180), Some("hflip,vflip"));
        assert_eq!(rotation_filters(270), Some("transpose=cclock"));
    }

    #[test]
    fn timestamps_convert_exactly() {
        let tb = ffmpeg::Rational::new(1, 30000);
        assert_eq!(seconds(1001, tb), Some(Rational::new(1001, 30000)));
        assert_eq!(seconds(5, ffmpeg::Rational::new(1, 0)), None);
        assert_eq!(av_ticks(Rational::new(3, 2)), 1_500_000);
    }
}

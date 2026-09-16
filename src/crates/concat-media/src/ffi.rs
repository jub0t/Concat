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

use crate::error::Error;

/// Makes sure the libraries are initialised and quiet. Cheap after the
/// first call; every entry point calls it.
pub fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = ffmpeg::init();
        // Errors only: a GUI has no terminal, and FFmpeg's warnings are
        // noise about files we are about to describe precisely in our own
        // error messages.
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Error);
    });
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

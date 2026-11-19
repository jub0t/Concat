// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A clip at another speed decodes under a number format that writes
//! decimals with a comma (#762).
//!
//! The locale is the process's, so this is a file of its own: its own test
//! binary, run on its own, with no other test to race. Only one test here,
//! for the same reason.

use std::ffi::{CStr, CString};

use concat_media::samples::{AudioDecoder, AudioOptions, SampleFormat};

#[cfg(windows)]
#[repr(C)]
struct Lconv {
    decimal_point: *mut std::ffi::c_char,
}

#[cfg(windows)]
unsafe extern "C" {
    fn localeconv() -> *mut Lconv;
}

#[cfg(unix)]
use libc::localeconv;

/// The C runtime's decimal point now.
fn decimal_point() -> String {
    // SAFETY: the runtime's own struct, read at once.
    unsafe {
        let conv = localeconv();
        CStr::from_ptr((*conv).decimal_point)
            .to_string_lossy()
            .into_owned()
    }
}

/// Sets `LC_NUMERIC` to `name`, returning whether the runtime took it.
fn set_numeric(name: &str) -> bool {
    let name = CString::new(name).expect("no NUL");
    // SAFETY: a NUL-terminated name for one category.
    !unsafe { libc::setlocale(libc::LC_NUMERIC, name.as_ptr()) }.is_null()
}

/// Puts `LC_NUMERIC` back as it was when the test began, however it ends.
struct Restore(CString);

impl Drop for Restore {
    fn drop(&mut self) {
        // SAFETY: the name the runtime gave, copied.
        unsafe { libc::setlocale(libc::LC_NUMERIC, self.0.as_ptr()) };
    }
}

#[test]
fn a_clip_at_another_speed_decodes_under_a_comma_locale() {
    // SAFETY: a query; the name is copied before any other locale call.
    let before =
        unsafe { CStr::from_ptr(libc::setlocale(libc::LC_NUMERIC, std::ptr::null())) }.to_owned();
    let _restore = Restore(before);

    // The names a comma locale goes by: glibc's and macOS's, then the
    // UCRT's.
    let candidates = [
        "de_DE.UTF-8",
        "de_DE.utf8",
        "fr_FR.UTF-8",
        "fr_FR.utf8",
        "de_DE",
        "de-DE",
        "German_Germany.1252",
    ];
    let Some(used) = candidates
        .iter()
        .find(|name| set_numeric(name) && decimal_point() == ",")
    else {
        println!(
            "SKIPPED: none of {candidates:?} could be set as a number format with a decimal comma \
             on this machine, so the guard was not exercised"
        );
        return;
    };
    println!("running under LC_NUMERIC={used}");

    let mkv = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/keyframes-3s-apart.mkv"
    );
    let options = AudioOptions {
        start: Some(1.0),
        duration: Some(1.5),
        filters: concat_media::audio::speed_filters(1.5, true),
        rate: 48_000,
        channels: 2,
        format: SampleFormat::I16,
        stream: None,
        stretch: None,
    };
    let mut decoder = AudioDecoder::open(mkv, &options).expect("opens");
    let mut frames = 0;
    while let Some(frame) = decoder.next_frame().expect("decodes at 1.5x") {
        frames += frame.samples();
    }
    assert!(
        (frames as i64 - 48_000).abs() < 2_400,
        "a second at 1.5x, got {frames} frames"
    );
    assert_eq!(decimal_point(), ".", "the number format was set back");
}

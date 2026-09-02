// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! End-to-end checks for the linked decoder.
//!
//! These are the claims that justify the FFI backend existing at all - real
//! presentation timestamps and frame-accurate seeking - so they are tested
//! against an actual file rather than asserted in a doc comment.
//!
//! The fixture is generated with the `ffmpeg` binary at test time. If FFmpeg
//! is not installed the tests skip rather than fail: a missing tool is a
//! missing tool, not a broken decoder.

#![cfg(feature = "ffi")]

use std::path::PathBuf;
use std::process::Command;

use wolfcut_core::time::Rational;
use wolfcut_media::decode::{FrameSource, SeekableSource};
use wolfcut_media::ffi::FfiDecoder;

const RATE: i64 = 30;
const SECONDS: i64 = 4;

/// Renders a deterministic clip and returns its path, or `None` without FFmpeg.
fn fixture() -> Option<PathBuf> {
    let path = std::env::temp_dir().join("wolfcut-ffi-fixture.mp4");
    if path.exists() {
        return Some(path);
    }

    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", &format!("testsrc2=size=320x240:rate={RATE}")])
        .args(["-t", &SECONDS.to_string()])
        // A short keyframe interval, so a seek has something nearby to land on
        // and the test measures our decode-forward rather than FFmpeg's luck.
        .args(["-g", "15", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&path)
        .status()
        .ok()?;

    status.success().then_some(path)
}

/// One frame's duration, as the tolerance for "landed on the right frame".
fn frame_duration() -> Rational {
    Rational::new(1, RATE)
}

#[test]
fn reports_real_presentation_timestamps() {
    let Some(path) = fixture() else { return };

    let mut decoder = FfiDecoder::open(&path, 160, 120).expect("open");
    let mut times = Vec::new();

    for _ in 0..10 {
        let frame = decoder.next_frame().expect("decode").expect("a frame");
        assert_eq!((frame.width(), frame.height()), (160, 120), "scaled to the requested size");

        let position = decoder.position().expect("a linked decoder always knows the time");
        times.push(position);
    }

    // The whole point: these come from the container, not from counting.
    assert_eq!(times[0], Rational::ZERO, "first frame starts at zero");

    for (index, time) in times.iter().enumerate() {
        let expected = frame_duration() * Rational::from_int(index as i64);
        assert_eq!(*time, expected, "frame {index} should sit exactly on its boundary");
    }

    assert!(times.windows(2).all(|pair| pair[0] < pair[1]), "timestamps must increase");
}

#[test]
fn seeks_to_an_exact_frame() {
    let Some(path) = fixture() else { return };

    let mut decoder = FfiDecoder::open(&path, 160, 120).expect("open");

    // Deliberately not a keyframe: at -g 15 the keyframes are every half
    // second, so 2.4s sits between two of them. Landing there is only possible
    // by decoding forward from the previous one, which is the thing a pipe
    // cannot do.
    let target = Rational::new(24, 10);
    decoder.seek(target).expect("seek");

    let mut landed = None;
    for _ in 0..64 {
        decoder.next_frame().expect("decode").expect("a frame");
        let position = decoder.position().expect("position");
        if position >= target {
            landed = Some(position);
            break;
        }
    }

    let landed = landed.expect("never reached the target after a seek");
    let drift = landed - target;
    assert!(
        drift >= Rational::ZERO && drift < frame_duration(),
        "landed at {landed} for a target of {target}, which is more than one frame out",
    );
}

#[test]
fn seeking_backwards_works_too() {
    let Some(path) = fixture() else { return };

    let mut decoder = FfiDecoder::open(&path, 64, 64).expect("open");

    // Run well into the file first, so the seek genuinely goes backwards and
    // has stale decoder state to flush.
    for _ in 0..60 {
        decoder.next_frame().expect("decode");
    }
    let before = decoder.position().expect("position");
    assert!(before > Rational::ONE, "should be past a second by now");

    decoder.seek(Rational::new(1, 2)).expect("seek back");
    decoder.next_frame().expect("decode").expect("a frame");

    let after = decoder.position().expect("position");
    assert!(after < before, "seeking back should move the position back");
    assert!(after <= Rational::new(6, 10), "should land near the half-second mark, got {after}");
}

#[test]
fn runs_to_the_end_and_stops() {
    let Some(path) = fixture() else { return };

    let mut decoder = FfiDecoder::open(&path, 64, 64).expect("open");
    let mut count = 0u32;
    while decoder.next_frame().expect("decode").is_some() {
        count += 1;
        assert!(count < 1000, "decoder never reported end of stream");
    }

    // Flushing at EOF is what makes the last few frames come out at all; a
    // decoder that forgets it silently loses them.
    let expected = (RATE * SECONDS) as u32;
    assert!(
        count.abs_diff(expected) <= 1,
        "decoded {count} frames, expected about {expected}",
    );

    assert!(decoder.next_frame().expect("decode").is_none(), "end of stream is sticky");
}

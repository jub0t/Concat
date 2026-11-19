// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A pip at the start of every source second, stretched, must be heard
//! within a millisecond of where the time map puts that second, at its own
//! pitch: the measure that found FFmpeg's atempo 53 ms early at half speed.

use super::*;

const RATE: u32 = 48_000;
const PIP: f64 = 0.02;
const PITCH: f64 = 1000.0;

/// Mono, `seconds` long, a 20 ms sine pip at every whole second.
fn pips(seconds: f64) -> Vec<f32> {
    let length = (seconds * f64::from(RATE)) as usize;
    let pip = (PIP * f64::from(RATE)) as usize;
    let mut samples = vec![0.0f32; length];
    let step = 2.0 * std::f64::consts::PI * PITCH / f64::from(RATE);
    let mut start = 0;
    while start + pip <= length {
        for offset in 0..pip {
            samples[start + offset] = 0.5 * (step * offset as f64).sin() as f32;
        }
        start += RATE as usize;
    }
    samples
}

/// The onsets, in seconds, of every burst in `samples`: its first sample
/// at three tenths of its own peak.
fn onsets(samples: &[f32]) -> Vec<f64> {
    let gap = (0.1 * f64::from(RATE)) as usize;
    let mut found = Vec::new();
    let mut at = 0;
    while at < samples.len() {
        if samples[at].abs() < 0.02 {
            at += 1;
            continue;
        }
        let start = at;
        let (mut end, mut quiet) = (at, 0);
        while at < samples.len() && quiet < gap {
            if samples[at].abs() >= 0.02 {
                end = at;
                quiet = 0;
            } else {
                quiet += 1;
            }
            at += 1;
        }
        let burst = &samples[start..=end];
        let peak = burst.iter().fold(0.0f32, |most, s| most.max(s.abs()));
        let onset = start
            + burst
                .iter()
                .position(|s| s.abs() >= 0.3 * peak)
                .unwrap_or(0);
        found.push(onset as f64 / f64::from(RATE));
    }
    found
}

/// A source over `samples`, handed out 1000 frames at a time as a decoder
/// would.
fn source(samples: Vec<f32>) -> impl Source {
    let mut at = 0;
    move |into: &mut Vec<f32>| {
        if at >= samples.len() {
            return false;
        }
        let end = (at + 1000).min(samples.len());
        into.extend_from_slice(&samples[at..end]);
        at = end;
        true
    }
}

/// Stretches `input` through `map` for `seconds` of output.
fn stretch(input: Vec<f32>, map: impl TimeMap, seconds: f64, limit: Option<u64>) -> Vec<f32> {
    let mut stretch = Stretch::new(RATE, 1, map, source(input), limit);
    let mut out = vec![0.0; (seconds * f64::from(RATE)) as usize];
    stretch.render(&mut out);
    out
}

/// The output time, in seconds, at which `map` reaches source second
/// `second`.
fn when(map: &impl TimeMap, second: f64) -> f64 {
    let target = second * f64::from(RATE);
    let (mut low, mut high) = (0.0, 1e9);
    for _ in 0..200 {
        let middle = (low + high) / 2.0;
        if map.source_at(middle) < target {
            low = middle;
        } else {
            high = middle;
        }
    }
    high / f64::from(RATE)
}

/// Every pip of a `source_seconds` input stretched through `map` is heard
/// within `within` milliseconds of its place, and nothing else is heard.
fn assert_placed(map: impl TimeMap + Copy, source_seconds: f64, within: f64, what: &str) {
    let expected: Vec<f64> = (0..)
        .map(f64::from)
        .take_while(|second| second + PIP <= source_seconds)
        .map(|second| when(&map, second))
        .collect();
    let length = when(&map, source_seconds) + 0.5;
    let heard = onsets(&stretch(pips(source_seconds), map, length, None));
    assert_eq!(
        heard.len(),
        expected.len(),
        "{what}: heard {heard:?}, expected {expected:?}"
    );
    for (heard, expected) in heard.iter().zip(&expected) {
        let off = (heard - expected) * 1000.0;
        assert!(
            off.abs() <= within,
            "{what}: a pip due at {expected:.4} s is heard {off:+.2} ms off; {within} ms is allowed"
        );
    }
}

#[test]
fn every_pip_lands_on_its_second_at_every_constant_speed() {
    for speed in [0.25, 0.5, 0.75, 1.0, 1.5, 2.0] {
        assert_placed(Constant(speed), 8.0, 1.0, &format!("{speed}x"));
    }
    // At 4x a 20 ms pip lasts 5 ms, less than one 5.3 ms grain: measured
    // up to 2.4 ms early, a twentieth of what is heard as out of sync.
    assert_placed(Constant(4.0), 8.0, 3.0, "4x");
}

/// Steps of speed, as a clip's `tempo` has them: `(output seconds, speed)`.
#[derive(Clone, Copy)]
struct Steps(&'static [(f64, f64)]);

impl TimeMap for Steps {
    fn source_at(&self, output: f64) -> f64 {
        let at = output / f64::from(RATE);
        let mut total = 0.0;
        for (index, &(from, speed)) in self.0.iter().enumerate() {
            if at <= from {
                break;
            }
            let until = self.0.get(index + 1).map_or(f64::INFINITY, |next| next.0);
            total += (at.min(until) - from) * speed;
        }
        total * f64::from(RATE)
    }
}

#[test]
fn a_jump_to_four_times_and_back_keeps_every_pip_in_place() {
    // Normal speed, 4x for a second, normal again: the map that put
    // Rubber Band 154 ms off and Signalsmith 197 ms off.
    assert_placed(
        Steps(&[(0.0, 1.0), (4.0, 4.0), (5.0, 1.0)]),
        12.0,
        3.0,
        "1x, 4x, 1x",
    );
    assert_placed(
        Steps(&[(0.0, 1.0), (2.0, 0.5), (6.0, 2.0)]),
        10.0,
        1.0,
        "1x, 0.5x, 2x",
    );
}

#[test]
fn the_pitch_is_kept_at_every_speed() {
    for speed in [0.25, 0.5, 2.0, 4.0] {
        let out = stretch(pips(4.0), Constant(speed), 4.0 / speed, None);
        // The middle of the first pip, where it is steady, by zero
        // crossings.
        let length = (PIP / speed * f64::from(RATE)) as usize;
        let middle = &out[length / 4..length * 3 / 4];
        let crossings = middle
            .windows(2)
            .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
            .count();
        let pitch = crossings as f64 / 2.0 / (middle.len() as f64 / f64::from(RATE));
        assert!(
            (pitch - PITCH).abs() / PITCH < 0.03,
            "{speed}x: the pip sounds at {pitch:.0} Hz, not {PITCH} Hz"
        );
    }
}

#[test]
fn nothing_past_the_out_point_is_heard() {
    // Five seconds of pips, the clip ending at 2.5 s of source: the pips
    // at 3 and 4 s must not be heard, nor any trace of them.
    let out = stretch(
        pips(5.0),
        Constant(0.5),
        10.0,
        Some(u64::from(RATE) * 5 / 2),
    );
    let heard = onsets(&out);
    assert_eq!(heard.len(), 3, "{heard:?}");
    let after = &out[(5.2 * f64::from(RATE)) as usize..];
    let loudest = after.iter().fold(0.0f32, |most, s| most.max(s.abs()));
    assert!(loudest < 1e-3, "{loudest} heard past the out-point");
}

#[test]
fn the_output_is_exactly_as_long_as_asked_and_silent_past_the_source() {
    let mut stretch = Stretch::new(RATE, 2, Constant(2.0), source(vec![0.1; 2 * 4_800]), None);
    let mut out = vec![1.0; 2 * 48_000];
    stretch.render(&mut out);
    assert!(
        out[2 * 12_000..].iter().all(|s| s.abs() < 1e-4),
        "silence after the source"
    );
}

/// How fast it is, in seconds of stretched stereo per second of work, at
/// half speed and at 2x. `cargo test -p concat-stretch --release -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn speed_of_the_stretch() {
    let seconds = 60.0;
    let input: Vec<f32> = pips(seconds).into_iter().flat_map(|s| [s, s]).collect();
    for speed in [0.5, 2.0] {
        let started = std::time::Instant::now();
        let mut stretch = Stretch::new(RATE, 2, Constant(speed), source(input.clone()), None);
        let frames = (seconds / speed * f64::from(RATE)) as usize;
        let mut out = vec![0.0; 2 * 4096];
        let mut done = 0;
        while done < frames {
            stretch.render(&mut out);
            done += 4096;
        }
        let took = started.elapsed().as_secs_f64();
        println!(
            "{speed}x: {:.1} s of stereo in {took:.3} s, {:.0}x real time",
            seconds / speed,
            seconds / speed / took
        );
    }
}

/// The share of a stretched held chord's energy that is off its
/// partials (more than 3 Hz from one), over one second from the middle of
/// `samples`: how much the stretch smears a steady tone into warble. The
/// energy on the partials by a DFT at 1 Hz bins around each, the whole by
/// Parseval.
fn leakage(samples: &[f32], partials: &[f64]) -> f64 {
    let n = RATE as usize;
    let start = (samples.len() - n) / 2;
    let window: Vec<f64> = (0..n)
        .map(|i| {
            let hann = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos();
            f64::from(samples[start + i]) * hann
        })
        .collect();
    // Positive frequencies hold half of Parseval's N * sum(x^2).
    let total = window.iter().map(|x| x * x).sum::<f64>() * n as f64 / 2.0;
    let mut inside = 0.0;
    for partial in partials {
        for bin in (*partial as i64 - 3)..=(*partial as i64 + 3) {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, sample) in window.iter().enumerate() {
                let phase =
                    2.0 * std::f64::consts::PI * (bin * i as i64 % n as i64) as f64 / n as f64;
                re += sample * phase.cos();
                im -= sample * phase.sin();
            }
            inside += re * re + im * im;
        }
    }
    1.0 - inside / total
}

#[test]
fn a_held_chord_stays_clean_at_every_speed() {
    // A bass at 55 Hz under three notes: measured -39 to -50 dB off the
    // partials at 0.25x to 4x with half-size grains, against -20 dB with
    // quarter-size ones, which warble.
    let partials = [55.0, 220.0, 330.0, 1760.0];
    let chord: Vec<f32> = (0..RATE as usize * 8)
        .map(|i| {
            let t = i as f64 / f64::from(RATE);
            let sum: f64 = partials
                .iter()
                .map(|f| (2.0 * std::f64::consts::PI * f * t).sin())
                .sum();
            (sum * 0.2) as f32
        })
        .collect();
    for speed in [0.25, 0.5, 2.0, 4.0] {
        let out = stretch(chord.clone(), Constant(speed), 8.0 / speed * 0.9, None);
        let off = leakage(&out, &partials);
        assert!(
            off < 3e-4,
            "{speed}x: {off:.2e} of the chord's energy is off its partials"
        );
    }
}

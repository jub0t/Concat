// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Sound that says where it is: a short pip at the start of every source
//! second, at a pitch that names the media it belongs to.
//!
//! The picture's codes say which source frame a frame shows; the pips say
//! the same of the sound. A pip's onset in the export is where that source
//! second landed on the timeline, so a sound that runs early or late, is
//! cut a frame off, plays the wrong clip or goes silent shows as a pip
//! missing, extra, mistimed or at the wrong pitch. The export's sound is
//! AAC, as a person's is, so the pips are long and far apart enough for a
//! lossy codec to keep their onsets, and pitches are an octave's half
//! apart so no codec can mistake one media's for another's.

use std::path::Path;

/// Samples a second of every pip file, and of the sound read back.
pub const RATE: u32 = 48_000;

/// How long a pip sounds.
pub const PIP_SECONDS: f64 = 0.02;

/// How loud a pip is, of full scale.
const LEVEL: f32 = 0.5;

/// A sample this loud or louder, of full scale, is sound rather than the
/// codec's noise floor.
const AUDIBLE: f32 = 0.02;

/// Quiet this long ends one pip and lets the next begin.
const GAP_SECONDS: f64 = 0.1;

/// A pip's onset is its first sample at this share of its own peak: past
/// a lossy codec's pre-echo, and a tenth of a millisecond into the pip at
/// the lowest pitch.
const ONSET_SHARE: f32 = 0.3;

/// The pitch, in hertz, of media `tag`'s pips, 1 to 7.
pub fn pitch(tag: u8) -> f64 {
    const PITCHES: [f64; 7] = [500.0, 707.0, 1000.0, 1414.0, 2000.0, 2828.0, 4000.0];
    PITCHES[usize::from(tag.clamp(1, 7) - 1)]
}

/// Mono samples at [`RATE`], `seconds` long, with media `tag`'s pip at the
/// start of every whole second in it that has room for the whole pip.
pub fn pips(tag: u8, seconds: f64) -> Vec<f32> {
    let length = (seconds * f64::from(RATE)).round() as usize;
    let mut samples = vec![0.0f32; length];
    let pip = (PIP_SECONDS * f64::from(RATE)).round() as usize;
    let step = 2.0 * std::f64::consts::PI * pitch(tag) / f64::from(RATE);
    let mut second = 0usize;
    loop {
        let start = second * RATE as usize;
        if start + pip > length {
            break;
        }
        for (offset, sample) in samples[start..start + pip].iter_mut().enumerate() {
            *sample = LEVEL * (step * offset as f64).sin() as f32;
        }
        second += 1;
    }
    samples
}

/// Mono samples at [`RATE`], `seconds` long: media `tag`'s pitch, steady,
/// at the level a pip is.
pub fn tone(tag: u8, seconds: f64) -> Vec<f32> {
    let length = (seconds * f64::from(RATE)).round() as usize;
    let step = 2.0 * std::f64::consts::PI * pitch(tag) / f64::from(RATE);
    (0..length)
        .map(|n| LEVEL * (step * n as f64).sin() as f32)
        .collect()
}

/// How loud media `tag`'s tone is in `samples` around `at` seconds, of the
/// level [`tone`] writes it at: its amplitude over 40 ms, weighted to the
/// middle, so another media's tone sounding with it does not count and a
/// fade is read where it is asked for, not averaged along its slope.
pub fn level(samples: &[f32], rate: u32, at: f64, tag: u8) -> f64 {
    let half = (0.02 * f64::from(rate)) as i64;
    let centre = (at * f64::from(rate)).round() as i64;
    let turn = 2.0 * std::f64::consts::PI * pitch(tag) / f64::from(rate);
    let (mut real, mut imaginary, mut weight) = (0.0f64, 0.0f64, 0.0f64);
    for offset in -half..=half {
        let window = 0.5 + 0.5 * (std::f64::consts::PI * offset as f64 / half as f64).cos();
        let sample = usize::try_from(centre + offset)
            .ok()
            .and_then(|index| samples.get(index))
            .map_or(0.0, |sample| f64::from(*sample));
        let angle = turn * (centre + offset) as f64;
        real += window * sample * angle.cos();
        imaginary += window * sample * angle.sin();
        weight += window;
    }
    2.0 * real.hypot(imaginary) / weight / f64::from(LEVEL)
}

/// Writes `samples`, mono at [`RATE`], as 16-bit PCM WAV.
pub fn write_wav(path: &Path, samples: &[f32]) -> Result<(), String> {
    let data = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(path, bytes)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

/// One pip heard in a stretch of sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Heard {
    /// Its onset, in seconds from the start of the sound.
    pub at: f64,
    /// The media whose pitch it is nearest, 1 to 7.
    pub tag: u8,
    /// How loud it peaks, of full scale.
    pub peak: f32,
}

/// Every pip in `samples`, mono at `rate`, in order.
pub fn hear(samples: &[f32], rate: u32) -> Vec<Heard> {
    let gap = (GAP_SECONDS * f64::from(rate)) as usize;
    let mut heard = Vec::new();
    let mut at = 0;
    while at < samples.len() {
        if samples[at].abs() < AUDIBLE {
            at += 1;
            continue;
        }
        // One burst: from here until a gap of quiet.
        let start = at;
        let mut end = at;
        let mut quiet = 0;
        while at < samples.len() && quiet < gap {
            if samples[at].abs() >= AUDIBLE {
                end = at;
                quiet = 0;
            } else {
                quiet += 1;
            }
            at += 1;
        }
        let burst = &samples[start..=end];
        let peak = burst
            .iter()
            .fold(0.0f32, |most, sample| most.max(sample.abs()));
        let onset = start
            + burst
                .iter()
                .position(|sample| sample.abs() >= ONSET_SHARE * peak)
                .unwrap_or(0);
        let body_end = (onset + (PIP_SECONDS * f64::from(rate)) as usize).min(end + 1);
        heard.push(Heard {
            at: onset as f64 / f64::from(rate),
            tag: nearest_tag(&samples[onset..body_end], rate),
            peak,
        });
    }
    heard
}

/// The media whose pitch carries the most of `body`'s energy.
fn nearest_tag(body: &[f32], rate: u32) -> u8 {
    (1..=7)
        .map(|tag| (tag, goertzel(body, rate, pitch(tag))))
        .fold(
            (1, f64::MIN),
            |best, next| if next.1 > best.1 { next } else { best },
        )
        .0
}

/// The energy of `samples` at `frequency`.
fn goertzel(samples: &[f32], rate: u32, frequency: f64) -> f64 {
    let coefficient = 2.0 * (2.0 * std::f64::consts::PI * frequency / f64::from(rate)).cos();
    let (mut previous, mut before) = (0.0f64, 0.0f64);
    for &sample in samples {
        let next = f64::from(sample) + coefficient * previous - before;
        before = previous;
        previous = next;
    }
    previous * previous + before * before - coefficient * previous * before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pip_is_heard_on_its_second_at_its_own_pitch() {
        for tag in 1..=7 {
            let heard = hear(&pips(tag, 3.5), RATE);
            let at: Vec<f64> = heard.iter().map(|pip| pip.at).collect();
            assert_eq!(heard.len(), 4, "tag {tag}: {heard:?}");
            for (second, time) in at.iter().enumerate() {
                assert!(
                    (time - second as f64).abs() < 0.0002,
                    "tag {tag}: pip {second} heard at {time}"
                );
            }
            assert!(
                heard.iter().all(|pip| pip.tag == tag),
                "tag {tag}: {heard:?}"
            );
        }
    }

    #[test]
    fn a_tones_level_is_read_where_it_is_asked_for_and_another_tone_does_not_count() {
        let mut both = tone(1, 2.0);
        // Tag 1 fades in over the first second, as the square of the time;
        // tag 3 sounds over it at a fifth.
        for (index, (sample, other)) in both.iter_mut().zip(tone(3, 2.0)).enumerate() {
            let t = (index as f32 / RATE as f32).min(1.0);
            *sample = *sample * t * t + 0.2 * other;
        }
        for (at, gain) in [(0.25, 0.0625), (0.5, 0.25), (0.75, 0.5625), (1.5, 1.0)] {
            let read = level(&both, RATE, at, 1);
            assert!((read - gain).abs() < 0.005, "at {at}: {read}, not {gain}");
            let other = level(&both, RATE, at, 3);
            assert!((other - 0.2).abs() < 0.005, "tag 3 at {at}: {other}");
        }
        assert!(level(&both, RATE, 1.0, 5) < 0.002);
    }

    #[test]
    fn a_pip_with_no_room_for_all_of_it_is_not_written() {
        assert_eq!(hear(&pips(1, 1.01), RATE).len(), 1);
        assert_eq!(hear(&pips(1, 1.02), RATE).len(), 2);
    }

    #[test]
    fn silence_and_the_noise_floor_hold_no_pip() {
        assert!(hear(&vec![0.0; RATE as usize], RATE).is_empty());
        let hiss: Vec<f32> = (0..RATE)
            .map(|n| if n % 2 == 0 { 0.01 } else { -0.01 })
            .collect();
        assert!(hear(&hiss, RATE).is_empty());
    }
}

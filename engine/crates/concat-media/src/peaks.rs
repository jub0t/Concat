// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Waveform peaks: min/max sample pairs, bucketed at a fixed rate.
//!
//! The timeline draws a clip's waveform from a few hundred buckets per
//! second, not from the samples themselves. This module produces those
//! buckets by streaming FFmpeg's decode of the file's audio - the samples
//! are folded into buckets as they arrive and never accumulate, so an
//! hour-long recording costs the same memory as a jingle.
//!
//! This used to live in the UI, which read the entire file over IPC and
//! decoded it with WebAudio on the main thread. Moving it here removed the
//! app's last whole-file read across the IPC boundary.

use std::io::Read;
use std::path::Path;
use std::process::Stdio;

use crate::binaries::ffmpeg;
use crate::error::{Error, Result};
use crate::process::{StderrTail, base_command};

/// The rate audio is resampled to before bucketing.
///
/// A fixed rate makes the bucket size exact - at 200 buckets per second a
/// 48 kHz stream folds precisely 240 samples per bucket - so the encoded
/// `buckets_per_second` is the requested number, not a near miss that
/// depends on the source file's native rate.
const PEAK_RATE: u32 = 48_000;

/// A file's waveform, reduced to per-bucket extremes.
///
/// Buckets are seeded at zero rather than ±infinity: silence reads as a
/// flat 0/0 pair, and a bucket's minimum can never sit above the axis.
/// That is the shape the timeline has always drawn, and the on-disk caches
/// already hold it.
pub struct Peaks {
    /// The lowest sample in each bucket, in [-1, 0].
    pub min: Vec<f32>,
    /// The highest sample in each bucket, in [0, 1].
    pub max: Vec<f32>,
    /// How many buckets cover one second of audio.
    pub buckets_per_second: f32,
}

impl Peaks {
    /// The wire and cache format the UI reads:
    /// `[buckets_per_second f32][count u32][min f32 x count][max f32 x count]`,
    /// little-endian. Kept byte-identical to what the UI used to write, so
    /// existing project caches stay valid.
    pub fn encode(&self) -> Vec<u8> {
        let count = self.min.len().min(self.max.len());
        let mut bytes = Vec::with_capacity(8 + count * 8);
        bytes.extend_from_slice(&self.buckets_per_second.to_le_bytes());
        bytes.extend_from_slice(&(count as u32).to_le_bytes());
        for value in self.min.iter().take(count) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in self.max.iter().take(count) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

/// Decodes one file's audio and reduces it to peaks.
///
/// The decode is mono 16-bit at [`PEAK_RATE`], piped back as WAV - `wav`
/// rather than raw `s16le` because the bundled FFmpeg is a trimmed build
/// and the raw muxer is not in it. A file with no audio stream fails the
/// FFmpeg run, and the error carries what FFmpeg said.
pub fn extract(path: &Path, buckets_per_second: u32) -> Result<Peaks> {
    let mut child = base_command(ffmpeg())
        .arg("-i")
        .arg(path)
        .args(["-vn", "-ac", "1"])
        .args(["-ar", &PEAK_RATE.to_string()])
        .args(["-c:a", "pcm_s16le", "-f", "wav", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| Error::Spawn { program: "ffmpeg", source })?;

    let mut tail = StderrTail::drain(&mut child);
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let parsed = peaks_from_wav(&mut stdout, path, buckets_per_second);

    if parsed.is_err() {
        // A parse failure leaves FFmpeg still writing into a pipe nobody
        // reads; waiting on it now would deadlock on the full pipe.
        let _ = child.kill();
    }
    let status = child.wait().map_err(|source| Error::Io { program: "ffmpeg", source })?;

    // FFmpeg's own account of the failure - "no audio stream", "no such
    // file" - beats whatever the parser tripped over downstream of it.
    if !status.success() {
        return Err(Error::Exited {
            program: "ffmpeg",
            path: path.to_path_buf(),
            status,
            stderr: tail.summary(),
        });
    }
    parsed
}

/// Folds a WAV stream into peaks without holding the samples.
///
/// A plain chunk walk, not a format library: the stream comes from our own
/// FFmpeg invocation above, so the only variability is which bookkeeping
/// chunks precede `data`. Sizes written to a pipe are `u32::MAX`
/// placeholders - the muxer could not seek back to patch them - in which
/// case the payload is simply the rest of the stream.
fn peaks_from_wav(
    reader: &mut impl Read,
    path: &Path,
    buckets_per_second: u32,
) -> Result<Peaks> {
    let malformed = |detail: &str| Error::Probe {
        path: path.to_path_buf(),
        detail: format!("ffmpeg piped back something other than the wav asked for: {detail}"),
    };
    let io = |source| Error::Io { program: "ffmpeg", source };

    let buckets_per_second = buckets_per_second.clamp(1, PEAK_RATE);
    let bucket_size = (PEAK_RATE / buckets_per_second) as usize;

    let mut riff = [0u8; 12];
    reader.read_exact(&mut riff).map_err(io)?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Err(malformed("missing RIFF/WAVE header"));
    }

    // Walk the bookkeeping chunks until `data`. Its payload runs to EOF
    // when the size is a placeholder.
    let mut remaining: Option<usize> = loop {
        let mut header = [0u8; 8];
        reader.read_exact(&mut header).map_err(io)?;
        let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        if &header[0..4] == b"data" {
            break if size == u32::MAX { None } else { Some(size as usize) };
        }
        // A placeholder size on any other chunk means the walk cannot
        // continue - refuse rather than misread samples as headers.
        if size == u32::MAX {
            return Err(malformed("placeholder size before the data chunk"));
        }
        // Chunks are word-aligned; an odd size is followed by a pad byte.
        let skip = size as usize + (size as usize & 1);
        std::io::copy(&mut reader.take(skip as u64), &mut std::io::sink()).map_err(io)?;
    };

    let mut min = Vec::new();
    let mut max = Vec::new();
    let mut low = 0.0f32;
    let mut high = 0.0f32;
    let mut filled = 0usize;

    let mut chunk = [0u8; 8192];
    // A sample split across two reads: its first byte waits here.
    let mut half: Option<u8> = None;
    loop {
        let want = remaining.map_or(chunk.len(), |left| left.min(chunk.len()));
        if want == 0 {
            break;
        }
        let read = reader.read(&mut chunk[..want]).map_err(io)?;
        if read == 0 {
            break;
        }
        if let Some(left) = remaining.as_mut() {
            *left -= read;
        }

        let mut bytes = &chunk[..read];
        if let Some(first) = half.take() {
            let sample = f32::from(i16::from_le_bytes([first, bytes[0]])) / 32768.0;
            fold(sample, &mut low, &mut high);
            bump(&mut filled, bucket_size, &mut min, &mut max, &mut low, &mut high);
            bytes = &bytes[1..];
        }
        for pair in bytes.chunks_exact(2) {
            let sample = f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0;
            fold(sample, &mut low, &mut high);
            bump(&mut filled, bucket_size, &mut min, &mut max, &mut low, &mut high);
        }
        if bytes.len() & 1 == 1 {
            half = Some(bytes[bytes.len() - 1]);
        }
    }

    // The trailing partial bucket still counts - dropping it would shave
    // the last fraction of a second off every waveform.
    if filled > 0 {
        min.push(low);
        max.push(high);
    }

    Ok(Peaks {
        min,
        max,
        buckets_per_second: PEAK_RATE as f32 / bucket_size as f32,
    })
}

fn fold(sample: f32, low: &mut f32, high: &mut f32) {
    if sample < *low {
        *low = sample;
    }
    if sample > *high {
        *high = sample;
    }
}

fn bump(
    filled: &mut usize,
    bucket_size: usize,
    min: &mut Vec<f32>,
    max: &mut Vec<f32>,
    low: &mut f32,
    high: &mut f32,
) {
    *filled += 1;
    if *filled == bucket_size {
        min.push(*low);
        max.push(*high);
        *low = 0.0;
        *high = 0.0;
        *filled = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WAV as FFmpeg writes one to a pipe: placeholder sizes, an `fmt `
    /// chunk, then `data` running to EOF.
    fn piped_wav(samples: &[i16]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 16]);
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn buckets_carry_the_extremes_and_the_tail_partial_counts() {
        // 240 samples per bucket at 200 buckets/second: one full bucket
        // holding the extremes, then a 10-sample partial.
        let mut samples = vec![0i16; 240];
        samples[7] = i16::MIN;
        samples[100] = 16384;
        samples.extend(std::iter::repeat_n(-8192i16, 10));

        let wav = piped_wav(&samples);
        let peaks = peaks_from_wav(&mut wav.as_slice(), Path::new("test.wav"), 200).unwrap();

        assert_eq!(peaks.buckets_per_second, 200.0);
        assert_eq!(peaks.min.len(), 2);
        assert_eq!(peaks.min[0], -1.0);
        assert_eq!(peaks.max[0], 0.5);
        assert_eq!(peaks.min[1], -0.25);
        // Seeded at zero: an all-negative bucket still reports max 0.
        assert_eq!(peaks.max[1], 0.0);
    }

    #[test]
    fn silence_is_flat_zeroes() {
        let wav = piped_wav(&[0i16; 480]);
        let peaks = peaks_from_wav(&mut wav.as_slice(), Path::new("test.wav"), 200).unwrap();
        assert_eq!(peaks.min, vec![0.0, 0.0]);
        assert_eq!(peaks.max, vec![0.0, 0.0]);
    }

    #[test]
    fn a_stream_that_is_not_wav_is_refused() {
        let junk = b"MPEG something else entirely, long enough to read".to_vec();
        let error = peaks_from_wav(&mut junk.as_slice(), Path::new("test.mp3"), 200);
        assert!(error.is_err());
    }

    #[test]
    fn encode_lays_out_rate_count_min_max() {
        let peaks = Peaks { min: vec![-0.5, 0.0], max: vec![0.25, 0.0], buckets_per_second: 200.0 };
        let bytes = peaks.encode();
        assert_eq!(bytes.len(), 8 + 2 * 8);
        assert_eq!(f32::from_le_bytes(bytes[0..4].try_into().unwrap()), 200.0);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
        assert_eq!(f32::from_le_bytes(bytes[8..12].try_into().unwrap()), -0.5);
        assert_eq!(f32::from_le_bytes(bytes[16..20].try_into().unwrap()), 0.25);
    }
}

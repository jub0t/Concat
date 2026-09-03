// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Waveform peaks: min/max sample pairs, bucketed at a fixed rate.
//!
//! The timeline draws a clip's waveform from a few hundred buckets per
//! second, not from the samples themselves. This module produces those
//! buckets by streaming the file's decoded audio - the samples are folded
//! into buckets as they arrive and never accumulate, so an hour-long
//! recording costs the same memory as a jingle.

use std::path::Path;

use crate::error::Result;
use crate::samples::{AudioDecoder, AudioOptions, SampleFormat};

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

    /// The inverse of [`Peaks::encode`]: `None` for bytes that do not have
    /// that shape, so a corrupt cache entry regenerates instead of drawing.
    pub fn decode(bytes: &[u8]) -> Option<Peaks> {
        if bytes.len() < 8 {
            return None;
        }
        let buckets_per_second = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let count = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        if bytes.len() != 8 + count * 8 {
            return None;
        }
        let floats = |offset: usize| -> Vec<f32> {
            bytes[offset..offset + count * 4]
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect()
        };
        Some(Peaks {
            min: floats(8),
            max: floats(8 + count * 4),
            buckets_per_second,
        })
    }
}

/// Decodes one file's audio and reduces it to peaks.
///
/// The decode is mono 16-bit at [`PEAK_RATE`]. A file with no audio stream
/// is an error that says so.
pub fn extract(path: &Path, buckets_per_second: u32) -> Result<Peaks> {
    let mut decoder = AudioDecoder::open(
        path,
        &AudioOptions {
            rate: PEAK_RATE,
            channels: 1,
            format: SampleFormat::I16,
            ..AudioOptions::default()
        },
    )?;
    let mut folder = Folder::new(buckets_per_second);
    while let Some(chunk) = decoder.next_i16()? {
        folder.fold_all(chunk.iter().map(|sample| f32::from(*sample) / 32768.0));
    }
    Ok(folder.finish())
}

/// Folds a sample stream into peaks without holding the samples.
struct Folder {
    bucket_size: usize,
    min: Vec<f32>,
    max: Vec<f32>,
    low: f32,
    high: f32,
    filled: usize,
}

impl Folder {
    fn new(buckets_per_second: u32) -> Self {
        let buckets_per_second = buckets_per_second.clamp(1, PEAK_RATE);
        Self {
            bucket_size: (PEAK_RATE / buckets_per_second) as usize,
            min: Vec::new(),
            max: Vec::new(),
            low: 0.0,
            high: 0.0,
            filled: 0,
        }
    }

    fn fold_all(&mut self, samples: impl Iterator<Item = f32>) {
        for sample in samples {
            if sample < self.low {
                self.low = sample;
            }
            if sample > self.high {
                self.high = sample;
            }
            self.filled += 1;
            if self.filled == self.bucket_size {
                self.min.push(self.low);
                self.max.push(self.high);
                self.low = 0.0;
                self.high = 0.0;
                self.filled = 0;
            }
        }
    }

    fn finish(mut self) -> Peaks {
        // The trailing partial bucket still counts - dropping it would shave
        // the last fraction of a second off every waveform.
        if self.filled > 0 {
            self.min.push(self.low);
            self.max.push(self.high);
        }
        Peaks {
            min: self.min,
            max: self.max,
            buckets_per_second: PEAK_RATE as f32 / self.bucket_size as f32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fold(samples: &[i16], buckets_per_second: u32) -> Peaks {
        let mut folder = Folder::new(buckets_per_second);
        folder.fold_all(samples.iter().map(|sample| f32::from(*sample) / 32768.0));
        folder.finish()
    }

    #[test]
    fn buckets_carry_the_extremes_and_the_tail_partial_counts() {
        // 240 samples per bucket at 200 buckets/second: one full bucket
        // holding the extremes, then a 10-sample partial.
        let mut samples = vec![0i16; 240];
        samples[7] = i16::MIN;
        samples[100] = 16384;
        samples.extend(std::iter::repeat_n(-8192i16, 10));

        let peaks = fold(&samples, 200);
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
        let peaks = fold(&[0i16; 480], 200);
        assert_eq!(peaks.min, vec![0.0, 0.0]);
        assert_eq!(peaks.max, vec![0.0, 0.0]);
    }

    #[test]
    fn a_file_with_no_audio_is_an_error() {
        assert!(extract(Path::new("does-not-exist.mp3"), 200).is_err());
    }

    #[test]
    fn encode_lays_out_rate_count_min_max_and_decode_reads_it_back() {
        let peaks = Peaks {
            min: vec![-0.5, 0.0],
            max: vec![0.25, 0.0],
            buckets_per_second: 200.0,
        };
        let bytes = peaks.encode();
        assert_eq!(bytes.len(), 8 + 2 * 8);
        assert_eq!(f32::from_le_bytes(bytes[0..4].try_into().unwrap()), 200.0);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
        assert_eq!(f32::from_le_bytes(bytes[8..12].try_into().unwrap()), -0.5);
        assert_eq!(f32::from_le_bytes(bytes[16..20].try_into().unwrap()), 0.25);
        let back = Peaks::decode(&bytes).expect("decodes");
        assert_eq!(back.min, peaks.min);
        assert_eq!(back.max, peaks.max);
        assert!(Peaks::decode(&bytes[..10]).is_none());
    }
}

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
#[derive(Clone, Debug, PartialEq)]
pub struct Peaks {
    /// The lowest sample in each bucket, in [-1, 0].
    pub min: Vec<f32>,
    /// The highest sample in each bucket, in [0, 1].
    pub max: Vec<f32>,
    /// How many buckets cover one second of audio.
    pub buckets_per_second: f32,
}

impl Peaks {
    /// The cache format:
    /// `[buckets_per_second f32][count u32][min f32 x count][max f32 x count]`,
    /// little-endian. Frozen: the caches beside existing projects hold it.
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

impl Peaks {
    /// How many buckets there are.
    pub fn len(&self) -> usize {
        self.min.len().min(self.max.len())
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The same waveform at `factor` buckets to one: each new bucket takes
    /// the extremes of the buckets it folds, so nothing quieter or louder
    /// than the fine shape appears at the coarse one. A trailing partial
    /// group still counts.
    pub fn coarser(&self, factor: usize) -> Peaks {
        let factor = factor.max(1);
        let count = self.len();
        let groups = count.div_ceil(factor);
        let mut min = Vec::with_capacity(groups);
        let mut max = Vec::with_capacity(groups);
        for group in 0..groups {
            let from = group * factor;
            let to = (from + factor).min(count);
            min.push(self.min[from..to].iter().copied().fold(0.0, f32::min));
            max.push(self.max[from..to].iter().copied().fold(0.0, f32::max));
        }
        Peaks {
            min,
            max,
            buckets_per_second: self.buckets_per_second / factor as f32,
        }
    }

    /// The extremes over the buckets covering `from` to `to` seconds of the
    /// file, `(low, high)` with low at or below zero and high at or above
    /// it. Past the end, or an empty span, is silence.
    pub fn extremes(&self, from: f32, to: f32) -> (f32, f32) {
        let count = self.len();
        if count == 0 || to <= from || self.buckets_per_second <= 0.0 {
            return (0.0, 0.0);
        }
        // The buckets that begin inside the span, so a bucket straddling
        // two columns is read by one of them and never both, with a hair of
        // float noise at a boundary forgiven. A span narrower than a bucket
        // reads the bucket it sits in.
        const HAIR: f32 = 1e-3;
        let start = from * self.buckets_per_second;
        let end = to * self.buckets_per_second;
        let mut first = ((start - HAIR).ceil().max(0.0) as usize).min(count);
        let mut last = ((end - HAIR).ceil().max(0.0) as usize).min(count);
        if first >= last {
            if start >= count as f32 {
                // Past the end is silence.
                return (0.0, 0.0);
            }
            first = (start.floor().max(0.0) as usize).min(count - 1);
            last = first + 1;
        }
        let (mut low, mut high) = (0.0f32, 0.0f32);
        for index in first..last.max(first) {
            low = low.min(self.min[index]);
            high = high.max(self.max[index]);
        }
        (low, high)
    }
}

/// A waveform at every resolution a drawing could want: the file's peaks
/// as extracted, then halved again and again until a level is a handful of
/// buckets. A drawing asks for the level whose buckets are about the size
/// of its columns, so a clip drawn across two thousand pixels reads two
/// thousand buckets and not two million, and a clip drawn across twenty
/// reads twenty - the same extremes either way, since every level is the
/// fold of the one beneath.
#[derive(Clone, Debug)]
pub struct Pyramid {
    /// Finest first.
    levels: Vec<Peaks>,
}

impl Pyramid {
    /// Every level, from `finest` down to a few buckets.
    pub fn of(finest: Peaks) -> Pyramid {
        let mut levels = vec![finest];
        while levels.last().is_some_and(|level| level.len() > 64) {
            let coarser = levels.last().expect("just checked").coarser(2);
            levels.push(coarser);
        }
        Pyramid { levels }
    }

    /// The peaks as extracted.
    pub fn finest(&self) -> &Peaks {
        &self.levels[0]
    }

    /// The level to draw a column of `seconds` from: the coarsest whose
    /// buckets are no larger than the column, so a column reads one bucket
    /// or a few and never a fraction of one. The finest when even it is
    /// coarser than the column.
    pub fn level_for(&self, seconds_per_column: f32) -> &Peaks {
        if seconds_per_column.is_nan() || seconds_per_column <= 0.0 {
            return self.finest();
        }
        self.levels
            .iter()
            .rev()
            .find(|level| level.buckets_per_second * seconds_per_column >= 1.0)
            .unwrap_or_else(|| self.finest())
    }

    /// How many levels there are.
    pub fn depth(&self) -> usize {
        self.levels.len()
    }
}

/// Decodes one file's audio and reduces it to peaks.
///
/// The decode is mono 16-bit at [`PEAK_RATE`], of the audio stream `stream`
/// names or the file's first. A file with no audio stream is an error that
/// says so.
pub fn extract(path: &Path, buckets_per_second: u32, stream: Option<usize>) -> Result<Peaks> {
    let mut decoder = AudioDecoder::open(
        path,
        &AudioOptions {
            rate: PEAK_RATE,
            channels: 1,
            format: SampleFormat::I16,
            stream,
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
        assert!(extract(Path::new("does-not-exist.mp3"), 200, None).is_err());
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

    #[test]
    fn a_coarser_level_keeps_the_extremes_and_the_tail() {
        let fine = Peaks {
            min: vec![-0.1, -0.9, -0.2, -0.3, -0.5],
            max: vec![0.4, 0.2, 0.8, 0.1, 0.6],
            buckets_per_second: 1000.0,
        };
        let half = fine.coarser(2);
        assert_eq!(half.min, vec![-0.9, -0.3, -0.5]);
        assert_eq!(half.max, vec![0.4, 0.8, 0.6]);
        assert_eq!(half.buckets_per_second, 500.0);
        assert_eq!(
            fine.coarser(0).len(),
            5,
            "a factor of nothing is the same shape"
        );
        assert!(
            Peaks {
                min: vec![],
                max: vec![],
                buckets_per_second: 1000.0
            }
            .coarser(4)
            .is_empty()
        );
    }

    #[test]
    fn the_pyramid_hands_out_the_level_that_fits_the_column() {
        let fine = Peaks {
            min: vec![-0.5; 4096],
            max: vec![0.5; 4096],
            buckets_per_second: 1000.0,
        };
        let pyramid = Pyramid::of(fine);
        assert_eq!(pyramid.depth(), 7, "4096 halves to 64 in six steps");
        // A column of one millisecond reads the finest; of a second, the
        // coarsest that still puts at least one bucket in it.
        assert_eq!(pyramid.level_for(0.001).buckets_per_second, 1000.0);
        let coarse = pyramid.level_for(1.0);
        assert!(
            coarse.buckets_per_second <= 1000.0 / 64.0 + 1e-6,
            "{}",
            coarse.buckets_per_second
        );
        assert!(coarse.buckets_per_second * 1.0 >= 1.0);
        // A column finer than the finest bucket still gets the finest.
        assert_eq!(pyramid.level_for(1e-9).buckets_per_second, 1000.0);
        assert_eq!(pyramid.level_for(f32::NAN).buckets_per_second, 1000.0);
        assert_eq!(pyramid.level_for(-1.0).buckets_per_second, 1000.0);
        // Every level says the same about the whole file.
        let (low, high) = pyramid.finest().extremes(0.0, 4.096);
        let (clow, chigh) = coarse.extremes(0.0, 4.096);
        assert_eq!((low, high), (clow, chigh));
        // Past the end is silence, not a panic.
        assert_eq!(coarse.extremes(100.0, 200.0), (0.0, 0.0));
        assert_eq!(coarse.extremes(2.0, 1.0), (0.0, 0.0));
    }
}

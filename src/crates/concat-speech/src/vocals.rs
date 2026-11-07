// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Vocals apart from music: a song split into its singing and everything
//! else, as two files that add back up to the song. What "remove the
//! vocals" and "keep the vocals" mean; `isolate` is the other kind, the
//! talking in a recording apart from its noise.
//!
//! The model is Deezer's Spleeter, two stems (MIT, weights included; the
//! ONNX export is sherpa-onnx's, fp16), trained on Deezer's own catalogue
//! rather than on MUSDB, whose research-only terms bind the weights of
//! nearly every separator newer than it. Two U-Nets, one per stem, each
//! answering the magnitude of its stem over the low 1024 bins of a 4096
//! point STFT at 44.1 kHz - up to 11 kHz - 512 frames at a time; the two
//! answers are made a ratio mask, as Spleeter makes them. At sixty times
//! the speed of the sound on one core of an M5, and some 20 MB a stem.
//!
//! The vocals are the song's spectrum under the vocal mask, and the rest
//! is the song less them, sample for sample: so the two add back to it,
//! and what lies above 11 kHz, which the networks never hear, stays with
//! the rest rather than being lost from both as a plain mask loses it.
//!
//! Blocks of 512 frames are worked side by side, each on its own: the
//! networks have no memory to warm. Each answers 448 frames and fades
//! into the next over 64.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use concat_media::samples::{AudioDecoder, AudioOptions, SampleFormat};
use concat_vision::runtime::{Data, Input, Model};
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

use crate::isolate::{ChunkOut, Piece, Stems, Wav, Writer, channels_of, part};

/// The networks' rate, and the stems'.
pub const RATE: u32 = 44_100;
const FFT: usize = 4096;
const HOP: usize = 1024;
const BINS: usize = FFT / 2 + 1;
/// The bins the networks hear: up to 11 kHz.
const HEARD: usize = 1024;
/// Frames the networks take at once.
const SPLIT: usize = 512;
/// Frames a block fades into the next over.
const FADE: usize = 64;
/// Frames a block answers for alone.
const CHUNK: usize = SPLIT - FADE;
/// The sum of the Hann window's squares at a quarter overlap.
const OVERLAP_GAIN: f32 = 1.5;

/// The two networks' files, as downloaded.
#[derive(Clone, Debug)]
pub struct Networks {
    /// The network that hears the vocals.
    pub vocals: PathBuf,
    /// The one that hears the rest.
    pub accompaniment: PathBuf,
}

/// What one worker holds while a block runs, measured on a ten-minute
/// stereo song.
const WORKER_BYTES: u64 = 250 << 20;

/// How many blocks run at once with `memory` bytes; see
/// `isolate::workers_for`, whose reasons hold here.
pub fn workers_for(memory: Option<u64>) -> usize {
    if let Some(asked) = std::env::var("CONCAT_ISOLATE_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
    {
        return asked.max(1);
    }
    let cores = std::thread::available_parallelism().map_or(2, |count| count.get());
    let most = if cfg!(any(target_os = "android", target_os = "ios")) {
        2
    } else {
        6
    };
    let by_memory = memory.map_or(2, |bytes| (bytes / 8 / WORKER_BYTES) as usize);
    cores.saturating_sub(2).min(most).min(by_memory).max(1)
}

/// Splits stream `stream` of `source` (the first when `None`) into its
/// vocals and the rest, as `stems`; see `isolate::isolate`, whose files and
/// stopping this shares.
pub fn separate(
    source: &Path,
    stream: Option<usize>,
    networks: &Networks,
    stems: &Stems,
    workers: usize,
    cancel: &AtomicBool,
    progress: crate::isolate::Progress<'_>,
) -> Result<(), String> {
    progress(0.0);
    let channels = channels_of(source, stream)?;
    let samples = length(source, stream, channels, cancel)?;
    if samples == 0 {
        return Err("the clip's file has no sound to separate".to_owned());
    }
    progress(0.03);
    // Every sample has its four frames: from the one that starts three
    // hops before the sound to the one that ends after it.
    let frames = (samples - 1) / HOP + 4;
    let blocks = frames.div_ceil(CHUNK);
    let dsp = Dsp::new();
    let voice_part = part(&stems.voice);
    let rest_part = part(&stems.rest);
    let writer = std::sync::Mutex::new(Writer {
        voice: Wav::create(&voice_part, RATE, channels, samples)?,
        rest: Wav::create(&rest_part, RATE, channels, samples)?,
        seams: std::collections::HashMap::new(),
    });
    let shape = Shape {
        samples,
        frames,
        blocks,
        channels,
    };
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let failed: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let progress = std::sync::Mutex::new(progress);
    std::thread::scope(|scope| {
        for _ in 0..workers.clamp(1, blocks) {
            scope.spawn(|| {
                let fail = |error: String| {
                    *failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
                    next.store(blocks, Ordering::Relaxed);
                };
                let mut nets = match Pair::load(networks) {
                    Ok(nets) => nets,
                    Err(error) => return fail(error),
                };
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= blocks || cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let result = block(source, stream, &dsp, &shape, &mut nets, index)
                        .and_then(|out| writer.lock().unwrap_or_else(|e| e.into_inner()).take(out));
                    if let Err(error) = result {
                        return fail(error);
                    }
                    let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                    let fraction = 0.03 + 0.97 * finished as f32 / blocks as f32;
                    (progress.lock().unwrap_or_else(|e| e.into_inner()))(fraction);
                }
            });
        }
    });
    let failure = failed.into_inner().unwrap_or_else(|e| e.into_inner());
    let failure = failure.or_else(|| {
        cancel
            .load(Ordering::Relaxed)
            .then(|| "vocal separation cancelled".to_owned())
    });
    if let Some(error) = failure {
        let _ = std::fs::remove_file(&voice_part);
        let _ = std::fs::remove_file(&rest_part);
        return Err(error);
    }
    let writer = writer.into_inner().unwrap_or_else(|e| e.into_inner());
    if !writer.seams.is_empty() {
        let _ = std::fs::remove_file(&voice_part);
        let _ = std::fs::remove_file(&rest_part);
        return Err("vocal separation: a block's seam was never met".to_owned());
    }
    writer.voice.finish()?;
    writer.rest.finish()?;
    for (part, whole) in [(&voice_part, &stems.voice), (&rest_part, &stems.rest)] {
        std::fs::rename(part, whole)
            .map_err(|error| format!("could not finish {}: {error}", whole.display()))?;
    }
    Ok(())
}

struct Shape {
    samples: usize,
    frames: usize,
    blocks: usize,
    channels: usize,
}

/// Frame `t` starts three hops before `t * HOP`.
fn first_sample(frame: usize) -> isize {
    (frame * HOP) as isize - (FFT - HOP) as isize
}

fn decoder(
    source: &Path,
    stream: Option<usize>,
    channels: usize,
    start: f64,
    duration: Option<f64>,
) -> Result<AudioDecoder, String> {
    AudioDecoder::open(
        source,
        &AudioOptions {
            start: (start > 0.0).then_some(start),
            duration,
            filters: Vec::new(),
            rate: RATE,
            channels: channels as u16,
            format: SampleFormat::F32,
            stream,
        },
    )
    .map_err(|error| error.to_string())
}

/// The sound's length in samples at [`RATE`], counted by decoding it: a
/// container's stated duration is not to the sample.
fn length(
    source: &Path,
    stream: Option<usize>,
    channels: usize,
    cancel: &AtomicBool,
) -> Result<usize, String> {
    let mut decoder = decoder(source, stream, channels, 0.0, None)?;
    let mut samples = 0;
    while let Some(chunk) = decoder.next_f32().map_err(|error| error.to_string())? {
        if cancel.load(Ordering::Relaxed) {
            return Err("vocal separation cancelled".to_owned());
        }
        samples += chunk.len() / channels;
    }
    Ok(samples)
}

struct Dsp {
    window: Vec<f32>,
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
}

impl Dsp {
    fn new() -> Dsp {
        let mut planner = RealFftPlanner::<f32>::new();
        Dsp {
            // Periodic Hann, as Spleeter's STFT has it.
            window: (0..FFT)
                .map(|i| {
                    (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / FFT as f64).cos()) as f32
                })
                .collect(),
            forward: planner.plan_fft_forward(FFT),
            inverse: planner.plan_fft_inverse(FFT),
        }
    }

    fn analyse(&self, frame: &[f32], out: &mut [Complex32]) {
        let mut windowed: Vec<f32> = frame.iter().zip(&self.window).map(|(s, w)| s * w).collect();
        let _ = self.forward.process(&mut windowed, out);
    }

    /// One frame back, windowed and scaled for overlap-add at a quarter hop.
    fn synthesise(&self, spectrum: &[Complex32], out: &mut [f32]) {
        let mut bins = spectrum.to_vec();
        bins[0].im = 0.0;
        bins[BINS - 1].im = 0.0;
        let _ = self.inverse.process(&mut bins, out);
        let scale = 1.0 / (FFT as f32 * OVERLAP_GAIN);
        for (sample, weight) in out.iter_mut().zip(&self.window) {
            *sample *= weight * scale;
        }
    }
}

/// One worker's two networks.
struct Pair {
    vocals: Model,
    accompaniment: Model,
}

impl Pair {
    fn load(networks: &Networks) -> Result<Pair, String> {
        let read = |path: &Path| {
            std::fs::read(path)
                .map_err(|error| format!("could not read {}: {error}", path.display()))
        };
        Ok(Pair {
            vocals: Model::from_bytes_on_cpu(&read(&networks.vocals)?, 1)?,
            accompaniment: Model::from_bytes_on_cpu(&read(&networks.accompaniment)?, 1)?,
        })
    }

    /// The vocal mask over `[2][frames][HEARD]` magnitudes, `frames` at most
    /// [`SPLIT`]: the two networks' answers made a ratio, as Spleeter
    /// makes it.
    fn mask(&mut self, magnitudes: Vec<f32>) -> Result<Vec<f32>, String> {
        let input = |data: Vec<f32>| Input {
            name: "x",
            dims: vec![2, 1, SPLIT, HEARD],
            data: Data::F32(data),
        };
        let vocals = self.vocals.run(vec![input(magnitudes.clone())], &["y"])?;
        let rest = self.accompaniment.run(vec![input(magnitudes)], &["y"])?;
        let (vocals, rest) = (&vocals[0].data, &rest[0].data);
        if vocals.len() != 2 * SPLIT * HEARD || rest.len() != vocals.len() {
            return Err(format!(
                "vocal separation: the networks answered {} and {} values",
                vocals.len(),
                rest.len()
            ));
        }
        Ok(vocals
            .iter()
            .zip(rest)
            .map(|(v, a)| (v * v + 5e-11) / (v * v + a * a + 1e-10))
            .collect())
    }
}

/// Block `index`: its frames decoded, heard, masked and laid back out.
fn block(
    source: &Path,
    stream: Option<usize>,
    dsp: &Dsp,
    shape: &Shape,
    nets: &mut Pair,
    index: usize,
) -> Result<ChunkOut, String> {
    let last_block = index + 1 == shape.blocks;
    let first = index * CHUNK;
    let last = (first + CHUNK).min(shape.frames);
    let owned_to = if last_block {
        shape.frames
    } else {
        (last + FADE).min(shape.frames)
    };
    let count = owned_to - first;
    let channels = shape.channels;

    let start = first_sample(first);
    let end = first_sample(owned_to - 1) + FFT as isize;
    let decode_from = start.max(0) as usize;
    let decode_to = (end.max(0) as usize).min(shape.samples);
    let span = (end - start) as usize;
    let lead = (decode_from as isize - start) as usize;
    let mut sides = vec![vec![0.0f32; span]; channels];
    if decode_to > decode_from {
        let mut decoder = decoder(
            source,
            stream,
            channels,
            decode_from as f64 / f64::from(RATE),
            Some((decode_to - decode_from) as f64 / f64::from(RATE)),
        )?;
        let mut at = lead;
        while let Some(interleaved) = decoder.next_f32().map_err(|error| error.to_string())? {
            for frame in interleaved.chunks_exact(channels) {
                if at >= lead + (decode_to - decode_from) || at >= span {
                    break;
                }
                for (side, sample) in sides.iter_mut().zip(frame) {
                    side[at] = *sample;
                }
                at += 1;
            }
        }
    }

    let zero = Complex32::new(0.0, 0.0);
    let spectra: Vec<Vec<Complex32>> = sides
        .iter()
        .map(|side| {
            let mut spectrum = vec![zero; count * BINS];
            for (i, out) in spectrum.chunks_exact_mut(BINS).enumerate() {
                let offset = i * HOP;
                dsp.analyse(&side[offset..offset + FFT], out);
            }
            spectrum
        })
        .collect();
    // The networks take two channels: a mono song is both.
    let mut magnitudes = vec![0.0f32; 2 * SPLIT * HEARD];
    for c in 0..2 {
        let spectrum = &spectra[c.min(channels - 1)];
        for t in 0..count {
            for f in 0..HEARD {
                magnitudes[(c * SPLIT + t) * HEARD + f] = spectrum[t * BINS + f].norm();
            }
        }
    }
    let masks = nets.mask(magnitudes)?;

    let weight = |t: usize| -> f32 {
        let mut w = 1.0;
        if index > 0 && t < first + FADE {
            w = (t - first + 1) as f32 / (FADE + 1) as f32;
        }
        if !last_block && t >= last {
            w = 1.0 - (t - last + 1) as f32 / (FADE + 1) as f32;
        }
        w
    };
    let out_from = start.max(0) as usize;
    let out_to = (end as usize).min(shape.samples);
    let length = out_to.saturating_sub(out_from);
    let mut voice = vec![0.0f32; length * channels];
    let mut row = vec![zero; BINS];
    let mut samples = vec![0.0f32; FFT];
    for (c, spectrum) in spectra.iter().enumerate() {
        for i in 0..count {
            row.copy_from_slice(&spectrum[i * BINS..(i + 1) * BINS]);
            for (f, bin) in row.iter_mut().enumerate() {
                let gain = if f < HEARD {
                    if channels == 1 {
                        0.5 * (masks[i * HEARD + f] + masks[(SPLIT + i) * HEARD + f])
                    } else {
                        masks[(c * SPLIT + i) * HEARD + f]
                    }
                } else {
                    0.0
                };
                *bin *= gain;
            }
            dsp.synthesise(&row, &mut samples);
            let w = weight(first + i);
            let begin = first_sample(first + i);
            for (k, value) in samples.iter().enumerate() {
                let s = begin + k as isize;
                if s < out_from as isize || s >= out_to as isize {
                    continue;
                }
                voice[(s as usize - out_from) * channels + c] += value * w;
            }
        }
    }
    let mut original = vec![0.0f32; length * channels];
    for (c, side) in sides.iter().enumerate() {
        for i in 0..length {
            original[i * channels + c] = side[((out_from + i) as isize - start) as usize];
        }
    }

    let seam = |block: usize| -> (usize, usize) {
        let first = block * CHUNK;
        (
            first_sample(first).max(0) as usize,
            (first_sample(first + FADE - 1) + FFT as isize).max(0) as usize,
        )
    };
    let cut = |from_sample: usize, to_sample: usize| -> Piece {
        let (a, b) = (
            from_sample.clamp(out_from, out_to) - out_from,
            to_sample.clamp(out_from, out_to) - out_from,
        );
        Piece {
            at: out_from + a,
            voice: voice[a * channels..b * channels].to_vec(),
            original: original[a * channels..b * channels].to_vec(),
        }
    };
    let head_end = if index > 0 {
        seam(index).1.min(out_to)
    } else {
        out_from
    };
    let tail_start = if last_block {
        out_to
    } else {
        seam(index + 1).0.min(out_to)
    };
    Ok(ChunkOut {
        index,
        head: (index > 0).then(|| cut(out_from, head_end)),
        tail: (!last_block).then(|| cut(tail_start, out_to)),
        body: cut(head_end, tail_start),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Analysis and synthesis at a quarter hop give back the signal.
    #[test]
    fn the_stft_gives_back_what_it_was_given() {
        let dsp = Dsp::new();
        let signal: Vec<f32> = (0..HOP * 20)
            .map(|i| ((i as f32) * 0.011).sin() * 0.4)
            .collect();
        let mut out = vec![0.0f32; signal.len()];
        let mut spectrum = vec![Complex32::new(0.0, 0.0); BINS];
        let mut frame = vec![0.0f32; FFT];
        let mut samples = vec![0.0f32; FFT];
        for t in 0..(signal.len() - 1) / HOP + 4 {
            for (k, v) in frame.iter_mut().enumerate() {
                let s = first_sample(t) + k as isize;
                *v = if s >= 0 && (s as usize) < signal.len() {
                    signal[s as usize]
                } else {
                    0.0
                };
            }
            dsp.analyse(&frame, &mut spectrum);
            dsp.synthesise(&spectrum, &mut samples);
            for (k, v) in samples.iter().enumerate() {
                let s = first_sample(t) + k as isize;
                if s >= 0 && (s as usize) < out.len() {
                    out[s as usize] += v;
                }
            }
        }
        let error = signal
            .iter()
            .zip(&out)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(error < 1e-4, "round trip off by {error}");
    }

    /// The real networks, by hand: `CONCAT_SPLEETER_DIR` holding
    /// `vocals.onnx` and `accompaniment.onnx`, `CONCAT_ISOLATE_IN` a song.
    #[test]
    #[ignore]
    fn separate_a_file_by_hand() {
        let dir = PathBuf::from(std::env::var("CONCAT_SPLEETER_DIR").expect("CONCAT_SPLEETER_DIR"));
        let input = std::env::var("CONCAT_ISOLATE_IN").expect("CONCAT_ISOLATE_IN");
        let networks = Networks {
            vocals: dir.join("vocals.onnx"),
            accompaniment: dir.join("accompaniment.onnx"),
        };
        let stems = Stems {
            voice: PathBuf::from(format!("{input}.vocals.wav")),
            rest: PathBuf::from(format!("{input}.music.wav")),
        };
        let workers = workers_for(Some(16 << 30));
        let started = std::time::Instant::now();
        separate(
            Path::new(&input),
            None,
            &networks,
            &stems,
            workers,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .expect("separates");
        eprintln!("separated in {:?} on {workers} workers", started.elapsed());
    }
}

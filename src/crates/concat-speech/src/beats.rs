// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A song's beats and downbeats: where each beat falls, how sure of it the
//! listening is, which beats start a bar, and the tempo they make.
//!
//! The listening is Beat This!'s small model (Foscarin, Schlüter and
//! Widmer, ISMIR 2024; CPJKU, MIT for its code and weights), exported to
//! ONNX by us at opset 17 with a time axis of any length and carried in
//! the app, 10 MB. It scores 88.8 beat F1 on GTZAN against the full
//! model's 89.1 at an eighth of its size; a classic onset tracker scores
//! some twenty points under either on real music, so there is no second
//! tracker to fall back to. See `reports/Beat detection 2026-11-19.md`.
//!
//! Everything around the network is ours and exact:
//! - the features are the model's own: mono at 22 050 Hz, a periodic Hann
//!   STFT of 1024 taps every 441 samples (50 frames a second), centred
//!   with reflected edges, magnitudes over √1024, 128 Slaney mel bands
//!   from 30 Hz to 11 kHz with no band normalisation, then ln(1 + 1000x).
//!   They match torchaudio's to 1e-4, which the tests hold them to;
//! - a song is listened to in windows ([`CHUNK`] frames) that overlap by
//!   [`BORDER`] frames each side, the earlier window's frames kept where
//!   two overlap, as the model was trained to be read;
//! - a beat is a frame whose activation is the largest within ±3 frames
//!   and above even odds, runs of adjacent ones merged to their middle,
//!   then placed between frames by the parabola through its neighbours:
//!   a frame is 20 ms, coarse for a 60 fps cut;
//! - a downbeat is the beat nearest each downbeat peak.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use concat_core::Halt;
use concat_media::samples::{AudioDecoder, AudioOptions, SampleFormat};
use concat_vision::runtime::{Data, Input, Model};
use realfft::RealFftPlanner;

/// The model, carried in the binary: our ONNX export of Beat This! small0.
const MODEL: &[u8] = include_bytes!("../assets/beat-this/small0.onnx");

/// Samples a second the model listens at.
pub const RATE: u32 = 22_050;
/// Frames a second of the model's features and activations.
pub const FPS: f64 = 50.0;
const FFT: usize = 1024;
const HOP: usize = 441;
const BINS: usize = FFT / 2 + 1;
const MELS: usize = 128;
const F_MIN: f64 = 30.0;
const F_MAX: f64 = 11_000.0;

/// Frames a window of listening holds. The attention in the model's
/// front end costs memory as the square of this - about 1.2 GB at 1500
/// frames, a third of it at 750 - and from 750 frames (15 s) up the beats
/// it finds are as good as at 1500 on every groove tried, where 300 frames
/// (6 s) loses a tenth of them. A phone listens in the shorter windows.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub const CHUNK: usize = 750;
/// Frames a window of listening holds on a computer: 30 s.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub const CHUNK: usize = 1500;
/// Frames each side of a window that are listened to but not kept.
pub const BORDER: usize = 6;

/// Which analysis made a result: a cache keyed on it runs again when the
/// listening or anything after it changes.
pub const VERSION: u32 = 1;

/// One beat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beat {
    /// Seconds into the sound.
    pub time: f64,
    /// How sure the listening is that a beat falls here, 0.5 to 1.
    pub confidence: f32,
    /// Whether it starts a bar.
    pub downbeat: bool,
}

/// A sound's beats.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Beats {
    /// Every beat, in order.
    pub beats: Vec<Beat>,
    /// Beats a minute, from the middle gap between beats; `None` with
    /// fewer than two.
    pub tempo: Option<f64>,
    /// How long the sound listened to was, in seconds.
    pub seconds: f64,
}

/// The beats of `path`'s audio stream `stream` (the first when `None`).
/// `cancel` stops it between windows; `progress` hears 0..1 as it goes.
pub fn analyse_file(
    path: &Path,
    stream: Option<usize>,
    cancel: &AtomicBool,
    progress: impl FnMut(f32),
) -> Result<Beats, Halt> {
    let samples = decode(path, stream, cancel)?;
    analyse(&samples, cancel, progress)
}

/// The beats of `samples`: mono, at [`RATE`].
///
/// One window at a time on every core. Measured on an M5 over three
/// minutes: 1.47 s so, against 2.30 s with a window held to four threads
/// and 1.51-1.55 s with two or five windows at once - the attention moves
/// a gigabyte a window, and several at once contend for memory, not cores.
pub fn analyse(
    samples: &[f32],
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<Beats, Halt> {
    let seconds = samples.len() as f64 / f64::from(RATE);
    let spect = log_mel(samples);
    let frames = spect.len() / MELS;
    if frames < 2 * BORDER + 2 {
        return Ok(Beats {
            seconds,
            ..Beats::default()
        });
    }
    let (beat, downbeat) = activations(&spect, frames, CHUNK, 1, cancel, &mut progress)?;
    Ok(read_beats(&beat, &downbeat, seconds))
}

impl Beats {
    /// The cache format: `CBT` and [`VERSION`] as a byte, the seconds and
    /// tempo (NaN for none) as f64, the count as u32, then each beat as
    /// its time f64, confidence f32 and a downbeat byte; little-endian.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(24 + self.beats.len() * 13);
        bytes.extend_from_slice(b"CBT");
        bytes.push(VERSION as u8);
        bytes.extend_from_slice(&self.seconds.to_le_bytes());
        bytes.extend_from_slice(&self.tempo.unwrap_or(f64::NAN).to_le_bytes());
        bytes.extend_from_slice(&(self.beats.len() as u32).to_le_bytes());
        for beat in &self.beats {
            bytes.extend_from_slice(&beat.time.to_le_bytes());
            bytes.extend_from_slice(&beat.confidence.to_le_bytes());
            bytes.push(u8::from(beat.downbeat));
        }
        bytes
    }

    /// What [`Beats::encode`] wrote, or `None` for anything else - another
    /// version's, a cut-short file - which is then listened to again.
    pub fn decode(bytes: &[u8]) -> Option<Beats> {
        if bytes.len() < 24 || &bytes[..3] != b"CBT" || bytes[3] != VERSION as u8 {
            return None;
        }
        let f64_at = |at: usize| {
            bytes
                .get(at..at + 8)
                .map(|b| f64::from_le_bytes(b.try_into().expect("8")))
        };
        let seconds = f64_at(4)?;
        let tempo = f64_at(12).filter(|tempo| tempo.is_finite());
        let count = u32::from_le_bytes(bytes.get(20..24)?.try_into().ok()?) as usize;
        if bytes.len() != 24 + count * 13 {
            return None;
        }
        let beats = (0..count)
            .map(|index| {
                let at = 24 + index * 13;
                Some(Beat {
                    time: f64_at(at)?,
                    confidence: f32::from_le_bytes(bytes.get(at + 8..at + 12)?.try_into().ok()?),
                    downbeat: bytes[at + 12] != 0,
                })
            })
            .collect::<Option<Vec<Beat>>>()?;
        Some(Beats {
            beats,
            tempo,
            seconds,
        })
    }
}

// ── the models ──────────────────────────────────────────────────────────

/// Loaded copies of the model, kept for the next song: a load costs
/// about 170 ms, longer than the features of a whole song. Each is kept
/// with the threads it was made for.
static LOADED: std::sync::Mutex<Vec<(usize, Model)>> = std::sync::Mutex::new(Vec::new());

/// A loaded model for `threads`, from those kept or new.
fn take_model(threads: usize) -> Result<Model, String> {
    let kept = {
        let mut loaded = LOADED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loaded
            .iter()
            .position(|(made_for, _)| *made_for == threads)
            .map(|index| loaded.swap_remove(index).1)
    };
    match kept {
        Some(model) => Ok(model),
        None => Model::from_bytes_on_cpu(MODEL, threads),
    }
}

/// Keeps `model` for the next song: as many as were in use at once.
fn keep_model(threads: usize, model: Model) {
    LOADED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push((threads, model));
}

/// Decodes `path` to mono at [`RATE`]: both channels at the rate, then
/// their mean, which is the model's downmix (a mono file comes back as
/// two equal channels, whose mean is itself).
fn decode(path: &Path, stream: Option<usize>, cancel: &AtomicBool) -> Result<Vec<f32>, Halt> {
    let mut decoder = AudioDecoder::open(
        path,
        &AudioOptions {
            start: None,
            duration: None,
            filters: Vec::new(),
            rate: RATE,
            channels: 2,
            format: SampleFormat::F32,
            stream,
            stretch: None,
        },
    )
    .map_err(|error| error.to_string())?;
    let mut mono = Vec::new();
    while let Some(interleaved) = decoder.next_f32().map_err(|error| error.to_string())? {
        if cancel.load(Ordering::Relaxed) {
            return Err(Halt::Cancelled);
        }
        mono.extend(
            interleaved
                .chunks_exact(2)
                .map(|pair| 0.5 * (pair[0] + pair[1])),
        );
    }
    Ok(mono)
}

// ── the features ────────────────────────────────────────────────────────

/// Slaney's mel scale: linear below 1 kHz, logarithmic above.
fn hz_to_mel(hz: f64) -> f64 {
    if hz >= 1000.0 {
        15.0 + (hz / 1000.0).ln() / (6.4_f64.ln() / 27.0)
    } else {
        3.0 * hz / 200.0
    }
}

fn mel_to_hz(mel: f64) -> f64 {
    if mel >= 15.0 {
        1000.0 * ((6.4_f64.ln() / 27.0) * (mel - 15.0)).exp()
    } else {
        200.0 * mel / 3.0
    }
}

/// One mel band: the first FFT bin it reads and its weights from there.
struct Band {
    first: usize,
    weights: Vec<f64>,
}

/// The 128 triangular bands over the 513 bins, as torchaudio's
/// `melscale_fbanks` makes them with no normalisation, kept sparse: each
/// band reads a handful of bins, not all 513.
fn bands() -> Vec<Band> {
    let lowest = hz_to_mel(F_MIN);
    let highest = hz_to_mel(F_MAX);
    let edges: Vec<f64> = (0..MELS + 2)
        .map(|index| mel_to_hz(lowest + (highest - lowest) * index as f64 / (MELS + 1) as f64))
        .collect();
    let nyquist = f64::from(RATE) / 2.0;
    let bin_hz = |bin: usize| nyquist * bin as f64 / (BINS - 1) as f64;
    (0..MELS)
        .map(|band| {
            let (left, centre, right) = (edges[band], edges[band + 1], edges[band + 2]);
            let weight = |hz: f64| {
                let rising = (hz - left) / (centre - left);
                let falling = (right - hz) / (right - centre);
                rising.min(falling).max(0.0)
            };
            let bins: Vec<usize> = (0..BINS).filter(|&bin| weight(bin_hz(bin)) > 0.0).collect();
            match (bins.first(), bins.last()) {
                (Some(&first), Some(&last)) => Band {
                    first,
                    weights: (first..=last).map(|bin| weight(bin_hz(bin))).collect(),
                },
                _ => Band {
                    first: 0,
                    weights: Vec::new(),
                },
            }
        })
        .collect()
}

/// The model's features of `samples` (mono at [`RATE`]): `frames × 128`,
/// row-major, one frame per [`HOP`] samples and one more.
pub fn log_mel(samples: &[f32]) -> Vec<f32> {
    let half = FFT / 2;
    if samples.len() <= half {
        return Vec::new();
    }
    // Centred: reflected about the first and last sample, as numpy's
    // "reflect" and torch's do.
    let length = samples.len();
    let at = |index: isize| -> f64 {
        let reflected = if index < 0 {
            -index
        } else if index >= length as isize {
            2 * (length as isize - 1) - index
        } else {
            index
        };
        f64::from(samples[reflected as usize])
    };
    let frames = 1 + length / HOP;
    let window: Vec<f64> = (0..FFT)
        .map(|n| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / FFT as f64).cos())
        .collect();
    let bands = bands();
    let mut planner = RealFftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(FFT);
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();
    let mut magnitudes = vec![0.0_f64; BINS];
    let scale = 1.0 / (FFT as f64).sqrt();
    let mut out = Vec::with_capacity(frames * MELS);
    for frame in 0..frames {
        let start = (frame * HOP) as isize - half as isize;
        for (n, value) in input.iter_mut().enumerate() {
            *value = at(start + n as isize) * window[n];
        }
        fft.process(&mut input, &mut spectrum)
            .expect("the buffers are the plan's own size");
        for (magnitude, bin) in magnitudes.iter_mut().zip(&spectrum) {
            *magnitude = bin.norm() * scale;
        }
        for band in &bands {
            let energy: f64 = band
                .weights
                .iter()
                .zip(&magnitudes[band.first..])
                .map(|(weight, magnitude)| weight * magnitude)
                .sum();
            out.push((1000.0 * energy).ln_1p() as f32);
        }
    }
    out
}

// ── the listening ───────────────────────────────────────────────────────

/// One window's frames of `spect` (`frames` of them), zero-filled where it
/// runs off the song's start and, by up to a border, its end; and its
/// length.
fn window_of(spect: &[f32], frames: usize, start: isize, chunk: usize) -> (Vec<f32>, usize) {
    let left = (-start).max(0) as usize;
    let from = start.max(0) as usize;
    let to = ((start + chunk as isize).max(0) as usize).min(frames);
    let right = ((start + chunk as isize) - frames as isize).clamp(0, BORDER as isize) as usize;
    let length = left + (to - from) + right;
    let mut window = vec![0.0_f32; length * MELS];
    window[left * MELS..(left + to - from) * MELS].copy_from_slice(&spect[from * MELS..to * MELS]);
    (window, length)
}

/// One window listened to: its beat and downbeat logits, a frame each.
fn listen(
    model: &mut Model,
    window: Vec<f32>,
    length: usize,
) -> Result<(Vec<f32>, Vec<f32>), String> {
    let mut outputs = model.run(
        vec![Input {
            name: "spect",
            dims: vec![1, length, MELS],
            data: Data::F32(window),
        }],
        &["beat", "downbeat"],
    )?;
    let down = outputs.pop().ok_or("the listening said nothing")?.data;
    let up = outputs.pop().ok_or("the listening said nothing")?.data;
    if up.len() != length || down.len() != length {
        return Err(format!(
            "the listening answered {} frames for a window of {length}",
            up.len()
        ));
    }
    Ok((up, down))
}

/// The beat and downbeat activations (logits) of `spect`'s `frames`
/// frames, listened to in windows of `chunk`, `workers` at once. The
/// result is the same however many: the windows are put together in
/// order afterwards, the border each side dropped and the earlier
/// window's frames kept where two overlap.
fn activations(
    spect: &[f32],
    frames: usize,
    chunk: usize,
    workers: usize,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(f32),
) -> Result<(Vec<f32>, Vec<f32>), Halt> {
    let starts = window_starts(frames, chunk);
    let workers = workers.clamp(1, starts.len());
    let cores = std::thread::available_parallelism().map_or(1, |cores| cores.get());
    let threads = (cores / workers).clamp(1, cores);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (send, heard) = std::sync::mpsc::channel::<Result<(usize, Vec<f32>, Vec<f32>), String>>();
    let mut listened: Vec<Option<(Vec<f32>, Vec<f32>)>> = vec![None; starts.len()];
    std::thread::scope(|scope| -> Result<(), String> {
        for _ in 0..workers {
            let (send, next, starts) = (send.clone(), &next, &starts);
            scope.spawn(move || {
                let mut model = match take_model(threads) {
                    Ok(model) => model,
                    Err(error) => {
                        let _ = send.send(Err(error));
                        return;
                    }
                };
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= starts.len() || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let (window, length) = window_of(spect, frames, starts[index], chunk);
                    let result =
                        listen(&mut model, window, length).map(|(up, down)| (index, up, down));
                    let failed = result.is_err();
                    if send.send(result).is_err() || failed {
                        break;
                    }
                }
                keep_model(threads, model);
            });
        }
        drop(send);
        let mut done = 0;
        for result in heard {
            let (index, up, down) = result?;
            listened[index] = Some((up, down));
            done += 1;
            progress(done as f32 / starts.len() as f32);
        }
        Ok(())
    })?;
    if cancel.load(Ordering::Relaxed) {
        return Err(Halt::Cancelled);
    }
    let mut beat = vec![f32::NAN; frames];
    let mut downbeat = vec![f32::NAN; frames];
    for (&start, window) in starts.iter().zip(listened) {
        let (up, down) = window.ok_or("a window went unheard")?;
        for offset in BORDER..up.len().saturating_sub(BORDER) {
            let frame = start + offset as isize;
            if frame < 0 || frame as usize >= frames {
                continue;
            }
            let frame = frame as usize;
            if beat[frame].is_nan() {
                beat[frame] = up[offset];
                downbeat[frame] = down[offset];
            }
        }
    }
    // A frame no window kept - none, with the starts as they are - reads
    // as no beat.
    for value in beat.iter_mut().chain(downbeat.iter_mut()) {
        if value.is_nan() {
            *value = -1000.0;
        }
    }
    Ok((beat, downbeat))
}

/// Where each window starts, in frames: a border before the song, then
/// every `chunk - 2 * BORDER`, the last moved back so it ends where the
/// song does rather than running a short window off its end.
fn window_starts(frames: usize, chunk: usize) -> Vec<isize> {
    let step = chunk - 2 * BORDER;
    let mut starts: Vec<isize> = (0..)
        .map(|index| index as isize * step as isize - BORDER as isize)
        .take_while(|&start| start < frames as isize - BORDER as isize)
        .collect();
    if frames > step
        && let Some(last) = starts.last_mut()
    {
        *last = frames as isize - (chunk - BORDER) as isize;
    }
    starts
}

// ── reading the activations ─────────────────────────────────────────────

/// Peaks of `logits`: frames that are the largest within ±3 and above
/// even odds, adjacent runs merged to their middle, each placed between
/// frames by the parabola through it and its neighbours. In frames, with
/// the logit at the peak.
fn peaks(logits: &[f32]) -> Vec<(f64, f32)> {
    let around = |frame: usize| {
        let low = frame.saturating_sub(3);
        let high = (frame + 3).min(logits.len() - 1);
        logits[low..=high]
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max)
    };
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for (frame, &logit) in logits.iter().enumerate() {
        if logit > 0.0 && logit == around(frame) {
            match runs.last_mut() {
                Some(run) if frame - run[run.len() - 1] <= 1 => run.push(frame),
                _ => runs.push(vec![frame]),
            }
        }
    }
    runs.into_iter()
        .map(|run| {
            let middle = run.iter().sum::<usize>() as f64 / run.len() as f64;
            let centre = run[run.len() / 2];
            let logit = logits[centre];
            // Only a lone peak is refined: a run's middle is already
            // between its frames.
            let refined = if run.len() == 1 && centre > 0 && centre + 1 < logits.len() {
                let (before, after) =
                    (f64::from(logits[centre - 1]), f64::from(logits[centre + 1]));
                let curve = before - 2.0 * f64::from(logit) + after;
                if curve < 0.0 {
                    (middle + 0.5 * (before - after) / curve).clamp(middle - 0.5, middle + 0.5)
                } else {
                    middle
                }
            } else {
                middle
            };
            (refined, logit)
        })
        .collect()
}

fn sigmoid(logit: f32) -> f32 {
    1.0 / (1.0 + (-logit).exp())
}

/// The beats the activations say, for a sound of `seconds`.
fn read_beats(beat: &[f32], downbeat: &[f32], seconds: f64) -> Beats {
    let mut beats: Vec<Beat> = peaks(beat)
        .into_iter()
        .map(|(frame, logit)| Beat {
            time: frame / FPS,
            confidence: sigmoid(logit),
            downbeat: false,
        })
        .collect();
    if !beats.is_empty() {
        for (frame, _) in peaks(downbeat) {
            let time = frame / FPS;
            let nearest = beats
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| (a.time - time).abs().total_cmp(&(b.time - time).abs()))
                .map(|(index, _)| index)
                .expect("there are beats");
            beats[nearest].downbeat = true;
        }
    }
    let mut gaps: Vec<f64> = beats
        .windows(2)
        .map(|pair| pair[1].time - pair[0].time)
        .collect();
    gaps.sort_by(f64::total_cmp);
    let tempo = gaps
        .get(gaps.len() / 2)
        .filter(|gap| **gap > 0.0)
        .map(|gap| 60.0 / gap);
    Beats {
        beats,
        tempo,
        seconds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The groove the parity fixtures were made from, sample for sample as
    /// `reports/beat-research/fixture.py` makes it: a kick on every beat at
    /// 120 BPM from 0.25 s, a snare on 2 and 4, a hat off the beat; 20 s.
    fn groove() -> Vec<f32> {
        let rate = f64::from(RATE);
        let (bpm, seconds, offset) = (120.0, 20.0, 0.25);
        let length = (seconds * rate) as usize;
        let mut y = vec![0.0_f64; length];
        let beat = 60.0 / bpm;
        let mut k = 0usize;
        let mut t: f64 = offset;
        while t < seconds - 0.3 {
            let i = (t * rate).round_ties_even() as usize;
            let n = 4000.min(length - i);
            let mut phase = 0.0;
            for m in 0..n {
                let tt = m as f64 / rate;
                phase += 50.0 + 100.0 * (-tt * 30.0).exp();
                y[i + m] += (2.0 * std::f64::consts::PI * phase / rate).sin() * (-tt * 8.0).exp();
                if k % 2 == 1 {
                    let ring: f64 = [190.0, 331.0, 587.0, 1213.0]
                        .iter()
                        .map(|f| (2.0 * std::f64::consts::PI * f * tt).sin())
                        .sum();
                    y[i + m] += 0.5 * ring * (-tt * 20.0).exp();
                }
            }
            let h = ((t + beat / 2.0) * rate).round_ties_even() as usize;
            for m in 0..1500.min(length - h) {
                let th = m as f64 / rate;
                y[h + m] += 0.2
                    * ((2.0 * std::f64::consts::PI * 7040.0 * th).sin()
                        + (2.0 * std::f64::consts::PI * 9011.0 * th).sin())
                    * (-th * 60.0).exp();
            }
            k += 1;
            t = offset + k as f64 * beat;
        }
        let peak = y.iter().fold(0.0_f64, |most, v| most.max(v.abs()));
        y.iter().map(|v| (v / peak * 0.8) as f32).collect()
    }

    /// The bands are torchaudio's: their weights sum to what its matrix
    /// sums to (502.2056 for these settings), and every band reads bins.
    #[test]
    fn the_bands_are_torchaudios() {
        let bands = bands();
        assert_eq!(bands.len(), MELS);
        let total: f64 = bands.iter().flat_map(|band| &band.weights).sum();
        assert!((total - 502.2056).abs() < 1e-3, "{total}");
        assert!(bands.iter().all(|band| !band.weights.is_empty()));
    }

    /// The features match the reference made with numpy over torchaudio's
    /// own filterbank (which matches torchaudio to 1.1e-4).
    #[test]
    fn the_features_match_the_reference() {
        let spect = log_mel(&groove());
        assert_eq!(spect.len() / MELS, 1001);
        let at = |frame: usize, band: usize| spect[frame * MELS + band];
        for (frame, band, want) in [
            (10, 5, 0.0),
            (50, 64, 1.352_349),
            (80, 120, 0.0),
            (500, 30, 0.807_722),
            (999, 0, 0.0),
        ] {
            let got = at(frame, band);
            assert!(
                (got - want).abs() < 1e-4,
                "[{frame},{band}] {got} vs {want}"
            );
        }
        let total: f64 = spect.iter().map(|v| f64::from(*v)).sum();
        assert!((total - 81_508.456).abs() < 0.5, "{total}");
    }

    /// The windows cover the song with a border each side, the last moved
    /// back to end where the song does.
    #[test]
    fn the_windows_cover_the_song() {
        assert_eq!(window_starts(1001, 1500), vec![-6]);
        assert_eq!(window_starts(3000, 1500), vec![-6, 1482, 1506]);
        assert_eq!(window_starts(3000, 750), vec![-6, 732, 1470, 2208, 2256]);
    }

    /// The listening matches the reference run on the same groove, window
    /// for window (six zero frames each side, as the inference pads): the
    /// same logits to 1e-3 and the same 29 beats and 17 downbeats. The
    /// beats it misses there are the model's own, logits a hair under even
    /// odds on a thin synthetic groove (-0.005 at frame 338); the bar
    /// decoder that keeps a tempo is what fills such gaps.
    #[test]
    fn the_listening_matches_the_reference() {
        let spect = log_mel(&groove());
        let frames = spect.len() / MELS;
        let (beat, downbeat) = activations(
            &spect,
            frames,
            CHUNK,
            1,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .expect("listens");
        for (frame, want) in [
            (13, 1.07846),
            (38, 0.47821),
            (338, -0.00462),
            (588, -0.01038),
            (900, -0.94117),
        ] {
            assert!(
                (beat[frame] - want).abs() < 1e-3,
                "logit[{frame}] {} vs {want}",
                beat[frame]
            );
        }
        let found = read_beats(&beat, &downbeat, 20.0);
        let frames: Vec<i64> = found
            .beats
            .iter()
            .map(|beat| (beat.time * FPS).round() as i64)
            .collect();
        let reference = [
            13, 38, 63, 88, 113, 138, 163, 188, 213, 238, 263, 288, 313, 363, 388, 413, 438, 463,
            488, 513, 538, 563, 613, 663, 713, 738, 763, 813, 863,
        ];
        assert_eq!(frames, reference.to_vec());
        let downbeats: Vec<i64> = found
            .beats
            .iter()
            .filter(|beat| beat.downbeat)
            .map(|beat| (beat.time * FPS).round() as i64)
            .collect();
        let downbeat_reference = [
            13, 63, 113, 163, 213, 263, 313, 363, 413, 463, 513, 563, 613, 663, 763, 813, 863,
        ];
        assert_eq!(downbeats, downbeat_reference.to_vec());
    }

    /// Every beat found is where a kick was put, within 20 ms - the
    /// sub-frame placement is what gets a 20 ms frame there - and the tempo
    /// is the groove's 120.
    #[test]
    fn the_beats_fall_on_the_groove() {
        let found = analyse(&groove(), &AtomicBool::new(false), |_| {}).expect("listens");
        assert!(found.beats.len() >= 29, "{}", found.beats.len());
        for beat in &found.beats {
            let truth = 0.25 + ((beat.time - 0.25) / 0.5).round() * 0.5;
            assert!((beat.time - truth).abs() < 0.02, "{} vs {truth}", beat.time);
            assert!(beat.confidence > 0.5);
        }
        let tempo = found.tempo.expect("a tempo");
        assert!((tempo - 120.0).abs() < 1.0, "{tempo}");
    }

    /// How long three minutes takes, stage by stage (run by hand:
    /// `cargo test -p concat-speech --features beats --release -- --ignored
    /// --nocapture three_minutes`).
    #[test]
    #[ignore = "a timing, not a check"]
    fn three_minutes() {
        let one = groove();
        let song: Vec<f32> = one.iter().copied().cycle().take(one.len() * 9).collect();
        let cores = std::thread::available_parallelism().map_or(1, |cores| cores.get());
        let spect = log_mel(&song);
        let frames = spect.len() / MELS;
        for workers in [1, 2, cores / 2] {
            // The first run of each count loads its models; the second is
            // what every later song costs.
            for run in ["first", "again"] {
                let begun = std::time::Instant::now();
                let (beat, downbeat) = activations(
                    &spect,
                    frames,
                    CHUNK,
                    workers,
                    &AtomicBool::new(false),
                    &mut |_| {},
                )
                .expect("listens");
                let found = read_beats(&beat, &downbeat, 180.0);
                eprintln!(
                    "{workers} window(s) at once, {run}: {:.0} ms, {} beats",
                    begun.elapsed().as_secs_f64() * 1e3,
                    found.beats.len()
                );
            }
        }
    }

    /// Several windows at once find exactly what one at a time does: the
    /// windows are put together in order, whoever finished first.
    #[test]
    fn windows_at_once_hear_the_same() {
        let one = groove();
        let song: Vec<f32> = one.iter().copied().cycle().take(one.len() * 4).collect();
        let spect = log_mel(&song);
        let frames = spect.len() / MELS;
        let hear = |workers| {
            activations(
                &spect,
                frames,
                CHUNK,
                workers,
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .expect("listens")
        };
        let (alone, together) = (hear(1), hear(3));
        assert_eq!(alone, together);
        assert!(read_beats(&alone.0, &alone.1, 80.0).beats.len() > 100);
    }

    /// A result comes back from its cache as it went in; another version's
    /// or a cut-short one does not come back at all.
    #[test]
    fn the_cache_keeps_the_beats() {
        let beats = Beats {
            beats: vec![
                Beat {
                    time: 0.26,
                    confidence: 0.75,
                    downbeat: true,
                },
                Beat {
                    time: 0.76,
                    confidence: 0.6,
                    downbeat: false,
                },
            ],
            tempo: Some(120.0),
            seconds: 20.0,
        };
        let bytes = beats.encode();
        assert_eq!(Beats::decode(&bytes), Some(beats.clone()));
        assert_eq!(Beats::decode(&bytes[..bytes.len() - 1]), None);
        let mut other = bytes.clone();
        other[3] = other[3].wrapping_add(1);
        assert_eq!(Beats::decode(&other), None);
        let none = Beats {
            tempo: None,
            ..Beats::default()
        };
        assert_eq!(Beats::decode(&none.encode()), Some(none));
    }

    /// Too short to listen to is no beats, not an error.
    #[test]
    fn a_blip_has_no_beats() {
        let found = analyse(&[0.0; 300], &AtomicBool::new(false), |_| {}).expect("listens");
        assert!(found.beats.is_empty());
        assert!(found.tempo.is_none());
    }

    #[test]
    fn listening_tells_its_cancel_from_a_file_it_cannot_read_under_a_folder_named_cancelled() {
        let missing = std::env::temp_dir()
            .join(format!("concat-beats-halt-{}", std::process::id()))
            .join("cancelled")
            .join("song.wav");
        let failed = analyse_file(&missing, None, &AtomicBool::new(false), |_| {})
            .expect_err("there is no song");
        assert!(!failed.is_cancelled(), "{failed}");
        assert!(
            failed.failure().is_some_and(
                |message| message.contains("song.wav") && message.contains("cancelled")
            ),
            "the failure names the file, folder and all: {failed}"
        );

        let stopped = analyse(&groove(), &AtomicBool::new(true), |_| {});
        assert_eq!(stopped, Err(Halt::Cancelled));
    }
}

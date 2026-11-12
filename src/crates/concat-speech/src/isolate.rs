// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Voice isolation: a recording split into the voice in it and everything
//! else, as two files that add back up to the recording.
//!
//! The voice is DeepFilterNet 3's (Hendrik Schröter, MIT or Apache-2.0,
//! weights included; `assets/deepfilternet3`): a speech enhancer two million
//! weights strong, run here on the CPU at a hundred times the speed of the
//! sound on one core of an M5 and well above it on any machine Concat runs
//! on, phones included. Everything else is the recording less that voice,
//! sample for sample, so voice and background played together at their
//! own levels are the recording, and either alone is what the clip's
//! Voice setting asks for.
//!
//! The networks are the three ONNX graphs upstream exports - encoder, ERB
//! gains, deep-filter coefficients - and what is around them is
//! re-written here from libDF: a Vorbis-windowed 960-point STFT at 48 kHz
//! with a 480 hop, the ERB band energies and the low spectrum as features,
//! each normalised by a running mean, and the decoders' gains and filters
//! laid back on the spectrum.
//!
//! A file is done in chunks, side by side, one a core. A chunk of a GRU
//! network wants the state the frames before it left, so each starts a
//! second early and throws that second away; the features' running means,
//! which a second would not settle, are carried over exactly from a quick
//! first pass over the whole sound. Measured against one pass over the
//! whole file, chunks so made come out 57 dB apart from it - the same
//! sound - where chunks without the first pass lost 0.3 dB of the voice.
//! Memory is a few chunks' worth whatever the file's length.
//!
//! The model hears the channels mixed to one; its gains and filters are
//! laid on each channel's own spectrum, which keeps the stereo picture at
//! one model's cost.

use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use concat_media::samples::{AudioDecoder, AudioOptions, SampleFormat};
use concat_vision::runtime::{Data, Input, Model};
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// The rate the network was trained at, and the stems' rate.
pub const RATE: u32 = 48_000;
const FFT: usize = 960;
const HOP: usize = 480;
const BINS: usize = FFT / 2 + 1;
const ERB_BANDS: usize = 32;
const DF_BINS: usize = 96;
const DF_ORDER: usize = 5;
const DF_LOOKAHEAD: usize = 2;
const CONV_LOOKAHEAD: usize = 2;
/// Frames a chunk is answerable for: ten seconds. With its warm-up and
/// its fade each holds some 70 MB of the networks' working tensors while
/// it runs, which is what bounds a run's memory.
const CHUNK: usize = 1000;
/// Frames run before a chunk and thrown away, for the GRUs' state: five
/// seconds. The networks remember for longer than one might think - a
/// one-second start came out 12 dB from a whole-file pass in the second
/// after a seam - and five measured 28 dB at the worst second and 40
/// overall, which is the same sound. Run time is cheap here; memory is
/// what a small machine is short of.
const WARM: usize = 500;
/// Frames over which one chunk hands over to the next, each weighed
/// against the other: one second.
const FADE: usize = 100;

const ENCODER: &[u8] = include_bytes!("../assets/deepfilternet3/enc.onnx");
const ERB_DECODER: &[u8] = include_bytes!("../assets/deepfilternet3/erb_dec.onnx");
const DF_DECODER: &[u8] = include_bytes!("../assets/deepfilternet3/df_dec.onnx");

/// The two files a recording is split into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stems {
    /// The voice: 16-bit PCM WAV at [`RATE`], the recording's channels.
    pub voice: PathBuf,
    /// The recording less the voice, in the same format.
    pub rest: PathBuf,
}

/// Where a run has got to, for a progress bar: 0 to 1.
pub type Progress<'a> = &'a mut (dyn FnMut(f32) + Send);

/// What one worker holds while a chunk runs - the networks' tensors and
/// the chunk's spectra - measured at some 280 MB on a ten-minute stereo
/// file.
const WORKER_BYTES: u64 = 300 << 20;

/// How many chunks run at once on a machine with `memory` bytes: all its
/// cores but two, left to playback and the window as transcription leaves
/// them (#388); at most two on a phone, which keeps it from heating up and
/// slowing down; and no more than an eighth of the memory holds, so an
/// 8 GB laptop runs three, not six, and stays usable while it works.
/// `CONCAT_ISOLATE_WORKERS` overrides it, for measuring.
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

/// Splits stream `stream` of `source` (the first when `None`) into
/// `stems`, written beside them as `.part` files and renamed when whole,
/// so a stopped run leaves nothing that could be taken for a finished one.
/// `cancel` stops it between chunks; `workers` chunks run at once (see
/// [`workers_for`]).
pub fn isolate(
    source: &Path,
    stream: Option<usize>,
    stems: &Stems,
    workers: usize,
    cancel: &AtomicBool,
    progress: Progress<'_>,
) -> Result<(), String> {
    let dsp = Dsp::new();
    progress(0.0);
    let channels = channels_of(source, stream)?;
    let survey = survey(source, stream, channels, &dsp, cancel)?;
    progress(0.05);
    if survey.samples == 0 {
        return Err("the clip's file has no sound to separate".to_owned());
    }
    let chunks = survey.frames.div_ceil(CHUNK);
    let channels = survey.channels;
    let voice_part = part(&stems.voice);
    let rest_part = part(&stems.rest);
    let writer = std::sync::Mutex::new(Writer {
        voice: Wav::create(&voice_part, RATE, channels, survey.samples)?,
        rest: Wav::create(&rest_part, RATE, channels, survey.samples)?,
        seams: std::collections::HashMap::new(),
    });

    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let failed: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let progress = std::sync::Mutex::new(progress);
    let workers = workers.clamp(1, chunks);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                let mut nets = match Networks::new() {
                    Ok(nets) => nets,
                    Err(error) => {
                        *failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
                        cancel_all(&next, chunks);
                        return;
                    }
                };
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= chunks || cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let result = chunk(source, stream, &dsp, &survey, &mut nets, index)
                        .and_then(|out| writer.lock().unwrap_or_else(|e| e.into_inner()).take(out));
                    if let Err(error) = result {
                        *failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
                        cancel_all(&next, chunks);
                        return;
                    }
                    let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                    let fraction = 0.05 + 0.95 * finished as f32 / chunks as f32;
                    (progress.lock().unwrap_or_else(|e| e.into_inner()))(fraction);
                }
            });
        }
    });
    let failure = failed.into_inner().unwrap_or_else(|e| e.into_inner());
    if failure.is_none() && cancel.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&voice_part);
        let _ = std::fs::remove_file(&rest_part);
        return Err("voice isolation cancelled".to_owned());
    }
    if let Some(error) = failure {
        let _ = std::fs::remove_file(&voice_part);
        let _ = std::fs::remove_file(&rest_part);
        return Err(error);
    }
    let writer = writer.into_inner().unwrap_or_else(|e| e.into_inner());
    if !writer.seams.is_empty() {
        let _ = std::fs::remove_file(&voice_part);
        let _ = std::fs::remove_file(&rest_part);
        return Err("voice isolation: a chunk's seam was never met".to_owned());
    }
    writer.voice.finish()?;
    writer.rest.finish()?;
    for (part, whole) in [(&voice_part, &stems.voice), (&rest_part, &stems.rest)] {
        std::fs::rename(part, whole)
            .map_err(|error| format!("could not finish {}: {error}", whole.display()))?;
    }
    Ok(())
}

fn cancel_all(next: &AtomicUsize, chunks: usize) {
    next.store(chunks, Ordering::Relaxed);
}

pub(crate) fn part(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

// ── the signal around the networks ──────────────────────────────────────

/// The STFT, its window and the ERB bands, made once.
struct Dsp {
    window: Vec<f32>,
    /// Upstream's analysis scale, `2 * hop / fft²`; synthesis undoes it.
    wnorm: f32,
    erb: [usize; ERB_BANDS],
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    /// The running means' weight on the past, per frame: `exp(-hop / sr)`
    /// for upstream's one-second time constant.
    alpha: f32,
}

impl Dsp {
    fn new() -> Dsp {
        let mut planner = RealFftPlanner::<f32>::new();
        let half = FFT / 2;
        let window = (0..FFT)
            .map(|i| {
                let sin = (0.5 * std::f64::consts::PI * (i as f64 + 0.5) / half as f64).sin();
                (0.5 * std::f64::consts::PI * sin * sin).sin() as f32
            })
            .collect();
        Dsp {
            window,
            wnorm: (2 * HOP) as f32 / (FFT * FFT) as f32,
            erb: erb_bands(),
            forward: planner.plan_fft_forward(FFT),
            inverse: planner.plan_fft_inverse(FFT),
            alpha: (-(HOP as f64) / f64::from(RATE)).exp() as f32,
        }
    }

    /// The spectrum of one frame of `FFT` samples.
    fn analyse(&self, frame: &[f32], out: &mut [Complex32]) {
        let mut windowed: Vec<f32> = frame
            .iter()
            .zip(&self.window)
            .map(|(sample, weight)| sample * weight)
            .collect();
        // The lengths are ours, so the plan cannot refuse them.
        let _ = self.forward.process(&mut windowed, out);
        for bin in out.iter_mut() {
            *bin *= self.wnorm;
        }
    }

    /// One frame back to `FFT` samples, windowed for overlap-add. The
    /// Vorbis window's squares sum to one at half overlap, so the adds
    /// need no normalising.
    fn synthesise(&self, spectrum: &[Complex32], out: &mut [f32]) {
        let mut bins = spectrum.to_vec();
        // The DC and Nyquist bins of a real signal's spectrum are real;
        // the filters can leave a rounding's worth of imaginary in them.
        bins[0].im = 0.0;
        bins[BINS - 1].im = 0.0;
        let _ = self.inverse.process(&mut bins, out);
        // realfft's inverse is unscaled: 1 / FFT, and the analysis scale off.
        let scale = 1.0 / (FFT as f32 * self.wnorm);
        for (sample, weight) in out.iter_mut().zip(&self.window) {
            *sample *= weight * scale;
        }
    }
}

fn freq_to_erb(hz: f32) -> f32 {
    9.265 * (hz / (24.7 * 9.265)).ln_1p()
}

fn erb_to_freq(erb: f32) -> f32 {
    24.7 * 9.265 * ((erb / 9.265).exp() - 1.0)
}

/// libDF's `erb_fb`: how many bins each of the 32 bands holds, at least
/// two, covering all 481.
fn erb_bands() -> [usize; ERB_BANDS] {
    let width = RATE as f32 / FFT as f32;
    let (low, high) = (freq_to_erb(0.0), freq_to_erb(RATE as f32 / 2.0));
    let step = (high - low) / ERB_BANDS as f32;
    let mut bands = [0usize; ERB_BANDS];
    let (mut previous, mut over) = (0i32, 0i32);
    for (i, band) in bands.iter_mut().enumerate() {
        let bin = (erb_to_freq(low + (i + 1) as f32 * step) / width).round() as i32;
        let mut count = bin - previous - over;
        if count < 2 {
            over = 2 - count;
            count = 2;
        } else {
            over = 0;
        }
        *band = count as usize;
        previous = bin;
    }
    bands[ERB_BANDS - 1] += 1;
    let total: usize = bands.iter().sum();
    if total > BINS {
        bands[ERB_BANDS - 1] -= total - BINS;
    }
    bands
}

/// The features' running means at one frame: upstream starts them at
/// these ramps.
#[derive(Clone)]
struct Norms {
    erb: [f32; ERB_BANDS],
    unit: [f32; DF_BINS],
}

impl Norms {
    fn start() -> Norms {
        let mut norms = Norms {
            erb: [0.0; ERB_BANDS],
            unit: [0.0; DF_BINS],
        };
        for (i, value) in norms.erb.iter_mut().enumerate() {
            *value = -60.0 + (-90.0 + 60.0) * i as f32 / (ERB_BANDS - 1) as f32;
        }
        for (i, value) in norms.unit.iter_mut().enumerate() {
            *value = 0.001 + (0.0001 - 0.001) * i as f32 / (DF_BINS - 1) as f32;
        }
        norms
    }

    /// One frame's features, the means moved on by it: the ERB bands'
    /// energies in dB less their mean, over 40, and the low spectrum over
    /// the root of its mean magnitude.
    fn features(
        &mut self,
        dsp: &Dsp,
        spectrum: &[Complex32],
    ) -> ([f32; ERB_BANDS], [Complex32; DF_BINS]) {
        let mut erb = [0.0f32; ERB_BANDS];
        let mut at = 0;
        for (band, &count) in erb.iter_mut().zip(&dsp.erb) {
            let energy: f32 = spectrum[at..at + count]
                .iter()
                .map(|bin| bin.norm_sqr())
                .sum();
            *band = 10.0 * (energy / count as f32 + 1e-10).log10();
            at += count;
        }
        let a = dsp.alpha;
        for (value, mean) in erb.iter_mut().zip(self.erb.iter_mut()) {
            *mean = *value * (1.0 - a) + *mean * a;
            *value = (*value - *mean) / 40.0;
        }
        let mut low = [Complex32::new(0.0, 0.0); DF_BINS];
        for ((out, bin), mean) in low
            .iter_mut()
            .zip(&spectrum[..DF_BINS])
            .zip(self.unit.iter_mut())
        {
            *mean = bin.norm() * (1.0 - a) + *mean * a;
            *out = bin / mean.sqrt();
        }
        (erb, low)
    }
}

/// What the first pass learnt: the sound's length, its channels, and the
/// running means where each chunk's run begins.
struct Survey {
    samples: usize,
    /// One or two, as the source is mono or not.
    channels: usize,
    frames: usize,
    norms: Vec<Norms>,
}

/// Frame `t` is the `FFT` samples about `t * HOP`: from `t * HOP - HOP`.
fn first_sample(frame: usize) -> isize {
    (frame * HOP) as isize - HOP as isize
}

/// Where chunk `index` begins its run: a second before its first frame.
fn warm_start(index: usize) -> usize {
    (index * CHUNK).saturating_sub(WARM)
}

/// The stems' channels: one for a mono source, two for any other. Read
/// from the file rather than decoded as stereo throughout: FFmpeg's mono
/// to stereo is three decibels down a side, and stems decoded so played
/// back three decibels quieter than the clip they stand in for.
pub(crate) fn channels_of(source: &Path, stream: Option<usize>) -> Result<usize, String> {
    let info = concat_media::probe::probe(source).map_err(|error| error.to_string())?;
    let audio = info
        .audio_streams
        .iter()
        .find(|audio| stream.is_none_or(|index| audio.index as usize == index))
        .or(info.audio.as_ref())
        .ok_or("the clip's file has no sound to separate")?;
    Ok(if audio.channels == 1 { 1 } else { 2 })
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

/// The first pass: the sound once through, mixed to one, framed, and its
/// features' means kept where each chunk will start.
fn survey(
    source: &Path,
    stream: Option<usize>,
    channels: usize,
    dsp: &Dsp,
    cancel: &AtomicBool,
) -> Result<Survey, String> {
    let mut decoder = decoder(source, stream, channels, 0.0, None)?;
    let mut norms = Norms::start();
    let mut kept = Vec::new();
    // The mono sound from `first_sample(frame)`, as far as decoded.
    let mut window: Vec<f32> = vec![0.0; HOP];
    let mut frame = 0usize;
    let mut samples = 0usize;
    let mut spectrum = vec![Complex32::new(0.0, 0.0); BINS];
    let mut take =
        |window: &mut Vec<f32>, frame: &mut usize, norms: &mut Norms, kept: &mut Vec<Norms>| {
            while window.len() >= FFT {
                if *frame == warm_start(kept.len()) && kept.len() * CHUNK <= *frame + WARM {
                    kept.push(norms.clone());
                }
                dsp.analyse(&window[..FFT], &mut spectrum);
                norms.features(dsp, &spectrum);
                window.drain(..HOP);
                *frame += 1;
            }
        };
    while let Some(interleaved) = decoder.next_f32().map_err(|error| error.to_string())? {
        if cancel.load(Ordering::Relaxed) {
            return Err("voice isolation cancelled".to_owned());
        }
        for frame in interleaved.chunks_exact(channels) {
            window.push(frame.iter().sum::<f32>() / channels as f32);
        }
        samples += interleaved.len() / channels;
        take(&mut window, &mut frame, &mut norms, &mut kept);
    }
    // The frames past the end: enough that every sample has its two.
    let frames = if samples == 0 {
        0
    } else {
        (samples - 1) / HOP + 2
    };
    while frame < frames {
        window.resize(window.len().max(FFT), 0.0);
        take(&mut window, &mut frame, &mut norms, &mut kept);
    }
    Ok(Survey {
        samples,
        channels,
        frames,
        norms: kept,
    })
}

// ── the networks ────────────────────────────────────────────────────────

/// One worker's three sessions.
struct Networks {
    encoder: Model,
    erb: Model,
    df: Model,
}

impl Networks {
    fn new() -> Result<Networks, String> {
        Ok(Networks {
            encoder: Model::from_bytes_on_cpu(ENCODER, 1)?,
            erb: Model::from_bytes_on_cpu(ERB_DECODER, 1)?,
            df: Model::from_bytes_on_cpu(DF_DECODER, 1)?,
        })
    }

    /// The ERB gains `[frames][32]` and deep-filter coefficients
    /// `[frames][96][order]` for `frames` frames of features.
    fn run(
        &mut self,
        erb: Vec<f32>,
        spec: Vec<f32>,
        frames: usize,
    ) -> Result<(Vec<f32>, Vec<f32>), String> {
        let mut encoded = self.encoder.run(
            vec![
                Input {
                    name: "feat_erb",
                    dims: vec![1, 1, frames, ERB_BANDS],
                    data: Data::F32(erb),
                },
                Input {
                    name: "feat_spec",
                    dims: vec![1, 2, frames, DF_BINS],
                    data: Data::F32(spec),
                },
            ],
            &["e0", "e1", "e2", "e3", "emb", "c0"],
        )?;
        let take = |output: concat_vision::runtime::Output, name: &'static str| Input {
            name,
            dims: output.dims,
            data: Data::F32(output.data),
        };
        let c0 = encoded.pop().ok_or("the encoder said nothing")?;
        let emb = encoded.pop().ok_or("the encoder said nothing")?;
        let e3 = encoded.pop().ok_or("the encoder said nothing")?;
        let e2 = encoded.pop().ok_or("the encoder said nothing")?;
        let e1 = encoded.pop().ok_or("the encoder said nothing")?;
        let e0 = encoded.pop().ok_or("the encoder said nothing")?;
        let coefs = self
            .df
            .run(vec![take(emb.clone(), "emb"), take(c0, "c0")], &["coefs"])?
            .pop()
            .ok_or("the deep filter said nothing")?;
        let gains = self
            .erb
            .run(
                vec![
                    take(emb, "emb"),
                    take(e3, "e3"),
                    take(e2, "e2"),
                    take(e1, "e1"),
                    take(e0, "e0"),
                ],
                &["m"],
            )?
            .pop()
            .ok_or("the ERB decoder said nothing")?;
        if gains.data.len() != frames * ERB_BANDS
            || coefs.data.len() != frames * DF_BINS * DF_ORDER * 2
        {
            return Err(format!(
                "voice isolation: the networks answered {:?} and {:?} for {frames} frames",
                gains.dims, coefs.dims
            ));
        }
        Ok((gains.data, coefs.data))
    }
}

/// A stretch of a stem: its first sample and its samples, interleaved in
/// the survey's channels.
pub(crate) struct Piece {
    pub(crate) at: usize,
    pub(crate) voice: Vec<f32>,
    /// The recording over the same stretch.
    pub(crate) original: Vec<f32>,
}

/// What a chunk answers: the stretch it alone covers, and the two seams
/// it shares with its neighbours - its frames weighed by the fade, to be
/// added to the neighbour's over the same samples.
pub(crate) struct ChunkOut {
    pub(crate) index: usize,
    pub(crate) body: Piece,
    /// The seam with the chunk before, which is seam `index`.
    pub(crate) head: Option<Piece>,
    /// The seam with the chunk after, seam `index + 1`.
    pub(crate) tail: Option<Piece>,
}

/// Seam `index`'s samples: from the start of the chunk's first frame to the
/// end of its last faded one.
fn seam_span(index: usize) -> (usize, usize) {
    let first = index * CHUNK;
    (
        first_sample(first).max(0) as usize,
        (first_sample(first + FADE - 1) + FFT as isize) as usize,
    )
}

/// One chunk: its own stretch of the sound decoded, separated, and laid
/// back out as samples.
fn chunk(
    source: &Path,
    stream: Option<usize>,
    dsp: &Dsp,
    survey: &Survey,
    nets: &mut Networks,
    index: usize,
) -> Result<ChunkOut, String> {
    let chunks = survey.frames.div_ceil(CHUNK);
    let last_chunk = index + 1 == chunks;
    let first = index * CHUNK;
    let last = (first + CHUNK).min(survey.frames);
    // The frames this chunk draws: its own, and the fade into the next.
    let owned_to = if last_chunk {
        survey.frames
    } else {
        (last + FADE).min(survey.frames)
    };
    let from = warm_start(index);
    // The networks run to the last drawn frame; the features two frames
    // past it, which the network reads ahead, and the deep filter two.
    let run_to = owned_to;
    let feature_to = (run_to + CONV_LOOKAHEAD).min(survey.frames);

    // The samples those frames cover: from the first's start to the end
    // of the last; before zero and past the end are silence.
    let start = first_sample(from);
    let end = first_sample(feature_to - 1) + FFT as isize;
    let decode_from = start.max(0) as usize;
    let decode_to = (end.max(0) as usize).min(survey.samples);
    let channels = survey.channels;
    let mut decoder = decoder(
        source,
        stream,
        channels,
        decode_from as f64 / f64::from(RATE),
        Some((decode_to - decode_from) as f64 / f64::from(RATE)),
    )?;
    let span = (end - start) as usize;
    let lead = (decode_from as isize - start) as usize;
    let mut sides = vec![vec![0.0f32; span]; channels];
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
    let mono: Vec<f32> = if channels == 1 {
        sides[0].clone()
    } else {
        sides[0]
            .iter()
            .zip(&sides[1])
            .map(|(l, r)| 0.5 * (l + r))
            .collect()
    };

    // Spectra: the mix the networks hear, and each channel's.
    let count = feature_to - from;
    let frame_of = |signal: &[f32], frame: usize, out: &mut [Complex32]| {
        let offset = (first_sample(frame) - start) as usize;
        dsp.analyse(&signal[offset..offset + FFT], out);
    };
    let zero = Complex32::new(0.0, 0.0);
    let mut mix = vec![zero; count * BINS];
    for (i, out) in mix.chunks_exact_mut(BINS).enumerate() {
        frame_of(&mono, from + i, out);
    }
    let spectra: Vec<Vec<Complex32>> = if channels == 1 {
        vec![mix.clone()]
    } else {
        sides
            .iter()
            .map(|side| {
                let mut spectrum = vec![zero; count * BINS];
                for (i, out) in spectrum.chunks_exact_mut(BINS).enumerate() {
                    frame_of(side, from + i, out);
                }
                spectrum
            })
            .collect()
    };

    // The features, the means carried in from the survey; the network
    // reads each frame's two frames ahead.
    let mut norms = survey
        .norms
        .get(index)
        .cloned()
        .ok_or("voice isolation: the first pass missed a chunk")?;
    let mut erb_features = vec![[0.0f32; ERB_BANDS]; count];
    let mut low_features = vec![[zero; DF_BINS]; count];
    for i in 0..count {
        let (erb, low) = norms.features(dsp, &mix[i * BINS..(i + 1) * BINS]);
        erb_features[i] = erb;
        low_features[i] = low;
    }
    let frames = run_to - from;
    let mut erb_in = vec![0.0f32; frames * ERB_BANDS];
    let mut spec_in = vec![0.0f32; 2 * frames * DF_BINS];
    for i in 0..frames {
        let ahead = i + CONV_LOOKAHEAD;
        if ahead >= count {
            continue;
        }
        erb_in[i * ERB_BANDS..(i + 1) * ERB_BANDS].copy_from_slice(&erb_features[ahead]);
        for (f, bin) in low_features[ahead].iter().enumerate() {
            spec_in[i * DF_BINS + f] = bin.re;
            spec_in[(frames + i) * DF_BINS + f] = bin.im;
        }
    }
    let (gains, coefs) = nets.run(erb_in, spec_in, frames)?;

    // Each drawn frame's weight: rising over the fade in from the chunk
    // before, falling over the fade out into the next; the two always sum
    // to one.
    let weight = |t: usize| -> f32 {
        let mut w = 1.0;
        if index > 0 && t < first + FADE {
            w = (t - first + 1) as f32 / (FADE + 1) as f32;
        }
        if !last_chunk && t >= last {
            w = 1.0 - (t - last + 1) as f32 / (FADE + 1) as f32;
        }
        w
    };

    // The voice, each channel: the gains over every band, the low bins
    // remade by the deep filter from the five frames about them, weighed,
    // and overlap-added.
    let out_from = first_sample(first).max(0) as usize;
    let out_to = ((first_sample(owned_to - 1) + FFT as isize) as usize).min(survey.samples);
    let length = out_to - out_from;
    let mut voice = vec![0.0f32; length * channels];
    let mut row = vec![zero; BINS];
    let mut samples = vec![0.0f32; FFT];
    for (c, spectrum) in spectra.iter().enumerate() {
        for t in first..owned_to {
            let i = t - from;
            row.copy_from_slice(&spectrum[i * BINS..(i + 1) * BINS]);
            let mut bin = 0;
            for (band, &width) in dsp.erb.iter().enumerate() {
                let gain = gains[i * ERB_BANDS + band];
                for value in &mut row[bin..bin + width] {
                    *value *= gain;
                }
                bin += width;
            }
            for f in 0..DF_BINS {
                let mut sum = zero;
                for k in 0..DF_ORDER {
                    let source = t as isize + k as isize - (DF_ORDER - 1 - DF_LOOKAHEAD) as isize;
                    if source < from as isize || source >= (from + count) as isize {
                        continue;
                    }
                    let j = source as usize - from;
                    let at = ((i * DF_BINS + f) * DF_ORDER + k) * 2;
                    sum += Complex32::new(coefs[at], coefs[at + 1]) * spectrum[j * BINS + f];
                }
                row[f] = sum;
            }
            dsp.synthesise(&row, &mut samples);
            let w = weight(t);
            let begin = first_sample(t);
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

    // Cut into the seams it shares and the body it does not.
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
        seam_span(index).1.min(out_to)
    } else {
        out_from
    };
    let tail_start = if last_chunk {
        out_to
    } else {
        seam_span(index + 1).0.min(out_to)
    };
    Ok(ChunkOut {
        index,
        head: (index > 0).then(|| cut(out_from, head_end)),
        tail: (!last_chunk).then(|| cut(tail_start, out_to)),
        body: cut(head_end, tail_start),
    })
}

/// The two stems, and the seams waiting for their second half.
pub(crate) struct Writer {
    pub(crate) voice: Wav,
    pub(crate) rest: Wav,
    pub(crate) seams: std::collections::HashMap<usize, Piece>,
}

impl Writer {
    fn put(&mut self, piece: &Piece) -> Result<(), String> {
        let rest: Vec<f32> = piece
            .original
            .iter()
            .zip(&piece.voice)
            .map(|(original, voice)| original - voice)
            .collect();
        self.voice.write_at(piece.at, &piece.voice)?;
        self.rest.write_at(piece.at, &rest)
    }

    /// A seam's half: written with the other half added when that is
    /// here already, kept until it is otherwise.
    fn meet(&mut self, seam: usize, piece: Piece) -> Result<(), String> {
        match self.seams.remove(&seam) {
            Some(other) => {
                if other.at != piece.at || other.voice.len() != piece.voice.len() {
                    return Err("voice isolation: a seam's halves do not line up".to_owned());
                }
                let voice = other
                    .voice
                    .iter()
                    .zip(&piece.voice)
                    .map(|(a, b)| a + b)
                    .collect();
                self.put(&Piece {
                    at: piece.at,
                    voice,
                    original: piece.original,
                })
            }
            None => {
                self.seams.insert(seam, piece);
                Ok(())
            }
        }
    }

    pub(crate) fn take(&mut self, out: ChunkOut) -> Result<(), String> {
        self.put(&out.body)?;
        if let Some(head) = out.head {
            self.meet(out.index, head)?;
        }
        if let Some(tail) = out.tail {
            self.meet(out.index + 1, tail)?;
        }
        Ok(())
    }
}

// ── the files ───────────────────────────────────────────────────────────

/// A 16-bit PCM WAV of known length, written in pieces in any order.
pub(crate) struct Wav {
    file: std::fs::File,
    path: PathBuf,
    channels: usize,
}

impl Wav {
    pub(crate) fn create(
        path: &Path,
        rate: u32,
        channels: usize,
        samples: usize,
    ) -> Result<Wav, String> {
        let fail = |error: std::io::Error| format!("could not write {}: {error}", path.display());
        let mut file = std::fs::File::create(path).map_err(fail)?;
        let data = (samples * channels * 2) as u32;
        let block = (channels * 2) as u16;
        let mut header = Vec::with_capacity(44);
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&(36 + data).to_le_bytes());
        header.extend_from_slice(b"WAVEfmt ");
        header.extend_from_slice(&16u32.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&(channels as u16).to_le_bytes());
        header.extend_from_slice(&rate.to_le_bytes());
        header.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
        header.extend_from_slice(&block.to_le_bytes());
        header.extend_from_slice(&16u16.to_le_bytes());
        header.extend_from_slice(b"data");
        header.extend_from_slice(&data.to_le_bytes());
        file.write_all(&header).map_err(fail)?;
        // The whole length up front, so a piece past the end is a seek.
        file.set_len(44 + u64::from(data)).map_err(fail)?;
        Ok(Wav {
            file,
            path: path.to_path_buf(),
            channels,
        })
    }

    /// `interleaved` at sample `at`, clipped to 16 bits with dither left
    /// out: the stems are for playing, and their sum is the recording to
    /// within one step.
    pub(crate) fn write_at(&mut self, at: usize, interleaved: &[f32]) -> Result<(), String> {
        let fail =
            |error: std::io::Error| format!("could not write {}: {error}", self.path.display());
        let bytes: Vec<u8> = interleaved
            .iter()
            .flat_map(|sample| ((sample.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes())
            .collect();
        self.file
            .seek(SeekFrom::Start(44 + (at * self.channels * 2) as u64))
            .map_err(fail)?;
        self.file.write_all(&bytes).map_err(fail)
    }

    pub(crate) fn finish(self) -> Result<(), String> {
        self.file
            .sync_all()
            .map_err(|error| format!("could not write {}: {error}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bands_cover_the_spectrum_two_bins_at_least() {
        let bands = erb_bands();
        assert_eq!(bands.iter().sum::<usize>(), BINS);
        assert!(bands.iter().all(|&count| count >= 2), "{bands:?}");
    }

    /// A frame analysed and synthesised, overlap-added, is the signal: the
    /// window and both scales are each other's inverse.
    #[test]
    fn the_stft_gives_back_what_it_was_given() {
        let dsp = Dsp::new();
        let signal: Vec<f32> = (0..HOP * 12)
            .map(|i| ((i as f32) * 0.013).sin() * 0.5 + ((i as f32) * 0.21).cos() * 0.2)
            .collect();
        let mut out = vec![0.0f32; signal.len()];
        let mut spectrum = vec![Complex32::new(0.0, 0.0); BINS];
        let mut frame = vec![0.0f32; FFT];
        let mut samples = vec![0.0f32; FFT];
        for t in 0..signal.len() / HOP + 2 {
            for (k, value) in frame.iter_mut().enumerate() {
                let s = first_sample(t) + k as isize;
                *value = if s >= 0 && (s as usize) < signal.len() {
                    signal[s as usize]
                } else {
                    0.0
                };
            }
            dsp.analyse(&frame, &mut spectrum);
            dsp.synthesise(&spectrum, &mut samples);
            for (k, value) in samples.iter().enumerate() {
                let s = first_sample(t) + k as isize;
                if s >= 0 && (s as usize) < out.len() {
                    out[s as usize] += value;
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

    /// The real thing on a real voice under a tone and noise: the voice
    /// comes out cleaner than it went in, and voice and rest add up to the
    /// recording.
    #[test]
    fn a_voice_comes_out_of_a_mix_and_the_stems_add_back_up() {
        let dir = std::env::temp_dir().join(format!("concat-isolate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let speech_file = concat!(env!("CARGO_MANIFEST_DIR"), "/voices/mark-f-smith.flac");
        let speech = decoder(Path::new(speech_file), None, 1, 0.0, None)
            .and_then(|mut d| d.collect_f32().map_err(|e| e.to_string()))
            .expect("the voice decodes");
        let mut seed = 1u32;
        let mix: Vec<f32> = speech
            .iter()
            .enumerate()
            .map(|(i, s)| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (seed >> 9) as f32 / (1u32 << 23) as f32 - 0.5;
                let tone = (i as f32 * 2.0 * std::f32::consts::PI * 220.0 / RATE as f32).sin();
                s + 0.12 * tone + 0.15 * noise
            })
            .collect();
        let mixed = dir.join("mix.wav");
        let mut wav = Wav::create(&mixed, RATE, 1, mix.len()).expect("a file");
        wav.write_at(0, &mix).expect("written");
        wav.finish().expect("finished");
        let stems = Stems {
            voice: dir.join("voice.wav"),
            rest: dir.join("rest.wav"),
        };
        isolate(
            &mixed,
            None,
            &stems,
            2,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .expect("isolates");
        let read = |path: &Path| -> Vec<f32> {
            // Mono, as written: read as stereo it would come back three
            // decibels down a side.
            decoder(path, None, 1, 0.0, None)
                .and_then(|mut d| d.collect_f32().map_err(|e| e.to_string()))
                .expect("decodes")
        };
        let (voice, rest) = (read(&stems.voice), read(&stems.rest));
        let length = speech.len().min(voice.len()).min(rest.len());
        let snr = |reference: &[f32], estimate: &[f32]| {
            let signal: f64 = reference.iter().map(|x| f64::from(*x).powi(2)).sum();
            let error: f64 = reference
                .iter()
                .zip(estimate)
                .map(|(a, b)| f64::from(a - b).powi(2))
                .sum();
            10.0 * (signal / error).log10()
        };
        let before = snr(&speech[..length], &mix[..length]);
        let after = snr(&speech[..length], &voice[..length]);
        let sum: Vec<f32> = voice.iter().zip(&rest).map(|(a, b)| a + b).collect();
        let whole = snr(&mix[..length], &sum[..length]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            after > before + 5.0,
            "voice {before:.1} dB in, {after:.1} dB out"
        );
        assert!(
            whole > 70.0,
            "voice + rest is {whole:.1} dB from the recording"
        );
    }
}

#[cfg(test)]
mod probe {
    /// Runs the engine on `CONCAT_ISOLATE_IN`, for comparing against a
    /// reference by hand. Ignored: it needs a file.
    #[test]
    #[ignore]
    fn isolate_a_file_by_hand() {
        let input = std::env::var("CONCAT_ISOLATE_IN").expect("CONCAT_ISOLATE_IN");
        let stems = super::Stems {
            voice: std::path::PathBuf::from(format!("{input}.voice.wav")),
            rest: std::path::PathBuf::from(format!("{input}.rest.wav")),
        };
        let started = std::time::Instant::now();
        let workers = super::workers_for(Some(16 << 30));
        super::isolate(
            std::path::Path::new(&input),
            None,
            &stems,
            workers,
            &Default::default(),
            &mut |_| {},
        )
        .expect("isolates");
        eprintln!("isolated in {:?} on {workers} workers", started.elapsed());
    }
}

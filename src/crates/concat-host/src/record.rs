// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Recording from the microphone: a voiceover, straight into a WAV file in
//! the project's audio folder.
//!
//! The microphone is opened when the voiceover panel opens and held until
//! it closes, so the meter shows the room before a word is spoken and a
//! take begins the instant Record is pressed, not after a device has woken
//! up. One thread owns the input stream from open to close (a cpal stream
//! may not cross threads) and writes the take. The callback only mixes
//! each buffer down to mono, measures it for the meter and, while a take is
//! being written, passes it on as 16-bit samples, so the device is never
//! kept waiting on the disk. A paused take is the same take with nothing
//! passed on: the pause is cut out of the file, and the picture, paused
//! with it, stays in step.
//!
//! The file is good on disk as it goes: its header is brought up to date
//! about once a second, so a take cut short by a crash or a pulled plug is
//! a WAV that plays up to the last second, not a file nothing opens.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// A sample at or past this is a sample the converter could not hold: the
/// meter's clip light.
const CLIP: f32 = 0.999;

/// The microphones the system offers, by the names it gives them, the
/// default first when it says which that is. Empty when there are none or
/// the system will not list them.
pub fn microphones() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::default_host();
    let default = host
        .default_input_device()
        .and_then(|device| device.name().ok());
    let mut names: Vec<String> = host
        .input_devices()
        .map(|devices| devices.filter_map(|device| device.name().ok()).collect())
        .unwrap_or_default();
    names.dedup();
    if let Some(default) = default {
        names.retain(|name| *name != default);
        names.insert(0, default);
    }
    names
}

/// The microphone, open. Dropping it closes the device, finishing any take
/// still being written so what was heard is kept.
pub struct Microphone {
    shared: Arc<Shared>,
    events: mpsc::Sender<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
    name: String,
    /// Samples a second, the device's own.
    rate: u32,
}

/// A finished take.
#[derive(Clone, Debug)]
pub struct Take {
    /// The WAV file, 16-bit mono at the device's own rate.
    pub path: PathBuf,
    /// Its length, in seconds: the time spent recording, pauses left out.
    pub seconds: f64,
    /// When the microphone heard the take's first sample, as near as the
    /// device says: the callback's time less the latency it reports. None
    /// when no sound arrived at all.
    pub first: Option<Instant>,
}

/// What the meter shows: the loudest sample and the average power since it
/// last looked, both `0..=1`, and whether anything reached the top.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Level {
    /// The loudest sample, held to the top of the bar.
    pub peak: f32,
    /// The root of the mean square: how loud it sounded, near enough.
    pub rms: f32,
    /// A sample reached the converter's limit: the clip light.
    pub clipped: bool,
}

/// What the callback, the writer and the window share.
struct Shared {
    /// A take is being written: set from begin to end, and clear while it
    /// is paused.
    writing: AtomicBool,
    /// The input gain, linear, as f32 bits.
    gain: AtomicU32,
    meter: Mutex<Meter>,
    /// When the take's first sample was heard. Cleared at begin.
    first: Mutex<Option<Instant>>,
    /// Samples in the take so far.
    written: AtomicU64,
    /// Anything but digital silence has arrived since the device opened.
    /// A system that keeps the microphone from an app tends to hand it
    /// zeros rather than an error, and this is how that is noticed.
    heard: AtomicBool,
    /// What went wrong with the device after it opened: unplugged, taken.
    failed: Mutex<Option<String>>,
}

impl Shared {
    fn new() -> Self {
        Shared {
            writing: AtomicBool::new(false),
            gain: AtomicU32::new(1.0f32.to_bits()),
            meter: Mutex::new(Meter::default()),
            first: Mutex::new(None),
            written: AtomicU64::new(0),
            heard: AtomicBool::new(false),
            failed: Mutex::new(None),
        }
    }

    /// One buffer from the device, already mono and gained: measured for
    /// the meter always, and handed back as 16-bit samples for the take
    /// only while one is being written. `heard` is when the buffer's first
    /// sample reached the microphone.
    fn capture(&self, mono: &[f32], heard: Option<Instant>) -> Option<Vec<i16>> {
        // The meter is the window's to read; a buffer that finds it busy
        // goes unmeasured rather than keeping the device waiting.
        if let Ok(mut meter) = self.meter.try_lock() {
            meter.add(mono);
        }
        if !self.heard.load(Ordering::Relaxed) && mono.iter().any(|sample| *sample != 0.0) {
            self.heard.store(true, Ordering::Relaxed);
        }
        if !self.writing.load(Ordering::Relaxed) {
            return None;
        }
        if let Ok(mut first) = self.first.try_lock()
            && first.is_none()
        {
            *first = heard;
        }
        Some(to_pcm(mono))
    }
}

/// What the writer thread is told.
enum Event {
    /// Samples for the take.
    Samples(Vec<i16>),
    /// Start a take in this file.
    Begin(PathBuf, mpsc::Sender<Result<(), String>>),
    /// Finish the take.
    End(mpsc::Sender<Result<Take, String>>),
    /// Close the device.
    Close,
}

impl Microphone {
    /// Opens the microphone the system calls `name`, or its default one
    /// when there is no such microphone or no name is given. Returns once
    /// the device is open and running, or with why it would not open: no
    /// microphone, or one the system will not hand over.
    pub fn open(name: Option<&str>) -> Result<Microphone, String> {
        let shared = Arc::new(Shared::new());
        let (events, inbox) = mpsc::channel::<Event>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(String, u32), String>>();
        let wanted = name.map(str::to_owned);
        let thread = {
            let shared = Arc::clone(&shared);
            let events = events.clone();
            std::thread::Builder::new()
                .name("audio-input".into())
                .spawn(move || run(wanted.as_deref(), &shared, events, &inbox, &ready_tx))
                .map_err(|error| format!("could not spawn the recording thread: {error}"))?
        };
        match ready_rx.recv() {
            Ok(Ok((name, rate))) => Ok(Microphone {
                shared,
                events,
                thread: Some(thread),
                name,
                rate,
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err("the recording thread stopped before the microphone opened".to_owned())
            }
        }
    }

    /// The name of the microphone that opened.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Starts a take in a new file at `path`, written from the next buffer
    /// on.
    pub fn begin(&self, path: PathBuf) -> Result<(), String> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)
                .map_err(|error| format!("could not make {}: {error}", folder.display()))?;
        }
        self.shared.writing.store(false, Ordering::Relaxed);
        *lock(&self.shared.first) = None;
        self.shared.written.store(0, Ordering::Relaxed);
        let (reply, answer) = mpsc::channel();
        self.events
            .send(Event::Begin(path, reply))
            .map_err(|_| "the microphone has closed".to_owned())?;
        answer
            .recv()
            .map_err(|_| "the microphone has closed".to_owned())??;
        self.shared.writing.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Holds the take: what is heard meanwhile is measured, not kept.
    pub fn pause(&self) {
        self.shared.writing.store(false, Ordering::Relaxed);
    }

    /// Carries the take on from where it was held.
    pub fn resume(&self) {
        self.shared.writing.store(true, Ordering::Relaxed);
    }

    /// Finishes the take and its file.
    pub fn end(&self) -> Result<Take, String> {
        self.shared.writing.store(false, Ordering::Relaxed);
        let (reply, answer) = mpsc::channel();
        self.events
            .send(Event::End(reply))
            .map_err(|_| "the microphone has closed".to_owned())?;
        answer
            .recv()
            .map_err(|_| "the microphone has closed".to_owned())?
    }

    /// What was heard since the last look: the meter's feed.
    pub fn level(&self) -> Level {
        lock(&self.shared.meter).take()
    }

    /// How long the take is so far, in seconds, pauses left out.
    pub fn seconds(&self) -> f64 {
        self.shared.written.load(Ordering::Relaxed) as f64 / f64::from(self.rate.max(1))
    }

    /// Whether anything but silence has come from the device yet.
    pub fn heard(&self) -> bool {
        self.shared.heard.load(Ordering::Relaxed)
    }

    /// The input gain, linear: what every sample is multiplied by before it
    /// is measured or kept.
    pub fn set_gain(&self, gain: f32) {
        self.shared
            .gain
            .store(gain.clamp(0.0, 16.0).to_bits(), Ordering::Relaxed);
    }

    /// What went wrong with the device after it opened, if anything has.
    pub fn failed(&self) -> Option<String> {
        lock(&self.shared.failed).clone()
    }
}

impl Drop for Microphone {
    fn drop(&mut self) {
        let _ = self.events.send(Event::Close);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The recording thread: opens the device, says so, and writes takes until
/// told to close; a take still open then is finished, not lost.
fn run(
    name: Option<&str>,
    shared: &Arc<Shared>,
    events: mpsc::Sender<Event>,
    inbox: &mpsc::Receiver<Event>,
    ready: &mpsc::Sender<Result<(String, u32), String>>,
) {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let opened = (|| -> Result<_, String> {
        let host = cpal::default_host();
        let named = name.and_then(|name| {
            host.input_devices()
                .ok()?
                .find(|device| device.name().is_ok_and(|found| found == name))
        });
        let device = match named {
            Some(device) => device,
            None => host
                .default_input_device()
                .ok_or_else(|| "no microphone was found".to_owned())?,
        };
        let supported = device
            .default_input_config()
            .map_err(|error| format!("the microphone would not say how it records: {error}"))?;
        let config: cpal::StreamConfig = supported.config();
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => input::<f32>(&device, &config, events, shared),
            cpal::SampleFormat::I16 => input::<i16>(&device, &config, events, shared),
            cpal::SampleFormat::U16 => input::<u16>(&device, &config, events, shared),
            cpal::SampleFormat::I32 => input::<i32>(&device, &config, events, shared),
            other => Err(format!(
                "the microphone records {other:?} samples, which Concat does not read"
            )),
        }?;
        stream
            .play()
            .map_err(|error| format!("the microphone would not start: {error}"))?;
        let name = device.name().unwrap_or_default();
        Ok((stream, config.sample_rate.0, name))
    })();

    let (stream, rate) = match opened {
        Ok((stream, rate, name)) => {
            let _ = ready.send(Ok((name, rate)));
            (stream, rate)
        }
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };

    let mut takes = Takes::new(rate, Arc::clone(shared));
    loop {
        match inbox.recv_timeout(Duration::from_millis(250)) {
            Ok(Event::Close) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(event) => takes.handle(event),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    // Closed first, so nothing more is sent; then whatever was already on
    // its way is written, and a take left open is finished.
    shared.writing.store(false, Ordering::Relaxed);
    drop(stream);
    while let Ok(event) = inbox.try_recv() {
        if let Event::Samples(_) = event {
            takes.handle(event);
        }
    }
    if let Some(Ok(take)) = takes.finish() {
        log::info!(
            "voiceover: closed mid-take, kept {:.2}s in {}",
            take.seconds,
            take.path.display()
        );
    }
}

/// The writer's side of the takes: one file at a time, written as samples
/// arrive and finished on End. Apart from the device, so a take can be
/// assembled without one.
struct Takes {
    rate: u32,
    shared: Arc<Shared>,
    open: Option<(PathBuf, WavWriter)>,
    /// Why the open take's file stopped being written, if it did.
    broken: Option<String>,
}

impl Takes {
    fn new(rate: u32, shared: Arc<Shared>) -> Self {
        Takes {
            rate,
            shared,
            open: None,
            broken: None,
        }
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Samples(samples) => {
                let Some((_, writer)) = self.open.as_mut() else {
                    return;
                };
                if self.broken.is_some() {
                    return;
                }
                match writer.write(&samples) {
                    Ok(()) => {
                        self.shared.written.store(writer.samples, Ordering::Relaxed);
                    }
                    Err(error) => {
                        log::warn!("voiceover: {error}");
                        self.broken = Some(error);
                    }
                }
            }
            Event::Begin(path, reply) => {
                // A take still open is finished first: two files are never
                // written at once.
                let _ = self.finish();
                let result = WavWriter::create(&path, self.rate).map(|writer| {
                    self.open = Some((path, writer));
                    self.broken = None;
                });
                let _ = reply.send(result);
            }
            Event::End(reply) => {
                let result = self
                    .finish()
                    .unwrap_or_else(|| Err("no take was being recorded".to_owned()));
                let _ = reply.send(result);
            }
            Event::Close => {}
        }
    }

    /// Finishes the open take, if there is one.
    fn finish(&mut self) -> Option<Result<Take, String>> {
        let (path, writer) = self.open.take()?;
        let finished = writer.finish();
        if let Some(error) = self.broken.take() {
            return Some(Err(error));
        }
        let seconds = match finished {
            Ok(seconds) => seconds,
            Err(error) => return Some(Err(error)),
        };
        let first = *lock(&self.shared.first);
        if first.is_none() {
            let why = lock(&self.shared.failed).clone();
            return Some(Err(
                why.unwrap_or_else(|| "the microphone sent no sound".to_owned())
            ));
        }
        Some(Ok(Take {
            path,
            seconds,
            first,
        }))
    }
}

/// The input stream for one sample format: each buffer mixed down to mono
/// at the input gain, measured, and sent to the writer while a take is
/// being written.
fn input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    events: mpsc::Sender<Event>,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    use cpal::traits::DeviceTrait;

    let channels = usize::from(config.channels.max(1));
    let shared = Arc::clone(shared);
    let failed = Arc::clone(&shared);
    device
        .build_input_stream(
            config,
            move |data: &[T], info: &cpal::InputCallbackInfo| {
                let stamp = info.timestamp();
                let latency = stamp
                    .callback
                    .duration_since(&stamp.capture)
                    .unwrap_or_default();
                let heard = Instant::now().checked_sub(latency);
                let gain = f32::from_bits(shared.gain.load(Ordering::Relaxed));
                let mono = mix_down(data.iter().map(|s| s.to_sample::<f32>()), channels, gain);
                if let Some(samples) = shared.capture(&mono, heard) {
                    let _ = events.send(Event::Samples(samples));
                }
            },
            move |error| {
                log::warn!("microphone: {error}");
                lock(&failed.failed)
                    .get_or_insert_with(|| format!("the microphone failed: {error}"));
            },
            None,
        )
        .map_err(|error| format!("the microphone would not open: {error}"))
}

/// Interleaved samples of `channels` channels as mono - the mean of each
/// frame's channels - times `gain`. A trailing part frame is dropped.
/// Unclamped: the meter wants to see a sample that went over.
fn mix_down(samples: impl Iterator<Item = f32>, channels: usize, gain: f32) -> Vec<f32> {
    let channels = channels.max(1);
    let mut mono = Vec::new();
    let mut sum = 0.0f32;
    let mut count = 0;
    for sample in samples {
        sum += sample;
        count += 1;
        if count == channels {
            mono.push(sum / channels as f32 * gain);
            sum = 0.0;
            count = 0;
        }
    }
    mono
}

/// Mono samples as 16-bit, held to the range.
fn to_pcm(mono: &[f32]) -> Vec<i16> {
    mono.iter()
        .map(|sample| (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16)
        .collect()
}

/// The meter's arithmetic: what has been heard since it was last read.
#[derive(Default)]
struct Meter {
    peak: f32,
    squares: f64,
    count: u64,
    clipped: bool,
}

impl Meter {
    fn add(&mut self, mono: &[f32]) {
        for sample in mono {
            let size = sample.abs();
            self.peak = self.peak.max(size);
            self.clipped |= size >= CLIP;
            self.squares += f64::from(*sample) * f64::from(*sample);
        }
        self.count += mono.len() as u64;
    }

    /// The reading, and a fresh start for the next one.
    fn take(&mut self) -> Level {
        let rms = if self.count == 0 {
            0.0
        } else {
            (self.squares / self.count as f64).sqrt() as f32
        };
        let level = Level {
            peak: self.peak.min(1.0),
            rms: rms.min(1.0),
            clipped: self.clipped,
        };
        *self = Meter::default();
        level
    }
}

/// A 16-bit mono WAV written as it goes: the header up front, its sizes
/// brought up to date every second or so and for good by
/// [`WavWriter::finish`].
struct WavWriter {
    out: BufWriter<File>,
    rate: u32,
    samples: u64,
    /// Samples written when the header was last brought up to date.
    noted: u64,
}

impl WavWriter {
    fn create(path: &Path, rate: u32) -> Result<Self, String> {
        let file = File::create(path)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        let mut writer = WavWriter {
            out: BufWriter::new(file),
            rate,
            samples: 0,
            noted: 0,
        };
        writer.header(0)?;
        Ok(writer)
    }

    fn header(&mut self, data_bytes: u32) -> Result<(), String> {
        let io = |error: std::io::Error| format!("could not write the recording: {error}");
        let out = &mut self.out;
        out.write_all(b"RIFF").map_err(io)?;
        out.write_all(&(36 + data_bytes).to_le_bytes())
            .map_err(io)?;
        out.write_all(b"WAVEfmt ").map_err(io)?;
        out.write_all(&16u32.to_le_bytes()).map_err(io)?;
        out.write_all(&1u16.to_le_bytes()).map_err(io)?; // PCM
        out.write_all(&1u16.to_le_bytes()).map_err(io)?; // mono
        out.write_all(&self.rate.to_le_bytes()).map_err(io)?;
        out.write_all(&(self.rate * 2).to_le_bytes()).map_err(io)?;
        out.write_all(&2u16.to_le_bytes()).map_err(io)?; // block align
        out.write_all(&16u16.to_le_bytes()).map_err(io)?;
        out.write_all(b"data").map_err(io)?;
        out.write_all(&data_bytes.to_le_bytes()).map_err(io)
    }

    fn write(&mut self, samples: &[i16]) -> Result<(), String> {
        for sample in samples {
            self.out
                .write_all(&sample.to_le_bytes())
                .map_err(|error| format!("could not write the recording: {error}"))?;
        }
        self.samples += samples.len() as u64;
        if self.samples - self.noted >= u64::from(self.rate) {
            self.checkpoint()?;
        }
        Ok(())
    }

    /// The sizes the header should say for what is written so far. A WAV's
    /// sizes are 32-bit: past about thirteen hours of mono at 48 kHz the
    /// header would wrap, so it says as much as it can.
    fn data_bytes(&self) -> u32 {
        u32::try_from(self.samples * 2).unwrap_or(u32::MAX - 36)
    }

    /// Writes what is buffered and brings the header's two sizes up to
    /// date, then carries on at the end: the file on disk is a whole WAV
    /// from here on, whatever happens to the process.
    fn checkpoint(&mut self) -> Result<(), String> {
        let io = |error: std::io::Error| format!("could not write the recording: {error}");
        let data_bytes = self.data_bytes();
        self.out.seek(SeekFrom::Start(4)).map_err(io)?;
        self.out
            .write_all(&(36 + data_bytes).to_le_bytes())
            .map_err(io)?;
        self.out.seek(SeekFrom::Start(40)).map_err(io)?;
        self.out.write_all(&data_bytes.to_le_bytes()).map_err(io)?;
        self.out.seek(SeekFrom::End(0)).map_err(io)?;
        self.out.flush().map_err(io)?;
        self.noted = self.samples;
        Ok(())
    }

    /// Fills in the sizes and closes the file; returns its length in seconds.
    fn finish(mut self) -> Result<f64, String> {
        self.checkpoint()
            .map_err(|error| format!("could not finish the recording: {error}"))?;
        Ok(self.samples as f64 / f64::from(self.rate))
    }
}

/// Where a take spoken from `playhead` lands on the timeline: shifted by how
/// long after the picture started rolling the microphone heard its first
/// sample, so the words sit under the frames they were spoken to. Never
/// before zero.
pub fn landing(playhead: f64, rolled: Instant, first: Option<Instant>) -> f64 {
    let offset = match first {
        Some(first) if first >= rolled => first.duration_since(rolled).as_secs_f64(),
        Some(first) => -rolled.duration_since(first).as_secs_f64(),
        None => 0.0,
    };
    (playhead + offset).max(0.0)
}

/// A gain in decibels as the factor samples are multiplied by.
pub fn gain_of(decibels: f32) -> f32 {
    10f32.powf(decibels / 20.0)
}

/// A level `0..=1` as the meter draws it: decibels from -60 to 0 spread
/// evenly along the bar, so a voice at a sensible level fills most of it
/// rather than its first tenth.
pub fn meter_position(level: f32) -> f32 {
    if level <= 0.001 {
        return 0.0;
    }
    ((20.0 * level.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("concat-record-{name}-{}.wav", std::process::id()))
    }

    #[test]
    fn stereo_mixes_down_to_the_mean_at_the_gain() {
        let mono = mix_down([0.5, 0.5, -1.0, 0.0, 0.25].into_iter(), 2, 1.0);
        // The fifth sample is half a frame and is not written.
        assert_eq!(mono, vec![0.5, -0.5]);
        assert_eq!(to_pcm(&mono), vec![16384, -16384]);
        let louder = mix_down([0.25, 0.25].into_iter(), 2, 2.0);
        assert_eq!(louder, vec![0.5]);
        // Over the top is held there in the file.
        assert_eq!(to_pcm(&[1.5, -3.0]), vec![i16::MAX, -i16::MAX]);
    }

    #[test]
    fn the_meter_reads_peak_power_and_clipping_then_starts_again() {
        let mut meter = Meter::default();
        meter.add(&[0.5, -0.5, 0.5, -0.5]);
        let level = meter.take();
        assert!((level.peak - 0.5).abs() < 1e-6);
        assert!((level.rms - 0.5).abs() < 1e-6);
        assert!(!level.clipped);

        meter.add(&[0.0, 1.2]);
        let level = meter.take();
        assert_eq!(level.peak, 1.0, "held to the top of the bar");
        assert!(level.clipped);
        assert_eq!(meter.take(), Level::default(), "read once, then fresh");
    }

    #[test]
    fn the_meter_spreads_decibels_along_the_bar() {
        assert_eq!(meter_position(0.0), 0.0);
        assert!((meter_position(1.0) - 1.0).abs() < 1e-6);
        // -20 dB is two thirds of the way along.
        assert!((meter_position(0.1) - 2.0 / 3.0).abs() < 1e-4);
        assert!((gain_of(6.0) - 1.995).abs() < 1e-3);
        assert_eq!(gain_of(0.0), 1.0);
    }

    #[test]
    fn a_wav_says_its_own_size_once_finished() {
        let path = temp("finished");
        let mut writer = WavWriter::create(&path, 48_000).expect("creates");
        writer.write(&[1, -1, 2, -2]).expect("writes");
        writer.write(&[3]).expect("writes");
        let seconds = writer.finish().expect("finishes");
        let bytes = std::fs::read(&path).expect("reads");
        let _ = std::fs::remove_file(&path);
        assert_eq!(bytes.len(), 44 + 10);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 36 + 10);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            48_000
        );
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 10);
        assert_eq!(i16::from_le_bytes([bytes[44], bytes[45]]), 1);
        assert!((seconds - 5.0 / 48_000.0).abs() < 1e-12);
    }

    #[test]
    fn a_wav_still_being_written_is_whole_on_disk_every_second() {
        let path = temp("checkpoint");
        let mut writer = WavWriter::create(&path, 4).expect("creates");
        // Under a second at 4 Hz: nothing noted yet.
        writer.write(&[1, 2, 3]).expect("writes");
        // The fourth sample is a second's worth; the header catches up
        // without the file being finished, as a crash would leave it.
        writer.write(&[4, 5]).expect("writes");
        let bytes = std::fs::read(&path).expect("reads");
        assert_eq!(bytes.len(), 44 + 10);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 36 + 10);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 10);
        // And writing carries on at the end, not over the header.
        writer.write(&[6]).expect("writes");
        writer.finish().expect("finishes");
        let bytes = std::fs::read(&path).expect("reads");
        let _ = std::fs::remove_file(&path);
        assert_eq!(bytes.len(), 44 + 12);
        assert_eq!(i16::from_le_bytes([bytes[54], bytes[55]]), 6);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 12);
    }

    #[test]
    fn a_paused_take_is_one_file_with_the_pause_cut_out() {
        let shared = Arc::new(Shared::new());
        let mut takes = Takes::new(8, Arc::clone(&shared));
        let path = temp("paused");
        let (reply, answer) = mpsc::channel();
        takes.handle(Event::Begin(path.clone(), reply));
        answer.recv().expect("answered").expect("begins");

        // Before the take is written to, a buffer is only measured.
        assert_eq!(shared.capture(&[0.9], None), None);
        shared.writing.store(true, Ordering::Relaxed);
        let start = Instant::now();
        let spoken = shared.capture(&[0.5, 0.5], Some(start)).expect("kept");
        takes.handle(Event::Samples(spoken));
        // Paused: heard, measured, not kept.
        shared.writing.store(false, Ordering::Relaxed);
        assert_eq!(shared.capture(&[0.25, 0.25, 0.25], None), None);
        // Resumed: the take carries on in the same file, and still began
        // when its first sample was heard.
        shared.writing.store(true, Ordering::Relaxed);
        let later = start + Duration::from_secs(5);
        let more = shared.capture(&[-0.5], Some(later)).expect("kept");
        takes.handle(Event::Samples(more));
        assert_eq!(shared.written.load(Ordering::Relaxed), 3);

        let (reply, answer) = mpsc::channel();
        takes.handle(Event::End(reply));
        let take = answer.recv().expect("answered").expect("a take");
        let bytes = std::fs::read(&path).expect("reads");
        let _ = std::fs::remove_file(&path);
        assert_eq!(take.first, Some(start));
        assert!((take.seconds - 3.0 / 8.0).abs() < 1e-12);
        assert_eq!(bytes.len(), 44 + 6);
        let samples: Vec<i16> = bytes[44..]
            .chunks(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(samples, vec![16384, 16384, -16384]);
        // The pause was still on the meter.
        assert!((lock(&shared.meter).take().peak - 0.9).abs() < 1e-6);
        assert!(shared.heard.load(Ordering::Relaxed));
    }

    #[test]
    fn a_take_that_heard_nothing_says_so() {
        let shared = Arc::new(Shared::new());
        let mut takes = Takes::new(8, Arc::clone(&shared));
        let path = temp("silent");
        let (reply, answer) = mpsc::channel();
        takes.handle(Event::Begin(path.clone(), reply));
        answer.recv().expect("answered").expect("begins");
        let (reply, answer) = mpsc::channel();
        takes.handle(Event::End(reply));
        let result = answer.recv().expect("answered");
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err());
        let (reply, answer) = mpsc::channel();
        takes.handle(Event::End(reply));
        assert!(answer.recv().expect("answered").is_err(), "nothing open");
    }

    #[test]
    fn a_take_lands_where_its_first_sample_was_heard() {
        let rolled = Instant::now();
        let late = rolled + Duration::from_millis(120);
        assert!((landing(10.0, rolled, Some(late)) - 10.12).abs() < 1e-9);
        let early = rolled
            .checked_sub(Duration::from_millis(30))
            .expect("not at boot");
        assert!((landing(10.0, rolled, Some(early)) - 9.97).abs() < 1e-9);
        assert_eq!(landing(0.01, rolled, Some(early)), 0.0, "never before zero");
        assert_eq!(landing(4.0, rolled, None), 4.0);
    }
}

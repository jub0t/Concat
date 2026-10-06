// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Recording from the microphone: a voiceover, straight into a WAV file in
//! the project's audio folder.
//!
//! One thread owns the input stream from open to close (a cpal stream may
//! not cross threads) and writes what the device hands it. The callback
//! only mixes each buffer down to mono, turns it into 16-bit samples and
//! passes it on, so the device is never kept waiting on the disk. What the
//! window reads while a take runs is the level for its meter; what it gets
//! at the end is the file, its length, and when its first sample was heard,
//! which is what lines the take up with the picture it was spoken over.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// A take in progress. Dropping it stops the take and keeps what was heard.
pub struct Recorder {
    stop: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
    thread: Option<std::thread::JoinHandle<Result<Take, String>>>,
}

/// A finished take.
#[derive(Clone, Debug)]
pub struct Take {
    /// The WAV file, 16-bit mono at the device's own rate.
    pub path: PathBuf,
    /// Its length, in seconds.
    pub seconds: f64,
    /// When the microphone heard the first sample, as near as the device
    /// says: the callback's time less the latency it reports. None when no
    /// sound arrived at all.
    pub first: Option<Instant>,
}

impl Recorder {
    /// Opens the default microphone and starts writing to `path`. Returns
    /// once the device is open and running, or with why it would not open:
    /// no microphone, or one the system will not hand over.
    pub fn start(path: PathBuf) -> Result<Recorder, String> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)
                .map_err(|error| format!("could not make {}: {error}", folder.display()))?;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let level = Arc::new(AtomicU32::new(0));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let thread = {
            let stop = Arc::clone(&stop);
            let level = Arc::clone(&level);
            std::thread::Builder::new()
                .name("audio-input".into())
                .spawn(move || record(&path, &stop, &level, &ready_tx))
                .map_err(|error| format!("could not spawn the recording thread: {error}"))?
        };
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Recorder {
                stop,
                level,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => Err(match thread.join() {
                Ok(Err(error)) => error,
                _ => "the recording thread stopped before the microphone opened".to_owned(),
            }),
        }
    }

    /// The loudest sample since the last look, `0..=1`: the meter's feed.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.swap(0, Ordering::Relaxed))
    }

    /// Stops the take and finishes the file.
    pub fn stop(mut self) -> Result<Take, String> {
        self.finish()
    }

    fn finish(&mut self) -> Result<Take, String> {
        self.stop.store(true, Ordering::Relaxed);
        let thread = self
            .thread
            .take()
            .ok_or_else(|| "the take was already stopped".to_owned())?;
        thread
            .join()
            .map_err(|_| "the recording thread panicked".to_owned())?
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.thread.is_some() {
            let _ = self.finish();
        }
    }
}

/// The recording thread: opens the device, says so, writes until told to
/// stop, then closes the stream and the file.
fn record(
    path: &Path,
    stop: &AtomicBool,
    level: &Arc<AtomicU32>,
    ready: &mpsc::Sender<Result<(), String>>,
) -> Result<Take, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let opened = (|| -> Result<_, String> {
        let device = cpal::default_host()
            .default_input_device()
            .ok_or_else(|| "no microphone was found".to_owned())?;
        let supported = device
            .default_input_config()
            .map_err(|error| format!("the microphone would not say how it records: {error}"))?;
        let config: cpal::StreamConfig = supported.config();
        let (tx, rx) = mpsc::channel::<Vec<i16>>();
        let first: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let failed: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => input::<f32>(&device, &config, tx, level, &first, &failed),
            cpal::SampleFormat::I16 => input::<i16>(&device, &config, tx, level, &first, &failed),
            cpal::SampleFormat::U16 => input::<u16>(&device, &config, tx, level, &first, &failed),
            cpal::SampleFormat::I32 => input::<i32>(&device, &config, tx, level, &first, &failed),
            other => Err(format!(
                "the microphone records {other:?} samples, which Concat does not read"
            )),
        }?;
        stream
            .play()
            .map_err(|error| format!("the microphone would not start: {error}"))?;
        let writer = WavWriter::create(path, config.sample_rate.0)?;
        Ok((stream, rx, writer, first, failed))
    })();

    let (stream, rx, mut writer, first, failed) = match opened {
        Ok(parts) => {
            let _ = ready.send(Ok(()));
            parts
        }
        Err(error) => {
            let _ = ready.send(Err(error.clone()));
            return Err(error);
        }
    };

    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(samples) => writer.write(&samples)?,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    // Closed first, so nothing more is sent; then whatever was already on
    // its way is written.
    drop(stream);
    while let Ok(samples) = rx.try_recv() {
        writer.write(&samples)?;
    }
    let seconds = writer.finish()?;
    let first = *first
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if first.is_none() {
        let why = failed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        return Err(why.unwrap_or_else(|| "the microphone sent no sound".to_owned()));
    }
    Ok(Take {
        path: path.to_owned(),
        seconds,
        first,
    })
}

/// The input stream for one sample format: each buffer mixed down to mono
/// 16-bit and sent to the writer, its loudest sample kept for the meter.
fn input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    tx: mpsc::Sender<Vec<i16>>,
    level: &Arc<AtomicU32>,
    first: &Arc<Mutex<Option<Instant>>>,
    failed: &Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    use cpal::traits::DeviceTrait;

    let channels = usize::from(config.channels.max(1));
    let level = Arc::clone(level);
    let first = Arc::clone(first);
    let failed = Arc::clone(failed);
    device
        .build_input_stream(
            config,
            move |data: &[T], info: &cpal::InputCallbackInfo| {
                if let Ok(mut first) = first.try_lock()
                    && first.is_none()
                {
                    let stamp = info.timestamp();
                    let latency = stamp
                        .callback
                        .duration_since(&stamp.capture)
                        .unwrap_or_default();
                    *first = Instant::now().checked_sub(latency);
                }
                let (mono, peak) = mix_down(data.iter().map(|s| s.to_sample::<f32>()), channels);
                let held = f32::from_bits(level.load(Ordering::Relaxed));
                if peak > held {
                    level.store(peak.to_bits(), Ordering::Relaxed);
                }
                let _ = tx.send(mono);
            },
            move |error| {
                log::warn!("microphone: {error}");
                if let Ok(mut failed) = failed.lock() {
                    failed.get_or_insert_with(|| format!("the microphone failed: {error}"));
                }
            },
            None,
        )
        .map_err(|error| format!("the microphone would not open: {error}"))
}

/// Interleaved samples of `channels` channels as 16-bit mono - the mean of
/// each frame's channels - and the loudest of them, `0..=1`.
fn mix_down(samples: impl Iterator<Item = f32>, channels: usize) -> (Vec<i16>, f32) {
    let channels = channels.max(1);
    let mut mono = Vec::new();
    let mut peak = 0.0f32;
    let mut sum = 0.0f32;
    let mut count = 0;
    for sample in samples {
        sum += sample;
        count += 1;
        if count == channels {
            let value = (sum / channels as f32).clamp(-1.0, 1.0);
            peak = peak.max(value.abs());
            mono.push((value * f32::from(i16::MAX)).round() as i16);
            sum = 0.0;
            count = 0;
        }
    }
    (mono, peak)
}

/// A 16-bit mono WAV written as it goes: the header up front with its sizes
/// left open, filled in by [`WavWriter::finish`].
struct WavWriter {
    out: BufWriter<File>,
    rate: u32,
    samples: u64,
}

impl WavWriter {
    fn create(path: &Path, rate: u32) -> Result<Self, String> {
        let file = File::create(path)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        let mut writer = WavWriter {
            out: BufWriter::new(file),
            rate,
            samples: 0,
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
        Ok(())
    }

    /// Fills in the sizes and closes the file; returns its length in seconds.
    fn finish(mut self) -> Result<f64, String> {
        // A WAV's sizes are 32-bit: past about thirteen hours of mono at
        // 48 kHz the header would wrap, so it says as much as it can.
        let data_bytes = u32::try_from(self.samples * 2).unwrap_or(u32::MAX - 36);
        self.out
            .seek(SeekFrom::Start(0))
            .map_err(|error| format!("could not finish the recording: {error}"))?;
        self.header(data_bytes)?;
        self.out
            .flush()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_mixes_down_to_the_mean_and_reports_its_peak() {
        let (mono, peak) = mix_down([0.5, 0.5, -1.0, 0.0, 0.25].into_iter(), 2);
        // The fifth sample is half a frame and is not written.
        assert_eq!(mono, vec![16384, -16384]);
        assert!((peak - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_wav_says_its_own_size_once_finished() {
        let path = std::env::temp_dir().join(format!("concat-record-{}.wav", std::process::id()));
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

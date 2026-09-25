// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Reverse: the span a clip covers of a media file, written backwards,
//! once.
//!
//! Playing a file backwards live means seeking to every frame in turn,
//! and each seek is a decode from the keyframe before it: choppy at best,
//! and wrong where a seek lands short of its frame. So a reverse is not
//! played, it is written. The span the clip covers is read forwards once,
//! turned round, and put in the project's cache as a file of its own,
//! picture and sound, which the clip then shows like any other file. The
//! import is never touched; the bin keeps it.
//!
//! Turning a span round without holding it whole - a minute of 4K is
//! tens of gigabytes of pixels - takes two passes. The first reads the
//! span forwards in runs of a few hundred megabytes, writes each run
//! backwards into a segment of its own, and moves on. The second reads
//! the segments last to first into the copy. Every frame is decoded and
//! encoded twice, the segments near-losslessly, and nothing is sought but
//! the span's start. The sound goes through the mixer's own graph with
//! `areverse`, which holds the span's samples - a few hundred megabytes
//! an hour - and is muxed in beside the picture.
//!
//! One job at a time through a [`SingleFlight`], like every long job the
//! host runs. The copy is named after the file's path, size and
//! modification time and the span it covers, so a file replaced on disk
//! gets a fresh one and the same span asked for twice gets the one
//! already there.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use concat_core::{Frame, FrameRate, Rational};
use concat_media::audio::{AudioClip, mix_to_file, mux};
use concat_media::{
    DecodeOptions, Decoder, EncodeOptions, Encoder, FrameSink, FrameSource, RateMode, VideoCodec,
};

use crate::jobs::SingleFlight;

/// What one reverse covers.
#[derive(Clone, Debug)]
pub struct ReverseRequest {
    /// The media file to read.
    pub media_path: String,
    /// A sound clip: no picture to turn, only the samples.
    pub audio_only: bool,
    /// Seconds into the file where the span begins.
    pub start: f64,
    /// Seconds of the file the span covers.
    pub duration: f64,
    /// Where the copy goes; see [`target_for`].
    pub target: PathBuf,
}

/// The reverse service: the one-job slot.
pub struct Reversers {
    gate: Arc<SingleFlight>,
}

/// Pixels a run of frames may hold before it is written out as a segment:
/// a hundred-odd frames of 1080p, a couple of dozen of 4K.
const RUN_BYTES: usize = 384 << 20;

/// Where the reversed copy of `media`'s span lives under `project`: in
/// the cache, named by the file's path, size and modification time and
/// the span in milliseconds. None for a file that cannot be stat'ed,
/// which cannot be read either.
pub fn target_for(
    project: &Path,
    media: &str,
    start: f64,
    duration: f64,
    audio_only: bool,
) -> Option<PathBuf> {
    let meta = std::fs::metadata(media).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    media.hash(&mut hasher);
    meta.len().hash(&mut hasher);
    if let Ok(modified) = meta.modified()
        && let Ok(since) = modified.duration_since(std::time::UNIX_EPOCH)
    {
        since.as_secs().hash(&mut hasher);
    }
    let extension = if audio_only { "m4a" } else { "mp4" };
    Some(project.join("cache").join(format!(
        "reverse-{:016x}-{}-{}.{extension}",
        hasher.finish(),
        (start * 1000.0).round() as u64,
        (duration * 1000.0).round() as u64
    )))
}

impl Default for Reversers {
    fn default() -> Self {
        Self::new()
    }
}

impl Reversers {
    /// A service with its slot free.
    pub fn new() -> Reversers {
        Reversers {
            gate: Arc::new(SingleFlight::new()),
        }
    }

    /// Whether a reverse is running.
    pub fn is_busy(&self) -> bool {
        self.gate.is_busy()
    }

    /// Asks the running reverse to stop after the frame in hand.
    pub fn cancel(&self) {
        self.gate.cancel();
    }

    /// Writes the copy `request` asks for, or finds it already written.
    /// Blocks until done, so run it on its own thread; `progress` is
    /// called with a fraction as it goes. Returns where the copy is.
    pub fn reverse(
        &self,
        request: &ReverseRequest,
        progress: &mut dyn FnMut(f32),
    ) -> Result<PathBuf, String> {
        if request.target.is_file() {
            return Ok(request.target.clone());
        }
        if !request.start.is_finite() || !request.duration.is_finite() || request.duration <= 0.0 {
            return Err("nothing to reverse".to_owned());
        }
        let job = self.gate.begin("reverse")?;
        if let Some(folder) = request.target.parent() {
            std::fs::create_dir_all(folder)
                .map_err(|error| format!("{}: {error}", folder.display()))?;
        }
        progress(0.0);
        let result = if request.audio_only {
            sound(request)
        } else {
            footage(request, &job, progress)
        };
        result.map(|()| request.target.clone())
    }
}

/// The span as the mixer sees it, with `chain` on it.
fn span_clip(request: &ReverseRequest, chain: &str) -> AudioClip {
    AudioClip {
        path: PathBuf::from(&request.media_path),
        stream: None,
        start: 0.0,
        duration: request.duration,
        source_start: request.start,
        speed: 1.0,
        preserve_pitch: true,
        volume: 1.0,
        volume_curve: concat_core::animate::Track::new(Vec::new()),
        fade_in: 0.0,
        fade_out: 0.0,
        filter_chain: chain.to_owned(),
    }
}

/// A partial file moved into place, or cleared away with the error.
fn land(partial: &Path, target: &Path, written: Result<(), String>) -> Result<(), String> {
    match written {
        Ok(()) => std::fs::rename(partial, target)
            .map_err(|error| format!("{}: {error}", target.display())),
        Err(error) => {
            let _ = std::fs::remove_file(partial);
            Err(error)
        }
    }
}

/// The span's sound, backwards, as a file of its own: the mixer's graph
/// with `areverse` on it.
fn sound(request: &ReverseRequest) -> Result<(), String> {
    let partial = request.target.with_extension("part.m4a");
    let written = mix_to_file(
        &[span_clip(request, "areverse")],
        request.duration,
        &partial,
    )
    .map_err(|error| error.to_string());
    land(&partial, &request.target, written)
}

/// The span's picture, backwards, with its sound beside it when the file
/// has any. Written under partial names and moved into place only when
/// whole, so a copy that is there is a copy that is complete.
fn footage(
    request: &ReverseRequest,
    job: &crate::jobs::Job,
    progress: &mut dyn FnMut(f32),
) -> Result<(), String> {
    let info = concat_media::probe(&request.media_path).map_err(|error| error.to_string())?;
    let video = info
        .video
        .as_ref()
        .ok_or_else(|| "no video stream".to_owned())?;
    let rate = video.frame_rate;
    // About how many frames the span holds, for the readout and for a
    // container that keeps no clock.
    let expected = (request.duration * rate.fps().as_f64()).max(1.0);
    // Even sides, which the encoder wants: a stray pixel at the right or
    // bottom is cropped rather than the whole copy refused.
    let width = video.width & !1;
    let height = video.height & !1;
    if width == 0 || height == 0 {
        return Err("the picture is too small to reverse".to_owned());
    }
    let end = request.start + request.duration;
    let partial_video = request.target.with_extension("part.mp4");

    // Pass one: the span forwards, in runs, each run written backwards as
    // a segment of its own.
    let start = Rational::approximate(request.start).unwrap_or(Rational::ZERO);
    let options = DecodeOptions::default()
        .in_software()
        .threaded(2)
        .starting_at(start);
    let mut decoder =
        Decoder::open(&request.media_path, &options).map_err(|error| error.to_string())?;
    let frame_bytes = width as usize * height as usize * concat_core::frame::BYTES_PER_PIXEL;
    let run = (RUN_BYTES / frame_bytes.max(1)).clamp(2, 240);
    let mut segments: Vec<PathBuf> = Vec::new();
    let mut held: Vec<Frame> = Vec::with_capacity(run);
    let mut count = 0u64;
    let passes = (|| -> Result<(), String> {
        loop {
            if job.cancelled() {
                return Err("reverse cancelled".to_owned());
            }
            let Some(frame) = decoder.next_frame().map_err(|error| error.to_string())? else {
                break;
            };
            // The span's end by the frame's own clock where the container
            // keeps one, and by count where it does not.
            let past = match decoder.position() {
                Some(at) => at.as_f64() >= end,
                None => count as f64 >= expected,
            };
            if past {
                break;
            }
            held.push(crate::enhance::crop(&frame, width, height));
            count += 1;
            if held.len() >= run {
                write_segment(request, &mut segments, &mut held, rate)?;
            }
            progress((0.5 * count as f64 / expected).min(0.5) as f32);
        }
        if !held.is_empty() {
            write_segment(request, &mut segments, &mut held, rate)?;
        }
        if segments.is_empty() {
            return Err("the span holds no frames".to_owned());
        }

        // Pass two: the segments last to first, into the copy.
        let mut encoder = Encoder::create(&partial_video, width, height, rate, &copy_options())
            .map_err(|error| error.to_string())?;
        let mut written = 0u64;
        let encoded = (|| -> Result<(), String> {
            for segment in segments.iter().rev() {
                let mut reader =
                    Decoder::open(segment, &DecodeOptions::default().in_software().threaded(2))
                        .map_err(|error| error.to_string())?;
                while let Some(frame) = reader.next_frame().map_err(|error| error.to_string())? {
                    if job.cancelled() {
                        return Err("reverse cancelled".to_owned());
                    }
                    encoder
                        .write_frame(&frame)
                        .map_err(|error| error.to_string())?;
                    written += 1;
                    progress((0.5 + 0.5 * written as f64 / count.max(1) as f64).min(0.99) as f32);
                }
            }
            encoder.finish().map_err(|error| error.to_string())
        })();
        if let Err(error) = encoded {
            let _ = std::fs::remove_file(&partial_video);
            return Err(error);
        }
        Ok(())
    })();
    for segment in &segments {
        let _ = std::fs::remove_file(segment);
    }
    passes?;

    // The sound, when the file has any, backwards beside the picture.
    if info.audio.is_none() {
        return land(&partial_video, &request.target, Ok(()));
    }
    let partial_audio = request.target.with_extension("part.m4a");
    let partial_muxed = request.target.with_extension("muxed.part.mp4");
    let muxed = mix_to_file(
        &[span_clip(request, "areverse")],
        request.duration,
        &partial_audio,
    )
    .and_then(|()| mux(&partial_video, &partial_audio, &partial_muxed))
    .map_err(|error| error.to_string());
    let _ = std::fs::remove_file(&partial_video);
    let _ = std::fs::remove_file(&partial_audio);
    land(&partial_muxed, &request.target, muxed)
}

/// The run in `held`, backwards, as the next segment; the run is emptied
/// either way.
fn write_segment(
    request: &ReverseRequest,
    segments: &mut Vec<PathBuf>,
    held: &mut Vec<Frame>,
    rate: FrameRate,
) -> Result<(), String> {
    let path = request
        .target
        .with_extension(format!("seg{:04}.part.mp4", segments.len()));
    let Some(first) = held.first() else {
        return Ok(());
    };
    let (width, height) = (first.width(), first.height());
    let written = Encoder::create(&path, width, height, rate, &segment_options())
        .and_then(|mut encoder| {
            for frame in held.iter().rev() {
                encoder.write_frame(frame)?;
            }
            encoder.finish()
        })
        .map_err(|error| error.to_string());
    held.clear();
    match written {
        Ok(()) => {
            segments.push(path);
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            Err(error)
        }
    }
}

/// A segment lives minutes and is read once: the fastest preset there is,
/// at a quality nothing is lost through.
fn segment_options() -> EncodeOptions {
    EncodeOptions {
        codec: VideoCodec::H264,
        preset: "ultrafast".to_owned(),
        crf: 8,
        rate_mode: RateMode::Vbr,
        bitrate_kbps: 0,
        ten_bit: false,
        color_range: concat_media::ColorRange::Limited,
        hardware: false,
        threads: 2,
    }
}

/// The copy the timeline reads for as long as the project lives:
/// near-transparent quality, as the enhanced copy is written.
fn copy_options() -> EncodeOptions {
    EncodeOptions {
        codec: VideoCodec::H264,
        preset: "medium".to_owned(),
        crf: 16,
        rate_mode: RateMode::Vbr,
        bitrate_kbps: 0,
        ten_bit: false,
        color_range: concat_media::ColorRange::Limited,
        hardware: false,
        threads: 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_is_named_after_the_file_and_the_span() {
        let dir = std::env::temp_dir().join(format!("concat-reverse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let media = dir.join("clip.mp4");
        std::fs::write(&media, b"not really").expect("write");
        let media = media.to_string_lossy().into_owned();
        let project = dir.join("project");
        let span = target_for(&project, &media, 1.5, 4.0, false).expect("named");
        assert!(span.starts_with(project.join("cache")));
        assert!(
            span.to_string_lossy().ends_with("-1500-4000.mp4"),
            "{span:?}"
        );
        assert_eq!(
            target_for(&project, &media, 1.5, 4.0, false),
            Some(span.clone())
        );
        assert_ne!(
            target_for(&project, &media, 2.0, 4.0, false),
            Some(span.clone())
        );
        let sound = target_for(&project, &media, 1.5, 4.0, true).expect("named");
        assert!(sound.to_string_lossy().ends_with(".m4a"), "{sound:?}");
        assert_eq!(
            target_for(
                &project,
                &dir.join("gone.mp4").to_string_lossy(),
                0.0,
                1.0,
                false
            ),
            None
        );
        // A file with different bytes is a different copy.
        std::fs::write(&media, b"not really, but longer").expect("write");
        assert_ne!(target_for(&project, &media, 1.5, 4.0, false), Some(span));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_is_sized_to_the_frame() {
        let hd = RUN_BYTES / (1920 * 1080 * 4);
        let uhd = RUN_BYTES / (3840 * 2160 * 4);
        assert!(hd > uhd && uhd >= 2, "{hd} {uhd}");
        assert_eq!((RUN_BYTES / (7680 * 4320 * 4)).clamp(2, 240), 3);
    }
}

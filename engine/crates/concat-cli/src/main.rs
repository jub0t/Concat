// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Command line driver for the Concat engine.
//!
//! This exists so the engine can be exercised end to end without a UI. The
//! `render` command is the vertical slice: probe, build a timeline, plan every
//! frame, decode, composite, encode.

use std::error::Error;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use concat_core::time::{FrameRate, Rational};
use concat_core::timeline::{Clip, MediaRef, Timeline, Track, TrackKind};
use concat_media::{EncodeOptions, Encoder, FrameSink, ReaderPool};
use concat_render::{Compositor, CpuCompositor, Layer, plan_frame};

#[derive(Parser)]
#[command(name = "concat-cli", version, about = "Concat engine command line")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report what is inside a media file.
    Probe {
        /// The file to inspect.
        path: PathBuf,
    },

    /// Decode, composite and re-encode a clip - the end-to-end slice.
    Render {
        /// Source video.
        input: PathBuf,
        /// Where to write the result.
        output: PathBuf,
        /// How many frames to render.
        #[arg(long, default_value_t = 90)]
        frames: u64,
        /// Fade up from black over this many frames. Zero disables the fade.
        #[arg(long, default_value_t = 15)]
        fade: u64,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Probe { path } => probe(&path),
        Command::Render {
            input,
            output,
            frames,
            fade,
        } => render(&input, &output, frames, fade),
    }
}

fn probe(path: &PathBuf) -> Result<(), Box<dyn Error>> {
    let info = concat_media::probe(path)?;

    println!("{}", info.path.display());
    match info.duration {
        Some(duration) => println!("  duration  {:.3}s ({duration})", duration.as_f64()),
        None => println!("  duration  unknown"),
    }

    match &info.video {
        Some(video) => println!(
            "  video     #{} {} {}x{} @ {} ({})",
            video.index,
            video.codec,
            video.width,
            video.height,
            video.frame_rate,
            video.frame_rate.fps()
        ),
        None => println!("  video     none"),
    }

    match &info.audio {
        Some(audio) => println!(
            "  audio     #{} {} {} Hz, {} channels",
            audio.index, audio.codec, audio.sample_rate, audio.channels
        ),
        None => println!("  audio     none"),
    }

    Ok(())
}

/// The vertical slice.
///
/// Frames come from the reader pool by (media, source time), which is what
/// makes this loop correct for *any* plan - overlapping clips, gaps, jumps -
/// not just one clip played start to finish. The shortcut this function
/// carried for its whole early life is gone; the pool it was waiting for
/// exists (`concat_media::pool`).
fn render(input: &PathBuf, output: &PathBuf, frames: u64, fade: u64) -> Result<(), Box<dyn Error>> {
    let info = concat_media::probe(input)?;
    let video = info.require_video()?;
    let (width, height) = (video.width, video.height);
    let rate = video.frame_rate;

    let timeline = single_clip_timeline(input, width, height, rate, frames);

    let mut pool = ReaderPool::with_defaults();
    let mut encoder = Encoder::create(output, width, height, rate, &EncodeOptions::default())?;
    let mut compositor = CpuCompositor;

    println!("rendering {frames} frames at {width}x{height} {rate}");

    for index in 0..frames {
        let plan = plan_frame(&timeline, rate.time_of_frame(index as i64));

        let composed = if plan.is_empty() {
            // A gap in the timeline is black, not an error.
            compositor.composite(width, height, &[])
        } else {
            let mut sources = Vec::with_capacity(plan.layers.len());
            for layer in &plan.layers {
                let frame =
                    pool.frame_at(&layer.media, layer.source_time, width, height, false, None)?;
                sources.push((frame, layer.opacity));
            }
            let layers: Vec<Layer<'_>> = sources
                .iter()
                .map(|(frame, opacity)| {
                    Layer::new(frame).with_opacity(opacity * fade_in(index, fade))
                })
                .collect();
            compositor.composite(width, height, &layers)
        };

        encoder.write_frame(&composed)?;
    }

    encoder.finish()?;
    println!("wrote {} frames to {}", encoder.written(), output.display());
    Ok(())
}

fn single_clip_timeline(
    input: &PathBuf,
    width: u32,
    height: u32,
    rate: FrameRate,
    frames: u64,
) -> Timeline {
    let mut timeline = Timeline::new(width, height, rate);
    let track = timeline.add_track(Track::new("V1", TrackKind::Video));
    let duration = rate.time_of_frame(frames as i64);
    timeline
        .add_clip(
            track,
            Clip::new(MediaRef::new(input), Rational::ZERO, duration),
        )
        .expect("the track was just added");
    timeline
}

/// Ramps from 0.0 to 1.0 over the first `fade` frames.
fn fade_in(index: u64, fade: u64) -> f32 {
    if fade == 0 || index >= fade {
        1.0
    } else {
        index as f32 / fade as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fade_ramps_then_holds() {
        assert_eq!(fade_in(0, 10), 0.0);
        assert_eq!(fade_in(5, 10), 0.5);
        assert_eq!(fade_in(10, 10), 1.0);
        assert_eq!(fade_in(99, 10), 1.0);
    }

    #[test]
    fn a_zero_length_fade_is_fully_opaque_immediately() {
        assert_eq!(fade_in(0, 0), 1.0);
    }

    #[test]
    fn the_timeline_covers_exactly_the_requested_frames() {
        let timeline =
            single_clip_timeline(&PathBuf::from("a.mp4"), 1920, 1080, FrameRate::NTSC_30, 90);
        assert_eq!(timeline.frame_count(), 90);
        assert_eq!(timeline.clip_count(), 1);
    }
}

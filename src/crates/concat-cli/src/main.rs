// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Command line driver for the Concat engine.
//!
//! This exists so the engine can be exercised end to end without a UI. The
//! `render` command is the vertical slice: probe, build a timeline, plan every
//! frame, decode, composite, encode. The `api` command is the Concat API's
//! first transport: JSON requests in, JSON responses and events out, one per
//! line, so a script in any language edits and exports a project. `preview`
//! is how the window's effect cards get their pictures: one still through one
//! package at its defaults.

use std::error::Error;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use concat_core::time::{FrameRate, Rational};
use concat_core::timeline::{Clip, MediaRef, Timeline, Track, TrackKind};
use concat_media::{
    DecodeOptions, Decoder, EncodeOptions, Encoder, FrameSink, FrameSource, ReaderPool,
};
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

    /// Speak the Concat API: a JSON request per line on stdin, a response per
    /// line on stdout, with events in between as they happen.
    Api {
        /// One request to run instead of reading stdin.
        request: Option<String>,
    },

    /// Run one picture through an effect at its defaults and write the
    /// result as a JPEG - how the effect cards' previews are made:
    /// `concat-cli preview assets/effect-preview-source.jpg out.jpg --effect concat.emboss`.
    Preview {
        /// A still, or the first frame of a video.
        input: PathBuf,
        /// Where to write the JPEG.
        output: PathBuf,
        /// The package's catalogue id or alias, e.g. `concat.emboss`.
        #[arg(long)]
        effect: String,
        /// Output width in pixels; the picture is scaled before the effect
        /// runs, so pixel-sized effects look as they will on the card.
        #[arg(long, default_value_t = 320)]
        width: u32,
        /// Output height in pixels.
        #[arg(long, default_value_t = 180)]
        height: u32,
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
        Command::Api { request } => api(request),
        Command::Preview {
            input,
            output,
            effect,
            width,
            height,
        } => preview(&input, &output, &effect, width, height),
    }
}

/// The stdin transport. Every line in is one request; every line out is
/// one JSON object, either an event or the response, flushed as written so
/// a caller reading a pipe sees progress as it happens. A line that is not
/// a request gets an error response and the loop goes on.
fn api(single: Option<String>) -> Result<(), Box<dyn Error>> {
    let mut api = concat_api::Api::new()?;
    let stdout = std::io::stdout();

    let mut serve = |line: &str| -> Result<(), Box<dyn Error>> {
        let mut out = stdout.lock();
        let response = match serde_json::from_str::<concat_api::Request>(line) {
            Ok(request) => api.dispatch(request, &mut |event| {
                // An event that cannot be written is a caller that went
                // away; the response's write will say so.
                let _ = serde_json::to_writer(&mut out, &event)
                    .and_then(|()| writeln!(out).map_err(serde_json::Error::io));
                let _ = out.flush();
            }),
            Err(error) => concat_api::Response::Error(format!("not a request: {error}")),
        };
        serde_json::to_writer(&mut out, &response)?;
        writeln!(out)?;
        out.flush()?;
        Ok(())
    };

    match single {
        Some(line) => serve(&line),
        None => {
            for line in std::io::stdin().lock().lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                serve(&line)?;
            }
            Ok(())
        }
    }
}

/// One frame of `input`, scaled to the card's size, through `effect` at its
/// defaults, as a JPEG at `output`.
fn preview(
    input: &PathBuf,
    output: &PathBuf,
    effect: &str,
    width: u32,
    height: u32,
) -> Result<(), Box<dyn Error>> {
    let catalogue = concat_effects::Catalogue::builtin();
    let package = catalogue
        .get(effect)
        .ok_or_else(|| format!("no package answers to {effect}"))?;
    let applied = concat_project::model::AppliedFilter::new(package.manifest.effect.id.clone());
    // The CPU chain: what a machine without a GPU renders, and what every
    // package has whether or not it also carries a shader.
    let chain = catalogue.video_chain(&[applied]);
    if chain.is_empty() {
        return Err(format!("{effect} has no FFmpeg chain to preview").into());
    }
    let mut decoder = Decoder::open(input, &DecodeOptions::default().scaled_to(width, height))?;
    let frame = decoder
        .next_frame()?
        .ok_or_else(|| format!("{} holds no picture", input.display()))?;
    let treated = concat_media::treat(&frame, &chain)?;
    std::fs::write(output, concat_media::jpeg(&treated, 3)?)?;
    println!("{effect}: {chain}\n  -> {}", output.display());
    Ok(())
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

    // Every audio stream, the default first: a recording with its tracks
    // apart lists them all, and a clip may play any one.
    if info.audio_streams.is_empty() {
        println!("  audio     none");
    }
    for audio in &info.audio_streams {
        let name = match (audio.title.is_empty(), audio.language.is_empty()) {
            (true, true) => String::new(),
            (false, true) => format!(" \"{}\"", audio.title),
            (true, false) => format!(" [{}]", audio.language),
            (false, false) => format!(" \"{}\" [{}]", audio.title, audio.language),
        };
        println!(
            "  audio     #{} {} {} Hz, {} channels{name}",
            audio.index, audio.codec, audio.sample_rate, audio.channels
        );
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

    let pool = ReaderPool::with_defaults();
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
                let frame = pool.frame_at(
                    &layer.media,
                    layer.source_time,
                    width,
                    height,
                    false,
                    None,
                    None,
                )?;
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

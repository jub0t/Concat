// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Command line driver for the Concat engine.
//!
//! This exists so the engine can be exercised end to end without a UI. The
//! `render` command is the vertical slice: probe, build a timeline, plan every
//! frame, decode, composite, encode. The `api` command is the Concat API's
//! first transport: JSON-RPC requests in, responses and events out, one per
//! line, so a script in any language edits and exports a project; `serve`
//! is the same API on a socket, for callers that are other processes.
//! `preview` is how the window's effect cards get their pictures: one still
//! through one package at its defaults. `check` is what an author runs on
//! an effect package before sharing it: the same load the window does, and
//! its fixtures, with every fault named.

use std::error::Error;
use std::io::{BufRead, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use concat_api::rpc::{Call, Message};
use concat_core::time::{FrameRate, Rational};
use concat_core::timeline::{Clip, MediaRef, Timeline, Track, TrackKind};
use concat_media::{
    DecodeOptions, Decoder, EncodeOptions, Encoder, FrameSink, FrameSource, ReaderPool,
};
use concat_render::{Compositor, CpuCompositor, plan_frame};

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

    /// Speak the Concat API: a JSON-RPC request per line on stdin, a
    /// response per line on stdout, with events from running jobs as they
    /// happen. A bare `{"method": ..., ...}` without the envelope works too.
    Api {
        /// One request to run instead of reading stdin.
        request: Option<String>,
    },

    /// Serve the Concat API on a socket until the process is stopped:
    /// JSON-RPC lines over TCP or a Unix socket, and gRPC in a build that
    /// has it. With no address given, JSON-RPC on 127.0.0.1:7420. Every
    /// connection presents a token first; with none given, one is minted
    /// and printed with the addresses.
    Serve {
        /// The TCP address for JSON-RPC lines, e.g. 127.0.0.1:7420.
        #[arg(long)]
        json: Option<SocketAddr>,
        /// A Unix socket path for JSON-RPC lines.
        #[arg(long)]
        socket: Option<PathBuf>,
        /// The TCP address for gRPC. Needs a build with the `grpc` feature.
        #[arg(long)]
        grpc: Option<SocketAddr>,
        /// The token every connection presents first. Minted when not
        /// given, and printed either way.
        #[arg(long, env = "CONCAT_API_TOKEN")]
        token: Option<String>,
        /// A folder the API may write under: created projects, exports,
        /// preview files. Repeatable. Your home when none is given; `/`
        /// for anywhere.
        #[arg(long = "root", value_name = "DIR")]
        roots: Vec<PathBuf>,
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

    /// Check effect packages before sharing them: load each folder the way
    /// the window does, hold its id against the built-ins, run its
    /// fixtures, and name every fault. `path` is one package folder - the
    /// one with `effect.toml` in it - or a folder of them, such as the
    /// app's own effects folder. Exits non-zero when any package fails.
    Check {
        /// A package folder, or a folder of package folders.
        path: PathBuf,
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
        Command::Serve {
            json,
            socket,
            grpc,
            token,
            roots,
        } => serve(json, socket, grpc, token, roots),
        Command::Preview {
            input,
            output,
            effect,
            width,
            height,
        } => preview(&input, &output, &effect, width, height),
        Command::Check { path } => check(&path),
    }
}

/// The stdin transport. Every line in is one call; every line out is one
/// JSON object, a response or an event, written whole under stdout's lock
/// and flushed, so a caller reading a pipe sees progress as it happens and
/// never half a line. A line that is not a call gets an error response and
/// the loop goes on. At the end of input the jobs still running - an export
/// begun on the last line - are waited for, so their events are written
/// before the process exits.
fn api(single: Option<String>) -> Result<(), Box<dyn Error>> {
    let mut api = concat_api::Api::new(Arc::new(|event| emit(&Message::Event(event))))?;
    let mut serve = |line: &str| {
        let (id, response) = match Call::parse(line) {
            Ok(call) => (call.id, api.dispatch(call.request)),
            Err((id, error)) => (id, concat_api::Response::Error(error)),
        };
        emit(&Message::Reply { id, response });
    };
    match single {
        Some(line) => serve(&line),
        None => {
            for line in std::io::stdin().lock().lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                serve(&line);
            }
        }
    }
    api.finish();
    Ok(())
}

/// One line to stdout. A line that cannot be written is a caller that
/// went away, and the loop learns that from stdin's end.
fn emit(message: &Message) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", message.to_json());
    let _ = out.flush();
}

/// The socket transports, until the process is stopped. Where they listen
/// is printed, one line each, and then the token they take, so a script
/// that started this knows where to connect and what to present.
fn serve(
    json: Option<SocketAddr>,
    socket: Option<PathBuf>,
    grpc: Option<SocketAddr>,
    token: Option<String>,
    roots: Vec<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    let nothing_asked = json.is_none() && socket.is_none() && grpc.is_none();
    let config = concat_server::Config {
        json: json.or_else(|| nothing_asked.then(|| DEFAULT_JSON.parse().expect("an address"))),
        socket,
        grpc,
        token,
        roots,
    };
    let server = concat_server::Server::start(config, concat_api::Api::new)?;
    if let Some(address) = server.json_addr() {
        println!(
            "Concat API {}: JSON-RPC on {address}",
            concat_api::API_VERSION
        );
    }
    if let Some(path) = server.socket_path() {
        println!(
            "Concat API {}: JSON-RPC on {}",
            concat_api::API_VERSION,
            path.display()
        );
    }
    if let Some(address) = server.grpc_addr() {
        println!("Concat API {}: gRPC on {address}", concat_api::API_VERSION);
    }
    println!(
        "Concat API {}: token {}",
        concat_api::API_VERSION,
        server.token()
    );
    let roots = server.roots();
    println!(
        "Concat API {}: writes under {}",
        concat_api::API_VERSION,
        if roots.is_empty() {
            "anywhere".to_owned()
        } else {
            roots
                .iter()
                .map(|root| root.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    loop {
        std::thread::park();
    }
}

/// Where `serve` listens when not told: loopback, on a port nothing else
/// is known to use.
const DEFAULT_JSON: &str = "127.0.0.1:7420";

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

/// Every package at `path` through the loader and its fixtures, one line
/// per package and one per fault. The built-ins are what the ids are held
/// against: a user package cannot take a name the window already knows.
fn check(path: &Path) -> Result<(), Box<dyn Error>> {
    let folders = if path.join("effect.toml").is_file() {
        vec![path.to_path_buf()]
    } else {
        let folders = concat_effects::package_folders(path)?;
        if folders.is_empty() {
            return Err(format!(
                "{}: no package here - a package is a folder with an effect.toml in it",
                path.display()
            )
            .into());
        }
        folders
    };
    let taken = concat_effects::Catalogue::builtin();
    let mut failed = 0;
    for folder in &folders {
        let problems = concat_effects::Package::check_folder(folder, taken);
        if problems.is_empty() {
            println!("ok    {}", folder.display());
        } else {
            failed += 1;
            println!("FAIL  {}", folder.display());
            for problem in problems {
                for line in problem.lines() {
                    println!("      {line}");
                }
            }
        }
    }
    if failed > 0 {
        return Err(format!("{failed} of {} package(s) failed", folders.len()).into());
    }
    println!("{} package(s) checked", folders.len());
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
        let mut plan = plan_frame(&timeline, rate.time_of_frame(index as i64));
        // The plan says what is on screen; the pool fills in the pictures.
        // A gap in the timeline is a plan with no layers, and black.
        for layer in &mut plan.layers {
            let frame = pool.frame_at(
                &layer.media,
                layer.source_time,
                width,
                height,
                false,
                None,
                None,
                None,
            )?;
            layer.source = Some(frame);
            layer.opacity *= fade_in(index, fade);
        }
        let composed = compositor.render(&plan);

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

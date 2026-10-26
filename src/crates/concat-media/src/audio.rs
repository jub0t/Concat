// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Planning and running the audio mix.
//!
//! The audio equivalent of `concat-render`'s frame plan: [`mix_graph`] turns a
//! set of audible clips into one FFmpeg filtergraph, as a pure function that
//! is unit-tested without running anything. [`mix_to_file`] hands the plan to
//! libavfilter and [`mux`] joins the result with the picture.
//!
//! This lives in the engine, not the app, so that there is exactly one
//! definition of what speed, fades and gain mean for sound - the same reason
//! `Clip::source_time_at` is the one definition for picture.

use std::path::Path;

use concat_core::animate::Track;
use concat_core::time::Rational;
use ffmpeg_the_third as ffmpeg;
use ffmpeg_the_third::codec::encoder;
use ffmpeg_the_third::filter;
use ffmpeg_the_third::util::channel_layout::ChannelLayout;
use ffmpeg_the_third::util::format::Sample;
use ffmpeg_the_third::util::frame::audio::Audio;

use crate::error::{Error, Result};
use crate::ffi;

/// The rate everything is mixed at.
pub const MIX_RATE: u32 = 48_000;

/// The playback rates the engine supports, as source-seconds per
/// timeline-second. One definition, shared by preview and export - the two
/// disagreeing at the extremes is how a preview stops matching its render.
pub const SPEED_RANGE: std::ops::RangeInclusive<f64> = 0.0625..=16.0;

/// Clamps a rate into [`SPEED_RANGE`].
pub fn clamp_speed(speed: f64) -> f64 {
    speed.clamp(*SPEED_RANGE.start(), *SPEED_RANGE.end())
}

/// One audible clip, in timeline terms.
#[derive(Clone, Debug)]
pub struct AudioClip {
    /// The media file.
    pub path: std::path::PathBuf,
    /// Which of the file's audio streams, by stream index; `None` is the
    /// first in file order. See `probe::audio_stream_index`.
    pub stream: Option<usize>,
    /// Where the clip sits on the timeline, in seconds.
    pub start: f64,
    /// How long it runs on the timeline, in seconds.
    pub duration: f64,
    /// Seconds into the source file where the clip begins.
    pub source_start: f64,
    /// Source seconds consumed per timeline second. One is normal.
    pub speed: f64,
    /// True holds pitch while the rate changes; false is the tape effect.
    pub preserve_pitch: bool,
    /// Linear gain, one being unity. The whole clip's, unless
    /// `volume_curve` says otherwise.
    pub volume: f64,
    /// Gain as it changes over the clip, in absolute linear gain against a
    /// fraction of the clip's timeline length. Empty is the constant in
    /// `volume`; a track supersedes it rather than multiplying it.
    ///
    /// The engine's own `Track` and not a list of pairs, so the ramp the
    /// mixer writes into the filtergraph and the ramp the monitor plays are
    /// read from one definition, easing included.
    pub volume_curve: Track,
    /// Fade up over this many seconds from the clip's start.
    pub fade_in: f64,
    /// Fade out over this many seconds into the clip's end.
    pub fade_out: f64,
    /// Extra FFmpeg audio filters, or empty. Validated, not trusted.
    pub filter_chain: String,
}

/// A treatment of the whole mix over a stretch of the timeline: a sound
/// layer. Its chain hears only what plays inside its span, and what it
/// gives back takes the untreated mix's place there by `strength`, ramped
/// in and out by its fades - the reading a picture layer gets over the
/// tracks beneath it, given to sound.
///
/// A bus over the sum and not the same chain copied onto every clip under
/// it: a compressor or a limiter over the sum is a different sound from
/// one on each part, and a reverb over the sum is one room rather than
/// several.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioBus {
    /// Where the span starts on the timeline, in seconds.
    pub start: f64,
    /// How long it runs, in seconds.
    pub duration: f64,
    /// The FFmpeg audio filters run over the mix. Validated, not trusted.
    pub filter_chain: String,
    /// How much of the treated sound replaces the untreated, `0..=1`: the
    /// layer's wet and dry.
    pub strength: f64,
    /// Seconds over which the treatment comes in from the span's start.
    pub fade_in: f64,
    /// Seconds over which it goes out into the span's end.
    pub fade_out: f64,
}

impl AudioBus {
    /// Where the span ends on the timeline.
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }

    /// Whether the bus changes anything: a chain to run, some strength to
    /// run it at, and some time to run it for.
    pub fn audible(&self) -> bool {
        !self.filter_chain.is_empty() && self.strength > 0.0 && self.duration > 0.0
    }
}

/// An FFmpeg `volume` expression for a gain that changes over a clip, or
/// Straight segments per eased one, when a gain ride is flattened for the
/// filtergraph.
///
/// A chord's error against the curve it cuts falls as the square of the
/// step count. Twelve puts the worst case on a full-span ease-in-out at a
/// few thousandths of the ride's range - about 0.03 dB on a gain ramp, which
/// is an order of magnitude under what anyone can hear - and keeps the
/// expression to a few hundred characters per key. Doubling it would buy
/// four times the accuracy nobody can hear, at twice the length.
const EASE_STEPS: usize = 12;

/// None when the track is empty and the scalar gain will do.
///
/// The shape is `Track::value_at` written out: hold the first key's value
/// before it, hold the last key's after it, and run straight between
/// neighbours. Nested `if`s rather than anything cleverer, because that is
/// what the expression language has.
///
/// Straight, and never eased, because an ease is a cubic bezier and solving
/// one for y at a given x takes a loop the expression language does not
/// have. `Track::resample` turns each eased segment into `EASE_STEPS`
/// straight ones through the same curve first, so the easing is applied in
/// the one place that understands it and this only ever writes lines.
///
/// `t` inside the graph is seconds from the clip's own start - the same
/// clock `afade`'s `st=` uses two filters later - so the fraction the track
/// is indexed by is `t / duration`.
fn volume_expr(track: &Track, duration: f64) -> Option<String> {
    let flat = track.resample(EASE_STEPS);
    let keys = flat.keys();
    let first = keys.first()?;
    if duration <= 0.0 {
        return Some(format!("{:.6}", first.value.max(0.0)));
    }
    let gain = |value: f64| format!("{:.6}", value.max(0.0));
    let x = format!("(t/{duration:.6})");

    // From the tail back: each step wraps what it has in "before this key,
    // ride this segment".
    let mut expr = gain(keys[keys.len() - 1].value);
    for pair in keys.windows(2).rev() {
        let (a, b) = (pair[0], pair[1]);
        let span = b.at - a.at;
        let segment = if span <= 0.0 {
            gain(b.value)
        } else {
            format!(
                "({}+({})*clip(({x}-{:.6})/{span:.6},0,1))",
                gain(a.value),
                format_args!("{:.6}", b.value.max(0.0) - a.value.max(0.0)),
                a.at
            )
        };
        expr = format!("if(lte({x},{:.6}),{segment},{expr})", b.at);
    }
    Some(format!(
        "if(lte({x},{:.6}),{},{expr})",
        first.at,
        gain(first.value)
    ))
}

/// Refuses a filter chain that could break out of its slot in the graph.
///
/// A chain is spliced into a filtergraph, where `;` starts a new chain and
/// `[..]` rebinds streams - a chain containing either is no longer a filter
/// applied to this clip, whatever else it might be.
pub fn validate_chain(chain: &str) -> Result<()> {
    let forbidden = |c: char| matches!(c, ';' | '[' | ']' | '\n' | '\r');
    match chain.chars().find(|&c| forbidden(c)) {
        None => Ok(()),
        Some(found) => Err(Error::InvalidFilterChain {
            chain: chain.to_owned(),
            detail: format!("contains {found:?}, which would escape the clip's filter slot"),
        }),
    }
}

/// The filters that retime one clip, in order. Empty at normal speed.
///
/// Pitch-preserving uses `atempo`, staged because one instance only accepts
/// [0.5, 2]. The tape effect first lands on [`MIX_RATE`] so the rate shift is
/// exact whatever the source's rate was, then shifts, then lands back.
pub fn speed_filters(speed: f64, preserve_pitch: bool) -> Vec<String> {
    let speed = clamp_speed(speed);
    if (speed - 1.0).abs() < f64::EPSILON {
        return Vec::new();
    }
    if preserve_pitch {
        tempo_stages(speed)
            .iter()
            .map(|factor| format!("atempo={factor:.6}"))
            .collect()
    } else {
        vec![
            format!("aresample={MIX_RATE}"),
            format!("asetrate={MIX_RATE}*{speed:.6}"),
            format!("aresample={MIX_RATE}"),
        ]
    }
}

/// Splits a rate into `atempo` stages that each stay inside its valid range.
///
/// One filter instance is limited, so 4x has to become 2x then 2x. Multiplying
/// the stages back together is what keeps the result exact.
pub fn tempo_stages(speed: f64) -> Vec<f64> {
    let mut stages = Vec::new();
    let mut remaining = clamp_speed(speed);

    while remaining > 2.0 {
        stages.push(2.0);
        remaining /= 2.0;
    }
    while remaining < 0.5 {
        stages.push(0.5);
        remaining /= 0.5;
    }

    stages.push(remaining);
    stages
}

/// Builds the filtergraph that trims, retimes, shapes, delays and mixes every
/// clip into one `[out]` stream, padded with silence past the last clip
/// for as long as it is read. Where it ends is the reader's call, by
/// sample count: see `mix_to_file`.
///
/// Input `N` is `clips[N].path`, bound to the label `[N:a]`, and every input
/// must actually contain an audio stream - the graph names `[N:a]` on it
/// rather than treating silence as a stream. The caller decides membership
/// by probing; this function decides what the mix means.
pub fn mix_graph(clips: &[AudioClip]) -> Result<String> {
    mix_graph_with(clips, &[])
}

/// [`mix_graph`] with sound layers over the mix: each bus treats the sum
/// of every clip for its span, in the order given.
pub fn mix_graph_with(clips: &[AudioClip], buses: &[AudioBus]) -> Result<String> {
    let heard: Vec<Heard> = clips.iter().map(Heard::whole).collect();
    mix_graph_from(clips, &heard, 0.0, buses)
}

/// How much of one clip an input of the mix carries, and how it landed.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Heard {
    /// Seconds into the clip's span where what is heard starts: zero for
    /// the whole clip, more for one a window cuts into.
    from: f64,
    /// Seconds into the clip's span where it stops.
    to: f64,
    /// How many seconds before that point the input actually starts. A
    /// container seek lands on the packet at or before the target - a
    /// whole AAC frame early, 21 ms - and `run_mix` measures the miss from
    /// the first decoded frame; on the continuous clock it stamps, the
    /// graph's cut is then exact. A lead below zero is sound that starts
    /// after that point - a file whose picture starts before its sound -
    /// and that much silence goes in front.
    lead: f64,
}

impl Heard {
    /// All of `clip`, landed exactly.
    fn whole(clip: &AudioClip) -> Heard {
        Heard {
            from: 0.0,
            to: clip.duration,
            lead: 0.0,
        }
    }

    /// The part of `clip` inside the timeline's `[from, to)`, or None when
    /// none of it is.
    fn inside(clip: &AudioClip, from: f64, to: f64) -> Option<Heard> {
        let start = (from - clip.start).max(0.0);
        let end = (to - clip.start).min(clip.duration);
        (end > start).then_some(Heard {
            from: start,
            to: end,
            lead: 0.0,
        })
    }
}

/// The few milliseconds a bus's hard edges are softened over. Its chain is
/// cut in and out of the mix where its span starts and ends, and an edge
/// with no length at all is a click.
const BUS_EDGE: f64 = 0.002;

/// One bus's stage of the graph, from `[s{index}]` to `[s{index + 1}]`,
/// with `origin` the timeline second the graph's clock starts at.
///
/// Three copies of the mix are summed: the mix itself, the mix taken away
/// again by the bus's ramp and strength, and the chain's output given back
/// by the same. Outside the span the second and third are silence - an
/// `afade` silences what lies before a fade in and after a fade out - so
/// the mix passes there sample for sample; inside it the treated sound has
/// taken `strength` of the untreated one's place. The chain is fed only
/// what plays inside the span, so an echo does not repeat what came before
/// the layer, and what it rings on with past the span is cut at the end.
fn bus_stage(index: usize, bus: &AudioBus, origin: f64) -> String {
    let start = (bus.start - origin).max(0.0);
    let end = start + bus.duration.max(0.0);
    let edge = BUS_EDGE.min(bus.duration / 2.0).max(1e-4);
    let ramp_in = bus.fade_in.clamp(edge, bus.duration.max(edge));
    let ramp_out = bus.fade_out.clamp(edge, bus.duration.max(edge));
    let strength = bus.strength.clamp(0.0, 1.0);
    let gate = format!(
        "afade=t=in:st={start:.6}:d={edge:.6},afade=t=out:st={:.6}:d={edge:.6}",
        (end - edge).max(start)
    );
    let ramp = format!(
        "afade=t=in:st={start:.6}:d={ramp_in:.6},afade=t=out:st={:.6}:d={ramp_out:.6}",
        (end - ramp_out).max(start)
    );
    let chain = &bus.filter_chain;
    let (here, next) = (format!("s{index}"), format!("s{}", index + 1));
    // Restamped after the sum, as before the first stage: a chain's
    // output need not stamp its frames the way its input did, and the
    // next stage's fades are placed by the stamps.
    format!(
        "[{here}]asplit=3[{here}d][{here}u][{here}t];\
         [{here}u]{ramp},volume=-{strength:.6}[{here}n];\
         [{here}t]{gate},{chain},{ramp},volume={strength:.6}[{here}w];\
         [{here}d][{here}n][{here}w]amix=inputs=3:normalize=0,aresample={MIX_RATE},asetpts=N/SR/TB[{next}]"
    )
}

/// The graph for `clips`, each heard as `heard` says, on a clock that
/// starts at the timeline's `origin` second, with `buses` over the sum.
fn mix_graph_from(
    clips: &[AudioClip],
    heard: &[Heard],
    origin: f64,
    buses: &[AudioBus],
) -> Result<String> {
    let mut chains: Vec<String> = Vec::new();

    for (index, clip) in clips.iter().enumerate() {
        validate_chain(&clip.filter_chain)?;
        let heard = heard
            .get(index)
            .copied()
            .unwrap_or_else(|| Heard::whole(clip));
        let delay_ms = ((clip.start + heard.from - origin) * 1000.0)
            .round()
            .max(0.0) as i64;
        let speed = clamp_speed(clip.speed);

        // Each input is cut to its in-point, restamped from zero, then delayed
        // to where it sits on the timeline. `all=1` applies the delay to
        // every channel; without it only the left channel moves, which is a
        // memorable way to discover the flag exists.
        //
        // The cut is by `lead`, not by the source's own timestamps:
        // `mix_to_file` stamps every frame it feeds from a running sample
        // count, so a source whose timestamp series jumps (edit lists,
        // paused recordings) cannot park frames outside the mix, and the
        // out-point is a sample count too - it stops feeding the input at
        // `duration * speed` source seconds; see `MixInput::clip_samples`.
        let lead = heard.lead;
        let mut stage = if lead > 0.0 {
            format!("[{index}:a]atrim=start={lead:.6},asetpts=PTS-STARTPTS")
        } else if lead < -0.0005 {
            format!("[{index}:a]adelay=delays={:.3}:all=1", -lead * 1000.0)
        } else {
            format!("[{index}:a]anull")
        };

        for filter in speed_filters(speed, clip.preserve_pitch) {
            stage.push(',');
            stage.push_str(&filter);
        }

        // A clip a window cuts into is shaped on its own clock still: its
        // gain ride and its fades are read from where the clip starts, so
        // what is heard of it here is what the whole clip has at that
        // moment. The stamps are moved on by what was cut, and back after.
        let cut = heard.from > 0.0;
        if cut {
            stage.push_str(&format!(",asetpts=PTS-STARTPTS+{:.6}/TB", heard.from));
        }

        // Filters first: they are what the clip *is* now, and gain and fades
        // are adjustments applied to that result. Reversing the order would
        // let a compressor inside a filter chain undo the fade.
        if !clip.filter_chain.is_empty() {
            stage.push(',');
            stage.push_str(&clip.filter_chain);
        }

        match volume_expr(&clip.volume_curve, clip.duration) {
            // `eval=frame` is what makes the expression a curve rather than
            // a number: without it `volume` evaluates once, at init, and a
            // ride would come out as whatever the gain was at t=0.
            Some(expr) => stage.push_str(&format!(",volume=volume='{expr}':eval=frame")),
            None if (clip.volume - 1.0).abs() > f64::EPSILON => {
                stage.push_str(&format!(",volume={:.4}", clip.volume.max(0.0)));
            }
            None => {}
        }
        if clip.fade_in > 0.0 {
            stage.push_str(&format!(",afade=t=in:st=0:d={:.4}", clip.fade_in));
        }
        if clip.fade_out > 0.0 {
            let start = (clip.duration - clip.fade_out).max(0.0);
            stage.push_str(&format!(
                ",afade=t=out:st={start:.4}:d={:.4}",
                clip.fade_out
            ));
        }
        if cut {
            stage.push_str(",asetpts=PTS-STARTPTS");
        }

        stage.push_str(&format!(",adelay={delay_ms}:all=1[a{index}]"));
        chains.push(stage);
    }

    let inputs: String = (0..clips.len())
        .map(|index| format!("[a{index}]"))
        .collect();
    let mix = match clips.len() {
        // Nothing to hear: silence, for as long as it is read, so a window
        // with no clip in it still comes out its length.
        0 => format!("anullsrc=r={MIX_RATE}:cl=stereo[mixed]"),
        // amix with one input would still apply its normalisation curve.
        1 => format!("{inputs}anull[mixed]"),
        count => format!("{inputs}amix=inputs={count}:normalize=0[mixed]"),
    };

    // The `aresample` is load-bearing, and not about sample rates.
    //
    // `atempo` emits a fractional number of samples per frame, and the leftover
    // rides along in the timestamps it produces. Mixing such a branch with an
    // untouched one gave `amix` an output whose timestamps eventually came out
    // as AV_NOPTS_VALUE, and the muxer rejected the packet:
    //
    //     non monotonically increasing dts to muxer: 9223372036854775807
    //
    // The effect was a two-second mix written as a ~40 ms file - so any export
    // of more than one clip, where any clip had a speed other than 1 with
    // "preserve pitch" on, produced a silently truncated soundtrack. The
    // exporter reported success either way, which is how it went unnoticed.
    //
    // Passing the mix through the resampler regenerates one continuous
    // timestamp series from the sample count and absorbs the drift. Verified
    // against every shape this function emits: both speed directions, the
    // multi-stage `atempo` used beyond 2x, the `asetrate` path, several clips
    // at once, and the single-clip `anull` path.
    //
    // Do not "simplify" this away because the rate on both sides is MIX_RATE.
    //
    // No `atrim=duration` after the pad. The mix's length used to be cut
    // there, by timestamp, and `amix` stops stamping its output the moment
    // its first input ends - a clip cut short of its file, which is what a
    // split makes - so the trim never saw the end come and the pad went on
    // forever: an export with a split clip never finished. `mix_to_file`
    // counts the samples it takes instead, on the clock it stamps itself.
    //
    // The sound layers go between the two: over the sum, and before the
    // pad, so a chain never runs over the silence the reader takes past
    // the end. Their fades are placed by the stamps, so the mix is
    // restamped from its own sample count first - `N/SR` is the graph's
    // clock, which starts at `origin` - rather than trusting what `amix`
    // and the resampler left on it.
    let mut stages = String::new();
    let mut count = 0;
    for bus in buses.iter().filter(|bus| bus.audible()) {
        validate_chain(&bus.filter_chain)?;
        stages.push(';');
        stages.push_str(&bus_stage(count, bus, origin));
        count += 1;
    }
    chains.push(if count == 0 {
        format!("{mix};[mixed]aresample={MIX_RATE},apad[out]")
    } else {
        format!("{mix};[mixed]aresample={MIX_RATE},asetpts=N/SR/TB[s0]{stages};[s{count}]apad[out]")
    });

    Ok(chains.join(";"))
}

/// The sample format the AAC encoder takes, which the graph's last stage
/// lands on so no conversion happens between them.
const MIX_FORMAT: Sample = Sample::F32(ffmpeg::util::format::sample::Type::Planar);

/// One open input of the mix: its demuxer and decoder, and where in the
/// graph its frames go.
struct MixInput {
    path: std::path::PathBuf,
    input: ffmpeg::format::context::Input,
    stream: usize,
    decoder: ffmpeg::codec::decoder::Audio,
    /// A packet the codec refused while full; see [`crate::feed`].
    held: crate::feed::Held<ffmpeg::Packet>,
    label: String,
    done: bool,
    /// Samples already sent to the graph for this clip.
    samples_sent: i64,
    /// How many samples this clip should contribute to the mix.
    clip_samples: i64,
}

impl MixInput {
    /// One decoded frame, or `None` at the end of the file.
    fn next(&mut self) -> Result<Option<Audio>> {
        if self.done {
            return Ok(None);
        }
        let read = || crate::feed::read_packet(&mut self.input, self.stream);
        match crate::feed::next_frame(&mut self.decoder, &mut self.held, read) {
            Ok(Some(mut frame)) => {
                ffi::name_channels(&mut frame);
                Ok(Some(frame))
            }
            Ok(None) => {
                self.done = true;
                Ok(None)
            }
            Err(stall) => Err(crate::feed::fail(stall, &self.path)),
        }
    }
}

/// The Opus encoder to mix into and the samples it takes: libopus where it
/// is linked, which the desktops' builds and Homebrew carry, and FFmpeg's
/// own otherwise, which every build has - a phone's included. A WebM takes
/// Opus or Vorbis and nothing else, and Opus is the better of the two.
fn opus_encoder() -> Result<(&'static str, Sample)> {
    use ffmpeg::util::format::sample::Type;
    if encoder::find_by_name("libopus").is_some() {
        return Ok(("libopus", Sample::F32(Type::Packed)));
    }
    if encoder::find_by_name("opus").is_some() {
        return Ok(("opus", Sample::F32(Type::Planar)));
    }
    Err(Error::Missing {
        what: "encoder",
        name: "opus".to_owned(),
    })
}

/// Mixes `clips` into one audio file at `destination`: 16-bit PCM for a
/// `.wav` destination (case-insensitive), Opus for a `.webm`, `.opus`,
/// `.ogg` or `.mka` one, AAC otherwise, at 48 kHz stereo.
///
/// Every clip's file must contain an audio stream; see [`mix_graph`].
pub fn mix_to_file(clips: &[AudioClip], duration: f64, destination: &Path) -> Result<()> {
    mix_to_file_with(clips, &[], duration, destination)
}

/// [`mix_to_file`] with sound layers over the mix; see [`AudioBus`].
pub fn mix_to_file_with(
    clips: &[AudioClip],
    buses: &[AudioBus],
    duration: f64,
    destination: &Path,
) -> Result<()> {
    let extension = destination
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
    let wav = extension.as_deref() == Some("wav");
    let opus = matches!(extension.as_deref(), Some("webm" | "opus" | "ogg" | "mka"));
    ffi::init();
    let (codec_name, mix_format) = if wav {
        (
            "pcm_s16le",
            Sample::I16(ffmpeg::util::format::sample::Type::Packed),
        )
    } else if opus {
        opus_encoder()?
    } else {
        ("aac", MIX_FORMAT)
    };
    // A bad chain is refused before any file is opened.
    for chain in clips
        .iter()
        .map(|clip| &clip.filter_chain)
        .chain(buses.iter().map(|bus| &bus.filter_chain))
    {
        validate_chain(chain)?;
    }

    // The encoder, and the file it writes into.
    let codec = encoder::find_by_name(codec_name).ok_or_else(|| Error::Missing {
        what: "encoder",
        name: codec_name.to_owned(),
    })?;
    let mut output = ffmpeg::format::output(destination)
        .map_err(|error| ffi::fail("create", destination, error))?;
    let global_header = output
        .format()
        .flags()
        .contains(ffmpeg::format::Flags::GLOBAL_HEADER);
    let mut audio = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .audio()
        .map_err(|error| ffi::fail("audio encoder", destination, error))?;
    audio.set_rate(MIX_RATE as i32);
    audio.set_format(mix_format);
    audio.set_ch_layout(ChannelLayout::STEREO);
    if !wav {
        audio.set_bit_rate(192_000);
    }
    audio.set_time_base(ffmpeg::Rational::new(1, MIX_RATE as i32));
    if global_header {
        audio.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
    }
    // FFmpeg's own Opus encoder is still marked experimental, and opens
    // only when told that is all right; libopus needs no such word.
    if codec_name == "opus" {
        audio.compliance(ffmpeg::codec::Compliance::Experimental);
    }
    let mut encoder = audio
        .open()
        .map_err(|error| ffi::fail("open encoder", destination, error))?;
    // PCM accepts variable-size frames. Use blocks rather than one sample
    // per packet; run_mix keeps the final block's exact sample count.
    let frame_size = if wav {
        1024
    } else {
        encoder.frame_size().max(1)
    };
    {
        let mut stream = output
            .add_stream(codec)
            .map_err(|error| ffi::fail("add stream", destination, error))?;
        stream.copy_parameters_from_context(&encoder);
        stream.set_time_base(ffmpeg::Rational::new(1, MIX_RATE as i32));
    }
    output
        .write_header()
        .map_err(|error| ffi::fail("write header", destination, error))?;
    let stream_time_base = output
        .stream(0)
        .map(|stream| stream.time_base())
        .unwrap_or(ffmpeg::Rational::new(1, MIX_RATE as i32));

    let encoder_time_base = ffmpeg::Rational::new(1, MIX_RATE as i32);
    let drain = |encoder: &mut encoder::audio::Encoder,
                 output: &mut ffmpeg::format::context::Output|
     -> Result<()> {
        loop {
            let mut packet = ffmpeg::Packet::empty();
            match encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(0);
                    packet.rescale_ts(encoder_time_base, stream_time_base);
                    packet
                        .write_interleaved(output)
                        .map_err(|error| ffi::fail("write packet", destination, error))?;
                }
                Err(ffmpeg::Error::Eof) => return Ok(()),
                Err(error) if ffi::is_again(&error) => return Ok(()),
                Err(error) => return Err(ffi::fail("encode", destination, error)),
            }
        }
    };

    run_mix(
        clips,
        buses,
        (0.0, duration),
        mix_format.name(),
        Some(frame_size),
        destination,
        |mixed| {
            encoder
                .send_frame(mixed)
                .map_err(|error| ffi::fail("encode", destination, error))?;
            drain(&mut encoder, &mut output)
        },
    )?;

    // Flushed and closed whichever way the mix ended: an m4a without its
    // trailer cannot be opened by the muxer.
    encoder
        .send_eof()
        .map_err(|error| ffi::fail("encode", destination, error))?;
    drain(&mut encoder, &mut output)?;
    output
        .write_trailer()
        .map_err(|error| ffi::fail("write trailer", destination, error))
}

/// The mix of the timeline's `[from, to)` - every clip inside it, cut at
/// its edges, with `buses` over the sum - as 16-bit stereo at
/// [`MIX_RATE`], handed to `write` a frame's interleaved bytes at a time.
///
/// What a player that mixes clips itself plays where a sound layer runs:
/// the layer is over the sum, which no one clip's sound can carry, so the
/// span is mixed here, once, the way the export mixes it. A clip a window
/// cuts into keeps its own clock (see `Heard`), so the window's sound is
/// the export's sound over the same stretch.
pub fn mix_pcm(
    clips: &[AudioClip],
    buses: &[AudioBus],
    from: f64,
    to: f64,
    mut write: impl FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    ffi::init();
    for chain in clips
        .iter()
        .map(|clip| &clip.filter_chain)
        .chain(buses.iter().map(|bus| &bus.filter_chain))
    {
        validate_chain(chain)?;
    }
    run_mix(
        clips,
        buses,
        (from, to),
        "s16",
        None,
        Path::new("the mix"),
        |mixed| {
            // Packed: one plane, two channels of two bytes a sample. The
            // plane can be padded past the samples, so it is cut to them.
            let bytes = mixed.samples() * 4;
            let plane = mixed.data(0);
            write(&plane[..bytes.min(plane.len())])
        },
    )
}

/// Opens the clips heard inside `window`, builds their graph and runs it,
/// handing `take` every frame of the mix in `format` (stereo, at
/// [`MIX_RATE`]) until the window's length has been taken. `frame_size`,
/// when given, is what each frame is cut to - an encoder's frame.
fn run_mix(
    clips: &[AudioClip],
    buses: &[AudioBus],
    window: (f64, f64),
    format: &str,
    frame_size: Option<u32>,
    destination: &Path,
    mut take: impl FnMut(&Audio) -> Result<()>,
) -> Result<()> {
    let (from, to) = window;
    let missing = |name: &str| Error::Missing {
        what: "filter",
        name: name.to_owned(),
    };

    // The clips with something inside the window, and how much of each.
    let mut members: Vec<AudioClip> = Vec::new();
    let mut heard: Vec<Heard> = Vec::new();
    for clip in clips {
        if let Some(part) = Heard::inside(clip, from, to) {
            members.push(clip.clone());
            heard.push(part);
        }
    }

    // Open every input first, and read one frame from each: the graph's
    // sources are described by what actually comes out of the decoders.
    let mut inputs: Vec<MixInput> = Vec::with_capacity(members.len());
    let mut first_frames: Vec<Audio> = Vec::with_capacity(members.len());
    let mut graph = filter::Graph::new();
    for (index, clip) in members.iter().enumerate() {
        let path = clip.path.as_path();
        let speed = clamp_speed(clip.speed);
        // Where in the source what is heard of the clip begins.
        let in_point = clip.source_start + heard[index].from * speed;
        let mut input =
            ffmpeg::format::input(path).map_err(|error| ffi::fail("open", path, error))?;
        let stream_index =
            crate::probe::audio_stream_index(&input, clip.stream).ok_or_else(|| {
                Error::NoAudioStream {
                    path: path.to_path_buf(),
                }
            })?;
        let stream = input.stream(stream_index).expect("just found");
        let time_base = stream.time_base();
        // Where the file's clock starts: taken off what is read, put back
        // on the seek, so the in-point is measured from the same instant
        // the picture's is. See `ffi::origin`.
        let start = ffi::origin(&input).as_f64();
        let decoder = ffmpeg::codec::Context::from_parameters(stream.parameters())
            .and_then(|context| context.decoder().audio())
            .map_err(|error| ffi::fail("open decoder", path, error))?;
        // Near the in-point: the packet of the sound at or before it, on the
        // sound's own timestamps. The graph cuts the rest, by the lead
        // measured off the first frame below.
        if in_point > 0.0 {
            let _ = ffi::seek_audio(&mut input, stream_index, in_point + start);
        }
        ffi::read_only(&mut input, stream_index);
        let label = format!("{index}:a");
        let mut mix_input = MixInput {
            path: path.to_path_buf(),
            input,
            stream: stream_index,
            decoder,
            held: None,
            label,
            done: false,
            samples_sent: 0,
            clip_samples: 0,
        };
        let mut first = mix_input.next()?.ok_or_else(|| Error::NoAudioStream {
            path: path.to_path_buf(),
        })?;
        // How far before the in-point the seek landed, by the first frame's
        // own timestamp - the one place a source timestamp is read, and
        // only to place the cut. A frame with none means nothing to cut;
        // one past the in-point is sound that starts after it, which is
        // put that far in with silence, so it stays with its picture.
        let landed = first
            .timestamp()
            .and_then(|ticks| ffi::seconds(ticks, time_base))
            .map_or(in_point, |at| at.as_f64() - start);
        let lead = in_point - landed;
        heard[index].lead = lead;
        // Our continuous PTS: sample count from the start of this clip.
        first.set_pts(Some(0));
        // The out-point lives here, not in the filtergraph: how many source
        // samples this clip may feed, the lead the graph cuts included. A
        // sped-up clip covers more source than its timeline length, hence
        // the speed factor. The first frame goes into the graph separately
        // below, so it counts towards the total.
        let span = heard[index].to - heard[index].from;
        mix_input.samples_sent = first.samples() as i64;
        mix_input.clip_samples =
            ((span * speed + lead).max(0.0) * f64::from(first.rate())).round() as i64;
        // One sample per time-base tick: the PTS we stamp is a plain
        // running sample count, so the graph's clock is the sample clock.
        let args = format!(
            "time_base=1/{}:sample_rate={}:sample_fmt={}:channel_layout={}",
            first.rate(),
            first.rate(),
            first.format().name(),
            first.ch_layout().description()
        );
        graph
            .add(
                &filter::find("abuffer").ok_or_else(|| missing("abuffer"))?,
                &mix_input.label,
                &args,
            )
            .map_err(|error| ffi::fail("buffer source", path, error))?;
        first_frames.push(first);
        inputs.push(mix_input);
    }
    graph
        .add(
            &filter::find("abuffersink").ok_or_else(|| missing("abuffersink"))?,
            "sink",
            "",
        )
        .map_err(|error| ffi::fail("buffer sink", destination, error))?;

    // The plan's `[out]` lands on the reader's own format before the sink.
    let graph_spec = mix_graph_from(&members, &heard, from, buses)?;
    let spec = format!(
        "{graph_spec};[out]aformat=sample_fmts={format}:sample_rates={MIX_RATE}:channel_layouts=stereo[sink]"
    );
    let mut parser = graph
        .input("sink", 0)
        .map_err(|error| ffi::fail("filter graph", destination, error))?;
    for input in &inputs {
        parser = parser
            .output(&input.label, 0)
            .map_err(|error| ffi::fail("filter graph", destination, error))?;
    }
    parser
        .parse(&spec)
        .map_err(|error| ffi::fail("filter graph", destination, error))?;
    graph
        .validate()
        .map_err(|error| ffi::fail("filter graph", destination, error))?;
    if let Some(frame_size) = frame_size {
        let mut sink = graph.get("sink").expect("the graph has a sink");
        sink.sink().set_frame_size(frame_size);
    }

    // Where the mix ends, in samples of our own clock. The graph pads
    // silence past the last clip for as long as it is asked, and its
    // timestamps are not to be trusted for the cut - `amix` drops them once
    // an input ends early - so the count of what has been taken is the
    // whole of the decision, and the last frame is shortened to land on it.
    let total_samples = ((to - from).max(0.0) * f64::from(MIX_RATE)).round() as i64;
    let mut written_samples: i64 = 0;

    // Feed the sources round-robin and drain the sink after every push, so
    // no input runs ahead of the others by more than a frame: what a filter
    // has not consumed yet sits in memory.
    for (index, first) in first_frames.into_iter().enumerate() {
        let mut context = graph.get(&inputs[index].label).expect("source exists");
        context
            .source()
            .add(&first)
            .map_err(|error| ffi::fail("filter", &inputs[index].path, error))?;
    }
    'mix: loop {
        // Pull everything the sink has.
        loop {
            let mut mixed = Audio::empty();
            let pulled = {
                let mut context = graph.get("sink").expect("the graph has a sink");
                context.sink().frame(&mut mixed)
            };
            match pulled {
                Ok(()) => {
                    let remaining = total_samples - written_samples;
                    if remaining <= 0 {
                        break 'mix;
                    }
                    if mixed.samples() as i64 > remaining {
                        mixed.set_samples(remaining as usize);
                    }
                    mixed.set_pts(Some(written_samples));
                    written_samples += mixed.samples() as i64;
                    take(&mixed)?;
                    if written_samples >= total_samples {
                        break 'mix;
                    }
                }
                // The graph ran dry before the timeline's end: nothing left
                // to pad from. Break, do not return: the caller still has
                // its encoder to flush and its trailer to write.
                Err(ffmpeg::Error::Eof) => break 'mix,
                Err(error) if ffi::is_again(&error) => break,
                Err(error) => return Err(ffi::fail("filter output", destination, error)),
            }
        }
        // Push one frame from every input that still has one.
        for input in inputs.iter_mut() {
            if input.done {
                continue;
            }
            match input.next()? {
                Some(mut frame) => {
                    let frame_samples = frame.samples() as i64;
                    // Our continuous PTS: whatever the source's timestamp
                    // series does from here on cannot move this frame out
                    // of its place in the mix.
                    frame.set_pts(Some(input.samples_sent));
                    // Stop feeding this input once we've sent its clip's
                    // worth. The graph pads the rest with silence, and the
                    // sink loop above ends the mix at the timeline's end.
                    if input.samples_sent >= input.clip_samples {
                        let mut context = graph.get(&input.label).expect("source exists");
                        let _ = context.source().flush();
                        input.done = true;
                        continue;
                    }
                    // Scope the add call so its borrow ends before the flush.
                    let added = {
                        let mut context = graph.get(&input.label).expect("source exists");
                        context.source().add(&frame)
                    };
                    input.samples_sent += frame_samples;
                    let mut context = graph.get(&input.label).expect("source exists");
                    match added {
                        Ok(()) => {}
                        // The graph closed this input's slot - its trim ran
                        // out or the mix ended while a frame was in flight.
                        // Take what it got and end the input gracefully
                        // instead of failing the whole mix on a trimmed clip.
                        Err(ffmpeg::Error::Eof) => {
                            let _ = context.source().flush();
                            input.done = true;
                        }
                        Err(error) => return Err(ffi::fail("filter", &input.path, error)),
                    }
                }
                None => {
                    let mut context = graph.get(&input.label).expect("source exists");
                    let _ = context.source().flush();
                    input.done = true;
                }
            }
        }
    }
    Ok(())
}

/// Joins an already-encoded video file and audio file into `output`.
///
/// Streams are copied, not re-encoded: the picture is already exactly as
/// asked, and a second pass would cost time and a generation of quality.
/// The result ends with the shorter of the two.
pub fn mux(video: &Path, audio: &Path, output: &Path) -> Result<()> {
    ffi::init();
    let video_in = ffmpeg::format::input(video).map_err(|error| ffi::fail("open", video, error))?;
    let audio_in = ffmpeg::format::input(audio).map_err(|error| ffi::fail("open", audio, error))?;
    let video_stream = video_in
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or_else(|| Error::NoVideoStream {
            path: video.to_path_buf(),
        })?;
    let audio_stream = audio_in
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .ok_or_else(|| Error::NoAudioStream {
            path: audio.to_path_buf(),
        })?;
    let (video_index, video_tb) = (video_stream.index(), video_stream.time_base());
    let (audio_index, audio_tb) = (audio_stream.index(), audio_stream.time_base());

    let mut out =
        ffmpeg::format::output(output).map_err(|error| ffi::fail("create", output, error))?;
    {
        let mut stream = out
            .add_stream(encoder::find(ffmpeg::codec::Id::None))
            .map_err(|error| ffi::fail("add stream", output, error))?;
        stream.set_parameters(video_stream.parameters());
        stream.set_time_base(video_tb);
        // A WebM's alpha layer is in the packets, but the track only says
        // it has one in its header: without the word a reader takes the
        // picture for opaque and never opens the decoder that reads it.
        if let Some(mode) = video_stream.metadata().get("alpha_mode") {
            stream.metadata_mut().set("alpha_mode", mode);
        }
    }
    {
        let mut stream = out
            .add_stream(encoder::find(ffmpeg::codec::Id::None))
            .map_err(|error| ffi::fail("add stream", output, error))?;
        stream.set_parameters(audio_stream.parameters());
        stream.set_time_base(audio_tb);
    }
    // The index up front, as the picture's own file had it - but not for
    // ProRes, where moving it writes gigabytes a minute again for a file
    // an editor reads whole.
    let header = if video_stream.parameters().id() == ffmpeg::codec::Id::PRORES {
        ffmpeg::Dictionary::new()
    } else {
        ffmpeg::dict! { "movflags" => "+faststart" }
    };
    out.write_header_with(header)
        .map_err(|error| ffi::fail("write header", output, error))?;
    let out_video_tb = out
        .stream(0)
        .map(|stream| stream.time_base())
        .unwrap_or(video_tb);
    let out_audio_tb = out
        .stream(1)
        .map(|stream| stream.time_base())
        .unwrap_or(audio_tb);

    // Whichever stream's next packet is due sooner goes next. The
    // interleaver only orders what it has been handed, and handing it one
    // packet from each in turn let the denser stream fall behind: at 30 fps
    // the sound (a packet every 21.3 ms) ran out of picture packets (every
    // 33.3 ms) two-thirds of the way through, and at 60 fps it was the
    // picture (every 16.7 ms) that stopped at three-quarters, one frame
    // held under the rest of the sound - issue #112. Ordering by due time
    // keeps the two abreast however their packet rates compare, and in
    // seconds rather than ticks, since the two files' time bases differ.
    let mut feeds = [
        Feed::new(video_in, video_index, video_tb, out_video_tb, 0, video),
        Feed::new(audio_in, audio_index, audio_tb, out_audio_tb, 1, audio),
    ];
    loop {
        for feed in &mut feeds {
            feed.fill()?;
        }
        // The file ends with the shorter of the two: once one stream has
        // run out, the other carries on only to where that one ended.
        let cutoff = feeds
            .iter()
            .filter(|feed| feed.next.is_none())
            .map(|feed| feed.end)
            .min();
        let Some(pick) = feeds
            .iter()
            .enumerate()
            .filter(|(_, feed)| feed.next.is_some())
            .min_by_key(|(_, feed)| feed.due())
            .map(|(index, _)| index)
        else {
            break;
        };
        if let Some(cutoff) = cutoff
            && feeds[pick].shown().is_some_and(|shown| shown >= cutoff)
        {
            break;
        }
        feeds[pick].write(&mut out, output)?;
    }
    out.write_trailer()
        .map_err(|error| ffi::fail("write trailer", output, error))
}

/// One input of the mux: its demuxer, which of its streams is wanted, and
/// the packet of that stream waiting to be written.
struct Feed<'a> {
    input: ffmpeg::format::context::Input,
    wanted: usize,
    /// The input stream's time base, and the output stream's.
    from: ffmpeg::Rational,
    to: ffmpeg::Rational,
    /// The output stream the packets go to.
    stream: usize,
    path: &'a Path,
    next: Option<ffmpeg::Packet>,
    done: bool,
    /// When the packet last written stops showing, in seconds.
    end: Rational,
}

impl<'a> Feed<'a> {
    fn new(
        input: ffmpeg::format::context::Input,
        wanted: usize,
        from: ffmpeg::Rational,
        to: ffmpeg::Rational,
        stream: usize,
        path: &'a Path,
    ) -> Self {
        Self {
            input,
            wanted,
            from,
            to,
            stream,
            path,
            next: None,
            done: false,
            end: Rational::ZERO,
        }
    }

    /// Reads ahead to the next packet of the wanted stream, unless one is
    /// already waiting or the input has ended.
    fn fill(&mut self) -> Result<()> {
        while self.next.is_none() && !self.done {
            let mut packet = ffmpeg::Packet::empty();
            match packet.read(&mut self.input) {
                Ok(()) => {
                    if packet.stream() == self.wanted {
                        self.next = Some(packet);
                    }
                }
                Err(ffmpeg::Error::Eof) => self.done = true,
                Err(error) => return Err(ffi::fail("read", self.path, error)),
            }
        }
        Ok(())
    }

    /// When the waiting packet is due, in seconds: its decode time, which
    /// is the order the interleaver wants, or its presentation time when
    /// the container gave no other. None, for a packet with neither, sorts
    /// first, so it goes out at once.
    fn due(&self) -> Option<Rational> {
        let packet = self.next.as_ref()?;
        let ticks = packet.dts().or(packet.pts())?;
        ffi::seconds(ticks, self.from)
    }

    /// When the waiting packet is shown, in seconds.
    fn shown(&self) -> Option<Rational> {
        let packet = self.next.as_ref()?;
        let ticks = packet.pts().or(packet.dts())?;
        ffi::seconds(ticks, self.from)
    }

    /// Writes the waiting packet to the output.
    fn write(&mut self, out: &mut ffmpeg::format::context::Output, output: &Path) -> Result<()> {
        let Some(mut packet) = self.next.take() else {
            return Ok(());
        };
        if let Some(ticks) = packet.pts().or(packet.dts())
            && let Some(end) = ffi::seconds(ticks + packet.duration(), self.from)
        {
            self.end = end;
        }
        packet.set_stream(self.stream);
        packet.rescale_ts(self.from, self.to);
        packet.set_position(-1);
        packet
            .write_interleaved(out)
            .map_err(|error| ffi::fail("write packet", output, error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(path: &str) -> AudioClip {
        AudioClip {
            path: path.into(),
            stream: None,
            start: 0.0,
            duration: 2.0,
            source_start: 0.0,
            speed: 1.0,
            preserve_pitch: true,
            volume: 1.0,
            volume_curve: Track::default(),
            fade_in: 0.0,
            fade_out: 0.0,
            filter_chain: String::new(),
        }
    }

    #[test]
    fn tempo_stages_multiply_back_to_the_requested_rate() {
        for speed in [0.25, 0.5, 0.9, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0] {
            let product: f64 = tempo_stages(speed).iter().product();
            assert!(
                (product - speed).abs() < 1e-9,
                "stages for {speed} multiplied to {product}",
            );
        }
    }

    #[test]
    fn every_stage_is_within_the_filter_range() {
        for speed in [0.0625, 0.25, 0.5, 1.0, 4.0, 8.0, 16.0] {
            for factor in tempo_stages(speed) {
                assert!(
                    (0.5..=2.0).contains(&factor),
                    "{factor} is outside atempo's range, from speed {speed}",
                );
            }
        }
    }

    #[test]
    fn out_of_range_speeds_clamp_rather_than_explode() {
        assert_eq!(clamp_speed(0.0), 0.0625);
        assert_eq!(clamp_speed(100.0), 16.0);
        assert_eq!(clamp_speed(1.0), 1.0);
    }

    #[test]
    fn normal_speed_needs_no_filters() {
        assert!(speed_filters(1.0, true).is_empty());
        assert!(speed_filters(1.0, false).is_empty());
    }

    #[test]
    fn the_tape_effect_lands_on_the_mix_rate_before_shifting() {
        let filters = speed_filters(2.0, false);
        assert_eq!(
            filters[0], "aresample=48000",
            "the shift must start from a known rate"
        );
        assert!(filters[1].starts_with("asetrate=48000*"));
        assert_eq!(filters[2], "aresample=48000");
    }

    #[test]
    fn a_single_clip_avoids_amix_normalisation() {
        let graph = mix_graph(&[clip("a.mp4")]).expect("valid");
        assert!(graph.contains("anull[mixed]"), "graph was: {graph}");
        assert!(!graph.contains("amix"), "graph was: {graph}");
    }

    #[test]
    fn several_clips_mix_without_normalisation() {
        let graph = mix_graph(&[clip("a.mp4"), clip("b.mp4")]).expect("valid");
        assert!(
            graph.contains("amix=inputs=2:normalize=0"),
            "graph was: {graph}"
        );
        assert!(
            graph.contains("[0:a]") && graph.contains("[1:a]"),
            "graph was: {graph}"
        );
    }

    #[test]
    fn the_final_stage_pads_and_leaves_the_end_to_the_reader() {
        let graph = mix_graph(&[clip("a.mp4")]).expect("valid");
        assert!(
            graph.ends_with("aresample=48000,apad[out]"),
            "graph was: {graph}"
        );
        assert!(!graph.contains("atrim=duration"), "graph was: {graph}");
    }

    #[test]
    fn a_sped_up_clip_retimes_in_the_graph_and_is_cut_by_sample_count() {
        let mut fast = clip("a.mp4");
        fast.speed = 2.0;
        let graph = mix_graph(&[fast]).expect("valid");
        // The out-point is `mix_to_file`'s sample counter, and an input
        // that landed on its in-point has nothing to cut.
        assert!(
            graph.contains("[0:a]anull,atempo=2.000000"),
            "graph was: {graph}"
        );
        assert!(!graph.contains("atrim=start"), "graph was: {graph}");
    }

    #[test]
    fn an_input_that_landed_early_is_cut_to_its_in_point() {
        let clips = [clip("a.mp4"), clip("b.mp4")];
        let mut heard: Vec<Heard> = clips.iter().map(Heard::whole).collect();
        heard[0].lead = 0.0213;
        let graph = mix_graph_from(&clips, &heard, 0.0, &[]).expect("valid");
        assert!(
            graph.contains("[0:a]atrim=start=0.021300,asetpts=PTS-STARTPTS,"),
            "graph was: {graph}"
        );
        assert!(graph.contains("[1:a]anull,"), "graph was: {graph}");
    }

    #[test]
    fn fades_and_volume_apply_after_the_filter_chain() {
        let mut shaped = clip("a.mp4");
        shaped.filter_chain = "highpass=f=80".to_owned();
        shaped.volume = 0.5;
        shaped.fade_out = 0.25;
        let graph = mix_graph(&[shaped]).expect("valid");

        let chain_at = graph.find("highpass").expect("chain present");
        let volume_at = graph.find(",volume=").expect("volume present");
        let fade_at = graph.find(",afade=t=out").expect("fade present");
        assert!(
            chain_at < volume_at && volume_at < fade_at,
            "graph was: {graph}"
        );
    }

    #[test]
    fn a_chain_that_escapes_its_slot_is_refused() {
        for bad in ["volume=2;amovie=x[a]", "anull[a1]", "a\nb"] {
            let mut hostile = clip("a.mp4");
            hostile.filter_chain = bad.to_owned();
            assert!(
                matches!(mix_graph(&[hostile]), Err(Error::InvalidFilterChain { .. })),
                "{bad:?} should have been refused",
            );
        }
    }

    /// Silence for `seconds`, as the AAC file the mix writes.
    fn silent_aac(destination: &Path, seconds: f64) {
        ffi::init();
        let codec = encoder::find_by_name("aac").expect("the linked FFmpeg encodes aac");
        let mut output = ffmpeg::format::output(destination).expect("creates");
        let mut audio = ffmpeg::codec::Context::new_with_codec(codec)
            .encoder()
            .audio()
            .expect("audio encoder");
        audio.set_rate(MIX_RATE as i32);
        audio.set_format(MIX_FORMAT);
        audio.set_ch_layout(ChannelLayout::STEREO);
        audio.set_time_base(ffmpeg::Rational::new(1, MIX_RATE as i32));
        if output
            .format()
            .flags()
            .contains(ffmpeg::format::Flags::GLOBAL_HEADER)
        {
            audio.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        }
        let mut encoder = audio.open().expect("opens");
        let frame_size = encoder.frame_size().max(1) as usize;
        {
            let mut stream = output.add_stream(codec).expect("adds a stream");
            stream.copy_parameters_from_context(&encoder);
            stream.set_time_base(ffmpeg::Rational::new(1, MIX_RATE as i32));
        }
        output.write_header().expect("writes the header");
        let stream_time_base = output.stream(0).expect("one stream").time_base();
        let drain = |encoder: &mut encoder::audio::Encoder,
                     output: &mut ffmpeg::format::context::Output| {
            loop {
                let mut packet = ffmpeg::Packet::empty();
                match encoder.receive_packet(&mut packet) {
                    Ok(()) => {
                        packet.set_stream(0);
                        packet.rescale_ts(
                            ffmpeg::Rational::new(1, MIX_RATE as i32),
                            stream_time_base,
                        );
                        packet.write_interleaved(output).expect("writes a packet");
                    }
                    Err(_) => return,
                }
            }
        };
        let total = (seconds * f64::from(MIX_RATE)) as i64;
        let mut written: i64 = 0;
        while written < total {
            let mut frame = Audio::new(
                MIX_FORMAT,
                frame_size,
                ffmpeg::util::channel_layout::ChannelLayoutMask::STEREO,
            );
            frame.set_ch_layout(ChannelLayout::STEREO);
            frame.set_rate(MIX_RATE);
            // The buffer comes uninitialised, and the encoder refuses a
            // NaN: silence is written, one plane a channel. By sample, not
            // by byte: the byte view knows the length of plane 0 only.
            for plane in 0..frame.planes() {
                frame.plane_mut::<f32>(plane).fill(0.0);
            }
            frame.set_pts(Some(written));
            written += frame_size as i64;
            encoder.send_frame(&frame).expect("encodes");
            drain(&mut encoder, &mut output);
        }
        encoder.send_eof().expect("flushes");
        drain(&mut encoder, &mut output);
        output.write_trailer().expect("writes the trailer");
    }

    /// Packets per output stream, in stream order.
    fn packets_of(path: &Path) -> Vec<usize> {
        let mut input = ffmpeg::format::input(path).expect("opens");
        let mut counts = vec![0; input.nb_streams() as usize];
        for item in input.packets() {
            let (stream, _) = item.expect("reads");
            counts[stream.index()] += 1;
        }
        counts
    }

    /// Issue #112: a 60 fps export froze at three-quarters of its length.
    /// The picture's packets come denser than the sound's, and copying one
    /// from each in turn had the sound reach its end first, which ended the
    /// file with a quarter of the picture unwritten.
    #[test]
    fn the_mux_keeps_the_whole_picture_when_its_packets_come_denser_than_the_sound() {
        use crate::{EncodeOptions, Encoder, FrameSink};
        use concat_core::frame::Frame;
        use concat_core::time::FrameRate;

        let dir = std::env::temp_dir();
        let video = dir.join("concat-mux-test-picture.mp4");
        let audio = dir.join("concat-mux-test-sound.m4a");
        let output = dir.join("concat-mux-test-joined.mp4");

        // Four seconds: 240 picture packets against about 190 of sound.
        let frames = 240;
        let options = EncodeOptions {
            preset: "ultrafast".to_owned(),
            ..EncodeOptions::default()
        };
        let mut encoder = Encoder::create(&video, 64, 64, FrameRate::SIXTY, &options)
            .expect("the linked FFmpeg encodes h264");
        let frame = Frame::black(64, 64);
        for _ in 0..frames {
            encoder.write_frame(&frame).expect("writes");
        }
        encoder.finish().expect("finishes");
        silent_aac(&audio, 4.0);

        let result = mux(&video, &audio, &output);
        let counts = packets_of(&output);
        let _ = std::fs::remove_file(&video);
        let _ = std::fs::remove_file(&audio);
        let _ = std::fs::remove_file(&output);
        result.expect("muxes");

        assert_eq!(counts[0], frames, "every picture packet is in the file");
        assert!(
            counts[1] > 150,
            "the sound is there too, not {} packets",
            counts[1]
        );
    }

    /// The export's mix keeps a file's sound with its picture whichever
    /// starts first: each fixture sounds a tone half a second after its
    /// first frame, and so does its clip in the mix, whole or cut 0.2 s in
    /// (#457).
    #[test]
    fn the_mix_keeps_sound_with_its_picture_whichever_starts_first() {
        use crate::{AudioDecoder, AudioOptions, SampleFormat};
        let output =
            std::env::temp_dir().join(format!("concat-mix-origin-{}.m4a", std::process::id()));
        for fixture in ["picture-starts-late.mov", "sound-starts-late.mov"] {
            for (source_start, want) in [(0.0, 0.5), (0.2, 0.3)] {
                let source = format!("{}/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
                let piece = AudioClip {
                    duration: 1.0,
                    source_start,
                    ..clip(&source)
                };
                mix_to_file(&[piece], 1.0, &output).expect("mixes");
                let options = AudioOptions {
                    rate: 8_000,
                    channels: 1,
                    format: SampleFormat::F32,
                    ..AudioOptions::default()
                };
                let mut decoder = AudioDecoder::open(&output, &options).expect("opens the mix");
                let mut samples: Vec<f32> = Vec::new();
                while let Some(frame) = decoder.next_frame().expect("decodes") {
                    let bytes = &frame.data(0)[..frame.samples() * 4];
                    samples.extend(
                        bytes
                            .chunks_exact(4)
                            .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])),
                    );
                }
                let onset = samples
                    .iter()
                    .position(|sample| sample.abs() > 0.05)
                    .expect("the tone is in the mix") as f64
                    / 8_000.0;
                assert!(
                    (onset - want).abs() < 0.015,
                    "{fixture} cut {source_start} s in: the tone is at {onset:.3} s, not {want}"
                );
            }
        }
        let _ = std::fs::remove_file(&output);
    }

    /// A trimmed clip's mix ends when the timeline does. Two pieces of one
    /// file, the second cut in at two seconds, mixed to sixteen: the mix
    /// must come out, and be sixteen seconds long.
    #[test]
    fn a_mix_of_a_trimmed_clip_ends_with_the_timeline() {
        let dir = std::env::temp_dir();
        let source = dir.join("concat-mix-trim-source.m4a");
        let output = dir.join("concat-mix-trim-out.m4a");
        silent_aac(&source, 6.0);
        let piece = |start: f64, source_start: f64, duration: f64| AudioClip {
            path: source.clone(),
            stream: None,
            start,
            duration,
            source_start,
            speed: 1.0,
            preserve_pitch: true,
            volume: 1.0,
            volume_curve: Track::default(),
            fade_in: 0.0,
            fade_out: 0.0,
            filter_chain: String::new(),
        };
        let clips = [piece(0.0, 0.0, 2.0), piece(2.0, 2.0, 4.0)];
        let target = output.clone();
        let worker = std::thread::spawn(move || mix_to_file(&clips, 16.0, &target));
        let started = std::time::Instant::now();
        while !worker.is_finished() {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(60),
                "the mix did not end: it is still writing after a minute"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        worker.join().expect("no panic").expect("mixes");
        let info = crate::probe(&output).expect("probes the mix");
        let _ = std::fs::remove_file(&source);
        let _ = std::fs::remove_file(&output);
        let seconds = info.duration.expect("a duration").as_f64();
        assert!(
            (seconds - 16.0).abs() < 0.1,
            "the mix is {seconds}s, not 16s"
        );
    }

    fn bus(start: f64, duration: f64, chain: &str) -> AudioBus {
        AudioBus {
            start,
            duration,
            filter_chain: chain.to_owned(),
            strength: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
        }
    }

    #[test]
    fn a_bus_runs_over_the_sum_between_the_resampler_and_the_pad() {
        let graph = mix_graph_with(
            &[clip("a.mp4"), clip("b.mp4")],
            &[bus(1.0, 2.0, "lowpass=f=300")],
        )
        .expect("valid");
        let mixed = graph
            .find("[mixed]aresample=48000,asetpts=N/SR/TB[s0]")
            .expect("into the bus");
        let chain = graph.find("lowpass=f=300").expect("the chain");
        let pad = graph.find("[s1]apad[out]").expect("out of the bus");
        assert!(mixed < chain && chain < pad, "graph was: {graph}");
        // Fed only what plays inside its span, and given back there.
        assert!(
            graph.contains(
                "afade=t=in:st=1.000000:d=0.002000,afade=t=out:st=2.998000:d=0.002000,lowpass"
            ),
            "graph was: {graph}"
        );
        assert!(
            graph.contains("amix=inputs=3:normalize=0"),
            "graph was: {graph}"
        );
    }

    #[test]
    fn a_bus_with_nothing_to_do_leaves_the_graph_as_it_was() {
        let plain = mix_graph(&[clip("a.mp4")]).expect("valid");
        let mut idle = bus(0.0, 1.0, "lowpass=f=300");
        idle.strength = 0.0;
        for buses in [vec![idle], vec![bus(0.0, 1.0, "")]] {
            assert_eq!(
                mix_graph_with(&[clip("a.mp4")], &buses).expect("valid"),
                plain
            );
        }
    }

    #[test]
    fn a_bus_chain_that_escapes_its_slot_is_refused() {
        assert!(matches!(
            mix_graph_with(&[clip("a.mp4")], &[bus(0.0, 1.0, "anull[x]")]),
            Err(Error::InvalidFilterChain { .. })
        ));
    }

    /// A clip a window cuts into keeps its own clock: its fades are read
    /// from where it starts, the stamps moved on by what was cut.
    #[test]
    fn a_clip_cut_by_a_window_is_shaped_on_its_own_clock() {
        let mut shaped = clip("a.mp4");
        shaped.start = 1.0;
        shaped.fade_in = 0.5;
        let heard = Heard::inside(&shaped, 1.25, 4.0).expect("inside");
        assert_eq!((heard.from, heard.to), (0.25, 2.0));
        let graph = mix_graph_from(&[shaped], &[heard], 1.25, &[]).expect("valid");
        assert!(
            graph.contains("asetpts=PTS-STARTPTS+0.250000/TB,afade=t=in:st=0:d=0.5000,asetpts=PTS-STARTPTS,adelay=0:all=1"),
            "graph was: {graph}"
        );
        assert!(Heard::inside(&clip("a.mp4"), 2.0, 3.0).is_none());
    }

    /// A 440 Hz tone at half scale, stereo 16-bit, as a WAV of `seconds`.
    fn tone_wav(path: &Path, seconds: f64) {
        let count = (seconds * f64::from(MIX_RATE)) as usize;
        let mut data = Vec::with_capacity(count * 4);
        for index in 0..count {
            let t = index as f64 / f64::from(MIX_RATE);
            let value = (0.5 * (2.0 * std::f64::consts::PI * 440.0 * t).sin() * 32767.0) as i16;
            data.extend_from_slice(&value.to_le_bytes());
            data.extend_from_slice(&value.to_le_bytes());
        }
        let mut bytes = Vec::with_capacity(44 + data.len());
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&MIX_RATE.to_le_bytes());
        bytes.extend_from_slice(&(MIX_RATE * 4).to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&data);
        std::fs::write(path, bytes).expect("writes the wav");
    }

    #[test]
    fn saving_audio_as_wav_writes_pcm_that_decodes_with_the_expected_samples() {
        use crate::{AudioDecoder, AudioOptions};

        let dir = std::env::temp_dir().join(format!("concat-save-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("creates a folder");
        let source = dir.join("tone.wav");
        tone_wav(&source, 1.0);
        let video = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("sound-starts-late.mov");
        // A partial final encoder block must not add or discard samples.
        let duration = 6007.0 / f64::from(MIX_RATE);
        for (input, extension) in [(&source, "wav"), (&video, "WAV")] {
            let piece = AudioClip {
                path: input.clone(),
                source_start: 0.6,
                duration,
                volume: 0.5,
                fade_in: 0.02,
                fade_out: 0.03,
                ..clip("")
            };
            let clips = [piece];
            let output = dir.join(format!("saved.{extension}"));
            mix_to_file(&clips, duration, &output).expect("saves the WAV");
            {
                let input = ffmpeg::format::input(&output).expect("opens the saved file");
                let stream = input
                    .streams()
                    .best(ffmpeg::media::Type::Audio)
                    .expect("has audio");
                assert_eq!(stream.parameters().id(), ffmpeg::codec::Id::PCM_S16LE);
            }
            let bytes = std::fs::read(&output).expect("reads the WAV");
            assert_eq!(&bytes[..4], b"RIFF");
            assert_eq!(&bytes[8..12], b"WAVE");
            let mut decoder = AudioDecoder::open(&output, &AudioOptions::default())
                .expect("Concat opens the saved WAV");
            let mut actual = Vec::new();
            while let Some(samples) = decoder.next_i16().expect("Concat decodes the WAV") {
                actual.extend(samples);
            }
            assert_eq!(actual.len(), 6007 * 2, "exact stereo sample count");
            assert!(actual.iter().any(|sample| i32::from(*sample).abs() > 1000));

            let mut expected = Vec::new();
            mix_pcm(&clips, &[], 0.0, duration, |chunk| {
                expected.extend_from_slice(chunk);
                Ok(())
            })
            .expect("mixes the reference PCM");
            let expected: Vec<i16> = expected
                .chunks_exact(2)
                .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
                .collect();
            assert_eq!(actual.len(), expected.len());
            assert!(
                actual
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| { (i32::from(*a) - i32::from(*b)).abs() <= 2 }),
                "the saved audio preserves the cut, gain and fades"
            );
        }
        std::fs::remove_dir_all(dir).expect("removes test files");
    }

    /// An overlay with alpha and its sound, joined the way an export joins
    /// them: a ProRes picture with AAC in a .mov, a VP9 one with Opus in a
    /// .webm, the only sound a WebM takes besides Vorbis. The joined WebM
    /// still says it has an alpha layer, so the import reads it with the
    /// decoder that keeps it.
    #[test]
    fn an_overlay_with_alpha_takes_its_sound_in_its_own_container() {
        use crate::decode::{DecodeOptions, Decoder, FrameSource};
        use crate::{AudioDecoder, AudioOptions, EncodeOptions, Encoder, FrameSink, VideoCodec};
        use concat_core::frame::Frame;
        use concat_core::time::FrameRate;

        let dir = std::env::temp_dir().join(format!("concat-alpha-mux-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("creates a folder");
        let tone = dir.join("tone.wav");
        tone_wav(&tone, 1.0);
        for codec in [VideoCodec::ProRes4444, VideoCodec::Vp9Alpha] {
            if !codec.available() {
                eprintln!("{} not in the linked FFmpeg; skipped", codec.label());
                continue;
            }
            let container = codec.container();
            let webm = container == "webm";
            let video = dir.join(format!("picture.{container}"));
            let sound = dir.join(if webm { "sound.webm" } else { "sound.m4a" });
            let output = dir.join(format!("joined.{container}"));

            // A second of a frame clear on its right half.
            let mut frame = Frame::transparent(64, 64);
            for y in 0..64 {
                for x in 0..32 {
                    frame.set_pixel(x, y, [200, 40, 40, 255]);
                }
            }
            let options = EncodeOptions {
                codec,
                preset: "ultrafast".to_owned(),
                hardware: false,
                ..EncodeOptions::default()
            };
            let mut encoder =
                Encoder::create(&video, 64, 64, FrameRate::THIRTY, &options).expect("opens");
            for _ in 0..30 {
                encoder.write_frame(&frame).expect("writes");
            }
            encoder.finish().expect("finishes");
            let piece = AudioClip {
                path: tone.clone(),
                duration: 1.0,
                ..clip("")
            };
            mix_to_file(&[piece], 1.0, &sound).expect("mixes");
            mux(&video, &sound, &output).expect("muxes");

            let counts = packets_of(&output);
            assert_eq!(counts[0], 30, "{}: every picture packet", codec.label());
            {
                let input = ffmpeg::format::input(&output).expect("opens");
                let picture = input
                    .streams()
                    .best(ffmpeg::media::Type::Video)
                    .expect("video");
                if webm {
                    assert_eq!(
                        picture.metadata().get("alpha_mode"),
                        Some("1"),
                        "the joined track still says it has alpha"
                    );
                }
                let heard = input
                    .streams()
                    .best(ffmpeg::media::Type::Audio)
                    .expect("audio");
                let wanted = if webm {
                    ffmpeg::codec::Id::OPUS
                } else {
                    ffmpeg::codec::Id::AAC
                };
                assert_eq!(heard.parameters().id(), wanted, "{}", codec.label());
            }
            let mut decoder =
                Decoder::open(&output, &DecodeOptions::default().in_software()).expect("opens");
            let back = decoder.next_frame().expect("decodes").expect("a frame");
            let [.., opaque] = back.pixel(16, 32).expect("inside");
            let [.., clear] = back.pixel(48, 32).expect("inside");
            assert!(opaque > 250, "{}: opaque came back {opaque}", codec.label());
            assert!(clear < 8, "{}: clear came back {clear}", codec.label());
            let mut sound = AudioDecoder::open(&output, &AudioOptions::default()).expect("opens");
            let mut audible = false;
            while let Some(samples) = sound.next_i16().expect("decodes") {
                audible |= samples.iter().any(|sample| i32::from(*sample).abs() > 1000);
            }
            assert!(audible, "{}: the tone is heard", codec.label());
        }
        std::fs::remove_dir_all(dir).expect("removes test files");
    }

    #[test]
    fn saving_to_m4a_keeps_aac_encoding_and_audible_samples() {
        use crate::{AudioDecoder, AudioOptions};

        let dir = std::env::temp_dir().join(format!("concat-save-aac-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("creates a folder");
        let source = dir.join("tone.wav");
        tone_wav(&source, 0.25);
        let piece = AudioClip {
            path: source,
            duration: 0.25,
            ..clip("")
        };
        let output = dir.join("saved.m4a");
        mix_to_file(&[piece], 0.25, &output).expect("saves AAC");
        {
            let input = ffmpeg::format::input(&output).expect("opens the saved file");
            let stream = input
                .streams()
                .best(ffmpeg::media::Type::Audio)
                .expect("has audio");
            assert_eq!(stream.parameters().id(), ffmpeg::codec::Id::AAC);
        }
        {
            let mut decoder =
                AudioDecoder::open(&output, &AudioOptions::default()).expect("opens the saved AAC");
            let mut audible = false;
            while let Some(samples) = decoder.next_i16().expect("decodes AAC") {
                audible |= samples.iter().any(|sample| i32::from(*sample).abs() > 1000);
            }
            assert!(audible);
        }
        std::fs::remove_dir_all(dir).expect("removes test files");
    }

    /// The left channel of `mix_pcm` over `[from, to)`.
    fn left(clips: &[AudioClip], buses: &[AudioBus], from: f64, to: f64) -> Vec<i16> {
        let mut bytes = Vec::new();
        mix_pcm(clips, buses, from, to, |chunk| {
            bytes.extend_from_slice(chunk);
            Ok(())
        })
        .expect("mixes");
        bytes
            .chunks_exact(4)
            .map(|frame| i16::from_le_bytes([frame[0], frame[1]]))
            .collect()
    }

    /// The loudest sample difference between `a` and `b` over seconds
    /// `[from, to)` of both.
    fn worst(a: &[i16], b: &[i16], from: f64, to: f64) -> i32 {
        let at = |seconds: f64| (seconds * f64::from(MIX_RATE)) as usize;
        (at(from)..at(to).min(a.len()).min(b.len()))
            .map(|index| (i32::from(a[index]) - i32::from(b[index])).abs())
            .max()
            .unwrap_or(0)
    }

    /// A sound layer changes the mix over its span and nowhere else, and a
    /// window of the mix - what playback plays where a layer runs - is the
    /// whole mix over the same stretch.
    #[test]
    fn a_bus_treats_its_span_alone_and_a_window_of_it_is_the_whole_mix_there() {
        let dir = std::env::temp_dir().join(format!("concat-bus-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let source = dir.join("tone.wav");
        tone_wav(&source, 4.0);
        let mut tone = clip("tone.wav");
        tone.path = source.clone();
        tone.duration = 4.0;
        let clips = [tone];
        // A low-pass far under the tone all but takes it away.
        let buses = [bus(1.0, 2.0, "lowpass=f=60,lowpass=f=60")];

        let plain = left(&clips, &[], 0.0, 4.0);
        let treated = left(&clips, &buses, 0.0, 4.0);
        let window = left(&clips, &buses, 0.5, 3.5);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(plain.len(), 4 * MIX_RATE as usize);
        assert_eq!(treated.len(), plain.len());
        assert_eq!(window.len(), 3 * MIX_RATE as usize);
        // Untouched before the span and after it, sample for sample.
        assert_eq!(worst(&plain, &treated, 0.0, 0.999), 0);
        assert_eq!(worst(&plain, &treated, 3.001, 4.0), 0);
        // Inside it the tone is all but gone.
        let peak = |samples: &[i16], from: f64, to: f64| {
            let at = |seconds: f64| (seconds * f64::from(MIX_RATE)) as usize;
            samples[at(from)..at(to)]
                .iter()
                .map(|sample| i32::from(*sample).abs())
                .max()
                .unwrap_or(0)
        };
        assert!(
            peak(&plain, 1.2, 2.8) > 15_000,
            "the tone is there untreated"
        );
        assert!(
            peak(&treated, 1.2, 2.8) < 3_000,
            "the tone came through at {}",
            peak(&treated, 1.2, 2.8)
        );
        // The window is the whole mix from half a second on.
        let offset = (0.5 * f64::from(MIX_RATE)) as usize;
        let difference = worst(&treated[offset..], &window, 0.0, 3.0);
        assert!(difference <= 1, "the window differs by {difference}");
    }

    /// A clip trimmed into a Program Stream mixes from its first sample,
    /// not from behind the in-point. The fixture's sound is interleaved
    /// behind its picture, and the mix's seek used to be made on the
    /// picture's timestamps, forward: it landed past the in-point and the
    /// overshoot went in as silence in front of the clip. The tone has
    /// sounded from half a second in, so a cut at 3.5 s is inside it.
    #[test]
    fn a_trimmed_program_stream_clip_mixes_from_its_first_sample() {
        let path = format!(
            "{}/fixtures/sound-behind-the-picture.mpg",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut piece = clip(&path);
        piece.duration = 0.6;
        piece.source_start = 3.5;
        let mixed = left(&[piece], &[], 0.0, 0.6);
        assert_eq!(mixed.len(), (0.6 * f64::from(MIX_RATE)) as usize);
        let first = mixed
            .iter()
            .position(|sample| i32::from(*sample).abs() > 1_500)
            .expect("the tone is in the mix");
        assert!(
            first < 48 * 30,
            "the tone starts {} samples in, not at once",
            first
        );
    }

    #[test]
    fn ordinary_chains_pass_validation() {
        for good in [
            "",
            "highpass=f=80,equalizer=f=300:t=q:w=1.1:g=-1.4",
            "volume=0.5",
        ] {
            assert!(validate_chain(good).is_ok(), "{good:?} should be fine");
        }
    }
}

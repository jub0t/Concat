// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

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
/// clip into one `[out]` stream of exactly `duration` seconds.
///
/// Input `N` is `clips[N].path`, bound to the label `[N:a]`, and every input
/// must actually contain an audio stream - the graph names `[N:a]` on it
/// rather than treating silence as a stream. The caller decides membership
/// by probing; this function decides what the mix means.
pub fn mix_graph(clips: &[AudioClip], duration: f64) -> Result<String> {
    let mut chains: Vec<String> = Vec::new();

    for (index, clip) in clips.iter().enumerate() {
        validate_chain(&clip.filter_chain)?;
        let delay_ms = (clip.start * 1000.0).round().max(0.0) as i64;
        let speed = clamp_speed(clip.speed);

        // Each input is trimmed to its in-point, restamped from zero, then
        // delayed to where it sits on the timeline. `all=1` applies the delay
        // to every channel; without it only the left channel moves, which is
        // a memorable way to discover the flag exists.
        //
        // A sped-up clip covers more source than its timeline length, so the
        // trim takes `duration * speed` before the rate is applied.
        let mut stage = format!(
            "[{index}:a]atrim=start={:.6}:duration={:.6},asetpts=PTS-STARTPTS",
            clip.source_start,
            clip.duration * speed
        );

        for filter in speed_filters(speed, clip.preserve_pitch) {
            stage.push(',');
            stage.push_str(&filter);
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

        stage.push_str(&format!(",adelay={delay_ms}:all=1[a{index}]"));
        chains.push(stage);
    }

    let inputs: String = (0..clips.len())
        .map(|index| format!("[a{index}]"))
        .collect();
    let mix = if clips.len() == 1 {
        // amix with one input would still apply its normalisation curve.
        format!("{inputs}anull[mixed]")
    } else {
        format!("{inputs}amix=inputs={}:normalize=0[mixed]", clips.len())
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
    chains.push(format!(
        "{mix};[mixed]aresample={MIX_RATE},apad,atrim=duration={duration:.6}[out]"
    ));

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
    label: String,
    done: bool,
}

impl MixInput {
    /// One decoded frame, or `None` at the end of the file.
    fn next(&mut self) -> Result<Option<Audio>> {
        if self.done {
            return Ok(None);
        }
        loop {
            let mut frame = Audio::empty();
            match self.decoder.receive_frame(&mut frame) {
                Ok(()) => return Ok(Some(frame)),
                Err(ffmpeg::Error::Eof) => {
                    self.done = true;
                    return Ok(None);
                }
                Err(error) if ffi::is_again(&error) => {}
                Err(error) => return Err(ffi::fail("decode", &self.path, error)),
            }
            loop {
                let mut packet = ffmpeg::Packet::empty();
                match packet.read(&mut self.input) {
                    Ok(()) => {
                        if packet.stream() != self.stream {
                            continue;
                        }
                        match self.decoder.send_packet(&packet) {
                            Ok(()) => break,
                            Err(error) if ffi::is_again(&error) => break,
                            Err(error) => return Err(ffi::fail("send packet", &self.path, error)),
                        }
                    }
                    Err(ffmpeg::Error::Eof) => {
                        let _ = self.decoder.send_eof();
                        break;
                    }
                    Err(error) => return Err(ffi::fail("read", &self.path, error)),
                }
            }
        }
    }
}

/// Mixes `clips` into one AAC file at `destination`.
///
/// Every clip's file must contain an audio stream; see [`mix_graph`].
pub fn mix_to_file(clips: &[AudioClip], duration: f64, destination: &Path) -> Result<()> {
    ffi::init();
    let graph_spec = mix_graph(clips, duration)?;
    let missing = |name: &str| Error::Missing {
        what: "filter",
        name: name.to_owned(),
    };

    // Open every input first, and read one frame from each: the graph's
    // sources are described by what actually comes out of the decoders.
    let mut inputs: Vec<MixInput> = Vec::with_capacity(clips.len());
    let mut first_frames: Vec<Audio> = Vec::with_capacity(clips.len());
    let mut graph = filter::Graph::new();
    for (index, clip) in clips.iter().enumerate() {
        let path = clip.path.as_path();
        let mut input =
            ffmpeg::format::input(path).map_err(|error| ffi::fail("open", path, error))?;
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Audio)
            .ok_or_else(|| Error::NoAudioStream {
                path: path.to_path_buf(),
            })?;
        let stream_index = stream.index();
        let time_base = stream.time_base();
        let decoder = ffmpeg::codec::Context::from_parameters(stream.parameters())
            .and_then(|context| context.decoder().audio())
            .map_err(|error| ffi::fail("open decoder", path, error))?;
        // Near the in-point; atrim in the graph does the exact cut.
        if clip.source_start > 0.0 {
            let target = (clip.source_start * f64::from(ffmpeg::sys::AV_TIME_BASE)) as i64;
            let _ = input.seek(target, ..=target);
        }
        let label = format!("{index}:a");
        let mut mix_input = MixInput {
            path: path.to_path_buf(),
            input,
            stream: stream_index,
            decoder,
            label,
            done: false,
        };
        let first = mix_input.next()?.ok_or_else(|| Error::NoAudioStream {
            path: path.to_path_buf(),
        })?;
        let args = format!(
            "time_base={}/{}:sample_rate={}:sample_fmt={}:channel_layout={}",
            time_base.numerator(),
            time_base.denominator(),
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

    // The plan's `[out]` lands on the encoder's own format before the sink.
    let spec = format!(
        "{graph_spec};[out]aformat=sample_fmts=fltp:sample_rates={MIX_RATE}:channel_layouts=stereo[sink]"
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

    // The encoder, and the file it writes into.
    let codec = encoder::find_by_name("aac").ok_or_else(|| Error::Missing {
        what: "encoder",
        name: "aac".to_owned(),
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
    audio.set_format(MIX_FORMAT);
    audio.set_ch_layout(ChannelLayout::STEREO);
    audio.set_bit_rate(192_000);
    audio.set_time_base(ffmpeg::Rational::new(1, MIX_RATE as i32));
    if global_header {
        audio.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
    }
    let mut encoder = audio
        .open()
        .map_err(|error| ffi::fail("open encoder", destination, error))?;
    let frame_size = encoder.frame_size().max(1);
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
    {
        let mut sink = graph.get("sink").expect("the graph has a sink");
        sink.sink().set_frame_size(frame_size);
    }

    let encoder_time_base = ffmpeg::Rational::new(1, MIX_RATE as i32);
    let mut written_samples: i64 = 0;
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
    let mut live = inputs.len();
    loop {
        // Pull everything the sink has.
        loop {
            let mut mixed = Audio::empty();
            let pulled = {
                let mut context = graph.get("sink").expect("the graph has a sink");
                context.sink().frame(&mut mixed)
            };
            match pulled {
                Ok(()) => {
                    mixed.set_pts(Some(written_samples));
                    written_samples += mixed.samples() as i64;
                    encoder
                        .send_frame(&mixed)
                        .map_err(|error| ffi::fail("encode", destination, error))?;
                    drain(&mut encoder, &mut output)?;
                }
                Err(ffmpeg::Error::Eof) => {
                    live = 0;
                    break;
                }
                Err(error) if ffi::is_again(&error) => break,
                Err(error) => return Err(ffi::fail("filter output", destination, error)),
            }
        }
        if live == 0 {
            break;
        }
        // Push one frame from every input that still has one.
        for input in inputs.iter_mut() {
            if input.done {
                continue;
            }
            match input.next()? {
                Some(frame) => {
                    let mut context = graph.get(&input.label).expect("source exists");
                    context
                        .source()
                        .add(&frame)
                        .map_err(|error| ffi::fail("filter", &input.path, error))?;
                }
                None => {
                    let mut context = graph.get(&input.label).expect("source exists");
                    let _ = context.source().flush();
                    live -= 1;
                }
            }
        }
        // `live` counting down to zero means every source was flushed; the
        // sink then drains to EOF on the next pass, which is what ends the
        // outer loop.
        if live == 0 {
            loop {
                let mut mixed = Audio::empty();
                let pulled = {
                    let mut context = graph.get("sink").expect("the graph has a sink");
                    context.sink().frame(&mut mixed)
                };
                match pulled {
                    Ok(()) => {
                        mixed.set_pts(Some(written_samples));
                        written_samples += mixed.samples() as i64;
                        encoder
                            .send_frame(&mixed)
                            .map_err(|error| ffi::fail("encode", destination, error))?;
                        drain(&mut encoder, &mut output)?;
                    }
                    Err(ffmpeg::Error::Eof) => break,
                    Err(error) if ffi::is_again(&error) => break,
                    Err(error) => return Err(ffi::fail("filter output", destination, error)),
                }
            }
            break;
        }
    }

    encoder
        .send_eof()
        .map_err(|error| ffi::fail("encode", destination, error))?;
    drain(&mut encoder, &mut output)?;
    output
        .write_trailer()
        .map_err(|error| ffi::fail("write trailer", destination, error))
}

/// Joins an already-encoded video file and audio file into `output`.
///
/// Streams are copied, not re-encoded: the picture is already exactly as
/// asked, and a second pass would cost time and a generation of quality.
/// The result ends with the shorter of the two.
pub fn mux(video: &Path, audio: &Path, output: &Path) -> Result<()> {
    ffi::init();
    let mut video_in =
        ffmpeg::format::input(video).map_err(|error| ffi::fail("open", video, error))?;
    let mut audio_in =
        ffmpeg::format::input(audio).map_err(|error| ffi::fail("open", audio, error))?;
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
    }
    {
        let mut stream = out
            .add_stream(encoder::find(ffmpeg::codec::Id::None))
            .map_err(|error| ffi::fail("add stream", output, error))?;
        stream.set_parameters(audio_stream.parameters());
        stream.set_time_base(audio_tb);
    }
    out.write_header_with(ffmpeg::dict! { "movflags" => "+faststart" })
        .map_err(|error| ffi::fail("write header", output, error))?;
    let out_video_tb = out
        .stream(0)
        .map(|stream| stream.time_base())
        .unwrap_or(video_tb);
    let out_audio_tb = out
        .stream(1)
        .map(|stream| stream.time_base())
        .unwrap_or(audio_tb);

    // Alternate one packet from each; `write_interleaved` orders them by
    // timestamp. When either input ends, the file ends: `-shortest`.
    let mut copy = |input: &mut ffmpeg::format::context::Input,
                    wanted: usize,
                    from: ffmpeg::Rational,
                    to: ffmpeg::Rational,
                    stream: usize,
                    path: &Path|
     -> Result<bool> {
        loop {
            let mut packet = ffmpeg::Packet::empty();
            match packet.read(input) {
                Ok(()) => {
                    if packet.stream() != wanted {
                        continue;
                    }
                    packet.set_stream(stream);
                    packet.rescale_ts(from, to);
                    packet.set_position(-1);
                    packet
                        .write_interleaved(&mut out)
                        .map_err(|error| ffi::fail("write packet", output, error))?;
                    return Ok(true);
                }
                Err(ffmpeg::Error::Eof) => return Ok(false),
                Err(error) => return Err(ffi::fail("read", path, error)),
            }
        }
    };
    loop {
        if !copy(&mut video_in, video_index, video_tb, out_video_tb, 0, video)? {
            break;
        }
        if !copy(&mut audio_in, audio_index, audio_tb, out_audio_tb, 1, audio)? {
            break;
        }
    }
    out.write_trailer()
        .map_err(|error| ffi::fail("write trailer", output, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(path: &str) -> AudioClip {
        AudioClip {
            path: path.into(),
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
        let graph = mix_graph(&[clip("a.mp4")], 2.0).expect("valid");
        assert!(graph.contains("anull[mixed]"), "graph was: {graph}");
        assert!(!graph.contains("amix"), "graph was: {graph}");
    }

    #[test]
    fn several_clips_mix_without_normalisation() {
        let graph = mix_graph(&[clip("a.mp4"), clip("b.mp4")], 3.0).expect("valid");
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
    fn the_final_stage_pads_and_trims_to_the_timeline_duration() {
        let graph = mix_graph(&[clip("a.mp4")], 4.5).expect("valid");
        assert!(
            graph.ends_with("aresample=48000,apad,atrim=duration=4.500000[out]"),
            "graph was: {graph}"
        );
    }

    #[test]
    fn a_sped_up_clip_trims_more_source_than_its_timeline_length() {
        let mut fast = clip("a.mp4");
        fast.speed = 2.0;
        let graph = mix_graph(&[fast], 2.0).expect("valid");
        assert!(graph.contains("duration=4.000000"), "graph was: {graph}");
        assert!(graph.contains("atempo=2.000000"), "graph was: {graph}");
    }

    #[test]
    fn fades_and_volume_apply_after_the_filter_chain() {
        let mut shaped = clip("a.mp4");
        shaped.filter_chain = "highpass=f=80".to_owned();
        shaped.volume = 0.5;
        shaped.fade_out = 0.25;
        let graph = mix_graph(&[shaped], 2.0).expect("valid");

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
                matches!(
                    mix_graph(&[hostile], 2.0),
                    Err(Error::InvalidFilterChain { .. })
                ),
                "{bad:?} should have been refused",
            );
        }
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

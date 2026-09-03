// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Rendering a timeline to a file.
//!
//! This is the seam where a flattened clip list becomes the engine's
//! `concat_core::Timeline`, and from there the engine decides everything -
//! what is on screen (`concat-render`), what the sound means
//! (`concat_media::audio`), and how bytes move (`concat-media`). Transition
//! semantics resolve here too: by the time the picture and sound paths read
//! the clip list, transitions have already become overlaps, opacity ramps and
//! fade filters.
//!
//! It lives in the engine so the CLI, the host and the window render one
//! way, and so the doctrine holds: the host adds a destination and reports
//! progress, nothing more.
//!
//! Picture is composited frame by frame - on the GPU when the `gpu` feature
//! is on and the machine has one, with the CPU compositor as the
//! always-correct fallback. Sound is planned by the engine as one FFmpeg
//! filtergraph and mixed in a single pass.

pub mod chains;
pub mod flatten;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use concat_core::frame::Frame;
use concat_core::time::{FrameRate, Rational};
use concat_core::timeline::{Clip, ClipId, MediaRef, Timeline, Track, TrackKind, Transform};
use concat_media::audio::{self, AudioClip};
use concat_media::{DecodeOptions, Decoder, EncodeOptions, Encoder, FrameSink, FrameSource};
use concat_render::{Compositor, CpuCompositor, Layer, Placement, plan_frame};
use serde::Deserialize;

/// What a flattened clip is. Typed, so a kind check the compiler has not
/// seen cannot exist - the document says "video"/"audio"/"image".
#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClipKind {
    /// Footage: pictures over time, possibly with its own sound.
    Video,
    /// Sound only. Contributes to the mix and never to the picture.
    Audio,
    /// A still: a one-frame stream, decoded looping.
    Image,
}

impl ClipKind {
    /// True for the kinds that put pixels on screen.
    fn is_visual(self) -> bool {
        matches!(self, ClipKind::Video | ClipKind::Image)
    }
}

/// One clip, as the frontend's flattener describes it.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExportClip {
    /// The media file this clip shows or plays.
    pub path: String,
    /// Whether the clip is footage, sound or a still.
    pub kind: ClipKind,
    /// Seconds into the timeline where the clip begins.
    pub start: f64,
    /// Seconds of timeline the clip covers.
    pub duration: f64,
    /// Seconds into the source where playback begins.
    pub source_start: f64,
    /// Index into the track stack, zero being bottom-most.
    pub track: usize,
    /// The track's picture is switched off.
    pub hidden: bool,
    /// The track's sound is switched off.
    pub muted: bool,
    /// Linear gain, 1 being unity.
    #[serde(default = "unity")]
    pub volume: f64,
    /// Audio fade up from the clip's start, in seconds.
    #[serde(default)]
    pub fade_in: f64,
    /// Audio fade out into the clip's end, in seconds.
    #[serde(default)]
    pub fade_out: f64,
    /// FFmpeg filter chain from the Filters tab, or empty.
    #[serde(default)]
    pub filter_chain: String,
    /// Playback rate. 1 is normal.
    #[serde(default = "unity")]
    pub speed: f64,
    /// False lets pitch rise with the rate, like tape.
    #[serde(default = "yes")]
    pub preserve_pitch: bool,
    /// Multiplier over the fitted size. 1 fills the frame, preserving aspect.
    #[serde(default = "unity")]
    pub scale: f64,
    /// Offset of the picture's centre from frame centre, frame-width fraction.
    #[serde(default)]
    pub offset_x: f64,
    /// Offset as a frame-height fraction.
    #[serde(default)]
    pub offset_y: f64,
    /// Clockwise rotation in degrees.
    #[serde(default)]
    pub rotation: f64,
    /// Blend strength over the layers beneath, 1 being solid. Defaulted for
    /// requests from a UI that predates it.
    #[serde(default = "unity")]
    pub opacity: f64,
    /// FFmpeg *video* filter chain from the Effects tab, or empty. Applied at
    /// decode, after scaling - see `DecodeOptions::filter_chain`.
    #[serde(default)]
    pub video_filter_chain: String,
    /// The transition into this clip's cut, when the UI put one there. The
    /// clip before it on the same track is found here, by adjacency - the UI
    /// only says what it wants, never how to overlap decoders.
    #[serde(default)]
    pub transition: Option<TransitionSpec>,
    /// Video opacity ramp up from the clip's start, in seconds. Set by
    /// transition resolution below, never by the UI directly.
    #[serde(default)]
    pub video_fade_in: f64,
    /// The source's pixel width, when the UI knows it. What makes an
    /// aspect-correct decode possible - absent, the frame is filled edge to
    /// edge the way it always was.
    #[serde(default)]
    pub media_width: Option<u32>,
    /// The source's pixel height; see `media_width`.
    #[serde(default)]
    pub media_height: Option<u32>,
    /// Whether the file carries an audio stream, when the UI knows (the
    /// document records it at import). Absent falls back to probing, so an
    /// older caller still exports correctly - just slower to start.
    #[serde(default)]
    pub has_audio: Option<bool>,
}

/// A transition on the cut into a clip.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TransitionSpec {
    /// "cross-fade", "fade-black" or "fade-white". Anything else is ignored.
    pub kind: String,
    /// Seconds the transition covers.
    pub duration: f64,
}

fn yes() -> bool {
    true
}

fn unity() -> f64 {
    1.0
}

/// Everything a full export needs: the destination, the output format, and
/// the flattened clip list.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    /// The file to write. Siblings named `.{stem}.concat-*` are used as
    /// scratch during the render and removed afterwards.
    pub output: String,
    /// Output frame width in pixels.
    pub width: u32,
    /// Output frame height in pixels.
    pub height: u32,
    /// Frame rate numerator - an exact fraction, so 29.97 stays 30000/1001.
    pub rate_num: i64,
    /// Frame rate denominator; see `rate_num`.
    pub rate_den: i64,
    /// Constant rate factor. Lower is better quality and a bigger file.
    pub crf: u8,
    /// The x264 speed/size preset name, e.g. "medium".
    pub preset: String,
    /// The flattened clip list to render.
    pub clips: Vec<ExportClip>,
}

/// What the export loop calls to report and to ask "should I stop?".
pub struct Reporter<'a> {
    /// Called with (frame, total, stage).
    pub progress: &'a mut dyn FnMut(i64, i64, &'static str),
    /// Checked between frames and stages; true aborts cleanly.
    pub cancel: &'a AtomicBool,
}

impl Reporter<'_> {
    fn emit(&mut self, frame: i64, total: i64, stage: &'static str) {
        (self.progress)(frame, total, stage);
    }

    fn cancelled(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Relaxed) {
            Err("export cancelled".to_owned())
        } else {
            Ok(())
        }
    }
}

/// Turns per-cut transition requests into things the renderer already knows
/// how to draw: overlapping clips, opacity ramps, and fade filters.
///
/// Track indices are doubled first, so an incoming cross-fade clip gets an
/// odd lane of its own directly above the pair it dissolves over - stacking
/// against every other track is preserved, and nothing else occupies odd
/// lanes. Cuts are collected before anything mutates, so resolving one
/// transition cannot unhook the adjacency test of the next.
fn resolve_transitions(clips: &mut [ExportClip], rate: FrameRate, bake_fades: bool) {
    for clip in clips.iter_mut() {
        clip.track *= 2;
    }

    let fps = rate.fps().as_f64();
    let frame = 1.0 / fps;

    struct Cut {
        incoming: usize,
        outgoing: usize,
        kind: String,
        duration: f64,
    }
    let mut cuts: Vec<Cut> = Vec::new();
    for (incoming, clip) in clips.iter().enumerate() {
        let Some(transition) = &clip.transition else {
            continue;
        };
        if clip.hidden || !clip.kind.is_visual() {
            continue;
        }
        // The outgoing clip is whatever ends where this one starts, on the
        // same lane. No match - the cut was edited apart - means the
        // transition is silently orphaned, exactly like the UI treats it.
        let outgoing = clips.iter().position(|other| {
            !other.hidden
                && other.kind.is_visual()
                && other.track == clip.track
                && (other.start + other.duration - clip.start).abs() < frame / 2.0
        });
        match outgoing {
            Some(outgoing) if outgoing != incoming => cuts.push(Cut {
                incoming,
                outgoing,
                kind: transition.kind.clone(),
                duration: transition.duration.max(0.0),
            }),
            _ => {}
        }
    }

    for cut in cuts {
        match cut.kind.as_str() {
            "cross-fade" => {
                let (a_track, a_duration) = {
                    let a = &clips[cut.outgoing];
                    (a.track, a.duration)
                };
                let b = &mut clips[cut.incoming];

                // The incoming clip extends backwards over the outgoing one,
                // showing the source it has *before* its in-point - the
                // handle, exactly what a dissolve consumes in any editor. No
                // handle, shorter dissolve: the duration clamps to what
                // actually exists rather than freezing or inventing frames.
                let mut d = cut.duration.min(a_duration).min(b.duration);
                if b.kind != ClipKind::Image {
                    d = d.min(b.source_start / b.speed.max(0.0625));
                }
                if d < frame {
                    continue;
                }
                b.start -= d;
                b.duration += d;
                if b.kind != ClipKind::Image {
                    b.source_start -= d * b.speed;
                }
                b.video_fade_in = d;
                // Sound rides the picture: the pre-roll fades in rather than
                // arriving at full level a dissolve early.
                b.fade_in = b.fade_in.max(d);
                b.track = a_track + 1;
            }
            "fade-black" | "fade-white" if bake_fades => {
                // Half the duration on each side of the cut, as fade filters
                // at decode. Frame-based, because the decoder emits exactly
                // one frame per output frame - so the fade lands on the same
                // frames the timeline arithmetic says it covers.
                let colour = if cut.kind == "fade-white" {
                    ":color=white"
                } else {
                    ""
                };
                let half = cut.duration / 2.0;
                {
                    let a = &mut clips[cut.outgoing];
                    let frames = ((half.min(a.duration) * fps).round() as i64).max(1);
                    let total = (a.duration * fps).round() as i64;
                    append_filter(
                        &mut a.video_filter_chain,
                        &format!(
                            "fade=t=out:start_frame={}:nb_frames={frames}{colour}",
                            (total - frames).max(0)
                        ),
                    );
                }
                {
                    let b = &mut clips[cut.incoming];
                    let frames = ((half.min(b.duration) * fps).round() as i64).max(1);
                    append_filter(
                        &mut b.video_filter_chain,
                        &format!("fade=t=in:start_frame=0:nb_frames={frames}{colour}"),
                    );
                }
            }
            // A kind this build does not know renders as a plain cut rather
            // than failing the export - the same degrade a missing effect
            // filter must NOT get, because there the user styled the picture.
            _ => {}
        }
    }
}

/// Appends one filter to a chain, comma-separated. Effects the user stacked
/// come first; a transition fades the styled picture, not the raw one.
fn append_filter(chain: &mut String, filter: &str) {
    if !chain.is_empty() {
        chain.push(',');
    }
    chain.push_str(filter);
}

/// Renders `request` and returns the path written.
pub fn render(request: &ExportRequest, mut reporter: Reporter<'_>) -> Result<String, String> {
    if request.clips.is_empty() {
        return Err("there is nothing on the timeline to export".to_owned());
    }

    let rate = FrameRate::new(Rational::new(request.rate_num, request.rate_den));
    let output = PathBuf::from(&request.output);

    // Transitions become overlaps, ramps and fade filters before anything
    // else reads the clip list, so the picture and sound paths below never
    // know transitions exist.
    let mut resolved = request.clips.clone();
    resolve_transitions(&mut resolved, rate, true);

    // Stills composite exactly like footage; they only differ in how they are
    // decoded, which is handled where the decoder is opened.
    let visible: Vec<&ExportClip> = resolved
        .iter()
        .filter(|clip| clip.kind.is_visual() && !clip.hidden)
        .collect();
    let audible: Vec<&ExportClip> = resolved
        .iter()
        .filter(|clip| clip.kind == ClipKind::Audio && !clip.muted)
        .collect();

    // A video clip carries its own sound, so an unmuted video track
    // contributes to the mix as well as to the picture.
    let mut sound: Vec<&ExportClip> = audible;
    sound.extend(
        resolved
            .iter()
            .filter(|clip| clip.kind == ClipKind::Video && !clip.muted),
    );

    // An unmuted clip can still have no audio stream - a screen recording, a
    // silent render. FFmpeg refuses a filtergraph that names `[N:a]` on such
    // an input rather than treating it as silence, so membership in the mix
    // needs the file's truth. The document learnt it at import and the UI
    // sends it along; a request that omits it falls back to probing, once
    // per unique path.
    let mut probed: HashMap<&str, bool> = HashMap::new();
    sound.retain(|clip| {
        clip.has_audio.unwrap_or_else(|| {
            *probed.entry(clip.path.as_str()).or_insert_with(|| {
                concat_media::probe(&clip.path).is_ok_and(|info| info.audio.is_some())
            })
        })
    });

    let timeline_end = resolved
        .iter()
        .map(|clip| clip.start + clip.duration)
        .fold(0.0f64, f64::max);
    let total_frames = (timeline_end * rate.fps().as_f64()).round() as i64;
    if total_frames <= 0 {
        return Err("the timeline is empty".to_owned());
    }

    // Render into siblings of the output so the move at the end stays on one
    // filesystem, then clean up whatever we made.
    let stem = output
        .file_stem()
        .map_or_else(|| "concat".into(), |s| s.to_string_lossy());
    let directory = output.parent().unwrap_or(Path::new("."));
    let silent = directory.join(format!(".{stem}.concat-video.mp4"));
    let mixed = directory.join(format!(".{stem}.concat-audio.m4a"));

    let result = (|| -> Result<(), String> {
        render_picture(
            request,
            rate,
            total_frames,
            &visible,
            &silent,
            &mut reporter,
        )?;

        if sound.is_empty() {
            std::fs::rename(&silent, &output)
                .map_err(|error| format!("could not write {}: {error}", output.display()))?;
            return Ok(());
        }

        reporter.cancelled()?;
        reporter.emit(0, total_frames, "mixing audio");
        let mix: Vec<AudioClip> = sound.iter().map(|clip| audio_clip(clip)).collect();
        audio::mix_to_file(&mix, timeline_end, &mixed).map_err(|error| error.to_string())?;

        reporter.cancelled()?;
        reporter.emit(total_frames, total_frames, "muxing");
        audio::mux(&silent, &mixed, &output).map_err(|error| error.to_string())
    })();

    let _ = std::fs::remove_file(&silent);
    let _ = std::fs::remove_file(&mixed);

    result.map(|()| output.to_string_lossy().into_owned())
}

/// The engine's view of one audible clip.
fn audio_clip(clip: &ExportClip) -> AudioClip {
    AudioClip {
        path: PathBuf::from(&clip.path),
        start: clip.start,
        duration: clip.duration,
        source_start: clip.source_start,
        speed: audio::clamp_speed(clip.speed),
        preserve_pitch: clip.preserve_pitch,
        volume: clip.volume,
        fade_in: clip.fade_in,
        fade_out: clip.fade_out,
        filter_chain: clip.filter_chain.clone(),
    }
}

/// The fraction-to-pixel placement of one composited layer: fitted and
/// centred is the base, the clip's transform moves it from there. The one
/// definition the exporter and the preview share - these two paths must
/// never place a picture differently, or the paused truth lies about the
/// file.
fn place_layer<'a>(
    frame: &'a Frame,
    opacity: f32,
    transform: &Transform,
    width: u32,
    height: u32,
) -> Layer<'a> {
    let x = (i64::from(width) - i64::from(frame.width())) / 2;
    let y = (i64::from(height) - i64::from(frame.height())) / 2;
    let placement = if transform.is_identity() {
        Placement::IDENTITY
    } else {
        Placement {
            scale: transform.scale as f32,
            rotation: transform.rotation.to_radians() as f32,
            translate_x: (transform.offset_x * f64::from(width)) as f32,
            translate_y: (transform.offset_y * f64::from(height)) as f32,
        }
    };
    Layer::new(frame)
        .at(x as i32, y as i32)
        .with_opacity(opacity)
        .with_placement(placement)
}

/// The best compositor this machine offers: the GPU when the `gpu` feature is
/// on and the machine has one, the CPU reference otherwise. Never an error -
/// a machine with no adapter renders slower, not not-at-all.
fn best_compositor() -> Box<dyn Compositor> {
    #[cfg(feature = "gpu")]
    if let Some(gpu) = concat_render::WgpuCompositor::new() {
        return Box::new(gpu);
    }
    Box::new(CpuCompositor)
}

/// Composites every frame of the timeline into a soundless video file.
fn render_picture(
    request: &ExportRequest,
    rate: FrameRate,
    total_frames: i64,
    visible: &[&ExportClip],
    destination: &Path,
    reporter: &mut Reporter<'_>,
) -> Result<(), String> {
    let BuiltTimeline {
        timeline,
        stills,
        decode_sizes,
        filter_chains,
    } = build_timeline(request, rate, visible);

    let mut encoder = Encoder::create(
        destination,
        request.width,
        request.height,
        rate,
        &EncodeOptions {
            crf: request.crf,
            preset: request.preset.clone(),
            ..EncodeOptions::default()
        },
    )
    .map_err(|error| error.to_string())?;

    let mut compositor = best_compositor();

    // One decoder per clip, opened at its in-point the first time the clip is
    // needed and dropped the moment it leaves the playhead. Every decoder is
    // opened at `output rate / clip speed`, so pulling exactly one frame per
    // output frame keeps each of them in step with the plan's source times
    // without any seeking - including retimed clips.
    let mut decoders: HashMap<ClipId, Decoder> = HashMap::new();

    for index in 0..total_frames {
        reporter.cancelled()?;

        let time = rate.time_of_frame(index);
        let plan = plan_frame(&timeline, time);

        let mut sources: Vec<(Frame, f32, Transform)> = Vec::with_capacity(plan.layers.len());
        for layer in &plan.layers {
            let decoder = match decoders.entry(layer.clip) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    // Aspect-correct: decode at the source's fitted size and
                    // let the compositor place it, rather than stretching to
                    // the frame and losing the picture's shape.
                    let (decode_width, decode_height) = decode_sizes
                        .get(&layer.clip)
                        .copied()
                        .unwrap_or((request.width, request.height));

                    // A retimed clip advances the source by `speed / fps`
                    // seconds per output frame, so the decoder must emit
                    // frames exactly that far apart: a rate of `fps / speed`.
                    // (At 2x, 300 output frames cover 20s of source - the
                    // decoder emits 15 frames per source second, not 60.)
                    let decode_rate = if layer.speed == Rational::ONE {
                        rate
                    } else {
                        FrameRate::new(rate.fps() / layer.speed)
                    };

                    let mut options = DecodeOptions::default()
                        .starting_at(layer.source_time)
                        .scaled_to(decode_width, decode_height)
                        .at_rate(decode_rate);

                    // Effects and transition fades, as one FFmpeg chain. The
                    // decoder guards the frame size after it, so an effect
                    // that resizes cannot shear the pipe.
                    if let Some(chain) = filter_chains.get(&layer.clip) {
                        options = options.filtered(chain.clone());
                    }

                    // A still is a one-frame stream. Without looping it would
                    // contribute a single frame and then disappear.
                    if stills.contains(&layer.clip) {
                        options = options.repeating().starting_at(Rational::ZERO);
                    }

                    entry.insert(
                        Decoder::open(&layer.media, &options).map_err(|error| error.to_string())?,
                    )
                }
            };

            // A source that has run out contributes nothing rather than
            // aborting the export - a clip trimmed past its media's end is a
            // mistake in the edit, not a failure of the renderer.
            if let Some(frame) = decoder.next_frame().map_err(|error| error.to_string())? {
                sources.push((frame, layer.opacity, layer.transform));
            }
        }

        let layers: Vec<Layer<'_>> = sources
            .iter()
            .map(|(frame, opacity, transform)| {
                place_layer(frame, *opacity, transform, request.width, request.height)
            })
            .collect();

        let composed = compositor.composite(request.width, request.height, &layers);
        encoder
            .write_frame(&composed)
            .map_err(|error| error.to_string())?;

        // Retire decoders whose clip has finished, so a long timeline does not
        // hold an ffmpeg process open for every clip it has ever passed.
        let live: Vec<ClipId> = plan.layers.iter().map(|layer| layer.clip).collect();
        decoders.retain(|clip, _| live.contains(clip));

        if index % 15 == 0 {
            reporter.emit(index, total_frames, "rendering");
        }
    }

    encoder.finish().map_err(|error| error.to_string())
}

/// An engine timeline plus the per-clip facts the decoders need that the
/// engine's model has no field for.
struct BuiltTimeline {
    timeline: Timeline,
    /// Clips that are stills: one-frame streams, decoded looping.
    stills: std::collections::HashSet<ClipId>,
    /// Contain-fitted decode size per clip, where the source's size is known.
    decode_sizes: HashMap<ClipId, (u32, u32)>,
    /// The clip's effect chain, where it has one.
    filter_chains: HashMap<ClipId, String>,
}

/// Converts the flattened clip list into an engine timeline.
fn build_timeline(
    request: &ExportRequest,
    rate: FrameRate,
    visible: &[&ExportClip],
) -> BuiltTimeline {
    let mut timeline = Timeline::new(request.width, request.height, rate);
    let mut stills = std::collections::HashSet::new();
    let mut decode_sizes: HashMap<ClipId, (u32, u32)> = HashMap::new();
    let mut filter_chains: HashMap<ClipId, String> = HashMap::new();

    let lanes = visible.iter().map(|clip| clip.track).max().unwrap_or(0) + 1;
    let tracks: Vec<_> = (0..lanes)
        .map(|index| timeline.add_track(Track::new(format!("T{index}"), TrackKind::Video)))
        .collect();

    for clip in visible {
        // Quantise to the frame grid on the way in. The UI works in f64
        // seconds; the engine works in exact rationals, and this is the seam
        // where a value stops being approximate.
        let start = quantise(clip.start, rate);
        let duration = quantise(clip.duration, rate);
        if duration.is_zero() {
            continue;
        }

        let mut engine_clip = Clip::new(MediaRef::new(&clip.path), start, duration);
        engine_clip.source_start = quantise(clip.source_start, rate);
        // The same clamp the audio path applies, so a 2x clip means the same
        // thing to picture and sound. A still has no meaningful rate.
        if clip.kind != ClipKind::Image {
            engine_clip.speed =
                Rational::approximate(audio::clamp_speed(clip.speed)).unwrap_or(Rational::ONE);
        }
        engine_clip.transform = Transform {
            scale: clip.scale,
            offset_x: clip.offset_x,
            offset_y: clip.offset_y,
            rotation: clip.rotation,
        };
        engine_clip.opacity = clip.opacity.clamp(0.0, 1.0) as f32;
        // Quantised like every other time: the ramp must land on the same
        // frame grid the overlap does, or the dissolve ends a frame early.
        engine_clip.video_fade_in = quantise(clip.video_fade_in, rate);

        if let Some(id) = timeline.add_clip(tracks[clip.track], engine_clip) {
            if clip.kind == ClipKind::Image {
                stills.insert(id);
            }
            if let Some(size) = fitted_size(request, clip) {
                decode_sizes.insert(id, size);
            }
            if !clip.video_filter_chain.is_empty() {
                filter_chains.insert(id, clip.video_filter_chain.clone());
            }
        }
    }

    BuiltTimeline {
        timeline,
        stills,
        decode_sizes,
        filter_chains,
    }
}

/// The source's contain-fitted size inside the output frame, or `None` when
/// the UI never learnt the source's dimensions.
fn fitted_size(request: &ExportRequest, clip: &ExportClip) -> Option<(u32, u32)> {
    let media_width = clip.media_width.filter(|value| *value > 0)?;
    let media_height = clip.media_height.filter(|value| *value > 0)?;

    let fit = (f64::from(request.width) / f64::from(media_width))
        .min(f64::from(request.height) / f64::from(media_height));
    let width = ((f64::from(media_width) * fit).round() as u32).max(2);
    let height = ((f64::from(media_height) * fit).round() as u32).max(2);
    // Even, because a decoder asked for an odd width may round it itself and
    // then every frame read is misaligned by a pixel's worth of bytes.
    Some((width & !1, height & !1))
}

fn quantise(seconds: f64, rate: FrameRate) -> Rational {
    rate.time_of_frame((seconds * rate.fps().as_f64()).round().max(0.0) as i64)
}

/// One paused-monitor frame: the same clip list the exporter takes, one
/// timestamp, a preview resolution.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewFrameRequest {
    /// The timeline instant to composite, in seconds.
    pub time: f64,
    /// Preview frame width in pixels.
    pub width: u32,
    /// Preview frame height in pixels.
    pub height: u32,
    /// Frame rate numerator - an exact fraction, so 29.97 stays 30000/1001.
    pub rate_num: i64,
    /// Frame rate denominator; see `rate_num`.
    pub rate_den: i64,
    /// The flattened clip list, exactly as an export would take it.
    pub clips: Vec<ExportClip>,
}

/// Composites the true frame at one instant, for the paused monitor.
///
/// The identical plan/composite the exporter runs, fed from the reader pool
/// so scrubbing revisits are cache hits. Fade-to-colour transitions are NOT
/// baked here - their filter frame numbers assume decode-from-clip-start,
/// which pooled seeks break - so the UI keeps drawing its veil, whose shape
/// already matches the exporter's. Returns raw RGBA, exactly
/// `width * height * 4` bytes.
pub fn preview_frame(
    pool: &concat_media::ReaderPool,
    request: &PreviewFrameRequest,
) -> Result<Vec<u8>, String> {
    let sources = preview_sources(pool, request)?;
    let layers = sources.layers();
    // CPU on purpose: one frame at preview size is milliseconds, and holding
    // a GPU context alive for occasional scrubs is not worth its memory. The
    // window composites on its own device through `preview_sources`.
    let composed = CpuCompositor.composite(request.width, request.height, &layers);
    Ok(composed.into_pixels())
}

/// The decoded pictures under the playhead and where each goes: what a
/// compositor needs to draw the paused monitor's frame, decoded but not yet
/// drawn, so a caller with a GPU can draw them where they are shown.
pub struct PreviewSources {
    sources: Vec<(std::sync::Arc<Frame>, f32, Transform)>,
    width: u32,
    height: u32,
}

impl PreviewSources {
    /// The layers, placed the exporter's way, bottom-most first.
    pub fn layers(&self) -> Vec<Layer<'_>> {
        self.sources
            .iter()
            .map(|(frame, opacity, transform)| {
                place_layer(frame.as_ref(), *opacity, transform, self.width, self.height)
            })
            .collect()
    }

    /// The output width the layers were placed for.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The output height the layers were placed for.
    pub fn height(&self) -> u32 {
        self.height
    }
}

/// Decodes the pictures the paused monitor's frame is made of, through the
/// reader pool so scrubbing revisits are cache hits. See [`preview_frame`]
/// for the rules on what counts as a failure.
pub fn preview_sources(
    pool: &concat_media::ReaderPool,
    request: &PreviewFrameRequest,
) -> Result<PreviewSources, String> {
    let rate = FrameRate::new(Rational::new(request.rate_num, request.rate_den));
    let BuiltTimeline {
        timeline,
        stills,
        decode_sizes,
        filter_chains,
    } = preview_timeline(request, rate);
    let plan = plan_frame(&timeline, quantise(request.time, rate));

    let mut sources: Vec<(std::sync::Arc<Frame>, f32, Transform)> =
        Vec::with_capacity(plan.layers.len());
    let mut failures: Vec<String> = Vec::new();
    for layer in &plan.layers {
        let (decode_width, decode_height) = decode_sizes
            .get(&layer.clip)
            .copied()
            .unwrap_or((request.width, request.height));
        let chain = filter_chains.get(&layer.clip).map(String::as_str);
        // A source that fails to decode contributes nothing rather than
        // blanking the monitor - same grace the exporter extends.
        match pool.frame_at(
            std::path::Path::new(&layer.media),
            layer.source_time,
            decode_width,
            decode_height,
            stills.contains(&layer.clip),
            chain,
        ) {
            Ok(frame) => sources.push((frame, layer.opacity, layer.transform)),
            Err(error) => failures.push(format!("{}: {error}", layer.media.display())),
        }
    }

    // Planned layers with nothing decoded is a failed preview, not a black
    // frame: compositing zero sources yields opaque black, and the caller
    // would draw that "truth" over its own perfectly good approximation. An
    // *empty plan* still composites - a gap in the timeline really is black.
    if sources.is_empty() && !plan.layers.is_empty() {
        return Err(format!(
            "no layer decoded for the paused preview: {}",
            failures.join(" / ")
        ));
    }

    Ok(PreviewSources {
        sources,
        width: request.width,
        height: request.height,
    })
}

/// The preview's timeline, built the exporter's way.
fn preview_timeline(request: &PreviewFrameRequest, rate: FrameRate) -> BuiltTimeline {
    let mut resolved = request.clips.clone();
    resolve_transitions(&mut resolved, rate, false);
    let visible: Vec<&ExportClip> = resolved
        .iter()
        .filter(|clip| clip.kind.is_visual() && !clip.hidden)
        .collect();

    // build_timeline reads only the output format off the request; the shim
    // keeps one conversion path rather than a preview-flavoured copy of it.
    let shim = ExportRequest {
        output: String::new(),
        width: request.width,
        height: request.height,
        rate_num: request.rate_num,
        rate_den: request.rate_den,
        crf: 18,
        preset: String::new(),
        clips: Vec::new(),
    };
    build_timeline(&shim, rate, &visible)
}

/// Warms the reader pool for the frames about to be presented.
///
/// The playback stream's decode-ahead half: while the UI presents the frame
/// at `request.time`, this decodes the next `frames` frame instants into the
/// pool's cache so the next pulls are hits, not decode waits. Requests stay
/// monotonic and one frame apart, which is exactly what keeps the pool's
/// readers rolling forward instead of respawning FFmpeg to seek.
///
/// Compositing is skipped - the pull composites - and so are failures: a
/// source that will not decode fails the pull too, and that is the path
/// with an error channel.
pub fn preview_prefetch(
    pool: &concat_media::ReaderPool,
    request: &PreviewFrameRequest,
    frames: u32,
) {
    let rate = FrameRate::new(Rational::new(request.rate_num, request.rate_den));
    let BuiltTimeline {
        timeline,
        stills,
        decode_sizes,
        filter_chains,
    } = preview_timeline(request, rate);
    let fps = rate.fps().as_f64();

    for ahead in 0..frames {
        let time = request.time + f64::from(ahead) / fps;
        let plan = plan_frame(&timeline, quantise(time, rate));
        for layer in &plan.layers {
            let (decode_width, decode_height) = decode_sizes
                .get(&layer.clip)
                .copied()
                .unwrap_or((request.width, request.height));
            let chain = filter_chains.get(&layer.clip).map(String::as_str);
            let _ = pool.frame_at(
                std::path::Path::new(&layer.media),
                layer.source_time,
                decode_width,
                decode_height,
                stills.contains(&layer.clip),
                chain,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(kind: &str, track: usize, start: f64, duration: f64, source_start: f64) -> ExportClip {
        ExportClip {
            path: format!("{kind}.mp4"),
            kind: match kind {
                "audio" => ClipKind::Audio,
                "image" => ClipKind::Image,
                _ => ClipKind::Video,
            },
            start,
            duration,
            source_start,
            track,
            hidden: false,
            muted: false,
            volume: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            filter_chain: String::new(),
            speed: 1.0,
            preserve_pitch: true,
            scale: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
            rotation: 0.0,
            opacity: 1.0,
            video_filter_chain: String::new(),
            transition: None,
            video_fade_in: 0.0,
            media_width: None,
            media_height: None,
            has_audio: None,
        }
    }

    fn spec(kind: &str, duration: f64) -> Option<TransitionSpec> {
        Some(TransitionSpec {
            kind: kind.to_owned(),
            duration,
        })
    }

    /// End to end against a real FFmpeg: the paused monitor's frame must show
    /// the footage, not an empty composite. Skips silently without FFmpeg.
    #[test]
    fn preview_frame_shows_the_footage_not_black() {
        use concat_core::frame::Frame;
        use concat_media::{EncodeOptions, Encoder, FrameSink};

        let path = std::env::temp_dir().join("concat-preview-test.mp4");
        let Ok(mut encoder) =
            Encoder::create(&path, 64, 64, FrameRate::THIRTY, &EncodeOptions::default())
        else {
            return; // no ffmpeg here
        };
        for _ in 0..30 {
            let mut frame = Frame::black(64, 64);
            frame.fill([200, 30, 30, 255]);
            encoder.write_frame(&frame).expect("writes");
        }
        encoder.finish().expect("finishes");

        let mut request_clip = clip("video", 0, 0.0, 1.0, 0.0);
        request_clip.path = path.to_string_lossy().into_owned();
        request_clip.media_width = Some(64);
        request_clip.media_height = Some(64);
        let request = PreviewFrameRequest {
            time: 0.5,
            width: 64,
            height: 64,
            rate_num: 30,
            rate_den: 1,
            clips: vec![request_clip],
        };

        let mut pool = concat_media::ReaderPool::new(16 * 1024 * 1024, 2);
        let bytes = preview_frame(&pool, &request).expect("previews");
        assert_eq!(bytes.len(), 64 * 64 * 4);
        let centre = (32 * 64 + 32) * 4;
        assert!(
            bytes[centre] > 120 && bytes[centre + 1] < 90,
            "centre pixel should be red-ish, got {:?}",
            &bytes[centre..centre + 4],
        );

        // A clip trimmed past its media's end: the paused monitor at that
        // time must show the last real frame, not a black composite and not
        // an error.
        let mut outliving = clip("video", 0, 0.0, 3.0, 0.0);
        outliving.path = path.to_string_lossy().into_owned();
        let late = PreviewFrameRequest {
            time: 2.5,
            width: 64,
            height: 64,
            rate_num: 30,
            rate_den: 1,
            clips: vec![outliving],
        };
        let bytes = preview_frame(&pool, &late).expect("previews past the media's end");
        assert!(
            bytes[centre] > 120 && bytes[centre + 1] < 90,
            "past the end should freeze on the last frame, got {:?}",
            &bytes[centre..centre + 4],
        );

        // A clip with an effect chain: the paused monitor must show the
        // processed pixels, not the raw decode. Negating red footage has to
        // come back cyan-ish.
        let mut effected = clip("video", 0, 0.0, 1.0, 0.0);
        effected.path = path.to_string_lossy().into_owned();
        effected.media_width = Some(64);
        effected.media_height = Some(64);
        effected.video_filter_chain = "negate".to_owned();
        let filtered = PreviewFrameRequest {
            time: 0.5,
            width: 64,
            height: 64,
            rate_num: 30,
            rate_den: 1,
            clips: vec![effected],
        };
        let bytes = preview_frame(&pool, &filtered).expect("previews with a chain");
        assert!(
            bytes[centre] < 90 && bytes[centre + 1] > 120,
            "the chain must be baked into the paused frame, got {:?}",
            &bytes[centre..centre + 4],
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_cross_fade_overlaps_the_incoming_clip_on_its_own_lane() {
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("video", 0, 4.0, 4.0, 2.0),
        ];
        clips[1].transition = spec("cross-fade", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);

        let b = &clips[1];
        assert_eq!(b.start, 3.0, "extends backwards over the cut");
        assert_eq!(b.duration, 5.0);
        assert_eq!(
            b.source_start, 1.0,
            "consumes the handle before the in-point"
        );
        assert_eq!(b.video_fade_in, 1.0);
        assert_eq!(b.fade_in, 1.0, "sound rides the picture");
        assert_eq!(clips[0].track, 0, "outgoing stays on its doubled lane");
        assert_eq!(b.track, 1, "incoming sits directly above it");
    }

    #[test]
    fn a_cross_fade_clamps_to_the_available_handle() {
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("video", 0, 4.0, 4.0, 0.25),
        ];
        clips[1].transition = spec("cross-fade", 2.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);

        // Only a quarter second of source exists before the in-point, so that
        // is the whole dissolve.
        assert_eq!(clips[1].video_fade_in, 0.25);
        assert_eq!(clips[1].source_start, 0.0);
        assert_eq!(clips[1].start, 3.75);
    }

    #[test]
    fn a_still_needs_no_handle_to_dissolve() {
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("image", 0, 4.0, 4.0, 0.0),
        ];
        clips[1].transition = spec("cross-fade", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);
        assert_eq!(clips[1].video_fade_in, 1.0);
        assert_eq!(
            clips[1].source_start, 0.0,
            "a still has no source clock to rewind"
        );
    }

    #[test]
    fn a_fade_to_black_splits_across_the_cut_as_fade_filters() {
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("video", 0, 4.0, 4.0, 0.0),
        ];
        clips[1].transition = spec("fade-black", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);

        // Half a second each side at 30fps is 15 frames.
        assert_eq!(
            clips[0].video_filter_chain,
            "fade=t=out:start_frame=105:nb_frames=15"
        );
        assert_eq!(
            clips[1].video_filter_chain,
            "fade=t=in:start_frame=0:nb_frames=15"
        );
        assert_eq!(clips[1].start, 4.0, "nothing moves for an edge fade");
    }

    #[test]
    fn a_fade_to_white_names_its_colour() {
        let mut clips = vec![
            clip("video", 0, 0.0, 2.0, 0.0),
            clip("video", 0, 2.0, 2.0, 0.0),
        ];
        clips[1].transition = spec("fade-white", 0.5);
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);
        assert!(clips[0].video_filter_chain.ends_with(":color=white"));
        assert!(clips[1].video_filter_chain.contains("t=in"));
    }

    #[test]
    fn transition_fades_append_after_the_clips_own_effects() {
        let mut clips = vec![
            clip("video", 0, 0.0, 2.0, 0.0),
            clip("video", 0, 2.0, 2.0, 0.0),
        ];
        clips[1].video_filter_chain = "hue=s=0".to_owned();
        clips[1].transition = spec("fade-black", 0.5);
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);
        assert!(
            clips[1].video_filter_chain.starts_with("hue=s=0,fade=t=in"),
            "was: {}",
            clips[1].video_filter_chain
        );
    }

    #[test]
    fn a_transition_with_no_adjacent_clip_is_orphaned_not_fatal() {
        let mut clips = vec![
            clip("video", 0, 0.0, 2.0, 0.0),
            clip("video", 0, 5.0, 2.0, 0.0),
        ];
        clips[1].transition = spec("cross-fade", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);
        assert_eq!(clips[1].start, 5.0);
        assert_eq!(clips[1].video_fade_in, 0.0);
    }

    #[test]
    fn track_indices_are_doubled_for_everyone() {
        let mut clips = vec![
            clip("video", 0, 0.0, 2.0, 0.0),
            clip("audio", 3, 0.0, 2.0, 0.0),
        ];
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);
        assert_eq!(clips[0].track, 0);
        assert_eq!(clips[1].track, 6);
    }

    #[test]
    fn an_unknown_transition_kind_renders_as_a_plain_cut() {
        let mut clips = vec![
            clip("video", 0, 0.0, 2.0, 0.0),
            clip("video", 0, 2.0, 2.0, 0.0),
        ];
        clips[1].transition = spec("wipe-left", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);
        assert_eq!(clips[1].start, 2.0);
        assert!(clips[1].video_filter_chain.is_empty());
    }
}

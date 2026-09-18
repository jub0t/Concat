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
mod resolve;

use resolve::{BuiltTimeline, RidingChain, Treatment, animation_of, build_timeline, quantise};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use concat_core::SpeedCurve;
use concat_core::animate::{Animation, Ease as AnimEase, Key as AnimKey, Track as AnimTrack};
use concat_core::frame::Frame;
use concat_core::shader::ShaderPass;
use concat_core::time::{FrameRate, Rational};
use concat_core::timeline::{Clip, ClipId, MediaRef, Timeline, Track, TrackKind, Transform};
use concat_effects::Catalogue;
use concat_media::audio::{self, AudioClip};
use concat_media::{
    DecodeOptions, Decoder, EncodeOptions, Encoder, FrameSink, FrameSource, RateMode, VideoCodec,
};
use concat_project::model::{AppliedFilter, Cutout};
use concat_render::{
    Compositor, CpuCompositor, Layer, Placement, Treatment as GpuTreatment, plan_frame,
};
use concat_vision::{Mapping, MaskStore};
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
    /// A treatment with no pixels of its own: its chain runs over everything
    /// composited beneath its track for its span, blended back by its
    /// opacity, ramped by its fades. See `composite_treated`.
    Layer,
}

impl ClipKind {
    /// True for the kinds that put pixels on screen.
    fn is_visual(self) -> bool {
        matches!(self, ClipKind::Video | ClipKind::Image)
    }
}

/// One clip, as the frontend's flattener describes it.
#[derive(Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExportClip {
    /// The media file this clip shows or plays.
    pub path: String,
    /// Which of the file's audio streams the clip plays, by stream index;
    /// absent is the first in file order. See the document's
    /// `Clip::audio_stream`.
    #[serde(default)]
    pub audio_stream: Option<u32>,
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
    /// Speed over the clip as `(at, speed)` points, `at` a fraction of the
    /// clip's length; empty for the constant `speed`. See `SpeedCurve`.
    #[serde(default)]
    pub speed_curve: Vec<(f64, f64)>,
    /// Played backwards.
    #[serde(default)]
    pub reverse: bool,
    /// Keys over the clip's placement and opacity, resolved by the UI from
    /// its animation presets. Empty for none.
    #[serde(default)]
    pub animation: Vec<ExportKey>,
    /// Mirrored left to right.
    #[serde(default)]
    pub flip_h: bool,
    /// Mirrored top to bottom.
    #[serde(default)]
    pub flip_v: bool,
    /// The blend mode's name; empty or "normal" is source-over.
    #[serde(default)]
    pub blend: String,
    /// Fractions cut off the source's left, top, right and bottom before it
    /// is fitted; absent for none.
    #[serde(default)]
    pub crop: Option<[f64; 4]>,
    /// The clip's applied effects, as the document holds them. When present
    /// they take precedence over `video_filter_chain`: the renderer builds
    /// the chain for its own backend from them, and runs the ones with
    /// shaders on the GPU.
    #[serde(default)]
    pub effects: Vec<AppliedFilter>,
    /// The fades transition resolution bakes in, kept apart from the effects
    /// so either can be rebuilt without the other.
    #[serde(skip)]
    pub transition_chain: String,
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
    /// Multipliers on the fitted width and height beyond `scale`, for a
    /// picture pulled along one axis; 1 keeps the aspect.
    #[serde(default = "unity")]
    pub stretch_x: f64,
    /// The height's half of `stretch_x`'s pair.
    #[serde(default = "unity")]
    pub stretch_y: f64,
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
    /// The background taken away by a mask, as the document holds it.
    /// Rendered only with `mask_dir`: a cutout whose masks are nowhere yet
    /// leaves the picture whole.
    #[serde(default)]
    pub cutout: Option<Cutout>,
    /// Draw the cutout tinted over the whole picture instead of cutting
    /// it: the view while the clip's brushes are in use. Preview only.
    #[serde(default)]
    pub highlighted: bool,
    /// Where this clip's media has its masks - see `concat_vision::mask_dir`
    /// - or empty when the flattener had no project folder to name it by.
    #[serde(default)]
    pub mask_dir: String,
}

impl ExportClip {
    /// A clip with nothing set but what places it: no file, unity gain,
    /// scale and speed, no fades, chains, keys or masks. Every literal in
    /// the workspace starts here and sets the few fields it means to, so a
    /// field added to this type is added in one place.
    pub fn blank(kind: ClipKind, start: f64, duration: f64, track: usize) -> ExportClip {
        ExportClip {
            path: String::new(),
            audio_stream: None,
            kind,
            start,
            duration,
            source_start: 0.0,
            track,
            hidden: false,
            muted: false,
            volume: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            filter_chain: String::new(),
            speed: 1.0,
            preserve_pitch: true,
            speed_curve: Vec::new(),
            reverse: false,
            animation: Vec::new(),
            flip_h: false,
            flip_v: false,
            blend: String::new(),
            crop: None,
            effects: Vec::new(),
            transition_chain: String::new(),
            scale: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
            rotation: 0.0,
            stretch_x: 1.0,
            stretch_y: 1.0,
            opacity: 1.0,
            video_filter_chain: String::new(),
            transition: None,
            video_fade_in: 0.0,
            media_width: None,
            media_height: None,
            has_audio: None,
            cutout: None,
            mask_dir: String::new(),
            highlighted: false,
        }
    }
}

/// One animation key, as the flattener hands it over.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExportKey {
    /// "scale", "offsetX", "offsetY", "rotation", "opacity" or "volume".
    pub property: String,
    /// Where in the clip, `0..=1`.
    pub at: f64,
    /// The value there. Relative to the clip's own for a key that came
    /// from an animation preset, and absolute for one the user set - which
    /// is the same thing, because `flatten::export_base` hands the engine a
    /// neutral constant for every property the user has keyed.
    pub value: f64,
    /// The timing function into this key, as a CSS cubic-bezier's two
    /// control points: `[x1, y1, x2, y2]`. Absent is a straight line.
    #[serde(default = "linear_ease")]
    pub ease: [f64; 4],
}

/// A straight line, for a spec that names no easing.
fn linear_ease() -> [f64; 4] {
    [0.0, 0.0, 1.0, 1.0]
}

/// A transition on the cut into a clip.
#[derive(Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TransitionSpec {
    /// "cross-fade", "fade-black", "fade-white", "push", "zoom", "wipe-left"
    /// or "wipe-right". Anything else renders as a cut.
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
    /// What to encode to, by name: "h264", "hevc" or "av1". H.264 when a
    /// request does not say.
    #[serde(default, deserialize_with = "codec_by_name")]
    pub codec: VideoCodec,
    /// Ten bits a channel rather than eight.
    #[serde(default)]
    pub ten_bit: bool,
    /// VBR (the CRF carries the quality) or CBR (the bitrate is the
    /// target). VBR when a request does not say.
    #[serde(default, deserialize_with = "rate_mode_by_name")]
    pub rate_mode: RateMode,
    /// Target bitrate in kbps, used when `rate_mode` is CBR. Zero means
    /// "not set" and the encoder falls back to VBR.
    #[serde(default)]
    pub bitrate_kbps: u32,
    /// The flattened clip list to render.
    pub clips: Vec<ExportClip>,
}

/// A codec named the way [`VideoCodec::name`] names it, refusing a name
/// the engine has no encoder for rather than quietly falling back.
fn codec_by_name<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<VideoCodec, D::Error> {
    let name = String::deserialize(deserializer)?;
    VideoCodec::parse(&name).ok_or_else(|| {
        serde::de::Error::custom(format!("unknown codec {name:?}: h264, hevc or av1"))
    })
}

/// A rate mode named the way [`RateMode::name`] names it, refusing a name
/// the engine does not know rather than quietly falling back to VBR.
fn rate_mode_by_name<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<RateMode, D::Error> {
    let name = String::deserialize(deserializer)?;
    RateMode::ALL
        .iter()
        .find(|mode| mode.name() == name)
        .copied()
        .ok_or_else(|| serde::de::Error::custom(format!("unknown rate mode {name:?}: vbr or cbr")))
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
/// how to draw: overlapping clips, opacity ramps, placement keys, and fade
/// and mask filters.
///
/// Track indices are doubled first, so an incoming clip gets an odd lane of
/// its own directly above the pair it crosses over - stacking against every
/// other track is preserved, and nothing else occupies odd lanes. Cuts are
/// collected before anything mutates, so resolving one transition cannot
/// unhook the adjacency test of the next.
///
/// Every kind but the fades to a colour is built on one overlap: the
/// incoming clip extends backwards over the outgoing one by the transition's
/// length, on the lane above, showing the handle before its in-point. What
/// differs is how the two are blended across that overlap - a dissolve
/// ramps the incoming clip's opacity, a push slides both, a zoom scales
/// both under a dissolve, a wipe uncovers the incoming one behind a moving
/// edge. The slides and scales are keys on the clips' animations, which
/// the plan already plays for the monitor and the export alike; the wipe is
/// a mask filter, which only the export bakes (`bake_fades`), and the
/// monitor shows a dissolve in its place - the same split the fades to a
/// colour make, and for the same reason: the filter counts frames from the
/// clip's start, which the monitor's pooled seeks do not.
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
            "cross-fade" | "push" | "zoom" | "wipe-left" | "wipe-right" => {
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
                // Sound rides the picture: the pre-roll fades in rather than
                // arriving at full level a dissolve early.
                b.fade_in = b.fade_in.max(d);
                b.track = a_track + 1;

                // How the two blend across the overlap. A shape that cannot
                // be applied - a clip the user has already keyed on the
                // property the shape would ride - falls back to the dissolve
                // rather than half-applying, so the cut still transitions.
                let shaped = match cut.kind.as_str() {
                    // The new picture slides in from the right and shoves the
                    // old one out to the left, edge to edge: both ride the
                    // same ease over the same seconds, which is what keeps
                    // them glued.
                    "push" => {
                        rides(clips, cut.outgoing, cut.incoming, "offsetX")
                            && ride(
                                &mut clips[cut.incoming],
                                "offsetX",
                                1.0,
                                0.0,
                                d,
                                true,
                                EASE_IN_OUT,
                            )
                            && ride(
                                &mut clips[cut.outgoing],
                                "offsetX",
                                0.0,
                                -1.0,
                                d,
                                false,
                                EASE_IN_OUT,
                            )
                    }
                    // The old picture grows as it dissolves into the new one,
                    // which settles from a little large to its own size.
                    "zoom" => {
                        rides(clips, cut.outgoing, cut.incoming, "scale")
                            && ride(
                                &mut clips[cut.incoming],
                                "scale",
                                1.25,
                                1.0,
                                d,
                                true,
                                EASE_OUT,
                            )
                            && ride(
                                &mut clips[cut.outgoing],
                                "scale",
                                1.0,
                                1.4,
                                d,
                                false,
                                EASE_IN,
                            )
                            && {
                                clips[cut.incoming].video_fade_in = d;
                                true
                            }
                    }
                    // A straight edge sweeps across and uncovers the new
                    // picture behind it. Baked only where the frame count
                    // means something; see the function's docs.
                    "wipe-left" | "wipe-right" if bake_fades => {
                        let frames = ((d * fps).round() as i64).max(1);
                        // `N` counts the decoded frames from the clip's new,
                        // earlier start, so the edge is at the left at 0 and
                        // off the far side by `frames`; after that the filter
                        // is switched off and the picture is whole.
                        let uncovered = if cut.kind == "wipe-right" {
                            format!("lt(X,W*(N+1)/{frames})")
                        } else {
                            format!("gte(X,W*(1-(N+1)/{frames}))")
                        };
                        append_filter(
                            &mut clips[cut.incoming].transition_chain,
                            &format!(
                                "format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':\
                                 a='alpha(X,Y)*{uncovered}':enable='lt(n,{frames})'"
                            ),
                        );
                        true
                    }
                    _ => false,
                };
                if !shaped {
                    clips[cut.incoming].video_fade_in = d;
                }
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
                        &mut a.transition_chain,
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
                        &mut b.transition_chain,
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

/// The timing functions the transition shapes ride on, as `ExportKey` holds
/// them: CSS `ease-in-out`, `ease-out` and `ease-in`.
const EASE_IN_OUT: [f64; 4] = [0.42, 0.0, 0.58, 1.0];
const EASE_OUT: [f64; 4] = [0.0, 0.0, 0.58, 1.0];
const EASE_IN: [f64; 4] = [0.42, 0.0, 1.0, 1.0];

/// Whether a transition may key `property` on both sides of a cut: neither
/// clip carries keys on it already. Two rides on one property is a question
/// with no good answer, and the ones the user set are the ones they will be
/// looking at.
fn rides(clips: &[ExportClip], outgoing: usize, incoming: usize, property: &str) -> bool {
    let keyed = |clip: &ExportClip| clip.animation.iter().any(|key| key.property == property);
    !keyed(&clips[outgoing]) && !keyed(&clips[incoming])
}

/// Keys `property` on `clip` from `from` to `to` over `seconds` at its head
/// (`at_head`) or its tail, easing into the second key. Values are relative
/// to the clip's own, as animation keys are: an offset adds, a scale
/// multiplies. The first key holds before it and the second after, so the
/// clip rests at `from` until the ride and at `to` past it.
fn ride(
    clip: &mut ExportClip,
    property: &str,
    from: f64,
    to: f64,
    seconds: f64,
    at_head: bool,
    ease: [f64; 4],
) -> bool {
    if clip.duration <= 0.0 {
        return false;
    }
    let fraction = (seconds / clip.duration).clamp(0.0, 1.0);
    let (start, end) = if at_head {
        (0.0, fraction)
    } else {
        (1.0 - fraction, 1.0)
    };
    clip.animation.push(ExportKey {
        property: property.to_owned(),
        at: start,
        value: from,
        ease: linear_ease(),
    });
    clip.animation.push(ExportKey {
        property: property.to_owned(),
        at: end,
        value: to,
        ease,
    });
    true
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
        .filter(|clip| (clip.kind.is_visual() || clip.kind == ClipKind::Layer) && !clip.hidden)
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
        let mix: Vec<AudioClip> = sound.iter().flat_map(|clip| audio_pieces(clip)).collect();
        audio::mix_to_file(&mix, timeline_end, &mixed).map_err(|error| error.to_string())?;

        reporter.cancelled()?;
        reporter.emit(total_frames, total_frames, "muxing");
        audio::mux(&silent, &mixed, &output).map_err(|error| error.to_string())
    })();

    let _ = std::fs::remove_file(&silent);
    let _ = std::fs::remove_file(&mixed);

    result.map(|()| output.to_string_lossy().into_owned())
}

/// The clip's gain track, or an empty one when its gain is the single number
/// in `ExportClip::volume`.
fn volume_track(clip: &ExportClip) -> AnimTrack {
    animation_of(&clip.animation)
        .map(|animation| animation.volume)
        .unwrap_or_default()
}

/// A gain track re-expressed against a piece covering `[x0, x1]` of the clip.
///
/// A piece has its own clock starting at zero, so a key three-quarters of
/// the way through the clip is nowhere near three-quarters of the way
/// through the piece that holds it. The ends are pinned to what the whole
/// track is worth there, which is what carries a ramp that started in an
/// earlier piece into this one.
fn track_slice(track: &AnimTrack, x0: f64, x1: f64) -> AnimTrack {
    if track.is_empty() {
        return AnimTrack::default();
    }
    let span = x1 - x0;
    if span <= 0.0 {
        return AnimTrack::new(vec![AnimKey {
            at: 0.0,
            value: track.value_at(x0, 1.0),
            ease: AnimEase::LINEAR,
        }]);
    }
    let mut keys = vec![AnimKey {
        at: 0.0,
        value: track.value_at(x0, 1.0),
        ease: AnimEase::LINEAR,
    }];
    keys.extend(
        track
            .keys()
            .iter()
            .filter(|key| key.at > x0 && key.at < x1)
            .map(|key| AnimKey {
                at: (key.at - x0) / span,
                value: key.value,
                ease: key.ease,
            }),
    );
    // The ease of whichever segment this end falls inside, so a ramp that
    // crosses the boundary keeps its shape rather than going linear at it.
    let closing = track
        .keys()
        .iter()
        .find(|key| key.at >= x1)
        .map_or(AnimEase::LINEAR, |key| key.ease);
    keys.push(AnimKey {
        at: 1.0,
        value: track.value_at(x1, 1.0),
        ease: closing,
    });
    AnimTrack::new(keys)
}

/// The engine's view of one audible clip - or several, when its speed
/// changes over it. Sound can only change tempo in steps, so a curve is cut
/// into pieces of constant rate, each at the mean of its stretch of the
/// curve and starting where the curve says the source had got to. A reverse
/// runs the pieces' sound backwards, and the pieces themselves in reverse
/// order of source, which is what playing the clip backwards means.
pub fn audio_pieces(clip: &ExportClip) -> Vec<AudioClip> {
    let mut chain = clip.filter_chain.clone();
    if clip.reverse {
        chain = if chain.is_empty() {
            "areverse".to_owned()
        } else {
            format!("areverse,{chain}")
        };
    }
    let track = volume_track(clip);
    let Some(curve) = SpeedCurve::new(&clip.speed_curve) else {
        return vec![AudioClip {
            path: PathBuf::from(&clip.path),
            stream: clip.audio_stream.map(|index| index as usize),
            start: clip.start,
            duration: clip.duration,
            source_start: clip.source_start,
            speed: audio::clamp_speed(clip.speed),
            preserve_pitch: clip.preserve_pitch,
            volume: clip.volume,
            volume_curve: track,
            fade_in: clip.fade_in,
            fade_out: clip.fade_out,
            filter_chain: chain,
        }];
    };
    // Pieces a tenth of a second long, or eight at least: fine enough that
    // a tempo step is not heard, coarse enough that the graph stays small.
    let count = ((clip.duration / 0.1).ceil() as usize).clamp(8, 400);
    let span = curve.mean() * clip.duration;
    curve
        .pieces(count)
        .into_iter()
        .map(|(x0, x1, consumed, mean)| {
            let piece_duration = (x1 - x0) * clip.duration;
            let forward = consumed * clip.duration;
            let source_start = if clip.reverse {
                // Backwards: this piece plays the source that ends where the
                // forward map had got to, so it starts one piece earlier.
                clip.source_start + (span - forward - mean * piece_duration).max(0.0)
            } else {
                clip.source_start + forward
            };
            let piece_start = clip.start + x0 * clip.duration;
            let piece_end = piece_start + piece_duration;
            // The clip's fades, as they fall on this piece.
            let fade_in = (clip.fade_in - x0 * clip.duration).clamp(0.0, piece_duration);
            let fade_out_from = clip.start + clip.duration - clip.fade_out;
            let fade_out = (piece_end - fade_out_from.max(piece_start)).clamp(0.0, piece_duration);
            AudioClip {
                path: PathBuf::from(&clip.path),
                stream: clip.audio_stream.map(|index| index as usize),
                start: piece_start,
                duration: piece_duration,
                source_start,
                speed: audio::clamp_speed(mean),
                preserve_pitch: clip.preserve_pitch,
                volume: clip.volume,
                volume_curve: track_slice(&track, x0, x1),
                fade_in: if clip.fade_in > 0.0 { fade_in } else { 0.0 },
                fade_out: if clip.fade_out > 0.0 { fade_out } else { 0.0 },
                filter_chain: chain.clone(),
            }
        })
        .collect()
}

/// One decoded picture on its way to the compositor, with everything the
/// layer it becomes needs. Generic over how the frame is held: owned in the
/// export, shared out of the reader pool in the monitor.
struct Source<F> {
    frame: F,
    opacity: f32,
    transform: Transform,
    track: usize,
    blend: concat_core::timeline::Blend,
    passes: Vec<ShaderPass>,
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
            stretch_x: transform.stretch_x as f32,
            stretch_y: transform.stretch_y as f32,
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
fn best_compositor() -> (Box<dyn Compositor>, bool) {
    #[cfg(feature = "gpu")]
    if let Some(gpu) = concat_render::WgpuCompositor::new() {
        return (Box::new(gpu), true);
    }
    (Box::new(CpuCompositor), false)
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
    let (mut compositor, gpu) = best_compositor();
    let BuiltTimeline {
        timeline,
        stills,
        decode_sizes,
        filter_chains,
        tracks,
        treatments,
        pre_chains,
        passes,
        riding,
        cutouts,
        highlight: _,
    } = build_timeline(request, rate, visible, gpu);

    let mut encoder = Encoder::create(
        destination,
        request.width,
        request.height,
        rate,
        &EncodeOptions {
            crf: request.crf,
            preset: request.preset.clone(),
            codec: request.codec,
            rate_mode: request.rate_mode,
            bitrate_kbps: request.bitrate_kbps,
            ten_bit: request.ten_bit,
            hardware: true,
        },
    )
    .map_err(|error| error.to_string())?;

    // One decoder per clip, opened at its in-point the first time the clip is
    // needed and dropped the moment it leaves the playhead. Every decoder is
    // opened at `output rate / clip speed`, so pulling exactly one frame per
    // output frame keeps each of them in step with the plan's source times
    // without any seeking - including retimed clips.
    let mut decoders: HashMap<ClipId, Decoder> = HashMap::new();
    // A clip whose speed changes, or runs backwards, cannot be followed by a
    // paced decoder: each of its frames is sought by its own source time,
    // through a pool that keeps the reader rolling where it can.
    let sought = concat_media::ReaderPool::with_defaults();

    for index in 0..total_frames {
        reporter.cancelled()?;

        let time = rate.time_of_frame(index);
        let frame_seconds = time.as_f64();
        let plan = plan_frame(&timeline, time);

        let mut sources: Vec<Source<Frame>> = Vec::with_capacity(plan.layers.len());
        for layer in &plan.layers {
            if !layer.paced {
                let (decode_width, decode_height) = decode_sizes
                    .get(&layer.clip)
                    .copied()
                    .unwrap_or((request.width, request.height));
                let chain = filter_chains.get(&layer.clip).map(String::as_str);
                let pre = pre_chains.get(&layer.clip).map(String::as_str);
                if let Ok(frame) = sought.frame_at(
                    &layer.media,
                    layer.source_time,
                    decode_width,
                    decode_height,
                    stills.contains(&layer.clip),
                    chain,
                    pre,
                ) {
                    let frame = cutouts
                        .get(&layer.clip)
                        .and_then(|job| job.cut(&frame, layer.source_time))
                        .unwrap_or_else(|| frame.as_ref().clone());
                    sources.push(Source {
                        frame,
                        opacity: layer.opacity,
                        transform: layer.transform,
                        track: tracks.get(&layer.clip).copied().unwrap_or(0),
                        blend: layer.blend,
                        passes: passes_at(&passes, &riding, layer.clip, frame_seconds),
                    });
                }
                continue;
            }
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
                    if let Some(pre) = pre_chains.get(&layer.clip) {
                        options = options.prefiltered(pre.clone());
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
                // The cutout, on the decoded picture before it is placed:
                // the same frame both compositors then draw.
                let frame = match cutouts
                    .get(&layer.clip)
                    .and_then(|job| job.cut(&frame, layer.source_time))
                {
                    Some(cut) => cut,
                    None => frame,
                };
                sources.push(Source {
                    frame,
                    opacity: layer.opacity,
                    transform: layer.transform,
                    track: tracks.get(&layer.clip).copied().unwrap_or(0),
                    blend: layer.blend,
                    passes: passes_at(&passes, &riding, layer.clip, frame_seconds),
                });
            }
        }

        let seconds = time.as_f64() as f32;
        let layers: Vec<(Layer<'_>, usize)> = sources
            .iter()
            .map(|source| {
                (
                    place_layer(
                        &source.frame,
                        source.opacity,
                        &source.transform,
                        request.width,
                        request.height,
                    )
                    .with_blend(source.blend)
                    .with_passes(&source.passes)
                    .at_time(seconds),
                    source.track,
                )
            })
            .collect();

        let composed = composite_treated(
            &mut *compositor,
            request.width,
            request.height,
            time,
            &layers,
            &treatments,
        );
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

/// The shader passes for one clip at one frame: built for the instant when
/// the chain rides, and the ones built once otherwise.
fn passes_at(
    passes: &HashMap<ClipId, Vec<ShaderPass>>,
    riding: &HashMap<ClipId, RidingChain>,
    clip: ClipId,
    seconds: f64,
) -> Vec<ShaderPass> {
    if let Some(chain) = riding.get(&clip) {
        let at = if chain.duration > 0.0 {
            ((seconds - chain.start) / chain.duration).clamp(0.0, 1.0)
        } else {
            0.0
        };
        return Catalogue::builtin().shader_passes_at(&chain.effects, at);
    }
    passes.get(&clip).cloned().unwrap_or_default()
}

/// `a` towards `b` by `amount`, per channel.
fn mix(a: &Frame, b: &Frame, amount: f32) -> Frame {
    let amount = amount.clamp(0.0, 1.0);
    let mut out = a.clone();
    for (pixel, over) in out.pixels_mut().iter_mut().zip(b.pixels().iter()) {
        let base = f32::from(*pixel);
        *pixel = (base + (f32::from(*over) - base) * amount).round() as u8;
    }
    out
}

/// Composites `sources` - each placed layer with the track it came from,
/// bottom-most first - with every treatment live at `time` applied to the
/// stack beneath its track. Without a live treatment this is one composite;
/// with one, the stack is drawn up to the treatment's track, run through its
/// chain, blended back by its strength, and used as the ground for the rest.
fn composite_treated(
    compositor: &mut dyn Compositor,
    width: u32,
    height: u32,
    time: Rational,
    sources: &[(Layer<'_>, usize)],
    treatments: &[Treatment],
) -> Frame {
    let mut live: Vec<&Treatment> = treatments
        .iter()
        .filter(|treatment| treatment.covers(time))
        .collect();
    if live.is_empty() {
        let layers: Vec<Layer<'_>> = sources.iter().map(|(layer, _)| *layer).collect();
        return compositor.composite(width, height, &layers);
    }
    live.sort_by_key(|treatment| treatment.track);

    // A compositor that runs passes applies every treatment without a
    // pixel leaving the GPU, so long as none of them needs FFmpeg for
    // a package that has no shader.
    if live.iter().all(|treatment| treatment.chain.is_empty()) {
        let gpu: Vec<GpuTreatment<'_>> = live
            .iter()
            .map(|treatment| GpuTreatment {
                track: treatment.track,
                passes: &treatment.passes,
                strength: treatment.strength_at(time),
            })
            .collect();
        if let Some(frame) =
            compositor.composite_treated(width, height, time.as_f64() as f32, sources, &gpu)
        {
            return frame;
        }
    }

    let mut ground: Option<Frame> = None;
    let mut next = 0;
    for treatment in live {
        let below = {
            let mut layers: Vec<Layer<'_>> = ground.as_ref().map(Layer::new).into_iter().collect();
            while next < sources.len() && sources[next].1 < treatment.track {
                layers.push(sources[next].0);
                next += 1;
            }
            compositor.composite(width, height, &layers)
        };
        let strength = treatment.strength_at(time);
        let treated = if strength <= 0.0 {
            below
        } else {
            // Shader passes run through the compositor over the ground as a
            // layer of its own; whatever is left for FFmpeg runs after.
            let shaded = if treatment.passes.is_empty() {
                below.clone()
            } else {
                let ground = [Layer::new(&below)
                    .with_passes(&treatment.passes)
                    .at_time(time.as_f64() as f32)];
                compositor.composite(width, height, &ground)
            };
            let result = if treatment.chain.is_empty() {
                Ok(shaded)
            } else {
                concat_media::treat(&shaded, &treatment.chain)
            };
            match result {
                Ok(treated) if strength >= 1.0 => treated,
                Ok(treated) => mix(&below, &treated, strength),
                // A chain FFmpeg refuses leaves the picture as it was
                // rather than blanking it; the export says nothing because
                // the catalogue validated every template at load.
                Err(_) => below,
            }
        };
        ground = Some(treated);
    }
    let mut layers: Vec<Layer<'_>> = ground.as_ref().map(Layer::new).into_iter().collect();
    layers.extend(sources[next..].iter().map(|(layer, _)| *layer));
    compositor.composite(width, height, &layers)
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
    let sources = preview_sources(pool, request, false)?;
    // CPU on purpose: one frame at preview size is milliseconds, and holding
    // a GPU context alive for occasional scrubs is not worth its memory. The
    // window composites on its own device through `preview_sources`.
    Ok(sources.composite(&mut CpuCompositor).into_pixels())
}

/// The decoded pictures under the playhead and where each goes: what a
/// compositor needs to draw the paused monitor's frame, decoded but not yet
/// drawn, so a caller with a GPU can draw them where they are shown.
pub struct PreviewSources {
    sources: Vec<Source<std::sync::Arc<Frame>>>,
    width: u32,
    height: u32,
    time: Rational,
    treatments: Vec<Treatment>,
}

impl PreviewSources {
    /// The layers, placed the exporter's way, bottom-most first. Bare: a
    /// caller drawing these itself is skipping the treatments, so check
    /// [`PreviewSources::has_treatments`] first.
    pub fn layers(&self) -> Vec<Layer<'_>> {
        let seconds = self.time.as_f64() as f32;
        self.sources
            .iter()
            .map(|source| {
                place_layer(
                    source.frame.as_ref(),
                    source.opacity,
                    &source.transform,
                    self.width,
                    self.height,
                )
                .with_blend(source.blend)
                .with_passes(&source.passes)
                .at_time(seconds)
            })
            .collect()
    }

    /// Whether a layer clip is live at this instant, in which case the frame
    /// cannot be drawn from `layers` alone: see [`PreviewSources::composite`].
    pub fn has_treatments(&self) -> bool {
        self.treatments
            .iter()
            .any(|treatment| treatment.covers(self.time))
    }

    /// The layers as [`PreviewSources::layers`] gives them, each with the
    /// track it came from, for a compositor applying treatments itself.
    pub fn placed(&self) -> Vec<(Layer<'_>, usize)> {
        let seconds = self.seconds();
        self.sources
            .iter()
            .map(|source| {
                (
                    place_layer(
                        source.frame.as_ref(),
                        source.opacity,
                        &source.transform,
                        self.width,
                        self.height,
                    )
                    .with_blend(source.blend)
                    .with_passes(&source.passes)
                    .at_time(seconds),
                    source.track,
                )
            })
            .collect()
    }

    /// The treatments live at this instant, in ascending track order, as
    /// a GPU compositor takes them - or None when one of them needs FFmpeg
    /// for a package with no shader, and only [`PreviewSources::composite`]
    /// can draw the frame.
    pub fn live_treatments(&self) -> Option<Vec<GpuTreatment<'_>>> {
        let mut live: Vec<&Treatment> = self
            .treatments
            .iter()
            .filter(|treatment| treatment.covers(self.time))
            .collect();
        if live.iter().any(|treatment| !treatment.chain.is_empty()) {
            return None;
        }
        live.sort_by_key(|treatment| treatment.track);
        Some(
            live.iter()
                .map(|treatment| GpuTreatment {
                    track: treatment.track,
                    passes: &treatment.passes,
                    strength: treatment.strength_at(self.time),
                })
                .collect(),
        )
    }

    /// The instant, in seconds, for passes that move.
    pub fn seconds(&self) -> f32 {
        self.time.as_f64() as f32
    }

    /// The frame, treatments included, drawn with `compositor`.
    pub fn composite(&self, compositor: &mut dyn Compositor) -> Frame {
        let placed = self.placed();
        composite_treated(
            compositor,
            self.width,
            self.height,
            self.time,
            &placed,
            &self.treatments,
        )
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
    gpu: bool,
) -> Result<PreviewSources, String> {
    preview_sources_of(pool, &preview_timeline(request, gpu), request.time)
}

/// [`preview_sources`] for one instant of a plan already built.
pub fn preview_sources_of(
    pool: &concat_media::ReaderPool,
    plan: &PreviewPlan,
    seconds: f64,
) -> Result<PreviewSources, String> {
    let rate = plan.rate;
    let BuiltTimeline {
        timeline,
        stills,
        decode_sizes,
        filter_chains,
        tracks,
        treatments,
        pre_chains,
        passes,
        riding,
        cutouts,
        highlight,
    } = &plan.built;
    let highlight = *highlight;
    let time = quantise(seconds, rate);
    let frame_seconds = time.as_f64();
    let plan_at = plan_frame(timeline, time);

    let mut sources: Vec<Source<std::sync::Arc<Frame>>> = Vec::with_capacity(plan_at.layers.len());
    let mut failures: Vec<String> = Vec::new();
    for layer in &plan_at.layers {
        let (decode_width, decode_height) = decode_sizes
            .get(&layer.clip)
            .copied()
            .unwrap_or((plan.width, plan.height));
        let chain = filter_chains.get(&layer.clip).map(String::as_str);
        let pre = pre_chains.get(&layer.clip).map(String::as_str);
        // A source that fails to decode contributes nothing rather than
        // blanking the monitor - same grace the exporter extends.
        match pool.frame_at(
            std::path::Path::new(&layer.media),
            layer.source_time,
            decode_width,
            decode_height,
            stills.contains(&layer.clip),
            chain,
            pre,
        ) {
            Ok(frame) => {
                let highlighted = highlight == Some(layer.clip);
                let frame = match cutouts.get(&layer.clip).and_then(|job| {
                    if highlighted {
                        job.highlight(&frame, layer.source_time)
                    } else {
                        job.cut(&frame, layer.source_time)
                    }
                }) {
                    Some(drawn) => std::sync::Arc::new(drawn),
                    None => frame,
                };
                sources.push(Source {
                    frame,
                    opacity: layer.opacity,
                    transform: layer.transform,
                    track: tracks.get(&layer.clip).copied().unwrap_or(0),
                    blend: layer.blend,
                    passes: passes_at(passes, riding, layer.clip, frame_seconds),
                })
            }
            Err(error) => failures.push(format!("{}: {error}", layer.media.display())),
        }
    }

    // Planned layers with nothing decoded is a failed preview, not a black
    // frame: compositing zero sources yields opaque black, and the caller
    // would draw that "truth" over its own perfectly good approximation. An
    // *empty plan* still composites - a gap in the timeline really is black.
    if sources.is_empty() && !plan_at.layers.is_empty() {
        return Err(format!(
            "no layer decoded for the paused preview: {}",
            failures.join(" / ")
        ));
    }

    Ok(PreviewSources {
        sources,
        width: plan.width,
        height: plan.height,
        time,
        treatments: treatments.clone(),
    })
}

/// The preview's timeline, built the exporter's way and kept: everything
/// about a clip list that does not depend on the instant, so a caller
/// showing many instants of one document builds it once. See
/// [`preview_plan`].
pub struct PreviewPlan {
    built: BuiltTimeline,
    rate: FrameRate,
    width: u32,
    height: u32,
}

/// Builds the plan for `clips` at one output size and rate. The costly
/// half of a preview - transitions resolved, the engine timeline, every
/// chain and pass, every cutout's masks found on disk - and the half that
/// only changes when the document does.
pub fn preview_plan(
    clips: &[ExportClip],
    width: u32,
    height: u32,
    rate_num: i64,
    rate_den: i64,
    gpu: bool,
) -> PreviewPlan {
    let rate = FrameRate::new(Rational::new(rate_num, rate_den));
    let mut resolved = clips.to_vec();
    resolve_transitions(&mut resolved, rate, false);
    let visible: Vec<&ExportClip> = resolved
        .iter()
        .filter(|clip| (clip.kind.is_visual() || clip.kind == ClipKind::Layer) && !clip.hidden)
        .collect();

    // build_timeline reads only the output format off the request; the shim
    // keeps one conversion path rather than a preview-flavoured copy of it.
    let shim = ExportRequest {
        output: String::new(),
        width,
        height,
        rate_num,
        rate_den,
        crf: 18,
        preset: String::new(),
        codec: VideoCodec::H264,
        ten_bit: false,
        rate_mode: RateMode::Vbr,
        bitrate_kbps: 0,
        clips: Vec::new(),
    };
    PreviewPlan {
        built: build_timeline(&shim, rate, &visible, gpu),
        rate,
        width,
        height,
    }
}

/// The plan a request describes; see [`preview_plan`].
fn preview_timeline(request: &PreviewFrameRequest, gpu: bool) -> PreviewPlan {
    preview_plan(
        &request.clips,
        request.width,
        request.height,
        request.rate_num,
        request.rate_den,
        gpu,
    )
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
    gpu: bool,
) {
    preview_prefetch_of(pool, &preview_timeline(request, gpu), request.time, frames);
}

/// [`preview_prefetch`] from a plan already built.
pub fn preview_prefetch_of(
    pool: &concat_media::ReaderPool,
    plan: &PreviewPlan,
    seconds: f64,
    frames: u32,
) {
    let rate = plan.rate;
    let BuiltTimeline {
        timeline,
        stills,
        decode_sizes,
        filter_chains,
        pre_chains,
        ..
    } = &plan.built;
    let fps = rate.fps().as_f64();

    for ahead in 0..frames {
        let time = seconds + f64::from(ahead) / fps;
        let plan_at = plan_frame(timeline, quantise(time, rate));
        for layer in &plan_at.layers {
            let (decode_width, decode_height) = decode_sizes
                .get(&layer.clip)
                .copied()
                .unwrap_or((plan.width, plan.height));
            let chain = filter_chains.get(&layer.clip).map(String::as_str);
            let pre = pre_chains.get(&layer.clip).map(String::as_str);
            let _ = pool.frame_at(
                std::path::Path::new(&layer.media),
                layer.source_time,
                decode_width,
                decode_height,
                stills.contains(&layer.clip),
                chain,
                pre,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    /// A treatment on track 1 runs over what track 0 drew and not over what
    /// track 2 draws on top of it, and its strength blends the result back.
    #[test]
    fn a_treatment_covers_the_stack_beneath_its_track_only() {
        fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Frame {
            let mut frame = Frame::black(width, height);
            for pixel in frame.pixels_mut().chunks_exact_mut(4) {
                pixel.copy_from_slice(&rgba);
            }
            frame
        }
        // Red fills the frame on track 0; a small blue square sits on
        // track 2 at the top-left.
        let red = solid(8, 8, [255, 0, 0, 255]);
        let blue = solid(2, 2, [0, 0, 255, 255]);
        let sources = [(Layer::new(&red), 0usize), (Layer::new(&blue), 2usize)];
        let negate = Treatment {
            start: Rational::ZERO,
            end: Rational::from_int(10),
            track: 1,
            chain: "negate".to_owned(),
            passes: Vec::new(),
            strength: 1.0,
            ramp_in: 0.0,
            ramp_out: 0.0,
        };
        let mut compositor = CpuCompositor;
        let out = composite_treated(
            &mut compositor,
            8,
            8,
            Rational::from_int(1),
            &sources,
            std::slice::from_ref(&negate),
        );
        // Red negated is cyan where nothing sits on top...
        let at = |x: usize, y: usize| &out.pixels()[(y * 8 + x) * 4..(y * 8 + x) * 4 + 3];
        assert_eq!(at(7, 7), &[0, 255, 255]);
        // ...and the blue square above the treatment is untouched.
        assert_eq!(at(0, 0), &[0, 0, 255]);

        // At half strength the ground is halfway between red and cyan.
        let half = Treatment {
            strength: 0.5,
            ..negate.clone()
        };
        let out = composite_treated(
            &mut compositor,
            8,
            8,
            Rational::from_int(1),
            &sources,
            &[half],
        );
        let pixel = &out.pixels()[(7 * 8 + 7) * 4..(7 * 8 + 7) * 4 + 3];
        assert!(pixel[0] > 120 && pixel[0] < 136, "{pixel:?}");
        assert!(pixel[1] > 120 && pixel[1] < 136, "{pixel:?}");

        // Outside its span the treatment does nothing.
        let out = composite_treated(
            &mut compositor,
            8,
            8,
            Rational::from_int(20),
            &sources,
            &[negate],
        );
        assert_eq!(at_of(&out, 7, 7), [255, 0, 0]);
        fn at_of(frame: &Frame, x: usize, y: usize) -> [u8; 3] {
            let p = &frame.pixels()[(y * 8 + x) * 4..(y * 8 + x) * 4 + 3];
            [p[0], p[1], p[2]]
        }
    }

    use super::*;

    fn clip(kind: &str, track: usize, start: f64, duration: f64, source_start: f64) -> ExportClip {
        let kind_of = match kind {
            "audio" => ClipKind::Audio,
            "image" => ClipKind::Image,
            _ => ClipKind::Video,
        };
        ExportClip {
            path: format!("{kind}.mp4"),
            source_start,
            ..ExportClip::blank(kind_of, start, duration, track)
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

        let pool = concat_media::ReaderPool::new(16 * 1024 * 1024, 2);
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

    /// The keys a resolved transition put on a clip's property, as
    /// `(at, value)` pairs.
    fn keys_on(clip: &ExportClip, property: &str) -> Vec<(f64, f64)> {
        clip.animation
            .iter()
            .filter(|key| key.property == property)
            .map(|key| (key.at, key.value))
            .collect()
    }

    #[test]
    fn a_push_slides_both_pictures_across_the_overlap() {
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("video", 0, 4.0, 4.0, 2.0),
        ];
        clips[1].transition = spec("push", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);

        // The overlap is the dissolve's: a second of pre-roll on the lane
        // above, sound fading in with it - but no picture fade.
        let b = &clips[1];
        assert_eq!((b.start, b.duration, b.source_start), (3.0, 5.0, 1.0));
        assert_eq!(b.track, 1);
        assert_eq!(b.fade_in, 1.0);
        assert_eq!(b.video_fade_in, 0.0, "a push does not dissolve");
        // The incoming picture comes in from a frame's width to the right
        // over its first second (a fifth of its new length).
        assert_eq!(keys_on(b, "offsetX"), vec![(0.0, 1.0), (0.2, 0.0)]);
        // The outgoing one leaves to the left over its last second.
        assert_eq!(
            keys_on(&clips[0], "offsetX"),
            vec![(0.75, 0.0), (1.0, -1.0)]
        );
        // Both ride the same ease, which is what keeps them edge to edge.
        assert_eq!(clips[0].animation[1].ease, clips[1].animation[1].ease);
        assert_eq!(clips[1].animation[1].ease, EASE_IN_OUT);
    }

    #[test]
    fn a_push_over_a_clip_the_user_keyed_becomes_a_dissolve() {
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("video", 0, 4.0, 4.0, 2.0),
        ];
        clips[0].animation.push(ExportKey {
            property: "offsetX".to_owned(),
            at: 0.5,
            value: 0.1,
            ease: linear_ease(),
        });
        clips[1].transition = spec("push", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);
        assert_eq!(clips[1].video_fade_in, 1.0, "falls back to the dissolve");
        assert!(
            keys_on(&clips[1], "offsetX").is_empty(),
            "nothing half-applied"
        );
        assert_eq!(
            keys_on(&clips[0], "offsetX").len(),
            1,
            "the user's key is untouched"
        );
    }

    #[test]
    fn a_zoom_scales_both_under_a_dissolve() {
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("video", 0, 4.0, 4.0, 2.0),
        ];
        clips[1].transition = spec("zoom", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);
        assert_eq!(clips[1].video_fade_in, 1.0);
        assert_eq!(keys_on(&clips[1], "scale"), vec![(0.0, 1.25), (0.2, 1.0)]);
        assert_eq!(keys_on(&clips[0], "scale"), vec![(0.75, 1.0), (1.0, 1.4)]);
    }

    #[test]
    fn a_wipe_is_a_mask_that_switches_off_after_the_overlap() {
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("video", 0, 4.0, 4.0, 2.0),
        ];
        clips[1].transition = spec("wipe-right", 0.5);
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);
        let b = &clips[1];
        assert_eq!((b.start, b.duration), (3.5, 4.5));
        assert_eq!(b.video_fade_in, 0.0, "the edge does the revealing");
        assert_eq!(
            b.transition_chain,
            "format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':\
             a='alpha(X,Y)*lt(X,W*(N+1)/15)':enable='lt(n,15)'"
        );
        assert_eq!(
            clips[0].transition_chain, "",
            "the outgoing picture is untouched"
        );

        // The other direction sweeps from the right edge.
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("video", 0, 4.0, 4.0, 2.0),
        ];
        clips[1].transition = spec("wipe-left", 0.5);
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);
        assert!(clips[1].transition_chain.contains("gte(X,W*(1-(N+1)/15))"));

        // Unbaked - the monitor - a wipe shows as a dissolve, like a fade
        // to a colour shows as the UI's veil.
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("video", 0, 4.0, 4.0, 2.0),
        ];
        clips[1].transition = spec("wipe-right", 0.5);
        resolve_transitions(&mut clips, FrameRate::THIRTY, false);
        assert_eq!(clips[1].transition_chain, "");
        assert_eq!(clips[1].video_fade_in, 0.5);
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
            clips[0].transition_chain,
            "fade=t=out:start_frame=105:nb_frames=15"
        );
        assert_eq!(
            clips[1].transition_chain,
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
        assert!(clips[0].transition_chain.ends_with(":color=white"));
        assert!(clips[1].transition_chain.contains("t=in"));
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
        // The fade lives beside the effects and joins after them when the
        // decoder's chain is built.
        let chain = resolve::full_chain(&clips[1], false);
        assert!(chain.starts_with("hue=s=0,fade=t=in"), "was: {chain}");
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
        // A kind no build knows - the wipes are known now.
        clips[1].transition = spec("spiral", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY, true);
        assert_eq!(clips[1].start, 2.0);
        assert!(clips[1].video_filter_chain.is_empty());
    }
}

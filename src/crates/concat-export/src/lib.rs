// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

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

use resolve::{
    BuiltTimeline, TransitionSpan, Treatment, animation_of, build_timeline, quantise, quantise_at,
};

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use concat_core::SpeedCurve;
use concat_core::animate::{Animation, Ease as AnimEase, Key as AnimKey, Track as AnimTrack};
use concat_core::frame::{Frame, Signal};
use concat_core::shader::{RevealMap, ShaderPass};
use concat_core::time::{FrameRate, Rational};
use concat_core::timeline::{Clip, ClipId, MediaRef, Timeline, Track, TrackKind, Transform};
use concat_effects::Catalogue;
use concat_media::audio::{self, AudioClip};
use concat_media::{
    DecodeOptions, Decoder, EncodeOptions, Encoder, FrameSink, FrameSource, RateMode, VideoCodec,
};
use concat_project::model::{AppliedFilter, ClipMask, ColorSpace, Cutout};
use concat_render::{
    Compositor, FramePlan, PlannedLayer, PlannedTreatment, Transition, WgpuCompositor, plan_frame,
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
    /// A treatment of the sound: its `filter_chain` runs over the mix of
    /// every audible clip for its span, its opacity how much of the treated
    /// sound replaces the untreated, ramped by its fades. No picture, and
    /// no sound of its own; see [`audio_buses`].
    #[serde(rename = "soundLayer")]
    SoundLayer,
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
    /// The levels the file is read as, over its own tag; absent reads the
    /// tag. See the document's `MediaItem::color_range`.
    #[serde(default)]
    pub color_range: Option<concat_project::model::ColorRange>,
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
    /// A slope for each of `speed_curve`'s points when the curve bends
    /// smoothly between them; empty for straight lines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speed_slopes: Vec<f64>,
    /// Keys over the clip's placement and opacity, resolved by the UI from
    /// Empty for none.
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
    /// The clip's applied effects, as the document holds them: each one's
    /// shader runs on the GPU. A link no package answers to draws nothing.
    #[serde(default)]
    pub effects: Vec<AppliedFilter>,
    /// The fades to a colour and the wipes transition resolution gives the
    /// clip, which the frame plan draws; see [`TransitionShape`].
    #[serde(skip)]
    pub transition_shapes: Vec<TransitionShape>,
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
    /// The transition into this clip's cut, when the UI put one there. The
    /// clip before it on the same track is found here, by adjacency - the UI
    /// only says what it wants, never how to overlap decoders.
    #[serde(default)]
    pub transition: Option<TransitionSpec>,
    /// Video opacity ramp up from the clip's start, in seconds. Set by
    /// transition resolution below, never by the UI directly.
    #[serde(default)]
    pub video_fade_in: f64,
    /// One frame - the one at `source_start` - held for the clip's length.
    /// Set by transition resolution where the outgoing clip of a cut has no
    /// footage after its out-point, never by the UI; see [`overlap`].
    #[serde(skip)]
    pub hold: bool,
    /// The source's pixel width, when the UI knows it. What makes an
    /// aspect-correct decode possible - absent, the frame is filled edge to
    /// edge the way it always was.
    #[serde(default)]
    pub media_width: Option<u32>,
    /// The source's pixel height; see `media_width`.
    #[serde(default)]
    pub media_height: Option<u32>,
    /// The source's length in seconds, when the UI knows it: how much
    /// footage lies past the clip's out-point for a transition to run the
    /// clip on into. Absent, a transition holds the clip's last frame
    /// instead.
    #[serde(default)]
    pub media_duration: Option<f64>,
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
    /// The shape that keeps part of the picture, as the document holds
    /// it; its keyed numbers ride in `animation` as `maskX` and on.
    #[serde(default)]
    pub mask: Option<ClipMask>,
    /// Draw the cutout tinted over the whole picture instead of cutting
    /// it: the view while the clip's brushes are in use. Preview only.
    #[serde(default)]
    pub highlighted: bool,
    /// Where this clip's media has its masks - see `concat_vision::mask_dir`
    /// - or empty when the flattener had no project folder to name it by.
    #[serde(default)]
    pub mask_dir: String,
    /// The camera's shake taken out, as the document holds it. Rendered
    /// only with `motion_dir`: a clip whose movement is not measured yet
    /// shows as shot.
    #[serde(default)]
    pub stabilize: Option<concat_project::model::Stabilize>,
    /// Where this clip's media has its measured movement - see
    /// `concat_vision::stabilize::motion_dir` - or empty when the
    /// flattener had no project folder to name it by.
    #[serde(default)]
    pub motion_dir: String,
    /// A title's per-word reveal order, baked at rasterize time and set by
    /// the host - never by the wire format, like `transition_chain`. Every
    /// shader pass over this clip carries it, so a reveal effect can read
    /// it back through `reveal_order()`.
    #[serde(skip)]
    pub reveal_map: Option<Arc<RevealMap>>,
    /// What fills the frame behind the picture where it does not reach, as
    /// the flattener resolved it - the clip's own background or its
    /// timeline's, for a picture on the lowest picture lane; absent for
    /// black and for every other clip. See `Timeline::background_of`.
    #[serde(default)]
    pub backdrop: Option<ExportBackdrop>,
}

/// A clip's background, resolved for drawing: the document's
/// `Background` with its colour read and its still found in the bin.
#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ExportBackdrop {
    /// The clip's own picture, enlarged to cover the frame and blurred.
    Blur {
        /// How soft, `0..=1`: a Gaussian of a twelfth of the frame's
        /// shorter side at one.
        amount: f64,
    },
    /// One opaque colour.
    Colour {
        /// Its channels.
        rgb: [u8; 3],
    },
    /// A still, enlarged to cover the frame.
    Image {
        /// The file.
        path: String,
        /// Its pixel width, where the bin knows it: what it is decoded at.
        width: Option<u32>,
        /// Its pixel height.
        height: Option<u32>,
    },
}

impl ExportBackdrop {
    /// The blur's spread, in pixels of a `width` by `height` frame, at
    /// `amount`: a twelfth of the shorter side at the most, so the same
    /// amount is the same softness at every size the frame is drawn at -
    /// the monitor's quarter and the export's whole alike.
    pub fn sigma(amount: f64, width: u32, height: u32) -> f32 {
        (amount.clamp(0.0, 1.0) * f64::from(width.min(height)) / 12.0) as f32
    }
}

impl ExportClip {
    /// The engine's curve for the clip's speed, smooth when it carries
    /// slopes; None for a constant rate.
    pub fn retime(&self) -> Option<SpeedCurve> {
        let slopes = (!self.speed_slopes.is_empty()).then_some(self.speed_slopes.as_slice());
        SpeedCurve::with_slopes(&self.speed_curve, slopes)
    }

    /// A clip with nothing set but what places it: no file, unity gain,
    /// scale and speed, no fades, chains, keys or masks. Every literal in
    /// the workspace starts here and sets the few fields it means to, so a
    /// field added to this type is added in one place.
    pub fn blank(kind: ClipKind, start: f64, duration: f64, track: usize) -> ExportClip {
        ExportClip {
            path: String::new(),
            audio_stream: None,
            color_range: None,
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
            speed_slopes: Vec::new(),
            animation: Vec::new(),
            flip_h: false,
            flip_v: false,
            blend: String::new(),
            crop: None,
            effects: Vec::new(),
            transition_shapes: Vec::new(),
            scale: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
            rotation: 0.0,
            stretch_x: 1.0,
            stretch_y: 1.0,
            opacity: 1.0,
            transition: None,
            video_fade_in: 0.0,
            hold: false,
            media_width: None,
            media_height: None,
            media_duration: None,
            has_audio: None,
            cutout: None,
            mask: None,
            mask_dir: String::new(),
            stabilize: None,
            motion_dir: String::new(),
            highlighted: false,
            reveal_map: None,
            backdrop: None,
        }
    }
}

/// One animation key, as the flattener hands it over.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExportKey {
    /// "scale", "offsetX", "offsetY", "rotation", "opacity" or "volume",
    /// or one of the mask's numbers: "maskX", "maskY", "maskWidth",
    /// "maskHeight", "maskRotation", "maskRoundness", "maskFeather".
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
    /// A packaged transition's knobs as set on the cut; the package's
    /// defaults for any left out.
    #[serde(default)]
    pub params: BTreeMap<String, f64>,
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
    /// The levels the file is written in and tagged with, by name:
    /// "limited" (16-235, what every player expects) or "full" (0-255).
    /// Limited when a request does not say.
    /// https://github.com/jub0t/Concat/issues/103
    #[serde(default, deserialize_with = "range_by_name")]
    pub color_range: concat_media::ColorRange,
    /// What the timeline is output in. SDR when a request does not say.
    #[serde(default)]
    pub color_space: ColorSpace,
    /// An HDR timeline written as HDR - HEVC or AV1 in ten bits, BT.2020
    /// with its HLG or PQ - rather than tone-mapped to SDR as the monitor
    /// shows it. Nothing for an SDR timeline. False when a request does not
    /// say.
    #[serde(default)]
    pub hdr: bool,
    /// Original timeline origin when rendering a selection moved to zero.
    /// Preserves the global video frame grid and shader clock; audio, clip
    /// clocks and keyframe fractions keep their local timing.
    #[serde(default)]
    pub effect_time_offset: f64,
    /// The flattened clip list to render.
    pub clips: Vec<ExportClip>,
}

/// The engine's word for a document's colour range: the two enums are one
/// idea kept in two crates, since the document knows nothing of FFmpeg
/// and the engine nothing of documents.
pub fn engine_range(range: concat_project::model::ColorRange) -> concat_media::ColorRange {
    match range {
        concat_project::model::ColorRange::Limited => concat_media::ColorRange::Limited,
        concat_project::model::ColorRange::Full => concat_media::ColorRange::Full,
    }
}

/// A colour range named the way [`concat_media::ColorRange::name`] names
/// it, refusing a name the engine does not know rather than quietly
/// writing video range.
fn range_by_name<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<concat_media::ColorRange, D::Error> {
    let name = String::deserialize(deserializer)?;
    concat_media::ColorRange::parse(&name).ok_or_else(|| {
        serde::de::Error::custom(format!("unknown colour range {name:?}: limited or full"))
    })
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

/// What a fade to a colour or a wipe does to one clip, in the timeline's
/// frames counted from the clip's own start: kept with the clip, and turned
/// into the frame plan's [`Transition`] at each instant ([`transitions_at`]),
/// so the monitor and the export draw it alike.
///
/// The counts reproduce the FFmpeg filters these replaced, frame for frame:
/// `fade` weighs the picture `(n - start) / frames` into a fade and the
/// colour the rest, and the wipe's edge stands at `(n + 1) / frames`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TransitionShape {
    /// The clip's last `frames` pulled towards `colour`, from `start`.
    FadeOut {
        /// The colour faded to, `0..=1` a channel.
        colour: [f32; 3],
        /// The first frame of the fade.
        start: i64,
        /// Its length.
        frames: i64,
    },
    /// The clip's first `frames` coming out of `colour`.
    FadeIn {
        /// The colour faded from.
        colour: [f32; 3],
        /// The fade's length.
        frames: i64,
    },
    /// The clip's first `frames` uncovered behind a moving edge.
    Wipe {
        /// The shown part grows from the right edge rather than the left.
        from_right: bool,
        /// The wipe's length.
        frames: i64,
    },
}

/// The frame plan's transitions for a clip at `frame`, counted in the
/// timeline's frames from the clip's start.
pub(crate) fn transitions_at(shapes: &[TransitionShape], frame: i64) -> Vec<Transition> {
    shapes
        .iter()
        .filter_map(|shape| match *shape {
            TransitionShape::FadeOut {
                colour,
                start,
                frames,
            } if frame >= start => Some(Transition::FadeTo {
                colour,
                amount: ((frame - start) as f32 / frames as f32).min(1.0),
            }),
            TransitionShape::FadeIn { colour, frames } if frame < frames => {
                Some(Transition::FadeTo {
                    colour,
                    amount: 1.0 - frame.max(0) as f32 / frames as f32,
                })
            }
            TransitionShape::Wipe { from_right, frames } if frame < frames => {
                Some(Transition::Wipe {
                    uncovered: ((frame.max(0) + 1) as f32 / frames as f32).min(1.0),
                    from_right,
                })
            }
            _ => None,
        })
        .collect()
}

/// Turns per-cut transition requests into things the renderer already knows
/// how to draw: overlapping clips, opacity ramps, placement keys, and fade
/// and mask filters. Returns the packaged transitions found along the way,
/// for the compositor to combine over; every other kind needs nothing more
/// than what it already baked into the clips themselves.
///
/// Track indices are spread apart first, so an incoming clip gets a lane of
/// its own directly above the clip it crosses from - stacking against every
/// other track is preserved, and nothing else occupies the lanes between
/// two tracks. They are doubled unless transitions follow one another, when
/// each incoming clip of the chain sits a lane above the last and the
/// tracks are spread as far as the longest chain climbs (see
/// [`lane_spacing`]). Cuts are collected before anything mutates, so
/// resolving one transition cannot unhook the adjacency test of the next,
/// and resolved in time order, so each clip of a chain has been lifted
/// before the clip after it is lifted above it.
///
/// Every kind but the fades to a colour is built on one overlap, centred on
/// the cut the way the timeline draws it: the incoming clip extends
/// backwards by half the transition's length, on the lane above, showing
/// the handle before its in-point, and the outgoing one runs on under it by
/// the other half, past its out-point (see [`overlap`]). What differs is how the two are blended across that overlap - a dissolve
/// ramps the incoming clip's opacity, a push slides both, a zoom scales
/// both under a dissolve, a wipe uncovers the incoming one behind a moving
/// edge. The slides and scales are keys on the clips' animations, which
/// the plan already plays for the monitor and the export alike; the wipe
/// and the fades to a colour are [`TransitionShape`]s, which the frame plan
/// draws at each instant from the clip's own time, the same way for both.
///
/// An id the catalogue knows as a transition package gets the same overlap
/// plus a dissolve ramp, but the returned [`TransitionSpan`] tells the
/// compositor a shader owns the blend where one can run: see
/// `combine_transition`. Legacy ids are matched first and never reach this
/// path, so a project saved before packaged transitions existed renders
/// exactly as it always did.
fn resolve_transitions(clips: &mut Vec<ExportClip>, rate: FrameRate) -> Vec<TransitionSpan> {
    resolve_transitions_at(clips, rate, 0.0)
}

fn resolve_transitions_at(
    clips: &mut Vec<ExportClip>,
    rate: FrameRate,
    origin: f64,
) -> Vec<TransitionSpan> {
    let mut spans: Vec<TransitionSpan> = Vec::new();

    let fps = rate.fps().as_f64();
    let frame = 1.0 / fps;

    // Lanes are only compared for equality here, so the cuts read the same
    // before the tracks are spread as after.
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
                params: transition.params.clone(),
            }),
            _ => {}
        }
    }
    cuts.sort_by(|one, other| {
        clips[one.incoming]
            .start
            .total_cmp(&clips[other.incoming].start)
    });
    let spacing = lane_spacing(clips.len(), &cuts);
    for clip in clips.iter_mut() {
        clip.track *= spacing;
    }

    for cut in cuts {
        match cut.kind.as_str() {
            "cross-fade" | "push" | "zoom" | "wipe-left" | "wipe-right" => {
                // The two pictures across the cut: the incoming clip on the
                // lane above, the outgoing one running on beneath it.
                let Some(Overlap {
                    incoming,
                    length: d,
                    outgoing_runs,
                }) = overlap(clips, &cut, frame)
                else {
                    continue;
                };

                // How the two blend across the overlap. A shape that cannot
                // be applied - a clip the user has already keyed on the
                // property the shape would ride, or an outgoing picture that
                // is a held frame rather than the clip a key could move -
                // falls back to the dissolve rather than half-applying, so
                // the cut still transitions.
                let shaped = match cut.kind.as_str() {
                    // The new picture slides in from the right and shoves the
                    // old one out to the left, edge to edge: both ride the
                    // same ease over the same seconds, which is what keeps
                    // them glued.
                    "push" => {
                        outgoing_runs
                            && rides(clips, cut.outgoing, incoming, "offsetX")
                            && ride(
                                &mut clips[incoming],
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
                        outgoing_runs
                            && rides(clips, cut.outgoing, incoming, "scale")
                            && ride(&mut clips[incoming], "scale", 1.25, 1.0, d, true, EASE_OUT)
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
                                clips[incoming].video_fade_in = d;
                                true
                            }
                    }
                    // A straight edge sweeps across and uncovers the new
                    // picture behind it, from the clip's new, earlier start.
                    "wipe-left" | "wipe-right" => {
                        let frames = ((d * fps).round() as i64).max(1);
                        clips[incoming]
                            .transition_shapes
                            .push(TransitionShape::Wipe {
                                // A wipe to the left uncovers the new picture
                                // from the right edge.
                                from_right: cut.kind == "wipe-left",
                                frames,
                            });
                        true
                    }
                    _ => false,
                };
                if !shaped {
                    clips[incoming].video_fade_in = d;
                }
            }
            "fade-black" | "fade-white" => {
                // Half the duration on each side of the cut, counted in the
                // timeline's frames, so the fade lands on the frames the
                // timeline arithmetic says it covers.
                let colour = if cut.kind == "fade-white" {
                    [1.0; 3]
                } else {
                    [0.0; 3]
                };
                let half = cut.duration / 2.0;
                {
                    let a = &mut clips[cut.outgoing];
                    let frames = ((half.min(a.duration) * fps).round() as i64).max(1);
                    let total = (a.duration * fps).round() as i64;
                    a.transition_shapes.push(TransitionShape::FadeOut {
                        colour,
                        start: (total - frames).max(0),
                        frames,
                    });
                }
                {
                    let b = &mut clips[cut.incoming];
                    let frames = ((half.min(b.duration) * fps).round() as i64).max(1);
                    b.transition_shapes
                        .push(TransitionShape::FadeIn { colour, frames });
                }
            }
            // A packaged transition: overlap the incoming clip onto the
            // outgoing one exactly as a dissolve does - the fallback where the
            // shader cannot run - and record a span the compositor combines
            // over with the package's two-input shader. Legacy ids are matched
            // above first, so an id aliased to a package still takes the
            // hand-written path saved projects expect.
            kind if Catalogue::builtin()
                .get(kind)
                .is_some_and(|package| package.transition().is_some()) =>
            {
                let Some(Overlap {
                    incoming,
                    length: d,
                    ..
                }) = overlap(clips, &cut, frame)
                else {
                    continue;
                };
                let b = &mut clips[incoming];
                // The dissolve a GPU-less path shows; the shader's own blend
                // overrides it where a GPU runs the transition.
                b.video_fade_in = d;
                let start = quantise_at(b.start, rate, origin);
                let end = start + quantise(d, rate);
                spans.push(TransitionSpan {
                    start,
                    end,
                    to_track: b.track,
                    id: cut.kind.clone(),
                    params: cut.params.clone(),
                });
            }
            // A kind this build does not know renders as a plain cut rather
            // than failing the export - the same degrade a missing effect
            // filter must NOT get, because there the user styled the picture.
            _ => {}
        }
    }
    spans
}

/// A cut with a transition on it: which clip comes in, which goes out, and
/// what the UI asked for across it.
struct Cut {
    incoming: usize,
    outgoing: usize,
    kind: String,
    duration: f64,
    params: BTreeMap<String, f64>,
}

/// How many lanes each track is spread across before its cuts are
/// resolved: its own, and one for each cut of the longest chain on any
/// track. A transition lifts its incoming clip a lane above the outgoing
/// one, and in a chain the outgoing clip was itself lifted for the cut
/// before it, so a chain of n cuts climbs n lanes. With fewer lanes than
/// that between two tracks, the chain climbs onto the next track's own
/// lane, and of two pictures on one lane only one is drawn
/// (`Timeline::clip_on_track_at`). Two, as it always was, where no
/// transition follows another. `cuts` are in time order, so a clip's
/// outgoing clip has its depth before the clip itself is reached.
fn lane_spacing(clips: usize, cuts: &[Cut]) -> usize {
    let mut depth = vec![0_usize; clips];
    for cut in cuts {
        depth[cut.incoming] = depth[cut.outgoing] + 1;
    }
    depth.into_iter().max().unwrap_or(0).max(1) + 1
}

/// What [`overlap`] built a transition on.
struct Overlap {
    /// The incoming clip, moved to the lane above the outgoing one and
    /// starting where the overlap does.
    incoming: usize,
    /// The overlap's length: the transition's, capped by both clips.
    length: f64,
    /// The outgoing clip itself runs on under the overlap. False when a
    /// held copy of its last frame does instead, which a key on the clip
    /// cannot move.
    outgoing_runs: bool,
}

/// The overlap a transition crosses, centred on the cut the way the
/// timeline draws it: the incoming clip on the lane above, from half the
/// transition's length before the cut, over the outgoing one running on
/// for the other half past it. The length is capped by both clips; `None`
/// for a cut too short to overlap at all.
///
/// Each side is the clip itself, extended with its source clock moved the
/// same way, into footage the edit left out: before the incoming clip's
/// in-point, after the outgoing one's out-point - a still has all it needs
/// either way. Winding a start back without the clock, as this once did,
/// played everything after the cut early and ran the last of it off the
/// file's end into black (#236).
///
/// Without footage after its out-point, the outgoing clip ends at the cut
/// and a copy of its last frame is held beneath for the rest: the
/// "repeated frames" every editor shows over a cut with no handle. The
/// hold is the clip with no sound, no keys and no fades of its own; a
/// keyed placement is held where its ride leaves it at the clip's tail.
/// The incoming side gets no hold, as one there would end at the cut and
/// leave the clip after it sharing a lane with the outgoing picture: what
/// it lacks before its in-point moves to after the cut instead, so the
/// transition leans later rather than freezing.
fn overlap(clips: &mut Vec<ExportClip>, cut: &Cut, frame: f64) -> Option<Overlap> {
    let a = &clips[cut.outgoing];
    let b = &clips[cut.incoming];
    let length = cut.duration.min(a.duration).min(b.duration);
    if length < frame {
        return None;
    }
    let mut lead = (length / 2.0).min(head_handle(b));
    if lead < frame / 2.0 {
        lead = 0.0;
    }
    let tail = length - lead;
    let a_track = a.track;
    let a_end = a.start + a.duration;
    let outgoing_runs = tail_handle(a, frame) + frame / 2.0 >= tail;

    {
        let b = &mut clips[cut.incoming];
        extend(b, lead, 0.0);
        // Sound rides the picture: each side fades across the overlap
        // rather than arriving or leaving at full level.
        b.fade_in = b.fade_in.max(length);
        b.track = a_track + 1;
    }
    if outgoing_runs {
        let a = &mut clips[cut.outgoing];
        extend(a, 0.0, tail);
        a.fade_out = a.fade_out.max(length);
    } else {
        let hold = last_frame_held(&clips[cut.outgoing], a_end, tail, frame);
        clips.push(hold);
    }
    Some(Overlap {
        incoming: cut.incoming,
        length,
        outgoing_runs,
    })
}

/// Footage before `clip`'s in-point, in timeline seconds. A clip whose
/// speed rides a curve has none to offer: its source clock is the curve's,
/// which says nothing about before its start.
fn head_handle(clip: &ExportClip) -> f64 {
    if clip.kind == ClipKind::Image {
        f64::INFINITY
    } else if !clip.speed_curve.is_empty() {
        0.0
    } else {
        clip.source_start / clip.speed.max(f64::EPSILON)
    }
}

/// Footage after `clip`'s out-point, in timeline seconds, a frame short of
/// the length the file declares so a container that rounds up is not read
/// past its last picture. Nothing where the length is unknown, or where a
/// curve sets the clock.
fn tail_handle(clip: &ExportClip, frame: f64) -> f64 {
    if clip.kind == ClipKind::Image {
        return f64::INFINITY;
    }
    if !clip.speed_curve.is_empty() {
        return 0.0;
    }
    clip.media_duration.map_or(0.0, |length| {
        let speed = clip.speed.max(f64::EPSILON);
        ((length - clip.source_start) / speed - clip.duration - frame).max(0.0)
    })
}

/// Lengthens `clip` by `head` seconds before its start and `tail` after
/// its end, its source clock moved with it. Keys sit at fractions of the
/// clip, so each is moved to stay at the instant it was at.
fn extend(clip: &mut ExportClip, head: f64, tail: f64) {
    if head <= 0.0 && tail <= 0.0 {
        return;
    }
    let old = clip.duration;
    let new = old + head + tail;
    if old > 0.0 {
        for key in &mut clip.animation {
            key.at = (key.at * old + head) / new;
        }
    }
    clip.start -= head;
    clip.duration = new;
    if clip.kind != ClipKind::Image {
        clip.source_start = (clip.source_start - head * clip.speed).max(0.0);
    }
}

/// A copy of `clip`'s last frame held from `start` for `length` seconds,
/// on its lane: silent, unkeyed and unfaded, placed where its keys leave
/// it at its tail.
fn last_frame_held(clip: &ExportClip, start: f64, length: f64, frame: f64) -> ExportClip {
    let mut hold = clip.clone();
    hold.hold = true;
    hold.start = start;
    hold.duration = length;
    if clip.kind != ClipKind::Image {
        let (played, rate) = match SpeedCurve::new(&clip.speed_curve) {
            Some(curve) => (clip.duration * curve.consumed(1.0), curve.speed_at(1.0)),
            None => (clip.duration * clip.speed, clip.speed),
        };
        hold.source_start = clip.source_start + (played - frame * rate).max(0.0);
    }
    hold.speed = 1.0;
    hold.speed_curve.clear();
    hold.speed_slopes.clear();
    hold.muted = true;
    hold.has_audio = Some(false);
    hold.volume = 0.0;
    hold.fade_in = 0.0;
    hold.fade_out = 0.0;
    hold.transition = None;
    hold.video_fade_in = 0.0;
    hold.transition_shapes.clear();
    // Keys are absolute and a keyed property's constant is the neutral
    // value, so the tail of each ride becomes the hold's constant.
    let tails: Vec<(String, f64)> = ["scale", "offsetX", "offsetY", "rotation", "opacity"]
        .into_iter()
        .filter_map(|property| {
            hold.animation
                .iter()
                .filter(|key| key.property == property)
                .max_by(|a, b| a.at.total_cmp(&b.at))
                .map(|key| (property.to_owned(), key.value))
        })
        .collect();
    hold.animation.clear();
    for (property, value) in tails {
        match property.as_str() {
            "scale" => hold.scale = value,
            "offsetX" => hold.offset_x = value,
            "offsetY" => hold.offset_y = value,
            "rotation" => hold.rotation = value,
            "opacity" => hold.opacity = value,
            _ => {}
        }
    }
    hold
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

/// Renders `request` and returns the path written, drawing on a device of
/// its own; [`render_on`] draws on one the caller lends.
pub fn render(request: &ExportRequest, reporter: Reporter<'_>) -> Result<String, String> {
    render_on(request, None, reporter)
}

/// Renders `request` and returns the path written, drawing on `compositor`
/// where the caller has one to lend - the window's, a sibling of the
/// monitor's on the device the window draws with - and on a device of its
/// own otherwise (see [`WgpuCompositor::new`]).
///
/// The window's device has drawn the monitor since the project opened:
/// whatever the machine and its drivers make of the compositor, they have
/// made of it already, frame after frame. A device opened for the export
/// alone is a second road through the drivers, on a thread of its own, and
/// on Windows laptops with NVIDIA chips it was a road that ended in the
/// driver at the first frame, taking the app with it and leaving nothing
/// in the log (issue #202).
pub fn render_on(
    request: &ExportRequest,
    compositor: Option<WgpuCompositor>,
    mut reporter: Reporter<'_>,
) -> Result<String, String> {
    if !request.effect_time_offset.is_finite()
        || request.effect_time_offset.abs() > concat_project::model::ranges::MAX_TIME
    {
        return Err("the effect timeline offset is not a valid time".into());
    }
    if request.clips.is_empty() {
        return Err("there is nothing on the timeline to export".to_owned());
    }

    let rate = FrameRate::checked(request.rate_num, request.rate_den).ok_or_else(|| {
        format!(
            "{}/{} is not a frame rate a video can have",
            request.rate_num, request.rate_den
        )
    })?;
    // A frame past what one texture can hold would panic the GPU device at
    // its first allocation, on this thread; the document and the API hold
    // their callers to the same ceiling, and the command line reaches here
    // with nothing in between.
    let side = 1..=concat_core::frame::MAX_SIDE;
    if !side.contains(&request.width) || !side.contains(&request.height) {
        return Err(format!(
            "{}×{} is not a frame size a video can have; the most is {} a side",
            request.width,
            request.height,
            concat_core::frame::MAX_SIDE
        ));
    }
    let output = PathBuf::from(&request.output);

    // Transitions become overlaps, ramps and fade filters before anything
    // else reads the clip list, so the picture and sound paths below never
    // know transitions exist.
    let mut resolved = request.clips.clone();
    let transitions = resolve_transitions_at(&mut resolved, rate, request.effect_time_offset);

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

    let (timeline_end, total_frames) = timeline_extent(&resolved, rate, request.effect_time_offset);
    if total_frames <= 0 {
        return Err("the timeline is empty".to_owned());
    }

    // Render into siblings of the output so the move at the end stays on one
    // filesystem, then clean up whatever we made.
    let stem = output
        .file_stem()
        .map_or_else(|| "concat".into(), |s| s.to_string_lossy());
    let directory = output.parent().unwrap_or(Path::new("."));
    // The folder the file goes in is made when it is not there. A phone's
    // Movies folder under the app's own files does not exist until the
    // first export, and every phone's first export stopped at "no such
    // file or directory" before a frame was drawn (#280, #147).
    if let Err(error) = std::fs::create_dir_all(directory) {
        return Err(format!(
            "could not create the folder {}: {error}",
            directory.display()
        ));
    }
    let silent = directory.join(format!(".{stem}.concat-video.mp4"));
    let mixed = directory.join(format!(".{stem}.concat-audio.m4a"));
    let muxed = directory.join(format!(".{stem}.concat-muxed.mp4"));

    // The output path is touched once, by the rename at the end, whichever
    // way the file was made. The mux used to write it directly, and a mux
    // that failed - the disk full, a read error - left a truncated file
    // where the previous export had been (audit 2026-10-09, finding 4).
    let result = (|| -> Result<(), String> {
        render_picture(
            request,
            compositor,
            rate,
            total_frames,
            &visible,
            transitions,
            &silent,
            &mut reporter,
        )?;

        let finished = if sound.is_empty() {
            &silent
        } else {
            reporter.cancelled()?;
            reporter.emit(0, total_frames, "mixing audio");
            let mix: Vec<AudioClip> = sound.iter().flat_map(|clip| audio_pieces(clip)).collect();
            let buses = audio_buses(&resolved);
            audio::mix_to_file_with(&mix, &buses, timeline_end.as_f64(), &mixed)
                .map_err(|error| error.to_string())?;

            reporter.cancelled()?;
            reporter.emit(total_frames, total_frames, "muxing");
            audio::mux(&silent, &mixed, &muxed).map_err(|error| error.to_string())?;
            &muxed
        };
        std::fs::rename(finished, &output)
            .map_err(|error| format!("could not write {}: {error}", output.display()))
    })();

    let _ = std::fs::remove_file(&silent);
    let _ = std::fs::remove_file(&mixed);
    let _ = std::fs::remove_file(&muxed);

    result.map(|()| output.to_string_lossy().into_owned())
}

/// The timeline's end and how many frames it runs for, on the frame grid the
/// engine quantises every clip onto: a clip's start and its duration are each
/// rounded to that grid separately on the way into the timeline, so the
/// timeline the frames are drawn from can run a frame past - or stop a frame
/// short of - what the f64 seconds add up to, whose own whole rounding is a
/// third answer again. A clip starting half a frame in and running a further
/// half-frame count lost the last frame of the last clip that way; a pair
/// that rounds down at both ends gained a black one after it. The mix takes
/// the same end, so the muxer joins two tracks of one length.
fn timeline_extent(clips: &[ExportClip], rate: FrameRate, origin: f64) -> (Rational, i64) {
    let end = clips
        .iter()
        .map(|clip| quantise_at(clip.start, rate, origin) + quantise(clip.duration, rate))
        .max()
        .unwrap_or(Rational::ZERO);
    (end, rate.frames_in(end))
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

/// The sound layers among `clips`, as the mixer runs them over the sum:
/// those on a lane that is not muted with a chain that does something, in
/// the order of their lanes from the bottom up and then of their starts,
/// so two that overlap are heard as the lanes stack them. Playback and
/// the export both read them from here.
pub fn audio_buses(clips: &[ExportClip]) -> Vec<audio::AudioBus> {
    let mut layers: Vec<&ExportClip> = clips
        .iter()
        .filter(|clip| clip.kind == ClipKind::SoundLayer && !clip.muted)
        .collect();
    layers.sort_by(|a, b| a.track.cmp(&b.track).then(a.start.total_cmp(&b.start)));
    layers
        .into_iter()
        .map(|clip| audio::AudioBus {
            start: clip.start,
            duration: clip.duration,
            filter_chain: clip.filter_chain.clone(),
            strength: clip.opacity.clamp(0.0, 1.0),
            fade_in: clip.fade_in,
            fade_out: clip.fade_out,
        })
        .filter(audio::AudioBus::audible)
        .collect()
}

/// The engine's view of one audible clip - or several, when its speed
/// changes over it. Sound can only change tempo in steps, so a curve is cut
/// into pieces of constant rate, each at the mean of its stretch of the
/// curve and starting where the curve says the source had got to.
pub fn audio_pieces(clip: &ExportClip) -> Vec<AudioClip> {
    let chain = clip.filter_chain.clone();
    let track = volume_track(clip);
    let Some(curve) = clip.retime() else {
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
    curve
        .pieces(count)
        .into_iter()
        .map(|(x0, x1, consumed, mean)| {
            let piece_duration = (x1 - x0) * clip.duration;
            let forward = consumed * clip.duration;
            let source_start = clip.source_start + forward;
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

/// The plan's layer for one clip at one instant, filled in: the picture
/// decoded, its effects resolved, on its track, and its crop and flips
/// where the plan draws them (`geometry`; see `resolve::planned_geometry`),
/// and its stabilization's correction at the frame it shows.
/// A clip without planned geometry has them baked in by the decoder's
/// chain, as the transition fades still are, and the cutout has already
/// cut the picture either way.
fn filled(
    layer: &PlannedLayer,
    frame: std::sync::Arc<Frame>,
    track: usize,
    effects: Vec<ShaderPass>,
    geometry: Option<&resolve::PlannedGeometry>,
    transitions: Vec<Transition>,
    stabilized: Option<&resolve::StabilizeJob>,
) -> PlannedLayer {
    let mut filled = PlannedLayer {
        source: Some(frame),
        track,
        effects,
        transitions,
        ..layer.clone()
    };
    if let Some(job) = stabilized {
        filled.warp = job.warp_at(layer.source_time);
    }
    if let Some(geometry) = geometry {
        filled.crop = geometry.crop;
        filled.flip_h = geometry.flip_h;
        filled.flip_v = geometry.flip_v;
    }
    filled
}

/// The plan's transitions for `layer` at `time`, from its clip's shapes.
fn shapes_at(
    shapes: &std::collections::HashMap<concat_core::timeline::ClipId, Vec<TransitionShape>>,
    layer: &PlannedLayer,
    time: Rational,
    rate: FrameRate,
) -> Vec<Transition> {
    match shapes.get(&layer.clip) {
        Some(shapes) => {
            let frame = ((time - layer.clip_start).as_f64() * rate.fps().as_f64()).round() as i64;
            transitions_at(shapes, frame)
        }
        None => Vec::new(),
    }
}

/// `layers` with every picture's background beneath it, in a `width` by
/// `height` frame: the one place the export and the monitor both put
/// backgrounds in, so the two cannot draw them two ways. A picture that
/// already hides the whole frame gets none - `opaque` says whose pixels
/// have no holes to see one through - and a still that cannot be read is
/// no background rather than a failed frame.
fn with_backdrops(
    layers: Vec<PlannedLayer>,
    backdrops: &HashMap<ClipId, ExportBackdrop>,
    opaque: impl Fn(ClipId) -> bool,
    pool: &concat_media::ReaderPool,
    (width, height): (u32, u32),
) -> Vec<PlannedLayer> {
    if backdrops.is_empty() {
        return layers;
    }
    let mut out = Vec::with_capacity(layers.len() + 1);
    for layer in layers {
        let drawn = backdrops
            .get(&layer.clip)
            .filter(|_| !layer.covers(width, height, opaque(layer.clip)))
            .and_then(|backdrop| backdrop_of(backdrop, pool, (width, height)))
            .and_then(|backdrop| layer.backdrop(&backdrop, width, height, blur_pass));
        out.extend(drawn);
        out.push(layer);
    }
    out
}

/// A clip's background as the frame plan draws it, at a `width` by
/// `height` frame: the blur's spread in that frame's pixels, a still read
/// through the pool at a size that covers it.
fn backdrop_of(
    backdrop: &ExportBackdrop,
    pool: &concat_media::ReaderPool,
    (width, height): (u32, u32),
) -> Option<concat_render::Backdrop> {
    match backdrop {
        ExportBackdrop::Blur { amount } => Some(concat_render::Backdrop::Blur {
            sigma: ExportBackdrop::sigma(*amount, width, height),
        }),
        ExportBackdrop::Colour { rgb } => Some(concat_render::Backdrop::Colour(*rgb)),
        ExportBackdrop::Image {
            path,
            width: still_width,
            height: still_height,
        } => {
            // Read at the size that covers the frame, its own shape kept;
            // the frame's when the bin never learnt the still's.
            let size = match (still_width, still_height) {
                (Some(w), Some(h)) if *w > 0 && *h > 0 => {
                    let cover =
                        (f64::from(width) / f64::from(*w)).max(f64::from(height) / f64::from(*h));
                    (
                        ((f64::from(*w) * cover).round() as u32).max(2),
                        ((f64::from(*h) * cover).round() as u32).max(2),
                    )
                }
                _ => (width, height),
            };
            let request = concat_media::FrameRequest::new(path, Rational::ZERO, size.0, size.1)
                .as_still(true)
                .at_any_size(true);
            match pool.frame(&request) {
                Ok(still) => Some(concat_render::Backdrop::Image(still)),
                Err(error) => {
                    log::debug!("background image {path}: {error}");
                    None
                }
            }
        }
    }
}

/// The Gaussian blur's pass at `radius` pixels: the package every blurred
/// background is drawn with, so it blurs exactly as the effect does.
fn blur_pass(radius: f32) -> Option<ShaderPass> {
    Catalogue::builtin().get("concat.gaussian-blur")?.pass(
        &BTreeMap::from([("radius".to_owned(), f64::from(radius))]),
        None,
    )
}

/// A stack already drawn, as a layer over the whole frame on the lowest
/// track: texel for pixel, since it is the frame's own size.
fn ground_layer(ground: Frame) -> PlannedLayer {
    PlannedLayer::picture(concat_render::detached_clip(), std::sync::Arc::new(ground))
}

/// The compositor an export draws with when its caller lends none: a
/// device of its own on the machine's GPU, or on its software adapter where
/// it has none (see [`WgpuCompositor::new`]). The one error is a machine
/// with neither, which is said in words a person can act on.
fn best_compositor() -> Result<WgpuCompositor, String> {
    WgpuCompositor::new().ok_or_else(|| NO_RENDERER.to_owned())
}

/// What an export or a preview says on a machine that offers no GPU and no
/// software renderer either: on Linux, where the software renderer is a
/// package away, which package.
#[cfg(target_os = "linux")]
const NO_RENDERER: &str = "no GPU or software renderer is available to draw with - install \
     Mesa's Vulkan drivers (lavapipe)";

/// On a phone there is nothing to install: the one GPU it has would not
/// open, and the log says which and why (#312).
#[cfg(any(target_os = "android", target_os = "ios"))]
const NO_RENDERER: &str = "this device's GPU could not be used to draw with";

/// Elsewhere the platform's own software renderer is always there, so
/// this is a machine whose GPU and fallback both refused.
#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "ios")))]
const NO_RENDERER: &str = "no GPU or software renderer is available to draw with";

/// How many times an export's device may die under it before the export
/// gives up: the first death is answered with a fresh device on the GPU,
/// which a reset adapter opens again; the next with the software adapter,
/// which draws the same picture however slowly; a third says the machine
/// cannot draw this export at all.
const DEVICE_LOSSES: u32 = 3;

/// A compositor to go on with after the export's device died, `losses`
/// deaths in; see [`DEVICE_LOSSES`]. It draws as the dead one did -
/// delivering HDR or not - and the log says what it is drawing on.
///
/// A device can die under an export: Windows resets an adapter whose
/// command takes too long, and a card shared with the window, the hardware
/// decoder and the encoder can run out of memory. Both ended the export
/// with "the GPU device was lost" on 8 GB NVIDIA cards (#223); the frames
/// already encoded are kept and the next is drawn on the new device. A
/// device that stops answering for [`concat_render::gpu::DEVICE_WAIT`], or
/// reports an error under a frame, counts as lost too (#297).
fn recovered(losses: u32, hdr: bool) -> Result<WgpuCompositor, String> {
    if losses >= DEVICE_LOSSES {
        return Err(format!(
            "the GPU device was lost {losses} times part way through the export - reset, \
             out of memory, or not answering for {} seconds; nothing on this machine can \
             draw it to the end",
            concat_render::gpu::DEVICE_WAIT.as_secs()
        ));
    }
    let fresh = if losses == 1 {
        WgpuCompositor::new()
    } else {
        WgpuCompositor::software()
    };
    let mut compositor = fresh.ok_or_else(|| {
        format!("the GPU device was lost part way through the export, and {NO_RENDERER}")
    })?;
    compositor.deliver_hdr(hdr);
    let adapter = compositor.adapter_info();
    log::warn!(
        "export: the GPU device was lost; going on with {} ({:?}, {:?})",
        adapter.name,
        adapter.device_type,
        adapter.backend
    );
    Ok(compositor)
}

/// A compositor for the frames drawn without a window: the API's and the
/// command line's previews. Made once and kept, since making a device is
/// far slower than drawing one small frame on it.
fn headless<T>(draw: impl FnOnce(&mut WgpuCompositor) -> T) -> Result<T, String> {
    static HEADLESS: std::sync::OnceLock<Option<std::sync::Mutex<WgpuCompositor>>> =
        std::sync::OnceLock::new();
    // A machine with nothing to draw on stays so for the process; why is
    // in the log above this line, adapter by adapter.
    let slot = HEADLESS
        .get_or_init(|| {
            let made = WgpuCompositor::new();
            match &made {
                Some(gpu) => {
                    let adapter = gpu.adapter_info();
                    log::info!(
                        "previews draw on {} ({:?}, {:?})",
                        adapter.name,
                        adapter.device_type,
                        adapter.backend
                    );
                }
                None => log::error!("previews have nothing to draw on: {NO_RENDERER}"),
            }
            made.map(std::sync::Mutex::new)
        })
        .as_ref()
        .ok_or_else(|| NO_RENDERER.to_owned())?;
    let mut gpu = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // A device that died under an earlier preview is not kept: every
    // preview after it would be black.
    if gpu.lost()
        && let Some(fresh) = WgpuCompositor::new()
    {
        log::warn!("previews: the GPU device was lost; drawing on a fresh one");
        *gpu = fresh;
    }
    Ok(draw(&mut gpu))
}

/// Composites every frame of the timeline into a soundless video file.
fn render_picture(
    request: &ExportRequest,
    compositor: Option<WgpuCompositor>,
    rate: FrameRate,
    total_frames: i64,
    visible: &[&ExportClip],
    transitions: Vec<TransitionSpan>,
    destination: &Path,
    reporter: &mut Reporter<'_>,
) -> Result<(), String> {
    let effect_offset = quantise(request.effect_time_offset, rate);
    let mut compositor = match compositor {
        Some(lent) => lent,
        None => best_compositor()?,
    };
    // Which chip and API the file is drawn on: the first thing a report of
    // an export gone wrong needs to say, and until now could not (#202).
    let adapter = compositor.adapter_info();
    log::info!(
        "export: drawing on {} ({:?}, {:?})",
        adapter.name,
        adapter.device_type,
        adapter.backend
    );
    // The request was held to the document's ceiling; a phone's GPU can
    // hold less, and a frame past it is said here rather than failed on.
    let most = compositor.max_side();
    if request.width > most || request.height > most {
        return Err(format!(
            "{}×{} is more than {} can draw; the most it takes is {most} a side",
            request.width, request.height, adapter.name
        ));
    }
    let BuiltTimeline {
        timeline,
        stills,
        decode_sizes,
        tracks,
        treatments,
        transitions,
        geometry,
        shapes,
        ranges,
        chains,
        reveal_maps,
        cutouts,
        stabilized,
        highlight: _,
        backdrops,
    } = build_timeline(request, rate, visible, transitions);

    let mut options = EncodeOptions {
        crf: request.crf,
        preset: request.preset.clone(),
        codec: request.codec,
        rate_mode: request.rate_mode,
        bitrate_kbps: request.bitrate_kbps,
        ten_bit: request.ten_bit,
        color_range: request.color_range,
        hardware: true,
        threads: 0,
        software_only: false,
    };
    // An HDR file comes out of the compositor as its signal, sixteen bits a
    // channel; anything else, SDR - an HDR timeline's tone-mapped.
    let signal = output_of(request.color_space);
    let hdr = request.hdr && signal != Signal::Sdr;
    compositor.deliver_hdr(hdr);

    // How many devices have died under this export so far; see `recovered`.
    let mut losses = 0;
    // A clip whose speed changes, or runs backwards, cannot be followed by a
    // paced decoder: each of its frames is sought by its own source time,
    // through a pool that keeps the reader rolling where it can.
    let sought = concat_media::ReaderPool::with_defaults();

    // Each pass renders the whole cut from its first frame. There is a
    // second only when the first one's hardware encoder stopped taking
    // frames part way through (#491): what it took is in a file it will
    // never finish, so the file is started again on the CPU's encoder -
    // slower, but it takes every frame - and the progress says so.
    'pass: loop {
        let stage = if options.software_only {
            RENDERING_IN_SOFTWARE
        } else {
            "rendering"
        };
        let mut encoder = if hdr {
            Encoder::create_hdr(
                destination,
                request.width,
                request.height,
                rate,
                &options,
                signal,
            )
        } else {
            Encoder::create(destination, request.width, request.height, rate, &options)
        }
        .map_err(|error| failed_encoding(&error, &options))?;
        log::info!(
            "export: encoding with {} ({})",
            encoder.encoder_name(),
            if encoder.on_hardware() {
                "hardware"
            } else {
                "software"
            }
        );

        // When progress was last said; see `progress_due`.
        let mut said = std::time::Instant::now();

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

            let mut layers: Vec<PlannedLayer> = Vec::with_capacity(plan.layers.len());
            for layer in &plan.layers {
                if !layer.paced {
                    let (decode_width, decode_height) = decode_sizes
                        .get(&layer.clip)
                        .copied()
                        .unwrap_or((request.width, request.height));
                    // A frame that cannot be read ends the export with the
                    // reason, as it does on the paced path below. A time past
                    // the media's end is not that: the pool answers it with the
                    // last frame. This path used to drop a failed frame on the
                    // floor, so a reversed or retimed clip whose file had gone
                    // bad exported as a missing layer and nobody was told.
                    let frame = sought
                        .frame_at(
                            &layer.media,
                            layer.source_time,
                            decode_width,
                            decode_height,
                            stills.contains(&layer.clip),
                            None,
                            None,
                            ranges.get(&layer.clip).copied(),
                            // Deep where the frame goes to the GPU as it is: a
                            // cutout cuts eight bits.
                            !cutouts.contains_key(&layer.clip),
                        )
                        .map_err(|error| error.to_string())?;
                    let frame = cutouts
                        .get(&layer.clip)
                        .and_then(|job| job.cut(&frame, layer.source_time))
                        .map_or(frame, std::sync::Arc::new);
                    layers.push(filled(
                        layer,
                        frame,
                        tracks.get(&layer.clip).copied().unwrap_or(0),
                        passes_at(&chains, &reveal_maps, &timeline, layer.clip, time),
                        geometry.get(&layer.clip),
                        shapes_at(&shapes, layer, time, rate),
                        stabilized.get(&layer.clip),
                    ));
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

                        // Deep - an HDR clip in its own signal, converted on
                        // the GPU - unless a cutout has to cut it in eight bits;
                        // the decoder keeps eight bits for a chain itself.
                        let mut options = DecodeOptions::default()
                            .starting_at(layer.source_time)
                            .scaled_to(decode_width, decode_height)
                            .at_rate(decode_rate)
                            .in_range(ranges.get(&layer.clip).copied())
                            .deep(!cutouts.contains_key(&layer.clip));

                        // A still is a one-frame stream. Without looping it would
                        // contribute a single frame and then disappear.
                        if stills.contains(&layer.clip) {
                            options = options.repeating().starting_at(Rational::ZERO);
                        }

                        entry.insert(
                            Decoder::open(&layer.media, &options)
                                .map_err(|error| error.to_string())?,
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
                    layers.push(filled(
                        layer,
                        std::sync::Arc::new(frame),
                        tracks.get(&layer.clip).copied().unwrap_or(0),
                        passes_at(&chains, &reveal_maps, &timeline, layer.clip, time),
                        geometry.get(&layer.clip),
                        shapes_at(&shapes, layer, time, rate),
                        stabilized.get(&layer.clip),
                    ));
                }
            }

            let frame_plan = FramePlan {
                time,
                width: request.width,
                height: request.height,
                layers: with_backdrops(
                    layers,
                    &backdrops,
                    |clip| !stills.contains(&clip) && !cutouts.contains_key(&clip),
                    &sought,
                    (request.width, request.height),
                ),
                treatments: Vec::new(),
                output: output_of(request.color_space),
            };
            // The plan is kept for a second drawing: a frame drawn on a device
            // that died under it is black, and is drawn again on the device
            // that takes over. Both drawings borrow it - the layers' pictures
            // were never copied, and now their passes are not either.
            let mut composed = composite_treated(
                &mut EffectClock {
                    inner: &mut compositor,
                    offset: effect_offset,
                },
                &frame_plan,
                &treatments,
                &transitions,
            );
            while compositor.lost() {
                losses += 1;
                let adapter = compositor.adapter_info();
                log::error!(
                    "export: the GPU device on {} was lost drawing frame {index} of {total_frames} \
                 ({:.3}s into the cut); the frame is drawn again on the next device",
                    adapter.name,
                    time.as_f64(),
                );
                compositor = recovered(losses, hdr)?;
                composed = composite_treated(
                    &mut EffectClock {
                        inner: &mut compositor,
                        offset: effect_offset,
                    },
                    &frame_plan,
                    &treatments,
                    &transitions,
                );
            }
            if let Err(error) = encoder.write_frame(&composed) {
                // The stopped encoder is closed before the file is opened
                // again, so the chip has its session back.
                drop(encoder);
                again_in_software(&error, &mut options, index, total_frames)?;
                reporter.emit(0, total_frames, RENDERING_IN_SOFTWARE);
                continue 'pass;
            }

            // Retire decoders whose clip has finished, so a long timeline does not
            // hold an ffmpeg process open for every clip it has ever passed.
            let live: Vec<ClipId> = plan.layers.iter().map(|layer| layer.clip).collect();
            decoders.retain(|clip, _| live.contains(clip));

            if progress_due(index, said.elapsed()) {
                reporter.emit(index, total_frames, stage);
                said = std::time::Instant::now();
            }
        }

        match encoder.finish() {
            Ok(()) => return Ok(()),
            Err(error) => {
                drop(encoder);
                again_in_software(&error, &mut options, total_frames, total_frames)?;
                reporter.emit(0, total_frames, RENDERING_IN_SOFTWARE);
            }
        }
    }
}

/// The stage an export reports while it renders again on the CPU's
/// encoder, its hardware one having stopped taking frames part way
/// through: from the first frame, once, the count back at nothing.
pub const RENDERING_IN_SOFTWARE: &str = "rendering in software";

/// Whether an export goes again after its encoder failed with `error`,
/// `frame` frames of `total` in, having opened it with `options`: once,
/// and only when it was the platform's hardware encoder that stopped
/// taking frames (#491) - a phone's chip that took the export's settings
/// and then went quiet. `options` is then set to the CPU's encoder alone,
/// and the export starts again from its first frame. Anything else ends
/// the export with its reason.
fn again_in_software(
    error: &concat_media::Error,
    options: &mut EncodeOptions,
    frame: i64,
    total: i64,
) -> Result<(), String> {
    if options.software_only || !error.stalled_on_hardware() {
        return Err(failed_encoding(error, options));
    }
    log::warn!(
        "export: {error}, {frame} frames of {total} in; starting again on the software encoder"
    );
    options.software_only = true;
    Ok(())
}

/// What an export says when its encoder failed with `error`: the reason,
/// and on the CPU's encoder, that this was the second try.
fn failed_encoding(error: &concat_media::Error, options: &EncodeOptions) -> String {
    if options.software_only {
        format!(
            "the hardware encoder stopped taking frames part way through, and the slower \
             software encoder the export went on with failed too: {error}"
        )
    } else {
        error.to_string()
    }
}

/// How many frames apart an export says how far it has got, at most.
const PROGRESS_FRAMES: i64 = 15;

/// And how long it goes without saying, at most, frame time allowing: a
/// frame that takes seconds - a heavy stack, a software adapter, a card
/// near its memory - shows as a bar that moves slowly rather than one that
/// sat still for fifteen of them and looked hung (#297).
const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Whether the frame at `index` is one to report, `since` the last report.
fn progress_due(index: i64, since: std::time::Duration) -> bool {
    index % PROGRESS_FRAMES == 0 || since >= PROGRESS_INTERVAL
}

/// The shader passes for one clip at one instant: every keyed knob at its
/// value there, the rest at their settings, laid out as the uniforms the
/// shaders read. `reveal_maps` carries a title's baked per-word order
/// alongside its chain - unlike an effect's knobs, it is fixed for the
/// clip's whole life, so it is looked up rather than resolved.
fn passes_at(
    chains: &HashMap<ClipId, Vec<AppliedFilter>>,
    reveal_maps: &HashMap<ClipId, Arc<RevealMap>>,
    timeline: &Timeline,
    clip: ClipId,
    time: Rational,
) -> Vec<ShaderPass> {
    let Some(effects) = chains.get(&clip) else {
        return Vec::new();
    };
    let at = timeline
        .clip(clip)
        .map_or(0.0, |engine_clip| engine_clip.fraction_at(time));
    Catalogue::builtin().shader_passes_at(effects, at, reveal_maps.get(&clip).cloned())
}

/// Only the shader clock moves. Planning, source decoding, animated effect
/// parameters and transition progress use the selection's local timeline.
struct EffectClock<'a> {
    inner: &'a mut dyn Compositor,
    offset: Rational,
}

impl Compositor for EffectClock<'_> {
    fn render(&mut self, plan: &FramePlan) -> Frame {
        if self.offset == Rational::ZERO {
            return self.inner.render(plan);
        }
        let mut shifted = plan.clone();
        shifted.time += self.offset;
        for layer in &mut shifted.layers {
            layer.clip_start += self.offset;
        }
        self.inner.render(&shifted)
    }

    fn combine(
        &mut self,
        width: u32,
        height: u32,
        time: f32,
        from: &Frame,
        to: &Frame,
        pass: &concat_core::shader::TransitionPass,
    ) -> Option<Frame> {
        self.inner.combine(
            width,
            height,
            time + self.offset.as_f64() as f32,
            from,
            to,
            pass,
        )
    }
}

/// Draws `plan` with every treatment live at its instant applied to the
/// stack beneath its track: each goes into the plan, and the compositor
/// applies it where it draws. The plan is borrowed: a frame carries its
/// treatments in a copy of its own only when one is live, which most frames
/// have none of - the export and the monitor both draw every frame through
/// here, and the copy is a whole plan's worth of layers and passes.
fn composite_treated(
    compositor: &mut dyn Compositor,
    plan: &FramePlan,
    treatments: &[Treatment],
    transitions: &[TransitionSpan],
) -> Frame {
    let time = plan.time;
    let mut live: Vec<&Treatment> = treatments
        .iter()
        .filter(|treatment| treatment.covers(time))
        .collect();
    live.sort_by_key(|treatment| treatment.track);
    let planned: Vec<PlannedTreatment> = live
        .iter()
        .map(|treatment| PlannedTreatment {
            track: treatment.track,
            effects: treatment.passes_at(time),
            strength: treatment.strength_at(time),
        })
        .collect();
    // A packaged transition live at this instant combines the outgoing stack
    // with the incoming picture through its shader, and an adjustment layer
    // live over the same frame grades it where it sits: one above the
    // incoming lane grades the combined picture, one below grades the
    // stacks being combined. Leaving the pairing to the dissolve turned
    // every transition under a filter layer into a plain fade.
    if let Some(span) = transitions.iter().find(|span| span.covers(time))
        && let Some(frame) = combine_transition(compositor, plan, span, &planned)
    {
        return frame;
    }
    if planned.is_empty() && plan.treatments.is_empty() {
        return compositor.render(plan);
    }
    let mut plan = plan.clone();
    plan.treatments = planned;
    compositor.render(&plan)
}

/// The two pictures a packaged transition combines: the outgoing stack,
/// strictly below the incoming lane, and the incoming picture over that
/// stack, its own layer at full opacity so the shader - not the dissolve
/// ramp - owns the blend.
fn transition_stages(plan: &FramePlan, span: &TransitionSpan) -> (FramePlan, FramePlan) {
    let stage = |layers: Vec<PlannedLayer>| FramePlan {
        time: plan.time,
        width: plan.width,
        height: plan.height,
        layers,
        treatments: Vec::new(),
        output: plan.output,
    };
    let from = plan
        .layers
        .iter()
        .filter(|layer| layer.track < span.to_track)
        .cloned()
        .collect();
    let to = plan
        .layers
        .iter()
        .filter(|layer| layer.track <= span.to_track)
        .cloned()
        .map(|mut layer| {
            if layer.track == span.to_track {
                layer.opacity = 1.0;
            }
            layer
        })
        .collect();
    (stage(from), stage(to))
}

/// The frame when a packaged transition is live: the outgoing stack
/// (everything below the incoming lane) combined with the incoming picture
/// (drawn over that stack at full opacity) through the transition's shader.
/// `None` when the compositor cannot run the shader or the package is unknown,
/// and the caller then shows the dissolve the incoming clip already carries.
fn combine_transition(
    compositor: &mut dyn Compositor,
    plan: &FramePlan,
    span: &TransitionSpan,
    treatments: &[PlannedTreatment],
) -> Option<Frame> {
    let pass =
        Catalogue::builtin().transition_pass(&span.id, &span.params, span.progress(plan.time))?;
    let (width, height, time, output) = (plan.width, plan.height, plan.time, plan.output);
    let stage = |layers: Vec<PlannedLayer>, treatments: Vec<PlannedTreatment>| FramePlan {
        time,
        width,
        height,
        layers,
        treatments,
        output,
    };
    // A layer's track is never the incoming clip's: lanes are spread when
    // the cuts are resolved, so each side of the incoming lane is clear.
    let (below, over): (Vec<PlannedTreatment>, Vec<PlannedTreatment>) = treatments
        .iter()
        .cloned()
        .partition(|treatment| treatment.track < span.to_track);
    let (mut from_plan, mut to_plan) = transition_stages(plan, span);
    from_plan.treatments = below.clone();
    to_plan.treatments = below;
    let from = compositor.render(&from_plan);
    let to = compositor.render(&to_plan);
    let combined = compositor.combine(width, height, time.as_f64() as f32, &from, &to, &pass)?;
    // Anything above the incoming lane draws over the combined picture, and
    // a layer above it grades the lot.
    let above: Vec<PlannedLayer> = plan
        .layers
        .iter()
        .filter(|layer| layer.track > span.to_track)
        .cloned()
        .collect();
    if above.is_empty() && over.is_empty() {
        Some(combined)
    } else {
        let mut layers = vec![ground_layer(combined)];
        layers.extend(above);
        Some(compositor.render(&stage(layers, over)))
    }
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
    /// What the timeline is output in; SDR when a request does not say.
    #[serde(default)]
    pub color_space: ColorSpace,
}

/// The signal a timeline of `space` is drawn for (`FramePlan::output`).
pub fn output_of(space: ColorSpace) -> Signal {
    match space {
        ColorSpace::Sdr => Signal::Sdr,
        ColorSpace::Hlg => Signal::Hlg,
        ColorSpace::Pq => Signal::Pq,
    }
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
    // The window composites on its own device through `preview_sources`;
    // this is the API's and the command line's frame, on the shared
    // headless compositor.
    headless(|gpu| sources.composite(gpu).into_pixels())
}

/// The paused monitor's frame, described but not yet drawn: the plan
/// with every picture decoded and placed, so a caller with a GPU can draw
/// it where it is shown.
pub struct PreviewSources {
    plan: FramePlan,
    treatments: Vec<Treatment>,
    transitions: Vec<TransitionSpan>,
}

impl PreviewSources {
    /// The frame's description, with every treatment that is shaders
    /// alone already in it; a compositor draws this and nothing else. A
    /// treatment that needs FFmpeg is not in it: see
    /// [`PreviewSources::needs_cpu`].
    pub fn plan(&self) -> &FramePlan {
        &self.plan
    }

    /// Whether the frame cannot be drawn from [`PreviewSources::plan`]
    /// alone, and can only be drawn whole through
    /// [`PreviewSources::composite`]: a live transition is a two-input
    /// combine a plain `FramePlan` has no way to describe.
    pub fn needs_cpu(&self) -> bool {
        self.transitions
            .iter()
            .any(|span| span.covers(self.plan.time))
    }

    /// The instant, in seconds.
    pub fn seconds(&self) -> f32 {
        self.plan.seconds()
    }

    /// The frame, treatments and transitions included, drawn with `compositor`.
    pub fn composite(&self, compositor: &mut dyn Compositor) -> Frame {
        composite_treated(compositor, &self.plan, &self.treatments, &self.transitions)
    }

    /// The frame as raw RGBA, drawn on the shared headless compositor: the
    /// monitor's picture where the window has no device of its own.
    pub fn pixels(&self) -> Result<Vec<u8>, String> {
        headless(|gpu| self.composite(gpu).into_pixels())
    }

    /// The frame drawn whole on the GPU and kept there, when what stops
    /// [`PreviewSources::plan`] drawing it alone is a packaged transition
    /// and nothing else: no live treatment, no layer above the incoming
    /// lane. `None` otherwise, or when the shader cannot run; the caller
    /// then takes [`PreviewSources::composite`]. The same pictures and the
    /// same shader as that path, without its three round trips through
    /// memory a frame.
    pub fn transition_texture(
        &self,
        gpu: &mut concat_render::WgpuCompositor,
    ) -> Option<concat_render::wgpu::Texture> {
        let time = self.plan.time;
        if self
            .treatments
            .iter()
            .any(|treatment| treatment.covers(time))
        {
            return None;
        }
        let span = self.transitions.iter().find(|span| span.covers(time))?;
        if self
            .plan
            .layers
            .iter()
            .any(|layer| layer.track > span.to_track)
        {
            return None;
        }
        let pass =
            Catalogue::builtin().transition_pass(&span.id, &span.params, span.progress(time))?;
        let (from, to) = transition_stages(&self.plan, span);
        gpu.render_transition_texture(&from, &to, &pass)
    }
}

/// Decodes the pictures the paused monitor's frame is made of, through the
/// reader pool so scrubbing revisits are cache hits. See [`preview_frame`]
/// for the rules on what counts as a failure.
pub fn preview_sources(
    pool: &concat_media::ReaderPool,
    request: &PreviewFrameRequest,
) -> Result<PreviewSources, String> {
    preview_sources_of(pool, &preview_timeline(request), request.time, false)
}

/// The pool's request for one planned layer: the source frame at the
/// level that covers the output, so the cached picture is the file's and
/// no knob invalidates it. `proxy` reads the file's stand-in where it has
/// one: for a picture that is moving, never for the paused monitor.
fn frame_request(
    plan: &PreviewPlan,
    layer: &concat_render::PlannedLayer,
    (width, height): (u32, u32),
    still: bool,
    proxy: bool,
) -> concat_media::FrameRequest {
    concat_media::FrameRequest::new(&layer.media, layer.source_time, width, height)
        .covering(plan.width.max(width), plan.height.max(height))
        .as_still(still)
        .from_proxy(proxy)
        .in_range(plan.built.ranges.get(&layer.clip).copied())
        // The compositor fits the picture into its place, so an untreated
        // frame is drawn at the level it was decoded at - deep, for an HDR
        // clip, unless a cutout has to cut it in eight bits.
        .at_any_size(true)
        .deep_when_untreated(!plan.built.cutouts.contains_key(&layer.clip))
}

/// [`preview_sources`] for one instant of a plan already built. With
/// `proxy`, a file with a stand-in is read from it.
pub fn preview_sources_of(
    pool: &concat_media::ReaderPool,
    plan: &PreviewPlan,
    seconds: f64,
    proxy: bool,
) -> Result<PreviewSources, String> {
    let rate = plan.rate;
    let BuiltTimeline {
        timeline,
        stills,
        decode_sizes,
        tracks,
        treatments,
        transitions,
        geometry,
        shapes,
        ranges: _,
        chains,
        reveal_maps,
        cutouts,
        stabilized,
        highlight,
        backdrops,
    } = &plan.built;
    let highlight = *highlight;
    let time = quantise(seconds, rate);
    let plan_at = plan_frame(timeline, time);

    let mut layers: Vec<PlannedLayer> = Vec::with_capacity(plan_at.layers.len());
    let mut failures: Vec<String> = Vec::new();
    for layer in &plan_at.layers {
        let (decode_width, decode_height) = decode_sizes
            .get(&layer.clip)
            .copied()
            .unwrap_or((plan.width, plan.height));
        // A source that fails to decode contributes nothing rather than
        // blanking the monitor - same grace the exporter extends.
        match pool.frame(&frame_request(
            plan,
            layer,
            (decode_width, decode_height),
            stills.contains(&layer.clip),
            proxy,
        )) {
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
                layers.push(filled(
                    layer,
                    frame,
                    tracks.get(&layer.clip).copied().unwrap_or(0),
                    passes_at(chains, reveal_maps, timeline, layer.clip, time),
                    geometry.get(&layer.clip),
                    shapes_at(shapes, layer, time, rate),
                    stabilized.get(&layer.clip),
                ))
            }
            Err(error) => failures.push(format!("{}: {error}", layer.media.display())),
        }
    }

    // Planned layers with nothing decoded is a failed preview, not a black
    // frame: compositing zero sources yields opaque black, and the caller
    // would draw that "truth" over its own perfectly good approximation. An
    // *empty plan* still composites - a gap in the timeline really is black.
    if layers.is_empty() && !plan_at.layers.is_empty() {
        return Err(format!(
            "no layer decoded for the paused preview: {}",
            failures.join(" / ")
        ));
    }

    let mut frame_plan = FramePlan {
        time,
        width: plan.width,
        height: plan.height,
        layers: with_backdrops(
            layers,
            backdrops,
            |clip| !stills.contains(&clip) && !cutouts.contains_key(&clip),
            pool,
            (plan.width, plan.height),
        ),
        treatments: Vec::new(),
        output: plan.output,
    };
    // Every live treatment goes into the plan now, so a caller drawing the
    // plan itself has them.
    frame_plan.treatments = treatments
        .iter()
        .filter(|treatment| treatment.covers(time))
        .map(|treatment| PlannedTreatment {
            track: treatment.track,
            effects: treatment.passes_at(time),
            strength: treatment.strength_at(time),
        })
        .collect();
    frame_plan
        .treatments
        .sort_by_key(|treatment| treatment.track);
    Ok(PreviewSources {
        plan: frame_plan,
        treatments: treatments.clone(),
        transitions: transitions.clone(),
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
    /// What the timeline is output in.
    output: Signal,
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
    color_space: ColorSpace,
) -> PreviewPlan {
    // A preview has no error to give, and a picture at the wrong pace beats
    // none; the document opens with a sane rate (VideoSettings::or) anyway.
    let rate = FrameRate::checked(rate_num, rate_den).unwrap_or(FrameRate::THIRTY);
    let mut resolved = clips.to_vec();
    let transitions = resolve_transitions(&mut resolved, rate);
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
        color_range: concat_media::ColorRange::Limited,
        color_space,
        hdr: false,
        effect_time_offset: 0.0,
        clips: Vec::new(),
    };
    PreviewPlan {
        built: build_timeline(&shim, rate, &visible, transitions),
        rate,
        width,
        height,
        output: output_of(color_space),
    }
}

/// The plan a request describes; see [`preview_plan`].
fn preview_timeline(request: &PreviewFrameRequest) -> PreviewPlan {
    preview_plan(
        &request.clips,
        request.width,
        request.height,
        request.rate_num,
        request.rate_den,
        request.color_space,
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
) {
    preview_prefetch_of(pool, &preview_timeline(request), request.time, frames);
}

/// [`preview_prefetch`] from a plan already built: every frame of the
/// next `frames` instants pulled through the pool, here and now.
pub fn preview_prefetch_of(
    pool: &concat_media::ReaderPool,
    plan: &PreviewPlan,
    seconds: f64,
    frames: u32,
) {
    for moment in preview_moments(plan, seconds, frames, false) {
        for request in &moment.frames {
            let _ = pool.frame(request);
        }
    }
}

/// The instants after `seconds` and the frames each is made of, as a
/// prefetcher takes them: the next `frames` output instants of `plan`,
/// nearest first, with every visible layer's request at each. Nothing is
/// decoded here; see `concat_media::Prefetcher::advance`.
pub fn preview_moments(
    plan: &PreviewPlan,
    seconds: f64,
    frames: u32,
    proxy: bool,
) -> Vec<concat_media::Moment> {
    let rate = plan.rate;
    let BuiltTimeline {
        timeline,
        stills,
        decode_sizes,
        ..
    } = &plan.built;
    let fps = rate.fps().as_f64();
    (1..=frames)
        .map(|ahead| {
            let time = seconds + f64::from(ahead) / fps;
            let plan_at = plan_frame(timeline, quantise(time, rate));
            concat_media::Moment {
                time,
                frames: plan_at
                    .layers
                    .iter()
                    .map(|layer| {
                        let size = decode_sizes
                            .get(&layer.clip)
                            .copied()
                            .unwrap_or((plan.width, plan.height));
                        frame_request(plan, layer, size, stills.contains(&layer.clip), proxy)
                    })
                    .collect(),
            }
        })
        .collect()
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
        let stack = |time: Rational| {
            let ground = ground_layer(red.clone());
            let mut top = PlannedLayer::picture(
                concat_render::detached_clip(),
                std::sync::Arc::new(blue.clone()),
            );
            top.track = 2;
            // Centred at (4, 4) by the fit; three pixels up and left puts
            // it in the corner.
            top.transform = Transform {
                offset_x: -3.0 / 8.0,
                offset_y: -3.0 / 8.0,
                ..Transform::default()
            };
            FramePlan {
                time,
                width: 8,
                height: 8,
                layers: vec![ground, top],
                treatments: Vec::new(),
                output: concat_core::frame::Signal::Sdr,
            }
        };
        let mono = Treatment {
            start: Rational::ZERO,
            end: Rational::from_int(10),
            track: 1,
            effects: vec![AppliedFilter::new("concat.mono")],
            strength: 1.0,
            ramp_in: 0.0,
            ramp_out: 0.0,
        };
        let Some(mut compositor) = WgpuCompositor::new() else {
            return; // no renderer here
        };
        let out = composite_treated(
            &mut compositor,
            &stack(Rational::from_int(1)),
            std::slice::from_ref(&mono),
            &[],
        );
        // Red through Mono is grey where nothing sits on top...
        let grey = at_of(&out, 7, 7);
        assert!(
            grey[0].abs_diff(grey[1]) <= 3 && grey[1].abs_diff(grey[2]) <= 3 && grey[0] < 200,
            "{grey:?}"
        );
        // ...and the blue square above the treatment is untouched.
        assert_eq!(at_of(&out, 0, 0), [0, 0, 255]);

        // At half strength the ground is between red and that grey.
        let half = Treatment {
            strength: 0.5,
            ..mono.clone()
        };
        let out = composite_treated(&mut compositor, &stack(Rational::from_int(1)), &[half], &[]);
        let pixel = at_of(&out, 7, 7);
        assert!(
            pixel[0] > pixel[1] + 60 && pixel[1] > grey[1] / 4 && pixel[0] < 255,
            "{pixel:?}"
        );

        // Outside its span the treatment does nothing.
        let out = composite_treated(
            &mut compositor,
            &stack(Rational::from_int(20)),
            &[mono],
            &[],
        );
        assert_eq!(at_of(&out, 7, 7), [255, 0, 0]);
        fn at_of(frame: &Frame, x: usize, y: usize) -> [u8; 3] {
            let p = &frame.pixels()[(y * 8 + x) * 4..(y * 8 + x) * 4 + 3];
            [p[0], p[1], p[2]]
        }
    }

    /// A filter layer over a cut with a packaged transition grades the
    /// transition, not a dissolve in its place: Push under a Mono layer is
    /// Push in grey. Before, any live adjustment layer sent the frame down
    /// the dissolve fallback, and every transition under a filter looked
    /// like the default fade. Skips without a GPU.
    #[test]
    fn a_filter_layer_over_a_transition_grades_the_transition() {
        let solid = |rgba: [u8; 4]| {
            let mut frame = Frame::black(16, 8);
            for pixel in frame.pixels_mut().chunks_exact_mut(4) {
                pixel.copy_from_slice(&rgba);
            }
            frame
        };
        let time = Rational::from_int(1);
        // Red going out on lane 0, white coming in on lane 2 halfway
        // through its dissolve ramp, as the resolved cut leaves them. White
        // stays white through Mono and red does not, so the graded push
        // keeps two sides where a graded dissolve would have one.
        let plan = || {
            let mut incoming = PlannedLayer::picture(
                concat_render::detached_clip(),
                std::sync::Arc::new(solid([255, 255, 255, 255])),
            );
            incoming.track = 2;
            incoming.opacity = 0.5;
            FramePlan {
                time,
                width: 16,
                height: 8,
                layers: vec![ground_layer(solid([255, 0, 0, 255])), incoming],
                treatments: Vec::new(),
                output: concat_core::frame::Signal::Sdr,
            }
        };
        let span = TransitionSpan {
            start: Rational::ZERO,
            end: Rational::from_int(2),
            to_track: 2,
            id: "concat.push".to_owned(),
            params: BTreeMap::new(),
        };
        let mono = Treatment {
            start: Rational::ZERO,
            end: Rational::from_int(10),
            track: 3,
            effects: vec![AppliedFilter::new("concat.mono")],
            strength: 1.0,
            ramp_in: 0.0,
            ramp_out: 0.0,
        };
        let Some(mut gpu) = WgpuCompositor::new() else {
            return; // no renderer here
        };
        let pushed = composite_treated(&mut gpu, &plan(), &[], std::slice::from_ref(&span));
        let graded = composite_treated(
            &mut gpu,
            &plan(),
            std::slice::from_ref(&mono),
            std::slice::from_ref(&span),
        );
        let at = |frame: &Frame, x: usize| {
            let p = &frame.pixels()[(4 * 16 + x) * 4..(4 * 16 + x) * 4 + 3];
            [p[0], p[1], p[2]]
        };
        // Halfway through a push the two sides are the two pictures, not
        // one mix of them; graded, each side is its own grey.
        let (left, right) = (at(&pushed, 1), at(&pushed, 14));
        assert_ne!(left, right, "the push drew: {left:?} {right:?}");
        let (left, right) = (at(&graded, 1), at(&graded, 14));
        for pixel in [left, right] {
            assert!(
                pixel[0].abs_diff(pixel[1]) <= 3 && pixel[1].abs_diff(pixel[2]) <= 3,
                "graded grey: {pixel:?}"
            );
        }
        assert!(
            left[0].abs_diff(right[0]) > 20,
            "still a push under the grade, not a dissolve: {left:?} {right:?}"
        );
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
            params: BTreeMap::new(),
        })
    }

    /// The monitor draws a wipe and a fade to black as the export does:
    /// halfway through the wipe the old picture on one side and the new on
    /// the other, and black at the fade's cut. It used to show a dissolve
    /// for the wipe and nothing at all for the fade, whose filters counted
    /// decoded frames the monitor's pooled seeks never had.
    #[test]
    fn the_monitor_draws_wipes_and_fades_as_the_export_does() {
        use concat_core::frame::Frame;
        use concat_media::{EncodeOptions, Encoder, FrameSink};

        let (width, height) = (64_u32, 36_u32);
        let solid = |name: &str, colour: [u8; 3]| -> Option<String> {
            let path = std::env::temp_dir()
                .join(format!("concat-preview-{name}-{}.mp4", std::process::id()));
            let mut encoder = Encoder::create(
                &path,
                width,
                height,
                FrameRate::THIRTY,
                &EncodeOptions {
                    crf: 12,
                    ..EncodeOptions::default()
                },
            )
            .ok()?;
            let mut frame = Frame::black(width, height);
            frame.fill([colour[0], colour[1], colour[2], 255]);
            for _ in 0..90 {
                encoder.write_frame(&frame).expect("writes");
            }
            encoder.finish().expect("finishes");
            Some(path.to_string_lossy().into_owned())
        };
        let (Some(red), Some(blue)) = (solid("red", [220, 30, 30]), solid("blue", [30, 30, 220]))
        else {
            return; // no ffmpeg here
        };
        let pool = concat_media::ReaderPool::new(16 * 1024 * 1024, 2);
        let look = |kind: &str, time: f64, x: f64| -> [u8; 3] {
            let mut first = clip("video", 0, 0.0, 2.0, 0.0);
            first.path = red.clone();
            let mut second = clip("video", 0, 2.0, 1.0, 1.0);
            second.path = blue.clone();
            second.transition = spec(kind, 1.0);
            for one in [&mut first, &mut second] {
                one.media_width = Some(width);
                one.media_height = Some(height);
            }
            let request = PreviewFrameRequest {
                time,
                width,
                height,
                rate_num: 30,
                rate_den: 1,
                clips: vec![first, second],
                color_space: concat_project::model::ColorSpace::Sdr,
            };
            let bytes = preview_frame(&pool, &request).expect("previews");
            let (px, py) = ((f64::from(width) * x) as u32, height / 2);
            let i = ((py * width + px) * 4) as usize;
            [bytes[i], bytes[i + 1], bytes[i + 2]]
        };
        let near = |got: [u8; 3], want: [u8; 3]| {
            got.iter()
                .zip(want)
                .all(|(got, want)| (i16::from(*got) - i16::from(want)).abs() <= 40)
        };
        for (kind, time, x, want) in [
            ("wipe-left", 2.0, 0.2, [220, 30, 30]),
            ("wipe-left", 2.0, 0.8, [30, 30, 220]),
            ("wipe-right", 2.0, 0.2, [30, 30, 220]),
            ("wipe-right", 2.0, 0.8, [220, 30, 30]),
            ("fade-black", 2.0, 0.5, [0, 0, 0]),
        ] {
            let got = look(kind, time, x);
            assert!(
                near(got, want),
                "{kind} at {time}s, {x} across: the monitor shows {got:?}, not {want:?}"
            );
        }
        let _ = std::fs::remove_file(&red);
        let _ = std::fs::remove_file(&blue);
    }

    /// The chain as it was met in an edit: three clips with a transition at
    /// each cut and a logo on the track above. The third clip shared the
    /// logo's lane, so the monitor and the export drew one or the other for
    /// the whole clip, whichever was placed last, and the clip was black
    /// under the logo. Both draw now, whatever order the clips come in.
    /// Skips without FFmpeg or a GPU.
    #[test]
    fn a_logo_over_a_chain_of_transitions_and_the_clips_beneath_it_all_draw() {
        use concat_core::frame::Frame;
        use concat_media::{EncodeOptions, Encoder, FrameSink};

        let (width, height) = (64_u32, 64_u32);
        let solid = |name: &str, colour: [u8; 3]| -> Option<String> {
            let path = std::env::temp_dir()
                .join(format!("concat-chain-{name}-{}.mp4", std::process::id()));
            let mut encoder = Encoder::create(
                &path,
                width,
                height,
                FrameRate::THIRTY,
                &EncodeOptions {
                    crf: 12,
                    ..EncodeOptions::default()
                },
            )
            .ok()?;
            let mut frame = Frame::black(width, height);
            frame.fill([colour[0], colour[1], colour[2], 255]);
            for _ in 0..90 {
                encoder.write_frame(&frame).expect("writes");
            }
            encoder.finish().expect("finishes");
            Some(path.to_string_lossy().into_owned())
        };
        let colours = [
            ("red", [220, 30, 30]),
            ("green", [30, 200, 30]),
            ("blue", [30, 30, 220]),
            ("white", [235, 235, 235]),
        ];
        let paths: Vec<String> = colours
            .iter()
            .filter_map(|(name, colour)| solid(name, *colour))
            .collect();
        let cleanup = || {
            for path in &paths {
                let _ = std::fs::remove_file(path);
            }
        };
        if paths.len() < colours.len() || WgpuCompositor::new().is_none() {
            cleanup();
            return; // no ffmpeg or no renderer here
        }

        let footage = |n: usize, track: usize, start: f64, source_start: f64| {
            let mut one = clip("video", track, start, 2.0, source_start);
            one.path = paths[n].clone();
            one.media_width = Some(width);
            one.media_height = Some(height);
            one.media_duration = Some(3.0);
            one
        };
        let mut chain = vec![
            footage(0, 0, 0.0, 0.5),
            footage(1, 0, 2.0, 0.5),
            footage(2, 0, 4.0, 0.5),
        ];
        chain[1].transition = spec("concat.dissolve", 0.5);
        chain[2].transition = spec("concat.whip-pan", 0.5);
        // A logo a quarter of the frame across, in the bottom right corner,
        // over all three clips.
        let mut logo = footage(3, 1, 0.0, 0.0);
        logo.duration = 6.0;
        logo.scale = 0.25;
        logo.offset_x = 0.375;
        logo.offset_y = 0.375;

        let pool = concat_media::ReaderPool::new(16 * 1024 * 1024, 2);
        let pixel = |clips: &[ExportClip], time: f64, x: u32, y: u32| -> [u8; 3] {
            let request = PreviewFrameRequest {
                time,
                width,
                height,
                rate_num: 30,
                rate_den: 1,
                clips: clips.to_vec(),
                color_space: concat_project::model::ColorSpace::Sdr,
            };
            let bytes = preview_frame(&pool, &request).expect("previews");
            let i = ((y * width + x) * 4) as usize;
            [bytes[i], bytes[i + 1], bytes[i + 2]]
        };
        let near = |got: [u8; 3], want: [u8; 3]| {
            got.iter()
                .zip(want)
                .all(|(got, want)| (i16::from(*got) - i16::from(want)).abs() <= 40)
        };
        // On one shared lane, the logo placed after the chain hid the third
        // clip and the logo placed before it was hidden by the third clip.
        for logo_first in [false, true] {
            let mut clips = chain.clone();
            if logo_first {
                clips.insert(0, logo.clone());
            } else {
                clips.push(logo.clone());
            }
            for (time, want) in [
                (1.0, [220, 30, 30]),
                (3.0, [30, 200, 30]),
                (5.0, [30, 30, 220]),
            ] {
                let centre = pixel(&clips, time, width / 2, height / 2);
                assert!(
                    near(centre, want),
                    "logo first: {logo_first}; at {time}s the clip shows {centre:?}, not {want:?}"
                );
                let corner = pixel(&clips, time, width - 8, height - 8);
                assert!(
                    near(corner, [235, 235, 235]),
                    "logo first: {logo_first}; at {time}s the logo shows {corner:?}"
                );
            }
        }
        cleanup();
    }

    /// The monitor draws a crop and a flip as the export does: the crop in
    /// the source's own terms, then the mirror, the kept part fitted and
    /// centred with bars around it. The monitor reads an untreated frame
    /// at its decoded level and leaves the fit, crop and flip to the frame
    /// plan, so this is what holds the two paths to one picture.
    #[test]
    fn the_monitor_crops_and_flips_as_the_export_does() {
        use concat_core::frame::Frame;
        use concat_media::{EncodeOptions, Encoder, FrameSink};

        let (width, height) = (128_u32, 72_u32);
        let path = std::env::temp_dir().join(format!(
            "concat-preview-geometry-{}.mp4",
            std::process::id()
        ));
        let Ok(mut encoder) = Encoder::create(
            &path,
            width,
            height,
            FrameRate::THIRTY,
            &EncodeOptions {
                crf: 12,
                ..EncodeOptions::default()
            },
        ) else {
            return; // no ffmpeg here
        };
        // Top left red, top right green, bottom left blue, bottom right
        // yellow.
        let colours = [[220, 30, 30], [30, 200, 30], [30, 30, 220], [230, 220, 30]];
        let mut frame = Frame::black(width, height);
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] =
                    colours[usize::from(y >= height / 2) * 2 + usize::from(x >= width / 2)];
                let at = ((y * width + x) * 4) as usize;
                frame.pixels_mut()[at..at + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
        for _ in 0..30 {
            encoder.write_frame(&frame).expect("writes");
        }
        encoder.finish().expect("finishes");

        let mut request_clip = clip("video", 0, 0.0, 1.0, 0.0);
        request_clip.path = path.to_string_lossy().into_owned();
        request_clip.media_width = Some(width);
        request_clip.media_height = Some(height);
        request_clip.crop = Some([0.0, 0.5, 0.0, 0.0]);
        request_clip.flip_h = true;
        let request = PreviewFrameRequest {
            time: 0.5,
            width,
            height,
            rate_num: 30,
            rate_den: 1,
            clips: vec![request_clip],
            color_space: concat_project::model::ColorSpace::Sdr,
        };
        let pool = concat_media::ReaderPool::new(16 * 1024 * 1024, 2);
        let bytes = preview_frame(&pool, &request).expect("previews");
        let _ = std::fs::remove_file(&path);
        let at = |x: f64, y: f64| {
            let (px, py) = (
                (f64::from(width) * x) as u32,
                (f64::from(height) * y) as u32,
            );
            let i = ((py * width + px) * 4) as usize;
            [bytes[i], bytes[i + 1], bytes[i + 2]]
        };
        let near = |got: [u8; 3], want: [u8; 3]| {
            got.iter()
                .zip(want)
                .all(|(got, want)| (i16::from(*got) - i16::from(want)).abs() <= 40)
        };
        // The bottom half kept, mirrored: yellow on the left, blue on the
        // right, black bars above and below.
        for ((x, y), want) in [
            ((0.25, 0.5), colours[3]),
            ((0.75, 0.5), colours[2]),
            ((0.5, 0.05), [0, 0, 0]),
            ((0.5, 0.95), [0, 0, 0]),
        ] {
            let got = at(x, y);
            assert!(
                near(got, want),
                "at ({x}, {y}) the monitor shows {got:?}, not {want:?}"
            );
        }
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
            color_space: concat_project::model::ColorSpace::Sdr,
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
            color_space: concat_project::model::ColorSpace::Sdr,
        };
        let bytes = preview_frame(&pool, &late).expect("previews past the media's end");
        assert!(
            bytes[centre] > 120 && bytes[centre + 1] < 90,
            "past the end should freeze on the last frame, got {:?}",
            &bytes[centre..centre + 4],
        );

        // A clip with an effect: the paused monitor must show the treated
        // pixels, not the raw decode. Red footage through Mono comes back
        // grey.
        let mut effected = clip("video", 0, 0.0, 1.0, 0.0);
        effected.path = path.to_string_lossy().into_owned();
        effected.media_width = Some(64);
        effected.media_height = Some(64);
        effected.effects = vec![AppliedFilter::new("concat.mono")];
        let filtered = PreviewFrameRequest {
            time: 0.5,
            width: 64,
            height: 64,
            rate_num: 30,
            rate_den: 1,
            clips: vec![effected],
            color_space: concat_project::model::ColorSpace::Sdr,
        };
        let bytes = preview_frame(&pool, &filtered).expect("previews with an effect");
        assert!(
            bytes[centre] < 200 && bytes[centre].abs_diff(bytes[centre + 1]) < 16,
            "the effect must be drawn into the paused frame, got {:?}",
            &bytes[centre..centre + 4],
        );

        let _ = std::fs::remove_file(&path);
    }

    /// A fresh adjustment layer - the colour panel's package at its
    /// defaults, laid over the footage through the command the library
    /// uses - leaves the picture exactly as it was, on the monitor's plan
    /// and through the export's compositing alike; graded, it changes the
    /// picture and the two still agree (#249). Skips without FFmpeg or a
    /// GPU.
    #[test]
    fn a_fresh_adjustment_layer_changes_nothing_on_the_monitor_or_in_the_export() {
        use concat_core::frame::Frame;
        use concat_media::{EncodeOptions, Encoder, FrameSink};
        use concat_project::commands::{ClipPatch, NewMedia};
        use concat_project::model::MediaKind;
        use concat_project::{Command, Editor};

        let (width, height) = (64_u32, 64_u32);
        let path = std::env::temp_dir().join(format!(
            "concat-adjustment-layer-{}.mp4",
            std::process::id()
        ));
        let Ok(mut encoder) = Encoder::create(
            &path,
            width,
            height,
            FrameRate::THIRTY,
            &EncodeOptions {
                crf: 12,
                ..EncodeOptions::default()
            },
        ) else {
            return; // no ffmpeg here
        };
        // Something in every channel at every level, so a grade that moved
        // any of them would show.
        let mut picture = Frame::black(width, height);
        for y in 0..height {
            for x in 0..width {
                picture.set_pixel(x, y, [(x * 4) as u8, (y * 4) as u8, 150, 255]);
            }
        }
        for _ in 0..60 {
            encoder.write_frame(&picture).expect("writes");
        }
        encoder.finish().expect("finishes");
        let Some(mut gpu) = WgpuCompositor::new() else {
            let _ = std::fs::remove_file(&path);
            return; // no renderer here
        };

        let mut editor = Editor::new();
        let media_id = editor
            .apply(Command::AddMedia {
                item: NewMedia {
                    path: path.to_string_lossy().into_owned(),
                    name: "gradient.mp4".into(),
                    duration: Some(2.0),
                    kind: MediaKind::Video,
                    width: Some(width),
                    height: Some(height),
                    frame_rate: None,
                    frame_rate_fraction: None,
                    video_codec: None,
                    audio_codec: None,
                    has_audio: false,
                    audio_tracks: Vec::new(),
                    origin: None,
                    color_space: Default::default(),
                },
            })
            .expect("adds media")
            .created_id
            .expect("an id");
        let track_id = editor.project().timelines[0].tracks[0].id.clone();
        editor
            .apply(Command::AddClip {
                media_id,
                track_id,
                start: 0.0,
                ripple: false,
            })
            .expect("adds the footage");
        let plain = flatten::flatten_timeline(editor.project(), None);
        let layer_id = editor
            .apply(Command::AddLayerClip {
                track_id: None,
                start: 0.0,
                duration: Some(2.0),
                effect_id: "concat.adjust".into(),
                name: "Adjustment".into(),
                sound: false,
            })
            .expect("adds the layer")
            .created_id
            .expect("an id");
        let fresh = flatten::flatten_timeline(editor.project(), None);
        assert!(
            fresh
                .iter()
                .any(|clip| clip.kind == ClipKind::Layer && clip.track > 0),
            "the layer is over the footage"
        );

        let pool = concat_media::ReaderPool::new(16 * 1024 * 1024, 2);
        let request = |clips: Vec<ExportClip>| PreviewFrameRequest {
            time: 1.0,
            width,
            height,
            rate_num: 30,
            rate_den: 1,
            clips,
            color_space: concat_project::model::ColorSpace::Sdr,
        };
        // The window draws the plan with every live treatment folded in;
        // the export draws through `composite_treated`.
        let mut both = |clips: Vec<ExportClip>| -> (Frame, Frame) {
            let sources = preview_sources(&pool, &request(clips)).expect("previews");
            let monitor = gpu.render(sources.plan());
            let export = sources.composite(&mut gpu);
            (monitor, export)
        };
        let furthest = |a: &Frame, b: &Frame| -> u8 {
            a.pixels()
                .iter()
                .zip(b.pixels())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0)
        };

        let (before, before_export) = both(plain);
        assert_eq!(furthest(&before, &before_export), 0, "untreated");
        let (monitor, export) = both(fresh);
        assert!(
            furthest(&before, &monitor) <= 1,
            "a fresh adjustment layer moved the monitor's picture by {}",
            furthest(&before, &monitor)
        );
        assert!(
            furthest(&before, &export) <= 1,
            "a fresh adjustment layer moved the exported picture by {}",
            furthest(&before, &export)
        );

        // A stop up: the picture changes, the same on both.
        let mut graded = AppliedFilter::new("concat.adjust");
        graded.params.insert("exposure".into(), 1.0);
        editor
            .apply(Command::UpdateClip {
                clip_id: layer_id,
                patch: ClipPatch {
                    video_effects: Some(vec![graded]),
                    ..ClipPatch::default()
                },
            })
            .expect("grades the layer");
        let (monitor, export) = both(flatten::flatten_timeline(editor.project(), None));
        eprintln!(
            "a stop up moves the picture by {} at most; monitor and export differ by {}",
            furthest(&before, &monitor),
            furthest(&monitor, &export)
        );
        assert!(
            furthest(&before, &monitor) > 20,
            "a stop up must show on the monitor"
        );
        assert!(
            furthest(&monitor, &export) <= 1,
            "the monitor and the export disagree by {}",
            furthest(&monitor, &export)
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A still keeps its own shape on the monitor: a portrait photograph
    /// in a square frame stands in the middle with the stage either side
    /// of it, not stretched across the frame (#229). Skips without FFmpeg.
    #[test]
    fn a_still_keeps_its_shape_on_the_monitor() {
        use concat_core::frame::Frame;

        let path = std::env::temp_dir().join("concat-preview-still-test.jpg");
        let mut tall = Frame::black(32, 64);
        tall.fill([200, 30, 30, 255]);
        let Ok(bytes) = concat_media::jpeg(&tall, 90) else {
            return; // no ffmpeg here
        };
        std::fs::write(&path, bytes).expect("writes the still");

        let mut still = clip("image", 0, 0.0, 1.0, 0.0);
        still.path = path.to_string_lossy().into_owned();
        still.media_width = Some(32);
        still.media_height = Some(64);
        let request = PreviewFrameRequest {
            time: 0.5,
            width: 64,
            height: 64,
            rate_num: 30,
            rate_den: 1,
            clips: vec![still],
            color_space: concat_project::model::ColorSpace::Sdr,
        };

        let pool = concat_media::ReaderPool::new(16 * 1024 * 1024, 2);
        let bytes = preview_frame(&pool, &request).expect("previews a still");
        assert_eq!(bytes.len(), 64 * 64 * 4);
        let at = |x: usize, y: usize| {
            let index = (y * 64 + x) * 4;
            [bytes[index], bytes[index + 1], bytes[index + 2]]
        };
        let centre = at(32, 32);
        assert!(
            centre[0] > 120 && centre[1] < 90,
            "the still is drawn in the middle, got {centre:?}"
        );
        for x in [2, 61] {
            let edge = at(x, 32);
            assert!(
                edge[0] < 90,
                "the stage shows either side of a portrait still, got {edge:?} at x={x}"
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    /// The folder the file goes in is made by the export: a phone's
    /// Movies folder does not exist until the first export (#280).
    #[test]
    fn an_export_makes_the_folder_it_writes_into() {
        use concat_core::frame::Frame;
        use concat_media::{EncodeOptions, Encoder, FrameSink};

        let root = std::env::temp_dir().join(format!(
            "concat-export-folder-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_nanos())
        ));
        let source = std::env::temp_dir().join("concat-export-folder-source.mp4");
        let Ok(mut encoder) = Encoder::create(
            &source,
            64,
            64,
            FrameRate::THIRTY,
            &EncodeOptions::default(),
        ) else {
            return; // no ffmpeg here
        };
        for _ in 0..6 {
            encoder.write_frame(&Frame::black(64, 64)).expect("writes");
        }
        encoder.finish().expect("finishes");

        let mut footage = clip("video", 0, 0.0, 0.2, 0.0);
        footage.path = source.to_string_lossy().into_owned();
        footage.media_width = Some(64);
        footage.media_height = Some(64);
        let nested = root.join("Movies").join("Concat");
        let request = ExportRequest {
            output: nested.join("out.mp4").to_string_lossy().into_owned(),
            width: 64,
            height: 64,
            rate_num: 30,
            rate_den: 1,
            crf: 30,
            preset: "ultrafast".into(),
            codec: VideoCodec::H264,
            ten_bit: false,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 0,
            color_range: concat_media::ColorRange::Limited,
            color_space: ColorSpace::Sdr,
            hdr: false,
            effect_time_offset: 0.0,
            clips: vec![footage],
        };
        assert!(!nested.exists());
        let cancel = AtomicBool::new(false);
        let mut progress = |_: i64, _: i64, _: &'static str| {};
        let reporter = Reporter {
            progress: &mut progress,
            cancel: &cancel,
        };
        // Whether or not this machine can draw the frames, the folder is
        // there before the first one is asked for.
        let result = render_on(&request, None, reporter);
        assert!(nested.is_dir(), "the export makes its folder: {result:?}");
        if result.is_ok() {
            assert!(nested.join("out.mp4").is_file());
        }
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&source);
    }

    /// A compositor that fails a frame - one the device refused, or did not
    /// finish within `DEVICE_WAIT` - does not hang the export or end it:
    /// the frame is drawn again on a fresh device and the file is written
    /// (#297). Lent dead, so its first frame is a failed one.
    #[test]
    fn an_export_goes_on_past_a_failed_frame() {
        use concat_core::frame::Frame;
        use concat_media::{EncodeOptions, Encoder, FrameSink};

        let Some(mut failing) = WgpuCompositor::new() else {
            return; // nothing to draw on here
        };
        // Wider than any GPU makes a texture.
        let mut wide = Frame::transparent(1 << 17, 2);
        wide.fill([255, 255, 255, 255]);
        failing.render(&FramePlan {
            layers: vec![PlannedLayer::picture(
                concat_render::detached_clip(),
                Arc::new(wide),
            )],
            ..FramePlan::empty(16, 16)
        });
        assert!(failing.lost(), "the refused frame failed");

        let dir = std::env::temp_dir().join(format!("concat-export-failed-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let source = dir.join("source.mp4");
        let Ok(mut encoder) = Encoder::create(
            &source,
            64,
            64,
            FrameRate::THIRTY,
            &EncodeOptions::default(),
        ) else {
            return; // no ffmpeg here
        };
        for _ in 0..6 {
            encoder.write_frame(&Frame::black(64, 64)).expect("writes");
        }
        encoder.finish().expect("finishes");
        let mut footage = clip("video", 0, 0.0, 0.2, 0.0);
        footage.path = source.to_string_lossy().into_owned();
        footage.media_width = Some(64);
        footage.media_height = Some(64);
        let output = dir.join("out.mp4");
        let request = ExportRequest {
            output: output.to_string_lossy().into_owned(),
            width: 64,
            height: 64,
            rate_num: 30,
            rate_den: 1,
            crf: 30,
            preset: "ultrafast".into(),
            codec: VideoCodec::H264,
            ten_bit: false,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 0,
            color_range: concat_media::ColorRange::Limited,
            color_space: ColorSpace::Sdr,
            hdr: false,
            effect_time_offset: 0.0,
            clips: vec![footage],
        };
        let cancel = AtomicBool::new(false);
        let mut progress = |_: i64, _: i64, _: &'static str| {};
        let reporter = Reporter {
            progress: &mut progress,
            cancel: &cancel,
        };
        let result = render_on(&request, Some(failing), reporter);
        let _ = std::fs::remove_dir_all(&dir);
        result.expect("the export goes on on a fresh device");
    }

    /// A device that dies under an export is replaced: first by a fresh
    /// one on the GPU, then by the software adapter; the third death ends
    /// the export with a message that says how many there were.
    #[test]
    fn an_export_gives_up_after_enough_device_losses() {
        let Err(error) = recovered(DEVICE_LOSSES, false) else {
            panic!("a third loss gives up");
        };
        assert!(error.contains("lost 3 times"), "{error}");
        assert!(error.contains("not answering"), "{error}");
        if WgpuCompositor::new().is_some() {
            let fresh = recovered(1, true).expect("a machine that draws opens another device");
            assert!(!fresh.lost());
        }
    }

    /// A slow export still moves: a frame that took over a second is
    /// reported whatever its number, and a fast one every fifteen frames
    /// as before.
    #[test]
    fn a_slow_export_reports_every_second() {
        let quick = std::time::Duration::from_millis(30);
        assert!(progress_due(0, quick));
        assert!(!progress_due(1, quick));
        assert!(progress_due(15, quick));
        assert!(progress_due(1, std::time::Duration::from_millis(1200)));
        assert!(progress_due(7, PROGRESS_INTERVAL));
    }

    /// A phone's chip that stops taking frames part way through an export
    /// sends the export round once more on the CPU's encoder; a second
    /// stall, or any other failure, ends it with words that say what was
    /// tried (#491).
    #[test]
    fn a_stalled_hardware_encoder_is_given_one_more_go_in_software() {
        let stall = |hardware| concat_media::Error::Stalled {
            operation: "encode",
            path: "out.mp4".into(),
            detail: "the encoder took no frame for 10 seconds".to_owned(),
            hardware,
        };
        let mut options = EncodeOptions::default();
        again_in_software(&stall(true), &mut options, 120, 480)
            .expect("the first stall on hardware goes again");
        assert!(options.software_only, "and goes again on the CPU");

        // Once: a stall on the CPU, or anything else, then fails.
        let error = again_in_software(&stall(false), &mut options, 30, 480)
            .expect_err("the CPU stalling too ends the export");
        assert!(error.contains("software encoder"), "{error}");
        assert!(error.contains("took no frame for 10 seconds"), "{error}");
        assert!(options.software_only);
        let error = again_in_software(&stall(true), &mut options, 30, 480)
            .expect_err("never a third pass, whatever the encoder says");
        assert!(error.contains("failed too"), "{error}");

        // A stall on the CPU first, or a failure that is not a stall, is
        // said as it always was.
        let mut cpu_first = EncodeOptions::default();
        assert_eq!(
            again_in_software(&stall(false), &mut cpu_first, 30, 480),
            Err("encode failed for out.mp4: the encoder took no frame for 10 seconds".to_owned())
        );
        let refused = concat_media::Error::Ffi {
            operation: "write packet",
            path: "out.mp4".into(),
            detail: "No space left on device".to_owned(),
        };
        let mut options = EncodeOptions::default();
        assert!(again_in_software(&refused, &mut options, 30, 480).is_err());
        assert!(!cpu_first.software_only && !options.software_only);
    }

    /// Only Linux is told to install a software renderer: a phone has
    /// nothing to install, and Windows and macOS have theirs built in.
    #[test]
    fn the_no_renderer_advice_fits_the_platform() {
        assert_eq!(
            NO_RENDERER.contains("lavapipe"),
            cfg!(target_os = "linux"),
            "{NO_RENDERER}"
        );
        if cfg!(any(target_os = "android", target_os = "ios")) {
            assert!(NO_RENDERER.contains("GPU could not be used"));
        }
    }

    /// Two clips cut at 4 s on one track, the incoming one with two
    /// seconds of footage before its in-point and the outgoing one with
    /// six past its out-point, unless a test takes them away.
    fn cut_with_handles() -> Vec<ExportClip> {
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("video", 0, 4.0, 4.0, 2.0),
        ];
        clips[0].media_duration = Some(10.0);
        clips
    }

    #[test]
    fn a_cross_fade_is_centred_on_the_cut() {
        let mut clips = cut_with_handles();
        clips[1].transition = spec("cross-fade", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY);

        let b = &clips[1];
        assert_eq!(b.start, 3.5, "starts half the transition before the cut");
        assert_eq!(b.duration, 4.5);
        assert_eq!(
            b.source_start, 1.5,
            "consumes the handle before the in-point"
        );
        assert_eq!(b.video_fade_in, 1.0);
        assert_eq!(b.fade_in, 1.0, "sound rides the picture");
        assert_eq!(b.track, 1, "incoming sits directly above the outgoing clip");

        let a = &clips[0];
        assert_eq!(
            (a.start, a.duration, a.source_start),
            (0.0, 4.5, 0.0),
            "runs on half the transition past the cut"
        );
        assert_eq!(a.fade_out, 1.0, "and its sound fades out across it");
        assert_eq!(a.track, 0, "outgoing stays on its doubled lane");
        assert_eq!(clips.len(), 2, "no hold: both sides have footage");
    }

    /// A clip lengthened for a transition keeps its keys at the instants
    /// they were at, although a key's place is a fraction of the clip.
    #[test]
    fn an_extended_clip_keeps_its_keys_where_they_were() {
        let mut clips = cut_with_handles();
        for (n, at) in [1.0, 0.0].into_iter().enumerate() {
            clips[n].animation.push(ExportKey {
                property: "scale".to_owned(),
                at,
                value: 2.0,
                ease: linear_ease(),
            });
        }
        clips[1].transition = spec("cross-fade", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY);
        assert_eq!(keys_on(&clips[0], "scale"), vec![(4.0 / 4.5, 2.0)]);
        assert_eq!(keys_on(&clips[1], "scale"), vec![(0.5 / 4.5, 2.0)]);
    }

    /// A packaged transition's knobs, as set on the cut, reach the span
    /// the compositor combines with: before, every span was built empty and
    /// every transition drew at its defaults, whatever the cut held.
    #[test]
    fn a_packaged_transitions_knobs_reach_its_span() {
        let mut clips = cut_with_handles();
        let mut transition = spec("concat.push", 1.0).expect("a spec");
        transition.params.insert("direction".to_owned(), 2.0);
        clips[1].transition = Some(transition);
        let spans = resolve_transitions(&mut clips, FrameRate::THIRTY);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].params.get("direction"), Some(&2.0));
    }

    #[test]
    fn a_packaged_transition_id_overlaps_the_clip_and_emits_a_span() {
        // A transition named by its package id (not one of the seven legacy
        // strings) takes the new path: the same overlap a dissolve gets, plus
        // a `TransitionSpan` for the compositor to combine over.
        let mut clips = cut_with_handles();
        clips[1].transition = spec("concat.dissolve", 1.0);
        let spans = resolve_transitions(&mut clips, FrameRate::THIRTY);

        let b = &clips[1];
        assert_eq!(b.start, 3.5, "centred on the cut, like a dissolve");
        assert_eq!(b.duration, 4.5);
        assert_eq!(b.video_fade_in, 1.0, "the GPU-less fallback dissolve");
        assert_eq!(b.track, 1);
        assert_eq!(clips[0].duration, 4.5);

        assert_eq!(spans.len(), 1, "one packaged transition over the cut");
        let span = &spans[0];
        assert_eq!(span.id, "concat.dissolve");
        assert_eq!(span.to_track, 1, "the incoming clip's own lane");
        assert_eq!(span.start, Rational::new(7, 2));
        assert_eq!(span.end, Rational::new(9, 2));
        assert!((span.progress(Rational::new(7, 2)) - 0.0).abs() < 1e-9);
        assert!((span.progress(Rational::from_int(4)) - 0.5).abs() < 1e-9);
        assert!((span.progress(Rational::new(9, 2)) - 1.0).abs() < 1e-9);
    }

    /// An incoming clip with less footage before its in-point than half
    /// the transition takes what it has - winding the start back further
    /// without the clock played everything after the cut early and ran the
    /// end off the file into black (#236) - and the rest of the overlap
    /// goes after the cut, where the outgoing clip runs on.
    #[test]
    fn a_short_handle_leans_the_overlap_after_the_cut() {
        let mut clips = cut_with_handles();
        clips[1].source_start = 0.25;
        clips[1].fade_in = 0.25;
        clips[1].transition = spec("cross-fade", 2.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY);

        let b = &clips[1];
        assert_eq!((b.start, b.duration, b.source_start), (3.75, 4.25, 0.0));
        assert_eq!(b.video_fade_in, 2.0);
        assert_eq!(b.track, 1);
        assert_eq!(b.fade_in, 2.0, "the dissolve's sound fade");
        assert_eq!(clips[0].duration, 5.75, "runs on for the other 1.75 s");
        assert_eq!(clips.len(), 2, "nothing is held");
    }

    #[test]
    fn an_untrimmed_incoming_clip_starts_the_overlap_at_the_cut() {
        let mut clips = cut_with_handles();
        clips[1].source_start = 0.0;
        clips[1].transition = spec("cross-fade", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY);

        let b = &clips[1];
        assert_eq!((b.start, b.duration, b.source_start), (4.0, 4.0, 0.0));
        assert_eq!(b.track, 1, "above the outgoing clip running on");
        assert_eq!(b.video_fade_in, 1.0);
        assert_eq!(clips[0].duration, 5.0);
    }

    /// An outgoing clip with no footage past its out-point - the file ends
    /// there, or its length is not known - ends at the cut, and its last
    /// frame is held beneath the incoming one for the rest of the overlap.
    #[test]
    fn an_outgoing_clip_without_footage_past_its_end_is_held() {
        let mut clips = cut_with_handles();
        clips[0].media_duration = Some(4.0);
        clips[1].transition = spec("concat.dissolve", 1.0);
        let spans = resolve_transitions(&mut clips, FrameRate::THIRTY);

        let a = &clips[0];
        assert_eq!((a.start, a.duration), (0.0, 4.0), "ends at the cut");
        assert_eq!(clips.len(), 3, "the hold is a clip of its own");
        let hold = &clips[2];
        assert!(hold.hold);
        assert_eq!((hold.start, hold.duration), (4.0, 0.5));
        assert_eq!(
            hold.source_start,
            4.0 - 1.0 / 30.0,
            "the frame the clip ends on"
        );
        assert_eq!(hold.track, 0, "on the outgoing clip's lane");
        assert!(hold.muted);
        assert_eq!(hold.has_audio, Some(false));

        let b = &clips[1];
        assert_eq!((b.start, b.track), (3.5, 1));
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start, Rational::new(7, 2));
        assert_eq!(spans[0].end, Rational::new(9, 2));
        assert_eq!(spans[0].to_track, 1);
    }

    /// A push cannot slide a held frame - the hold carries no keys - so
    /// over an outgoing clip without footage past its end it dissolves;
    /// a keyed placement on the clip is held where its ride leaves it.
    #[test]
    fn a_push_over_a_held_outgoing_frame_becomes_a_dissolve() {
        let mut clips = cut_with_handles();
        clips[0].media_duration = None;
        for (at, value) in [(0.0, 1.0), (1.0, 2.0)] {
            clips[0].animation.push(ExportKey {
                property: "scale".to_owned(),
                at,
                value,
                ease: linear_ease(),
            });
        }
        clips[1].transition = spec("push", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY);

        let hold = &clips[2];
        assert!(hold.animation.is_empty(), "the ride stays on the clip");
        assert_eq!(hold.scale, 2.0, "held where the ride ends");
        assert_eq!(clips[1].video_fade_in, 1.0, "falls back to the dissolve");
        assert!(keys_on(&clips[1], "offsetX").is_empty());
        assert!(keys_on(&clips[0], "offsetX").is_empty());
        assert_eq!(
            keys_on(&clips[0], "scale").len(),
            2,
            "the user's keys are untouched"
        );
    }

    #[test]
    fn a_still_needs_no_handle_to_dissolve() {
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("image", 0, 4.0, 4.0, 0.0),
        ];
        clips[1].transition = spec("cross-fade", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY);
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
        let mut clips = cut_with_handles();
        clips[1].transition = spec("push", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY);

        // The overlap is the dissolve's: half a second either side of the
        // cut, the incoming clip on the lane above, sound fading in with
        // it - but no picture fade.
        let b = &clips[1];
        assert_eq!((b.start, b.duration, b.source_start), (3.5, 4.5, 1.5));
        assert_eq!(b.track, 1);
        assert_eq!(b.fade_in, 1.0);
        assert_eq!(b.video_fade_in, 0.0, "a push does not dissolve");
        // The incoming picture comes in from a frame's width to the right
        // over its first second.
        assert_eq!(keys_on(b, "offsetX"), vec![(0.0, 1.0), (1.0 / 4.5, 0.0)]);
        // The outgoing one leaves to the left over its last second.
        assert_eq!(
            keys_on(&clips[0], "offsetX"),
            vec![(1.0 - 1.0 / 4.5, 0.0), (1.0, -1.0)]
        );
        // Both ride the same ease, which is what keeps them edge to edge.
        assert_eq!(clips[0].animation[1].ease, clips[1].animation[1].ease);
        assert_eq!(clips[1].animation[1].ease, EASE_IN_OUT);
    }

    #[test]
    fn a_push_over_a_clip_the_user_keyed_becomes_a_dissolve() {
        let mut clips = cut_with_handles();
        clips[0].animation.push(ExportKey {
            property: "offsetX".to_owned(),
            at: 0.5,
            value: 0.1,
            ease: linear_ease(),
        });
        clips[1].transition = spec("push", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY);
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
        let mut clips = cut_with_handles();
        clips[1].transition = spec("zoom", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY);
        assert_eq!(clips[1].video_fade_in, 1.0);
        assert_eq!(
            keys_on(&clips[1], "scale"),
            vec![(0.0, 1.25), (1.0 / 4.5, 1.0)]
        );
        assert_eq!(
            keys_on(&clips[0], "scale"),
            vec![(1.0 - 1.0 / 4.5, 1.0), (1.0, 1.4)]
        );
    }

    #[test]
    fn a_wipe_is_an_edge_the_plan_draws_until_the_overlap_ends() {
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("video", 0, 4.0, 4.0, 2.0),
        ];
        clips[1].transition = spec("wipe-right", 0.5);
        resolve_transitions(&mut clips, FrameRate::THIRTY);
        let b = &clips[1];
        assert_eq!((b.start, b.duration), (3.75, 4.25));
        assert_eq!(b.video_fade_in, 0.0, "the edge does the revealing");
        assert_eq!(
            b.transition_shapes,
            vec![TransitionShape::Wipe {
                from_right: false,
                frames: 15
            }]
        );
        assert!(
            clips[0].transition_shapes.is_empty(),
            "the outgoing picture is untouched"
        );
        // The edge stands at (n + 1) / 15 of the width, as the mask filter
        // it replaced did, and is gone once the overlap is over.
        let at = |frame| transitions_at(&clips[1].transition_shapes, frame);
        assert_eq!(
            at(0),
            vec![Transition::Wipe {
                uncovered: 1.0 / 15.0,
                from_right: false
            }]
        );
        assert_eq!(
            at(14),
            vec![Transition::Wipe {
                uncovered: 1.0,
                from_right: false
            }]
        );
        assert!(at(15).is_empty());

        // The other direction uncovers from the right edge.
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("video", 0, 4.0, 4.0, 2.0),
        ];
        clips[1].transition = spec("wipe-left", 0.5);
        resolve_transitions(&mut clips, FrameRate::THIRTY);
        assert_eq!(
            clips[1].transition_shapes,
            vec![TransitionShape::Wipe {
                from_right: true,
                frames: 15
            }]
        );
    }

    #[test]
    fn a_fade_to_black_splits_across_the_cut() {
        let mut clips = vec![
            clip("video", 0, 0.0, 4.0, 0.0),
            clip("video", 0, 4.0, 4.0, 0.0),
        ];
        clips[1].transition = spec("fade-black", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY);

        // Half a second each side at 30fps is 15 frames.
        let black = [0.0; 3];
        assert_eq!(
            clips[0].transition_shapes,
            vec![TransitionShape::FadeOut {
                colour: black,
                start: 105,
                frames: 15
            }]
        );
        assert_eq!(
            clips[1].transition_shapes,
            vec![TransitionShape::FadeIn {
                colour: black,
                frames: 15
            }]
        );
        assert_eq!(clips[1].start, 4.0, "nothing moves for an edge fade");

        // Weighed as FFmpeg's fade did: the picture whole at the fade's
        // first frame out, the colour whole at the first frame in.
        let fade = |shapes: &[TransitionShape], frame| match transitions_at(shapes, frame)[..] {
            [Transition::FadeTo { amount, .. }] => Some(amount),
            _ => None,
        };
        assert_eq!(fade(&clips[0].transition_shapes, 104), None);
        assert_eq!(fade(&clips[0].transition_shapes, 105), Some(0.0));
        assert_eq!(fade(&clips[0].transition_shapes, 119), Some(14.0 / 15.0));
        assert_eq!(fade(&clips[1].transition_shapes, 0), Some(1.0));
        let last = fade(&clips[1].transition_shapes, 14).expect("still fading");
        assert!((last - 1.0 / 15.0).abs() < 1e-6, "was {last}");
        assert_eq!(fade(&clips[1].transition_shapes, 15), None);
    }

    #[test]
    fn a_fade_to_white_names_its_colour() {
        let mut clips = vec![
            clip("video", 0, 0.0, 2.0, 0.0),
            clip("video", 0, 2.0, 2.0, 0.0),
        ];
        clips[1].transition = spec("fade-white", 0.5);
        resolve_transitions(&mut clips, FrameRate::THIRTY);
        assert!(matches!(
            clips[0].transition_shapes[..],
            [TransitionShape::FadeOut {
                colour: [1.0, 1.0, 1.0],
                ..
            }]
        ));
        assert!(matches!(
            clips[1].transition_shapes[..],
            [TransitionShape::FadeIn {
                colour: [1.0, 1.0, 1.0],
                ..
            }]
        ));
    }

    #[test]
    fn transition_fades_leave_the_clips_own_effects_alone() {
        let mut clips = vec![
            clip("video", 0, 0.0, 2.0, 0.0),
            clip("video", 0, 2.0, 2.0, 0.0),
        ];
        clips[1].effects = vec![AppliedFilter::new("concat.mono")];
        clips[1].transition = spec("fade-black", 0.5);
        resolve_transitions(&mut clips, FrameRate::THIRTY);
        // The fade is the plan's, drawn over the treated picture; the
        // clip's effects are as they were.
        assert_eq!(clips[1].effects, vec![AppliedFilter::new("concat.mono")]);
        assert_eq!(clips[1].transition_shapes.len(), 1);
    }

    /// The crop and flips are the frame plan's, drawn before the clip's
    /// effects run over the picture; a clip with neither plans none.
    #[test]
    fn geometry_goes_to_the_plan() {
        let mut clips = [
            clip("video", 0, 0.0, 2.0, 0.0),
            clip("video", 0, 2.0, 2.0, 0.0),
        ];
        clips[0].flip_h = true;
        clips[0].crop = Some([0.25, 0.0, 0.0, 0.5]);
        clips[1].flip_v = true;
        clips[1].effects = vec![AppliedFilter::new("concat.mono")];
        let planned = resolve::planned_geometry(&clips[0]).expect("planned");
        assert!(planned.flip_h && !planned.flip_v);
        assert_eq!(planned.crop, concat_render::Crop::of([0.25, 0.0, 0.0, 0.5]));
        let planned = resolve::planned_geometry(&clips[1]).expect("planned, effects or not");
        assert!(planned.flip_v);

        let untouched = clip("video", 0, 0.0, 2.0, 0.0);
        assert_eq!(resolve::planned_geometry(&untouched), None);
    }

    #[test]
    fn a_transition_with_no_adjacent_clip_is_orphaned_not_fatal() {
        let mut clips = vec![
            clip("video", 0, 0.0, 2.0, 0.0),
            clip("video", 0, 5.0, 2.0, 0.0),
        ];
        clips[1].transition = spec("cross-fade", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY);
        assert_eq!(clips[1].start, 5.0);
        assert_eq!(clips[1].video_fade_in, 0.0);
    }

    #[test]
    fn track_indices_are_doubled_for_everyone() {
        let mut clips = vec![
            clip("video", 0, 0.0, 2.0, 0.0),
            clip("audio", 3, 0.0, 2.0, 0.0),
        ];
        resolve_transitions(&mut clips, FrameRate::THIRTY);
        assert_eq!(clips[0].track, 0);
        assert_eq!(clips[1].track, 6);
    }

    /// No lane holds two pictures at one instant. Where two would, the
    /// engine draws only the one placed last (`clip_on_track_at`) and the
    /// other vanishes.
    fn assert_one_picture_per_lane(clips: &[ExportClip], case: &str) {
        for (n, one) in clips.iter().enumerate() {
            for other in &clips[n + 1..] {
                if one.track != other.track || !one.kind.is_visual() || !other.kind.is_visual() {
                    continue;
                }
                let shared = (one.start + one.duration).min(other.start + other.duration)
                    - one.start.max(other.start);
                assert!(
                    shared < 1e-9,
                    "{case}: lane {} holds two pictures at once for {shared} s",
                    one.track
                );
            }
        }
    }

    /// Two transitions in a row with a logo on the track above. The clip
    /// after the second cut was lifted one lane above a clip already lifted
    /// for the first, which is the logo's own lane, and one of the two
    /// vanished for the whole clip. Every clip of a chain now sits above the
    /// one it crosses from and beneath the track above.
    #[test]
    fn a_chain_of_transitions_stays_beneath_the_track_above() {
        for (first, second) in [
            ("concat.dissolve", "concat.whip-pan"),
            ("cross-fade", "cross-fade"),
            ("concat.zoom-blur-transition", "push"),
            ("wipe-left", "concat.dissolve"),
        ] {
            let case = format!("{first} then {second}");
            let mut clips = vec![
                clip("video", 0, 0.0, 4.0, 0.0),
                clip("video", 0, 4.0, 4.0, 2.0),
                clip("video", 0, 8.0, 4.0, 2.0),
                clip("image", 1, 0.0, 12.0, 0.0),
            ];
            for footage in &mut clips[..3] {
                footage.media_duration = Some(10.0);
            }
            clips[1].transition = spec(first, 1.0);
            clips[2].transition = spec(second, 1.0);
            let spans = resolve_transitions(&mut clips, FrameRate::THIRTY);

            let [a, b, c, logo] = [0, 1, 2, 3].map(|n| clips[n].track);
            assert!(
                a < b && b < c,
                "{case}: each clip sits above the one it crosses from: {a} {b} {c}"
            );
            assert!(
                c < logo,
                "{case}: the chain stays beneath the logo: {c} {logo}"
            );
            assert_one_picture_per_lane(&clips, &case);
            for span in &spans {
                assert!(span.to_track < logo, "{case}: a span on the logo's lane");
            }
        }
    }

    /// The cuts of a chain are resolved in time order, whatever order the
    /// clips are listed in. In list order, a third clip listed before the
    /// second was lifted before the second was, onto the lane the second
    /// then took.
    #[test]
    fn a_chain_resolves_the_same_whatever_order_its_clips_are_listed_in() {
        let chain = || {
            let mut clips = vec![
                clip("video", 0, 0.0, 4.0, 0.0),
                clip("video", 0, 4.0, 4.0, 2.0),
                clip("video", 0, 8.0, 4.0, 2.0),
            ];
            for footage in &mut clips {
                footage.media_duration = Some(10.0);
            }
            clips[1].transition = spec("concat.dissolve", 1.0);
            clips[2].transition = spec("concat.dissolve", 1.0);
            clips
        };
        let mut listed = chain();
        resolve_transitions(&mut listed, FrameRate::THIRTY);
        let mut backwards = chain();
        backwards.reverse();
        resolve_transitions(&mut backwards, FrameRate::THIRTY);
        assert_one_picture_per_lane(&backwards, "listed backwards");
        backwards.reverse();
        for (one, other) in listed.iter().zip(&backwards) {
            assert_eq!(
                (one.start, one.duration, one.track),
                (other.start, other.duration, other.track)
            );
        }
    }

    /// A long chain climbs a lane for every cut, and the tracks above are
    /// spread far enough apart to make room for it, in the order they were.
    #[test]
    fn the_lanes_spread_as_far_as_the_longest_chain_needs() {
        let mut clips: Vec<ExportClip> = (0..6)
            .map(|n| {
                let mut footage = clip("video", 0, 4.0 * n as f64, 4.0, 2.0);
                footage.media_duration = Some(10.0);
                if n > 0 {
                    footage.transition = spec("concat.dissolve", 1.0);
                }
                footage
            })
            .collect();
        clips.push(clip("image", 1, 0.0, 24.0, 0.0));
        clips.push(clip("image", 2, 0.0, 24.0, 0.0));
        resolve_transitions(&mut clips, FrameRate::THIRTY);

        let chain: Vec<usize> = clips[..6].iter().map(|footage| footage.track).collect();
        assert!(chain.windows(2).all(|pair| pair[0] < pair[1]), "{chain:?}");
        let (logo, title) = (clips[6].track, clips[7].track);
        assert!(chain[5] < logo && logo < title, "{chain:?} {logo} {title}");
        assert_one_picture_per_lane(&clips, "six clips, five cuts");
    }

    /// Transitions that never follow one another keep the lanes every
    /// project has had: doubled, each incoming clip on the odd lane above.
    #[test]
    fn single_transitions_keep_the_doubled_lanes() {
        let mut clips = cut_with_handles();
        clips[1].transition = spec("concat.dissolve", 1.0);
        clips.push(clip("image", 1, 0.0, 8.0, 0.0));
        resolve_transitions(&mut clips, FrameRate::THIRTY);
        assert_eq!([clips[0].track, clips[1].track, clips[2].track], [0, 1, 2]);
    }

    #[test]
    fn an_unknown_transition_kind_renders_as_a_plain_cut() {
        let mut clips = vec![
            clip("video", 0, 0.0, 2.0, 0.0),
            clip("video", 0, 2.0, 2.0, 0.0),
        ];
        // A kind no build knows - the wipes are known now.
        clips[1].transition = spec("spiral", 1.0);
        resolve_transitions(&mut clips, FrameRate::THIRTY);
        assert_eq!(clips[1].start, 2.0);
        assert!(clips[1].transition_shapes.is_empty());
    }

    /// The exported frame count is the timeline's own, not the f64 seconds'
    /// rounded whole. A clip's start and its duration are each rounded to the
    /// frame grid on the way into the timeline, and the sum rounded once
    /// disagrees a frame either side: half a frame under half a frame lost the
    /// last frame of the last clip - the engine drew to frame 301, the export
    /// stopped at 300 - and a pair that rounds down at both ends gained a
    /// black frame after everything ended.
    #[test]
    fn the_exported_frame_count_is_the_timelines_own() {
        let half = vec![clip("video", 0, 3.5 / 30.0, 296.5 / 30.0, 0.0)];
        let (end, frames) = timeline_extent(&half, FrameRate::THIRTY, 0.0);
        assert_eq!(end, Rational::new(301, 30));
        assert_eq!(frames, 301, "the last frame of the last clip is drawn");

        let down = vec![clip("video", 0, 3.4 / 30.0, 296.4 / 30.0, 0.0)];
        let (end, frames) = timeline_extent(&down, FrameRate::THIRTY, 0.0);
        assert_eq!(end, Rational::new(299, 30));
        assert_eq!(frames, 299, "no black frame past the end");
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Deciding what is on screen at a given instant, and describing it whole.
//!
//! A [`FramePlan`] is the contract between the editor model and whatever
//! draws pixels: the one description of a frame that both compositors
//! consume, and nothing else reaches them. It is built in two steps.
//!
//! [`plan_frame`] answers "what is on screen, from where, and how strongly":
//! for each visible layer, bottom-most first, the media, the exact source
//! timestamp, the placement, the opacity and the blend. It touches no
//! pixels and does no IO, so it is fast, exactly testable, and identical for
//! the CPU and GPU backends. The executor then fills in what the model has
//! no field for - the decoded picture, the crop, the flips, the effect chain
//! resolved for this frame, the mask, the transitions at the cut, the
//! treatments live over the stack - and hands the plan to a compositor.
//!
//! What a compositor does with one layer, in order: the crop, the flips and
//! the fit make the picture; the effects run over it; the mask, the fades
//! and the wipes weigh it; the placement puts it in the frame; the blend
//! meets what is beneath. The geometry and the weighing are worked out
//! here, once, so the two backends cannot drift on them.

use std::path::PathBuf;
use std::sync::Arc;

use concat_core::frame::{Frame, Signal};
use concat_core::shader::ShaderPass;
use concat_core::time::Rational;
use concat_core::timeline::{Blend, ClipId, Timeline, TrackKind, Transform};

/// Fractions cut off a source's left, top, right and bottom before it is
/// fitted, as the document stores them.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Crop {
    /// Cut off the left edge, `0..1` of the width.
    pub left: f32,
    /// Cut off the top edge, `0..1` of the height.
    pub top: f32,
    /// Cut off the right edge, `0..1` of the width.
    pub right: f32,
    /// Cut off the bottom edge, `0..1` of the height.
    pub bottom: f32,
}

impl Crop {
    /// The whole source.
    pub const NONE: Crop = Crop {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    /// The document's four fractions, in its order.
    pub fn of(edges: [f64; 4]) -> Crop {
        let [left, top, right, bottom] = edges.map(|edge| edge.clamp(0.0, 1.0) as f32);
        Crop {
            left,
            top,
            right,
            bottom,
        }
    }

    /// True when nothing is cut.
    pub fn is_none(&self) -> bool {
        self.left <= 0.0 && self.top <= 0.0 && self.right <= 0.0 && self.bottom <= 0.0
    }

    /// What is left of a `width` by `height` source, in its pixels: the
    /// origin and the size. At least a tenth each way, as the document's
    /// crop is bounded, and never past the source's edge.
    pub fn rect(&self, width: u32, height: u32) -> [f32; 4] {
        let (w, h) = (width as f32, height as f32);
        let x = (self.left.clamp(0.0, 1.0) * w).min(w - 1.0).max(0.0);
        let y = (self.top.clamp(0.0, 1.0) * h).min(h - 1.0).max(0.0);
        let cw = ((1.0 - self.left - self.right).max(0.1) * w)
            .max(1.0)
            .min(w - x);
        let ch = ((1.0 - self.top - self.bottom).max(0.1) * h)
            .max(1.0)
            .min(h - y);
        [x, y, cw, ch]
    }
}

/// What a cut does to a layer at one frame: the picture pulled towards a
/// colour, or uncovered behind a moving edge. Resolved by the executor
/// from the transition's shape and the frame's place in it; the compositor
/// only ever sees an amount.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Transition {
    /// The picture mixed towards `colour` by `amount`, `0..=1`: a fade to
    /// black or to white, at one frame of it.
    FadeTo {
        /// The colour faded to, `0..=1` a channel.
        colour: [f32; 3],
        /// How far towards it: one is the colour outright.
        amount: f32,
    },
    /// A straight vertical edge with the picture shown on one side and
    /// gone on the other. `uncovered` is the fraction of the picture's
    /// width shown: from the left edge, or from the right when `from_right`.
    Wipe {
        /// How much of the width is shown, `0..=1`.
        uncovered: f32,
        /// Whether the shown part is at the right edge rather than the left.
        from_right: bool,
    },
}

/// A layer's transitions folded into what they do to one pixel: the colour
/// scaled and offset, and the span of the width kept. Every fade composes
/// into one affine map of the colour, and every wipe into one edge a side,
/// so a compositor applies the lot in one line.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shading {
    /// What the colour is multiplied by.
    pub scale: f32,
    /// What is then added to it.
    pub offset: [f32; 3],
    /// A pixel is kept where its `u` is below this.
    pub left_edge: f32,
    /// A pixel is kept where its `u` is at or beyond this.
    pub right_edge: f32,
}

impl Shading {
    /// Nothing done: the colour as it is, the whole width kept.
    pub const NONE: Shading = Shading {
        scale: 1.0,
        offset: [0.0; 3],
        left_edge: 2.0,
        right_edge: -1.0,
    };

    /// The transitions of a layer, folded in order.
    pub fn of(transitions: &[Transition]) -> Shading {
        let mut shading = Shading::NONE;
        for transition in transitions {
            match *transition {
                Transition::FadeTo { colour, amount } => {
                    let amount = amount.clamp(0.0, 1.0);
                    shading.scale *= 1.0 - amount;
                    for (channel, target) in shading.offset.iter_mut().zip(colour) {
                        *channel = *channel * (1.0 - amount) + target * amount;
                    }
                }
                Transition::Wipe {
                    uncovered,
                    from_right,
                } => {
                    let uncovered = uncovered.clamp(0.0, 1.0);
                    if from_right {
                        shading.right_edge = shading.right_edge.max(1.0 - uncovered);
                    } else {
                        shading.left_edge = shading.left_edge.min(uncovered);
                    }
                }
            }
        }
        shading
    }

    /// Whether a pixel `u` of the way across the picture is shown at all.
    pub fn keeps(&self, u: f32) -> bool {
        u < self.left_edge && u >= self.right_edge
    }

    /// One channel of the colour, `0..=1`, through the fades.
    pub fn colour(&self, channel: usize, value: f32) -> f32 {
        value * self.scale + self.offset[channel]
    }
}

/// One visible layer at one instant.
#[derive(Clone, Debug)]
pub struct PlannedLayer {
    /// Which clip produced this layer.
    pub clip: ClipId,
    /// Where this clip begins on the timeline - not the source's own time,
    /// `source_time`, but where the clip itself sits. A shader effect reads
    /// the gap between this and the frame's own time as `frame.clip_time`,
    /// so a one-shot look can size itself to however long the clip has
    /// been on screen instead of the timeline's absolute clock.
    pub clip_start: Rational,
    /// The track the layer sits on, bottom-most zero: every layer on a
    /// lower track is under it, and a treatment names the track it sits
    /// above.
    pub track: usize,
    /// The file to pull pixels from.
    pub media: PathBuf,
    /// The timestamp *within that file* to pull.
    pub source_time: Rational,
    /// Source seconds consumed per timeline second. A decoder that pulls one
    /// frame per output frame must decode at `output_rate / speed` to stay in
    /// step with `source_time`.
    pub speed: Rational,
    /// Whether one decoder paced at `output rate / speed` follows this clip;
    /// false for a curve or a reverse, where each frame is sought.
    pub paced: bool,
    /// The decoded picture, once the executor has fetched it: the source at
    /// whatever size the decoder gave it. `None` until then, and a layer
    /// without one draws nothing rather than failing the frame.
    pub source: Option<Arc<Frame>>,
    /// What is cut off the source's edges before it is fitted. `NONE` for
    /// a picture the decoder already cropped.
    pub crop: Crop,
    /// Mirrored left to right, after the crop and before the effects.
    pub flip_h: bool,
    /// Mirrored top to bottom, the same.
    pub flip_v: bool,
    /// The clip's placement in the frame, resolution-independent, over the
    /// fitted picture.
    pub transform: Transform,
    /// The effects to run over the picture, in order, each resolved for
    /// this frame. The GPU runs the shader; the CPU runs its kernel for the
    /// package, or draws the picture untreated when it has none.
    pub effects: Vec<ShaderPass>,
    /// A matte over the picture, its alpha the coverage, sampled across the
    /// picture whatever its own size: what a cutout leaves. `None` keeps
    /// the picture whole.
    pub mask: Option<Arc<Frame>>,
    /// What the cuts at either end do to the picture at this frame.
    pub transitions: Vec<Transition>,
    /// How the layer's colour meets what is beneath it.
    pub blend: Blend,
    /// Blend strength over everything beneath, in `0.0..=1.0`.
    pub opacity: f32,
}

impl PlannedLayer {
    /// A layer of `frame` with nothing else set: opaque, unmoved, whole,
    /// untreated, on track zero. Where a test or a tool that already has
    /// its pixels starts from.
    pub fn picture(clip: ClipId, frame: Arc<Frame>) -> PlannedLayer {
        PlannedLayer {
            clip,
            clip_start: Rational::ZERO,
            track: 0,
            media: PathBuf::new(),
            source_time: Rational::ZERO,
            speed: Rational::ONE,
            paced: true,
            source: Some(frame),
            crop: Crop::NONE,
            flip_h: false,
            flip_v: false,
            transform: Transform::default(),
            effects: Vec::new(),
            mask: None,
            transitions: Vec::new(),
            blend: Blend::Normal,
            opacity: 1.0,
        }
    }

    /// How hard the layer is drawn: its opacity, held to `0..=1`, and
    /// nothing at all for a number that is not one - a NaN from a bad
    /// key is an invisible layer, not a frame of garbage.
    pub fn weight(&self) -> f32 {
        if self.opacity.is_finite() {
            self.opacity.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The transitions folded; see [`Shading`].
    pub fn shading(&self) -> Shading {
        Shading::of(&self.transitions)
    }

    /// Where the picture's texels come from and where they land, for an
    /// output `width` by `height`; see [`Geometry`].
    pub fn geometry(&self, source: &Frame, width: u32, height: u32) -> Geometry {
        Geometry::of(self, source.width(), source.height(), width, height)
    }

    /// Whether the picture has to be made before the effects can run: the
    /// effects read texels of the picture as it will be seen, so a crop, a
    /// flip or a size the fit changes cannot wait for the placement. With
    /// no effects, all three fold into the placement for nothing.
    pub fn needs_preparing(&self, geometry: &Geometry) -> bool {
        !self.effects.is_empty()
            && (!self.crop.is_none() || self.flip_h || self.flip_v || !geometry.fits_source())
    }
}

/// Where a layer's picture comes from and where it lands: the arithmetic
/// both compositors share, so a picture cannot be placed two ways.
///
/// The picture is the crop rectangle of the source, fitted inside the
/// output - contain, the aspect kept - and centred; that fitted size is
/// what the effects run at and what the transform then scales, turns and
/// moves about the centre. A decoder that already cropped and fitted the
/// source hands back a picture the fit leaves alone.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Geometry {
    /// The source pixels the picture is made of: x, y, width, height.
    pub source_rect: [f32; 4],
    /// The size of the source the rectangle is of.
    pub source_size: (u32, u32),
    /// The picture's size once fitted: the size the effects run at.
    pub fitted: (u32, u32),
    /// Where the picture's centre lands in the output, after translation.
    pub centre: (f32, f32),
    /// The transform's scale per axis over the fitted size: the scale
    /// times the stretch.
    pub scale: (f32, f32),
    /// Clockwise rotation about the centre, radians.
    pub rotation: f32,
}

impl Geometry {
    /// The geometry of `layer` over a `source_width` by `source_height`
    /// source in an output `width` by `height`.
    pub fn of(
        layer: &PlannedLayer,
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Geometry {
        let source_rect = layer.crop.rect(source_width, source_height);
        let [_, _, cw, ch] = source_rect;
        let fit = (width as f32 / cw).min(height as f32 / ch);
        let fitted = (
            ((cw * fit).round() as u32).max(1),
            ((ch * fit).round() as u32).max(1),
        );
        // Centred by whole pixels, the way a frame the decoder fitted has
        // always been placed, then moved by the transform's fractions.
        let base_x = (i64::from(width) - i64::from(fitted.0)) / 2;
        let base_y = (i64::from(height) - i64::from(fitted.1)) / 2;
        let transform = &layer.transform;
        let centre = (
            base_x as f32 + fitted.0 as f32 / 2.0 + (transform.offset_x * f64::from(width)) as f32,
            base_y as f32 + fitted.1 as f32 / 2.0 + (transform.offset_y * f64::from(height)) as f32,
        );
        let scale = (
            ((transform.scale * transform.stretch_x) as f32).max(1e-6),
            ((transform.scale * transform.stretch_y) as f32).max(1e-6),
        );
        Geometry {
            source_rect,
            source_size: (source_width, source_height),
            fitted,
            centre,
            scale,
            rotation: transform.rotation.to_radians() as f32,
        }
    }

    /// The geometry once the picture has been made at its fitted size:
    /// the whole of it, unflipped, the same placement.
    pub fn prepared(&self) -> Geometry {
        Geometry {
            source_rect: [0.0, 0.0, self.fitted.0 as f32, self.fitted.1 as f32],
            source_size: self.fitted,
            ..*self
        }
    }

    /// True when the fitted picture is the whole source at its own size.
    pub fn fits_source(&self) -> bool {
        let [x, y, w, h] = self.source_rect;
        x == 0.0
            && y == 0.0
            && w == self.source_size.0 as f32
            && h == self.source_size.1 as f32
            && self.fitted == self.source_size
    }

    /// True when the placement is the fitted picture, unscaled, unturned,
    /// and moved by whole pixels: the case a compositor can copy rather
    /// than resample.
    pub fn is_aligned(&self) -> bool {
        self.scale == (1.0, 1.0)
            && self.rotation == 0.0
            && (self.centre.0 - self.fitted.0 as f32 / 2.0).fract() == 0.0
            && (self.centre.1 - self.fitted.1 as f32 / 2.0).fract() == 0.0
    }

    /// The top-left corner of an aligned placement, in output pixels.
    pub fn corner(&self) -> (i32, i32) {
        (
            (self.centre.0 - self.fitted.0 as f32 / 2.0).round() as i32,
            (self.centre.1 - self.fitted.1 as f32 / 2.0).round() as i32,
        )
    }

    /// The source pixel under a point of the picture, `u` and `v` in
    /// `0..1` across the picture as it is seen, through `flip_h` and
    /// `flip_v`: the texel coordinate a sampler reads at, its centre at
    /// `.0`, like the GPU's.
    pub fn source_of(&self, u: f32, v: f32, flip_h: bool, flip_v: bool) -> (f32, f32) {
        let u = if flip_h { 1.0 - u } else { u };
        let v = if flip_v { 1.0 - v } else { v };
        let [x, y, w, h] = self.source_rect;
        (x + u * w - 0.5, y + v * h - 0.5)
    }

    /// The same, as texture coordinates in `0..1` of the source.
    pub fn uv_of(&self, u: f32, v: f32, flip_h: bool, flip_v: bool) -> (f32, f32) {
        let u = if flip_h { 1.0 - u } else { u };
        let v = if flip_v { 1.0 - v } else { v };
        let [x, y, w, h] = self.source_rect;
        (
            (x + u * w) / self.source_size.0 as f32,
            (y + v * h) / self.source_size.1 as f32,
        )
    }
}

/// A layer clip live at this frame: its effects run over everything
/// composited beneath its track, and the result is blended back over the
/// untreated stack by its strength.
#[derive(Clone, Debug)]
pub struct PlannedTreatment {
    /// The track the layer sits on; every layer on a lower track is under it.
    pub track: usize,
    /// The effects to run over the stack beneath, in order, resolved for
    /// this frame.
    pub effects: Vec<ShaderPass>,
    /// How much of the treated stack to keep over the untreated, `0..=1`.
    pub strength: f32,
}

/// Everything needed to draw one output frame.
#[derive(Clone, Debug)]
pub struct FramePlan {
    /// The timeline instant this describes.
    pub time: Rational,
    /// Output width.
    pub width: u32,
    /// Output height.
    pub height: u32,
    /// Visible layers, bottom-most first.
    pub layers: Vec<PlannedLayer>,
    /// The treatments live at this instant, in ascending track order.
    pub treatments: Vec<PlannedTreatment>,
    /// What the timeline is output in: SDR, or HDR in HLG or PQ. On an HDR
    /// timeline an HDR clip keeps its light above white as it uploads, and
    /// the resolve rolls the frame off for an SDR screen.
    pub output: Signal,
}

impl FramePlan {
    /// A frame with nothing on it: black.
    pub fn empty(width: u32, height: u32) -> FramePlan {
        FramePlan {
            time: Rational::ZERO,
            width,
            height,
            layers: Vec::new(),
            treatments: Vec::new(),
            output: Signal::Sdr,
        }
    }

    /// True if nothing is on screen. The result is a black frame, not an error:
    /// gaps in a timeline are ordinary.
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// The instant in seconds, for effects that move.
    pub fn seconds(&self) -> f32 {
        self.time.as_f64() as f32
    }
}

/// A clip id for a layer that belongs to no timeline: a picture handed
/// in directly, a trial run of a package, a test. Minted from a timeline
/// of its own, since an id is an arena's.
pub fn detached_clip() -> ClipId {
    use concat_core::time::FrameRate;
    use concat_core::timeline::{Clip, MediaRef, Track};
    let mut timeline = Timeline::new(16, 16, FrameRate::THIRTY);
    let track = timeline.add_track(Track::new("detached", TrackKind::Video));
    timeline
        .add_clip(
            track,
            Clip::new(MediaRef::new("detached"), Rational::ZERO, Rational::ONE),
        )
        .expect("the track was just added")
}

/// Works out what is on screen at `time`.
///
/// Audio tracks and disabled tracks are skipped. A track with no clip under the
/// playhead simply contributes nothing. Every layer comes back with no
/// picture, no effects and no mask: those are the executor's to fill in.
pub fn plan_frame(timeline: &Timeline, time: Rational) -> FramePlan {
    let mut layers = Vec::new();

    for (track, (track_id, lane)) in timeline.tracks().enumerate() {
        if !lane.enabled || lane.kind != TrackKind::Video {
            continue;
        }
        let Some(clip_id) = timeline.clip_on_track_at(track_id, time) else {
            continue;
        };
        let Some(clip) = timeline.clip(clip_id) else {
            continue;
        };
        let Some(source_time) = clip.source_time_at(time) else {
            continue;
        };

        layers.push(PlannedLayer {
            clip: clip_id,
            clip_start: clip.start,
            track,
            media: clip.media.path.clone(),
            source_time,
            speed: clip.speed,
            paced: clip.is_paced(),
            source: None,
            crop: Crop::NONE,
            flip_h: false,
            flip_v: false,
            transform: clip.transform_at(time),
            effects: Vec::new(),
            mask: None,
            transitions: Vec::new(),
            blend: clip.blend,
            // The fade ramp multiplies in here, so the compositor only ever
            // sees a per-frame opacity - it has no idea fades exist.
            opacity: (clip.opacity_at(time) * clip.video_fade_factor(time)).clamp(0.0, 1.0),
        });
    }

    FramePlan {
        time,
        width: timeline.width,
        height: timeline.height,
        layers,
        treatments: Vec::new(),
        output: Signal::Sdr,
    }
}

#[cfg(test)]
mod tests {
    use concat_core::time::FrameRate;
    use concat_core::timeline::{Clip, MediaRef, Track};

    use super::*;

    fn seconds(value: i64) -> Rational {
        Rational::from_int(value)
    }

    #[test]
    fn stacks_layers_bottom_most_first() {
        let mut timeline = Timeline::new(640, 360, FrameRate::THIRTY);
        let lower = timeline.add_track(Track::new("V1", TrackKind::Video));
        let upper = timeline.add_track(Track::new("V2", TrackKind::Video));
        timeline
            .add_clip(
                lower,
                Clip::new(MediaRef::new("bottom.mp4"), seconds(0), seconds(10)),
            )
            .expect("track exists");
        timeline
            .add_clip(
                upper,
                Clip::new(MediaRef::new("top.mp4"), seconds(0), seconds(10)),
            )
            .expect("track exists");

        let plan = plan_frame(&timeline, seconds(4));
        let media: Vec<_> = plan
            .layers
            .iter()
            .map(|layer| layer.media.clone())
            .collect();
        assert_eq!(
            media,
            vec![PathBuf::from("bottom.mp4"), PathBuf::from("top.mp4")]
        );
        assert_eq!((plan.width, plan.height), (640, 360));
        assert_eq!(plan.layers[0].track, 0);
        assert_eq!(plan.layers[1].track, 1);
    }

    #[test]
    fn maps_each_layer_to_its_own_source_time() {
        let mut timeline = Timeline::new(640, 360, FrameRate::THIRTY);
        let track = timeline.add_track(Track::new("V1", TrackKind::Video));
        let mut clip = Clip::new(MediaRef::new("a.mp4"), seconds(5), seconds(10));
        clip.source_start = seconds(30);
        timeline.add_clip(track, clip).expect("track exists");

        let plan = plan_frame(&timeline, seconds(7));
        assert_eq!(
            plan.layers[0].source_time,
            seconds(32),
            "2s into a clip that starts at 30s"
        );
    }

    #[test]
    fn a_retimed_clip_plans_scaled_source_times() {
        let mut timeline = Timeline::new(640, 360, FrameRate::THIRTY);
        let track = timeline.add_track(Track::new("V1", TrackKind::Video));
        let mut clip = Clip::new(MediaRef::new("a.mp4"), seconds(0), seconds(4));
        clip.speed = Rational::from_int(2);
        timeline.add_clip(track, clip).expect("track exists");

        let plan = plan_frame(&timeline, seconds(3));
        assert_eq!(
            plan.layers[0].source_time,
            seconds(6),
            "3s in at 2x is 6s of source"
        );
        assert_eq!(plan.layers[0].speed, Rational::from_int(2));
    }

    #[test]
    fn a_gap_plans_to_nothing() {
        let mut timeline = Timeline::new(640, 360, FrameRate::THIRTY);
        let track = timeline.add_track(Track::new("V1", TrackKind::Video));
        timeline
            .add_clip(
                track,
                Clip::new(MediaRef::new("a.mp4"), seconds(0), seconds(2)),
            )
            .expect("track exists");

        assert!(plan_frame(&timeline, seconds(5)).is_empty());
    }

    #[test]
    fn skips_disabled_and_audio_tracks() {
        let mut timeline = Timeline::new(640, 360, FrameRate::THIRTY);
        let muted = timeline.add_track(Track::new("V1", TrackKind::Video));
        let audio = timeline.add_track(Track::new("A1", TrackKind::Audio));
        timeline.track_mut(muted).expect("track exists").enabled = false;

        for track in [muted, audio] {
            timeline
                .add_clip(
                    track,
                    Clip::new(MediaRef::new("a.mp4"), seconds(0), seconds(10)),
                )
                .expect("track exists");
        }

        assert!(plan_frame(&timeline, seconds(1)).is_empty());
    }

    #[test]
    fn a_video_fade_ramps_the_planned_opacity() {
        let mut timeline = Timeline::new(640, 360, FrameRate::THIRTY);
        let track = timeline.add_track(Track::new("V1", TrackKind::Video));
        let mut clip = Clip::new(MediaRef::new("a.mp4"), seconds(0), seconds(4));
        clip.video_fade_in = Rational::from_int(2);
        timeline.add_clip(track, clip).expect("track exists");

        assert_eq!(plan_frame(&timeline, seconds(0)).layers[0].opacity, 0.0);
        assert_eq!(plan_frame(&timeline, seconds(1)).layers[0].opacity, 0.5);
        assert_eq!(plan_frame(&timeline, seconds(3)).layers[0].opacity, 1.0);
    }

    #[test]
    fn clamps_a_nonsense_opacity() {
        let mut timeline = Timeline::new(640, 360, FrameRate::THIRTY);
        let track = timeline.add_track(Track::new("V1", TrackKind::Video));
        let mut clip = Clip::new(MediaRef::new("a.mp4"), seconds(0), seconds(10));
        clip.opacity = 4.0;
        timeline.add_clip(track, clip).expect("track exists");

        assert_eq!(plan_frame(&timeline, seconds(1)).layers[0].opacity, 1.0);
    }

    fn a_clip() -> ClipId {
        let mut timeline = Timeline::new(8, 8, FrameRate::THIRTY);
        let track = timeline.add_track(Track::new("V1", TrackKind::Video));
        timeline
            .add_clip(
                track,
                Clip::new(MediaRef::new("a.mp4"), seconds(0), seconds(1)),
            )
            .expect("track exists")
    }

    /// A crop's rectangle is the source's pixels, never past its edge; a
    /// fitted picture keeps its aspect and sits in the middle by whole
    /// pixels; a picture the decoder fitted already is left as it is.
    #[test]
    fn the_geometry_fits_the_crop_and_centres_it() {
        let mut layer = PlannedLayer::picture(a_clip(), Arc::new(Frame::black(100, 50)));
        let whole = Geometry::of(&layer, 100, 50, 100, 50);
        assert!(whole.fits_source());
        assert!(whole.is_aligned());
        assert_eq!(whole.corner(), (0, 0));

        let fitted = Geometry::of(&layer, 100, 50, 200, 200);
        assert_eq!(fitted.fitted, (200, 100));
        assert_eq!(fitted.centre, (100.0, 100.0));
        assert_eq!(fitted.corner(), (0, 50));
        assert!(!fitted.fits_source());

        layer.crop = Crop::of([0.25, 0.0, 0.25, 0.0]);
        let cropped = Geometry::of(&layer, 100, 50, 100, 50);
        assert_eq!(cropped.source_rect, [25.0, 0.0, 50.0, 50.0]);
        assert_eq!(cropped.fitted, (50, 50));
        assert_eq!(cropped.corner(), (25, 0));
        // The first texel of the crop is a sampler's half-texel in.
        assert_eq!(cropped.source_of(0.0, 0.0, false, false), (24.5, -0.5));
        assert_eq!(cropped.source_of(1.0, 0.0, true, false), (24.5, -0.5));
        assert_eq!(cropped.uv_of(1.0, 1.0, false, false), (0.75, 1.0));
    }

    /// Fades compose into one map of the colour and wipes into one edge a
    /// side, so two fades to black at half are three quarters black and a
    /// wipe keeps only what its edge has uncovered.
    #[test]
    fn transitions_fold_into_one_shading() {
        let shading = Shading::of(&[
            Transition::FadeTo {
                colour: [0.0; 3],
                amount: 0.5,
            },
            Transition::FadeTo {
                colour: [1.0; 3],
                amount: 0.5,
            },
        ]);
        assert!((shading.colour(0, 1.0) - 0.75).abs() < 1e-6);
        assert!((shading.colour(0, 0.0) - 0.5).abs() < 1e-6);
        assert!(shading.keeps(0.0) && shading.keeps(0.999));

        let wipe = Shading::of(&[Transition::Wipe {
            uncovered: 0.25,
            from_right: true,
        }]);
        assert!(!wipe.keeps(0.5));
        assert!(wipe.keeps(0.8));
        let wipe = Shading::of(&[Transition::Wipe {
            uncovered: 0.25,
            from_right: false,
        }]);
        assert!(wipe.keeps(0.1));
        assert!(!wipe.keeps(0.5));
        assert_eq!(Shading::of(&[]), Shading::NONE);
    }
}

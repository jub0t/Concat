// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The flattened clip list becoming the engine's timeline.
//!
//! `flatten` turns the document into `ExportClip`s; this turns those into
//! a `concat_core::Timeline` plus the per-clip facts the decoders and the
//! compositor need that the engine's model has no field for. It is the
//! one place a document value stops being approximate: every time is
//! quantised to the frame grid here, and every chain, pass, size and mask
//! a clip renders with is decided here. `render` and the preview read
//! what this builds and never look at an `ExportClip` again.

use super::*;

/// The tint a highlighted cutout wears: the interface's accent, the same
/// lime the brushes and the selection are drawn in.
pub(crate) const HIGHLIGHT: [u8; 3] = [0xcb, 0xf5, 0x3f];

/// A cutout as the frame loop runs it: the masks, what to paint on them,
/// and how a decoded pixel finds its place in the source.
pub(crate) struct CutoutJob {
    pub(crate) store: MaskStore,
    pub(crate) cutout: Cutout,
    pub(crate) mapping: Mapping,
    /// The source's width over its height, for round brushes.
    pub(crate) aspect: f32,
}

impl CutoutJob {
    /// The job for a clip, or `None` when it has no cutout or no masks to
    /// cut with.
    pub(crate) fn of(clip: &ExportClip) -> Option<CutoutJob> {
        let cutout = clip.cutout.clone()?;
        if clip.mask_dir.is_empty() {
            return None;
        }
        let aspect = match (clip.media_width, clip.media_height) {
            (Some(width), Some(height)) if width > 0 && height > 0 => width as f32 / height as f32,
            _ => 1.0,
        };
        Some(CutoutJob {
            store: MaskStore::open(Path::new(&clip.mask_dir)),
            cutout,
            mapping: Mapping {
                crop: clip
                    .crop
                    .map(|edges| edges.map(|edge| edge as f32))
                    .unwrap_or([0.0; 4]),
                flip_h: clip.flip_h,
                flip_v: clip.flip_v,
            },
            aspect,
        })
    }

    /// The frame with its background gone, when the instant has a mask.
    /// `None` leaves the picture whole: an instant not analysed yet is
    /// shown as shot rather than not at all.
    pub(crate) fn cut(&self, frame: &Frame, source_time: Rational) -> Option<Frame> {
        let mask = self
            .store
            .resolved(source_time.as_f64(), &self.cutout, self.aspect)?;
        let mut out = frame.clone();
        concat_vision::cut(&mut out, &mask, &self.mapping);
        Some(out)
    }

    /// The frame whole, with what the cutout keeps tinted over it: the
    /// painting view. `None` as for `cut`.
    pub(crate) fn highlight(&self, frame: &Frame, source_time: Rational) -> Option<Frame> {
        let mask = self
            .store
            .resolved(source_time.as_f64(), &self.cutout, self.aspect)?;
        let mut out = frame.clone();
        concat_vision::highlight(&mut out, &mask, &self.mapping, HIGHLIGHT);
        Some(out)
    }
}

/// An engine timeline plus the per-clip facts the decoders need that the
/// engine's model has no field for.
pub(crate) struct BuiltTimeline {
    pub(crate) timeline: Timeline,
    /// Clips that are stills: one-frame streams, decoded looping.
    pub(crate) stills: std::collections::HashSet<ClipId>,
    /// Contain-fitted decode size per clip, where the source's size is known.
    pub(crate) decode_sizes: HashMap<ClipId, (u32, u32)>,
    /// The clip's effect chain, where it has one.
    pub(crate) filter_chains: HashMap<ClipId, String>,
    /// Each picture's track, so a treatment knows what lies beneath it.
    pub(crate) tracks: HashMap<ClipId, usize>,
    /// The clip's pre-fit chain - its crop - where it has one.
    pub(crate) pre_chains: HashMap<ClipId, String>,
    /// The levels the clip's file is read as, where the person has said.
    pub(crate) ranges: HashMap<ClipId, concat_media::ColorRange>,
    /// The clip's applied effects, on a GPU renderer: the passes are
    /// resolved from them at each frame, because a knob with keys is worth
    /// something different each frame and the resolution is cheap.
    pub(crate) chains: HashMap<ClipId, Vec<AppliedFilter>>,
    /// A title's per-word reveal order, for the clips that have one -
    /// carried beside `chains` rather than inside it, since it is baked
    /// once by the host and never resolved per frame the way effects are.
    pub(crate) reveal_maps: HashMap<ClipId, Arc<RevealMap>>,
    /// The clip whose cutout is drawn tinted rather than cut, if one is.
    pub(crate) highlight: Option<ClipId>,
    /// The layers: treatments over the stack, by span.
    pub(crate) treatments: Vec<Treatment>,
    /// The packaged transitions over cuts, resolved before the timeline.
    pub(crate) transitions: Vec<TransitionSpan>,
    /// The clips whose background a mask takes away.
    pub(crate) cutouts: HashMap<ClipId, CutoutJob>,
}

/// A packaged transition over a cut. The incoming clip has been overlapped
/// onto the outgoing one and moved to `to_track`; over `[start, end)` the
/// compositor combines the stack below `to_track` (the outgoing picture) with
/// the incoming layer through the package's two-input shader. A machine with
/// no GPU shows the dissolve the incoming clip already carries instead.
#[derive(Clone, Debug)]
pub(crate) struct TransitionSpan {
    pub(crate) start: Rational,
    pub(crate) end: Rational,
    pub(crate) to_track: usize,
    pub(crate) id: String,
    pub(crate) params: BTreeMap<String, f64>,
}

impl TransitionSpan {
    pub(crate) fn covers(&self, time: Rational) -> bool {
        self.start <= time && time < self.end
    }

    /// How far through the cut `time` is, `0..=1`.
    pub(crate) fn progress(&self, time: Rational) -> f64 {
        let span = self.end.as_f64() - self.start.as_f64();
        if span <= 0.0 {
            return 1.0;
        }
        ((time.as_f64() - self.start.as_f64()) / span).clamp(0.0, 1.0)
    }
}

/// A layer clip, as the compositor needs it: when, over which tracks, what
/// chain, and how hard.
#[derive(Clone, Debug)]
pub(crate) struct Treatment {
    pub(crate) start: Rational,
    pub(crate) end: Rational,
    pub(crate) track: usize,
    pub(crate) chain: String,
    /// The layer's applied effects, when the renderer runs shaders; the
    /// chain is then whatever the GPU cannot. Resolved to passes at each
    /// frame, so a keyed knob rides.
    pub(crate) effects: Vec<AppliedFilter>,
    pub(crate) strength: f32,
    pub(crate) ramp_in: f64,
    pub(crate) ramp_out: f64,
}

impl Treatment {
    pub(crate) fn covers(&self, time: Rational) -> bool {
        self.start <= time && time < self.end
    }

    /// The shader passes at `time`, each keyed knob at its value there. A
    /// layer is never a title, so it never carries a reveal map.
    pub(crate) fn passes_at(&self, time: Rational) -> Vec<ShaderPass> {
        let span = (self.end - self.start).as_f64();
        let at = if span > 0.0 {
            ((time - self.start).as_f64() / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Catalogue::builtin().shader_passes_at(&self.effects, at, None)
    }

    /// How hard the treatment is applied at `time`: the strength, eased in
    /// and out over the ramps at either end.
    pub(crate) fn strength_at(&self, time: Rational) -> f32 {
        let at = time.as_f64() - self.start.as_f64();
        let left = self.end.as_f64() - time.as_f64();
        let mut ramp = 1.0_f64;
        if self.ramp_in > 0.0 && at < self.ramp_in {
            ramp = ramp.min(at / self.ramp_in);
        }
        if self.ramp_out > 0.0 && left < self.ramp_out {
            ramp = ramp.min(left / self.ramp_out);
        }
        (f64::from(self.strength) * ramp.clamp(0.0, 1.0)) as f32
    }
}

/// Converts the flattened clip list into an engine timeline.
pub(crate) fn build_timeline(
    request: &ExportRequest,
    rate: FrameRate,
    visible: &[&ExportClip],
    gpu: bool,
    transitions: Vec<TransitionSpan>,
) -> BuiltTimeline {
    let mut timeline = Timeline::new(request.width, request.height, rate);
    let mut stills = std::collections::HashSet::new();
    let mut decode_sizes: HashMap<ClipId, (u32, u32)> = HashMap::new();
    let mut filter_chains: HashMap<ClipId, String> = HashMap::new();
    let mut tracks_of: HashMap<ClipId, usize> = HashMap::new();
    let mut treatments: Vec<Treatment> = Vec::new();
    let mut pre_chains: HashMap<ClipId, String> = HashMap::new();
    let mut ranges: HashMap<ClipId, concat_media::ColorRange> = HashMap::new();
    let mut chains: HashMap<ClipId, Vec<AppliedFilter>> = HashMap::new();
    let mut reveal_maps: HashMap<ClipId, Arc<RevealMap>> = HashMap::new();
    let mut cutouts: HashMap<ClipId, CutoutJob> = HashMap::new();
    let mut highlight: Option<ClipId> = None;

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

        // A layer has no pixels to decode: it is a treatment over the
        // stack, kept beside the timeline rather than in it.
        if clip.kind == ClipKind::Layer {
            let chain = full_chain(clip, gpu);
            let effects = if gpu {
                shaded(&clip.effects)
            } else {
                Vec::new()
            };
            if !chain.is_empty() || !effects.is_empty() {
                treatments.push(Treatment {
                    start,
                    end: start + duration,
                    track: clip.track,
                    chain,
                    effects,
                    strength: clip.opacity.clamp(0.0, 1.0) as f32,
                    ramp_in: clip.fade_in.max(0.0),
                    ramp_out: clip.fade_out.max(0.0),
                });
            }
            continue;
        }

        let mut engine_clip = Clip::new(MediaRef::new(&clip.path), start, duration);
        engine_clip.source_start = quantise(clip.source_start, rate);
        // The same clamp the audio path applies, so a 2x clip means the same
        // thing to picture and sound. A still has no meaningful rate.
        if clip.kind != ClipKind::Image {
            engine_clip.speed =
                Rational::approximate(audio::clamp_speed(clip.speed)).unwrap_or(Rational::ONE);
            engine_clip.retime = SpeedCurve::new(&clip.speed_curve);
        }
        engine_clip.animation = animation_of(&clip.animation);
        engine_clip.blend = concat_core::timeline::Blend::parse(&clip.blend);
        engine_clip.transform = Transform {
            scale: clip.scale,
            offset_x: clip.offset_x,
            offset_y: clip.offset_y,
            rotation: clip.rotation,
            stretch_x: clip.stretch_x,
            stretch_y: clip.stretch_y,
        };
        engine_clip.opacity = clip.opacity.clamp(0.0, 1.0) as f32;
        // Quantised like every other time: the ramp must land on the same
        // frame grid the overlap does, or the dissolve ends a frame early.
        engine_clip.video_fade_in = quantise(clip.video_fade_in, rate);

        if let Some(id) = timeline.add_clip(tracks[clip.track], engine_clip) {
            tracks_of.insert(id, clip.track);
            if clip.kind == ClipKind::Image {
                stills.insert(id);
            }
            if let Some(size) = fitted_size(request, clip) {
                decode_sizes.insert(id, size);
            }
            let chain = full_chain(clip, gpu);
            if !chain.is_empty() {
                filter_chains.insert(id, chain);
            }
            let pre = pre_chain(clip);
            if !pre.is_empty() {
                pre_chains.insert(id, pre);
            }
            if let Some(range) = clip.color_range {
                ranges.insert(id, crate::engine_range(range));
            }
            if gpu {
                let effects = shaded(&clip.effects);
                if !effects.is_empty() {
                    chains.insert(id, effects);
                }
                if let Some(map) = &clip.reveal_map {
                    reveal_maps.insert(id, Arc::clone(map));
                }
            }
            if let Some(job) = CutoutJob::of(clip) {
                cutouts.insert(id, job);
            }
            if clip.highlighted {
                highlight = Some(id);
            }
        }
    }

    BuiltTimeline {
        timeline,
        stills,
        decode_sizes,
        filter_chains,
        tracks: tracks_of,
        treatments,
        transitions,
        pre_chains,
        ranges,
        chains,
        reveal_maps,
        cutouts,
        highlight,
    }
}

/// The chain that runs in the source's own pixels before the fit: the crop.
pub(crate) fn pre_chain(clip: &ExportClip) -> String {
    match clip.crop {
        Some([left, top, right, bottom])
            if left > 0.0 || top > 0.0 || right > 0.0 || bottom > 0.0 =>
        {
            let w = (1.0 - left - right).max(0.1);
            let h = (1.0 - top - bottom).max(0.1);
            // Even sizes, for the same reason `fitted_size` wants them.
            format!(
                "crop=w=floor(iw*{w:.4}/2)*2:h=floor(ih*{h:.4}/2)*2:x=floor(iw*{left:.4}):y=floor(ih*{top:.4})"
            )
        }
        _ => String::new(),
    }
}

/// The clip's FFmpeg chain for one backend: flips first - a flip is a
/// treatment of the picture like any other, and comes first so the effects
/// see the picture the viewer will - then the effects this backend runs as
/// chains, then the transition fades. On the GPU every effect with a shader
/// is left out here and carried by [`shader_passes`] instead.
pub(crate) fn full_chain(clip: &ExportClip, gpu: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    if clip.flip_h {
        parts.push("hflip".to_owned());
    }
    if clip.flip_v {
        parts.push("vflip".to_owned());
    }
    let effects = if clip.effects.is_empty() {
        clip.video_filter_chain.clone()
    } else if gpu {
        Catalogue::builtin().video_chain_gpu(&clip.effects)
    } else {
        Catalogue::builtin().video_chain(&clip.effects)
    };
    if !effects.is_empty() {
        parts.push(effects);
    }
    if !clip.transition_chain.is_empty() {
        parts.push(clip.transition_chain.clone());
    }
    parts.join(",")
}

/// The enabled entries of a chain whose package has a shader: the ones a
/// renderer that runs shaders resolves to passes each frame.
pub(crate) fn shaded(effects: &[AppliedFilter]) -> Vec<AppliedFilter> {
    let catalogue = Catalogue::builtin();
    effects
        .iter()
        .filter(|applied| {
            applied.enabled
                && catalogue
                    .get(&applied.id)
                    .is_some_and(|package| package.shader().is_some())
        })
        .cloned()
        .collect()
}

/// The engine's keys for a flattened clip's animation, or None for none.
pub(crate) fn animation_of(keys: &[ExportKey]) -> Option<Animation> {
    use concat_core::animate::{Ease, Key, Track};
    if keys.is_empty() {
        return None;
    }
    let mut tracks: [Vec<Key>; 6] = Default::default();
    for key in keys {
        let slot = match key.property.as_str() {
            "scale" => 0,
            "offsetX" => 1,
            "offsetY" => 2,
            "rotation" => 3,
            "opacity" => 4,
            "volume" => 5,
            _ => continue,
        };
        let [x1, y1, x2, y2] = key.ease;
        tracks[slot].push(Key {
            at: key.at,
            value: key.value,
            ease: Ease::new(x1, y1, x2, y2),
        });
    }
    let [scale, x, y, rotation, opacity, volume] = tracks;
    let animation = Animation {
        scale: Track::new(scale),
        offset_x: Track::new(x),
        offset_y: Track::new(y),
        rotation: Track::new(rotation),
        opacity: Track::new(opacity),
        volume: Track::new(volume),
    };
    (!animation.is_empty()).then_some(animation)
}

/// The source's contain-fitted size inside the output frame, or `None` when
/// the UI never learnt the source's dimensions.
pub(crate) fn fitted_size(request: &ExportRequest, clip: &ExportClip) -> Option<(u32, u32)> {
    let media_width = clip.media_width.filter(|value| *value > 0)?;
    let media_height = clip.media_height.filter(|value| *value > 0)?;
    // What is left after the crop is what gets fitted.
    let (media_width, media_height) = match clip.crop {
        Some([left, top, right, bottom]) => (
            (f64::from(media_width) * (1.0 - left - right).max(0.1))
                .round()
                .max(2.0) as u32,
            (f64::from(media_height) * (1.0 - top - bottom).max(0.1))
                .round()
                .max(2.0) as u32,
        ),
        None => (media_width, media_height),
    };

    let fit = (f64::from(request.width) / f64::from(media_width))
        .min(f64::from(request.height) / f64::from(media_height));
    let width = ((f64::from(media_width) * fit).round() as u32).max(2);
    let height = ((f64::from(media_height) * fit).round() as u32).max(2);
    // Even, because a decoder asked for an odd width may round it itself and
    // then every frame read is misaligned by a pixel's worth of bytes.
    Some((width & !1, height & !1))
}

pub(crate) fn quantise(seconds: f64, rate: FrameRate) -> Rational {
    rate.time_of_frame((seconds * rate.fps().as_f64()).round().max(0.0) as i64)
}

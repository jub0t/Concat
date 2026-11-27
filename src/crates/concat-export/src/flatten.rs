// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A document's sequence, as the renderer draws it: the one translator
//! from the document to [`ExportClip`]s.
//!
//! The document says time in ticks, keys at ticks of an item's content
//! time, speed as a time map from content to source, effects with ids,
//! masks with animated numbers. The renderer takes seconds, keys at a
//! share of the clip, a speed curve by share, effects as package links,
//! and masks as the engine's riding masks. Everything is said once here,
//! so what is saved and what is drawn cannot drift apart.
//!
//! Titles and shapes are not flattened here: they are drawn to pictures
//! by the host and join the list as stills (`concat-host`'s titles).
//! Nested sequences are not drawn yet.

use std::path::Path;

use concat_core::animate::{Ease as AnimEase, Key as AnimKey, MaskTracks, RidingMask, Track};
use concat_core::mask::{MaskMode as EngineMode, MaskShape as EngineShape, ShapeMask, StackedMask};
use concat_core::motion::{Cycle, Looping, Motion as EngineMotion, Move as EngineMove, Moving};
use concat_doc::model::{
    Animated, Background, Blend, Content, Ease, Effect, Entry, Item, ItemId, LaneId, Mask,
    MaskMode, MaskShape, MaskSource, Media, MediaKind, Model, Motion, SequenceId, Take, TimeMap,
};
use concat_doc::time::{Tick, TickDur, TickLen};
use concat_effects::{AppliedFilter, ParamKey};

use crate::chains::audio_filter_chain;
use crate::{ClipKind, ExportBackdrop, ExportClip, ExportKey, TransitionSpec};

/// How many straight pieces a smooth stretch of a speed curve is drawn
/// with: the speed on one is a parabola, and eight chords of it are within
/// a hundredth of the speed on every curve the commands make.
const PIECES: u32 = 8;

/// Every item of `sequence` the renderer draws, in lane order from the
/// bottom, as [`ExportClip`]s. Titles, shapes and nested sequences are
/// left out; an item whose file or lane has gone is skipped.
///
/// With the project folder, a clip with a cutout is told where its file's
/// masks are, a stabilized one where its file's movement is, and one whose
/// voice is set apart where the parts are; without it all three are
/// carried and not rendered.
pub fn clips(model: &Model, sequence: SequenceId, project_dir: Option<&Path>) -> Vec<ExportClip> {
    clips_of(model, sequence, None, project_dir)
}

/// [`clips`] of some of the sequence's items alone, `only`, as if nothing
/// else were on it: what a render of a selection draws.
pub fn clips_of(
    model: &Model,
    sequence: SequenceId,
    only: Option<&[ItemId]>,
    project_dir: Option<&Path>,
) -> Vec<ExportClip> {
    let wanted = |id: ItemId| only.is_none_or(|only| only.contains(&id));
    let lanes: Vec<LaneId> = model.lanes.index().of(sequence).map(|(id, _)| id).collect();
    let Some(seq) = model.sequences.get(sequence) else {
        return Vec::new();
    };
    // The lowest lane with a picture on it: where the backgrounds draw.
    let ground = lanes.iter().position(|lane| {
        model.items.index().on_lane(*lane).any(|(id, _, _)| {
            wanted(id)
                && model
                    .items
                    .get(id)
                    .is_some_and(|item| is_picture(model, item))
        })
    });
    let mut out = Vec::new();
    for (index, lane_id) in lanes.iter().enumerate() {
        let Some(lane) = model.lanes.get(*lane_id) else {
            continue;
        };
        for (id, _, _) in model.items.index().on_lane(*lane_id) {
            let Some(item) = model.items.get(id).filter(|_| wanted(id)) else {
                continue;
            };
            let backdrop = if ground == Some(index) {
                item.picture
                    .background
                    .as_ref()
                    .or(Some(&seq.background))
                    .and_then(|background| backdrop_of(model, background))
            } else {
                None
            };
            if let Some(mut clip) = clip_of(model, item, index, project_dir) {
                clip.hidden |= !lane.visible;
                clip.muted |= lane.muted;
                clip.backdrop = backdrop.filter(|_| clip.kind.is_visual());
                out.push(clip);
            }
        }
    }
    out
}

/// Whether an item puts a picture of a file on the screen.
fn is_picture(model: &Model, item: &Item) -> bool {
    match &item.content {
        Content::Media(cut) => {
            cut.take == Take::Whole
                && model
                    .media
                    .get(cut.media)
                    .is_some_and(|media| media.kind.has_picture() == Some(true))
        }
        _ => false,
    }
}

/// Seconds, from a place.
fn secs(t: Tick) -> f64 {
    t.display_seconds()
}

/// Seconds, from a distance.
fn dur(d: TickDur) -> f64 {
    d.display_seconds()
}

/// Seconds, from a length.
fn len(l: TickLen) -> f64 {
    TickDur::from(l).display_seconds()
}

/// One item as the renderer takes it, on stack place `index`.
fn clip_of(
    model: &Model,
    item: &Item,
    index: usize,
    project_dir: Option<&Path>,
) -> Option<ExportClip> {
    let (start, duration) = (secs(item.start), len(item.len));
    let blank = |kind| ExportClip::blank(kind, start, duration, index);
    match &item.content {
        // Drawn to pictures by the host; not drawn yet.
        Content::Title(_) | Content::Shape(_) | Content::Nest(_) | Content::Unknown { .. } => None,
        // A treatment of the pictures beneath: its chain over the stack
        // under its lane for its span, its opacity the strength, its fades
        // the ramps.
        Content::Layer => Some(ExportClip {
            muted: true,
            volume: 0.0,
            fade_in: dur(item.sound.fade_in),
            fade_out: dur(item.sound.fade_out),
            effects: links(item, &item.effects),
            opacity: base(item, &item.picture.opacity),
            has_audio: Some(false),
            ..blank(ClipKind::Layer)
        }),
        // A treatment of the sound: its filters over the mix for its span.
        Content::SoundLayer => Some(ExportClip {
            hidden: true,
            muted: item.sound.muted,
            volume: 0.0,
            fade_in: dur(item.sound.fade_in),
            fade_out: dur(item.sound.fade_out),
            filter_chain: audio_filter_chain(&links(item, &item.filters)),
            opacity: base(item, &item.picture.opacity),
            has_audio: Some(false),
            ..blank(ClipKind::SoundLayer)
        }),
        // A transition placed as an item: its package over what is beneath.
        Content::Transition(effect) => Some(ExportClip {
            hidden: !effect.enabled,
            muted: true,
            volume: 0.0,
            has_audio: Some(false),
            transition: Some(TransitionSpec {
                kind: effect.package.clone(),
                duration,
                params: values(effect),
            }),
            ..blank(ClipKind::Transition)
        }),
        Content::Media(cut) => {
            let media = model.media.get(cut.media)?;
            let kind = match (&media.kind, &cut.take) {
                (MediaKind::Audio, _) | (_, Take::Sound) => ClipKind::Audio,
                (MediaKind::Image, Take::Whole) => ClipKind::Image,
                (MediaKind::Video, Take::Whole) => ClipKind::Video,
                _ => return None,
            };
            // An enhanced copy, while it shows, is what is read; the copy
            // starts `copy_start` into the original.
            let (file, shift) = match &cut.enhanced {
                Some(enhanced) if enhanced.on => match model.media.get(enhanced.copy) {
                    Some(copy) => (copy.as_ref(), dur(enhanced.copy_start)),
                    None => (media.as_ref(), 0.0),
                },
                _ => (media.as_ref(), 0.0),
            };
            let timing = timing_of(&cut.time, item);
            Some(ExportClip {
                path: file.path.clone(),
                audio_stream: cut.stream,
                color_range: file.range.as_ref().and_then(crate::engine_range),
                source_start: (timing.source_start - shift).max(0.0),
                muted: item.sound.muted,
                volume: base(item, &item.sound.volume),
                fade_in: dur(item.sound.fade_in),
                fade_out: dur(item.sound.fade_out),
                filter_chain: audio_filter_chain(&links(item, &item.filters)),
                speed: timing.speed,
                preserve_pitch: item.sound.keep_pitch,
                speed_curve: timing.curve,
                animation: keys(item),
                motion: motion(&item.picture.motion),
                flip_h: item.picture.flip_h,
                flip_v: item.picture.flip_v,
                blend: blend_name(&item.picture.blend).to_owned(),
                crop: item
                    .picture
                    .crop
                    .filter(|c| c.left != 0.0 || c.top != 0.0 || c.right != 0.0 || c.bottom != 0.0)
                    .map(|c| [c.left, c.top, c.right, c.bottom]),
                effects: links(item, &item.effects),
                scale: base(item, &item.picture.scale),
                offset_x: base(item, &item.picture.offset_x),
                offset_y: base(item, &item.picture.offset_y),
                rotation: base(item, &item.picture.rotation),
                stretch_x: base(item, &item.picture.stretch_x),
                stretch_y: base(item, &item.picture.stretch_y),
                opacity: base(item, &item.picture.opacity),
                transition: item.cut_in.as_ref().map(|cut_in| TransitionSpec {
                    kind: cut_in.effect.package.clone(),
                    duration: len(cut_in.len),
                    params: values(&cut_in.effect),
                }),
                media_width: file.size.map(|size| size.width.get()),
                media_height: file.size.map(|size| size.height.get()),
                media_duration: file.length.map(len),
                has_audio: Some(file.has_sound),
                cutout: item.picture.cutout.clone(),
                masks: if kind == ClipKind::Audio {
                    Vec::new()
                } else {
                    masks(item)
                },
                mask_dir: match (&item.picture.cutout, project_dir) {
                    (Some(cutout), Some(dir)) => {
                        concat_vision::mask_dir(dir, &file.path, &cutout.subject)
                            .to_string_lossy()
                            .into_owned()
                    }
                    _ => String::new(),
                },
                stabilize: item
                    .picture
                    .stabilize
                    .as_ref()
                    .filter(|_| kind == ClipKind::Video)
                    .map(|s| concat_vision::stabilize::Settings {
                        smoothness: s.smoothness,
                        max_zoom: s.max_zoom,
                        tripod: s.tripod,
                        keep_rotation: s.keep_rotation,
                    }),
                motion_dir: match (&item.picture.stabilize, project_dir) {
                    (Some(_), Some(dir)) if kind == ClipKind::Video => {
                        concat_vision::stabilize::motion_dir(dir, &file.path)
                            .to_string_lossy()
                            .into_owned()
                    }
                    _ => String::new(),
                },
                voice: voice_of(item, file, cut.stream, kind, project_dir),
                ..blank(kind)
            })
        }
    }
}

/// The parts a clip's sound is split into, once both are made.
fn voice_of(
    item: &Item,
    file: &Media,
    stream: Option<u32>,
    kind: ClipKind,
    project_dir: Option<&Path>,
) -> Option<crate::VoiceStems> {
    let (mix, dir) = (item.sound.voice.as_ref()?, project_dir?);
    if kind == ClipKind::Image {
        return None;
    }
    let stems = crate::voice_stems_dir(dir, &file.path, stream, &mix.kind);
    let (voice, rest) = (stems.join("voice.wav"), stems.join("rest.wav"));
    // Until both parts are there the clip plays as recorded, and the host
    // is making them.
    (voice.is_file() && rest.is_file()).then(|| crate::VoiceStems {
        voice: voice.to_string_lossy().into_owned(),
        rest: rest.to_string_lossy().into_owned(),
        voice_gain: mix.voice,
        rest_gain: mix.background,
    })
}

/// What a time map says the renderer plays: where in the source the clip
/// starts, its mean pace, and its curve by share of the clip when the pace
/// changes along it.
struct Timing {
    source_start: f64,
    speed: f64,
    curve: Vec<(f64, f64)>,
}

/// The timing of a map over an item's window.
fn timing_of(map: &TimeMap, item: &Item) -> Timing {
    let from = item.offset;
    let to = item.offset + item.len;
    let source_start = map.source_of(from).map_or(0.0, secs);
    match map {
        TimeMap::Linear { speed, .. } => Timing {
            source_start,
            speed: speed.display().abs(),
            curve: Vec::new(),
        },
        TimeMap::Keyed { first, rest, .. } => {
            let end = map.source_of(to).map_or(source_start, secs);
            let window = len(item.len);
            let speed = if window > 0.0 {
                ((end - source_start) / window).abs()
            } else {
                1.0
            };
            let share = |t: Tick| ((t - from).get() as f64 / item.len.get() as f64).clamp(0.0, 1.0);
            let mut curve: Vec<(f64, f64)> = Vec::new();
            let mut at = *first;
            for (stretch, next) in rest {
                // Only what the window shows.
                if next.at <= from || at.at >= to {
                    at = *next;
                    continue;
                }
                let (a, b) = (at.at.max(from), next.at.min(to));
                let pieces = match stretch {
                    concat_doc::model::TimeCurve::Smooth { .. } => PIECES,
                    _ => 1,
                };
                let span = (b - a).get();
                for k in 0..=pieces {
                    let t = Tick::new(a.get() + span * i64::from(k) / i64::from(pieces));
                    // The pace leaving a key is this stretch's; at the far
                    // end it is this stretch's arriving.
                    let probe = if k == pieces {
                        Tick::new(t.get() - 1).max(a)
                    } else {
                        t
                    };
                    if let Some(pace) = map.pace_at(probe) {
                        curve.push((share(t), pace.abs()));
                    }
                }
                at = *next;
            }
            Timing {
                source_start,
                speed,
                curve,
            }
        }
        TimeMap::Unknown(_) => Timing {
            source_start,
            speed: 1.0,
            curve: Vec::new(),
        },
    }
}

/// The share of an item a content tick is at.
fn share_of(item: &Item, at: Tick) -> f64 {
    (at - item.offset).get() as f64 / item.len.get() as f64
}

/// An easing as four numbers; one this build cannot read is a straight
/// line.
fn ease_of(ease: &Ease) -> [f64; 4] {
    match ease {
        Ease::Bezier { x1, y1, x2, y2 } => [*x1, *y1, *x2, *y2],
        Ease::Unknown(_) => [0.0, 0.0, 1.0, 1.0],
    }
}

/// The constant the engine is handed for one of an item's numbers: its
/// own value until it is keyed, and then the neutral value, since a keyed
/// number's keys travel as they are rather than as a factor on it - the
/// engine multiplies a scale or an opacity by its keys, so an item at zero
/// opacity could never be keyed back up otherwise.
fn base(item: &Item, number: &Animated<f64>) -> f64 {
    if !number.is_keyed() {
        return number.value;
    }
    let neutral_one = [
        &item.picture.scale,
        &item.picture.stretch_x,
        &item.picture.stretch_y,
        &item.picture.opacity,
        &item.sound.volume,
    ];
    if neutral_one.iter().any(|n| std::ptr::eq(*n, number)) {
        1.0
    } else {
        0.0
    }
}

/// The keys the engine plays, number by number.
pub fn keys(item: &Item) -> Vec<ExportKey> {
    let p = &item.picture;
    let numbers: [(&str, &Animated<f64>); 8] = [
        ("scale", &p.scale),
        ("offsetX", &p.offset_x),
        ("offsetY", &p.offset_y),
        ("rotation", &p.rotation),
        ("stretchX", &p.stretch_x),
        ("stretchY", &p.stretch_y),
        ("opacity", &p.opacity),
        ("volume", &item.sound.volume),
    ];
    let mut out = Vec::new();
    for (name, number) in numbers {
        out.extend(number.keys.iter().map(|key| ExportKey {
            property: name.to_owned(),
            at: share_of(item, key.at),
            value: key.value,
            ease: ease_of(&key.ease),
        }));
    }
    out
}

/// An item's gain over its length, as `(share, gain)` pairs, or empty for
/// an item whose gain is one number.
pub fn volume_curve(item: &Item) -> Vec<(f64, f64)> {
    item.sound
        .volume
        .keys
        .iter()
        .map(|key| (share_of(item, key.at), key.value))
        .collect()
}

/// An item's number at a share of it: the key's value there, or its own.
pub fn value_at(item: &Item, number: &Animated<f64>, share: f64) -> f64 {
    if !number.is_keyed() {
        return number.value;
    }
    track(item, number).value_at(share, number.value)
}

/// The engine's track of one number's keys.
fn track(item: &Item, number: &Animated<f64>) -> Track {
    Track::new(
        number
            .keys
            .iter()
            .map(|key| {
                let [x1, y1, x2, y2] = ease_of(&key.ease);
                AnimKey {
                    at: share_of(item, key.at),
                    value: key.value,
                    ease: AnimEase::new(x1, y1, x2, y2),
                }
            })
            .collect(),
    )
}

/// An effect's knobs at their set values; a knob with keys and no value
/// has its first key's.
fn values(effect: &Effect) -> std::collections::BTreeMap<String, f64> {
    effect
        .params
        .iter()
        .filter_map(|(name, param)| {
            let value = param
                .value
                .or_else(|| param.keys.first().map(|key| key.value))?;
            Some((name.clone(), value))
        })
        .collect()
}

/// A chain as the renderer runs it: each effect this build can read, as a
/// link to its package with its keys by share of the item.
pub fn links(item: &Item, chain: &[Entry<Effect>]) -> Vec<AppliedFilter> {
    chain
        .iter()
        .filter_map(Entry::known)
        .map(|effect| link_of(effect, |at| share_of(item, at)))
        .collect()
}

/// One effect as a link, its keys placed by `share`.
pub fn link_of(effect: &Effect, share: impl Fn(Tick) -> f64) -> AppliedFilter {
    AppliedFilter {
        id: effect.package.clone(),
        params: values(effect),
        enabled: effect.enabled,
        keys: effect
            .params
            .iter()
            .filter(|(_, param)| param.is_keyed())
            .map(|(name, param)| {
                let keys = param
                    .keys
                    .iter()
                    .map(|key| {
                        let [x1, y1, x2, y2] = ease_of(&key.ease);
                        ParamKey {
                            at: share(key.at),
                            value: key.value,
                            ease: AnimEase::new(x1, y1, x2, y2),
                        }
                    })
                    .collect();
                (name.clone(), keys)
            })
            .collect(),
    }
}

/// The blend's name, as the engine reads it.
pub fn blend_name(blend: &Blend) -> &'static str {
    match blend {
        Blend::Normal | Blend::Unknown(_) => "normal",
        Blend::Multiply => "multiply",
        Blend::Screen => "screen",
        Blend::Add => "add",
        Blend::Lighten => "lighten",
        Blend::Darken => "darken",
    }
}

/// An item's entrance, exit and loop as the engine plays them; a move this
/// build does not know is left out.
pub fn motion(motion: &Motion) -> Option<EngineMotion> {
    let moving = |m: &concat_doc::model::Move| {
        Some(Moving {
            kind: EngineMove::parse(&m.name)?,
            seconds: len(m.len),
        })
    };
    let engine = EngineMotion {
        enter: motion.enter.as_ref().and_then(moving),
        exit: motion.exit.as_ref().and_then(moving),
        cycle: motion.repeat.as_ref().and_then(|l| {
            Some(Looping {
                kind: Cycle::parse(&l.name)?,
                seconds: len(l.lap),
                amount: l.amount,
            })
        }),
    };
    (!engine.is_empty()).then_some(engine)
}

/// An item's masks, top first, as the engine draws them; a mask this build
/// cannot read is left out.
pub fn masks(item: &Item) -> Vec<RidingMask> {
    item.masks
        .iter()
        .filter_map(Entry::known)
        .filter_map(|mask| riding(item, mask))
        .collect()
}

/// One mask as the engine draws it.
fn riding(item: &Item, mask: &Mask) -> Option<RidingMask> {
    let MaskSource::Shape {
        shape,
        width,
        height,
        roundness,
    } = &mask.source
    else {
        return None;
    };
    let shape = match shape {
        MaskShape::Linear => EngineShape::Linear,
        MaskShape::Mirror => EngineShape::Mirror,
        MaskShape::Circle => EngineShape::Circle,
        MaskShape::Rectangle => EngineShape::Rectangle,
        MaskShape::Heart => EngineShape::Heart,
        MaskShape::Star => EngineShape::Star,
        MaskShape::Unknown(_) => return None,
    };
    let mode = match mask.mode {
        MaskMode::Add => EngineMode::Add,
        MaskMode::Subtract => EngineMode::Subtract,
        MaskMode::Intersect => EngineMode::Intersect,
        MaskMode::None => EngineMode::None,
        MaskMode::Unknown(_) => return None,
    };
    let t = |number: &Animated<f64>| track(item, number);
    Some(RidingMask {
        base: StackedMask {
            shape: ShapeMask {
                shape,
                x: mask.x.value,
                y: mask.y.value,
                width: width.value,
                height: height.value,
                rotation: mask.rotation.value,
                roundness: roundness.value,
                feather: mask.feather.value,
                invert: mask.invert,
            },
            mode,
            opacity: mask.opacity.value,
            expand: mask.expand.value,
        },
        scale: mask.scale.value,
        tracks: MaskTracks {
            x: t(&mask.x),
            y: t(&mask.y),
            width: t(width),
            height: t(height),
            rotation: t(&mask.rotation),
            roundness: t(roundness),
            feather: t(&mask.feather),
            scale: t(&mask.scale),
            opacity: t(&mask.opacity),
            expand: t(&mask.expand),
        },
    })
}

/// A background as it is drawn: a blur and a colour as they are, a still
/// found in the bin. None for none, and for a still that is not there - a
/// background that cannot be drawn is no background, never a failed frame.
fn backdrop_of(model: &Model, background: &Background) -> Option<ExportBackdrop> {
    match background {
        Background::None | Background::Unknown(_) => None,
        Background::Blur { amount } => Some(ExportBackdrop::Blur { amount: *amount }),
        Background::Color(color) => Some(ExportBackdrop::Colour {
            rgb: [color.r, color.g, color.b],
        }),
        Background::Image(media) => model
            .media
            .get(*media)
            .filter(|item| item.kind == MediaKind::Image)
            .map(|item| ExportBackdrop::Image {
                path: item.path.clone(),
                width: item.size.map(|size| size.width.get()),
                height: item.size.map(|size| size.height.get()),
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_doc::commands::{self, Command};
    use concat_doc::model::{MediaKind, Size};
    use concat_doc::{Document, Origin};

    fn run(document: &mut Document<Model>, command: Command) -> commands::Applied {
        let outcome = commands::run(document, &command, Origin(0)).expect("applies");
        if let Some(commit) = outcome.commit {
            *document = commit.into_document();
        }
        outcome.applied
    }

    fn video(path: &str, seconds: i64) -> Media {
        Media {
            path: path.to_owned(),
            name: path.to_owned(),
            kind: MediaKind::Video,
            length: TickLen::new(seconds * concat_doc::TICKS_PER_SECOND),
            size: Size::new(1920, 1080),
            has_sound: true,
            ..Media::default()
        }
    }

    /// A document with one timeline and a ten-second file on it at two
    /// seconds in.
    fn one_clip() -> (Document<Model>, SequenceId) {
        let mut document = Document::new();
        run(&mut document, Command::AddTimeline);
        let sequence = document.view.showing.expect("a timeline shows");
        let media = run(
            &mut document,
            Command::AddMedia {
                media: video("/a.mp4", 10),
            },
        )
        .created
        .expect("a file");
        let media: concat_doc::model::MediaId = media.typed().expect("a file id");
        run(
            &mut document,
            Command::AddClipAtFirstFree {
                media,
                start: Tick::from_whole_seconds(2).expect("a tick"),
            },
        );
        (document, sequence)
    }

    #[test]
    fn a_clip_of_a_file_flattens_in_seconds_on_the_bottom_lane() {
        let (document, sequence) = one_clip();
        let flat = clips(&document, sequence, None);
        assert_eq!(flat.len(), 1);
        let clip = &flat[0];
        assert_eq!(clip.path, "/a.mp4");
        assert!(clip.kind == ClipKind::Video);
        assert!((clip.start - 2.0).abs() < 1e-9);
        assert!((clip.duration - 10.0).abs() < 1e-9);
        assert_eq!(clip.track, 0);
        assert_eq!(clip.media_width, Some(1920));
        assert_eq!(clip.has_audio, Some(true));
        assert!((clip.speed - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_speed_ramp_flattens_to_a_curve_by_share_of_the_clip() {
        let (mut document, sequence) = one_clip();
        let item = document.items.ids().next().expect("an item");
        run(
            &mut document,
            Command::SetClipSpeedCurve {
                item,
                curve: Some(vec![
                    concat_doc::model::SpeedPoint {
                        at: 0.0,
                        speed: 1.0,
                        slope: None,
                    },
                    concat_doc::model::SpeedPoint {
                        at: 1.0,
                        speed: 2.0,
                        slope: None,
                    },
                ]),
                ripple: false,
            },
        );
        let clip = &clips(&document, sequence, None)[0];
        assert!(!clip.speed_curve.is_empty());
        let (first, last) = (
            clip.speed_curve[0],
            clip.speed_curve[clip.speed_curve.len() - 1],
        );
        assert!(
            first.0.abs() < 1e-9 && (first.1 - 1.0).abs() < 0.05,
            "{first:?}"
        );
        assert!(
            (last.0 - 1.0).abs() < 1e-9 && (last.1 - 2.0).abs() < 0.05,
            "{last:?}"
        );
        // Ten seconds of source at a mean of one and a half: 6.67 s.
        assert!(
            (clip.duration - 10.0 / 1.5).abs() < 0.01,
            "{}",
            clip.duration
        );
        assert!((clip.speed - 1.5).abs() < 0.01);
    }
}

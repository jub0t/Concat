// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A clip of the earlier file as an item: its place and length, what it
//! shows, its picture and sound, keys, effects, masks and the transition
//! on its cut. Its lane, and what it is tied to, are the timeline's to
//! settle once every clip of it is read.

use std::collections::{BTreeMap, HashMap};

use concat_doc::commands::{self, Command, MIN_LEN, speed_from_decimal};
use concat_doc::model::{
    Animated, Background, Blend, BrushTool, Content, Crop, CutIn, Cutout, CutoutMode, Effect,
    Enhanced, Entry, Frame, Item, ItemId, Key, Lane, LaneId, Loop, Mask, MaskId, MaskMode,
    MaskShape, MaskSource, Media, MediaCut, MediaId, MediaKind, Model, Motion, Move, OrderKey,
    Param, Picture, Sequence, Sound, Speed, SpeedPoint, Stabilize, Stroke, Subject, Take, TimeMap,
    VoiceKind, VoiceMix,
};
use concat_doc::{Document, Origin, Tick, TickDur, TickLen};
use serde_json::Value;

use crate::ImportError;
use crate::build::{Build, animated, key_list, show, tidy_keys};
use crate::look;
use crate::read::{Fields, Kept, color, dur, tick};

/// The least a size above zero is taken to be.
const TINY: f64 = 1e-6;

/// How far apart two times may be and still be the same time to a
/// person: a millisecond.
const HAIR: i64 = commands::HAIR.get();

/// A clip, read, waiting for its lane and its tie.
pub(crate) struct Clip<'a> {
    /// Its id in the earlier file.
    pub(crate) old: String,
    /// Where notes about it say it is.
    pub(crate) place: String,
    /// Its new id.
    pub(crate) id: ItemId,
    /// The track it named.
    pub(crate) track: Option<&'a str>,
    /// The item, on no lane yet.
    pub(crate) item: Item,
    /// The clip it was detached from, by its earlier id.
    pub(crate) detached_from: Option<&'a str>,
    /// What it kept that has no field, put in its residual at the end.
    pub(crate) kept: Kept,
    /// The clip as the file had it.
    pub(crate) source: &'a Value,
}

/// What reading a clip needs from the rest of the file.
pub(crate) struct Bin<'b> {
    /// The new id of each file, by its id in the earlier file.
    pub(crate) media: &'b HashMap<String, MediaId>,
}

impl Bin<'_> {
    fn get(&self, old: &str) -> Option<MediaId> {
        self.media.get(old).copied()
    }
}

/// The kinds the new file's content is spelled with, which a kind of the
/// earlier file that this build does not know must not be read back as.
const NEW_KINDS: [&str; 7] = [
    "media",
    "nest",
    "title",
    "shape",
    "transition",
    "layer",
    "sound_layer",
];

/// Reads one clip of a timeline.
pub(crate) fn clip<'a>(
    build: &mut Build,
    bin: &Bin<'_>,
    value: &'a Value,
) -> Result<Clip<'a>, ImportError> {
    let mut f = Fields::of(value);
    let old = f.string_or("id", "");
    let kind = f.string_or("kind", "video");
    let name = f.string_or("name", "clip");
    let place = format!("clip \"{name}\" ({old})");
    let id: ItemId = build.mint()?;
    let track = f.str("trackId");
    let mut kept = Kept::default();

    // Where it is: its end is the tick nearest where it ends, so two
    // clips that met in seconds meet in ticks.
    let start_s = f.f64_or("start", 0.0);
    let duration_s = f.f64_or("duration", 1.0);
    let mut start = tick(start_s).unwrap_or(Tick::ZERO);
    if start < Tick::ZERO {
        build.note(
            &place,
            "it started before the timeline did; it starts at its start now",
        );
        start = Tick::ZERO;
    }
    let end = tick(start_s + duration_s).unwrap_or(start);
    let len = match end.checked_since(start).and_then(|d| TickLen::new(d.get())) {
        Some(len) => len,
        None => {
            build.note(
                &place,
                format!(
                    "its length of {} seconds is no length; it is a sixtieth of a second now",
                    show(duration_s)
                ),
            );
            MIN_LEN
        }
    };

    let placeholder: LaneId = commands::placeholder();
    let mut item = Item::new(placeholder, start, len, Content::Layer);
    item.name = name;

    let media_kind = matches!(kind.as_str(), "video" | "audio" | "image");
    let known = media_kind
        || matches!(
            kind.as_str(),
            "text" | "shape" | "layer" | "soundLayer" | "transition"
        );
    if !known {
        // A kind this build does not know: kept whole as its content.
        let mut rest = serde_json::Map::new();
        for (key, field) in f.rest() {
            rest.insert(key.to_owned(), field.clone());
        }
        let kind_name = if NEW_KINDS.contains(&kind.as_str()) {
            format!("earlier-{kind}")
        } else {
            kind.clone()
        };
        build.note(
            &place,
            format!("it is a kind of clip this version does not know (\"{kind}\"); it is kept as it was and does not play"),
        );
        item.content = Content::Unknown {
            kind: kind_name,
            raw: build.raw(&Value::Object(rest)),
        };
        return Ok(Clip {
            old,
            place,
            id,
            track,
            item,
            detached_from: None,
            kept,
            source: value,
        });
    }

    item.picture = picture(build, &place, &mut f, bin, &mut kept);
    item.sound = sound(build, &place, &mut f, &mut kept);

    let mut detached_from = None;
    match kind.as_str() {
        "video" | "audio" | "image" => {
            match media_cut(build, &place, &mut f, bin, &kind, start, len, &mut kept) {
                Some((cut, len)) => {
                    item.content = Content::Media(cut);
                    item.len = len;
                }
                None => {
                    // Its file is not in the bin: kept, and offline.
                    let mut rest = serde_json::Map::new();
                    for (key, field) in f.rest() {
                        rest.insert(key.to_owned(), field.clone());
                    }
                    item.content = Content::Unknown {
                        kind: kind.clone(),
                        raw: build.raw(&Value::Object(rest)),
                    };
                    return Ok(Clip {
                        old,
                        place,
                        id,
                        track,
                        item,
                        detached_from: None,
                        kept,
                        source: value,
                    });
                }
            }
            detached_from = f.str("detachedFrom");
        }
        _ => {
            // Nothing behind it to be cut, played at a speed or in a
            // stream: what the earlier file kept of those did nothing.
            f.skip(&[
                "mediaId",
                "sourceStart",
                "speed",
                "speedCurve",
                "audioStream",
                "detachedFrom",
                "enhanced",
            ]);
        }
    }
    let (offset, len) = (item.offset, item.len);

    // The keys of its own numbers, on its content time.
    keys(build, &place, &mut f, &mut item, offset, len, &mut kept);

    // Its chains and masks.
    let mut masks: HashMap<String, MaskId> = HashMap::new();
    for (at, mask) in f.list("masks").iter().enumerate() {
        match self::mask(build, &place, mask, offset, len)? {
            Some((old_id, mask)) => {
                if let Some(old_id) = old_id {
                    masks.insert(old_id, mask.id);
                }
                item.masks.push(Entry::Known(mask));
            }
            None => {
                build.note(
                    &place,
                    format!(
                        "its mask {} could not be read; it is kept as it was",
                        at + 1
                    ),
                );
                item.masks.push(Entry::Unread(build.raw(mask)));
            }
        }
    }
    item.effects = chain(build, &place, f.list("videoEffects"), offset, len, &masks)?;
    item.filters = chain(build, &place, f.list("filters"), offset, len, &masks)?;

    match kind.as_str() {
        "text" => {
            item.content = Content::Title(match f.object("text") {
                Some(text) => look::title(build, &place, text, &mut kept, "text"),
                None => Default::default(),
            });
        }
        "shape" => {
            item.content = Content::Shape(match f.object("shape") {
                Some(shape) => look::shape(build, &place, shape, &mut kept, "shape"),
                None => Default::default(),
            });
        }
        "layer" => item.content = Content::Layer,
        "soundLayer" => item.content = Content::SoundLayer,
        "transition" => {
            // Its package is the one link of its chain.
            let mut chain = std::mem::take(&mut item.effects);
            let first = chain.iter().position(|entry| entry.known().is_some());
            match first {
                Some(at) => {
                    if let Entry::Known(effect) = chain.remove(at) {
                        item.content = Content::Transition(effect);
                    }
                    if !chain.is_empty() {
                        build.note(
                            &place,
                            "a transition plays one package; the other effects on it are kept on it and do not play",
                        );
                        // Kept as they were, which a transition item does
                        // not read: the file still has them.
                        item.effects = chain;
                    }
                }
                None => {
                    build.note(
                        &place,
                        "this transition names no package; it is an empty layer now, which plays as it did: not at all",
                    );
                    item.effects = chain;
                    item.content = Content::Layer;
                }
            }
        }
        _ => {}
    }

    // The transition on the cut into it.
    if let Some(cut) = f.object("transitionIn") {
        item.cut_in = cut_in(build, &place, cut, &mut kept)?;
    }
    kept.rest("", &f);

    Ok(Clip {
        old,
        place,
        id,
        track,
        item,
        detached_from,
        kept,
        source: value,
    })
}

// --- Picture and sound ------------------------------------------------------

fn picture(
    build: &mut Build,
    place: &str,
    f: &mut Fields<'_>,
    bin: &Bin<'_>,
    kept: &mut Kept,
) -> Picture {
    let base = Picture::default();
    let mut p = Picture {
        offset_x: Animated::fixed(f.f64_or("offsetX", 0.0)),
        offset_y: Animated::fixed(f.f64_or("offsetY", 0.0)),
        scale: Animated::fixed(f.f64_or("scale", 1.0)),
        rotation: Animated::fixed(f.f64_or("rotation", 0.0)),
        stretch_x: Animated::fixed(f.f64_or("stretchX", 1.0)),
        stretch_y: Animated::fixed(f.f64_or("stretchY", 1.0)),
        opacity: Animated::fixed(f.f64_or("opacity", 1.0)),
        flip_h: f.bool_or("flipH", false),
        flip_v: f.bool_or("flipV", false),
        ..base
    };
    p.blend = match f.str("blend") {
        None | Some("normal") => Blend::Normal,
        Some("multiply") => Blend::Multiply,
        Some("screen") => Blend::Screen,
        Some("add") => Blend::Add,
        Some("lighten") => Blend::Lighten,
        Some("darken") => Blend::Darken,
        Some(other) => Blend::Unknown(build.raw(&Value::String(other.to_owned()))),
    };
    if let Some(crop) = f.object("crop") {
        let mut c = Fields::of(crop);
        let crop_value = Crop {
            left: c.f64_or("left", 0.0),
            top: c.f64_or("top", 0.0),
            right: c.f64_or("right", 0.0),
            bottom: c.f64_or("bottom", 0.0),
        };
        kept.rest("crop", &c);
        let sides = [
            crop_value.left,
            crop_value.top,
            crop_value.right,
            crop_value.bottom,
        ];
        let fits = sides.iter().all(|side| (0.0..1.0).contains(side))
            && crop_value.left + crop_value.right < 1.0
            && crop_value.top + crop_value.bottom < 1.0;
        if fits {
            p.crop = (sides != [0.0; 4]).then_some(crop_value);
        } else {
            build.note(
                place,
                "its crop left nothing of the picture; it is kept as it was and not applied",
            );
            kept.keep("", "crop", crop.clone());
        }
    }
    if let Some(cutout) = f.object("cutout") {
        p.cutout = Some(self::cutout(build, place, cutout, kept));
    }
    if let Some(stabilize) = f.object("stabilize") {
        let mut s = Fields::of(stabilize);
        let smoothness = s.f64_or("smoothness", 0.5);
        let max_zoom = s.f64_or("maxZoom", 1.2);
        p.stabilize = Some(Stabilize {
            smoothness: build.within(place, "stabilize smoothness", smoothness, 0.0, 1.0),
            max_zoom: build.within(place, "stabilize zoom", max_zoom, 1.0, f64::MAX),
            keep_rotation: s.bool_or("keepRotation", false),
            tripod: s.bool_or("tripod", false),
        });
        kept.rest("stabilize", &s);
    }
    if let Some(background) = f.object("background") {
        p.background = self::background(build, place, background, bin, kept, "background");
    }
    if let Some(motion) = f.object("motion") {
        p.motion = self::motion(build, place, motion, kept);
    }
    p
}

/// A background, of a clip or of a timeline. A still that is not in the
/// bin, or is no still, is kept as it was and draws nothing, as the
/// earlier file had it.
pub(crate) fn background(
    build: &mut Build,
    place: &str,
    value: &Value,
    bin: &Bin<'_>,
    kept: &mut Kept,
    path: &str,
) -> Option<Background> {
    let mut b = Fields::of(value);
    let background = match b.str("kind") {
        None | Some("none") => Background::None,
        Some("blur") => {
            let amount = b.f64_or("amount", 0.5);
            Background::Blur {
                amount: build.within(place, "background blur", amount, 0.0, 1.0),
            }
        }
        Some("color") => match b.str("color").and_then(color) {
            Some(colour) => Background::Color(colour),
            None => {
                build.note(
                    place,
                    "its background colour could not be read; it is kept as it was",
                );
                kept.keep("", path, value.clone());
                return None;
            }
        },
        Some("image") => {
            let still = b.str("mediaId").and_then(|old| bin.get(old)).filter(|id| {
                build
                    .model
                    .media
                    .get(*id)
                    .is_some_and(|m| m.kind == MediaKind::Image)
            });
            match still {
                Some(id) => Background::Image(id),
                None => {
                    build.note(
                        place,
                        "its background picture is not a still in the bin; it is kept as it was and draws nothing, as before",
                    );
                    kept.keep("", path, value.clone());
                    return None;
                }
            }
        }
        Some(_) => Background::Unknown(build.raw(value)),
    };
    if !matches!(background, Background::Unknown(_)) {
        kept.rest(path, &b);
    }
    Some(background)
}

fn cutout(build: &mut Build, place: &str, value: &Value, kept: &mut Kept) -> Cutout {
    let mut c = Fields::of(value);
    let mode = match c.str("mode") {
        None | Some("auto") => CutoutMode::Auto,
        Some("custom") => CutoutMode::Custom,
        Some(other) => CutoutMode::Unknown(build.raw(&Value::String(other.to_owned()))),
    };
    let subject = match c.str("subject") {
        None | Some("auto") => Subject::Auto,
        Some("person") => Subject::Person,
        Some("object") => Subject::Object,
        Some(other) => Subject::Unknown(build.raw(&Value::String(other.to_owned()))),
    };
    let feather = c.f64_or("feather", 0.01);
    let feather = build.within(place, "cutout feather", feather, 0.0, f64::MAX);
    let mut strokes = Vec::new();
    for (at, stroke) in c.list("strokes").iter().enumerate() {
        let mut s = Fields::of(stroke);
        let tool = match s.str("tool") {
            Some("smartBrush") => Some(BrushTool::SmartBrush),
            Some("brush") => Some(BrushTool::Brush),
            Some("smartEraser") => Some(BrushTool::SmartEraser),
            Some("eraser") => Some(BrushTool::Eraser),
            Some(other) => Some(BrushTool::Unknown(
                build.raw(&Value::String(other.to_owned())),
            )),
            None => None,
        };
        let size = s.f64("size").filter(|size| *size > 0.0);
        let points: Option<Vec<[f64; 2]>> = s
            .list("points")
            .iter()
            .map(|point| match point.as_array().map(Vec::as_slice) {
                Some([x, y]) => Some([x.as_f64()?, y.as_f64()?]),
                _ => None,
            })
            .collect();
        match (tool, size, points) {
            (Some(tool), Some(size), Some(points)) => {
                let at_s = s.f64("at").and_then(tick);
                strokes.push(Stroke {
                    tool,
                    size,
                    points,
                    at: at_s,
                });
                kept.rest(&format!("cutout/strokes/{at}"), &s);
            }
            _ => {
                build.note(
                    place,
                    "a brush stroke of its cutout could not be read; it is kept as it was",
                );
                kept.keep("cutout/strokes", &at.to_string(), stroke.clone());
            }
        }
    }
    kept.rest("cutout", &c);
    Cutout {
        mode,
        subject,
        feather,
        strokes,
    }
}

fn motion(build: &mut Build, place: &str, value: &Value, kept: &mut Kept) -> Motion {
    let mut m = Fields::of(value);
    let mut a_move = |build: &mut Build, key: &str, m: &mut Fields<'_>| -> Option<Move> {
        let value = m.object(key)?;
        let mut v = Fields::of(value);
        let Some(name) = v.str("kind").map(str::to_owned) else {
            kept.keep("motion", key, value.clone());
            return None;
        };
        let seconds = v.f64_or("seconds", 0.5);
        let len = length(build, place, "move", seconds);
        kept.rest(&format!("motion/{key}"), &v);
        Some(Move { name, len })
    };
    let enter = a_move(build, "in", &mut m);
    let exit = a_move(build, "out", &mut m);
    let repeat = match m.object("loop") {
        Some(value) => {
            let mut v = Fields::of(value);
            match v.str("kind").map(str::to_owned) {
                Some(name) => {
                    let seconds = v.f64_or("seconds", 1.0);
                    let amount = v.f64_or("amount", 1.0);
                    let repeat = Loop {
                        name,
                        lap: length(build, place, "loop", seconds),
                        amount: build.within(place, "loop amount", amount, 0.0, 2.0),
                    };
                    kept.rest("motion/loop", &v);
                    Some(repeat)
                }
                None => {
                    kept.keep("motion", "loop", value.clone());
                    None
                }
            }
        }
        None => None,
    };
    kept.rest("motion", &m);
    Motion {
        enter,
        exit,
        repeat,
    }
}

/// A length in seconds that must be more than nothing.
fn length(build: &mut Build, place: &str, what: &str, seconds: f64) -> TickLen {
    match tick(seconds).and_then(|t| TickLen::new(t.get())) {
        Some(len) => len,
        None => {
            build.note(
                place,
                format!(
                    "its {what} of {} seconds is no length; it is a sixtieth of a second now",
                    show(seconds)
                ),
            );
            MIN_LEN
        }
    }
}

fn sound(build: &mut Build, place: &str, f: &mut Fields<'_>, kept: &mut Kept) -> Sound {
    let fade = |build: &mut Build, f: &mut Fields<'_>, key: &str| -> TickDur {
        let seconds = f.f64_or(key, 0.0);
        match dur(seconds) {
            Some(d) if d >= TickDur::ZERO => d,
            _ => {
                build.note(
                    place,
                    format!(
                        "its {key} of {} seconds is no fade; it has none now",
                        show(seconds)
                    ),
                );
                TickDur::ZERO
            }
        }
    };
    let fade_in = fade(build, f, "fadeIn");
    let fade_out = fade(build, f, "fadeOut");
    let voice = f.object("voice").map(|voice| {
        let mut v = Fields::of(voice);
        let kind = match v.str("kind") {
            None | Some("speech") => VoiceKind::Speech,
            Some("vocals") => VoiceKind::Vocals,
            Some(other) => VoiceKind::Unknown(build.raw(&Value::String(other.to_owned()))),
        };
        let voice_gain = v.f64_or("voice", 1.0);
        let background = v.f64_or("background", 0.0);
        let mix = VoiceMix {
            kind,
            voice: build.within(place, "voice level", voice_gain, 0.0, f64::MAX),
            background: build.within(place, "background level", background, 0.0, f64::MAX),
        };
        kept.rest("voice", &v);
        mix
    });
    Sound {
        volume: Animated::fixed(f.f64_or("volume", 1.0)),
        fade_in,
        fade_out,
        muted: f.bool_or("muted", false),
        keep_pitch: f.bool_or("preservePitch", true),
        voice,
    }
}

/// The keys of an item's own numbers, put on its content time, and its
/// numbers held to what each can be.
fn keys(
    build: &mut Build,
    place: &str,
    f: &mut Fields<'_>,
    item: &mut Item,
    offset: Tick,
    len: TickLen,
    kept: &mut Kept,
) {
    let mut by: BTreeMap<&str, Vec<Key<f64>>> = BTreeMap::new();
    const PROPERTIES: [&str; 8] = [
        "scale", "offsetX", "offsetY", "rotation", "stretchX", "stretchY", "opacity", "volume",
    ];
    for (at, key) in f.list("keys").iter().enumerate() {
        let mut k = Fields::of(key);
        let property = k
            .str("property")
            .and_then(|p| PROPERTIES.iter().find(|q| **q == p));
        let read = property.and_then(|_| crate::build::one_key(build, place, &mut k, offset, len));
        match (property, read) {
            (Some(property), Some(read)) => {
                kept.rest(&format!("keys/{at}"), &k);
                by.entry(property).or_default().push(read);
            }
            _ => {
                build.note(place, "one of its keys could not be read, or is of a property this version does not key; it is kept as it was");
                kept.keep("keys", &at.to_string(), key.clone());
            }
        }
    }
    let mut take = |build: &mut Build, name: &str| -> Vec<Key<f64>> {
        let mut keys = by.remove(name).unwrap_or_default();
        tidy_keys(build, place, name, &mut keys);
        keys
    };
    let p = &mut item.picture;
    let (ox, oy) = (p.offset_x.value, p.offset_y.value);
    let (sc, ro, sx, sy, op) = (
        p.scale.value,
        p.rotation.value,
        p.stretch_x.value,
        p.stretch_y.value,
        p.opacity.value,
    );
    let vol = item.sound.volume.value;
    let keys_ox = take(build, "offsetX");
    let keys_oy = take(build, "offsetY");
    let keys_sc = take(build, "scale");
    let keys_ro = take(build, "rotation");
    let keys_sx = take(build, "stretchX");
    let keys_sy = take(build, "stretchY");
    let keys_op = take(build, "opacity");
    let keys_vol = take(build, "volume");
    let any = f64::MIN;
    item.picture.offset_x = animated(
        build,
        place,
        "horizontal position",
        ox,
        keys_ox,
        any,
        f64::MAX,
    );
    item.picture.offset_y = animated(
        build,
        place,
        "vertical position",
        oy,
        keys_oy,
        any,
        f64::MAX,
    );
    item.picture.scale = animated(build, place, "scale", sc, keys_sc, TINY, f64::MAX);
    item.picture.rotation = animated(build, place, "rotation", ro, keys_ro, any, f64::MAX);
    item.picture.stretch_x = animated(build, place, "width stretch", sx, keys_sx, TINY, f64::MAX);
    item.picture.stretch_y = animated(build, place, "height stretch", sy, keys_sy, TINY, f64::MAX);
    item.picture.opacity = animated(build, place, "opacity", op, keys_op, 0.0, 1.0);
    item.sound.volume = animated(build, place, "volume", vol, keys_vol, 0.0, f64::MAX);
}

// --- What it shows ------------------------------------------------------------

/// A cut of a file: what of it plays, from where, at what pace. `None`
/// for a clip whose file is not in the bin. The length comes back as the
/// window that fits the file, and under a speed curve as the curve has
/// it.
fn media_cut(
    build: &mut Build,
    place: &str,
    f: &mut Fields<'_>,
    bin: &Bin<'_>,
    kind: &str,
    start: Tick,
    len: TickLen,
    kept: &mut Kept,
) -> Option<(MediaCut, TickLen)> {
    let _ = start;
    let named = f.str("mediaId").unwrap_or("");
    let enhanced_value = f.object("enhanced");
    // With an Enhance, the clip is the original's.
    let original = enhanced_value
        .and_then(|e| e.get("original"))
        .and_then(Value::as_str)
        .and_then(|old| bin.get(old));
    let Some(media_id) = original.or_else(|| bin.get(named)) else {
        build.note(
            place,
            format!("its file ({named}) is not in the bin; it is kept as it was and does not play"),
        );
        return None;
    };
    let file = build.model.media.get(media_id)?.as_ref().clone();
    let take = match (kind, file.kind.can_sound()) {
        ("audio", Some(true)) if file.kind == MediaKind::Video => Take::Sound,
        ("audio", Some(false)) => {
            build.note(
                place,
                "it was the sound of a still, which has none; it is the still now",
            );
            Take::Whole
        }
        _ => Take::Whole,
    };
    let stream = match f.u32("audioStream") {
        Some(index)
            if !file.streams.is_empty() && !file.streams.iter().any(|s| s.index == index) =>
        {
            build.note(
                place,
                format!("it played sound stream {index}, which its file does not list; it plays the first"),
            );
            kept.keep("", "audioStream", Value::from(index));
            None
        }
        other => other,
    };

    // Enhance: the copy, where it starts in the original, and whether
    // it shows. While it shows, the earlier file's in-point is in the
    // copy's seconds; here it is always in the original's.
    let mut enhanced = None;
    let mut copy_offset = 0.0;
    if let Some(value) = enhanced_value {
        let mut e = Fields::of(value);
        e.skip(&["original"]);
        let copy = e.str("copy").and_then(|old| bin.get(old));
        let on = e.bool_or("on", false);
        let offset_s = e.f64_or("offset", 0.0);
        match copy {
            Some(copy) if copy != media_id => {
                let copy_start = dur(offset_s)
                    .filter(|d| *d >= TickDur::ZERO)
                    .unwrap_or(TickDur::ZERO);
                if on {
                    copy_offset = offset_s.max(0.0);
                }
                enhanced = Some(Enhanced {
                    copy,
                    copy_start,
                    on,
                });
                kept.rest("enhanced", &e);
            }
            _ => {
                build.note(place, "its enhanced copy is not in the bin; it shows the original, and the Enhance is kept as it was");
                kept.keep("", "enhanced", value.clone());
            }
        }
    }

    let speed_s = f.f64_or("speed", 1.0);
    let speed = speed_from_decimal(speed_s.abs()).unwrap_or(Speed::NORMAL);
    if speed_s <= 0.0 || (speed.display() - speed_s).abs() > 1e-6 {
        build.note(
            place,
            format!(
                "its speed of {} is past what a clip can play at; it plays at {} now",
                show(speed_s),
                show(speed.display())
            ),
        );
    }
    let source_start = f.f64_or("sourceStart", 0.0) + copy_offset;
    let mut src_in = tick(source_start).unwrap_or(Tick::ZERO);
    let mut len = len;

    // The window shows only what the file has.
    if file.kind != MediaKind::Image {
        if src_in < Tick::ZERO {
            build.note(
                place,
                "it began before its file does; it begins at the file's start now",
            );
            src_in = Tick::ZERO;
        }
        if let Some(length) = file.length {
            let file_end = Tick::ZERO + length;
            if src_in >= file_end {
                // Nothing of the file is left after the in-point: the
                // window is moved back to end where the file does.
                let span = speed.of(len.dur()).unwrap_or(TickDur::ZERO);
                let back = file_end
                    .checked_sub(span)
                    .unwrap_or(Tick::ZERO)
                    .max(Tick::ZERO);
                build.note(
                    place,
                    "it began after its file ends; it shows the end of the file now",
                );
                src_in = back;
            }
            if speed
                .of(len.dur())
                .map(|d| src_in + d)
                .is_some_and(|end| end > file_end)
            {
                let room = (file_end - src_in).get();
                let fits =
                    concat_doc::rescale(room, i64::from(speed.den()), i64::from(speed.num()))
                        .and_then(TickLen::new)
                        .unwrap_or(TickLen::ONE);
                if len.get() - fits.get() > HAIR {
                    build.note(
                        place,
                        format!(
                            "it ran {} seconds past the end of its file; it ends where the file does now",
                            show(TickDur::new(len.get() - fits.get()).display_seconds())
                        ),
                    );
                }
                len = fits;
            }
        }
    }
    let mut time = TimeMap::Linear { src_in, speed };

    // While the copy shows, the window shows only what the copy has.
    if let Some(enhanced_now) = &mut enhanced
        && enhanced_now.on
        && let Some(copy) = build.model.media.get(enhanced_now.copy)
        && copy.kind != MediaKind::Image
    {
        let from = Tick::ZERO + enhanced_now.copy_start;
        let end = speed.of(len.dur()).map(|d| src_in + d);
        let inside = src_in >= from
            && copy
                .length
                .is_none_or(|l| end.is_some_and(|end| end <= from + l));
        if !inside {
            build.note(
                place,
                "its enhanced copy does not cover all it shows; it shows the original now",
            );
            enhanced_now.on = false;
        }
    }

    // A speed curve: position keys, as the document makes them from one.
    let curve = f.list("speedCurve");
    if !curve.is_empty() {
        let points: Vec<SpeedPoint> = curve
            .iter()
            .filter_map(|point| {
                Some(SpeedPoint {
                    at: point.get("at")?.as_f64()?,
                    speed: point.get("speed")?.as_f64()?,
                    slope: point.get("slope").and_then(Value::as_f64),
                })
            })
            .collect();
        if points.len() < curve.len() {
            build.note(
                place,
                "some points of its speed curve could not be read; they are kept as they were",
            );
            kept.keep("", "speedCurve", Value::Array(curve.to_vec()));
        }
        let cut = MediaCut {
            media: media_id,
            take: take.clone(),
            stream,
            time: time.clone(),
            enhanced: None,
        };
        match curved(&file, cut, len, points) {
            Some((map, curved_len)) => {
                if (curved_len.get() - len.get()).abs() > HAIR {
                    build.note(
                        place,
                        format!(
                            "under its speed curve it lasts {} seconds, where the file said {}",
                            show(curved_len.dur().display_seconds()),
                            show(len.dur().display_seconds())
                        ),
                    );
                }
                time = map;
                len = curved_len;
            }
            None => {
                build.note(place, "its speed curve could not be made; it plays at the curve's mean speed, and the curve is kept as it was");
                kept.keep("", "speedCurve", Value::Array(curve.to_vec()));
            }
        }
    }

    Some((
        MediaCut {
            media: media_id,
            take,
            stream,
            time,
            enhanced,
        },
        len,
    ))
}

/// The time map and length a speed curve gives a cut, worked out as the
/// document's own command works one out, on a document of the cut alone.
fn curved(
    file: &Media,
    cut: MediaCut,
    len: TickLen,
    points: Vec<SpeedPoint>,
) -> Option<(TimeMap, TickLen)> {
    let empty = Document::<Model>::new();
    let mut draft = empty.begin("Speed curve", Origin(0));
    let media: MediaId = draft.mint().ok()?;
    draft.media.insert(media, file.clone()).ok()?;
    let sequence = draft.mint().ok()?;
    draft
        .sequences
        .insert(sequence, Sequence::new("Curve", Frame::hd()))
        .ok()?;
    let lane = draft.mint().ok()?;
    draft
        .lanes
        .insert(lane, Lane::new(sequence, OrderKey::middle()))
        .ok()?;
    let item: ItemId = draft.mint().ok()?;
    let content = Content::Media(MediaCut { media, ..cut });
    draft
        .items
        .insert(item, Item::new(lane, Tick::ZERO, len, content))
        .ok()?;
    let command = Command::SetClipSpeedCurve {
        item,
        curve: Some(points),
        ripple: false,
    };
    commands::apply(&mut draft, &command).ok()?;
    let after = draft.items.get(item)?;
    match &after.content {
        Content::Media(cut) if matches!(cut.time, TimeMap::Keyed { .. }) => {
            Some((cut.time.clone(), after.len))
        }
        _ => None,
    }
}

// --- Effects, masks, the cut ----------------------------------------------

/// A chain of applied effects or filters.
fn chain(
    build: &mut Build,
    place: &str,
    list: &[Value],
    offset: Tick,
    len: TickLen,
    masks: &HashMap<String, MaskId>,
) -> Result<Vec<Entry<Effect>>, ImportError> {
    let mut chain = Vec::with_capacity(list.len());
    for value in list {
        chain.push(match effect(build, place, value, offset, len, masks)? {
            Some(effect) => Entry::Known(effect),
            None => {
                build.note(
                    place,
                    "one of its effects names no package; it is kept as it was and does not play",
                );
                Entry::Unread(build.raw(value))
            }
        });
    }
    Ok(chain)
}

/// One applied effect: its package, its knobs and their keys. `None` for
/// one that names no package.
fn effect(
    build: &mut Build,
    place: &str,
    value: &Value,
    offset: Tick,
    len: TickLen,
    masks: &HashMap<String, MaskId>,
) -> Result<Option<Effect>, ImportError> {
    let mut f = Fields::of(value);
    let Some(package) = f.str("id").filter(|p| !p.is_empty()).map(str::to_owned) else {
        return Ok(None);
    };
    let mut kept = Kept::default();
    let mut effect = Effect::new(build.mint()?, package.clone());
    let place = format!("{place}, effect {package}");
    effect.enabled = f.bool_or("enabled", true);
    if let Some(params) = f.object("params").and_then(Value::as_object) {
        for (name, number) in params {
            match number.as_f64() {
                Some(number) if !name.is_empty() => {
                    effect
                        .params
                        .insert(name.clone(), Param::fixed(Some(number)));
                }
                _ => kept.keep("params", name, number.clone()),
            }
        }
    }
    if let Some(keys) = f.object("keys").and_then(Value::as_object) {
        for (name, list) in keys {
            let Some(list) = list.as_array().filter(|_| !name.is_empty()) else {
                kept.keep("keys", name, list.clone());
                continue;
            };
            let read = key_list(
                build,
                &place,
                name,
                list,
                offset,
                len,
                &mut kept,
                &format!("keys/{name}"),
            );
            if read.is_empty() {
                continue;
            }
            effect
                .params
                .entry(name.clone())
                .or_insert_with(|| Param::fixed(None))
                .keys = read;
        }
    }
    if let Some(mask) = f.str("mask") {
        match masks.get(mask) {
            Some(id) => effect.mask = Some(*id),
            None => kept.keep("", "mask", Value::String(mask.to_owned())),
        }
    }
    kept.rest("", &f);
    if !kept.is_empty() {
        let names = kept.names().join(", ");
        build.note(
            &place,
            format!("kept {names}, which this version has no place for"),
        );
    }
    effect.residual = kept.residual(&mut build.words);
    Ok(Some(effect))
}

/// One mask: its shape, place and numbers, and their keys. `None` for
/// one that cannot be read.
fn mask(
    build: &mut Build,
    place: &str,
    value: &Value,
    offset: Tick,
    len: TickLen,
) -> Result<Option<(Option<String>, Mask)>, ImportError> {
    if !value.is_object() {
        return Ok(None);
    }
    let mut f = Fields::of(value);
    let old = f.str("id").map(str::to_owned);
    let mut kept = Kept::default();
    let mut mask = Mask::circle(build.mint()?);
    mask.name = f.string_or("name", "");
    let place = format!("{place}, mask \"{}\"", mask.name);
    mask.mode = match f.str("mode") {
        None | Some("add") => MaskMode::Add,
        Some("subtract") => MaskMode::Subtract,
        Some("intersect") => MaskMode::Intersect,
        Some("none") => MaskMode::None,
        Some(other) => MaskMode::Unknown(build.raw(&Value::String(other.to_owned()))),
    };
    mask.invert = f.bool_or("invert", false);
    let mut keys: BTreeMap<String, Vec<Key<f64>>> = BTreeMap::new();
    if let Some(list) = f.object("keys").and_then(Value::as_object) {
        for (name, list) in list {
            match list.as_array() {
                Some(list) => {
                    let read = key_list(
                        build,
                        &place,
                        name,
                        list,
                        offset,
                        len,
                        &mut kept,
                        &format!("keys/{name}"),
                    );
                    keys.insert(name.clone(), read);
                }
                None => kept.keep("keys", name, list.clone()),
            }
        }
    }
    let mut number = |build: &mut Build,
                      f: &mut Fields<'_>,
                      name: &str,
                      what: &str,
                      or: f64,
                      low: f64,
                      high: f64| {
        let value = f.f64_or(name, or);
        let keys = keys.remove(name).unwrap_or_default();
        animated(build, &place, what, value, keys, low, high)
    };
    let any = f64::MIN;
    mask.x = number(build, &mut f, "x", "mask position", 0.0, any, f64::MAX);
    mask.y = number(build, &mut f, "y", "mask position", 0.0, any, f64::MAX);
    mask.scale = number(build, &mut f, "scale", "mask scale", 1.0, TINY, f64::MAX);
    mask.rotation = number(
        build,
        &mut f,
        "rotation",
        "mask rotation",
        0.0,
        any,
        f64::MAX,
    );
    mask.feather = number(build, &mut f, "feather", "mask feather", 0.0, 0.0, 1.0);
    mask.expand = number(build, &mut f, "expand", "mask expand", 0.0, any, f64::MAX);
    mask.opacity = number(build, &mut f, "opacity", "mask opacity", 1.0, 0.0, 1.0);
    match f.object("source") {
        Some(source)
            if source
                .get("kind")
                .and_then(Value::as_str)
                .is_none_or(|k| k == "shape") =>
        {
            let mut s = Fields::of(source);
            s.skip(&["kind"]);
            let shape = match s.str("shape") {
                None | Some("circle") => MaskShape::Circle,
                Some("linear") => MaskShape::Linear,
                Some("mirror") => MaskShape::Mirror,
                Some("rectangle") => MaskShape::Rectangle,
                Some("heart") => MaskShape::Heart,
                Some("star") => MaskShape::Star,
                Some(other) => MaskShape::Unknown(build.raw(&Value::String(other.to_owned()))),
            };
            let width = number(build, &mut s, "width", "mask width", 0.66, 0.0, f64::MAX);
            let height = number(build, &mut s, "height", "mask height", 0.66, 0.0, f64::MAX);
            let roundness = number(build, &mut s, "roundness", "mask roundness", 0.0, 0.0, 1.0);
            kept.rest("source", &s);
            mask.source = MaskSource::Shape {
                shape,
                width,
                height,
                roundness,
            };
        }
        Some(source) => mask.source = MaskSource::Unknown(build.raw(source)),
        None => {}
    }
    for (name, _) in keys {
        build.note(
            &place,
            format!("its keys of \"{name}\", a number masks do not have, are kept as they were"),
        );
    }
    kept.rest("", &f);
    mask.residual = kept.residual(&mut build.words);
    Ok(Some((old, mask)))
}

/// The transition on the cut into a clip.
fn cut_in(
    build: &mut Build,
    place: &str,
    value: &Value,
    kept: &mut Kept,
) -> Result<Option<CutIn>, ImportError> {
    let mut f = Fields::of(value);
    let package = f.str("id").filter(|p| !p.is_empty()).map(str::to_owned);
    let seconds = f.f64_or("duration", 1.0);
    let len = tick(seconds).and_then(|t| TickLen::new(t.get()));
    let (Some(package), Some(len)) = (package, len) else {
        build.note(place, "the transition on its cut names no package or has no length; it is kept as it was and does not play");
        kept.keep("", "transitionIn", value.clone());
        return Ok(None);
    };
    let mut effect = Effect::new(build.mint()?, package);
    if let Some(params) = f.object("params").and_then(Value::as_object) {
        for (name, number) in params {
            match number.as_f64() {
                Some(number) if !name.is_empty() => {
                    effect
                        .params
                        .insert(name.clone(), Param::fixed(Some(number)));
                }
                _ => kept.keep("transitionIn/params", name, number.clone()),
            }
        }
    }
    let mut own = Kept::default();
    own.rest("", &f);
    effect.residual = own.residual(&mut build.words);
    Ok(Some(CutIn { effect, len }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_share_of_a_clip_lands_on_the_tick_nearest() {
        let len = TickLen::new(1_411_200_000).unwrap_or(TickLen::ONE);
        assert_eq!(
            crate::build::at_share(Tick::ZERO, len, 0.5),
            Some(Tick::new(705_600_000))
        );
        assert_eq!(
            crate::build::at_share(Tick::new(10), len, 1.0),
            Some(Tick::new(1_411_200_010))
        );
    }
}

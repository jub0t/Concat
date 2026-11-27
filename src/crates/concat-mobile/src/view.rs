// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The edit as the phone app reads it.
//!
//! Every reply of [`crate::EditorApi::call`] that carries an editor view
//! (`project.open`, `project.get`, `edit.apply`, `media.import`, ...) has,
//! on the phone, this view in its `project` field in place of the
//! document's file. The rest of the reply is the editor view's own:
//! `canUndo`, `canRedo`, `settings` and, when an edit made something,
//! `createdId`. The app reads nothing of the document's file format: the
//! view is made here, from the typed document, in the names and units the
//! app's model (`android-native`'s `api/Model.kt`) reads.
//!
//! Every time is in seconds and every id a string the app hands back as
//! it is and never takes apart. An object the app sends back whole in a
//! command - a title's style, a chain, a mask, a crop, an animation, a
//! caption lane's look, the cover - is spelled as the command takes it
//! (`concat_api::wire`), so sending it back unchanged changes nothing.
//!
//! `project`:
//!
//! | Field | What it is |
//! |---|---|
//! | `activeTimelineId` | the timeline showing; absent when there is none |
//! | `timelines` | every timeline: `id`, `name`, `video` (`width`, `height`, `rateNum`, `rateDen`, as `project.setVideo` takes it); the one showing also has `tracks` and `clips` |
//! | `media` | the bin: `id`, `path`, `name`, `kind` (`video`, `audio`, `image`, `unknown`), `duration` in seconds when the file has a length, `width` and `height` when it has a picture |
//! | `cover` | `{"kind":"frame","time":s}` or `{"kind":"picture","path":..}`; absent for none |
//!
//! A track, bottom of the stack first, the last one drawn on top: `id`,
//! `visible`, `muted`, and on a caption lane `captions`, the lane's look
//! (`{style, offsetY}`, as `setCaptionLook` takes it).
//!
//! A clip, every item on the showing timeline's tracks:
//!
//! | Field | What it is |
//! |---|---|
//! | `id`, `trackId` | the item and its lane |
//! | `kind` | `video`, `audio` (a recording, or a sound detached from a video), `image`, `text`, `shape`, `layer`, `soundLayer`, `transition`, `timeline` (a nested timeline), `unknown` |
//! | `mediaId` | the bin item it is cut from; absent when it is cut from none |
//! | `name` | what the lane calls it; the file's name when the item has none |
//! | `start`, `duration` | where it sits on the timeline and how long, seconds |
//! | `sourceStart` | seconds into the file where what it shows begins: the earlier end of the span it shows, whichever way it plays |
//! | `speed` | seconds of file per second of timeline: the clip's speed, or the average over a speed curve; its size, whichever way it plays |
//! | `volume` | its level, 1 as recorded |
//! | `text` | a title's words and look, as `updateClip`'s `text` takes it |
//! | `videoEffects` | the picture chain in order, as `updateClip` takes it; on a transition, its package first, with its knobs |
//! | `filters` | the sound chain, the same way |
//! | `scale`, `offsetX`, `offsetY`, `rotation`, `opacity` | the placement as it is without its keys: a factor over the fitted size, shares of the frame's width and height (right and down), clockwise degrees, 0 to 1 |
//! | `muted`, `flipH` | its own sound off; mirrored left to right |
//! | `fadeIn`, `fadeOut` | seconds |
//! | `keys` | each key of its placement, opacity and level: `property` (as `setClipKey` names it), `at` (a share of the clip, 0 at its start and 1 at its end; outside that for a key left beyond a trim), `value`, `ease` (a timing function's two control points) |
//! | `crop` | `left`, `top`, `right`, `bottom`, each a share cut off; absent for none |
//! | `masks` | the masks, top first, as `updateClip` takes them |
//! | `motion` | the entrance (`in`), exit (`out`) and loop (`loop`); absent for none |
//!
//! Left out of a clip's chains and masks is only what this build cannot
//! read: an entry kept from a later build, a mask of a kind it does not
//! know.

use std::collections::BTreeMap;

use concat_api::wire;
use concat_api::{EditorView, Response};
use concat_doc::Tick;
use concat_doc::model::{
    Animated, Content, Cover, Ease, Effect, Entry, Item, ItemId, Key, Lane, LaneId, Mask, MaskMode,
    MaskShape, MaskSource, Media, MediaId, MediaKind, Model, Motion, Sequence, SequenceId, Take,
    TimeMap,
};
use serde_json::{Map, Value, json};

/// The reply to a request that answered with `view`, the project's view
/// in it made from `document`: the phone's `{"result": ...}`.
pub(crate) fn reply(view: &EditorView, document: &Model) -> Value {
    let mut out = Map::new();
    out.insert("project".into(), project(document));
    out.insert("canUndo".into(), json!(view.can_undo));
    out.insert("canRedo".into(), json!(view.can_redo));
    out.insert(
        "settings".into(),
        serde_json::to_value(&view.settings).unwrap_or_default(),
    );
    if let Some(created) = &view.created_id {
        out.insert("createdId".into(), json!(created));
    }
    json!({ "result": out })
}

/// The reply when the view's document does not read: the edit stands, but
/// the phone cannot be shown it.
pub(crate) fn unread(why: String) -> Response {
    Response::Error(concat_api::ApiError::failed(format!(
        "the project could not be shown: {why}"
    )))
}

/// The project as the phone reads it: see [the module](self).
pub(crate) fn project(document: &Model) -> Value {
    let showing = document.view.showing;
    let timelines: Vec<Value> = document
        .sequences
        .iter()
        .map(|(id, sequence)| timeline(document, id, sequence, showing == Some(id)))
        .collect();
    let mut out = Map::new();
    if let Some(showing) = showing {
        out.insert("activeTimelineId".into(), json!(showing.to_string()));
    }
    out.insert("timelines".into(), Value::Array(timelines));
    out.insert(
        "media".into(),
        document
            .media
            .iter()
            .map(|(id, media)| bin_item(id, media))
            .collect(),
    );
    if let Some(cover) = cover(document) {
        out.insert("cover".into(), cover);
    }
    Value::Object(out)
}

fn timeline(document: &Model, id: SequenceId, sequence: &Sequence, showing: bool) -> Value {
    let mut out = Map::new();
    out.insert("id".into(), json!(id.to_string()));
    out.insert("name".into(), json!(sequence.name));
    out.insert(
        "video".into(),
        serde_json::to_value(wire::VideoSettings::of(&sequence.frame)).unwrap_or_default(),
    );
    if showing {
        // The index walks the stack from the bottom, as the app lists it.
        let lanes: Vec<LaneId> = document
            .lanes
            .index()
            .of(id)
            .map(|(lane, _)| lane)
            .collect();
        let tracks: Vec<Value> = lanes
            .iter()
            .filter_map(|lane| Some(track(*lane, document.lanes.get(*lane)?)))
            .collect();
        let clips: Vec<Value> = lanes
            .iter()
            .flat_map(|lane| document.items.index().on_lane(*lane))
            .filter_map(|(item, _, _)| Some(clip(document, item, document.items.get(item)?)))
            .collect();
        out.insert("tracks".into(), Value::Array(tracks));
        out.insert("clips".into(), Value::Array(clips));
    }
    Value::Object(out)
}

fn track(id: LaneId, lane: &Lane) -> Value {
    let mut out = json!({
        "id": id.to_string(),
        "visible": lane.visible,
        "muted": lane.muted,
    });
    if let Some(look) = &lane.captions {
        out["captions"] = serde_json::to_value(wire::CaptionLook {
            style: wire::TextStyle::of(&look.title),
            offset_y: look.offset_y,
        })
        .unwrap_or_default();
    }
    out
}

/// What an item is, in the app's word.
fn kind(document: &Model, item: &Item) -> &'static str {
    match &item.content {
        Content::Media(cut) => match document.media.get(cut.media).map(|media| &media.kind) {
            Some(MediaKind::Video) if cut.take == Take::Sound => "audio",
            Some(MediaKind::Video) => "video",
            Some(MediaKind::Audio) => "audio",
            Some(MediaKind::Image) => "image",
            Some(MediaKind::Unknown(_)) | None => "unknown",
        },
        Content::Title(_) => "text",
        Content::Shape(_) => "shape",
        Content::Layer => "layer",
        Content::SoundLayer => "soundLayer",
        Content::Transition(_) => "transition",
        Content::Nest(_) => "timeline",
        Content::Unknown { .. } => "unknown",
    }
}

/// The span of its file a cut shows, as (where it begins, seconds of file
/// per second of timeline): the earlier end of the span, and the size of
/// the pace, so a clip played backwards reads as the span it covers.
fn span(item: &Item, time: &TimeMap) -> (f64, f64) {
    let from = time.source_of(item.offset);
    let to = item.window_end().and_then(|end| time.source_of(end));
    let start = match (from, to) {
        (Some(a), Some(b)) => a.min(b).display_seconds(),
        (Some(a), None) | (None, Some(a)) => a.display_seconds(),
        (None, None) => 0.0,
    };
    let speed = match (time, from, to) {
        (TimeMap::Linear { speed, .. }, _, _) => speed.display().abs(),
        (_, Some(a), Some(b)) => (b.get() - a.get()).unsigned_abs() as f64 / item.len.get() as f64,
        _ => 1.0,
    };
    (start, speed)
}

fn clip(document: &Model, id: ItemId, item: &Item) -> Value {
    // A key's place as a share of the clip, the inverse of where
    // `setClipKey` puts one.
    let share = |at: Tick| (at.get() - item.offset.get()) as f64 / item.len.get() as f64;
    let mut out = Map::new();
    let mut put = |key: &str, value: Value| {
        out.insert(key.into(), value);
    };
    put("id", json!(id.to_string()));
    put("trackId", json!(item.lane.to_string()));
    put("kind", json!(kind(document, item)));
    put("start", json!(item.start.display_seconds()));
    put("duration", json!(item.len.dur().display_seconds()));
    let mut name = item.name.clone();
    let (mut source_start, mut speed) = (0.0, 1.0);
    match &item.content {
        Content::Media(cut) => {
            put("mediaId", json!(cut.media.to_string()));
            if name.is_empty()
                && let Some(media) = document.media.get(cut.media)
            {
                name.clone_from(&media.name);
            }
            (source_start, speed) = span(item, &cut.time);
        }
        Content::Nest(nest) => (source_start, speed) = span(item, &nest.time),
        Content::Title(title) => put(
            "text",
            serde_json::to_value(wire::TextStyle::of(title)).unwrap_or_default(),
        ),
        Content::Shape(_)
        | Content::Layer
        | Content::SoundLayer
        | Content::Transition(_)
        | Content::Unknown { .. } => {}
    }
    put("name", json!(name));
    put("sourceStart", json!(source_start));
    put("speed", json!(speed));
    put("volume", json!(item.sound.volume.value));

    let mut picture_chain: Vec<wire::AppliedFilter> = Vec::new();
    if let Content::Transition(effect) = &item.content {
        picture_chain.push(link(effect, &share));
    }
    picture_chain.extend(chain(&item.effects, &share));
    put(
        "videoEffects",
        serde_json::to_value(picture_chain).unwrap_or_default(),
    );
    put(
        "filters",
        serde_json::to_value(chain(&item.filters, &share)).unwrap_or_default(),
    );

    let picture = &item.picture;
    put("scale", json!(picture.scale.value));
    put("offsetX", json!(picture.offset_x.value));
    put("offsetY", json!(picture.offset_y.value));
    put("rotation", json!(picture.rotation.value));
    put("opacity", json!(picture.opacity.value));
    put("muted", json!(item.sound.muted));
    put("flipH", json!(picture.flip_h));
    put("fadeIn", json!(item.sound.fade_in.display_seconds()));
    put("fadeOut", json!(item.sound.fade_out.display_seconds()));
    let numbers: [(&str, &Animated<f64>); 8] = [
        ("scale", &picture.scale),
        ("offsetX", &picture.offset_x),
        ("offsetY", &picture.offset_y),
        ("rotation", &picture.rotation),
        ("stretchX", &picture.stretch_x),
        ("stretchY", &picture.stretch_y),
        ("opacity", &picture.opacity),
        ("volume", &item.sound.volume),
    ];
    let keys: Vec<Value> = numbers
        .into_iter()
        .flat_map(|(property, number)| {
            number.keys.iter().map(move |key| {
                json!({
                    "property": property,
                    "at": share(key.at),
                    "value": key.value,
                    "ease": ease(&key.ease),
                })
            })
        })
        .collect();
    put("keys", Value::Array(keys));
    if let Some(crop) = picture.crop {
        put(
            "crop",
            serde_json::to_value(wire::Crop {
                left: crop.left,
                top: crop.top,
                right: crop.right,
                bottom: crop.bottom,
            })
            .unwrap_or_default(),
        );
    }
    let masks: Vec<wire::Mask> = item
        .masks
        .iter()
        .filter_map(Entry::known)
        .filter_map(|held| mask(held, &share))
        .collect();
    put("masks", serde_json::to_value(masks).unwrap_or_default());
    if picture.motion != Motion::default() {
        put(
            "motion",
            serde_json::to_value(motion(&picture.motion)).unwrap_or_default(),
        );
    }
    Value::Object(out)
}

/// A key's easing as the wire spells it; a linear one for an easing this
/// build does not know, which plays as one.
fn ease(ease: &Ease) -> wire::KeyEase {
    match ease {
        Ease::Bezier { x1, y1, x2, y2 } => [*x1, *y1, *x2, *y2],
        Ease::Unknown(_) => [0.0, 0.0, 1.0, 1.0],
    }
}

fn keys(keys: &[Key<f64>], share: &impl Fn(Tick) -> f64) -> Vec<wire::ParamKey> {
    keys.iter()
        .map(|key| wire::ParamKey {
            at: share(key.at),
            value: key.value,
            ease: ease(&key.ease),
        })
        .collect()
}

/// An effect as a link of a chain: its package, the knobs set, whether it
/// runs, and the knobs that ride.
fn link(effect: &Effect, share: &impl Fn(Tick) -> f64) -> wire::AppliedFilter {
    wire::AppliedFilter {
        id: effect.package.clone(),
        params: effect
            .params
            .iter()
            .filter_map(|(name, param)| param.value.map(|value| (name.clone(), value)))
            .collect(),
        enabled: effect.enabled,
        keys: effect
            .params
            .iter()
            .filter(|(_, param)| param.is_keyed())
            .map(|(name, param)| (name.clone(), keys(&param.keys, share)))
            .collect(),
    }
}

fn chain(entries: &[Entry<Effect>], share: &impl Fn(Tick) -> f64) -> Vec<wire::AppliedFilter> {
    entries
        .iter()
        .filter_map(Entry::known)
        .map(|effect| link(effect, share))
        .collect()
}

/// A mask as `updateClip` takes it; none for one this build cannot spell.
fn mask(mask: &Mask, share: &impl Fn(Tick) -> f64) -> Option<wire::Mask> {
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
        MaskShape::Linear => wire::MaskShape::Linear,
        MaskShape::Mirror => wire::MaskShape::Mirror,
        MaskShape::Circle => wire::MaskShape::Circle,
        MaskShape::Rectangle => wire::MaskShape::Rectangle,
        MaskShape::Heart => wire::MaskShape::Heart,
        MaskShape::Star => wire::MaskShape::Star,
        MaskShape::Unknown(_) => return None,
    };
    let mode = match mask.mode {
        MaskMode::Add => wire::MaskMode::Add,
        MaskMode::Subtract => wire::MaskMode::Subtract,
        MaskMode::Intersect => wire::MaskMode::Intersect,
        MaskMode::None => wire::MaskMode::None,
        MaskMode::Unknown(_) => return None,
    };
    let numbers: [(&str, &Animated<f64>); 10] = [
        ("x", &mask.x),
        ("y", &mask.y),
        ("scale", &mask.scale),
        ("rotation", &mask.rotation),
        ("feather", &mask.feather),
        ("expand", &mask.expand),
        ("opacity", &mask.opacity),
        ("width", width),
        ("height", height),
        ("roundness", roundness),
    ];
    let ridden: BTreeMap<String, Vec<wire::ParamKey>> = numbers
        .into_iter()
        .filter(|(_, number)| number.is_keyed())
        .map(|(name, number)| (name.to_owned(), keys(&number.keys, share)))
        .collect();
    Some(wire::Mask {
        id: mask.id.to_string(),
        name: mask.name.clone(),
        mode,
        invert: mask.invert,
        source: wire::MaskSource::Shape {
            shape,
            width: Some(width.value),
            height: Some(height.value),
            roundness: roundness.value,
        },
        x: mask.x.value,
        y: mask.y.value,
        scale: mask.scale.value,
        rotation: mask.rotation.value,
        feather: mask.feather.value,
        expand: mask.expand.value,
        opacity: mask.opacity.value,
        keys: ridden,
    })
}

fn motion(motion: &Motion) -> wire::ClipMotion {
    let moving = |step: &concat_doc::model::Move| wire::MotionMove {
        kind: step.name.clone(),
        seconds: step.len.dur().display_seconds(),
    };
    wire::ClipMotion {
        enter: motion.enter.as_ref().map(moving),
        exit: motion.exit.as_ref().map(moving),
        repeat: motion.repeat.as_ref().map(|lap| wire::ClipLoop {
            kind: lap.name.clone(),
            seconds: lap.lap.dur().display_seconds(),
            amount: lap.amount,
        }),
    }
}

fn bin_item(id: MediaId, media: &Media) -> Value {
    let mut out = json!({
        "id": id.to_string(),
        "path": media.path,
        "name": media.name,
        "kind": match media.kind {
            MediaKind::Video => "video",
            MediaKind::Audio => "audio",
            MediaKind::Image => "image",
            MediaKind::Unknown(_) => "unknown",
        },
    });
    if let Some(length) = media.length {
        out["duration"] = json!(length.dur().display_seconds());
    }
    if let Some(size) = media.size {
        out["width"] = json!(size.width.get());
        out["height"] = json!(size.height.get());
    }
    out
}

/// The cover as `setCover` takes it; none for a cover of a kind this
/// build does not know, which shows as a frame early in the edit.
fn cover(document: &Model) -> Option<Value> {
    let cover = match document.cover.as_ref()? {
        Cover::Frame { into } => wire::Cover::Frame {
            time: into.display_seconds(),
        },
        Cover::Picture { media } => wire::Cover::Picture {
            path: document.media.get(*media)?.path.clone(),
        },
        Cover::Unknown(_) => return None,
    };
    serde_json::to_value(cover).ok()
}

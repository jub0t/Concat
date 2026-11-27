// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A project as an agent reads it: the timeline showing, its lanes and
//! clips in time order, the bin, and what the last edit made - a page of
//! JSON where the editor's view is the whole document.
//!
//! The view carries the document as its file has it, so it is read here
//! by the file's own reader: a view from the window's server reads
//! exactly as one from this process. Places and lengths are given in
//! seconds, as the tools take them. It decides nothing about the edit; it
//! only leaves out what an agent rarely needs: a field at its default, a
//! title's whole style, which `project_get` still returns in full.

use concat_doc::model::{
    Animated, Content, Effect, Entry, Item, ItemId, Lane, LaneId, Media, MediaId, MediaKind, Model,
    ShapeKind, Take, TimeMap,
};
use serde_json::{Map, Value, json};

/// The summary of an editor view (`project.get`'s reply, and every
/// edit's).
pub fn view(view: &Value) -> Value {
    let settings = &view["settings"];
    let rate_num = settings["rateNum"].as_f64().unwrap_or(0.0);
    let rate_den = settings["rateDen"].as_f64().unwrap_or(1.0).max(1.0);
    let mut summary = json!({
        "project": {
            "name": settings["name"],
            "width": settings["width"],
            "height": settings["height"],
            "fps": round(rate_num / rate_den, 3),
        },
        "canUndo": view["canUndo"],
        "canRedo": view["canRedo"],
    });
    let project = &view["project"];
    match concat_doc::file::read(project.to_string().as_bytes()) {
        Ok(opened) => {
            let doc = opened.document.tables();
            summary["timeline"] = timeline_of(doc, project);
            summary["media"] = doc
                .media
                .iter()
                .map(|(id, media)| media_of(id, media))
                .collect();
            if doc.sequences.len() > 1 {
                summary["timelines"] = doc
                    .sequences
                    .iter()
                    .map(|(id, sequence)| json!({ "id": id.to_string(), "name": sequence.name }))
                    .collect();
            }
        }
        Err(refused) => summary["unread"] = json!(refused.to_string()),
    }
    if let Some(created) = view.get("createdId").filter(|id| !id.is_null()) {
        summary["createdId"] = created.clone();
    }
    summary
}

/// The timeline showing: its lanes from the top down, and its clips in
/// time order.
fn timeline_of(doc: &Model, project: &Value) -> Value {
    let Some(showing) = doc.view.showing else {
        return Value::Null;
    };
    let Some(sequence) = doc.sequences.get(showing) else {
        return Value::Null;
    };
    // The lanes are kept from the bottom of the stack up; a person reads
    // them from the top down.
    let mut lanes: Vec<LaneId> = doc.lanes.index().of(showing).map(|(id, _)| id).collect();
    lanes.reverse();
    let mut clips: Vec<(ItemId, &Item)> = lanes
        .iter()
        .flat_map(|lane| doc.items.index().on_lane(*lane))
        .filter_map(|(id, _, _)| Some((id, &**doc.items.get(id)?)))
        .collect();
    clips.sort_by_key(|(_, item)| item.start);
    let end = clips
        .iter()
        .map(|(_, item)| item.start + item.len)
        .max()
        .map_or(0.0, |end| end.display_seconds());
    json!({
        "id": showing.to_string(),
        "name": sequence.name,
        "duration": round(end, 3),
        "tracks": lanes
            .iter()
            .enumerate()
            .filter_map(|(index, id)| Some(track_of(index, *id, doc.lanes.get(*id)?)))
            .collect::<Vec<_>>(),
        "clips": clips
            .into_iter()
            .map(|(id, item)| clip_of(doc, project, id, item))
            .collect::<Vec<_>>(),
    })
}

fn track_of(index: usize, id: LaneId, lane: &Lane) -> Value {
    let mut out = json!({ "id": id.to_string(), "index": index });
    if !lane.visible {
        out["hidden"] = json!(true);
    }
    if lane.muted {
        out["muted"] = json!(true);
    }
    out
}

/// What an item is, in a word.
fn kind_of(doc: &Model, item: &Item) -> &'static str {
    match &item.content {
        Content::Media(cut) => match doc.media.get(cut.media).map(|media| &media.kind) {
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

fn shape_name(kind: &ShapeKind) -> &'static str {
    match kind {
        ShapeKind::Square => "square",
        ShapeKind::Circle => "circle",
        ShapeKind::Triangle => "triangle",
        ShapeKind::Parallelogram => "parallelogram",
        ShapeKind::Trapezoid => "trapezoid",
        ShapeKind::Line => "line",
        ShapeKind::Arrow => "arrow",
        ShapeKind::Polygon => "polygon",
        ShapeKind::Star => "star",
        ShapeKind::Heart => "heart",
        ShapeKind::Unknown(_) => "unknown",
    }
}

/// The packages of a chain's effects, in order.
fn packages(chain: &[Entry<Effect>]) -> Vec<Value> {
    chain
        .iter()
        .filter_map(|entry| match entry {
            Entry::Known(effect) => Some(json!(effect.package)),
            Entry::Unread(_) => None,
        })
        .collect()
}

/// One clip: where it is, what it is, and whatever about it is not the
/// default.
fn clip_of(doc: &Model, project: &Value, id: ItemId, item: &Item) -> Value {
    let start = item.start.display_seconds();
    let duration = item.len.dur().display_seconds();
    let mut out = Map::new();
    out.insert("id".into(), json!(id.to_string()));
    out.insert("kind".into(), json!(kind_of(doc, item)));
    out.insert("track".into(), json!(item.lane.to_string()));
    out.insert("start".into(), json!(round(start, 4)));
    out.insert("duration".into(), json!(round(duration, 4)));
    out.insert("end".into(), json!(round(start + duration, 4)));
    if !item.name.is_empty() {
        out.insert("name".into(), json!(item.name));
    }
    let mut number = |key: &str, value: f64, default: f64| {
        if (value - default).abs() > 1e-9 {
            out.insert(key.into(), json!(round(value, 4)));
        }
    };
    match &item.content {
        Content::Media(cut) => {
            match &cut.time {
                TimeMap::Linear { speed, .. } => number("speed", speed.display(), 1.0),
                TimeMap::Keyed { .. } | TimeMap::Unknown(_) => {}
            }
            let from = cut
                .time
                .source_at(item.offset)
                .map_or(0.0, |at| at.display_seconds());
            out.insert("media".into(), json!(cut.media.to_string()));
            out.insert("sourceStart".into(), json!(round(from, 4)));
            if matches!(cut.time, TimeMap::Keyed { .. }) {
                out.insert("speedCurve".into(), json!(true));
            }
        }
        Content::Title(title) => {
            out.insert("text".into(), json!(title.content));
            // Words in a look of their own, as the document has them:
            // characters into the text, and only the fields each overrides.
            if !title.runs.is_empty() {
                let runs = &project["items"][id.to_string()]["content"]["title"]["runs"];
                out.insert("textRuns".into(), runs.clone());
            }
        }
        Content::Shape(style) => {
            out.insert("shape".into(), json!(shape_name(&style.kind)));
        }
        _ => {}
    }
    let picture = &item.picture;
    let sound = &item.sound;
    let mut number = |key: &str, value: f64, default: f64| {
        if (value - default).abs() > 1e-9 {
            out.insert(key.into(), json!(round(value, 4)));
        }
    };
    number("volume", sound.volume.value, 1.0);
    number("opacity", picture.opacity.value, 1.0);
    number("scale", picture.scale.value, 1.0);
    number("offsetX", picture.offset_x.value, 0.0);
    number("offsetY", picture.offset_y.value, 0.0);
    number("rotation", picture.rotation.value, 0.0);
    number("stretchX", picture.stretch_x.value, 1.0);
    number("stretchY", picture.stretch_y.value, 1.0);
    number("fadeIn", sound.fade_in.display_seconds(), 0.0);
    number("fadeOut", sound.fade_out.display_seconds(), 0.0);
    if sound.muted {
        out.insert("muted".into(), json!(true));
    }
    for (chain, name) in [(&item.effects, "effects"), (&item.filters, "audioFilters")] {
        let ids = packages(chain);
        if !ids.is_empty() {
            out.insert(name.into(), Value::Array(ids));
        }
    }
    if let Some(cut_in) = &item.cut_in {
        out.insert(
            "transitionIn".into(),
            json!({
                "id": cut_in.effect.package,
                "duration": round(cut_in.len.dur().display_seconds(), 4),
            }),
        );
    }
    let keyed = |value: &Animated<f64>| !value.keys.is_empty();
    let keyframed: Vec<&str> = [
        ("scale", keyed(&picture.scale)),
        ("offsetX", keyed(&picture.offset_x)),
        ("offsetY", keyed(&picture.offset_y)),
        ("rotation", keyed(&picture.rotation)),
        ("stretchX", keyed(&picture.stretch_x)),
        ("stretchY", keyed(&picture.stretch_y)),
        ("opacity", keyed(&picture.opacity)),
        ("volume", keyed(&sound.volume)),
    ]
    .into_iter()
    .filter_map(|(name, has)| has.then_some(name))
    .collect();
    if !keyframed.is_empty() {
        out.insert("keyframed".into(), json!(keyframed));
    }
    if picture.cutout.is_some() {
        out.insert("cutout".into(), json!(true));
    }
    Value::Object(out)
}

fn media_of(id: MediaId, media: &Media) -> Value {
    let mut out = json!({
        "id": id.to_string(),
        "name": media.name,
        "kind": match media.kind {
            MediaKind::Video => "video",
            MediaKind::Audio => "audio",
            MediaKind::Image => "image",
            MediaKind::Unknown(_) => "unknown",
        },
        "path": media.path,
    });
    if let Some(length) = media.length {
        out["duration"] = json!(round(length.dur().display_seconds(), 3));
    }
    if let Some(size) = media.size {
        out["width"] = json!(size.width);
        out["height"] = json!(size.height);
    }
    if media.has_sound {
        out["hasAudio"] = json!(true);
    }
    if media.placeholder {
        out["placeholder"] = json!(true);
    }
    out
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_reads_as_the_timeline_showing_in_time_order() {
        let project: Value = serde_json::from_str(
            r##"{"concat-project":1,
"mint":{"0":9},
"view":{"showing":"tl2","tabs":["tl1","tl2"]},
"media":{
  "m3":{"path":"/a.mp4","name":"a.mp4","length":8820000000,"rate":{"num":30,"den":1},"size":{"width":1920,"height":1080},"has_sound":true}
},
"sequences":{
  "tl1":{"name":"One","frame":{"width":1920,"height":1080,"rate":{"num":30000,"den":1001}}},
  "tl2":{"name":"Two","frame":{"width":1920,"height":1080,"rate":{"num":30000,"den":1001}}}
},
"lanes":{
  "t4":{"sequence":"tl2","order":"i","visible":false,"muted":true},
  "t5":{"sequence":"tl2","order":"r"}
},
"items":{
  "c6":{"lane":"t4","start":0,"len":3528000000,"content":{"media":{"media":"m3","time":{"linear":{"src_in":705600000,"speed":{"num":2,"den":1}}}}},"picture":{"scale":{"value":1,"keys":[{"at":0,"value":1},{"at":705600000,"value":2}]}},"effects":[{"id":"fx7","package":"concat.warm"}]},
  "c8":{"lane":"t5","start":2822400000,"len":1411200000,"content":{"title":{"content":"Hi"}},"name":"Hi","picture":{"opacity":0.5}}
}
}"##,
        )
        .expect("the document is JSON");
        let view = json!({
            "project": project,
            "canUndo": true, "canRedo": false,
            "settings": { "name": "P", "width": 1920, "height": 1080, "rateNum": 30000, "rateDen": 1001 },
            "createdId": "c8"
        });
        let summary = super::view(&view);
        assert!(summary.get("unread").is_none(), "{summary}");
        assert_eq!(summary["project"]["fps"], 29.97);
        assert_eq!(summary["timeline"]["id"], "tl2");
        assert_eq!(summary["timeline"]["duration"], 6.0);
        assert_eq!(summary["createdId"], "c8");
        assert_eq!(summary["timelines"].as_array().map(Vec::len), Some(2));
        let clips = summary["timeline"]["clips"].as_array().expect("clips");
        assert_eq!(clips[0]["id"], "c6", "time order");
        assert_eq!(clips[0]["kind"], "video");
        assert_eq!(clips[0]["speed"], 2.0);
        assert_eq!(clips[0]["sourceStart"], 1.0);
        assert_eq!(clips[0]["duration"], 5.0);
        assert_eq!(clips[0]["effects"], json!(["concat.warm"]));
        assert_eq!(clips[0]["keyframed"], json!(["scale"]));
        assert!(clips[0].get("volume").is_none(), "a default is left out");
        assert_eq!(clips[1]["text"], "Hi");
        assert_eq!(clips[1]["kind"], "text");
        assert_eq!(clips[1]["opacity"], 0.5);
        assert!(clips[1].get("media").is_none());
        // From the top of the stack down.
        let tracks = summary["timeline"]["tracks"].as_array().expect("tracks");
        assert_eq!(tracks[0], json!({ "id": "t5", "index": 0 }));
        assert_eq!(
            tracks[1],
            json!({ "id": "t4", "index": 1, "hidden": true, "muted": true })
        );
        assert_eq!(summary["media"][0]["hasAudio"], true);
        assert_eq!(summary["media"][0]["duration"], 12.5);
    }

    #[test]
    fn a_project_that_does_not_read_says_why() {
        let summary = super::view(&json!({ "project": { "nothing": true } }));
        assert!(summary["unread"].is_string(), "{summary}");
    }
}

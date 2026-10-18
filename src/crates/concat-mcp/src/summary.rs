// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! A project as an agent reads it: the active timeline's lanes and clips
//! in time order, the bin, and what the last edit made - a page of JSON
//! where the editor's view is the whole document.
//!
//! Read from the view's JSON, not its Rust types, so it reads a view from
//! the window's server exactly as one from this process. It decides
//! nothing about the edit; it only leaves out what an agent rarely needs:
//! a field at its default, a title's whole style, which `project_get`
//! still returns in full.

use serde_json::{Map, Value, json};

/// The summary of an editor view (`project.get`'s reply, and every
/// edit's).
pub fn view(view: &Value) -> Value {
    let project = &view["project"];
    let settings = &view["settings"];
    let active = project["activeTimelineId"].as_str().unwrap_or_default();
    let timelines = project["timelines"].as_array().cloned().unwrap_or_default();
    let timeline = timelines
        .iter()
        .find(|timeline| timeline["id"].as_str() == Some(active))
        .or_else(|| timelines.first())
        .cloned()
        .unwrap_or(Value::Null);

    let mut clips: Vec<&Value> = timeline["clips"]
        .as_array()
        .map(|clips| clips.iter().collect())
        .unwrap_or_default();
    clips.sort_by(|left, right| {
        let start = |clip: &Value| clip["start"].as_f64().unwrap_or(0.0);
        start(left).total_cmp(&start(right))
    });
    let length = clips
        .iter()
        .map(|clip| clip["start"].as_f64().unwrap_or(0.0) + clip["duration"].as_f64().unwrap_or(0.0))
        .fold(0.0_f64, f64::max);

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
        "timeline": {
            "id": timeline["id"],
            "name": timeline["name"],
            "duration": round(length, 3),
            "tracks": timeline["tracks"]
                .as_array()
                .map(|tracks| tracks.iter().enumerate().map(|(index, track)| track_of(index, track)).collect::<Vec<_>>())
                .unwrap_or_default(),
            "clips": clips.into_iter().map(clip_of).collect::<Vec<_>>(),
        },
        "media": project["media"]
            .as_array()
            .map(|media| media.iter().map(media_of).collect::<Vec<_>>())
            .unwrap_or_default(),
    });
    if timelines.len() > 1 {
        summary["timelines"] = timelines
            .iter()
            .map(|timeline| json!({ "id": timeline["id"], "name": timeline["name"] }))
            .collect();
    }
    if let Some(created) = view.get("createdId").filter(|id| !id.is_null()) {
        summary["createdId"] = created.clone();
    }
    summary
}

fn track_of(index: usize, track: &Value) -> Value {
    let mut out = json!({ "id": track["id"], "index": index });
    if track["visible"] == json!(false) {
        out["hidden"] = json!(true);
    }
    if track["muted"] == json!(true) {
        out["muted"] = json!(true);
    }
    out
}

/// One clip: where it is, what it is, and whatever about it is not the
/// default.
fn clip_of(clip: &Value) -> Value {
    let start = clip["start"].as_f64().unwrap_or(0.0);
    let duration = clip["duration"].as_f64().unwrap_or(0.0);
    let mut out = Map::new();
    out.insert("id".into(), clip["id"].clone());
    out.insert("kind".into(), clip["kind"].clone());
    out.insert("track".into(), clip["trackId"].clone());
    out.insert("start".into(), json!(round(start, 4)));
    out.insert("duration".into(), json!(round(duration, 4)));
    out.insert("end".into(), json!(round(start + duration, 4)));
    if let Some(name) = clip["name"].as_str().filter(|name| !name.is_empty()) {
        out.insert("name".into(), json!(name));
    }
    if let Some(media) = clip["mediaId"].as_str().filter(|media| !media.is_empty()) {
        out.insert("media".into(), json!(media));
        out.insert("sourceStart".into(), json!(round(clip["sourceStart"].as_f64().unwrap_or(0.0), 4)));
    }
    if let Some(text) = clip.pointer("/text/content") {
        out.insert("text".into(), text.clone());
    }
    if let Some(shape) = clip.pointer("/shape/kind") {
        out.insert("shape".into(), shape.clone());
    }
    for (key, default) in [
        ("speed", 1.0),
        ("volume", 1.0),
        ("opacity", 1.0),
        ("scale", 1.0),
        ("offsetX", 0.0),
        ("offsetY", 0.0),
        ("rotation", 0.0),
        ("fadeIn", 0.0),
        ("fadeOut", 0.0),
    ] {
        if let Some(value) = clip[key].as_f64()
            && (value - default).abs() > 1e-9
        {
            out.insert(key.into(), json!(round(value, 4)));
        }
    }
    if clip["muted"] == json!(true) {
        out.insert("muted".into(), json!(true));
    }
    for (key, name) in [("videoEffects", "effects"), ("filters", "audioFilters")] {
        let ids: Vec<Value> = clip[key]
            .as_array()
            .map(|chain| chain.iter().map(|link| link["id"].clone()).collect())
            .unwrap_or_default();
        if !ids.is_empty() {
            out.insert(name.into(), Value::Array(ids));
        }
    }
    if let Some(transition) = clip.get("transitionIn").filter(|value| !value.is_null()) {
        out.insert("transitionIn".into(), transition.clone());
    }
    if let Some(keys) = clip["keys"].as_array().filter(|keys| !keys.is_empty()) {
        let mut properties: Vec<Value> = Vec::new();
        for key in keys {
            if !properties.contains(&key["property"]) {
                properties.push(key["property"].clone());
            }
        }
        out.insert("keyframed".into(), Value::Array(properties));
    }
    if clip.get("cutout").is_some_and(|cutout| !cutout.is_null()) {
        out.insert("cutout".into(), json!(true));
    }
    Value::Object(out)
}

fn media_of(item: &Value) -> Value {
    let mut out = json!({
        "id": item["id"],
        "name": item["name"],
        "kind": item["kind"],
        "path": item["path"],
    });
    if let Some(duration) = item["duration"].as_f64() {
        out["duration"] = json!(round(duration, 3));
    }
    if item["width"].is_number() {
        out["width"] = item["width"].clone();
        out["height"] = item["height"].clone();
    }
    if item["hasAudio"] == json!(true) {
        out["hasAudio"] = json!(true);
    }
    if item["placeholder"] == json!(true) {
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
    fn a_view_reads_as_the_active_timeline_in_time_order() {
        let view = json!({
            "project": {
                "media": [{ "id": "m1", "name": "a.mp4", "kind": "video", "path": "/a.mp4",
                            "duration": 12.5, "width": 1920, "height": 1080, "hasAudio": true }],
                "activeTimelineId": "tl2",
                "timelines": [
                    { "id": "tl1", "name": "One", "tracks": [], "clips": [] },
                    { "id": "tl2", "name": "Two",
                      "tracks": [{ "id": "t1", "visible": true, "muted": false },
                                 { "id": "t2", "visible": false, "muted": true }],
                      "clips": [
                        { "id": "c2", "kind": "text", "trackId": "t1", "start": 4.0, "duration": 2.0,
                          "name": "Hi", "mediaId": "", "volume": 1.0, "opacity": 0.5, "speed": 1.0,
                          "text": { "content": "Hi" } },
                        { "id": "c1", "kind": "video", "trackId": "t2", "start": 0.0, "duration": 5.0,
                          "mediaId": "m1", "sourceStart": 1.0, "volume": 1.0, "opacity": 1.0, "speed": 2.0,
                          "videoEffects": [{ "id": "concat.warm" }],
                          "keys": [{ "property": "scale" }, { "property": "scale" }] }
                      ] }
                ]
            },
            "canUndo": true, "canRedo": false,
            "settings": { "name": "P", "width": 1920, "height": 1080, "rateNum": 30000, "rateDen": 1001 },
            "createdId": "c2"
        });
        let summary = super::view(&view);
        assert_eq!(summary["project"]["fps"], 29.97);
        assert_eq!(summary["timeline"]["id"], "tl2");
        assert_eq!(summary["timeline"]["duration"], 6.0);
        assert_eq!(summary["createdId"], "c2");
        assert_eq!(summary["timelines"].as_array().map(Vec::len), Some(2));
        let clips = summary["timeline"]["clips"].as_array().expect("clips");
        assert_eq!(clips[0]["id"], "c1", "time order");
        assert_eq!(clips[0]["speed"], 2.0);
        assert_eq!(clips[0]["effects"], json!(["concat.warm"]));
        assert_eq!(clips[0]["keyframed"], json!(["scale"]));
        assert!(clips[0].get("volume").is_none(), "a default is left out");
        assert_eq!(clips[1]["text"], "Hi");
        assert_eq!(clips[1]["opacity"], 0.5);
        assert!(clips[1].get("media").is_none());
        let tracks = summary["timeline"]["tracks"].as_array().expect("tracks");
        assert_eq!(tracks[1], json!({ "id": "t2", "index": 1, "hidden": true, "muted": true }));
        assert_eq!(summary["media"][0]["hasAudio"], true);
    }
}

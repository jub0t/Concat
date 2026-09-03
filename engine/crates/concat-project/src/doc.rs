// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Reading and writing `concat.json`.
//!
//! Projects saved before the renames carry the same document as
//! `wolfcut.json`. The name on disk is the host's concern:
//! it keeps whichever name a project already has, and this module never
//! looks at a filename.
//!
//! The reader began as a port of the UI's `persist.ts` (deleted; decision
//! 0007) and its tolerance rules are the contract with every document
//! already on disk: every field defaults rather than being trusted, clips whose
//! track or media vanished are dropped, text clips survive without media,
//! legacy flat documents load as a single timeline. A hand-edited or older
//! file must degrade to something openable, never to a load error.
//!
//! The writer produces the same structure `toDocument` writes - including the
//! flat `tracks`/`clips` mirror of the active timeline that keeps documents
//! openable in builds that predate multiple timelines.

use serde_json::{Map, Value, json};

use crate::model::{
    AppliedFilter, Clip, ClipKind, CustomFont, MediaItem, MediaKind, Project, TextAlign,
    TextStyle, Timeline, Track, Transition,
};

/// Bumped only when a change cannot be absorbed by defaulting.
const DOCUMENT_VERSION: u64 = 1;

fn text(value: Option<&Value>, fallback: &str) -> String {
    value.and_then(Value::as_str).unwrap_or(fallback).to_owned()
}

fn number(value: Option<&Value>, fallback: f64) -> f64 {
    value.and_then(Value::as_f64).filter(|n| n.is_finite()).unwrap_or(fallback)
}

fn flag(value: Option<&Value>, fallback: bool) -> bool {
    value.and_then(Value::as_bool).unwrap_or(fallback)
}

fn opt_u32(value: Option<&Value>) -> Option<u32> {
    value.and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok())
}

fn opt_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

fn read_media(raw: Option<&Value>) -> Vec<MediaItem> {
    let Some(entries) = raw.and_then(Value::as_array) else { return Vec::new() };
    entries
        .iter()
        .filter_map(|entry| {
            let id = entry.get("id")?.as_str()?.to_owned();
            let path = entry.get("path")?.as_str()?.to_owned();
            let kind = match entry.get("kind").and_then(Value::as_str) {
                Some("audio") => MediaKind::Audio,
                Some("image") => MediaKind::Image,
                _ => MediaKind::Video,
            };
            Some(MediaItem {
                name: text(entry.get("name"), &path),
                duration: entry.get("duration").and_then(Value::as_f64),
                kind,
                width: opt_u32(entry.get("width")),
                height: opt_u32(entry.get("height")),
                frame_rate: entry.get("frameRate").and_then(Value::as_f64),
                frame_rate_fraction: opt_string(entry.get("frameRateFraction")),
                video_codec: opt_string(entry.get("videoCodec")),
                audio_codec: opt_string(entry.get("audioCodec")),
                has_audio: flag(entry.get("hasAudio"), false),
                placeholder: flag(entry.get("placeholder"), false),
                id,
                path,
            })
        })
        .collect()
}

fn read_tracks(raw: Option<&Value>) -> Vec<Track> {
    let Some(entries) = raw.and_then(Value::as_array) else { return Vec::new() };
    entries
        .iter()
        .filter_map(|entry| {
            let id = entry.get("id")?.as_str()?.to_owned();
            Some(Track {
                name: text(entry.get("name"), &id),
                visible: flag(entry.get("visible"), true),
                muted: flag(entry.get("muted"), false),
                id,
            })
        })
        .collect()
}

fn read_filters(raw: Option<&Value>) -> Vec<AppliedFilter> {
    let Some(entries) = raw.and_then(Value::as_array) else { return Vec::new() };
    entries
        .iter()
        .filter_map(|entry| {
            let id = entry.get("id")?.as_str()?.to_owned();
            let params = entry
                .get("params")
                .and_then(Value::as_object)
                .map(|map| {
                    map.iter()
                        .filter_map(|(key, value)| Some((key.clone(), value.as_f64()?)))
                        .collect()
                })
                .unwrap_or_default();
            Some(AppliedFilter { id, params, enabled: flag(entry.get("enabled"), true) })
        })
        .collect()
}

fn read_text_style(raw: Option<&Value>) -> TextStyle {
    let base = TextStyle::default();
    let Some(style) = raw.filter(|value| value.is_object()) else { return base };
    TextStyle {
        content: text(style.get("content"), &base.content),
        font_family: text(style.get("fontFamily"), &base.font_family),
        // Clamped, not just defaulted: a hand-edited 0 would render an
        // invisible title.
        font_size: number(style.get("fontSize"), base.font_size).clamp(0.01, 1.0),
        font_weight: number(style.get("fontWeight"), base.font_weight).clamp(100.0, 900.0),
        italic: flag(style.get("italic"), base.italic),
        color: text(style.get("color"), &base.color),
        align: match style.get("align").and_then(Value::as_str) {
            Some("left") => TextAlign::Left,
            Some("right") => TextAlign::Right,
            _ => TextAlign::Center,
        },
        opacity: number(style.get("opacity"), base.opacity).clamp(0.0, 1.0),
        stroke_width: number(style.get("strokeWidth"), base.stroke_width).max(0.0),
        stroke_color: text(style.get("strokeColor"), &base.stroke_color),
        shadow: flag(style.get("shadow"), base.shadow),
        background: text(style.get("background"), &base.background),
        line_height: number(style.get("lineHeight"), base.line_height).max(0.5),
        tracking: number(style.get("tracking"), base.tracking),
    }
}

fn read_clips(raw: Option<&Value>, tracks: &[Track], media: &[MediaItem]) -> Vec<Clip> {
    let Some(entries) = raw.and_then(Value::as_array) else { return Vec::new() };
    entries
        .iter()
        .filter_map(|entry| {
            let id = entry.get("id")?.as_str()?.to_owned();
            let track_id = entry.get("trackId")?.as_str()?.to_owned();
            // A clip whose track vanished has nowhere to live.
            if !tracks.iter().any(|track| track.id == track_id) {
                return None;
            }

            let is_text = entry.get("kind").and_then(Value::as_str) == Some("text");
            let media_id = if is_text {
                String::new()
            } else {
                let media_id = entry.get("mediaId")?.as_str()?.to_owned();
                // Only the kinds that come from the bin are dropped when
                // their source is gone.
                if !media.iter().any(|item| item.id == media_id) {
                    return None;
                }
                media_id
            };

            let kind = if is_text {
                ClipKind::Text
            } else {
                match entry.get("kind").and_then(Value::as_str) {
                    Some("audio") => ClipKind::Audio,
                    Some("image") => ClipKind::Image,
                    _ => ClipKind::Video,
                }
            };

            Some(Clip {
                name: text(entry.get("name"), "clip"),
                kind,
                start: number(entry.get("start"), 0.0).max(0.0),
                duration: number(entry.get("duration"), 1.0).max(0.01),
                source_start: number(entry.get("sourceStart"), 0.0).max(0.0),
                volume: number(entry.get("volume"), 1.0).max(0.0),
                fade_in: number(entry.get("fadeIn"), 0.0).max(0.0),
                fade_out: number(entry.get("fadeOut"), 0.0).max(0.0),
                scale: number(entry.get("scale"), 1.0).max(0.05),
                offset_x: number(entry.get("offsetX"), 0.0),
                offset_y: number(entry.get("offsetY"), 0.0),
                rotation: number(entry.get("rotation"), 0.0),
                // Clamped: a hand-edited 2 would export differently from how
                // the preview clamps it on screen.
                opacity: number(entry.get("opacity"), 1.0).clamp(0.0, 1.0),
                speed: number(entry.get("speed"), 1.0).clamp(0.0625, 16.0),
                preserve_pitch: flag(entry.get("preservePitch"), true),
                filters: read_filters(entry.get("filters")),
                video_effects: read_filters(entry.get("videoEffects")),
                muted: flag(entry.get("muted"), false).then_some(true),
                detached_from: opt_string(entry.get("detachedFrom")),
                transition_in: entry.get("transitionIn").and_then(|transition| {
                    Some(Transition {
                        id: transition.get("id")?.as_str()?.to_owned(),
                        duration: number(transition.get("duration"), 1.0).max(0.1),
                    })
                }),
                text: is_text.then(|| read_text_style(entry.get("text"))),
                id,
                track_id,
                media_id,
            })
        })
        .collect()
}

/// Rebuilds a project from a document. Returns None only when there is
/// nothing recognisable to load at all.
pub fn from_document(document: &Value) -> Option<Project> {
    if !document.is_object() {
        return None;
    }
    let media = read_media(document.get("media"));

    // The timelines array is the source of truth when present and usable; a
    // file from before multiple timelines loads its flat fields as the one
    // timeline they always were.
    let mut timelines: Vec<Timeline> = Vec::new();
    if let Some(entries) = document.get("timelines").and_then(Value::as_array) {
        for entry in entries {
            let Some(id) = entry.get("id").and_then(Value::as_str) else { continue };
            if timelines.iter().any(|existing| existing.id == id) {
                continue;
            }
            let tracks = read_tracks(entry.get("tracks"));
            if tracks.is_empty() {
                continue;
            }
            let clips = read_clips(entry.get("clips"), &tracks, &media);
            timelines.push(Timeline {
                id: id.to_owned(),
                name: text(entry.get("name"), "Timeline"),
                tracks,
                clips,
            });
        }
    }
    if timelines.is_empty() {
        let tracks = read_tracks(document.get("tracks"));
        if tracks.is_empty() {
            return None;
        }
        let clips = read_clips(document.get("clips"), &tracks, &media);
        timelines.push(Timeline {
            id: "TL1".to_owned(),
            name: "Timeline 1".to_owned(),
            tracks,
            clips,
        });
    }

    let active_timeline_id = document
        .get("activeTimelineId")
        .and_then(Value::as_str)
        .filter(|id| timelines.iter().any(|timeline| timeline.id == *id))
        .unwrap_or(&timelines[0].id)
        .to_owned();

    let fonts = document
        .get("fonts")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    Some(CustomFont {
                        family: entry.get("family")?.as_str()?.to_owned(),
                        path: entry.get("path")?.as_str()?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(Project { media, fonts, timelines, active_timeline_id })
}

/// Settings the host manages around the edit: the manifest's identity fields.
#[derive(Clone, Debug)]
pub struct DocumentSettings {
    /// The project's display name, written as the document's `name` field.
    pub name: String,
    /// Output frame width in pixels.
    pub width: u32,
    /// Output frame height in pixels.
    pub height: u32,
    /// Numerator of the output frame rate, e.g. 30000 for 29.97fps.
    pub rate_num: i64,
    /// Denominator of the output frame rate, e.g. 1001 for 29.97fps.
    pub rate_den: i64,
}

/// Builds the full `concat.json` document.
pub fn to_document(settings: &DocumentSettings, project: &Project) -> Value {
    let active = project.active();
    let mut document = Map::new();
    document.insert("concat".into(), json!("0.1.0"));
    document.insert("version".into(), json!(DOCUMENT_VERSION));
    document.insert("name".into(), json!(settings.name));
    document.insert(
        "video".into(),
        json!({
            "width": settings.width,
            "height": settings.height,
            "rateNum": settings.rate_num,
            "rateDen": settings.rate_den,
        }),
    );
    document.insert("media".into(), serde_json::to_value(&project.media).expect("serialises"));
    // The flat mirror of the active timeline, for builds that predate
    // multiple timelines.
    document.insert("tracks".into(), serde_json::to_value(&active.tracks).expect("serialises"));
    document.insert("clips".into(), serde_json::to_value(&active.clips).expect("serialises"));
    document.insert("fonts".into(), serde_json::to_value(&project.fonts).expect("serialises"));
    document
        .insert("timelines".into(), serde_json::to_value(&project.timelines).expect("serialises"));
    document.insert("activeTimelineId".into(), json!(project.active_timeline_id));
    Value::Object(document)
}

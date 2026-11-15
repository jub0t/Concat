// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The document: `concat.json` in and out.
//!
//! Reading is three steps, each with one job:
//!
//! 1. **Migrate.** The JSON is walked through [`MIGRATIONS`], oldest shape
//!    first, until it has the shape this build writes. A document from a
//!    later build than this one is refused whole rather than read in part
//!    and then saved over.
//! 2. **Parse.** The model's own `serde` derives read it, a media item, a
//!    timeline and a clip at a time ([`crate::unread`]). Every struct
//!    defaults a missing field, and every field this build does not know
//!    lands in the struct's `extra` map, so it is written back out
//!    untouched. An entry that does not parse - a clip of a kind from a
//!    newer build, a keyframe among readable ones - and one with nowhere to
//!    live - a clip whose lane or media is gone, a timeline with no lanes -
//!    is not dropped: it is held, as the JSON it was, in
//!    [`Project::unread`], and written back where it came from.
//! 3. **Settle.** What no derive can decide: every number is pulled into
//!    the range a command holds it to ([`Clip::tidy`]), and the active
//!    timeline is one that exists.
//!
//! The standing rule through all three: a hand-edited or older file
//! degrades to something openable, and what this build cannot read stays
//! in the file. Nothing short of a document that is not an object, or that
//! has no timeline this build can read, fails the load.
//!
//! Writing is the derives again, the held entries put back, plus a mirror
//! of the active timeline in the flat `tracks` / `clips` / `video` fields
//! that builds from before multiple timelines read, so a file saved here
//! still opens there.

use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::model::{Clip, ColorSpace, MediaItem, Project, Timeline, VideoSettings};

/// Bumped only when a change cannot be absorbed by defaulting. Read back
/// on load: a document from a later version is refused whole rather than
/// read in part and then saved over. Every bump adds a step to
/// [`MIGRATIONS`].
///
/// 2: a project's background colour and transparent frame, styled ranges
/// in a title, sound fades, linkage and transition clips. Every one of
/// them defaults, so a version 1 document reads unchanged. But 0.2.8 and
/// older, which read version 1, open a version 2 document and drop what
/// they do not know, and their next autosave deletes it. They refuse a
/// version 2 document whole ("saved by a newer Concat"), which keeps it
/// intact.
pub const DOCUMENT_VERSION: u64 = 2;

/// The version a document says it is. Documents from before the field
/// existed are version 1: that is the version their shape was given.
pub fn document_version(document: &Value) -> u64 {
    document.get("version").and_then(Value::as_u64).unwrap_or(1)
}

/// One step of the reader's migration table: a name for the log, and the
/// rewrite. Each step is idempotent - a document already in the shape it
/// produces passes through unchanged - so the table is simply applied in
/// order, and a document at any age comes out at the current shape.
struct Migration {
    #[allow(dead_code)]
    name: &'static str,
    apply: fn(&mut Map<String, Value>),
}

/// The migration table, oldest shape first. Add to the end; never reorder
/// or remove, because a document written at any point in the project's
/// history has to walk the whole way up.
const MIGRATIONS: &[Migration] = &[
    Migration {
        name: "timelines from the flat fields",
        apply: lift_flat_timeline,
    },
    Migration {
        name: "every timeline carries its frame",
        apply: give_timelines_their_frame,
    },
    Migration {
        name: "version 2: fields older builds would drop",
        apply: fields_older_builds_would_drop,
    },
];

/// Version 2 changes no shape: everything it added defaults when absent,
/// so a version 1 document already is a version 2 one. The bump is there
/// so that builds which cannot keep those fields refuse the document
/// rather than lose them (see [`DOCUMENT_VERSION`]).
fn fields_older_builds_would_drop(_document: &mut Map<String, Value>) {}

/// A document from before multiple timelines holds one, as the top-level
/// `tracks`, `clips` and `video`. Lifted into `timelines` as the one
/// timeline it always was, leaving the flat fields in place: they are
/// written back as the mirror every save writes.
fn lift_flat_timeline(document: &mut Map<String, Value>) {
    let has_timelines = document
        .get("timelines")
        .and_then(Value::as_array)
        .is_some_and(|entries| !entries.is_empty());
    if has_timelines {
        return;
    }
    let tracks = document.get("tracks").cloned().unwrap_or(Value::Null);
    if tracks.as_array().is_none_or(|tracks| tracks.is_empty()) {
        return;
    }
    let clips = document.get("clips").cloned().unwrap_or(json!([]));
    let mut timeline = json!({
        "id": "TL1",
        "name": "Timeline 1",
        "tracks": tracks,
        "clips": clips,
    });
    if let Some(video) = document.get("video").cloned()
        && let Some(timeline) = timeline.as_object_mut()
    {
        timeline.insert("video".to_owned(), video);
    }
    document.insert("timelines".to_owned(), json!([timeline]));
}

/// Timelines from before each carried its own frame take the document's
/// top-level `video` block, a field at a time: a timeline naming a zero
/// dimension or rate gets the shared value for that field rather than the
/// zero, and one naming nothing gets the block whole.
fn give_timelines_their_frame(document: &mut Map<String, Value>) {
    let shared = document
        .get("video")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let Some(timelines) = document.get_mut("timelines").and_then(Value::as_array_mut) else {
        return;
    };
    for timeline in timelines.iter_mut().filter_map(Value::as_object_mut) {
        let mut video = timeline
            .get("video")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for key in ["width", "height", "rateNum", "rateDen"] {
            let usable = video
                .get(key)
                .and_then(Value::as_i64)
                .is_some_and(|n| n > 0);
            if !usable && let Some(value) = shared.get(key) {
                video.insert(key.to_owned(), value.clone());
            }
        }
        if !video.is_empty() {
            timeline.insert("video".to_owned(), Value::Object(video));
        }
    }
}

/// The document at this build's shape, or `None` for one from a later
/// build.
fn migrate(document: &Value) -> Option<Map<String, Value>> {
    let mut document = document.as_object()?.clone();
    if document_version(&Value::Object(document.clone())) > DOCUMENT_VERSION {
        return None;
    }
    for migration in MIGRATIONS {
        (migration.apply)(&mut document);
    }
    Some(document)
}

/// The top-level keys the writer owns. Read, they would land in the
/// project's `extra` map and be written twice; they are the writer's to
/// produce afresh.
const WRITER_OWNED: &[&str] = &[
    "concat",
    "version",
    "name",
    "video",
    "tracks",
    "clips",
    "media",
    "fonts",
    "timelines",
    "activeTimelineId",
];

/// Rebuilds a project from a document. Returns None only when there is
/// nothing to open: a value that is not an object, a document from a later
/// build, or one with no timeline this build can read that has a lane.
pub fn from_document(document: &Value) -> Option<Project> {
    let migrated = migrate(document)?;
    settle(crate::unread::read(migrated, document)?)
}

/// Everything the derives cannot decide about a freshly read project; see
/// the module doc. What has nowhere to live was held by the read already.
/// `None` when no timeline was read.
fn settle(mut project: Project) -> Option<Project> {
    for key in WRITER_OWNED {
        project.extra.remove(*key);
    }
    project.media = std::mem::take(&mut project.media)
        .into_iter()
        .map(MediaItem::tidy)
        .collect();
    project.timelines = std::mem::take(&mut project.timelines)
        .into_iter()
        .map(|timeline| {
            let timeline = Arc::try_unwrap(timeline).unwrap_or_else(|shared| (*shared).clone());
            Arc::new(settle_timeline(timeline))
        })
        .collect();
    if project.timelines.is_empty() {
        return None;
    }
    if !project
        .timelines
        .iter()
        .any(|timeline| timeline.id == project.active_timeline_id)
    {
        project.active_timeline_id = project.timelines[0].id.clone();
    }
    Some(project)
}

/// One timeline settled: a name, a frame, every clip's numbers in range.
fn settle_timeline(mut timeline: Timeline) -> Timeline {
    if timeline.name.trim().is_empty() {
        timeline.name = "Timeline".to_owned();
    }
    timeline.video = timeline.video.or(VideoSettings::default());
    timeline.clips = std::mem::take(&mut timeline.clips)
        .into_iter()
        .map(|clip| {
            let clip = Arc::try_unwrap(clip).unwrap_or_else(|shared| (*shared).clone());
            Arc::new(settle_clip(clip))
        })
        .collect();
    timeline
}

/// One clip settled: the fields its kind does not have cleared - titles,
/// layers and shapes have no media - and every number in range.
fn settle_clip(mut clip: Clip) -> Clip {
    use crate::model::ClipKind;
    match clip.kind {
        ClipKind::Text
        | ClipKind::Layer
        | ClipKind::SoundLayer
        | ClipKind::Shape
        | ClipKind::Transition => clip.media_id.clear(),
        ClipKind::Video | ClipKind::Audio | ClipKind::Image => {}
    }
    if clip.kind != ClipKind::Text {
        clip.text = None;
    }
    if clip.kind != ClipKind::Shape {
        clip.shape = None;
    }
    clip.tidy()
}

/// Settings the host manages around the edit: the manifest's identity fields.
///
/// The frame and rate here are what a project *starts* as - the launch
/// screen's pickers, handed to the first timeline of a project that has no
/// document yet. Once there is a document, every timeline carries its own
/// (`Timeline::video`), and the ones here are not consulted again: the
/// document is the edit, and the manifest is the same file.
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
    /// What shows where no picture covers the frame; see
    /// `VideoSettings::canvas`.
    pub canvas: [u8; 4],
}

impl DocumentSettings {
    /// The frame and rate, as the first timeline of a fresh project gets
    /// them.
    pub fn video(&self) -> VideoSettings {
        VideoSettings {
            width: self.width,
            height: self.height,
            rate_num: self.rate_num,
            rate_den: self.rate_den,
            color_space: ColorSpace::Sdr,
            canvas: self.canvas,
        }
    }
}

/// Builds the full `concat.json` document.
pub fn to_document(settings: &DocumentSettings, project: &Project) -> Value {
    fn list(value: Value) -> Vec<Value> {
        match value {
            Value::Array(items) => items,
            _ => Vec::new(),
        }
    }
    let active = project.active();
    let mut media = list(serde_json::to_value(&project.media).expect("serialises"));
    let mut fonts = list(serde_json::to_value(&project.fonts).expect("serialises"));
    let mut timelines = list(serde_json::to_value(&project.timelines).expect("serialises"));
    // What this build could not read, back where it was read from.
    project
        .unread
        .restore(&mut media, &mut fonts, &mut timelines);
    // The flat mirror of the active timeline, for builds that predate
    // multiple timelines: the timeline as it is written, held clips and all.
    let (tracks, clips) = timelines
        .iter()
        .find(|timeline| timeline.get("id").and_then(Value::as_str) == Some(active.id.as_str()))
        .map(|timeline| (timeline["tracks"].clone(), timeline["clips"].clone()))
        .unwrap_or_else(|| (json!([]), json!([])));
    let mut document = Map::new();
    document.insert("concat".into(), json!("0.1.0"));
    document.insert("version".into(), json!(DOCUMENT_VERSION));
    document.insert("name".into(), json!(settings.name));
    // The active timeline's frame, at the top level where every build has
    // read it - the same mirror `tracks` and `clips` are, and for the same
    // reader.
    document.insert(
        "video".into(),
        json!({
            "width": active.video.width,
            "height": active.video.height,
            "rateNum": active.video.rate_num,
            "rateDen": active.video.rate_den,
        }),
    );
    document.insert("media".into(), Value::Array(media));
    document.insert("tracks".into(), tracks);
    document.insert("clips".into(), clips);
    document.insert("fonts".into(), Value::Array(fonts));
    document.insert("timelines".into(), Value::Array(timelines));
    document.insert("activeTimelineId".into(), json!(project.active_timeline_id));
    // Whatever a newer or a different build put at the top level, back
    // where it was; never over a field this writer owns.
    for (key, value) in &project.extra {
        document.entry(key.clone()).or_insert_with(|| value.clone());
    }
    Value::Object(document)
}

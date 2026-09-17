// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The document: `concat.json` in and out.
//!
//! Reading is three steps, each with one job:
//!
//! 1. **Migrate.** The JSON is walked through [`MIGRATIONS`], oldest shape
//!    first, until it has the shape this build writes. A document from a
//!    later build than this one is refused whole rather than read in part
//!    and then saved over.
//! 2. **Parse.** The model's own `serde` derives read it. Every struct
//!    defaults a missing field, every list keeps the entries that parse and
//!    drops the rest, and every field this build does not know lands in the
//!    struct's `extra` map, so it is written back out untouched. See
//!    `model::wire` for the tolerant readers.
//! 3. **Settle.** What no derive can decide: a clip whose track or media is
//!    gone has nowhere to live and is dropped, a timeline with no lanes is
//!    no timeline, every number is pulled into the range a command holds it
//!    to ([`Clip::tidy`]), and the active timeline is one that exists.
//!
//! The standing rule through all three: a hand-edited or older file
//! degrades to something openable. Nothing short of a document that is not
//! an object, or that has no timeline in it, fails the load.
//!
//! Writing is the derives again, plus a mirror of the active timeline in
//! the flat `tracks` / `clips` / `video` fields that builds from before
//! multiple timelines read, so a file saved here still opens there.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::model::{Clip, MediaItem, Project, Timeline, VideoSettings};

/// Bumped only when a change cannot be absorbed by defaulting. Read back
/// on load: a document from a later version is refused whole rather than
/// read in part and then saved over. Every bump adds a step to
/// [`MIGRATIONS`].
pub const DOCUMENT_VERSION: u64 = 1;

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
];

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
/// build, or one with no timeline that has a lane.
pub fn from_document(document: &Value) -> Option<Project> {
    let document = migrate(document)?;
    let project: Project = serde_json::from_value(Value::Object(document)).ok()?;
    settle(project)
}

/// Everything the derives cannot decide about a freshly parsed project;
/// see the module doc. `None` when no timeline survives.
fn settle(mut project: Project) -> Option<Project> {
    for key in WRITER_OWNED {
        project.extra.remove(*key);
    }
    project.media = project
        .media
        .into_iter()
        // An empty path is a template's placeholder, not a broken entry.
        .filter(|item| !item.id.is_empty())
        .map(MediaItem::tidy)
        .collect();
    let media: Vec<MediaItem> = project.media.clone();
    project
        .fonts
        .retain(|font| !font.family.is_empty() && !font.path.is_empty());

    let mut seen: HashSet<String> = HashSet::new();
    project.timelines = project
        .timelines
        .into_iter()
        .filter_map(|timeline| {
            let timeline = Arc::try_unwrap(timeline).unwrap_or_else(|shared| (*shared).clone());
            settle_timeline(timeline, &media)
        })
        .filter(|timeline| seen.insert(timeline.id.clone()))
        .map(Arc::new)
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

/// One timeline settled: lanes and clips with ids, clips on lanes that
/// exist and media that is in the bin, every number in range. `None` for
/// a timeline with no id or no lane.
fn settle_timeline(mut timeline: Timeline, media: &[MediaItem]) -> Option<Timeline> {
    if timeline.id.is_empty() {
        return None;
    }
    if timeline.name.trim().is_empty() {
        timeline.name = "Timeline".to_owned();
    }
    timeline.video = timeline.video.or(VideoSettings::default());
    let mut seen: HashSet<String> = HashSet::new();
    timeline
        .tracks
        .retain(|track| !track.id.is_empty() && seen.insert(track.id.clone()));
    if timeline.tracks.is_empty() {
        return None;
    }
    let tracks = timeline.tracks.clone();
    timeline.clips = timeline
        .clips
        .into_iter()
        .filter_map(|clip| {
            let clip = Arc::try_unwrap(clip).unwrap_or_else(|shared| (*shared).clone());
            settle_clip(clip, &tracks, media)
        })
        .map(Arc::new)
        .collect();
    Some(timeline)
}

/// One clip settled, or `None` for one with nowhere to live: no id, a
/// track that vanished, or - for the kinds that come from the bin - media
/// that is gone. Titles and layers have no media and need none.
fn settle_clip(
    mut clip: Clip,
    tracks: &[crate::model::Track],
    media: &[MediaItem],
) -> Option<Clip> {
    use crate::model::ClipKind;
    if clip.id.is_empty() {
        return None;
    }
    if !tracks.iter().any(|track| track.id == clip.track_id) {
        return None;
    }
    match clip.kind {
        ClipKind::Text | ClipKind::Layer => clip.media_id.clear(),
        ClipKind::Video | ClipKind::Audio | ClipKind::Image => {
            if !media.iter().any(|item| item.id == clip.media_id) {
                return None;
            }
        }
    }
    if clip.kind != ClipKind::Text {
        clip.text = None;
    }
    Some(clip.tidy())
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
        }
    }
}

/// Builds the full `concat.json` document.
pub fn to_document(settings: &DocumentSettings, project: &Project) -> Value {
    let active = project.active();
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
    document.insert(
        "media".into(),
        serde_json::to_value(&project.media).expect("serialises"),
    );
    // The flat mirror of the active timeline, for builds that predate
    // multiple timelines.
    document.insert(
        "tracks".into(),
        serde_json::to_value(&active.tracks).expect("serialises"),
    );
    document.insert(
        "clips".into(),
        serde_json::to_value(&active.clips).expect("serialises"),
    );
    document.insert(
        "fonts".into(),
        serde_json::to_value(&project.fonts).expect("serialises"),
    );
    document.insert(
        "timelines".into(),
        serde_json::to_value(&project.timelines).expect("serialises"),
    );
    document.insert("activeTimelineId".into(), json!(project.active_timeline_id));
    // Whatever a newer or a different build put at the top level, back
    // where it was; never over a field this writer owns.
    for (key, value) in &project.extra {
        document.entry(key.clone()).or_insert_with(|| value.clone());
    }
    Value::Object(document)
}

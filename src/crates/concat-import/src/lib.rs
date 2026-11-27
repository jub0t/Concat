// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Opens a project saved by an earlier Concat as a new document.
//!
//! One way: the earlier file goes in, a [`Document`] of the [`Model`]
//! comes out, with [`Note`]s in plain words on what was carried over
//! approximately and what could not be, each naming the clip. The file
//! is read on its own terms, from its JSON; nothing of the earlier app is
//! linked.
//!
//! # What is carried over
//!
//! The bin with what each probe found; every timeline, its frame (a rate
//! written 23.976 or 29.97 is its exact fraction over 1001), background
//! and linkage; its tracks as lanes, bottom to top, with their toggles
//! and a caption lane's look; every clip, of every kind, at the tick
//! nearest where it starts and where it ends, so clips that met still
//! meet; speed, and a speed curve as the position keys the document's own
//! command makes of one; keys on the item's content time; effects and
//! their knobs and keys; masks and their keys; crop, cutout, stabilize,
//! voice, motion, the transition on a cut and a transition placed as a
//! clip; detached sound tied to its picture; fonts, the cover, beat marks
//! and which timeline was showing.
//!
//! Two clips that overlap on one track, which a lane cannot hold, go on
//! a new lane above the track, each with a note. A field the new
//! document has no place for is kept in its record's residual, under
//! `imported`, and said; a record that cannot be read at all is kept as
//! its text in its place. Nothing is dropped without a note.
//!
//! The project's name is the manifest's, which the host keeps, and is
//! not part of the document.

#![forbid(unsafe_code)]

mod build;
mod clip;
mod look;
mod media;
mod read;
mod timeline;

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use concat_doc::model::{Cover, Font, ItemId, Media, MediaKind, Model, Rule, SequenceId};
use concat_doc::{Document, Mint, Rejected, Revision, TypedId, Writer};
use serde_json::Value;

pub use concat_doc::file::Note;

use crate::build::Build;
use crate::read::{Fields, Kept, dur};

/// Why a file was not imported. Nothing was made of it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ImportError {
    /// It is not JSON.
    NotJson(String),
    /// It is JSON, and not an edit saved by an earlier Concat.
    NotAnEdit,
    /// It holds more ids than a document can count.
    TooLarge,
    /// The document made of it broke a rule of the model that setting
    /// the clips at fault aside did not mend.
    Rejected(String),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::NotJson(why) => {
                write!(f, "the project's file is not readable JSON: {why}")
            }
            ImportError::NotAnEdit => f.write_str("the file is not a project of an earlier Concat"),
            ImportError::TooLarge => {
                f.write_str("the project holds more than a document can count")
            }
            ImportError::Rejected(why) => {
                write!(f, "the project could not be made into a document: {why}")
            }
        }
    }
}

impl std::error::Error for ImportError {}

/// A project, imported.
#[derive(Debug)]
pub struct Imported {
    /// The new document. It keeps every rule of the model.
    pub document: Document<Model>,
    /// What was carried over approximately and what could not be.
    pub notes: Vec<Note>,
    /// Each clip's new id, by its timeline's id and its own in the
    /// earlier file.
    pub items: BTreeMap<(String, String), ItemId>,
    /// Each timeline's sequence, by its id in the earlier file.
    pub sequences: BTreeMap<String, SequenceId>,
}

/// The keys an earlier Concat names the build that wrote a file under:
/// `concat`, or `wolfcut` in a file whose manifest is `wolfcut.json`.
const WRITERS: [&str; 2] = ["concat", "wolfcut"];

/// Whether a project file's JSON is an edit saved by an earlier Concat:
/// it says which Concat and which version wrote it, and holds timelines
/// or tracks.
pub fn is_earlier_edit(value: &Value) -> bool {
    WRITERS.iter().any(|key| value.get(key).is_some())
        && value.get("version").is_some()
        && (value.get("timelines").is_some() || value.get("tracks").is_some())
}

/// Imports a project saved by an earlier Concat: the new document, and
/// the notes on what was carried over approximately or could not be.
pub fn import_project(text: &str) -> Result<(Document<Model>, Vec<Note>), ImportError> {
    import(text).map(|imported| (imported.document, imported.notes))
}

/// [`import_project`], with what the earlier ids became.
pub fn import(text: &str) -> Result<Imported, ImportError> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| ImportError::NotJson(error.to_string()))?;
    if !is_earlier_edit(&value) {
        return Err(ImportError::NotAnEdit);
    }
    let mut build = Build::new();
    let mut top = Fields::of(&value);
    top.skip(&WRITERS);
    top.skip(&["version", "name"]);
    let top_video = top.object("video");
    let top_tracks = top.list("tracks");
    let top_clips = top.list("clips");

    let media = media::bin(&mut build, top.list("media"))?;
    let bin = clip::Bin { media: &media };

    // Every timeline; a file from before there were several has one, at
    // the top.
    let timelines = top.list("timelines");
    let active = top.str("activeTimelineId").unwrap_or("").to_owned();
    let lone = serde_json::json!({ "id": "TL1", "name": "Timeline 1" });
    let mut made = Vec::new();
    if timelines.is_empty() {
        let tracks: &[Value] = if top_tracks.is_empty() {
            STARTER.as_slice()
        } else {
            top_tracks
        };
        made.push(timeline::timeline(
            &mut build, &bin, &lone, tracks, top_clips, top_video, 1,
        )?);
    } else {
        for (at, line) in timelines.iter().enumerate() {
            made.push(timeline::timeline(
                &mut build,
                &bin,
                line,
                &[],
                &[],
                top_video,
                at + 1,
            )?);
        }
        // The top-level clips are the showing timeline's, written twice.
        let showing = timelines
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some(active.as_str()));
        if let Some(showing) = showing
            && !top_clips.is_empty()
            && showing
                .get("clips")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                != Some(top_clips)
        {
            build.note(
                "",
                "the clips at the top of the file differ from its showing timeline's; the timeline's are used",
            );
        }
    }

    // The view: the showing timeline, and every one open as a tab.
    let showing = made
        .iter()
        .find(|m| m.old == active)
        .or(made.first())
        .map(|m| m.sequence);
    build.model.view.showing = showing;
    build.model.view.tabs = made.iter().map(|m| m.sequence).collect();

    // Fonts, each file once.
    for (at, font) in top.list("fonts").iter().enumerate() {
        let mut f = Fields::of(font);
        match (f.str("family"), f.str("path")) {
            (Some(family), Some(path))
                if !build.model.fonts.iter().any(|font| font.path == path) =>
            {
                build.model.fonts.push(Font {
                    family: family.to_owned(),
                    path: path.to_owned(),
                });
            }
            _ => build.note(
                &format!("font {}", at + 1),
                "this font is another's file too, or could not be read; the first is kept",
            ),
        }
    }

    // The cover: a frame of the edit, or a still.
    if let Some(cover) = top.object("cover") {
        build.model.cover = self::cover(&mut build, cover)?;
    }

    // What else the file held.
    let mut kept = Kept::default();
    kept.rest("", &top);
    if !kept.is_empty() {
        let names = kept.names().join(", ");
        build.note(
            "",
            format!("kept {names}, which this version has no place for"),
        );
    }
    build.model.residual = kept.residual(&mut build.words);

    let mut sources: HashMap<ItemId, &Value> = HashMap::new();
    let mut items = BTreeMap::new();
    let mut sequences = BTreeMap::new();
    for m in &made {
        sources.extend(m.sources.iter().map(|(id, v)| (*id, *v)));
        for (old, id) in &m.items {
            items.insert((m.old.clone(), old.clone()), *id);
        }
        sequences.insert(m.old.clone(), m.sequence);
    }
    let document = finish(&mut build, &sources)?;
    Ok(Imported {
        document,
        notes: build.notes,
        items,
        sequences,
    })
}

/// The four lanes a timeline started with.
static STARTER: std::sync::LazyLock<Vec<Value>> = std::sync::LazyLock::new(|| {
    ["T1", "T2", "T3", "T4"]
        .iter()
        .map(|id| serde_json::json!({ "id": id }))
        .collect()
});

fn cover(build: &mut Build, value: &Value) -> Result<Option<Cover>, ImportError> {
    let mut c = Fields::of(value);
    match c.str("kind") {
        Some("frame") => {
            let into = dur(c.f64_or("time", 0.0).max(0.0)).unwrap_or_default();
            Ok(Some(Cover::Frame { into }))
        }
        Some("picture") => {
            let path = c.string_or("path", "");
            let found = build
                .model
                .media
                .iter()
                .find(|(_, m)| m.path == path && m.kind == MediaKind::Image)
                .map(|(id, _)| id);
            let media = match found {
                Some(id) => id,
                None => {
                    // The cover is a still of the bin in the new document.
                    let id = build.mint()?;
                    let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned();
                    let still = Media {
                        path: path.clone(),
                        name,
                        kind: MediaKind::Image,
                        ..Media::default()
                    };
                    build
                        .model
                        .media
                        .insert(id, still)
                        .map_err(|error| ImportError::Rejected(error.to_string()))?;
                    build.note(
                        "the cover",
                        "its picture is in the bin now, where a cover's picture is kept",
                    );
                    id
                }
            };
            Ok(Some(Cover::Picture { media }))
        }
        _ => {
            let raw = build.raw(value);
            Ok(Some(Cover::Unknown(raw)))
        }
    }
}

/// The document: checked against every rule as a file is. A clip that
/// breaks one is set aside, kept whole and inert on a lane of its own,
/// and said; then the rest is checked again.
fn finish(
    build: &mut Build,
    sources: &HashMap<ItemId, &Value>,
) -> Result<Document<Model>, ImportError> {
    for _ in 0..8 {
        let mut counts: BTreeMap<Writer, u64> = build.words.highest.clone();
        let own = counts.entry(Writer::LOCAL).or_insert(0);
        *own = (*own).max(build.count());
        let mint = Mint::resume(counts).map_err(|_| ImportError::TooLarge)?;
        let violations = match Document::load(build.model.clone(), mint, Revision::default()) {
            Ok(document) => return Ok(document),
            Err(Rejected::Unminted(ids)) => {
                return Err(ImportError::Rejected(format!("ids never made: {ids:?}")));
            }
            Err(Rejected::Invalid(violations)) => violations,
        };
        let mut aside: Vec<ItemId> = Vec::new();
        for violation in &violations {
            let Some(id) = violation.id else {
                return Err(ImportError::Rejected(format!("{violation}")));
            };
            let item = id
                .typed::<ItemId>()
                .or_else(|| build.model.items.index().holding(id).next());
            match item {
                Some(item) if !aside.contains(&item) => aside.push(item),
                Some(_) => {}
                None => return Err(ImportError::Rejected(format!("{violation}"))),
            }
        }
        for item in aside {
            let name = build
                .model
                .items
                .get(item)
                .map(|i| i.name.clone())
                .unwrap_or_default();
            let why: Vec<String> = violations
                .iter()
                .filter(|v| v.id == Some(item.any()))
                .map(|v| describe(&v.rule))
                .collect();
            build.note(
                &format!("clip \"{name}\""),
                format!("it broke a rule of the new document ({}); it is kept as it was, on a lane of its own, and does not play", why.join("; ")),
            );
            timeline::set_aside(build, item, sources.get(&item).copied())?;
        }
    }
    Err(ImportError::Rejected(
        "clips kept breaking the document's rules".to_owned(),
    ))
}

fn describe(rule: &Rule) -> String {
    format!("{rule:?}")
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What this build cannot read, kept as it was read.
//!
//! The rule: a build never deletes what it cannot read. A clip of a kind
//! from a newer build, a media item or a timeline whose fields do not
//! parse, a keyframe or an effect among readable ones that does not, and
//! anything the reader's settle step has nowhere to put - a clip on a lane
//! that is gone, a timeline with no lanes - is lifted out of the document
//! whole, as the JSON it was, and held in [`Unread`]. The rest opens and is
//! edited as usual. The writer puts every held value back where it came
//! from, unchanged, so the next save writes the file a newer build (or a
//! repaired one) can still read in full.
//!
//! What a held value belongs to decides whether it is written: clips go
//! back into their timeline, parts into their clip, while that owner is
//! still there. A timeline or a clip the person deleted takes its unread
//! parts with it, as it would have taken readable ones.
//!
//! Reading is one parse: every media item, timeline and clip is read here,
//! entry by entry, and the readable ones are what the project is built
//! from. A clip's lists are checked by count first, so the common clip,
//! every entry of which reads, is never parsed twice.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::model::{
    AppliedFilter, AudioTrack, Clip, ClipKey, ClipKind, CustomFont, Mask, MediaItem, MediaKind,
    ParamKey, Project, SpeedPoint, TextRun, Timeline, Track,
};

/// Everything the reader lifted out of a document because this build cannot
/// read it, each value exactly as it was in the file, with where it goes
/// back. Empty for a document this build reads in full.
///
/// Nothing changes it after the read, and every undo snapshot and every
/// view of the project clones it, so it is shared rather than copied: a
/// held timeline of a thousand clips costs a pointer per command.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Unread {
    held: Option<Arc<Held>>,
}

/// What [`Unread`] holds.
#[derive(Debug, Default, PartialEq)]
struct Held {
    /// Bin entries, by their place in the bin.
    media: Vec<Entry>,
    /// Whole timelines, their lanes and clips inside them.
    timelines: Vec<Entry>,
    /// Clips of readable timelines, by the timeline's id.
    clips: Vec<(String, Entry)>,
    /// Pieces of something readable: an entry of one of its lists (a
    /// keyframe, a mask, an effect, a point of a speed curve, a lane, a
    /// styled range of a title, a sound stream, a font) or one of its
    /// optional blocks (a crop, a motion, a bin entry's origin).
    parts: Vec<Part>,
}

/// One value as it was read, and its place in the list it came from.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    index: usize,
    value: Value,
}

/// What an unread part belongs to.
#[derive(Clone, Debug, PartialEq)]
enum Owner {
    /// The document itself (the font list).
    Project,
    /// A bin entry, by id.
    Media(String),
    /// A timeline, by id.
    Timeline(String),
    /// A clip, by its timeline's id and its own.
    Clip { timeline: String, clip: String },
}

/// A piece of something readable that does not read itself.
#[derive(Clone, Debug, PartialEq)]
struct Part {
    owner: Owner,
    /// The path inside its owner's object, in the document's spelling:
    /// `["keys"]`, `["text", "runs"]`, `["crop"]`.
    field: Field,
    slot: Slot,
    entry: Entry,
}

/// Where a part goes back.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Slot {
    /// An entry of the list at the part's field, at the entry's place.
    Entry,
    /// The field's whole value - a clip's crop, a bin entry's origin -
    /// put back only where the owner has none: one the person has set
    /// since is theirs, and wins.
    Block,
}

/// How much of a document this build could not read: what the person is
/// told on open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnreadSummary {
    /// Clips of readable timelines.
    pub clips: usize,
    /// Bin entries.
    pub media: usize,
    /// Whole timelines.
    pub timelines: usize,
    /// Keyframes, effects, masks, lanes, styled ranges and the like, inside
    /// things that did open.
    pub parts: usize,
}

impl UnreadSummary {
    /// True when the document was read in full.
    pub fn is_empty(&self) -> bool {
        *self == UnreadSummary::default()
    }
}

/// A list's path inside its owner's object.
type Field = &'static [&'static str];

/// A part of a clip that does not read, before it knows its owner.
type ClipPart = (Field, Slot, Entry);

const FONTS: &[&str] = &["fonts"];
const AUDIO_TRACKS: &[&str] = &["audioTracks"];
const TRACKS: &[&str] = &["tracks"];
const KEYS: &[&str] = &["keys"];
const SPEED_CURVE: &[&str] = &["speedCurve"];
const MASKS: &[&str] = &["masks"];
const FILTERS: &[&str] = &["filters"];
const VIDEO_EFFECTS: &[&str] = &["videoEffects"];
const TEXT_RUNS: &[&str] = &["text", "runs"];
const TEXT: &[&str] = &["text"];
const SHAPE: &[&str] = &["shape"];
const CROP: &[&str] = &["crop"];
const CUTOUT: &[&str] = &["cutout"];
const MOTION: &[&str] = &["motion"];
const STABILIZE: &[&str] = &["stabilize"];
const VOICE: &[&str] = &["voice"];
const TRANSITION_IN: &[&str] = &["transitionIn"];
const ENHANCED: &[&str] = &["enhanced"];
const BACKGROUND: &[&str] = &["background"];
const COLOR_RANGE: &[&str] = &["colorRange"];
const ORIGIN: &[&str] = &["origin"];

impl Unread {
    /// The counts the person is told.
    pub fn summary(&self) -> UnreadSummary {
        self.held
            .as_deref()
            .map_or_else(UnreadSummary::default, |held| UnreadSummary {
                clips: held.clips.len(),
                media: held.media.len(),
                timelines: held.timelines.len(),
                parts: held.parts.len(),
            })
    }

    /// True when nothing was lifted out.
    pub fn is_empty(&self) -> bool {
        self.held.is_none()
    }

    /// Every `id` anywhere in the held values, for the id mint to step
    /// past: an id minted later must never equal one the file still holds.
    pub fn ids(&self) -> Vec<&str> {
        self.held.as_deref().map_or_else(Vec::new, Held::ids)
    }

    /// The ids of the masks held for clip `clip_id`: a mask added to that
    /// clip must not take one, or the file would hold two masks of one id.
    pub fn mask_ids(&self, clip_id: &str) -> Vec<&str> {
        let Some(held) = self.held.as_deref() else {
            return Vec::new();
        };
        held.parts
            .iter()
            .filter(|part| {
                part.field == MASKS
                    && matches!(&part.owner, Owner::Clip { clip, .. } if clip == clip_id)
            })
            .filter_map(|part| part.entry.value.get("id").and_then(Value::as_str))
            .collect()
    }

    /// Puts every held value back into the lists the writer produced; see
    /// [`Held::restore`].
    pub(crate) fn restore(
        &self,
        media: &mut Vec<Value>,
        fonts: &mut Vec<Value>,
        timelines: &mut Vec<Value>,
    ) {
        if let Some(held) = self.held.as_deref() {
            held.restore(media, fonts, timelines);
        }
    }
}

impl Held {
    fn is_empty(&self) -> bool {
        self.media.is_empty()
            && self.timelines.is_empty()
            && self.clips.is_empty()
            && self.parts.is_empty()
    }

    fn ids(&self) -> Vec<&str> {
        fn walk<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
            match value {
                Value::Object(map) => {
                    if let Some(Value::String(id)) = map.get("id") {
                        out.push(id);
                    }
                    for value in map.values() {
                        walk(value, out);
                    }
                }
                Value::Array(items) => {
                    for item in items {
                        walk(item, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        let entries = self
            .media
            .iter()
            .chain(&self.timelines)
            .chain(self.clips.iter().map(|(_, entry)| entry))
            .chain(self.parts.iter().map(|part| &part.entry));
        for entry in entries {
            walk(&entry.value, &mut out);
        }
        out
    }

    /// Puts every held value back into the lists the writer produced:
    /// parts into their owners first, while the lists hold only what was
    /// readable, then clips into their timelines, then whole bin entries
    /// and timelines. Each goes back at the place it was read from, so a
    /// document opened and saved without an edit comes out in the order it
    /// went in.
    fn restore(&self, media: &mut Vec<Value>, fonts: &mut Vec<Value>, timelines: &mut Vec<Value>) {
        for part in &self.parts {
            let owner = match &part.owner {
                Owner::Project => {
                    insert(fonts, &part.entry);
                    continue;
                }
                Owner::Media(id) => by_id(media, id),
                Owner::Timeline(id) => by_id(timelines, id),
                Owner::Clip { timeline, clip } => by_id(timelines, timeline)
                    .and_then(|timeline| timeline.get_mut("clips"))
                    .and_then(Value::as_array_mut)
                    .and_then(|clips| by_id(clips, clip)),
            };
            match (owner, part.slot) {
                (Some(owner), Slot::Entry) => insert_into(owner, part.field, &part.entry),
                (Some(owner), Slot::Block) => fill(owner, part.field, &part.entry.value),
                (None, _) => {}
            }
        }
        for (timeline, entry) in &self.clips {
            if let Some(timeline) = by_id(timelines, timeline) {
                insert_into(timeline, &["clips"], entry);
            }
        }
        for entry in &self.media {
            insert(media, entry);
        }
        for entry in &self.timelines {
            insert(timelines, entry);
        }
    }
}

/// The object in `items` whose `id` is `id`.
fn by_id<'a>(items: &'a mut [Value], id: &str) -> Option<&'a mut Value> {
    items
        .iter_mut()
        .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
}

/// `entry` back into `list` at its place, or at the end of a list that
/// has since grown shorter.
fn insert(list: &mut Vec<Value>, entry: &Entry) {
    let at = entry.index.min(list.len());
    list.insert(at, entry.value.clone());
}

/// `entry` back into the list at `path` inside `owner`. The list is made
/// when the writer left it out for being empty; an object on the way to it
/// that is gone (a title's style on a clip that is no longer a title) is
/// not, and the entry goes with it.
fn insert_into(owner: &mut Value, path: &[&str], entry: &Entry) {
    let Some((last, path)) = path.split_last() else {
        return;
    };
    let mut at = owner;
    for key in path {
        match at.get_mut(*key) {
            Some(next) if next.is_object() => at = next,
            _ => return,
        }
    }
    let Some(object) = at.as_object_mut() else {
        return;
    };
    let list = object
        .entry((*last).to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !list.is_array() {
        *list = Value::Array(Vec::new());
    }
    if let Value::Array(items) = list {
        insert(items, entry);
    }
}

/// `value` as the field at `path` inside `owner`, unless the owner has a
/// value there already: one the person set after the open is theirs.
fn fill(owner: &mut Value, path: &[&str], value: &Value) {
    let Some((last, path)) = path.split_last() else {
        return;
    };
    let mut at = owner;
    for key in path {
        match at.get_mut(*key) {
            Some(next) if next.is_object() => at = next,
            _ => return,
        }
    }
    let Some(object) = at.as_object_mut() else {
        return;
    };
    match object.get(*last) {
        None | Some(Value::Null) => {
            object.insert((*last).to_owned(), value.clone());
        }
        Some(_) => {}
    }
}

/// The value at `path` inside `value`.
fn lookup<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |at, key| at.get(*key))
}

/// The entries of the list at `path` that do not read as `T`, when fewer
/// than all of them did (`read`, the count the tolerant reader kept).
fn unreadable<T: DeserializeOwned>(raw: &Value, path: &[&str], read: usize) -> Vec<Entry> {
    let Some(Value::Array(items)) = lookup(raw, path) else {
        return Vec::new();
    };
    if items.len() == read {
        return Vec::new();
    }
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| T::deserialize(*item).is_err())
        .map(|(index, item)| Entry {
            index,
            value: item.clone(),
        })
        .collect()
}

/// [`unreadable`] for a list of effects or masks, whose entries carry key
/// runs the tolerant reader thins out rather than refusing: an entry with
/// a key that does not read is unreadable whole, and taken out of `read`
/// (the entries the tolerant reader kept, in order). In this build that
/// effect or mask is not shown at all, rather than shown with keys
/// missing - a different picture from the one the file describes - and
/// the file keeps it intact.
fn unreadable_keyed<T: DeserializeOwned>(
    raw: &Value,
    path: &[&str],
    read: &mut Vec<T>,
    runs_of: fn(&T) -> &BTreeMap<String, Vec<ParamKey>>,
) -> Vec<Entry> {
    let Some(Value::Array(items)) = lookup(raw, path) else {
        return Vec::new();
    };
    let written: usize = items.iter().map(|item| run_count(item.get("keys"))).sum();
    let kept: usize = read
        .iter()
        .map(|entry| runs_of(entry).values().map(Vec::len).sum::<usize>())
        .sum();
    if items.len() == read.len() && written == kept {
        return Vec::new();
    }
    let mut typed = std::mem::take(read).into_iter();
    let mut held = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let entry = || Entry {
            index,
            value: item.clone(),
        };
        // The entries the tolerant reader kept are the ones that parse, in
        // order: one that does not was never in `read`.
        if T::deserialize(item).is_err() {
            held.push(entry());
            continue;
        }
        let kept = typed.next();
        if runs_read(item.get("keys")) {
            read.extend(kept);
        } else {
            held.push(entry());
        }
    }
    held
}

/// A mask's key runs, for [`unreadable_keyed`].
fn mask_runs(mask: &Mask) -> &BTreeMap<String, Vec<ParamKey>> {
    &mask.keys
}

/// An effect's key runs, for [`unreadable_keyed`].
fn link_runs(link: &AppliedFilter) -> &BTreeMap<String, Vec<ParamKey>> {
    &link.keys
}

/// How many keys a run map holds as written: a run that is not a list, or
/// a map that is not one, counts as one the reader could not keep.
fn run_count(runs: Option<&Value>) -> usize {
    match runs {
        None | Some(Value::Null) => 0,
        Some(Value::Object(runs)) => runs
            .values()
            .map(|run| match run {
                Value::Array(keys) => keys.len(),
                _ => 1,
            })
            .sum(),
        Some(_) => 1,
    }
}

/// Whether every key of a run map reads.
fn runs_read(runs: Option<&Value>) -> bool {
    match runs {
        None | Some(Value::Null) => true,
        Some(Value::Object(runs)) => runs.values().all(|run| match run {
            Value::Array(keys) => keys.iter().all(|key| ParamKey::deserialize(key).is_ok()),
            _ => false,
        }),
        Some(_) => false,
    }
}

/// Whether `raw` holds a value at `path` that is not null: what an optional
/// block that read as nothing was, when it was something.
fn present(raw: &Value, path: &[&str]) -> bool {
    lookup(raw, path).is_some_and(|value| !value.is_null())
}

/// The blocks among `blocks` that read as nothing though the file has
/// them, each held whole.
fn unread_blocks(raw: &Value, blocks: &[(Field, bool)]) -> Vec<(Field, Value)> {
    blocks
        .iter()
        .filter(|(field, none)| *none && present(raw, field))
        .filter_map(|(field, _)| lookup(raw, field).map(|value| (*field, value.clone())))
        .collect()
}

/// The list at `key`, taken out of the document; not a list is an empty
/// one, as the tolerant reader has always taken it.
fn take_list(document: &mut Map<String, Value>, key: &str) -> Vec<Value> {
    match document.remove(key) {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    }
}

/// Reads a migrated document into a project, every entry this build
/// cannot read or has nowhere to put held in the project's
/// [`Project::unread`]. `None` when the document's own fields do not read.
/// The project still needs the reader's settle step; it may have no
/// timeline at all.
///
/// `original` is the document as the file has it. A timeline held whole is
/// held as it is there, not as the migrations left it: they give a
/// timeline without a frame the document's, and a held one would pick up
/// whichever timeline was active at each save.
pub(crate) fn read(mut document: Map<String, Value>, original: &Value) -> Option<Project> {
    let media_raw = take_list(&mut document, "media");
    let fonts_raw = take_list(&mut document, "fonts");
    let timelines_raw = take_list(&mut document, "timelines");
    let mut project: Project = serde_json::from_value(Value::Object(document)).ok()?;
    let mut unread = Held::default();

    for (index, raw) in media_raw.into_iter().enumerate() {
        // The tolerant reader takes a kind it does not know as video; here
        // it is media from a newer build, held whole, and its clips with it.
        let known = raw
            .get("kind")
            .is_none_or(|kind| kind.is_null() || MediaKind::deserialize(kind).is_ok());
        match MediaItem::deserialize(&raw) {
            Ok(item) if known && !item.id.is_empty() => {
                let owner = Owner::Media(item.id.clone());
                for entry in unreadable::<AudioTrack>(&raw, AUDIO_TRACKS, item.audio_tracks.len()) {
                    unread.parts.push(Part {
                        owner: owner.clone(),
                        field: AUDIO_TRACKS,
                        slot: Slot::Entry,
                        entry,
                    });
                }
                let blocks = [
                    (COLOR_RANGE, item.color_range.is_none()),
                    (ORIGIN, item.origin.is_none()),
                ];
                for (field, value) in unread_blocks(&raw, &blocks) {
                    unread.parts.push(Part {
                        owner: owner.clone(),
                        field,
                        slot: Slot::Block,
                        entry: Entry { index: 0, value },
                    });
                }
                project.media.push(item);
            }
            _ => unread.media.push(Entry { index, value: raw }),
        }
    }

    for (index, raw) in fonts_raw.into_iter().enumerate() {
        match CustomFont::deserialize(&raw) {
            Ok(font) if !font.family.is_empty() && !font.path.is_empty() => {
                project.fonts.push(font);
            }
            _ => unread.parts.push(Part {
                owner: Owner::Project,
                field: FONTS,
                slot: Slot::Entry,
                entry: Entry { index, value: raw },
            }),
        }
    }

    let media: HashSet<&str> = project.media.iter().map(|item| item.id.as_str()).collect();
    let mut seen: HashSet<String> = HashSet::new();
    let mut timelines: Vec<Arc<Timeline>> = Vec::new();
    for (index, mut raw) in timelines_raw.into_iter().enumerate() {
        match read_timeline(&mut raw, &media, &seen, &mut unread) {
            Some(timeline) => {
                seen.insert(timeline.id.clone());
                timelines.push(Arc::new(timeline));
            }
            None => {
                let value = original
                    .get("timelines")
                    .and_then(|timelines| timelines.get(index))
                    .cloned()
                    .unwrap_or(raw);
                unread.timelines.push(Entry { index, value });
            }
        }
    }
    project.timelines = timelines;
    project.unread = Unread {
        held: (!unread.is_empty()).then(|| Arc::new(unread)),
    };
    Some(project)
}

/// One timeline read, its unreadable lanes and clips held in `unread`, or
/// `None` - with `raw` exactly as it came - for one this build cannot read
/// or that has nowhere to put a clip: no id, one a timeline before it
/// took, or no lane.
fn read_timeline(
    raw: &mut Value,
    media: &HashSet<&str>,
    seen: &HashSet<String>,
    unread: &mut Held,
) -> Option<Timeline> {
    let map = raw.as_object_mut()?;
    // Read apart from the timeline's own fields: lanes are checked one by
    // one, and clips are parsed once, below.
    let clips = map.remove("clips");
    let tracks = map.remove("tracks");
    let lanes = Timeline::deserialize(&*raw)
        .ok()
        .filter(|timeline| !timeline.id.is_empty() && !seen.contains(&timeline.id))
        .and_then(|timeline| {
            let (kept, held) = read_lanes(tracks.as_ref());
            (!kept.is_empty()).then_some((timeline, kept, held))
        });
    let Some((mut timeline, kept, held)) = lanes else {
        let map = raw.as_object_mut().expect("an object");
        if let Some(clips) = clips {
            map.insert("clips".to_owned(), clips);
        }
        if let Some(tracks) = tracks {
            map.insert("tracks".to_owned(), tracks);
        }
        return None;
    };
    for entry in held {
        unread.parts.push(Part {
            owner: Owner::Timeline(timeline.id.clone()),
            field: TRACKS,
            slot: Slot::Entry,
            entry,
        });
    }
    let lanes: HashSet<&str> = kept.iter().map(|track| track.id.as_str()).collect();
    let mut read = Vec::new();
    if let Some(Value::Array(clips)) = clips {
        for (index, raw) in clips.into_iter().enumerate() {
            match read_clip(&raw, &lanes, media) {
                Some((clip, parts)) => {
                    for (field, slot, entry) in parts {
                        unread.parts.push(Part {
                            owner: Owner::Clip {
                                timeline: timeline.id.clone(),
                                clip: clip.id.clone(),
                            },
                            field,
                            slot,
                            entry,
                        });
                    }
                    read.push(Arc::new(clip));
                }
                None => unread
                    .clips
                    .push((timeline.id.clone(), Entry { index, value: raw })),
            }
        }
    }
    timeline.tracks = kept;
    timeline.clips = read;
    Some(timeline)
}

/// A timeline's lanes: those that read, with an id no lane before them
/// took, and the rest as they were.
fn read_lanes(tracks: Option<&Value>) -> (Vec<Track>, Vec<Entry>) {
    let mut ids: HashSet<String> = HashSet::new();
    let mut kept = Vec::new();
    let mut held = Vec::new();
    if let Some(Value::Array(items)) = tracks {
        for (index, item) in items.iter().enumerate() {
            match Track::deserialize(item) {
                Ok(track) if !track.id.is_empty() && ids.insert(track.id.clone()) => {
                    kept.push(track);
                }
                _ => held.push(Entry {
                    index,
                    value: item.clone(),
                }),
            }
        }
    }
    (kept, held)
}

/// One clip read, with the entries of its lists that do not read, or
/// `None` for a clip this build cannot read or place: a kind it does not
/// know, a field that does not parse, no id, a lane that is not there, or
/// media that is not in the bin.
fn read_clip(
    raw: &Value,
    lanes: &HashSet<&str>,
    media: &HashSet<&str>,
) -> Option<(Clip, Vec<ClipPart>)> {
    // The tolerant reader takes a kind it does not know as video; here it
    // is a clip from a newer build, kept whole.
    if let Some(kind) = raw.get("kind")
        && !kind.is_null()
        && ClipKind::deserialize(kind).is_err()
    {
        return None;
    }
    let mut clip = Clip::deserialize(raw).ok()?;
    if clip.id.is_empty() || !lanes.contains(clip.track_id.as_str()) {
        return None;
    }
    if matches!(
        clip.kind,
        ClipKind::Video | ClipKind::Audio | ClipKind::Image
    ) && !media.contains(clip.media_id.as_str())
    {
        return None;
    }
    // A title without its words or a shape without its figure is an empty
    // clip: one whose block does not read is held whole.
    if (clip.kind == ClipKind::Text && clip.text.is_none() && present(raw, TEXT))
        || (clip.kind == ClipKind::Shape && clip.shape.is_none() && present(raw, SHAPE))
    {
        return None;
    }
    let mut parts = Vec::new();
    let blocks = [
        (CROP, clip.crop.is_none()),
        (CUTOUT, clip.cutout.is_none()),
        (MOTION, clip.motion.is_none()),
        (STABILIZE, clip.stabilize.is_none()),
        (VOICE, clip.voice.is_none()),
        (TRANSITION_IN, clip.transition_in.is_none()),
        (ENHANCED, clip.enhanced.is_none()),
        (BACKGROUND, clip.background.is_none()),
    ];
    for (field, value) in unread_blocks(raw, &blocks) {
        parts.push((field, Slot::Block, Entry { index: 0, value }));
    }
    let mut hold = |field: Field, entries: Vec<Entry>| {
        parts.extend(entries.into_iter().map(|entry| (field, Slot::Entry, entry)));
    };
    hold(KEYS, unreadable::<ClipKey>(raw, KEYS, clip.keys.len()));
    hold(
        SPEED_CURVE,
        unreadable::<SpeedPoint>(
            raw,
            SPEED_CURVE,
            clip.speed_curve.as_ref().map_or(0, Vec::len),
        ),
    );
    hold(
        MASKS,
        unreadable_keyed(raw, MASKS, &mut clip.masks, mask_runs),
    );
    hold(
        FILTERS,
        unreadable_keyed(raw, FILTERS, &mut clip.filters, link_runs),
    );
    hold(
        VIDEO_EFFECTS,
        unreadable_keyed(raw, VIDEO_EFFECTS, &mut clip.video_effects, link_runs),
    );
    // Only a title keeps its style; on any other clip the reader lets it
    // go, styled ranges and all.
    if clip.kind == ClipKind::Text
        && let Some(text) = &clip.text
    {
        hold(
            TEXT_RUNS,
            unreadable::<TextRun>(raw, TEXT_RUNS, text.runs.len()),
        );
    }
    Some((clip, parts))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::UnreadSummary;
    use crate::commands::{ClipMove, Command};
    use crate::doc::{DocumentSettings, from_document, to_document};
    use crate::editor::Editor;

    fn settings() -> DocumentSettings {
        DocumentSettings {
            name: "Test".to_owned(),
            width: 1920,
            height: 1080,
            rate_num: 30,
            rate_den: 1,
            canvas: [0, 0, 0, 255],
        }
    }

    // Each entry this build cannot read or place, as the file holds it.
    fn unknown_kind() -> Value {
        json!({ "id": "c3", "trackId": "T1", "kind": "hologram", "start": 4.0,
                "duration": 1.0, "beam": { "colour": "teal" } })
    }
    fn wrong_type() -> Value {
        json!({ "id": "c4", "trackId": "T1", "mediaId": "m1", "kind": "video",
                "start": "soon", "duration": 1.0 })
    }
    fn unreadable_media() -> Value {
        json!({ "id": "m9", "path": 42, "name": "b.mp4", "kind": "video" })
    }
    fn clip_of_unreadable_media() -> Value {
        json!({ "id": "c5", "trackId": "T1", "mediaId": "m9", "kind": "video",
                "start": 6.0, "duration": 1.0 })
    }
    fn unreadable_key() -> Value {
        json!({ "property": "blur", "at": 0.5, "value": 2.0 })
    }
    fn unreadable_effect() -> Value {
        json!({ "id": "glow", "params": { "amount": "lots" } })
    }
    fn on_a_missing_lane() -> Value {
        json!({ "id": "c6", "trackId": "ghost", "kind": "text", "start": 0.0,
                "duration": 1.0, "text": { "content": "Hello" } })
    }
    fn timeline_without_lanes() -> Value {
        json!({ "id": "TL2", "name": "Empty", "tracks": [],
                "clips": [{ "id": "c7", "trackId": "T9", "kind": "text",
                            "start": 0.0, "duration": 1.0 }] })
    }

    fn document() -> Value {
        json!({
            "name": "Mixed", "version": 2,
            "media": [
                { "id": "m1", "path": "/a.mp4", "name": "a.mp4", "kind": "video",
                  "hasAudio": false },
                unreadable_media()
            ],
            "timelines": [
                { "id": "TL1", "name": "Main",
                  "tracks": [{ "id": "T1" }, { "id": "T2" }],
                  "clips": [
                      { "id": "c1", "trackId": "T1", "mediaId": "m1", "kind": "video",
                        "start": 0.0, "duration": 2.0,
                        "keys": [{ "property": "opacity", "at": 0.0, "value": 0.5 },
                                 unreadable_key()],
                        "videoEffects": [{ "id": "sepia" }, unreadable_effect()] },
                      { "id": "c2", "trackId": "T2", "mediaId": "m1", "kind": "video",
                        "start": 3.0, "duration": 1.0 },
                      unknown_kind(),
                      wrong_type(),
                      clip_of_unreadable_media(),
                      on_a_missing_lane()
                  ] },
                timeline_without_lanes()
            ],
            "activeTimelineId": "TL1"
        })
    }

    fn by_id<'a>(list: &'a Value, id: &str) -> &'a Value {
        list.as_array()
            .expect("a list")
            .iter()
            .find(|item| item["id"] == id)
            .unwrap_or_else(|| panic!("{id} is written"))
    }

    fn ids(list: &Value) -> Vec<&str> {
        list.as_array()
            .expect("a list")
            .iter()
            .map(|item| item["id"].as_str().expect("an id"))
            .collect()
    }

    /// Every unreadable entry of [`document`], unchanged, in `saved`.
    fn holds_everything(saved: &Value) {
        assert_eq!(by_id(&saved["media"], "m9"), &unreadable_media());
        assert_eq!(by_id(&saved["timelines"], "TL2"), &timeline_without_lanes());
        let main = by_id(&saved["timelines"], "TL1");
        assert_eq!(by_id(&main["clips"], "c3"), &unknown_kind());
        assert_eq!(by_id(&main["clips"], "c4"), &wrong_type());
        assert_eq!(by_id(&main["clips"], "c5"), &clip_of_unreadable_media());
        assert_eq!(by_id(&main["clips"], "c6"), &on_a_missing_lane());
        let c1 = by_id(&main["clips"], "c1");
        assert!(
            c1["keys"].as_array().unwrap().contains(&unreadable_key()),
            "{}",
            c1["keys"]
        );
        assert_eq!(c1["keys"].as_array().unwrap().len(), 2);
        assert_eq!(c1["videoEffects"][1], unreadable_effect(), "in its place");
        assert_eq!(c1["videoEffects"][0]["id"], "sepia");
        // The flat mirror of the active timeline is that timeline as written.
        assert_eq!(saved["clips"], main["clips"]);
    }

    #[test]
    fn what_cannot_be_read_is_held_and_written_back_unchanged() {
        let project = from_document(&document()).expect("opens");
        // The readable parts open as they always did.
        assert_eq!(ids(&serde_json::to_value(&project.media).unwrap()), ["m1"]);
        assert_eq!(project.timelines.len(), 1);
        let main = project.active();
        assert_eq!(main.id, "TL1");
        let clips: Vec<&str> = main.clips.iter().map(|clip| clip.id.as_str()).collect();
        assert_eq!(clips, ["c1", "c2"]);
        let c1 = main.clip("c1").expect("c1");
        assert_eq!(c1.keys.len(), 1);
        assert_eq!(c1.video_effects.len(), 1);
        assert_eq!(c1.video_effects[0].id, "sepia");
        assert_eq!(
            project.unread_summary(),
            UnreadSummary {
                clips: 4,
                media: 1,
                timelines: 1,
                parts: 2,
            }
        );

        let saved = to_document(&settings(), &project);
        holds_everything(&saved);
        // In the order they were read.
        assert_eq!(
            ids(&by_id(&saved["timelines"], "TL1")["clips"]),
            ["c1", "c2", "c3", "c4", "c5", "c6"]
        );
        assert_eq!(ids(&saved["timelines"]), ["TL1", "TL2"]);
        assert_eq!(ids(&saved["media"]), ["m1", "m9"]);
        // And reading what was written changes nothing more.
        let again = from_document(&saved).expect("reopens");
        assert_eq!(again, project);
        assert_eq!(to_document(&settings(), &again), saved);
    }

    #[test]
    fn what_cannot_be_read_survives_an_edit_and_a_save() {
        let mut editor = Editor::from_document(&document()).expect("opens");
        editor
            .apply(Command::MoveClips {
                moves: vec![ClipMove {
                    clip_id: "c2".to_owned(),
                    start: 5.0,
                    track_id: "T2".to_owned(),
                }],
            })
            .expect("moves");
        let saved = editor.to_document(&settings());
        holds_everything(&saved);
        let main = by_id(&saved["timelines"], "TL1");
        assert_eq!(by_id(&main["clips"], "c2")["start"], 5.0);

        // A clip minted now never takes an id the file still holds.
        let created = editor
            .apply(Command::AddTextClip {
                track_id: Some("T1".to_owned()),
                above: false,
                start: 8.0,
                style: None,
                duration: Some(1.0),
                offset_y: None,
            })
            .expect("adds a title")
            .created_id
            .expect("an id");
        assert!(
            !["c3", "c4", "c5", "c6", "c7", "m9", "TL2"].contains(&created.as_str()),
            "{created}"
        );

        // Undo keeps it too: a snapshot is the project, held entries and all.
        assert!(editor.undo());
        holds_everything(&editor.to_document(&settings()));

        let reopened = Editor::from_document(&saved).expect("reopens");
        assert_eq!(
            reopened.project().unread_summary(),
            editor.project().unread_summary()
        );
        holds_everything(&reopened.to_document(&settings()));
    }

    #[test]
    fn a_deleted_owner_takes_its_unread_parts_with_it() {
        let mut editor = Editor::from_document(&document()).expect("opens");
        editor
            .apply(Command::RemoveClips {
                clip_ids: vec!["c1".to_owned()],
                ripple: false,
            })
            .expect("removes");
        let saved = editor.to_document(&settings());
        let text = saved.to_string();
        assert!(!text.contains("blur"), "the clip's unread key went with it");
        assert!(!text.contains("glow"));
        // What belongs to the timeline, not to the clip, stays.
        assert_eq!(
            by_id(&by_id(&saved["timelines"], "TL1")["clips"], "c3"),
            &unknown_kind()
        );
    }

    #[test]
    fn a_flat_document_keeps_what_it_cannot_read() {
        // The shape from before multiple timelines: one timeline, flat.
        let document = json!({
            "name": "Old", "version": 1,
            "media": [],
            "tracks": [{ "id": "T1" }, { "visible": false }],
            "clips": [
                { "id": "c1", "trackId": "T1", "kind": "text", "start": 0.0, "duration": 1.0 },
                unknown_kind()
            ]
        });
        let project = from_document(&document).expect("opens");
        assert_eq!(
            project.unread_summary(),
            UnreadSummary {
                clips: 1,
                parts: 1,
                ..UnreadSummary::default()
            }
        );
        let saved = to_document(&settings(), &project);
        let timeline = &saved["timelines"][0];
        assert_eq!(by_id(&timeline["clips"], "c3"), &unknown_kind());
        assert_eq!(timeline["tracks"][1], json!({ "visible": false }));
        assert_eq!(saved["clips"], timeline["clips"]);
        assert_eq!(saved["tracks"], timeline["tracks"]);
    }

    /// A document whose optional blocks, key runs and media kind come from
    /// a newer build. No enum inside a title's style refuses an unknown
    /// value (case and alignment fall back), so the title's block is one a
    /// newer build reshaped: a shadow that became an object.
    fn newer_blocks() -> Value {
        json!({
            "name": "Blocks", "version": 2,
            "media": [
                { "id": "m1", "path": "/a.mp4", "name": "a.mp4", "kind": "video",
                  "origin": "telepathy" },
                { "id": "m2", "path": "/b.obj", "name": "b.obj", "kind": "hologram" }
            ],
            "timelines": [{
                "id": "TL1", "name": "Main", "tracks": [{ "id": "T1" }],
                "clips": [
                    { "id": "c1", "trackId": "T1", "mediaId": "m1", "kind": "video",
                      "start": 0.0, "duration": 2.0,
                      "crop": { "left": "a lot" },
                      "videoEffects": [
                          { "id": "sepia" },
                          { "id": "blur", "keys": { "amount": [
                              { "at": 0.0, "value": 1.0 },
                              { "at": "half", "value": 2.0 } ] } }
                      ],
                      "masks": [
                          { "id": "mask-1", "keys": { "x": [{ "at": "half", "value": 0.1 }] } }
                      ] },
                    { "id": "c2", "trackId": "T1", "kind": "text", "start": 3.0,
                      "duration": 1.0,
                      "text": { "content": "Hello", "shadow": { "blur": 4, "kind": "glow" } } },
                    { "id": "c3", "trackId": "T1", "kind": "shape", "start": 3.0,
                      "duration": 1.0,
                      "shape": { "kind": "square", "width": "wide" } },
                    { "id": "c4", "trackId": "T1", "mediaId": "m2", "kind": "video",
                      "start": 5.0, "duration": 1.0 }
                ]
            }],
            "activeTimelineId": "TL1"
        })
    }

    #[test]
    fn blocks_runs_and_kinds_from_a_newer_build_are_held() {
        let document = newer_blocks();
        let mut editor = Editor::from_document(&document).expect("opens");
        let project = editor.project();
        // A title whose style does not read, a shape whose figure does not,
        // and a clip of media of a kind from a newer build: held whole.
        let clips: Vec<&str> = project
            .active()
            .clips
            .iter()
            .map(|clip| clip.id.as_str())
            .collect();
        assert_eq!(clips, ["c1"]);
        let media: Vec<&str> = project.media.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(media, ["m1"], "media of an unknown kind is held");
        // The crop and the origin as blocks; the effect and the mask with a
        // key that does not read, whole.
        let c1 = project.active().clip("c1").expect("c1");
        assert!(c1.crop.is_none());
        assert_eq!(c1.video_effects.len(), 1);
        assert!(c1.masks.is_empty());
        assert_eq!(
            project.unread_summary(),
            UnreadSummary {
                clips: 3,
                media: 1,
                timelines: 0,
                parts: 4,
            }
        );
        // A mask added to c1 does not take the held mask's id.
        assert_eq!(project.unread.mask_ids("c1"), ["mask-1"]);
        assert_eq!(
            crate::model::Mask::next_id_past(&c1.masks, &project.unread.mask_ids("c1")),
            "mask-2"
        );

        // An edit elsewhere, a save: every held piece is the file's, byte
        // for byte, in its place.
        editor
            .apply(Command::MoveClips {
                moves: vec![ClipMove {
                    clip_id: "c1".to_owned(),
                    start: 0.5,
                    track_id: "T1".to_owned(),
                }],
            })
            .expect("moves");
        let saved = editor.to_document(&settings());
        let written = &saved["timelines"][0]["clips"];
        let read = &document["timelines"][0]["clips"];
        for id in ["c2", "c3", "c4"] {
            assert_eq!(by_id(written, id), by_id(read, id), "{id}");
        }
        let c1 = by_id(written, "c1");
        assert_eq!(c1["crop"], json!({ "left": "a lot" }));
        assert_eq!(c1["videoEffects"][0]["id"], "sepia");
        assert_eq!(
            c1["videoEffects"][1],
            by_id(read, "c1")["videoEffects"][1],
            "held whole, in its place"
        );
        assert_eq!(c1["masks"], by_id(read, "c1")["masks"]);
        assert_eq!(c1["start"], 0.5);
        assert_eq!(saved["media"][0]["origin"], "telepathy");
        assert_eq!(saved["media"][1], document["media"][1]);
        let reopened = Editor::from_document(&saved).expect("reopens");
        assert_eq!(
            reopened.project().unread_summary(),
            editor.project().unread_summary()
        );
    }

    #[test]
    fn a_block_the_person_set_since_wins_over_the_held_one() {
        use crate::commands::ClipPatch;
        use crate::model::Crop;
        let mut editor = Editor::from_document(&newer_blocks()).expect("opens");
        let crop = Crop {
            left: 0.1,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        };
        editor
            .apply(Command::UpdateClip {
                clip_id: "c1".to_owned(),
                patch: ClipPatch {
                    crop: Some(Some(crop)),
                    ..ClipPatch::default()
                },
            })
            .expect("crops");
        let saved = editor.to_document(&settings());
        let c1 = by_id(&saved["timelines"][0]["clips"], "c1");
        assert_eq!(c1["crop"], serde_json::to_value(crop).unwrap());
        let reopened = Editor::from_document(&saved).expect("reopens");
        assert_eq!(
            reopened.project().active().clip("c1").unwrap().crop,
            Some(crop)
        );
        assert_eq!(
            reopened.project().unread_summary().parts,
            3,
            "the held crop is gone; the origin, the effect and the mask stay"
        );
    }

    #[test]
    fn a_document_read_in_full_holds_nothing() {
        let mut editor = Editor::new();
        editor
            .apply(Command::AddTextClip {
                track_id: None,
                above: true,
                start: 0.0,
                style: None,
                duration: Some(2.0),
                offset_y: None,
            })
            .expect("adds a title");
        let saved = editor.to_document(&settings());
        let reopened = Editor::from_document(&saved).expect("reopens");
        assert!(reopened.project().unread_summary().is_empty());
        assert_eq!(reopened.to_document(&settings()), saved);
    }
}

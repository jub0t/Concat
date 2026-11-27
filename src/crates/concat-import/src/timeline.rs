// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A timeline of the earlier file as a sequence: its frame, its tracks as
//! lanes, its clips placed on them so that no two overlap, and what each
//! is tied to.

use std::collections::HashMap;
use std::num::NonZeroU32;

use concat_doc::model::{
    Color, Content, Entry, Frame, Item, ItemId, Lane, LaneId, Link, LinkKind, Marker, OrderKey,
    Sequence, SequenceId, Take,
};
use concat_doc::{Rate, Tick, TickDur};
use serde_json::Value;

use crate::ImportError;
use crate::build::Build;
use crate::clip::{self, Bin, Clip};
use crate::look;
use crate::media::{color_space, rate};
use crate::read::{Fields, Kept, color, dur, tick};

/// How far two clips of one track may overlap and still be taken to
/// meet: a millisecond, the width of the earlier file's rounding.
const HAIR: i64 = concat_doc::commands::HAIR.get();

/// A timeline, made.
pub(crate) struct Made<'a> {
    /// Its id in the earlier file.
    pub(crate) old: String,
    /// Its sequence.
    pub(crate) sequence: SequenceId,
    /// Each clip's new id, by its id in the earlier file.
    pub(crate) items: HashMap<String, ItemId>,
    /// Each new item's clip as the earlier file had it.
    pub(crate) sources: HashMap<ItemId, &'a Value>,
}

/// The frame a timeline is drawn to, from its `video` block.
pub(crate) fn frame(
    build: &mut Build,
    place: &str,
    video: Option<&Value>,
    kept: &mut Kept,
) -> Frame {
    let mut frame = Frame::hd();
    let Some(video) = video else {
        return frame;
    };
    let mut v = Fields::of(video);
    match (
        v.u32("width").and_then(NonZeroU32::new),
        v.u32("height").and_then(NonZeroU32::new),
    ) {
        (Some(width), Some(height)) => {
            frame.width = width;
            frame.height = height;
        }
        _ => build.note(place, "its frame has no size; it is 1920 by 1080 now"),
    }
    let num = v.value("rateNum").and_then(Value::as_u64);
    let den = v.value("rateDen").and_then(Value::as_u64);
    match num.zip(den).and_then(|(num, den)| rate(num, den)) {
        Some(found) => frame.rate = found,
        None => {
            build.note(
                place,
                "its frame rate could not be read; it runs at 30 a second now",
            );
            frame.rate = Rate::FPS_30;
        }
    }
    if let Some(canvas) = v.str("canvas") {
        match color(canvas) {
            Some(colour) => frame.canvas = colour,
            None => kept.keep("video", "canvas", Value::String(canvas.to_owned())),
        }
    }
    frame.color = color_space(build, v.str("colorSpace"));
    kept.rest("video", &v);
    frame
}

/// Keys for `n` lanes, bottom to top, as short as they can be.
fn order_keys(n: usize) -> Vec<OrderKey> {
    fn fill(low: Option<&OrderKey>, high: Option<&OrderKey>, n: usize, out: &mut Vec<OrderKey>) {
        if n == 0 {
            return;
        }
        let Some(middle) = OrderKey::between(low, high) else {
            return;
        };
        let below = n / 2;
        fill(low, Some(&middle), below, out);
        out.push(middle.clone());
        fill(Some(&middle), high, n - below - 1, out);
    }
    let mut out = Vec::with_capacity(n);
    fill(None, None, n, &mut out);
    out
}

/// A track of the earlier file: its lane's settings and where notes say
/// it is.
struct Track {
    old: String,
    visible: bool,
    muted: bool,
    captions: Option<concat_doc::model::CaptionLook>,
    kept: Kept,
}

/// Reads one timeline and puts it in the document.
pub(crate) fn timeline<'a>(
    build: &mut Build,
    bin: &Bin<'_>,
    value: &'a Value,
    tracks_value: &'a [Value],
    clips_value: &'a [Value],
    top_video: Option<&Value>,
    number: usize,
) -> Result<Made<'a>, ImportError> {
    let mut f = Fields::of(value);
    let old = f.string_or("id", "");
    let mut name = f.string_or("name", "");
    let place = format!("timeline \"{name}\" ({old})");
    if name.trim().is_empty() {
        name = format!("Timeline {number}");
    }
    let mut kept = Kept::default();
    let video = f.object("video").or(top_video);
    let frame = frame(build, &place, video, &mut kept);
    let mut sequence = Sequence::new(name, frame);
    if let Some(background) = f.object("background")
        && let Some(read) =
            clip::background(build, &place, background, bin, &mut kept, "background")
    {
        sequence.background = read;
    }
    sequence.linkage = f.bool_or("linkage", true);
    for (at, marker) in f.list("markers").iter().enumerate() {
        match self::marker(build, marker)? {
            Some(marker) => sequence.markers.push(Entry::Known(marker)),
            None => kept.keep("markers", &at.to_string(), marker.clone()),
        }
    }
    let sequence_id: SequenceId = build.mint()?;

    // Its tracks, bottom to top: the first is the main lane, and a track
    // added later is above those before it.
    let tracks_list = if f.has("tracks") {
        f.list("tracks")
    } else {
        tracks_value
    };
    let mut tracks: Vec<Track> = Vec::with_capacity(tracks_list.len());
    for track in tracks_list {
        let mut t = Fields::of(track);
        let old = t.string_or("id", "");
        let mut track_kept = Kept::default();
        let captions = t
            .object("captions")
            .map(|look| look::caption_look(build, &format!("track {old}"), look, &mut track_kept));
        let read = Track {
            old,
            visible: t.bool_or("visible", true),
            muted: t.bool_or("muted", false),
            captions,
            kept: track_kept,
        };
        let mut read = read;
        read.kept.rest("", &t);
        tracks.push(read);
    }

    // Its clips.
    let clips_list = if f.has("clips") {
        f.list("clips")
    } else {
        clips_value
    };
    let mut clips: Vec<Clip<'a>> = Vec::with_capacity(clips_list.len());
    for value in clips_list {
        clips.push(clip::clip(build, bin, value)?);
    }
    kept.rest("", &f);
    if !kept.is_empty() {
        let names = kept.names().join(", ");
        build.note(
            &place,
            format!("kept {names}, which this version has no place for"),
        );
    }
    sequence.residual = kept.residual(&mut build.words);
    build
        .model
        .sequences
        .insert(sequence_id, sequence)
        .map_err(|error| ImportError::Rejected(error.to_string()))?;

    // Each clip on its track; a clip of a track the timeline does not
    // have goes on a lane of its own on top.
    let track_of: HashMap<&str, usize> = tracks
        .iter()
        .enumerate()
        .map(|(at, track)| (track.old.as_str(), at))
        .collect();
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); tracks.len() + 1];
    for (at, clip) in clips.iter().enumerate() {
        match clip.track.and_then(|t| track_of.get(t)) {
            Some(track) => groups[*track].push(at),
            None => {
                build.note(
                    &clip.place,
                    "the track it named is not in its timeline; it is on a lane of its own on top",
                );
                groups[tracks.len()].push(at);
            }
        }
    }

    // No two clips overlap on a lane: one that overlaps another of its
    // track goes on a new lane just above the track.
    struct Planned {
        track: Option<usize>,
        clips: Vec<usize>,
        end: Tick,
    }
    let mut lanes: Vec<Planned> = Vec::new();
    for (track, group) in groups.iter_mut().enumerate() {
        let is_track = track < tracks.len();
        if group.is_empty() && !is_track {
            continue;
        }
        group.sort_by_key(|at| (clips[*at].item.start, *at));
        let first = lanes.len();
        lanes.push(Planned {
            track: is_track.then_some(track),
            clips: Vec::new(),
            end: Tick::MIN,
        });
        for &at in group.iter() {
            let start = clips[at].item.start;
            // A clip that overlaps the one before it on its lane by less
            // than the file's rounding meets it.
            if let Some(&last) = lanes[first].clips.last()
                && lanes[first].end > start
                && (lanes[first].end - start).get() <= HAIR
                && let Some(trimmed) = start
                    .checked_since(clips[last].item.start)
                    .and_then(|d| concat_doc::TickLen::new(d.get()))
            {
                clips[last].item.len = trimmed;
                lanes[first].end = start;
            }
            let free = (first..lanes.len()).find(|lane| lanes[*lane].end <= start);
            let lane = match free {
                Some(lane) => lane,
                None => {
                    lanes.push(Planned {
                        track: is_track.then_some(track),
                        clips: Vec::new(),
                        end: Tick::MIN,
                    });
                    lanes.len() - 1
                }
            };
            if lane != first {
                let blocking = lanes[first].clips.last().map(|b| clips[*b].place.clone());
                let with = blocking.unwrap_or_default();
                let on = clips[at].track.unwrap_or("");
                build.note(
                    &clips[at].place,
                    format!("it overlapped {with} on track {on}, which a lane cannot hold; it is on a new lane above that track"),
                );
            }
            let end = clips[at].item.end().unwrap_or(Tick::MAX);
            lanes[lane].clips.push(at);
            lanes[lane].end = end;
        }
    }

    // The lanes, bottom to top.
    let keys = order_keys(lanes.len());
    let mut lane_of: Vec<usize> = vec![0; clips.len()];
    for (position, (planned, order)) in lanes.iter().zip(keys).enumerate() {
        let id: LaneId = build.mint()?;
        let mut lane = Lane::new(sequence_id, order);
        if let Some(track) = planned.track.map(|t| &tracks[t]) {
            lane.visible = track.visible;
            lane.muted = track.muted;
            lane.captions = track.captions.clone();
        }
        let first_of_track =
            position == 0 || planned.track.is_none() || lanes[position - 1].track != planned.track;
        if first_of_track && let Some(track) = planned.track {
            let kept = std::mem::take(&mut tracks[track].kept);
            if !kept.is_empty() {
                let names = kept.names().join(", ");
                build.note(
                    &format!("track {}", tracks[track].old),
                    format!("kept {names}, which this version has no place for"),
                );
            }
            lane.residual = kept.residual(&mut build.words);
        }
        build
            .model
            .lanes
            .insert(id, lane)
            .map_err(|error| ImportError::Rejected(error.to_string()))?;
        for &at in &planned.clips {
            clips[at].item.lane = id;
            lane_of[at] = position;
        }
    }

    // A transition placed as a clip acts on what is beneath it; one over
    // nothing is a layer of its package, which plays as it did: over
    // nothing.
    for at in 0..clips.len() {
        if !clips[at].item.is_transition() {
            continue;
        }
        let (start, end) = (
            clips[at].item.start,
            clips[at].item.end().unwrap_or(Tick::MAX),
        );
        let beneath = clips.iter().enumerate().any(|(other, c)| {
            lane_of[other] < lane_of[at]
                && !c.item.is_transition()
                && c.item.start < end
                && c.item.end().unwrap_or(Tick::MAX) > start
        });
        if !beneath {
            let item = &mut clips[at].item;
            if let Content::Transition(effect) =
                std::mem::replace(&mut item.content, Content::Layer)
            {
                item.effects.insert(0, Entry::Known(effect));
            }
            let place = clips[at].place.clone();
            build.note(
                &place,
                "this transition is over no clip; it is a layer of its package now",
            );
        }
    }

    // Detached sound tied to the clip it came from.
    let index: HashMap<String, usize> = clips
        .iter()
        .enumerate()
        .map(|(at, clip)| (clip.old.clone(), at))
        .collect();
    for at in 0..clips.len() {
        let Some(from) = clips[at].detached_from else {
            continue;
        };
        let to = index.get(from).copied().filter(|to| *to != at);
        let fits = to.is_some_and(|to| {
            let (sound, picture) = (&clips[at].item.content, &clips[to].item.content);
            match (sound, picture) {
                (Content::Media(sound), Content::Media(picture)) => {
                    sound.take == Take::Sound
                        && picture.take == Take::Whole
                        && sound.media == picture.media
                        && clips[to].detached_from.is_none()
                }
                _ => false,
            }
        });
        match to {
            Some(to) if fits => {
                let to_id = clips[to].id;
                clips[at].item.link = Some(Link {
                    to: to_id,
                    kind: LinkKind::SoundOf,
                });
            }
            _ => {
                let place = clips[at].place.clone();
                build.note(&place, format!("it was detached from clip {from}, which is not a picture of its file in its timeline; it is kept apart"));
                clips[at]
                    .kept
                    .keep("", "detachedFrom", Value::String(from.to_owned()));
            }
        }
    }

    // In the document.
    let mut items = HashMap::with_capacity(clips.len());
    let mut sources = HashMap::with_capacity(clips.len());
    for clip in clips {
        let Clip {
            old,
            place,
            id,
            mut item,
            kept,
            source,
            ..
        } = clip;
        if !kept.is_empty() && !matches!(item.content, Content::Unknown { .. }) {
            let names = kept.names().join(", ");
            build.note(
                &place,
                format!("kept {names}, which this version has no place for"),
            );
        }
        if !matches!(item.content, Content::Unknown { .. }) {
            item.residual = kept.residual(&mut build.words);
        }
        build
            .model
            .items
            .insert(id, item)
            .map_err(|error| ImportError::Rejected(error.to_string()))?;
        match items.entry(old) {
            std::collections::hash_map::Entry::Occupied(_) => build.note(
                &place,
                "its id is another clip's too; what named it names the first",
            ),
            std::collections::hash_map::Entry::Vacant(free) => {
                free.insert(id);
            }
        }
        sources.insert(id, source);
    }
    Ok(Made {
        old,
        sequence: sequence_id,
        items,
        sources,
    })
}

/// A marker: a place, a length, words and a colour.
fn marker(build: &mut Build, value: &Value) -> Result<Option<Marker>, ImportError> {
    let mut m = Fields::of(value);
    let at = ["time", "at", "start"].iter().find_map(|key| m.f64(key));
    let Some(at) = at.and_then(tick) else {
        return Ok(None);
    };
    let len = ["duration", "length"]
        .iter()
        .find_map(|key| m.f64(key))
        .and_then(dur)
        .filter(|d| *d >= TickDur::ZERO)
        .unwrap_or(TickDur::ZERO);
    let name = ["name", "label", "text"]
        .iter()
        .find_map(|key| m.str(key))
        .unwrap_or("")
        .to_owned();
    let colour: Option<Color> = m.str("color").and_then(color);
    let mut kept = Kept::default();
    m.skip(&["id"]);
    kept.rest("", &m);
    Ok(Some(Marker {
        id: build.mint()?,
        at,
        len,
        name,
        color: colour,
        residual: kept.residual(&mut build.words),
    }))
}

/// Makes an item that broke a rule inert: kept whole as the clip it was,
/// on a lane of its own on top of its sequence, tied to nothing.
pub(crate) fn set_aside(
    build: &mut Build,
    id: ItemId,
    source: Option<&Value>,
) -> Result<(), ImportError> {
    let Some(item) = build.model.items.get(id).map(|item| item.as_ref().clone()) else {
        return Ok(());
    };
    let sequence = build.model.lanes.get(item.lane).map(|lane| lane.sequence);
    let Some(sequence) = sequence else {
        return Ok(());
    };
    let top = build
        .model
        .lanes
        .index()
        .of(sequence)
        .map(|(_, order)| order.clone())
        .max();
    let order = OrderKey::between(top.as_ref(), None).unwrap_or_else(OrderKey::middle);
    let lane: LaneId = build.mint()?;
    build
        .model
        .lanes
        .insert(lane, Lane::new(sequence, order))
        .map_err(|error| ImportError::Rejected(error.to_string()))?;
    let raw = build.raw(source.unwrap_or(&Value::Null));
    let mut inert = Item::new(
        lane,
        item.start,
        item.len,
        Content::Unknown {
            kind: "earlier-clip".to_owned(),
            raw,
        },
    );
    inert.name = item.name.clone();
    build
        .model
        .items
        .replace(id, inert)
        .map_err(|error| ImportError::Rejected(error.to_string()))?;
    // What was tied to it is tied to nothing now.
    let tied: Vec<ItemId> = build.model.items.index().tied_to(id).collect();
    for other in tied {
        build
            .model
            .items
            .update(other, |item| item.link = None)
            .map_err(|error| ImportError::Rejected(error.to_string()))?;
    }
    Ok(())
}

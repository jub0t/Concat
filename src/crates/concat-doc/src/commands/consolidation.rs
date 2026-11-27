// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Which clips may be consolidated into one file: the rule a selection
//! must pass before it is rendered and put back as one clip
//! ([`Command::ConsolidateClips`](super::Command::ConsolidateClips)).
//!
//! The render is of the clips alone, opaque, so the rule refuses what the
//! one file could not stand in for: clips of anything but footage, clips
//! on two lanes, clips that overlap, a sound detached from one of them, a
//! clip left out from inside the span, something visible beneath the
//! span, and a transition on the cut into the span or out of it.

use crate::model::{Content, ItemId, LinkKind, MediaKind, Model, Take};
use crate::time::Tick;

/// The selected clips in timeline order, or why they cannot be
/// consolidated, in plain words: the first rule they break.
pub fn consolidation(model: &Model, items: &[ItemId]) -> Result<Vec<ItemId>, String> {
    let refuse = |why: &str| Err(why.to_owned());
    let mut seen: Vec<ItemId> = Vec::new();
    for id in items {
        if seen.contains(id) {
            return refuse("Select two or more distinct video clips to merge.");
        }
        seen.push(*id);
    }
    if seen.len() < 2 {
        return refuse("Select two or more distinct video clips to merge.");
    }
    let mut clips = Vec::with_capacity(seen.len());
    for id in &seen {
        let Some(item) = model.items.get(*id) else {
            return refuse("A selected clip no longer exists.");
        };
        if model.view.showing.is_some_and(|showing| {
            model
                .lanes
                .get(item.lane)
                .is_some_and(|lane| lane.sequence != showing)
        }) {
            return refuse("A selected clip no longer exists.");
        }
        clips.push((*id, item));
    }
    let lane = clips[0].1.lane;
    let footage = |item: &crate::model::Item| match &item.content {
        Content::Media(cut) => {
            cut.take == Take::Whole
                && model
                    .media
                    .get(cut.media)
                    .is_some_and(|media| media.kind == MediaKind::Video)
        }
        _ => false,
    };
    if clips
        .iter()
        .any(|(_, item)| item.lane != lane || !footage(item))
    {
        return refuse("Select video clips on the same track to merge.");
    }
    let Some(lane_row) = model.lanes.get(lane) else {
        return refuse("That track no longer exists.");
    };
    clips.sort_by_key(|(_, item)| item.start);
    for pair in clips.windows(2) {
        if pair[1].1.start < pair[0].1.start + pair[0].1.len {
            return refuse("Overlapping clips cannot be merged.");
        }
    }
    let selected: Vec<ItemId> = clips.iter().map(|(id, _)| *id).collect();
    for id in &selected {
        let detached = model.items.index().tied_to(*id).any(|other| {
            model.items.get(other).is_some_and(|item| {
                item.link
                    .as_ref()
                    .is_some_and(|link| link.to == *id && link.kind == LinkKind::SoundOf)
            })
        });
        if detached {
            return refuse("Reattach detached audio before merging its video clips.");
        }
    }
    let from = clips[0].1.start;
    let to: Tick = clips
        .iter()
        .map(|(_, item)| item.start + item.len)
        .max()
        .unwrap_or(from);
    let inside = model
        .items
        .index()
        .overlapping(lane, from, to)
        .into_iter()
        .any(|(id, start, end)| !selected.contains(&id) && start < to && end > from);
    if inside {
        return refuse("Select every clip inside the span to merge.");
    }
    for (below, _) in model.lanes.index().of(lane_row.sequence) {
        if below == lane {
            break;
        }
        if !model.lanes.get(below).is_some_and(|lane| lane.visible) {
            continue;
        }
        let covered = model
            .items
            .index()
            .overlapping(below, from, to)
            .into_iter()
            .any(|(id, start, end)| {
                start < to
                    && end > from
                    && model
                        .items
                        .get(id)
                        .is_some_and(|item| !is_sound(model, item))
            });
        if covered {
            return refuse(
                "Clips above other visible content cannot be merged into an opaque video.",
            );
        }
    }
    let after_edge = model.items.index().on_lane(lane).any(|(id, start, _)| {
        start == to
            && !selected.contains(&id)
            && model
                .items
                .get(id)
                .is_some_and(|item| item.cut_in.is_some())
    });
    if clips[0].1.cut_in.is_some() || after_edge {
        return refuse("Include both sides of a transition in the merge.");
    }
    Ok(selected)
}

/// Whether an item is sound alone.
fn is_sound(model: &Model, item: &crate::model::Item) -> bool {
    match &item.content {
        Content::Media(cut) => {
            cut.take == Take::Sound
                || model
                    .media
                    .get(cut.media)
                    .is_some_and(|media| media.kind == MediaKind::Audio)
        }
        Content::SoundLayer => true,
        _ => false,
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Placing clips, dragging them, trimming, cutting, copying, joining and
//! taking them away, and a video's sound.

use std::collections::{BTreeMap, BTreeSet};

use crate::id::TypedId;
use crate::model::{
    Content, Effect, Enhanced, Entry, Item, ItemId, LaneId, Link, LinkKind, Media, MediaCut,
    MediaId, MediaKind, Picture, ShapeStyle, Sound, Take, Title,
};
use crate::time::{Tick, TickDur, TickLen};

use super::place::{Stack, is_free, lanes};
use super::speed::{window_limits, with_in_point};
use super::{
    ClipMove, CommandError, Ed, Edge, FREEZE_LEN, MIN_LEN, MIN_TRANSITION, STILL_LEN, TITLE_LEN,
    TRANSITION_LEN, first_line, refuse,
};

fn floor_zero(t: Tick) -> Tick {
    t.max(Tick::ZERO)
}

/// How long a clip of a file lasts when placed.
fn length_of(file: &Media) -> TickLen {
    match (&file.kind, file.length) {
        (MediaKind::Video | MediaKind::Audio, Some(length)) => length,
        _ => STILL_LEN,
    }
}

impl Ed<'_> {
    fn media(&self, id: MediaId) -> Result<std::sync::Arc<Media>, CommandError> {
        self.d.media.get(id).cloned().ok_or(CommandError::Gone {
            what: "file",
            id: id.any(),
        })
    }

    /// A clip of the whole of a file, not yet placed.
    fn cut(&self, media: MediaId, lane: LaneId, start: Tick) -> Result<Item, CommandError> {
        let file = self.media(media)?;
        let mut item = Item::new(
            lane,
            start,
            length_of(&file),
            Content::Media(MediaCut::whole(media)),
        );
        item.name.clone_from(&file.name);
        Ok(item)
    }

    /// `addClip`.
    pub(crate) fn add_clip(
        &mut self,
        media: MediaId,
        lane: LaneId,
        start: Tick,
        ripple: bool,
    ) -> Result<ItemId, CommandError> {
        let sequence = self.sequence_of(lane)?;
        let start = floor_zero(start);
        let mut item = self.cut(media, lane, start)?;
        if !is_free(self.d, lane, start, item.len, &[]) {
            if ripple {
                self.ripple(lane, start, item.len.dur(), true, &[])?;
            }
            if !is_free(self.d, lane, start, item.len, &[]) {
                item.lane = self.new_lane(sequence, Stack::Above(lane))?;
            }
        }
        self.insert_item(item)
    }

    /// `addClipAtFirstFree`.
    pub(crate) fn add_clip_first_free(
        &mut self,
        media: MediaId,
        start: Tick,
    ) -> Result<ItemId, CommandError> {
        let sequence = self.showing()?;
        let start = floor_zero(start);
        let len = length_of(&*self.media(media)?);
        let lane = self.first_free(sequence, start, len)?;
        let item = self.cut(media, lane, start)?;
        self.insert_item(item)
    }

    /// `addClipsInSequence`.
    pub(crate) fn add_clips_in_sequence(
        &mut self,
        media: &[MediaId],
        start: Tick,
        lane: Option<LaneId>,
        ripple: bool,
    ) -> Result<(), CommandError> {
        let mut at = floor_zero(start);
        let mut made = Vec::new();
        match lane {
            Some(lane) => {
                for file in media {
                    let id = self.add_clip(*file, lane, at, ripple)?;
                    let item = self.item(id)?;
                    at = item.start + item.len;
                    made.push(id);
                }
            }
            None => {
                let sequence = self.showing()?;
                let mut total = TickDur::ZERO;
                for file in media {
                    total = total + length_of(&*self.media(*file)?).dur();
                }
                let Some(total) = total.to_len() else {
                    return Ok(());
                };
                let lane = self.first_free(sequence, at, total)?;
                for file in media {
                    let item = self.cut(*file, lane, at)?;
                    at += item.len;
                    made.push(self.insert_item(item)?);
                }
            }
        }
        self.out.created = made.first().map(|id| id.any());
        self.out.selection = Some(made);
        Ok(())
    }

    /// `addTextClip`.
    pub(crate) fn add_title(
        &mut self,
        start: Tick,
        len: Option<TickLen>,
        lane: Option<LaneId>,
        above: bool,
        offset_y: Option<f64>,
        title: Option<&Title>,
    ) -> Result<ItemId, CommandError> {
        let start = floor_zero(start);
        let len = len.unwrap_or(TITLE_LEN);
        let title = title.cloned().unwrap_or_default();
        let lane = self.lane_for(lane, above, start, len)?;
        let name = first_line(&title.content);
        let mut item = Item::new(lane, start, len, Content::Title(title));
        item.name = name;
        if let Some(y) = offset_y.filter(|y| y.is_finite()) {
            item.picture.offset_y.value = y.clamp(-3.0, 3.0);
        }
        self.insert_item(item)
    }

    /// `addShapeClip`.
    pub(crate) fn add_shape(
        &mut self,
        start: Tick,
        len: Option<TickLen>,
        name: &str,
        style: Option<&ShapeStyle>,
        lane: Option<LaneId>,
        above: bool,
    ) -> Result<ItemId, CommandError> {
        let start = floor_zero(start);
        let len = len.unwrap_or(TITLE_LEN);
        let lane = self.lane_for(lane, above, start, len)?;
        let mut item = Item::new(
            lane,
            start,
            len,
            Content::Shape(style.cloned().unwrap_or_default()),
        );
        item.name = name.to_owned();
        self.insert_item(item)
    }

    /// `addLayerClip`.
    pub(crate) fn add_layer(
        &mut self,
        start: Tick,
        len: Option<TickLen>,
        package: &str,
        name: &str,
        sound: bool,
        lane: Option<LaneId>,
    ) -> Result<ItemId, CommandError> {
        let start = floor_zero(start);
        let len = len.unwrap_or(TITLE_LEN);
        let lane = self.lane_for(lane, true, start, len)?;
        let effect = Effect::new(self.mint()?, package);
        let content = if sound {
            Content::SoundLayer
        } else {
            Content::Layer
        };
        let mut item = Item::new(lane, start, len, content);
        item.name = name.to_owned();
        if sound {
            item.filters.push(Entry::Known(effect));
        } else {
            item.effects.push(Entry::Known(effect));
        }
        self.insert_item(item)
    }

    /// `addTransitionClip`.
    pub(crate) fn add_transition(
        &mut self,
        start: Tick,
        len: Option<TickLen>,
        package: &str,
        name: &str,
        lane: Option<LaneId>,
    ) -> Result<ItemId, CommandError> {
        let start = floor_zero(start);
        let len = len.unwrap_or(TRANSITION_LEN).max(MIN_TRANSITION);
        let lane = self.lane_for(lane, true, start, len)?;
        let effect = Effect::new(self.mint()?, package);
        let mut item = Item::new(lane, start, len, Content::Transition(effect));
        item.name = name.to_owned();
        self.insert_item(item)
    }

    /// `moveClips`: each clip to its place; one that would land on
    /// another goes to the first lane above that is free there, or a new
    /// one on top.
    pub(crate) fn move_clips(&mut self, moves: &[ClipMove]) -> Result<(), CommandError> {
        let mut moved = Vec::new();
        for step in moves {
            let Some(item) = self.d.items.get(step.item).cloned() else {
                continue;
            };
            let home = self.sequence_of(item.lane)?;
            let lane = step
                .lane
                .filter(|lane| self.d.lanes.get(*lane).is_some_and(|l| l.sequence == home))
                .unwrap_or(item.lane);
            let start = floor_zero(step.start);
            self.update(step.item, |item| {
                item.lane = lane;
                item.start = start;
            })?;
            moved.push(step.item);
        }
        for id in moved {
            let item = self.item(id)?;
            if !is_free(self.d, item.lane, item.start, item.len, &[id]) {
                let lane = self.free_from(item.lane, item.start, item.len, &[id])?;
                self.update(id, |item| item.lane = lane)?;
            }
        }
        Ok(())
    }

    /// `moveClipsToNewTrack`.
    pub(crate) fn move_to_new_lane(
        &mut self,
        moves: &[ClipMove],
        bottom: bool,
    ) -> Result<(), CommandError> {
        let mut home = None;
        let mut landing: Vec<(ItemId, Tick, Tick)> = Vec::new();
        for step in moves {
            let Some(item) = self.d.items.get(step.item).cloned() else {
                continue;
            };
            let sequence = self.sequence_of(item.lane)?;
            if *home.get_or_insert(sequence) != sequence {
                continue;
            }
            let start = floor_zero(step.start);
            let end = start + item.len;
            if landing.iter().any(|(_, s, e)| start < *e && *s < end) {
                continue;
            }
            landing.push((step.item, start, end));
        }
        let Some(sequence) = home.filter(|_| !landing.is_empty()) else {
            return Ok(());
        };
        let lane = self.new_lane(sequence, if bottom { Stack::Bottom } else { Stack::Top })?;
        for (id, start, _) in landing {
            self.update(id, |item| {
                item.lane = lane;
                item.start = start;
            })?;
        }
        self.out.created = Some(lane.any());
        Ok(())
    }

    /// `trimClip`.
    pub(crate) fn trim(
        &mut self,
        id: ItemId,
        edge: Edge,
        delta: TickDur,
        ripple: bool,
    ) -> Result<(), CommandError> {
        let Some(item) = self.d.items.get(id).cloned() else {
            return Ok(());
        };
        let floor = MIN_LEN.min(item.len).dur();
        let (low, high) = window_limits(self.d, &item);
        let end = item.start + item.len;
        let window_end = item.offset + item.len;
        let neighbours: Vec<(ItemId, Tick, Tick)> =
            self.d.items.index().on_lane(item.lane).collect();
        match edge {
            Edge::Tail => {
                let mut change = delta;
                // Not shorter than the floor.
                change = change.max(floor - item.len.dur());
                if let Some(high) = high {
                    change = change.min(high - window_end);
                }
                if !ripple {
                    let next = neighbours
                        .iter()
                        .filter(|(other, start, _)| *other != id && *start >= end)
                        .map(|(_, start, _)| *start)
                        .min();
                    if let Some(next) = next {
                        change = change.min(next - end);
                    }
                }
                let Some(len) = (item.len.dur() + change).to_len() else {
                    return Ok(());
                };
                if change == TickDur::ZERO {
                    return Ok(());
                }
                self.update(id, |item| item.len = len)?;
                if ripple {
                    self.ripple(item.lane, end, change, true, &[id])?;
                }
            }
            Edge::Head => {
                let mut shift = delta;
                shift = shift.min(item.len.dur() - floor);
                if let Some(low) = low {
                    shift = shift.max(low - item.offset);
                }
                if !ripple {
                    shift = shift.max(Tick::ZERO - item.start);
                    let previous = neighbours
                        .iter()
                        .filter(|(other, _, e)| *other != id && *e <= item.start)
                        .map(|(_, _, e)| *e)
                        .max();
                    if let Some(previous) = previous {
                        shift = shift.max(previous - item.start);
                    }
                }
                if shift == TickDur::ZERO {
                    return Ok(());
                }
                let Some(len) = (item.len.dur() - shift).to_len() else {
                    return Ok(());
                };
                self.update(id, |item| {
                    item.offset += shift;
                    item.len = len;
                    if !ripple {
                        item.start += shift;
                    }
                })?;
                if ripple {
                    self.ripple(item.lane, end, -shift, true, &[id])?;
                }
            }
        }
        Ok(())
    }

    /// Cuts one item in two at a timeline tick strictly inside it, and
    /// gives the tail's id.
    pub(crate) fn split_one(
        &mut self,
        id: ItemId,
        at: Tick,
        margin: TickDur,
    ) -> Result<Option<ItemId>, CommandError> {
        let Some(item) = self.d.items.get(id).cloned() else {
            return Ok(None);
        };
        let end = item.start + item.len;
        if at <= item.start + margin || at >= end - margin || at <= item.start || at >= end {
            return Ok(None);
        }
        let (Some(head), Some(tail)) = ((at - item.start).to_len(), (end - at).to_len()) else {
            return Ok(None);
        };
        let cut = item.offset + head;
        let mut copy = self.copy_of(&item)?;
        copy.start = at;
        copy.offset = cut;
        copy.len = tail;
        copy.cut_in = None;
        copy.sound.fade_in = TickDur::ZERO;
        copy.picture.motion.enter = None;
        copy.markers
            .retain(|m| m.known().is_none_or(|m| m.at >= cut));
        self.update(id, |item| {
            item.len = head;
            item.sound.fade_out = TickDur::ZERO;
            item.picture.motion.exit = None;
            item.markers
                .retain(|m| m.known().is_none_or(|m| m.at < cut));
        })?;
        Ok(Some(self.insert_item(copy)?))
    }

    /// `splitClips`.
    pub(crate) fn split_clips(&mut self, items: &[ItemId], at: Tick) -> Result<(), CommandError> {
        let mut first = None;
        for id in items {
            if let Some(tail) = self.split_one(*id, at, MIN_LEN.dur())? {
                first.get_or_insert(tail);
            }
        }
        if let Some(tail) = first {
            self.out.created = Some(tail.any());
        }
        Ok(())
    }

    /// `duplicateClips`.
    pub(crate) fn duplicate(&mut self, items: &[ItemId]) -> Result<(), CommandError> {
        let mut known: Vec<(ItemId, std::sync::Arc<Item>)> = items
            .iter()
            .filter_map(|id| self.d.items.get(*id).cloned().map(|item| (*id, item)))
            .collect();
        known.sort_by_key(|(id, item)| (std::cmp::Reverse(item.start), *id));
        known.dedup_by_key(|(id, _)| *id);
        let mut copies = Vec::new();
        for (_, item) in &known {
            let mut copy = self.copy_of(item)?;
            copy.cut_in = None;
            copy.link = None;
            copy.start =
                super::place::gap_from(self.d, item.lane, item.start + item.len, item.len, &[]);
            copies.push(self.insert_item(copy)?);
        }
        self.out.created = copies.last().map(|id| id.any());
        copies.reverse();
        self.out.selection = Some(copies);
        Ok(())
    }

    /// `removeClips`.
    pub(crate) fn remove_clips(
        &mut self,
        items: &[ItemId],
        ripple: bool,
    ) -> Result<(), CommandError> {
        let sequences = self.sequences_of(items);
        let mut spans: BTreeMap<LaneId, Vec<(Tick, TickDur)>> = BTreeMap::new();
        let mut gone = BTreeSet::new();
        for id in items {
            if let Some(item) = self.d.items.get(*id).cloned()
                && gone.insert(*id)
            {
                spans
                    .entry(item.lane)
                    .or_default()
                    .push((item.start, item.len.dur()));
            }
        }
        for id in &gone {
            self.take_out(*id)?;
        }
        if ripple {
            for (lane, spans) in spans {
                let on: Vec<(ItemId, Tick, Tick)> = self.d.items.index().on_lane(lane).collect();
                for (id, start, _) in on {
                    let by = spans
                        .iter()
                        .filter(|(s, _)| *s < start)
                        .fold(TickDur::ZERO, |sum, (_, len)| sum + *len);
                    if by > TickDur::ZERO {
                        self.update(id, |item| item.start = floor_zero(start - by))?;
                    }
                }
            }
        }
        for sequence in sequences {
            self.sweep_transitions(sequence)?;
        }
        Ok(())
    }

    /// `mergeClips`.
    pub(crate) fn merge(&mut self, items: &[ItemId]) -> Result<(), CommandError> {
        let mut pieces: Vec<(ItemId, std::sync::Arc<Item>)> = Vec::new();
        for id in items {
            if pieces.iter().any(|(p, _)| p == id) {
                continue;
            }
            pieces.push((*id, self.item(*id)?));
        }
        if pieces.len() < 2 {
            return Ok(());
        }
        pieces.sort_by_key(|(_, item)| item.start);
        let (first_id, first) = pieces[0].clone();
        for pair in pieces.windows(2) {
            let (a, b) = (&pair[0].1, &pair[1].1);
            if a.lane != b.lane {
                return refuse("Only pieces on one lane can be merged.");
            }
            if a.content != b.content {
                return refuse("Only pieces of one clip can be merged.");
            }
            if a.start + a.len != b.start {
                return refuse("Merged pieces must touch.");
            }
            if a.offset + a.len != b.offset {
                return refuse("Pieces can be merged only in the order they were cut.");
            }
        }
        let (_, last) = pieces[pieces.len() - 1].clone();
        let Some(len) = ((last.start + last.len) - first.start).to_len() else {
            return Ok(());
        };
        let mut markers = Vec::new();
        for (id, piece) in &pieces[1..] {
            markers.extend(piece.markers.iter().cloned());
            let tied: Vec<ItemId> = self.d.items.index().tied_to(*id).collect();
            for sound in tied {
                self.update(sound, |s| {
                    if let Some(link) = &mut s.link {
                        link.to = first_id;
                    }
                })?;
            }
            self.d.items.remove(*id)?;
        }
        self.update(first_id, |item| {
            item.len = len;
            item.sound.fade_out = last.sound.fade_out;
            item.picture
                .motion
                .exit
                .clone_from(&last.picture.motion.exit);
            item.markers.extend(markers);
        })?;
        self.out.created = Some(first_id.any());
        Ok(())
    }

    /// `freezeFrame`.
    pub(crate) fn freeze(
        &mut self,
        id: ItemId,
        at: Tick,
        len: Option<TickLen>,
        still: Option<&Media>,
    ) -> Result<(), CommandError> {
        let Some(item) = self.d.items.get(id).cloned() else {
            return Ok(());
        };
        let Content::Media(cut) = &item.content else {
            return Ok(());
        };
        let kind = self.media(cut.media)?.kind.clone();
        if cut.take != Take::Whole || !matches!(kind, MediaKind::Video | MediaKind::Image) {
            return Ok(());
        }
        if at <= item.start || at >= item.start + item.len {
            return refuse("A freeze frame is made inside the clip.");
        }
        let media = match (&kind, still) {
            (MediaKind::Image, None) => cut.media,
            (_, Some(still)) => self.add_media(still)?,
            (_, None) => return refuse("A freeze frame of a video needs its still."),
        };
        let hold = len.unwrap_or(FREEZE_LEN).max(MIN_LEN);
        self.split_one(id, at, TickDur::ZERO)?;
        self.ripple(item.lane, at, hold.dur(), true, &[])?;
        let mut frozen = self.copy_of(&item)?;
        frozen.content = Content::Media(MediaCut::whole(media));
        frozen.start = at;
        frozen.offset = Tick::ZERO;
        frozen.len = hold;
        frozen.cut_in = None;
        frozen.link = None;
        frozen.markers.clear();
        frozen.sound = Sound::default();
        frozen.picture.motion = Default::default();
        frozen.name.clone_from(&self.media(media)?.name);
        drop_keys(&mut frozen);
        let made = self.insert_item(frozen)?;
        self.out.created = Some(made.any());
        self.out.selection = Some(vec![made]);
        Ok(())
    }

    /// `consolidateClips`.
    pub(crate) fn consolidate(
        &mut self,
        items: &[ItemId],
        file: &Media,
    ) -> Result<(), CommandError> {
        let mut pieces: Vec<(ItemId, std::sync::Arc<Item>)> = Vec::new();
        for id in items {
            if !pieces.iter().any(|(p, _)| p == id) {
                pieces.push((*id, self.item(*id)?));
            }
        }
        if pieces.len() < 2 {
            return refuse("Choose two or more clips to consolidate.");
        }
        if pieces.iter().any(|(_, p)| p.lane != pieces[0].1.lane) {
            return refuse("Only clips on one lane can be consolidated.");
        }
        pieces.sort_by_key(|(_, item)| item.start);
        let (keep, first) = pieces[0].clone();
        let end = pieces
            .iter()
            .map(|(_, p)| p.start + p.len)
            .max()
            .unwrap_or(first.start + first.len);
        let Some(len) = (end - first.start).to_len() else {
            return Ok(());
        };
        let media = self.add_media(file)?;
        for (id, _) in &pieces[1..] {
            self.take_out(*id)?;
        }
        let name = file.name.clone();
        self.update(keep, |item| {
            let mut fresh = Item::new(
                item.lane,
                item.start,
                len,
                Content::Media(MediaCut::whole(media)),
            );
            fresh.name = name;
            *item = fresh;
        })?;
        let sequences = self.sequences_of(&[keep]);
        for sequence in sequences {
            self.sweep_transitions(sequence)?;
        }
        Ok(())
    }

    /// `replaceClipMedia`.
    pub(crate) fn replace_media(
        &mut self,
        id: ItemId,
        file: &Media,
        enhanced: bool,
        copy_start: TickDur,
        source_start: Option<Tick>,
    ) -> Result<(), CommandError> {
        let Some(item) = self.d.items.get(id).cloned() else {
            return Ok(());
        };
        let Content::Media(cut) = &item.content else {
            return Ok(());
        };
        let media = self.add_media(file)?;
        let mut cut = cut.clone();
        let copy_start = copy_start.max(TickDur::ZERO);
        if enhanced {
            if media == cut.media {
                return Ok(());
            }
            cut.enhanced = Some(Enhanced {
                copy: media,
                copy_start,
                on: true,
            });
            if let Some(start) = source_start {
                let original = floor_zero(start) + copy_start;
                cut.time = with_in_point(&cut.time, item.offset, original).unwrap_or(cut.time);
            }
        } else {
            cut.media = media;
            cut.enhanced = None;
            cut.stream = None;
            if let Some(start) = source_start {
                cut.time =
                    with_in_point(&cut.time, item.offset, floor_zero(start)).unwrap_or(cut.time);
            }
        }
        self.update(id, |item| item.content = Content::Media(cut))
    }

    /// `setClipEnhanced`.
    pub(crate) fn set_enhanced(&mut self, id: ItemId, on: bool) -> Result<(), CommandError> {
        let Some(item) = self.d.items.get(id) else {
            return Ok(());
        };
        let Content::Media(MediaCut {
            enhanced: Some(_), ..
        }) = &item.content
        else {
            return Ok(());
        };
        self.update(id, |item| {
            if let Content::Media(MediaCut {
                enhanced: Some(e), ..
            }) = &mut item.content
            {
                e.on = on;
            }
        })
    }

    /// `detachAudio`.
    pub(crate) fn detach(&mut self, id: ItemId) -> Result<(), CommandError> {
        let Some(item) = self.d.items.get(id).cloned() else {
            return Ok(());
        };
        let Content::Media(cut) = &item.content else {
            return Ok(());
        };
        let file = self.media(cut.media)?;
        let tied = self.d.items.index().tied_to(id).next().is_some();
        if file.kind != MediaKind::Video
            || !file.has_sound
            || cut.take != Take::Whole
            || item.sound.muted
            || tied
        {
            return Ok(());
        }
        let sequence = self.sequence_of(item.lane)?;
        let lane = lanes(self.d, sequence)
            .into_iter()
            .find(|lane| is_free(self.d, *lane, item.start, item.len, &[]));
        let lane = match lane {
            Some(lane) => lane,
            None => self.new_lane(sequence, Stack::Top)?,
        };
        let mut sound_cut = cut.clone();
        sound_cut.take = Take::Sound;
        sound_cut.enhanced = None;
        let mut sound = Item::new(lane, item.start, item.len, Content::Media(sound_cut));
        sound.offset = item.offset;
        sound.name.clone_from(&item.name);
        sound.sound = item.sound.clone();
        sound.filters = item.filters.clone();
        sound.link = Some(Link {
            to: id,
            kind: LinkKind::SoundOf,
        });
        self.update(id, |item| {
            item.sound.muted = true;
            item.filters.clear();
        })?;
        let made = self.insert_item(sound)?;
        self.out.created = Some(made.any());
        Ok(())
    }

    /// `reattachAudio`.
    pub(crate) fn reattach(&mut self, id: ItemId) -> Result<(), CommandError> {
        let Some(item) = self.d.items.get(id).cloned() else {
            return Ok(());
        };
        let video = match &item.link {
            Some(Link {
                to,
                kind: LinkKind::SoundOf,
            }) => *to,
            _ => id,
        };
        if !self.d.items.contains(video) {
            return Ok(());
        }
        let mut sounds: Vec<(Tick, ItemId)> = self
            .d
            .items
            .index()
            .tied_to(video)
            .filter_map(|s| {
                let sound = self.d.items.get(s)?;
                matches!(
                    &sound.link,
                    Some(Link {
                        kind: LinkKind::SoundOf,
                        ..
                    })
                )
                .then_some((sound.start, s))
            })
            .collect();
        if sounds.is_empty() {
            return Ok(());
        }
        sounds.sort();
        let filters = self.item(sounds[0].1)?.filters.clone();
        let sequences = self.sequences_of(&[video]);
        for (_, sound) in &sounds {
            self.d.items.remove(*sound)?;
        }
        self.update(video, |item| {
            item.sound.muted = false;
            item.filters = filters;
        })?;
        for sequence in sequences {
            self.sweep_transitions(sequence)?;
        }
        Ok(())
    }
}

/// Every key of an item taken off, each number left at its value.
fn drop_keys(item: &mut Item) {
    let p: &mut Picture = &mut item.picture;
    for a in [
        &mut p.offset_x,
        &mut p.offset_y,
        &mut p.scale,
        &mut p.rotation,
        &mut p.stretch_x,
        &mut p.stretch_y,
        &mut p.opacity,
    ] {
        a.keys.clear();
    }
    for effect in item.effects.iter_mut().filter_map(Entry::known_mut) {
        for param in effect.params.values_mut() {
            param.keys.clear();
        }
        effect.params.retain(|_, param| param.value.is_some());
    }
    for mask in item.masks.iter_mut().filter_map(Entry::known_mut) {
        for a in [
            &mut mask.x,
            &mut mask.y,
            &mut mask.scale,
            &mut mask.rotation,
            &mut mask.feather,
            &mut mask.expand,
            &mut mask.opacity,
        ] {
            a.keys.clear();
        }
    }
}

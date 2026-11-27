// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The bin, caption lanes, lanes and timelines.

use crate::id::TypedId;
use crate::model::{
    Background, CaptionLook, Content, Cover, Frame, Item, ItemId, LaneId, Media, MediaId, Sequence,
    SequenceId,
};
use crate::time::Tick;

use super::place::{Stack, is_free, lanes, nearest_gap};
use super::speed::with_in_point;
use super::{CommandError, Cue, Ed, LaneFlag, first_line, refuse};

/// A caption look as a lane holds it: no words or runs of its own.
fn tidy_look(look: &CaptionLook) -> CaptionLook {
    let mut look = look.clone();
    look.title.content.clear();
    look.title.runs.clear();
    if !look.offset_y.is_finite() {
        look.offset_y = 0.0;
    }
    look.offset_y = look.offset_y.clamp(-2.0, 2.0);
    look
}

impl Ed<'_> {
    /// A file into the bin, or the one already there at its path.
    pub(crate) fn add_media(&mut self, media: &Media) -> Result<MediaId, CommandError> {
        if !media.path.is_empty()
            && let Some((id, _)) = self.d.media.iter().find(|(_, m)| m.path == media.path)
        {
            return Ok(id);
        }
        let id: MediaId = self.mint()?;
        self.d.media.insert(id, media.clone())?;
        Ok(id)
    }

    /// `removeMedia`.
    pub(crate) fn remove_media(&mut self, media: MediaId) -> Result<(), CommandError> {
        if !self.d.media.contains(media) {
            return Ok(());
        }
        let users: Vec<ItemId> = self.d.items.index().using_media(media).collect();
        let sequences = self.sequences_of(&users);
        for id in users {
            let Some(item) = self.d.items.get(id).cloned() else {
                continue;
            };
            if matches!(&item.content, Content::Media(cut) if cut.media == media) {
                self.take_out(id)?;
                continue;
            }
            self.update(id, |item| {
                if let Content::Media(cut) = &mut item.content
                    && cut.enhanced.as_ref().is_some_and(|e| e.copy == media)
                {
                    cut.enhanced = None;
                }
                if item.picture.background.as_ref().and_then(Background::media) == Some(media) {
                    item.picture.background = None;
                }
            })?;
        }
        let behind: Vec<SequenceId> = self
            .d
            .sequences
            .iter()
            .filter(|(_, s)| s.background.media() == Some(media))
            .map(|(id, _)| id)
            .collect();
        for id in behind {
            self.d
                .sequences
                .update(id, |s| s.background = Background::None)?;
        }
        if matches!(self.d.cover, Some(Cover::Picture { media: cover }) if cover == media) {
            self.d.cover = None;
        }
        self.d.media.remove(media)?;
        for sequence in sequences {
            self.sweep_transitions(sequence)?;
        }
        Ok(())
    }

    /// `fillSlot`.
    pub(crate) fn fill_slot(&mut self, media: MediaId, file: &Media) -> Result<(), CommandError> {
        let Some(slot) = self.d.media.get(media).cloned() else {
            return Err(CommandError::Gone {
                what: "file",
                id: media.any(),
            });
        };
        if !slot.placeholder {
            return refuse("Only a template's slot can be filled.");
        }
        let mut new = file.clone();
        new.placeholder = false;
        self.d.media.replace(media, new)?;
        let users: Vec<ItemId> = self.d.items.index().using_media(media).collect();
        for id in users {
            let name = file.name.clone();
            self.update(id, |item| {
                let offset = item.offset;
                if let Content::Media(cut) = &mut item.content
                    && cut.media == media
                {
                    if let Some(time) = with_in_point(&cut.time, offset, Tick::ZERO) {
                        cut.time = time;
                    }
                    cut.stream = None;
                    cut.enhanced = None;
                    item.name = name;
                }
            })?;
        }
        Ok(())
    }

    /// `placeCaptions`.
    pub(crate) fn place_captions(
        &mut self,
        cues: &[Cue],
        look: &CaptionLook,
        lane: Option<LaneId>,
    ) -> Result<(), CommandError> {
        let (lane, look) = match lane {
            Some(lane) => {
                let held = self
                    .d
                    .lanes
                    .get(lane)
                    .ok_or(CommandError::Gone {
                        what: "lane",
                        id: lane.any(),
                    })?
                    .captions
                    .clone();
                match held {
                    Some(look) => (lane, look),
                    None => {
                        let look = tidy_look(look);
                        let set = look.clone();
                        self.d.lanes.update(lane, |l| l.captions = Some(set))?;
                        (lane, look)
                    }
                }
            }
            None => {
                let sequence = self.showing()?;
                let lane = self.new_lane(sequence, Stack::Top)?;
                let look = tidy_look(look);
                let set = look.clone();
                self.d.lanes.update(lane, |l| l.captions = Some(set))?;
                (lane, look)
            }
        };
        let mut cues: Vec<&Cue> = cues.iter().filter(|c| !c.words.trim().is_empty()).collect();
        cues.sort_by_key(|c| c.start);
        for (n, cue) in cues.iter().enumerate() {
            let start = cue.start.max(Tick::ZERO);
            let mut end = start + cue.len;
            if let Some(next) = cues.get(n + 1)
                && next.start > start
            {
                end = end.min(next.start);
            }
            let Some(len) = (end - start).to_len() else {
                continue;
            };
            let start = if is_free(self.d, lane, start, len, &[]) {
                start
            } else {
                nearest_gap(self.d, lane, start, len, &[])
            };
            let mut item = Item::new(
                lane,
                start,
                len,
                Content::Title(look.title_for(cue.words.clone(), Vec::new())),
            );
            item.name = first_line(&cue.words);
            item.picture.offset_y.value = look.offset_y;
            self.insert_item(item)?;
        }
        self.out.created = Some(lane.any());
        Ok(())
    }

    /// `setCaptionLook`.
    pub(crate) fn set_caption_look(
        &mut self,
        lane: LaneId,
        look: Option<&CaptionLook>,
    ) -> Result<(), CommandError> {
        if !self.d.lanes.contains(lane) {
            return Ok(());
        }
        let look = look.map(tidy_look);
        self.d
            .lanes
            .update(lane, |l| l.captions.clone_from(&look))?;
        let Some(look) = look else {
            return Ok(());
        };
        let on: Vec<ItemId> = self
            .d
            .items
            .index()
            .on_lane(lane)
            .map(|(id, _, _)| id)
            .collect();
        for id in on {
            self.update(id, |item| {
                if let Content::Title(title) = &item.content {
                    let restyled = look.title_for(title.content.clone(), title.runs.clone());
                    item.content = Content::Title(restyled);
                    item.picture.offset_y.value = look.offset_y;
                }
            })?;
        }
        Ok(())
    }

    /// `removeTrack`.
    pub(crate) fn remove_lane(&mut self, lane: LaneId) -> Result<(), CommandError> {
        let sequence = self.sequence_of(lane)?;
        if lanes(self.d, sequence).len() <= 1 {
            return refuse("A timeline keeps at least one lane.");
        }
        let on: Vec<ItemId> = self
            .d
            .items
            .index()
            .on_lane(lane)
            .map(|(id, _, _)| id)
            .collect();
        for id in on {
            self.take_out(id)?;
        }
        self.d.lanes.remove(lane)?;
        self.sweep_transitions(sequence)
    }

    /// `setTrackFlag`.
    pub(crate) fn set_lane_flag(
        &mut self,
        lane: LaneId,
        flag: LaneFlag,
        value: bool,
    ) -> Result<(), CommandError> {
        if !self.d.lanes.contains(lane) {
            return Ok(());
        }
        self.d.lanes.update(lane, |l| match flag {
            LaneFlag::Visible => l.visible = value,
            LaneFlag::Muted => l.muted = value,
        })?;
        Ok(())
    }

    /// `addTimeline`.
    pub(crate) fn add_timeline(&mut self) -> Result<(), CommandError> {
        let highest = self
            .d
            .sequences
            .iter()
            .filter_map(|(_, s)| s.name.strip_prefix("Timeline ")?.trim().parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        let frame = self
            .d
            .view
            .showing
            .and_then(|s| self.d.sequences.get(s))
            .map_or_else(Frame::hd, |s| s.frame.clone());
        let id: SequenceId = self.mint()?;
        let name = format!("Timeline {}", highest.saturating_add(1));
        self.d.sequences.insert(id, Sequence::new(name, frame))?;
        for _ in 0..4 {
            self.new_lane(id, Stack::Top)?;
        }
        self.d.view.showing = Some(id);
        self.d.view.tabs.push(id);
        self.out.created = Some(id.any());
        Ok(())
    }

    /// `removeTimeline`.
    pub(crate) fn remove_timeline(&mut self, sequence: SequenceId) -> Result<(), CommandError> {
        if !self.d.sequences.contains(sequence) {
            return Ok(());
        }
        if self.d.sequences.len() <= 1 {
            return refuse("A project keeps at least one timeline.");
        }
        if self.d.items.index().nesting(sequence).next().is_some() {
            return refuse("That timeline is placed inside another; take it out of there first.");
        }
        for lane in lanes(self.d, sequence) {
            let on: Vec<ItemId> = self
                .d
                .items
                .index()
                .on_lane(lane)
                .map(|(id, _, _)| id)
                .collect();
            for id in on {
                self.d.items.remove(id)?;
            }
            self.d.lanes.remove(lane)?;
        }
        self.d.sequences.remove(sequence)?;
        let tabs = &mut self.d.view.tabs;
        let place = tabs.iter().position(|t| *t == sequence);
        tabs.retain(|t| *t != sequence);
        if self.d.view.showing == Some(sequence) {
            let tabs = &self.d.view.tabs;
            let next = place
                .and_then(|at| {
                    tabs.get(at)
                        .or_else(|| at.checked_sub(1).and_then(|b| tabs.get(b)))
                })
                .or(tabs.first())
                .copied()
                .or_else(|| self.d.sequences.ids().next());
            self.d.view.showing = next;
        }
        Ok(())
    }

    /// `selectTimeline`.
    pub(crate) fn select_timeline(&mut self, sequence: SequenceId) -> Result<(), CommandError> {
        if !self.d.sequences.contains(sequence) {
            return Ok(());
        }
        self.d.view.showing = Some(sequence);
        if !self.d.view.tabs.contains(&sequence) {
            self.d.view.tabs.push(sequence);
        }
        Ok(())
    }

    /// `moveTimeline`.
    pub(crate) fn move_timeline(
        &mut self,
        sequence: SequenceId,
        index: usize,
    ) -> Result<(), CommandError> {
        let tabs = &mut self.d.view.tabs;
        let Some(at) = tabs.iter().position(|t| *t == sequence) else {
            return Ok(());
        };
        tabs.remove(at);
        let index = index.min(tabs.len());
        tabs.insert(index, sequence);
        Ok(())
    }
}

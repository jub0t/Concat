// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Where things go: free lanes, gaps, new lanes, and moving what follows
//! along a lane.

use std::collections::BTreeSet;

use crate::model::{Item, ItemId, Lane, LaneId, Model, OrderKey, SequenceId};
use crate::time::{Tick, TickDur, TickLen};

use super::{CommandError, Ed};

/// Where a new lane goes in its sequence's stack.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Stack {
    /// Above every other.
    Top,
    /// Under every other.
    Bottom,
    /// Just above this one.
    Above(LaneId),
}

/// A sequence's lanes, bottom first.
pub(crate) fn lanes(m: &Model, sequence: SequenceId) -> Vec<LaneId> {
    m.lanes.index().of(sequence).map(|(id, _)| id).collect()
}

/// Whether nothing but `except` is on a lane over `[start, start + len)`.
pub(crate) fn is_free(
    m: &Model,
    lane: LaneId,
    start: Tick,
    len: TickLen,
    except: &[ItemId],
) -> bool {
    let end = start + len;
    m.items
        .index()
        .overlapping(lane, start, end)
        .iter()
        .all(|(id, _, _)| except.contains(id))
}

/// The first place at or after `from` where `len` fits on a lane.
pub(crate) fn gap_from(
    m: &Model,
    lane: LaneId,
    from: Tick,
    len: TickLen,
    except: &[ItemId],
) -> Tick {
    let mut at = from;
    loop {
        let end = at + len;
        let blocking = m
            .items
            .index()
            .overlapping(lane, at, end)
            .into_iter()
            .filter(|(id, _, _)| !except.contains(id))
            .map(|(_, _, end)| end)
            .max();
        match blocking {
            Some(end) => at = end,
            None => return at,
        }
    }
}

/// The nearest place to `want` where `len` fits on a lane, not before
/// zero: the first gap after, or the last before, whichever is nearer.
pub(crate) fn nearest_gap(
    m: &Model,
    lane: LaneId,
    want: Tick,
    len: TickLen,
    except: &[ItemId],
) -> Tick {
    let after = gap_from(m, lane, want, len, except);
    // Walk back over what is in the way.
    let mut at = want;
    let mut before = None;
    for _ in 0..10_000 {
        if at < Tick::ZERO {
            break;
        }
        let blocking = m
            .items
            .index()
            .overlapping(lane, at, at + len)
            .into_iter()
            .filter(|(id, _, _)| !except.contains(id))
            .map(|(_, start, _)| start)
            .min();
        match blocking {
            None => {
                before = Some(at);
                break;
            }
            Some(start) => match start.checked_sub(len) {
                Some(next) if next < at => at = next,
                _ => break,
            },
        }
    }
    match before {
        Some(before) if want - before < after - want => before,
        _ => after,
    }
}

impl Ed<'_> {
    /// A new lane in a sequence's stack.
    pub(crate) fn new_lane(
        &mut self,
        sequence: SequenceId,
        at: Stack,
    ) -> Result<LaneId, CommandError> {
        let keys: Vec<(LaneId, OrderKey)> = self
            .d
            .lanes
            .index()
            .of(sequence)
            .map(|(id, key)| (id, key.clone()))
            .collect();
        let order = match at {
            Stack::Top => OrderKey::between(keys.last().map(|(_, k)| k), None),
            Stack::Bottom => OrderKey::between(None, keys.first().map(|(_, k)| k)),
            Stack::Above(lane) => {
                let here = keys.iter().position(|(id, _)| *id == lane);
                match here {
                    Some(here) => {
                        OrderKey::between(Some(&keys[here].1), keys.get(here + 1).map(|(_, k)| k))
                    }
                    None => OrderKey::between(keys.last().map(|(_, k)| k), None),
                }
            }
        };
        let order = order.unwrap_or_else(OrderKey::middle);
        let id: LaneId = self.mint()?;
        self.d.lanes.insert(id, Lane::new(sequence, order))?;
        Ok(id)
    }

    /// The lowest lane free over a span, or a new one on top.
    pub(crate) fn first_free(
        &mut self,
        sequence: SequenceId,
        start: Tick,
        len: TickLen,
    ) -> Result<LaneId, CommandError> {
        let free = lanes(self.d, sequence)
            .into_iter()
            .find(|lane| is_free(self.d, *lane, start, len, &[]));
        match free {
            Some(lane) => Ok(lane),
            None => self.new_lane(sequence, Stack::Top),
        }
    }

    /// The first lane free over a span above the highest one with
    /// something in it there, or a new one on top.
    pub(crate) fn first_free_above(
        &mut self,
        sequence: SequenceId,
        start: Tick,
        len: TickLen,
    ) -> Result<LaneId, CommandError> {
        let all = lanes(self.d, sequence);
        let highest = all
            .iter()
            .rposition(|lane| !is_free(self.d, *lane, start, len, &[]));
        let from = highest.map_or(0, |at| at + 1);
        match all.get(from) {
            Some(lane) => Ok(*lane),
            None => self.new_lane(sequence, Stack::Top),
        }
    }

    /// The first lane above `lane` (it included) free over a span, or a
    /// new one on top.
    pub(crate) fn free_from(
        &mut self,
        lane: LaneId,
        start: Tick,
        len: TickLen,
        except: &[ItemId],
    ) -> Result<LaneId, CommandError> {
        let sequence = self.sequence_of(lane)?;
        let all = lanes(self.d, sequence);
        let from = all.iter().position(|l| *l == lane).unwrap_or(0);
        let free = all[from..]
            .iter()
            .find(|l| is_free(self.d, **l, start, len, except))
            .copied();
        match free {
            Some(lane) => Ok(lane),
            None => self.new_lane(sequence, Stack::Top),
        }
    }

    /// A lane named for a placement: it, if free over the span; a new
    /// lane just above it if not. With none named, the lowest free, or
    /// with `above` the first free above everything there.
    pub(crate) fn lane_for(
        &mut self,
        lane: Option<LaneId>,
        above: bool,
        start: Tick,
        len: TickLen,
    ) -> Result<LaneId, CommandError> {
        match lane {
            Some(lane) => {
                let sequence = self.sequence_of(lane)?;
                if is_free(self.d, lane, start, len, &[]) {
                    Ok(lane)
                } else {
                    self.new_lane(sequence, Stack::Above(lane))
                }
            }
            None => {
                let sequence = self.showing()?;
                if above {
                    self.first_free_above(sequence, start, len)
                } else {
                    self.first_free(sequence, start, len)
                }
            }
        }
    }

    /// Puts a new item in, under a new id.
    pub(crate) fn insert_item(&mut self, item: Item) -> Result<ItemId, CommandError> {
        let id: ItemId = self.mint()?;
        self.d.items.insert(id, item)?;
        Ok(id)
    }

    /// Moves every item on a lane that starts at or after `at`, but those
    /// in `except`, by `by`. A move left stops where the first of them
    /// meets what stays, or zero. With `linked`, and the lane the main
    /// lane of a sequence whose linkage is on, every other lane of the
    /// sequence moves the same way, each as far as it can.
    pub(crate) fn ripple(
        &mut self,
        lane: LaneId,
        at: Tick,
        by: TickDur,
        linked: bool,
        except: &[ItemId],
    ) -> Result<(), CommandError> {
        if by == TickDur::ZERO {
            return Ok(());
        }
        let sequence = self.sequence_of(lane)?;
        let mut moving = vec![lane];
        let all = lanes(self.d, sequence);
        let linkage = self.d.sequences.get(sequence).is_some_and(|s| s.linkage);
        if linked && linkage && all.first() == Some(&lane) {
            moving.extend(all.iter().copied().filter(|l| *l != lane));
        }
        for lane in moving {
            self.shift_lane(lane, at, by, except)?;
        }
        Ok(())
    }

    fn shift_lane(
        &mut self,
        lane: LaneId,
        at: Tick,
        by: TickDur,
        except: &[ItemId],
    ) -> Result<(), CommandError> {
        let on: Vec<(ItemId, Tick, Tick)> = self.d.items.index().on_lane(lane).collect();
        let movers: Vec<(ItemId, Tick)> = on
            .iter()
            .filter(|(id, start, _)| *start >= at && !except.contains(id))
            .map(|(id, start, _)| (*id, *start))
            .collect();
        let Some(first) = movers.iter().map(|(_, s)| *s).min() else {
            return Ok(());
        };
        let mut by = by;
        if by < TickDur::ZERO {
            let movers: BTreeSet<ItemId> = movers.iter().map(|(id, _)| *id).collect();
            let floor = on
                .iter()
                .filter(|(id, start, _)| {
                    !movers.contains(id) && *start < first && !except.contains(id)
                })
                .map(|(_, _, end)| *end)
                .max()
                .unwrap_or(Tick::ZERO)
                .max(Tick::ZERO);
            let room = first - floor;
            if room < TickDur::ZERO {
                return Ok(());
            }
            if -by > room {
                by = -room;
            }
        }
        if by == TickDur::ZERO {
            return Ok(());
        }
        for (id, start) in movers {
            self.update(id, |item| item.start = start + by)?;
        }
        Ok(())
    }

    /// Takes away the transitions of a sequence left over nothing.
    pub(crate) fn sweep_transitions(&mut self, sequence: SequenceId) -> Result<(), CommandError> {
        let all = lanes(self.d, sequence);
        let mut gone = Vec::new();
        for (at, lane) in all.iter().enumerate() {
            for (id, start, end) in self.d.items.index().transitions_on(*lane) {
                let beneath = all[..at].iter().any(|below| {
                    self.d
                        .items
                        .index()
                        .overlapping(*below, start, end)
                        .iter()
                        .any(|(other, _, _)| {
                            self.d.items.get(*other).is_some_and(|i| !i.is_transition())
                        })
                });
                if !beneath {
                    gone.push(id);
                }
            }
        }
        for id in gone {
            self.take_out(id)?;
        }
        Ok(())
    }

    /// Takes an item out, untying what was tied to it.
    pub(crate) fn take_out(&mut self, id: ItemId) -> Result<(), CommandError> {
        if !self.d.items.contains(id) {
            return Ok(());
        }
        let tied: Vec<ItemId> = self.d.items.index().tied_to(id).collect();
        for other in tied {
            self.update(other, |item| item.link = None)?;
        }
        self.d.items.remove(id)?;
        Ok(())
    }

    /// The sequences the items of a list are in.
    pub(crate) fn sequences_of(&self, items: &[ItemId]) -> BTreeSet<SequenceId> {
        items
            .iter()
            .filter_map(|id| self.d.items.get(*id))
            .filter_map(|item| self.d.lanes.get(item.lane).map(|lane| lane.sequence))
            .collect()
    }
}

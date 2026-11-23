// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Tables that keep an index of their rows, and the indexes the model
//! keeps.
//!
//! An index is derived: it holds ids and places that the rows already
//! say, and nothing else. It is part of each snapshot, so a document
//! answers "what is on this lane at this tick" in the time of one
//! lookup, and it shares its nodes between snapshots as the rows do. It
//! is never saved and never compared: two documents are equal by their
//! rows.
//!
//! An [`Indexed`] table is changed only through the edits it has, each
//! of which takes the old row out of the index and puts the new one in,
//! so the index cannot drift from the rows.

use std::fmt;
use std::ops::RangeBounds;
use std::sync::Arc;

use crate::id::{AnyId, MAX_COUNTER, RawId, TypedId, Writer};
use crate::map::OrdMap;
use crate::table::{AnyTable, Table, TableError};
use crate::time::Tick;

use super::ids::{ItemId, LaneId, MediaId, SequenceId, known};
use super::item::Item;
use super::order::OrderKey;
use super::sequence::{Lane, Sequence};

/// What keeps an index of a table's rows up to date.
pub trait Derive<I, T>: Clone + Default {
    /// A row has come in under `id`.
    fn add(&mut self, id: I, row: &T);
    /// The row under `id` has gone.
    fn remove(&mut self, id: I, row: &T);
}

/// A table and an index of its rows.
///
/// Read as a [`Table`] through [`rows`](Indexed::rows); edited as one,
/// with the same strict edits.
pub struct Indexed<I, T, X> {
    rows: Table<I, T>,
    index: X,
}

impl<I: TypedId, T, X: Derive<I, T>> Indexed<I, T, X> {
    /// A table with nothing in it.
    pub fn new() -> Self {
        Indexed {
            rows: Table::new(),
            index: X::default(),
        }
    }

    /// The rows.
    pub fn rows(&self) -> &Table<I, T> {
        &self.rows
    }

    /// The index.
    pub fn index(&self) -> &X {
        &self.index
    }

    /// How many rows there are.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The row under an id.
    pub fn get(&self, id: I) -> Option<&Arc<T>> {
        self.rows.get(id)
    }

    /// Whether there is a row under an id.
    pub fn contains(&self, id: I) -> bool {
        self.rows.contains(id)
    }

    /// Every id and its row, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (I, &Arc<T>)> {
        self.rows.iter()
    }

    /// Every id, in order.
    pub fn ids(&self) -> impl Iterator<Item = I> + '_ {
        self.rows.ids()
    }

    /// The ids in a range and their rows.
    pub fn range(&self, ids: impl RangeBounds<I>) -> impl Iterator<Item = (I, &Arc<T>)> {
        self.rows.range(ids)
    }

    /// Puts a new row in under an id that is not there yet.
    pub fn insert(&mut self, id: I, row: impl Into<Arc<T>>) -> Result<(), TableError> {
        let row = row.into();
        self.rows.insert(id, Arc::clone(&row))?;
        self.index.add(id, &row);
        Ok(())
    }

    /// Puts a new row under an id that is there, and gives back the old.
    pub fn replace(&mut self, id: I, row: impl Into<Arc<T>>) -> Result<Arc<T>, TableError> {
        let row = row.into();
        let old = self.rows.replace(id, Arc::clone(&row))?;
        self.index.remove(id, &old);
        self.index.add(id, &row);
        Ok(old)
    }

    /// Changes the row under an id where it is.
    pub fn update(&mut self, id: I, change: impl FnOnce(&mut T)) -> Result<(), TableError>
    where
        T: Clone,
    {
        let old = Arc::clone(self.rows.get(id).ok_or(TableError::Missing(id.any()))?);
        self.rows.update(id, change)?;
        self.index.remove(id, &old);
        if let Some(new) = self.rows.get(id) {
            self.index.add(id, new);
        }
        Ok(())
    }

    /// Takes out the row under an id, and gives it back.
    pub fn remove(&mut self, id: I) -> Result<Arc<T>, TableError> {
        let old = self.rows.remove(id)?;
        self.index.remove(id, &old);
        Ok(old)
    }
}

impl<I: TypedId, T, X: Derive<I, T>> Default for Indexed<I, T, X> {
    fn default() -> Self {
        Indexed::new()
    }
}

impl<I, T, X: Clone> Clone for Indexed<I, T, X> {
    /// A few pointers copied.
    fn clone(&self) -> Self {
        Indexed {
            rows: self.rows.clone(),
            index: self.index.clone(),
        }
    }
}

impl<I: TypedId, T: PartialEq, X> PartialEq for Indexed<I, T, X> {
    /// Equal by their rows; the index follows from them.
    fn eq(&self, other: &Self) -> bool {
        self.rows == other.rows
    }
}

impl<I: TypedId, T: fmt::Debug, X> fmt::Debug for Indexed<I, T, X> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.rows.fmt(f)
    }
}

impl<I: TypedId, T, X: Derive<I, T>, V: Into<Arc<T>>> FromIterator<(I, V)> for Indexed<I, T, X> {
    fn from_iter<A: IntoIterator<Item = (I, V)>>(rows: A) -> Self {
        let rows: Table<I, T> = rows.into_iter().collect();
        let mut index = X::default();
        for (id, row) in rows.iter() {
            index.add(id, row);
        }
        Indexed { rows, index }
    }
}

impl<I: TypedId, T, X> AnyTable for Indexed<I, T, X> {
    fn tag(&self) -> &'static str {
        I::TAG
    }

    fn holds(&self, raw: RawId) -> bool {
        self.rows.contains(I::from_raw(raw))
    }

    fn count(&self) -> usize {
        self.rows.len()
    }
}

/// The lowest id of a kind there can be.
pub(crate) fn lowest<I: TypedId>() -> I {
    match RawId::from_parts(Writer::LOCAL, 1) {
        Some(raw) => I::from_raw(raw),
        None => unreachable!("one is a count"),
    }
}

/// The highest.
pub(crate) fn highest<I: TypedId>() -> I {
    match RawId::from_parts(Writer(u16::MAX), MAX_COUNTER) {
        Some(raw) => I::from_raw(raw),
        None => unreachable!("the last count is a count"),
    }
}

/// Where an item ends, for an index: past the end of time if its end
/// does not fit, which no item of a document reaches.
fn end_of(item: &Item) -> Tick {
    item.end().unwrap_or(Tick::MAX)
}

/// The items table's index: what is on each lane in time order, and who
/// names each file, sequence, item and nested id.
#[derive(Clone, Default)]
pub struct ItemIndex {
    /// Every item by lane and start, with where it ends.
    by_lane: OrdMap<(LaneId, Tick, ItemId), Tick>,
    /// The transitions alone, the same way.
    transitions: OrdMap<(LaneId, Tick, ItemId), Tick>,
    /// Every file an item names, and the item.
    by_media: OrdMap<(MediaId, ItemId), ()>,
    /// Every sequence an item nests, and the item.
    by_nest: OrdMap<(SequenceId, ItemId), ()>,
    /// Every item tied to, and the item tied to it.
    by_link: OrdMap<(ItemId, ItemId), ()>,
    /// Every effect, mask and marker id, and the item that holds it.
    nested: OrdMap<(AnyId, ItemId), ()>,
}

impl Derive<ItemId, Item> for ItemIndex {
    fn add(&mut self, id: ItemId, item: &Item) {
        self.by_lane
            .insert((item.lane, item.start, id), end_of(item));
        if item.is_transition() {
            self.transitions
                .insert((item.lane, item.start, id), end_of(item));
        }
        for media in item.media() {
            self.by_media.insert((media, id), ());
        }
        if let Some(sequence) = item.nests() {
            self.by_nest.insert((sequence, id), ());
        }
        if let Some(link) = &item.link {
            self.by_link.insert((link.to, id), ());
        }
        for nested in item.nested_ids() {
            self.nested.insert((nested, id), ());
        }
    }

    fn remove(&mut self, id: ItemId, item: &Item) {
        self.by_lane.remove(&(item.lane, item.start, id));
        self.transitions.remove(&(item.lane, item.start, id));
        for media in item.media() {
            self.by_media.remove(&(media, id));
        }
        if let Some(sequence) = item.nests() {
            self.by_nest.remove(&(sequence, id));
        }
        if let Some(link) = &item.link {
            self.by_link.remove(&(link.to, id));
        }
        for nested in item.nested_ids() {
            self.nested.remove(&(nested, id));
        }
    }
}

/// Entries of one lane of a lane-and-start index that overlap
/// `[from, to)`, in time order. On a lane where nothing overlaps, every
/// item before the last that starts before `to` ends before it starts,
/// so the walk back stops at the first that ends by `from`.
fn overlapping_in(
    map: &OrdMap<(LaneId, Tick, ItemId), Tick>,
    lane: LaneId,
    from: Tick,
    to: Tick,
) -> Vec<(ItemId, Tick, Tick)> {
    let mut found = Vec::new();
    let mut probe = (lane, to, lowest::<ItemId>());
    while let Some(((on, start, id), end)) = map.before(&probe) {
        if *on != lane || *end <= from {
            break;
        }
        found.push((*id, *start, *end));
        probe = (lane, *start, *id);
    }
    found.reverse();
    found
}

/// Every entry of one lane, in time order.
fn on_lane_in(
    map: &OrdMap<(LaneId, Tick, ItemId), Tick>,
    lane: LaneId,
) -> impl Iterator<Item = (ItemId, Tick, Tick)> + '_ {
    map.range((lane, Tick::MIN, lowest::<ItemId>())..)
        .take_while(move |((on, _, _), _)| *on == lane)
        .map(|((_, start, id), end)| (*id, *start, *end))
}

impl ItemIndex {
    /// The item on a lane at a tick, if there is one: the one that has
    /// started at or before it and not yet ended.
    pub fn at(&self, lane: LaneId, t: Tick) -> Option<ItemId> {
        // The last item to start at or before `t`.
        let probe = match t.checked_add(crate::time::TickDur::new(1)) {
            Some(next) => (lane, next, lowest::<ItemId>()),
            None => (lane, t, highest::<ItemId>()),
        };
        let ((on, _, id), end) = self.by_lane.before(&probe)?;
        (*on == lane && *end > t).then_some(*id)
    }

    /// The items on a lane that overlap `[from, to)`, in time order, each
    /// with where it starts and ends.
    pub fn overlapping(&self, lane: LaneId, from: Tick, to: Tick) -> Vec<(ItemId, Tick, Tick)> {
        overlapping_in(&self.by_lane, lane, from, to)
    }

    /// Every item on a lane, in time order, each with where it starts and
    /// ends.
    pub fn on_lane(&self, lane: LaneId) -> impl Iterator<Item = (ItemId, Tick, Tick)> + '_ {
        on_lane_in(&self.by_lane, lane)
    }

    /// The item on a lane that starts last before a key of the index:
    /// for the checks, which walk a lane from a place.
    pub(crate) fn before(
        &self,
        lane: LaneId,
        start: Tick,
        id: ItemId,
    ) -> Option<(ItemId, Tick, Tick)> {
        let ((on, start, id), end) = self.by_lane.before(&(lane, start, id))?;
        (*on == lane).then_some((*id, *start, *end))
    }

    /// The items on a lane from a key of the index on, in time order.
    pub(crate) fn from(
        &self,
        lane: LaneId,
        start: Tick,
        id: ItemId,
    ) -> impl Iterator<Item = (ItemId, Tick, Tick)> + '_ {
        self.by_lane
            .range((lane, start, id)..)
            .take_while(move |((on, _, _), _)| *on == lane)
            .map(|((_, start, id), end)| (*id, *start, *end))
    }

    /// The transitions on a lane that overlap `[from, to)`.
    pub fn transitions_overlapping(
        &self,
        lane: LaneId,
        from: Tick,
        to: Tick,
    ) -> Vec<(ItemId, Tick, Tick)> {
        overlapping_in(&self.transitions, lane, from, to)
    }

    /// Every transition on a lane, in time order.
    pub fn transitions_on(&self, lane: LaneId) -> impl Iterator<Item = (ItemId, Tick, Tick)> + '_ {
        on_lane_in(&self.transitions, lane)
    }

    /// Every item that names a file.
    pub fn using_media(&self, media: MediaId) -> impl Iterator<Item = ItemId> + '_ {
        self.by_media
            .range((media, lowest::<ItemId>())..)
            .take_while(move |((of, _), _)| *of == media)
            .map(|((_, id), _)| *id)
    }

    /// Every item that nests a sequence.
    pub fn nesting(&self, sequence: SequenceId) -> impl Iterator<Item = ItemId> + '_ {
        self.by_nest
            .range((sequence, lowest::<ItemId>())..)
            .take_while(move |((of, _), _)| *of == sequence)
            .map(|((_, id), _)| *id)
    }

    /// Every item tied to an item.
    pub fn tied_to(&self, item: ItemId) -> impl Iterator<Item = ItemId> + '_ {
        self.by_link
            .range((item, lowest::<ItemId>())..)
            .take_while(move |((to, _), _)| *to == item)
            .map(|((_, id), _)| *id)
    }

    /// Every item that holds an effect, mask or marker of this id: one,
    /// in a document.
    pub fn holding(&self, nested: AnyId) -> impl Iterator<Item = ItemId> + '_ {
        self.nested
            .range((nested, lowest::<ItemId>())..)
            .take_while(move |((of, _), _)| *of == nested)
            .map(|((_, id), _)| *id)
    }
}

/// The items of a document, with their index.
pub type Items = Indexed<ItemId, Item, ItemIndex>;

/// The lanes table's index: each sequence's lanes in stack order.
#[derive(Clone, Default)]
pub struct LaneIndex {
    by_sequence: OrdMap<(SequenceId, OrderKey, LaneId), ()>,
}

impl Derive<LaneId, Lane> for LaneIndex {
    fn add(&mut self, id: LaneId, lane: &Lane) {
        self.by_sequence
            .insert((lane.sequence, lane.order.clone(), id), ());
    }

    fn remove(&mut self, id: LaneId, lane: &Lane) {
        self.by_sequence
            .remove(&(lane.sequence, lane.order.clone(), id));
    }
}

impl LaneIndex {
    /// A sequence's lanes from the bottom of the stack to the top, each
    /// with its key.
    pub fn of(&self, sequence: SequenceId) -> impl Iterator<Item = (LaneId, &OrderKey)> + '_ {
        self.by_sequence
            .range((sequence, OrderKey::floor(), lowest::<LaneId>())..)
            .take_while(move |((of, _, _), _)| *of == sequence)
            .map(|((_, order, id), _)| (*id, order))
    }

    /// The lanes of a sequence below a place in its stack, top first.
    pub fn below<'a>(
        &'a self,
        sequence: SequenceId,
        order: &'a OrderKey,
    ) -> impl Iterator<Item = LaneId> + 'a {
        let mut lanes: Vec<LaneId> = self
            .of(sequence)
            .take_while(|(_, key)| *key < order)
            .map(|(id, _)| id)
            .collect();
        lanes.reverse();
        lanes.into_iter()
    }

    /// The lanes of a sequence above a place in its stack, bottom first.
    pub fn above<'a>(
        &'a self,
        sequence: SequenceId,
        order: &'a OrderKey,
    ) -> impl Iterator<Item = LaneId> + 'a {
        self.by_sequence
            .range((sequence, order.clone(), highest::<LaneId>())..)
            .take_while(move |((of, _, _), _)| *of == sequence)
            .filter(move |((_, key, _), _)| key > order)
            .map(|((_, _, id), _)| *id)
    }

    /// The lanes of a sequence at a place in its stack: one, in a
    /// document.
    pub fn at<'a>(
        &'a self,
        sequence: SequenceId,
        order: &'a OrderKey,
    ) -> impl Iterator<Item = LaneId> + 'a {
        self.by_sequence
            .range((sequence, order.clone(), lowest::<LaneId>())..)
            .take_while(move |((of, key, _), _)| *of == sequence && key == order)
            .map(|((_, _, id), _)| *id)
    }
}

/// The lanes of a document, with their index.
pub type Lanes = Indexed<LaneId, Lane, LaneIndex>;

/// The sequences table's index: the markers each holds, by id.
#[derive(Clone, Default)]
pub struct SequenceIndex {
    nested: OrdMap<(AnyId, SequenceId), ()>,
}

impl Derive<SequenceId, Sequence> for SequenceIndex {
    fn add(&mut self, id: SequenceId, sequence: &Sequence) {
        for marker in known(&sequence.markers) {
            self.nested.insert((marker.id.any(), id), ());
        }
    }

    fn remove(&mut self, id: SequenceId, sequence: &Sequence) {
        for marker in known(&sequence.markers) {
            self.nested.remove(&(marker.id.any(), id));
        }
    }
}

impl SequenceIndex {
    /// Every sequence that holds a marker of this id: one, in a
    /// document.
    pub fn holding(&self, nested: AnyId) -> impl Iterator<Item = SequenceId> + '_ {
        self.nested
            .range((nested, lowest::<SequenceId>())..)
            .take_while(move |((of, _), _)| *of == nested)
            .map(|((_, id), _)| *id)
    }
}

/// The sequences of a document, with their index.
pub type Sequences = Indexed<SequenceId, Sequence, SequenceIndex>;

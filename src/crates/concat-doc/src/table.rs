// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A table: the things of one kind, by id, and what changed between two
//! of them.
//!
//! A [`Table`] is an [`OrdMap`] from one kind of id to its value behind
//! an `Arc`. Copying a table copies a pointer; a value that an edit did
//! not touch is the same allocation in the table before and the table
//! after, which is what lets everything downstream ask "is this the same
//! clip as last time" by comparing two pointers.
//!
//! [`TableChanges`] is the other half: what was added, removed and
//! replaced between two tables. It is worked out from the two tables,
//! never written down by the code that made the change, so it cannot
//! leave out something a command did on the side.

use std::fmt;
use std::ops::RangeBounds;
use std::sync::Arc;

use crate::id::{AnyId, RawId, TypedId};
use crate::map::{Diff, OrdMap};

/// The things of one kind in a document, each under its id.
///
/// The edits are strict: [`insert`](Table::insert) is for an id that is
/// not there, [`replace`](Table::replace) and [`remove`](Table::remove)
/// for one that is, and each says so when it is not. Putting a new thing
/// over an old one by accident is an id used twice, and silence about it
/// is how that goes unseen.
///
/// Like the map under it, a table is changed in place and its copies are
/// not: a copy is the table as it was.
pub struct Table<I, T> {
    rows: OrdMap<I, Arc<T>>,
}

/// An edit that a table refused.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TableError {
    /// There is already a value under the id.
    Occupied(AnyId),
    /// There is nothing under the id.
    Missing(AnyId),
}

impl TableError {
    /// The id the edit named.
    pub fn id(self) -> AnyId {
        match self {
            TableError::Occupied(id) | TableError::Missing(id) => id,
        }
    }
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TableError::Occupied(id) => write!(f, "{id} is already in the table"),
            TableError::Missing(id) => write!(f, "{id} is not in the table"),
        }
    }
}

impl std::error::Error for TableError {}

impl<I: TypedId, T> Table<I, T> {
    /// A table with nothing in it.
    pub const fn new() -> Self {
        Table {
            rows: OrdMap::new(),
        }
    }

    /// How many things are in it.
    pub const fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether it is empty.
    pub const fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The value under an id.
    pub fn get(&self, id: I) -> Option<&Arc<T>> {
        self.rows.get(&id)
    }

    /// Whether there is a value under an id.
    pub fn contains(&self, id: I) -> bool {
        self.rows.contains_key(&id)
    }

    /// Every id and its value, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (I, &Arc<T>)> {
        self.rows.iter().map(|(id, value)| (*id, value))
    }

    /// Every id, in order.
    pub fn ids(&self) -> impl Iterator<Item = I> + '_ {
        self.rows.keys().copied()
    }

    /// Every value, in id order.
    pub fn values(&self) -> impl Iterator<Item = &Arc<T>> {
        self.rows.values()
    }

    /// The ids in a range and their values, in id order.
    pub fn range(&self, ids: impl RangeBounds<I>) -> impl Iterator<Item = (I, &Arc<T>)> {
        self.rows.range(ids).map(|(id, value)| (*id, value))
    }

    /// Whether the two are one and the same table: copies with no edit to
    /// either since. `true` means equal without looking.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        self.rows.ptr_eq(&other.rows)
    }

    /// Puts a new thing in under an id that is not there yet.
    pub fn insert(&mut self, id: I, value: impl Into<Arc<T>>) -> Result<(), TableError> {
        if self.rows.contains_key(&id) {
            return Err(TableError::Occupied(id.any()));
        }
        self.rows.insert(id, value.into());
        Ok(())
    }

    /// Puts a new value under an id that is there, and gives back the
    /// old one.
    pub fn replace(&mut self, id: I, value: impl Into<Arc<T>>) -> Result<Arc<T>, TableError> {
        match self.rows.get_mut(&id) {
            Some(held) => Ok(std::mem::replace(held, value.into())),
            None => Err(TableError::Missing(id.any())),
        }
    }

    /// Changes the value under an id where it is.
    ///
    /// The value is copied first if anything else still holds it, as the
    /// document the edit started from does; a second change in the same
    /// transaction then finds the copy and changes that.
    pub fn update(&mut self, id: I, change: impl FnOnce(&mut T)) -> Result<(), TableError>
    where
        T: Clone,
    {
        match self.rows.get_mut(&id) {
            Some(held) => {
                change(Arc::make_mut(held));
                Ok(())
            }
            None => Err(TableError::Missing(id.any())),
        }
    }

    /// Takes out what is under an id, and gives it back.
    pub fn remove(&mut self, id: I) -> Result<Arc<T>, TableError> {
        self.rows.remove(&id).ok_or(TableError::Missing(id.any()))
    }
}

impl<I, T> Clone for Table<I, T> {
    /// One pointer copied.
    fn clone(&self) -> Self {
        Table {
            rows: self.rows.clone(),
        }
    }
}

impl<I: TypedId, T> Default for Table<I, T> {
    fn default() -> Self {
        Table::new()
    }
}

impl<I: TypedId, T: fmt::Debug> fmt::Debug for Table<I, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.rows.fmt(f)
    }
}

impl<I: TypedId, T: PartialEq> PartialEq for Table<I, T> {
    fn eq(&self, other: &Self) -> bool {
        self.rows == other.rows
    }
}

impl<I: TypedId, T: Eq> Eq for Table<I, T> {}

impl<I: TypedId, T, V: Into<Arc<T>>> FromIterator<(I, V)> for Table<I, T> {
    /// Builds a table in one pass from things in id order, as a file has
    /// them. Of two things under one id the later is kept; whether a file
    /// may say an id twice is for its reader to decide before this.
    fn from_iter<A: IntoIterator<Item = (I, V)>>(rows: A) -> Self {
        Table {
            rows: rows
                .into_iter()
                .map(|(id, value)| (id, value.into()))
                .collect(),
        }
    }
}

/// A table with its kind forgotten, for what must ask every table of a
/// document the same question.
pub trait AnyTable {
    /// The letter of the kind of id this table is keyed by.
    fn tag(&self) -> &'static str;

    /// Whether the table holds something under this number. The caller
    /// has checked the letter.
    fn holds(&self, raw: RawId) -> bool;

    /// How many things are in the table.
    fn count(&self) -> usize;
}

impl<I: TypedId, T> AnyTable for Table<I, T> {
    fn tag(&self) -> &'static str {
        I::TAG
    }

    fn holds(&self, raw: RawId) -> bool {
        self.contains(I::from_raw(raw))
    }

    fn count(&self) -> usize {
        self.len()
    }
}

/// What is different between an older table and a newer one.
///
/// Each list is in id order. A value that was replaced by one equal to
/// it is not a change and is not here: the lists are what a comparison of
/// the two tables entry by entry would find, at the cost of only the
/// parts the two do not share.
pub struct TableChanges<I, T> {
    /// Ids the newer table has and the older did not, with their values.
    pub added: Vec<(I, Arc<T>)>,
    /// Ids the older table had and the newer does not, with the values
    /// they had.
    pub removed: Vec<(I, Arc<T>)>,
    /// Ids in both whose value is different.
    pub replaced: Vec<Replaced<I, T>>,
}

/// An id whose value changed, with the value before and the value after.
pub struct Replaced<I, T> {
    /// Whose value it is.
    pub id: I,
    /// The value in the older table.
    pub old: Arc<T>,
    /// The value in the newer.
    pub new: Arc<T>,
}

impl<I: TypedId, T: PartialEq> TableChanges<I, T> {
    /// What changed from `old` to `new`.
    pub fn between(old: &Table<I, T>, new: &Table<I, T>) -> Self {
        let mut changes = TableChanges::default();
        for diff in OrdMap::diff(&old.rows, &new.rows, Arc::ptr_eq) {
            match diff {
                Diff::Added(id, value) => changes.added.push((*id, Arc::clone(value))),
                Diff::Removed(id, value) => changes.removed.push((*id, Arc::clone(value))),
                // Two allocations and one value: written again as it was.
                Diff::Changed(_, old, new) if old == new => {}
                Diff::Changed(id, old, new) => changes.replaced.push(Replaced {
                    id: *id,
                    old: Arc::clone(old),
                    new: Arc::clone(new),
                }),
            }
        }
        changes
    }
}

impl<I: TypedId, T> TableChanges<I, T> {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.replaced.is_empty()
    }

    /// How many ids were added, removed or replaced.
    pub fn len(&self) -> usize {
        self.added.len() + self.removed.len() + self.replaced.len()
    }

    /// Every id that was added, removed or replaced: the added in order,
    /// then the removed, then the replaced.
    pub fn ids(&self) -> impl Iterator<Item = I> + '_ {
        let added = self.added.iter().map(|(id, _)| *id);
        let removed = self.removed.iter().map(|(id, _)| *id);
        added
            .chain(removed)
            .chain(self.replaced.iter().map(|replaced| replaced.id))
    }

    /// Every value that the newer table no longer holds: those removed
    /// and the old ones of those replaced. What a step back would bring
    /// back, and so what keeping the step keeps alive.
    pub fn gone(&self) -> impl Iterator<Item = &Arc<T>> {
        let removed = self.removed.iter().map(|(_, value)| value);
        removed.chain(self.replaced.iter().map(|replaced| &replaced.old))
    }
}

impl<I, T> Default for TableChanges<I, T> {
    fn default() -> Self {
        TableChanges {
            added: Vec::new(),
            removed: Vec::new(),
            replaced: Vec::new(),
        }
    }
}

impl<I: Clone, T> Clone for TableChanges<I, T> {
    fn clone(&self) -> Self {
        TableChanges {
            added: self.added.clone(),
            removed: self.removed.clone(),
            replaced: self.replaced.clone(),
        }
    }
}

impl<I: Clone, T> Clone for Replaced<I, T> {
    fn clone(&self) -> Self {
        Replaced {
            id: self.id.clone(),
            old: Arc::clone(&self.old),
            new: Arc::clone(&self.new),
        }
    }
}

impl<I: fmt::Debug, T: fmt::Debug> fmt::Debug for TableChanges<I, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableChanges")
            .field("added", &self.added)
            .field("removed", &self.removed)
            .field("replaced", &self.replaced)
            .finish()
    }
}

impl<I: fmt::Debug, T: fmt::Debug> fmt::Debug for Replaced<I, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {:?} -> {:?}", self.id, self.old, self.new)
    }
}

impl<I: PartialEq, T: PartialEq> PartialEq for TableChanges<I, T> {
    fn eq(&self, other: &Self) -> bool {
        self.added == other.added
            && self.removed == other.removed
            && self.replaced == other.replaced
    }
}

impl<I: PartialEq, T: PartialEq> PartialEq for Replaced<I, T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.old == other.old && self.new == other.new
    }
}

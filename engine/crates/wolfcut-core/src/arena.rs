// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Generational arenas.
//!
//! Editor data is a graph - clips reference media, effects reference clips,
//! nodes reference nodes - and WolfCut models that with handles into an arena
//! rather than with pointers. See
//! `docs/decisions/0003-arena-handles-not-pointers.md` for why.
//!
//! The short version: [`Id`] is `Copy`, `Eq` and `Hash`, so it goes into undo
//! records and across threads without ceremony, and the generation counter
//! turns a stale handle into `None` instead of a use-after-free.
//!
//! ```
//! use wolfcut_core::arena::Arena;
//!
//! let mut arena = Arena::new();
//! let id = arena.insert("clip");
//! assert_eq!(arena.get(id), Some(&"clip"));
//!
//! arena.remove(id);
//! assert_eq!(arena.get(id), None); // the stale handle is simply empty
//! ```

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

/// A handle to a value in an [`Arena`].
///
/// The type parameter is a marker only - it stops you from indexing a track
/// arena with a clip handle - so `Id` is `Copy` regardless of whether `T` is.
pub struct Id<T> {
    index: u32,
    generation: u32,
    // `fn() -> T` rather than `T` so the handle stays Send + Sync + Copy
    // whatever T happens to be.
    marker: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    fn new(index: u32, generation: u32) -> Self {
        Self { index, generation, marker: PhantomData }
    }

    /// The slot this handle points at. Useful for stable display ordering; it
    /// is not a unique identity on its own, because slots are reused.
    pub const fn index(self) -> u32 {
        self.index
    }
}

impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Id<T> {}

impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Id<T> {}

impl<T> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = std::any::type_name::<T>().rsplit("::").next().unwrap_or("?");
        write!(f, "{name}#{}v{}", self.index, self.generation)
    }
}

enum Slot<T> {
    Occupied { value: T, generation: u32 },
    Vacant { generation: u32 },
}

/// A slab of `T` addressed by [`Id`], where removed slots are reused without
/// letting old handles alias the new occupant.
pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    len: usize,
}

impl<T> Arena<T> {
    /// An empty arena.
    pub fn new() -> Self {
        Self { slots: Vec::new(), free: Vec::new(), len: 0 }
    }

    /// An empty arena with room for `capacity` values.
    pub fn with_capacity(capacity: usize) -> Self {
        Self { slots: Vec::with_capacity(capacity), free: Vec::new(), len: 0 }
    }

    /// Stores `value` and returns a handle to it.
    ///
    /// # Panics
    /// If the arena would exceed `u32::MAX` slots.
    pub fn insert(&mut self, value: T) -> Id<T> {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            let generation = match slot {
                Slot::Vacant { generation } => *generation,
                Slot::Occupied { .. } => unreachable!("free list pointed at an occupied slot"),
            };
            *slot = Slot::Occupied { value, generation };
            self.len += 1;
            Id::new(index, generation)
        } else {
            let index = u32::try_from(self.slots.len()).expect("arena exceeded u32::MAX slots");
            self.slots.push(Slot::Occupied { value, generation: 0 });
            self.len += 1;
            Id::new(index, 0)
        }
    }

    /// Borrows the value behind `id`, or `None` if it has been removed.
    pub fn get(&self, id: Id<T>) -> Option<&T> {
        match self.slots.get(id.index as usize)? {
            Slot::Occupied { value, generation } if *generation == id.generation => Some(value),
            _ => None,
        }
    }

    /// Mutably borrows the value behind `id`, or `None` if it has been removed.
    pub fn get_mut(&mut self, id: Id<T>) -> Option<&mut T> {
        match self.slots.get_mut(id.index as usize)? {
            Slot::Occupied { value, generation } if *generation == id.generation => Some(value),
            _ => None,
        }
    }

    /// True if `id` still refers to a live value.
    pub fn contains(&self, id: Id<T>) -> bool {
        self.get(id).is_some()
    }

    /// Removes and returns the value behind `id`, invalidating every handle to
    /// it. Returns `None` if it was already gone.
    pub fn remove(&mut self, id: Id<T>) -> Option<T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        match slot {
            Slot::Occupied { generation, .. } if *generation == id.generation => {}
            _ => return None,
        }
        let next_generation = id.generation.wrapping_add(1);
        let previous = std::mem::replace(slot, Slot::Vacant { generation: next_generation });
        self.free.push(id.index);
        self.len -= 1;
        match previous {
            Slot::Occupied { value, .. } => Some(value),
            Slot::Vacant { .. } => unreachable!("just checked this slot was occupied"),
        }
    }

    /// How many live values the arena holds.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// True if the arena holds no live values.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Iterates over live values in slot order, paired with their handles.
    pub fn iter(&self) -> impl Iterator<Item = (Id<T>, &T)> {
        self.slots.iter().enumerate().filter_map(|(index, slot)| match slot {
            Slot::Occupied { value, generation } => {
                Some((Id::new(index as u32, *generation), value))
            }
            Slot::Vacant { .. } => None,
        })
    }

    /// Mutably iterates over live values in slot order, paired with handles.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Id<T>, &mut T)> {
        self.slots.iter_mut().enumerate().filter_map(|(index, slot)| match slot {
            Slot::Occupied { value, generation } => {
                Some((Id::new(index as u32, *generation), value))
            }
            Slot::Vacant { .. } => None,
        })
    }
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: fmt::Debug> fmt::Debug for Arena<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_returns_values() {
        let mut arena = Arena::new();
        let a = arena.insert(1);
        let b = arena.insert(2);
        assert_eq!(arena.get(a), Some(&1));
        assert_eq!(arena.get(b), Some(&2));
        assert_eq!(arena.len(), 2);
    }

    #[test]
    fn a_stale_handle_reads_as_empty_not_as_the_new_occupant() {
        // This is the entire point of the generation counter.
        let mut arena = Arena::new();
        let old = arena.insert("first");
        arena.remove(old);
        let new = arena.insert("second");

        assert_eq!(old.index(), new.index(), "the slot should have been reused");
        assert_ne!(old, new);
        assert_eq!(arena.get(old), None);
        assert_eq!(arena.get(new), Some(&"second"));
    }

    #[test]
    fn removing_twice_is_harmless() {
        let mut arena = Arena::new();
        let id = arena.insert(7);
        assert_eq!(arena.remove(id), Some(7));
        assert_eq!(arena.remove(id), None);
        assert!(arena.is_empty());
    }

    #[test]
    fn iteration_skips_holes() {
        let mut arena = Arena::new();
        let a = arena.insert("a");
        let b = arena.insert("b");
        let c = arena.insert("c");
        arena.remove(b);

        let seen: Vec<_> = arena.iter().map(|(id, value)| (id, *value)).collect();
        assert_eq!(seen, vec![(a, "a"), (c, "c")]);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A sorted map whose copies share everything they have in common.
//!
//! Undo by snapshot keeps the document as it was before every step. Kept
//! as whole copies, a thousand steps of a hundred thousand items are a
//! hundred million pointers; kept as an [`OrdMap`], a step costs the few
//! nodes on the way down to what it changed, and the rest is the same
//! memory as the step before. The same sharing is what makes the
//! difference between two snapshots cheap to find: [`OrdMap::diff`] walks
//! the two together and steps over every part they share without looking
//! inside it.
//!
//! # What it is
//!
//! A B+ tree of reference-counted nodes. A leaf is a sorted run of up to
//! `B` entries; a branch is a sorted run of up to `B` children, each with
//! the lowest key that may be under it. Copying the map copies one
//! pointer. Changing a copy rebuilds the nodes from the root down to the
//! entry, and only those; a node that no other copy holds is changed in
//! place, so a run of edits to one copy pays for the path once.
//!
//! All of it is safe code.
//!
//! # Measured
//!
//! The design left the choice of this map to measurement, between
//! `rpds` 1.2.1 (its `RedBlackTreeMapSync`, MIT) and a tree of our own.
//! Measured on 2026-11-20 on an Apple M5 (10 cores, 16 GB), macOS, rustc
//! 1.93.1, release profile, by
//! `cargo run --release -p concat-doc --example map_bench`. Keys are
//! `u64` and values an `Arc` of 64 bytes. The benchmark takes the median
//! of nine runs; it was itself run three times, and each figure here is
//! the middle of its three, since the machine was doing other work at
//! the time (a load of about 3).
//! After a time, in brackets, is the memory still held afterwards that
//! was not held before; for an edit that includes the 80 bytes of the new
//! value.
//!
//! | Work | 1,000: own | 1,000: `rpds` | 10,000: own | 10,000: `rpds` | 100,000: own | 100,000: `rpds` |
//! |---|---|---|---|---|---|---|
//! | Build from sorted entries | 1.7 us (20 KB) | 119 us (78 KB) | 14 us (198 KB) | 1.7 ms (781 KB) | 135 us (1.98 MB) | 24.7 ms (7.81 MB) |
//! | Read every entry in order | 0.6 us | 3.0 us | 7.3 us | 35 us | 84 us | 348 us |
//! | Look one up | 12 ns | 8 ns | 18 ns | 20 ns | 25 ns | 79 ns |
//! | One insert, a new highest key | 141 ns (816 B) | 243 ns (1000 B) | 199 ns (1152 B) | 327 ns (1168 B) | 245 ns (1424 B) | 404 ns (1392 B) |
//! | One insert, a new key anywhere | 197 ns (1140 B) | 237 ns (742 B) | 245 ns (1402 B) | 328 ns (915 B) | 320 ns (1885 B) | 418 ns (1095 B) |
//! | One replace | 151 ns (842 B) | 167 ns (628 B) | 197 ns (1104 B) | 230 ns (802 B) | 246 ns (1395 B) | 310 ns (983 B) |
//! | One remove | 185 ns (761 B) | 292 ns (1026 B) | 250 ns (1023 B) | 408 ns (1349 B) | 329 ns (1314 B) | 580 ns (1743 B) |
//! | 1,000 edits in a row, every map kept: each | 164 ns (776 B) | 195 ns (641 B) | 215 ns (1065 B) | 255 ns (834 B) | 273 ns (1376 B) | 312 ns (1040 B) |
//! | A drag of 1 entry: each of 600 recomputations | 205 ns | 219 ns | 278 ns | 343 ns | 352 ns | 384 ns |
//! | A drag of 50 entries: each of 600 recomputations | 4.7 us | 6.5 us | 8.0 us | 10.7 us | 11.6 us | 16.7 us |
//! | What differs after 1 edit | 136 ns | 6.9 us | 157 ns | 70 us | 259 ns | 726 us |
//! | What differs after 50 edits | 1.9 us | 6.8 us | 3.7 us | 69 us | 5.5 us | 735 us |
//!
//! The thousand edits are three replaces in four, the rest inserts and
//! removes; kept as an undo stack they add 0.76, 1.04 and 1.34 MB with
//! this tree and 0.63, 0.81 and 1.02 MB with `rpds`.
//!
//! **The choice is the tree of our own, and `rpds` is not a dependency.**
//! What decides it is finding what differs. `rpds` has no way to step
//! over what two maps share, so the two must be read side by side from
//! end to end: 0.7 ms at a hundred thousand entries, on every commit,
//! where this tree takes a quarter of a microsecond. That one function
//! feeds the commit's checks, the window, the engine and the save. This
//! tree is also quicker at every edit (by 1.1 to 1.8 times), reads in
//! order four times faster, builds from a file's entries 70 to 180 times
//! faster and holds the whole map in a quarter of the memory.
//!
//! What `rpds` does better: a step of undo costs it about a quarter less
//! memory (1.0 KB against 1.4 KB at a hundred thousand entries), since it
//! copies one small node per level where this copies a run of sixteen;
//! and at a thousand entries its lookup is 4 ns quicker. Neither is what
//! the document is short of: a thousand steps of undo are 1.3 MB.
//!
//! The node size was measured the same way, at a hundred thousand
//! entries:
//!
//! | Work | 8 a node | 16 a node | 32 a node | 64 a node |
//! |---|---|---|---|---|
//! | Build from sorted entries | 263 us (2.46 MB) | 135 us (1.98 MB) | 90 us (1.76 MB) | 70 us (1.66 MB) |
//! | Read every entry in order | 93 us | 84 us | 81 us | 80 us |
//! | One insert, a new key anywhere | 361 ns (1598 B) | 320 ns (1885 B) | 292 ns (2400 B) | 301 ns (3252 B) |
//! | One replace | 268 ns (1138 B) | 246 ns (1395 B) | 235 ns (1814 B) | 259 ns (2696 B) |
//! | One remove | 357 ns (1057 B) | 329 ns (1314 B) | 310 ns (1734 B) | 330 ns (2616 B) |
//! | 1,000 edits in a row, every map kept: each | 328 ns (1212 B) | 273 ns (1376 B) | 257 ns (1793 B) | 279 ns (2725 B) |
//! | A drag of 50 entries: each recomputation | 13.2 us | 11.6 us | 12.2 us | 14.3 us |
//! | What differs after 1 edit | 235 ns | 259 ns | 274 ns | 425 ns |
//! | What differs after 50 edits | 4.1 us | 5.5 us | 8.9 us | 12.7 us |
//!
//! Sixteen is the default. Thirty-two edits a few nanoseconds faster,
//! and pays for it with a third more memory for every step of undo and
//! half as long again to find what differs after a drag. Eight saves an
//! eighth of the memory of a step and finds what differs a little
//! sooner, is slower to edit and to build, and its whole tree is a
//! quarter larger.

use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt;
use std::ops::{Bound, RangeBounds};
use std::sync::Arc;

/// A sorted map that is cheap to copy and shares what its copies have in
/// common.
///
/// It is used as a `BTreeMap` is: [`insert`](OrdMap::insert),
/// [`remove`](OrdMap::remove) and [`get_mut`](OrdMap::get_mut) change
/// this map. They never change a copy of it: a copy taken before an edit
/// is the map as it was, for as long as it is kept.
///
/// ```
/// use concat_doc::OrdMap;
///
/// let mut now: OrdMap<u32, &str> = [(1, "one"), (2, "two")].into_iter().collect();
/// let before = now.clone();
/// now.insert(3, "three");
/// now.remove(&1);
///
/// assert_eq!(before.len(), 2);
/// assert_eq!(now.keys().copied().collect::<Vec<_>>(), [2, 3]);
/// ```
///
/// `B` is how many entries a node holds at most, and is at least 4. The
/// default is the measured choice; see [the module](self).
pub struct OrdMap<K, V, const B: usize = 16> {
    root: Option<Arc<Node<K, V>>>,
    len: usize,
}

/// A child of a branch, with the lowest key that may be under it.
type Kid<K, V> = (K, Arc<Node<K, V>>);

/// A node: a run of entries, or a run of children.
///
/// In a branch each child is kept with a key that is at or below every
/// key under it and above every key under the child before it. It is the
/// lowest key the child has held since it was last rebuilt, so it may be
/// lower than the child's lowest key now, and that is all that is asked
/// of it: it sends a search the right way.
enum Node<K, V> {
    Leaf(Vec<(K, V)>),
    Branch(Vec<Kid<K, V>>),
}

impl<K: Clone, V: Clone> Clone for Node<K, V> {
    /// A copy with room for one more entry: a node is copied in order to
    /// be changed, and most often by one insert.
    fn clone(&self) -> Node<K, V> {
        fn roomy<T: Clone>(entries: &[T]) -> Vec<T> {
            let mut copy = Vec::with_capacity(entries.len() + 1);
            copy.extend_from_slice(entries);
            copy
        }
        match self {
            Node::Leaf(entries) => Node::Leaf(roomy(entries)),
            Node::Branch(kids) => Node::Branch(roomy(kids)),
        }
    }
}

impl<K, V> Node<K, V> {
    fn len(&self) -> usize {
        match self {
            Node::Leaf(entries) => entries.len(),
            Node::Branch(kids) => kids.len(),
        }
    }

    /// A key at or below every key under this node: a leaf's first key,
    /// a branch's first low key. `None` for a node with nothing in it.
    fn low(&self) -> Option<&K> {
        match self {
            Node::Leaf(entries) => entries.first().map(|(key, _)| key),
            Node::Branch(kids) => kids.first().map(|(low, _)| low),
        }
    }
}

/// What an insert did to the node it was sent into.
enum Inserted<K, V> {
    /// The key was there; this is the value it had.
    Replaced(V),
    /// The key is new and the node had room.
    Added,
    /// The key is new and the node was full: this is the half that was
    /// split off to its right, with its lowest key.
    Split(K, Arc<Node<K, V>>),
}

/// Which child a key belongs under: the last whose low key is at or
/// below it, and the first if there is none.
fn child_index<K, Q, V>(kids: &[Kid<K, V>], key: &Q) -> usize
where
    K: Borrow<Q>,
    Q: Ord + ?Sized,
{
    kids.partition_point(|(low, _)| low.borrow() <= key)
        .saturating_sub(1)
}

/// Splits a run that has grown past `max`, given where the new entry
/// went. `None` if it still fits.
///
/// A run is cut in half, except when the new entry is its last: then the
/// run is left full and the new entry starts the next one. Ids are made
/// in rising order, so most inserts are at the far end, and cutting in
/// half there would leave every node behind it half empty for good.
fn split<T>(entries: &mut Vec<T>, added_at: usize, max: usize) -> Option<Vec<T>> {
    if entries.len() <= max {
        return None;
    }
    let cut = if added_at + 1 == entries.len() {
        added_at
    } else {
        entries.len() / 2
    };
    Some(entries.split_off(cut))
}

fn insert_into<K, V, const B: usize>(node: &mut Arc<Node<K, V>>, key: K, value: V) -> Inserted<K, V>
where
    K: Ord + Clone,
    V: Clone,
{
    match Arc::make_mut(node) {
        Node::Leaf(entries) => match entries.binary_search_by(|(have, _)| have.cmp(&key)) {
            Ok(at) => Inserted::Replaced(std::mem::replace(&mut entries[at].1, value)),
            Err(at) => {
                entries.insert(at, (key, value));
                match split(entries, at, B) {
                    None => Inserted::Added,
                    Some(right) => {
                        let low = right[0].0.clone();
                        Inserted::Split(low, Arc::new(Node::Leaf(right)))
                    }
                }
            }
        },
        Node::Branch(kids) => {
            let at = child_index(kids, &key);
            if at == 0 && key < kids[0].0 {
                // Below everything here: the first child's low key goes
                // down with it, so that it stays at or below all under it.
                kids[0].0 = key.clone();
            }
            match insert_into::<K, V, B>(&mut kids[at].1, key, value) {
                Inserted::Split(low, right) => {
                    kids.insert(at + 1, (low, right));
                    match split(kids, at + 1, B) {
                        None => Inserted::Added,
                        Some(right) => {
                            let low = right[0].0.clone();
                            Inserted::Split(low, Arc::new(Node::Branch(right)))
                        }
                    }
                }
                done => done,
            }
        }
    }
}

/// Takes a key out from under a node. The key is known to be there.
fn remove_from<K, Q, V, const B: usize>(node: &mut Arc<Node<K, V>>, key: &Q) -> Option<V>
where
    K: Ord + Clone + Borrow<Q>,
    Q: Ord + ?Sized,
    V: Clone,
{
    match Arc::make_mut(node) {
        Node::Leaf(entries) => {
            let at = entries
                .binary_search_by(|(have, _)| have.borrow().cmp(key))
                .ok()?;
            Some(entries.remove(at).1)
        }
        Node::Branch(kids) => {
            let at = child_index(kids, key);
            let removed = remove_from::<K, Q, V, B>(&mut kids[at].1, key)?;
            refill::<K, V, B>(kids, at);
            Some(removed)
        }
    }
}

/// After a removal under `kids[at]`: if that child has grown thin, joins
/// it to a neighbour, or evens the two out if they would not fit in one
/// node.
///
/// Thin is under a quarter full. Joining at a half, as the textbook tree
/// does, makes a full node out of two halves, which the next insert
/// splits and the next removal joins again.
fn refill<K: Clone, V: Clone, const B: usize>(kids: &mut Vec<Kid<K, V>>, at: usize) {
    let thin = (B / 4).max(1);
    let have = kids[at].1.len();
    if have >= thin {
        return;
    }
    if kids.len() == 1 {
        if have == 0 {
            // Nothing left under the only child: the branch is empty too,
            // and the level above takes it out in the same way.
            kids.clear();
        }
        return;
    }
    let left = if at > 0 { at - 1 } else { at };
    if kids[left].1.len() + kids[left + 1].1.len() <= B {
        let (_, right) = kids.remove(left + 1);
        let right = Arc::try_unwrap(right).unwrap_or_else(|shared| (*shared).clone());
        match (Arc::make_mut(&mut kids[left].1), right) {
            (Node::Leaf(into), Node::Leaf(mut from)) => into.append(&mut from),
            (Node::Branch(into), Node::Branch(mut from)) => into.append(&mut from),
            // The children of one branch are all at one height: all
            // leaves or all branches. `OrdMap::check` holds every test
            // to it.
            _ => unreachable!("two children of one branch are the same kind of node"),
        }
        return;
    }
    let (head, tail) = kids.split_at_mut(left + 1);
    let low = match (
        Arc::make_mut(&mut head[left].1),
        Arc::make_mut(&mut tail[0].1),
    ) {
        (Node::Leaf(a), Node::Leaf(b)) => even_out(a, b),
        (Node::Branch(a), Node::Branch(b)) => even_out(a, b),
        _ => unreachable!("two children of one branch are the same kind of node"),
    };
    tail[0].0 = low;
}

/// Moves entries across the join of two runs until each has half, and
/// says what the right one's lowest key now is. Between them they hold
/// more than one node's worth, so neither is left empty.
fn even_out<K: Clone, T>(left: &mut Vec<(K, T)>, right: &mut Vec<(K, T)>) -> K {
    let half = (left.len() + right.len()) / 2;
    if left.len() < half {
        let take = half - left.len();
        left.extend(right.drain(..take));
    } else {
        let moved = left.split_off(half);
        right.splice(0..0, moved);
    }
    right[0].0.clone()
}

impl<K, V, const B: usize> OrdMap<K, V, B> {
    /// Refuses, when the crate using it is built, a node too small to be
    /// a tree.
    const NODE_HOLDS_ENOUGH: () = assert!(B >= 4, "a node holds at least four entries");

    /// A map with nothing in it.
    pub const fn new() -> Self {
        let () = Self::NODE_HOLDS_ENOUGH;
        OrdMap { root: None, len: 0 }
    }

    /// How many entries there are.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether there are none.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the two are one and the same tree: copies of each other
    /// with no change to either since. `true` means equal without
    /// looking; `false` does not mean different.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (&self.root, &other.root) {
            (Some(mine), Some(theirs)) => Arc::ptr_eq(mine, theirs),
            (None, None) => true,
            _ => false,
        }
    }

    /// The entry with the lowest key.
    pub fn first(&self) -> Option<(&K, &V)> {
        let mut node = self.root.as_deref()?;
        loop {
            match node {
                Node::Leaf(entries) => return entries.first().map(|(key, value)| (key, value)),
                Node::Branch(kids) => node = &kids.first()?.1,
            }
        }
    }

    /// The entry with the highest key.
    pub fn last(&self) -> Option<(&K, &V)> {
        let mut node = self.root.as_deref()?;
        loop {
            match node {
                Node::Leaf(entries) => return entries.last().map(|(key, value)| (key, value)),
                Node::Branch(kids) => node = &kids.last()?.1,
            }
        }
    }

    /// How many branches are above the leaves.
    fn height(&self) -> u32 {
        let mut height = 0;
        let mut node = self.root.as_deref();
        while let Some(Node::Branch(kids)) = node {
            height += 1;
            node = kids.first().map(|(_, kid)| &**kid);
        }
        height
    }
}

impl<K: Ord, V, const B: usize> OrdMap<K, V, B> {
    /// Every entry, in key order.
    pub fn iter(&self) -> Iter<'_, K, V> {
        let mut iter = Iter {
            above: Vec::new(),
            leaf: [].iter(),
            end: Bound::Unbounded,
        };
        if let Some(root) = &self.root {
            iter.descend(root);
        }
        iter
    }

    /// Every key, in order.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|(key, _)| key)
    }

    /// Every value, in key order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, value)| value)
    }

    /// The value under a key.
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let mut node = self.root.as_deref()?;
        loop {
            match node {
                Node::Leaf(entries) => {
                    let at = entries
                        .binary_search_by(|(have, _)| have.borrow().cmp(key))
                        .ok()?;
                    return Some(&entries[at].1);
                }
                Node::Branch(kids) => node = &kids[child_index(kids, key)].1,
            }
        }
    }

    /// Whether there is a value under a key.
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.get(key).is_some()
    }

    /// The entries whose keys are in a range, in key order.
    ///
    /// ```
    /// use concat_doc::OrdMap;
    ///
    /// let squares: OrdMap<u32, u32> = (1..=9).map(|n| (n, n * n)).collect();
    /// let some: Vec<u32> = squares.range(3..6).map(|(_, square)| *square).collect();
    /// assert_eq!(some, [9, 16, 25]);
    /// ```
    pub fn range<R>(&self, range: R) -> Iter<'_, K, V>
    where
        K: Clone,
        R: RangeBounds<K>,
    {
        let mut iter = Iter {
            above: Vec::new(),
            leaf: [].iter(),
            end: range.end_bound().cloned(),
        };
        let Some(mut node) = self.root.as_ref() else {
            return iter;
        };
        let start = range.start_bound();
        loop {
            match &**node {
                Node::Leaf(entries) => {
                    let from = match start {
                        Bound::Unbounded => 0,
                        Bound::Included(key) => entries.partition_point(|(have, _)| have < key),
                        Bound::Excluded(key) => entries.partition_point(|(have, _)| have <= key),
                    };
                    iter.leaf = entries[from..].iter();
                    return iter;
                }
                Node::Branch(kids) => {
                    let at = match start {
                        Bound::Unbounded => 0,
                        Bound::Included(key) | Bound::Excluded(key) => child_index(kids, key),
                    };
                    iter.above.push(kids[at + 1..].iter());
                    node = &kids[at].1;
                }
            }
        }
    }
}

impl<K: Ord + Clone, V: Clone, const B: usize> OrdMap<K, V, B> {
    /// Puts a value under a key, and gives back the value that was there.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let Some(root) = &mut self.root else {
            self.root = Some(Arc::new(Node::Leaf(vec![(key, value)])));
            self.len = 1;
            return None;
        };
        match insert_into::<K, V, B>(root, key, value) {
            Inserted::Replaced(old) => Some(old),
            Inserted::Added => {
                self.len += 1;
                None
            }
            Inserted::Split(low, right) => {
                self.len += 1;
                // What is left of a node that split has entries in it;
                // were it ever not so, the key that split it is still
                // above none of them.
                let left_low = root.low().unwrap_or(&low).clone();
                let left = (left_low, Arc::clone(root));
                *root = Arc::new(Node::Branch(vec![left, (low, right)]));
                None
            }
        }
    }

    /// Takes a key out, and gives back the value that was under it.
    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        // Looked up first, so that taking out what is not there rebuilds
        // nothing and leaves this map sharing all it shared.
        if !self.contains_key(key) {
            return None;
        }
        let removed = remove_from::<K, Q, V, B>(self.root.as_mut()?, key);
        self.len -= 1;
        // A root with nothing in it, or with one child, is not needed.
        loop {
            let lower = match self.root.as_deref() {
                Some(Node::Leaf(entries)) if entries.is_empty() => None,
                Some(Node::Branch(kids)) if kids.is_empty() => None,
                Some(Node::Branch(kids)) if kids.len() == 1 => Some(Arc::clone(&kids[0].1)),
                _ => break,
            };
            self.root = lower;
        }
        removed
    }

    /// The value under a key, to change in place.
    ///
    /// In place for this map only: the nodes on the way down are made
    /// this map's own first, so no copy of it sees the change.
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        if !self.contains_key(key) {
            return None;
        }
        let mut node = self.root.as_mut()?;
        loop {
            match Arc::make_mut(node) {
                Node::Leaf(entries) => {
                    let at = entries
                        .binary_search_by(|(have, _)| have.borrow().cmp(key))
                        .ok()?;
                    return Some(&mut entries[at].1);
                }
                Node::Branch(kids) => {
                    let at = child_index(kids, key);
                    node = &mut kids[at].1;
                }
            }
        }
    }

    /// Builds the tree from entries already in rising key order with no
    /// key twice, in one pass and with every node as full as it can be.
    fn from_sorted(sorted: Vec<(K, V)>) -> Self {
        /// Cuts a run into nodes of at most `max`, all within one of the
        /// same size, so that the last is not a stub.
        fn pack<T, N>(run: Vec<T>, max: usize, node: impl Fn(Vec<T>) -> N) -> Vec<N> {
            let nodes = run.len().div_ceil(max);
            let (each, spare) = (run.len() / nodes, run.len() % nodes);
            let mut run = run.into_iter();
            (0..nodes)
                .map(|nth| node(run.by_ref().take(each + usize::from(nth < spare)).collect()))
                .collect()
        }

        let () = Self::NODE_HOLDS_ENOUGH;
        let len = sorted.len();
        if len == 0 {
            return OrdMap::new();
        }
        let mut level = pack(sorted, B, |entries| {
            (entries[0].0.clone(), Arc::new(Node::Leaf(entries)))
        });
        while level.len() > 1 {
            level = pack(level, B, |kids| {
                (kids[0].0.clone(), Arc::new(Node::Branch(kids)))
            });
        }
        OrdMap {
            root: level.pop().map(|(_, root)| root),
            len,
        }
    }
}

impl<K: Ord, V, const B: usize> OrdMap<K, V, B> {
    /// What differs between two maps: every key that is in one and not
    /// the other, and every key in both whose two values `same` says are
    /// not the same. In key order.
    ///
    /// The two are walked together, and wherever both hold the very same
    /// node, the walk steps over it: nothing under a shared node is
    /// looked at, and `same` is never asked about it. Between a map and a
    /// copy of it that has had a few edits, the cost is those few paths
    /// and not the size of the map.
    ///
    /// `same` is for values that are equal without being one value:
    /// between two `Arc`s it is `Arc::ptr_eq`, which is how
    /// [`TableChanges`](crate::TableChanges) asks.
    ///
    /// ```
    /// use concat_doc::{Diff, OrdMap};
    ///
    /// let old: OrdMap<u32, char> = [(1, 'a'), (2, 'b'), (3, 'c')].into_iter().collect();
    /// let mut new = old.clone();
    /// new.insert(2, 'B');
    /// new.remove(&3);
    /// new.insert(4, 'd');
    ///
    /// let changes: Vec<_> = OrdMap::diff(&old, &new, |a, b| a == b).collect();
    /// assert_eq!(changes, [
    ///     Diff::Changed(&2, &'b', &'B'),
    ///     Diff::Removed(&3, &'c'),
    ///     Diff::Added(&4, &'d'),
    /// ]);
    /// ```
    pub fn diff<'a, F>(old: &'a Self, new: &'a Self, same: F) -> DiffIter<'a, K, V, F>
    where
        F: FnMut(&V, &V) -> bool,
    {
        if old.ptr_eq(new) {
            return DiffIter {
                old: Walk::default(),
                new: Walk::default(),
                same,
            };
        }
        DiffIter {
            old: Walk::over(old),
            new: Walk::over(new),
            same,
        }
    }
}

impl<K, V, const B: usize> Clone for OrdMap<K, V, B> {
    /// One pointer copied. The copy shares the whole tree.
    fn clone(&self) -> Self {
        OrdMap {
            root: self.root.clone(),
            len: self.len,
        }
    }
}

impl<K, V, const B: usize> Default for OrdMap<K, V, B> {
    fn default() -> Self {
        OrdMap::new()
    }
}

impl<K: Ord + fmt::Debug, V: fmt::Debug, const B: usize> fmt::Debug for OrdMap<K, V, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K: Ord, V: PartialEq, const B: usize> PartialEq for OrdMap<K, V, B> {
    /// The same keys with equal values, however the two trees are cut
    /// into nodes.
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || (self.len == other.len && self.iter().eq(other.iter()))
    }
}

impl<K: Ord, V: Eq, const B: usize> Eq for OrdMap<K, V, B> {}

impl<K: Ord + Clone, V: Clone, const B: usize> FromIterator<(K, V)> for OrdMap<K, V, B> {
    /// Entries already in rising key order, as a file has them, are built
    /// into a tree in one pass. In any other order they are sorted first;
    /// of two entries with one key, the later is kept.
    fn from_iter<I: IntoIterator<Item = (K, V)>>(entries: I) -> Self {
        let mut entries: Vec<(K, V)> = entries.into_iter().collect();
        if !entries.is_sorted_by(|a, b| a.0 < b.0) {
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            entries.dedup_by(|later, earlier| {
                let twice = later.0 == earlier.0;
                if twice {
                    std::mem::swap(later, earlier);
                }
                twice
            });
        }
        OrdMap::from_sorted(entries)
    }
}

impl<K: Ord + Clone, V: Clone, const B: usize> Extend<(K, V)> for OrdMap<K, V, B> {
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, entries: I) {
        for (key, value) in entries {
            self.insert(key, value);
        }
    }
}

impl<'a, K: Ord, V, const B: usize> IntoIterator for &'a OrdMap<K, V, B> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;

    fn into_iter(self) -> Iter<'a, K, V> {
        self.iter()
    }
}

/// Entries of an [`OrdMap`] in key order: all of them, or those in a
/// range.
pub struct Iter<'a, K, V> {
    /// For each branch above the leaf being read, the children not yet
    /// gone into.
    above: Vec<std::slice::Iter<'a, Kid<K, V>>>,
    leaf: std::slice::Iter<'a, (K, V)>,
    /// Where a range ends. Once passed, there is nothing more.
    end: Bound<K>,
}

impl<'a, K, V> Iter<'a, K, V> {
    /// Goes down to the first leaf under a node.
    fn descend(&mut self, mut node: &'a Arc<Node<K, V>>) {
        loop {
            match &**node {
                Node::Leaf(entries) => {
                    self.leaf = entries.iter();
                    return;
                }
                Node::Branch(kids) => {
                    let mut kids = kids.iter();
                    let Some((_, first)) = kids.next() else {
                        return;
                    };
                    self.above.push(kids);
                    node = first;
                }
            }
        }
    }
}

impl<'a, K: Ord, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<(&'a K, &'a V)> {
        loop {
            if let Some((key, value)) = self.leaf.next() {
                let past = match &self.end {
                    Bound::Unbounded => false,
                    Bound::Included(end) => key > end,
                    Bound::Excluded(end) => key >= end,
                };
                if past {
                    self.above.clear();
                    self.leaf = [].iter();
                    return None;
                }
                return Some((key, value));
            }
            // This leaf is done: on to the next child of the nearest
            // branch that has one.
            loop {
                match self.above.last_mut()?.next() {
                    Some((_, next)) => {
                        self.descend(next);
                        break;
                    }
                    None => {
                        self.above.pop();
                    }
                }
            }
        }
    }
}

/// One difference between two maps, as [`OrdMap::diff`] gives them.
#[derive(PartialEq, Eq, Debug)]
pub enum Diff<'a, K, V> {
    /// A key the new map has and the old did not, with its value.
    Added(&'a K, &'a V),
    /// A key the old map had and the new does not, with the value it had.
    Removed(&'a K, &'a V),
    /// A key in both, with its old value and its new.
    Changed(&'a K, &'a V, &'a V),
}

impl<'a, K, V> Diff<'a, K, V> {
    /// The key that differs.
    pub fn key(&self) -> &'a K {
        match self {
            Diff::Added(key, _) | Diff::Removed(key, _) | Diff::Changed(key, _, _) => key,
        }
    }

    /// The value the old map had under the key, if it had one.
    pub fn old_value(&self) -> Option<&'a V> {
        match self {
            Diff::Added(..) => None,
            Diff::Removed(_, old) | Diff::Changed(_, old, _) => Some(old),
        }
    }

    /// The value the new map has under the key, if it has one.
    pub fn new_value(&self) -> Option<&'a V> {
        match self {
            Diff::Removed(..) => None,
            Diff::Added(_, new) | Diff::Changed(_, _, new) => Some(new),
        }
    }
}

impl<K, V> Clone for Diff<'_, K, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K, V> Copy for Diff<'_, K, V> {}

/// A walk over a tree in key order that can stop in front of a whole
/// node, to be stepped over or gone into.
struct Walk<'a, K, V> {
    /// The part of each node on the way down that is still ahead, and,
    /// for a branch, how many branches high its children are.
    ahead: Vec<Ahead<'a, K, V>>,
}

enum Ahead<'a, K, V> {
    Entries(&'a [(K, V)]),
    Kids(&'a [Kid<K, V>], u32),
}

/// What a walk is standing in front of.
enum Next<'a, K, V> {
    Entry(&'a K, &'a V),
    Node(&'a Arc<Node<K, V>>, u32),
    End,
}

impl<K, V> Default for Walk<'_, K, V> {
    fn default() -> Self {
        Walk { ahead: Vec::new() }
    }
}

impl<'a, K, V> Walk<'a, K, V> {
    fn over<const B: usize>(map: &'a OrdMap<K, V, B>) -> Self {
        let mut walk = Walk::default();
        if let Some(root) = &map.root {
            walk.enter(root, map.height());
        }
        walk
    }

    fn enter(&mut self, node: &'a Node<K, V>, height: u32) {
        self.ahead.push(match node {
            Node::Leaf(entries) => Ahead::Entries(entries),
            Node::Branch(kids) => Ahead::Kids(kids, height.saturating_sub(1)),
        });
    }

    fn next(&mut self) -> Next<'a, K, V> {
        loop {
            let Some(top) = self.ahead.last() else {
                return Next::End;
            };
            match *top {
                Ahead::Entries(entries) => {
                    if let Some((key, value)) = entries.first() {
                        return Next::Entry(key, value);
                    }
                }
                Ahead::Kids(kids, height) => {
                    if let Some((_, kid)) = kids.first() {
                        return Next::Node(kid, height);
                    }
                }
            }
            // Nothing of this node is left ahead: back up to the one above.
            self.ahead.pop();
        }
    }

    /// Steps over what the walk is in front of, entry or whole node.
    fn step_over(&mut self) {
        match self.ahead.last_mut() {
            Some(Ahead::Entries(entries)) => *entries = entries.get(1..).unwrap_or_default(),
            Some(Ahead::Kids(kids, _)) => *kids = kids.get(1..).unwrap_or_default(),
            None => {}
        }
    }

    /// Goes into the node the walk is in front of.
    fn step_into(&mut self) {
        if let Next::Node(node, height) = self.next() {
            self.step_over();
            self.enter(node, height);
        }
    }
}

/// The differences between two maps, in key order. See [`OrdMap::diff`].
pub struct DiffIter<'a, K, V, F> {
    old: Walk<'a, K, V>,
    new: Walk<'a, K, V>,
    same: F,
}

impl<'a, K: Ord, V, F: FnMut(&V, &V) -> bool> Iterator for DiffIter<'a, K, V, F> {
    type Item = Diff<'a, K, V>;

    fn next(&mut self) -> Option<Diff<'a, K, V>> {
        // Both walks have always been through exactly the keys below
        // some point, so two entries side by side can be compared, and
        // one node met by both at once holds the same keys for both.
        loop {
            match (self.old.next(), self.new.next()) {
                (Next::End, Next::End) => return None,
                (Next::Node(old, old_height), Next::Node(new, new_height)) => {
                    if Arc::ptr_eq(old, new) {
                        self.old.step_over();
                        self.new.step_over();
                    } else {
                        // The taller is opened first, so that the two
                        // come level and can meet at a shared node.
                        if old_height >= new_height {
                            self.old.step_into();
                        }
                        if new_height >= old_height {
                            self.new.step_into();
                        }
                    }
                }
                (Next::Node(..), _) => self.old.step_into(),
                (_, Next::Node(..)) => self.new.step_into(),
                (Next::Entry(key, old), Next::End) => {
                    self.old.step_over();
                    return Some(Diff::Removed(key, old));
                }
                (Next::End, Next::Entry(key, new)) => {
                    self.new.step_over();
                    return Some(Diff::Added(key, new));
                }
                (Next::Entry(old_key, old), Next::Entry(new_key, new)) => {
                    match old_key.cmp(new_key) {
                        Ordering::Less => {
                            self.old.step_over();
                            return Some(Diff::Removed(old_key, old));
                        }
                        Ordering::Greater => {
                            self.new.step_over();
                            return Some(Diff::Added(new_key, new));
                        }
                        Ordering::Equal => {
                            self.old.step_over();
                            self.new.step_over();
                            if !(self.same)(old, new) {
                                return Some(Diff::Changed(old_key, old, new));
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
impl<K: Ord + fmt::Debug, V, const B: usize> OrdMap<K, V, B> {
    /// Panics unless the tree is a tree: every node holds from 1 to `B`
    /// entries, the root is not a branch of one, every leaf is at one
    /// depth, keys rise from left to right, every child's low key is at
    /// or below all under it and above all under the child before, and
    /// the count is the number of entries.
    pub(crate) fn check(&self) {
        fn walk<'a, K: Ord + fmt::Debug, V>(
            node: &'a Node<K, V>,
            max: usize,
            depth: usize,
            leaves_at: &mut Option<usize>,
            last: &mut Option<&'a K>,
            count: &mut usize,
        ) {
            assert!(
                (1..=max).contains(&node.len()),
                "a node of {} entries",
                node.len()
            );
            match node {
                Node::Leaf(entries) => {
                    assert_eq!(
                        *leaves_at.get_or_insert(depth),
                        depth,
                        "leaves at two depths"
                    );
                    for (key, _) in entries {
                        assert!(
                            last.is_none_or(|last| last < key),
                            "{key:?} is out of order"
                        );
                        *last = Some(key);
                        *count += 1;
                    }
                }
                Node::Branch(kids) => {
                    for (low, kid) in kids {
                        assert!(
                            last.is_none_or(|last| last < low),
                            "low key {low:?} too low"
                        );
                        let before = *count;
                        // Everything under the child is above the last key
                        // before it; make it also at or above the low key.
                        let mut floor = *last;
                        walk(kid, max, depth + 1, leaves_at, &mut floor, count);
                        assert!(*count > before);
                        let first = first_key(kid).expect("a child with an entry");
                        assert!(low <= first, "low key {low:?} is above {first:?}");
                        *last = floor;
                    }
                }
            }
        }

        fn first_key<K, V>(mut node: &Node<K, V>) -> Option<&K> {
            loop {
                match node {
                    Node::Leaf(entries) => return entries.first().map(|(key, _)| key),
                    Node::Branch(kids) => node = &kids.first()?.1,
                }
            }
        }

        let Some(root) = self.root.as_deref() else {
            assert_eq!(self.len, 0);
            return;
        };
        if let Node::Branch(kids) = root {
            assert!(kids.len() >= 2, "a root with one child");
        }
        let (mut leaves_at, mut last, mut count) = (None, None, 0);
        walk(root, B, 0, &mut leaves_at, &mut last, &mut count);
        assert_eq!(count, self.len, "the count is wrong");
        assert_eq!(leaves_at, Some(self.height() as usize));
    }

    /// Where every node of the tree is in memory: a test's way to count
    /// the nodes two trees share, and so to prove that an edit rebuilt a
    /// path and not the tree.
    pub(crate) fn nodes(&self) -> Vec<*const ()> {
        fn walk<K, V>(node: &Arc<Node<K, V>>, into: &mut Vec<*const ()>) {
            into.push(Arc::as_ptr(node).cast());
            if let Node::Branch(kids) = &**node {
                for (_, kid) in kids {
                    walk(kid, into);
                }
            }
        }
        let mut nodes = Vec::new();
        if let Some(root) = &self.root {
            walk(root, &mut nodes);
        }
        nodes
    }
}

#[cfg(test)]
mod tests;

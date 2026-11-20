// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The map against a plain `BTreeMap`, and the tree against its own
//! shape.
//!
//! Most of these run on nodes of four, the smallest there are, so that a
//! few dozen entries are already a tree four levels deep and every split,
//! join and evening-out happens many times over.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::ops::Bound;

use proptest::prelude::*;

use super::*;

type Small = OrdMap<u32, u32, 4>;

/// What differs between two plain maps, found the slow way: every key of
/// either, looked up in both.
fn naive_diff(
    old: &BTreeMap<u32, u32>,
    new: &BTreeMap<u32, u32>,
) -> Vec<(u32, Option<u32>, Option<u32>)> {
    let mut keys: Vec<u32> = old.keys().chain(new.keys()).copied().collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .map(|key| (key, old.get(&key).copied(), new.get(&key).copied()))
        .filter(|(_, old, new)| old != new)
        .collect()
}

fn diff_of<const B: usize>(
    old: &OrdMap<u32, u32, B>,
    new: &OrdMap<u32, u32, B>,
) -> Vec<(u32, Option<u32>, Option<u32>)> {
    OrdMap::diff(old, new, |a, b| a == b)
        .map(|diff| {
            (
                *diff.key(),
                diff.old_value().copied(),
                diff.new_value().copied(),
            )
        })
        .collect()
}

fn entries<const B: usize>(map: &OrdMap<u32, u32, B>) -> Vec<(u32, u32)> {
    map.iter().map(|(key, value)| (*key, *value)).collect()
}

fn plain(map: &BTreeMap<u32, u32>) -> Vec<(u32, u32)> {
    map.iter().map(|(key, value)| (*key, *value)).collect()
}

/// How many nodes of `edited` are not also nodes of `base`.
fn nodes_rebuilt<const B: usize>(
    base: &OrdMap<u32, u32, B>,
    edited: &OrdMap<u32, u32, B>,
) -> usize {
    let shared: HashSet<*const ()> = base.nodes().into_iter().collect();
    edited
        .nodes()
        .into_iter()
        .filter(|node| !shared.contains(node))
        .count()
}

#[test]
fn an_empty_map_has_nothing_in_it() {
    let map = Small::new();
    map.check();
    assert_eq!(map.len(), 0);
    assert!(map.is_empty());
    assert_eq!(map.get(&1), None);
    assert_eq!(map.first(), None);
    assert_eq!(map.last(), None);
    assert_eq!(map.iter().next(), None);
    assert_eq!(map.range(..).next(), None);
    assert!(map.ptr_eq(&Small::default()));
    assert_eq!(format!("{map:?}"), "{}");
}

#[test]
fn insert_adds_or_replaces_and_says_which() {
    let mut map = Small::new();
    assert_eq!(map.insert(5, 50), None);
    assert_eq!(map.insert(3, 30), None);
    assert_eq!(map.insert(5, 55), Some(50));
    assert_eq!(map.len(), 2);
    assert_eq!(map.get(&5), Some(&55));
    assert_eq!(map.get(&3), Some(&30));
    assert_eq!(map.get(&4), None);
    assert!(map.contains_key(&3));
    assert!(!map.contains_key(&4));
    assert_eq!(format!("{map:?}"), "{3: 30, 5: 55}");
}

#[test]
fn remove_takes_out_what_is_there_and_nothing_else() {
    let mut map: Small = (0..100).map(|n| (n, n)).collect();
    assert_eq!(map.remove(&40), Some(40));
    assert_eq!(map.remove(&40), None);
    assert_eq!(map.remove(&1000), None);
    assert_eq!(map.len(), 99);
    assert_eq!(map.get(&40), None);
    assert_eq!(map.get(&41), Some(&41));
    map.check();
}

#[test]
fn a_copy_is_the_map_as_it_was() {
    let mut map: Small = (0..200).map(|n| (n, n)).collect();
    let before = map.clone();
    assert!(before.ptr_eq(&map));
    for n in (0..200).step_by(3) {
        map.remove(&n);
    }
    for n in 200..260 {
        map.insert(n, 0);
    }
    *map.get_mut(&1).unwrap() = 99;
    assert!(!before.ptr_eq(&map));
    assert_eq!(
        entries(&before),
        (0..200).map(|n| (n, n)).collect::<Vec<_>>()
    );
    before.check();
    map.check();
    assert_eq!(map.get(&1), Some(&99));
    assert_eq!(before.get(&1), Some(&1));
}

#[test]
fn taking_out_or_changing_what_is_not_there_rebuilds_nothing() {
    let mut map: Small = (0..100).map(|n| (2 * n, n)).collect();
    let before = map.clone();
    assert_eq!(map.remove(&7), None);
    assert_eq!(map.get_mut(&7), None);
    assert!(map.ptr_eq(&before));
}

#[test]
fn one_edit_rebuilds_the_path_to_it_and_shares_the_rest() {
    let base: OrdMap<u32, u32> = (0..100_000).map(|n| (2 * n, n)).collect();
    base.check();
    let height = base.height() as usize;
    let all = base.nodes().len();
    assert!(all > 6000, "{all} nodes");

    let mut replaced = base.clone();
    replaced.insert(50_000, 1);
    assert_eq!(nodes_rebuilt(&base, &replaced), height + 1);

    let mut changed = base.clone();
    *changed.get_mut(&50_000).unwrap() = 1;
    assert_eq!(nodes_rebuilt(&base, &changed), height + 1);

    // A new key in a full leaf splits it, and may split what is above.
    let mut added = base.clone();
    added.insert(50_001, 1);
    let rebuilt = nodes_rebuilt(&base, &added);
    assert!(
        (height + 1..=2 * (height + 1) + 1).contains(&rebuilt),
        "{rebuilt} rebuilt"
    );

    // Taking one out may also rebuild a neighbour at each level.
    let mut removed = base.clone();
    removed.remove(&50_000);
    assert!(nodes_rebuilt(&base, &removed) <= 2 * (height + 1));
    removed.check();
    added.check();
}

#[test]
fn a_map_nobody_else_holds_is_edited_where_it_is() {
    let mut map: OrdMap<u32, u32> = (0..10_000).map(|n| (n, n)).collect();
    let nodes: HashSet<*const ()> = map.nodes().into_iter().collect();
    for n in 0..10_000 {
        map.insert(n, n + 1);
    }
    let after: HashSet<*const ()> = map.nodes().into_iter().collect();
    assert_eq!(nodes, after);
}

#[test]
fn keys_added_in_rising_order_leave_the_nodes_full() {
    let mut map: OrdMap<u32, u32> = OrdMap::new();
    for n in 0..10_000 {
        map.insert(n, n);
    }
    map.check();
    // Each leaf but the last holds sixteen: 625 leaves, and the branches.
    let nodes = map.nodes().len();
    assert!(
        nodes <= 625 + 42 + 4 + 1,
        "{nodes} nodes for 10,000 entries"
    );
}

#[test]
fn a_map_built_from_a_list_is_packed_and_is_the_same_map() {
    let sorted: Vec<(u32, u32)> = (0..10_000).map(|n| (n, n * 2)).collect();
    let built: OrdMap<u32, u32> = sorted.iter().copied().collect();
    built.check();
    assert_eq!(built.len(), 10_000);
    assert_eq!(entries(&built), sorted);
    assert!(built.nodes().len() <= 625 + 40 + 3 + 1);

    let mut one_by_one: OrdMap<u32, u32> = OrdMap::new();
    one_by_one.extend(sorted.iter().rev().copied());
    assert_eq!(built, one_by_one);
    assert!(!built.ptr_eq(&one_by_one));

    for size in 0..70_u32 {
        let small: Small = (0..size).map(|n| (n, n)).collect();
        small.check();
        assert_eq!(small.len(), size as usize);
    }
}

#[test]
fn a_list_out_of_order_is_sorted_and_the_later_of_two_is_kept() {
    let map: Small = [(5, 1), (2, 1), (9, 1), (2, 2), (5, 3), (5, 4), (1, 1)]
        .into_iter()
        .collect();
    map.check();
    assert_eq!(entries(&map), [(1, 1), (2, 2), (5, 4), (9, 1)]);
    // In order but with a key twice is also not "sorted".
    let twice: Small = [(1, 1), (2, 1), (2, 2), (3, 1)].into_iter().collect();
    assert_eq!(entries(&twice), [(1, 1), (2, 2), (3, 1)]);
}

#[test]
fn the_first_and_last_entries_are_the_lowest_and_highest_keys() {
    let mut map: Small = (10..90).map(|n| (n, n)).collect();
    assert_eq!(map.first(), Some((&10, &10)));
    assert_eq!(map.last(), Some((&89, &89)));
    map.insert(3, 0);
    map.remove(&89);
    assert_eq!(map.first(), Some((&3, &0)));
    assert_eq!(map.last(), Some((&88, &88)));
    assert_eq!(map.keys().count(), map.len());
    assert_eq!(map.values().copied().max(), Some(88));
    assert_eq!((&map).into_iter().count(), map.len());
}

#[test]
fn a_range_is_every_entry_between_its_bounds_and_no_other() {
    let plain: BTreeMap<u32, u32> = (0..120).map(|n| (3 * n, n)).collect();
    let map: Small = plain.iter().map(|(key, value)| (*key, *value)).collect();
    let bounds = |at: u32| [Bound::Included(at), Bound::Excluded(at), Bound::Unbounded];
    for low in [0, 1, 3, 4, 177, 179, 180, 357, 358, 500] {
        for high in [0, 1, 3, 4, 177, 179, 180, 357, 358, 500] {
            for start in bounds(low) {
                for end in bounds(high) {
                    // The plain map panics on a range that runs backwards
                    // or is empty at one excluded point; here it is empty.
                    let backwards = match (start, end) {
                        (Bound::Included(a), Bound::Included(b)) => a > b,
                        (Bound::Unbounded, _) | (_, Bound::Unbounded) => false,
                        (
                            Bound::Included(a) | Bound::Excluded(a),
                            Bound::Included(b) | Bound::Excluded(b),
                        ) => a >= b,
                    };
                    let got: Vec<(u32, u32)> =
                        map.range((start, end)).map(|(k, v)| (*k, *v)).collect();
                    let want: Vec<(u32, u32)> = if backwards {
                        Vec::new()
                    } else {
                        plain.range((start, end)).map(|(k, v)| (*k, *v)).collect()
                    };
                    assert_eq!(got, want, "{start:?} to {end:?}");
                }
            }
        }
    }
    assert_eq!(map.range(..).count(), 120);
    assert_eq!(
        map.range(30..=36).map(|(key, _)| *key).collect::<Vec<_>>(),
        [30, 33, 36]
    );
}

#[test]
fn a_map_does_not_differ_from_itself_or_its_copy() {
    let map: Small = (0..500).map(|n| (n, n)).collect();
    let asked = Cell::new(0);
    let count = |a: &Small, b: &Small| {
        OrdMap::diff(a, b, |x, y| {
            asked.set(asked.get() + 1);
            x == y
        })
        .count()
    };
    assert_eq!(count(&map, &map), 0);
    assert_eq!(count(&map, &map.clone()), 0);
    assert_eq!(count(&Small::new(), &Small::new()), 0);
    assert_eq!(asked.get(), 0);
}

#[test]
fn what_differs_is_found_without_looking_inside_what_is_shared() {
    let base: OrdMap<u32, u32> = (0..100_000).map(|n| (2 * n, n)).collect();
    let mut edited = base.clone();
    edited.insert(100_000, 7);
    let asked = Cell::new(0_usize);
    let found: Vec<_> = OrdMap::diff(&base, &edited, |a, b| {
        asked.set(asked.get() + 1);
        a == b
    })
    .collect();
    assert_eq!(found, [Diff::Changed(&100_000, &50_000, &7)]);
    // Only the one leaf the two do not share was read, of 6,250.
    assert!(asked.get() <= 16, "{} values compared", asked.get());

    edited.insert(11, 1);
    edited.remove(&180_000);
    edited.insert(250_000, 2);
    asked.set(0);
    let found = diff_of(&base, &edited);
    assert_eq!(
        found,
        [
            (11, None, Some(1)),
            (100_000, Some(50_000), Some(7)),
            (180_000, Some(90_000), None),
            (250_000, None, Some(2)),
        ]
    );
}

#[test]
fn what_differs_is_right_between_trees_of_different_heights_and_of_no_shared_part() {
    let small: Small = (0..3).map(|n| (n, n)).collect();
    let mut big: Small = small.clone();
    for n in 3..300 {
        big.insert(n, n);
    }
    assert!(big.height() > small.height() + 2);
    let grown: Vec<_> = (3..300).map(|n| (n, None, Some(n))).collect();
    assert_eq!(diff_of(&small, &big), grown);
    let shrunk: Vec<_> = (3..300).map(|n| (n, Some(n), None)).collect();
    assert_eq!(diff_of(&big, &small), shrunk);
    assert_eq!(
        diff_of(&Small::new(), &small),
        [(0, None, Some(0)), (1, None, Some(1)), (2, None, Some(2))]
    );
    assert_eq!(diff_of(&small, &Small::new()).len(), 3);

    // Built apart: equal, and sharing nothing.
    let again: Small = (0..300).map(|n| (n, n)).collect();
    assert_eq!(diff_of(&big, &again), []);
    let evens: Small = (0..300).step_by(2).map(|n| (n, n + 1)).collect();
    let want: Vec<_> = (0..300)
        .map(|n| (n, Some(n), (n % 2 == 0).then_some(n + 1)))
        .collect();
    assert_eq!(diff_of(&big, &evens), want);
}

#[test]
fn a_difference_says_its_key_and_both_values() {
    let (key, old, new) = (1_u32, 2_u32, 3_u32);
    let added = Diff::Added(&key, &new);
    let removed = Diff::Removed(&key, &old);
    let changed = Diff::Changed(&key, &old, &new);
    assert_eq!(
        (added.key(), added.old_value(), added.new_value()),
        (&1, None, Some(&3))
    );
    assert_eq!(
        (removed.key(), removed.old_value(), removed.new_value()),
        (&1, Some(&2), None)
    );
    assert_eq!(
        (changed.key(), changed.old_value(), changed.new_value()),
        (&1, Some(&2), Some(&3))
    );
    let copy = changed;
    assert_eq!(copy, changed);
}

#[test]
fn maps_are_equal_by_what_they_hold_and_not_by_how_they_are_cut() {
    let built: Small = (0..50).map(|n| (n, n)).collect();
    let mut grown = Small::new();
    for n in (0..50).rev() {
        grown.insert(n, n);
    }
    assert_eq!(built, grown);
    grown.insert(7, 0);
    assert_ne!(built, grown);
    grown.insert(7, 7);
    grown.insert(50, 50);
    assert_ne!(built, grown);
}

#[test]
fn emptying_a_map_one_key_at_a_time_keeps_it_a_tree_all_the_way_down() {
    for order in [1_u32, 7, 13, 101, 499] {
        let mut map: Small = (0..500).map(|n| (n, n)).collect();
        for n in 0..500 {
            assert_eq!(map.remove(&(n * order % 500)), Some(n * order % 500));
            map.check();
        }
        assert!(map.is_empty());
        assert!(map.ptr_eq(&Small::new()));
    }
}

#[test]
fn a_map_that_shrank_gives_its_nodes_back() {
    let mut map: OrdMap<u32, u32> = (0..10_000).map(|n| (n, n)).collect();
    let full = map.nodes().len();
    for n in (0..10_000).filter(|n| n % 16 != 0) {
        map.remove(&n);
    }
    map.check();
    assert_eq!(map.len(), 625);
    // One entry in sixteen is left, and no leaf was left with one: thin
    // leaves were joined, so the tree is a fraction of what it was.
    let left = map.nodes().len();
    assert!(
        left <= 220 && left * 3 < full,
        "{left} nodes of {full} for 625 entries"
    );
    assert_eq!(
        entries(&map),
        (0..10_000).step_by(16).map(|n| (n, n)).collect::<Vec<_>>()
    );
}

#[test]
fn a_thin_node_beside_a_full_one_takes_half_of_what_it_has() {
    // Packed, every leaf holds eight. Taking a leaf down to one entry
    // leaves it too thin to stand and its neighbour too full to join.
    type Eights = OrdMap<u32, u32, 8>;
    let packed: Eights = (0..512).map(|n| (n, n)).collect();
    let leaves = packed.nodes().len();

    // The first leaf, whose only neighbour is to its right.
    let mut front = packed.clone();
    for n in 0..7 {
        front.remove(&n);
        front.check();
    }
    // The last leaf, whose only neighbour is to its left.
    let mut back = packed.clone();
    for n in 505..512 {
        back.remove(&n);
        back.check();
    }
    // One in the middle.
    let mut middle = packed.clone();
    for n in 257..264 {
        middle.remove(&n);
        middle.check();
    }
    for (map, gone) in [(&front, 0..7), (&back, 505..512), (&middle, 257..264)] {
        // Evened out, not joined: as many nodes as before.
        assert_eq!(map.nodes().len(), leaves);
        for n in 0..512 {
            assert_eq!(map.get(&n), (!gone.contains(&n)).then_some(&n), "key {n}");
        }
        assert_eq!(map.len(), 505);
    }
    packed.check();
    assert_eq!(packed.len(), 512);
}

/// One thing done to a map.
#[derive(Clone, Debug)]
enum Op {
    Insert(u32, u32),
    Remove(u32),
    Change(u32, u32),
    /// Keep the map as it is now, to be looked at again at the end.
    Keep,
}

fn op(keys: u32) -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..keys, any::<u32>()).prop_map(|(key, value)| Op::Insert(key, value)),
        3 => (0..keys).prop_map(Op::Remove),
        1 => (0..keys, any::<u32>()).prop_map(|(key, value)| Op::Change(key, value)),
        1 => Just(Op::Keep),
    ]
}

/// Runs a list of edits on a map and on a plain one, keeping copies on
/// the way, and holds the map to the plain one in every way there is.
fn run<const B: usize>(start: Vec<(u32, u32)>, ops: Vec<Op>) -> Result<(), TestCaseError> {
    let mut plain_map: BTreeMap<u32, u32> = start.iter().copied().collect();
    let mut map: OrdMap<u32, u32, B> = start.into_iter().collect();
    map.check();
    let mut kept = vec![(map.clone(), plain_map.clone())];
    for op in ops {
        match op {
            Op::Insert(key, value) => {
                prop_assert_eq!(map.insert(key, value), plain_map.insert(key, value));
            }
            Op::Remove(key) => prop_assert_eq!(map.remove(&key), plain_map.remove(&key)),
            Op::Change(key, value) => {
                let (ours, theirs) = (map.get_mut(&key), plain_map.get_mut(&key));
                prop_assert_eq!(ours.is_some(), theirs.is_some());
                if let (Some(ours), Some(theirs)) = (ours, theirs) {
                    *ours = value;
                    *theirs = value;
                }
            }
            Op::Keep => kept.push((map.clone(), plain_map.clone())),
        }
        map.check();
        prop_assert_eq!(map.len(), plain_map.len());
    }
    kept.push((map.clone(), plain_map.clone()));

    // Every copy is still what it was when it was taken.
    for (copy, was) in &kept {
        copy.check();
        prop_assert_eq!(entries(copy), plain(was));
        prop_assert_eq!(
            copy.first().map(|(k, v)| (*k, *v)),
            was.first_key_value().map(|(k, v)| (*k, *v))
        );
        prop_assert_eq!(
            copy.last().map(|(k, v)| (*k, *v)),
            was.last_key_value().map(|(k, v)| (*k, *v))
        );
        for key in was.keys() {
            prop_assert_eq!(copy.get(key), was.get(key));
        }
    }
    // What differs between two of them is what the slow way finds: each
    // against the one kept next, against the first and against the last,
    // both ways round.
    let ends = [0, kept.len() - 1];
    for (at, (copy, was)) in kept.iter().enumerate() {
        for other in ends.into_iter().chain([(at + 1).min(kept.len() - 1)]) {
            let (other, other_was) = &kept[other];
            prop_assert_eq!(diff_of(copy, other), naive_diff(was, other_was));
            prop_assert_eq!(diff_of(other, copy), naive_diff(other_was, was));
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn the_map_is_a_plain_sorted_map_and_its_copies_never_change(
        start in prop::collection::vec((0..80_u32, any::<u32>()), 0..80),
        ops in prop::collection::vec(op(80), 0..200),
    ) {
        run::<4>(start, ops)?;
    }

    #[test]
    fn the_same_emptying_a_small_map_over_and_over(
        ops in prop::collection::vec(op(12), 0..300),
    ) {
        run::<5>(Vec::new(), ops)?;
    }

    #[test]
    fn a_range_of_the_map_is_the_range_of_a_plain_one(
        start in prop::collection::vec((0..200_u32, any::<u32>()), 0..150),
        low in 0..210_u32,
        len in 0..210_u32,
        closed in any::<bool>(),
    ) {
        let plain_map: BTreeMap<u32, u32> = start.iter().copied().collect();
        let map: Small = start.into_iter().collect();
        let high = low + len;
        let got: Vec<(u32, u32)> = if closed {
            map.range(low..=high).map(|(k, v)| (*k, *v)).collect()
        } else {
            map.range(low..high).map(|(k, v)| (*k, *v)).collect()
        };
        let want: Vec<(u32, u32)> = if closed {
            plain_map.range(low..=high).map(|(k, v)| (*k, *v)).collect()
        } else {
            plain_map.range(low..high).map(|(k, v)| (*k, *v)).collect()
        };
        prop_assert_eq!(got, want);
        let from: Vec<u32> = map.range(low..).map(|(k, _)| *k).collect();
        let want_from: Vec<u32> = plain_map.range(low..).map(|(k, _)| *k).collect();
        prop_assert_eq!(from, want_from);
    }
}

proptest! {
    // Fewer and larger: wider nodes, where the cases above are many
    // levels of the smallest.
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn the_same_taking_a_packed_map_apart(
        size in 0..300_u32,
        ops in prop::collection::vec(
            prop_oneof![
                6 => (0..300_u32).prop_map(Op::Remove),
                1 => (0..300_u32, any::<u32>()).prop_map(|(key, value)| Op::Insert(key, value)),
                1 => Just(Op::Keep),
            ],
            0..300,
        ),
    ) {
        // Nodes of eight and of sixteen are thin under two and under
        // four: a thin one beside a full one is evened out, not joined.
        let start: Vec<(u32, u32)> = (0..size).map(|n| (n, n)).collect();
        run::<8>(start.clone(), ops.clone())?;
        run::<16>(start, ops)?;
    }

    #[test]
    fn the_same_with_keys_far_apart_and_the_default_node(
        start in prop::collection::vec((0..3000_u32, any::<u32>()), 0..500),
        ops in prop::collection::vec(op(3000), 0..120),
    ) {
        run::<16>(start, ops)?;
    }
}

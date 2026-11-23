// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The measurements behind the choice of the document's map.
//!
//! ```text
//! cargo run --release -p concat-doc --example map_bench
//! ```
//!
//! It times the work a document does on its tables, at a thousand, ten
//! thousand and a hundred thousand entries, and counts the memory each
//! leaves held: one edit, a thousand edits with every earlier map kept
//! (the undo stack), a drag recomputed six hundred times from the map it
//! started on, finding what differs between two maps, building from
//! sorted entries (a file being read) and reading every entry in order.
//! Keys are `u64` and values an `Arc` of 64 bytes, as a table's are.
//! Every figure is the median of nine runs; a run that is over in under a
//! microsecond is done a thousand times and divided.
//!
//! Memory is counted by a global allocator that adds up what is asked for
//! and given back. That is the one piece of `unsafe` near this crate, and
//! it is here, in the benchmark, and not in the library.
//!
//! The results are written up in the documentation of `concat_doc::map`.
//!
//! # The other candidate
//!
//! The map was chosen against `rpds` 1.2.1, which is no longer a
//! dependency. To measure it again, add `rpds = "1"` to this crate's
//! dev-dependencies, give it the adapter below, and add
//! `run::<Rpds>(size)` beside the other in `main`:
//!
//! ```text
//! #[derive(Clone)]
//! struct Rpds(rpds::RedBlackTreeMapSync<u64, Arc<Item>>);
//!
//! impl Candidate for Rpds {
//!     const NAME: &'static str = "rpds 1.2.1 RedBlackTreeMapSync";
//!
//!     fn build(sorted: Vec<(u64, Arc<Item>)>) -> Self {
//!         Rpds(sorted.into_iter().collect())
//!     }
//!     fn insert(&mut self, key: u64, value: Arc<Item>) {
//!         self.0.insert_mut(key, value);
//!     }
//!     fn remove(&mut self, key: u64) {
//!         self.0.remove_mut(&key);
//!     }
//!     fn get(&self, key: u64) -> Option<&Arc<Item>> {
//!         self.0.get(&key)
//!     }
//!     fn sum(&self) -> u64 {
//!         self.0.iter().map(|(key, item)| key ^ item.0[0]).fold(0, u64::wrapping_add)
//!     }
//!     // It has no way to step over what two maps share, so the two are
//!     // read side by side, which is the whole of both.
//!     fn differences(old: &Self, new: &Self) -> usize {
//!         let (mut a, mut b) = (old.0.iter().peekable(), new.0.iter().peekable());
//!         let mut count = 0;
//!         loop {
//!             match (a.peek(), b.peek()) {
//!                 (None, None) => return count,
//!                 (Some(_), None) => { count += 1; a.next(); }
//!                 (None, Some(_)) => { count += 1; b.next(); }
//!                 (Some((ka, va)), Some((kb, vb))) => match ka.cmp(kb) {
//!                     Ordering::Less => { count += 1; a.next(); }
//!                     Ordering::Greater => { count += 1; b.next(); }
//!                     Ordering::Equal => {
//!                         count += usize::from(!Arc::ptr_eq(va, vb));
//!                         a.next();
//!                         b.next();
//!                     }
//!                 },
//!             }
//!         }
//!     }
//! }
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering as Atomic};
use std::time::{Duration, Instant};

use concat_doc::OrdMap;

/// The bytes asked for and not yet given back.
static HELD: AtomicIsize = AtomicIsize::new(0);

struct Counting;

// SAFETY: every call is handed to the system's allocator unchanged; this
// only adds up the sizes on the way past.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        HELD.fetch_add(layout.size() as isize, Atomic::Relaxed);
        // SAFETY: the caller's layout, passed on as it came.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        HELD.fetch_sub(layout.size() as isize, Atomic::Relaxed);
        // SAFETY: a pointer this allocator gave out, with its layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        HELD.fetch_add(new_size as isize - layout.size() as isize, Atomic::Relaxed);
        // SAFETY: as `dealloc`, with the caller's new size.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn held() -> isize {
    HELD.load(Atomic::Relaxed)
}

/// What a table holds an `Arc` of: 64 bytes.
pub struct Item([u64; 8]);

fn item(seed: u64) -> Arc<Item> {
    Arc::new(Item([seed; 8]))
}

/// A map being measured.
trait Candidate: Clone {
    const NAME: &'static str;

    fn build(sorted: Vec<(u64, Arc<Item>)>) -> Self;
    fn insert(&mut self, key: u64, value: Arc<Item>);
    fn remove(&mut self, key: u64);
    fn get(&self, key: u64) -> Option<&Arc<Item>>;
    fn sum(&self) -> u64;
    fn differences(old: &Self, new: &Self) -> usize;
}

#[derive(Clone)]
struct Own<const B: usize>(OrdMap<u64, Arc<Item>, B>);

impl<const B: usize> Candidate for Own<B> {
    const NAME: &'static str = match B {
        8 => "own tree, 8 a node",
        16 => "own tree, 16 a node",
        32 => "own tree, 32 a node",
        64 => "own tree, 64 a node",
        _ => "own tree",
    };

    fn build(sorted: Vec<(u64, Arc<Item>)>) -> Self {
        Own(sorted.into_iter().collect())
    }

    fn insert(&mut self, key: u64, value: Arc<Item>) {
        self.0.insert(key, value);
    }

    fn remove(&mut self, key: u64) {
        self.0.remove(&key);
    }

    fn get(&self, key: u64) -> Option<&Arc<Item>> {
        self.0.get(&key)
    }

    fn sum(&self) -> u64 {
        self.0
            .iter()
            .map(|(key, item)| key ^ item.0[0])
            .fold(0, u64::wrapping_add)
    }

    fn differences(old: &Self, new: &Self) -> usize {
        OrdMap::diff(&old.0, &new.0, Arc::ptr_eq).count()
    }
}

/// A small generator of the same numbers every run, so that every
/// candidate is asked for the same keys.
struct Keys(u64);

impl Keys {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A key that is in a map of `size` entries. The keys there are the
    /// even numbers from 2, so that every odd number is a new key.
    fn present(&mut self, size: u64) -> u64 {
        2 * (self.next() % size + 1)
    }

    fn absent(&mut self, size: u64) -> u64 {
        self.present(size) - 1
    }
}

const RUNS: usize = 9;

/// The median of nine runs of something that says how long it took and
/// how many bytes it left held.
fn median(mut run: impl FnMut() -> (Duration, isize)) -> (Duration, isize) {
    let mut times = Vec::with_capacity(RUNS);
    let mut bytes = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let (time, held) = run();
        times.push(time);
        bytes.push(held);
    }
    times.sort();
    bytes.sort();
    (times[RUNS / 2], bytes[RUNS / 2])
}

fn sorted(size: u64) -> Vec<(u64, Arc<Item>)> {
    (1..=size).map(|n| (2 * n, item(n))).collect()
}

fn nanos(time: Duration, per: u32) -> String {
    let each = time.as_nanos() as f64 / f64::from(per);
    if each >= 1_000_000.0 {
        format!("{:.2} ms", each / 1_000_000.0)
    } else if each >= 1_000.0 {
        format!("{:.2} us", each / 1_000.0)
    } else {
        format!("{each:.0} ns")
    }
}

fn bytes(held: isize, per: isize) -> String {
    let each = held / per;
    if each >= 10 * 1024 * 1024 {
        format!("{:.1} MB", each as f64 / (1024.0 * 1024.0))
    } else if each >= 10 * 1024 {
        format!("{:.1} KB", each as f64 / 1024.0)
    } else {
        format!("{each} B")
    }
}

fn row(what: &str, size: u64, time: Duration, per: u32, memory: Option<String>) {
    println!(
        "| {what} | {size} | {} | {} |",
        nanos(time, per),
        memory.unwrap_or_default()
    );
}

/// One edit to a copy of the base, a thousand times over on a thousand
/// copies, each copy kept so that its memory can be counted and so that
/// freeing it is not in the time.
fn one_edit<C: Candidate>(
    what: &str,
    size: u64,
    base: &C,
    seed: u64,
    edit: impl Fn(&mut C, &mut Keys, u64),
) {
    const EDITS: usize = 1000;
    let (time, held_after) = median(|| {
        let mut keys = Keys(seed);
        let mut kept: Vec<C> = Vec::with_capacity(EDITS);
        let before = held();
        let start = Instant::now();
        for _ in 0..EDITS {
            let mut copy = base.clone();
            edit(&mut copy, &mut keys, size);
            kept.push(copy);
        }
        let time = start.elapsed();
        (time, held() - before)
    });
    row(
        what,
        size,
        time,
        EDITS as u32,
        Some(bytes(held_after, EDITS as isize)),
    );
}

fn run<C: Candidate>(size: u64) {
    println!("\n### {}, {size} entries\n", C::NAME);
    println!("| Work | Entries | Time | Added |");
    println!("|---|---|---|---|");

    // A file being read: a tree from entries in order. What is added is
    // the whole tree, the pointers to the values in it but not the values
    // themselves; the list it was built from is gone by the end, so its
    // size is added back.
    let (time, held_after) = median(|| {
        let entries = sorted(size);
        let list = (entries.capacity() * size_of::<(u64, Arc<Item>)>()) as isize;
        let before = held();
        let start = Instant::now();
        let map = C::build(entries);
        let time = start.elapsed();
        let held_after = held() - before + list;
        black_box(&map);
        (time, held_after)
    });
    row(
        "build from sorted",
        size,
        time,
        1,
        Some(bytes(held_after, 1)),
    );

    let base = C::build(sorted(size));

    let (time, _) = median(|| {
        let start = Instant::now();
        let mut sum = 0_u64;
        for _ in 0..10 {
            sum = sum.wrapping_add(black_box(&base).sum());
        }
        black_box(sum);
        (start.elapsed(), 0)
    });
    row("read every entry in order", size, time, 10, None);

    let (time, _) = median(|| {
        let mut keys = Keys(11);
        let start = Instant::now();
        let mut found = 0_usize;
        for _ in 0..1000 {
            found += usize::from(base.get(keys.present(size)).is_some());
        }
        black_box(found);
        (start.elapsed(), 0)
    });
    row("look one up", size, time, 1000, None);

    one_edit(
        "one insert, a new key at the end",
        size,
        &base,
        1,
        |map: &mut C, keys, size| map.insert(2 * size + 2 + keys.next() % 1000, item(7)),
    );
    one_edit(
        "one insert, a new key anywhere",
        size,
        &base,
        2,
        |map: &mut C, keys, size| {
            map.insert(keys.absent(size), item(7));
        },
    );
    one_edit("one replace", size, &base, 3, |map: &mut C, keys, size| {
        map.insert(keys.present(size), item(7));
    });
    one_edit("one remove", size, &base, 4, |map: &mut C, keys, size| {
        map.remove(keys.present(size));
    });

    // The undo stack: a thousand edits one after another, each map kept.
    // Three in four replace an entry, the rest add one or take one out.
    const STEPS: usize = 1000;
    let (time, held_after) = median(|| {
        let mut keys = Keys(5);
        let mut stack: Vec<C> = Vec::with_capacity(STEPS + 1);
        stack.push(base.clone());
        let before = held();
        let start = Instant::now();
        for step in 0..STEPS {
            let mut next = stack[step].clone();
            match step % 8 {
                0 => next.insert(keys.absent(size), item(step as u64)),
                1 => next.remove(keys.present(size)),
                _ => next.insert(keys.present(size), item(step as u64)),
            }
            stack.push(next);
        }
        let time = start.elapsed();
        (time, held() - before)
    });
    row(
        "1,000 edits in a row, every map kept: each",
        size,
        time,
        STEPS as u32,
        Some(bytes(held_after, STEPS as isize)),
    );
    row(
        "the same: all 1,000",
        size,
        time,
        1,
        Some(bytes(held_after, 1)),
    );

    // A drag: every pointer event starts again from the map the drag
    // began on, and the map from the event before is let go.
    const EVENTS: usize = 600;
    for moved in [1_usize, 50] {
        let (time, _) = median(|| {
            let mut keys = Keys(6);
            let targets: Vec<u64> = (0..moved).map(|_| keys.present(size)).collect();
            let mut shown = base.clone();
            let start = Instant::now();
            for event in 0..EVENTS {
                let mut next = base.clone();
                for target in &targets {
                    next.insert(*target, item(event as u64));
                }
                shown = next;
            }
            let time = start.elapsed();
            black_box(&shown);
            (time, 0)
        });
        let what = format!("a drag of {moved}: each of 600 recomputations");
        row(&what, size, time, EVENTS as u32, None);
    }

    for edits in [1_usize, 50] {
        let mut keys = Keys(8);
        let mut edited = base.clone();
        for edit in 0..edits {
            match edit % 8 {
                0 => edited.insert(keys.absent(size), item(1)),
                1 => edited.remove(keys.present(size)),
                _ => edited.insert(keys.present(size), item(1)),
            }
        }
        let (time, _) = median(|| {
            let start = Instant::now();
            let mut found = 0;
            for _ in 0..100 {
                found += C::differences(black_box(&base), black_box(&edited));
            }
            black_box(found);
            (start.elapsed(), 0)
        });
        let what = format!("what differs after {edits}");
        row(&what, size, time, 100, None);
    }
}

fn main() {
    println!("Median of {RUNS} runs. Keys u64, values Arc of 64 bytes.");
    for size in [1_000_u64, 10_000, 100_000] {
        run::<Own<8>>(size);
        run::<Own<16>>(size);
        run::<Own<32>>(size);
        run::<Own<64>>(size);
    }
}

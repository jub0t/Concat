// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The same line said over and over, written a few times and then counted.
//!
//! A compositor that has lost its device says so on every frame it is
//! asked for: one report's log held the same two lines 782 times each,
//! and what happened before them had to be dug out from under. The first
//! ten of a kind are written as they come; after that a line is written
//! once per [`WINDOW`] with how many more there were, and once more when
//! it stops. Two different lines taking turns are counted each on their
//! own, which is what a run of "repeated N times" on consecutive lines
//! alone would miss.

use std::time::{Duration, Instant};

/// How long one count runs: a storm costs a line per kind in each of these.
pub(crate) const WINDOW: Duration = Duration::from_secs(5);

/// How many of the same line are written in a window before the rest are
/// counted. Ten: what a person does over and over - undo, undo, undo - is
/// the story of the run and reads as it happened, and a failure on every
/// frame is past ten in a third of a second.
pub(crate) const BURST: u32 = 10;

/// How many kinds of line are watched at once. A storm is a handful of
/// kinds; past this the least recent is forgotten.
const WATCHED: usize = 48;

/// What to do with a line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Write it.
    Write,
    /// Write it, and say this many more of it came before and went unwritten.
    WriteCounting(u32),
    /// Count it and write nothing.
    Count,
}

/// A line that stopped coming with some of it still uncounted for: to be
/// written once more, with the number.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Owed {
    pub(crate) level: log::Level,
    pub(crate) target: String,
    pub(crate) body: String,
    pub(crate) more: u32,
}

struct Kind {
    key: u64,
    /// When the window being counted opened.
    opened: Instant,
    /// When the line last came.
    last: Instant,
    /// How many came in this window, written or not.
    seen: u32,
    /// The line itself, kept from the first one counted and not written,
    /// so it can be said again with its count.
    owed: Option<(log::Level, String, String)>,
}

impl Kind {
    fn unwritten(&self) -> u32 {
        self.seen.saturating_sub(BURST)
    }
}

/// The lines lately written, by kind.
#[derive(Default)]
pub(crate) struct Repeats {
    kinds: Vec<Kind>,
}

impl Repeats {
    /// What to do with a line of this kind arriving now. `evicted` is what
    /// had to be forgotten to make room, when that still owed a count.
    pub(crate) fn admit(
        &mut self,
        level: log::Level,
        target: &str,
        body: &str,
        now: Instant,
        evicted: &mut Vec<Owed>,
    ) -> Verdict {
        let key = key(level, target, body);
        if let Some(kind) = self.kinds.iter_mut().find(|kind| kind.key == key) {
            kind.last = now;
            if now.duration_since(kind.opened) >= WINDOW {
                let more = kind.unwritten();
                kind.opened = now;
                kind.seen = 1;
                kind.owed = None;
                return if more > 0 {
                    Verdict::WriteCounting(more)
                } else {
                    Verdict::Write
                };
            }
            kind.seen += 1;
            if kind.seen <= BURST {
                return Verdict::Write;
            }
            if kind.owed.is_none() {
                kind.owed = Some((level, target.to_owned(), body.to_owned()));
            }
            return Verdict::Count;
        }
        if self.kinds.len() >= WATCHED
            && let Some(oldest) = (0..self.kinds.len()).min_by_key(|index| self.kinds[*index].last)
        {
            let kind = self.kinds.swap_remove(oldest);
            evicted.extend(owed(kind));
        }
        self.kinds.push(Kind {
            key,
            opened: now,
            last: now,
            seen: 1,
            owed: None,
        });
        Verdict::Write
    }

    /// The counts owed for lines that have stopped coming: every kind whose
    /// window is over, or every kind at all when `all` - the end of a run.
    pub(crate) fn settle(&mut self, now: Instant, all: bool) -> Vec<Owed> {
        let mut settled = Vec::new();
        let mut index = 0;
        while index < self.kinds.len() {
            let over = now.duration_since(self.kinds[index].opened) >= WINDOW;
            if all || over {
                settled.extend(owed(self.kinds.swap_remove(index)));
            } else {
                index += 1;
            }
        }
        settled
    }
}

fn owed(kind: Kind) -> Option<Owed> {
    let more = kind.unwritten();
    let (level, target, body) = kind.owed?;
    (more > 0).then_some(Owed {
        level,
        target,
        body,
        more,
    })
}

/// What makes two lines the same line: how bad, where from, and the words.
/// FNV-1a - a few dozen bytes, hashed while a lock is held, with nobody to
/// defend against.
fn key(level: log::Level, target: &str, body: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    feed(&[level as u8]);
    feed(target.as_bytes());
    feed(&[0]);
    feed(body.as_bytes());
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Level;

    fn admit(repeats: &mut Repeats, body: &str, at: Instant) -> Verdict {
        let mut evicted = Vec::new();
        let verdict = repeats.admit(Level::Error, "concat_render::gpu", body, at, &mut evicted);
        assert!(evicted.is_empty());
        verdict
    }

    #[test]
    fn the_first_few_are_written_and_the_rest_counted() {
        let mut repeats = Repeats::default();
        let start = Instant::now();
        for _ in 0..BURST {
            assert_eq!(admit(&mut repeats, "device lost", start), Verdict::Write);
        }
        for _ in 0..10 {
            assert_eq!(admit(&mut repeats, "device lost", start), Verdict::Count);
        }
        // A different line is its own kind, however the two are shuffled.
        assert_eq!(admit(&mut repeats, "no adapter", start), Verdict::Write);
        assert_eq!(admit(&mut repeats, "device lost", start), Verdict::Count);
    }

    #[test]
    fn a_storm_is_written_once_a_window_with_its_count() {
        let mut repeats = Repeats::default();
        let start = Instant::now();
        for _ in 0..BURST + 7 {
            admit(&mut repeats, "device lost", start);
        }
        assert_eq!(
            admit(&mut repeats, "device lost", start + WINDOW),
            Verdict::WriteCounting(7)
        );
        // The window that opened then starts its own count.
        assert_eq!(
            admit(&mut repeats, "device lost", start + WINDOW),
            Verdict::Write
        );
    }

    #[test]
    fn a_line_that_stops_is_owed_its_count_once() {
        let mut repeats = Repeats::default();
        let start = Instant::now();
        for _ in 0..BURST + 4 {
            admit(&mut repeats, "device lost", start);
        }
        admit(&mut repeats, "said once", start);
        assert!(repeats.settle(start, false).is_empty());
        let owed = repeats.settle(start + WINDOW, false);
        assert_eq!(owed.len(), 1);
        assert_eq!(owed[0].body, "device lost");
        assert_eq!(owed[0].more, 4);
        assert!(repeats.settle(start + WINDOW * 2, true).is_empty());
    }

    #[test]
    fn the_end_of_a_run_settles_what_is_still_being_counted() {
        let mut repeats = Repeats::default();
        let start = Instant::now();
        for _ in 0..BURST + 2 {
            admit(&mut repeats, "device lost", start);
        }
        let owed = repeats.settle(start, true);
        assert_eq!(owed.len(), 1);
        assert_eq!(owed[0].more, 2);
    }

    #[test]
    fn a_kind_pushed_out_takes_its_count_with_it() {
        let mut repeats = Repeats::default();
        let start = Instant::now();
        for _ in 0..BURST + 5 {
            admit(&mut repeats, "device lost", start);
        }
        let mut evicted = Vec::new();
        for index in 0..WATCHED {
            repeats.admit(
                Level::Info,
                "concat",
                &format!("line {index}"),
                start + Duration::from_millis(1),
                &mut evicted,
            );
        }
        assert_eq!(evicted.len(), 1);
        assert_eq!(evicted[0].more, 5);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! An item's three times, and the map from the one it is edited on to
//! the one its source is read at.
//!
//! - **Timeline time** is where the item sits on its lane: from `start`,
//!   for `len`.
//! - **Content time** is the item's own axis, which trimming does not
//!   move. The item shows `[offset, offset + len)` of it. A trim moves
//!   the window over it and a split cuts the window in two; neither
//!   touches the keys or the time map, which are both on this axis, so a
//!   split and a merge give back the item to the tick.
//! - **Source time** is a place in the file or the nested sequence. A
//!   [`TimeMap`] takes content time to it: a constant [`Speed`] from an
//!   in-point, or position keys for speed that changes.
//!
//! Timeline tick `t` of an item is content tick `t - start + offset`.
//!
//! # The range
//!
//! Every tick the document holds, and every place the checks work out
//! from them, lies within [`TICK_LIMIT`] of zero: a little over a
//! century either way. Two of them added still fit in an `i64` four
//! times over, so no sum of document values can overflow, and what is
//! multiplied (a content time by a speed) is worked out with checked
//! arithmetic and refused when it leaves the range.

use crate::time::{Tick, TickDur, TickLen, rescale};

use super::ids::Raw;

/// How far from zero any tick of the document may be: 100 years of 366
/// days, about 2.23 × 10^18 ticks.
pub const TICK_LIMIT: i64 = 100 * 366 * 86_400 * crate::time::TICKS_PER_SECOND;

/// Whether a tick is inside the document's range.
pub fn tick_in_range(t: Tick) -> bool {
    (-TICK_LIMIT..=TICK_LIMIT).contains(&t.get())
}

/// Whether a distance is inside the document's range.
pub fn dur_in_range(d: TickDur) -> bool {
    (-TICK_LIMIT..=TICK_LIMIT).contains(&d.get())
}

/// Whether a length is inside the document's range.
pub fn len_in_range(l: TickLen) -> bool {
    l.get() <= TICK_LIMIT
}

/// The slowest an item may play its source: a sixteenth.
pub const SLOWEST: Speed = Speed { num: 1, den: 16 };

/// The fastest: sixteen times.
pub const FASTEST: Speed = Speed { num: 16, den: 1 };

/// Source ticks per content tick, as a whole number over a whole number:
/// above zero forward, below zero in reverse. Never zero, and always in
/// lowest terms, so two speeds are the same speed exactly when they are
/// equal.
///
/// A speed is a pace; whether it is one the document allows (from a
/// sixteenth to sixteen times, either way) is checked at commit.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Speed {
    num: i32,
    den: u32,
}

impl Speed {
    /// The source at its own pace.
    pub const NORMAL: Speed = Speed { num: 1, den: 1 };

    /// `num / den`, in lowest terms. `None` when either is zero.
    pub const fn new(num: i32, den: u32) -> Option<Speed> {
        if num == 0 || den == 0 {
            return None;
        }
        let mut a = num.unsigned_abs();
        let mut b = den;
        while b != 0 {
            let rest = a % b;
            a = b;
            b = rest;
        }
        // `a` divides `num`, so the quotient fits; it is taken in 64 bits
        // because `a` may be 2^31, which an `i32` cannot hold.
        Some(Speed {
            num: (num as i64 / a as i64) as i32,
            den: den / a,
        })
    }

    /// Source ticks, signed.
    pub const fn num(self) -> i32 {
        self.num
    }

    /// Content ticks; above zero.
    pub const fn den(self) -> u32 {
        self.den
    }

    /// Whether the source plays backwards.
    pub const fn is_reverse(self) -> bool {
        self.num < 0
    }

    /// Whether its size, either way, is from [`SLOWEST`] to [`FASTEST`].
    pub fn is_allowed(self) -> bool {
        let num = i64::from(self.num.unsigned_abs());
        let den = i64::from(self.den);
        num * 16 >= den && num <= den * 16
    }

    /// How far the source moves over a content distance: `d * num / den`,
    /// rounded down. `None` where it does not fit.
    pub fn of(self, d: TickDur) -> Option<TickDur> {
        d.scaled(i64::from(self.num), i64::from(self.den))
    }

    /// The speed as a fraction, for showing to a person.
    pub fn display(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }
}

/// How content time becomes source time.
#[derive(Clone, PartialEq, Debug)]
pub enum TimeMap {
    /// Content tick `c` is source tick `src_in + c * speed`.
    Linear {
        /// The source tick at content tick zero.
        src_in: Tick,
        /// Source ticks per content tick.
        speed: Speed,
    },
    /// Position keys: at each content tick, the source tick it shows,
    /// and between two keys the curve the second names. The keys run
    /// forward in content time and the source either rises all the way
    /// or falls all the way: no key repeats a source tick, and no stretch
    /// plays faster than sixteen times or slower than a sixteenth.
    Keyed {
        /// The first key.
        first: TimePoint,
        /// The rest, each with how it is reached from the one before.
        /// Never empty: a map has at least two keys.
        rest: Vec<(TimeCurve, TimePoint)>,
    },
    /// A kind of map this build does not know.
    Unknown(Raw),
}

/// A position key: this content tick shows this source tick.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimePoint {
    /// On the item's content axis.
    pub at: Tick,
    /// In the source.
    pub source: Tick,
}

/// How the source moves between two position keys.
#[derive(Clone, PartialEq, Debug)]
pub enum TimeCurve {
    /// At one speed: the straight line between the two.
    Linear,
    /// Easing from one speed into another: the cubic through the two
    /// keys that leaves the first at `leave` and reaches the second at
    /// `arrive`. A speed that changes in a straight line - a ramp - is
    /// one of these exactly, with each speed the ramp's at that key.
    Smooth {
        /// Source ticks per content tick as it leaves the earlier key.
        leave: Speed,
        /// The same as it reaches the later key.
        arrive: Speed,
    },
    /// A curve this build does not know.
    Unknown(Raw),
}

impl TimeMap {
    /// The source at its own pace from `src_in`.
    pub const fn from_in(src_in: Tick) -> TimeMap {
        TimeMap::Linear {
            src_in,
            speed: Speed::NORMAL,
        }
    }

    /// Every key, first to last; none for a linear map.
    pub fn points(&self) -> Vec<TimePoint> {
        match self {
            TimeMap::Keyed { first, rest } => std::iter::once(*first)
                .chain(rest.iter().map(|(_, point)| *point))
                .collect(),
            TimeMap::Linear { .. } | TimeMap::Unknown(_) => Vec::new(),
        }
    }

    /// The source tick at a content tick, where it is known exactly: on a
    /// linear map anywhere, on a keyed one at a key or on a straight
    /// stretch between two. `None` past the keys, inside a smooth stretch,
    /// for a map this build does not know, and where the answer does not
    /// fit.
    pub fn source_at(&self, c: Tick) -> Option<Tick> {
        match self {
            TimeMap::Linear { src_in, speed } => {
                src_in.checked_add(speed.of(TickDur::new(c.get()))?)
            }
            TimeMap::Keyed { first, rest } => {
                let mut from = *first;
                if c == from.at {
                    return Some(from.source);
                }
                for (curve, to) in rest {
                    if c <= to.at && c >= from.at {
                        if c == to.at {
                            return Some(to.source);
                        }
                        return match curve {
                            TimeCurve::Linear => {
                                let run = to.at.checked_since(from.at)?.get();
                                let rise = to.source.checked_since(from.source)?.get();
                                let into = c.checked_since(from.at)?.get();
                                from.source
                                    .checked_add(TickDur::new(rescale(into, rise, run)?))
                            }
                            TimeCurve::Smooth { .. } | TimeCurve::Unknown(_) => None,
                        };
                    }
                    from = *to;
                }
                None
            }
            TimeMap::Unknown(_) => None,
        }
    }
}

impl Default for TimeMap {
    fn default() -> TimeMap {
        TimeMap::from_in(Tick::ZERO)
    }
}

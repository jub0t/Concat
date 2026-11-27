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
//!   in-point, or position keys for speed that changes. Keys worked out
//!   from a speed graph keep the graph beside them, as it was drawn
//!   ([`SpeedGraph`]): the keys are what plays, and the graph is what the
//!   speed pane shows.
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
        /// The speed graph the keys were worked out from, as the person
        /// drew it; none for keys that came from anywhere else. The keys
        /// are what plays: the graph is what the speed pane shows.
        drawn: Option<SpeedGraph>,
    },
    /// A kind of map this build does not know.
    Unknown(Raw),
}

/// One point of a speed graph, as the speed pane draws it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpeedPoint {
    /// Where, as a share of the span the graph is drawn over: 0 at its
    /// start, 1 at its end.
    pub at: f64,
    /// Source per timeline there: its size, whichever way the source
    /// runs.
    pub speed: f64,
    /// How fast the speed changes there, per unit of `at`. When every
    /// point has one the curve bends through them; otherwise it runs
    /// straight between them.
    pub slope: Option<f64>,
}

/// A speed graph as a person drew it: the points, and the span of
/// content time they were drawn over.
///
/// The span is the window the item showed when the graph was set. A trim
/// or a split moves the window and leaves the span, so the points keep
/// their places on content time, as the keys worked out from them do.
#[derive(Clone, PartialEq, Debug)]
pub struct SpeedGraph {
    /// Where on content time the span starts.
    pub from: Tick,
    /// How long the span is.
    pub len: TickLen,
    /// The points, in order of `at`. Two at one share are a step.
    pub points: Vec<SpeedPoint>,
}

/// What is wrong with a speed graph.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GraphFault {
    /// Its span starts or ends outside the document's range.
    OutOfRange,
    /// It has no points.
    Empty,
    /// One of its numbers is not a finite number.
    NotFinite,
    /// A point is outside the span: its share is below 0 or above 1.
    OutOfBounds,
    /// Its points are not in order of their shares.
    OutOfOrder,
    /// A speed is slower than a sixteenth or faster than sixteen times.
    SpeedNotAllowed,
}

impl SpeedGraph {
    /// What is wrong with it, if anything: the first fault found.
    pub fn fault(&self) -> Option<GraphFault> {
        let ends = self.from.checked_add(self.len).is_some_and(tick_in_range);
        if !tick_in_range(self.from) || !len_in_range(self.len) || !ends {
            return Some(GraphFault::OutOfRange);
        }
        if self.points.is_empty() {
            return Some(GraphFault::Empty);
        }
        let finite = |p: &SpeedPoint| {
            p.at.is_finite() && p.speed.is_finite() && p.slope.is_none_or(f64::is_finite)
        };
        if !self.points.iter().all(finite) {
            return Some(GraphFault::NotFinite);
        }
        if self.points.iter().any(|p| !(0.0..=1.0).contains(&p.at)) {
            return Some(GraphFault::OutOfBounds);
        }
        if self.points.windows(2).any(|pair| pair[0].at > pair[1].at) {
            return Some(GraphFault::OutOfOrder);
        }
        let (low, high) = (SLOWEST.display(), FASTEST.display());
        if self.points.iter().any(|p| !(low..=high).contains(&p.speed)) {
            return Some(GraphFault::SpeedNotAllowed);
        }
        None
    }
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
            TimeMap::Keyed { first, rest, .. } => std::iter::once(*first)
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
            TimeMap::Keyed { first, rest, .. } => {
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

    /// The source at content tick `c`, on a smooth stretch too: there the
    /// source is the cubic through the two keys with the speeds leaving
    /// and arriving as its slopes (a Hermite curve), rounded to a tick.
    /// `None` past a keyed map's keys and for a map this build cannot
    /// read.
    pub fn source_of(&self, c: Tick) -> Option<Tick> {
        if let Some(exact) = self.source_at(c) {
            return Some(exact);
        }
        let (from, to, leave, arrive) = self.smooth_stretch(c)?;
        let w = (to.at.get() - from.at.get()) as f64;
        let u = (c.get() - from.at.get()) as f64 / w;
        let (s0, s1) = (from.source.get() as f64, to.source.get() as f64);
        let (m0, m1) = (leave * w, arrive * w);
        let (u2, u3) = (u * u, u * u * u);
        let p = (2.0 * u3 - 3.0 * u2 + 1.0) * s0
            + (u3 - 2.0 * u2 + u) * m0
            + (-2.0 * u3 + 3.0 * u2) * s1
            + (u3 - u2) * m1;
        Some(Tick::new(p.round() as i64))
    }

    /// The pace at content tick `c`: source ticks per content tick,
    /// signed, so below zero runs the source backwards. On a keyed map's
    /// key the pace leaving it. `None` where [`TimeMap::source_of`] is.
    pub fn pace_at(&self, c: Tick) -> Option<f64> {
        match self {
            TimeMap::Linear { speed, .. } => Some(speed.display()),
            TimeMap::Keyed { first, rest, .. } => {
                let mut from = *first;
                for (curve, to) in rest {
                    if c >= from.at && c < to.at {
                        let w = (to.at.get() - from.at.get()) as f64;
                        let rise = (to.source.get() - from.source.get()) as f64;
                        return match curve {
                            TimeCurve::Linear => Some(rise / w),
                            TimeCurve::Smooth { leave, arrive } => {
                                let u = (c.get() - from.at.get()) as f64 / w;
                                let (m0, m1) = (leave.display() * w, arrive.display() * w);
                                // The Hermite curve's slope, per share of
                                // the stretch, over the stretch's width.
                                let d = (6.0 * u * u - 6.0 * u) * from.source.get() as f64
                                    + (3.0 * u * u - 4.0 * u + 1.0) * m0
                                    + (-6.0 * u * u + 6.0 * u) * to.source.get() as f64
                                    + (3.0 * u * u - 2.0 * u) * m1;
                                Some(d / w)
                            }
                            TimeCurve::Unknown(_) => None,
                        };
                    }
                    from = *to;
                }
                // At the last key, or past it: the last stretch's pace
                // arriving, if `c` is the last key.
                if c == from.at {
                    return match rest.last() {
                        Some((TimeCurve::Smooth { arrive, .. }, _)) => Some(arrive.display()),
                        Some((TimeCurve::Linear, to)) => {
                            let before = rest.len().checked_sub(2).map_or(*first, |at| rest[at].1);
                            let w = (to.at.get() - before.at.get()) as f64;
                            Some((to.source.get() - before.source.get()) as f64 / w)
                        }
                        _ => None,
                    };
                }
                None
            }
            TimeMap::Unknown(_) => None,
        }
    }

    /// The smooth stretch `c` is on, with its speeds leaving and arriving.
    fn smooth_stretch(&self, c: Tick) -> Option<(TimePoint, TimePoint, f64, f64)> {
        let TimeMap::Keyed { first, rest, .. } = self else {
            return None;
        };
        let mut from = *first;
        for (curve, to) in rest {
            if c >= from.at && c <= to.at {
                return match curve {
                    TimeCurve::Smooth { leave, arrive } => {
                        Some((from, *to, leave.display(), arrive.display()))
                    }
                    _ => None,
                };
            }
            from = *to;
        }
        None
    }
}

impl Default for TimeMap {
    fn default() -> TimeMap {
        TimeMap::from_in(Tick::ZERO)
    }
}

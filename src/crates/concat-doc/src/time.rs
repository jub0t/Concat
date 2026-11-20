// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Time as whole ticks, and the frame grid as a function of a rate.
//!
//! A time in seconds, held as a fraction, cannot be compared for
//! equality, drifts when lengths are added up, and lands a cut on the
//! wrong side of a frame edge by the last digit. Here a time is a whole
//! number of [`Tick`]s, 705,600,000 to the second. That number is chosen
//! so that the things an editor places are whole counts of it: one frame
//! at 24, 25, 30, 50, 60 and 120 a second, one frame at each of those
//! slowed by 1000/1001, and one sample at 44.1, 48, 96 and 192 kHz. A
//! count of them stays exact in a reader that holds every number as a
//! double for 147 days of timeline.
//!
//! Three rules hold everywhere in this module:
//!
//! - **A place and a length are different types.** A [`Tick`] is where
//!   something is, a [`TickDur`] is how far apart two places are, and a
//!   [`TickLen`] is a length that is known to be more than nothing. The
//!   arithmetic between them is the arithmetic that means something: two
//!   places cannot be added, and a length cannot be handed over where a
//!   place is wanted.
//! - **Nothing wraps.** The operators panic when the answer does not fit,
//!   as the standard library's own times do, and every one of them has a
//!   `checked_` form for a number that came from outside.
//! - **One rounding rule.** Every multiply-then-divide goes through
//!   [`rescale`], which works in 128 bits and rounds toward negative
//!   infinity. A day of ticks times a rate's numerator does not fit in 64
//!   bits, and a rule that rounded toward zero would put two frames at
//!   tick zero's edge, one from each side.
//!
//! # The grid
//!
//! A [`Rate`] is a whole number of frames over a whole number of seconds,
//! always in lowest terms. Frame `k` of a rate starts at
//!
//! ```text
//! floor(k * 705,600,000 * den / num)
//! ```
//!
//! ticks ([`Rate::frame_start`]), and the frame showing at a tick is the
//! last one that has started ([`Rate::frame_at`]). For every rate with a
//! name here that division is exact, so a frame's edge is a tick and not
//! near one. A rate the base does not divide, such as 2997/100, still has
//! a grid by the same rule; its frames are then not all one length, which
//! [`Rate::frame_len`] says by answering `None`.
//!
//! A sample rate is a rate like any other, and its frames are samples:
//! [`Rate::hz`].

use std::fmt;
use std::num::NonZeroI64;
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

/// How many ticks there are in a second.
pub const TICKS_PER_SECOND: i64 = 705_600_000;

/// `value * num / den`, rounded toward negative infinity.
///
/// The one place a time is multiplied and then divided. The product is
/// taken in 128 bits, where two 64-bit numbers always fit, so only the
/// answer can be too large, and then this says `None` and does not wrap.
/// `den` must be more than zero; a `num` below zero is how a reversed
/// clip's time is worked out.
///
/// Rounding down, and not toward zero, is what keeps a grid even across
/// tick zero: `-1` of something a third as fast is `-1`, the cell before
/// zero, and not `0`.
///
/// ```
/// use concat_doc::rescale;
///
/// assert_eq!(rescale(7, 1, 2), Some(3));
/// assert_eq!(rescale(-7, 1, 2), Some(-4));
/// assert_eq!(rescale(i64::MAX, 4, 2), None);
/// assert_eq!(rescale(1, 1, 0), None);
/// ```
pub fn rescale(value: i64, num: i64, den: i64) -> Option<i64> {
    if den <= 0 {
        return None;
    }
    let product = i128::from(value) * i128::from(num);
    // With a divisor above zero, Euclid's quotient is the floor.
    i64::try_from(product.div_euclid(i128::from(den))).ok()
}

/// The same, rounded toward positive infinity: the floor of the negated
/// value, negated back. Private, so that [`rescale`] stays the one rule.
fn rescale_up(value: i64, num: i64, den: i64) -> Option<i64> {
    rescale(value.checked_neg()?, num, den)?.checked_neg()
}

/// A place in time: a whole number of ticks from zero.
///
/// In a file it is a plain whole number.
///
/// ```
/// use concat_doc::{Tick, TickDur};
///
/// let start = Tick::new(1_000);
/// let end = start + TickDur::new(500);
/// assert_eq!(end - start, TickDur::new(500));
/// assert_eq!(Tick::MAX.checked_add(TickDur::new(1)), None);
/// ```
///
/// Two places cannot be added:
///
/// ```compile_fail
/// use concat_doc::Tick;
/// let nonsense = Tick::new(1) + Tick::new(2);
/// ```
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Tick(i64);

/// How far one place is from another, in ticks. It may be below zero: a
/// move to the left is one.
///
/// In a file it is a plain whole number.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TickDur(i64);

/// A length that is more than nothing.
///
/// What a clip's length is: it cannot be made from zero or from a number
/// below it, and reading one of those from a file is an error, so "a clip
/// of no length" is not a state any code has to look out for.
///
/// ```
/// use concat_doc::{Tick, TickDur, TickLen};
///
/// assert!(TickLen::new(0).is_none());
/// assert!(TickLen::new(-5).is_none());
/// let len = TickLen::new(5).unwrap();
/// assert_eq!(Tick::new(10) + len, Tick::new(15));
/// assert_eq!(TickDur::from(len), TickDur::new(5));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "i64")]
pub struct TickLen(NonZeroI64);

impl Tick {
    /// Tick zero.
    pub const ZERO: Tick = Tick(0);
    /// The earliest place there is.
    pub const MIN: Tick = Tick(i64::MIN);
    /// The latest place there is.
    pub const MAX: Tick = Tick(i64::MAX);

    /// The place this many ticks from zero.
    pub const fn new(ticks: i64) -> Tick {
        Tick(ticks)
    }

    /// How many ticks from zero this is.
    pub const fn get(self) -> i64 {
        self.0
    }

    /// The place this many whole seconds from zero, exactly.
    pub const fn from_whole_seconds(seconds: i64) -> Option<Tick> {
        match seconds.checked_mul(TICKS_PER_SECOND) {
            Some(ticks) => Some(Tick(ticks)),
            None => None,
        }
    }

    /// This place moved by a length; `None` where the answer does not fit.
    pub fn checked_add(self, by: impl Into<TickDur>) -> Option<Tick> {
        self.0.checked_add(by.into().0).map(Tick)
    }

    /// This place moved back by a length; `None` where the answer does
    /// not fit.
    pub fn checked_sub(self, by: impl Into<TickDur>) -> Option<Tick> {
        self.0.checked_sub(by.into().0).map(Tick)
    }

    /// How far this place is after `earlier`, below zero when it is
    /// before; `None` where the answer does not fit.
    pub const fn checked_since(self, earlier: Tick) -> Option<TickDur> {
        match self.0.checked_sub(earlier.0) {
            Some(ticks) => Some(TickDur(ticks)),
            None => None,
        }
    }

    /// Seconds, for showing to a person and for nothing else.
    ///
    /// The answer is the nearest fraction and no longer the place itself.
    /// Arithmetic is done in ticks; a number that came out of here must
    /// never be worked on and put back.
    pub fn display_seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    /// The tick nearest to a number of seconds that a person typed.
    ///
    /// The way in for "go to 12.5 s", and for nothing that was a tick
    /// before: a place that goes out through [`Tick::display_seconds`]
    /// and back through here is not promised to be the same place.
    /// `None` for what is not a number or does not fit.
    pub fn from_display_seconds(seconds: f64) -> Option<Tick> {
        ticks_nearest(seconds).map(Tick)
    }

    /// Which sample is playing at this place, at so many samples a
    /// second. The same grid as a frame rate's: see [`Rate::frame_at`].
    pub fn sample_at(self, hz: u32) -> Option<i64> {
        Rate::hz(hz).ok()?.frame_at(self)
    }

    /// Where a sample starts, at so many samples a second. See
    /// [`Rate::frame_start`].
    pub fn of_sample(index: i64, hz: u32) -> Option<Tick> {
        Rate::hz(hz).ok()?.frame_start(index)
    }
}

impl TickDur {
    /// No distance at all.
    pub const ZERO: TickDur = TickDur(0);
    /// One second.
    pub const SECOND: TickDur = TickDur(TICKS_PER_SECOND);

    /// A distance of this many ticks.
    pub const fn new(ticks: i64) -> TickDur {
        TickDur(ticks)
    }

    /// How many ticks this is.
    pub const fn get(self) -> i64 {
        self.0
    }

    /// This many whole seconds, exactly.
    pub const fn from_whole_seconds(seconds: i64) -> Option<TickDur> {
        match seconds.checked_mul(TICKS_PER_SECOND) {
            Some(ticks) => Some(TickDur(ticks)),
            None => None,
        }
    }

    /// This many thousandths of a second, exactly.
    pub const fn from_millis(millis: i64) -> Option<TickDur> {
        match millis.checked_mul(TICKS_PER_SECOND / 1000) {
            Some(ticks) => Some(TickDur(ticks)),
            None => None,
        }
    }

    /// Whether this is more than nothing.
    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    /// This as a length, if it is more than nothing.
    pub const fn to_len(self) -> Option<TickLen> {
        TickLen::new(self.0)
    }

    /// The two added; `None` where the answer does not fit.
    pub const fn checked_add(self, other: TickDur) -> Option<TickDur> {
        match self.0.checked_add(other.0) {
            Some(ticks) => Some(TickDur(ticks)),
            None => None,
        }
    }

    /// The difference; `None` where the answer does not fit.
    pub const fn checked_sub(self, other: TickDur) -> Option<TickDur> {
        match self.0.checked_sub(other.0) {
            Some(ticks) => Some(TickDur(ticks)),
            None => None,
        }
    }

    /// So many times this; `None` where the answer does not fit.
    pub const fn checked_mul(self, times: i64) -> Option<TickDur> {
        match self.0.checked_mul(times) {
            Some(ticks) => Some(TickDur(ticks)),
            None => None,
        }
    }

    /// The same distance the other way; `None` for the one distance that
    /// has no opposite.
    pub const fn checked_neg(self) -> Option<TickDur> {
        match self.0.checked_neg() {
            Some(ticks) => Some(TickDur(ticks)),
            None => None,
        }
    }

    /// This distance at another speed: `self * num / den`, rounded down
    /// by [`rescale`].
    pub fn scaled(self, num: i64, den: i64) -> Option<TickDur> {
        rescale(self.0, num, den).map(TickDur)
    }

    /// Seconds, for showing to a person and for nothing else. See
    /// [`Tick::display_seconds`].
    pub fn display_seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    /// The distance nearest to a number of seconds that a person typed.
    /// See [`Tick::from_display_seconds`].
    pub fn from_display_seconds(seconds: f64) -> Option<TickDur> {
        ticks_nearest(seconds).map(TickDur)
    }

    /// How many whole samples fit in this, at so many samples a second.
    /// See [`Rate::frames_in`].
    pub fn samples(self, hz: u32) -> Option<i64> {
        Rate::hz(hz).ok()?.frames_in(self)
    }
}

impl TickLen {
    /// One tick, the shortest length there is.
    pub const ONE: TickLen = match TickLen::new(1) {
        Some(one) => one,
        // Worked out when the crate is built, and one is above zero.
        None => panic!("one tick is a length"),
    };

    /// A length of this many ticks, if that is more than none.
    pub const fn new(ticks: i64) -> Option<TickLen> {
        if ticks <= 0 {
            return None;
        }
        match NonZeroI64::new(ticks) {
            Some(ticks) => Some(TickLen(ticks)),
            None => None,
        }
    }

    /// How many ticks long this is; always more than zero.
    pub const fn get(self) -> i64 {
        self.0.get()
    }

    /// The same length as a distance.
    pub const fn dur(self) -> TickDur {
        TickDur(self.0.get())
    }

    /// The two end to end; `None` where the answer does not fit.
    pub const fn checked_add(self, other: TickLen) -> Option<TickLen> {
        match self.get().checked_add(other.get()) {
            Some(ticks) => TickLen::new(ticks),
            None => None,
        }
    }
}

/// The whole number of ticks nearest to a number of seconds.
fn ticks_nearest(seconds: f64) -> Option<i64> {
    let ticks = (seconds * TICKS_PER_SECOND as f64).round();
    // 2^63 as a double is exact, and the first number that does not fit.
    let limit = 9_223_372_036_854_775_808.0_f64;
    if ticks.is_nan() || ticks >= limit || ticks < -limit {
        return None;
    }
    Some(ticks as i64)
}

/// What the operators say when an answer does not fit. They do not wrap:
/// a wrapped time is a wrong time that looks like a right one.
const OVERFLOW: &str = "tick arithmetic overflowed; use the checked form for numbers from outside";

impl From<TickLen> for TickDur {
    fn from(len: TickLen) -> TickDur {
        len.dur()
    }
}

impl TryFrom<TickDur> for TickLen {
    type Error = NotALength;

    fn try_from(dur: TickDur) -> Result<TickLen, NotALength> {
        dur.to_len().ok_or(NotALength(dur.0))
    }
}

impl TryFrom<i64> for TickLen {
    type Error = NotALength;

    fn try_from(ticks: i64) -> Result<TickLen, NotALength> {
        TickLen::new(ticks).ok_or(NotALength(ticks))
    }
}

impl From<TickLen> for i64 {
    fn from(len: TickLen) -> i64 {
        len.get()
    }
}

/// A length was wanted and the number was zero or below it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NotALength(pub i64);

impl fmt::Display for NotALength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a length must be more than zero ticks, and this is {}",
            self.0
        )
    }
}

impl std::error::Error for NotALength {}

impl<D: Into<TickDur>> Add<D> for Tick {
    type Output = Tick;

    /// Panics where the answer does not fit; see [`Tick::checked_add`].
    fn add(self, by: D) -> Tick {
        self.checked_add(by).expect(OVERFLOW)
    }
}

impl<D: Into<TickDur>> AddAssign<D> for Tick {
    fn add_assign(&mut self, by: D) {
        *self = *self + by;
    }
}

impl Sub<TickDur> for Tick {
    type Output = Tick;

    /// Panics where the answer does not fit; see [`Tick::checked_sub`].
    fn sub(self, by: TickDur) -> Tick {
        self.checked_sub(by).expect(OVERFLOW)
    }
}

impl Sub<TickLen> for Tick {
    type Output = Tick;

    /// Panics where the answer does not fit; see [`Tick::checked_sub`].
    fn sub(self, by: TickLen) -> Tick {
        self.checked_sub(by).expect(OVERFLOW)
    }
}

impl SubAssign<TickDur> for Tick {
    fn sub_assign(&mut self, by: TickDur) {
        *self = *self - by;
    }
}

impl Sub<Tick> for Tick {
    type Output = TickDur;

    /// Panics where the answer does not fit; see [`Tick::checked_since`].
    fn sub(self, earlier: Tick) -> TickDur {
        self.checked_since(earlier).expect(OVERFLOW)
    }
}

impl Add for TickDur {
    type Output = TickDur;

    /// Panics where the answer does not fit; see [`TickDur::checked_add`].
    fn add(self, other: TickDur) -> TickDur {
        self.checked_add(other).expect(OVERFLOW)
    }
}

impl Sub for TickDur {
    type Output = TickDur;

    /// Panics where the answer does not fit; see [`TickDur::checked_sub`].
    fn sub(self, other: TickDur) -> TickDur {
        self.checked_sub(other).expect(OVERFLOW)
    }
}

impl Neg for TickDur {
    type Output = TickDur;

    /// Panics for the one distance with no opposite; see
    /// [`TickDur::checked_neg`].
    fn neg(self) -> TickDur {
        self.checked_neg().expect(OVERFLOW)
    }
}

impl Mul<i64> for TickDur {
    type Output = TickDur;

    /// Panics where the answer does not fit; see [`TickDur::checked_mul`].
    fn mul(self, times: i64) -> TickDur {
        self.checked_mul(times).expect(OVERFLOW)
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for TickDur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for TickLen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Which way a place is moved onto a grid.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Snap {
    /// To the edge at or before it: the start of the frame it is in.
    Floor,
    /// To the nearer edge; halfway between two, to the later.
    Nearest,
    /// To the edge at or after it.
    Ceil,
}

/// Frames a second, or samples a second, as a whole number over a whole
/// number, always in lowest terms.
///
/// The two numbers cannot be set from outside, so neither is ever zero
/// and 60/2 is never anything but 30/1: two rates are the same rate
/// exactly when they are equal.
///
/// ```
/// use concat_doc::{Rate, Tick};
///
/// assert_eq!(Rate::new(60_000, 2_002).unwrap(), Rate::FPS_29_97);
/// assert_eq!(Rate::from_decimal(29.97).unwrap(), Rate::FPS_29_97);
/// assert_eq!(Rate::FPS_29_97.frame_start(1), Some(Tick::new(23_543_520)));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RateOnDisk", into = "RateOnDisk")]
pub struct Rate {
    num: u32,
    den: u32,
}

/// A rate as a file spells it: `{"num": 30000, "den": 1001}`.
#[derive(Serialize, Deserialize)]
struct RateOnDisk {
    num: u32,
    den: u32,
}

impl TryFrom<RateOnDisk> for Rate {
    type Error = RateError;

    fn try_from(disk: RateOnDisk) -> Result<Rate, RateError> {
        Rate::new(disk.num, disk.den)
    }
}

impl From<Rate> for RateOnDisk {
    fn from(rate: Rate) -> RateOnDisk {
        RateOnDisk {
            num: rate.num,
            den: rate.den,
        }
    }
}

/// Why a number is not a rate.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RateError {
    /// One of the two numbers was zero.
    Zero,
    /// Not a number above zero.
    NotANumber,
    /// More frames a second than a rate can hold.
    TooLarge,
    /// Not one of the known rates, and finer than a thousandth of a
    /// frame a second: not something a person or a file means by a rate.
    TooFine,
}

impl fmt::Display for RateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RateError::Zero => "a rate has no zero in it",
            RateError::NotANumber => "a rate is a number above zero",
            RateError::TooLarge => "the rate is too large to hold",
            RateError::TooFine => "the rate is finer than a thousandth and not a known rate",
        })
    }
}

impl std::error::Error for RateError {}

const fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let rest = a % b;
        a = b;
        b = rest;
    }
    a
}

impl Rate {
    /// 24 frames a second.
    pub const FPS_24: Rate = Rate { num: 24, den: 1 };
    /// 25 frames a second.
    pub const FPS_25: Rate = Rate { num: 25, den: 1 };
    /// 30 frames a second.
    pub const FPS_30: Rate = Rate { num: 30, den: 1 };
    /// 50 frames a second.
    pub const FPS_50: Rate = Rate { num: 50, den: 1 };
    /// 60 frames a second.
    pub const FPS_60: Rate = Rate { num: 60, den: 1 };
    /// 120 frames a second.
    pub const FPS_120: Rate = Rate { num: 120, den: 1 };
    /// 24000/1001 frames a second, which is written 23.976.
    pub const FPS_23_976: Rate = Rate {
        num: 24_000,
        den: 1001,
    };
    /// 30000/1001 frames a second, which is written 29.97.
    pub const FPS_29_97: Rate = Rate {
        num: 30_000,
        den: 1001,
    };
    /// 60000/1001 frames a second, which is written 59.94.
    pub const FPS_59_94: Rate = Rate {
        num: 60_000,
        den: 1001,
    };
    /// 120000/1001 frames a second, which is written 119.88.
    pub const FPS_119_88: Rate = Rate {
        num: 120_000,
        den: 1001,
    };
    /// 44,100 samples a second.
    pub const HZ_44_100: Rate = Rate {
        num: 44_100,
        den: 1,
    };
    /// 48,000 samples a second.
    pub const HZ_48_000: Rate = Rate {
        num: 48_000,
        den: 1,
    };

    /// `num` frames in `den` seconds, brought to lowest terms. Refused
    /// when either is zero.
    pub const fn new(num: u32, den: u32) -> Result<Rate, RateError> {
        if num == 0 || den == 0 {
            return Err(RateError::Zero);
        }
        let shared = gcd(num, den);
        Ok(Rate {
            num: num / shared,
            den: den / shared,
        })
    }

    /// A whole number of samples, or of frames, a second.
    pub const fn hz(per_second: u32) -> Result<Rate, RateError> {
        Rate::new(per_second, 1)
    }

    /// The rate a decimal number means.
    ///
    /// Nobody means 2997/100 by 29.97: they mean 30000/1001, and over two
    /// hours the two are frames apart. So the spellings in use for the
    /// slowed rates - 23.976 and 23.98, 29.97, 47.952 and 47.95, 59.94,
    /// 119.88, 239.76, and any closer writing of the same fractions -
    /// become those fractions. Any other number is taken at its word to
    /// the thousandth: 12.5 is 25/2. A number that is not above zero, is
    /// too large, or is finer than a thousandth without being one of the
    /// known rates is refused, since nothing sensible wrote it.
    pub fn from_decimal(per_second: f64) -> Result<Rate, RateError> {
        if !per_second.is_finite() || per_second <= 0.0 {
            return Err(RateError::NotANumber);
        }
        for whole in [24_u32, 30, 48, 60, 120, 240] {
            let slowed = f64::from(whole) * 1000.0 / 1001.0;
            // To the thousandth this is the fraction itself; the two
            // rounded to a hundredth are the spellings below.
            if (per_second - slowed).abs() < 0.001 {
                return Rate::new(whole * 1000, 1001);
            }
        }
        for (spelling, whole) in [(23.98, 24_u32), (47.95, 48)] {
            if (per_second - spelling).abs() < 1e-9 {
                return Rate::new(whole * 1000, 1001);
            }
        }
        let thousandths = per_second * 1000.0;
        let whole = thousandths.round();
        if whole > f64::from(u32::MAX) {
            return Err(RateError::TooLarge);
        }
        if (thousandths - whole).abs() > 1e-6 || whole < 1.0 {
            return Err(RateError::TooFine);
        }
        Rate::new(whole as u32, 1000)
    }

    /// Frames, in lowest terms; never zero.
    pub const fn num(self) -> u32 {
        self.num
    }

    /// Seconds, in lowest terms; never zero.
    pub const fn den(self) -> u32 {
        self.den
    }

    /// Frames a second as a fraction, for showing to a person and for
    /// nothing else.
    pub fn display_per_second(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }

    /// Ticks in `den` seconds: what a frame number is multiplied by
    /// before it is divided by `num`. At most 2^32 seconds of ticks,
    /// which fits with room to spare.
    const fn ticks_in_den(self) -> i64 {
        TICKS_PER_SECOND * self.den as i64
    }

    /// How long every frame is, if they are all one length: that is, if
    /// a frame of this rate is a whole number of ticks. `None` for a rate
    /// the tick does not divide, whose frames differ by a tick.
    pub const fn frame_len(self) -> Option<TickLen> {
        let ticks = self.ticks_in_den();
        if ticks % self.num as i64 != 0 {
            return None;
        }
        TickLen::new(ticks / self.num as i64)
    }

    /// Where frame `k` starts: `floor(k * 705,600,000 * den / num)`.
    ///
    /// The grid is defined by this and by nothing stored: frame `k` runs
    /// from here to where frame `k + 1` starts. `None` where the answer
    /// does not fit.
    pub fn frame_start(self, k: i64) -> Option<Tick> {
        rescale(k, self.ticks_in_den(), i64::from(self.num)).map(Tick)
    }

    /// Which frame is showing at a place: the last one that has started
    /// at or before it.
    ///
    /// The largest `k` whose start is at or before `t`. A start is a
    /// floor, so that is the largest `k` with
    /// `k * ticks < (t + 1) * num`, one less than that quotient rounded
    /// up. On a grid the tick divides this is plainly
    /// `floor(t * num / ticks)`; said this way it is also right on a grid
    /// the tick does not divide, where the plain form can name a frame
    /// that starts a tick later. `None` where the answer does not fit.
    pub fn frame_at(self, t: Tick) -> Option<i64> {
        let after = t.0.checked_add(1)?;
        rescale_up(after, i64::from(self.num), self.ticks_in_den())?.checked_sub(1)
    }

    /// A place moved onto this rate's grid.
    ///
    /// `None` where the answer does not fit.
    pub fn snap(self, t: Tick, to: Snap) -> Option<Tick> {
        let frame = self.frame_at(t)?;
        let before = self.frame_start(frame)?;
        if before == t {
            return Some(t);
        }
        let after = self.frame_start(frame.checked_add(1)?)?;
        Some(match to {
            Snap::Floor => before,
            Snap::Ceil => after,
            Snap::Nearest => {
                if t.checked_since(before)? < after.checked_since(t)? {
                    before
                } else {
                    after
                }
            }
        })
    }

    /// How long `frames` frames are, counted from frame zero: where frame
    /// `frames` starts, as a length. `None` where the answer does not
    /// fit.
    pub fn span(self, frames: i64) -> Option<TickDur> {
        self.frame_start(frames).map(|at| TickDur(at.0))
    }

    /// How many whole frames, counted from frame zero, fit in a length.
    /// The other way round from [`Rate::span`]. `None` where the answer
    /// does not fit.
    pub fn frames_in(self, dur: TickDur) -> Option<i64> {
        self.frame_at(Tick(dur.0))
    }
}

impl fmt::Display for Rate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

impl fmt::Debug for Rate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Rate({self})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_NAMED: [Rate; 12] = [
        Rate::FPS_24,
        Rate::FPS_25,
        Rate::FPS_30,
        Rate::FPS_50,
        Rate::FPS_60,
        Rate::FPS_120,
        Rate::FPS_23_976,
        Rate::FPS_29_97,
        Rate::FPS_59_94,
        Rate::FPS_119_88,
        Rate::HZ_44_100,
        Rate::HZ_48_000,
    ];

    #[test]
    fn rescale_rounds_toward_negative_infinity() {
        assert_eq!(rescale(7, 1, 2), Some(3));
        assert_eq!(rescale(-7, 1, 2), Some(-4));
        assert_eq!(rescale(-1, 1, 3), Some(-1));
        assert_eq!(rescale(-3, 1, 3), Some(-1));
        assert_eq!(rescale(-4, 1, 3), Some(-2));
        assert_eq!(rescale(0, 5, 3), Some(0));
        // A negative numerator, as a reversed clip has.
        assert_eq!(rescale(7, -1, 2), Some(-4));
        assert_eq!(rescale(-7, -1, 2), Some(3));
    }

    #[test]
    fn rescale_refuses_a_divisor_that_is_not_above_zero() {
        assert_eq!(rescale(1, 1, 0), None);
        assert_eq!(rescale(1, 1, -1), None);
    }

    #[test]
    fn rescale_says_none_when_only_the_answer_is_too_large() {
        assert_eq!(rescale(i64::MAX, 4, 2), None);
        assert_eq!(rescale(i64::MIN, 4, 2), None);
        // The product is far past 64 bits and the answer fits.
        assert_eq!(rescale(i64::MAX, i64::MAX, i64::MAX), Some(i64::MAX));
        assert_eq!(rescale(i64::MIN, i64::MAX, i64::MAX), Some(i64::MIN));
    }

    #[test]
    fn a_day_times_the_largest_numerator_does_not_overflow() {
        let day = 24 * 3600 * TICKS_PER_SECOND;
        let max = i64::from(u32::MAX);
        // 46 bits times 32 bits: 78 bits inside, the day again outside.
        assert_eq!(rescale(day, max, max), Some(day));
        // The same through the grid: a rate of u32::MAX frames a second.
        let rate = Rate::new(u32::MAX, 1).unwrap();
        // Its frames are a sixth of a tick long: six more of them start
        // inside the day's last tick, and the last to start is showing.
        assert_eq!(rate.frame_at(Tick::new(day)), Some(24 * 3600 * max + 6));
        assert_eq!(rate.frame_start(24 * 3600 * max), Some(Tick::new(day)));
        // And a rate of one frame every u32::MAX seconds.
        let slow = Rate::new(1, u32::MAX).unwrap();
        assert_eq!(slow.frame_at(Tick::new(day)), Some(0));
        assert_eq!(slow.frame_start(1), Some(Tick::new(TICKS_PER_SECOND * max)));
    }

    #[test]
    fn one_hour_is_2_540_160_000_000_ticks() {
        assert_eq!(
            Tick::from_whole_seconds(3600),
            Some(Tick::new(2_540_160_000_000))
        );
        assert_eq!(
            TickDur::from_whole_seconds(3600),
            Some(TickDur::new(2_540_160_000_000))
        );
    }

    #[test]
    fn a_frame_of_29_97_is_23_543_520_ticks() {
        assert_eq!(Rate::FPS_29_97.frame_len(), TickLen::new(23_543_520));
        assert_eq!(Rate::FPS_23_976.frame_len(), TickLen::new(29_429_400));
        assert_eq!(Rate::FPS_59_94.frame_len(), TickLen::new(11_771_760));
        assert_eq!(Rate::FPS_119_88.frame_len(), TickLen::new(5_885_880));
    }

    #[test]
    fn a_hundred_frames_of_23_976_are_exactly_125_frames_of_29_97() {
        let span = Rate::FPS_23_976.span(100).unwrap();
        assert_eq!(span, TickDur::new(2_942_940_000));
        assert_eq!(Rate::FPS_29_97.frames_in(span), Some(125));
        assert_eq!(Rate::FPS_29_97.span(125), Some(span));
    }

    #[test]
    fn five_frames_of_29_97_are_exactly_8008_samples_at_48_khz() {
        let span = Rate::FPS_29_97.span(5).unwrap();
        assert_eq!(span.samples(48_000), Some(8008));
        assert_eq!(Rate::HZ_48_000.span(8008), Some(span));
        // One frame is 1601.6 samples: 1601 whole ones fit.
        assert_eq!(Rate::FPS_29_97.span(1).unwrap().samples(48_000), Some(1601));
    }

    #[test]
    fn a_sample_is_a_whole_number_of_ticks_at_the_named_rates() {
        assert_eq!(Rate::HZ_48_000.frame_len(), TickLen::new(14_700));
        assert_eq!(Rate::HZ_44_100.frame_len(), TickLen::new(16_000));
        assert_eq!(Tick::of_sample(48_000, 48_000), Tick::from_whole_seconds(1));
        assert_eq!(Tick::new(14_699).sample_at(48_000), Some(0));
        assert_eq!(Tick::new(14_700).sample_at(48_000), Some(1));
        assert_eq!(Tick::new(-1).sample_at(48_000), Some(-1));
        assert_eq!(Tick::new(1).sample_at(0), None);
    }

    #[test]
    fn every_named_rate_is_in_lowest_terms_and_has_whole_tick_frames() {
        for rate in ALL_NAMED {
            assert_eq!(Rate::new(rate.num(), rate.den()), Ok(rate), "{rate}");
            assert!(rate.frame_len().is_some(), "{rate}");
        }
    }

    #[test]
    fn a_rate_is_reduced_and_never_holds_a_zero() {
        let rate = Rate::new(60, 2).unwrap();
        assert_eq!((rate.num(), rate.den()), (30, 1));
        assert_eq!(Rate::new(0, 1), Err(RateError::Zero));
        assert_eq!(Rate::new(1, 0), Err(RateError::Zero));
        assert_eq!(Rate::hz(0), Err(RateError::Zero));
        assert_eq!(Rate::new(u32::MAX, u32::MAX), Rate::new(1, 1));
    }

    #[test]
    fn the_known_decimals_become_their_exact_fractions() {
        for (decimal, rate) in [
            (23.976, Rate::FPS_23_976),
            (23.98, Rate::FPS_23_976),
            (29.97, Rate::FPS_29_97),
            (59.94, Rate::FPS_59_94),
            (119.88, Rate::FPS_119_88),
            (30_000.0 / 1001.0, Rate::FPS_29_97),
            (24_000.0 / 1001.0, Rate::FPS_23_976),
        ] {
            assert_eq!(Rate::from_decimal(decimal), Ok(rate), "{decimal}");
        }
        assert_eq!(Rate::from_decimal(47.952), Rate::new(48_000, 1001));
        assert_eq!(Rate::from_decimal(47.95), Rate::new(48_000, 1001));
        assert_eq!(Rate::from_decimal(239.76), Rate::new(240_000, 1001));
    }

    #[test]
    fn any_other_decimal_is_taken_at_its_word_to_the_thousandth() {
        assert_eq!(Rate::from_decimal(24.0), Ok(Rate::FPS_24));
        assert_eq!(Rate::from_decimal(25.0), Ok(Rate::FPS_25));
        assert_eq!(Rate::from_decimal(12.5), Rate::new(25, 2));
        assert_eq!(Rate::from_decimal(0.001), Rate::new(1, 1000));
        assert_eq!(Rate::from_decimal(48_000.0), Ok(Rate::HZ_48_000));
        // Near a slowed rate and not one of its spellings: not guessed at.
        assert_eq!(Rate::from_decimal(29.975), Rate::new(29_975, 1000));
        assert_eq!(Rate::from_decimal(23.97), Rate::new(2397, 100));
    }

    #[test]
    fn nonsense_is_not_a_rate() {
        assert_eq!(Rate::from_decimal(f64::NAN), Err(RateError::NotANumber));
        assert_eq!(
            Rate::from_decimal(f64::INFINITY),
            Err(RateError::NotANumber)
        );
        assert_eq!(Rate::from_decimal(0.0), Err(RateError::NotANumber));
        assert_eq!(Rate::from_decimal(-24.0), Err(RateError::NotANumber));
        assert_eq!(Rate::from_decimal(5.0e6), Err(RateError::TooLarge));
        assert_eq!(Rate::from_decimal(1.0e300), Err(RateError::TooLarge));
        assert_eq!(Rate::from_decimal(24.000_05), Err(RateError::TooFine));
        assert_eq!(Rate::from_decimal(0.000_4), Err(RateError::TooFine));
    }

    /// The grid rule on a rate the tick does not divide, checked against
    /// the definition for every tick of the first frames and for the
    /// frames either side of zero.
    #[test]
    fn a_rate_the_tick_does_not_divide_has_a_grid_by_the_stated_rule() {
        let rate = Rate::new(2997, 100).unwrap();
        assert_eq!(rate.frame_len(), None);
        // floor(k * 70,560,000,000 / 2997)
        assert_eq!(rate.frame_start(0), Some(Tick::new(0)));
        assert_eq!(rate.frame_start(1), Some(Tick::new(23_543_543)));
        assert_eq!(rate.frame_start(2), Some(Tick::new(47_087_087)));
        assert_eq!(rate.frame_start(3), Some(Tick::new(70_630_630)));
        assert_eq!(rate.frame_start(-1), Some(Tick::new(-23_543_544)));
        // The frames are not all one length: 543, 544, 543.
        for k in -50..50_i64 {
            let start = rate.frame_start(k).unwrap();
            let next = rate.frame_start(k + 1).unwrap();
            assert!(start < next);
            // The frame showing at its own first tick, at its last, and
            // the tick before it starts.
            assert_eq!(rate.frame_at(start), Some(k), "frame {k} at its start");
            assert_eq!(
                rate.frame_at(next - TickDur::new(1)),
                Some(k),
                "frame {k} at its end"
            );
            assert_eq!(rate.frame_at(start - TickDur::new(1)), Some(k - 1));
            assert_eq!(rate.snap(start, Snap::Floor), Some(start));
            assert_eq!(rate.snap(start, Snap::Ceil), Some(start));
            assert_eq!(rate.snap(start + TickDur::new(1), Snap::Floor), Some(start));
            assert_eq!(rate.snap(start + TickDur::new(1), Snap::Ceil), Some(next));
        }
        // 2997 frames are 100 seconds to the tick, though no one frame is
        // a whole number of ticks.
        assert_eq!(rate.span(2997), TickDur::from_whole_seconds(100));
    }

    #[test]
    fn a_place_before_zero_is_in_the_frame_that_started_before_it() {
        let rate = Rate::FPS_25;
        let frame = rate.frame_len().unwrap().get();
        assert_eq!(rate.frame_at(Tick::new(0)), Some(0));
        assert_eq!(rate.frame_at(Tick::new(-1)), Some(-1));
        assert_eq!(rate.frame_at(Tick::new(-frame)), Some(-1));
        assert_eq!(rate.frame_at(Tick::new(-frame - 1)), Some(-2));
        assert_eq!(rate.frame_start(-2), Some(Tick::new(-2 * frame)));
        assert_eq!(
            rate.snap(Tick::new(-1), Snap::Floor),
            Some(Tick::new(-frame))
        );
        assert_eq!(rate.snap(Tick::new(-1), Snap::Ceil), Some(Tick::new(0)));
        assert_eq!(rate.snap(Tick::new(-1), Snap::Nearest), Some(Tick::new(0)));
        assert_eq!(
            rate.snap(Tick::new(1 - frame), Snap::Nearest),
            Some(Tick::new(-frame))
        );
        assert_eq!(rate.frames_in(TickDur::new(-1)), Some(-1));
    }

    #[test]
    fn snapping_goes_down_up_or_to_the_nearer_edge_and_a_tie_goes_later() {
        let rate = Rate::FPS_50;
        let frame = rate.frame_len().unwrap().get();
        assert_eq!(frame % 2, 0);
        let half = Tick::new(3 * frame + frame / 2);
        assert_eq!(rate.snap(half, Snap::Floor), Some(Tick::new(3 * frame)));
        assert_eq!(rate.snap(half, Snap::Ceil), Some(Tick::new(4 * frame)));
        assert_eq!(rate.snap(half, Snap::Nearest), Some(Tick::new(4 * frame)));
        let under = half - TickDur::new(1);
        assert_eq!(rate.snap(under, Snap::Nearest), Some(Tick::new(3 * frame)));
        let on = Tick::new(7 * frame);
        for to in [Snap::Floor, Snap::Nearest, Snap::Ceil] {
            assert_eq!(rate.snap(on, to), Some(on));
        }
    }

    #[test]
    fn the_grid_says_none_at_the_ends_of_time_and_does_not_wrap() {
        assert_eq!(Rate::FPS_24.frame_start(i64::MAX), None);
        assert_eq!(Rate::FPS_24.frame_at(Tick::MAX), None);
        assert_eq!(
            Rate::new(u32::MAX, 1)
                .unwrap()
                .frame_at(Tick::new(i64::MAX - 1)),
            None
        );
        assert_eq!(Rate::FPS_24.snap(Tick::new(i64::MAX - 1), Snap::Ceil), None);
        assert!(Rate::FPS_24.frame_at(Tick::MIN).is_some());
    }

    #[test]
    fn places_and_lengths_add_up_only_the_ways_that_mean_something() {
        let a = Tick::new(100);
        let b = Tick::new(40);
        assert_eq!(a - b, TickDur::new(60));
        assert_eq!(b - a, TickDur::new(-60));
        assert_eq!(b + TickDur::new(60), a);
        assert_eq!(a - TickDur::new(60), b);
        assert_eq!(a - TickLen::new(60).unwrap(), b);
        assert_eq!(TickDur::new(5) + TickDur::new(6), TickDur::new(11));
        assert_eq!(TickDur::new(5) - TickDur::new(6), TickDur::new(-1));
        assert_eq!(-TickDur::new(5), TickDur::new(-5));
        assert_eq!(TickDur::new(5) * 3, TickDur::new(15));
        let mut at = a;
        at += TickDur::new(1);
        at -= TickDur::new(2);
        assert_eq!(at, Tick::new(99));
        assert_eq!(TickDur::new(7).scaled(-1, 2), Some(TickDur::new(-4)));
        assert_eq!(TickDur::from_millis(1), Some(TickDur::new(705_600)));
    }

    #[test]
    fn checked_arithmetic_says_none_at_the_ends() {
        assert_eq!(Tick::MAX.checked_add(TickDur::new(1)), None);
        assert_eq!(Tick::MIN.checked_sub(TickDur::new(1)), None);
        assert_eq!(Tick::MAX.checked_since(Tick::new(-1)), None);
        assert_eq!(TickDur::new(i64::MAX).checked_add(TickDur::new(1)), None);
        assert_eq!(TickDur::new(i64::MIN).checked_sub(TickDur::new(1)), None);
        assert_eq!(TickDur::new(i64::MIN).checked_neg(), None);
        assert_eq!(TickDur::new(i64::MAX).checked_mul(2), None);
        assert_eq!(Tick::from_whole_seconds(i64::MAX), None);
        assert_eq!(TickDur::from_millis(i64::MAX), None);
        let longest = TickLen::new(i64::MAX).unwrap();
        assert_eq!(longest.checked_add(TickLen::ONE), None);
        assert_eq!(TickLen::ONE.checked_add(TickLen::ONE), TickLen::new(2));
    }

    #[test]
    #[should_panic(expected = "tick arithmetic overflowed")]
    fn an_operator_panics_and_does_not_wrap() {
        let _ = Tick::MAX + TickDur::new(1);
    }

    #[test]
    fn a_length_is_never_zero_or_less() {
        assert_eq!(TickLen::new(0), None);
        assert_eq!(TickLen::new(-1), None);
        assert_eq!(TickLen::new(i64::MIN), None);
        assert_eq!(TickLen::ONE.get(), 1);
        assert_eq!(TickDur::new(0).to_len(), None);
        assert_eq!(TickDur::new(9).to_len().map(TickLen::get), Some(9));
        assert_eq!(TickLen::try_from(TickDur::new(-3)), Err(NotALength(-3)));
        assert!(TickDur::new(1).is_positive());
        assert!(!TickDur::ZERO.is_positive());
    }

    #[test]
    fn seconds_for_display_are_the_nearest_and_refuse_what_is_not_a_number() {
        assert_eq!(Tick::new(TICKS_PER_SECOND / 2).display_seconds(), 0.5);
        assert_eq!(TickDur::SECOND.display_seconds(), 1.0);
        assert_eq!(
            Tick::from_display_seconds(12.5),
            Some(Tick::new(8_820_000_000))
        );
        assert_eq!(
            Tick::from_display_seconds(-0.5),
            Some(Tick::new(-352_800_000))
        );
        assert_eq!(TickDur::from_display_seconds(1.0), Some(TickDur::SECOND));
        assert_eq!(Tick::from_display_seconds(f64::NAN), None);
        assert_eq!(Tick::from_display_seconds(f64::INFINITY), None);
        assert_eq!(Tick::from_display_seconds(1.0e12), None);
        assert_eq!(Tick::from_display_seconds(-1.0e12), None);
        assert_eq!(Rate::FPS_25.display_per_second(), 25.0);
    }

    #[test]
    fn a_tick_is_a_whole_number_in_a_file() {
        assert_eq!(
            serde_json::to_string(&Tick::new(2_540_160_000_000)).unwrap(),
            "2540160000000"
        );
        assert_eq!(serde_json::to_string(&TickDur::new(-7)).unwrap(), "-7");
        assert_eq!(
            serde_json::to_string(&TickLen::new(7).unwrap()).unwrap(),
            "7"
        );
        assert_eq!(serde_json::from_str::<Tick>("-12").unwrap(), Tick::new(-12));
        assert_eq!(
            serde_json::from_str::<TickDur>("12").unwrap(),
            TickDur::new(12)
        );
        assert_eq!(
            serde_json::from_str::<TickLen>("12").unwrap(),
            TickLen::new(12).unwrap()
        );
        // Neither a fraction nor a string is a tick, and no length is zero.
        assert!(serde_json::from_str::<Tick>("1.0").is_err());
        assert!(serde_json::from_str::<Tick>("1.5").is_err());
        assert!(serde_json::from_str::<Tick>("\"1\"").is_err());
        assert!(serde_json::from_str::<Tick>("9223372036854775808").is_err());
        assert!(serde_json::from_str::<TickLen>("0").is_err());
        assert!(serde_json::from_str::<TickLen>("-4").is_err());
    }

    #[test]
    fn a_rate_is_read_reduced_and_refused_with_a_zero() {
        let json = serde_json::to_string(&Rate::FPS_29_97).unwrap();
        assert_eq!(json, r#"{"num":30000,"den":1001}"#);
        assert_eq!(
            serde_json::from_str::<Rate>(&json).unwrap(),
            Rate::FPS_29_97
        );
        let loose: Rate = serde_json::from_str(r#"{"num":60,"den":2}"#).unwrap();
        assert_eq!(loose, Rate::FPS_30);
        assert!(serde_json::from_str::<Rate>(r#"{"num":0,"den":1}"#).is_err());
        assert!(serde_json::from_str::<Rate>(r#"{"num":24,"den":0}"#).is_err());
        assert!(serde_json::from_str::<Rate>(r#"{"num":-24,"den":1}"#).is_err());
    }

    #[test]
    fn a_rate_prints_as_its_fraction() {
        assert_eq!(Rate::FPS_24.to_string(), "24");
        assert_eq!(Rate::FPS_29_97.to_string(), "30000/1001");
        assert_eq!(format!("{:?}", Rate::FPS_29_97), "Rate(30000/1001)");
        assert_eq!(Tick::new(-3).to_string(), "-3");
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Exact rational time.
//!
//! Video editing is frame-accurate arithmetic on rates that are not
//! representable in binary floating point. 29.97 fps is really 30000/1001, and
//! an `f64` accumulator drifts: after an hour of adding `1.0 / 29.97` you are
//! visibly off. So every timestamp in WolfCut is a [`Rational`] number of
//! **seconds**, and conversions to `f64` happen only at the edges - display,
//! and arguments handed to FFmpeg.
//!
//! ```
//! use wolfcut_core::time::{FrameRate, Rational};
//!
//! let rate = FrameRate::NTSC_30;             // 30000/1001
//! let t = rate.time_of_frame(1800);          // exactly 60.06 seconds
//! assert_eq!(t, Rational::new(60060, 1000));
//! assert_eq!(rate.frame_at(t), 1800);        // and it round-trips, forever
//! ```

use std::cmp::Ordering;
use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// Greatest common divisor of two integers, ignoring sign.
const fn gcd(a: i128, b: i128) -> i128 {
    let mut a = if a < 0 { -a } else { a };
    let mut b = if b < 0 { -b } else { b };
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// An exact rational number, always stored in lowest terms with a positive
/// denominator.
///
/// Used throughout WolfCut to mean *seconds* unless a doc comment says otherwise.
///
/// Arithmetic is performed in `i128` and reduced before being stored back into
/// `i64`. That is enough headroom for any realistic timeline; a value that
/// genuinely cannot be represented panics rather than silently wrapping.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rational {
    num: i64,
    den: i64,
}

impl Rational {
    /// Zero seconds.
    pub const ZERO: Self = Self { num: 0, den: 1 };
    /// One second.
    pub const ONE: Self = Self { num: 1, den: 1 };

    /// Builds `num / den` in lowest terms.
    ///
    /// # Panics
    /// If `den` is zero.
    pub fn new(num: i64, den: i64) -> Self {
        Self::reduce(i128::from(num), i128::from(den))
    }

    /// Builds a whole number of seconds.
    pub const fn from_int(value: i64) -> Self {
        Self { num: value, den: 1 }
    }

    /// Parses FFmpeg's `"num/den"` fraction syntax, also accepting a bare
    /// integer or decimal. Returns `None` on anything else, including `"0/0"`,
    /// which FFmpeg emits for streams with no meaningful frame rate.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Some((num, den)) = text.split_once('/') {
            let num: i64 = num.trim().parse().ok()?;
            let den: i64 = den.trim().parse().ok()?;
            if den == 0 {
                return None;
            }
            return Some(Self::new(num, den));
        }
        if let Some((whole, frac)) = text.split_once('.') {
            let sign = if whole.starts_with('-') { -1 } else { 1 };
            let whole: i64 = whole.trim().parse().ok()?;
            let digits = frac.trim();
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let den = 10i64.checked_pow(u32::try_from(digits.len()).ok()?)?;
            let frac: i64 = digits.parse().ok()?;
            return Some(Self::new(whole.checked_mul(den)? + sign * frac, den));
        }
        text.parse().ok().map(Self::from_int)
    }

    fn reduce(num: i128, den: i128) -> Self {
        assert!(den != 0, "Rational denominator must not be zero");
        Self::checked_reduce(num, den).expect("rational overflowed i64")
    }

    /// Builds `num / den` in lowest terms, or `None` when the value cannot be
    /// represented. The non-panicking path for numbers that come from a file
    /// rather than from our own arithmetic - a hostile container must produce
    /// an error, never a crash.
    pub fn checked_new(num: i128, den: i128) -> Option<Self> {
        if den == 0 {
            return None;
        }
        Self::checked_reduce(num, den)
    }

    fn checked_reduce(num: i128, den: i128) -> Option<Self> {
        let (num, den) = if den < 0 { (-num, -den) } else { (num, den) };
        let divisor = gcd(num, den).max(1);
        let (num, den) = (num / divisor, den / divisor);
        Some(Self {
            num: i64::try_from(num).ok()?,
            den: i64::try_from(den).ok()?,
        })
    }

    /// The numerator, in lowest terms.
    pub const fn numerator(self) -> i64 {
        self.num
    }

    /// The denominator, in lowest terms. Always positive.
    pub const fn denominator(self) -> i64 {
        self.den
    }

    /// True if this is exactly zero.
    pub const fn is_zero(self) -> bool {
        self.num == 0
    }

    /// True if this is less than zero.
    pub const fn is_negative(self) -> bool {
        self.num < 0
    }

    /// The multiplicative inverse.
    ///
    /// # Panics
    /// If this is zero.
    pub fn recip(self) -> Self {
        assert!(!self.is_zero(), "cannot take the reciprocal of zero");
        Self::reduce(i128::from(self.den), i128::from(self.num))
    }

    /// The largest integer less than or equal to this value.
    pub const fn floor(self) -> i64 {
        self.num.div_euclid(self.den)
    }

    /// The smallest integer greater than or equal to this value.
    pub const fn ceil(self) -> i64 {
        -((-self.num).div_euclid(self.den))
    }

    /// Clamps to the inclusive range `[low, high]`.
    pub fn clamp_to(self, low: Self, high: Self) -> Self {
        if self < low {
            low
        } else if self > high {
            high
        } else {
            self
        }
    }

    /// The nearest rational with denominator 1,000,000 - the seam where a
    /// UI's `f64` becomes exact. Returns `None` for NaN or infinite values and
    /// for magnitudes an `i64` of microseconds cannot hold.
    ///
    /// This is for values *entering* the engine. Values that are already
    /// `Rational` must never round-trip through here.
    pub fn approximate(value: f64) -> Option<Self> {
        if !value.is_finite() {
            return None;
        }
        let scaled = (value * 1_000_000.0).round();
        if scaled.abs() >= i64::MAX as f64 {
            return None;
        }
        Some(Self::new(scaled as i64, 1_000_000))
    }

    /// Lossy conversion, for display and for arguments handed to FFmpeg.
    ///
    /// Do not convert back. If a value round-trips through `f64` it is no
    /// longer exact, and that is how frame drift starts.
    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Self) -> Ordering {
        // Denominators are always positive, so cross-multiplying preserves order.
        let left = i128::from(self.num) * i128::from(other.den);
        let right = i128::from(other.num) * i128::from(self.den);
        left.cmp(&right)
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Add for Rational {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        let num = i128::from(self.num) * i128::from(rhs.den) + i128::from(rhs.num) * i128::from(self.den);
        Self::reduce(num, i128::from(self.den) * i128::from(rhs.den))
    }
}

impl Sub for Rational {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        self + (-rhs)
    }
}

impl Mul for Rational {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self::reduce(
            i128::from(self.num) * i128::from(rhs.num),
            i128::from(self.den) * i128::from(rhs.den),
        )
    }
}

impl Div for Rational {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        assert!(!rhs.is_zero(), "cannot divide a Rational by zero");
        Self::reduce(
            i128::from(self.num) * i128::from(rhs.den),
            i128::from(self.den) * i128::from(rhs.num),
        )
    }
}

impl Neg for Rational {
    type Output = Self;
    fn neg(self) -> Self {
        Self { num: -self.num, den: self.den }
    }
}

impl AddAssign for Rational {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl SubAssign for Rational {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl From<i64> for Rational {
    fn from(value: i64) -> Self {
        Self::from_int(value)
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

impl fmt::Debug for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

/// Frames per second, as an exact fraction.
///
/// Always positive. Construct the broadcast rates from the associated
/// constants rather than typing `29.97` anywhere.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FrameRate(Rational);

impl FrameRate {
    /// 23.976 fps (24000/1001).
    pub const FILM_NTSC: Self = Self(Rational { num: 24000, den: 1001 });
    /// 24 fps.
    pub const FILM: Self = Self(Rational { num: 24, den: 1 });
    /// 25 fps.
    pub const PAL: Self = Self(Rational { num: 25, den: 1 });
    /// 29.97 fps (30000/1001).
    pub const NTSC_30: Self = Self(Rational { num: 30000, den: 1001 });
    /// 30 fps.
    pub const THIRTY: Self = Self(Rational { num: 30, den: 1 });
    /// 59.94 fps (60000/1001).
    pub const NTSC_60: Self = Self(Rational { num: 60000, den: 1001 });
    /// 60 fps.
    pub const SIXTY: Self = Self(Rational { num: 60, den: 1 });

    /// Builds a frame rate from an exact fraction of frames per second.
    ///
    /// # Panics
    /// If `fps` is zero or negative.
    pub fn new(fps: Rational) -> Self {
        assert!(!fps.is_zero() && !fps.is_negative(), "frame rate must be positive");
        Self(fps)
    }

    /// Builds a whole-number frame rate.
    pub fn from_int(fps: u32) -> Self {
        Self::new(Rational::from_int(i64::from(fps)))
    }

    /// Frames per second.
    pub const fn fps(self) -> Rational {
        self.0
    }

    /// How long one frame lasts, in seconds.
    pub fn frame_duration(self) -> Rational {
        self.0.recip()
    }

    /// The timestamp at which frame `index` begins. Frame 0 begins at zero.
    pub fn time_of_frame(self, index: i64) -> Rational {
        Rational::from_int(index) * self.frame_duration()
    }

    /// The index of the frame that is on screen at `time`.
    ///
    /// Frames are half-open intervals: a timestamp exactly on a boundary
    /// belongs to the frame that starts there.
    pub fn frame_at(self, time: Rational) -> i64 {
        (time * self.0).floor()
    }

    /// How many whole frames fit in `duration`.
    pub fn frames_in(self, duration: Rational) -> i64 {
        (duration * self.0).floor()
    }
}

impl fmt::Display for FrameRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3} fps", self.0.as_f64())
    }
}

/// A half-open span of time, `[start, start + duration)`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimeRange {
    /// When the span begins.
    pub start: Rational,
    /// How long the span lasts. Never negative.
    pub duration: Rational,
}

impl TimeRange {
    /// Builds a span.
    ///
    /// # Panics
    /// If `duration` is negative.
    pub fn new(start: Rational, duration: Rational) -> Self {
        assert!(!duration.is_negative(), "TimeRange duration must not be negative");
        Self { start, duration }
    }

    /// The first instant after the span.
    pub fn end(self) -> Rational {
        self.start + self.duration
    }

    /// True if `time` falls inside the span. The end is excluded.
    pub fn contains(self, time: Rational) -> bool {
        time >= self.start && time < self.end()
    }

    /// True if the two spans share at least one instant.
    pub fn overlaps(self, other: Self) -> bool {
        self.start < other.end() && other.start < self.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduces_to_lowest_terms() {
        let r = Rational::new(6, 8);
        assert_eq!((r.numerator(), r.denominator()), (3, 4));
    }

    #[test]
    fn normalises_a_negative_denominator() {
        let r = Rational::new(1, -2);
        assert_eq!((r.numerator(), r.denominator()), (-1, 2));
    }

    #[test]
    fn equal_values_written_differently_compare_equal() {
        assert_eq!(Rational::new(1, 2), Rational::new(50, 100));
    }

    #[test]
    fn arithmetic_is_exact() {
        let third = Rational::new(1, 3);
        assert_eq!(third + third + third, Rational::ONE);
    }

    #[test]
    fn ntsc_never_drifts() {
        // The whole reason this module exists: an f64 accumulator is visibly
        // wrong here, and this is exact after an hour of frames.
        let rate = FrameRate::NTSC_30;
        let mut clock = Rational::ZERO;
        for _ in 0..107_892 {
            clock += rate.frame_duration();
        }
        assert_eq!(clock, rate.time_of_frame(107_892));
        assert_eq!(rate.frame_at(clock), 107_892);
    }

    #[test]
    fn frame_boundaries_belong_to_the_frame_that_starts_there() {
        let rate = FrameRate::THIRTY;
        assert_eq!(rate.frame_at(rate.time_of_frame(7)), 7);
        assert_eq!(rate.frame_at(rate.time_of_frame(7) - Rational::new(1, 1_000_000)), 6);
    }

    #[test]
    fn floor_and_ceil_handle_negatives() {
        assert_eq!(Rational::new(-3, 2).floor(), -2);
        assert_eq!(Rational::new(-3, 2).ceil(), -1);
        assert_eq!(Rational::new(3, 2).floor(), 1);
        assert_eq!(Rational::new(3, 2).ceil(), 2);
    }

    #[test]
    fn parses_ffmpeg_fractions() {
        assert_eq!(Rational::parse("30000/1001"), Some(FrameRate::NTSC_30.fps()));
        assert_eq!(Rational::parse("25"), Some(Rational::from_int(25)));
        assert_eq!(Rational::parse("1.5"), Some(Rational::new(3, 2)));
        assert_eq!(Rational::parse("-1.25"), Some(Rational::new(-5, 4)));
        assert_eq!(Rational::parse("0/0"), None);
        assert_eq!(Rational::parse("N/A"), None);
    }

    #[test]
    fn ranges_are_half_open() {
        let range = TimeRange::new(Rational::from_int(2), Rational::from_int(3));
        assert!(range.contains(Rational::from_int(2)));
        assert!(!range.contains(Rational::from_int(5)));
        assert_eq!(range.end(), Rational::from_int(5));
    }

    #[test]
    fn touching_ranges_do_not_overlap() {
        let a = TimeRange::new(Rational::ZERO, Rational::from_int(2));
        let b = TimeRange::new(Rational::from_int(2), Rational::from_int(2));
        assert!(!a.overlaps(b));
        assert!(a.overlaps(TimeRange::new(Rational::ONE, Rational::from_int(2))));
    }
}

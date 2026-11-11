// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Speed that changes over a clip.
//!
//! A constant speed is one number. A *curve* is speed as a function of where
//! in the clip you are: a handful of points, `(x, speed)` with `x` the
//! fraction of the clip's timeline length, `0` at its start and `1` at its
//! end, and speed in source seconds per timeline second.
//!
//! Between two points the speed runs either in a straight line or, when the
//! curve carries a tangent at every point, along a cubic Hermite segment.
//! Everything the editor needs from a curve has a closed form: the speed at
//! a place, its rate of change, and the area under it, which is how much
//! source has been consumed by then. So the time map costs a binary search
//! and a few multiplications per frame, with nothing allocated.
//!
//! The units are deliberately relative. A curve stored as fractions means
//! the same thing after the clip is trimmed or the project re-timed, and a
//! preset can be applied to any clip without knowing how long it is.

/// A curve's rises shallower than this count as flat: the tangents of a
/// flat stretch are zero, so it cannot bulge above or below its level.
const FLAT_RISE: f64 = 1e-12;

/// The longest pair of tangents, in rises of their stretch, that keeps a
/// Hermite segment from overshooting the speeds it joins (Fritsch and
/// Carlson's circle of radius three).
const MONOTONE_RADIUS: f64 = 3.0;

/// A clip's speed over its length, tidied and ready to evaluate: see the
/// module's notes.
#[derive(Clone, PartialEq, Debug)]
pub struct SpeedCurve {
    /// `(x, speed)` with `x` in `0..=1` ascending, the first at 0 and the
    /// last at 1, speed within `MIN_SPEED..=MAX_SPEED`. Two points that
    /// share an `x` make a step.
    points: Vec<(f64, f64)>,
    /// One tangent a point, in speed per unit of `x`, for a smooth curve:
    /// already held to the monotone limit. None for straight lines.
    tangents: Option<Vec<f64>>,
    /// The source consumed from the start up to each point: worked out once,
    /// so the area to any place is one stretch's share plus a lookup.
    consumed_to: Vec<f64>,
}

/// Where a place falls on the curve: the stretch it is on and how far along
/// it, `0..=1`.
#[derive(Clone, Copy)]
struct Place {
    stretch: usize,
    along: f64,
}

impl SpeedCurve {
    /// The floor a curve's speed is held at: a stop would consume no source
    /// and the map would stop being invertible.
    pub const MIN_SPEED: f64 = 0.0625;
    /// And the ceiling, matching the constant rate's.
    pub const MAX_SPEED: f64 = 16.0;

    /// A curve of straight lines through `points`, or None when not one of
    /// them is a pair of numbers. See [`SpeedCurve::with_tangents`] for how
    /// the points are tidied.
    pub fn new(points: &[(f64, f64)]) -> Option<SpeedCurve> {
        Self::with_tangents(points, None)
    }

    /// A curve through `points`, smooth when `tangents` has one for every
    /// point (counted as given, before anything is dropped), straight lines
    /// otherwise.
    ///
    /// The points are tidied: a point whose place or speed is not a number
    /// is dropped, places are held to `0..=1` and speeds to the range, a
    /// tangent that is not a number is flat, and the points are put in
    /// order of place, keeping the order of points that share one. A curve
    /// that does not reach an end is held level out to it, so a single
    /// point is a flat curve. A smooth curve's tangents are then held to
    /// what keeps every stretch between the speeds it joins.
    pub fn with_tangents(points: &[(f64, f64)], tangents: Option<&[f64]>) -> Option<SpeedCurve> {
        let tangents = tangents.filter(|tangents| tangents.len() == points.len());
        let mut kept: Vec<(f64, f64, f64)> = points
            .iter()
            .enumerate()
            .filter(|(_, (x, speed))| x.is_finite() && speed.is_finite())
            .map(|(index, &(x, speed))| {
                let tangent = tangents
                    .map(|tangents| tangents[index])
                    .filter(|tangent| tangent.is_finite())
                    .unwrap_or(0.0);
                (
                    x.clamp(0.0, 1.0),
                    speed.clamp(Self::MIN_SPEED, Self::MAX_SPEED),
                    tangent,
                )
            })
            .collect();
        if kept.is_empty() {
            return None;
        }
        // Stable, so the two sides of a step keep their order.
        kept.sort_by(|a, b| a.0.total_cmp(&b.0));
        let first = kept[0];
        if first.0 > 0.0 {
            kept.insert(0, (0.0, first.1, 0.0));
        }
        let last = kept[kept.len() - 1];
        if last.0 < 1.0 {
            kept.push((1.0, last.1, 0.0));
        }

        let points: Vec<(f64, f64)> = kept.iter().map(|&(x, speed, _)| (x, speed)).collect();
        let tangents = tangents.map(|_| {
            let mut tangents: Vec<f64> = kept.iter().map(|&(_, _, tangent)| tangent).collect();
            limit_to_monotone(&points, &mut tangents);
            tangents
        });
        let mut curve = SpeedCurve {
            consumed_to: Vec::with_capacity(points.len()),
            points,
            tangents,
        };
        let mut total = 0.0;
        curve.consumed_to.push(total);
        for stretch in 0..curve.points.len() - 1 {
            total += curve.area_along(stretch, 1.0);
            curve.consumed_to.push(total);
        }
        Some(curve)
    }

    /// The points, as tidied.
    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }

    /// The tangents, one a point, held to the monotone limit; None for a
    /// curve of straight lines.
    pub fn tangents(&self) -> Option<&[f64]> {
        self.tangents.as_deref()
    }

    /// Speed at `x`, a fraction of the clip's length; the end speeds hold
    /// outside it. On a step the speed arriving there counts: a clip ends
    /// on the speed it was going at.
    pub fn speed_at(&self, x: f64) -> f64 {
        self.speed_on(self.arriving(x))
    }

    /// Speed just after `x`: the same as [`SpeedCurve::speed_at`] except on
    /// a step, where it is the speed the step jumps to. A piece of the clip
    /// that starts at `x` starts at this speed.
    pub fn speed_leaving(&self, x: f64) -> f64 {
        self.speed_on(self.leaving(x))
    }

    /// The rate the speed changes at `x`, in speed per unit of `x`: zero
    /// outside the clip and on a step.
    pub fn tangent_at(&self, x: f64) -> f64 {
        if !(0.0..=1.0).contains(&x) {
            return 0.0;
        }
        self.tangent_on(self.arriving(x))
    }

    /// The rate of change just after `x`, as [`SpeedCurve::speed_leaving`]
    /// is to [`SpeedCurve::speed_at`].
    pub fn tangent_leaving(&self, x: f64) -> f64 {
        if !(0.0..=1.0).contains(&x) {
            return 0.0;
        }
        self.tangent_on(self.leaving(x))
    }

    /// The area under the curve from the start to `x`: how much of the
    /// source, as a fraction of the clip's timeline length, has been consumed
    /// by then. Nothing before the start, all of it after the end.
    pub fn consumed(&self, x: f64) -> f64 {
        let place = self.arriving(x);
        self.consumed_to[place.stretch] + self.area_along(place.stretch, place.along)
    }

    /// The average speed: source seconds consumed per timeline second over
    /// the whole clip.
    pub fn mean(&self) -> f64 {
        self.consumed_to[self.consumed_to.len() - 1].max(Self::MIN_SPEED)
    }

    /// The clip cut into `count` equal timeline pieces, each `(x0, x1,
    /// consumed_at_x0, mean_speed)` - the constant-rate approximation a
    /// sound path that can only change tempo in steps needs.
    pub fn pieces(&self, count: usize) -> Vec<(f64, f64, f64, f64)> {
        let count = count.max(1);
        let mut from = 0.0;
        (0..count)
            .map(|index| {
                let x0 = index as f64 / count as f64;
                let x1 = (index + 1) as f64 / count as f64;
                let to = self.consumed(x1);
                let piece = (x0, x1, from, ((to - from) / (x1 - x0)).max(Self::MIN_SPEED));
                from = to;
                piece
            })
            .collect()
    }

    /// The stretch `x` falls on, counting a place shared by two stretches
    /// as the end of the earlier: the first stretch that ends at or after
    /// `x`. A stretch of no width there (a step) is taken at its end.
    fn arriving(&self, x: f64) -> Place {
        let x = held(x);
        let stretch = self.points[1..].partition_point(|point| point.0 < x);
        self.place(stretch.min(self.points.len() - 2), x)
    }

    /// The stretch `x` falls on, counting a shared place as the start of
    /// the later: the first that ends after `x`, or the last at the end.
    fn leaving(&self, x: f64) -> Place {
        let x = held(x);
        let stretch = self.points[1..].partition_point(|point| point.0 <= x);
        self.place(stretch.min(self.points.len() - 2), x)
    }

    fn place(&self, stretch: usize, x: f64) -> Place {
        let (x0, _) = self.points[stretch];
        let (x1, _) = self.points[stretch + 1];
        let width = x1 - x0;
        let along = if width > 0.0 {
            ((x - x0) / width).clamp(0.0, 1.0)
        } else {
            1.0
        };
        Place { stretch, along }
    }

    /// A stretch's ends: its width, the two speeds and the two tangents
    /// scaled to the width (zero for straight lines).
    fn ends(&self, stretch: usize) -> (f64, f64, f64, f64, f64) {
        let (x0, v0) = self.points[stretch];
        let (x1, v1) = self.points[stretch + 1];
        let width = x1 - x0;
        let (m0, m1) = match &self.tangents {
            Some(tangents) => (tangents[stretch] * width, tangents[stretch + 1] * width),
            None => (0.0, 0.0),
        };
        (width, v0, v1, m0, m1)
    }

    fn speed_on(&self, place: Place) -> f64 {
        let (_, v0, v1, m0, m1) = self.ends(place.stretch);
        let u = place.along;
        if self.tangents.is_none() {
            return v0 + (v1 - v0) * u;
        }
        let (u2, u3) = (u * u, u * u * u);
        (2.0 * u3 - 3.0 * u2 + 1.0) * v0
            + (u3 - 2.0 * u2 + u) * m0
            + (-2.0 * u3 + 3.0 * u2) * v1
            + (u3 - u2) * m1
    }

    fn tangent_on(&self, place: Place) -> f64 {
        let (width, v0, v1, m0, m1) = self.ends(place.stretch);
        if width <= 0.0 {
            return 0.0;
        }
        if self.tangents.is_none() {
            return (v1 - v0) / width;
        }
        let u = place.along;
        let u2 = u * u;
        ((6.0 * u2 - 6.0 * u) * v0
            + (3.0 * u2 - 4.0 * u + 1.0) * m0
            + (-6.0 * u2 + 6.0 * u) * v1
            + (3.0 * u2 - 2.0 * u) * m1)
            / width
    }

    /// The area under one stretch from its start to `u` of the way along:
    /// a trapezoid for a straight line, the Hermite basis integrated for a
    /// smooth one.
    fn area_along(&self, stretch: usize, u: f64) -> f64 {
        let (width, v0, v1, m0, m1) = self.ends(stretch);
        if width <= 0.0 || u <= 0.0 {
            return 0.0;
        }
        if self.tangents.is_none() {
            let reached = v0 + (v1 - v0) * u;
            return width * u * (v0 + reached) / 2.0;
        }
        let (u2, u3) = (u * u, u * u * u);
        let u4 = u2 * u2;
        width
            * ((u - u3 + u4 / 2.0) * v0
                + (u4 / 4.0 - 2.0 * u3 / 3.0 + u2 / 2.0) * m0
                + (u3 - u4 / 2.0) * v1
                + (u4 / 4.0 - u3 / 3.0) * m1)
    }
}

/// A place held to the clip; one that is not a number reads as the start.
fn held(x: f64) -> f64 {
    if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) }
}

/// Fritsch and Carlson's limit, stretch by stretch from the start: a
/// tangent against its stretch's direction is flattened, a pair too long
/// for the stretch's rise is shortened until it fits, and a flat stretch
/// has flat ends. Points are shared, so the slope one stretch leaves on its
/// end is where the next starts from - which is why a peak ends flat. What
/// comes out never leaves the two speeds each stretch joins, so it never
/// leaves the range either.
fn limit_to_monotone(points: &[(f64, f64)], tangents: &mut [f64]) {
    for stretch in 0..points.len() - 1 {
        let (x0, v0) = points[stretch];
        let (x1, v1) = points[stretch + 1];
        let width = x1 - x0;
        if width <= 0.0 {
            continue;
        }
        let rise = (v1 - v0) / width;
        if rise.abs() < FLAT_RISE {
            tangents[stretch] = 0.0;
            tangents[stretch + 1] = 0.0;
            continue;
        }
        let a = (tangents[stretch] / rise).max(0.0);
        let b = (tangents[stretch + 1] / rise).max(0.0);
        let length = a.hypot(b);
        let scale = if length > MONOTONE_RADIUS {
            MONOTONE_RADIUS / length
        } else {
            1.0
        };
        tangents[stretch] = scale * a * rise;
        tangents[stretch + 1] = scale * b * rise;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_curve_is_a_constant_speed() {
        let curve = SpeedCurve::new(&[(0.0, 2.0), (1.0, 2.0)]).unwrap();
        assert_eq!(curve.speed_at(0.3), 2.0);
        assert!((curve.consumed(0.5) - 1.0).abs() < 1e-12);
        assert!((curve.mean() - 2.0).abs() < 1e-12);
    }

    #[test]
    fn the_ends_are_anchored_and_the_area_is_exact() {
        // Speed ramps from 1 to 3 over the clip: the area is the trapezoid.
        let curve = SpeedCurve::new(&[(0.25, 1.0), (0.75, 3.0)]).unwrap();
        assert_eq!(curve.points()[0], (0.0, 1.0));
        assert_eq!(curve.points()[curve.points().len() - 1], (1.0, 3.0));
        assert!((curve.speed_at(0.5) - 2.0).abs() < 1e-12);
        // 0..0.25 at 1, 0.25..0.75 averaging 2, 0.75..1 at 3.
        assert!((curve.mean() - (0.25 + 1.0 + 0.75)).abs() < 1e-12);
        assert!((curve.consumed(0.5) - (0.25 + 0.25 * 1.5)).abs() < 1e-12);
    }

    #[test]
    fn pieces_add_up_to_the_whole() {
        let curve = SpeedCurve::new(&[(0.0, 0.5), (0.5, 4.0), (1.0, 0.5)]).unwrap();
        let pieces = curve.pieces(8);
        assert_eq!(pieces.len(), 8);
        let total: f64 = pieces
            .iter()
            .map(|(x0, x1, _, mean)| (x1 - x0) * mean)
            .sum();
        assert!((total - curve.mean()).abs() < 1e-9);
        assert!((pieces[3].2 - curve.consumed(3.0 / 8.0)).abs() < 1e-12);
    }

    #[test]
    fn nonsense_is_tidied_or_refused() {
        assert!(SpeedCurve::new(&[]).is_none());
        let curve = SpeedCurve::new(&[(2.0, 0.0), (-1.0, 100.0)]).unwrap();
        assert_eq!(curve.points()[0], (0.0, SpeedCurve::MAX_SPEED));
        assert_eq!(curve.points()[1], (1.0, SpeedCurve::MIN_SPEED));
    }

    #[test]
    fn a_step_is_left_at_the_speed_it_jumps_to() {
        let curve = SpeedCurve::with_tangents(
            &[(0.0, 0.5), (0.5, 0.5), (0.5, 2.0), (1.0, 4.0)],
            Some(&[0.0, 0.0, 0.0, 0.0]),
        )
        .unwrap();
        assert_eq!(curve.speed_at(0.5), 0.5, "arriving: before the jump");
        assert_eq!(curve.speed_leaving(0.5), 2.0, "leaving: after it");
        // Away from a step both sides agree.
        for x in [0.0, 0.25, 0.75, 1.0] {
            assert_eq!(curve.speed_at(x), curve.speed_leaving(x), "at {x}");
            assert_eq!(curve.tangent_at(x), curve.tangent_leaving(x), "at {x}");
        }
        // The ease after the step starts flat, and is steepest half way.
        assert_eq!(curve.tangent_leaving(0.5), 0.0);
        assert!((curve.tangent_leaving(0.75) - 6.0).abs() < 1e-12);
        assert_eq!(curve.tangent_leaving(1.5), 0.0, "past the end");
    }

    #[test]
    fn a_place_that_is_not_a_number_reads_as_the_start() {
        let curve = SpeedCurve::new(&[(0.0, 0.5), (1.0, 2.0)]).unwrap();
        assert_eq!(curve.speed_at(f64::NAN), 0.5);
        assert_eq!(curve.consumed(f64::NAN), 0.0);
    }
}

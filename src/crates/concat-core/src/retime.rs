// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Speed that changes over a clip.
//!
//! A constant speed is one number. A *curve* is speed as a function of where
//! in the clip you are: a handful of points, `(x, speed)` with `x` the
//! fraction of the clip's timeline length, joined by straight lines - or,
//! when the points carry slopes, by smooth cubic bends. The source time at
//! any instant is then the area under that line up to it - which is why the
//! curve lives here, beside the clip's affine map: it is the same fact, and
//! every renderer has to agree with it.
//!
//! The units are deliberately relative. A curve stored as fractions means
//! the same thing after the clip is trimmed or the project re-timed, and a
//! preset can be applied to any clip without knowing how long it is.
//!
//! A smooth curve is a cubic Hermite spline: each point has a slope, the
//! speed's rate of change there, shared by the two stretches either side so
//! the bend is seamless. A slope of zero eases in and out of the point, the
//! shape speed ramps are usually drawn with. Slopes are held to what keeps
//! each stretch monotone (Fritsch-Carlson), so a bend never overshoots the
//! speeds it joins and never dips below the floor; and a stretch cut in two
//! is still the same cubic, so a split or a trim keeps the curve exactly.

/// Speed over a clip, as points joined by straight lines or smooth bends.
#[derive(Clone, PartialEq, Debug)]
pub struct SpeedCurve {
    /// `(x, speed)` with `x` in `0..=1` ascending and speed positive.
    points: Vec<(f64, f64)>,
    /// A slope per point, speed per unit of `x`, for a smooth curve; None
    /// joins the points with straight lines.
    slopes: Option<Vec<f64>>,
}

impl SpeedCurve {
    /// The floor a curve's speed is held at: a stop would consume no source
    /// and the map would stop being invertible.
    pub const MIN_SPEED: f64 = 0.0625;
    /// And the ceiling, matching the constant rate's.
    pub const MAX_SPEED: f64 = 16.0;

    /// A curve through these points, joined by straight lines, tidied:
    /// sorted by `x`, clamped into the unit interval and the speed range,
    /// and anchored at both ends so the line covers the whole clip. Fewer
    /// than one point is `None`.
    pub fn new(points: &[(f64, f64)]) -> Option<SpeedCurve> {
        SpeedCurve::with_slopes(points, None)
    }

    /// A curve through these points, smooth when `slopes` gives one for
    /// every point and straight otherwise, tidied as [`SpeedCurve::new`]
    /// tidies and with the slopes held to monotone stretches.
    pub fn with_slopes(points: &[(f64, f64)], slopes: Option<&[f64]>) -> Option<SpeedCurve> {
        let slopes = slopes.filter(|slopes| slopes.len() == points.len());
        let mut knots: Vec<(f64, f64, f64)> = points
            .iter()
            .enumerate()
            .filter(|(_, (x, speed))| x.is_finite() && speed.is_finite())
            .map(|(index, &(x, speed))| {
                let slope = slopes
                    .map(|slopes| slopes[index])
                    .filter(|slope| slope.is_finite())
                    .unwrap_or(0.0);
                (
                    x.clamp(0.0, 1.0),
                    speed.clamp(Self::MIN_SPEED, Self::MAX_SPEED),
                    slope,
                )
            })
            .collect();
        if knots.is_empty() {
            return None;
        }
        knots.sort_by(|a, b| a.0.total_cmp(&b.0));
        if knots[0].0 > 0.0 {
            knots.insert(0, (0.0, knots[0].1, 0.0));
        }
        if knots[knots.len() - 1].0 < 1.0 {
            knots.push((1.0, knots[knots.len() - 1].1, 0.0));
        }
        let smooth = slopes.is_some();
        if smooth {
            monotone(&mut knots);
        }
        Some(SpeedCurve {
            points: knots.iter().map(|&(x, speed, _)| (x, speed)).collect(),
            slopes: smooth.then(|| knots.iter().map(|&(_, _, slope)| slope).collect()),
        })
    }

    /// The points, as tidied.
    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }

    /// The slopes, one a point, for a smooth curve; None for straight
    /// lines.
    pub fn slopes(&self) -> Option<&[f64]> {
        self.slopes.as_deref()
    }

    /// The stretch `x` falls in, as the index of the point that starts it,
    /// and how far along it `x` is, `0..=1`. Points sharing an `x` make a
    /// step, whose stretch has no width.
    fn stretch(&self, x: f64) -> (usize, f64) {
        let last = self.points.len() - 1;
        for index in 0..last {
            let (x0, x1) = (self.points[index].0, self.points[index + 1].0);
            if x <= x1 {
                if x1 <= x0 {
                    return (index, 1.0);
                }
                return (index, (x - x0) / (x1 - x0));
            }
        }
        (last.saturating_sub(1), 1.0)
    }

    /// The ends of stretch `index`: width, speeds and, for a smooth curve,
    /// the slopes scaled to the stretch (speed per unit of `u`).
    fn ends(&self, index: usize) -> (f64, f64, f64, Option<(f64, f64)>) {
        let (x0, v0) = self.points[index];
        let (x1, v1) = self.points[index + 1];
        let width = x1 - x0;
        let slopes = self
            .slopes
            .as_ref()
            .map(|slopes| (slopes[index] * width, slopes[index + 1] * width));
        (width, v0, v1, slopes)
    }

    /// Speed at `x`, a fraction of the clip's length.
    pub fn speed_at(&self, x: f64) -> f64 {
        if self.points.len() < 2 {
            return self.points[0].1;
        }
        let x = x.clamp(0.0, 1.0);
        let (index, u) = self.stretch(x);
        let (_, v0, v1, slopes) = self.ends(index);
        match slopes {
            Some((m0, m1)) => {
                let (u2, u3) = (u * u, u * u * u);
                (2.0 * u3 - 3.0 * u2 + 1.0) * v0
                    + (u3 - 2.0 * u2 + u) * m0
                    + (-2.0 * u3 + 3.0 * u2) * v1
                    + (u3 - u2) * m1
            }
            None => v0 + (v1 - v0) * u,
        }
    }

    /// How fast the speed is changing at `x`: speed per unit of `x`. Zero
    /// past the ends, where the speed holds, and on a step.
    pub fn slope_at(&self, x: f64) -> f64 {
        if self.points.len() < 2 || !(0.0..=1.0).contains(&x) {
            return 0.0;
        }
        let (index, u) = self.stretch(x);
        let (width, v0, v1, slopes) = self.ends(index);
        if width <= 0.0 {
            return 0.0;
        }
        let per_u = match slopes {
            Some((m0, m1)) => {
                let u2 = u * u;
                (6.0 * u2 - 6.0 * u) * v0
                    + (3.0 * u2 - 4.0 * u + 1.0) * m0
                    + (-6.0 * u2 + 6.0 * u) * v1
                    + (3.0 * u2 - 2.0 * u) * m1
            }
            None => v1 - v0,
        };
        per_u / width
    }

    /// The area under stretch `index` from its start to `u` of the way
    /// along it.
    fn area(&self, index: usize, u: f64) -> f64 {
        let (width, v0, v1, slopes) = self.ends(index);
        let shape = match slopes {
            // The integrals of the four Hermite basis functions.
            Some((m0, m1)) => {
                let (u2, u3, u4) = (u * u, u * u * u, u * u * u * u);
                (u - u3 + u4 / 2.0) * v0
                    + (u4 / 4.0 - 2.0 * u3 / 3.0 + u2 / 2.0) * m0
                    + (u3 - u4 / 2.0) * v1
                    + (u4 / 4.0 - u3 / 3.0) * m1
            }
            // The trapezoid up to `u`.
            None => u * (v0 + (v0 + (v1 - v0) * u)) / 2.0,
        };
        width * shape
    }

    /// The area under the curve from the start to `x`: how much of the
    /// source, as a fraction of the clip's timeline length, has been consumed
    /// by then. Exact for both joins: a trapezoid a straight stretch, the
    /// cubic's integral a smooth one.
    pub fn consumed(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        let mut total = 0.0;
        for index in 0..self.points.len().saturating_sub(1) {
            let (x0, x1) = (self.points[index].0, self.points[index + 1].0);
            if x <= x0 {
                break;
            }
            if x1 <= x0 {
                continue;
            }
            let u = ((x - x0) / (x1 - x0)).min(1.0);
            total += self.area(index, u);
            if x <= x1 {
                break;
            }
        }
        total
    }

    /// The average speed: source seconds consumed per timeline second over
    /// the whole clip.
    pub fn mean(&self) -> f64 {
        self.consumed(1.0).max(Self::MIN_SPEED)
    }

    /// The clip cut into `count` equal timeline pieces, each `(x0, x1,
    /// consumed_at_x0, mean_speed)` - the constant-rate approximation a
    /// sound path that can only change tempo in steps needs.
    pub fn pieces(&self, count: usize) -> Vec<(f64, f64, f64, f64)> {
        let count = count.max(1);
        (0..count)
            .map(|index| {
                let x0 = index as f64 / count as f64;
                let x1 = (index + 1) as f64 / count as f64;
                let from = self.consumed(x0);
                let to = self.consumed(x1);
                (x0, x1, from, ((to - from) / (x1 - x0)).max(Self::MIN_SPEED))
            })
            .collect()
    }
}

/// Holds each stretch's slopes to a monotone cubic (Fritsch-Carlson): a
/// slope against the stretch's direction, or on a flat stretch, is zero,
/// and a pair too steep for the rise is scaled down until the cubic cannot
/// overshoot. Steps - points sharing an `x` - have no stretch to hold.
fn monotone(knots: &mut [(f64, f64, f64)]) {
    for index in 0..knots.len().saturating_sub(1) {
        let (x0, v0, _) = knots[index];
        let (x1, v1, _) = knots[index + 1];
        let width = x1 - x0;
        if width <= 0.0 {
            continue;
        }
        let rise = (v1 - v0) / width;
        if rise.abs() < 1e-12 {
            knots[index].2 = 0.0;
            knots[index + 1].2 = 0.0;
            continue;
        }
        let alpha = (knots[index].2 / rise).max(0.0);
        let beta = (knots[index + 1].2 / rise).max(0.0);
        let length = (alpha * alpha + beta * beta).sqrt();
        let tau = if length > 3.0 { 3.0 / length } else { 1.0 };
        knots[index].2 = tau * alpha * rise;
        knots[index + 1].2 = tau * beta * rise;
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
    fn a_smooth_curve_eases_through_its_points_and_its_area_is_exact() {
        // Zero slopes: each stretch is the smoothstep between its speeds.
        let curve = SpeedCurve::with_slopes(&[(0.0, 1.0), (1.0, 3.0)], Some(&[0.0, 0.0])).unwrap();
        assert!((curve.speed_at(0.5) - 2.0).abs() < 1e-12);
        assert!(
            curve.speed_at(0.1) < 1.0 + 2.0 * 0.1,
            "it leaves the point gently"
        );
        assert!(curve.slope_at(0.0).abs() < 1e-12 && curve.slope_at(1.0).abs() < 1e-12);
        // The smoothstep's area is the straight line's: symmetric about it.
        assert!((curve.mean() - 2.0).abs() < 1e-12);
        // And the area up to any point is the integral, checked by sums.
        let steps = 20_000;
        let mut sum = 0.0;
        for step in 0..steps {
            let x = (step as f64 + 0.5) / steps as f64 * 0.37;
            sum += curve.speed_at(x) * 0.37 / steps as f64;
        }
        assert!((curve.consumed(0.37) - sum).abs() < 1e-7);
    }

    #[test]
    fn a_smooth_bend_never_overshoots() {
        // Slopes far too steep for the rise are held back.
        let curve = SpeedCurve::with_slopes(
            &[(0.0, 1.0), (0.5, 4.0), (1.0, 1.0)],
            Some(&[40.0, 40.0, -40.0]),
        )
        .unwrap();
        for step in 0..=200 {
            let speed = curve.speed_at(step as f64 / 200.0);
            assert!((1.0 - 1e-9..=4.0 + 1e-9).contains(&speed), "{speed}");
        }
        // A peak's slope is flat: the bend turns there.
        assert!(curve.slope_at(0.5).abs() < 1e-9);
    }

    #[test]
    fn a_smooth_stretch_cut_in_two_is_the_same_cubic() {
        let whole = SpeedCurve::with_slopes(&[(0.0, 0.5), (1.0, 4.0)], Some(&[0.0, 0.0])).unwrap();
        // The piece over 0.3..1 of the whole, as a split's tail holds it:
        // the cut's speed and slope, re-expressed over the piece's length.
        let (a, b) = (0.3, 1.0);
        let piece = SpeedCurve::with_slopes(
            &[(0.0, whole.speed_at(a)), (1.0, whole.speed_at(b))],
            Some(&[whole.slope_at(a) * (b - a), whole.slope_at(b) * (b - a)]),
        )
        .unwrap();
        for step in 0..=10 {
            let u = step as f64 / 10.0;
            let x = a + u * (b - a);
            assert!((piece.speed_at(u) - whole.speed_at(x)).abs() < 1e-9);
        }
        let consumed = (whole.consumed(b) - whole.consumed(a)) / (b - a);
        assert!((piece.mean() - consumed).abs() < 1e-9);
    }

    #[test]
    fn slopes_that_do_not_match_the_points_make_straight_lines() {
        let curve = SpeedCurve::with_slopes(&[(0.0, 1.0), (1.0, 3.0)], Some(&[0.0])).unwrap();
        assert!(curve.slopes().is_none());
        assert!((curve.speed_at(0.25) - 1.5).abs() < 1e-12);
    }
}

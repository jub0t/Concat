// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The Speed tab's curve graph, as numbers: the scale speed is drawn on,
//! what putting down, dragging and taking off points does to a curve, the
//! Fastest and Slowest sliders, Smooth and Straight, the line drawn, and
//! the presets by name. The window draws what these say and hands every
//! gesture back here, so all of it can be tested without one.
//!
//! The graph's height is speed on a scale of doublings: 1/16x at the foot,
//! 1x half way up, 16x at the top, each doubling an eighth of the height.
//! Across is the clip, start to end, as the curve's `at` is.

use std::fmt::Write as _;

use concat_project::model::SpeedPoint;
use concat_project::model::ranges::{MAX_SPEED, MIN_SPEED};
use concat_project::speed::{self, PRESETS};

use crate::i18n::t;

/// The least room a point keeps from its neighbours across the clip, so
/// two points never land on one place by a drag and the order they are in
/// is the order they stay in.
pub const GAP: f64 = 0.002;

/// Doublings each side of 1x the graph shows: 1/16x to 16x.
const DOUBLINGS: f64 = 4.0;

/// How far from 1x a point has to be to count as faster or slower.
const NORMAL_BAND: f64 = 1e-6;

/// The Fastest slider's lowest target: a fast side cannot be brought down
/// to normal speed, where it would stop being a side.
const FASTEST_FLOOR: f64 = 1.1;

/// The Slowest slider's highest target, for the same reason.
const SLOWEST_CEILING: f64 = 0.9;

/// Steps a smooth stretch is drawn in: fine enough to look curved at the
/// graph's size, few enough to redraw on every pointer move.
const BEND_STEPS: u32 = 16;

/// Where `speed` sits up the graph, `0..=1`; held to the graph's ends.
pub fn height_of(speed: f64) -> f64 {
    if speed.is_nan() {
        return 0.5;
    }
    ((speed.log2() + DOUBLINGS) / (2.0 * DOUBLINGS)).clamp(0.0, 1.0)
}

/// The speed at `height` up the graph, held to the range; a height that is
/// not a number reads as normal speed.
pub fn speed_at_height(height: f64) -> f64 {
    if !height.is_finite() {
        return 1.0;
    }
    (height * 2.0 * DOUBLINGS - DOUBLINGS)
        .exp2()
        .clamp(MIN_SPEED, MAX_SPEED)
}

/// The curve a clip starts with when Curve is chosen: level at `speed`,
/// one point at each end, smooth, so nothing changes until a point moves.
pub fn flat(speed: f64) -> Vec<SpeedPoint> {
    let speed = speed.clamp(MIN_SPEED, MAX_SPEED);
    [0.0, 1.0]
        .into_iter()
        .map(|at| SpeedPoint {
            at,
            speed,
            slope: Some(0.0),
        })
        .collect()
}

/// Puts a point down, or moves one: `index` -1 puts one down `across` the
/// clip at `height` up the graph, anything else moves that point there.
/// Returns where the point now is in the list, None for an index that is
/// not one of the points.
///
/// A new point lands in time order, after any point at the same place, a
/// gap inside the ends so it is never mistaken for one, smooth when the
/// curve is and straight when it is not; across that is not a number puts
/// it in the middle. A moved point keeps its slope and stays a gap inside
/// its neighbours, so the order never changes under a drag; the two ends
/// move in speed alone, as the curve always spans the clip.
pub fn set_point(
    points: &mut Vec<SpeedPoint>,
    index: i32,
    across: f64,
    height: f64,
) -> Option<usize> {
    let speed = speed_at_height(height);
    let Ok(index) = usize::try_from(index) else {
        let at = if across.is_nan() {
            0.5
        } else {
            across.clamp(GAP, 1.0 - GAP)
        };
        let slope = (points.is_empty() || speed::is_smooth(points)).then_some(0.0);
        let place = points.partition_point(|held| held.at <= at);
        points.insert(place, SpeedPoint { at, speed, slope });
        return Some(place);
    };
    let last = points.len().checked_sub(1)?;
    if index > last {
        return None;
    }
    let point = &mut points[index];
    point.speed = speed;
    if index == 0 || index == last || across.is_nan() {
        return Some(index);
    }
    let low = points[index - 1].at + GAP;
    let high = points[index + 1].at - GAP;
    if low <= high {
        points[index].at = across.clamp(low, high);
    }
    Some(index)
}

/// Takes a point off. The two ends stay, and an index that is not a point
/// between them changes nothing.
pub fn remove_point(points: &mut Vec<SpeedPoint>, index: i32) {
    if let Ok(index) = usize::try_from(index)
        && index > 0
        && index + 1 < points.len()
    {
        points.remove(index);
    }
}

/// The `chosen` points of `from` moved together into `points`: `dx` across
/// the clip and `dy` up the graph. The ends move in speed alone, and the
/// shift in time is held to what keeps every moved point between the
/// points that stay, so the group keeps its shape and its order; each
/// height is held to the graph on its own.
pub fn move_chosen(
    points: &mut [SpeedPoint],
    from: &[SpeedPoint],
    chosen: &[usize],
    dx: f64,
    dy: f64,
) {
    if points.len() != from.len() || from.is_empty() || !dx.is_finite() || !dy.is_finite() {
        return;
    }
    let last = from.len() - 1;
    let moves = |index: usize| index > 0 && index < last && chosen.contains(&index);
    let (mut low, mut high) = (f64::NEG_INFINITY, f64::INFINITY);
    for index in (1..last).filter(|&index| moves(index)) {
        // The nearest points that stay, either side, and how many moving
        // ones stand between: each needs its gap.
        let before = (0..index).rev().find(|&held| !moves(held)).unwrap_or(0);
        let after = (index + 1..=last)
            .find(|&held| !moves(held))
            .unwrap_or(last);
        low = low.max(from[before].at + GAP * (index - before) as f64 - from[index].at);
        high = high.min(from[after].at - GAP * (after - index) as f64 - from[index].at);
    }
    let dx = if low <= high {
        dx.clamp(low, high)
    } else {
        0.0
    };
    for &index in chosen.iter().filter(|&&index| index <= last) {
        let point = from[index];
        points[index] = SpeedPoint {
            at: if moves(index) {
                point.at + dx
            } else {
                point.at
            },
            speed: speed_at_height(height_of(point.speed) + dy),
            slope: point.slope,
        };
    }
}

/// The fastest speed the curve reaches above normal (`faster`), or the
/// slowest below it; None when it never goes that way. A curve's extremes
/// are at its points: no stretch between two of them overshoots.
pub fn extreme_speed(points: &[SpeedPoint], faster: bool) -> Option<f64> {
    let speeds = points.iter().map(|point| point.speed);
    if faster {
        speeds
            .filter(|&speed| speed > 1.0 + NORMAL_BAND)
            .max_by(f64::total_cmp)
    } else {
        speeds
            .filter(|&speed| speed < 1.0 - NORMAL_BAND)
            .min_by(f64::total_cmp)
    }
}

/// The Fastest slider (`faster`) or the Slowest: brings that side's extreme
/// to `target` and every other point on that side of 1x along with it, by
/// raising each speed to one power. On the graph's scale of doublings that
/// stretches the side's shape towards or away from 1x without changing it,
/// so the same target twice changes nothing more. A slope follows its
/// speed by the chain rule. The target is held off 1x, so a side is never
/// flattened away; points on the other side, or at 1x, are left alone.
pub fn stretch_side(points: &mut [SpeedPoint], faster: bool, target: f64) {
    if target.is_nan() {
        return;
    }
    let target = if faster {
        target.clamp(FASTEST_FLOOR, MAX_SPEED)
    } else {
        target.clamp(MIN_SPEED, SLOWEST_CEILING)
    };
    let Some(extreme) = extreme_speed(points, faster) else {
        return;
    };
    let power = target.ln() / extreme.ln();
    for point in points.iter_mut() {
        let on_side = if faster {
            point.speed > 1.0
        } else {
            point.speed < 1.0
        };
        if !on_side {
            continue;
        }
        let speed = point.speed;
        point.speed = speed.powf(power).clamp(MIN_SPEED, MAX_SPEED);
        point.slope = point
            .slope
            .map(|slope| slope * power * speed.powf(power - 1.0));
    }
}

/// Smooth bends every point with a flat slope - easing in and out of each -
/// and Straight takes every slope away, joining the points with lines.
pub fn set_bends(points: &mut [SpeedPoint], smooth: bool) {
    for point in points {
        point.slope = smooth.then_some(0.0);
    }
}

/// The line the graph draws, as path commands on a unit square with `y`
/// down: through the curve as it plays - held level to an end the points
/// stop short of - and, where it bends, in steps along each stretch. Empty
/// for no curve.
pub fn drawn_path(points: &[SpeedPoint]) -> String {
    let Some(curve) = speed::curve_of(points) else {
        return String::new();
    };
    let knots = curve.points();
    let smooth = curve.tangents().is_some();
    let steps = if smooth { BEND_STEPS } else { 1 };
    let mut path = String::with_capacity(knots.len() * steps as usize * 16);
    let mut put = |verb: char, x: f64, speed: f64| {
        if !path.is_empty() {
            path.push(' ');
        }
        let _ = write!(path, "{verb} {x:.4} {:.4}", 1.0 - height_of(speed));
    };
    put('M', knots[0].0, knots[0].1);
    for pair in knots.windows(2) {
        let ((x0, _), (x1, v1)) = (pair[0], pair[1]);
        if x1 > x0 {
            for step in 1..steps {
                let x = x0 + (x1 - x0) * f64::from(step) / f64::from(steps);
                put('L', x, curve.speed_at(x));
            }
        }
        put('L', x1, v1);
    }
    path
}

/// The presets' names in the interface's language, in their order.
pub fn preset_labels() -> Vec<String> {
    PRESETS.iter().map(|preset| t(preset.label)).collect()
}

/// The points of the preset at `index` in that order, eased; None past
/// either end.
pub fn preset_at(index: i32) -> Option<Vec<SpeedPoint>> {
    let preset = PRESETS.get(usize::try_from(index).ok()?)?;
    speed::preset_curve(preset.id)
}

/// Where in that order the preset these points still are stands, or -1
/// for a curve shaped by hand.
pub fn preset_position(points: &[SpeedPoint]) -> i32 {
    speed::matching_preset(points)
        .and_then(|id| PRESETS.iter().position(|preset| preset.id == id))
        .map_or(-1, |index| index as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chosen_points_move_together_and_stop_at_the_points_that_stay() {
        let mut from = flat(1.0);
        for at in [0.2, 0.4, 0.6] {
            set_point(&mut from, -1, at, 0.5);
        }
        // 0 · 0.2 · 0.4 · 0.6 · 1, the middle two chosen.
        let mut points = from.clone();
        move_chosen(&mut points, &from, &[2, 3], 0.1, 0.125);
        assert!((points[2].at - 0.5).abs() < 1e-9 && (points[3].at - 0.7).abs() < 1e-9);
        assert!((points[2].speed - 2.0).abs() < 1e-9 && (points[3].speed - 2.0).abs() < 1e-9);
        assert_eq!(points[1], from[1]);
        // Pushed far right, the last moved point stops a gap short of the end,
        // and the one before keeps its distance.
        move_chosen(&mut points, &from, &[2, 3], 5.0, 0.0);
        assert!((points[3].at - (1.0 - GAP)).abs() < 1e-9);
        assert!((points[3].at - points[2].at - 0.2).abs() < 1e-9);
        // An end chosen moves in speed only.
        move_chosen(&mut points, &from, &[0], 0.3, -0.125);
        assert_eq!(points[0].at, 0.0);
        assert!((points[0].speed - 0.5).abs() < 1e-9);
    }
}

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
use concat_project::speed::{self, SPEED_PRESETS};

use crate::i18n::t;

/// The least room a point keeps from its neighbours across the clip, so
/// two points never land on one place by a drag and the order they are in
/// is the order they stay in.
pub const GAP: f64 = 0.002;

/// Doublings each side of 1x the graph shows: 1/16x to 16x.
const OCTAVES: f64 = 4.0;

/// How far from 1x a point has to be to count as faster or slower.
const UNITY_BAND: f64 = 1e-6;

/// The Fastest slider's lowest target: a fast side cannot be brought down
/// to normal speed, where it would stop being a side.
const FAST_FLOOR: f64 = 1.1;

/// The Slowest slider's highest target, for the same reason.
const SLOW_CEILING: f64 = 0.9;

/// Steps a smooth stretch is drawn in: fine enough to look curved at the
/// graph's size, few enough to redraw on every pointer move.
const SMOOTH_STEPS: u32 = 16;

/// Where `speed` sits up the graph, `0..=1`; held to the graph's ends.
pub fn height_for(speed: f64) -> f64 {
    if speed.is_nan() {
        return 0.5;
    }
    ((speed.log2() + OCTAVES) / (2.0 * OCTAVES)).clamp(0.0, 1.0)
}

/// The speed at `height` up the graph, held to the range; a height that is
/// not a number reads as normal speed.
pub fn speed_at_height(height: f64) -> f64 {
    if !height.is_finite() {
        return 1.0;
    }
    (height * 2.0 * OCTAVES - OCTAVES)
        .exp2()
        .clamp(MIN_SPEED, MAX_SPEED)
}

/// The curve a clip starts with when Curve is chosen: level at `speed`,
/// one point at each end, smooth, so nothing changes until a point moves.
pub fn level_curve(speed: f64) -> Vec<SpeedPoint> {
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

/// Puts a point down, or moves one: `slot` -1 puts one down `across` the
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
pub fn place_point(
    points: &mut Vec<SpeedPoint>,
    slot: i32,
    across: f64,
    height: f64,
) -> Option<usize> {
    let speed = speed_at_height(height);
    let Ok(slot) = usize::try_from(slot) else {
        let at = if across.is_nan() {
            0.5
        } else {
            across.clamp(GAP, 1.0 - GAP)
        };
        let slope = (points.is_empty() || speed::is_eased(points)).then_some(0.0);
        let place = points.partition_point(|other| other.at <= at);
        points.insert(place, SpeedPoint { at, speed, slope });
        return Some(place);
    };
    let final_slot = points.len().checked_sub(1)?;
    if slot > final_slot {
        return None;
    }
    let node = &mut points[slot];
    node.speed = speed;
    if slot == 0 || slot == final_slot || across.is_nan() {
        return Some(slot);
    }
    let low = points[slot - 1].at + GAP;
    let high = points[slot + 1].at - GAP;
    if low <= high {
        points[slot].at = across.clamp(low, high);
    }
    Some(slot)
}

/// Takes a point off. The two ends stay, and an index that is not a point
/// between them changes nothing.
pub fn drop_point(points: &mut Vec<SpeedPoint>, slot: i32) {
    if let Ok(slot) = usize::try_from(slot)
        && slot > 0
        && slot + 1 < points.len()
    {
        points.remove(slot);
    }
}

/// The `picked` points of `from` moved together into `points`: `dx` across
/// the clip and `dy` up the graph. The ends move in speed alone, and the
/// shift in time is held to what keeps every moved point between the
/// points that stay, so the group keeps its shape and its order; each
/// height is held to the graph on its own.
pub fn move_selected(
    points: &mut [SpeedPoint],
    from: &[SpeedPoint],
    picked: &[usize],
    dx: f64,
    dy: f64,
) {
    if points.len() != from.len() || from.is_empty() || !dx.is_finite() || !dy.is_finite() {
        return;
    }
    let final_slot = from.len() - 1;
    let is_moving = |slot: usize| slot > 0 && slot < final_slot && picked.contains(&slot);
    let (mut low, mut high) = (f64::NEG_INFINITY, f64::INFINITY);
    for slot in (1..final_slot).filter(|&slot| is_moving(slot)) {
        // The nearest points that stay, either side, and how many moving
        // ones stand between: each needs its gap.
        let prior = (0..slot)
            .rev()
            .find(|&other| !is_moving(other))
            .unwrap_or(0);
        let next_fixed = (slot + 1..=final_slot)
            .find(|&other| !is_moving(other))
            .unwrap_or(final_slot);
        low = low.max(from[prior].at + GAP * (slot - prior) as f64 - from[slot].at);
        high = high.min(from[next_fixed].at - GAP * (next_fixed - slot) as f64 - from[slot].at);
    }
    let dx = if low <= high {
        dx.clamp(low, high)
    } else {
        0.0
    };
    for &slot in picked.iter().filter(|&&slot| slot <= final_slot) {
        let node = from[slot];
        points[slot] = SpeedPoint {
            at: if is_moving(slot) {
                node.at + dx
            } else {
                node.at
            },
            speed: speed_at_height(height_for(node.speed) + dy),
            slope: node.slope,
        };
    }
}

/// The fastest speed the curve reaches above normal (`upward`), or the
/// slowest below it; None when it never goes that way. A curve's extremes
/// are at its points: no stretch between two of them overshoots.
pub fn extreme_of(points: &[SpeedPoint], upward: bool) -> Option<f64> {
    let speeds = points.iter().map(|node| node.speed);
    if upward {
        speeds
            .filter(|&speed| speed > 1.0 + UNITY_BAND)
            .max_by(f64::total_cmp)
    } else {
        speeds
            .filter(|&speed| speed < 1.0 - UNITY_BAND)
            .min_by(f64::total_cmp)
    }
}

/// The Fastest slider (`upward`) or the Slowest: brings that side's extreme
/// to `target` and every other point on that side of 1x along with it, by
/// raising each speed to one power. On the graph's scale of doublings that
/// stretches the side's shape towards or away from 1x without changing it,
/// so the same target twice changes nothing more. A slope follows its
/// speed by the chain rule. The target is held off 1x, so a side is never
/// flattened away; points on the other side, or at 1x, are left alone.
pub fn stretch_extreme(points: &mut [SpeedPoint], upward: bool, target: f64) {
    if target.is_nan() {
        return;
    }
    let target = if upward {
        target.clamp(FAST_FLOOR, MAX_SPEED)
    } else {
        target.clamp(MIN_SPEED, SLOW_CEILING)
    };
    let Some(extreme) = extreme_of(points, upward) else {
        return;
    };
    let power = target.ln() / extreme.ln();
    for node in points.iter_mut() {
        let on_side = if upward {
            node.speed > 1.0
        } else {
            node.speed < 1.0
        };
        if !on_side {
            continue;
        }
        let speed = node.speed;
        node.speed = speed.powf(power).clamp(MIN_SPEED, MAX_SPEED);
        node.slope = node
            .slope
            .map(|slope| slope * power * speed.powf(power - 1.0));
    }
}

/// Smooth bends every point with a flat slope - easing in and out of each -
/// and Straight takes every slope away, joining the points with lines.
pub fn apply_easing(points: &mut [SpeedPoint], eased_flag: bool) {
    for node in points {
        node.slope = eased_flag.then_some(0.0);
    }
}

/// The line the graph draws, as path commands on a unit square with `y`
/// down: through the curve as it plays - held level to an end the points
/// stop short of - and, where it bends, in steps along each stretch. Empty
/// for no curve.
pub fn curve_path(points: &[SpeedPoint]) -> String {
    let Some(curve) = speed::engine_curve(points) else {
        return String::new();
    };
    let vertices = curve.points();
    let eased_flag = curve.tangents().is_some();
    let segments = if eased_flag { SMOOTH_STEPS } else { 1 };
    let mut path = String::with_capacity(vertices.len() * segments as usize * 16);
    let mut put = |verb: char, x: f64, speed: f64| {
        if !path.is_empty() {
            path.push(' ');
        }
        let _ = write!(path, "{verb} {x:.4} {:.4}", 1.0 - height_for(speed));
    };
    put('M', vertices[0].0, vertices[0].1);
    for pair in vertices.windows(2) {
        let ((x0, _), (x1, v1)) = (pair[0], pair[1]);
        if x1 > x0 {
            for step in 1..segments {
                let x = x0 + (x1 - x0) * f64::from(step) / f64::from(segments);
                put('L', x, curve.speed_at(x));
            }
        }
        put('L', x1, v1);
    }
    path
}

/// The presets' names in the interface's language, in their order.
pub fn preset_labels() -> Vec<String> {
    SPEED_PRESETS.iter().map(|preset| t(preset.label)).collect()
}

/// The points of the preset at `slot` in that order, eased; None past
/// either end.
pub fn preset_by_index(slot: i32) -> Option<Vec<SpeedPoint>> {
    let preset = SPEED_PRESETS.get(usize::try_from(slot).ok()?)?;
    speed::preset_points(preset.id)
}

/// Where in that order the preset these points still are stands, or -1
/// for a curve shaped by hand.
pub fn preset_index(points: &[SpeedPoint]) -> i32 {
    speed::preset_matching(points)
        .and_then(|id| SPEED_PRESETS.iter().position(|preset| preset.id == id))
        .map_or(-1, |slot| slot as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chosen_points_move_together_and_stop_at_the_points_that_stay() {
        let mut from = level_curve(1.0);
        for at in [0.2, 0.4, 0.6] {
            place_point(&mut from, -1, at, 0.5);
        }
        // 0 · 0.2 · 0.4 · 0.6 · 1, the middle two chosen.
        let mut points = from.clone();
        move_selected(&mut points, &from, &[2, 3], 0.1, 0.125);
        assert!((points[2].at - 0.5).abs() < 1e-9 && (points[3].at - 0.7).abs() < 1e-9);
        assert!((points[2].speed - 2.0).abs() < 1e-9 && (points[3].speed - 2.0).abs() < 1e-9);
        assert_eq!(points[1], from[1]);
        // Pushed far right, the last moved point stops a gap short of the end,
        // and the one before keeps its distance.
        move_selected(&mut points, &from, &[2, 3], 5.0, 0.0);
        assert!((points[3].at - (1.0 - GAP)).abs() < 1e-9);
        assert!((points[3].at - points[2].at - 0.2).abs() < 1e-9);
        // An end chosen moves in speed only.
        move_selected(&mut points, &from, &[0], 0.3, -0.125);
        assert_eq!(points[0].at, 0.0);
        assert!((points[0].speed - 0.5).abs() < 1e-9);
    }
}

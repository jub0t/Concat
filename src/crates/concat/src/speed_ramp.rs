// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The Speed tab's curve, as the inspector reads and edits it.
//!
//! The document holds a curve as `(at, speed)` points over the clip (see
//! concat-project's `speed`); the graph shows speed on a scale of
//! doublings, so a point's height is `log2(speed)` mapped from the engine's
//! range, 1/16x to 16x, onto `0..1`. Here is the arithmetic between the two
//! and the edits a drag makes; what the speed does to the picture is the
//! engine's `SpeedCurve`.

use concat_project::model::SpeedPoint;
use concat_project::model::ranges::{MAX_SPEED, MIN_SPEED};
use concat_project::speed::{PRESETS, curve_of, preset_of};

use crate::i18n::t;

/// The two ends' heights on the graph, in doublings either side of 1x.
const DOUBLINGS: f64 = 4.0;
/// How close two points may come in time: a hair, so a drag can make the
/// near-vertical step of a jump without the two swapping places.
const GAP: f64 = 0.002;

/// Where `speed` sits on the graph, `0..1`.
pub fn height_of(speed: f64) -> f64 {
    ((speed.clamp(MIN_SPEED, MAX_SPEED).log2() + DOUBLINGS) / (2.0 * DOUBLINGS)).clamp(0.0, 1.0)
}

/// The speed at height `y` of the graph.
pub fn speed_at_height(y: f64) -> f64 {
    let y = if y.is_finite() {
        y.clamp(0.0, 1.0)
    } else {
        0.5
    };
    (y * 2.0 * DOUBLINGS - DOUBLINGS)
        .exp2()
        .clamp(MIN_SPEED, MAX_SPEED)
}

/// A curve that is one rate the whole way, smooth: what Curve starts
/// from, so switching to it changes nothing until a point moves.
pub fn flat(speed: f64) -> Vec<SpeedPoint> {
    let point = |at| SpeedPoint {
        at,
        speed,
        slope: Some(0.0),
    };
    vec![point(0.0), point(1.0)]
}

/// The curve made smooth - easing in and out of every point - or joined
/// by straight lines.
pub fn set_smooth(points: &mut [SpeedPoint], smooth: bool) {
    for point in points {
        point.slope = smooth.then_some(0.0);
    }
}

/// The fastest the curve goes, when it goes faster than 1x anywhere, or
/// the slowest, when it goes slower: what the Speed tab's two sliders
/// show. A curve's extremes are at its points, since no stretch overshoots.
pub fn extreme(points: &[SpeedPoint], faster: bool) -> Option<f64> {
    let speeds = points.iter().map(|point| point.speed);
    if faster {
        speeds
            .fold(None, |most: Option<f64>, speed| {
                Some(most.map_or(speed, |most| most.max(speed)))
            })
            .filter(|&most| most > 1.0 + 1e-6)
    } else {
        speeds
            .fold(None, |least: Option<f64>, speed| {
                Some(least.map_or(speed, |least| least.min(speed)))
            })
            .filter(|&least| least < 1.0 - 1e-6)
    }
}

/// The nearest a slider may bring its extreme to 1x: the stretch past 1x
/// is then still there to be shown and pulled out again.
pub const NEAREST: f64 = 0.1;

/// The fast stretches of the curve (or the slow ones) brought to
/// `target` at their extreme, the rest of each stretch following in
/// proportion on the graph's scale: a Flash in that peaks at 8x and is
/// brought to 4x passes 2.5x where it passed 4x. The speeds at or the other
/// side of 1x stay as they are. Each point's slope follows its speed, so a
/// smooth curve keeps its shape.
pub fn set_extreme(points: &mut [SpeedPoint], faster: bool, target: f64) {
    let Some(extreme) = extreme(points, faster) else {
        return;
    };
    let target = if faster {
        target.clamp(1.0 + NEAREST, MAX_SPEED)
    } else {
        target.clamp(MIN_SPEED, 1.0 - NEAREST)
    };
    if !target.is_finite() {
        return;
    }
    // Speeds on the graph's scale are logarithms, so bringing the extreme
    // to the target is one power over every speed on its side of 1x.
    let power = target.ln() / extreme.ln();
    for point in points {
        let beyond = if faster {
            point.speed > 1.0
        } else {
            point.speed < 1.0
        };
        if !beyond {
            continue;
        }
        let speed = point.speed.powf(power).clamp(MIN_SPEED, MAX_SPEED);
        point.slope = point
            .slope
            .map(|slope| slope * power * point.speed.powf(power - 1.0));
        point.speed = speed;
    }
}

/// Point `index` moved to `(x, y)` on the graph, or a new point put down
/// there when `index` is negative. The ends stay at the clip's ends and a
/// point between them stays between its neighbours, so the indices the
/// graph holds still name the same points mid-drag.
pub fn set_point(points: &mut Vec<SpeedPoint>, index: i32, x: f64, y: f64) {
    let speed = speed_at_height(y);
    let x = if x.is_finite() {
        x.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let Ok(index) = usize::try_from(index) else {
        // A new point goes in where it falls in time, never on an end.
        let x = x.clamp(GAP, 1.0 - GAP);
        let at = points.partition_point(|point| point.at <= x);
        // It bends as the rest of the curve does.
        let slope = concat_project::speed::is_smooth(points).then_some(0.0);
        points.insert(
            at,
            SpeedPoint {
                at: x,
                speed,
                slope,
            },
        );
        return;
    };
    let last = points.len().saturating_sub(1);
    if index > last {
        return;
    }
    let at = if index == 0 {
        0.0
    } else if index == last {
        1.0
    } else {
        x.clamp(points[index - 1].at + GAP, points[index + 1].at - GAP)
    };
    points[index] = SpeedPoint {
        at,
        speed,
        slope: points[index].slope,
    };
}

/// Point `index` taken off. The two ends stay: a curve spans the clip.
pub fn remove_point(points: &mut Vec<SpeedPoint>, index: i32) {
    match usize::try_from(index) {
        Ok(index) if index > 0 && index + 1 < points.len() => {
            points.remove(index);
        }
        _ => {}
    }
}

/// The curve drawn in a unit box, y downwards, on the graph's scale: a
/// line through its points, and for a smooth curve sixteen steps along
/// each bend between them.
pub fn path(points: &[SpeedPoint]) -> String {
    let Some(curve) = curve_of(points) else {
        return String::new();
    };
    let smooth = curve.slopes().is_some();
    let knots = curve.points();
    let mut path = String::new();
    let mut to = |verb: char, x: f64, speed: f64| {
        let y = 1.0 - height_of(speed);
        path.push_str(&format!("{verb} {x:.4} {y:.4} "));
    };
    to('M', knots[0].0, knots[0].1);
    for pair in knots.windows(2) {
        let ((x0, _), (x1, speed)) = (pair[0], pair[1]);
        if smooth && x1 > x0 {
            for step in 1..16 {
                let x = x0 + (x1 - x0) * f64::from(step) / 16.0;
                to('L', x, curve.speed_at(x));
            }
        }
        to('L', x1, speed);
    }
    path.trim_end().to_owned()
}

/// The presets' names, in the order [`PRESETS`] lists them.
pub fn preset_names() -> Vec<String> {
    PRESETS
        .iter()
        .map(|preset| match preset.id {
            "montage" => t("speedRamp.montage"),
            "hero" => t("speedRamp.hero"),
            "bullet" => t("speedRamp.bullet"),
            "jump-cut" => t("speedRamp.jumpCut"),
            "flash-in" => t("speedRamp.flashIn"),
            "flash-out" => t("speedRamp.flashOut"),
            other => other.to_owned(),
        })
        .collect()
}

/// The preset these points still are, as an index into [`PRESETS`], or -1
/// once they have been shaped by hand.
pub fn preset_index(points: &[SpeedPoint]) -> i32 {
    preset_of(points)
        .and_then(|id| PRESETS.iter().position(|preset| preset.id == id))
        .map_or(-1, |index| index as i32)
}

/// The points of preset `index`, or None for an index past the list.
pub fn preset_points(index: i32) -> Option<Vec<SpeedPoint>> {
    let preset = PRESETS.get(usize::try_from(index).ok()?)?;
    concat_project::speed::preset(preset.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scale_is_doublings_with_one_in_the_middle() {
        assert!((height_of(1.0) - 0.5).abs() < 1e-12);
        assert!((height_of(MIN_SPEED)).abs() < 1e-12);
        assert!((height_of(MAX_SPEED) - 1.0).abs() < 1e-12);
        assert!((height_of(2.0) - 0.5 - (0.5 - height_of(0.5))).abs() < 1e-12);
        for speed in [0.1, 0.5, 1.0, 3.0, 12.0] {
            assert!((speed_at_height(height_of(speed)) - speed).abs() < 1e-9);
        }
    }

    #[test]
    fn the_ends_stay_put_and_the_middle_stays_between() {
        let mut points = flat(1.0);
        set_point(&mut points, -1, 0.5, height_of(4.0));
        assert_eq!(points.len(), 3);
        assert!((points[1].speed - 4.0).abs() < 1e-9);
        // An end only moves in speed.
        set_point(&mut points, 0, 0.4, height_of(2.0));
        assert_eq!(points[0].at, 0.0);
        assert!((points[0].speed - 2.0).abs() < 1e-9);
        // The middle cannot pass an end.
        set_point(&mut points, 1, 1.0, 0.5);
        assert!(points[1].at < 1.0);
        // The ends do not come off; the middle does.
        remove_point(&mut points, 0);
        remove_point(&mut points, 2);
        assert_eq!(points.len(), 3);
        remove_point(&mut points, 1);
        assert_eq!(points.len(), 2);
    }

    #[test]
    fn a_slider_brings_the_extreme_and_its_side_along() {
        let mut points = preset_points(4).unwrap(); // Flash in: 8x, 4x, 1x, 1x
        assert_eq!(extreme(&points, true), Some(8.0));
        assert_eq!(extreme(&points, false), None, "nothing is slower than 1x");
        set_extreme(&mut points, true, 4.0);
        assert!((points[0].speed - 4.0).abs() < 1e-9);
        // 8x is three doublings and 4x two: every fast speed keeps two
        // thirds of its doublings, so 4x becomes 2^(4/3).
        assert!(
            (points[1].speed - 4f64.powf(2.0 / 3.0)).abs() < 1e-9,
            "in proportion on the scale"
        );
        assert!((points[2].speed - 1.0).abs() < 1e-12, "1x stays");
        // Absolute, not relative: the same target twice changes nothing.
        let once = points.clone();
        set_extreme(&mut points, true, 4.0);
        for (again, before) in points.iter().zip(&once) {
            assert!((again.speed - before.speed).abs() < 1e-9);
        }
        // It never flattens the stretch away.
        set_extreme(&mut points, true, 1.0);
        assert!(extreme(&points, true).is_some());
    }

    #[test]
    fn smooth_or_straight_is_every_point_or_none() {
        let mut points = flat(1.0);
        assert!(concat_project::speed::is_smooth(&points));
        set_point(&mut points, -1, 0.5, 0.8);
        assert!(concat_project::speed::is_smooth(&points));
        set_smooth(&mut points, false);
        assert!(points.iter().all(|point| point.slope.is_none()));
        set_point(&mut points, -1, 0.3, 0.2);
        assert!(points.iter().all(|point| point.slope.is_none()));
        // A smooth path bends; a straight one is its points.
        assert_eq!(path(&points).matches('L').count(), points.len() - 1);
        set_smooth(&mut points, true);
        assert!(path(&points).matches('L').count() > points.len() * 8);
    }

    #[test]
    fn a_new_point_lands_in_time_order() {
        let mut points = flat(1.0);
        set_point(&mut points, -1, 0.7, 0.5);
        set_point(&mut points, -1, 0.2, 0.5);
        let ats: Vec<f64> = points.iter().map(|point| point.at).collect();
        assert_eq!(ats, vec![0.0, 0.2, 0.7, 1.0]);
    }

    #[test]
    fn a_preset_is_named_until_it_is_shaped() {
        let mut points = preset_points(1).unwrap();
        assert_eq!(preset_index(&points), 1);
        let at = points[2].at;
        set_point(&mut points, 2, at, 0.9);
        assert_eq!(preset_index(&points), -1);
        assert!(preset_points(99).is_none());
    }
}

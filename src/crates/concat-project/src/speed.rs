// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Speed curves: the presets, and the arithmetic the commands share.
//!
//! The shapes are the ones editors' users know by name. Each is a handful
//! of `(at, speed)` points over the clip, joined by straight lines; the
//! engine's [`SpeedCurve`] turns them into a time map.

use concat_core::SpeedCurve;

use crate::model::SpeedPoint;

/// The named shapes, in the order a menu lists them.
pub const PRESETS: &[(&str, &[(f64, f64)])] = &[
    // Fast, then a breath, then fast again, twice: a cut that keeps moving.
    (
        "Montage",
        &[
            (0.0, 1.0),
            (0.2, 2.0),
            (0.35, 0.5),
            (0.5, 2.0),
            (0.65, 0.5),
            (0.8, 2.0),
            (1.0, 1.0),
        ],
    ),
    // Normal in, a held slow moment in the middle, normal out.
    (
        "Hero",
        &[
            (0.0, 1.0),
            (0.35, 1.0),
            (0.5, 0.25),
            (0.65, 1.0),
            (1.0, 1.0),
        ],
    ),
    // Fast, slammed to slow, snapped back to fast.
    (
        "Bullet",
        &[(0.0, 3.0), (0.4, 3.0), (0.5, 0.3), (0.6, 3.0), (1.0, 3.0)],
    ),
    // Normal with a skip through the middle.
    (
        "Jump Cut",
        &[(0.0, 1.0), (0.45, 1.0), (0.5, 6.0), (0.55, 1.0), (1.0, 1.0)],
    ),
    // Rushes in, settles to normal.
    (
        "Flash In",
        &[(0.0, 4.0), (0.2, 4.0), (0.35, 1.0), (1.0, 1.0)],
    ),
    // Normal, then rushes out.
    (
        "Flash Out",
        &[(0.0, 1.0), (0.65, 1.0), (0.8, 4.0), (1.0, 4.0)],
    ),
];

/// The preset at `index`, or None past the end.
pub fn preset(index: usize) -> Option<Vec<SpeedPoint>> {
    PRESETS.get(index).map(|(_, points)| {
        points
            .iter()
            .map(|&(at, speed)| SpeedPoint { at, speed })
            .collect()
    })
}

/// Which preset `curve` is, or None for a custom shape.
pub fn preset_of(curve: &[SpeedPoint]) -> Option<usize> {
    PRESETS.iter().position(|(_, points)| {
        points.len() == curve.len()
            && points.iter().zip(curve).all(|(&(at, speed), point)| {
                (point.at - at).abs() < 1e-9 && (point.speed - speed).abs() < 1e-9
            })
    })
}

/// The engine's curve for these points, or None when they make no curve.
pub fn curve_of(points: &[SpeedPoint]) -> Option<SpeedCurve> {
    let raw: Vec<(f64, f64)> = points
        .iter()
        .map(|point: &SpeedPoint| (point.at, point.speed))
        .collect();
    SpeedCurve::new(&raw)
}

/// Source seconds per timeline second over a whole clip with these points;
/// the constant rate's equivalent for a curve.
pub fn mean_of(points: &[SpeedPoint]) -> f64 {
    curve_of(points).map(|curve| curve.mean()).unwrap_or(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip_and_average_sensibly() {
        for (index, (name, _)) in PRESETS.iter().enumerate() {
            let points = preset(index).expect("in range");
            assert_eq!(preset_of(&points), Some(index), "{name}");
            let mean = mean_of(&points);
            assert!(mean > 0.2 && mean < 4.0, "{name} averages {mean}");
        }
        assert!(preset(PRESETS.len()).is_none());
        assert_eq!(
            preset_of(&[SpeedPoint {
                at: 0.0,
                speed: 7.0
            }]),
            None
        );
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Speed curves: the arithmetic the commands share.
//!
//! A curve is a handful of `(at, speed)` points over the clip, joined by
//! straight lines; the engine's [`SpeedCurve`] turns them into a time map.

use concat_core::SpeedCurve;

use crate::model::SpeedPoint;

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

/// Named speed curve presets matching CapCut and industry speed ramping standards.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpeedPreset {
    Montage,
    Hero,
    BulletTime,
    JumpCut,
    FlashIn,
    FlashOut,
}

impl SpeedPreset {
    /// Returns the points defining the curve.
    pub fn points(self) -> Vec<SpeedPoint> {
        match self {
            SpeedPreset::Montage => vec![
                SpeedPoint { at: 0.0, speed: 2.5 },
                SpeedPoint { at: 0.2, speed: 0.5 },
                SpeedPoint { at: 0.4, speed: 2.5 },
                SpeedPoint { at: 0.7, speed: 0.4 },
                SpeedPoint { at: 1.0, speed: 2.5 },
            ],
            SpeedPreset::Hero => vec![
                SpeedPoint { at: 0.0, speed: 1.0 },
                SpeedPoint { at: 0.25, speed: 2.8 },
                SpeedPoint { at: 0.45, speed: 0.3 },
                SpeedPoint { at: 0.7, speed: 0.3 },
                SpeedPoint { at: 1.0, speed: 1.0 },
            ],
            SpeedPreset::BulletTime => vec![
                SpeedPoint { at: 0.0, speed: 1.2 },
                SpeedPoint { at: 0.3, speed: 1.2 },
                SpeedPoint { at: 0.42, speed: 0.2 },
                SpeedPoint { at: 0.58, speed: 0.2 },
                SpeedPoint { at: 0.7, speed: 1.2 },
                SpeedPoint { at: 1.0, speed: 1.2 },
            ],
            SpeedPreset::JumpCut => vec![
                SpeedPoint { at: 0.0, speed: 3.0 },
                SpeedPoint { at: 0.25, speed: 0.8 },
                SpeedPoint { at: 0.5, speed: 3.0 },
                SpeedPoint { at: 0.75, speed: 0.8 },
                SpeedPoint { at: 1.0, speed: 3.0 },
            ],
            SpeedPreset::FlashIn => vec![
                SpeedPoint { at: 0.0, speed: 4.0 },
                SpeedPoint { at: 0.3, speed: 1.8 },
                SpeedPoint { at: 0.6, speed: 1.0 },
                SpeedPoint { at: 1.0, speed: 1.0 },
            ],
            SpeedPreset::FlashOut => vec![
                SpeedPoint { at: 0.0, speed: 1.0 },
                SpeedPoint { at: 0.5, speed: 1.0 },
                SpeedPoint { at: 0.8, speed: 1.8 },
                SpeedPoint { at: 1.0, speed: 4.0 },
            ],
        }
    }

    /// Converts a 1-based preset index to a `SpeedPreset`, with 0 meaning None.
    pub fn from_index(index: i32) -> Option<Self> {
        match index {
            1 => Some(SpeedPreset::Montage),
            2 => Some(SpeedPreset::Hero),
            3 => Some(SpeedPreset::BulletTime),
            4 => Some(SpeedPreset::JumpCut),
            5 => Some(SpeedPreset::FlashIn),
            6 => Some(SpeedPreset::FlashOut),
            _ => None,
        }
    }

    /// Converts a `SpeedPreset` to its 1-based index.
    pub fn to_index(self) -> i32 {
        match self {
            SpeedPreset::Montage => 1,
            SpeedPreset::Hero => 2,
            SpeedPreset::BulletTime => 3,
            SpeedPreset::JumpCut => 4,
            SpeedPreset::FlashIn => 5,
            SpeedPreset::FlashOut => 6,
        }
    }
}

/// Identifies if a list of points matches a known preset, returning its index (1-6) or 0.
pub fn detect_preset(points: &[SpeedPoint]) -> i32 {
    let presets = [
        (1, SpeedPreset::Montage),
        (2, SpeedPreset::Hero),
        (3, SpeedPreset::BulletTime),
        (4, SpeedPreset::JumpCut),
        (5, SpeedPreset::FlashIn),
        (6, SpeedPreset::FlashOut),
    ];
    for (idx, preset) in presets {
        let p = preset.points();
        if p.len() == points.len() {
            let matched = p.iter().zip(points.iter()).all(|(a, b)| {
                (a.at - b.at).abs() < 1e-4 && (a.speed - b.speed).abs() < 1e-4
            });
            if matched {
                return idx;
            }
        }
    }
    0
}

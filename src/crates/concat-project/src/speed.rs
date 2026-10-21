// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Speed curves: the arithmetic the commands share.
//!
//! A curve is a handful of `(at, speed)` points over the clip, joined by
//! straight lines; the engine's [`SpeedCurve`] turns them into a time map.

use concat_core::SpeedCurve;

use crate::model::ranges::{MAX_SPEED, MIN_SPEED};
use crate::model::{Clip, SpeedPoint};

/// The engine's curve for these points, or None when they make no curve.
/// Smooth when every point carries a slope, straight otherwise.
pub fn curve_of(points: &[SpeedPoint]) -> Option<SpeedCurve> {
    let raw: Vec<(f64, f64)> = points
        .iter()
        .map(|point: &SpeedPoint| (point.at, point.speed))
        .collect();
    let slopes: Option<Vec<f64>> = points.iter().map(|point| point.slope).collect();
    SpeedCurve::with_slopes(&raw, slopes.as_deref())
}

/// Whether these points bend smoothly rather than meet in straight lines.
pub fn is_smooth(points: &[SpeedPoint]) -> bool {
    !points.is_empty() && points.iter().all(|point| point.slope.is_some())
}

/// Source seconds per timeline second over a whole clip with these points;
/// the constant rate's equivalent for a curve.
pub fn mean_of(points: &[SpeedPoint]) -> f64 {
    curve_of(points).map(|curve| curve.mean()).unwrap_or(1.0)
}

/// A shape a clip's speed can be given in one go: points over the clip's
/// length, so the same preset fits a clip of any length. Choosing one
/// writes ordinary points, which are the clip's own from then on.
pub struct SpeedPreset {
    /// The name the window looks its label up by.
    pub id: &'static str,
    /// `(at, speed)`, ascending. Straight lines join them, so a bend that
    /// should read as smooth is given a point either side of it.
    pub points: &'static [(f64, f64)],
}

/// The presets, in the order the window lists them. The names are the
/// ones people bring from other editors.
pub const PRESETS: &[SpeedPreset] = &[
    // Slow, fast, slow, fast: a rhythm for a sequence of shots.
    SpeedPreset {
        id: "montage",
        points: &[
            (0.0, 0.6),
            (0.15, 0.6),
            (0.3, 3.0),
            (0.45, 0.6),
            (0.6, 0.6),
            (0.75, 3.0),
            (0.9, 0.6),
            (1.0, 0.6),
        ],
    },
    // Fast in, a long slow moment in the middle, fast out.
    SpeedPreset {
        id: "hero",
        points: &[
            (0.0, 2.5),
            (0.25, 2.5),
            (0.38, 0.5),
            (0.62, 0.5),
            (0.75, 2.5),
            (1.0, 2.5),
        ],
    },
    // Fast, then nearly stopped, then fast: the bullet-time moment.
    SpeedPreset {
        id: "bullet",
        points: &[
            (0.0, 3.0),
            (0.3, 3.0),
            (0.42, 0.2),
            (0.58, 0.2),
            (0.7, 3.0),
            (1.0, 3.0),
        ],
    },
    // Normal, a short burst very fast through the middle, normal.
    SpeedPreset {
        id: "jump-cut",
        points: &[
            (0.0, 1.0),
            (0.38, 1.0),
            (0.45, 8.0),
            (0.55, 8.0),
            (0.62, 1.0),
            (1.0, 1.0),
        ],
    },
    // Rushing in, settling to normal.
    SpeedPreset {
        id: "flash-in",
        points: &[(0.0, 8.0), (0.12, 4.0), (0.3, 1.0), (1.0, 1.0)],
    },
    // Normal, then rushing out.
    SpeedPreset {
        id: "flash-out",
        points: &[(0.0, 1.0), (0.7, 1.0), (0.88, 4.0), (1.0, 8.0)],
    },
];

/// The points of the preset called `id`, smooth - easing in and out of
/// every point, the way ramps are drawn - or None for a name there is no
/// preset by.
pub fn preset(id: &str) -> Option<Vec<SpeedPoint>> {
    PRESETS.iter().find(|preset| preset.id == id).map(|preset| {
        preset
            .points
            .iter()
            .map(|&(at, speed)| SpeedPoint {
                at,
                speed,
                slope: Some(0.0),
            })
            .collect()
    })
}

/// Which preset these points are, unchanged but for smooth or straight,
/// or None for a curve of the clip's own: what the window's picker shows
/// as chosen.
pub fn preset_of(points: &[SpeedPoint]) -> Option<&'static str> {
    PRESETS
        .iter()
        .find(|preset| {
            preset.points.len() == points.len()
                && preset
                    .points
                    .iter()
                    .zip(points)
                    .all(|(&(at, speed), point)| {
                        (at - point.at).abs() < 1e-9 && (speed - point.speed).abs() < 1e-9
                    })
        })
        .map(|preset| preset.id)
}

/// A clip's curve after its span changed, so each stretch of it plays the
/// speed it played before - the curve's counterpart of
/// [`Clip::rewindow_keys`], and called with the same arguments: `old` is
/// the length the curve was drawn over, `[from, to]` the window of it the
/// clip now covers, in seconds from its old start. A window past either end
/// holds the end's speed. The clip's `speed` becomes the new curve's mean.
///
/// Returns the source seconds the curve consumed between the old start and
/// `from` - negative for a window that starts earlier - which is how far
/// the in-point moves when the head goes; None, and nothing changed, for a
/// clip with no curve or a window that makes no sense.
pub fn rewindow_clip(clip: &mut Clip, old: f64, from: f64, to: f64) -> Option<f64> {
    let sane = old.is_finite() && old > 0.0 && from.is_finite() && to.is_finite() && to > from;
    if !sane {
        return None;
    }
    let curve = curve_of(clip.speed_curve.as_deref()?)?;
    let (a, b) = (from / old, to / old);
    // A smooth curve keeps its slopes, re-expressed over the window's
    // length, and gets the curve's own slope at a cut: the piece is then
    // the same cubic the whole was.
    let slopes = curve.slopes();
    let slope = |at: f64| slopes.map(|_| curve.slope_at(at) * (b - a));
    let mut points = vec![SpeedPoint {
        at: 0.0,
        speed: curve.speed_at(a),
        slope: slope(a),
    }];
    points.extend(
        curve
            .points()
            .iter()
            .enumerate()
            .filter(|(_, (at, _))| *at > a && *at < b)
            .map(|(index, &(at, speed))| SpeedPoint {
                at: (at - a) / (b - a),
                speed,
                slope: slopes.map(|slopes| slopes[index] * (b - a)),
            }),
    );
    points.push(SpeedPoint {
        at: 1.0,
        speed: curve.speed_at(b),
        slope: slope(b),
    });
    let consumed = if a >= 0.0 {
        old * curve.consumed(a)
    } else {
        from * curve.speed_at(0.0)
    };
    clip.speed = mean_of(&points).clamp(MIN_SPEED, MAX_SPEED);
    clip.speed_curve = Some(points);
    Some(consumed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ClipKind;

    fn points(raw: &[(f64, f64)]) -> Vec<SpeedPoint> {
        raw.iter()
            .map(|&(at, speed)| SpeedPoint {
                at,
                speed,
                slope: None,
            })
            .collect()
    }

    fn curved(raw: &[(f64, f64)], duration: f64) -> Clip {
        let mut clip = Clip::blank("c1", "v1", ClipKind::Video, "clip", 0.0, duration);
        clip.speed_curve = Some(points(raw));
        clip.speed = mean_of(clip.speed_curve.as_deref().unwrap());
        clip
    }

    /// Source seconds at `t` seconds into a clip, the way the engine maps it.
    fn source_at(clip: &Clip, t: f64) -> f64 {
        let curve = curve_of(clip.speed_curve.as_deref().unwrap()).unwrap();
        clip.source_start + clip.duration * curve.consumed(t / clip.duration)
    }

    #[test]
    fn every_preset_is_a_curve_inside_the_engine_range() {
        for preset in PRESETS {
            let points = super::preset(preset.id).unwrap();
            assert!(curve_of(&points).is_some(), "{}", preset.id);
            assert_eq!(points.first().map(|p| p.at), Some(0.0), "{}", preset.id);
            assert_eq!(points.last().map(|p| p.at), Some(1.0), "{}", preset.id);
            assert!(
                points.windows(2).all(|pair| pair[0].at <= pair[1].at),
                "{} is not in order",
                preset.id
            );
            assert!(
                points
                    .iter()
                    .all(|p| (MIN_SPEED..=MAX_SPEED).contains(&p.speed)),
                "{}",
                preset.id
            );
            assert_eq!(preset_of(&points), Some(preset.id));
        }
        assert!(super::preset("no such preset").is_none());
    }

    #[test]
    fn a_moved_point_is_no_longer_the_preset() {
        let mut points = preset("hero").unwrap();
        points[2].speed = 0.7;
        assert_eq!(preset_of(&points), None);
    }

    #[test]
    fn the_two_halves_of_a_split_play_what_the_whole_played() {
        let whole = curved(&[(0.0, 0.5), (0.4, 4.0), (1.0, 1.0)], 10.0);
        let cut = 3.0;
        let mut head = whole.clone();
        let mut tail = whole.clone();
        assert_eq!(rewindow_clip(&mut head, 10.0, 0.0, cut), Some(0.0));
        head.duration = cut;
        let shift = rewindow_clip(&mut tail, 10.0, cut, 10.0).unwrap();
        tail.source_start += shift;
        tail.duration = 10.0 - cut;
        for t in [0.0, 1.0, 2.5, 3.0] {
            assert!(
                (source_at(&head, t) - source_at(&whole, t)).abs() < 1e-9,
                "head at {t}"
            );
        }
        for t in [0.0, 0.5, 4.0, 7.0] {
            assert!(
                (source_at(&tail, t) - source_at(&whole, cut + t)).abs() < 1e-9,
                "tail at {t}"
            );
        }
        // Each piece's rate is its own share of the curve, and together
        // they cover the source the whole did.
        let covered = head.duration * head.speed + tail.duration * tail.speed;
        assert!((covered - whole.duration * whole.speed).abs() < 1e-9);
    }

    #[test]
    fn a_smooth_ramp_split_in_two_plays_what_the_whole_played() {
        let mut whole = curved(&[(0.0, 0.5), (0.4, 4.0), (1.0, 1.0)], 10.0);
        for point in whole.speed_curve.as_mut().unwrap() {
            point.slope = Some(0.0);
        }
        whole.speed = mean_of(whole.speed_curve.as_deref().unwrap());
        let cut = 2.7;
        let mut head = whole.clone();
        let mut tail = whole.clone();
        rewindow_clip(&mut head, 10.0, 0.0, cut);
        head.duration = cut;
        tail.source_start += rewindow_clip(&mut tail, 10.0, cut, 10.0).unwrap();
        tail.duration = 10.0 - cut;
        assert!(is_smooth(head.speed_curve.as_deref().unwrap()));
        assert!(is_smooth(tail.speed_curve.as_deref().unwrap()));
        for t in [0.0, 1.3, cut] {
            assert!(
                (source_at(&head, t) - source_at(&whole, t)).abs() < 1e-9,
                "head at {t}"
            );
        }
        for t in [0.0, 0.9, 4.0, 7.3] {
            assert!(
                (source_at(&tail, t) - source_at(&whole, cut + t)).abs() < 1e-9,
                "tail at {t}"
            );
        }
    }

    #[test]
    fn a_window_past_the_end_holds_the_end_speed() {
        let mut clip = curved(&[(0.0, 1.0), (1.0, 2.0)], 4.0);
        let shift = rewindow_clip(&mut clip, 4.0, -1.0, 6.0).unwrap();
        // A second before the old start at the start's speed.
        assert!((shift + 1.0).abs() < 1e-12);
        let curve = clip.speed_curve.unwrap();
        assert_eq!(curve.first().map(|p| p.speed), Some(1.0));
        assert_eq!(curve.last().map(|p| p.speed), Some(2.0));
    }

    #[test]
    fn a_clip_without_a_curve_is_left_alone() {
        let mut clip = Clip::blank("c1", "v1", ClipKind::Video, "clip", 0.0, 4.0);
        clip.speed = 2.0;
        assert_eq!(rewindow_clip(&mut clip, 4.0, 1.0, 4.0), None);
        assert_eq!(clip.speed, 2.0);
        assert!(clip.speed_curve.is_none());
    }
}

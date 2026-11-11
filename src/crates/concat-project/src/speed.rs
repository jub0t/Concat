// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Speed curves: the arithmetic the commands share.
//!
//! A curve is a handful of `(at, speed)` points over the clip, joined by
//! straight lines, or bent smoothly through them when every point carries a
//! slope; the engine's [`SpeedCurve`] turns them into a time map. This is
//! where the project's points meet the engine: making the engine's curve
//! from them, tidying a curve that arrives from outside, the presets, and
//! what a trim or a split does to a curve.

use concat_core::SpeedCurve;

use crate::model::ranges::{MAX_SPEED, MIN_SPEED};
use crate::model::{Clip, SpeedPoint};

/// The engine's curve for these points, or None when they make no curve.
pub fn curve_of(points: &[SpeedPoint]) -> Option<SpeedCurve> {
    let raw: Vec<(f64, f64)> = points
        .iter()
        .map(|point: &SpeedPoint| (point.at, point.speed))
        .collect();
    // A slope on some points and not others is no way to bend a curve:
    // such a curve plays as straight lines.
    let slopes: Option<Vec<f64>> = points.iter().map(|point| point.slope).collect();
    SpeedCurve::with_tangents(&raw, slopes.as_deref())
}

/// Whether the curve bends smoothly through its points: it has points, and
/// every one of them has a slope.
pub fn is_smooth(points: &[SpeedPoint]) -> bool {
    !points.is_empty() && points.iter().all(|point| point.slope.is_some())
}

/// Source seconds per timeline second over a whole clip with these points;
/// the constant rate's equivalent for a curve.
pub fn mean_of(points: &[SpeedPoint]) -> f64 {
    curve_of(points).map(|curve| curve.mean()).unwrap_or(1.0)
}

/// The points as they will be played: in order of `at` (points that share
/// one keep their order, a step), every `at` held to `0..=1` and every
/// speed to the range, and reaching both ends of the clip, held level out
/// to an end the points stop short of. A smooth curve stays smooth, a slope
/// that is not a number being a flat one; anything else is straight lines.
/// None when no point is a pair of numbers.
///
/// A curve that arrives from outside is stored this way, so what is saved
/// is what plays, and reading the project back cannot change the clip.
pub fn normalised(points: &[SpeedPoint]) -> Option<Vec<SpeedPoint>> {
    let smooth = is_smooth(points);
    let mut kept: Vec<SpeedPoint> = points
        .iter()
        .filter(|point| point.at.is_finite() && point.speed.is_finite())
        .map(|point| SpeedPoint {
            at: point.at.clamp(0.0, 1.0),
            speed: point.speed.clamp(MIN_SPEED, MAX_SPEED),
            slope: point
                .slope
                .filter(|_| smooth)
                .map(|slope| if slope.is_finite() { slope } else { 0.0 }),
        })
        .collect();
    if kept.is_empty() {
        return None;
    }
    // Stable, so the two sides of a step keep their order.
    kept.sort_by(|a, b| a.at.total_cmp(&b.at));
    let flat = smooth.then_some(0.0);
    let first = kept[0];
    if first.at > 0.0 {
        kept.insert(
            0,
            SpeedPoint {
                at: 0.0,
                speed: first.speed,
                slope: flat,
            },
        );
    }
    let last = kept[kept.len() - 1];
    if last.at < 1.0 {
        kept.push(SpeedPoint {
            at: 1.0,
            speed: last.speed,
            slope: flat,
        });
    }
    Some(kept)
}

/// Source seconds a clip has consumed `offset` timeline seconds after its
/// start: along its curve inside it, and at its end speeds beyond either
/// end - before the start reaching back at the speed it starts at, after
/// the end going on at the speed it ends at. A clip without a curve runs at
/// its rate throughout.
pub fn consumed_by(clip: &Clip, offset: f64) -> f64 {
    let length = clip.duration;
    match clip.speed_curve.as_deref().and_then(curve_of) {
        Some(curve) if length > 0.0 => {
            if offset < 0.0 {
                offset * curve.speed_at(0.0)
            } else if offset > length {
                length * curve.consumed(1.0) + (offset - length) * curve.speed_at(1.0)
            } else {
                length * curve.consumed(offset / length)
            }
        }
        _ => offset * clip.speed,
    }
}

/// The speed a clip starts at: its curve's first, or its rate.
pub fn start_speed(clip: &Clip) -> f64 {
    clip.speed_curve
        .as_deref()
        .and_then(curve_of)
        .map(|curve| curve.speed_at(0.0))
        .unwrap_or(clip.speed)
}

/// Gives a clip the part of its curve a trim or a split leaves it, and the
/// rate that part averages. `old` is the length the curve was drawn over and
/// `[from, to]` the window of it the clip now covers, in seconds from the
/// old start; a window past either end holds the end speeds out there.
///
/// The new curve starts at the speed the old one was going at `from` and
/// ends at the speed it was going at `to`, with the old points between at
/// their places in the window. A cut on a step takes the side of it the
/// piece is on: a piece that starts there starts at the speed the step
/// jumps to, one that ends there ends at the speed before it. A smooth
/// curve's slopes are re-expressed over the window's shorter or longer
/// span, and the cut points get the slope the old curve had there, so each
/// piece is the same cubic the whole was and plays exactly what the whole
/// played. A clip without a curve, or a window that is not one, is left as
/// it was.
pub fn rewindow_curve(clip: &mut Clip, old: f64, from: f64, to: f64) {
    let sane = old.is_finite() && old > 0.0 && from.is_finite() && to.is_finite() && to > from;
    if !sane {
        return;
    }
    let Some(curve) = clip.speed_curve.as_deref().and_then(curve_of) else {
        return;
    };
    let (a, b) = (from / old, to / old);
    let span = b - a;
    let tangents = curve.tangents();
    let bend = |tangent: f64| tangents.map(|_| tangent * span);
    let mut points = Vec::with_capacity(curve.points().len() + 2);
    points.push(SpeedPoint {
        at: 0.0,
        speed: curve.speed_leaving(a),
        slope: bend(curve.tangent_leaving(a)),
    });
    for (index, &(x, speed)) in curve.points().iter().enumerate() {
        if a < x && x < b {
            points.push(SpeedPoint {
                at: (x - a) / span,
                speed,
                slope: tangents.map(|tangents| tangents[index] * span),
            });
        }
    }
    points.push(SpeedPoint {
        at: 1.0,
        speed: curve.speed_at(b),
        slope: bend(curve.tangent_at(b)),
    });
    clip.speed = mean_of(&points).clamp(MIN_SPEED, MAX_SPEED);
    clip.speed_curve = Some(points);
}

/// A shape that ships with the editor: points of `(at, speed)`, eased
/// through every one when it is chosen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    /// What the shape is called in code and in the API.
    pub id: &'static str,
    /// The interface's key for its name.
    pub label: &'static str,
    /// The points, `at` ascending from 0 to 1.
    pub points: &'static [(f64, f64)],
}

/// The shipped shapes, in the order the interface lists them.
pub const PRESETS: [Preset; 6] = [
    // Slow, a burst, slow, a burst, slow: two beats cut together.
    Preset {
        id: "montage",
        label: "speedRamp.montage",
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
    // Fast in, a slow middle for the moment, fast out.
    Preset {
        id: "hero",
        label: "speedRamp.hero",
        points: &[
            (0.0, 2.5),
            (0.25, 2.5),
            (0.38, 0.5),
            (0.62, 0.5),
            (0.75, 2.5),
            (1.0, 2.5),
        ],
    },
    // The same, harder: near stopped in the middle.
    Preset {
        id: "bullet",
        label: "speedRamp.bullet",
        points: &[
            (0.0, 3.0),
            (0.3, 3.0),
            (0.42, 0.2),
            (0.58, 0.2),
            (0.7, 3.0),
            (1.0, 3.0),
        ],
    },
    // Normal, a rush through the middle, normal.
    Preset {
        id: "jump-cut",
        label: "speedRamp.jumpCut",
        points: &[
            (0.0, 1.0),
            (0.38, 1.0),
            (0.45, 8.0),
            (0.55, 8.0),
            (0.62, 1.0),
            (1.0, 1.0),
        ],
    },
    // Rushing in and settling to normal.
    Preset {
        id: "flash-in",
        label: "speedRamp.flashIn",
        points: &[(0.0, 8.0), (0.12, 4.0), (0.3, 1.0), (1.0, 1.0)],
    },
    // Normal, then rushing out.
    Preset {
        id: "flash-out",
        label: "speedRamp.flashOut",
        points: &[(0.0, 1.0), (0.7, 1.0), (0.88, 4.0), (1.0, 8.0)],
    },
];

/// How near two places or two speeds have to be for a curve to still be a
/// preset's.
const PRESET_TOLERANCE: f64 = 1e-9;

/// The points of the preset `id`, eased through each (slope 0): a clip's
/// own from then on. None for an id that is not a preset.
pub fn preset_curve(id: &str) -> Option<Vec<SpeedPoint>> {
    let preset = PRESETS.iter().find(|preset| preset.id == id)?;
    Some(
        preset
            .points
            .iter()
            .map(|&(at, speed)| SpeedPoint {
                at,
                speed,
                slope: Some(0.0),
            })
            .collect(),
    )
}

/// The preset these points still are, smooth or straight, or None once a
/// point has been moved, added or taken away.
pub fn matching_preset(points: &[SpeedPoint]) -> Option<&'static str> {
    let near = |a: f64, b: f64| (a - b).abs() <= PRESET_TOLERANCE;
    PRESETS
        .iter()
        .find(|preset| {
            preset.points.len() == points.len()
                && preset
                    .points
                    .iter()
                    .zip(points)
                    .all(|(&(at, speed), point)| near(at, point.at) && near(speed, point.speed))
        })
        .map(|preset| preset.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ClipKind;

    fn point(at: f64, speed: f64, slope: Option<f64>) -> SpeedPoint {
        SpeedPoint { at, speed, slope }
    }

    fn clip_with(points: Vec<SpeedPoint>, duration: f64) -> Clip {
        let mut clip = Clip::blank("c1", "t1", ClipKind::Video, "clip", 0.0, duration);
        clip.speed = mean_of(&points);
        clip.speed_curve = Some(points);
        clip
    }

    #[test]
    fn a_curve_from_outside_is_stored_as_it_plays() {
        let given = [
            point(0.8, 50.0, Some(1.0)),
            point(-0.5, 2.0, Some(f64::NAN)),
            point(0.3, 0.5, Some(2.0)),
        ];
        let tidied = normalised(&given).unwrap();
        assert_eq!(
            tidied,
            vec![
                point(0.0, 2.0, Some(0.0)),
                point(0.3, 0.5, Some(2.0)),
                point(0.8, 16.0, Some(1.0)),
                point(1.0, 16.0, Some(0.0)),
            ]
        );
        // The engine makes the same curve of both, and the tidied one is
        // already as tidy as it gets.
        assert_eq!(curve_of(&tidied), curve_of(&given));
        assert_eq!(normalised(&tidied), Some(tidied));
        // Points that only start at the middle are held level to the start.
        let late = normalised(&[point(0.5, 3.0, None)]).unwrap();
        assert_eq!(
            late,
            vec![
                point(0.0, 3.0, None),
                point(0.5, 3.0, None),
                point(1.0, 3.0, None)
            ]
        );
        // Slopes on some points only play straight, and are stored so.
        let mixed = normalised(&[point(0.0, 1.0, Some(4.0)), point(1.0, 2.0, None)]).unwrap();
        assert!(mixed.iter().all(|point| point.slope.is_none()));
        assert!(normalised(&[]).is_none());
        assert!(normalised(&[point(f64::NAN, 1.0, None)]).is_none());
    }

    #[test]
    fn a_curve_set_from_outside_reopens_playing_the_same() {
        use crate::commands::NewMedia;
        use crate::doc::DocumentSettings;
        use crate::model::MediaKind;
        use crate::{Command, Editor};

        let mut editor = Editor::new();
        let media = editor
            .apply(Command::AddMedia {
                item: NewMedia {
                    path: "/ramp.mp4".to_owned(),
                    name: "ramp.mp4".to_owned(),
                    duration: Some(10.0),
                    kind: MediaKind::Video,
                    width: Some(1280),
                    height: Some(720),
                    frame_rate: Some(25.0),
                    frame_rate_fraction: Some("25/1".to_owned()),
                    video_codec: Some("h264".to_owned()),
                    audio_codec: None,
                    has_audio: false,
                    audio_tracks: Vec::new(),
                    origin: None,
                    color_space: Default::default(),
                },
            })
            .unwrap()
            .created_id
            .unwrap();
        let track_id = editor.project().active().tracks[0].id.clone();
        let id = editor
            .apply(Command::AddClip {
                media_id: media,
                track_id,
                start: 0.0,
                ripple: false,
            })
            .unwrap()
            .created_id
            .unwrap();
        // Out of order, past the clip's end and past the fastest speed: as
        // played, double from the start to 4x at the end.
        editor
            .apply(Command::SetClipSpeedCurve {
                clip_id: id.clone(),
                curve: Some(vec![point(1.5, 4.0, None), point(0.0, 2.0, None)]),
                ripple: false,
            })
            .unwrap();
        let set = editor.project().active().clip(&id).unwrap().clone();
        assert_eq!(
            set.speed_curve,
            Some(vec![point(0.0, 2.0, None), point(1.0, 4.0, None)])
        );
        assert_eq!(set.speed, 3.0);
        let settings = DocumentSettings {
            name: "ramp".to_owned(),
            width: 1280,
            height: 720,
            rate_num: 25,
            rate_den: 1,
            canvas: [0, 0, 0, 255],
        };
        let reopened = Editor::from_document(&editor.to_document(&settings)).unwrap();
        let back = reopened.project().active().clip(&id).unwrap();
        assert_eq!(back.speed_curve, set.speed_curve);
        assert_eq!(mean_of(back.speed_curve.as_deref().unwrap()), set.speed);
        assert_eq!(back.duration, set.duration);
    }

    #[test]
    fn a_split_on_a_step_keeps_the_jump_between_the_pieces() {
        // Half speed to the middle, then a jump to double, held.
        for slope in [None, Some(0.0)] {
            let step = vec![
                point(0.0, 0.5, slope),
                point(0.5, 0.5, slope),
                point(0.5, 2.0, slope),
                point(1.0, 2.0, slope),
            ];
            let whole = clip_with(step, 10.0);
            let mut head = whole.clone();
            let mut tail = whole.clone();
            rewindow_curve(&mut head, 10.0, 0.0, 5.0);
            rewindow_curve(&mut tail, 10.0, 5.0, 10.0);
            let (head_curve, tail_curve) = (head.speed_curve.unwrap(), tail.speed_curve.unwrap());
            assert_eq!(head_curve.len(), 2);
            assert_eq!(head_curve[1].speed, 0.5, "the head ends before the jump");
            assert_eq!(tail_curve[0].speed, 2.0, "the tail starts after it");
            assert_eq!(head.speed, 0.5);
            assert_eq!(tail.speed, 2.0);
            // Between them, the footage the whole covered.
            let covered = 5.0 * head.speed + 5.0 * tail.speed;
            assert!((covered - consumed_by(&whole, 10.0)).abs() < 1e-12);
        }
    }

    #[test]
    fn a_trim_on_a_step_starts_the_clip_after_the_jump() {
        let step = vec![
            point(0.0, 1.0, None),
            point(0.25, 1.0, None),
            point(0.25, 4.0, None),
            point(1.0, 1.0, None),
        ];
        let mut clip = clip_with(step, 8.0);
        rewindow_curve(&mut clip, 8.0, 2.0, 8.0);
        let curve = clip.speed_curve.unwrap();
        assert_eq!(curve, vec![point(0.0, 4.0, None), point(1.0, 1.0, None)]);
    }

    #[test]
    fn the_footage_reached_beyond_a_curved_clip_runs_at_its_end_speeds() {
        let clip = clip_with(vec![point(0.0, 0.5, None), point(1.0, 2.0, None)], 8.0);
        assert_eq!(consumed_by(&clip, -2.0), -1.0, "back at half speed");
        assert_eq!(consumed_by(&clip, 8.0), 10.0);
        assert_eq!(consumed_by(&clip, 9.0), 12.0, "on at double");
        assert_eq!(start_speed(&clip), 0.5);
    }
}

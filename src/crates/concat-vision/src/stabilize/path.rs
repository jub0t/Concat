// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The correction: from the measured movement to how each frame is moved.
//!
//! The movement between frames, composed frame after frame, is the
//! camera's path: where the first frame's scene sits in every later one.
//! The path is split into four curves - the shift across and down, the
//! turn, and the zoom (as a logarithm, so smoothing it is fair to a zoom
//! in and a zoom out) - and each is smoothed by a Gaussian whose width is
//! the Smoothness setting, measured in seconds of the timeline: a clip at
//! four times its speed shows four times the source in that time, and is
//! smoothed over four times the source frames, so it looks as calm.
//! Tripod takes the curves' mean instead: one still camera.
//!
//! A frame is then drawn as the smooth camera would have seen it: each
//! point of the picture shown is looked for where the real camera put it,
//! `real ∘ smooth⁻¹`. That leaves the frame's edges moving about inside
//! the picture, so the picture is zoomed by the least that hides them for
//! every frame of the clip - one zoom, so the picture does not breathe -
//! up to the setting's most. Where that is not enough, the frames that
//! would show an edge have their correction eased off just so far, and the
//! easing is itself smoothed so it does not jerk.
//!
//! The curves are padded at either end by reflecting them through their
//! end points, so a pan that runs to the end of the clip stays a pan
//! rather than slowing to a stop the smoothing invented.

use super::fit::Similarity;
use super::store::Motion;

/// What a clip's stabilization asks for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Settings {
    /// `0..=1`; see [`sigma_seconds`].
    pub smoothness: f64,
    /// The most the picture may be zoomed to hide the edges, `>= 1`.
    pub max_zoom: f64,
    /// One still camera for the whole clip.
    pub tripod: bool,
    /// Leave the camera's roll alone.
    pub keep_rotation: bool,
}

/// The Gaussian's width, in seconds of timeline, for a smoothness: a
/// tenth of a second at none - only the jitter goes - to three seconds at
/// all, evenly in between on a log scale.
pub fn sigma_seconds(smoothness: f64) -> f64 {
    0.1 * 30f64.powf(smoothness.clamp(0.0, 1.0))
}

/// The correction for one clip: for every frame it shows, the map from a
/// point of the picture as drawn to the point of the frame as shot that
/// is drawn there, in normalised units.
#[derive(Clone, Debug, PartialEq)]
pub struct Correction {
    /// The frames' times, seconds of source, ascending.
    times: Vec<f64>,
    /// Each frame's map, picture to source.
    maps: Vec<Similarity>,
    /// The usual time between frames.
    interval: f64,
    /// The zoom applied over the whole clip, `>= 1`.
    pub zoom: f64,
    /// The zoom the whole correction would need to keep every edge out
    /// of view; more than `zoom` when the setting held it back.
    pub needed: f64,
    /// How shaky the clip is as shot: the root mean square of how far
    /// each frame sits from the smooth path, as a fraction of the frame's
    /// height.
    pub shake: f64,
}

impl Correction {
    /// A clip with nothing to correct.
    pub fn none() -> Correction {
        Correction {
            times: Vec::new(),
            maps: Vec::new(),
            interval: 1.0 / 30.0,
            zoom: 1.0,
            needed: 1.0,
            shake: 0.0,
        }
    }

    /// Whether there is a correction at all.
    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }

    /// The map for the frame on screen at `seconds` of source: the last
    /// frame shown at or before it. The identity outside the frames
    /// measured, so a stretch not analysed shows as shot.
    pub fn at(&self, seconds: f64) -> Similarity {
        let index = self.times.partition_point(|&at| at <= seconds + 1e-4);
        if index == 0 {
            return Similarity::IDENTITY;
        }
        let index = index - 1;
        // Past the last frame by more than a frame or two is past what was
        // measured.
        if seconds - self.times[index] > 2.5 * self.interval {
            return Similarity::IDENTITY;
        }
        self.maps[index]
    }
}

/// The four curves of a camera path.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Pose {
    tx: f64,
    ty: f64,
    angle: f64,
    log_scale: f64,
}

impl Pose {
    fn of(similarity: &Similarity, previous_angle: Option<f64>) -> Pose {
        let mut angle = similarity.angle();
        // Unwrapped, so a slow turn past half a revolution is a curve and
        // not a jump.
        if let Some(previous) = previous_angle {
            let tau = std::f64::consts::TAU;
            angle += ((previous - angle) / tau).round() * tau;
        }
        Pose {
            tx: similarity.tx,
            ty: similarity.ty,
            angle,
            log_scale: similarity.scale().max(1e-9).ln(),
        }
    }

    fn similarity(&self) -> Similarity {
        Similarity::new(self.angle, self.log_scale.exp(), self.tx, self.ty)
    }

    fn lerp(&self, other: &Pose, t: f64) -> Pose {
        Pose {
            tx: self.tx + (other.tx - self.tx) * t,
            ty: self.ty + (other.ty - self.ty) * t,
            angle: self.angle + (other.angle - self.angle) * t,
            log_scale: self.log_scale + (other.log_scale - self.log_scale) * t,
        }
    }
}

/// The least zoom about the centre at which `map` - picture to source -
/// keeps the whole picture inside the source frame, `aspect` wide and one
/// high. Infinite when no zoom does.
pub fn zoom_needed(map: &Similarity, aspect: f64) -> f64 {
    let (hw, hh) = (aspect / 2.0, 0.5);
    let mut zoom: f64 = 1.0;
    for corner in [(-hw, -hh), (hw, -hh), (-hw, hh), (hw, hh)] {
        // The linear part of the map on the corner; the shift is apart.
        let x = map.a * corner.0 - map.b * corner.1;
        let y = map.b * corner.0 + map.a * corner.1;
        for (reach, shift, half) in [(x, map.tx, hw), (y, map.ty, hh)] {
            // reach / zoom + shift must stay within ±half.
            let room = if reach > 0.0 {
                half - shift
            } else {
                half + shift
            };
            if reach.abs() <= 1e-12 {
                if shift.abs() > half {
                    return f64::INFINITY;
                }
                continue;
            }
            if room <= 0.0 {
                return f64::INFINITY;
            }
            zoom = zoom.max(reach.abs() / room);
        }
    }
    zoom
}

/// `values` smoothed by a Gaussian of `sigma` samples: three box passes,
/// which come within a few percent of one and cost the same at any width.
/// The ends are padded by reflecting through the end points.
pub fn gaussian(values: &[f64], sigma: f64) -> Vec<f64> {
    let n = values.len();
    if n < 2 || sigma.is_nan() || sigma < 0.5 {
        return values.to_vec();
    }
    // Three boxes of width w have the variance of a Gaussian of sigma
    // when 3 (w² - 1) / 12 = sigma².
    let width = (4.0 * sigma * sigma + 1.0).sqrt();
    let radius = ((width - 1.0) / 2.0).round().max(1.0) as usize;
    let pad = 3 * radius + 1;
    // Reflected through a line fitted to each end rather than through the
    // end sample itself: a shaky first frame is not pinned in place.
    let reach = (2 * radius + 1).min(n);
    let pivots = (
        line_at(&values[..reach], 0.0),
        line_at(&values[n - reach..], (reach - 1) as f64),
    );
    let mut padded: Vec<f64> = (-(pad as isize)..(n + pad) as isize)
        .map(|i| reflected(values, pivots, i))
        .collect();
    for _ in 0..3 {
        padded = box_mean(&padded, radius);
    }
    padded[pad..pad + n].to_vec()
}

/// `values` at `i`, past either end reflected through that end's pivot -
/// and past the far end of the reflection reflected again - so a straight
/// line carries on straight however far it is read.
fn reflected(values: &[f64], (start, end): (f64, f64), mut i: isize) -> f64 {
    let last = values.len() as isize - 1;
    if last <= 0 {
        return values.first().copied().unwrap_or(0.0);
    }
    // Each reflection is `2·end − value`, so a run of them adds up to a
    // signed sum of end values and one value inside.
    let mut sign = 1.0;
    let mut offset = 0.0;
    loop {
        if i < 0 {
            offset += sign * 2.0 * start;
            sign = -sign;
            i = -i;
        } else if i > last {
            offset += sign * 2.0 * end;
            sign = -sign;
            i = 2 * last - i;
        } else {
            return offset + sign * values[i as usize];
        }
    }
}

/// The least-squares line through `values`, at index `x`.
fn line_at(values: &[f64], x: f64) -> f64 {
    let n = values.len() as f64;
    if values.len() < 2 {
        return values.first().copied().unwrap_or(0.0);
    }
    let mean_x = (n - 1.0) / 2.0;
    let mean_y = values.iter().sum::<f64>() / n;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (i, v) in values.iter().enumerate() {
        let dx = i as f64 - mean_x;
        sxy += dx * (v - mean_y);
        sxx += dx * dx;
    }
    mean_y + sxy / sxx * (x - mean_x)
}

/// The mean over `2 radius + 1` samples around each, fewer at the ends.
fn box_mean(values: &[f64], radius: usize) -> Vec<f64> {
    let n = values.len();
    let mut prefix = vec![0.0; n + 1];
    for (i, v) in values.iter().enumerate() {
        prefix[i + 1] = prefix[i] + v;
    }
    (0..n)
        .map(|i| {
            let from = i.saturating_sub(radius);
            let to = (i + radius + 1).min(n);
            (prefix[to] - prefix[from]) / (to - from) as f64
        })
        .collect()
}

/// The least over `2 radius + 1` samples around each.
fn box_min(values: &[f64], radius: usize) -> Vec<f64> {
    let n = values.len();
    (0..n)
        .map(|i| {
            let from = i.saturating_sub(radius);
            let to = (i + radius + 1).min(n);
            values[from..to]
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min)
        })
        .collect()
}

/// The correction for the clip showing `from..to` seconds of source of
/// the media whose movement is `motion`, at `speed` source seconds per
/// timeline second.
pub fn correct(
    motion: &Motion,
    (from, to): (f64, f64),
    speed: f64,
    settings: &Settings,
) -> Correction {
    let interval = motion.interval();
    let speed = if speed.is_finite() {
        speed.abs().max(1.0 / 16.0)
    } else {
        1.0
    };
    let sigma_source = sigma_seconds(settings.smoothness) * speed;
    let sigma_frames = sigma_source / interval;
    // The frames the clip shows, and the smoothing's reach either side,
    // which it reads but does not correct for.
    let reach = 3.0 * sigma_source + interval;
    let first = motion
        .frames
        .partition_point(|frame| frame.at < from - reach - 1e-6);
    let end = motion
        .frames
        .partition_point(|frame| frame.at <= to + reach + 1e-6);
    let frames = &motion.frames[first..end.max(first)];
    if frames.len() < 2 {
        return Correction::none();
    }
    let aspect = motion.aspect;

    // The path: where the window's first frame's scene sits in each frame.
    let mut path: Vec<Similarity> = Vec::with_capacity(frames.len());
    let mut camera = Similarity::IDENTITY;
    for (i, frame) in frames.iter().enumerate() {
        if i > 0 {
            camera = camera.then(&frame.movement());
        }
        path.push(camera);
    }
    let mut poses: Vec<Pose> = Vec::with_capacity(path.len());
    for similarity in &path {
        let previous = poses.last().map(|pose: &Pose| pose.angle);
        poses.push(Pose::of(similarity, previous));
    }

    // The frames on screen: from the one showing at `from` to the one at
    // `to`, which is where the zoom is decided and the shake measured.
    let shown_from = frames
        .partition_point(|frame| frame.at <= from + 1e-4)
        .saturating_sub(1);
    let shown_to = frames
        .partition_point(|frame| frame.at <= to + 1e-4)
        .max(shown_from + 1);
    let shown = shown_from..shown_to;

    let smooth: Vec<Pose> = if settings.tripod {
        let count = shown.len() as f64;
        let mut mean = Pose::default();
        for pose in &poses[shown.clone()] {
            mean.tx += pose.tx / count;
            mean.ty += pose.ty / count;
            mean.angle += pose.angle / count;
            mean.log_scale += pose.log_scale / count;
        }
        poses
            .iter()
            .map(|pose| Pose {
                angle: if settings.keep_rotation {
                    pose.angle
                } else {
                    mean.angle
                },
                ..mean
            })
            .collect()
    } else {
        let curve = |pick: fn(&Pose) -> f64| {
            gaussian(&poses.iter().map(pick).collect::<Vec<_>>(), sigma_frames)
        };
        let (tx, ty, angle, log_scale) = (
            curve(|p| p.tx),
            curve(|p| p.ty),
            curve(|p| p.angle),
            curve(|p| p.log_scale),
        );
        (0..poses.len())
            .map(|i| Pose {
                tx: tx[i],
                ty: ty[i],
                angle: if settings.keep_rotation {
                    poses[i].angle
                } else {
                    angle[i]
                },
                log_scale: log_scale[i],
            })
            .collect()
    };

    // How far towards the smooth path each frame goes: all the way, unless
    // the zoom allowed cannot hide the edges that shows.
    let map_at = |i: usize, towards: f64| -> Similarity {
        let target = poses[i].lerp(&smooth[i], towards).similarity();
        target.inverse().then(&path[i])
    };
    let full: Vec<Similarity> = (0..poses.len()).map(|i| map_at(i, 1.0)).collect();
    let needed = shown
        .clone()
        .map(|i| zoom_needed(&full[i], aspect))
        .fold(1.0, f64::max);
    let cap = if settings.max_zoom.is_finite() {
        settings.max_zoom.max(1.0)
    } else {
        1.0
    };
    let zoom = needed.min(cap);
    let mut maps = full;
    if needed > zoom + 1e-9 {
        // Each frame's own furthest, by bisection: none of the way always
        // fits, since it is the frame as shot.
        let mut eased: Vec<f64> = (0..poses.len())
            .map(|i| {
                if zoom_needed(&maps[i], aspect) <= zoom {
                    return 1.0;
                }
                let (mut low, mut high) = (0.0, 1.0);
                for _ in 0..14 {
                    let mid = 0.5 * (low + high);
                    if zoom_needed(&map_at(i, mid), aspect) <= zoom {
                        low = mid;
                    } else {
                        high = mid;
                    }
                }
                low
            })
            .collect();
        // Smoothed without rising past any frame's own: the least over a
        // window, then the mean over the same window, each at most what
        // every frame in reach allows.
        let radius = sigma_frames.round().max(1.0) as usize;
        let smoothed = box_mean(&box_min(&eased, radius), radius);
        for (i, value) in smoothed.into_iter().enumerate() {
            if zoom_needed(&map_at(i, value), aspect) <= zoom + 1e-9 {
                eased[i] = value;
            }
        }
        maps = (0..poses.len()).map(|i| map_at(i, eased[i])).collect();
    }

    // The shake: how far the frame's middle is from where the smooth
    // camera would have had it.
    let mut squares = 0.0;
    for i in shown.clone() {
        let (x, y) = maps[i].apply((0.0, 0.0));
        squares += x * x + y * y;
    }
    let shake = (squares / shown.len() as f64).sqrt();

    let unzoom = Similarity::scaling(1.0 / zoom);
    Correction {
        times: frames.iter().map(|frame| frame.at).collect(),
        maps: maps.iter().map(|map| unzoom.then(map)).collect(),
        interval,
        zoom,
        needed,
        shake,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stabilize::store::{FrameMotion, Step};

    /// A clip at thirty frames a second whose camera shook by `shake(i)`
    /// between frames.
    fn shot(frames: usize, shake: impl Fn(usize) -> Similarity) -> Motion {
        Motion {
            aspect: 16.0 / 9.0,
            covered: vec![(0.0, frames as f64 / 30.0)],
            frames: (0..frames)
                .map(|i| FrameMotion {
                    at: i as f64 / 30.0,
                    step: if i == 0 { Step::Start } else { Step::Tracked },
                    motion: shake(i),
                })
                .collect(),
        }
    }

    fn settings(smoothness: f64, max_zoom: f64) -> Settings {
        Settings {
            smoothness,
            max_zoom,
            tripod: false,
            keep_rotation: false,
        }
    }

    #[test]
    fn a_gaussian_keeps_a_line_and_flattens_a_wobble() {
        let line: Vec<f64> = (0..50).map(|i| 0.3 + 0.01 * i as f64).collect();
        let smoothed = gaussian(&line, 6.0);
        for (a, b) in line.iter().zip(&smoothed) {
            assert!((a - b).abs() < 1e-9, "a line through the ends stays a line");
        }
        let wobble: Vec<f64> = (0..200)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let smoothed = gaussian(&wobble, 4.0);
        assert!(smoothed[20..180].iter().all(|v| v.abs() < 0.02));
    }

    #[test]
    fn the_zoom_needed_hides_a_shift_and_a_turn() {
        let aspect = 16.0 / 9.0;
        assert!((zoom_needed(&Similarity::IDENTITY, aspect) - 1.0).abs() < 1e-12);
        // A shift of a tenth of the height down: the bottom edge has 0.4
        // of room for 0.5 of reach, so the picture zooms by 1.25.
        let down = Similarity::new(0.0, 1.0, 0.0, 0.1);
        assert!((zoom_needed(&down, aspect) - 1.25).abs() < 1e-9);
        // And that zoom does keep every corner inside.
        let zoomed = Similarity::scaling(1.0 / 1.25).then(&down);
        for corner in [(-aspect / 2.0, -0.5), (aspect / 2.0, 0.5)] {
            let (x, y) = zoomed.apply(corner);
            assert!(x.abs() <= aspect / 2.0 + 1e-9 && y.abs() <= 0.5 + 1e-9);
        }
        // A turn needs more than nothing, and a turn the other way as much.
        let left = zoom_needed(&Similarity::new(0.02, 1.0, 0.0, 0.0), aspect);
        let right = zoom_needed(&Similarity::new(-0.02, 1.0, 0.0, 0.0), aspect);
        assert!(left > 1.01 && (left - right).abs() < 1e-9);
    }

    /// A camera shaking a few pixels back and forth is held still: each
    /// frame's map undoes where the shake put it.
    #[test]
    fn a_shake_is_corrected_and_zoomed_over() {
        let h = 1080.0;
        // Frame k sits 4 px right on odd frames: the step alternates.
        let motion = shot(90, |i| {
            let dx = if i % 2 == 1 { 4.0 / h } else { -4.0 / h };
            Similarity::new(0.0, 1.0, dx, 0.0)
        });
        let correction = correct(&motion, (0.0, 3.0), 1.0, &settings(0.5, 1.3));
        assert!(correction.zoom > 1.0 && correction.zoom < 1.01);
        assert_eq!(correction.zoom, correction.needed);
        for frame in 10..80 {
            let at = frame as f64 / 30.0;
            // The camera's offset at this frame, relative to its mean.
            let offset = if frame % 2 == 1 { 2.0 / h } else { -2.0 / h };
            // The picture's middle is looked for where the shake put it.
            let (x, _) = correction.at(at).apply((0.0, 0.0));
            assert!((x - offset).abs() < 0.2 / h, "frame {frame}: {} px", x * h);
        }
        assert!(
            (correction.shake * h - 2.0).abs() < 0.2,
            "{}",
            correction.shake * h
        );
    }

    /// A steady pan stays a pan: nothing to correct, nothing to zoom.
    #[test]
    fn a_steady_pan_is_left_alone() {
        let motion = shot(120, |_| Similarity::new(0.0, 1.0, 0.005, 0.0));
        let correction = correct(&motion, (0.5, 3.5), 1.0, &settings(0.8, 1.3));
        assert!(correction.zoom < 1.0 + 1e-6, "{}", correction.zoom);
        assert!(correction.at(2.0).is_identity(1e-9));
    }

    /// Tripod locks the frame to one place: every frame's map undoes the
    /// whole of the pan, around its mean.
    #[test]
    fn tripod_locks_the_frame() {
        let motion = shot(60, |_| Similarity::new(0.0, 1.0, 0.001, 0.0));
        let tripod = Settings {
            tripod: true,
            ..settings(0.5, 1.5)
        };
        let correction = correct(&motion, (0.0, 2.0), 1.0, &tripod);
        // The scene point at the middle of the frame at the mean pose
        // stays put: frame k's map sends the middle where frame k has it.
        let first = correction.at(0.0).apply((0.0, 0.0)).0;
        let last = correction.at(59.0 / 30.0).apply((0.0, 0.0)).0;
        assert!(((last - first) - 59.0 * 0.001).abs() < 1e-6);
    }

    /// Asked for more than the zoom allows, the correction is eased
    /// rather than showing an edge: every frame's map stays inside.
    #[test]
    fn a_capped_zoom_eases_the_correction_rather_than_show_an_edge() {
        // A big lurch half way through.
        let motion = shot(90, |i| {
            Similarity::new(0.0, 1.0, if i == 45 { 0.15 } else { 0.0 }, 0.0)
        });
        let correction = correct(&motion, (0.0, 3.0), 1.0, &settings(1.0, 1.05));
        assert!(correction.needed > 1.05);
        assert!((correction.zoom - 1.05).abs() < 1e-12);
        for frame in 0..90 {
            let map = correction.at(frame as f64 / 30.0);
            assert!(zoom_needed(&map, 16.0 / 9.0) <= 1.0 + 1e-6, "frame {frame}");
        }
    }

    /// At four times the speed the same timeline smoothness reaches over
    /// four times the source frames.
    #[test]
    fn a_faster_clip_is_smoothed_over_more_source() {
        let motion = shot(600, |i| {
            Similarity::new(0.0, 1.0, 0.002 * ((i as f64) * 0.15).sin(), 0.0)
        });
        let slow = correct(&motion, (5.0, 15.0), 1.0, &settings(0.3, 1.5));
        let fast = correct(&motion, (5.0, 15.0), 4.0, &settings(0.3, 1.5));
        assert!(fast.needed > slow.needed);
    }

    #[test]
    fn outside_what_was_measured_is_left_as_shot() {
        let motion = shot(30, |_| Similarity::new(0.01, 1.0, 0.003, 0.0));
        let correction = correct(&motion, (0.0, 5.0), 1.0, &settings(0.5, 1.2));
        assert!(correction.at(4.0).is_identity(0.0));
        assert!(correction.at(-1.0).is_identity(0.0));
    }
}

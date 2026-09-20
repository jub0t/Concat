// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! A small source-space translation tracker for geometric masks.
//!
//! This deliberately tracks position only. Size, rotation, feather and shape
//! remain authored properties, while Position X/Y receive keyframes. A
//! luminance template is searched near its previous location and refreshed
//! gently, which is predictable for the short editorial shots this control
//! targets and has no model download or network dependency.

use concat_core::frame::Frame;

/// Translation tracker initialised from the pixels inside a mask.
pub struct TranslationTracker {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    samples: Vec<Sample>,
}

struct Sample {
    dx: i32,
    dy: i32,
    luminance: f32,
}

impl TranslationTracker {
    /// Captures a sparse template around a normalised centre and size.
    pub fn new(frame: &Frame, centre: (f64, f64), size: (f64, f64)) -> Option<Self> {
        if frame.width() < 4 || frame.height() < 4 {
            return None;
        }
        let x = (centre.0.clamp(0.0, 1.0) * f64::from(frame.width() - 1)).round() as i32;
        let y = (centre.1.clamp(0.0, 1.0) * f64::from(frame.height() - 1)).round() as i32;
        let half_w = (size.0.clamp(0.03, 1.0) * f64::from(frame.width()) * 0.5)
            .round()
            .clamp(4.0, 64.0) as i32;
        let half_h = (size.1.clamp(0.03, 1.0) * f64::from(frame.height()) * 0.5)
            .round()
            .clamp(4.0, 64.0) as i32;
        let stride = (((half_w * 2 * half_h * 2) as f64 / 900.0).sqrt())
            .ceil()
            .max(1.0) as usize;
        let mut samples = Vec::new();
        for dy in (-half_h..=half_h).step_by(stride) {
            for dx in (-half_w..=half_w).step_by(stride) {
                samples.push(Sample {
                    dx,
                    dy,
                    luminance: luma(frame, x + dx, y + dy),
                });
            }
        }
        (!samples.is_empty()).then_some(Self {
            x,
            y,
            width: frame.width(),
            height: frame.height(),
            samples,
        })
    }

    /// Locates the template in the next frame and returns its normalised
    /// centre. Untextured, ambiguous or lost matches hold the previous
    /// position and do not refresh the template, so it can recover later.
    pub fn step(&mut self, frame: &Frame) -> (f64, f64) {
        // The template's offsets are in pixels. A differently sized frame
        // cannot be compared to it (and an empty frame cannot be sampled).
        if frame.width() != self.width || frame.height() != self.height {
            return self.centre();
        }
        let contrast = self.contrast();
        if contrast < 0.02 {
            return self.centre();
        }
        let radius =
            ((frame.width().min(frame.height()) as f64 * 0.07).round() as i32).clamp(6, 20);
        // Include every pixel: a coarse grid can miss both an exact match
        // and an equally good repeated feature on its skipped pixels.
        let mut candidates = Vec::with_capacity(((radius * 2 + 1).pow(2)) as usize);
        let mut best = (self.error(frame, self.x, self.y), self.x, self.y);
        for y in (self.y - radius).max(0)..=(self.y + radius).min(self.height as i32 - 1) {
            for x in (self.x - radius).max(0)..=(self.x + radius).min(self.width as i32 - 1) {
                let error = self.error(frame, x, y);
                let tied = (error - best.0).abs() <= 1e-6;
                let closer = distance_squared(x, y, self.x, self.y)
                    < distance_squared(best.1, best.2, self.x, self.y);
                if error < best.0 - 1e-6 || (tied && closer) {
                    best = (error, x, y);
                }
                candidates.push((error, x, y));
            }
        }

        // Adjacent pixels belong to the same match basin. A second good
        // match outside it makes the displacement uncertain. The absolute
        // error check also rejects occlusion even when it has one winner.
        let runner_up = candidates
            .iter()
            .filter(|(_, x, y)| distance_squared(*x, *y, best.1, best.2) > 4)
            .map(|(error, _, _)| *error)
            .fold(f32::INFINITY, f32::min);
        if best.0 > (contrast * 0.6).min(0.15) || runner_up - best.0 < (best.0 * 0.1).max(0.003) {
            return self.centre();
        }
        self.x = best.1;
        self.y = best.2;
        // Slow adaptation survives modest lighting changes without letting a
        // bad frame immediately replace the thing being tracked.
        for sample in &mut self.samples {
            let now = luma(frame, self.x + sample.dx, self.y + sample.dy);
            sample.luminance = sample.luminance * 0.9 + now * 0.1;
        }
        self.centre()
    }

    fn centre(&self) -> (f64, f64) {
        (
            f64::from(self.x) / f64::from(self.width - 1),
            f64::from(self.y) / f64::from(self.height - 1),
        )
    }

    fn contrast(&self) -> f32 {
        let count = self.samples.len() as f32;
        let mean = self
            .samples
            .iter()
            .map(|sample| sample.luminance)
            .sum::<f32>()
            / count;
        (self
            .samples
            .iter()
            .map(|sample| (sample.luminance - mean).powi(2))
            .sum::<f32>()
            / count)
            .sqrt()
    }

    fn error(&self, frame: &Frame, x: i32, y: i32) -> f32 {
        self.samples
            .iter()
            .map(|sample| (luma(frame, x + sample.dx, y + sample.dy) - sample.luminance).abs())
            .sum::<f32>()
            / self.samples.len().max(1) as f32
    }
}

fn distance_squared(x: i32, y: i32, other_x: i32, other_y: i32) -> i32 {
    (x - other_x).pow(2) + (y - other_y).pow(2)
}

fn luma(frame: &Frame, x: i32, y: i32) -> f32 {
    let x = x.clamp(0, frame.width() as i32 - 1) as usize;
    let y = y.clamp(0, frame.height() as i32 - 1) as usize;
    let at = (y * frame.width() as usize + x) * 4;
    let pixel = &frame.pixels()[at..at + 3];
    (f32::from(pixel[0]) * 0.2126 + f32::from(pixel[1]) * 0.7152 + f32::from(pixel[2]) * 0.0722)
        / 255.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(value: u8) -> Frame {
        let mut frame = Frame::black(64, 48);
        for pixel in frame.pixels_mut().chunks_exact_mut(4) {
            pixel[..3].fill(value);
        }
        frame
    }

    fn subject_tracker() -> TranslationTracker {
        TranslationTracker::new(&square(0), (28.0 / 63.0, 23.0 / 47.0), (0.2, 0.3)).unwrap()
    }

    fn square(offset: i32) -> Frame {
        let mut rgba = vec![0; 64 * 48 * 4];
        for y in 0..48 {
            for x in 0..64 {
                let at = (y * 64 + x) * 4;
                let lit =
                    (x as i32) >= 22 + offset && (x as i32) < 34 + offset && (17..29).contains(&y);
                let value = if lit { 240 } else { 20 };
                rgba[at..at + 3].fill(value);
                rgba[at + 3] = 255;
            }
        }
        Frame::from_rgba(64, 48, rgba).unwrap()
    }

    #[test]
    fn follows_a_translated_subject() {
        let mut tracker = subject_tracker();
        assert_eq!(tracker.step(&square(5)), (33.0 / 63.0, 23.0 / 47.0));
        assert_eq!(tracker.step(&square(2)), (30.0 / 63.0, 23.0 / 47.0));
    }

    #[test]
    fn a_uniform_template_does_not_drift_or_adapt() {
        let first = solid(100);
        let mut tracker = TranslationTracker::new(&first, (0.5, 0.5), (0.3, 0.3)).unwrap();
        let start = tracker.centre();
        let template: Vec<_> = tracker
            .samples
            .iter()
            .map(|sample| sample.luminance)
            .collect();
        for value in [100, 100, 120, 0, 255] {
            assert_eq!(tracker.step(&solid(value)), start);
        }
        assert_eq!(
            tracker
                .samples
                .iter()
                .map(|sample| sample.luminance)
                .collect::<Vec<_>>(),
            template
        );
    }

    #[test]
    fn a_stationary_textured_subject_does_not_drift() {
        let mut tracker = subject_tracker();
        let start = tracker.centre();
        for _ in 0..10 {
            assert_eq!(tracker.step(&square(0)), start);
        }
    }

    #[test]
    fn repeated_features_do_not_choose_an_arbitrary_displacement() {
        let stripes = |offset: usize| {
            let mut frame = Frame::black(64, 48);
            for (i, pixel) in frame.pixels_mut().chunks_exact_mut(4).enumerate() {
                pixel[..3].fill(if (i % 64 + offset) % 4 < 2 { 20 } else { 240 });
            }
            frame
        };
        let mut tracker = TranslationTracker::new(&stripes(0), (0.5, 0.5), (0.3, 0.3)).unwrap();
        let start = tracker.centre();
        assert_eq!(tracker.step(&stripes(0)), start);
        assert_eq!(tracker.step(&stripes(1)), start);
    }

    #[test]
    fn occlusion_holds_the_last_position_and_preserves_recovery() {
        let mut tracker = subject_tracker();
        let start = tracker.centre();
        let template: Vec<_> = tracker
            .samples
            .iter()
            .map(|sample| sample.luminance)
            .collect();
        for _ in 0..30 {
            assert_eq!(tracker.step(&solid(100)), start);
        }
        assert_eq!(
            tracker
                .samples
                .iter()
                .map(|sample| sample.luminance)
                .collect::<Vec<_>>(),
            template
        );
        assert_eq!(tracker.step(&square(5)), (33.0 / 63.0, 23.0 / 47.0));
    }

    #[test]
    fn a_unique_but_bad_match_is_not_accepted() {
        let mut tracker = subject_tracker();
        let start = tracker.centre();
        let mut different = square(4);
        for pixel in different.pixels_mut().chunks_exact_mut(4) {
            // A same-shaped but strongly different subject still has a
            // unique best match. Uniqueness alone is not confidence.
            let value = if pixel[0] == 240 { 140 } else { 90 };
            pixel[..3].fill(value);
        }
        assert_eq!(tracker.step(&different), start);
        assert_eq!(tracker.step(&square(4)), (32.0 / 63.0, 23.0 / 47.0));
    }

    #[test]
    fn modest_lighting_changes_keep_the_subject() {
        let mut tracker = subject_tracker();
        let mut brighter = square(3);
        for pixel in brighter.pixels_mut().chunks_exact_mut(4) {
            for value in &mut pixel[..3] {
                *value += 8;
            }
        }
        assert_eq!(tracker.step(&brighter), (31.0 / 63.0, 23.0 / 47.0));
    }

    #[test]
    fn mismatched_or_empty_frames_do_not_destroy_the_template() {
        let mut tracker = subject_tracker();
        let start = tracker.centre();
        assert_eq!(tracker.step(&Frame::black(0, 0)), start);
        assert_eq!(tracker.step(&Frame::black(32, 24)), start);
        assert_eq!(tracker.step(&square(5)), (33.0 / 63.0, 23.0 / 47.0));
    }
}

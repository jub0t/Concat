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
        (!samples.is_empty()).then_some(Self { x, y, samples })
    }

    /// Locates the template in the next frame and returns its normalised
    /// centre. Search is coarse then one-pixel refined around the winner.
    pub fn step(&mut self, frame: &Frame) -> (f64, f64) {
        let radius =
            ((frame.width().min(frame.height()) as f64 * 0.07).round() as i32).clamp(6, 20);
        let mut best = (f32::INFINITY, self.x, self.y);
        for dy in (-radius..=radius).step_by(2) {
            for dx in (-radius..=radius).step_by(2) {
                let error = self.error(frame, self.x + dx, self.y + dy);
                if error < best.0 {
                    best = (error, self.x + dx, self.y + dy);
                }
            }
        }
        let coarse = (best.1, best.2);
        for dy in -2..=2 {
            for dx in -2..=2 {
                let error = self.error(frame, coarse.0 + dx, coarse.1 + dy);
                if error < best.0 {
                    best = (error, coarse.0 + dx, coarse.1 + dy);
                }
            }
        }
        self.x = best.1.clamp(0, frame.width() as i32 - 1);
        self.y = best.2.clamp(0, frame.height() as i32 - 1);
        // Slow adaptation survives modest lighting changes without letting a
        // bad frame immediately replace the thing being tracked.
        for sample in &mut self.samples {
            let now = luma(frame, self.x + sample.dx, self.y + sample.dy);
            sample.luminance = sample.luminance * 0.9 + now * 0.1;
        }
        (
            f64::from(self.x) / f64::from(frame.width().saturating_sub(1).max(1)),
            f64::from(self.y) / f64::from(frame.height().saturating_sub(1).max(1)),
        )
    }

    fn error(&self, frame: &Frame, x: i32, y: i32) -> f32 {
        self.samples
            .iter()
            .map(|sample| (luma(frame, x + sample.dx, y + sample.dy) - sample.luminance).abs())
            .sum::<f32>()
            / self.samples.len().max(1) as f32
    }
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
        let first = square(0);
        let mut tracker =
            TranslationTracker::new(&first, (28.0 / 63.0, 23.0 / 47.0), (0.2, 0.3)).unwrap();
        let (x, _) = tracker.step(&square(5));
        assert!((x - 33.0 / 63.0).abs() < 0.04, "tracked x was {x}");
    }
}

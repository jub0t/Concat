// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! How alike two frames are, as a number: what the parity suite holds the
//! GPU to, and what a benchmark reports.
//!
//! [`ssim`] is the structural similarity index over the luminance, on
//! 8×8 windows, the usual constants: one for identical pictures, near
//! zero for unrelated ones. It forgives a rounding and a resampling's
//! blur and does not forgive a missing layer, a wrong placement or a
//! shader that did something else, which is the distinction a parity
//! check needs.

use concat_core::frame::{BYTES_PER_PIXEL, Frame};

/// The window the index is averaged over, a side.
const WINDOW: usize = 8;

/// The structural similarity of `a` and `b`, `0..=1`; zero when their
/// sizes differ, one when they are the same picture.
pub fn ssim(a: &Frame, b: &Frame) -> f64 {
    if a.width() != b.width() || a.height() != b.height() || a.width() == 0 || a.height() == 0 {
        return 0.0;
    }
    let (width, height) = (a.width() as usize, a.height() as usize);
    let luma = |frame: &Frame| -> Vec<f64> {
        frame
            .pixels()
            .chunks_exact(BYTES_PER_PIXEL)
            .map(|px| {
                0.2126 * f64::from(px[0]) + 0.7152 * f64::from(px[1]) + 0.0722 * f64::from(px[2])
            })
            .collect()
    };
    let (la, lb) = (luma(a), luma(b));
    let (c1, c2) = ((0.01f64 * 255.0).powi(2), (0.03f64 * 255.0).powi(2));
    let mut total = 0.0;
    let mut windows = 0.0;
    let step = WINDOW.min(width).min(height).max(1);
    for y in (0..height).step_by(step) {
        for x in (0..width).step_by(step) {
            let (w, h) = (step.min(width - x), step.min(height - y));
            let n = (w * h) as f64;
            let (mut ma, mut mb) = (0.0, 0.0);
            for row in y..y + h {
                for col in x..x + w {
                    ma += la[row * width + col];
                    mb += lb[row * width + col];
                }
            }
            ma /= n;
            mb /= n;
            let (mut va, mut vb, mut cov) = (0.0, 0.0, 0.0);
            for row in y..y + h {
                for col in x..x + w {
                    let (da, db) = (la[row * width + col] - ma, lb[row * width + col] - mb);
                    va += da * da;
                    vb += db * db;
                    cov += da * db;
                }
            }
            let denominator = if n > 1.0 { n - 1.0 } else { 1.0 };
            va /= denominator;
            vb /= denominator;
            cov /= denominator;
            total += ((2.0 * ma * mb + c1) * (2.0 * cov + c2))
                / ((ma * ma + mb * mb + c1) * (va + vb + c2));
            windows += 1.0;
        }
    }
    (total / windows).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32, seed: u8) -> Frame {
        let mut frame = Frame::black(width, height);
        for y in 0..height {
            for x in 0..width {
                let v = (x * 7 + y * 3 + u32::from(seed)) % 256;
                frame.set_pixel(x, y, [v as u8, (v / 2) as u8, 255 - v as u8, 255]);
            }
        }
        frame
    }

    #[test]
    fn the_same_picture_is_one_and_a_different_size_is_zero() {
        let a = gradient(32, 24, 0);
        assert!((ssim(&a, &a) - 1.0).abs() < 1e-9);
        assert_eq!(ssim(&a, &gradient(16, 24, 0)), 0.0);
        assert_eq!(ssim(&Frame::black(0, 0), &Frame::black(0, 0)), 0.0);
    }

    #[test]
    fn a_rounding_is_forgiven_and_a_missing_layer_is_not() {
        let a = gradient(32, 32, 0);
        let mut rounded = a.clone();
        for px in rounded.pixels_mut().chunks_exact_mut(4) {
            px[0] = px[0].saturating_add(1);
        }
        assert!(ssim(&a, &rounded) > 0.99, "{}", ssim(&a, &rounded));
        assert!(ssim(&a, &Frame::black(32, 32)) < 0.5);
        let mut flat = Frame::black(32, 32);
        flat.fill([128, 128, 128, 255]);
        assert!(ssim(&a, &flat) < 0.9);
        // A window smaller than eight a side still counts.
        let tiny = gradient(3, 5, 0);
        assert!((ssim(&tiny, &tiny) - 1.0).abs() < 1e-9);
    }
}

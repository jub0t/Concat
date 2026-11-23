// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The FLIP gate: how different a picture looks from its reference, pixel
//! by pixel, as NVIDIA's FLIP measures it, and the two numbers the gate is
//! set on.
//!
//! FLIP (Andersson and others, "FLIP: A Difference Evaluator for
//! Alternating Images", 2020) answers what a person flipping between two
//! pictures would see: it filters both by the eye's contrast sensitivity
//! at a stated viewing distance, compares colour in a perceptually even
//! space and edges and points apart from it, and gives every pixel an
//! error from nought, the same, to one. It is the measure wgpu's own test
//! suite judges its pictures by, through the same `nv-flip` crate, and the
//! two numbers here are the two its tests bound:
//!
//! - the **mean** of the map, which the paper recommends where one number
//!   must stand for the whole, and which a change spread over the frame
//!   moves;
//! - a **high percentile**, taken FLIP's way: by weight of error, not by
//!   count of pixels. The pixels are sorted by error and summed from the
//!   least; the percentile is the error of the pixel at which the running
//!   sum passes 99% of the total. Counted by pixels, the 99th percentile
//!   of a picture with a wrong speck on it is nought, since 99% of its
//!   pixels are right; by weight it is the speck's own error, since nearly
//!   all the error there is lies in the speck. So this is the number a
//!   small, strong change moves, and the mean cannot.
//!
//! Both are worked out here, in double precision, from the map `nv-flip`
//! returns, rather than asked of its pool, which sums in single
//! precision. A test below holds them to the pool's.

/// How many pixels fill a degree of the viewer's sight: FLIP's own
/// default, a 4K monitor 0.7 m wide seen from 0.7 m. It sets how fine a
/// detail still counts, and the bounds in this module are sized for it.
pub const PIXELS_PER_DEGREE: f32 = nv_flip::DEFAULT_PIXELS_PER_DEGREE;

/// The share of the error, by weight, under the percentile gated on.
pub const PERCENTILE: f64 = 0.99;

/// What FLIP makes of two pictures.
pub struct Flip {
    /// The mean error over the picture.
    pub mean: f64,
    /// The error at [`PERCENTILE`] of the error's weight.
    pub percentile: f64,
    /// Every pixel's error, nought to one, row by row.
    pub map: Vec<f32>,
    /// The map as a picture, three bytes a pixel, in FLIP's own magma
    /// colours: black for the same, through purple and orange to pale
    /// yellow for as different as can be.
    pub heat: Vec<u8>,
}

/// Compares `got` with `reference`: both opaque RGB, three bytes a pixel,
/// `width` × `height`, which the caller has checked.
pub fn flip(reference: &[u8], got: &[u8], width: u32, height: u32) -> Flip {
    let reference = nv_flip::FlipImageRgb8::with_data(width, height, reference);
    let got = nv_flip::FlipImageRgb8::with_data(width, height, got);
    let error = nv_flip::flip(reference, got, PIXELS_PER_DEGREE);
    let map = error.to_vec();
    let heat = error.apply_color_lut(&nv_flip::magma_lut()).to_vec();
    let (mean, percentile) = pool(&map);
    Flip {
        mean,
        percentile,
        map,
        heat,
    }
}

/// The mean of `map` and its [`PERCENTILE`] by weight.
fn pool(map: &[f32]) -> (f64, f64) {
    if map.is_empty() {
        return (0.0, 0.0);
    }
    let mut sorted: Vec<f32> = map.to_vec();
    sorted.sort_by(f32::total_cmp);
    let total: f64 = sorted.iter().map(|&error| f64::from(error)).sum();
    let mut running = 0.0;
    let mut percentile = 0.0;
    for &error in &sorted {
        running += f64::from(error);
        if running > PERCENTILE * total {
            percentile = f64::from(error);
            break;
        }
    }
    (total / map.len() as f64, percentile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_speck_is_nothing_to_the_mean_and_everything_to_the_percentile() {
        // A thousand pixels, three of them wrong.
        let mut map = vec![0.0_f32; 1000];
        map[400] = 0.5;
        map[401] = 0.75;
        map[402] = 0.75;
        let (mean, percentile) = pool(&map);
        assert!((mean - 0.002).abs() < 1e-9, "{mean}");
        assert_eq!(percentile, 0.75);
        // And where the error is even, the percentile is that error.
        let (mean, percentile) = pool(&[0.25; 64]);
        assert_eq!((mean, percentile), (0.25, 0.25));
        assert_eq!(pool(&[0.0; 64]), (0.0, 0.0));
        assert_eq!(pool(&[]), (0.0, 0.0));
    }

    #[test]
    fn the_mean_and_percentile_are_the_ones_flips_own_pool_gives() {
        let (width, height) = (64_u32, 48_u32);
        let mut reference = Vec::new();
        let mut got = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let level = (x * 4) as u8;
                reference.extend_from_slice(&[level, (y * 5) as u8, 128]);
                // A band a little lighter, and a block of it much darker.
                let lift = if (20..28).contains(&y) { 6 } else { 0 };
                let drop = if (40..48).contains(&x) && y < 8 {
                    90
                } else {
                    0
                };
                got.extend_from_slice(&[
                    level.saturating_add(lift).saturating_sub(drop),
                    (y * 5) as u8,
                    128,
                ]);
            }
        }
        let ours = flip(&reference, &got, width, height);
        let mut pool = nv_flip::FlipPool::from_image(&nv_flip::flip(
            nv_flip::FlipImageRgb8::with_data(width, height, &reference),
            nv_flip::FlipImageRgb8::with_data(width, height, &got),
            PIXELS_PER_DEGREE,
        ));
        assert!(ours.mean > 0.01, "the pictures differ: {}", ours.mean);
        assert!((ours.mean - f64::from(pool.mean())).abs() < 1e-5);
        let theirs = f64::from(pool.get_percentile(PERCENTILE as f32, true));
        assert!(
            (ours.percentile - theirs).abs() < 1e-3,
            "{} against the pool's {theirs}",
            ours.percentile
        );
        assert_eq!(ours.map.len(), (width * height) as usize);
        assert_eq!(ours.heat.len(), (width * height * 3) as usize);
    }

    #[test]
    fn the_same_picture_twice_is_no_error_anywhere_and_a_black_map() {
        let picture: Vec<u8> = (0..32 * 32 * 3).map(|at| (at % 251) as u8).collect();
        let same = flip(&picture, &picture, 32, 32);
        assert_eq!((same.mean, same.percentile), (0.0, 0.0));
        assert!(same.map.iter().all(|&error| error == 0.0));
        // Magma's first colour: all but black.
        assert!(
            same.heat.iter().all(|&level| level < 8),
            "{:?}",
            &same.heat[..3]
        );
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The colour gate: how far the mean colour of each 8 × 8 block of a
//! picture is from the same block of its reference, in CIE ΔE2000, and
//! which block is furthest.
//!
//! FLIP judges a picture pixel by pixel and the gate on it pools those
//! judgements, so a small flat region gone off its hue can sit under
//! both of FLIP's bounds when the rest of the frame carries the usual
//! rounding noise: the region's share of the error is too small to reach
//! the percentile, and far too small to move the mean. And where an
//! adapter's edges have had the percentile loosened for them, a whole
//! field may shift by as much as an edge does and pass. A block's mean
//! does not pool across the frame, and an edge drawn a little
//! differently hardly moves it. Averaging 64 pixels takes the noise out
//! and leaves the shift, so the worst block states the largest flat
//! colour error anywhere in the picture, in a unit where one is about
//! the least difference a person sees side by side.
//!
//! The means are taken in linear light, as light adds, then put into
//! CIELAB under D65. ΔE2000 is implemented here from Sharma, Wu and Dalal,
//! "The CIEDE2000 Color-Difference Formula: Implementation Notes,
//! Supplementary Test Data, and Mathematical Observations" (Color Research
//! and Application 30, 2005), and held to all 34 of that paper's test
//! pairs below.

/// The side of a block, in pixels.
pub const BLOCK: u32 = 8;

/// The block furthest from its reference.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorstBlock {
    /// Its ΔE2000.
    pub delta_e: f64,
    /// Its left edge, in pixels.
    pub x: u32,
    /// Its top edge, in pixels.
    pub y: u32,
}

/// An sRGB level as linear light, 0 to 1.
fn linear(level: u8) -> f64 {
    let value = f64::from(level) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear sRGB as CIELAB under D65. The white is the matrix's own row
/// sums, so a grey has no chroma at all rather than a rounding's worth.
pub fn linear_to_lab(rgb: [f64; 3]) -> [f64; 3] {
    const M: [[f64; 3]; 3] = [
        [0.412_456_4, 0.357_576_1, 0.180_437_5],
        [0.212_672_9, 0.715_152_2, 0.072_175_0],
        [0.019_333_9, 0.119_192_0, 0.950_304_1],
    ];
    let f = |row: &[f64; 3]| {
        let white: f64 = row.iter().sum();
        let t = (row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2]) / white;
        const DELTA: f64 = 6.0 / 29.0;
        if t > DELTA * DELTA * DELTA {
            t.cbrt()
        } else {
            t / (3.0 * DELTA * DELTA) + 4.0 / 29.0
        }
    };
    let (fx, fy, fz) = (f(&M[0]), f(&M[1]), f(&M[2]));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// An sRGB colour as CIELAB under D65.
pub fn srgb_to_lab(rgb: [u8; 3]) -> [f64; 3] {
    linear_to_lab([linear(rgb[0]), linear(rgb[1]), linear(rgb[2])])
}

/// The CIE ΔE2000 difference between two CIELAB colours, with the
/// parametric factors at one. The steps and their numbering are the
/// paper's.
pub fn delta_e_2000(first: [f64; 3], second: [f64; 3]) -> f64 {
    let [l1, a1, b1] = first;
    let [l2, a2, b2] = second;

    // 1. C' and h' of each colour.
    let c_mean = (a1.hypot(b1) + a2.hypot(b2)) / 2.0;
    let c_mean7 = c_mean.powi(7);
    let g = 0.5 * (1.0 - (c_mean7 / (c_mean7 + 25.0_f64.powi(7))).sqrt());
    let (a1, a2) = ((1.0 + g) * a1, (1.0 + g) * a2);
    let (c1, c2) = (a1.hypot(b1), a2.hypot(b2));
    let hue = |a: f64, b: f64| {
        if a == 0.0 && b == 0.0 {
            0.0
        } else {
            b.atan2(a).to_degrees().rem_euclid(360.0)
        }
    };
    let (h1, h2) = (hue(a1, b1), hue(a2, b2));

    // 2. The differences in lightness, chroma and hue.
    let delta_l = l2 - l1;
    let delta_c = c2 - c1;
    let delta_h = if c1 * c2 == 0.0 {
        0.0
    } else if (h2 - h1).abs() <= 180.0 {
        h2 - h1
    } else if h2 - h1 > 180.0 {
        h2 - h1 - 360.0
    } else {
        h2 - h1 + 360.0
    };
    let delta_big_h = 2.0 * (c1 * c2).sqrt() * (delta_h / 2.0).to_radians().sin();

    // 3. The difference itself.
    let l_mean = (l1 + l2) / 2.0;
    let c_mean = (c1 + c2) / 2.0;
    let h_mean = if c1 * c2 == 0.0 {
        h1 + h2
    } else if (h1 - h2).abs() <= 180.0 {
        (h1 + h2) / 2.0
    } else if h1 + h2 < 360.0 {
        (h1 + h2 + 360.0) / 2.0
    } else {
        (h1 + h2 - 360.0) / 2.0
    };
    let cos = |degrees: f64| degrees.to_radians().cos();
    let t =
        1.0 - 0.17 * cos(h_mean - 30.0) + 0.24 * cos(2.0 * h_mean) + 0.32 * cos(3.0 * h_mean + 6.0)
            - 0.20 * cos(4.0 * h_mean - 63.0);
    let delta_theta = 30.0 * (-((h_mean - 275.0) / 25.0).powi(2)).exp();
    let c_mean7 = c_mean.powi(7);
    let r_c = 2.0 * (c_mean7 / (c_mean7 + 25.0_f64.powi(7))).sqrt();
    let l_50 = (l_mean - 50.0).powi(2);
    let s_l = 1.0 + 0.015 * l_50 / (20.0 + l_50).sqrt();
    let s_c = 1.0 + 0.045 * c_mean;
    let s_h = 1.0 + 0.015 * c_mean * t;
    let r_t = -(2.0 * delta_theta).to_radians().sin() * r_c;
    let (l, c, h) = (delta_l / s_l, delta_c / s_c, delta_big_h / s_h);
    (l * l + c * c + h * h + r_t * c * h).sqrt()
}

/// The block of `got` whose mean colour is furthest from the same block
/// of `reference`. Both are opaque RGB, three bytes a pixel, `width` ×
/// `height`. A picture whose sides are not multiples of eight ends in
/// narrower blocks along its right and bottom edges, which are judged as
/// they are.
pub fn worst_block(reference: &[u8], got: &[u8], width: u32, height: u32) -> WorstBlock {
    let table: Vec<f64> = (0..=255).map(linear).collect();
    let mean = |pixels: &[u8], left: u32, top: u32| {
        let right = (left + BLOCK).min(width);
        let bottom = (top + BLOCK).min(height);
        let mut sum = [0.0; 3];
        for y in top..bottom {
            let row = (y as usize * width as usize + left as usize) * 3;
            let row = &pixels[row..row + (right - left) as usize * 3];
            for pixel in row.chunks_exact(3) {
                for (total, &level) in sum.iter_mut().zip(pixel) {
                    *total += table[usize::from(level)];
                }
            }
        }
        let count = f64::from((right - left) * (bottom - top));
        linear_to_lab(sum.map(|total| total / count))
    };
    let mut worst = WorstBlock {
        delta_e: 0.0,
        x: 0,
        y: 0,
    };
    for top in (0..height).step_by(BLOCK as usize) {
        for left in (0..width).step_by(BLOCK as usize) {
            let delta_e = delta_e_2000(mean(reference, left, top), mean(got, left, top));
            if delta_e > worst.delta_e {
                worst = WorstBlock {
                    delta_e,
                    x: left,
                    y: top,
                };
            }
        }
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Table 1 of Sharma, Wu and Dalal (2005): L*, a*, b* of each colour
    /// of a pair, and the pair's ΔE2000. Checked against the data file the
    /// authors publish beside the paper.
    const PAIRS: [[f64; 7]; 34] = [
        [50.0, 2.6772, -79.7751, 50.0, 0.0, -82.7485, 2.0425],
        [50.0, 3.1571, -77.2803, 50.0, 0.0, -82.7485, 2.8615],
        [50.0, 2.8361, -74.0200, 50.0, 0.0, -82.7485, 3.4412],
        [50.0, -1.3802, -84.2814, 50.0, 0.0, -82.7485, 1.0000],
        [50.0, -1.1848, -84.8006, 50.0, 0.0, -82.7485, 1.0000],
        [50.0, -0.9009, -85.5211, 50.0, 0.0, -82.7485, 1.0000],
        [50.0, 0.0, 0.0, 50.0, -1.0, 2.0, 2.3669],
        [50.0, -1.0, 2.0, 50.0, 0.0, 0.0, 2.3669],
        [50.0, 2.49, -0.001, 50.0, -2.49, 0.0009, 7.1792],
        [50.0, 2.49, -0.001, 50.0, -2.49, 0.0010, 7.1792],
        [50.0, 2.49, -0.001, 50.0, -2.49, 0.0011, 7.2195],
        [50.0, 2.49, -0.001, 50.0, -2.49, 0.0012, 7.2195],
        [50.0, -0.001, 2.49, 50.0, 0.0009, -2.49, 4.8045],
        [50.0, -0.001, 2.49, 50.0, 0.0010, -2.49, 4.8045],
        [50.0, -0.001, 2.49, 50.0, 0.0011, -2.49, 4.7461],
        [50.0, 2.5, 0.0, 50.0, 0.0, -2.5, 4.3065],
        [50.0, 2.5, 0.0, 73.0, 25.0, -18.0, 27.1492],
        [50.0, 2.5, 0.0, 61.0, -5.0, 29.0, 22.8977],
        [50.0, 2.5, 0.0, 56.0, -27.0, -3.0, 31.9030],
        [50.0, 2.5, 0.0, 58.0, 24.0, 15.0, 19.4535],
        [50.0, 2.5, 0.0, 50.0, 3.1736, 0.5854, 1.0000],
        [50.0, 2.5, 0.0, 50.0, 3.2972, 0.0, 1.0000],
        [50.0, 2.5, 0.0, 50.0, 1.8634, 0.5757, 1.0000],
        [50.0, 2.5, 0.0, 50.0, 3.2592, 0.3350, 1.0000],
        [
            60.2574, -34.0099, 36.2677, 60.4626, -34.1751, 39.4387, 1.2644,
        ],
        [
            63.0109, -31.0961, -5.8663, 62.8187, -29.7946, -4.0864, 1.2630,
        ],
        [61.2901, 3.7196, -5.3901, 61.4292, 2.2480, -4.9620, 1.8731],
        [35.0831, -44.1164, 3.7933, 35.0232, -40.0716, 1.5901, 1.8645],
        [
            22.7233, 20.0904, -46.6940, 23.0331, 14.9730, -42.5619, 2.0373,
        ],
        [36.4612, 47.8580, 18.3852, 36.2715, 50.5065, 21.2231, 1.4146],
        [90.8027, -2.0831, 1.4410, 91.1528, -1.6435, 0.0447, 1.4441],
        [90.9257, -0.5406, -0.9208, 88.6381, -0.8985, -0.7239, 1.5381],
        [6.7747, -0.2908, -2.4247, 5.8714, -0.0985, -2.2286, 0.6377],
        [2.0776, 0.0795, -1.1350, 0.9033, -0.0636, -0.5514, 0.9082],
    ];

    #[test]
    fn all_34_published_pairs_come_out_to_four_places_either_way_round() {
        for (at, pair) in PAIRS.iter().enumerate() {
            let first = [pair[0], pair[1], pair[2]];
            let second = [pair[3], pair[4], pair[5]];
            for (one, other) in [(first, second), (second, first)] {
                let found = delta_e_2000(one, other);
                assert!(
                    (found - pair[6]).abs() < 0.000_05,
                    "pair {}: {found:.5}, published as {:.4}",
                    at + 1,
                    pair[6]
                );
            }
        }
    }

    #[test]
    fn a_colour_is_no_distance_from_itself() {
        for pair in PAIRS {
            let colour = [pair[0], pair[1], pair[2]];
            assert_eq!(delta_e_2000(colour, colour), 0.0);
        }
    }

    #[test]
    fn white_grey_and_the_primaries_land_where_cielab_puts_them() {
        let close = |found: [f64; 3], wanted: [f64; 3]| {
            found
                .iter()
                .zip(wanted)
                .all(|(found, wanted)| (found - wanted).abs() < 0.01)
        };
        assert!(close(srgb_to_lab([255, 255, 255]), [100.0, 0.0, 0.0]));
        assert!(close(srgb_to_lab([0, 0, 0]), [0.0, 0.0, 0.0]));
        // Mid grey, and the three primaries, as every sRGB-to-Lab table
        // has them under D65.
        assert!(close(srgb_to_lab([119, 119, 119]), [50.03, 0.0, 0.0]));
        assert!(close(srgb_to_lab([255, 0, 0]), [53.24, 80.09, 67.20]));
        assert!(close(srgb_to_lab([0, 255, 0]), [87.73, -86.18, 83.18]));
        assert!(close(srgb_to_lab([0, 0, 255]), [32.30, 79.19, -107.86]));
    }

    #[test]
    fn the_worst_block_is_the_one_that_changed_and_is_named_by_its_corner() {
        let (width, height) = (20, 12);
        let reference = vec![128_u8; width * height * 3];
        let mut got = reference.clone();
        // One pixel of the block at (16, 8), which is 4 × 4 here: the
        // picture's sides are not multiples of eight.
        let at = (9 * width + 17) * 3;
        got[at] = 255;
        let worst = worst_block(&reference, &got, width as u32, height as u32);
        assert_eq!((worst.x, worst.y), (16, 8));
        assert!(worst.delta_e > 1.0, "{}", worst.delta_e);
        let same = worst_block(&reference, &reference, width as u32, height as u32);
        assert_eq!(same.delta_e, 0.0);
    }
}

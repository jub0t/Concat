// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How far apart renders of one frame come: the measurement a bound is
//! set from.
//!
//! Whether a software adapter draws the same bytes every run is a
//! question to answer by drawing, not by reasoning. Draw one frame
//! several times, on one machine or on several of one family, and hand
//! the pictures here: the answer is either that they are byte for byte
//! the same, or how far the furthest two are from each other in the
//! three numbers a golden is bounded by. A bound for that family then
//! has a measurement under it and not a guess.

use serde::Serialize;

use super::bounds::Measured;
use super::judge;
use super::picture::Rgba8Image;

/// What a set of renders of one frame measured.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spread {
    /// How many renders there were.
    pub renders: usize,
    /// Every render is byte for byte the first.
    pub identical: bool,
    /// How many of the pairs of renders differ at all.
    pub differing_pairs: usize,
    /// The most pixels any pair differs in.
    pub differing_pixels: usize,
    /// The most levels any channel of any pixel differs by in any pair,
    /// alpha included.
    pub levels: u8,
    /// The worst of each of a golden's three numbers over every pair:
    /// each the largest found, not one pair's three. All nought when the
    /// renders are identical.
    pub worst: Measured,
}

impl Spread {
    /// The spread in a line.
    pub fn summary(&self) -> String {
        if self.identical {
            return format!("{} renders, byte for byte the same", self.renders);
        }
        format!(
            "{} renders, {} pairs of them different: up to {} pixels and {} levels apart, FLIP's mean up to {:.4}, its 99th percentile up to {:.4}, the worst block's ΔE up to {:.2}",
            self.renders,
            self.differing_pairs,
            self.differing_pixels,
            self.levels,
            self.worst.flip_mean,
            self.worst.flip_p99,
            self.worst.delta_e
        )
    }
}

/// Measures how far apart `renders` of one frame are, every pair of
/// them. An error if there are fewer than two, or they are not all one
/// size.
pub fn spread(renders: &[Rgba8Image]) -> Result<Spread, String> {
    let [first, rest @ ..] = renders else {
        return Err("no renders to compare".to_owned());
    };
    if rest.is_empty() {
        return Err("one render has nothing to be compared with".to_owned());
    }
    if let Some(at) = rest
        .iter()
        .position(|render| (render.width(), render.height()) != (first.width(), first.height()))
    {
        let odd = &rest[at];
        return Err(format!(
            "render {} is {} × {} and the first {} × {}",
            at + 1,
            odd.width(),
            odd.height(),
            first.width(),
            first.height()
        ));
    }
    let mut spread = Spread {
        renders: renders.len(),
        identical: true,
        differing_pairs: 0,
        differing_pixels: 0,
        levels: 0,
        worst: Measured {
            flip_mean: 0.0,
            flip_p99: 0.0,
            delta_e: 0.0,
        },
    };
    for (at, one) in renders.iter().enumerate() {
        for other in &renders[at + 1..] {
            if one.pixels() == other.pixels() {
                continue;
            }
            spread.identical = false;
            spread.differing_pairs += 1;
            let mut pixels = 0;
            for (a, b) in one
                .pixels()
                .chunks_exact(4)
                .zip(other.pixels().chunks_exact(4))
            {
                pixels += usize::from(a != b);
                for (a, b) in a.iter().zip(b) {
                    spread.levels = spread.levels.max(a.abs_diff(*b));
                }
            }
            spread.differing_pixels = spread.differing_pixels.max(pixels);
            let found = judge(one, other).measured;
            spread.worst.flip_mean = spread.worst.flip_mean.max(found.flip_mean);
            spread.worst.flip_p99 = spread.worst.flip_p99.max(found.flip_p99);
            spread.worst.delta_e = spread.worst.delta_e.max(found.delta_e);
        }
    }
    Ok(spread)
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The camera's movement between two frames, measured.
//!
//! Corners are where tracking works: a point on an edge can slide along
//! it, a point on a flat wall can be anywhere. Shi-Tomasi's measure - the
//! smaller eigenvalue of the gradients' structure around a pixel - says
//! how cornery a pixel is, and the strongest are taken cell by cell over
//! a grid so the tracks cover the frame rather than bunch on one busy
//! patch. Each is followed into the next frame by Lucas-Kanade optical
//! flow, coarse to fine over an image pyramid so a fast move is found at
//! a size where it is small, and then followed back again: a track that
//! does not return to where it started was lost, and is dropped.
//!
//! The tracks that survive are fitted by [`super::fit::ransac`]. Its
//! agreeing tracks carry on into the next frame; when too few are left,
//! the grid is topped up with fresh corners in the cells they left empty.
//! A frame with too little texture to find any - a white wall, a black
//! frame - is said to be flat, and the caller records no movement there.

use concat_core::frame::Frame;

use super::fit::{self, Pair, Similarity};

/// One channel of a picture, as floats: brightness `0..=255`.
#[derive(Clone, Debug)]
pub struct Gray {
    /// Pixels across.
    pub width: usize,
    /// Pixels down.
    pub height: usize,
    /// Row by row.
    pub data: Vec<f32>,
}

impl Gray {
    /// A picture of one brightness.
    pub fn filled(width: usize, height: usize, value: f32) -> Gray {
        Gray {
            width,
            height,
            data: vec![value; width * height],
        }
    }

    /// A frame's luma, Rec. 601 weights: what the eye calls detail.
    pub fn of_frame(frame: &Frame) -> Gray {
        let (width, height) = (frame.width() as usize, frame.height() as usize);
        let data = frame
            .pixels()
            .chunks_exact(4)
            .map(|px| {
                0.299 * f32::from(px[0]) + 0.587 * f32::from(px[1]) + 0.114 * f32::from(px[2])
            })
            .collect();
        Gray {
            width,
            height,
            data,
        }
    }

    #[inline]
    fn at(&self, x: isize, y: isize) -> f32 {
        let x = x.clamp(0, self.width as isize - 1) as usize;
        let y = y.clamp(0, self.height as isize - 1) as usize;
        self.data[y * self.width + x]
    }

    /// The brightness at `(x, y)`, pixel centres at whole numbers, read
    /// bilinearly with the edges held.
    #[inline]
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        let x0 = x.floor();
        let y0 = y.floor();
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as isize, y0 as isize);
        if x0 >= 0 && y0 >= 0 && (x0 as usize) + 1 < self.width && (y0 as usize) + 1 < self.height {
            let at = y0 as usize * self.width + x0 as usize;
            let (a, b) = (self.data[at], self.data[at + 1]);
            let (c, d) = (self.data[at + self.width], self.data[at + self.width + 1]);
            let top = a + (b - a) * fx;
            let bottom = c + (d - c) * fx;
            return top + (bottom - top) * fy;
        }
        let top = self.at(x0, y0) * (1.0 - fx) + self.at(x0 + 1, y0) * fx;
        let bottom = self.at(x0, y0 + 1) * (1.0 - fx) + self.at(x0 + 1, y0 + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// Half the size each way, blurred first by the binomial five-tap so
    /// what is dropped cannot alias into what is kept.
    fn half(&self) -> Gray {
        const TAPS: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];
        let (w, h) = (self.width, self.height);
        let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
        // Across, at every row and every other column.
        let mut across = vec![0.0f32; hw * h];
        for y in 0..h {
            for x in 0..hw {
                let cx = (2 * x) as isize;
                let mut sum = 0.0;
                for (k, tap) in TAPS.iter().enumerate() {
                    sum += tap * self.at(cx + k as isize - 2, y as isize);
                }
                across[y * hw + x] = sum;
            }
        }
        let across = Gray {
            width: hw,
            height: h,
            data: across,
        };
        let mut data = vec![0.0f32; hw * hh];
        for y in 0..hh {
            let cy = (2 * y) as isize;
            for x in 0..hw {
                let mut sum = 0.0;
                for (k, tap) in TAPS.iter().enumerate() {
                    sum += tap * across.at(x as isize, cy + k as isize - 2);
                }
                data[y * hw + x] = sum;
            }
        }
        Gray {
            width: hw,
            height: hh,
            data,
        }
    }

    /// Scharr's derivatives across and down, in brightness per pixel.
    fn gradients(&self) -> (Gray, Gray) {
        let (w, h) = (self.width, self.height);
        let mut gx = vec![0.0f32; w * h];
        let mut gy = vec![0.0f32; w * h];
        for y in 0..h as isize {
            for x in 0..w as isize {
                let at = y as usize * w + x as usize;
                gx[at] = (3.0 * (self.at(x + 1, y - 1) - self.at(x - 1, y - 1))
                    + 10.0 * (self.at(x + 1, y) - self.at(x - 1, y))
                    + 3.0 * (self.at(x + 1, y + 1) - self.at(x - 1, y + 1)))
                    / 32.0;
                gy[at] = (3.0 * (self.at(x - 1, y + 1) - self.at(x - 1, y - 1))
                    + 10.0 * (self.at(x, y + 1) - self.at(x, y - 1))
                    + 3.0 * (self.at(x + 1, y + 1) - self.at(x + 1, y - 1)))
                    / 32.0;
            }
        }
        (
            Gray {
                width: w,
                height: h,
                data: gx,
            },
            Gray {
                width: w,
                height: h,
                data: gy,
            },
        )
    }
}

/// One level of a pyramid: the picture and its derivatives.
struct Level {
    image: Gray,
    gx: Gray,
    gy: Gray,
}

/// A picture at full size and at every halving down to a few dozen
/// pixels, each with its derivatives.
pub struct Pyramid {
    levels: Vec<Level>,
}

/// The tracking window's half-width: fifteen pixels square.
const RADIUS: isize = 7;
/// The most halvings: at 480 pixels, the top is 60 across, where a move of
/// a hundred pixels at full size is a dozen.
const LEVELS: usize = 4;
/// Steps of the flow per level before it stops refining.
const ITERATIONS: usize = 20;
/// A step this small, in pixels, is converged.
const CONVERGED: f32 = 0.01;
/// A track whose way back lands further than this from its start, in
/// full-size pixels, was lost.
const ROUND_TRIP: f32 = 0.7;

impl Pyramid {
    /// The pyramid of `image`.
    pub fn of(image: Gray) -> Pyramid {
        let mut levels = Vec::with_capacity(LEVELS);
        let mut current = image;
        loop {
            let (gx, gy) = current.gradients();
            let small = current.width.min(current.height);
            let next = (levels.len() + 1 < LEVELS && small >= 48).then(|| current.half());
            levels.push(Level {
                image: current,
                gx,
                gy,
            });
            match next {
                Some(next) => current = next,
                None => break,
            }
        }
        Pyramid { levels }
    }

    /// The full-size picture.
    pub fn base(&self) -> &Gray {
        &self.levels[0].image
    }

    /// Where the point at `from` in this picture went in `next`, or `None`
    /// when the flow lost it: out of the picture, or on texture too weak
    /// to say. Positions in full-size pixels, centres at whole numbers.
    pub fn flow(&self, next: &Pyramid, from: (f32, f32)) -> Option<(f32, f32)> {
        let levels = self.levels.len().min(next.levels.len());
        let side = (2 * RADIUS + 1) as usize;
        let mut window_i = vec![0.0f32; side * side];
        let mut window_x = vec![0.0f32; side * side];
        let mut window_y = vec![0.0f32; side * side];
        let mut guess = (0.0f32, 0.0f32);
        for level in (0..levels).rev() {
            let scale = 1.0 / (1u32 << level) as f32;
            let here = &self.levels[level];
            let there = &next.levels[level].image;
            let (px, py) = (from.0 * scale, from.1 * scale);
            let (w, h) = (here.image.width as f32, here.image.height as f32);
            if px < 0.0 || py < 0.0 || px > w - 1.0 || py > h - 1.0 {
                return None;
            }
            let (mut gxx, mut gxy, mut gyy) = (0.0f32, 0.0f32, 0.0f32);
            let mut k = 0;
            for dy in -RADIUS..=RADIUS {
                for dx in -RADIUS..=RADIUS {
                    let (x, y) = (px + dx as f32, py + dy as f32);
                    let ix = here.gx.sample(x, y);
                    let iy = here.gy.sample(x, y);
                    window_i[k] = here.image.sample(x, y);
                    window_x[k] = ix;
                    window_y[k] = iy;
                    gxx += ix * ix;
                    gxy += ix * iy;
                    gyy += iy * iy;
                    k += 1;
                }
            }
            let det = gxx * gyy - gxy * gxy;
            let area = (side * side) as f32;
            let min_eigen = 0.5 * (gxx + gyy - ((gxx - gyy).powi(2) + 4.0 * gxy * gxy).sqrt());
            if det.abs() < 1e-6 || min_eigen / area < 0.25 {
                if level == 0 {
                    return None;
                }
                guess = (guess.0 * 2.0, guess.1 * 2.0);
                continue;
            }
            let mut step = (0.0f32, 0.0f32);
            for _ in 0..ITERATIONS {
                let (qx, qy) = (px + guess.0 + step.0, py + guess.1 + step.1);
                if qx < -(RADIUS as f32)
                    || qy < -(RADIUS as f32)
                    || qx > w - 1.0 + RADIUS as f32
                    || qy > h - 1.0 + RADIUS as f32
                {
                    return None;
                }
                let (mut bx, mut by) = (0.0f32, 0.0f32);
                let mut k = 0;
                for dy in -RADIUS..=RADIUS {
                    for dx in -RADIUS..=RADIUS {
                        let residual = window_i[k] - there.sample(qx + dx as f32, qy + dy as f32);
                        bx += residual * window_x[k];
                        by += residual * window_y[k];
                        k += 1;
                    }
                }
                let ex = (gyy * bx - gxy * by) / det;
                let ey = (gxx * by - gxy * bx) / det;
                step.0 += ex;
                step.1 += ey;
                if ex * ex + ey * ey < CONVERGED * CONVERGED {
                    break;
                }
            }
            guess = (guess.0 + step.0, guess.1 + step.1);
            if level > 0 {
                guess = (guess.0 * 2.0, guess.1 * 2.0);
            }
        }
        let to = (from.0 + guess.0, from.1 + guess.1);
        let base = &next.levels[0].image;
        (to.0 >= 0.0
            && to.1 >= 0.0
            && to.0 <= base.width as f32 - 1.0
            && to.1 <= base.height as f32 - 1.0)
            .then_some(to)
    }

    /// The strongest corners, spread cell by cell over a grid of cells
    /// about `cell` pixels square, at most `per_cell` in each, and none
    /// nearer than `spacing` pixels to another or to one of `taken`.
    pub fn corners(
        &self,
        cell: usize,
        per_cell: usize,
        spacing: f32,
        taken: &[(f32, f32)],
    ) -> Vec<(f32, f32)> {
        let level = &self.levels[0];
        let (w, h) = (level.image.width, level.image.height);
        let margin = RADIUS as usize + 3;
        if w <= 2 * margin + 2 || h <= 2 * margin + 2 {
            return Vec::new();
        }
        // The structure tensor summed over five by five, by running sums.
        let products: [Vec<f32>; 3] = [
            level.gx.data.iter().map(|g| g * g).collect(),
            level
                .gx
                .data
                .iter()
                .zip(&level.gy.data)
                .map(|(a, b)| a * b)
                .collect(),
            level.gy.data.iter().map(|g| g * g).collect(),
        ];
        let boxed: Vec<Vec<f32>> = products.iter().map(|p| box5(p, w, h)).collect();
        let mut score = vec![0.0f32; w * h];
        let mut strongest = 0.0f32;
        for y in margin..h - margin {
            for x in margin..w - margin {
                let at = y * w + x;
                let (a, b, c) = (boxed[0][at], boxed[1][at], boxed[2][at]);
                let lambda = 0.5 * (a + c - ((a - c) * (a - c) + 4.0 * b * b).sqrt());
                score[at] = lambda;
                strongest = strongest.max(lambda);
            }
        }
        // Weaker than a hundredth of the best, or than an average gradient
        // of about one level a pixel, is not a corner worth following.
        let floor = (strongest * 0.01).max(25.0);
        let columns = w.div_ceil(cell).max(1);
        let rows = h.div_ceil(cell).max(1);
        let mut cells: Vec<Vec<(f32, usize)>> = vec![Vec::new(); columns * rows];
        for y in margin..h - margin {
            for x in margin..w - margin {
                let at = y * w + x;
                let s = score[at];
                if s < floor {
                    continue;
                }
                // A local maximum over its eight neighbours.
                let peak = [
                    at - w - 1,
                    at - w,
                    at - w + 1,
                    at - 1,
                    at + 1,
                    at + w - 1,
                    at + w,
                    at + w + 1,
                ]
                .iter()
                .all(|&n| score[n] <= s);
                if peak {
                    cells[(y / cell) * columns + x / cell].push((s, at));
                }
            }
        }
        let mut chosen: Vec<(f32, f32)> = Vec::new();
        let near = |p: (f32, f32), list: &[(f32, f32)]| {
            list.iter()
                .any(|q| (p.0 - q.0).powi(2) + (p.1 - q.1).powi(2) < spacing * spacing)
        };
        // A cell already holding tracks has that much less room.
        let mut held = vec![0usize; columns * rows];
        for p in taken {
            let (cx, cy) = (p.0.max(0.0) as usize / cell, p.1.max(0.0) as usize / cell);
            if cx < columns && cy < rows {
                held[cy * columns + cx] += 1;
            }
        }
        for (index, candidates) in cells.iter_mut().enumerate() {
            candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
            let mut room = per_cell.saturating_sub(held[index]);
            for &(_, at) in candidates.iter() {
                if room == 0 {
                    break;
                }
                let p = ((at % w) as f32, (at / w) as f32);
                if near(p, taken) || near(p, &chosen) {
                    continue;
                }
                chosen.push(p);
                room -= 1;
            }
        }
        chosen
    }
}

/// `values` summed over the five by five around each pixel, the edges
/// held.
fn box5(values: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut across = vec![0.0f32; w * h];
    for y in 0..h {
        let row = &values[y * w..(y + 1) * w];
        for x in 0..w {
            let mut sum = 0.0;
            for d in -2isize..=2 {
                sum += row[(x as isize + d).clamp(0, w as isize - 1) as usize];
            }
            across[y * w + x] = sum;
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0.0;
            for d in -2isize..=2 {
                sum += across[(y as isize + d).clamp(0, h as isize - 1) as usize * w + x];
            }
            out[y * w + x] = sum;
        }
    }
    out
}

/// What one frame told the tracker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Measured {
    /// The first frame: nothing to measure against.
    First,
    /// The camera moved like this from the frame before, in normalised
    /// units: where a still point of the scene was, to where it is now.
    Moved(Similarity),
    /// Too little texture, or too little agreement, to say.
    Flat,
}

/// Cells about this many pixels square at the analysis size: eight across
/// a 480-pixel frame.
const CELL: usize = 60;
/// Corners a cell may hold.
const PER_CELL: usize = 5;
/// The nearest two corners may be, in pixels.
const SPACING: f32 = 8.0;
/// A track agreeing with the camera lands within this many pixels of
/// where the camera's movement sends it.
const AGREE: f64 = 1.5;
/// Fewer agreeing tracks than this is no answer.
const MIN_AGREEING: usize = 10;

/// Follows the camera through a run of frames, one at a time.
pub struct Tracker {
    previous: Option<Pyramid>,
    points: Vec<(f32, f32)>,
    threads: usize,
}

impl Default for Tracker {
    fn default() -> Self {
        Tracker::new()
    }
}

impl Tracker {
    /// A tracker that has seen nothing.
    pub fn new() -> Tracker {
        Tracker {
            previous: None,
            points: Vec::new(),
            threads: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
                .clamp(1, 8),
        }
    }

    /// Forgets the frames seen: the next one starts a new run.
    pub fn reset(&mut self) {
        self.previous = None;
        self.points.clear();
    }

    /// Takes the next frame of the run and measures how the camera moved
    /// from the one before.
    pub fn push(&mut self, image: Gray) -> Measured {
        let next = Pyramid::of(image);
        let Some(previous) = self.previous.take() else {
            self.points = next.corners(CELL, PER_CELL, SPACING, &[]);
            self.previous = Some(next);
            return Measured::First;
        };
        let (w, h) = (next.base().width, next.base().height);
        let tracked = self.follow(&previous, &next);
        let normal = |p: (f32, f32)| {
            (
                (f64::from(p.0) + 0.5 - w as f64 / 2.0) / h as f64,
                (f64::from(p.1) + 0.5 - h as f64 / 2.0) / h as f64,
            )
        };
        let pairs: Vec<Pair> = tracked
            .iter()
            .map(|&(from, to)| (normal(from), normal(to)))
            .collect();
        let robust = fit::ransac(&pairs, AGREE / h as f64, MIN_AGREEING);
        let (measured, mut survivors) = match &robust {
            Some(robust) => (
                Measured::Moved(robust.similarity),
                tracked
                    .iter()
                    .zip(&robust.inliers)
                    .filter(|(_, inlier)| **inlier)
                    .map(|((_, to), _)| *to)
                    .collect::<Vec<_>>(),
            ),
            None => (Measured::Flat, Vec::new()),
        };
        // Topped up when a good share of the grid has gone quiet.
        let full = w.div_ceil(CELL) * h.div_ceil(CELL) * PER_CELL;
        if survivors.len() < full * 3 / 5 {
            let fresh = next.corners(CELL, PER_CELL, SPACING, &survivors);
            survivors.extend(fresh);
        }
        self.points = survivors;
        self.previous = Some(next);
        measured
    }

    /// Every point followed from `previous` into `next` and back, the
    /// ones that made the round trip paired with where they went. Split
    /// over a few threads: the flow is most of an analysis's time.
    fn follow(&self, previous: &Pyramid, next: &Pyramid) -> Vec<((f32, f32), (f32, f32))> {
        let one = |from: (f32, f32)| -> Option<((f32, f32), (f32, f32))> {
            let to = previous.flow(next, from)?;
            let back = next.flow(previous, to)?;
            ((back.0 - from.0).powi(2) + (back.1 - from.1).powi(2) <= ROUND_TRIP * ROUND_TRIP)
                .then_some((from, to))
        };
        let chunk = self.points.len().div_ceil(self.threads).max(16);
        if self.threads <= 1 || self.points.len() <= chunk {
            return self.points.iter().filter_map(|&p| one(p)).collect();
        }
        std::thread::scope(|scope| {
            let handles: Vec<_> = self
                .points
                .chunks(chunk)
                .map(|part| {
                    scope.spawn(move || part.iter().filter_map(|&p| one(p)).collect::<Vec<_>>())
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|handle| handle.join().unwrap_or_default())
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture with texture everywhere: a sum of waves of a few
    /// periods and angles, not periodic over the frame, with corners
    /// wherever two cross.
    fn scene(x: f64, y: f64) -> f64 {
        let mut v = 128.0;
        for (k, (fx, fy, phase)) in [
            (0.071, 0.023, 0.3),
            (-0.031, 0.083, 1.1),
            (0.11, 0.097, 2.0),
            (0.017, -0.13, 0.7),
            (0.29, 0.05, 1.7),
        ]
        .iter()
        .enumerate()
        {
            v += (30.0 / (k as f64 + 1.0).sqrt()) * (x * fx + y * fy + phase).sin();
        }
        // Soft-edged squares, for corners proper: a lens never draws an
        // edge sharper than a pixel.
        v + 25.0
            * (3.0
                * (x * std::f64::consts::PI / 23.0).sin()
                * (y * std::f64::consts::PI / 19.0).sin())
            .tanh()
    }

    /// The scene seen through `camera` (normalised units): each pixel of
    /// the frame shows the scene point the inverse movement brings it
    /// from.
    fn frame(width: usize, height: usize, camera: &Similarity) -> Gray {
        let back = camera.inverse();
        let h = height as f64;
        let mut data = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let n = (
                    (x as f64 + 0.5 - width as f64 / 2.0) / h,
                    (y as f64 + 0.5 - h / 2.0) / h,
                );
                let (sx, sy) = back.apply(n);
                let (px, py) = (sx * h + width as f64 / 2.0, sy * h + h / 2.0);
                // Supersampled four times, as a lens integrates.
                let mut sum = 0.0;
                for (ox, oy) in [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)] {
                    sum += scene(px + ox, py + oy);
                }
                data.push((sum / 4.0) as f32);
            }
        }
        Gray {
            width,
            height,
            data,
        }
    }

    fn moved(measured: Measured) -> Similarity {
        match measured {
            Measured::Moved(similarity) => similarity,
            other => panic!("expected a movement, got {other:?}"),
        }
    }

    #[test]
    fn a_shift_is_recovered() {
        let (w, h) = (320, 180);
        let shift = Similarity::new(0.0, 1.0, 3.4 / h as f64, -2.2 / h as f64);
        let mut tracker = Tracker::new();
        assert_eq!(
            tracker.push(frame(w, h, &Similarity::IDENTITY)),
            Measured::First
        );
        let found = moved(tracker.push(frame(w, h, &shift)));
        let px = |v: f64| v * h as f64;
        assert!(
            (px(found.tx) - 3.4).abs() < 0.05 && (px(found.ty) + 2.2).abs() < 0.05,
            "{found:?}"
        );
        assert!(found.angle().abs() < 0.002 && (found.scale() - 1.0).abs() < 0.002);
    }

    #[test]
    fn a_turn_and_a_zoom_are_recovered() {
        let (w, h) = (320, 180);
        let camera = Similarity::new(1.5f64.to_radians(), 1.02, -4.0 / h as f64, 1.0 / h as f64);
        let mut tracker = Tracker::new();
        tracker.push(frame(w, h, &Similarity::IDENTITY));
        let found = moved(tracker.push(frame(w, h, &camera)));
        assert!(
            (found.angle().to_degrees() - 1.5).abs() < 0.05,
            "angle {}",
            found.angle().to_degrees()
        );
        assert!(
            (found.scale() - 1.02).abs() < 0.002,
            "scale {}",
            found.scale()
        );
        assert!(((found.tx - camera.tx) * h as f64).abs() < 0.2);
        assert!(((found.ty - camera.ty) * h as f64).abs() < 0.2);
    }

    /// A move of twenty pixels is found through the pyramid, and the
    /// tracks carry on into a third frame.
    #[test]
    fn a_fast_move_is_found_and_tracks_carry_on() {
        let (w, h) = (320, 180);
        let first = Similarity::new(0.0, 1.0, 20.0 / h as f64, 6.0 / h as f64);
        let second = Similarity::new(0.0, 1.0, 26.0 / h as f64, 2.0 / h as f64);
        let mut tracker = Tracker::new();
        tracker.push(frame(w, h, &Similarity::IDENTITY));
        let found = moved(tracker.push(frame(w, h, &first)));
        assert!(((found.tx * h as f64) - 20.0).abs() < 0.15, "{found:?}");
        let found = moved(tracker.push(frame(w, h, &second)));
        assert!(((found.tx * h as f64) - 6.0).abs() < 0.15, "{found:?}");
        assert!(((found.ty * h as f64) + 4.0).abs() < 0.15, "{found:?}");
    }

    /// How long a frame takes at the analysis size; run with `--ignored
    /// --nocapture` in release to see it.
    #[test]
    #[ignore]
    fn time_a_frame_at_the_analysis_size() {
        let (w, h) = (480, 270);
        let frames: Vec<Gray> = (0..31)
            .map(|i| {
                let t = i as f64;
                let camera = Similarity::new(
                    0.002 * (t * 0.7).sin(),
                    1.0,
                    (3.0 * (t * 1.3).sin() + t) / h as f64,
                    (2.0 * (t * 0.9).cos()) / h as f64,
                );
                frame(w, h, &camera)
            })
            .collect();
        let mut tracker = Tracker::new();
        let started = std::time::Instant::now();
        for frame in frames {
            tracker.push(frame);
        }
        let each = started.elapsed().as_secs_f64() * 1000.0 / 31.0;
        println!(
            "{w}x{h}: {each:.2} ms a frame, {} tracks",
            tracker.points.len()
        );
    }

    #[test]
    fn a_blank_frame_is_flat() {
        let mut tracker = Tracker::new();
        tracker.push(Gray::filled(200, 120, 90.0));
        assert_eq!(tracker.push(Gray::filled(200, 120, 90.0)), Measured::Flat);
    }
}

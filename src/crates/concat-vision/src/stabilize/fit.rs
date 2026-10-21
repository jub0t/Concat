// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! One similarity fitted to many tracks, robustly.
//!
//! A similarity is a turn, a uniform zoom and a shift: four numbers, and
//! the most a camera's shake needs. Written the way a complex number
//! multiplies, `p' = z·p + t` with `z = a + bi`, it composes and inverts
//! in a line each and fits by least squares in closed form.
//!
//! The fit is RANSAC: two tracks fix a similarity, so pairs are drawn at
//! random, each one's answer is scored by how many tracks agree with it,
//! and the best is fitted again over everything that agreed. A person
//! walking through the frame is a cluster of tracks that agree with each
//! other and with nothing else; as long as the background has more, the
//! background wins.

/// `p' = z·p + t`, `z = a + bi`: a turn by `atan2(b, a)`, a zoom by
/// `|z|`, then a shift by `t`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Similarity {
    /// The real half of `z`: the zoom times the cosine of the turn.
    pub a: f64,
    /// The imaginary half: the zoom times its sine.
    pub b: f64,
    /// The shift across.
    pub tx: f64,
    /// The shift down.
    pub ty: f64,
}

impl Default for Similarity {
    fn default() -> Self {
        Similarity::IDENTITY
    }
}

impl Similarity {
    /// Nothing moved.
    pub const IDENTITY: Similarity = Similarity {
        a: 1.0,
        b: 0.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// A turn by `angle` radians (clockwise on screen, `y` running down),
    /// a zoom by `scale`, then a shift by `(tx, ty)`.
    pub fn new(angle: f64, scale: f64, tx: f64, ty: f64) -> Similarity {
        let (sin, cos) = angle.sin_cos();
        Similarity {
            a: scale * cos,
            b: scale * sin,
            tx,
            ty,
        }
    }

    /// A zoom about the origin.
    pub fn scaling(scale: f64) -> Similarity {
        Similarity {
            a: scale,
            b: 0.0,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// Where `p` goes.
    pub fn apply(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (
            self.a * x - self.b * y + self.tx,
            self.b * x + self.a * y + self.ty,
        )
    }

    /// This, then `after`: `after(self(p))`.
    pub fn then(&self, after: &Similarity) -> Similarity {
        let a = after.a * self.a - after.b * self.b;
        let b = after.a * self.b + after.b * self.a;
        let (tx, ty) = after.apply((self.tx, self.ty));
        Similarity { a, b, tx, ty }
    }

    /// The map back. A degenerate one - no size at all - is its own.
    pub fn inverse(&self) -> Similarity {
        let norm = self.a * self.a + self.b * self.b;
        if norm <= f64::MIN_POSITIVE {
            return *self;
        }
        let a = self.a / norm;
        let b = -self.b / norm;
        Similarity {
            a,
            b,
            tx: -(a * self.tx - b * self.ty),
            ty: -(b * self.tx + a * self.ty),
        }
    }

    /// The zoom.
    pub fn scale(&self) -> f64 {
        self.a.hypot(self.b)
    }

    /// The turn, radians.
    pub fn angle(&self) -> f64 {
        self.b.atan2(self.a)
    }

    /// Whether every number is one.
    pub fn is_finite(&self) -> bool {
        self.a.is_finite() && self.b.is_finite() && self.tx.is_finite() && self.ty.is_finite()
    }

    /// Whether this moves nothing, to within `epsilon`.
    pub fn is_identity(&self, epsilon: f64) -> bool {
        (self.a - 1.0).abs() <= epsilon
            && self.b.abs() <= epsilon
            && self.tx.abs() <= epsilon
            && self.ty.abs() <= epsilon
    }
}

/// One track: where a point was, and where it went.
pub type Pair = ((f64, f64), (f64, f64));

/// The least-squares similarity taking each pair's first point to its
/// second, or `None` with fewer than two distinct points.
pub fn least_squares(pairs: &[Pair]) -> Option<Similarity> {
    if pairs.len() < 2 {
        return None;
    }
    let n = pairs.len() as f64;
    let (mut px, mut py, mut qx, mut qy) = (0.0, 0.0, 0.0, 0.0);
    for &((x, y), (u, v)) in pairs {
        px += x;
        py += y;
        qx += u;
        qy += v;
    }
    let (px, py, qx, qy) = (px / n, py / n, qx / n, qy / n);
    let (mut dot, mut cross, mut norm) = (0.0, 0.0, 0.0);
    for &((x, y), (u, v)) in pairs {
        let (x, y, u, v) = (x - px, y - py, u - qx, v - qy);
        dot += x * u + y * v;
        cross += x * v - y * u;
        norm += x * x + y * y;
    }
    if norm <= 1e-18 {
        return None;
    }
    let a = dot / norm;
    let b = cross / norm;
    let similarity = Similarity {
        a,
        b,
        tx: qx - (a * px - b * py),
        ty: qy - (b * px + a * py),
    };
    similarity.is_finite().then_some(similarity)
}

/// The similarity two pairs fix exactly: `z = (q1 - q0) / (p1 - p0)`.
fn through(first: &Pair, second: &Pair) -> Option<Similarity> {
    let ((x0, y0), (u0, v0)) = *first;
    let ((x1, y1), (u1, v1)) = *second;
    let (dx, dy) = (x1 - x0, y1 - y0);
    let (du, dv) = (u1 - u0, v1 - v0);
    let norm = dx * dx + dy * dy;
    if norm <= 1e-12 {
        return None;
    }
    let a = (du * dx + dv * dy) / norm;
    let b = (dv * dx - du * dy) / norm;
    Some(Similarity {
        a,
        b,
        tx: u0 - (a * x0 - b * y0),
        ty: v0 - (b * x0 + a * y0),
    })
}

/// What a robust fit found.
#[derive(Clone, Debug, PartialEq)]
pub struct Robust {
    /// The similarity, fitted over the tracks that agree with it.
    pub similarity: Similarity,
    /// Which tracks agree, in the order given.
    pub inliers: Vec<bool>,
}

impl Robust {
    /// How many tracks agree.
    pub fn count(&self) -> usize {
        self.inliers.iter().filter(|&&inlier| inlier).count()
    }
}

/// A small, seeded generator: the same tracks always fit the same way,
/// so an analysis run twice caches the same answer.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// The similarity most tracks agree on, a track agreeing when it lands
/// within `threshold` of where the similarity sends it. `None` when no
/// pair of tracks fixes one that at least `min_inliers` agree with, or
/// the answer is no camera's: a zoom past a quarter either way between
/// two frames.
pub fn ransac(pairs: &[Pair], threshold: f64, min_inliers: usize) -> Option<Robust> {
    let n = pairs.len();
    if n < 2 || n < min_inliers {
        return None;
    }
    let squared = threshold * threshold;
    let agreeing = |similarity: &Similarity| -> Vec<bool> {
        pairs
            .iter()
            .map(|&(p, (u, v))| {
                let (x, y) = similarity.apply(p);
                (x - u) * (x - u) + (y - v) * (y - v) <= squared
            })
            .collect()
    };
    let mut random = XorShift(0x9e37_79b9_7f4a_7c15 ^ n as u64);
    let mut best: Option<(usize, Similarity)> = None;
    let mut wanted = 500usize;
    let mut tried = 0usize;
    while tried < wanted {
        tried += 1;
        let i = random.below(n);
        let mut j = random.below(n);
        if i == j {
            j = (j + 1) % n;
        }
        let Some(candidate) = through(&pairs[i], &pairs[j]) else {
            continue;
        };
        if !(0.75..=1.333).contains(&candidate.scale()) {
            continue;
        }
        let count = agreeing(&candidate)
            .iter()
            .filter(|&&inlier| inlier)
            .count();
        if best.is_none_or(|(most, _)| count > most) {
            best = Some((count, candidate));
            // Enough draws that a clean pair was very likely among them:
            // 99.9% sure, at the share of tracks this one says agree.
            let share = count as f64 / n as f64;
            let clean = share * share;
            if clean >= 1.0 {
                break;
            }
            if clean > 0.0 {
                let needed = ((1.0 - 0.999f64).ln() / (1.0 - clean).ln()).ceil();
                wanted = wanted.min((needed as usize).max(16));
            }
        }
    }
    let (_, mut similarity) = best?;
    // Fitted again over its own agreement, twice: the second pass picks up
    // the tracks the sharper answer now agrees with.
    let mut inliers = agreeing(&similarity);
    for _ in 0..2 {
        let kept: Vec<Pair> = pairs
            .iter()
            .zip(&inliers)
            .filter(|(_, inlier)| **inlier)
            .map(|(pair, _)| *pair)
            .collect();
        if let Some(refit) = least_squares(&kept) {
            similarity = refit;
        }
        inliers = agreeing(&similarity);
    }
    let robust = Robust {
        similarity,
        inliers,
    };
    (robust.count() >= min_inliers && (0.75..=1.333).contains(&similarity.scale()))
        .then_some(robust)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn a_similarity_composes_and_inverts() {
        let s = Similarity::new(0.1, 1.05, 0.02, -0.03);
        let t = Similarity::new(-0.3, 0.9, -0.1, 0.05);
        let p = (0.3, -0.2);
        let composed = s.then(&t).apply(p);
        let stepwise = t.apply(s.apply(p));
        assert!(close(composed.0, stepwise.0, 1e-12) && close(composed.1, stepwise.1, 1e-12));
        let back = s.inverse().apply(s.apply(p));
        assert!(close(back.0, p.0, 1e-12) && close(back.1, p.1, 1e-12));
        assert!(close(s.angle(), 0.1, 1e-12) && close(s.scale(), 1.05, 1e-12));
    }

    #[test]
    fn least_squares_recovers_an_exact_similarity() {
        let truth = Similarity::new(0.02, 1.01, 0.005, -0.004);
        let pairs: Vec<Pair> = (0..20)
            .map(|i| {
                let p = ((i % 5) as f64 * 0.2 - 0.4, (i / 5) as f64 * 0.2 - 0.3);
                (p, truth.apply(p))
            })
            .collect();
        let fitted = least_squares(&pairs).expect("fits");
        assert!(fitted.then(&truth.inverse()).is_identity(1e-9));
    }

    /// A subject walking across a third of the tracks moves them its own
    /// way; the fit follows the background's two thirds.
    #[test]
    fn ransac_rejects_a_moving_subject() {
        let camera = Similarity::new(-0.01, 1.0, 0.004, 0.002);
        let mut pairs: Vec<Pair> = Vec::new();
        for i in 0..60 {
            // A little noise, as real tracks have: a few thousandths of a
            // pixel at 480 lines is ~1e-5 of the height.
            let jitter = ((i * 37 % 11) as f64 - 5.0) * 2e-5;
            let p = (
                (i % 10) as f64 * 0.15 - 0.7 + jitter,
                (i / 10) as f64 * 0.15 - 0.4,
            );
            let (u, v) = camera.apply(p);
            pairs.push((p, (u + jitter, v - jitter)));
        }
        // The subject: a block of 30 tracks all shifted far to the right.
        for i in 0..30 {
            let p = (0.1 + (i % 6) as f64 * 0.02, 0.1 + (i / 6) as f64 * 0.02);
            let (u, v) = camera.apply(p);
            pairs.push((p, (u + 0.05, v + 0.01)));
        }
        let robust = ransac(&pairs, 2.0 / 480.0, 10).expect("fits");
        assert!(
            robust.similarity.then(&camera.inverse()).is_identity(2e-4),
            "{:?}",
            robust.similarity
        );
        assert!(robust.inliers[..60].iter().all(|&inlier| inlier));
        assert!(robust.inliers[60..].iter().all(|&inlier| !inlier));
        // And plain least squares, by contrast, is dragged off.
        let dragged = least_squares(&pairs).expect("fits");
        assert!(!dragged.then(&camera.inverse()).is_identity(2e-3));
    }

    #[test]
    fn too_few_agreeing_tracks_is_no_answer() {
        let pairs: Vec<Pair> = (0..8)
            .map(|i| {
                let p = (i as f64 * 0.1, (i * i) as f64 * 0.01);
                (p, (p.1 * 3.0, -p.0 * 2.0 + i as f64))
            })
            .collect();
        assert!(ransac(&pairs, 1e-3, 6).is_none());
    }
}

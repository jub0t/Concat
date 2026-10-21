// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The phone's lanes keep moving after the finger lets go.
//!
//! On a phone the lanes slide under a playhead held at their middle, and a
//! drag along them is a scrub. A scrub that stopped dead the instant the
//! finger left the glass read as sluggish next to every other list on the
//! phone, which carries on at the speed it was thrown and slows to a stop.
//! This is that: how fast the finger was going when it let go, and how the
//! lanes run down from there.
//!
//! Both halves are pure - time is a number of seconds from any origin the
//! caller likes - so they are tested here; the studio holds the timer that
//! drives a glide and the clock that stamps the samples.

use std::collections::VecDeque;
use std::time::Instant;

/// How far back a release looks for the finger's speed. A tenth of a
/// second is Android's and iOS's own window: long enough to smooth a
/// jittery sample or two, short enough that a drag which slowed before
/// letting go is not thrown at the speed it had earlier.
const HORIZON: f64 = 0.1;
/// A finger that has not moved for this long before it lifts was held
/// still, and lets go without throwing anything - however fast it went
/// before it stopped. Android's figure.
const STOPPED: f64 = 0.04;
/// The most samples kept; at a touchscreen's 120 Hz a tenth of a second
/// is a dozen, and an old one falls out the moment a new one comes in.
const KEPT: usize = 32;

/// The run-down: the speed falls by `e` every `TAU` seconds - 0.95 a
/// 16 ms frame, the friction of a phone's own lists - so a throw at
/// 3000 px/s travels about a thousand pixels before it stops.
pub const TAU: f64 = 0.312;
/// Slower than this, in pixels a second, a release is a lift, not a throw.
pub const MIN_THROW: f64 = 60.0;
/// The fastest throw honoured: a flick across a small screen can measure
/// absurd speeds from two samples a millisecond apart.
pub const MAX_THROW: f64 = 8000.0;
/// A glide slower than this has stopped: under a pixel every two frames.
pub const MIN_SPEED: f64 = 20.0;

/// The finger's recent positions, and its speed from them.
#[derive(Default)]
pub struct Tracker {
    /// (when, where): seconds, and the position along the lanes in
    /// whatever unit the caller tracks - the speed comes back in that
    /// unit per second.
    samples: VecDeque<(f64, f64)>,
}

impl Tracker {
    /// A new gesture: the last one's samples say nothing about this one.
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// The finger was at `position` at time `at`.
    pub fn push(&mut self, at: f64, position: f64) {
        if !(at.is_finite() && position.is_finite()) {
            return;
        }
        // Out of order is a clock gone wrong; start again from here.
        if self.samples.back().is_some_and(|&(last, _)| at < last) {
            self.samples.clear();
        }
        if self.samples.len() == KEPT {
            self.samples.pop_front();
        }
        self.samples.push_back((at, position));
    }

    /// The speed at time `now`, the moment of the release: the slope of
    /// the least-squares line through the samples of the last `HORIZON`,
    /// which a single late or early sample cannot swing the way the
    /// difference of the last two can. Zero when the finger had stopped
    /// before it lifted, or there is too little to tell by.
    pub fn velocity(&self, now: f64) -> f64 {
        let Some(&(newest, _)) = self.samples.back() else {
            return 0.0;
        };
        if now - newest > STOPPED {
            return 0.0;
        }
        let recent: Vec<(f64, f64)> = self
            .samples
            .iter()
            .copied()
            .filter(|&(at, _)| newest - at <= HORIZON)
            .collect();
        if recent.len() < 2 {
            return 0.0;
        }
        let n = recent.len() as f64;
        let mean_t = recent.iter().map(|&(at, _)| at).sum::<f64>() / n;
        let mean_x = recent.iter().map(|&(_, x)| x).sum::<f64>() / n;
        let (mut covariance, mut variance) = (0.0, 0.0);
        for &(at, x) in &recent {
            covariance += (at - mean_t) * (x - mean_x);
            variance += (at - mean_t) * (at - mean_t);
        }
        // All the samples at one instant: a speed cannot be told.
        if variance < 1e-9 {
            return 0.0;
        }
        covariance / variance
    }
}

/// One throw running down: where it has got to, by the clock.
///
/// Time-based rather than a fixed factor per tick, so a timer that fires
/// late - a busy frame, a phone throttling itself - moves the lanes as far
/// as they would have gone, and the glide takes the same time on a 60 Hz
/// screen as on a 120 Hz one.
#[derive(Clone, Copy, Debug)]
pub struct Glide {
    /// The speed it was thrown at, in pixels a second; the sign is the
    /// direction.
    speed: f64,
    /// Pixels travelled as of the last step.
    travelled: f64,
}

impl Glide {
    /// A throw at `speed` pixels a second, or None when that is too slow
    /// to be one. Clamped to `MAX_THROW`.
    pub fn thrown(speed: f64) -> Option<Self> {
        if !speed.is_finite() || speed.abs() < MIN_THROW {
            return None;
        }
        Some(Self {
            speed: speed.clamp(-MAX_THROW, MAX_THROW),
            travelled: 0.0,
        })
    }

    /// Pixels travelled `elapsed` seconds after the throw.
    pub fn distance(speed: f64, elapsed: f64) -> f64 {
        speed * TAU * (1.0 - (-elapsed.max(0.0) / TAU).exp())
    }

    /// The speed `elapsed` seconds after the throw.
    pub fn speed_at(speed: f64, elapsed: f64) -> f64 {
        speed * (-elapsed.max(0.0) / TAU).exp()
    }

    /// The pixels to move since the last step, `elapsed` seconds after the
    /// throw, and whether the glide is still going after them.
    pub fn step(&mut self, elapsed: f64) -> (f64, bool) {
        let to = Self::distance(self.speed, elapsed);
        let moved = to - self.travelled;
        self.travelled = to;
        (
            moved,
            Self::speed_at(self.speed, elapsed).abs() >= MIN_SPEED,
        )
    }
}

/// A throw as the studio keeps it: the samples of the drag in progress,
/// and the glide running, if one is. The clock lives here so the samples
/// and the glide read the same one.
pub struct Coast {
    pub tracker: Tracker,
    /// A finger is on the lanes, and the scrubs it makes are its samples.
    pub tracking: bool,
    /// The glide, when it was thrown, and the furthest it may go: the end
    /// of the cut, or where it started if that was already past it.
    pub glide: Option<(Glide, Instant, f32)>,
    epoch: Instant,
}

impl Default for Coast {
    fn default() -> Self {
        Self {
            tracker: Tracker::default(),
            tracking: false,
            glide: None,
            epoch: Instant::now(),
        }
    }
}

impl Coast {
    /// Seconds since the studio began: the samples' clock.
    pub fn clock(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_steady_drag_measures_its_speed() {
        let mut tracker = Tracker::default();
        // 1200 px/s, sampled every 8 ms for a fifth of a second.
        for step in 0..25 {
            let at = f64::from(step) * 0.008;
            tracker.push(at, 1200.0 * at);
        }
        let speed = tracker.velocity(0.192 + 0.005);
        assert!((speed - 1200.0).abs() < 1e-6, "{speed}");
    }

    #[test]
    fn only_the_last_tenth_of_a_second_counts() {
        let mut tracker = Tracker::default();
        // Fast, then slow for the last 120 ms: the release is slow.
        let mut x = 0.0;
        for step in 0..30 {
            let at = f64::from(step) * 0.01;
            x += if at < 0.18 { 30.0 } else { 2.0 };
            tracker.push(at, x);
        }
        let speed = tracker.velocity(0.29);
        assert!((speed - 200.0).abs() < 1.0, "{speed}");
    }

    #[test]
    fn a_finger_held_still_before_it_lifts_throws_nothing() {
        let mut tracker = Tracker::default();
        for step in 0..10 {
            let at = f64::from(step) * 0.01;
            tracker.push(at, 2000.0 * at);
        }
        assert!(tracker.velocity(0.09 + 0.01) > 1000.0);
        // The same drag, lifted 60 ms after its last move.
        assert_eq!(tracker.velocity(0.09 + 0.06), 0.0);
    }

    #[test]
    fn a_jittery_sample_does_not_swing_the_speed() {
        let mut tracker = Tracker::default();
        for step in 0..12 {
            let at = f64::from(step) * 0.008;
            // Every third sample a frame late in its stamp: the position is
            // right, the time is not.
            let stamp = if step % 3 == 2 { at + 0.004 } else { at };
            tracker.push(stamp, -900.0 * at);
        }
        let speed = tracker.velocity(0.09);
        assert!((speed + 900.0).abs() < 150.0, "{speed}");
    }

    #[test]
    fn too_little_to_tell_by_is_no_speed() {
        let mut tracker = Tracker::default();
        assert_eq!(tracker.velocity(0.0), 0.0);
        tracker.push(1.0, 10.0);
        assert_eq!(tracker.velocity(1.0), 0.0);
        tracker.push(1.0, 50.0);
        assert_eq!(tracker.velocity(1.0), 0.0, "two samples at one instant");
        tracker.push(f64::NAN, 3.0);
        tracker.clear();
        assert_eq!(tracker.velocity(1.0), 0.0);
        // A clock that runs backwards starts the samples again.
        tracker.push(2.0, 0.0);
        tracker.push(2.01, 10.0);
        tracker.push(1.0, 0.0);
        assert_eq!(tracker.velocity(1.0), 0.0);
    }

    #[test]
    fn a_throw_runs_down_and_stops() {
        let mut glide = Glide::thrown(3000.0).expect("a throw");
        // 0.95 a 16 ms frame.
        let ratio = Glide::speed_at(1.0, 0.016);
        assert!((ratio - 0.95).abs() < 0.002, "{ratio}");
        let mut total = 0.0;
        let mut frames = 0;
        loop {
            frames += 1;
            let (moved, going) = glide.step(f64::from(frames) * 0.016);
            assert!(moved > 0.0, "always forward");
            total += moved;
            if !going {
                break;
            }
            assert!(frames < 200, "a glide ends");
        }
        // It covers nearly all of speed × TAU, and in about a second and
        // a half: ln(3000 / 20) × TAU.
        assert!(
            total > 0.99 * 3000.0 * TAU && total <= 3000.0 * TAU,
            "{total}"
        );
        assert!((90..=100).contains(&frames), "{frames}");
    }

    #[test]
    fn a_late_timer_moves_as_far_as_a_punctual_one() {
        let mut punctual = Glide::thrown(-2000.0).expect("a throw");
        let mut late = Glide::thrown(-2000.0).expect("a throw");
        let mut on_time = 0.0;
        for frame in 1..=30 {
            on_time += punctual.step(f64::from(frame) * 0.016).0;
        }
        // The late one fires twice, the second time very late.
        let lagging = late.step(0.1).0 + late.step(0.48).0;
        assert!((on_time - lagging).abs() < 1e-9, "{on_time} {lagging}");
        assert!(on_time < 0.0, "backwards stays backwards");
    }

    #[test]
    fn a_slow_release_is_no_throw_and_a_wild_one_is_capped() {
        assert!(Glide::thrown(MIN_THROW / 2.0).is_none());
        assert!(Glide::thrown(-MIN_THROW / 2.0).is_none());
        assert!(Glide::thrown(f64::NAN).is_none());
        let mut wild = Glide::thrown(1e7).expect("a throw");
        let (moved, _) = wild.step(10.0);
        assert!(moved <= MAX_THROW * TAU + 1e-6, "{moved}");
    }
}

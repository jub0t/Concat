// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Placement that changes over a clip.
//!
//! A clip's transform and opacity are one value each. An *animation* is a
//! set of keys on top of them: for each of scale, the two offsets, rotation
//! and opacity, a list of `(at, value)` with `at` a fraction of the clip's
//! length, eased between neighbours. The values are relative to the clip's
//! own - a scale key is a factor, an offset key an addition, an opacity key
//! a factor - so the same animation means the same motion on any clip, and
//! a preset never has to know where the clip sits.
//!
//! This is the one place the per-frame arithmetic lives; the plan asks the
//! clip, and the clip asks here.

use crate::timeline::Transform;

/// How a key is approached from the one before it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Ease {
    /// Straight line.
    #[default]
    Linear,
    /// Starts slow, arrives fast.
    In,
    /// Starts fast, arrives slow.
    Out,
    /// Slow at both ends.
    InOut,
}

impl Ease {
    /// The eased fraction for a linear one.
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            Ease::In => t * t,
            Ease::Out => 1.0 - (1.0 - t) * (1.0 - t),
            Ease::InOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
        }
    }
}

/// One key of one property.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Key {
    /// Where in the clip, `0..=1`.
    pub at: f64,
    /// The value there, in the property's relative terms.
    pub value: f64,
    /// How this key is approached from the previous one.
    pub ease: Ease,
}

/// The keys of one property, sorted by `at`.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Track {
    keys: Vec<Key>,
}

impl Track {
    /// A track through these keys, sorted.
    pub fn new(mut keys: Vec<Key>) -> Track {
        keys.retain(|key| key.at.is_finite() && key.value.is_finite());
        for key in &mut keys {
            key.at = key.at.clamp(0.0, 1.0);
        }
        keys.sort_by(|a, b| a.at.total_cmp(&b.at));
        Track { keys }
    }

    /// The keys, sorted.
    pub fn keys(&self) -> &[Key] {
        &self.keys
    }

    /// True with no keys: the property is the clip's own.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The value at `x`: `rest` with no keys, the first key's before it,
    /// the last key's after it, and eased between neighbours.
    pub fn value_at(&self, x: f64, rest: f64) -> f64 {
        let Some(first) = self.keys.first() else {
            return rest;
        };
        if x <= first.at {
            return first.value;
        }
        for pair in self.keys.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if x <= b.at {
                if b.at <= a.at {
                    return b.value;
                }
                let t = b.ease.apply((x - a.at) / (b.at - a.at));
                return a.value + (b.value - a.value) * t;
            }
        }
        self.keys[self.keys.len() - 1].value
    }
}

/// Every animatable property's keys.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Animation {
    /// A factor on the clip's scale; 1 is as placed.
    pub scale: Track,
    /// Added to the clip's horizontal offset, in frame widths.
    pub offset_x: Track,
    /// Added to the clip's vertical offset, in frame heights.
    pub offset_y: Track,
    /// Added to the clip's rotation, in degrees.
    pub rotation: Track,
    /// A factor on the clip's opacity; 1 is as set.
    pub opacity: Track,
}

impl Animation {
    /// True when no property has a key.
    pub fn is_empty(&self) -> bool {
        self.scale.is_empty()
            && self.offset_x.is_empty()
            && self.offset_y.is_empty()
            && self.rotation.is_empty()
            && self.opacity.is_empty()
    }

    /// The clip's transform at `x`, a fraction of its length.
    pub fn transform_at(&self, base: Transform, x: f64) -> Transform {
        Transform {
            scale: (base.scale * self.scale.value_at(x, 1.0)).max(0.001),
            offset_x: base.offset_x + self.offset_x.value_at(x, 0.0),
            offset_y: base.offset_y + self.offset_y.value_at(x, 0.0),
            rotation: base.rotation + self.rotation.value_at(x, 0.0),
            stretch_x: base.stretch_x,
            stretch_y: base.stretch_y,
        }
    }

    /// The clip's opacity at `x`.
    pub fn opacity_at(&self, base: f32, x: f64) -> f32 {
        (f64::from(base) * self.opacity.value_at(x, 1.0)).clamp(0.0, 1.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(at: f64, value: f64, ease: Ease) -> Key {
        Key { at, value, ease }
    }

    #[test]
    fn a_track_holds_its_ends_and_eases_between() {
        let track = Track::new(vec![
            key(0.5, 1.0, Ease::Linear),
            key(0.0, 0.0, Ease::Linear),
        ]);
        assert_eq!(track.value_at(-1.0, 7.0), 0.0);
        assert_eq!(track.value_at(0.25, 7.0), 0.5);
        assert_eq!(track.value_at(0.9, 7.0), 1.0);
        assert_eq!(Track::default().value_at(0.5, 7.0), 7.0);

        let eased = Track::new(vec![key(0.0, 0.0, Ease::Linear), key(1.0, 1.0, Ease::Out)]);
        assert!(eased.value_at(0.5, 0.0) > 0.5);
        let eased = Track::new(vec![key(0.0, 0.0, Ease::Linear), key(1.0, 1.0, Ease::In)]);
        assert!(eased.value_at(0.5, 0.0) < 0.5);
    }

    #[test]
    fn an_animation_is_relative_to_the_clip() {
        let animation = Animation {
            scale: Track::new(vec![
                key(0.0, 0.5, Ease::Linear),
                key(1.0, 1.0, Ease::Linear),
            ]),
            offset_x: Track::new(vec![
                key(0.0, 0.5, Ease::Linear),
                key(1.0, 0.0, Ease::Linear),
            ]),
            opacity: Track::new(vec![
                key(0.0, 0.0, Ease::Linear),
                key(0.5, 1.0, Ease::Linear),
            ]),
            ..Animation::default()
        };
        let base = Transform {
            scale: 2.0,
            offset_x: 0.1,
            offset_y: -0.2,
            rotation: 10.0,
            stretch_x: 1.0,
            stretch_y: 1.0,
        };
        let mid = animation.transform_at(base, 0.5);
        assert!((mid.scale - 1.5).abs() < 1e-12);
        assert!((mid.offset_x - 0.35).abs() < 1e-12);
        assert_eq!(mid.offset_y, -0.2);
        assert_eq!(mid.rotation, 10.0);
        assert!((animation.opacity_at(0.8, 0.25) - 0.4).abs() < 1e-6);
        assert_eq!(animation.opacity_at(0.8, 0.9), 0.8);
        assert!(!animation.is_empty());
        assert!(Animation::default().is_empty());
    }
}

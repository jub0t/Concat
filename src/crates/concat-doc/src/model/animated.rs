// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A number that may change over an item's time.
//!
//! One type for every animatable number the document has: an item's
//! placement, opacity and level, each parameter of an effect, each
//! number of a mask. Its keys are at ticks of the item's content time,
//! so trimming, splitting and moving an item leave them where they are
//! in its picture.

use crate::time::Tick;

use super::ids::Raw;

/// A value, and the keys it rides on when it has any.
///
/// With no keys it is `value` throughout. With keys it is played from
/// them, and `value` is what it goes back to when they are taken off,
/// as the published API has it: clearing a property's keys returns it
/// to its constant.
///
/// `T` is what a key holds; `B` is what the value without keys is. They
/// are the same for an item's own numbers, which always have one. An
/// effect's parameter may have keys and no value of its own, falling
/// back to the package's default, so its `B` is an `Option`.
#[derive(Clone, PartialEq, Debug)]
pub struct Animated<T, B = T> {
    /// The value without keys.
    pub value: B,
    /// The keys, in order of `at`, no two at one tick.
    pub keys: Vec<Key<T>>,
}

impl<T, B> Animated<T, B> {
    /// One value throughout.
    pub const fn fixed(value: B) -> Self {
        Animated {
            value,
            keys: Vec::new(),
        }
    }

    /// Whether it has keys.
    pub fn is_keyed(&self) -> bool {
        !self.keys.is_empty()
    }
}

/// One key: the value at one tick of the item's content time, and how
/// it is reached from the key before.
#[derive(Clone, PartialEq, Debug)]
pub struct Key<T> {
    /// Where, on the item's content axis.
    pub at: Tick,
    /// The value there.
    pub value: T,
    /// How it is approached from the key before; on the first key it is
    /// kept and not read.
    pub ease: Ease,
}

impl<T> Key<T> {
    /// A key reached in a straight line.
    pub const fn linear(at: Tick, value: T) -> Self {
        Key {
            at,
            value,
            ease: Ease::LINEAR,
        }
    }
}

/// How a key is approached from the one before it.
#[derive(Clone, PartialEq, Debug)]
pub enum Ease {
    /// A cubic Bézier from (0, 0) to (1, 1) through these two control
    /// points, as CSS's timing functions are: time across, progress up.
    /// Each `x` is from 0 to 1, so that time only goes forward.
    Bezier {
        /// The first control point's time.
        x1: f64,
        /// The first control point's progress.
        y1: f64,
        /// The second control point's time.
        x2: f64,
        /// The second control point's progress.
        y2: f64,
    },
    /// An easing this build does not know.
    Unknown(Raw),
}

impl Ease {
    /// A straight line: (0, 0, 1, 1).
    pub const LINEAR: Ease = Ease::Bezier {
        x1: 0.0,
        y1: 0.0,
        x2: 1.0,
        y2: 1.0,
    };
}

impl Default for Ease {
    fn default() -> Ease {
        Ease::LINEAR
    }
}

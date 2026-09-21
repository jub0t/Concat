// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Animation presets: the named ways a clip comes in, goes out, or moves
//! for its whole length, as keys the engine can play.
//!
//! The document stores a name and a length per slot, not keys: a preset
//! re-materialises for the clip's current length every time it is asked
//! for, so trimming a clip keeps its half-second fade a half-second. The
//! shapes are relative to the clip's own placement (see the engine's
//! `animate` module), so a preset never has to know where the clip sits.

use concat_core::animate::{Animation, Ease, Key, Track};

use crate::model::{AnimationSlot, Clip, ClipAnimation};

/// One shape: what it is called, and its keys over the slot as `(property,
/// at-within-slot, value, ease)`.
struct Shape {
    name: &'static str,
    keys: &'static [(Prop, f64, f64, Ease)],
}

#[derive(Clone, Copy)]
enum Prop {
    Scale,
    X,
    Y,
    Rotation,
    Opacity,
}

use Prop::{Opacity, Rotation, Scale, X, Y};

/// The In shapes, in the order a menu lists them.
const IN: &[Shape] = &[
    Shape {
        name: "Fade",
        keys: &[
            (Opacity, 0.0, 0.0, Ease::LINEAR),
            (Opacity, 1.0, 1.0, Ease::OUT),
        ],
    },
    Shape {
        name: "Zoom In",
        keys: &[
            (Scale, 0.0, 0.5, Ease::LINEAR),
            (Scale, 1.0, 1.0, Ease::OUT),
            (Opacity, 0.0, 0.0, Ease::LINEAR),
            (Opacity, 1.0, 1.0, Ease::OUT),
        ],
    },
    Shape {
        name: "Zoom Out",
        keys: &[
            (Scale, 0.0, 1.6, Ease::LINEAR),
            (Scale, 1.0, 1.0, Ease::OUT),
            (Opacity, 0.0, 0.0, Ease::LINEAR),
            (Opacity, 1.0, 1.0, Ease::OUT),
        ],
    },
    Shape {
        name: "Slide Up",
        keys: &[(Y, 0.0, 0.6, Ease::LINEAR), (Y, 1.0, 0.0, Ease::OUT)],
    },
    Shape {
        name: "Slide Down",
        keys: &[(Y, 0.0, -0.6, Ease::LINEAR), (Y, 1.0, 0.0, Ease::OUT)],
    },
    Shape {
        name: "Slide Left",
        keys: &[(X, 0.0, 0.6, Ease::LINEAR), (X, 1.0, 0.0, Ease::OUT)],
    },
    Shape {
        name: "Slide Right",
        keys: &[(X, 0.0, -0.6, Ease::LINEAR), (X, 1.0, 0.0, Ease::OUT)],
    },
    Shape {
        name: "Spin",
        keys: &[
            (Rotation, 0.0, -180.0, Ease::LINEAR),
            (Rotation, 1.0, 0.0, Ease::OUT),
            (Scale, 0.0, 0.3, Ease::LINEAR),
            (Scale, 1.0, 1.0, Ease::OUT),
            (Opacity, 0.0, 0.0, Ease::LINEAR),
            (Opacity, 1.0, 1.0, Ease::OUT),
        ],
    },
    Shape {
        name: "Bounce In",
        keys: &[
            (Scale, 0.0, 0.3, Ease::LINEAR),
            (Scale, 0.6, 1.12, Ease::OUT),
            (Scale, 0.8, 0.96, Ease::IN_OUT),
            (Scale, 1.0, 1.0, Ease::OUT),
            (Opacity, 0.0, 0.0, Ease::LINEAR),
            (Opacity, 0.3, 1.0, Ease::OUT),
        ],
    },
    Shape {
        name: "Pop",
        keys: &[
            (Scale, 0.0, 0.0, Ease::LINEAR),
            (Scale, 0.15, 1.15, Ease::OUT),
            (Scale, 0.25, 1.0, Ease::IN_OUT),
            (Opacity, 0.0, 0.0, Ease::LINEAR),
            (Opacity, 0.15, 1.0, Ease::OUT),
        ],
    },
    Shape {
        name: "Roll In",
        keys: &[
            (Rotation, 0.0, -120.0, Ease::LINEAR),
            (Rotation, 1.0, 0.0, Ease::OUT),
            (X, 0.0, -0.7, Ease::LINEAR),
            (X, 1.0, 0.0, Ease::OUT),
            (Opacity, 0.0, 0.0, Ease::LINEAR),
            (Opacity, 0.3, 1.0, Ease::OUT),
        ],
    },
    Shape {
        name: "Drop In",
        keys: &[
            (Y, 0.0, -0.8, Ease::LINEAR),
            (Y, 0.7, 0.05, Ease::OUT),
            (Y, 1.0, 0.0, Ease::IN_OUT),
            (Opacity, 0.0, 0.0, Ease::LINEAR),
            (Opacity, 0.2, 1.0, Ease::OUT),
        ],
    },
    Shape {
        name: "Float Up",
        keys: &[
            (Y, 0.0, 0.25, Ease::LINEAR),
            (Y, 1.0, 0.0, Ease::OUT),
            (Opacity, 0.0, 0.0, Ease::LINEAR),
            (Opacity, 1.0, 1.0, Ease::OUT),
        ],
    },
    Shape {
        name: "Flicker In",
        keys: &[
            (Opacity, 0.0, 0.0, Ease::LINEAR),
            (Opacity, 0.15, 1.0, Ease::LINEAR),
            (Opacity, 0.25, 0.2, Ease::LINEAR),
            (Opacity, 0.4, 1.0, Ease::LINEAR),
            (Opacity, 0.5, 0.4, Ease::LINEAR),
            (Opacity, 0.65, 1.0, Ease::LINEAR),
            (Opacity, 1.0, 1.0, Ease::OUT),
        ],
    },
    Shape {
        name: "Elastic In",
        keys: &[
            (Scale, 0.0, 0.0, Ease::LINEAR),
            (Scale, 0.4, 1.2, Ease::OUT),
            (Scale, 0.6, 0.92, Ease::IN_OUT),
            (Scale, 0.8, 1.06, Ease::IN_OUT),
            (Scale, 1.0, 1.0, Ease::IN_OUT),
            (Opacity, 0.0, 0.0, Ease::LINEAR),
            (Opacity, 0.2, 1.0, Ease::OUT),
        ],
    },
    Shape {
        name: "Swing In",
        keys: &[
            (Rotation, 0.0, -25.0, Ease::LINEAR),
            (Rotation, 0.35, 12.0, Ease::IN_OUT),
            (Rotation, 0.6, -6.0, Ease::IN_OUT),
            (Rotation, 0.8, 3.0, Ease::IN_OUT),
            (Rotation, 1.0, 0.0, Ease::IN_OUT),
            (Opacity, 0.0, 0.0, Ease::LINEAR),
            (Opacity, 0.15, 1.0, Ease::OUT),
        ],
    },
];

/// The Out shapes: the In shapes run backwards, and eased in rather than
/// out, which is what leaving looks like.
const OUT: &[Shape] = &[
    Shape {
        name: "Fade",
        keys: &[
            (Opacity, 0.0, 1.0, Ease::LINEAR),
            (Opacity, 1.0, 0.0, Ease::IN),
        ],
    },
    Shape {
        name: "Zoom In",
        keys: &[
            (Scale, 0.0, 1.0, Ease::LINEAR),
            (Scale, 1.0, 1.6, Ease::IN),
            (Opacity, 0.0, 1.0, Ease::LINEAR),
            (Opacity, 1.0, 0.0, Ease::IN),
        ],
    },
    Shape {
        name: "Zoom Out",
        keys: &[
            (Scale, 0.0, 1.0, Ease::LINEAR),
            (Scale, 1.0, 0.5, Ease::IN),
            (Opacity, 0.0, 1.0, Ease::LINEAR),
            (Opacity, 1.0, 0.0, Ease::IN),
        ],
    },
    Shape {
        name: "Slide Up",
        keys: &[(Y, 0.0, 0.0, Ease::LINEAR), (Y, 1.0, -0.6, Ease::IN)],
    },
    Shape {
        name: "Slide Down",
        keys: &[(Y, 0.0, 0.0, Ease::LINEAR), (Y, 1.0, 0.6, Ease::IN)],
    },
    Shape {
        name: "Slide Left",
        keys: &[(X, 0.0, 0.0, Ease::LINEAR), (X, 1.0, -0.6, Ease::IN)],
    },
    Shape {
        name: "Slide Right",
        keys: &[(X, 0.0, 0.0, Ease::LINEAR), (X, 1.0, 0.6, Ease::IN)],
    },
    Shape {
        name: "Spin",
        keys: &[
            (Rotation, 0.0, 0.0, Ease::LINEAR),
            (Rotation, 1.0, 180.0, Ease::IN),
            (Scale, 0.0, 1.0, Ease::LINEAR),
            (Scale, 1.0, 0.3, Ease::IN),
            (Opacity, 0.0, 1.0, Ease::LINEAR),
            (Opacity, 1.0, 0.0, Ease::IN),
        ],
    },
    Shape {
        name: "Bounce In",
        keys: &[
            (Scale, 0.0, 1.0, Ease::LINEAR),
            (Scale, 0.2, 1.12, Ease::IN_OUT),
            (Scale, 1.0, 0.3, Ease::IN),
            (Opacity, 0.0, 1.0, Ease::LINEAR),
            (Opacity, 0.7, 1.0, Ease::LINEAR),
            (Opacity, 1.0, 0.0, Ease::IN),
        ],
    },
    Shape {
        name: "Pop",
        keys: &[
            (Scale, 0.0, 1.0, Ease::LINEAR),
            (Scale, 0.85, 1.0, Ease::LINEAR),
            (Scale, 1.0, 0.0, Ease::IN),
            (Opacity, 0.0, 1.0, Ease::LINEAR),
            (Opacity, 0.85, 1.0, Ease::LINEAR),
            (Opacity, 1.0, 0.0, Ease::IN),
        ],
    },
    Shape {
        name: "Roll In",
        keys: &[
            (Rotation, 0.0, 0.0, Ease::LINEAR),
            (Rotation, 1.0, 120.0, Ease::IN),
            (X, 0.0, 0.0, Ease::LINEAR),
            (X, 1.0, 0.7, Ease::IN),
            (Opacity, 0.0, 1.0, Ease::LINEAR),
            (Opacity, 0.7, 1.0, Ease::LINEAR),
            (Opacity, 1.0, 0.0, Ease::IN),
        ],
    },
    Shape {
        name: "Drop In",
        keys: &[
            (Y, 0.0, 0.0, Ease::LINEAR),
            (Y, 1.0, 0.8, Ease::IN),
            (Opacity, 0.0, 1.0, Ease::LINEAR),
            (Opacity, 0.6, 1.0, Ease::LINEAR),
            (Opacity, 1.0, 0.0, Ease::IN),
        ],
    },
    Shape {
        name: "Float Up",
        keys: &[
            (Y, 0.0, 0.0, Ease::LINEAR),
            (Y, 1.0, -0.25, Ease::IN),
            (Opacity, 0.0, 1.0, Ease::LINEAR),
            (Opacity, 1.0, 0.0, Ease::IN),
        ],
    },
    Shape {
        name: "Flicker Out",
        keys: &[
            (Opacity, 0.0, 1.0, Ease::LINEAR),
            (Opacity, 0.35, 1.0, Ease::LINEAR),
            (Opacity, 0.5, 0.4, Ease::LINEAR),
            (Opacity, 0.6, 1.0, Ease::LINEAR),
            (Opacity, 0.75, 0.2, Ease::LINEAR),
            (Opacity, 0.85, 1.0, Ease::LINEAR),
            (Opacity, 1.0, 0.0, Ease::IN),
        ],
    },
    Shape {
        name: "Elastic Out",
        keys: &[
            (Scale, 0.0, 1.0, Ease::LINEAR),
            (Scale, 0.2, 1.06, Ease::IN_OUT),
            (Scale, 0.4, 0.92, Ease::IN_OUT),
            (Scale, 0.6, 1.2, Ease::IN),
            (Scale, 1.0, 0.0, Ease::IN),
            (Opacity, 0.0, 1.0, Ease::LINEAR),
            (Opacity, 0.8, 1.0, Ease::LINEAR),
            (Opacity, 1.0, 0.0, Ease::IN),
        ],
    },
    Shape {
        name: "Swing Out",
        keys: &[
            (Rotation, 0.0, 0.0, Ease::LINEAR),
            (Rotation, 0.2, -3.0, Ease::IN_OUT),
            (Rotation, 0.4, 6.0, Ease::IN_OUT),
            (Rotation, 0.65, -12.0, Ease::IN_OUT),
            (Rotation, 1.0, 25.0, Ease::IN),
            (Opacity, 0.0, 1.0, Ease::LINEAR),
            (Opacity, 0.85, 1.0, Ease::LINEAR),
            (Opacity, 1.0, 0.0, Ease::IN),
        ],
    },
];

/// The Combo shapes: over the whole clip.
const COMBO: &[Shape] = &[
    Shape {
        name: "Pulse",
        keys: &[
            (Scale, 0.0, 1.0, Ease::LINEAR),
            (Scale, 0.5, 1.08, Ease::IN_OUT),
            (Scale, 1.0, 1.0, Ease::IN_OUT),
        ],
    },
    Shape {
        name: "Shake",
        keys: &[
            (X, 0.0, 0.0, Ease::LINEAR),
            (X, 0.1, 0.02, Ease::LINEAR),
            (X, 0.2, -0.02, Ease::LINEAR),
            (X, 0.3, 0.02, Ease::LINEAR),
            (X, 0.4, -0.02, Ease::LINEAR),
            (X, 0.5, 0.02, Ease::LINEAR),
            (X, 0.6, -0.02, Ease::LINEAR),
            (X, 0.7, 0.02, Ease::LINEAR),
            (X, 0.8, -0.02, Ease::LINEAR),
            (X, 0.9, 0.02, Ease::LINEAR),
            (X, 1.0, 0.0, Ease::LINEAR),
        ],
    },
    Shape {
        name: "Spin",
        keys: &[
            (Rotation, 0.0, 0.0, Ease::LINEAR),
            (Rotation, 1.0, 360.0, Ease::LINEAR),
        ],
    },
    Shape {
        name: "Bounce",
        keys: &[
            (Y, 0.0, 0.0, Ease::LINEAR),
            (Y, 0.25, -0.06, Ease::OUT),
            (Y, 0.5, 0.0, Ease::IN),
            (Y, 0.75, -0.03, Ease::OUT),
            (Y, 1.0, 0.0, Ease::IN),
        ],
    },
    Shape {
        name: "Drift",
        keys: &[
            (Scale, 0.0, 1.0, Ease::LINEAR),
            (Scale, 1.0, 1.12, Ease::LINEAR),
        ],
    },
    Shape {
        name: "Sway",
        keys: &[
            (X, 0.0, 0.0, Ease::IN_OUT),
            (X, 0.25, 0.08, Ease::IN_OUT),
            (X, 0.5, 0.0, Ease::IN_OUT),
            (X, 0.75, -0.08, Ease::IN_OUT),
            (X, 1.0, 0.0, Ease::IN_OUT),
        ],
    },
    Shape {
        name: "Wave",
        keys: &[
            (Y, 0.0, 0.0, Ease::IN_OUT),
            (Y, 0.25, -0.05, Ease::IN_OUT),
            (Y, 0.5, 0.0, Ease::IN_OUT),
            (Y, 0.75, 0.05, Ease::IN_OUT),
            (Y, 1.0, 0.0, Ease::IN_OUT),
        ],
    },
    Shape {
        name: "Heartbeat",
        keys: &[
            (Scale, 0.0, 1.0, Ease::LINEAR),
            (Scale, 0.15, 1.12, Ease::OUT),
            (Scale, 0.3, 1.0, Ease::IN),
            (Scale, 0.45, 1.08, Ease::OUT),
            (Scale, 0.6, 1.0, Ease::IN),
            (Scale, 1.0, 1.0, Ease::LINEAR),
        ],
    },
    Shape {
        name: "Breathe",
        keys: &[
            (Scale, 0.0, 1.0, Ease::IN_OUT),
            (Scale, 0.5, 1.05, Ease::IN_OUT),
            (Scale, 1.0, 1.0, Ease::IN_OUT),
            (Opacity, 0.0, 1.0, Ease::IN_OUT),
            (Opacity, 0.5, 0.85, Ease::IN_OUT),
            (Opacity, 1.0, 1.0, Ease::IN_OUT),
        ],
    },
    Shape {
        name: "Wobble",
        keys: &[
            (Rotation, 0.0, 0.0, Ease::IN_OUT),
            (Rotation, 0.25, 8.0, Ease::IN_OUT),
            (Rotation, 0.5, 0.0, Ease::IN_OUT),
            (Rotation, 0.75, -8.0, Ease::IN_OUT),
            (Rotation, 1.0, 0.0, Ease::IN_OUT),
        ],
    },
    Shape {
        name: "Jelly",
        keys: &[
            (Scale, 0.0, 1.0, Ease::IN_OUT),
            (Scale, 0.125, 1.06, Ease::IN_OUT),
            (Scale, 0.25, 0.97, Ease::IN_OUT),
            (Scale, 0.375, 1.03, Ease::IN_OUT),
            (Scale, 0.5, 0.99, Ease::IN_OUT),
            (Scale, 0.625, 1.02, Ease::IN_OUT),
            (Scale, 0.75, 0.995, Ease::IN_OUT),
            (Scale, 1.0, 1.0, Ease::IN_OUT),
        ],
    },
    Shape {
        name: "Flutter",
        keys: &[
            (Rotation, 0.0, 0.0, Ease::LINEAR),
            (Rotation, 0.125, 3.0, Ease::LINEAR),
            (Rotation, 0.25, -3.0, Ease::LINEAR),
            (Rotation, 0.375, 3.0, Ease::LINEAR),
            (Rotation, 0.5, -3.0, Ease::LINEAR),
            (Rotation, 0.625, 3.0, Ease::LINEAR),
            (Rotation, 0.75, -3.0, Ease::LINEAR),
            (Rotation, 0.875, 3.0, Ease::LINEAR),
            (Rotation, 1.0, 0.0, Ease::LINEAR),
            (Scale, 0.0, 1.0, Ease::LINEAR),
            (Scale, 0.5, 0.97, Ease::LINEAR),
            (Scale, 1.0, 1.0, Ease::LINEAR),
        ],
    },
];

/// The Loop shapes: repeating motions over the clip.
const LOOP: &[Shape] = &[
    Shape {
        name: "Spin",
        keys: &[
            (Rotation, 0.0, 0.0, Ease::LINEAR),
            (Rotation, 1.0, 360.0, Ease::LINEAR),
        ],
    },
    Shape {
        name: "Pulse",
        keys: &[
            (Scale, 0.0, 1.0, Ease::IN_OUT),
            (Scale, 0.5, 1.15, Ease::IN_OUT),
            (Scale, 1.0, 1.0, Ease::IN_OUT),
        ],
    },
    Shape {
        name: "Breathe",
        keys: &[
            (Scale, 0.0, 1.0, Ease::IN_OUT),
            (Scale, 0.5, 1.06, Ease::IN_OUT),
            (Scale, 1.0, 1.0, Ease::IN_OUT),
            (Opacity, 0.0, 1.0, Ease::IN_OUT),
            (Opacity, 0.5, 0.85, Ease::IN_OUT),
            (Opacity, 1.0, 1.0, Ease::IN_OUT),
        ],
    },
    Shape {
        name: "Wobble",
        keys: &[
            (Rotation, 0.0, 0.0, Ease::IN_OUT),
            (Rotation, 0.25, 8.0, Ease::IN_OUT),
            (Rotation, 0.5, 0.0, Ease::IN_OUT),
            (Rotation, 0.75, -8.0, Ease::IN_OUT),
            (Rotation, 1.0, 0.0, Ease::IN_OUT),
        ],
    },
    Shape {
        name: "Bounce",
        keys: &[
            (Y, 0.0, 0.0, Ease::OUT),
            (Y, 0.4, -0.15, Ease::IN),
            (Y, 0.5, 0.0, Ease::OUT),
            (Y, 0.75, -0.06, Ease::IN),
            (Y, 1.0, 0.0, Ease::OUT),
        ],
    },
    Shape {
        name: "Jitter",
        keys: &[
            (X, 0.0, 0.0, Ease::LINEAR),
            (X, 0.2, 0.03, Ease::LINEAR),
            (X, 0.4, -0.03, Ease::LINEAR),
            (X, 0.6, 0.02, Ease::LINEAR),
            (X, 0.8, -0.02, Ease::LINEAR),
            (X, 1.0, 0.0, Ease::LINEAR),
            (Y, 0.0, 0.0, Ease::LINEAR),
            (Y, 0.25, -0.03, Ease::LINEAR),
            (Y, 0.5, 0.03, Ease::LINEAR),
            (Y, 0.75, -0.02, Ease::LINEAR),
            (Y, 1.0, 0.0, Ease::LINEAR),
        ],
    },
    Shape {
        name: "Float",
        keys: &[
            (Y, 0.0, 0.0, Ease::IN_OUT),
            (Y, 0.5, -0.08, Ease::IN_OUT),
            (Y, 1.0, 0.0, Ease::IN_OUT),
        ],
    },
    Shape {
        name: "Flicker",
        keys: &[
            (Opacity, 0.0, 1.0, Ease::LINEAR),
            (Opacity, 0.1, 0.3, Ease::LINEAR),
            (Opacity, 0.18, 1.0, Ease::LINEAR),
            (Opacity, 0.3, 1.0, Ease::LINEAR),
            (Opacity, 0.38, 0.4, Ease::LINEAR),
            (Opacity, 0.5, 1.0, Ease::LINEAR),
            (Opacity, 0.7, 1.0, Ease::LINEAR),
            (Opacity, 0.78, 0.2, Ease::LINEAR),
            (Opacity, 0.86, 1.0, Ease::LINEAR),
            (Opacity, 1.0, 1.0, Ease::LINEAR),
        ],
    },
];

fn shapes(slot: AnimationSlot) -> &'static [Shape] {
    match slot {
        AnimationSlot::In => IN,
        AnimationSlot::Out => OUT,
        AnimationSlot::Combo => COMBO,
        AnimationSlot::Loop => LOOP,
    }
}

/// The names a slot offers, in menu order.
pub fn names(slot: AnimationSlot) -> Vec<&'static str> {
    shapes(slot).iter().map(|shape| shape.name).collect()
}

/// Where a name sits in its slot's menu, or None for a name the slot does
/// not know.
pub fn index_of(slot: AnimationSlot, name: &str) -> Option<usize> {
    shapes(slot).iter().position(|shape| shape.name == name)
}

/// The engine's keys for everything set on `clip`, over its current
/// length, or None when nothing is set. In and Out keys are placed over
/// their slot's seconds at the head and tail; a Combo runs the whole clip.
/// A slot longer than the clip is squeezed to it, and a head and tail that
/// would overlap share the clip in half.
pub fn animation_of(clip: &Clip) -> Option<Animation> {
    let duration = clip.duration.max(1e-6);
    let mut tracks: [Vec<Key>; 5] = Default::default();
    let mut any = false;

    let mut lay =
        |slot: AnimationSlot, set: &Option<ClipAnimation>, other: &Option<ClipAnimation>| {
            let Some(set) = set else { return };
            let Some(shape) = shapes(slot).iter().find(|shape| shape.name == set.preset) else {
                return;
            };
            if slot == AnimationSlot::Loop {
                let cycle_secs = set.duration.max(0.2);
                let cycles = (duration / cycle_secs).ceil() as usize;
                let cycle_frac = cycle_secs / duration;
                for c in 0..cycles {
                    let c_start = c as f64 * cycle_frac;
                    if c_start >= 1.0 {
                        break;
                    }
                    for &(prop, at, value, ease) in shape.keys {
                        let key_at = c_start + cycle_frac * at;
                        if key_at <= 1.0 + 1e-5 {
                            let key = Key {
                                at: key_at.min(1.0),
                                value,
                                ease,
                            };
                            tracks[match prop {
                                Scale => 0,
                                X => 1,
                                Y => 2,
                                Rotation => 3,
                                Opacity => 4,
                            }]
                            .push(key);
                            any = true;
                        }
                    }
                }
                return;
            }
            // The slot's share of the clip, as a fraction of its length.
            let mut span = (set.duration.max(0.05) / duration).min(1.0);
            if let Some(other) = other {
                let both = span + (other.duration.max(0.05) / duration).min(1.0);
                if both > 1.0 {
                    span /= both;
                }
            }
            let (from, to) = match slot {
                AnimationSlot::In => (0.0, span),
                AnimationSlot::Out => (1.0 - span, 1.0),
                AnimationSlot::Combo | AnimationSlot::Loop => (0.0, 1.0),
            };
            for &(prop, at, value, ease) in shape.keys {
                let key = Key {
                    at: from + (to - from) * at,
                    value,
                    ease,
                };
                tracks[match prop {
                    Scale => 0,
                    X => 1,
                    Y => 2,
                    Rotation => 3,
                    Opacity => 4,
                }]
                .push(key);
                any = true;
            }
        };
    lay(AnimationSlot::In, &clip.animation_in, &clip.animation_out);
    lay(AnimationSlot::Out, &clip.animation_out, &clip.animation_in);
    lay(AnimationSlot::Combo, &clip.animation_combo, &None);
    lay(AnimationSlot::Loop, &clip.animation_loop, &None);
    if !any {
        return None;
    }
    let [scale, x, y, rotation, opacity] = tracks;
    Some(Animation {
        scale: Track::new(scale),
        offset_x: Track::new(x),
        offset_y: Track::new(y),
        rotation: Track::new(rotation),
        opacity: Track::new(opacity),
        // Presets are picture only: no shape in the catalogue touches gain,
        // and a preset that silently rode the fader would be a surprise. A
        // keyed volume comes from the clip's own keys instead - see
        // `concat_export::flatten::export_keys`.
        volume: Track::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, Editor};

    #[test]
    fn every_slot_names_its_shapes_and_finds_them_again() {
        for slot in [
            AnimationSlot::In,
            AnimationSlot::Out,
            AnimationSlot::Combo,
            AnimationSlot::Loop,
        ] {
            let names = names(slot);
            assert!(!names.is_empty());
            for (index, name) in names.iter().enumerate() {
                assert_eq!(index_of(slot, name), Some(index));
            }
            assert_eq!(index_of(slot, "Nothing"), None);
        }
    }

    #[test]
    fn a_loop_animation_repeats_across_the_clip() {
        let mut editor = Editor::new();
        let id = editor
            .apply(Command::AddTextClip {
                above: false,
                track_id: None,
                start: 0.0,
                style: None,
                duration: Some(4.0),
                offset_y: None,
            })
            .unwrap()
            .created_id
            .unwrap();
        editor
            .apply(Command::SetClipAnimation {
                clip_id: id.clone(),
                slot: AnimationSlot::Loop,
                animation: Some(ClipAnimation {
                    preset: "Pulse".to_owned(),
                    duration: 1.0, // 1 second per cycle -> 4 cycles over 4 seconds
                }),
            })
            .unwrap();
        let clip = editor.project().active().clip(&id).unwrap();
        let animation = animation_of(clip).expect("a pulse loop is set");
        // Peak of pulse at half of each cycle: 0.5s / 4s = 0.125, 1.5s / 4s = 0.375
        let scale_0 = animation.transform_at(Default::default(), 0.0).scale;
        let scale_peak1 = animation.transform_at(Default::default(), 0.125).scale;
        let scale_mid = animation.transform_at(Default::default(), 0.25).scale;
        let scale_peak2 = animation.transform_at(Default::default(), 0.375).scale;
        assert_eq!(scale_0, 1.0);
        assert!((scale_peak1 - 1.15).abs() < 1e-3);
        assert_eq!(scale_mid, 1.0);
        assert!((scale_peak2 - 1.15).abs() < 1e-3);
    }

    #[test]
    fn a_fade_in_lands_over_its_seconds_at_the_head() {
        let mut editor = Editor::new();
        let id = editor
            .apply(Command::AddTextClip {
                above: false,
                track_id: None,
                start: 0.0,
                style: None,
                duration: Some(4.0),
                offset_y: None,
            })
            .unwrap()
            .created_id
            .unwrap();
        editor
            .apply(Command::SetClipAnimation {
                clip_id: id.clone(),
                slot: AnimationSlot::In,
                animation: Some(ClipAnimation {
                    preset: "Fade".to_owned(),
                    duration: 1.0,
                }),
            })
            .unwrap();
        let clip = editor.project().active().clip(&id).unwrap();
        let animation = animation_of(clip).expect("a fade is set");
        // A second of a four-second clip: the fade is done by a quarter.
        assert!((animation.opacity_at(1.0, 0.0) - 0.0).abs() < 1e-9);
        assert!(animation.opacity_at(1.0, 0.125) > 0.0 && animation.opacity_at(1.0, 0.125) < 1.0);
        assert!((animation.opacity_at(1.0, 0.25) - 1.0).abs() < 1e-9);
        assert!((animation.opacity_at(1.0, 0.9) - 1.0).abs() < 1e-9);
        assert_eq!(animation.transform_at(Default::default(), 0.1).scale, 1.0);
    }

    /// Elastic In starts a clip at zero scale, overshoots past 1.0, and
    /// settles - the spring shape a "Bounce In" alone does not have.
    #[test]
    fn an_elastic_in_overshoots_before_settling() {
        let mut editor = Editor::new();
        let id = editor
            .apply(Command::AddTextClip {
                above: false,
                track_id: None,
                start: 0.0,
                style: None,
                duration: Some(4.0),
                offset_y: None,
            })
            .unwrap()
            .created_id
            .unwrap();
        editor
            .apply(Command::SetClipAnimation {
                clip_id: id.clone(),
                slot: AnimationSlot::In,
                animation: Some(ClipAnimation {
                    preset: "Elastic In".to_owned(),
                    duration: 1.0,
                }),
            })
            .unwrap();
        let clip = editor.project().active().clip(&id).unwrap();
        let animation = animation_of(clip).expect("an elastic-in is set");
        // The In slot is 1s of a 4s clip: a quarter of the whole clip.
        assert!(animation.transform_at(Default::default(), 0.0).scale < 0.01);
        // The overshoot key sits at 40% of the slot, 10% of the clip.
        assert!(animation.transform_at(Default::default(), 0.1).scale > 1.1);
        // Settled by the slot's end, and held there for the rest of the clip.
        assert!((animation.transform_at(Default::default(), 0.25).scale - 1.0).abs() < 1e-9);
        assert!((animation.transform_at(Default::default(), 0.9).scale - 1.0).abs() < 1e-9);
    }
}

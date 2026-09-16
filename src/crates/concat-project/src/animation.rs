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
];

fn shapes(slot: AnimationSlot) -> &'static [Shape] {
    match slot {
        AnimationSlot::In => IN,
        AnimationSlot::Out => OUT,
        AnimationSlot::Combo => COMBO,
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
                AnimationSlot::Combo => (0.0, 1.0),
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
        for slot in [AnimationSlot::In, AnimationSlot::Out, AnimationSlot::Combo] {
            let names = names(slot);
            assert!(!names.is_empty());
            for (index, name) in names.iter().enumerate() {
                assert_eq!(index_of(slot, name), Some(index));
            }
            assert_eq!(index_of(slot, "Nothing"), None);
        }
    }

    #[test]
    fn a_fade_in_lands_over_its_seconds_at_the_head() {
        let mut editor = Editor::new();
        let id = editor
            .apply(Command::AddTextClip {
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
}

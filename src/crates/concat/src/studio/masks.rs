// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The selected clip's masks: which one the inspector and the monitor work
//! on, and what the Mask section's controls do to them.
//!
//! A clip carries a list of masks, top first. One of them is current: the
//! one picked in the section's list, or the first when none is or the one
//! picked is gone. Its numbers are the section's knobs, its outline and
//! grips the monitor's, and its keys the keyframe cluster's mask rows. The
//! pick is the studio's, not the document's - it is where the person is
//! looking, not a fact about the edit - so undo leaves it, and a pick of a
//! mask the clip no longer has falls back to the first.

use super::*;
use concat_project::model::{Mask, MaskMode, MaskShape};

/// The number of a mask a field writes, by the name the mask keys it
/// under; None for a field that is not one of a mask's numbers.
pub(super) fn mask_number_of(field: ClipField) -> Option<&'static str> {
    Some(match field {
        ClipField::MaskX => "x",
        ClipField::MaskY => "y",
        ClipField::MaskWidth => "width",
        ClipField::MaskHeight => "height",
        ClipField::MaskRotation => "rotation",
        ClipField::MaskRoundness => "roundness",
        ClipField::MaskFeather => "feather",
        ClipField::MaskExpand => "expand",
        ClipField::MaskOpacity => "opacity",
        _ => return None,
    })
}

/// The mask fields, in the order the keyframe cluster publishes their
/// rows after the clip's own six.
pub(super) const MASK_KEY_FIELDS: [ClipField; 9] = [
    ClipField::MaskX,
    ClipField::MaskY,
    ClipField::MaskWidth,
    ClipField::MaskHeight,
    ClipField::MaskRotation,
    ClipField::MaskRoundness,
    ClipField::MaskFeather,
    ClipField::MaskExpand,
    ClipField::MaskOpacity,
];

/// How a mask's number reads on a keyframe row: its label, its range, the
/// knob's step and default, its format, its unit and the factor the unit
/// is shown at. The inspector's knobs have the same.
pub(super) struct MaskKnob {
    pub label: String,
    pub minimum: f64,
    pub maximum: f64,
    pub step: f32,
    pub default_value: f32,
    pub fmt: ParamFormat,
    pub unit: &'static str,
    pub unit_scale: f32,
}

/// See [`MaskKnob`]. None for a name that is not a mask's number.
pub(super) fn mask_knob(number: &str) -> Option<MaskKnob> {
    let knob = |label: String,
                (minimum, maximum): (f64, f64),
                step,
                default_value,
                fmt,
                unit,
                unit_scale| MaskKnob {
        label,
        minimum,
        maximum,
        step,
        default_value,
        fmt,
        unit,
        unit_scale,
    };
    Some(match number {
        "x" => knob(
            t("common.positionX"),
            (-1.0, 1.0),
            0.005,
            0.0,
            ParamFormat::Centred,
            "%",
            100.0,
        ),
        "y" => knob(
            t("common.positionY"),
            (-1.0, 1.0),
            0.005,
            0.0,
            ParamFormat::Centred,
            "%",
            100.0,
        ),
        "width" => knob(
            t("common.width"),
            (0.01, 4.0),
            0.01,
            0.66,
            ParamFormat::Fraction,
            "%",
            100.0,
        ),
        "height" => knob(
            t("common.height"),
            (0.01, 4.0),
            0.01,
            0.66,
            ParamFormat::Fraction,
            "%",
            100.0,
        ),
        "rotation" => knob(
            t("common.rotation"),
            (-180.0, 180.0),
            1.0,
            0.0,
            ParamFormat::Degrees,
            "°",
            1.0,
        ),
        "roundness" => knob(
            t("mask.roundness"),
            (0.0, 1.0),
            0.01,
            0.0,
            ParamFormat::Fraction,
            "%",
            100.0,
        ),
        "feather" => knob(
            t("clipInspector.feather"),
            (0.0, 1.0),
            0.01,
            0.0,
            ParamFormat::Fraction,
            "%",
            100.0,
        ),
        "expand" => knob(
            t("mask.expand"),
            (-0.5, 0.5),
            0.005,
            0.0,
            ParamFormat::Centred,
            "%",
            100.0,
        ),
        "opacity" => knob(
            t("common.opacity"),
            (0.0, 1.0),
            0.01,
            1.0,
            ParamFormat::Fraction,
            "%",
            100.0,
        ),
        _ => return None,
    })
}

/// The name the panel gives a mask: its own, or "Mask N" by its place.
pub(super) fn mask_label(mask: &Mask, index: usize) -> String {
    if mask.name.is_empty() {
        tf("mask.numbered", &[&(index + 1).to_string()])
    } else {
        mask.name.clone()
    }
}

/// Whether a field is the Mask section's.
fn is_mask_field(field: ClipField) -> bool {
    mask_number_of(field).is_some()
        || matches!(
            field,
            ClipField::MaskShape
                | ClipField::MaskInvert
                | ClipField::MaskMode
                | ClipField::MaskAdd
                | ClipField::MaskSelect
                | ClipField::MaskRemove
                | ClipField::MaskMove
        )
}

/// The place of the current mask in `masks`: the one `focus` names, or the
/// first; None when there are none.
pub(super) fn current_index(masks: &[Mask], focus: Option<&str>) -> Option<usize> {
    focus
        .and_then(|focus| masks.iter().position(|mask| mask.id == focus))
        .or((!masks.is_empty()).then_some(0))
}

impl Studio {
    /// The clip's current mask; see the module.
    pub(super) fn current_mask<'a>(&self, clip: &'a Clip) -> Option<&'a Mask> {
        current_index(&clip.masks, self.mask_focus.as_deref()).map(|index| &clip.masks[index])
    }

    /// A Mask section field, on the echo of every clip the inspector writes
    /// to, each one's own current mask. True when `field` is the section's
    /// - handled, or nothing to handle - so the general setter leaves it.
    pub(super) fn mask_set(&mut self, field: ClipField, value: f64) -> bool {
        if !is_mask_field(field) {
            return false;
        }
        let ids = self.edit_targets();
        if ids.is_empty() {
            return true;
        }
        self.commit_targets = ids.clone();
        self.begin_echo();
        let focus = self.mask_focus.clone();
        let place = (value >= 0.0).then_some(value.round() as usize);
        // What the current mask becomes once the edit lands: Some(None) for
        // none, None to leave the pick alone.
        let mut refocus: Option<Option<String>> = None;
        for id in ids {
            let at = self
                .clip(&id)
                .map(|clip| place_in(clip, self.playhead))
                .unwrap_or(0.0);
            let Some(clip) = self.echo_clip_mut(&id) else {
                continue;
            };
            if !clip.kind.is_visual() {
                continue;
            }
            let current = current_index(&clip.masks, focus.as_deref());
            let shape = place.and_then(|place| MaskShape::ALL.get(place).copied());
            match field {
                ClipField::MaskAdd => {
                    let Some(shape) = shape else { continue };
                    let id = Mask::next_id(&clip.masks);
                    clip.masks.push(Mask::of(id.clone(), "", shape));
                    refocus = Some(Some(id));
                }
                ClipField::MaskSelect => {
                    if let Some(mask) = place.and_then(|place| clip.masks.get(place)) {
                        refocus = Some(Some(mask.id.clone()));
                    }
                }
                ClipField::MaskRemove => {
                    let Some(index) = current else { continue };
                    clip.masks.remove(index);
                    // The one below takes its place, or the one above when
                    // it was the last.
                    let next = index.min(clip.masks.len().saturating_sub(1));
                    refocus = Some(clip.masks.get(next).map(|mask| mask.id.clone()));
                }
                ClipField::MaskMove => {
                    let Some(index) = current else { continue };
                    let last = clip.masks.len() as i64 - 1;
                    let to = (index as i64 + value.round() as i64).clamp(0, last) as usize;
                    let mask = clip.masks.remove(index);
                    refocus = Some(Some(mask.id.clone()));
                    clip.masks.insert(to, mask);
                }
                ClipField::MaskShape => {
                    let Some(shape) = shape else { continue };
                    match current {
                        // A new shape keeps where the mask was, its turn,
                        // softness, mode, invert and keys, and takes the
                        // new shape's own size.
                        Some(index) => {
                            let held = &mut clip.masks[index];
                            if held.shape() != Some(shape) {
                                let roundness = held.get("roundness").unwrap_or(0.0);
                                held.source = Mask::of("", "", shape).source;
                                held.set("roundness", roundness);
                            }
                        }
                        // A shape picked on a clip with no mask puts one on.
                        None => {
                            let id = Mask::next_id(&clip.masks);
                            clip.masks.push(Mask::of(id.clone(), "", shape));
                            refocus = Some(Some(id));
                        }
                    }
                }
                ClipField::MaskMode => {
                    let mode = place.and_then(|place| MaskMode::ALL.get(place).copied());
                    if let (Some(index), Some(mode)) = (current, mode) {
                        clip.masks[index].mode = mode;
                    }
                }
                ClipField::MaskInvert => {
                    if let Some(index) = current {
                        clip.masks[index].invert = value != 0.0;
                    }
                }
                field => {
                    if let (Some(index), Some(number)) = (current, mask_number_of(field)) {
                        clip.masks[index].write_at(number, value, at);
                    }
                }
            }
        }
        if let Some(focus) = refocus {
            self.mask_focus = focus;
        }
        true
    }

    /// The cluster's mask rows, after the clip's six: the current mask's
    /// numbers in [`MASK_KEY_FIELDS`] order, each blank when it has no such
    /// number or the clip has no mask.
    pub(super) fn mask_key_rows(&self, clip: &Clip, at: f64) -> Vec<ClipKeyData> {
        let current = self.current_mask(clip);
        MASK_KEY_FIELDS
            .iter()
            .map(|&field| {
                let number = mask_number_of(field).expect("a mask field");
                let Some(mask) = current.filter(|mask| mask.get(number).is_some()) else {
                    return ClipKeyData {
                        field,
                        keyed: false,
                        here: false,
                        prev: false,
                        next: false,
                    };
                };
                let keys = mask.keys_on(number);
                let near = |key: &model::ParamKey| (key.at - at).abs() <= model::KEY_EPSILON;
                ClipKeyData {
                    field,
                    keyed: !keys.is_empty(),
                    here: keys.iter().any(near),
                    prev: keys.iter().any(|key| key.at < at - model::KEY_EPSILON),
                    next: keys.iter().any(|key| key.at > at + model::KEY_EPSILON),
                }
            })
            .collect()
    }

    /// The commands a press on a mask field's diamond makes: a key on the
    /// current mask's number at the playhead, at the value it shows there,
    /// or the key there taken off. Empty when there is nothing to key.
    pub(super) fn mask_toggle_commands(&self, field: ClipField) -> Vec<Command> {
        let Some(number) = mask_number_of(field) else {
            return Vec::new();
        };
        let Some((clip, at)) = self.key_point() else {
            return Vec::new();
        };
        let Some(mask) = self
            .current_mask(clip)
            .filter(|mask| mask.get(number).is_some())
        else {
            return Vec::new();
        };
        let (clip_id, mask_id, number) = (clip.id.clone(), mask.id.clone(), number.to_owned());
        let here = mask
            .keys_on(&number)
            .iter()
            .find(|key| (key.at - at).abs() <= model::KEY_EPSILON);
        match here {
            Some(key) => vec![Command::ClearMaskKey {
                clip_id,
                mask_id,
                number,
                at: key.at,
            }],
            None => {
                let ease = mask
                    .keys_on(&number)
                    .iter()
                    .rfind(|key| key.at < at)
                    .map_or(model::KeyEase::LINEAR, |key| key.ease);
                let value = mask.value_at(&number, at);
                vec![Command::SetMaskKey {
                    clip_id,
                    mask_id,
                    number,
                    at,
                    value,
                    ease,
                }]
            }
        }
    }

    /// The current mask's keys on a mask field's number, in order; empty
    /// when the field is not a mask's or there is no mask. What stepping
    /// between keys walks.
    pub(super) fn mask_key_times(&self, field: ClipField) -> Vec<f64> {
        let Some(number) = mask_number_of(field) else {
            return Vec::new();
        };
        self.sole_selection()
            .and_then(|id| self.clip(&id))
            .and_then(|clip| self.current_mask(clip))
            .map(|mask| mask.keys_on(number).iter().map(|key| key.at).collect())
            .unwrap_or_default()
    }

    /// The command that takes every key off a mask field's number of the
    /// current mask, if it has any.
    pub(super) fn mask_clear_command(&self, field: ClipField) -> Option<Command> {
        let number = mask_number_of(field)?;
        let clip = self.sole_selection().and_then(|id| self.clip(&id))?;
        let mask = self
            .current_mask(clip)
            .filter(|mask| mask.is_keyed(number))?;
        Some(Command::ClearMaskKeys {
            clip_id: clip.id.clone(),
            mask_id: mask.id.clone(),
            number: number.to_owned(),
        })
    }

    /// The Mask section's rows for the selected clip, and the current
    /// one's place among them.
    pub(super) fn mask_rows(&self, clip: &Clip) -> (Vec<MaskRowData>, i32) {
        let rows = clip
            .masks
            .iter()
            .map(|mask| MaskRowData {
                name: mask.name.as_str().into(),
                shape: mask
                    .shape()
                    .and_then(|shape| MaskShape::ALL.iter().position(|s| *s == shape))
                    .map_or(-1, |place| place as i32),
                mode: MaskMode::ALL
                    .iter()
                    .position(|mode| *mode == mask.mode)
                    .map_or(0, |place| place as i32),
                invert: mask.invert,
                keyed: !mask.keys.is_empty(),
            })
            .collect();
        let current =
            current_index(&clip.masks, self.mask_focus.as_deref()).map_or(-1, |index| index as i32);
        (rows, current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masks(ids: &[&str]) -> Vec<Mask> {
        ids.iter()
            .map(|id| Mask::of(*id, "", MaskShape::Circle))
            .collect()
    }

    #[test]
    fn the_current_mask_is_the_picked_one_or_the_first() {
        let list = masks(&["mask-1", "mask-2"]);
        assert_eq!(current_index(&list, Some("mask-2")), Some(1));
        assert_eq!(current_index(&list, Some("gone")), Some(0));
        assert_eq!(current_index(&list, None), Some(0));
        assert_eq!(current_index(&[], Some("mask-1")), None);
    }

    #[test]
    fn every_mask_number_field_names_a_number_a_shape_has() {
        let mask = Mask::of("m", "", MaskShape::Rectangle);
        for field in MASK_KEY_FIELDS {
            let number = mask_number_of(field).expect("a number");
            assert!(mask.get(number).is_some(), "{number}");
            assert!(is_mask_field(field));
        }
        assert!(!is_mask_field(ClipField::Scale));
    }
}

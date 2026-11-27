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
use concat_core::mask::{MaskMode as EngineMode, MaskShape as EngineShape};
use concat_doc::commands::MaskNumber;
use concat_doc::model::{Entry, Mask};

/// The number of a mask a field writes; None for a field that is not one
/// of a mask's numbers.
pub(super) fn mask_number_of(field: ClipField) -> Option<MaskNumber> {
    Some(match field {
        ClipField::MaskX => MaskNumber::X,
        ClipField::MaskY => MaskNumber::Y,
        ClipField::MaskWidth => MaskNumber::Width,
        ClipField::MaskHeight => MaskNumber::Height,
        ClipField::MaskRotation => MaskNumber::Rotation,
        ClipField::MaskRoundness => MaskNumber::Roundness,
        ClipField::MaskFeather => MaskNumber::Feather,
        ClipField::MaskExpand => MaskNumber::Expand,
        ClipField::MaskOpacity => MaskNumber::Opacity,
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

/// See [`MaskKnob`]. None for the mask's scale, which no knob shows.
pub(super) fn mask_knob(number: MaskNumber) -> Option<MaskKnob> {
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
        MaskNumber::X => knob(
            t("common.positionX"),
            (-1.0, 1.0),
            0.005,
            0.0,
            ParamFormat::Centred,
            "%",
            100.0,
        ),
        MaskNumber::Y => knob(
            t("common.positionY"),
            (-1.0, 1.0),
            0.005,
            0.0,
            ParamFormat::Centred,
            "%",
            100.0,
        ),
        MaskNumber::Width => knob(
            t("common.width"),
            (0.01, 4.0),
            0.01,
            0.66,
            ParamFormat::Fraction,
            "%",
            100.0,
        ),
        MaskNumber::Height => knob(
            t("common.height"),
            (0.01, 4.0),
            0.01,
            0.66,
            ParamFormat::Fraction,
            "%",
            100.0,
        ),
        MaskNumber::Rotation => knob(
            t("common.rotation"),
            (-180.0, 180.0),
            1.0,
            0.0,
            ParamFormat::Degrees,
            "°",
            1.0,
        ),
        MaskNumber::Roundness => knob(
            t("mask.roundness"),
            (0.0, 1.0),
            0.01,
            0.0,
            ParamFormat::Fraction,
            "%",
            100.0,
        ),
        MaskNumber::Feather => knob(
            t("clipInspector.feather"),
            (0.0, 1.0),
            0.01,
            0.0,
            ParamFormat::Fraction,
            "%",
            100.0,
        ),
        MaskNumber::Expand => knob(
            t("mask.expand"),
            (-0.5, 0.5),
            0.005,
            0.0,
            ParamFormat::Centred,
            "%",
            100.0,
        ),
        MaskNumber::Opacity => knob(
            t("common.opacity"),
            (0.0, 1.0),
            0.01,
            1.0,
            ParamFormat::Fraction,
            "%",
            100.0,
        ),
        MaskNumber::Scale => return None,
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
/// first; None when there are none. A mask made on the echo has no id of
/// the document's yet: a focus on one is on the newest, the last.
pub(super) fn current_index(masks: &[&Mask], focus: Option<&str>) -> Option<usize> {
    let fresh = doc_view::new_mask_id().to_string();
    focus
        .and_then(|focus| {
            if focus == fresh {
                masks.len().checked_sub(1)
            } else {
                masks.iter().position(|mask| mask.id.to_string() == focus)
            }
        })
        .or((!masks.is_empty()).then_some(0))
}

/// Where a mask's shape sits in the picker's list; -1 for none.
pub(super) fn shape_place(mask: &Mask) -> i32 {
    doc_view::mask_shape(mask)
        .and_then(|shape| EngineShape::ALL.iter().position(|s| *s == shape))
        .map_or(-1, |place| place as i32)
}

/// Where a mask's mode sits in the picker's list; the first for one this
/// build does not know.
pub(super) fn mode_place(mask: &Mask) -> i32 {
    doc_view::engine_mask_mode(&mask.mode)
        .and_then(|mode| EngineMode::ALL.iter().position(|m| *m == mode))
        .map_or(0, |place| place as i32)
}

impl Studio {
    /// The clip's current mask; see the module.
    pub(super) fn current_mask<'a>(&self, clip: &'a Item) -> Option<&'a Mask> {
        let masks = doc_view::masks(&clip.masks);
        current_index(&masks, self.mask_focus.as_deref()).map(|index| masks[index])
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
        let fresh = doc_view::new_mask_id();
        for id in ids {
            let Some(clip) = self.clip(&id) else {
                continue;
            };
            if !self.kind(clip).is_visual() {
                continue;
            }
            let at = doc_view::content_at(clip, doc_view::tick(f64::from(self.playhead)));
            self.echo_edit(&id, |clip| {
                // The masks this build read, edited as a list; the entries it
                // could not read stay, after them.
                let mut masks = doc_view::masks_owned(&clip.masks);
                let unread: Vec<Entry<Mask>> = clip
                    .masks
                    .iter()
                    .filter(|entry| entry.known().is_none())
                    .cloned()
                    .collect();
                let current = current_index(&masks.iter().collect::<Vec<_>>(), focus.as_deref());
                let shape = place.and_then(|place| EngineShape::ALL.get(place).copied());
                match field {
                    ClipField::MaskAdd => {
                        if let Some(shape) = shape {
                            masks.push(doc_view::mask_of(fresh, shape));
                            refocus = Some(Some(fresh.to_string()));
                        }
                    }
                    ClipField::MaskSelect => {
                        if let Some(mask) = place.and_then(|place| masks.get(place)) {
                            refocus = Some(Some(mask.id.to_string()));
                        }
                    }
                    ClipField::MaskRemove => {
                        if let Some(index) = current {
                            masks.remove(index);
                            // The one below takes its place, or the one above
                            // when it was the last.
                            let next = index.min(masks.len().saturating_sub(1));
                            refocus = Some(masks.get(next).map(|mask| mask.id.to_string()));
                        }
                    }
                    ClipField::MaskMove => {
                        if let Some(index) = current {
                            let last = masks.len() as i64 - 1;
                            let to = (index as i64 + value.round() as i64).clamp(0, last) as usize;
                            let mask = masks.remove(index);
                            refocus = Some(Some(mask.id.to_string()));
                            masks.insert(to, mask);
                        }
                    }
                    ClipField::MaskShape => {
                        if let Some(shape) = shape {
                            match current {
                                // A new shape keeps where the mask was, its
                                // turn, softness, mode, invert and keys, and
                                // takes the new shape's own size.
                                Some(index) => {
                                    let held = &mut masks[index];
                                    if doc_view::mask_shape(held) != Some(shape) {
                                        let roundness =
                                            doc_view::mask_number(held, MaskNumber::Roundness)
                                                .cloned();
                                        held.source = doc_view::mask_of(fresh, shape).source;
                                        if let (Some(roundness), Some(number)) = (
                                            roundness,
                                            doc_view::mask_number_mut(held, MaskNumber::Roundness),
                                        ) {
                                            *number = roundness;
                                        }
                                    }
                                }
                                // A shape picked on a clip with no mask puts
                                // one on.
                                None => {
                                    masks.push(doc_view::mask_of(fresh, shape));
                                    refocus = Some(Some(fresh.to_string()));
                                }
                            }
                        }
                    }
                    ClipField::MaskMode => {
                        let mode = place.and_then(|place| EngineMode::ALL.get(place).copied());
                        if let (Some(index), Some(mode)) = (current, mode) {
                            masks[index].mode = doc_view::doc_mask_mode(mode);
                        }
                    }
                    ClipField::MaskInvert => {
                        if let Some(index) = current {
                            masks[index].invert = value != 0.0;
                        }
                    }
                    field => {
                        if let (Some(index), Some(number)) = (current, mask_number_of(field))
                            && let Some(held) = doc_view::mask_number_mut(&mut masks[index], number)
                        {
                            doc_view::write_at(held, doc_view::mask_clamp(number, value), at);
                        }
                    }
                }
                clip.masks = masks.into_iter().map(Entry::Known).chain(unread).collect();
            });
        }
        if let Some(focus) = refocus {
            self.mask_focus = focus;
        }
        true
    }

    /// The cluster's mask rows, after the clip's six: the current mask's
    /// numbers in [`MASK_KEY_FIELDS`] order, each blank when it has no such
    /// number or the clip has no mask.
    pub(super) fn mask_key_rows(&self, clip: &Item, at: Tick) -> Vec<ClipKeyData> {
        let current = self.current_mask(clip);
        MASK_KEY_FIELDS
            .iter()
            .map(|&field| {
                let number = mask_number_of(field).expect("a mask field");
                let Some(held) = current.and_then(|mask| doc_view::mask_number(mask, number))
                else {
                    return ClipKeyData {
                        field,
                        keyed: false,
                        here: false,
                        prev: false,
                        next: false,
                    };
                };
                let keys = &held.keys;
                let (prev, next) = doc_view::keys_around(keys, at);
                ClipKeyData {
                    field,
                    keyed: !keys.is_empty(),
                    here: doc_view::key_near(keys, at).is_some(),
                    prev: prev.is_some(),
                    next: next.is_some(),
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
        let Some(item) = self
            .sole_selection()
            .and_then(|id| doc_view::parse::<ItemId>(&id))
        else {
            return Vec::new();
        };
        let Some((clip, at)) = self.key_point() else {
            return Vec::new();
        };
        let Some(mask) = self.current_mask(clip) else {
            return Vec::new();
        };
        let Some(held) = doc_view::mask_number(mask, number) else {
            return Vec::new();
        };
        match doc_view::key_near(&held.keys, at) {
            Some(key) => vec![Command::ClearMaskKey {
                item,
                mask: mask.id,
                number,
                at: key.at,
            }],
            None => vec![Command::SetMaskKey {
                item,
                mask: mask.id,
                number,
                at,
                value: doc_view::value_at(held, at),
                ease: doc_view::ease_before(&held.keys, at),
            }],
        }
    }

    /// The current mask's keys on a mask field's number, in order; empty
    /// when the field is not a mask's or there is no mask. What stepping
    /// between keys walks.
    pub(super) fn mask_key_times(&self, field: ClipField) -> Vec<Tick> {
        let Some(number) = mask_number_of(field) else {
            return Vec::new();
        };
        self.sole_selection()
            .and_then(|id| self.clip(&id))
            .and_then(|clip| self.current_mask(clip))
            .and_then(|mask| doc_view::mask_number(mask, number))
            .map(|held| held.keys.iter().map(|key| key.at).collect())
            .unwrap_or_default()
    }

    /// The command that takes every key off a mask field's number of the
    /// current mask, if it has any.
    pub(super) fn mask_clear_command(&self, field: ClipField) -> Option<Command> {
        let number = mask_number_of(field)?;
        let id = self.sole_selection()?;
        let clip = self.clip(&id)?;
        let mask = self.current_mask(clip).filter(|mask| {
            doc_view::mask_number(mask, number).is_some_and(|held| held.is_keyed())
        })?;
        Some(Command::ClearMaskKeys {
            item: doc_view::parse(&id)?,
            mask: mask.id,
            number,
        })
    }

    /// The Mask section's rows for the selected clip, and the current
    /// one's place among them.
    pub(super) fn mask_rows(&self, clip: &Item) -> (Vec<MaskRowData>, i32) {
        let masks = doc_view::masks(&clip.masks);
        let rows = masks
            .iter()
            .map(|mask| MaskRowData {
                name: mask.name.as_str().into(),
                shape: shape_place(mask),
                mode: mode_place(mask),
                invert: mask.invert,
                keyed: doc_view::mask_keyed(mask),
            })
            .collect();
        let current =
            current_index(&masks, self.mask_focus.as_deref()).map_or(-1, |index| index as i32);
        (rows, current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masks(ids: &[&str]) -> Vec<Mask> {
        ids.iter()
            .map(|id| doc_view::mask_of(doc_view::parse(id).unwrap(), EngineShape::Circle))
            .collect()
    }

    #[test]
    fn the_current_mask_is_the_picked_one_or_the_first() {
        let list = masks(&["mk1", "mk2"]);
        let list: Vec<&Mask> = list.iter().collect();
        assert_eq!(current_index(&list, Some("mk2")), Some(1));
        assert_eq!(current_index(&list, Some("mk9")), Some(0));
        assert_eq!(current_index(&list, None), Some(0));
        assert_eq!(current_index(&[], Some("mk1")), None);
        // A mask made on the echo is the newest.
        let fresh = doc_view::new_mask_id().to_string();
        assert_eq!(current_index(&list, Some(&fresh)), Some(1));
    }

    #[test]
    fn every_mask_number_field_names_a_number_a_shape_has() {
        let mask = doc_view::mask_of(doc_view::new_mask_id(), EngineShape::Rectangle);
        for field in MASK_KEY_FIELDS {
            let number = mask_number_of(field).expect("a number");
            assert!(doc_view::mask_number(&mask, number).is_some(), "{number:?}");
            assert!(mask_knob(number).is_some(), "{number:?}");
            assert!(is_mask_field(field));
        }
        assert!(!is_mask_field(ClipField::Scale));
    }
}

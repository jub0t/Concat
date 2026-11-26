// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! Keyframes.

use concat_core::Halt;
use concat_doc::commands::MaskNumber;
use concat_doc::model::{Ease, Key};

use super::*;

/// The keys of the colour link of a clip's picture chain, and its place in
/// the chain the inspector counts: what the Adjust panel keys.
fn adjust_link(clip: &Item) -> Option<&Effect> {
    doc_view::chain(&clip.effects)
        .into_iter()
        .find(|link| link.package == ADJUST_ID)
}

/// The key of a run within a hair of `at`, if there is one.
fn key_here(keys: &[Key<f64>], at: Tick) -> Option<&Key<f64>> {
    doc_view::key_near(keys, at)
}

impl Studio {
    // ── keyframes ──

    /// The keyable property a field names, or None for a field that is a
    /// constant and stays one.
    pub fn key_property_of(field: ClipField) -> Option<KeyProperty> {
        Some(match field {
            ClipField::Scale => KeyProperty::Scale,
            ClipField::OffsetX => KeyProperty::OffsetX,
            ClipField::OffsetY => KeyProperty::OffsetY,
            ClipField::Rotation => KeyProperty::Rotation,
            ClipField::StretchX => KeyProperty::StretchX,
            ClipField::StretchY => KeyProperty::StretchY,
            ClipField::Opacity => KeyProperty::Opacity,
            ClipField::Volume => KeyProperty::Volume,
            _ => return None,
        })
    }

    /// The Speed tab's word on the selected clip: Constant or Curve, the
    /// curve's points, its line and which points are chosen, Smooth or
    /// Straight, the preset it still is, its fastest and slowest, and where
    /// the playhead crosses it. Points are synced row by row, so a point
    /// under the pointer is not dropped when the graph is published again.
    pub(super) fn publish_speed_ramp(&self, app: &App, models: &Models) {
        let ramp = app.global::<SpeedRamp>();
        let id = self.speed_clip();
        let clip = id.as_deref().and_then(|id| self.clip(id));
        let points = id
            .as_deref()
            .and_then(|id| self.speed_points(id))
            .unwrap_or_default();
        ramp.set_usable(clip.is_some());
        ramp.set_has_curve(!points.is_empty());
        // No curve shows as smooth: a curve begins smooth.
        ramp.set_eased(points.is_empty() || speed_ramp::is_eased(&points));
        ramp.set_active_preset(speed_ramp::preset_index(&points));
        let extreme = |faster| speed_ramp::extreme_of(&points, faster).unwrap_or(0.0) as f32;
        ramp.set_peak_speed(extreme(true));
        ramp.set_trough_speed(extreme(false));
        ramp.set_line_path(speed_ramp::curve_path(&points).into());
        let across = clip.and_then(|clip| {
            let local = f64::from(self.playhead) - doc_view::start_s(clip);
            let length = doc_view::len_s(clip);
            (0.0..=length)
                .contains(&local)
                .then(|| (local / length) as f32)
        });
        ramp.set_playhead_at(across.unwrap_or(-1.0));
        let last = points.len().saturating_sub(1);
        sync(
            &models.ramp_points,
            points
                .iter()
                .enumerate()
                .map(|(index, point)| SpeedPointData {
                    index: index as i32,
                    x: point.at as f32,
                    y: speed_ramp::height_for(point.speed) as f32,
                    end: index == 0 || index == last,
                })
                .collect(),
        );
        let chosen = self.speed_chosen_now();
        sync(
            &models.curve_selected,
            (0..points.len())
                .map(|index| chosen.contains(&index))
                .collect(),
        );
        sync(
            &models.preset_labels,
            speed_ramp::preset_labels()
                .into_iter()
                .map(SharedString::from)
                .collect(),
        );
    }

    /// The selected clip and where the playhead is on its content time, or
    /// None when there is no sole selection or the playhead is outside it.
    ///
    /// Outside is not held to an end: a key put on from outside the clip
    /// would land on its first or last frame, which is never what the person
    /// pressing the diamond meant.
    pub fn key_point(&self) -> Option<(&Item, Tick)> {
        let clip = self.sole_selection().and_then(|id| self.clip(&id))?;
        let playhead = doc_view::tick(f64::from(self.playhead));
        (clip.start..=doc_view::end(clip))
            .contains(&playhead)
            .then(|| (clip, doc_view::content_at(clip, playhead)))
    }

    /// The selected clip, its id as the document's and where the playhead
    /// is on its content time, as [`Studio::key_point`] has them.
    fn key_target(&self) -> Option<(ItemId, &Item, Tick)> {
        let id = doc_view::parse::<ItemId>(&self.sole_selection()?)?;
        let (clip, at) = self.key_point()?;
        Some((id, clip, at))
    }

    /// The selected clip with its id as the document's.
    fn sole_item(&self) -> Option<(ItemId, &Item)> {
        let id = self.sole_selection()?;
        Some((doc_view::parse(&id)?, self.clip(&id)?))
    }

    /// The timeline second a content tick of `clip` shows at, for the
    /// playhead to go to.
    fn seek_to_key(&mut self, clip: &Item, at: Tick) {
        let place = doc_view::timeline_at(clip, at);
        self.seek(doc_view::secs(place) as f32);
    }

    /// The keyframe cluster's eight rows, in the order the `Keyframes` global
    /// indexes them. Empty when nothing can be keyed, which is what greys
    /// every cluster in the inspector at once.
    pub fn key_rows(&self) -> Vec<ClipKeyData> {
        let Some((clip, at)) = self.key_point() else {
            return Vec::new();
        };
        let mut rows: Vec<ClipKeyData> = doc_view::KEY_PROPERTIES
            .iter()
            .map(|&property| {
                let keys = &doc_view::animated(clip, property).keys;
                let (prev, next) = doc_view::keys_around(keys, at);
                ClipKeyData {
                    field: key_field_of(property),
                    keyed: !keys.is_empty(),
                    here: key_here(keys, at).is_some(),
                    prev: prev.is_some(),
                    next: next.is_some(),
                }
            })
            .collect();
        rows.extend(self.mask_key_rows(clip, at));
        rows
    }

    /// The Keyframes pane's rows: the keyable properties of the selected clip,
    /// and every Adjust knob of it that carries keys, each with its keys
    /// along the clip - where, what value as a fraction of the knob's
    /// range, and the ease into it - what the knob is worth at the
    /// playhead, and the ride over the clip as a path for the curve view.
    pub fn key_editor_rows(&self) -> Vec<KeyRowData> {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return Vec::new();
        };
        let at = doc_view::content_at(clip, doc_view::tick(f64::from(self.playhead)));
        // A run of keys as marks: each at its share of the clip, with its
        // value as a fraction of the range and the ease into it, and the
        // ride as a path - a hold to the first key, a cubic per segment
        // (the ease's control points are exactly the curve's, in time and
        // value), a hold after the last.
        let marks_and_curve = |keys: &[(Tick, f64, Ease)],
                               to_fraction: &dyn Fn(f64) -> f64|
         -> (ModelRc<KeyMarkData>, String, i32) {
            let marks: Vec<KeyMarkData> = keys
                .iter()
                .enumerate()
                .map(|(index, (key_at, value, ease))| {
                    let [x1, y1, x2, y2] = doc_view::ease_parts(ease);
                    KeyMarkData {
                        at: doc_view::share_of(clip, *key_at) as f32,
                        here: doc_view::near(*key_at, at),
                        value: to_fraction(*value).clamp(0.0, 1.0) as f32,
                        ease_x1: x1 as f32,
                        ease_y1: y1 as f32,
                        ease_x2: x2 as f32,
                        ease_y2: y2 as f32,
                        eased: index > 0,
                    }
                })
                .collect();
            let mut curve = String::new();
            if let (Some(first), Some(last)) = (marks.first(), marks.last()) {
                let y = |value: f32| 1.0 - value;
                curve.push_str(&format!(
                    "M 0 {:.4} L {:.4} {:.4}",
                    y(first.value),
                    first.at,
                    y(first.value)
                ));
                for pair in marks.windows(2) {
                    let (a, b) = (&pair[0], &pair[1]);
                    let (dx, dy) = (b.at - a.at, b.value - a.value);
                    curve.push_str(&format!(
                        " C {:.4} {:.4} {:.4} {:.4} {:.4} {:.4}",
                        a.at + b.ease_x1 * dx,
                        y(a.value + b.ease_y1 * dy),
                        a.at + b.ease_x2 * dx,
                        y(a.value + b.ease_y2 * dy),
                        b.at,
                        y(b.value)
                    ));
                }
                curve.push_str(&format!(" L 1 {:.4}", y(last.value)));
            }
            // The key the ease menu edits: the one at the playhead, else
            // the next, when it has a key behind it to ease from.
            let ease_index = keys
                .iter()
                .enumerate()
                .position(|(index, (key_at, _, _))| {
                    index > 0 && (*key_at >= at || doc_view::near(*key_at, at))
                })
                .map_or(-1, |index| index as i32);
            (ModelRc::new(VecModel::from(marks)), curve, ease_index)
        };
        let run_of = |keys: &[Key<f64>], to_knob: &dyn Fn(f64) -> f64| -> Vec<(Tick, f64, Ease)> {
            keys.iter()
                .map(|key| (key.at, to_knob(key.value), key.ease.clone()))
                .collect()
        };
        // Only a picture or a shape is pulled out of its shape: a title's
        // glyphs never are, and a sound has no shape. Their Width and
        // Height rows would key something nothing draws, so they are left
        // out unless keys already sit on them to be taken off.
        let pulled = matches!(self.kind(clip), Kind::Video | Kind::Image | Kind::Shape);
        let mut rows = Vec::new();
        for property in doc_view::KEY_PROPERTIES {
            let held = doc_view::animated(clip, property);
            let pull = matches!(property, KeyProperty::StretchX | KeyProperty::StretchY);
            if pull && !pulled && !held.is_keyed() {
                continue;
            }
            // The same ranges and units as the inspector's knobs. The level
            // knob speaks decibels, floored where silence is.
            let (label, minimum, maximum, step, default_value, fmt, unit, unit_scale) =
                match property {
                    KeyProperty::Scale => (
                        t("common.scale"),
                        0.05,
                        8.0,
                        0.01,
                        1.0,
                        ParamFormat::Fraction,
                        "%",
                        100.0,
                    ),
                    KeyProperty::OffsetX => (
                        t("common.positionX"),
                        -1.0,
                        1.0,
                        0.005,
                        0.0,
                        ParamFormat::Centred,
                        "%",
                        100.0,
                    ),
                    KeyProperty::OffsetY => (
                        t("common.positionY"),
                        -1.0,
                        1.0,
                        0.005,
                        0.0,
                        ParamFormat::Centred,
                        "%",
                        100.0,
                    ),
                    KeyProperty::Rotation => (
                        t("common.rotation"),
                        -180.0,
                        180.0,
                        1.0,
                        0.0,
                        ParamFormat::Degrees,
                        "°",
                        1.0,
                    ),
                    // A pull reads as the inspector's Width and Height
                    // do: a percentage of the scaled size, 100 % being
                    // the picture's own shape.
                    KeyProperty::StretchX => (
                        t("common.width"),
                        0.1,
                        4.0,
                        0.01,
                        1.0,
                        ParamFormat::Fraction,
                        "%",
                        100.0,
                    ),
                    KeyProperty::StretchY => (
                        t("common.height"),
                        0.1,
                        4.0,
                        0.01,
                        1.0,
                        ParamFormat::Fraction,
                        "%",
                        100.0,
                    ),
                    KeyProperty::Opacity => (
                        t("common.opacity"),
                        0.0,
                        1.0,
                        0.01,
                        1.0,
                        ParamFormat::Fraction,
                        "%",
                        100.0,
                    ),
                    KeyProperty::Volume => (
                        t("common.level"),
                        -60.0,
                        24.0,
                        0.5,
                        0.0,
                        ParamFormat::Gain,
                        "",
                        1.0,
                    ),
                };
            let to_knob = |value: f64| knob_value(property, value);
            let keys = run_of(&held.keys, &to_knob);
            let (marks, curve, ease_index) =
                marks_and_curve(&keys, &|value| (value - minimum) / (maximum - minimum));
            let (prev, next) = doc_view::keys_around(&held.keys, at);
            rows.push(KeyRowData {
                field: key_field_of(property),
                param: SharedString::default(),
                label: label.into(),
                keys: marks,
                state: ClipKeyData {
                    field: key_field_of(property),
                    keyed: !keys.is_empty(),
                    here: key_here(&held.keys, at).is_some(),
                    prev: prev.is_some(),
                    next: next.is_some(),
                },
                value: to_knob(shown(clip, property, at)) as f32,
                minimum: minimum as f32,
                maximum: maximum as f32,
                step,
                default_value,
                fmt,
                unit: unit.into(),
                unit_scale,
                curve: curve.into(),
                ease_index,
            });
        }
        // Every mask's numbers: all of the current mask's, which the Mask
        // section is showing, and the keyed ones of the rest. A row is the
        // number's field with the mask's id beside it.
        let current = self.current_mask(clip).map(|mask| mask.id);
        for (index, mask) in doc_view::masks(&clip.masks).into_iter().enumerate() {
            let is_current = current == Some(mask.id);
            for field in masks::MASK_KEY_FIELDS {
                let number = masks::mask_number_of(field).expect("a mask field");
                let (Some(held), Some(knob)) =
                    (doc_view::mask_number(mask, number), masks::mask_knob(number))
                else {
                    continue;
                };
                if !is_current && !held.is_keyed() {
                    continue;
                }
                let keys = run_of(&held.keys, &|value| value);
                let span = knob.maximum - knob.minimum;
                let (marks, curve, ease_index) =
                    marks_and_curve(&keys, &|value| (value - knob.minimum) / span);
                let (prev, next) = doc_view::keys_around(&held.keys, at);
                rows.push(KeyRowData {
                    field,
                    param: doc_view::text(mask.id),
                    label: format!("{} ({})", knob.label, masks::mask_label(mask, index)).into(),
                    keys: marks,
                    state: ClipKeyData {
                        field,
                        keyed: held.is_keyed(),
                        here: key_here(&held.keys, at).is_some(),
                        prev: prev.is_some(),
                        next: next.is_some(),
                    },
                    value: doc_view::value_at(held, at) as f32,
                    minimum: knob.minimum as f32,
                    maximum: knob.maximum as f32,
                    step: knob.step,
                    default_value: knob.default_value,
                    fmt: knob.fmt,
                    unit: knob.unit.into(),
                    unit_scale: knob.unit_scale,
                    curve: curve.into(),
                    ease_index,
                });
            }
        }
        if let Some(link) = adjust_link(clip) {
            let inside = self.key_point().map(|(_, at)| at);
            for row in adjust_rows(&doc_view::chain(&clip.effects), inside) {
                if !row.keyed {
                    continue;
                }
                let keys = run_of(doc_view::param_keys(link, &row.key), &|value| value);
                let (minimum, maximum) = (f64::from(row.min), f64::from(row.max));
                let (marks, curve, ease_index) = marks_and_curve(&keys, &|value| {
                    if maximum > minimum {
                        (value - minimum) / (maximum - minimum)
                    } else {
                        0.5
                    }
                });
                rows.push(KeyRowData {
                    field: ClipField::Scale,
                    param: row.key.clone(),
                    label: row.label.clone(),
                    keys: marks,
                    state: ClipKeyData {
                        field: ClipField::Scale,
                        keyed: true,
                        here: row.here,
                        prev: row.prev,
                        next: row.next,
                    },
                    value: row.value,
                    minimum: row.min,
                    maximum: row.max,
                    step: row.step,
                    default_value: row.default_value,
                    fmt: row.fmt,
                    unit: row.unit.clone(),
                    unit_scale: 1.0,
                    curve: curve.into(),
                    ease_index,
                });
            }
            // A wheel that rides is one row, drawn from its master's keys;
            // its puck's keys sit at the same instants and move with them.
            if let Some(package) = Catalogue::builtin().get(ADJUST_ID) {
                let (wheels, _) = grading_rows(package, Some(link), -1, Some(inside));
                for wheel in wheels.into_iter().filter(|wheel| wheel.keyed) {
                    let [_, _, master] = grading::wheel_keys(&wheel.key);
                    let keys = run_of(doc_view::param_keys(link, &master), &|value| value);
                    let (minimum, maximum) = (f64::from(wheel.min), f64::from(wheel.max));
                    let (marks, curve, ease_index) = marks_and_curve(&keys, &|value| {
                        if maximum > minimum {
                            (value - minimum) / (maximum - minimum)
                        } else {
                            0.5
                        }
                    });
                    rows.push(KeyRowData {
                        field: ClipField::Scale,
                        param: master.as_str().into(),
                        label: wheel.label.clone(),
                        keys: marks,
                        state: ClipKeyData {
                            field: ClipField::Scale,
                            keyed: true,
                            here: wheel.here,
                            prev: wheel.prev,
                            next: wheel.next,
                        },
                        value: wheel.master,
                        minimum: wheel.min,
                        maximum: wheel.max,
                        step: 0.01,
                        default_value: wheel.default_value,
                        fmt: format_of(""),
                        unit: SharedString::default(),
                        unit_scale: 1.0,
                        curve: curve.into(),
                        ease_index,
                    });
                }
            }
        }
        rows
    }

    /// The key a row of the Keyframes pane names at `from` - a share of the
    /// clip, as the pane places its marks - and the commands that take it
    /// to `to` with `value`, or with its own value when that is None, and
    /// `ease`, or its own. `field` and `param` name the row as the pane
    /// does: a property, a mask's number with the mask's id, or an Adjust
    /// knob by its key. Empty when there is no such key.
    fn rekey(
        &self,
        field: ClipField,
        param: &str,
        from: f32,
        to: f32,
        value: Option<&dyn Fn(f64, f64) -> f64>,
        ease: Option<Ease>,
    ) -> Vec<Command> {
        let Some((item, clip)) = self.sole_item() else {
            return Vec::new();
        };
        let from = doc_view::at_share(clip, f64::from(from));
        let to = doc_view::at_share(clip, f64::from(to));
        if let Some(number) = masks::mask_number_of(field).filter(|_| !param.is_empty()) {
            let Some(mask) = doc_view::masks(&clip.masks)
                .into_iter()
                .find(|mask| mask.id.to_string() == param)
            else {
                return Vec::new();
            };
            let Some(key) = doc_view::mask_number(mask, number)
                .and_then(|held| key_here(&held.keys, from))
                .cloned()
            else {
                return Vec::new();
            };
            let value = match (value, masks::mask_knob(number)) {
                (Some(value), Some(knob)) => value(knob.minimum, knob.maximum),
                _ => key.value,
            };
            return vec![
                Command::ClearMaskKey {
                    item,
                    mask: mask.id,
                    number,
                    at: key.at,
                },
                Command::SetMaskKey {
                    item,
                    mask: mask.id,
                    number,
                    at: to,
                    value,
                    ease: ease.unwrap_or(key.ease),
                },
            ];
        }
        if param.is_empty() {
            let Some(property) = Self::key_property_of(field) else {
                return Vec::new();
            };
            let Some(key) = key_here(&doc_view::animated(clip, property).keys, from).cloned()
            else {
                return Vec::new();
            };
            let value = match value {
                Some(value) => {
                    let (minimum, maximum) = knob_range(property);
                    model_value(property, value(minimum, maximum))
                }
                None => key.value,
            };
            return vec![
                Command::ClearClipKey {
                    item,
                    property,
                    at: key.at,
                },
                Command::SetClipKey {
                    item,
                    property,
                    at: to,
                    value,
                    ease: ease.unwrap_or(key.ease),
                },
            ];
        }
        let Some(link) = adjust_link(clip) else {
            return Vec::new();
        };
        let Some(key) = key_here(doc_view::param_keys(link, param), from).cloned() else {
            return Vec::new();
        };
        let value = match value {
            Some(value) => {
                let Some(row) = adjust_rows(&doc_view::chain(&clip.effects), Some(from))
                    .into_iter()
                    .find(|row| row.key == param)
                else {
                    return Vec::new();
                };
                value(f64::from(row.min), f64::from(row.max))
            }
            None => key.value,
        };
        let mut commands = vec![
            Command::ClearEffectKey {
                item,
                effect: link.id,
                param: param.to_owned(),
                at: key.at,
            },
            Command::SetEffectKey {
                item,
                effect: link.id,
                param: param.to_owned(),
                at: to,
                value,
                ease: ease.unwrap_or(key.ease),
            },
        ];
        commands.extend(partners_moved(link, item, param, key.at, to));
        commands
    }

    /// Moves the key at `from` to `to` and gives it `fraction` of its
    /// row's range, keeping its ease: a drag of a handle in the curve view.
    pub fn drag_key(&mut self, field: ClipField, param: &str, from: f32, to: f32, fraction: f32) {
        let fraction = f64::from(fraction).clamp(0.0, 1.0);
        let value = move |minimum: f64, maximum: f64| minimum + fraction * (maximum - minimum);
        let commands = self.rekey(field, param, from, to.clamp(0.0, 1.0), Some(&value), None);
        if !commands.is_empty() {
            self.apply(Command::Batch { commands });
        }
    }

    /// Every key's place on the selected clip's content time: its own
    /// numbers', its masks' and its colour link's.
    fn key_places(clip: &Item) -> Vec<Tick> {
        let mut ats: Vec<Tick> = doc_view::KEY_PROPERTIES
            .iter()
            .flat_map(|&property| doc_view::animated(clip, property).keys.iter().map(|key| key.at))
            .collect();
        for mask in doc_view::masks(&clip.masks) {
            for number in doc_view::MASK_NUMBERS {
                if let Some(held) = doc_view::mask_number(mask, number) {
                    ats.extend(held.keys.iter().map(|key| key.at));
                }
            }
        }
        if let Some(link) = adjust_link(clip) {
            ats.extend(doc_view::param_key_places(link));
        }
        ats
    }

    /// The selected clip's keys as instants on the timeline, every property
    /// and Adjust knob together, one diamond per instant: what the ruler
    /// draws while exactly one clip is selected (#188).
    pub fn key_marks(&self) -> Vec<f32> {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return Vec::new();
        };
        let mut ats = Self::key_places(clip);
        ats.sort();
        ats.dedup_by(|a, b| doc_view::near(*a, *b));
        ats.iter()
            .map(|at| doc_view::secs(doc_view::timeline_at(clip, *at)) as f32)
            .collect()
    }

    /// Moves every key of the selected clip that sits at the instant
    /// `from` to the instant `to`, as one undo step: what a dragged ruler
    /// diamond does.
    pub fn move_keys_at(&mut self, from: f32, to: f32) {
        let Some((item, clip)) = self.sole_item() else {
            return;
        };
        let from_at = doc_view::content_free(clip, doc_view::tick(f64::from(from)));
        let to_at = doc_view::content_at(clip, doc_view::tick(f64::from(to)));
        let mut commands = Vec::new();
        for property in doc_view::KEY_PROPERTIES {
            let Some(key) = key_here(&doc_view::animated(clip, property).keys, from_at) else {
                continue;
            };
            commands.push(Command::ClearClipKey {
                item,
                property,
                at: key.at,
            });
            commands.push(Command::SetClipKey {
                item,
                property,
                at: to_at,
                value: key.value,
                ease: key.ease.clone(),
            });
        }
        for mask in doc_view::masks(&clip.masks) {
            for number in doc_view::MASK_NUMBERS {
                let Some(key) =
                    doc_view::mask_number(mask, number).and_then(|held| key_here(&held.keys, from_at))
                else {
                    continue;
                };
                commands.push(Command::ClearMaskKey {
                    item,
                    mask: mask.id,
                    number,
                    at: key.at,
                });
                commands.push(Command::SetMaskKey {
                    item,
                    mask: mask.id,
                    number,
                    at: to_at,
                    value: key.value,
                    ease: key.ease.clone(),
                });
            }
        }
        if let Some(link) = adjust_link(clip) {
            for name in link.params.keys() {
                let Some(key) = key_here(doc_view::param_keys(link, name), from_at) else {
                    continue;
                };
                commands.push(Command::ClearEffectKey {
                    item,
                    effect: link.id,
                    param: name.clone(),
                    at: key.at,
                });
                commands.push(Command::SetEffectKey {
                    item,
                    effect: link.id,
                    param: name.clone(),
                    at: to_at,
                    value: key.value,
                    ease: key.ease.clone(),
                });
            }
        }
        if !commands.is_empty() {
            self.apply(Command::Batch { commands });
        }
    }

    /// Moves the playhead to `at` of the selected clip, `0..1`.
    pub fn jump_to_key(&mut self, at: f32) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)).cloned() else {
            return;
        };
        let at = doc_view::at_share(&clip, f64::from(at));
        self.seek_to_key(&clip, at);
    }

    /// Moves the key at `from` to `to` on a property, or on the Adjust
    /// knob `param` names: the same value and ease at the new place, as
    /// one undo step.
    pub fn move_key(&mut self, field: ClipField, param: &str, from: f32, to: f32) {
        let commands = self.rekey(field, param, from, to.clamp(0.0, 1.0), None, None);
        if !commands.is_empty() {
            self.apply(Command::Batch { commands });
        }
    }

    /// Sets the ease into the key at `at`. A drag of the curve editor is
    /// many of these, folded into one undo step.
    pub fn set_key_ease(&mut self, field: ClipField, param: &str, at: f32, ease: [f32; 4]) {
        let Some((item, clip)) = self.sole_item() else {
            return;
        };
        let ease = doc_view::ease_of(ease.map(f64::from));
        let at = doc_view::at_share(clip, f64::from(at));
        if let Some(number) = masks::mask_number_of(field).filter(|_| !param.is_empty()) {
            let Some(mask) = doc_view::masks(&clip.masks)
                .into_iter()
                .find(|mask| mask.id.to_string() == param)
            else {
                return;
            };
            let Some(key) = doc_view::mask_number(mask, number)
                .and_then(|held| key_here(&held.keys, at))
                .cloned()
            else {
                return;
            };
            let command = Command::SetMaskKey {
                item,
                mask: mask.id,
                number,
                at: key.at,
                value: key.value,
                ease,
            };
            self.apply_within("ease", command);
            return;
        }
        let command = if param.is_empty() {
            let Some(property) = Self::key_property_of(field) else {
                return;
            };
            let Some(key) = key_here(&doc_view::animated(clip, property).keys, at) else {
                return;
            };
            Command::SetClipKey {
                item,
                property,
                at: key.at,
                value: key.value,
                ease,
            }
        } else {
            let Some(link) = adjust_link(clip) else {
                return;
            };
            if key_here(doc_view::param_keys(link, param), at).is_none() {
                return;
            }
            let mut commands: Vec<Command> = std::iter::once(param.to_owned())
                .chain(wheel_partners(param))
                .filter_map(|name| {
                    let key = key_here(doc_view::param_keys(link, &name), at)?;
                    Some(Command::SetEffectKey {
                        item,
                        effect: link.id,
                        at: key.at,
                        value: key.value,
                        param: name,
                        ease: ease.clone(),
                    })
                })
                .collect();
            if commands.len() == 1 {
                commands.remove(0)
            } else {
                Command::Batch { commands }
            }
        };
        self.apply_within("ease", command);
    }

    /// Moves the playhead to the nearest key behind (-1) or ahead (+1) on
    /// any knob of the selected clip.
    pub fn step_any_key(&mut self, delta: i32) {
        let Some((clip, at)) = self.key_point() else {
            return;
        };
        let candidates = Self::key_places(clip);
        let target = if delta < 0 {
            candidates
                .into_iter()
                .filter(|&key_at| key_at < at && !doc_view::near(key_at, at))
                .max()
        } else {
            candidates
                .into_iter()
                .filter(|&key_at| key_at > at && !doc_view::near(key_at, at))
                .min()
        };
        let Some(target) = target else {
            return;
        };
        let clip = clip.clone();
        self.seek_to_key(&clip, target);
    }

    /// Takes every key off the selected clip: its properties and its
    /// Adjust knobs, as one undo step.
    pub fn clear_all_keys(&mut self) {
        let Some((item, clip)) = self.sole_item() else {
            return;
        };
        let mut commands: Vec<Command> = doc_view::KEY_PROPERTIES
            .iter()
            .filter(|&&property| doc_view::animated(clip, property).is_keyed())
            .map(|&property| Command::ClearClipKeys { item, property })
            .collect();
        for mask in doc_view::masks(&clip.masks) {
            for number in doc_view::MASK_NUMBERS {
                if doc_view::mask_number(mask, number).is_some_and(|held| held.is_keyed()) {
                    commands.push(Command::ClearMaskKeys {
                        item,
                        mask: mask.id,
                        number,
                    });
                }
            }
        }
        if let Some(link) = adjust_link(clip) {
            for (name, param) in &link.params {
                if !param.keys.is_empty() {
                    commands.push(Command::ClearEffectKeys {
                        item,
                        effect: link.id,
                        param: name.clone(),
                    });
                }
            }
        }
        if !commands.is_empty() {
            self.apply(Command::Batch { commands });
        }
    }

    /// Puts a key on the field at the playhead, or takes off the one there.
    ///
    /// The value a new key gets is whatever the property is worth at that
    /// instant - the constant on a clip with no keys, and the ride's own
    /// value on one that has them. That is what makes the diamond safe to
    /// press: it never moves the picture, it only says "hold this here".
    pub fn toggle_key(&mut self, field: ClipField) {
        if masks::mask_number_of(field).is_some() {
            let commands = self.mask_toggle_commands(field);
            if !commands.is_empty() {
                self.apply(Command::Batch { commands });
            }
            return;
        }
        let Some(property) = Self::key_property_of(field) else {
            return;
        };
        let Some((item, clip, at)) = self.key_target() else {
            return;
        };
        let held = doc_view::animated(clip, property);
        let command = match key_here(&held.keys, at) {
            Some(key) => Command::ClearClipKey {
                item,
                property,
                at: key.at,
            },
            // The ease of whichever key this one is joining behind, so
            // laying a run of keys down does not alternate between shapes.
            // The first key on a property has nothing to inherit and gets
            // the straight line.
            None => Command::SetClipKey {
                item,
                property,
                at,
                value: doc_view::value_at(held, at),
                ease: doc_view::ease_before(&held.keys, at),
            },
        };
        self.apply(command);
    }

    /// Takes every key off a field, leaving it its constant.
    pub fn clear_keys_on(&mut self, field: ClipField) {
        if masks::mask_number_of(field).is_some() {
            if let Some(command) = self.mask_clear_command(field) {
                self.apply(command);
            }
            return;
        }
        let Some(property) = Self::key_property_of(field) else {
            return;
        };
        let Some(item) = self
            .sole_selection()
            .and_then(|id| doc_view::parse::<ItemId>(&id))
        else {
            return;
        };
        self.apply(Command::ClearClipKeys { item, property });
    }

    /// Moves the playhead to this field's previous (-1) or next (+1) key.
    pub fn step_key(&mut self, field: ClipField, delta: i32) {
        let Some((clip, at)) = self.key_point() else {
            return;
        };
        let clip = clip.clone();
        let times: Vec<Tick> = if masks::mask_number_of(field).is_some() {
            self.mask_key_times(field)
        } else if let Some(property) = Self::key_property_of(field) {
            doc_view::animated(&clip, property)
                .keys
                .iter()
                .map(|key| key.at)
                .collect()
        } else {
            return;
        };
        let target = if delta < 0 {
            times
                .into_iter()
                .filter(|&key| key < at && !doc_view::near(key, at))
                .max()
        } else {
            times
                .into_iter()
                .filter(|&key| key > at && !doc_view::near(key, at))
                .min()
        };
        if let Some(target) = target {
            self.seek_to_key(&clip, target);
        }
    }

    /// The audio tracks the Audio panel offers for the selection: one label
    /// per sound stream of the selected clip's file when it has more than
    /// one, else nothing - a file with one stream has nothing to choose.
    pub(super) fn audio_track_labels(&self) -> Vec<SharedString> {
        let Some(media) = self
            .sole_selection()
            .and_then(|id| self.clip(&id))
            .and_then(|clip| doc_view::media_of(self.doc(), clip))
        else {
            return Vec::new();
        };
        if media.streams.len() < 2 {
            return Vec::new();
        }
        media
            .streams
            .iter()
            .enumerate()
            .map(|(position, track)| audio_track_label(track, position).into())
            .collect()
    }

    /// The selected transition, as the inspector's panel for it shows it.
    pub(super) fn selected_transition_data(&self) -> crate::ui::SelectedTransitionData {
        let Some(clip) = self.selected_transition() else {
            return Default::default();
        };
        let Some(cut) = clip.cut_in.as_ref() else {
            return Default::default();
        };
        let name = Catalogue::builtin()
            .get(&cut.effect.package)
            .map(|package| package.manifest.effect.name.clone())
            .unwrap_or_else(|| label_of(&cut.effect.package));
        crate::ui::SelectedTransitionData {
            present: true,
            name: name.into(),
            duration: doc_view::len_secs(cut.len) as f32,
            into: clip.name.as_str().into(),
        }
    }

    /// The selected transition's own knobs as the panel draws them: the
    /// choices as segments, the numbers as sliders, each at what the cut
    /// holds or the package's default.
    pub(super) fn transition_knobs(
        &self,
    ) -> (Vec<AppliedParamData>, Vec<crate::ui::TransitionChoiceData>) {
        let Some(transition) = self
            .selected_transition()
            .and_then(|clip| clip.cut_in.as_ref())
            .map(|cut| &cut.effect)
        else {
            return Default::default();
        };
        let Some(package) = Catalogue::builtin().get(&transition.package) else {
            return Default::default();
        };
        let (mut knobs, mut choices) = (Vec::new(), Vec::new());
        for param in &package.manifest.params {
            let value = doc_view::param_or(transition, &param.key, param.default);
            match param.kind {
                ParamType::Enum if !param.values.is_empty() => {
                    let options: Vec<SharedString> = param
                        .values
                        .iter()
                        .enumerate()
                        .map(|(index, at)| {
                            param
                                .labels
                                .get(index)
                                .cloned()
                                .unwrap_or_else(|| format!("{at}"))
                                .into()
                        })
                        .collect();
                    let current = param
                        .values
                        .iter()
                        .position(|at| (at - value).abs() < 1e-9)
                        .unwrap_or(0);
                    choices.push(crate::ui::TransitionChoiceData {
                        key: param.key.as_str().into(),
                        label: param.label.as_str().into(),
                        options: ModelRc::new(VecModel::from(options)),
                        values: ModelRc::new(VecModel::from(
                            param.values.iter().map(|at| *at as f32).collect::<Vec<_>>(),
                        )),
                        current: current as i32,
                    });
                }
                ParamType::Float | ParamType::Int => {
                    let step = if param.step > 0.0 {
                        param.step
                    } else if param.kind == ParamType::Int {
                        1.0
                    } else {
                        (param.max - param.min) / 200.0
                    };
                    knobs.push(AppliedParamData {
                        entry: 0,
                        key: param.key.as_str().into(),
                        label: param.label.as_str().into(),
                        group: "".into(),
                        unit: param.unit.as_str().into(),
                        min: param.min as f32,
                        max: param.max as f32,
                        step: step as f32,
                        default_value: param.default as f32,
                        value: value as f32,
                        fmt: format_of(&param.unit),
                        keyable: false,
                        keyed: false,
                        here: false,
                        prev: false,
                        next: false,
                    });
                }
                _ => {}
            }
        }
        (knobs, choices)
    }

    pub(super) fn selected(&self) -> SelectedClipData {
        // Titles share their styling controls; a sound batch exposes volume
        // alone, using the first unlocked clip as the displayed reference.
        let mut targets = self.edit_targets();
        let batch_volume = targets.is_empty();
        if batch_volume {
            targets = self.volume_targets();
        }
        let Some(id) = targets.first() else {
            return SelectedClipData::default();
        };
        let Some(clip) = self.clip(id) else {
            return SelectedClipData::default();
        };
        let media = doc_view::media_of(self.doc(), clip);
        let cut = match &clip.content {
            Content::Media(cut) => Some(cut),
            _ => None,
        };
        // A keyed property shows what it is worth at the playhead, which is
        // what the knob edits.
        let at = doc_view::content_at(clip, doc_view::tick(f64::from(self.playhead)));
        let text = doc_view::title(clip).cloned().unwrap_or_default();
        // Where the family sits in the picker's list; none when nothing on
        // this machine has it, which the picker shows in red.
        let family_row = self
            .font_families()
            .iter()
            .position(|family| family.eq_ignore_ascii_case(text.font_family.trim_matches('"')));
        // Some words selected in the text box: the typography controls
        // style those words, so they show what the first of them wears -
        // a run's override where it has one, the title's look elsewhere.
        let words = self.styled_words().filter(|(words_id, _, _)| words_id == id);
        let look = words
            .as_ref()
            .and_then(|(_, start, _)| {
                text.runs
                    .iter()
                    .find(|run| run.start as usize <= *start && *start < run.end as usize)
                    .cloned()
            })
            .unwrap_or_default();
        let words_styled = words.as_ref().is_some_and(|(_, start, end)| {
            text.runs
                .iter()
                .any(|run| (run.start as usize) < *end && *start < run.end as usize)
        });
        let fill = doc_view::to_slint(look.color.unwrap_or(text.color));
        let stroke = doc_view::to_slint(text.stroke_color);
        let plate = doc_view::to_slint(text.plate.unwrap_or(model::Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }));
        let picture = &clip.picture;
        let cutout = picture.cutout.as_ref();
        let analysis = cutout.zip(doc_view::media_id(clip)).and_then(|(cutout, media)| {
            self.cutout_jobs
                .get(&Self::analysis_key(&media.to_string(), &cutout.subject))
                .copied()
        });
        let mut enhance = self.enhance_readout(id, clip);
        let shape = doc_view::shape(clip).cloned().unwrap_or_default();
        let steady = self.stabilize_readout(clip);
        let enhance_preview = enhance.preview.is_some();
        let (enhance_before, enhance_after, (estimate_total, estimate_frames, estimate_per_frame)) =
            enhance.preview.take().unwrap_or_default();
        let current_mask = self.current_mask(clip);
        let fresh_mask = doc_view::mask_of(doc_view::new_mask_id(), concat_core::mask::MaskShape::Circle);
        let mask_number = |number: MaskNumber| -> f32 {
            doc_view::mask_value_at(current_mask.unwrap_or(&fresh_mask), number, at) as f32
        };
        let (mask_rows, mask_index) = self.mask_rows(clip);
        let motion = &picture.motion;
        let voice = clip.sound.voice.as_ref();
        let stabilize = picture.stabilize.clone();
        let colour = |colour: Option<model::Color>| {
            colour.map_or(slint::Color::from_rgb_u8(255, 255, 255), doc_view::to_slint)
        };
        let gradient = shape.gradient.clone();
        let shadow = shape.shadow.clone();
        let enhanced = cut.and_then(|cut| cut.enhanced.as_ref());
        SelectedClipData {
            present: true,
            batch: targets.len() as i32,
            batch_volume,
            has_sound: self.clip_has_sound(clip),
            frame_width: self.output_size().0 as i32,
            frame_height: self.output_size().1 as i32,
            id: id.as_str().into(),
            name: clip.name.as_str().into(),
            kind: kind_of(self.doc(), clip),
            duration: doc_view::len_s(clip) as f32,
            transition_id: clip
                .cut_in
                .as_ref()
                .map(|cut| cut.effect.package.as_str())
                .unwrap_or_default()
                .into(),
            transition_name: clip
                .cut_in
                .as_ref()
                .and_then(|cut| Catalogue::builtin().get(&cut.effect.package))
                .map(|package| package.manifest.effect.name.clone())
                .unwrap_or_default()
                .into(),
            transition_duration: clip
                .cut_in
                .as_ref()
                .map(|cut| doc_view::len_secs(cut.len) as f32)
                .unwrap_or(0.5),
            scale: shown(clip, KeyProperty::Scale, at) as f32,
            offset_x: shown(clip, KeyProperty::OffsetX, at) as f32,
            offset_y: shown(clip, KeyProperty::OffsetY, at) as f32,
            rotation: shown(clip, KeyProperty::Rotation, at) as f32,
            stretch_x: shown(clip, KeyProperty::StretchX, at) as f32,
            stretch_y: shown(clip, KeyProperty::StretchY, at) as f32,
            opacity: shown(clip, KeyProperty::Opacity, at) as f32,
            volume: shown(clip, KeyProperty::Volume, at) as f32,
            // The row of the Audio panel's list: the stream's place in the
            // file's list, the first for the file's own.
            audio_track: cut
                .and_then(|cut| cut.stream)
                .zip(media)
                .and_then(|(stream, media)| {
                    media.streams.iter().position(|held| held.index == stream)
                })
                .unwrap_or(0) as i32,
            speed: doc_view::speed(clip) as f32,
            preserve_pitch: clip.sound.keep_pitch,
            flip_h: picture.flip_h,
            flip_v: picture.flip_v,
            blend: concat_core::Blend::ALL
                .iter()
                .position(|mode| *mode == doc_view::engine_blend(&picture.blend))
                .unwrap_or(0) as i32,
            crop_left: picture.crop.map(|crop| crop.left as f32).unwrap_or(0.0),
            crop_top: picture.crop.map(|crop| crop.top as f32).unwrap_or(0.0),
            crop_right: picture.crop.map(|crop| crop.right as f32).unwrap_or(0.0),
            crop_bottom: picture.crop.map(|crop| crop.bottom as f32).unwrap_or(0.0),
            masks: ModelRc::new(VecModel::from(mask_rows)),
            mask_index,
            mask: current_mask.map_or(-1, masks::shape_place),
            mask_invert: current_mask.is_some_and(|mask| mask.invert),
            mask_mode: current_mask.map_or(0, masks::mode_place),
            motion_in: motion_place(motion.enter.as_ref().map(|m| m.name.as_str()), false),
            motion_in_seconds: motion
                .enter
                .as_ref()
                .map_or(0.5, |m| doc_view::len_secs(m.len)) as f32,
            motion_out: motion_place(motion.exit.as_ref().map(|m| m.name.as_str()), false),
            motion_out_seconds: motion
                .exit
                .as_ref()
                .map_or(0.5, |m| doc_view::len_secs(m.len)) as f32,
            motion_loop: motion_place(motion.repeat.as_ref().map(|m| m.name.as_str()), true),
            motion_loop_seconds: motion
                .repeat
                .as_ref()
                .map_or(1.0, |m| doc_view::len_secs(m.lap)) as f32,
            motion_loop_amount: motion.repeat.as_ref().map_or(1.0, |m| m.amount) as f32,
            mask_x: mask_number(MaskNumber::X),
            mask_y: mask_number(MaskNumber::Y),
            mask_width: mask_number(MaskNumber::Width),
            mask_height: mask_number(MaskNumber::Height),
            mask_rotation: mask_number(MaskNumber::Rotation),
            mask_roundness: mask_number(MaskNumber::Roundness),
            mask_feather: mask_number(MaskNumber::Feather),
            mask_expand: mask_number(MaskNumber::Expand),
            mask_opacity: mask_number(MaskNumber::Opacity),
            fade_in: doc_view::dur_secs(clip.sound.fade_in) as f32,
            wave_head: self.wave_end(clip, false),
            wave_tail: self.wave_end(clip, true),
            fade_out: doc_view::dur_secs(clip.sound.fade_out) as f32,
            content: text.content.as_str().into(),
            font_family: text.font_family.trim_matches('"').into(),
            family_row: family_row.unwrap_or(0) as i32,
            font_missing: family_row.is_none()
                && !text.font_family.trim_matches('"').trim().is_empty(),
            font_size: (text.font_size * look.font_scale.unwrap_or(1.0)) as f32,
            font_weight: look.font_weight.unwrap_or(text.font_weight) as f32,
            italic: look.italic.unwrap_or(text.italic),
            underline: look.underline.unwrap_or(text.underline),
            strikethrough: look.strikethrough.unwrap_or(text.strikethrough),
            styling_words: words.is_some(),
            words_styled,
            text_case: match text.case {
                model::TextCase::Upper => 1,
                model::TextCase::Lower => 2,
                model::TextCase::Sentence => 3,
                model::TextCase::Original | model::TextCase::Unknown(_) => 0,
            },
            fill,
            fill_hex: hex_of(fill).into(),
            align: align_of(&text.align),
            text_opacity: text.opacity as f32,
            stroke_width: text.stroke_width as f32,
            stroke,
            stroke_hex: hex_of(stroke).into(),
            stroke_opacity: f32::from(stroke.alpha()) / 255.0,
            shadow: text.shadow,
            plate,
            plate_hex: hex_of(plate).into(),
            plated: plate.alpha() > 0,
            plate_opacity: f32::from(plate.alpha()) / 255.0,
            plate_radius: text.plate_radius as f32,
            plate_padding_x: text.plate_padding_x as f32,
            plate_padding_y: text.plate_padding_y as f32,
            line_height: text.line_height as f32,
            tracking: text.tracking as f32,
            text_width: text.max_width as f32,
            text_height: text.max_height as f32,
            cutout: match cutout {
                None => 0,
                Some(cutout) if cutout.mode == model::CutoutMode::Auto => 1,
                Some(_) => 2,
            },
            cutout_feather: cutout.map_or(doc_view::CUTOUT_FEATHER, |cutout| cutout.feather)
                as f32,
            cutout_strokes: cutout.map_or(0, |cutout| cutout.strokes.len() as i32),
            cutout_subject: cutout.map_or(0, |cutout| match cutout.subject {
                model::Subject::Person => 1,
                model::Subject::Object => 2,
                model::Subject::Auto | model::Subject::Unknown(_) => 0,
            }),
            cutout_progress: analysis.map(|(_, fraction)| fraction).unwrap_or(-1.0),
            cutout_fetching: analysis.is_some_and(|(fetching, _)| fetching),
            cutout_empty: self.cutout_empty(clip),
            stabilize: stabilize.is_some(),
            stabilize_smoothness: stabilize
                .as_ref()
                .map_or(0.5, |stabilize| stabilize.smoothness) as f32,
            stabilize_max_zoom: stabilize
                .as_ref()
                .map_or(1.2, |stabilize| stabilize.max_zoom) as f32,
            stabilize_tripod: stabilize.as_ref().is_some_and(|stabilize| stabilize.tripod),
            stabilize_progress: doc_view::media_id(clip)
                .and_then(|media| self.stabilize_jobs.get(&media.to_string()).copied())
                .filter(|_| stabilize.is_some())
                .unwrap_or(-1.0),
            stabilize_zoom: steady.map_or(-1.0, |(zoom, _)| zoom as f32),
            stabilize_needed: steady.map_or(-1.0, |(_, needed)| needed as f32),
            voice_mode: match voice {
                None => 0,
                Some(mix) if mix.voice == 1.0 && mix.background == 0.0 => 1,
                Some(mix) if mix.voice == 0.0 && mix.background == 1.0 => 2,
                Some(_) => 3,
            },
            voice_level: voice.map_or(1.0, |mix| mix.voice as f32),
            background_level: voice.map_or(1.0, |mix| mix.background as f32),
            voice_progress: self.voice_progress(clip).0,
            voice_fetching: self.voice_progress(clip).1,
            voice_kind: match voice.map(|mix| &mix.kind) {
                Some(model::VoiceKind::Vocals) => 0,
                _ => 1,
            },
            enhance_progress: self
                .enhance_jobs
                .get(id)
                .map(|(_, fraction)| *fraction)
                .unwrap_or(-1.0),
            enhance_fetching: self
                .enhance_jobs
                .get(id)
                .is_some_and(|(fetching, _)| *fetching),
            reverse_progress: self.reverse_jobs.get(id).copied().unwrap_or(-1.0),
            enhance_linked: enhanced.is_some(),
            enhance_on: self.enhance_ticked(id, clip),
            enhance_busy: !self.enhance_jobs.is_empty() || self.enhance_previewing.is_some(),
            enhance_scale: enhance.scale,
            shape_kind: doc_view::SHAPE_KINDS
                .iter()
                .position(|held| *held == shape.kind)
                .unwrap_or(0) as i32,
            shape_is_line: doc_view::is_line(&shape.kind),
            shape_width: shape.width as f32,
            shape_height: doc_view::figure_box(&shape).1 as f32,
            shape_radius: shape.corner_radius as f32,
            shape_sides: shape.sides as i32,
            shape_points: shape.points as i32,
            shape_inner: shape.inner_radius as f32,
            shape_fill_on: shape.fill.is_some(),
            shape_fill: colour(shape.fill),
            shape_gradient: match &gradient {
                None => 0,
                Some(gradient) if gradient.radial => 2,
                Some(_) => 1,
            },
            shape_gradient_color: colour(gradient.as_ref().map(|gradient| gradient.color)),
            shape_gradient_angle: gradient.as_ref().map_or(90.0, |g| g.angle) as f32,
            shape_stroke_on: shape.stroke.is_some() && shape.stroke_width > 0.0,
            // A line is drawn in its outline's colour, or its fill's.
            shape_stroke: match (shape.stroke, shape.fill) {
                (Some(stroke), _) => doc_view::to_slint(stroke),
                (None, Some(fill)) if doc_view::is_line(&shape.kind) => doc_view::to_slint(fill),
                _ => slint::Color::from_rgb_u8(255, 255, 255),
            },
            shape_stroke_width: shape.stroke_width as f32,
            shape_stroke_align: match shape.stroke_align {
                model::StrokeAlign::Inside => 1,
                model::StrokeAlign::Outside => 2,
                model::StrokeAlign::Center | model::StrokeAlign::Unknown(_) => 0,
            },
            shape_dash: match shape.dash {
                model::StrokeDash::Dashed => 1,
                model::StrokeDash::Dotted => 2,
                model::StrokeDash::Solid | model::StrokeDash::Unknown(_) => 0,
            },
            shape_start_head: doc_view::LINE_HEADS
                .iter()
                .position(|head| *head == doc_view::start_head(&shape))
                .unwrap_or(0) as i32,
            shape_end_head: doc_view::LINE_HEADS
                .iter()
                .position(|head| *head == doc_view::end_head(&shape))
                .unwrap_or(0) as i32,
            shape_trim_start: shape.trim_start as f32,
            shape_trim_end: shape.trim_end as f32,
            shape_shadow_on: shadow.is_some(),
            shape_shadow_color: shadow.as_ref().map_or(
                slint::Color::from_argb_u8(128, 0, 0, 0),
                |shadow| doc_view::to_slint(shadow.color),
            ),
            shape_shadow_blur: shadow.as_ref().map_or(0.02, |s| s.blur) as f32,
            shape_shadow_distance: shadow.as_ref().map_or(0.012, |s| s.distance) as f32,
            shape_shadow_angle: shadow.as_ref().map_or(90.0, |s| s.angle) as f32,
            enhance_auto: self.enhance_scale <= 0.0,
            enhance_auto_scale: enhance.auto_scale,
            enhance_max_scale: enhance.max_scale,
            enhance_source_size: enhance.source.into(),
            enhance_output_size: enhance.output.into(),
            enhance_size_note: enhance.size_note.into(),
            enhance_strength: self.enhance_strength,
            enhance_previewing: self
                .enhance_previewing
                .as_ref()
                .is_some_and(|(previewing, _, _)| previewing == id),
            enhance_preview_fetch: self
                .enhance_previewing
                .as_ref()
                .filter(|(previewing, fetching, _)| previewing == id && *fetching)
                .map(|(_, _, fraction)| *fraction)
                .unwrap_or(-1.0),
            enhance_preview,
            enhance_before,
            enhance_after,
            enhance_estimate_total: estimate_total.into(),
            enhance_estimate_frames: estimate_frames.into(),
            enhance_estimate_per_frame: estimate_per_frame.into(),
        }
    }

    /// What the Enhance section shows for `clip`, under `id`, beyond its
    /// switch: the sizes its file can take, the preview made for it at the
    /// section's settings, if there is one, and what a whole run would cost.
    pub(super) fn enhance_readout(&self, id: &str, clip: &Item) -> EnhanceReadout {
        let mut readout = EnhanceReadout::default();
        let Some((media_id, media)) = self.enhance_source(clip) else {
            return readout;
        };
        let (width, height) = media
            .size
            .map_or((0, 0), |size| (size.width.get(), size.height.get()));
        let scale = self.enhance_scale_of(media);
        // The row shows what will happen: an ask this file cannot take as
        // the scale it is held to.
        readout.scale = scale;
        readout.auto_scale = concat_vision::enhance::auto_scale(width, height);
        readout.max_scale = concat_vision::enhance::max_scale(width, height);
        if width > 0 && height > 0 {
            let (ow, oh) = concat_vision::enhance::size_at(width, height, scale);
            readout.source = format!("{width} × {height}");
            readout.output = format!("{ow} × {oh}");
        }
        readout.size_note = if readout.max_scale <= 1.0 {
            t("studio.enhanceAt2k")
        } else {
            String::new()
        };
        let still = self.kind(clip) == Kind::Image;
        readout.preview = self
            .enhance_preview_for(id, &media_id.to_string(), scale)
            .map(|preview| {
                // The model's work on what a run covers - the span the clip
                // shows and a second either side, as `enhance_clip` cuts it -
                // at the warm tile time the preview measured.
                let frames = if still {
                    1.0
                } else {
                    let span = doc_view::len_s(clip) * doc_view::speed(clip).abs()
                        + 2.0 * super::ENHANCE_MARGIN;
                    let length = media.length.map_or(f64::MAX, doc_view::len_secs);
                    let rate = media.rate.map_or(30.0, |rate| rate.display_per_second());
                    span.min(length) * rate
                };
                let per_frame = preview.tile_seconds * preview.tiles_per_frame as f64;
                let total = concat_host::enhance::estimate_seconds(
                    preview.tile_seconds,
                    preview.tiles_per_frame,
                    frames,
                );
                // The time, and for a video what it is made of: the frames
                // and each one's time. A still is one frame, said once.
                let estimate = if still {
                    (crate::format::span(total), String::new(), String::new())
                } else {
                    (
                        crate::format::span(total),
                        format!("{:.0}", frames.max(1.0)),
                        format!("{per_frame:.1} s"),
                    )
                };
                (preview.before.clone(), preview.after.clone(), estimate)
            });
        readout
    }

    /// The source second of `clip` under the playhead, held to the clip.
    pub(super) fn source_at_playhead(&self, clip: &Item) -> f64 {
        let at = doc_view::content_at(clip, doc_view::tick(f64::from(self.playhead)));
        doc_view::time_map(clip)
            .and_then(|map| doc_view::source_at(map, at))
            .map_or_else(|| doc_view::in_point_s(clip), doc_view::secs)
    }

    /// Whether the cutout model found nothing at the playhead's frame of
    /// `clip`, so the picture is showing as shot. Only an automatic cutout
    /// says so: a custom one is whatever was painted.
    fn cutout_empty(&self, clip: &Item) -> bool {
        let Some(cutout) = clip.picture.cutout.as_ref() else {
            return false;
        };
        if cutout.mode != model::CutoutMode::Auto {
            return false;
        }
        let (Some(session), Some(media)) =
            (self.session.as_ref(), doc_view::media_of(self.doc(), clip))
        else {
            return false;
        };
        let project = std::path::Path::new(session.path());
        let store = concat_vision::MaskStore::open(&concat_vision::mask_dir(
            project,
            &media.path,
            &cutout.subject,
        ));
        store
            .mask_at(self.source_at_playhead(clip))
            .is_some_and(|mask| mask.is_blank())
    }

    /// Starts reading the first smart stroke whose region is not there
    /// yet, unless one is being read. Called after every change, and by
    /// the finished job for whatever is next.
    pub fn ensure_regions(&mut self) {
        if self.region_job.is_some() || self.host.brushes.is_busy() {
            return;
        }
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let project = std::path::PathBuf::from(session.path());
        let mut next: Option<(String, RegionRequest)> = None;
        let mut tripped = Vec::new();
        'clips: for (_, clip) in doc_view::items(self.doc()) {
            let Some(cutout) = clip.picture.cutout.as_ref() else {
                continue;
            };
            if cutout.mode != model::CutoutMode::Custom || !self.kind(clip).is_visual() {
                continue;
            }
            let (Some(media_id), Some(media)) =
                (doc_view::media_id(clip), doc_view::media_of(self.doc(), clip))
            else {
                continue;
            };
            // The source the clip shows, as the mask analysis reckons it.
            let Some((from, span)) = doc_view::covered(clip) else {
                continue;
            };
            let (a, b) = (doc_view::secs(from), doc_view::secs(from + span));
            let range = (a.min(b), a.max(b));
            for stroke in &cutout.strokes {
                let request = RegionRequest {
                    project: project.clone(),
                    media_path: media.path.clone(),
                    media_size: media
                        .size
                        .map_or((0, 0), |size| (size.width.get(), size.height.get())),
                    still: media.kind == model::MediaKind::Image,
                    subject: cutout.subject.clone(),
                    ranges: vec![range],
                    stroke: stroke.clone(),
                };
                if !request.outstanding() {
                    continue;
                }
                // The app died reading this one last time: reading it
                // again by itself closed the app on every open, with the
                // project shut inside. Painting the stroke again reads it.
                if request.tripped() {
                    tripped.push(request.dir());
                    continue;
                }
                next = Some((
                    Self::analysis_key(&media_id.to_string(), &cutout.subject),
                    request,
                ));
                break 'clips;
            }
        }
        for dir in tripped {
            if self.tripped_strokes.insert(dir) {
                log::warn!(
                    "cutout: skipped a Smart brush stroke: Concat closed while reading it last time, and would again; painting it again reads it"
                );
                self.notify(&t("studio.smartBrushSkipped"), true);
            }
        }
        let Some((key, request)) = next else {
            return;
        };
        self.region_job = Some(key.clone());
        self.cutout_jobs.entry(key.clone()).or_insert((false, 0.0));
        let brushes = Arc::clone(&self.host.brushes);
        let epoch = crate::host::project_epoch();
        spawn_long_in_project(
            move || {
                let reporting = key.clone();
                let result = brushes.read(&request, &mut |progress| {
                    let now = match progress {
                        concat_host::cutout::Progress::Fetching { received, total } => {
                            (true, received as f32 / total.max(1) as f32)
                        }
                        concat_host::cutout::Progress::Analysing(fraction) => (false, fraction),
                    };
                    let key = reporting.clone();
                    on_ui_in_project(epoch, move |studio, _, _| {
                        if let Some(held) = studio.cutout_jobs.get_mut(&key) {
                            *held = now;
                        }
                    });
                });
                (key, result)
            },
            |studio, _, _, (key, result)| {
                studio.region_job = None;
                studio.cutout_jobs.remove(&key);
                studio.pending_stroke = None;
                match result {
                    Ok(()) => {
                        studio.request_preview();
                        studio.ensure_regions();
                    }
                    Err(Halt::Cancelled) => {}
                    Err(Halt::Failed(error)) => {
                        studio.notify(&tf("studio.smartBrush", &[&error]), true);
                    }
                }
            },
        );
    }

    /// The menus, the dialogs, the bin and the engine lists.
    pub fn publish_chrome(&self, app: &App, models: &Models) {
        let editor = app.global::<Editor>();

        // The language, when it changed: one property the whole tree
        // reads, written only when it differs so nothing re-evaluates
        // for nothing.
        let words = app.global::<I18n>();
        let lang = SharedString::from(i18n::current());
        if words.get_lang() != lang {
            words.set_lang(lang);
        }

        // The catalogue's shelves. The groups are built in and never change;
        // the entries are whatever each library's search, shelf and star let
        // through, so they are rebuilt here and `sync` makes an unchanged
        // one a no-op.
        let starred = &self.prefs.favourites;
        let stamp = ShelfStamp {
            catalogue: std::ptr::from_ref(Catalogue::builtin()) as usize,
            cards: self.cards_drawn,
            lang: i18n::current(),
            views: self
                .library
                .iter()
                .map(|view| {
                    (
                        view.query.clone(),
                        view.group,
                        view.favourites,
                        view.category.clone(),
                    )
                })
                .collect(),
            favourites: starred.clone(),
        };
        if self.shelf_stamp.borrow().as_ref() != Some(&stamp) {
            // The looks made from tables are the Adjustments page's, not
            // the Filters page's.
            let (groups, entries) = shelves(
                SHELF_KINDS[0],
                &self.library[0],
                starred,
                &self.look_art,
                &self.card_paths,
                |package| !is_lut(package),
            );
            sync(&models.filter_groups, groups);
            sync(&models.catalogue_filters, entries);
            // Every one of them, on every shelf: the user's own tables
            // are one short list, browsed whole.
            let every = LibraryView {
                group: -1,
                ..LibraryView::default()
            };
            let (_, entries) = shelves(
                SHELF_KINDS[0],
                &every,
                starred,
                &self.look_art,
                &self.card_paths,
                is_lut,
            );
            sync(&models.catalogue_luts, entries);
            app.global::<Editor>().set_adjustment_art(
                Catalogue::builtin()
                    .get(ADJUST_ID)
                    .map(|package| package_art(package, &self.look_art, &self.card_paths))
                    .unwrap_or_default(),
            );
            let (groups, entries) = shelves(
                SHELF_KINDS[1],
                &self.library[1],
                starred,
                &self.look_art,
                &self.card_paths,
                |_| true,
            );
            sync(&models.effect_groups, groups);
            sync(&models.catalogue_effects, entries);
            let (groups, entries) = shelves(
                SHELF_KINDS[2],
                &self.library[2],
                starred,
                &self.look_art,
                &self.card_paths,
                |_| true,
            );
            sync(&models.audio_groups, groups);
            sync(&models.catalogue_audio, entries);
            let (_, entries) = shelves(
                SHELF_KINDS[2],
                &self.library[2],
                starred,
                &self.look_art,
                &self.card_paths,
                |package| !TREATMENTS.contains(&package.manifest.effect.id.as_str()),
            );
            sync(&models.catalogue_audio_library, entries);
            let (groups, entries) = shelves(
                SHELF_KINDS[3],
                &self.library[3],
                starred,
                &self.look_art,
                &self.card_paths,
                |_| true,
            );
            sync(&models.transition_groups, groups);
            sync(&models.catalogue_transitions, entries);
            *self.shelf_stamp.borrow_mut() = Some(stamp);
        }
        sync(
            &models.library_views,
            self.library
                .iter()
                .map(|view| LibraryViewData {
                    query: view.query.as_str().into(),
                    group: view.group,
                    favourites: view.favourites,
                    category: view.category.as_str().into(),
                })
                .collect(),
        );

        app.set_on_start(self.on_start);
        app.set_project_name(self.project_name.as_str().into());
        self.publish_status(app);
        set_changed(app.get_start(), self.start.data(), same_start, |data| {
            app.set_start(data)
        });
        app.set_recents_total(self.recents.len() as i32);
        sync(
            &models.recents,
            crate::panes::start::shown(&self.recents, &self.start.query, self.start.sort)
                .into_iter()
                .map(|recent| {
                    let project = &recent.info;
                    RecentProjectData {
                        path: project.path.as_str().into(),
                        name: project.name.as_str().into(),
                        frame: format!("{} × {}", project.width, project.height).into(),
                        // When it was last worked on, which is what a
                        // person means by recent; when it was last opened
                        // where the file keeps no time.
                        when: when_phrase(recent.edited_at.unwrap_or(project.opened_at)).into(),
                        length: self
                            .lengths
                            .get(&project.path)
                            .copied()
                            .flatten()
                            .filter(|seconds| *seconds > 0.0)
                            .map(running_time)
                            .unwrap_or_default()
                            .into(),
                        ratio: project.width as f32 / project.height.max(1) as f32,
                        missing: !recent.present,
                        poster: self.posters.get(&project.path).cloned().unwrap_or_default(),
                    }
                })
                .collect(),
        );

        // The Text page's presets: the look each card draws its name in.
        // All of them for the phone's one shelf; on the desktop, Styles
        // has the Latin ones and Languages, under it, the rest.
        let cards: Vec<TextPresetData> = self.text_presets.iter().map(preset_card).collect();
        let latin = |card: &TextPresetData| card.language.eq_ignore_ascii_case("latin");
        sync(
            &models.style_presets,
            cards.iter().filter(|card| latin(card)).cloned().collect(),
        );
        sync(
            &models.language_presets,
            cards.iter().filter(|card| !latin(card)).cloned().collect(),
        );
        sync(&models.text_presets, cards);

        // The bin.
        // The Media shelves count the imports; what the editor made is
        // counted on its own shelf under Generated. See MediaBin::shows.
        let items: Vec<&model::Media> = doc_view::bin(self.doc()).map(|(_, item)| item).collect();
        sync(&models.media, self.media.rows(self));
        let imported = |kind: Option<model::MediaKind>| {
            items
                .iter()
                .filter(|item| {
                    item.origin.is_none() && kind.as_ref().is_none_or(|kind| item.kind == *kind)
                })
                .count() as i32
        };
        editor.set_media_count_all(imported(None));
        editor.set_media_count_video(imported(Some(model::MediaKind::Video)));
        editor.set_media_count_audio(imported(Some(model::MediaKind::Audio)));
        editor.set_media_count_images(imported(Some(model::MediaKind::Image)));
        editor.set_media_count_speech(
            items
                .iter()
                .filter(|item| item.origin == Some(model::MediaOrigin::Speech))
                .count() as i32,
        );
        editor.set_media_count_processed(
            items
                .iter()
                .filter(|item| item.origin == Some(model::MediaOrigin::Processed))
                .count() as i32,
        );
        editor.set_media_selected_count(self.media.selected.len() as i32);
        editor.set_media_filter(self.media.filter);
        editor.set_media_jump_token(self.media_jump);
        editor.set_captions_jump_token(self.captions_jump);
        editor.set_importing(false);

        let (width, height) = self.output_size();
        editor.set_output_width(width as i32);
        editor.set_output_height(height as i32);
        editor.set_ratio_index(
            OUTPUTS
                .iter()
                .position(|size| *size == (width as i32, height as i32))
                .map_or(-1, |index| index as i32),
        );
        editor.set_quality_index(self.quality_of() as i32);

        // The Details panel, and the sheet its Modify button opens. The
        // frame, rate and duration are the active timeline's, and the panel
        // says so by name; the name and folder are the project's.
        let folder: SharedString = self
            .session
            .as_ref()
            .map(|session| session.path().to_owned())
            .unwrap_or_else(|| "—".to_owned())
            .into();
        let timeline_name: SharedString = doc_view::sequence(self.doc())
            .map(|sequence| sequence.name.as_str())
            .unwrap_or_default()
            .into();
        editor.set_project_name(self.project_name.as_str().into());
        editor.set_project_folder(folder.clone());
        editor.set_timeline_name(timeline_name.clone());
        editor.set_project_output(format!("{width} × {height}").into());
        editor.set_project_rate(format!("{:.2} fps", self.frame_rate()).into());
        editor.set_project_duration(frames_timecode(self.duration(), self.frame_rate()).into());
        editor.set_count_media(self.doc().media.len() as i32);
        editor.set_count_tracks(doc_view::lanes(self.doc()).len() as i32);
        editor.set_count_clips(doc_view::items(self.doc()).len() as i32);
        // What the file holds and this build cannot show: a row only while
        // there is any, its whole sentence behind it.
        let notes = self
            .session
            .as_ref()
            .map(|session| session.notes())
            .unwrap_or_default();
        editor.set_unread_count(super::project_files::unread_count(notes).into());
        editor.set_unread_notice(
            super::project_files::unread_notice(notes)
                .unwrap_or_default()
                .into(),
        );
        app.set_project_sheet(self.project_sheet.data(self));

        let rows = self.menu();
        editor.set_menu_height(Self::menu_height(&rows));
        sync(&models.menu, rows);
        editor.set_menu_token(self.menu_token);

        set_changed(
            app.get_export(),
            self.export.data(self),
            same_export,
            |data| app.set_export(data),
        );
        set_changed(
            app.get_settings(),
            self.settings.data(self),
            same_settings,
            |data| app.set_settings(data),
        );
        sync(&models.transcribers, self.settings.transcriber_rows());
        sync(&models.voices, self.settings.voice_rows());
        sync(&models.versions, self.settings.version_rows());
        sync(&models.version_details, self.settings.version_details());
        set_changed(
            app.get_relink(),
            self.relink.data(self),
            same_relink,
            |data| app.set_relink(data),
        );
        app.set_confirm(self.confirm_data());

        // The speech sheets, and the lists they choose from.
        let transcribers = installed(&self.settings.transcribers);
        sync(
            &models.caption_models,
            transcribers
                .iter()
                .map(|model| SharedString::from(model.name.as_str()))
                .collect(),
        );
        sync(&models.caption_languages, self.captions.language_rows(self));
        let captions = self.captions.data(self);
        // The captions' style picker: the presets its search lets through,
        // by name or by the writing they are for, and the one picked.
        let query = self.captions.style_query.trim().to_lowercase();
        sync(
            &models.caption_styles,
            self.text_presets
                .iter()
                .filter(|preset| {
                    query.is_empty()
                        || preset.name.to_lowercase().contains(&query)
                        || preset.language.to_lowercase().contains(&query)
                })
                .map(preset_card)
                .collect(),
        );
        let picked = crate::panes::captions::caption_style_id(self);
        if let Some(preset) = self.text_presets.iter().find(|preset| preset.id == picked) {
            let styles = app.global::<crate::ui::CaptionStyles>();
            if styles.get_current().id != preset.id.as_str() {
                styles.set_current(preset_card(preset));
            }
        }
        let editor = app.global::<Editor>();
        set_changed(
            editor.get_captions_sheet(),
            captions.clone(),
            same_captions,
            |data| editor.set_captions_sheet(data),
        );
        set_changed(app.get_captions(), captions, same_captions, |data| {
            app.set_captions(data)
        });
        let voices = installed(&self.settings.voices);
        sync(
            &models.speech_models,
            voices
                .iter()
                .map(|model| SharedString::from(model.name.as_str()))
                .collect(),
        );
        sync(
            &models.speech_model_details,
            self.speech.model_detail_rows(self),
        );
        sync(&models.speakers, self.speech.speaker_rows());
        sync(&models.speaker_details, self.speech.speaker_detail_rows());
        sync(&models.speaker_avatars, self.speech.speaker_avatar_rows());
        sync(&models.speech_samples, self.speech.sample_rows(self));
        sync(
            &models.speech_sample_waves,
            self.speech.sample_wave_rows(self),
        );
        sync(
            &models.speech_sample_details,
            self.speech.sample_detail_rows(self),
        );
        app.global::<crate::ui::Speech>()
            .set_data(self.speech.data(self));
        app.set_new_voice(self.speech.new_voice_data());
        // The Audio page.
        sync(&models.sounds_mine, self.sounds.mine());
        sync(&models.sounds_concat, self.sounds.concat_rows());
        {
            use crate::panes::sounds::ConcatState;
            let page = app.global::<crate::ui::Sounds>();
            page.set_concat_state(match self.sounds.concat {
                ConcatState::Idle => 0,
                ConcatState::Loading => 1,
                ConcatState::Ready => 2,
                ConcatState::Soon => 3,
                ConcatState::Offline => 4,
            });
            page.set_concat_category(self.sounds.category as i32);
        }
        app.global::<crate::ui::Sounds>()
            .set_importing(self.sounds.importing);
        let (saved, builtin, styles, ready) = self.voice_cards();
        sync(&models.voices_saved, saved);
        sync(&models.voices_builtin, builtin);
        sync(&models.voice_styles, styles);
        app.global::<crate::ui::Sounds>().set_voices_ready(ready);
        self.publish_source(app);
        sync(&models.voiceover_devices, self.voiceover.device_rows());
        app.set_voiceover(self.voiceover.data(self));
        editor.set_recording(self.voiceover.recording());
        editor.set_captions_open(self.captions.open || self.captions.shown);
        editor.set_speech_open(self.speech.open);
        editor.set_voiceover_open(self.voiceover.is_open());

        let bar = self.menu_bar();
        app.set_app_menu_height(Self::menu_height(&bar));
        sync(&models.bar, bar);
        app.set_app_menu_token(self.menu_bar_token);
        app.set_open_menu(self.open_menu);
    }

    /// Asks the workers for anything the launch screen or the bin is
    /// missing. Separate from `publish` because it mutates.
    pub fn refresh_art(&mut self) {
        // Called after every callback, the pointer stream included. The
        // scan over every media item - stats for proxies, JPEGs read back,
        // decodes queued - runs when the document, the screen or what is
        // pending changed, and is one comparison otherwise. The lanes'
        // cell strips follow the view, which moves on that stream, so
        // they are asked for every time; that walk is hash lookups
        // (audit 2026-09-23, #11).
        let stamp = (
            self.revision,
            self.on_start,
            self.art_pending.len() + self.posters_pending.len(),
            // Scrub copies switched on in Settings are seen to at once.
            self.prefs.scrub_copies(),
        );
        if self.art_stamp == Some(stamp) {
            if !self.on_start {
                self.request_window_art();
            }
            return;
        }
        self.art_stamp = Some(stamp);
        if self.on_start {
            self.request_posters();
        } else {
            self.assign_media_rows();
            self.request_media_art();
        }
    }

    /// The right-click menu: the bin card's when it was opened on one,
    /// else the clip's.
    pub(super) fn menu(&self) -> Vec<MenuItemData> {
        if let Some(id) = self.menu_media.as_deref() {
            return self.media_menu(id);
        }
        let Some(id) = self.menu_target.as_deref() else {
            return Vec::new();
        };
        let Some(clip) = self.clip(id) else {
            return Vec::new();
        };
        let kind = self.kind(clip);
        let locked = self.lane_locked(clip.lane);
        let playhead = f64::from(self.playhead);
        let straddled = doc_view::start_s(clip) < playhead && playhead < doc_view::end_s(clip);

        let action =
            |id: &str, label: String, glyph: Glyph, shortcut: &str, enabled: bool| MenuItemData {
                id: id.into(),
                label: label.into(),
                kind: MenuRow::Action,
                glyph,
                shortcut: shortcut.into(),
                enabled,
                danger: false,
                checkable: false,
                checked: false,
            };
        let check = |id: &str, label: &str, shortcut: &str, on: bool, enabled: bool| MenuItemData {
            id: id.into(),
            label: label.into(),
            kind: MenuRow::Action,
            glyph: Glyph::None,
            shortcut: shortcut.into(),
            enabled,
            danger: false,
            checkable: true,
            checked: on,
        };
        let rule = || MenuItemData {
            kind: MenuRow::Separator,
            ..Default::default()
        };

        let mut rows = vec![
            action(
                "copy",
                t("common.copy"),
                Glyph::Copy,
                &keymap::label(&self.prefs, "copy"),
                true,
            ),
            action(
                "duplicate",
                t("studio.duplicate"),
                Glyph::Plus,
                &keymap::label(&self.prefs, "duplicate"),
                !locked,
            ),
            action(
                "paste",
                t("studio.paste"),
                Glyph::Plus,
                &keymap::label(&self.prefs, "paste"),
                !self.clipboard.is_empty(),
            ),
            rule(),
            action(
                "split",
                t("studio.splitAtPlayhead"),
                Glyph::Split,
                &keymap::label(&self.prefs, "split"),
                straddled && !locked,
            ),
            // Delete Left and Right, here as on the keys; on a
            // phone, whose More is this menu, the only way to them.
            action(
                "delete-left",
                t("shortcuts.deleteLeft"),
                Glyph::None,
                &keymap::label(&self.prefs, "delete-left"),
                straddled && !locked,
            ),
            action(
                "delete-right",
                t("shortcuts.deleteRight"),
                Glyph::None,
                &keymap::label(&self.prefs, "delete-right"),
                straddled && !locked,
            ),
            action(
                "merge",
                self.merge_label(),
                Glyph::Merge,
                "",
                self.merge_blocked().is_none(),
            ),
            action(
                "freeze",
                t("studio.freezeFrame"),
                Glyph::Frame,
                &keymap::label(&self.prefs, "freeze"),
                straddled && !locked && matches!(kind, Kind::Video | Kind::Image),
            ),
            // The picture restored and enlarged by the model: the row opens
            // the clip's Enhance in the inspector, whose switch runs it.
            action(
                "enhance",
                t("common.enhance"),
                Glyph::Sparkle,
                "",
                !locked && matches!(kind, Kind::Video | Kind::Image),
            ),
            rule(),
        ];
        // The picture sized to the frame, whole or covering it: beside the
        // inspector's two buttons, where a vertical cut is assembled clip
        // by clip.
        if kind.is_visual() {
            // A picture can be sized to the frame when its size is known: a
            // file's, or a title's or a shape's own.
            let frameable = !locked
                && doc_view::media_of(self.doc(), clip).is_none_or(|media| media.size.is_some());
            rows.push(action(
                "fit",
                t("studio.fitToFrame"),
                Glyph::Fit,
                "",
                frameable,
            ));
            rows.push(action(
                "fill",
                t("studio.fillFrame"),
                Glyph::Maximize,
                "",
                frameable,
            ));
            rows.push(rule());
        }
        // One slot, two verbs: the sound is either on its picture or off it.
        // Shown on every video clip, so the verb is where a person looks for
        // it, and greyed rather than gone when the file has no sound.
        let (can_detach, can_reattach) = self.sound_placement(id, clip);
        if can_reattach {
            rows.push(action(
                "reattach",
                t("studio.reattachAudio"),
                Glyph::Merge,
                "",
                !locked,
            ));
            rows.push(rule());
        } else if kind == Kind::Video {
            rows.push(action(
                "detach",
                t("studio.detachAudio"),
                Glyph::Waveform,
                "",
                !locked && can_detach && self.clip_has_sound(clip),
            ));
            rows.push(rule());
        }
        // The clip's sound, written out as it plays, as a file of its own
        // in the bin. For anything with sound in it; a clip whose track is
        // muted still renders, since the file is the clip's and not the
        // mix's.
        if kind.is_audible() {
            rows.push(action(
                "export-audio",
                t("studio.exportAudio"),
                Glyph::Waveform,
                "",
                self.clip_has_sound(clip),
            ));
            // What is said in it, as titles on the timeline: the captions
            // sheet, opened on this clip. Here so the transcriber is found
            // where the sound is, not only on the tray (#302). The captions
            // land on a text lane above, not on this clip's track, so a
            // locked track does not grey it.
            rows.push(action(
                "captions",
                t("captions.generateCaptions"),
                Glyph::TextMark,
                "",
                self.clip_has_sound(clip),
            ));
            rows.push(rule());
            // Voice isolation, both ways, one click each: ticked when it is
            // what the clip plays, and chosen again to play it as recorded
            // (#120).
            let voiced = self.clip_has_sound(clip) && !locked;
            let mix = clip.sound.voice.as_ref();
            rows.push(check(
                "voice-keep",
                &t("voice.keepOnly"),
                "",
                mix.is_some_and(|mix| mix.voice > 0.0 && mix.background == 0.0),
                voiced,
            ));
            rows.push(check(
                "voice-remove",
                &t("voice.remove"),
                "",
                mix.is_some_and(|mix| mix.voice == 0.0),
                voiced,
            ));
            rows.push(rule());
        }
        let audible = kind != Kind::Image;
        rows.push(check(
            "mute",
            &t("studio.mute"),
            &keymap::label(&self.prefs, "mute"),
            clip.sound.volume.value <= 0.0,
            !locked && audible,
        ));
        rows.push(check("lock", &t("studio.lockTrack"), "", locked, true));
        rows.push(rule());
        rows.push(MenuItemData {
            id: "delete".into(),
            label: t("common.delete").into(),
            kind: MenuRow::Action,
            glyph: Glyph::Trash,
            shortcut: keymap::label(&self.prefs, "delete").into(),
            enabled: !locked,
            danger: true,
            checkable: false,
            checked: false,
        });
        // One Delete, not a Delete and a Ripple Delete: whether a delete
        // closes the gap is the tray's magnetic switch, a setting of the
        // timeline rather than a choice to make at every clip.
        rows
    }

    /// The right-click menu for a card in the bin: the file's verbs, and
    /// the one attribute a file has that every clip cut from it inherits.
    ///
    /// A picture's colour range is the media's rather than any clip's -
    /// the fix for a file that lies about its levels is a fact about the
    /// file - so it is set here, off the card, the way Resolve keeps Clip
    /// Attributes off the media pool, and not in a clip's inspector where
    /// it read as a property of the cut. Auto is the file's tag, and the
    /// codec's convention where there is none; see
    /// `concat_media::ColorRange::implied`.
    /// https://github.com/jub0t/Concat/issues/103
    fn media_menu(&self, id: &str) -> Vec<MenuItemData> {
        let Some(item) = doc_view::parse::<MediaId>(id).and_then(|id| doc_view::media(self.doc(), id))
        else {
            return Vec::new();
        };
        let action = |id: &str, label: String, glyph: Glyph, danger: bool| MenuItemData {
            id: id.into(),
            label: label.into(),
            kind: MenuRow::Action,
            glyph,
            shortcut: "".into(),
            enabled: true,
            danger,
            checkable: false,
            checked: false,
        };
        // The levels go where a shortcut would: the number is what the
        // word means, and a person who knows one knows the other.
        let check = |id: &str, label: String, levels: &str, on: bool| MenuItemData {
            id: id.into(),
            label: label.into(),
            kind: MenuRow::Action,
            glyph: Glyph::None,
            shortcut: levels.into(),
            enabled: true,
            danger: false,
            checkable: true,
            checked: on,
        };
        let rule = || MenuItemData {
            kind: MenuRow::Separator,
            ..Default::default()
        };
        let mut rows = vec![
            action("add", t("studio.addAtPlayhead"), Glyph::Plus, false),
            rule(),
        ];
        if item.kind != model::MediaKind::Audio {
            let range = item.range.as_ref();
            rows.push(MenuItemData {
                label: t("common.colourRange").to_uppercase().into(),
                kind: MenuRow::Label,
                ..Default::default()
            });
            rows.push(check("range-auto", t("common.auto"), "", range.is_none()));
            rows.push(check(
                "range-limited",
                t("common.limited"),
                "16-235",
                range == Some(&model::ColorRange::Limited),
            ));
            rows.push(check(
                "range-full",
                t("common.full"),
                "0-255",
                range == Some(&model::ColorRange::Full),
            ));
            rows.push(rule());
        }
        rows.push(action("remove", t("common.remove"), Glyph::Trash, true));
        rows
    }

    /// A row of a bin card's menu, for the media it was opened on.
    pub fn media_action(&mut self, id: &str, action: &str) {
        log::debug!("media menu: {action} on {id}");
        let _asked = Asked::by("media menu");
        let Some(row) = self.media.row_of(id) else {
            return;
        };
        let Some(media) = doc_view::parse::<MediaId>(id)
            .filter(|media| doc_view::media(self.doc(), *media).is_some())
        else {
            return;
        };
        let range = |range| Command::SetMediaColorRange { media, range };
        match action {
            "add" => self.place_at_playhead(&format!("media:{row}")),
            "remove" => self.handle(crate::panes::Msg::Media(
                crate::panes::media_bin::MediaMsg::Remove(row),
            )),
            "range-auto" => {
                self.apply(range(None));
            }
            "range-limited" => {
                self.apply(range(Some(model::ColorRange::Limited)));
            }
            "range-full" => {
                self.apply(range(Some(model::ColorRange::Full)));
            }
            _ => {}
        }
    }

    pub fn menu_height(rows: &[MenuItemData]) -> f32 {
        let metrics = |kind: MenuRow| match kind {
            MenuRow::Action => 26.0,
            MenuRow::Label => 24.0,
            MenuRow::Separator => 9.0,
        };
        rows.iter().map(|row| metrics(row.kind)).sum::<f32>() + 12.0
    }

    fn menu_bar(&self) -> Vec<MenuItemData> {
        let row =
            |id: &str, label: String, glyph: Glyph, shortcut: &str, enabled: bool| MenuItemData {
                id: id.into(),
                label: label.into(),
                kind: MenuRow::Action,
                glyph,
                shortcut: shortcut.into(),
                enabled,
                danger: false,
                checkable: false,
                checked: false,
            };
        let rule = || MenuItemData {
            kind: MenuRow::Separator,
            ..Default::default()
        };
        let check = |id: &str, label: &str, on: bool| MenuItemData {
            id: id.into(),
            label: label.into(),
            kind: MenuRow::Action,
            glyph: Glyph::None,
            shortcut: "".into(),
            enabled: true,
            danger: false,
            checkable: true,
            checked: on,
        };
        let selected = self.selection.len();
        let playhead = f64::from(self.playhead);
        let straddled = doc_view::items(self.doc()).iter().any(|(_, clip)| {
            doc_view::start_s(clip) + f64::from(MIN_DURATION) < playhead
                && playhead < doc_view::end_s(clip) - f64::from(MIN_DURATION)
        });
        let (can_undo, can_redo) = self.session.as_ref().map_or((false, false), |session| {
            (session.can_undo(), session.can_redo())
        });
        let has_selection_media = !self.media.selected.is_empty();

        match self.open_menu {
            0 => vec![
                row(
                    "add-selected",
                    t("studio.addSelectedToTimeline"),
                    Glyph::Plus,
                    "",
                    has_selection_media,
                ),
                row(
                    "open",
                    t("studio.openProject"),
                    Glyph::Import,
                    &platform::keys(&["Control", "O"]),
                    true,
                ),
                row(
                    "import",
                    t("studio.importMedia"),
                    Glyph::Import,
                    &keymap::label(&self.prefs, "import"),
                    true,
                ),
                row(
                    "save",
                    t("studio.save"),
                    Glyph::Import,
                    &keymap::label(&self.prefs, "save"),
                    true,
                ),
                row(
                    "export",
                    t("studio.export"),
                    Glyph::Export,
                    &keymap::label(&self.prefs, "export"),
                    !doc_view::items(self.doc()).is_empty(),
                ),
                row(
                    "template",
                    t("studio.saveAsTemplate"),
                    Glyph::Slot,
                    "",
                    true,
                ),
                row("speech", t("studio.textToSpeech"), Glyph::Volume, "", true),
                row(
                    "clear-cache",
                    t("studio.clearProjectCache"),
                    Glyph::None,
                    "",
                    true,
                ),
                rule(),
                row(
                    "settings",
                    t("studio.settings"),
                    Glyph::Settings,
                    &keymap::label(&self.prefs, "settings"),
                    true,
                ),
                rule(),
                // The last row is each platform's own word for leaving:
                // Close Window on a Mac, where ⌘W is it; Exit on Windows,
                // where Alt+F4 is; Quit on Linux, with Ctrl+Q. Off the Mac
                // Ctrl+W closes the document instead, which is the project.
                row(
                    "close-project",
                    t("studio.closeProject"),
                    Glyph::Import,
                    &if platform::MACOS {
                        String::new()
                    } else {
                        platform::keys(&["Control", "W"])
                    },
                    true,
                ),
                MenuItemData {
                    id: "close-window".into(),
                    label: if platform::MACOS {
                        t("studio.closeWindow")
                    } else if cfg!(windows) {
                        t("studio.exit")
                    } else {
                        t("studio.quit")
                    }
                    .into(),
                    kind: MenuRow::Action,
                    glyph: Glyph::Close,
                    shortcut: if platform::MACOS {
                        platform::keys(&["Control", "W"])
                    } else if cfg!(windows) {
                        platform::keys(&["Alt", "F4"])
                    } else {
                        platform::keys(&["Control", "Q"])
                    }
                    .into(),
                    enabled: true,
                    danger: true,
                    checkable: false,
                    checked: false,
                },
            ],
            1 => vec![
                row(
                    "undo",
                    t("studio.undo"),
                    Glyph::Undo,
                    &keymap::label(&self.prefs, "undo"),
                    can_undo,
                ),
                row(
                    "redo",
                    t("studio.redo"),
                    Glyph::Redo,
                    &keymap::label(&self.prefs, "redo"),
                    can_redo,
                ),
                rule(),
                row(
                    "split",
                    t("studio.splitAtPlayhead"),
                    Glyph::Razor,
                    &keymap::label(&self.prefs, "split"),
                    straddled,
                ),
                MenuItemData {
                    id: "delete".into(),
                    label: if selected > 1 {
                        tf("studio.deleteClips", &[&selected])
                    } else {
                        t("studio.deleteClip")
                    }
                    .into(),
                    kind: MenuRow::Action,
                    glyph: Glyph::Trash,
                    shortcut: keymap::label(&self.prefs, "delete").into(),
                    enabled: selected > 0,
                    danger: true,
                    checkable: false,
                    checked: false,
                },
                rule(),
                MenuItemData {
                    id: "snap".into(),
                    label: t("common.snapToEdges").into(),
                    kind: MenuRow::Action,
                    glyph: Glyph::None,
                    shortcut: keymap::label(&self.prefs, "snap").into(),
                    enabled: true,
                    danger: false,
                    checkable: true,
                    checked: self.lanes.snap,
                },
                MenuItemData {
                    id: "magnetic".into(),
                    label: t("common.magneticTimeline").into(),
                    kind: MenuRow::Action,
                    glyph: Glyph::None,
                    shortcut: "".into(),
                    enabled: true,
                    danger: false,
                    checkable: true,
                    checked: self.prefs.magnetic(),
                },
                MenuItemData {
                    id: "linkage".into(),
                    label: t("common.linkage").into(),
                    kind: MenuRow::Action,
                    glyph: Glyph::None,
                    shortcut: "".into(),
                    enabled: true,
                    danger: false,
                    checkable: true,
                    checked: doc_view::sequence(self.doc()).is_some_and(|sequence| sequence.linkage),
                },
            ],
            2 => vec![
                row(
                    "zoom-in",
                    t("common.zoomIn"),
                    Glyph::Plus,
                    &keymap::label(&self.prefs, "zoom-in"),
                    true,
                ),
                row(
                    "zoom-out",
                    t("common.zoomOut"),
                    Glyph::Minus,
                    &keymap::label(&self.prefs, "zoom-out"),
                    true,
                ),
                rule(),
                check("sort-added", &t("studio.sortAdded"), self.media.sort == 0),
                check("sort-name", &t("studio.sortName"), self.media.sort == 1),
                check("sort-kind", &t("studio.sortType"), self.media.sort == 2),
                rule(),
                row(
                    "start",
                    t("common.goToStart"),
                    Glyph::SkipBack,
                    &keymap::label(&self.prefs, "start"),
                    true,
                ),
                row(
                    "end",
                    t("common.goToEnd"),
                    Glyph::SkipForward,
                    &keymap::label(&self.prefs, "end"),
                    true,
                ),
                row(
                    "prev-edit",
                    t("shortcuts.prevEdit"),
                    Glyph::None,
                    &keymap::label(&self.prefs, "prev-edit"),
                    true,
                ),
                row(
                    "next-edit",
                    t("shortcuts.nextEdit"),
                    Glyph::None,
                    &keymap::label(&self.prefs, "next-edit"),
                    true,
                ),
            ],
            _ => Vec::new(),
        }
    }

    /// The clip's track flags, as one command per flag that changed, plus
    /// the lock, which is the window's.
    pub fn track_flags(&mut self, row: i32, visible: bool, muted: bool, locked: bool) {
        let Some(lane) = self.row_track(row) else {
            return;
        };
        let Some(track) = doc_view::lane(self.doc(), lane).cloned() else {
            return;
        };
        let mut commands = Vec::new();
        if track.visible != visible {
            commands.push(Command::SetTrackFlag {
                lane,
                flag: LaneFlag::Visible,
                value: visible,
            });
        }
        if track.muted != muted {
            commands.push(Command::SetTrackFlag {
                lane,
                flag: LaneFlag::Muted,
                value: muted,
            });
        }
        let view = self.lanes.lane_view.entry(lane.to_string()).or_default();
        view.locked = locked;
        if locked {
            let doomed: Vec<String> = doc_view::on_lane(self.doc(), lane)
                .map(|(id, _)| id.to_string())
                .collect();
            self.selection.retain(|held| !doomed.contains(held));
        }
        match commands.len() {
            0 => {}
            1 => {
                self.apply(commands.remove(0));
            }
            _ => {
                self.apply(Command::Batch { commands });
            }
        }
    }

    pub fn set_lane_size(&mut self, row: i32, size: TrackSize) {
        if let Some(lane) = self.row_track(row) {
            self.lanes.lane_view.entry(lane.to_string()).or_default().size = size;
        }
    }

    /// A quarter turn clockwise on every selected picture, at the playhead:
    /// a keyed rotation gets a key there and a constant one turns whole, as
    /// the stage's own rotate grip writes it. One undo step for the lot.
    pub fn rotate_selected(&mut self) {
        self.flush_commit();
        let playhead = self.playhead;
        let ids: Vec<String> = self
            .selection
            .iter()
            .filter(|id| {
                self.clip(id).is_some_and(|clip| {
                    self.kind(clip) != Kind::Audio && !self.lane_locked(clip.lane)
                })
            })
            .cloned()
            .collect();
        let mut commands = Vec::new();
        for id in ids {
            let (Some(before), Some(item)) =
                (self.clip(&id).cloned(), doc_view::parse::<ItemId>(&id))
            else {
                continue;
            };
            let mut after = before.clone();
            let at = doc_view::content_at(&after, doc_view::tick(f64::from(playhead)));
            // Kept in the inspector's range, wrapping rather than stopping,
            // as the grip does.
            let next = (shown(&after, KeyProperty::Rotation, at) + 90.0 + 180.0)
                .rem_euclid(360.0)
                - 180.0;
            write_keyable(&mut after, KeyProperty::Rotation, next, at);
            if after.picture.rotation.value != before.picture.rotation.value {
                commands.push(Command::SetClipTransform {
                    item,
                    offset_x: None,
                    offset_y: None,
                    rotation: Some(after.picture.rotation.value),
                    scale: None,
                    stretch_x: None,
                    stretch_y: None,
                });
            }
            commands.extend(key_commands(item, &before, &after));
        }
        match commands.len() {
            0 => {}
            1 => {
                self.apply(commands.remove(0));
            }
            _ => {
                self.apply(Command::Batch { commands });
            }
        }
    }

    pub fn toggle_flip_h(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let commands: Vec<Command> = self
            .selection
            .iter()
            .filter_map(|id| {
                let clip = self.clip(id)?;
                Some(Command::UpdateClip {
                    item: doc_view::parse(id)?,
                    patch: ItemPatch {
                        flip_h: Some(!clip.picture.flip_h),
                        ..Default::default()
                    },
                })
            })
            .collect();
        if !commands.is_empty() {
            self.apply(Command::Batch { commands });
        }
    }

    pub fn toggle_lock(&mut self, track_id: &str) {
        let view = self.lanes.lane_view.entry(track_id.to_owned()).or_default();
        view.locked = !view.locked;
        if view.locked
            && let Some(lane) = doc_view::parse::<LaneId>(track_id)
        {
            let doomed: Vec<String> = doc_view::on_lane(self.doc(), lane)
                .map(|(id, _)| id.to_string())
                .collect();
            self.selection.retain(|held| !doomed.contains(held));
        }
    }

    /// Changes the active timeline's output size from the monitor's picker.
    /// An edit like any other - undoable, and this timeline's alone.
    pub fn set_output(&mut self, index: usize) {
        let (width, height) = OUTPUTS[index.min(OUTPUTS.len() - 1)];
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let (Some(sequence), Some(width), Some(height)) = (
            session.showing(),
            std::num::NonZeroU32::new(width as u32),
            std::num::NonZeroU32::new(height as u32),
        ) else {
            return;
        };
        let frame = model::Frame {
            width,
            height,
            ..session.frame()
        };
        self.apply(Command::SetTimelineVideo { sequence, frame });
        self.request_preview();
    }
}

/// A text preset as its card draws it: enough of the look to set its
/// sample in. The Text page's shelves and the captions' style picker.
pub(crate) fn preset_card(preset: &TextPreset) -> TextPresetData {
    let plate = preset
        .style
        .plate
        .map_or(slint::Color::from_argb_u8(0, 0, 0, 0), doc_view::to_slint);
    TextPresetData {
        id: preset.id.as_str().into(),
        name: preset.name.as_str().into(),
        language: preset.language.as_str().into(),
        sample: preset.sample.as_str().into(),
        family: preset.style.font_family.trim_matches('"').into(),
        weight: preset.style.font_weight.round() as i32,
        italic: preset.style.italic,
        fill: doc_view::to_slint(preset.style.color),
        plate,
        plated: plate.alpha() > 0,
        stroke: doc_view::to_slint(preset.style.stroke_color),
        stroke_width: preset.style.stroke_width as f32,
        align: align_of(&preset.style.align),
    }
}

/// Where a move or a loop named `kind` sits in its picker's list; -1 for
/// none or a name the app does not know.
fn motion_place(kind: Option<&str>, looping: bool) -> i32 {
    let Some(kind) = kind else { return -1 };
    let place = if looping {
        concat_core::motion::Cycle::parse(kind).and_then(|found| {
            concat_core::motion::Cycle::ALL
                .iter()
                .position(|c| *c == found)
        })
    } else {
        concat_core::motion::Move::parse(kind).and_then(|found| {
            concat_core::motion::Move::ALL
                .iter()
                .position(|m| *m == found)
        })
    };
    place.map_or(-1, |place| place as i32)
}

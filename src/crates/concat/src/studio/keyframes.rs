// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! Keyframes.

use super::*;

impl Studio {
    // ── keyframes ──

    /// The keyable property a field names, or None for a field that is a
    /// constant and stays one.
    pub fn key_property_of(field: ClipField) -> Option<model::KeyProperty> {
        Some(match field {
            ClipField::Scale => model::KeyProperty::Scale,
            ClipField::OffsetX => model::KeyProperty::OffsetX,
            ClipField::OffsetY => model::KeyProperty::OffsetY,
            ClipField::Rotation => model::KeyProperty::Rotation,
            ClipField::Opacity => model::KeyProperty::Opacity,
            ClipField::Volume => model::KeyProperty::Volume,
            ClipField::MaskX => model::KeyProperty::MaskX,
            ClipField::MaskY => model::KeyProperty::MaskY,
            ClipField::MaskWidth => model::KeyProperty::MaskWidth,
            ClipField::MaskHeight => model::KeyProperty::MaskHeight,
            ClipField::MaskRotation => model::KeyProperty::MaskRotation,
            ClipField::MaskRoundness => model::KeyProperty::MaskRoundness,
            ClipField::MaskFeather => model::KeyProperty::MaskFeather,
            _ => return None,
        })
    }

    /// The selected clip and where the playhead is inside it, `0..=1`, or
    /// None when there is no sole selection or the playhead is outside it.
    ///
    /// Outside is not clamped to an end: a key put on from outside the clip
    /// would land on its first or last frame, which is never what the person
    /// pressing the diamond meant.
    /// The Speed tab's curve for the selected clip: its points on the
    /// graph's scale, the line through them, whether it bends, which preset
    /// it still is, where the playhead crosses it, and its two extremes.
    pub(super) fn publish_speed_ramp(&self, app: &App, models: &Models) {
        let ramp = app.global::<SpeedRamp>();
        if models.speed_presets.row_count() == 0 {
            sync(
                &models.speed_presets,
                speed_ramp::preset_names()
                    .into_iter()
                    .map(SharedString::from)
                    .collect(),
            );
        }
        let clip = self.sole_selection().and_then(|id| self.clip(&id));
        let curve = clip.and_then(|clip| clip.speed_curve.as_deref());
        let points = curve.unwrap_or_default();
        sync(
            &models.speed_points,
            points
                .iter()
                .enumerate()
                .map(|(index, point)| CurvePointData {
                    index: index as i32,
                    x: point.at as f32,
                    y: speed_ramp::height_of(point.speed) as f32,
                })
                .collect(),
        );
        ramp.set_curved(curve.is_some());
        ramp.set_smooth(curve.is_none_or(concat_project::speed::is_smooth));
        ramp.set_path(speed_ramp::path(points).into());
        ramp.set_preset(curve.map_or(-1, speed_ramp::preset_index));
        ramp.set_at(self.key_point().map_or(-1.0, |(_, at)| at as f32));
        ramp.set_fastest(speed_ramp::extreme(points, true).map_or(-1.0, |speed| speed as f32));
        ramp.set_slowest(speed_ramp::extreme(points, false).map_or(-1.0, |speed| speed as f32));
    }

    pub fn key_point(&self) -> Option<(&Clip, f64)> {
        let clip = self.sole_selection().and_then(|id| self.clip(&id))?;
        if clip.duration <= 0.0 {
            return None;
        }
        let local = f64::from(self.playhead) - clip.start;
        (0.0..=clip.duration)
            .contains(&local)
            .then(|| (clip, local / clip.duration))
    }

    /// The keyframe cluster's six rows, in the order the `Keyframes` global
    /// indexes them. Empty when nothing can be keyed, which is what greys
    /// every cluster in the inspector at once.
    pub fn key_rows(&self) -> Vec<ClipKeyData> {
        let Some((clip, at)) = self.key_point() else {
            return Vec::new();
        };
        model::KeyProperty::ALL
            .iter()
            .map(|&property| {
                let (prev, next) = clip.keys_around(property, at);
                ClipKeyData {
                    field: key_field_of(property),
                    keyed: clip.is_keyed(property),
                    here: clip.key_at(property, at).is_some(),
                    prev: prev.is_some(),
                    next: next.is_some(),
                }
            })
            .collect()
    }

    /// The Keyframes pane's rows: the six properties of the selected clip,
    /// and every Adjust knob of it that carries keys, each with its keys
    /// along the clip - where, what value as a fraction of the knob's
    /// range, and the ease into it - what the knob is worth at the
    /// playhead, and the ride over the clip as a path for the curve view.
    pub fn key_editor_rows(&self) -> Vec<KeyRowData> {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return Vec::new();
        };
        let at = place_in(clip, self.playhead);
        let near = |a: f64, b: f64| (a - b).abs() <= model::KEY_EPSILON;
        // A run of keys as marks: each with its value as a fraction of the
        // range and the ease into it, and the ride as a path - a hold to
        // the first key, a cubic per segment (the ease's control points are
        // exactly the curve's, in time and value), a hold after the last.
        let marks_and_curve = |keys: &[(f64, f64, model::KeyEase)],
                               to_fraction: &dyn Fn(f64) -> f64|
         -> (ModelRc<KeyMarkData>, String, i32) {
            let marks: Vec<KeyMarkData> = keys
                .iter()
                .enumerate()
                .map(|(index, (key_at, value, ease))| {
                    let [x1, y1, x2, y2] = ease.0;
                    KeyMarkData {
                        at: *key_at as f32,
                        here: near(*key_at, at),
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
            let ease_index = marks
                .iter()
                .position(|mark| mark.eased && f64::from(mark.at) >= at - model::KEY_EPSILON)
                .map_or(-1, |index| index as i32);
            (ModelRc::new(VecModel::from(marks)), curve, ease_index)
        };
        let mut rows = Vec::new();
        for property in model::KeyProperty::ALL {
            // The mask's numbers are rows only while there is a mask, or
            // keys left from one: seven rows of nothing otherwise.
            if property.is_mask() && clip.mask.is_none() && !clip.is_keyed(property) {
                continue;
            }
            // The same ranges and units as the inspector's knobs. The level
            // knob speaks decibels, floored where silence is.
            let (label, minimum, maximum, step, default_value, fmt, unit, unit_scale) =
                match property {
                    model::KeyProperty::Scale => (
                        t("common.scale"),
                        0.05,
                        8.0,
                        0.01,
                        1.0,
                        ParamFormat::Fraction,
                        "%",
                        100.0,
                    ),
                    model::KeyProperty::OffsetX => (
                        t("common.positionX"),
                        -1.0,
                        1.0,
                        0.005,
                        0.0,
                        ParamFormat::Centred,
                        "%",
                        100.0,
                    ),
                    model::KeyProperty::OffsetY => (
                        t("common.positionY"),
                        -1.0,
                        1.0,
                        0.005,
                        0.0,
                        ParamFormat::Centred,
                        "%",
                        100.0,
                    ),
                    model::KeyProperty::Rotation => (
                        t("common.rotation"),
                        -180.0,
                        180.0,
                        1.0,
                        0.0,
                        ParamFormat::Degrees,
                        "°",
                        1.0,
                    ),
                    model::KeyProperty::Opacity => (
                        t("common.opacity"),
                        0.0,
                        1.0,
                        0.01,
                        1.0,
                        ParamFormat::Fraction,
                        "%",
                        100.0,
                    ),
                    model::KeyProperty::Volume => (
                        t("common.level"),
                        -60.0,
                        24.0,
                        0.5,
                        0.0,
                        ParamFormat::Gain,
                        "",
                        1.0,
                    ),
                    model::KeyProperty::MaskX => (
                        tf("mask.row", &[&t("common.positionX")]),
                        -1.0,
                        1.0,
                        0.005,
                        0.0,
                        ParamFormat::Centred,
                        "%",
                        100.0,
                    ),
                    model::KeyProperty::MaskY => (
                        tf("mask.row", &[&t("common.positionY")]),
                        -1.0,
                        1.0,
                        0.005,
                        0.0,
                        ParamFormat::Centred,
                        "%",
                        100.0,
                    ),
                    model::KeyProperty::MaskWidth => (
                        tf("mask.row", &[&t("common.width")]),
                        0.01,
                        4.0,
                        0.01,
                        0.66,
                        ParamFormat::Fraction,
                        "%",
                        100.0,
                    ),
                    model::KeyProperty::MaskHeight => (
                        tf("mask.row", &[&t("common.height")]),
                        0.01,
                        4.0,
                        0.01,
                        0.66,
                        ParamFormat::Fraction,
                        "%",
                        100.0,
                    ),
                    model::KeyProperty::MaskRotation => (
                        tf("mask.row", &[&t("common.rotation")]),
                        -180.0,
                        180.0,
                        1.0,
                        0.0,
                        ParamFormat::Degrees,
                        "°",
                        1.0,
                    ),
                    model::KeyProperty::MaskRoundness => (
                        tf("mask.row", &[&t("mask.roundness")]),
                        0.0,
                        1.0,
                        0.01,
                        0.0,
                        ParamFormat::Fraction,
                        "%",
                        100.0,
                    ),
                    model::KeyProperty::MaskFeather => (
                        tf("mask.row", &[&t("clipInspector.feather")]),
                        0.0,
                        1.0,
                        0.01,
                        0.0,
                        ParamFormat::Fraction,
                        "%",
                        100.0,
                    ),
                };
            let to_knob = |value: f64| knob_value(property, value);
            let keys: Vec<(f64, f64, model::KeyEase)> = clip
                .keys_on(property)
                .map(|key| (key.at, to_knob(key.value), key.ease))
                .collect();
            let (marks, curve, ease_index) =
                marks_and_curve(&keys, &|value| (value - minimum) / (maximum - minimum));
            let (prev, next) = clip.keys_around(property, at);
            rows.push(KeyRowData {
                field: key_field_of(property),
                param: SharedString::default(),
                label: label.into(),
                keys: marks,
                state: ClipKeyData {
                    field: key_field_of(property),
                    keyed: !keys.is_empty(),
                    here: clip.key_at(property, at).is_some(),
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
        if let Some(link) = clip
            .video_effects
            .iter()
            .find(|entry| entry.id == ADJUST_ID)
        {
            let inside = self.key_point().map(|(_, at)| at);
            for row in adjust_rows(&clip.video_effects, inside) {
                if !row.keyed {
                    continue;
                }
                let keys: Vec<(f64, f64, model::KeyEase)> = link
                    .keys_on(&row.key)
                    .iter()
                    .map(|key| (key.at, key.value, key.ease))
                    .collect();
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
                    let keys: Vec<(f64, f64, model::KeyEase)> = link
                        .keys_on(&master)
                        .iter()
                        .map(|key| (key.at, key.value, key.ease))
                        .collect();
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

    /// Moves the key at `from` to `to` and gives it `fraction` of its
    /// row's range, keeping its ease: a drag of a handle in the curve view.
    pub fn drag_key(&mut self, field: ClipField, param: &str, from: f32, to: f32, fraction: f32) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return;
        };
        let clip_id = clip.id.clone();
        let (from, to) = (f64::from(from), f64::from(to).clamp(0.0, 1.0));
        let fraction = f64::from(fraction).clamp(0.0, 1.0);
        let commands = if param.is_empty() {
            let Some(property) = Self::key_property_of(field) else {
                return;
            };
            let Some(index) = clip.key_at(property, from) else {
                return;
            };
            let (minimum, maximum) = knob_range(property);
            let value = model_value(property, minimum + fraction * (maximum - minimum));
            let ease = clip.keys[index].ease;
            vec![
                Command::ClearClipKey {
                    clip_id: clip_id.clone(),
                    property,
                    at: from,
                },
                Command::SetClipKey {
                    clip_id,
                    property,
                    at: to,
                    value,
                    ease,
                },
            ]
        } else {
            let Some(entry) = clip
                .video_effects
                .iter()
                .position(|entry| entry.id == ADJUST_ID)
            else {
                return;
            };
            let link = &clip.video_effects[entry];
            let Some(index) = link.key_at(param, from) else {
                return;
            };
            let Some(row) = adjust_rows(&clip.video_effects, Some(from))
                .into_iter()
                .find(|row| row.key == param)
            else {
                return;
            };
            let (minimum, maximum) = (f64::from(row.min), f64::from(row.max));
            let value = minimum + fraction * (maximum - minimum);
            let ease = link.keys_on(param)[index].ease;
            let mut commands = vec![
                Command::ClearEffectKey {
                    clip_id: clip_id.clone(),
                    entry,
                    key: param.to_owned(),
                    at: from,
                },
                Command::SetEffectKey {
                    clip_id: clip_id.clone(),
                    entry,
                    key: param.to_owned(),
                    at: to,
                    value,
                    ease,
                },
            ];
            commands.extend(partners_moved(link, &clip_id, entry, param, from, to));
            commands
        };
        self.apply(Command::Batch { commands });
    }

    /// The selected clip's keys as instants on the timeline, every property
    /// and Adjust knob together, one diamond per instant: what the ruler
    /// draws while exactly one clip is selected (#188).
    pub fn key_marks(&self) -> Vec<f32> {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return Vec::new();
        };
        let mut ats: Vec<f64> = model::KeyProperty::ALL
            .iter()
            .flat_map(|&property| clip.keys_on(property).map(|key| key.at))
            .collect();
        if let Some(link) = clip
            .video_effects
            .iter()
            .find(|entry| entry.id == ADJUST_ID)
        {
            ats.extend(link.keys.values().flatten().map(|key| key.at));
        }
        ats.sort_by(f64::total_cmp);
        ats.dedup_by(|a, b| (*a - *b).abs() <= model::KEY_EPSILON);
        ats.iter()
            .map(|at| (clip.start + at * clip.duration) as f32)
            .collect()
    }

    /// Moves every key of the selected clip that sits at the instant
    /// `from` to the instant `to`, as one undo step: what a dragged ruler
    /// diamond does.
    pub fn move_keys_at(&mut self, from: f32, to: f32) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return;
        };
        if clip.duration <= 0.0 {
            return;
        }
        let clip_id = clip.id.clone();
        let from_at = (f64::from(from) - clip.start) / clip.duration;
        let to_at = ((f64::from(to) - clip.start) / clip.duration).clamp(0.0, 1.0);
        let mut commands = Vec::new();
        for property in model::KeyProperty::ALL {
            let Some(index) = clip.key_at(property, from_at) else {
                continue;
            };
            let key = &clip.keys[index];
            commands.push(Command::ClearClipKey {
                clip_id: clip_id.clone(),
                property,
                at: key.at,
            });
            commands.push(Command::SetClipKey {
                clip_id: clip_id.clone(),
                property,
                at: to_at,
                value: key.value,
                ease: key.ease,
            });
        }
        if let Some(entry) = clip
            .video_effects
            .iter()
            .position(|entry| entry.id == ADJUST_ID)
        {
            let link = &clip.video_effects[entry];
            for name in link.keys.keys() {
                let Some(index) = link.key_at(name, from_at) else {
                    continue;
                };
                let key = &link.keys_on(name)[index];
                commands.push(Command::ClearEffectKey {
                    clip_id: clip_id.clone(),
                    entry,
                    key: name.clone(),
                    at: key.at,
                });
                commands.push(Command::SetEffectKey {
                    clip_id: clip_id.clone(),
                    entry,
                    key: name.clone(),
                    at: to_at,
                    value: key.value,
                    ease: key.ease,
                });
            }
        }
        if !commands.is_empty() {
            self.apply(Command::Batch { commands });
        }
    }

    /// Moves the playhead to `at` of the selected clip, `0..1`.
    pub fn jump_to_key(&mut self, at: f32) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return;
        };
        let (start, duration) = (clip.start, clip.duration);
        self.seek((start + f64::from(at).clamp(0.0, 1.0) * duration) as f32);
    }

    /// Moves the key at `from` to `to` on a property, or on the Adjust
    /// knob `param` names: the same value and ease at the new place, as
    /// one undo step.
    pub fn move_key(&mut self, field: ClipField, param: &str, from: f32, to: f32) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return;
        };
        let clip_id = clip.id.clone();
        let (from, to) = (f64::from(from), f64::from(to).clamp(0.0, 1.0));
        let commands = if param.is_empty() {
            let Some(property) = Self::key_property_of(field) else {
                return;
            };
            let Some(index) = clip.key_at(property, from) else {
                return;
            };
            let key = &clip.keys[index];
            let (value, ease) = (key.value, key.ease);
            vec![
                Command::ClearClipKey {
                    clip_id: clip_id.clone(),
                    property,
                    at: from,
                },
                Command::SetClipKey {
                    clip_id,
                    property,
                    at: to,
                    value,
                    ease,
                },
            ]
        } else {
            let Some(entry) = clip
                .video_effects
                .iter()
                .position(|entry| entry.id == ADJUST_ID)
            else {
                return;
            };
            let link = &clip.video_effects[entry];
            let Some(index) = link.key_at(param, from) else {
                return;
            };
            let key = &link.keys_on(param)[index];
            let (value, ease) = (key.value, key.ease);
            let mut commands = vec![
                Command::ClearEffectKey {
                    clip_id: clip_id.clone(),
                    entry,
                    key: param.to_owned(),
                    at: from,
                },
                Command::SetEffectKey {
                    clip_id: clip_id.clone(),
                    entry,
                    key: param.to_owned(),
                    at: to,
                    value,
                    ease,
                },
            ];
            commands.extend(partners_moved(link, &clip_id, entry, param, from, to));
            commands
        };
        self.apply(Command::Batch { commands });
    }

    /// Sets the ease into the key at `at`. A drag of the curve editor is
    /// many of these, folded into one undo step.
    pub fn set_key_ease(&mut self, field: ClipField, param: &str, at: f32, ease: [f32; 4]) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return;
        };
        let clip_id = clip.id.clone();
        let at = f64::from(at);
        let ease = model::KeyEase(ease.map(f64::from)).sane();
        let command = if param.is_empty() {
            let Some(property) = Self::key_property_of(field) else {
                return;
            };
            let Some(index) = clip.key_at(property, at) else {
                return;
            };
            Command::SetClipKey {
                clip_id,
                property,
                at,
                value: clip.keys[index].value,
                ease,
            }
        } else {
            let Some(entry) = clip
                .video_effects
                .iter()
                .position(|entry| entry.id == ADJUST_ID)
            else {
                return;
            };
            let link = &clip.video_effects[entry];
            if link.key_at(param, at).is_none() {
                return;
            }
            let mut commands: Vec<Command> = std::iter::once(param.to_owned())
                .chain(wheel_partners(param))
                .filter_map(|name| {
                    let index = link.key_at(&name, at)?;
                    Some(Command::SetEffectKey {
                        clip_id: clip_id.clone(),
                        entry,
                        value: link.keys_on(&name)[index].value,
                        key: name,
                        at,
                        ease,
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
        let mut candidates: Vec<f64> = model::KeyProperty::ALL
            .iter()
            .flat_map(|&property| clip.keys_on(property).map(|key| key.at))
            .collect();
        if let Some(link) = clip
            .video_effects
            .iter()
            .find(|entry| entry.id == ADJUST_ID)
        {
            candidates.extend(link.keys.values().flatten().map(|key| key.at));
        }
        let target = if delta < 0 {
            candidates
                .iter()
                .copied()
                .filter(|&key_at| key_at < at - model::KEY_EPSILON)
                .reduce(f64::max)
        } else {
            candidates
                .iter()
                .copied()
                .filter(|&key_at| key_at > at + model::KEY_EPSILON)
                .reduce(f64::min)
        };
        let Some(target) = target else {
            return;
        };
        let (start, duration) = (clip.start, clip.duration);
        self.seek((start + target * duration) as f32);
    }

    /// Takes every key off the selected clip: its properties and its
    /// Adjust knobs, as one undo step.
    pub fn clear_all_keys(&mut self) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return;
        };
        let clip_id = clip.id.clone();
        let mut commands: Vec<Command> = model::KeyProperty::ALL
            .iter()
            .filter(|&&property| clip.is_keyed(property))
            .map(|&property| Command::ClearClipKeys {
                clip_id: clip_id.clone(),
                property,
            })
            .collect();
        if let Some(entry) = clip
            .video_effects
            .iter()
            .position(|entry| entry.id == ADJUST_ID)
        {
            for key in clip.video_effects[entry].keys.keys() {
                commands.push(Command::ClearEffectKeys {
                    clip_id: clip_id.clone(),
                    entry,
                    key: key.clone(),
                });
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
        let Some(property) = Self::key_property_of(field) else {
            return;
        };
        let Some((clip, at)) = self.key_point() else {
            return;
        };
        let clip_id = clip.id.clone();
        let command = if clip.key_at(property, at).is_some() {
            Command::ClearClipKey {
                clip_id,
                property,
                at,
            }
        } else {
            let value = clip.value_at(property, at);
            // The ease of whichever key this one is joining behind, so
            // laying a run of keys down does not alternate between shapes.
            // The first key on a property has nothing to inherit and gets
            // the straight line.
            let ease = clip
                .keys_on(property)
                .rfind(|key| key.at < at)
                .map_or(model::KeyEase::LINEAR, |key| key.ease);
            Command::SetClipKey {
                clip_id,
                property,
                at,
                value,
                ease,
            }
        };
        self.apply(command);
    }

    /// Takes every key off a field, leaving it its constant.
    pub fn clear_keys_on(&mut self, field: ClipField) {
        let Some(property) = Self::key_property_of(field) else {
            return;
        };
        let Some(clip_id) = self.sole_selection() else {
            return;
        };
        self.apply(Command::ClearClipKeys { clip_id, property });
    }

    /// Moves the playhead to this field's previous (-1) or next (+1) key.
    pub fn step_key(&mut self, field: ClipField, delta: i32) {
        let Some(property) = Self::key_property_of(field) else {
            return;
        };
        let Some((clip, at)) = self.key_point() else {
            return;
        };
        let (start, duration) = (clip.start, clip.duration);
        let (prev, next) = clip.keys_around(property, at);
        let Some(target) = (if delta < 0 { prev } else { next }) else {
            return;
        };
        self.seek((start + target * duration) as f32);
    }

    /// The audio tracks the Audio panel offers for the selection: one label
    /// per track of the selected clip's media when it has more than one,
    /// else nothing - a file with one track has nothing to choose.
    pub(super) fn audio_track_labels(&self) -> Vec<SharedString> {
        let Some(item) = self
            .sole_selection()
            .and_then(|id| self.clip(&id))
            .and_then(|clip| self.project().media_by_id(&clip.media_id))
        else {
            return Vec::new();
        };
        if item.audio_tracks.len() < 2 {
            return Vec::new();
        }
        item.audio_tracks
            .iter()
            .enumerate()
            .map(|(position, track)| audio_track_label(track, position).into())
            .collect()
    }

    /// The selection, flattened for the inspector: exactly one clip or
    /// nothing.
    /// The selected transition, as the inspector's panel for it shows it.
    pub(super) fn selected_transition_data(&self) -> crate::ui::SelectedTransitionData {
        let Some(clip) = self.selected_transition() else {
            return Default::default();
        };
        let transition = clip.transition_in.as_ref().expect("held to one");
        let name = Catalogue::builtin()
            .get(&transition.id)
            .map(|package| package.manifest.effect.name.clone())
            .unwrap_or_else(|| label_of(&transition.id));
        crate::ui::SelectedTransitionData {
            present: true,
            name: name.into(),
            duration: transition.duration as f32,
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
            .and_then(|clip| clip.transition_in.as_ref())
        else {
            return Default::default();
        };
        let Some(package) = Catalogue::builtin().get(&transition.id) else {
            return Default::default();
        };
        let (mut knobs, mut choices) = (Vec::new(), Vec::new());
        for param in &package.manifest.params {
            let value = transition
                .params
                .get(&param.key)
                .copied()
                .unwrap_or(param.default);
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
        let Some(clip) = targets.first().and_then(|id| self.clip(id)) else {
            return SelectedClipData::default();
        };
        // A keyed property shows what it is worth at the playhead, which is
        // what the knob edits.
        let at = place_in(clip, self.playhead);
        let text = clip.text.clone().unwrap_or_default();
        // Where the family sits in the picker's list; none when nothing on
        // this machine has it, which the picker shows in red.
        let family_row = self
            .font_families()
            .iter()
            .position(|family| family.eq_ignore_ascii_case(text.font_family.trim_matches('"')));
        // Some words selected in the text box: the typography controls
        // style those words, so they show what the first of them wears -
        // a run's override where it has one, the title's look elsewhere.
        let words = self.styled_words().filter(|(id, _, _)| *id == clip.id);
        let look = words
            .as_ref()
            .map(|(_, start, _)| text.run_style_at(*start))
            .unwrap_or_default();
        let words_styled = words.as_ref().is_some_and(|(_, start, end)| {
            text.runs
                .iter()
                .any(|run| run.start < *end && *start < run.end)
        });
        let fill = look
            .color
            .as_deref()
            .map_or_else(|| colour_of(&text.color), colour_of);
        let stroke = colour_of(&text.stroke_color);
        let plate = colour_of(&text.background);
        let analysis = clip.cutout.as_ref().and_then(|cutout| {
            self.cutout_jobs
                .get(&Self::analysis_key(&clip.media_id, cutout.subject))
                .copied()
        });
        let mut enhance = self.enhance_readout(clip);
        let shape = clip.shape.clone().unwrap_or_default();
        let steady = self.stabilize_readout(clip);
        let enhance_preview = enhance.preview.is_some();
        let (enhance_before, enhance_after, (estimate_total, estimate_frames, estimate_per_frame)) =
            enhance.preview.take().unwrap_or_default();
        SelectedClipData {
            present: true,
            batch: targets.len() as i32,
            batch_volume,
            frame_width: self.output_size().0 as i32,
            frame_height: self.output_size().1 as i32,
            id: clip.id.as_str().into(),
            name: clip.name.as_str().into(),
            kind: kind_of(clip),
            duration: clip.duration as f32,
            transition_id: clip
                .transition_in
                .as_ref()
                .map(|transition| transition.id.as_str())
                .unwrap_or_default()
                .into(),
            transition_name: clip
                .transition_in
                .as_ref()
                .and_then(|transition| Catalogue::builtin().get(&transition.id))
                .map(|package| package.manifest.effect.name.clone())
                .unwrap_or_default()
                .into(),
            transition_duration: clip
                .transition_in
                .as_ref()
                .map(|transition| transition.duration as f32)
                .unwrap_or(0.5),
            scale: shown(clip, model::KeyProperty::Scale, at) as f32,
            offset_x: shown(clip, model::KeyProperty::OffsetX, at) as f32,
            offset_y: shown(clip, model::KeyProperty::OffsetY, at) as f32,
            rotation: shown(clip, model::KeyProperty::Rotation, at) as f32,
            stretch_x: clip.stretch_x as f32,
            stretch_y: clip.stretch_y as f32,
            opacity: shown(clip, model::KeyProperty::Opacity, at) as f32,
            volume: shown(clip, model::KeyProperty::Volume, at) as f32,
            audio_track: self
                .project()
                .media_by_id(&clip.media_id)
                .map_or(0, |item| item.audio_track_position(clip.audio_stream))
                as i32,
            speed: clip.speed as f32,
            preserve_pitch: clip.preserve_pitch,
            flip_h: clip.flip_h,
            flip_v: clip.flip_v,
            blend: concat_core::Blend::ALL
                .iter()
                .position(|mode| *mode == concat_core::Blend::parse(&clip.blend))
                .unwrap_or(0) as i32,
            crop_left: clip.crop.map(|crop| crop.left as f32).unwrap_or(0.0),
            crop_top: clip.crop.map(|crop| crop.top as f32).unwrap_or(0.0),
            crop_right: clip.crop.map(|crop| crop.right as f32).unwrap_or(0.0),
            crop_bottom: clip.crop.map(|crop| crop.bottom as f32).unwrap_or(0.0),
            mask: clip
                .mask
                .and_then(|mask| model::MaskShape::ALL.iter().position(|s| *s == mask.shape))
                .map_or(-1, |index| index as i32),
            mask_invert: clip.mask.is_some_and(|mask| mask.invert),
            mask_x: shown(clip, model::KeyProperty::MaskX, at) as f32,
            mask_y: shown(clip, model::KeyProperty::MaskY, at) as f32,
            mask_width: shown(clip, model::KeyProperty::MaskWidth, at) as f32,
            mask_height: shown(clip, model::KeyProperty::MaskHeight, at) as f32,
            mask_rotation: shown(clip, model::KeyProperty::MaskRotation, at) as f32,
            mask_roundness: shown(clip, model::KeyProperty::MaskRoundness, at) as f32,
            mask_feather: shown(clip, model::KeyProperty::MaskFeather, at) as f32,
            fade_in: clip.fade_in as f32,
            fade_out: clip.fade_out as f32,
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
            text_case: match text.text_case {
                model::TextCase::Original => 0,
                model::TextCase::Upper => 1,
                model::TextCase::Lower => 2,
                model::TextCase::Sentence => 3,
            },
            fill,
            fill_hex: hex_of(fill).into(),
            align: align_of(text.align),
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
            plate_radius: text.background_radius as f32,
            plate_padding_x: text.background_padding_x as f32,
            plate_padding_y: text.background_padding_y as f32,
            line_height: text.line_height as f32,
            tracking: text.tracking as f32,
            text_width: text.max_width as f32,
            text_height: text.max_height as f32,
            cutout: match &clip.cutout {
                None => 0,
                Some(cutout) if cutout.mode == model::CutoutMode::Auto => 1,
                Some(_) => 2,
            },
            cutout_feather: clip
                .cutout
                .as_ref()
                .map(|cutout| cutout.feather as f32)
                .unwrap_or(model::DEFAULT_FEATHER as f32),
            cutout_strokes: clip
                .cutout
                .as_ref()
                .map(|cutout| cutout.strokes.len() as i32)
                .unwrap_or(0),
            cutout_subject: clip
                .cutout
                .as_ref()
                .map(|cutout| match cutout.subject {
                    model::Subject::Auto => 0,
                    model::Subject::Person => 1,
                    model::Subject::Object => 2,
                })
                .unwrap_or(0),
            cutout_progress: analysis.map(|(_, fraction)| fraction).unwrap_or(-1.0),
            cutout_fetching: analysis.is_some_and(|(fetching, _)| fetching),
            cutout_empty: self.cutout_empty(clip),
            stabilize: clip.stabilize.is_some(),
            stabilize_smoothness: clip.stabilize.unwrap_or_default().smoothness as f32,
            stabilize_max_zoom: clip.stabilize.unwrap_or_default().max_zoom as f32,
            stabilize_tripod: clip.stabilize.is_some_and(|stabilize| stabilize.tripod),
            stabilize_progress: self
                .stabilize_jobs
                .get(&clip.media_id)
                .copied()
                .filter(|_| clip.stabilize.is_some())
                .unwrap_or(-1.0),
            stabilize_zoom: steady.map_or(-1.0, |(zoom, _)| zoom as f32),
            stabilize_needed: steady.map_or(-1.0, |(_, needed)| needed as f32),
            enhance_progress: self
                .enhance_jobs
                .get(&clip.id)
                .map(|(_, fraction)| *fraction)
                .unwrap_or(-1.0),
            enhance_fetching: self
                .enhance_jobs
                .get(&clip.id)
                .is_some_and(|(fetching, _)| *fetching),
            reverse_progress: self.reverse_jobs.get(&clip.id).copied().unwrap_or(-1.0),
            enhance_linked: clip.enhanced.is_some(),
            enhance_on: self.enhance_ticked(clip),
            enhance_busy: !self.enhance_jobs.is_empty() || self.enhance_previewing.is_some(),
            enhance_scale: enhance.scale,
            shape_kind: model::ShapeKind::ALL
                .iter()
                .position(|kind| *kind == shape.kind)
                .unwrap_or(0) as i32,
            shape_is_line: shape.kind.is_line(),
            shape_width: shape.width as f32,
            shape_height: shape.figure_box().1 as f32,
            shape_radius: shape.corner_radius as f32,
            shape_sides: shape.sides as i32,
            shape_points: shape.points as i32,
            shape_inner: shape.inner_radius as f32,
            shape_fill_on: !shape.fill.is_empty(),
            shape_fill: if shape.fill.is_empty() {
                slint::Color::from_rgb_u8(255, 255, 255)
            } else {
                crate::format::colour_of(&shape.fill)
            },
            shape_gradient: match &shape.gradient {
                None => 0,
                Some(gradient) if gradient.radial => 2,
                Some(_) => 1,
            },
            shape_gradient_color: crate::format::colour_of(
                &shape.gradient.clone().unwrap_or_default().color,
            ),
            shape_gradient_angle: shape.gradient.as_ref().map_or(90.0, |g| g.angle) as f32,
            shape_stroke_on: !shape.stroke.is_empty() && shape.stroke_width > 0.0,
            // A line is drawn in its outline's colour, or its fill's.
            shape_stroke: match (shape.stroke.is_empty(), shape.fill.is_empty()) {
                (false, _) => crate::format::colour_of(&shape.stroke),
                (true, false) if shape.kind.is_line() => crate::format::colour_of(&shape.fill),
                _ => slint::Color::from_rgb_u8(255, 255, 255),
            },
            shape_stroke_width: shape.stroke_width as f32,
            shape_stroke_align: match shape.stroke_align {
                model::StrokeAlign::Center => 0,
                model::StrokeAlign::Inside => 1,
                model::StrokeAlign::Outside => 2,
            },
            shape_dash: match shape.dash {
                model::StrokeDash::Solid => 0,
                model::StrokeDash::Dashed => 1,
                model::StrokeDash::Dotted => 2,
            },
            shape_start_head: model::LineHead::ALL
                .iter()
                .position(|head| *head == shape.start())
                .unwrap_or(0) as i32,
            shape_end_head: model::LineHead::ALL
                .iter()
                .position(|head| *head == shape.end())
                .unwrap_or(0) as i32,
            shape_trim_start: shape.trim_start as f32,
            shape_trim_end: shape.trim_end as f32,
            shape_shadow_on: shape.shadow.is_some(),
            shape_shadow_color: crate::format::colour_of(
                &shape.shadow.clone().unwrap_or_default().color,
            ),
            shape_shadow_blur: shape.shadow.as_ref().map_or(0.02, |s| s.blur) as f32,
            shape_shadow_distance: shape.shadow.as_ref().map_or(0.012, |s| s.distance) as f32,
            shape_shadow_angle: shape.shadow.as_ref().map_or(90.0, |s| s.angle) as f32,
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
                .is_some_and(|(id, _, _)| *id == clip.id),
            enhance_preview_fetch: self
                .enhance_previewing
                .as_ref()
                .filter(|(id, fetching, _)| *id == clip.id && *fetching)
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

    /// What the Enhance section shows for `clip` beyond its switch: the
    /// sizes its file can take, the preview made for it at the section's
    /// settings, if there is one, and what a whole run would cost.
    pub(super) fn enhance_readout(&self, clip: &Clip) -> EnhanceReadout {
        let mut readout = EnhanceReadout::default();
        let Some(media) = self.enhance_source(clip) else {
            return readout;
        };
        let (width, height) = (media.width.unwrap_or(0), media.height.unwrap_or(0));
        let scale = self.enhance_scale_of(&media);
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
        readout.preview = self
            .enhance_preview_for(clip, &media, scale)
            .map(|preview| {
                // The model's work on what a run covers - the span the clip
                // shows and a second either side, as `enhance_clip` cuts it -
                // at the warm tile time the preview measured.
                let frames = if clip.kind == model::ClipKind::Image {
                    1.0
                } else {
                    let span = clip.duration * clip.speed + 2.0 * super::ENHANCE_MARGIN;
                    span.min(media.duration.unwrap_or(f64::MAX)) * media.frame_rate.unwrap_or(30.0)
                };
                let per_frame = preview.tile_seconds * preview.tiles_per_frame as f64;
                let total = concat_host::enhance::estimate_seconds(
                    preview.tile_seconds,
                    preview.tiles_per_frame,
                    frames,
                );
                // The time, and for a video what it is made of: the frames
                // and each one's time. A still is one frame, said once.
                let estimate = if clip.kind == model::ClipKind::Image {
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

    /// The source instant of `clip` under the playhead, held to the clip.
    pub(super) fn source_at_playhead(&self, clip: &Clip) -> f64 {
        let along = (f64::from(self.playhead) - clip.start).clamp(0.0, clip.duration);
        clip.source_start + along * clip.speed
    }

    /// Whether the cutout model found nothing at the playhead's frame of
    /// `clip`, so the picture is showing as shot. Only an automatic cutout
    /// says so: a custom one is whatever was painted.
    fn cutout_empty(&self, clip: &Clip) -> bool {
        let Some(cutout) = clip.cutout.as_ref() else {
            return false;
        };
        if cutout.mode != model::CutoutMode::Auto {
            return false;
        }
        let (Some(session), Some(media)) = (
            self.session.as_ref(),
            self.project().media_by_id(&clip.media_id),
        ) else {
            return false;
        };
        let project = std::path::Path::new(session.path());
        let store = concat_vision::MaskStore::open(&concat_vision::mask_dir(
            project,
            &media.path,
            cutout.subject,
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
        'clips: for clip in &self.timeline().clips {
            let Some(cutout) = clip.cutout.as_ref() else {
                continue;
            };
            if cutout.mode != model::CutoutMode::Custom || !clip.kind.is_visual() {
                continue;
            }
            let Some(media) = self.project().media_by_id(&clip.media_id) else {
                continue;
            };
            // The source the clip shows, as the mask analysis reckons it.
            let range = (
                clip.source_start,
                clip.source_start + clip.duration * clip.speed.max(0.0625),
            );
            for stroke in &cutout.strokes {
                let request = RegionRequest {
                    project: project.clone(),
                    media_path: media.path.clone(),
                    media_size: (media.width.unwrap_or(0), media.height.unwrap_or(0)),
                    still: media.kind == model::MediaKind::Image,
                    subject: cutout.subject,
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
                next = Some((Self::analysis_key(&media.id, cutout.subject), request));
                break 'clips;
            }
        }
        for dir in tripped {
            if self.tripped_strokes.insert(dir) {
                log::warn!("smart brush: a stroke the app closed while reading is skipped");
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
        spawn_in_project(
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
                    Err(error) if error.contains("cancelled") => {}
                    Err(error) => studio.notify(&tf("studio.smartBrush", &[&error]), true),
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
        app.set_project_status(
            if self.dirty {
                "unsaved changes"
            } else {
                "saved"
            }
            .into(),
        );
        app.set_toast(ToastData {
            token: self.toast.token,
            message: self.toast.message.as_str().into(),
            failed: self.toast.failed,
        });
        app.set_start(self.start.data());
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
                        detail: format!(
                            "{} × {} · {}",
                            project.width,
                            project.height,
                            rate_label(project.rate_num, project.rate_den)
                        )
                        .into(),
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
        let cards: Vec<TextPresetData> = self
            .text_presets
            .iter()
            .map(|preset| {
                let plate = colour_of(&preset.style.background);
                TextPresetData {
                    id: preset.id.as_str().into(),
                    name: preset.name.as_str().into(),
                    language: preset.language.as_str().into(),
                    sample: preset.sample.as_str().into(),
                    family: preset.style.font_family.trim_matches('"').into(),
                    weight: preset.style.font_weight.round() as i32,
                    italic: preset.style.italic,
                    fill: colour_of(&preset.style.color),
                    plate,
                    plated: plate.alpha() > 0,
                    stroke: colour_of(&preset.style.stroke_color),
                    stroke_width: preset.style.stroke_width as f32,
                    align: align_of(preset.style.align),
                }
            })
            .collect();
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
        let items = &self.project().media;
        sync(&models.media, self.media.rows(self));
        let imported = |kind: Option<model::MediaKind>| {
            items
                .iter()
                .filter(|item| item.origin.is_none() && kind.is_none_or(|kind| item.kind == kind))
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
        editor.set_preview_effects_bypassed(self.preview_effects_bypassed);

        // The Details panel, and the sheet its Modify button opens. The
        // frame, rate and duration are the active timeline's, and the panel
        // says so by name; the name and folder are the project's.
        let folder: SharedString = self
            .session
            .as_ref()
            .map(|session| session.path().to_owned())
            .unwrap_or_else(|| "—".to_owned())
            .into();
        let timeline_name: SharedString = self.timeline().name.as_str().into();
        editor.set_project_name(self.project_name.as_str().into());
        editor.set_project_folder(folder.clone());
        editor.set_timeline_name(timeline_name.clone());
        editor.set_project_output(format!("{width} × {height}").into());
        editor.set_project_rate(format!("{:.2} fps", self.frame_rate()).into());
        editor.set_project_duration(frames_timecode(self.duration(), self.frame_rate()).into());
        editor.set_count_media(self.project().media.len() as i32);
        editor.set_count_tracks(self.timeline().tracks.len() as i32);
        editor.set_count_clips(self.timeline().clips.len() as i32);
        app.set_project_sheet(self.project_sheet.data(self));

        let rows = self.menu();
        editor.set_menu_height(Self::menu_height(&rows));
        sync(&models.menu, rows);
        editor.set_menu_token(self.menu_token);

        app.set_export(self.export.data(self));
        app.set_settings(self.settings.data(self));
        sync(&models.transcribers, self.settings.transcriber_rows());
        sync(&models.voices, self.settings.voice_rows());
        sync(&models.versions, self.settings.version_rows());
        sync(&models.version_details, self.settings.version_details());
        app.set_relink(self.relink.data(self));
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
        app.global::<Editor>().set_captions_sheet(captions.clone());
        app.set_captions(captions);
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
        app.set_speech(self.speech.data(self));
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
            // Proxies switched on in Settings are seen to at once.
            self.prefs.proxies,
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
        let Some(clip) = self.menu_target.as_ref().and_then(|id| self.clip(id)) else {
            return Vec::new();
        };
        let locked = self.locked(&clip.track_id);
        let playhead = f64::from(self.playhead);
        let straddled = clip.start < playhead && playhead < clip.start + clip.duration;

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
                straddled
                    && !locked
                    && (clip.kind == model::ClipKind::Video || clip.kind == model::ClipKind::Image),
            ),
            // The picture restored and enlarged by the model: the row opens
            // the clip's Enhance in the inspector, whose switch runs it.
            action(
                "enhance",
                t("common.enhance"),
                Glyph::Sparkle,
                "",
                !locked
                    && (clip.kind == model::ClipKind::Video || clip.kind == model::ClipKind::Image),
            ),
            rule(),
        ];
        // The picture sized to the frame, whole or covering it: beside the
        // inspector's two buttons, where a vertical cut is assembled clip
        // by clip.
        if clip.kind.is_visual() {
            let frameable = !locked
                && self
                    .project()
                    .media_by_id(&clip.media_id)
                    .and_then(|media| {
                        clip.framing_scale(
                            model::Framing::Fit,
                            (media.width, media.height),
                            self.output_size(),
                            0.0,
                        )
                    })
                    .is_some();
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
        let (can_detach, can_reattach) = self.sound_placement(clip);
        if can_reattach {
            rows.push(action(
                "reattach",
                t("studio.reattachAudio"),
                Glyph::Merge,
                "",
                !locked,
            ));
            rows.push(rule());
        } else if clip.kind == model::ClipKind::Video {
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
        if clip.kind == model::ClipKind::Audio || clip.kind == model::ClipKind::Video {
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
        }
        let audible = clip.kind != model::ClipKind::Image;
        rows.push(check(
            "mute",
            &t("studio.mute"),
            &keymap::label(&self.prefs, "mute"),
            clip.volume <= 0.0,
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
        let Some(item) = self.project().media_by_id(id) else {
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
            let range = item.color_range;
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
                range == Some(model::ColorRange::Limited),
            ));
            rows.push(check(
                "range-full",
                t("common.full"),
                "0-255",
                range == Some(model::ColorRange::Full),
            ));
            rows.push(rule());
        }
        rows.push(action("remove", t("common.remove"), Glyph::Trash, true));
        rows
    }

    /// A row of a bin card's menu, for the media it was opened on.
    pub fn media_action(&mut self, id: &str, action: &str) {
        log::info!("media menu: {action} on {id}");
        let Some(row) = self.media.row_of(id) else {
            return;
        };
        if self.project().media_by_id(id).is_none() {
            return;
        }
        let range = |range| Command::SetMediaColorRange {
            media_id: id.to_owned(),
            range,
        };
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
        let straddled = self.timeline().clips.iter().any(|clip| {
            clip.start + f64::from(MIN_DURATION) < playhead
                && playhead < clip.start + clip.duration - f64::from(MIN_DURATION)
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
                    !self.timeline().clips.is_empty(),
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
                    checked: self.timeline().linkage,
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
        let Some(track) = self.row_track(row).cloned() else {
            return;
        };
        let mut commands = Vec::new();
        if track.visible != visible {
            commands.push(Command::SetTrackFlag {
                track_id: track.id.clone(),
                flag: TrackFlag::Visible,
                value: visible,
            });
        }
        if track.muted != muted {
            commands.push(Command::SetTrackFlag {
                track_id: track.id.clone(),
                flag: TrackFlag::Muted,
                value: muted,
            });
        }
        let view = self.lanes.lane_view.entry(track.id.clone()).or_default();
        view.locked = locked;
        if locked {
            let doomed: Vec<String> = self
                .timeline()
                .clips
                .iter()
                .filter(|clip| clip.track_id == track.id)
                .map(|clip| clip.id.clone())
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
        if let Some(id) = self.row_track(row).map(|track| track.id.clone()) {
            self.lanes.lane_view.entry(id).or_default().size = size;
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
                    clip.kind != model::ClipKind::Audio && !self.locked(&clip.track_id)
                })
            })
            .cloned()
            .collect();
        let mut commands = Vec::new();
        for id in ids {
            let Some(before) = self.clip(&id).cloned() else {
                continue;
            };
            let mut after = before.clone();
            let at = place_in(&after, playhead);
            // Kept in the inspector's range, wrapping rather than stopping,
            // as the grip does.
            let next = (shown(&after, model::KeyProperty::Rotation, at) + 90.0 + 180.0)
                .rem_euclid(360.0)
                - 180.0;
            write_keyable(&mut after, model::KeyProperty::Rotation, next, at);
            if after.rotation != before.rotation {
                commands.push(Command::SetClipTransform {
                    clip_id: id.clone(),
                    scale: Some(after.scale),
                    offset_x: Some(after.offset_x),
                    offset_y: Some(after.offset_y),
                    rotation: Some(after.rotation),
                    stretch_x: Some(after.stretch_x),
                    stretch_y: Some(after.stretch_y),
                });
            }
            commands.extend(key_commands(&id, &before, &after));
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
                self.clip(id).map(|clip| Command::UpdateClip {
                    clip_id: id.clone(),
                    patch: ClipPatch {
                        flip_h: Some(!clip.flip_h),
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
        if view.locked {
            let doomed: Vec<String> = self
                .timeline()
                .clips
                .iter()
                .filter(|clip| clip.track_id == track_id)
                .map(|clip| clip.id.clone())
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
        let video = model::VideoSettings {
            width: width as u32,
            height: height as u32,
            ..session.video()
        };
        let timeline_id = self.project().active_timeline_id.clone();
        self.apply(Command::SetTimelineVideo { timeline_id, video });
        self.request_preview();
    }
}

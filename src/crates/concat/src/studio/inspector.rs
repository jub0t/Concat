// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! The inspector.

use super::*;

impl Studio {
    // ── the inspector ──

    /// One field of the selected clip, on the echo. `clip_commit` turns the
    /// accumulated edits into commands.
    pub fn clip_set(&mut self, field: ClipField, value: f32) {
        // Some words selected in the text box: the typography they can
        // wear goes on those words alone.
        if let Some((id, start, end)) = self.styled_words()
            && let Some(patch) = self.word_patch(&id, field, f64::from(value))
        {
            self.style_words(&id, start, end, &patch);
            return;
        }
        let mut ids = self.edit_targets();
        // A batch of sounds takes the level alone; see `batch_volume`.
        let batch_volume = ids.is_empty() && field == ClipField::Volume;
        if batch_volume {
            ids = self.volume_targets();
        }
        if ids.is_empty() {
            return;
        }
        self.commit_targets = ids.clone();
        self.begin_echo();
        let value = f64::from(value);
        // A batch keeps its shape: the knob shows the first clip's value,
        // and what it is turned to is read against that; see `batch_value`.
        let reference = (ids.len() > 1)
            .then(|| relative_property(field))
            .flatten()
            .and_then(|property| {
                self.clip(&ids[0])
                    .map(|clip| shown(clip, property, place_in(clip, self.playhead)))
            });
        for id in ids {
            // The media's tracks, read before the echo is borrowed: a row of
            // the Audio panel's list is a stream index of the file.
            let audio_tracks: Vec<u32> = if field == ClipField::AudioTrack {
                self.clip(&id)
                    .and_then(|clip| self.project().media_by_id(&clip.media_id))
                    .map(|item| item.audio_tracks.iter().map(|track| track.index).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            if field == ClipField::Stabilize
                && value != 0.0
                && let Some(media_id) = self
                    .clip(&id)
                    .filter(|clip| clip.stabilize.is_none())
                    .map(|clip| clip.media_id.clone())
            {
                // Turned on anew: a measurement the app died in is tried
                // again, since the person asked.
                self.stabilize_retry.insert(media_id);
            }
            // Where the playhead sits in the clip: a keyed property is written
            // there, as a key.
            let at = self
                .clip(&id)
                .map(|clip| place_in(clip, self.playhead))
                .unwrap_or(0.0);
            let value = match (reference, relative_property(field)) {
                (Some(reference), Some(property)) => self
                    .clip(&id)
                    .map(|clip| batch_value(field, value, reference, shown(clip, property, at)))
                    .unwrap_or(value),
                _ => value,
            };
            // The clip as the document has it, for a batch level: a drag's
            // every step is read against where it started, not against
            // the last step, so a ride's shape is not worn down on the way.
            let original = batch_volume
                .then(|| {
                    self.session
                        .as_ref()
                        .and_then(|session| session.project().active().clip(&id))
                        .cloned()
                })
                .flatten();
            let Some(clip) = self.echo_clip_mut(&id) else {
                continue;
            };
            if let Some(original) = original {
                crate::batch_volume::set_level(clip, &original, value, at);
                continue;
            }
            let text = clip.text.get_or_insert_with(TextStyle::default);
            match field {
                ClipField::Scale => {
                    write_keyable(clip, model::KeyProperty::Scale, value.clamp(0.05, 8.0), at)
                }
                ClipField::AudioTrack => {
                    // The first row is the file's default and is stored as such,
                    // so a clip on the first track saves as every clip did before
                    // there were tracks to choose.
                    let row = value.max(0.0) as usize;
                    clip.audio_stream = (row > 0).then(|| audio_tracks.get(row).copied()).flatten();
                }
                ClipField::StretchX => clip.stretch_x = value.clamp(0.1, 10.0),
                ClipField::StretchY => clip.stretch_y = value.clamp(0.1, 10.0),
                ClipField::OffsetX => write_keyable(
                    clip,
                    model::KeyProperty::OffsetX,
                    value.clamp(-1.0, 1.0),
                    at,
                ),
                ClipField::OffsetY => write_keyable(
                    clip,
                    model::KeyProperty::OffsetY,
                    value.clamp(-1.0, 1.0),
                    at,
                ),
                ClipField::Rotation => write_keyable(
                    clip,
                    model::KeyProperty::Rotation,
                    value.clamp(-180.0, 180.0),
                    at,
                ),
                ClipField::Opacity => {
                    write_keyable(clip, model::KeyProperty::Opacity, value.clamp(0.0, 1.0), at)
                }
                ClipField::CutoutFeather => {
                    clip.cutout.get_or_insert_with(model::Cutout::auto).feather =
                        value.clamp(0.0, model::MAX_FEATHER);
                }
                // Only footage has a camera to steady.
                ClipField::Stabilize => {
                    clip.stabilize = (value != 0.0 && clip.kind == model::ClipKind::Video)
                        .then(|| clip.stabilize.unwrap_or_default());
                }
                ClipField::StabilizeSmoothness => {
                    if let Some(stabilize) = clip.stabilize.as_mut() {
                        stabilize.smoothness = value.clamp(0.0, 1.0);
                    }
                }
                ClipField::StabilizeMaxZoom => {
                    if let Some(stabilize) = clip.stabilize.as_mut() {
                        stabilize.max_zoom = value.clamp(1.0, model::MAX_STABILIZE_ZOOM);
                    }
                }
                ClipField::StabilizeTripod => {
                    if let Some(stabilize) = clip.stabilize.as_mut() {
                        stabilize.tripod = value != 0.0;
                    }
                }
                // The shape's place in the picker, or below zero for none.
                // A new shape keeps where the mask was, its turn, softness
                // and invert, and takes the new shape's own size.
                ClipField::MaskShape => {
                    let shape = (value >= 0.0)
                        .then(|| model::MaskShape::ALL.get(value as usize).copied())
                        .flatten();
                    clip.mask = shape.map(|shape| match clip.mask {
                        Some(held) if held.shape == shape => held,
                        Some(held) => model::ClipMask {
                            x: held.x,
                            y: held.y,
                            rotation: held.rotation,
                            roundness: held.roundness,
                            feather: held.feather,
                            invert: held.invert,
                            ..model::ClipMask::of(shape)
                        },
                        None => model::ClipMask::of(shape),
                    });
                }
                ClipField::MaskInvert => {
                    if let Some(mask) = clip.mask.as_mut() {
                        mask.invert = value != 0.0;
                    }
                }
                ClipField::MaskX
                | ClipField::MaskY
                | ClipField::MaskWidth
                | ClipField::MaskHeight
                | ClipField::MaskRotation
                | ClipField::MaskRoundness
                | ClipField::MaskFeather => {
                    if let Some(property) = Self::key_property_of(field) {
                        write_keyable(clip, property, model::ClipMask::clamp(property, value), at);
                    }
                }
                ClipField::Volume => {
                    write_keyable(clip, model::KeyProperty::Volume, value.max(0.0), at)
                }
                ClipField::Speed => {
                    // A rate set by hand is a constant rate: the curve goes,
                    // as SetClipSpeed takes it.
                    let speed = value.clamp(0.0625, 16.0);
                    clip.duration =
                        (clip.duration * clip.speed / speed).max(f64::from(MIN_DURATION));
                    clip.speed = speed;
                    clip.speed_curve = None;
                }
                ClipField::PreservePitch => clip.preserve_pitch = value != 0.0,
                ClipField::FlipH => clip.flip_h = value != 0.0,
                ClipField::FlipV => clip.flip_v = value != 0.0,
                ClipField::Blend => {
                    let mode = concat_core::Blend::ALL
                        .get(value.max(0.0) as usize)
                        .copied()
                        .unwrap_or_default();
                    clip.blend = if mode == concat_core::Blend::Normal {
                        String::new()
                    } else {
                        mode.name().to_owned()
                    };
                }
                ClipField::CropLeft
                | ClipField::CropTop
                | ClipField::CropRight
                | ClipField::CropBottom => {
                    let mut crop = clip.crop.unwrap_or_default();
                    let edge = match field {
                        ClipField::CropLeft => &mut crop.left,
                        ClipField::CropTop => &mut crop.top,
                        ClipField::CropRight => &mut crop.right,
                        _ => &mut crop.bottom,
                    };
                    *edge = value.clamp(0.0, 0.9);
                    let crop = crop.tidy();
                    clip.crop = (!crop.is_none()).then_some(crop);
                }
                ClipField::FadeIn => clip.fade_in = value.clamp(0.0, clip.duration / 2.0),
                ClipField::FadeOut => clip.fade_out = value.clamp(0.0, clip.duration / 2.0),
                ClipField::FontSize => text.font_size = value.clamp(0.01, 0.5),
                ClipField::FontWeight => text.font_weight = value.clamp(100.0, 900.0),
                ClipField::Italic => text.italic = value != 0.0,
                ClipField::Underline => text.underline = value != 0.0,
                ClipField::Strikethrough => text.strikethrough = value != 0.0,
                ClipField::TextCase => {
                    text.text_case = match value as i32 {
                        1 => model::TextCase::Upper,
                        2 => model::TextCase::Lower,
                        3 => model::TextCase::Sentence,
                        _ => model::TextCase::Original,
                    };
                }
                ClipField::TextOpacity => text.opacity = value.clamp(0.0, 1.0),
                ClipField::Align => {
                    text.align = match value as i32 {
                        0 => TextAlign::Left,
                        2 => TextAlign::Right,
                        _ => TextAlign::Center,
                    }
                }
                ClipField::StrokeWidth => text.stroke_width = value.clamp(0.0, 0.15),
                ClipField::Shadow => text.shadow = value != 0.0,
                ClipField::LineHeight => text.line_height = value.clamp(0.7, 2.5),
                ClipField::Tracking => text.tracking = value.clamp(-0.05, 0.3),
                ClipField::TextWidth => text.max_width = value.clamp(0.0, 2.0),
                ClipField::TextHeight => text.max_height = value.clamp(0.0, 2.0),
                // The stroke's opacity is its colour's alpha; see `hex_rgba`.
                ClipField::StrokeOpacity => {
                    let edge = colour_of(&text.stroke_color);
                    text.stroke_color = hex_rgba(slint::Color::from_argb_u8(
                        (value.clamp(0.0, 1.0) * 255.0).round() as u8,
                        edge.red(),
                        edge.green(),
                        edge.blue(),
                    ));
                }
                // The background's opacity is its colour's alpha the same way.
                // Never zero from the dial: fully transparent is how "no
                // background" is stored, and the switch is what takes it away.
                ClipField::BackgroundOpacity => {
                    let plate = colour_of(&text.background);
                    text.background = hex_with_alpha(slint::Color::from_argb_u8(
                        (value.clamp(0.01, 1.0) * 255.0).round() as u8,
                        plate.red(),
                        plate.green(),
                        plate.blue(),
                    ));
                }
                ClipField::BackgroundRadius => text.background_radius = value.clamp(0.0, 0.2),
                ClipField::BackgroundPaddingX => {
                    text.background_padding_x = value.clamp(0.0, 0.5);
                }
                ClipField::BackgroundPaddingY => {
                    text.background_padding_y = value.clamp(0.0, 0.5);
                }
                ClipField::ShapeKind
                | ClipField::ShapeWidth
                | ClipField::ShapeHeight
                | ClipField::ShapeRadius
                | ClipField::ShapeSides
                | ClipField::ShapePoints
                | ClipField::ShapeInner
                | ClipField::ShapeFillOn
                | ClipField::ShapeGradient
                | ClipField::ShapeGradientAngle
                | ClipField::ShapeStrokeOn
                | ClipField::ShapeStrokeWidth
                | ClipField::ShapeStrokeAlign
                | ClipField::ShapeDash
                | ClipField::ShapeStartHead
                | ClipField::ShapeEndHead
                | ClipField::ShapeTrimStart
                | ClipField::ShapeTrimEnd
                | ClipField::ShapeShadowOn
                | ClipField::ShapeShadowBlur
                | ClipField::ShapeShadowDistance
                | ClipField::ShapeShadowAngle => {
                    if clip.kind == model::ClipKind::Shape {
                        set_shape(
                            clip.shape.get_or_insert_with(model::ShapeStyle::default),
                            field,
                            value,
                        );
                    }
                }
            }
            // A media clip has no text; the placeholder must not linger.
            if clip.kind != model::ClipKind::Text {
                clip.text = None;
            }
        }
    }

    pub fn clip_set_text(&mut self, field: ClipTextField, value: &str) {
        self.set_text_at(field, value, None);
    }

    /// The words typed into the Text tab's box, with where the caret ended
    /// up - a byte into `value`, as the box counts - so a run beside the
    /// edit knows whether the new letters are its own; see
    /// `concat_project::runs`.
    pub fn clip_set_words(&mut self, value: &str, caret: i32) {
        let caret = (caret >= 0).then(|| char_at(value, caret));
        self.set_text_at(ClipTextField::Content, value, caret);
    }

    fn set_text_at(&mut self, field: ClipTextField, value: &str, caret: Option<usize>) {
        let ids = self.edit_targets();
        if ids.is_empty() {
            return;
        }
        // The words are each title's own: a batch is styled together, never
        // made to say the same thing.
        if field == ClipTextField::Content && ids.len() > 1 {
            return;
        }
        // One of the app's own fonts goes on the project before the title
        // is set in it, as a command of its own: the painter reads the
        // project's fonts, and the echo below is not the project.
        if field == ClipTextField::FontFamily {
            self.ensure_font(value);
        }
        self.commit_targets = ids.clone();
        self.begin_echo();
        for id in ids {
            let Some(clip) = self.echo_clip_mut(&id) else {
                continue;
            };
            if clip.kind != model::ClipKind::Text {
                continue;
            }
            let text = clip.text.get_or_insert_with(TextStyle::default);
            match field {
                ClipTextField::Content => {
                    // The runs ride along with the words they are on.
                    text.set_content(value.to_owned(), caret);
                    let first = value.lines().next().unwrap_or("").trim().to_owned();
                    clip.name = if first.is_empty() {
                        "Title".into()
                    } else {
                        first
                    };
                }
                ClipTextField::FontFamily => text.font_family = value.to_owned(),
                _ => {}
            }
        }
        // The words are on the echo now; show them. A title being typed is
        // painted in memory at the monitor's size, the way a grip drag is,
        // so the picture keeps up with the keystrokes while the commit
        // still lands once, on the way out of the field.
        //
        // Pending from the first keystroke, not from the field's blur: the
        // echo is dropped by anything that changes the edit - a press on a
        // clip, an undo, a command from the menu - and `flush_commit` only
        // lands what is pending. Words typed and not yet blurred were on
        // the echo alone, and went with it. Nothing starts the timer here;
        // the blur does, and a flush before then lands them too.
        self.commit_pending = true;
        self.request_preview();
    }
}

/// One of the Shape section's numbers onto `shape`. A switch turned on
/// brings back a visible default rather than an invisible one: a fill in
/// white, an outline in white a little over a pixel thick, a soft shadow.
fn set_shape(shape: &mut model::ShapeStyle, field: ClipField, value: f64) {
    let index = value.max(0.0).round() as usize;
    match field {
        ClipField::ShapeKind => {
            if let Some(kind) = model::ShapeKind::ALL.get(index) {
                shape.kind = *kind;
            }
        }
        ClipField::ShapeWidth => shape.width = value.clamp(0.01, 4.0),
        ClipField::ShapeHeight => shape.height = value.clamp(0.01, 4.0),
        ClipField::ShapeRadius => shape.corner_radius = value.clamp(0.0, 1.0),
        ClipField::ShapeSides => shape.sides = (value.round() as u32).clamp(3, 12),
        ClipField::ShapePoints => shape.points = (value.round() as u32).clamp(3, 24),
        ClipField::ShapeInner => shape.inner_radius = value.clamp(0.05, 0.95),
        ClipField::ShapeFillOn => {
            if value != 0.0 {
                if shape.fill.is_empty() {
                    shape.fill = "#ffffff".to_owned();
                }
            } else {
                shape.fill.clear();
            }
        }
        ClipField::ShapeGradient => {
            shape.gradient = match index {
                0 => None,
                _ => Some(model::ShapeGradient {
                    radial: index == 2,
                    ..shape.gradient.clone().unwrap_or_default()
                }),
            };
        }
        ClipField::ShapeGradientAngle => {
            if let Some(gradient) = shape.gradient.as_mut() {
                gradient.angle = value.rem_euclid(360.0);
            }
        }
        ClipField::ShapeStrokeOn => {
            if value != 0.0 {
                if shape.stroke.is_empty() {
                    shape.stroke = "#ffffff".to_owned();
                }
                if shape.stroke_width <= 0.0 {
                    shape.stroke_width = 0.012;
                }
            } else {
                shape.stroke.clear();
            }
        }
        ClipField::ShapeStrokeWidth => shape.stroke_width = value.clamp(0.0, 0.5),
        ClipField::ShapeStrokeAlign => {
            shape.stroke_align = match index {
                1 => model::StrokeAlign::Inside,
                2 => model::StrokeAlign::Outside,
                _ => model::StrokeAlign::Center,
            };
        }
        ClipField::ShapeDash => {
            shape.dash = match index {
                1 => model::StrokeDash::Dashed,
                2 => model::StrokeDash::Dotted,
                _ => model::StrokeDash::Solid,
            };
        }
        ClipField::ShapeStartHead => {
            shape.start_head = model::LineHead::ALL.get(index).copied();
        }
        ClipField::ShapeEndHead => {
            shape.end_head = model::LineHead::ALL.get(index).copied();
        }
        // Each end held on its own side of the other.
        ClipField::ShapeTrimStart => {
            shape.trim_start = value.clamp(0.0, 1.0).min(shape.trim_end);
        }
        ClipField::ShapeTrimEnd => {
            shape.trim_end = value.clamp(0.0, 1.0).max(shape.trim_start);
        }
        ClipField::ShapeShadowOn => {
            shape.shadow = (value != 0.0).then(|| shape.shadow.clone().unwrap_or_default());
        }
        ClipField::ShapeShadowBlur => {
            if let Some(shadow) = shape.shadow.as_mut() {
                shadow.blur = value.clamp(0.0, 0.2);
            }
        }
        ClipField::ShapeShadowDistance => {
            if let Some(shadow) = shape.shadow.as_mut() {
                shadow.distance = value.clamp(0.0, 0.2);
            }
        }
        ClipField::ShapeShadowAngle => {
            if let Some(shadow) = shape.shadow.as_mut() {
                shadow.angle = value.rem_euclid(360.0);
            }
        }
        _ => {}
    }
}

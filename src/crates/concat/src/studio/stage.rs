// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! The stage: what is drawn over the monitor and handled on it.

use super::*;

impl Studio {
    // ── the stage ──
    //
    // The monitor as a place to edit, not only to look: the pictures under
    // the playhead can be grabbed, slid, scaled and turned where they are
    // drawn. The same echo-then-command shape as the lanes and the
    // inspector, with one difference: the echo is composited too, so the
    // frame follows the pointer and not only the outline does.

    /// Where `clip` lands in the output frame. The compositor's own rule —
    /// contain-fitted, then the transform — restated in fractions.
    pub fn footprint(&self, clip: &Clip) -> Footprint {
        let (width, height) = self.output_size();
        let (width, height) = (f64::from(width.max(1)), f64::from(height.max(1)));
        let painted = matches!(clip.kind, model::ClipKind::Text | model::ClipKind::Shape)
            .then(|| self.title_blocks.get(&clip.id).copied())
            .flatten();
        let (w, h) = if let Some(((bw, bh), _)) = painted {
            // The painter said how big the block came out.
            (
                f64::from(bw) * clip.scale / width,
                f64::from(bh) * clip.scale / height,
            )
        } else if clip.kind == model::ClipKind::Text {
            // Not painted yet: a reading of the style until it is. An em is
            // `font_size` of the frame's height, a glyph runs about six
            // tenths of one, and a little air is left around the block the
            // way the plate would.
            let text = clip.text.clone().unwrap_or_default();
            let lines: Vec<&str> = text.content.lines().collect();
            let rows = lines.len().max(1) as f64;
            let longest = lines
                .iter()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0)
                .max(1) as f64;
            let em = text.font_size.max(0.005) * height;
            let glyph = 0.6 * em + text.tracking * height;
            (
                (longest * glyph + 0.6 * em) * clip.scale / width,
                (rows * em * text.line_height.max(0.5) + 0.5 * em) * clip.scale / height,
            )
        } else if clip.kind == model::ClipKind::Shape {
            // Not painted yet: the figure's box from its style, the way the
            // painter sizes it, until the painter says.
            let shape = clip.shape.clone().unwrap_or_default();
            let side = shape.size.clamp(0.01, 2.0) * height;
            let (w, h) = match shape.kind {
                model::ShapeKind::Square | model::ShapeKind::Circle => (side, side),
                model::ShapeKind::Triangle => (side, side * 0.866),
                model::ShapeKind::Parallelogram | model::ShapeKind::Trapezoid => (side, side * 0.6),
                model::ShapeKind::Line => (side, shape.stroke_width * height),
                model::ShapeKind::Arrow => (side, (shape.stroke_width * 4.0).max(0.18) * side),
            };
            (w * clip.scale / width, h * clip.scale / height)
        } else {
            let media = self.project().media_by_id(&clip.media_id);
            let source = media
                .and_then(|item| Some((item.width?, item.height?)))
                .filter(|(w, h)| *w > 0 && *h > 0)
                .map(|(w, h)| (f64::from(w), f64::from(h)))
                // What is left after the crop is what gets fitted.
                .map(|(w, h)| match clip.crop {
                    Some(crop) => (
                        w * (1.0 - crop.left - crop.right).max(0.1),
                        h * (1.0 - crop.top - crop.bottom).max(0.1),
                    ),
                    None => (w, h),
                });
            match source {
                Some((sw, sh)) => {
                    let fit = (width / sw).min(height / sh);
                    (
                        sw * fit * clip.scale / width,
                        sh * fit * clip.scale / height,
                    )
                }
                // Dimensions never learnt: the frame's own shape, which is
                // what the compositor falls back to as well.
                None => (clip.scale, clip.scale),
            }
        };
        // Then pulled along each axis, as the compositor pulls it - a
        // picture, that is. A title's box is its style's, and the compositor
        // never stretches one; see titles.rs.
        // https://github.com/jub0t/Concat/issues/119
        let (w, h) = if clip.kind == model::ClipKind::Text {
            (w, h)
        } else {
            (w * clip.stretch_x, h * clip.stretch_y)
        };
        // The placement at the playhead: the clip's own, or its ride's
        // where a property is keyed, so the box follows the keys.
        let at = place_in(clip, self.playhead);
        let base = concat_core::timeline::Transform {
            scale: shown(clip, model::KeyProperty::Scale, at),
            offset_x: shown(clip, model::KeyProperty::OffsetX, at),
            offset_y: shown(clip, model::KeyProperty::OffsetY, at),
            rotation: shown(clip, model::KeyProperty::Rotation, at),
            stretch_x: clip.stretch_x,
            stretch_y: clip.stretch_y,
        };
        let placed = base;
        let factor = placed.scale / clip.scale.max(1e-6);
        // A title anchored on an edge paints its block beside the clip's
        // position, not on it; the box goes where the block is. The offset
        // is canvas pixels, so it scales, stretches and turns with the clip
        // before it becomes a fraction of the frame.
        let (ax, ay) = match painted {
            Some((_, (dx, dy))) if dx != 0 || dy != 0 => {
                let px = f64::from(dx) * placed.scale * clip.stretch_x;
                let py = f64::from(dy) * placed.scale * clip.stretch_y;
                let (sin, cos) = placed.rotation.to_radians().sin_cos();
                (
                    (px * cos - py * sin) / width,
                    (px * sin + py * cos) / height,
                )
            }
            _ => (0.0, 0.0),
        };
        Footprint {
            cx: 0.5 + placed.offset_x + ax,
            cy: 0.5 + placed.offset_y + ay,
            w: w * factor,
            h: h * factor,
            rotation: placed.rotation,
        }
    }

    /// The pictures under the playhead on lanes that are showing, bottom of
    /// the stack first — the order the compositor lays them down, and the
    /// order the overlay draws them in.
    pub(super) fn stage_clips(&self) -> Vec<&Clip> {
        let timeline = self.timeline();
        let playhead = f64::from(self.playhead);
        let showing: HashSet<&str> = timeline
            .tracks
            .iter()
            .filter(|track| track.visible)
            .map(|track| track.id.as_str())
            .collect();
        let mut clips: Vec<&Clip> = timeline
            .clips
            .iter()
            .map(|clip| clip.as_ref())
            .filter(|clip| {
                (clip.kind.is_visual()
                    || matches!(clip.kind, model::ClipKind::Text | model::ClipKind::Shape))
                    && showing.contains(clip.track_id.as_str())
                    && clip.start <= playhead
                    && playhead < clip.start + clip.duration
            })
            .collect();
        // Rows count from the top; the bottom of the stack is the highest.
        clips.sort_by_key(|clip| std::cmp::Reverse(self.row_of(&clip.track_id)));
        clips
    }

    pub fn stage_items(&self) -> Vec<StageItemData> {
        self.stage_clips()
            .into_iter()
            .map(|clip| {
                let footprint = self.footprint(clip);
                let selected = self.selection.iter().any(|id| id == &clip.id);
                let mut item = StageItemData {
                    id: clip.id.as_str().into(),
                    kind: kind_of(clip),
                    selected,
                    cx: footprint.cx as f32,
                    cy: footprint.cy as f32,
                    w: footprint.w as f32,
                    h: footprint.h as f32,
                    rotation: footprint.rotation as f32,
                    scale: clip.scale as f32,
                    mask: -1,
                    ..Default::default()
                };
                // The mask's outline and grips, on the one clip selected.
                if selected
                    && self.selection.len() == 1
                    && let Some(frame) = self.mask_frame(clip)
                {
                    let grips = frame.grips().map(|grip| frame.fraction(grip));
                    item.mask = model::MaskShape::ALL
                        .iter()
                        .position(|shape| *shape == frame.mask.shape)
                        .map_or(-1, |index| index as i32);
                    item.mask_path = frame.outline().into();
                    (item.mask_cx, item.mask_cy) = grips[0];
                    (item.mask_wx, item.mask_wy) = grips[1];
                    (item.mask_hx, item.mask_hy) = grips[2];
                    (item.mask_kx, item.mask_ky) = grips[3];
                    (item.mask_rx, item.mask_ry) = grips[4];
                    (item.mask_fx, item.mask_fy) = grips[5];
                }
                item
            })
            .collect()
    }

    /// The clip's mask placed in the frame at the playhead, its keyed
    /// numbers at their ride's values; None for a clip with no mask or one
    /// that is not a picture.
    fn mask_frame(&self, clip: &Clip) -> Option<MaskFrame> {
        let held = clip.mask.filter(|_| clip.kind.is_visual())?;
        let at = place_in(clip, self.playhead);
        let mut mask = held;
        for property in model::KeyProperty::MASK {
            mask.set(property, shown(clip, property, at));
        }
        Some(MaskFrame::new(
            mask,
            &self.footprint(clip),
            self.output_size(),
            (clip.stretch_x, clip.stretch_y),
        ))
    }

    /// The topmost picture under a frame point, skipping locked lanes the
    /// way a press on the lanes does.
    fn stage_hit(&self, x: f64, y: f64) -> Option<String> {
        let frame = self.output_size();
        self.stage_clips()
            .into_iter()
            .rev()
            .filter(|clip| !self.locked(&clip.track_id))
            .find(|clip| self.footprint(clip).contains(x, y, frame))
            .map(|clip| clip.id.clone())
    }

    /// A press on the stage floor: resolve the selection, then arm a move
    /// of everything selected that is under the playhead. Empty stage
    /// clears the selection, as an empty lane does.
    pub fn stage_pressed(&mut self, x: f32, y: f32, additive: bool) {
        // An armed eyedropper takes the press before a brush or a move.
        if let Some(pick) = self.picking.clone() {
            self.gesture = Gesture::None;
            self.pick_colour_at(pick, f64::from(x), f64::from(y));
            return;
        }
        if let Some(clip) = self.paint_target() {
            let point = self.stage_to_source(&clip, f64::from(x), f64::from(y));
            self.gesture = Gesture::Paint {
                clip: clip.id.clone(),
                tool: BRUSHES[self.brush.min(BRUSHES.len() - 1)],
                size: self.brush_size,
                at: self.source_at_playhead(&clip),
                points: vec![point],
                screen: vec![(x, y)],
            };
            return;
        }
        let (x, y) = (f64::from(x), f64::from(y));
        // As on the lanes: what the inspector holds for the current
        // selection lands before the selection changes under it - and a
        // press on the floor, which clears it, is the commonest way out of
        // a title's text field.
        self.flush_commit();
        let Some(id) = self.stage_hit(x, y) else {
            if !additive {
                self.selection.clear();
                self.transition_selected = None;
            }
            self.gesture = Gesture::None;
            return;
        };
        let already = self.selection.iter().any(|held| held == &id);
        if additive {
            if already {
                self.selection.retain(|held| held != &id);
            } else {
                self.selection.push(id.clone());
            }
        } else if !already {
            self.selection = vec![id.clone()];
        }
        if !self.selection.iter().any(|held| held == &id) {
            self.gesture = Gesture::None;
            return;
        }
        let origins: Vec<StageOrigin> = self
            .stage_clips()
            .into_iter()
            .filter(|clip| {
                self.selection.iter().any(|held| held == &clip.id) && !self.locked(&clip.track_id)
            })
            .map(|clip| {
                // A keyed clip is dragged from where its ride has it at
                // the playhead, not from a constant nothing shows.
                let at = place_in(clip, self.playhead);
                StageOrigin {
                    clip: clip.id.clone(),
                    offset_x: shown(clip, model::KeyProperty::OffsetX, at),
                    offset_y: shown(clip, model::KeyProperty::OffsetY, at),
                }
            })
            .collect();
        self.begin_echo();
        self.stage_guides.clear();
        self.gesture = Gesture::StageMove {
            primary: id,
            origins,
            from: (x, y),
        };
    }

    /// Where a picture's bounds pull to on one axis. Every candidate is a
    /// feature of the moving picture — an edge or the centre — against a
    /// target — the frame's edges and centre, and every other picture's —
    /// and the nearest pair inside `pull` wins. Returns the shift that lands
    /// it and the target's position, for the guide.
    pub(super) fn stage_snap(features: [f64; 3], targets: &[f64], pull: f64) -> Option<(f64, f64)> {
        let mut best: Option<(f64, f64)> = None;
        for feature in features {
            for &target in targets {
                let shift = target - feature;
                if shift.abs() < pull && best.is_none_or(|(held, _)| shift.abs() < held.abs()) {
                    best = Some((shift, target));
                }
            }
        }
        best
    }

    /// A press on a grip of `id`'s box: a corner scales, the handle above
    /// turns. Both work about the picture's centre, in frame pixels.
    pub fn stage_grip_pressed(&mut self, id: &str, grip: i32, x: f32, y: f32) {
        let Some(clip) = self.clip(id).cloned() else {
            return;
        };
        if self.locked(&clip.track_id) {
            return;
        }
        let (width, height) = self.output_size();
        // A grip of the mask: the mask is what moves, not the picture.
        if let Some(grip) = MaskGrip::of(grip) {
            let Some(frame) = self.mask_frame(&clip) else {
                return;
            };
            self.flush_commit();
            self.begin_echo();
            self.gesture = Gesture::StageMask {
                clip: id.to_owned(),
                grip,
                frame,
                from: (
                    f64::from(x) * f64::from(width),
                    f64::from(y) * f64::from(height),
                ),
            };
            return;
        }
        let footprint = self.footprint(&clip);
        let centre = (
            footprint.cx * f64::from(width),
            footprint.cy * f64::from(height),
        );
        let dx = f64::from(x) * f64::from(width) - centre.0;
        let dy = f64::from(y) * f64::from(height) - centre.1;
        let half = footprint.half_bounds((width, height));
        self.flush_commit();
        self.begin_echo();
        self.gesture = if grip == 4 {
            Gesture::StageRotate {
                clip: id.to_owned(),
                rotation: shown(
                    &clip,
                    model::KeyProperty::Rotation,
                    place_in(&clip, self.playhead),
                ),
                centre,
                from: dy.atan2(dx),
            }
        } else if (grip == 6 || grip == 8) && clip.kind == model::ClipKind::Text {
            // A title's side grips set where its words wrap, not how wide
            // its glyphs are; see `Gesture::TextWidth`.
            Gesture::TextWidth {
                clip: id.to_owned(),
                centre,
                rotation: clip.rotation,
            }
        } else if (grip == 5 || grip == 7) && clip.kind == model::ClipKind::Text {
            // And the top and bottom grips size its box, never its glyphs.
            // https://github.com/jub0t/Concat/issues/119
            Gesture::TextHeight {
                clip: id.to_owned(),
                centre,
                rotation: clip.rotation,
            }
        } else if grip >= 5 {
            // 5 top, 6 right, 7 bottom, 8 left: the pointer's offset from
            // the centre, turned back into the box's own frame.
            let across = grip == 6 || grip == 8;
            let (sin, cos) = clip.rotation.to_radians().sin_cos();
            let along = if across {
                dx * cos + dy * sin
            } else {
                -dx * sin + dy * cos
            };
            Gesture::StageStretch {
                clip: id.to_owned(),
                across,
                stretch: if across {
                    clip.stretch_x
                } else {
                    clip.stretch_y
                },
                centre,
                from: along.abs().max(1.0),
                rotation: clip.rotation,
            }
        } else {
            Gesture::StageScale {
                clip: id.to_owned(),
                scale: shown(
                    &clip,
                    model::KeyProperty::Scale,
                    place_in(&clip, self.playhead),
                ),
                centre,
                from: dx.hypot(dy).max(1.0),
                half,
            }
        };
    }

    /// The pointer moved with a stage gesture live: the echo follows, and
    /// the monitor composites it.
    pub fn stage_dragged(&mut self, x: f32, y: f32, snap: bool) {
        let mut gesture = std::mem::replace(&mut self.gesture, Gesture::None);
        if let Gesture::Paint {
            clip,
            points,
            screen,
            ..
        } = &mut gesture
        {
            if let Some(clip) = self.clip(clip).cloned() {
                points.push(self.stage_to_source(&clip, f64::from(x), f64::from(y)));
                screen.push((x, y));
            }
            self.gesture = gesture;
            return;
        }
        let (x, y) = (f64::from(x), f64::from(y));
        let (width, height) = self.output_size();
        match &gesture {
            Gesture::StageMove {
                primary,
                origins,
                from,
            } => {
                let (mut dx, mut dy) = (x - from.0, y - from.1);
                if snap {
                    // Shift holds the axis the drag is mostly along, measured
                    // in pixels so a tall frame does not bias it.
                    if (dx * f64::from(width)).abs() >= (dy * f64::from(height)).abs() {
                        dy = 0.0;
                    } else {
                        dx = 0.0;
                    }
                }
                self.stage_guides.clear();
                if self.lanes.snap {
                    // The same pull on both axes, in frame pixels: a
                    // hundredth of the long side, which is about eight
                    // pixels on a stage of the size a laptop gives it.
                    let pull = 0.01 * f64::from(width.max(height));
                    let frame = (width, height);
                    let moving = origins
                        .iter()
                        .find(|origin| &origin.clip == primary)
                        .and_then(|origin| self.clip(&origin.clip).map(|clip| (origin, clip)));
                    if let Some((origin, clip)) = moving {
                        let (hw, hh) = self.footprint(clip).half_bounds(frame);
                        let cx = 0.5 + origin.offset_x + dx;
                        let cy = 0.5 + origin.offset_y + dy;
                        // The frame's own lines, then every picture that is
                        // staying put.
                        let mut xs = vec![0.0, 0.5, 1.0];
                        let mut ys = vec![0.0, 0.5, 1.0];
                        for other in self.stage_clips() {
                            if origins.iter().any(|origin| origin.clip == other.id) {
                                continue;
                            }
                            let footprint = self.footprint(other);
                            let (ow, oh) = footprint.half_bounds(frame);
                            xs.extend([footprint.cx - ow, footprint.cx, footprint.cx + ow]);
                            ys.extend([footprint.cy - oh, footprint.cy, footprint.cy + oh]);
                        }
                        if let Some((shift, at)) =
                            Self::stage_snap([cx - hw, cx, cx + hw], &xs, pull / f64::from(width))
                        {
                            dx += shift;
                            self.stage_guides.push(StageGuideData {
                                vertical: true,
                                at: at as f32,
                            });
                        }
                        if let Some((shift, at)) =
                            Self::stage_snap([cy - hh, cy, cy + hh], &ys, pull / f64::from(height))
                        {
                            dy += shift;
                            self.stage_guides.push(StageGuideData {
                                vertical: false,
                                at: at as f32,
                            });
                        }
                    }
                }
                let playhead = self.playhead;
                for origin in origins {
                    if let Some(clip) = self.echo_clip_mut(&origin.clip) {
                        let at = place_in(clip, playhead);
                        let (x, y) = (
                            (origin.offset_x + dx).clamp(-1.0, 1.0),
                            (origin.offset_y + dy).clamp(-1.0, 1.0),
                        );
                        write_keyable(clip, model::KeyProperty::OffsetX, x, at);
                        write_keyable(clip, model::KeyProperty::OffsetY, y, at);
                    }
                }
            }
            Gesture::StageScale {
                clip,
                scale,
                centre,
                from,
                half,
            } => {
                let dx = x * f64::from(width) - centre.0;
                let dy = y * f64::from(height) - centre.1;
                let mut next = scale * dx.hypot(dy) / from;
                if snap {
                    next = (next * 20.0).round() / 20.0;
                }
                self.stage_guides.clear();
                if self.lanes.snap && *scale > 0.0 {
                    // The edges pull to the same lines a move pulls to - the
                    // frame's, and every other picture's - but here the pull
                    // sets the size, not the place: the scale that lands the
                    // nearest edge on its target, when one is inside reach.
                    let pull = 0.01 * f64::from(width.max(height));
                    let frame = (width, height);
                    let (cx, cy) = (centre.0 / f64::from(width), centre.1 / f64::from(height));
                    let mut xs = vec![0.0, 0.5, 1.0];
                    let mut ys = vec![0.0, 0.5, 1.0];
                    for other in self.stage_clips() {
                        if other.id == *clip {
                            continue;
                        }
                        let footprint = self.footprint(other);
                        let (ow, oh) = footprint.half_bounds(frame);
                        xs.extend([footprint.cx - ow, footprint.cx, footprint.cx + ow]);
                        ys.extend([footprint.cy - oh, footprint.cy, footprint.cy + oh]);
                    }
                    // Four edges: each is the centre plus or minus a half
                    // bound that grows with the scale.
                    let edges = [
                        (-1.0, half.0, cx, &xs, true, width),
                        (1.0, half.0, cx, &xs, true, width),
                        (-1.0, half.1, cy, &ys, false, height),
                        (1.0, half.1, cy, &ys, false, height),
                    ];
                    let mut best: Option<(f64, f64, bool, f64)> = None;
                    for (sign, base, at_centre, targets, vertical, extent) in edges {
                        if base <= 0.0 {
                            continue;
                        }
                        let edge = at_centre + sign * base * next / scale;
                        for &target in targets.iter() {
                            let distance = (target - edge).abs() * f64::from(extent);
                            let wanted = (target - at_centre) * sign;
                            if distance < pull
                                && wanted > 0.0
                                && best.is_none_or(|(held, ..)| distance < held)
                            {
                                best = Some((distance, scale * wanted / base, vertical, target));
                            }
                        }
                    }
                    if let Some((_, snapped, vertical, at)) = best {
                        next = snapped;
                        self.stage_guides.push(StageGuideData {
                            vertical,
                            at: at as f32,
                        });
                    }
                }
                let playhead = self.playhead;
                if let Some(clip) = self.echo_clip_mut(clip) {
                    let at = place_in(clip, playhead);
                    write_keyable(clip, model::KeyProperty::Scale, next.clamp(0.05, 8.0), at);
                }
            }
            Gesture::StageStretch {
                clip,
                across,
                stretch,
                centre,
                from,
                rotation,
            } => {
                let dx = x * f64::from(width) - centre.0;
                let dy = y * f64::from(height) - centre.1;
                let (sin, cos) = rotation.to_radians().sin_cos();
                let along = if *across {
                    dx * cos + dy * sin
                } else {
                    -dx * sin + dy * cos
                };
                let mut next = stretch * along.abs() / from;
                if snap {
                    next = (next * 20.0).round() / 20.0;
                }
                if let Some(clip) = self.echo_clip_mut(clip) {
                    if *across {
                        clip.stretch_x = next.clamp(0.1, 10.0);
                    } else {
                        clip.stretch_y = next.clamp(0.1, 10.0);
                    }
                }
            }
            Gesture::TextWidth {
                clip,
                centre,
                rotation,
            } => {
                let dx = x * f64::from(width) - centre.0;
                let dy = y * f64::from(height) - centre.1;
                let (sin, cos) = rotation.to_radians().sin_cos();
                let along = (dx * cos + dy * sin).abs();
                // Twice the reach, as a fraction of the frame; no narrower
                // than a few percent, or every word stands alone.
                let mut next = (2.0 * along / f64::from(width)).clamp(0.03, 2.0);
                if snap {
                    next = (next * 20.0).round() / 20.0;
                }
                if let Some(clip) = self.echo_clip_mut(clip) {
                    clip.text.get_or_insert_with(TextStyle::default).max_width = next;
                }
            }
            Gesture::TextHeight {
                clip,
                centre,
                rotation,
            } => {
                let dx = x * f64::from(width) - centre.0;
                let dy = y * f64::from(height) - centre.1;
                let (sin, cos) = rotation.to_radians().sin_cos();
                // The pointer's reach across the box's own axis.
                let across = (-dx * sin + dy * cos).abs();
                let mut next = (2.0 * across / f64::from(height)).clamp(0.03, 2.0);
                if snap {
                    next = (next * 20.0).round() / 20.0;
                }
                if let Some(clip) = self.echo_clip_mut(clip) {
                    clip.text.get_or_insert_with(TextStyle::default).max_height = next;
                }
            }
            Gesture::StageMask {
                clip,
                grip,
                frame,
                from,
            } => {
                let to = (x * f64::from(width), y * f64::from(height));
                let mut next = frame.dragged(*grip, *from, to);
                if snap && *grip == MaskGrip::Rotate {
                    next.rotation = (next.rotation / 15.0).round() * 15.0;
                }
                let start = frame.mask;
                let playhead = self.playhead;
                if let Some(clip) = self.echo_clip_mut(clip) {
                    let at = place_in(clip, playhead);
                    // Only the numbers the grip moves: a keyed one the drag
                    // left alone must not gain a key here.
                    for property in model::KeyProperty::MASK {
                        if let (Some(value), Some(was)) = (next.get(property), start.get(property))
                            && value != was
                        {
                            write_keyable(clip, property, value, at);
                        }
                    }
                }
            }
            Gesture::StageRotate {
                clip,
                rotation,
                centre,
                from,
            } => {
                let dx = x * f64::from(width) - centre.0;
                let dy = y * f64::from(height) - centre.1;
                let swept = (dy.atan2(dx) - from).to_degrees();
                let mut next = rotation + swept;
                if snap {
                    next = (next / 15.0).round() * 15.0;
                }
                // Kept in the inspector's range, wrapping rather than
                // stopping: a turn through the bottom carries on.
                next = (next + 180.0).rem_euclid(360.0) - 180.0;
                let playhead = self.playhead;
                if let Some(clip) = self.echo_clip_mut(clip) {
                    let at = place_in(clip, playhead);
                    write_keyable(clip, model::KeyProperty::Rotation, next, at);
                }
            }
            _ => {
                self.gesture = gesture;
                return;
            }
        }
        self.gesture = gesture;
        self.request_preview();
    }

    /// The stage gesture is over: whatever the echo moved becomes one
    /// transform command per picture, batched when there are several.
    pub fn stage_released(&mut self) {
        self.stage_guides.clear();
        let overlay = matches!(self.gesture, Gesture::Paint { .. })
            .then(|| self.stroke_overlay())
            .filter(|(path, _, _)| !path.is_empty());
        let gesture = std::mem::replace(&mut self.gesture, Gesture::None);
        let touched: Vec<String> = match gesture {
            Gesture::StageMove { origins, .. } => {
                origins.into_iter().map(|origin| origin.clip).collect()
            }
            Gesture::StageScale { clip, .. }
            | Gesture::StageRotate { clip, .. }
            | Gesture::StageStretch { clip, .. }
            | Gesture::StageMask { clip, .. }
            | Gesture::TextWidth { clip, .. } => vec![clip],
            Gesture::Paint {
                clip,
                tool,
                size,
                at,
                points,
                ..
            } => {
                // The stroke becomes one command, and one undo step; a
                // smart stroke then has its thing read from the frame, and
                // stays drawn until it has been.
                let stroke = model::Stroke {
                    tool,
                    size,
                    points: points.clone(),
                    at: Some(at),
                };
                self.pending_stroke = stroke.is_smart().then_some(overlay).flatten();
                self.apply(Command::AddCutoutStroke {
                    clip_id: clip,
                    stroke,
                });
                self.ensure_regions();
                return;
            }
            other => {
                // Not ours to end; a lane gesture is still live.
                self.gesture = other;
                return;
            }
        };
        let Some(echo) = self.echo.take() else {
            return;
        };
        let mut commands = Vec::new();
        for id in touched {
            let (Some(after), Some(before)) = (
                echo.active().clip(&id),
                self.session
                    .as_ref()
                    .and_then(|session| session.project().active().clip(&id)),
            ) else {
                continue;
            };
            if after.scale != before.scale
                || after.offset_x != before.offset_x
                || after.offset_y != before.offset_y
                || after.rotation != before.rotation
                || after.stretch_x != before.stretch_x
                || after.stretch_y != before.stretch_y
            {
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
            // A keyed property is dragged as a key at the playhead, not as
            // its constant - see `write_keyable` - so the keys the gesture
            // put on the echo have to be committed too, or the picture
            // snaps back to the ride the document still holds.
            // https://github.com/jub0t/Concat/issues/228
            commands.extend(key_commands(&id, before, after));
            // A mask's grips move its numbers that hold still.
            if after.mask != before.mask {
                commands.push(Command::UpdateClip {
                    clip_id: id.clone(),
                    patch: ClipPatch {
                        mask: Some(after.mask),
                        ..Default::default()
                    },
                });
            }
            // A title's wrap width, from its side grips.
            if after.text != before.text {
                commands.push(Command::UpdateClip {
                    clip_id: id,
                    patch: ClipPatch {
                        text: Some(after.text.clone()),
                        ..Default::default()
                    },
                });
            }
        }
        match commands.len() {
            0 => {
                // A click that moved nothing: the echo is gone and the
                // monitor goes back to the document.
                self.request_preview();
            }
            1 => {
                self.apply(commands.remove(0));
            }
            _ => {
                self.apply(Command::Batch { commands });
            }
        }
    }
}

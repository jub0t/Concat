// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah
//! The stage: what is drawn over the monitor and handled on it.

use super::*;

/// Where on its content time a clip is shown at the playhead, held to its
/// window: where a keyed number is read and written.
fn at_playhead(clip: &Item, playhead: f32) -> Tick {
    doc_view::content_at(clip, doc_view::tick(f64::from(playhead)))
}

/// What one of a clip's numbers shows at `at`: its value, or its keys'.
fn shown_at(clip: &Item, property: KeyProperty, at: Tick) -> f64 {
    doc_view::value_at(doc_view::animated(clip, property), at)
}

/// Writes one of a clip's numbers as a gesture means it: its value while
/// it has no keys, the key at `at` once it rides.
fn write_number(clip: &mut Item, property: KeyProperty, value: f64, at: Tick) {
    doc_view::write_at(doc_view::animated_mut(clip, property), value, at);
}

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
    pub fn footprint(&self, clip: &Item) -> Footprint {
        let (width, height) = self.output_size();
        let (width, height) = (f64::from(width.max(1)), f64::from(height.max(1)));
        let kind = self.kind(clip);
        let scale = clip.picture.scale.value;
        let painted = matches!(kind, Kind::Text | Kind::Shape)
            .then(|| {
                let id = doc_view::id_of(self.doc(), clip)?;
                self.title_blocks.get(&id.to_string()).copied()
            })
            .flatten();
        let (w, h) = if let Some(((bw, bh), _)) = painted {
            // The painter said how big the block came out.
            (
                f64::from(bw) * scale / width,
                f64::from(bh) * scale / height,
            )
        } else if kind == Kind::Text {
            // Not painted yet: a reading of the style until it is. An em is
            // `font_size` of the frame's height, a glyph runs about six
            // tenths of one, and a little air is left around the block the
            // way the plate would.
            let text = doc_view::title(clip).cloned().unwrap_or_default();
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
                (longest * glyph + 0.6 * em) * scale / width,
                (rows * em * text.line_height.max(0.5) + 0.5 * em) * scale / height,
            )
        } else if kind == Kind::Shape {
            // Not painted yet: the figure's box from its style, the way the
            // painter sizes it, until the painter says.
            let (w, h) = doc_view::figure_box(&doc_view::shape(clip).cloned().unwrap_or_default());
            (w * height * scale / width, h * height * scale / height)
        } else {
            let source = doc_view::media_of(self.doc(), clip)
                .and_then(|item| item.size)
                .map(|size| (f64::from(size.width.get()), f64::from(size.height.get())))
                // What is left after the crop is what gets fitted.
                .map(|(w, h)| match clip.picture.crop {
                    Some(crop) => (
                        w * (1.0 - crop.left - crop.right).max(0.1),
                        h * (1.0 - crop.top - crop.bottom).max(0.1),
                    ),
                    None => (w, h),
                });
            match source {
                Some((sw, sh)) => {
                    let fit = (width / sw).min(height / sh);
                    (sw * fit * scale / width, sh * fit * scale / height)
                }
                // Dimensions never learnt: the frame's own shape, which is
                // what the compositor falls back to as well.
                None => (scale, scale),
            }
        };
        // The placement at the playhead: the clip's own, or its ride's
        // where a property is keyed, so the box follows the keys.
        let at = at_playhead(clip, self.playhead);
        let base = concat_core::timeline::Transform {
            scale: shown_at(clip, KeyProperty::Scale, at),
            offset_x: shown_at(clip, KeyProperty::OffsetX, at),
            offset_y: shown_at(clip, KeyProperty::OffsetY, at),
            rotation: shown_at(clip, KeyProperty::Rotation, at),
            stretch_x: shown_at(clip, KeyProperty::StretchX, at),
            stretch_y: shown_at(clip, KeyProperty::StretchY, at),
        };
        // Then pulled along each axis, as the compositor pulls it - a
        // picture, that is. A title's box is its style's, and the compositor
        // never stretches one; see titles.rs.
        // https://github.com/jub0t/Concat/issues/119
        let (w, h) = if kind == Kind::Text {
            (w, h)
        } else {
            (w * base.stretch_x, h * base.stretch_y)
        };
        let placed = base;
        let factor = placed.scale / scale.max(1e-6);
        // A title anchored on an edge paints its block beside the clip's
        // position, not on it; the box goes where the block is. The offset
        // is canvas pixels, so it scales, stretches and turns with the clip
        // before it becomes a fraction of the frame.
        let (ax, ay) = match painted {
            Some((_, (dx, dy))) if dx != 0 || dy != 0 => {
                let px = f64::from(dx) * placed.scale * placed.stretch_x;
                let py = f64::from(dy) * placed.scale * placed.stretch_y;
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
    /// order the overlay draws them in — each with its id as the panels
    /// hold it.
    pub(super) fn stage_clips(&self) -> Vec<(String, &Item)> {
        let doc = self.doc();
        let playhead = doc_view::tick(f64::from(self.playhead));
        // The lanes come bottom first, and each lane's items in time
        // order, so the stack's order is the order they come in.
        doc_view::items(doc)
            .into_iter()
            .filter(|(_, clip)| {
                self.kind(clip).is_visual()
                    && doc_view::lane(doc, clip.lane).is_some_and(|lane| lane.visible)
                    && clip.start <= playhead
                    && playhead < doc_view::end(clip)
            })
            .map(|(id, clip)| (id.to_string(), clip))
            .collect()
    }

    pub fn stage_items(&self) -> Vec<StageItemData> {
        self.stage_clips()
            .into_iter()
            .map(|(id, clip)| {
                let footprint = self.footprint(clip);
                let selected = self.selection.contains(&id);
                let mut item = StageItemData {
                    id: id.as_str().into(),
                    kind: kind_of(self.doc(), clip),
                    selected,
                    cx: footprint.cx as f32,
                    cy: footprint.cy as f32,
                    w: footprint.w as f32,
                    h: footprint.h as f32,
                    rotation: footprint.rotation as f32,
                    scale: clip.picture.scale.value as f32,
                    mask: -1,
                    ..Default::default()
                };
                // The mask's outline and grips, on the one clip selected.
                if selected
                    && self.selection.len() == 1
                    && let Some(frame) = self.mask_frame(clip)
                {
                    let grips = frame.grips().map(|grip| frame.fraction(grip));
                    item.mask = concat_core::mask::MaskShape::ALL
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
    fn mask_frame(&self, clip: &Item) -> Option<MaskFrame> {
        if !self.kind(clip).is_visual() {
            return None;
        }
        let at = at_playhead(clip, self.playhead);
        let mask = masking::MaskAt::of(self.current_mask(clip)?, at)?;
        Some(MaskFrame::new(
            mask,
            &self.footprint(clip),
            self.output_size(),
            (
                shown_at(clip, KeyProperty::StretchX, at),
                shown_at(clip, KeyProperty::StretchY, at),
            ),
        ))
    }

    /// The topmost picture under a frame point, skipping locked lanes the
    /// way a press on the lanes does.
    fn stage_hit(&self, x: f64, y: f64) -> Option<String> {
        let frame = self.output_size();
        self.stage_clips()
            .into_iter()
            .rev()
            .filter(|(_, clip)| !self.lane_locked(clip.lane))
            .find(|(_, clip)| self.footprint(clip).contains(x, y, frame))
            .map(|(id, _)| id)
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
        if let (Some(clip), Some(id)) = (self.paint_target().cloned(), self.sole_selection()) {
            let point = self.stage_to_source(&clip, f64::from(x), f64::from(y));
            self.gesture = Gesture::Paint {
                clip: id,
                tool: BRUSHES[self.brush.min(BRUSHES.len() - 1)].clone(),
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
        let playhead = self.playhead;
        let origins: Vec<StageOrigin> = self
            .stage_clips()
            .into_iter()
            .filter(|(shown, clip)| {
                self.selection.iter().any(|held| held == shown) && !self.lane_locked(clip.lane)
            })
            .map(|(shown, clip)| {
                // A keyed clip is dragged from where its ride has it at
                // the playhead, not from a constant nothing shows.
                let at = at_playhead(clip, playhead);
                StageOrigin {
                    clip: shown,
                    offset_x: shown_at(clip, KeyProperty::OffsetX, at),
                    offset_y: shown_at(clip, KeyProperty::OffsetY, at),
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
        if self.lane_locked(clip.lane) {
            return;
        }
        let kind = self.kind(&clip);
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
        let at = at_playhead(&clip, self.playhead);
        let rotation = clip.picture.rotation.value;
        self.flush_commit();
        self.begin_echo();
        self.gesture = if grip == 4 {
            Gesture::StageRotate {
                clip: id.to_owned(),
                rotation: shown_at(&clip, KeyProperty::Rotation, at),
                centre,
                from: dy.atan2(dx),
            }
        } else if (grip == 6 || grip == 8) && kind == Kind::Text {
            // A title's side grips set where its words wrap, not how wide
            // its glyphs are; see `Gesture::TextWidth`.
            Gesture::TextWidth {
                clip: id.to_owned(),
                centre,
                rotation,
            }
        } else if (grip == 5 || grip == 7) && kind == Kind::Text {
            // And the top and bottom grips size its box, never its glyphs.
            // https://github.com/jub0t/Concat/issues/119
            Gesture::TextHeight {
                clip: id.to_owned(),
                centre,
                rotation,
            }
        } else if grip >= 5 {
            // 5 top, 6 right, 7 bottom, 8 left: the pointer's offset from
            // the centre, turned back into the box's own frame.
            let across = grip == 6 || grip == 8;
            let (sin, cos) = rotation.to_radians().sin_cos();
            let along = if across {
                dx * cos + dy * sin
            } else {
                -dx * sin + dy * cos
            };
            // A shape's side grips pull its own width and height, which
            // keeps its outline even and its corners round; a picture's
            // pull its stretch.
            let figure = doc_view::shape(&clip)
                .filter(|shape| !doc_view::is_line(&shape.kind) || across)
                .map(doc_view::figure_box);
            Gesture::StageStretch {
                clip: id.to_owned(),
                across,
                stretch: match (figure, across) {
                    (Some((w, _)), true) => w,
                    (Some((_, h)), false) => h,
                    // A keyed pull is dragged from where its ride has it.
                    (None, true) => shown_at(&clip, KeyProperty::StretchX, at),
                    (None, false) => shown_at(&clip, KeyProperty::StretchY, at),
                },
                centre,
                from: along.abs().max(1.0),
                rotation,
            }
        } else {
            Gesture::StageScale {
                clip: id.to_owned(),
                scale: shown_at(&clip, KeyProperty::Scale, at),
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
        let playhead = self.playhead;
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
                        for (other_id, other) in self.stage_clips() {
                            if origins.iter().any(|origin| origin.clip == other_id) {
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
                for origin in origins {
                    let (x, y) = (
                        (origin.offset_x + dx).clamp(-1.0, 1.0),
                        (origin.offset_y + dy).clamp(-1.0, 1.0),
                    );
                    self.echo_edit(&origin.clip, |clip| {
                        let at = at_playhead(clip, playhead);
                        write_number(clip, KeyProperty::OffsetX, x, at);
                        write_number(clip, KeyProperty::OffsetY, y, at);
                    });
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
                    for (other_id, other) in self.stage_clips() {
                        if other_id == *clip {
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
                self.echo_edit(clip, |clip| {
                    let at = at_playhead(clip, playhead);
                    write_number(clip, KeyProperty::Scale, next.clamp(0.05, 8.0), at);
                });
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
                let across = *across;
                self.echo_edit(clip, |clip| {
                    let at = at_playhead(clip, playhead);
                    if let Some(shape) = doc_view::shape_mut(clip)
                        && (across || !doc_view::is_line(&shape.kind))
                    {
                        if across {
                            shape.width = next.clamp(0.01, 4.0);
                        } else {
                            shape.height = next.clamp(0.01, 4.0);
                        }
                    } else if across {
                        write_number(clip, KeyProperty::StretchX, next.clamp(0.1, 10.0), at);
                    } else {
                        write_number(clip, KeyProperty::StretchY, next.clamp(0.1, 10.0), at);
                    }
                });
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
                self.echo_edit(clip, |clip| {
                    if let Some(text) = doc_view::title_mut(clip) {
                        text.max_width = next;
                    }
                });
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
                self.echo_edit(clip, |clip| {
                    if let Some(text) = doc_view::title_mut(clip) {
                        text.max_height = next;
                    }
                });
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
                let focus = self.mask_focus.clone();
                self.echo_edit(clip, |clip| {
                    let at = at_playhead(clip, playhead);
                    let Some(index) =
                        masks::current_index(&doc_view::masks(&clip.masks), focus.as_deref())
                    else {
                        return;
                    };
                    let Some(mask) = clip
                        .masks
                        .iter_mut()
                        .filter_map(model::Entry::known_mut)
                        .nth(index)
                    else {
                        return;
                    };
                    // Only the numbers the grip moves: a keyed one the drag
                    // left alone must not gain a key here.
                    for ((number, value), (_, was)) in
                        next.numbers().into_iter().zip(start.numbers())
                    {
                        if value != was
                            && let Some(held) = doc_view::mask_number_mut(mask, number)
                        {
                            doc_view::write_at(held, doc_view::mask_clamp(number, value), at);
                        }
                    }
                });
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
                self.echo_edit(clip, |clip| {
                    let at = at_playhead(clip, playhead);
                    write_number(clip, KeyProperty::Rotation, next, at);
                });
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
            | Gesture::TextWidth { clip, .. }
            | Gesture::TextHeight { clip, .. } => vec![clip],
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
                let smart = matches!(
                    tool,
                    model::BrushTool::SmartBrush | model::BrushTool::SmartEraser
                );
                let stroke = model::Stroke {
                    tool,
                    size,
                    points,
                    at: Some(doc_view::tick(at)),
                };
                self.pending_stroke = smart.then_some(overlay).flatten();
                if let Some(item) = doc_view::parse::<ItemId>(&clip) {
                    self.apply(Command::AddCutoutStroke { item, stroke });
                }
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
            let (Some(item), Some(after), Some(before)) = (
                doc_view::parse::<ItemId>(&id),
                doc_view::item_by(&echo, &id),
                doc_view::item_by(self.committed(), &id),
            ) else {
                continue;
            };
            let (a, b) = (&after.picture, &before.picture);
            let moved = [
                (&a.scale, &b.scale),
                (&a.offset_x, &b.offset_x),
                (&a.offset_y, &b.offset_y),
                (&a.rotation, &b.rotation),
                (&a.stretch_x, &b.stretch_x),
                (&a.stretch_y, &b.stretch_y),
            ]
            .iter()
            .any(|(after, before)| after.value != before.value);
            if moved {
                commands.push(Command::SetClipTransform {
                    item,
                    scale: Some(a.scale.value),
                    offset_x: Some(a.offset_x.value),
                    offset_y: Some(a.offset_y.value),
                    rotation: Some(a.rotation.value),
                    stretch_x: Some(a.stretch_x.value),
                    stretch_y: Some(a.stretch_y.value),
                });
            }
            // A keyed property is dragged as a key at the playhead, not as
            // its constant, so the keys the gesture put on the echo have to
            // be committed too, or the picture snaps back to the ride the
            // document still holds.
            // https://github.com/jub0t/Concat/issues/228
            commands.extend(key_commands(item, before, after));
            // A mask's grips move its numbers, held or keyed: the masks
            // carry their own keys, so the list is the whole of it.
            if after.masks != before.masks {
                commands.push(Command::UpdateClip {
                    item,
                    patch: ItemPatch {
                        masks: Some(doc_view::masks_owned(&after.masks)),
                        ..Default::default()
                    },
                });
            }
            // A shape's width or height, from its side grips.
            if let Some(shape) = doc_view::shape(after)
                && doc_view::shape(before) != Some(shape)
            {
                commands.push(Command::UpdateClip {
                    item,
                    patch: ItemPatch {
                        shape: Some(shape.clone()),
                        ..Default::default()
                    },
                });
            }
            // A title's wrap width or box height, from its grips.
            if let Some(text) = doc_view::title(after)
                && doc_view::title(before) != Some(text)
            {
                commands.push(Command::UpdateClip {
                    item,
                    patch: ItemPatch {
                        title: Some(text.clone()),
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

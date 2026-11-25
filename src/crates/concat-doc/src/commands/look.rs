// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! An item's look and sound: its settings, its keys, its picture's
//! placement and treatments, and a title's words styled apart.

use crate::model::ids::known;
use crate::model::{
    Animated, Background, Content, Cutout, CutoutMode, Ease, EffectId, Entry, Item, ItemId, Key,
    MaskId, MaskSource, MediaKind, Stabilize, Stroke, Subject, Take, TextRun, VoiceMix,
};
use crate::time::{Tick, TickDur};

use super::{
    CommandError, Ed, Framing, HAIR, ItemPatch, KeyProperty, MaskNumber, RunStyle, first_line,
    keep_ids,
};

/// A background as a sequence or an item holds it: a blur held to 0 to 1.
pub(crate) fn tidy_background(background: &Background) -> Background {
    match background {
        Background::Blur { amount } => Background::Blur {
            amount: if amount.is_finite() {
                amount.clamp(0.0, 1.0)
            } else {
                0.0
            },
        },
        other => other.clone(),
    }
}

/// A value held to what an item's number may be.
fn hold(property: KeyProperty, value: f64) -> f64 {
    match property {
        KeyProperty::Scale => value.clamp(0.05, 8.0),
        KeyProperty::StretchX | KeyProperty::StretchY => value.clamp(0.1, 10.0),
        KeyProperty::Opacity => value.clamp(0.0, 1.0),
        KeyProperty::Volume => value.max(0.0),
        KeyProperty::OffsetX | KeyProperty::OffsetY => value.clamp(-3.0, 3.0),
        KeyProperty::Rotation => value,
    }
}

fn property(item: &mut Item, property: KeyProperty) -> &mut Animated<f64> {
    let p = &mut item.picture;
    match property {
        KeyProperty::Scale => &mut p.scale,
        KeyProperty::OffsetX => &mut p.offset_x,
        KeyProperty::OffsetY => &mut p.offset_y,
        KeyProperty::Rotation => &mut p.rotation,
        KeyProperty::StretchX => &mut p.stretch_x,
        KeyProperty::StretchY => &mut p.stretch_y,
        KeyProperty::Opacity => &mut p.opacity,
        KeyProperty::Volume => &mut item.sound.volume,
    }
}

fn hold_mask(number: MaskNumber, value: f64) -> f64 {
    match number {
        MaskNumber::Feather | MaskNumber::Opacity | MaskNumber::Roundness => value.clamp(0.0, 1.0),
        MaskNumber::Scale => value.max(0.01),
        MaskNumber::Width | MaskNumber::Height => value.max(0.0),
        MaskNumber::X | MaskNumber::Y | MaskNumber::Rotation | MaskNumber::Expand => value,
    }
}

/// A key put at `at`, replacing any within a hair of it.
fn put_key<T>(keys: &mut Vec<Key<T>>, at: Tick, value: T, ease: &Ease) {
    take_key(keys, at);
    let place = keys
        .iter()
        .position(|key| key.at > at)
        .unwrap_or(keys.len());
    keys.insert(
        place,
        Key {
            at,
            value,
            ease: ease.clone(),
        },
    );
}

/// The key within a hair of `at` taken off.
fn take_key<T>(keys: &mut Vec<Key<T>>, at: Tick) {
    keys.retain(|key| {
        let off = key.at - at;
        off > HAIR || off < -HAIR
    });
}

/// The rotation brought into (-180, 180].
fn wrap(degrees: f64) -> f64 {
    let mut d = degrees % 360.0;
    if d <= -180.0 {
        d += 360.0;
    } else if d > 180.0 {
        d -= 360.0;
    }
    d
}

/// The effect of an item under an id, wherever the item holds it.
fn effect_mut(item: &mut Item, id: EffectId) -> Option<&mut crate::model::Effect> {
    let chains = item.effects.iter_mut().chain(item.filters.iter_mut());
    for effect in chains.filter_map(Entry::known_mut) {
        if effect.id == id {
            return Some(effect);
        }
    }
    if let Content::Transition(effect) = &mut item.content
        && effect.id == id
    {
        return Some(effect);
    }
    item.cut_in
        .as_mut()
        .map(|c| &mut c.effect)
        .filter(|e| e.id == id)
}

fn mask_number(item: &mut Item, mask: MaskId, number: MaskNumber) -> Option<&mut Animated<f64>> {
    let mask = item
        .masks
        .iter_mut()
        .filter_map(Entry::known_mut)
        .find(|m| m.id == mask)?;
    Some(match number {
        MaskNumber::X => &mut mask.x,
        MaskNumber::Y => &mut mask.y,
        MaskNumber::Scale => &mut mask.scale,
        MaskNumber::Rotation => &mut mask.rotation,
        MaskNumber::Feather => &mut mask.feather,
        MaskNumber::Expand => &mut mask.expand,
        MaskNumber::Opacity => &mut mask.opacity,
        MaskNumber::Width | MaskNumber::Height | MaskNumber::Roundness => match &mut mask.source {
            MaskSource::Shape {
                width,
                height,
                roundness,
                ..
            } => match number {
                MaskNumber::Width => width,
                MaskNumber::Height => height,
                _ => roundness,
            },
            MaskSource::Unknown(_) => return None,
        },
    })
}

/// The runs of a title with `[start, end)` given `style` over what they
/// had, or with `None` given back to the title.
fn restyle(runs: &[TextRun], start: u32, end: u32, style: Option<&RunStyle>) -> Vec<TextRun> {
    let mut edges: Vec<u32> = vec![start, end];
    for run in runs {
        edges.push(run.start);
        edges.push(run.end);
    }
    edges.sort_unstable();
    edges.dedup();
    let mut out: Vec<TextRun> = Vec::new();
    for pair in edges.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let mut piece = runs
            .iter()
            .find(|r| r.start <= a && b <= r.end)
            .cloned()
            .unwrap_or_default();
        piece.start = a;
        piece.end = b;
        if a >= start && b <= end {
            match style {
                Some(s) => {
                    piece.color = s.color.or(piece.color);
                    piece.font_scale = s.font_scale.or(piece.font_scale);
                    piece.font_weight = s.font_weight.or(piece.font_weight);
                    piece.italic = s.italic.or(piece.italic);
                    piece.underline = s.underline.or(piece.underline);
                    piece.strikethrough = s.strikethrough.or(piece.strikethrough);
                    piece.stroke_color = s.stroke_color.or(piece.stroke_color);
                }
                None => {
                    piece = TextRun {
                        start: a,
                        end: b,
                        ..TextRun::default()
                    }
                }
            }
        }
        let plain = TextRun {
            start: a,
            end: b,
            ..TextRun::default()
        };
        if piece == plain {
            continue;
        }
        match out.last_mut() {
            Some(last)
                if last.end == a
                    && TextRun {
                        start: 0,
                        end: 0,
                        ..last.clone()
                    } == TextRun {
                        start: 0,
                        end: 0,
                        ..piece.clone()
                    } =>
            {
                last.end = b;
            }
            _ => out.push(piece),
        }
    }
    out
}

impl Ed<'_> {
    /// Changes an item if it is there; an unknown item is nothing.
    fn touch(&mut self, id: ItemId, change: impl FnOnce(&mut Item)) -> Result<(), CommandError> {
        if self.d.items.contains(id) {
            self.update(id, change)?;
        }
        Ok(())
    }

    /// `updateClip`.
    pub(crate) fn update_clip(
        &mut self,
        id: ItemId,
        patch: &ItemPatch,
    ) -> Result<(), CommandError> {
        let Some(item) = self.d.items.get(id).cloned() else {
            return Ok(());
        };
        let mut patch = patch.clone();
        if let Some(effects) = patch.effects.take() {
            let held: Vec<EffectId> = known(&item.effects).map(|e| e.id).collect();
            patch.effects = Some(keep_ids(self, &held, effects, |e| &mut e.id)?);
        }
        if let Some(filters) = patch.filters.take() {
            let held: Vec<EffectId> = known(&item.filters).map(|e| e.id).collect();
            patch.filters = Some(keep_ids(self, &held, filters, |e| &mut e.id)?);
        }
        if let Some(masks) = patch.masks.take() {
            let held: Vec<MaskId> = known(&item.masks).map(|m| m.id).collect();
            patch.masks = Some(keep_ids(self, &held, masks, |m| &mut m.id)?);
        }
        if let Some(Some(cut)) = &mut patch.cut_in {
            cut.effect.id = match &item.cut_in {
                Some(old) => old.effect.id,
                None => self.mint()?,
            };
        }
        self.update(id, |item| {
            let p = patch;
            if let Some(stream) = p.stream
                && let Content::Media(cut) = &mut item.content
            {
                cut.stream = stream;
            }
            if let Some(background) = p.background {
                item.picture.background = background.as_ref().map(tidy_background);
            }
            if let Some(blend) = p.blend {
                item.picture.blend = blend;
            }
            if let Some(crop) = p.crop {
                item.picture.crop = crop;
            }
            if let Some(fade) = p.fade_in {
                item.sound.fade_in = fade.max(TickDur::ZERO);
            }
            if let Some(fade) = p.fade_out {
                item.sound.fade_out = fade.max(TickDur::ZERO);
            }
            if let Some(filters) = p.filters {
                item.filters = filters.into_iter().map(Entry::Known).collect();
            }
            if let Some(effects) = p.effects {
                item.effects = effects.into_iter().map(Entry::Known).collect();
            }
            if let Some(flip) = p.flip_h {
                item.picture.flip_h = flip;
            }
            if let Some(flip) = p.flip_v {
                item.picture.flip_v = flip;
            }
            if let Some(masks) = p.masks {
                item.masks = masks.into_iter().map(Entry::Known).collect();
            }
            if let Some(motion) = p.motion {
                item.picture.motion = motion;
            }
            if let Some(muted) = p.muted {
                item.sound.muted = muted;
            }
            if let Some(name) = p.name {
                item.name = name;
            }
            if let Some(opacity) = p.opacity.filter(|o| o.is_finite()) {
                item.picture.opacity.value = opacity.clamp(0.0, 1.0);
            }
            if let Some(keep) = p.keep_pitch {
                item.sound.keep_pitch = keep;
            }
            if let Some(shape) = p.shape
                && let Content::Shape(old) = &mut item.content
            {
                *old = shape;
            }
            if let Some(title) = p.title
                && let Content::Title(old) = &mut item.content
            {
                item.name = first_line(&title.content);
                *old = title;
            }
            if let Some(cut_in) = p.cut_in {
                item.cut_in = cut_in;
            }
            if let Some(volume) = p.volume.filter(|v| v.is_finite()) {
                item.sound.volume.value = volume.max(0.0);
            }
        })
    }

    /// `setClipTransform`: offsets, rotation, scale, stretches.
    pub(crate) fn set_transform(
        &mut self,
        id: ItemId,
        values: [Option<f64>; 6],
    ) -> Result<(), CommandError> {
        let order = [
            KeyProperty::OffsetX,
            KeyProperty::OffsetY,
            KeyProperty::Rotation,
            KeyProperty::Scale,
            KeyProperty::StretchX,
            KeyProperty::StretchY,
        ];
        self.touch(id, |item| {
            for (which, value) in order.into_iter().zip(values) {
                if let Some(value) = value.filter(|v| v.is_finite()) {
                    let value = if which == KeyProperty::Rotation {
                        wrap(value)
                    } else {
                        hold(which, value)
                    };
                    property(item, which).value = value;
                }
            }
        })
    }

    /// `frameClip`.
    pub(crate) fn frame_clip(
        &mut self,
        id: ItemId,
        framing: Framing,
        at: Tick,
    ) -> Result<(), CommandError> {
        let Some(item) = self.d.items.get(id).cloned() else {
            return Ok(());
        };
        let Content::Media(cut) = &item.content else {
            return Ok(());
        };
        let Some(file) = self.d.media.get(cut.media) else {
            return Ok(());
        };
        if cut.take != Take::Whole || !matches!(file.kind, MediaKind::Video | MediaKind::Image) {
            return Ok(());
        }
        let Some(size) = file.size else {
            return Ok(());
        };
        let Some(sequence) = self
            .d
            .lanes
            .get(item.lane)
            .and_then(|l| self.d.sequences.get(l.sequence))
        else {
            return Ok(());
        };
        let (fw, fh) = (
            f64::from(sequence.frame.width.get()),
            f64::from(sequence.frame.height.get()),
        );
        let crop = item.picture.crop.unwrap_or(crate::model::Crop {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        });
        let w = f64::from(size.width.get()) * (1.0 - crop.left - crop.right);
        let h = f64::from(size.height.get()) * (1.0 - crop.top - crop.bottom);
        if w <= 0.0 || h <= 0.0 {
            return Ok(());
        }
        // The picture's size before its scale: fitted whole in the frame.
        let fit = (fw / w).min(fh / h);
        let (pw, ph) = (w * fit, h * fit);
        let turn = item.picture.rotation.value.to_radians();
        let (c, s) = (turn.cos().abs(), turn.sin().abs());
        let scale = match framing {
            Framing::Fit => (fw / (pw * c + ph * s)).min(fh / (pw * s + ph * c)),
            Framing::Fill => ((fw * c + fh * s) / pw).max((fw * s + fh * c) / ph),
        };
        let scale = hold(KeyProperty::Scale, scale);
        self.update(id, |item| {
            for (which, value) in [
                (KeyProperty::Scale, scale),
                (KeyProperty::OffsetX, 0.0),
                (KeyProperty::OffsetY, 0.0),
                (KeyProperty::StretchX, 1.0),
                (KeyProperty::StretchY, 1.0),
            ] {
                let number = property(item, which);
                if number.is_keyed() {
                    put_key(&mut number.keys, at, value, &Ease::LINEAR);
                } else {
                    number.value = value;
                }
            }
        })
    }

    /// `setClipKey`.
    pub(crate) fn set_clip_key(
        &mut self,
        id: ItemId,
        which: KeyProperty,
        at: Tick,
        value: f64,
        ease: &Ease,
    ) -> Result<(), CommandError> {
        if !value.is_finite() {
            return super::refuse("A number in that edit is not a number.");
        }
        let value = hold(which, value);
        self.touch(id, |item| {
            put_key(&mut property(item, which).keys, at, value, ease)
        })
    }

    /// `clearClipKey` with a place, `clearClipKeys` without.
    pub(crate) fn clear_clip_key(
        &mut self,
        id: ItemId,
        which: KeyProperty,
        at: Option<Tick>,
    ) -> Result<(), CommandError> {
        self.touch(id, |item| {
            let keys = &mut property(item, which).keys;
            match at {
                Some(at) => take_key(keys, at),
                None => keys.clear(),
            }
        })
    }

    /// `setEffectKey`.
    pub(crate) fn set_effect_key(
        &mut self,
        id: ItemId,
        effect: EffectId,
        param: &str,
        at: Tick,
        value: f64,
        ease: &Ease,
    ) -> Result<(), CommandError> {
        if !value.is_finite() {
            return super::refuse("A number in that edit is not a number.");
        }
        if param.is_empty() {
            return super::refuse("A knob needs a name.");
        }
        self.touch(id, |item| {
            if let Some(effect) = effect_mut(item, effect) {
                let knob = effect
                    .params
                    .entry(param.to_owned())
                    .or_insert_with(|| Animated::fixed(None));
                put_key(&mut knob.keys, at, value, ease);
            }
        })
    }

    /// `clearEffectKey` with a place, `clearEffectKeys` without.
    pub(crate) fn clear_effect_key(
        &mut self,
        id: ItemId,
        effect: EffectId,
        param: &str,
        at: Option<Tick>,
    ) -> Result<(), CommandError> {
        self.touch(id, |item| {
            if let Some(effect) = effect_mut(item, effect)
                && let Some(knob) = effect.params.get_mut(param)
            {
                match at {
                    Some(at) => take_key(&mut knob.keys, at),
                    None => knob.keys.clear(),
                }
                if knob.keys.is_empty() && knob.value.is_none() {
                    effect.params.remove(param);
                }
            }
        })
    }

    /// `setMaskKey`.
    pub(crate) fn set_mask_key(
        &mut self,
        id: ItemId,
        mask: MaskId,
        number: MaskNumber,
        at: Tick,
        value: f64,
        ease: &Ease,
    ) -> Result<(), CommandError> {
        if !value.is_finite() {
            return super::refuse("A number in that edit is not a number.");
        }
        let value = hold_mask(number, value);
        self.touch(id, |item| {
            if let Some(n) = mask_number(item, mask, number) {
                put_key(&mut n.keys, at, value, ease);
            }
        })
    }

    /// `clearMaskKey` with a place, `clearMaskKeys` without.
    pub(crate) fn clear_mask_key(
        &mut self,
        id: ItemId,
        mask: MaskId,
        number: MaskNumber,
        at: Option<Tick>,
    ) -> Result<(), CommandError> {
        self.touch(id, |item| {
            if let Some(n) = mask_number(item, mask, number) {
                match at {
                    Some(at) => take_key(&mut n.keys, at),
                    None => n.keys.clear(),
                }
            }
        })
    }

    /// `setClipCutout`.
    pub(crate) fn set_cutout(
        &mut self,
        id: ItemId,
        cutout: Option<&Cutout>,
    ) -> Result<(), CommandError> {
        let cutout = cutout.map(|c| {
            let mut c = c.clone();
            c.feather = if c.feather.is_finite() {
                c.feather.max(0.0)
            } else {
                0.01
            };
            c.strokes
                .retain(|s| !s.points.is_empty() && s.size.is_finite() && s.size > 0.0);
            c
        });
        self.touch(id, |item| item.picture.cutout = cutout)
    }

    /// `setClipStabilize`.
    pub(crate) fn set_stabilize(
        &mut self,
        id: ItemId,
        stabilize: Option<&Stabilize>,
    ) -> Result<(), CommandError> {
        let stabilize = stabilize.map(|s| {
            let mut s = s.clone();
            s.smoothness = if s.smoothness.is_finite() {
                s.smoothness.clamp(0.0, 1.0)
            } else {
                0.5
            };
            s.max_zoom = if s.max_zoom.is_finite() {
                s.max_zoom.clamp(1.0, 4.0)
            } else {
                1.2
            };
            s
        });
        self.touch(id, |item| item.picture.stabilize = stabilize)
    }

    /// `setClipVoice`: nothing for an item with no sound.
    pub(crate) fn set_voice(
        &mut self,
        id: ItemId,
        voice: Option<&VoiceMix>,
    ) -> Result<(), CommandError> {
        let Some(item) = self.d.items.get(id) else {
            return Ok(());
        };
        let sounds = match &item.content {
            Content::Media(cut) => self
                .d
                .media
                .get(cut.media)
                .is_some_and(|m| m.has_sound && m.kind != MediaKind::Image),
            Content::Nest(_) => true,
            _ => false,
        };
        if !sounds {
            return Ok(());
        }
        let voice = voice.map(|v| {
            let mut v = v.clone();
            let gain = |g: f64| {
                if g.is_finite() {
                    g.clamp(0.0, 4.0)
                } else {
                    1.0
                }
            };
            v.voice = gain(v.voice);
            v.background = gain(v.background);
            v
        });
        self.update(id, |item| item.sound.voice = voice)
    }

    /// `addCutoutStroke`.
    pub(crate) fn add_stroke(&mut self, id: ItemId, stroke: &Stroke) -> Result<(), CommandError> {
        if stroke.points.is_empty() || !stroke.size.is_finite() || stroke.size <= 0.0 {
            return Ok(());
        }
        let stroke = stroke.clone();
        self.touch(id, |item| {
            let cutout = item.picture.cutout.get_or_insert_with(|| Cutout {
                mode: CutoutMode::Custom,
                subject: Subject::Auto,
                feather: 0.01,
                strokes: Vec::new(),
            });
            cutout.mode = CutoutMode::Custom;
            cutout.strokes.push(stroke);
        })
    }

    /// `styleTextRange`.
    pub(crate) fn style_text(
        &mut self,
        id: ItemId,
        start: u32,
        end: u32,
        style: &RunStyle,
    ) -> Result<(), CommandError> {
        if style.is_empty() {
            return Ok(());
        }
        self.touch(id, |item| {
            if let Content::Title(title) = &mut item.content {
                let count = title.content.chars().count() as u32;
                let (start, end) = (start.min(count), end.min(count));
                if start < end {
                    title.runs = restyle(&title.runs, start, end, Some(style));
                }
            }
        })
    }

    /// `clearTextStyles`.
    pub(crate) fn clear_text_styles(
        &mut self,
        id: ItemId,
        start: Option<u32>,
        end: Option<u32>,
    ) -> Result<(), CommandError> {
        self.touch(id, |item| {
            if let Content::Title(title) = &mut item.content {
                let count = title.content.chars().count() as u32;
                let (start, end) = match (start, end) {
                    (Some(s), Some(e)) => (s.min(count), e.min(count)),
                    _ => (0, count),
                };
                if start < end {
                    title.runs = restyle(&title.runs, start, end, None);
                } else if start == 0 && end == 0 {
                    title.runs.clear();
                }
            }
        })
    }
}

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
        if self.mask_set(field, f64::from(value)) {
            return;
        }
        // A title's look on a caption is every caption's on its lane.
        let mut ids = if text_style_field(field) {
            self.style_targets()
        } else {
            self.edit_targets()
        };
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
                self.clip(&ids[0]).map(|clip| {
                    let at = doc_view::content_at(clip, doc_view::tick(f64::from(self.playhead)));
                    shown(clip, property, at)
                })
            });
        for id in ids {
            let Some(clip) = self.clip(&id) else {
                continue;
            };
            let kind = self.kind(clip);
            // The file's sound streams: a row of the Audio panel's list is a
            // stream of the file.
            let streams: Vec<u32> = if field == ClipField::AudioTrack {
                doc_view::media_of(self.doc(), clip)
                    .map(|media| media.streams.iter().map(|stream| stream.index).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            // Where the playhead sits in the clip's content: a keyed number
            // is written there, as a key.
            let at = doc_view::content_at(clip, doc_view::tick(f64::from(self.playhead)));
            let value = match (reference, relative_property(field)) {
                (Some(reference), Some(property)) => {
                    batch_value(field, value, reference, shown(clip, property, at))
                }
                _ => value,
            };
            let steadied_anew =
                (field == ClipField::Stabilize && value != 0.0 && clip.picture.stabilize.is_none())
                    .then(|| doc_view::media_id(clip))
                    .flatten();
            if let Some(media) = steadied_anew {
                // Turned on anew: a measurement the app died in is tried
                // again, since the person asked.
                self.stabilize_retry.insert(media.to_string());
                self.failed_analyses.clear();
            }
            // The clip as the document has it, for a batch level: a drag's
            // every step is read against where it started, not against
            // the last step, so a ride's shape is not worn down on the way.
            let original = batch_volume
                .then(|| doc_view::item_by(self.committed(), &id).cloned())
                .flatten();
            self.echo_edit(&id, |clip| {
                if let Some(original) = &original {
                    crate::batch_volume::set_level(clip, original, value, at);
                    return;
                }
                set_field(clip, kind, field, value, at, &streams);
            });
        }
    }

    pub fn clip_set_text(&mut self, field: ClipTextField, value: &str) {
        self.set_text_at(field, value, None);
    }

    /// The words typed into the Text tab's box, with where the caret ended
    /// up - a byte into `value`, as the box counts - so a run beside the
    /// edit knows whether the new letters are its own; see `set_words`.
    pub fn clip_set_words(&mut self, value: &str, caret: i32) {
        let caret = (caret >= 0).then(|| char_at(value, caret));
        self.set_text_at(ClipTextField::Content, value, caret);
    }

    fn set_text_at(&mut self, field: ClipTextField, value: &str, caret: Option<usize>) {
        // The words are the caption's own; its face is its lane's.
        let ids = if field == ClipTextField::Content {
            self.edit_targets()
        } else {
            self.style_targets()
        };
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
            if self.kind_by(&id) != Kind::Text {
                continue;
            }
            self.echo_edit(&id, |clip| {
                let Some(text) = doc_view::title_mut(clip) else {
                    return;
                };
                match field {
                    ClipTextField::Content => {
                        // The runs ride along with the words they are on.
                        set_words(text, value.to_owned(), caret);
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
            });
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

/// Whether `field` is part of a title's look - what a caption shares with
/// every caption on its lane - rather than its placement, timing or chain.
fn text_style_field(field: ClipField) -> bool {
    matches!(
        field,
        ClipField::FontSize
            | ClipField::FontWeight
            | ClipField::Italic
            | ClipField::Underline
            | ClipField::Strikethrough
            | ClipField::TextCase
            | ClipField::TextOpacity
            | ClipField::Align
            | ClipField::StrokeWidth
            | ClipField::Shadow
            | ClipField::LineHeight
            | ClipField::Tracking
            | ClipField::TextWidth
            | ClipField::TextHeight
            | ClipField::StrokeOpacity
            | ClipField::BackgroundOpacity
            | ClipField::BackgroundRadius
            | ClipField::BackgroundPaddingX
            | ClipField::BackgroundPaddingY
    )
}

/// One of the inspector's fields onto `clip`, an item of `kind` on the
/// echo, `at` the playhead's place on its content time - where a keyed
/// number is written, as a key - and `streams` its file's sound streams,
/// for the Audio panel's list.
fn set_field(clip: &mut Item, kind: Kind, field: ClipField, value: f64, at: Tick, streams: &[u32]) {
    match field {
        // The Mask section's, which `mask_set` took before this.
        ClipField::MaskShape
        | ClipField::MaskInvert
        | ClipField::MaskMode
        | ClipField::MaskAdd
        | ClipField::MaskSelect
        | ClipField::MaskRemove
        | ClipField::MaskMove
        | ClipField::MaskX
        | ClipField::MaskY
        | ClipField::MaskWidth
        | ClipField::MaskHeight
        | ClipField::MaskRotation
        | ClipField::MaskRoundness
        | ClipField::MaskFeather
        | ClipField::MaskExpand
        | ClipField::MaskOpacity => {}
        ClipField::Scale => write_keyable(clip, KeyProperty::Scale, value.clamp(0.05, 8.0), at),
        ClipField::AudioTrack => {
            // The first row is the file's default and is stored as such, so
            // a clip on the first stream saves as one that never chose.
            let row = value.max(0.0) as usize;
            if let Content::Media(cut) = &mut clip.content {
                cut.stream = (row > 0).then(|| streams.get(row).copied()).flatten();
            }
        }
        ClipField::StretchX => {
            write_keyable(clip, KeyProperty::StretchX, value.clamp(0.1, 10.0), at)
        }
        ClipField::StretchY => {
            write_keyable(clip, KeyProperty::StretchY, value.clamp(0.1, 10.0), at)
        }
        ClipField::OffsetX => write_keyable(clip, KeyProperty::OffsetX, value.clamp(-1.0, 1.0), at),
        ClipField::OffsetY => write_keyable(clip, KeyProperty::OffsetY, value.clamp(-1.0, 1.0), at),
        ClipField::Rotation => {
            write_keyable(clip, KeyProperty::Rotation, value.clamp(-180.0, 180.0), at)
        }
        ClipField::Opacity => write_keyable(clip, KeyProperty::Opacity, value.clamp(0.0, 1.0), at),
        ClipField::CutoutFeather => {
            clip.picture
                .cutout
                .get_or_insert_with(doc_view::cutout_auto)
                .feather = value.clamp(0.0, doc_view::CUTOUT_FEATHER_MAX);
        }
        // Only footage has a camera to steady.
        ClipField::Stabilize => {
            let held = clip.picture.stabilize.clone();
            clip.picture.stabilize = (value != 0.0 && kind == Kind::Video)
                .then(|| held.unwrap_or_else(doc_view::stabilize_default));
        }
        ClipField::StabilizeSmoothness => {
            if let Some(stabilize) = clip.picture.stabilize.as_mut() {
                stabilize.smoothness = value.clamp(0.0, 1.0);
            }
        }
        ClipField::StabilizeMaxZoom => {
            if let Some(stabilize) = clip.picture.stabilize.as_mut() {
                stabilize.max_zoom = value.clamp(1.0, doc_view::STABILIZE_ZOOM_MAX);
            }
        }
        ClipField::StabilizeTripod => {
            if let Some(stabilize) = clip.picture.stabilize.as_mut() {
                stabilize.tripod = value != 0.0;
            }
        }
        // Only what plays sound has a voice in it. A new way keeps the kind
        // the clip had.
        ClipField::VoiceMode => {
            let sounding = matches!(kind, Kind::Video | Kind::Audio);
            let voice_kind = clip
                .sound
                .voice
                .as_ref()
                .map_or(model::VoiceKind::Speech, |mix| mix.kind.clone());
            clip.sound.voice = match value as i32 {
                1 if sounding => Some(doc_view::keep_voice(voice_kind)),
                2 if sounding => Some(doc_view::remove_voice(voice_kind)),
                _ => None,
            };
        }
        ClipField::VoiceKind => {
            if let Some(mix) = clip.sound.voice.as_mut() {
                mix.kind = if value as i32 == 1 {
                    model::VoiceKind::Speech
                } else {
                    model::VoiceKind::Vocals
                };
            }
        }
        ClipField::VoiceLevel => {
            if let Some(mix) = clip.sound.voice.as_mut() {
                mix.voice = value.clamp(0.0, doc_view::VOICE_GAIN_MAX);
            }
        }
        ClipField::BackgroundLevel => {
            if let Some(mix) = clip.sound.voice.as_mut() {
                mix.background = value.clamp(0.0, doc_view::VOICE_GAIN_MAX);
            }
        }
        ClipField::MotionIn
        | ClipField::MotionInSeconds
        | ClipField::MotionOut
        | ClipField::MotionOutSeconds
        | ClipField::MotionLoop
        | ClipField::MotionLoopSeconds
        | ClipField::MotionLoopAmount => set_motion(&mut clip.picture.motion, field, value),
        ClipField::Volume => write_keyable(clip, KeyProperty::Volume, value.max(0.0), at),
        ClipField::Speed => {
            // A rate set by hand is a constant rate, so a curve the clip had
            // ends; the footage it covered is kept.
            let speed = value.clamp(crate::speed_ramp::MIN_SPEED, crate::speed_ramp::MAX_SPEED);
            if let Some((from, span)) = doc_view::covered(clip) {
                let sign = if span < TickDur::ZERO { -1.0 } else { 1.0 };
                let ticks = ((span.get().abs() as f64 / speed).round() as i64)
                    .max(concat_doc::commands::MIN_LEN.get());
                if let (Some(map), Some(len)) = (
                    super::chains::linear_map(clip, from, sign * speed),
                    TickLen::new(ticks),
                ) {
                    if let Some(time) = super::chains::time_map_mut(clip) {
                        *time = map;
                    }
                    clip.len = len;
                }
            }
        }
        ClipField::PreservePitch => clip.sound.keep_pitch = value != 0.0,
        ClipField::FlipH => clip.picture.flip_h = value != 0.0,
        ClipField::FlipV => clip.picture.flip_v = value != 0.0,
        ClipField::Blend => {
            let mode = concat_core::Blend::ALL
                .get(value.max(0.0) as usize)
                .copied()
                .unwrap_or_default();
            clip.picture.blend = doc_view::doc_blend(mode);
        }
        ClipField::CropLeft | ClipField::CropTop | ClipField::CropRight | ClipField::CropBottom => {
            let mut crop = clip.picture.crop.unwrap_or(model::Crop {
                left: 0.0,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
            });
            let edge = match field {
                ClipField::CropLeft => &mut crop.left,
                ClipField::CropTop => &mut crop.top,
                ClipField::CropRight => &mut crop.right,
                _ => &mut crop.bottom,
            };
            *edge = value.clamp(0.0, 0.9);
            clip.picture.crop = tidy_crop(crop);
        }
        ClipField::FadeIn => {
            let half = doc_view::len_s(clip) / 2.0;
            clip.sound.fade_in = doc_view::dur(value.clamp(0.0, half));
        }
        ClipField::FadeOut => {
            let half = doc_view::len_s(clip) / 2.0;
            clip.sound.fade_out = doc_view::dur(value.clamp(0.0, half));
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
            if let Some(shape) = doc_view::shape_mut(clip) {
                set_shape(shape, field, value);
            }
        }
        // The rest are a title's look; nothing else has one.
        field => {
            if let Some(text) = doc_view::title_mut(clip) {
                set_title(text, field, value);
            }
        }
    }
}

/// A crop as an item holds it: two opposite edges always leaving some of
/// the picture between them, and none at all when nothing is cut.
fn tidy_crop(mut crop: model::Crop) -> Option<model::Crop> {
    const MOST: f64 = 0.95;
    for (a, b) in [
        (&mut crop.left, &mut crop.right),
        (&mut crop.top, &mut crop.bottom),
    ] {
        let both = *a + *b;
        if both > MOST {
            *a *= MOST / both;
            *b *= MOST / both;
        }
    }
    let none = [crop.left, crop.top, crop.right, crop.bottom]
        .iter()
        .all(|edge| *edge <= 0.0);
    (!none).then_some(crop)
}

/// One of the Animate cards' fields onto an item's motion: a move or a loop
/// picked by its place in the card's list - none past it - or its length,
/// or a loop's amount.
fn set_motion(motion: &mut model::Motion, field: ClipField, value: f64) {
    let place = usize::try_from(value.round() as i64).ok();
    let length = |seconds: f64| doc_view::len(seconds.clamp(0.05, 60.0));
    match field {
        ClipField::MotionIn | ClipField::MotionOut => {
            let held = if field == ClipField::MotionIn {
                &mut motion.enter
            } else {
                &mut motion.exit
            };
            let len = held.as_ref().map_or_else(|| length(0.5), |held| held.len);
            *held = place
                .and_then(|place| concat_core::motion::Move::ALL.get(place))
                .map(|kind| model::Move {
                    name: kind.name().to_owned(),
                    len,
                });
        }
        ClipField::MotionLoop => {
            let (lap, amount) = motion
                .repeat
                .as_ref()
                .map_or((length(1.0), 1.0), |held| (held.lap, held.amount));
            motion.repeat = place
                .and_then(|place| concat_core::motion::Cycle::ALL.get(place))
                .map(|kind| model::Loop {
                    name: kind.name().to_owned(),
                    lap,
                    amount,
                });
        }
        ClipField::MotionInSeconds => {
            if let Some(held) = motion.enter.as_mut() {
                held.len = length(value);
            }
        }
        ClipField::MotionOutSeconds => {
            if let Some(held) = motion.exit.as_mut() {
                held.len = length(value);
            }
        }
        ClipField::MotionLoopSeconds => {
            if let Some(held) = motion.repeat.as_mut() {
                held.lap = length(value);
            }
        }
        _ => {
            if let Some(held) = motion.repeat.as_mut() {
                held.amount = value.clamp(0.0, 2.0);
            }
        }
    }
}

/// One of the Text tab's numbers onto a title's look.
fn set_title(text: &mut Title, field: ClipField, value: f64) {
    match field {
        ClipField::FontSize => text.font_size = value.clamp(0.01, 0.5),
        ClipField::FontWeight => text.font_weight = value.clamp(100.0, 900.0),
        ClipField::Italic => text.italic = value != 0.0,
        ClipField::Underline => text.underline = value != 0.0,
        ClipField::Strikethrough => text.strikethrough = value != 0.0,
        ClipField::TextCase => {
            text.case = match value as i32 {
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
        // The outline's opacity is its colour's alpha.
        ClipField::StrokeOpacity => {
            text.stroke_color.a = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        // The plate's opacity is its colour's alpha the same way, never zero
        // from the dial: the switch is what takes the plate away.
        ClipField::BackgroundOpacity => {
            let plate = text.plate.unwrap_or(model::Color {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            });
            text.plate = Some(model::Color {
                a: (value.clamp(0.01, 1.0) * 255.0).round() as u8,
                ..plate
            });
        }
        ClipField::BackgroundRadius => text.plate_radius = value.clamp(0.0, 0.2),
        ClipField::BackgroundPaddingX => text.plate_padding_x = value.clamp(0.0, 0.5),
        ClipField::BackgroundPaddingY => text.plate_padding_y = value.clamp(0.0, 0.5),
        _ => {}
    }
}

/// New words for a title, its styled stretches riding along with the words
/// they are on. The words that changed are what lies between the longest
/// start and the longest end the old and new words share: a stretch before
/// them stays, one after them moves by as much as the words grew, one they
/// cut into keeps what is left of it. Letters typed at the end of a
/// stretch are its own - the way typing on from styled words goes on in
/// their style - unless the caret, a character into the new words, says
/// they were put down in front of it instead.
fn set_words(text: &mut Title, words: String, caret: Option<usize>) {
    let old: Vec<char> = text.content.chars().collect();
    let new: Vec<char> = words.chars().collect();
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - head;
    let tail = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    let (cut_end, typed_end) = (old.len() - tail, new.len() - tail);
    let typed = typed_end - head;
    // Typing in front of a stretch's start: the caret stops before it.
    let in_front = caret.is_some_and(|caret| caret == head);
    // Letters put down between two others, nothing taken away.
    let inserted = cut_end == head && typed > 0;
    let moved = |at: usize, end: bool| -> usize {
        if at < head {
            at
        } else if at == head && inserted {
            // An end there takes the letters, a start goes on past them.
            if end && in_front { at } else { typed_end }
        } else if at >= cut_end {
            at - cut_end + typed_end
        } else if end {
            typed_end
        } else {
            head
        }
    };
    let runs = std::mem::take(&mut text.runs)
        .into_iter()
        .map(|mut run| {
            let (start, end) = (run.start as usize, run.end as usize);
            run.start = u32::try_from(moved(start, false)).unwrap_or(u32::MAX);
            run.end = u32::try_from(moved(end, true)).unwrap_or(u32::MAX);
            run
        })
        .collect();
    text.runs = crate::presets::tidy_runs(runs, &words);
    text.content = words;
}

/// One of the Shape section's numbers onto `shape`. A switch turned on
/// brings back a visible default rather than an invisible one: a fill in
/// white, an outline in white a little over a pixel thick, a soft shadow.
fn set_shape(shape: &mut model::ShapeStyle, field: ClipField, value: f64) {
    let index = value.max(0.0).round() as usize;
    match field {
        ClipField::ShapeKind => {
            if let Some(kind) = doc_view::SHAPE_KINDS.get(index) {
                shape.kind = kind.clone();
            }
        }
        ClipField::ShapeWidth => shape.width = value.clamp(0.01, 4.0),
        ClipField::ShapeHeight => shape.height = value.clamp(0.01, 4.0),
        ClipField::ShapeRadius => shape.corner_radius = value.clamp(0.0, 1.0),
        ClipField::ShapeSides => shape.sides = (value.round() as u32).clamp(3, 12),
        ClipField::ShapePoints => shape.points = (value.round() as u32).clamp(3, 24),
        ClipField::ShapeInner => shape.inner_radius = value.clamp(0.05, 0.95),
        ClipField::ShapeFillOn => {
            shape.fill = (value != 0.0).then(|| shape.fill.unwrap_or(model::Color::WHITE));
        }
        ClipField::ShapeGradient => {
            shape.gradient = match index {
                0 => None,
                _ => Some(model::Gradient {
                    radial: index == 2,
                    ..shape.gradient.clone().unwrap_or(model::Gradient {
                        color: model::Color::BLACK,
                        angle: 90.0,
                        radial: false,
                    })
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
                shape.stroke.get_or_insert(model::Color::WHITE);
                if shape.stroke_width <= 0.0 {
                    shape.stroke_width = 0.012;
                }
            } else {
                shape.stroke = None;
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
            shape.start_head = doc_view::LINE_HEADS.get(index).cloned();
        }
        ClipField::ShapeEndHead => {
            shape.end_head = doc_view::LINE_HEADS.get(index).cloned();
        }
        // Each end held on its own side of the other.
        ClipField::ShapeTrimStart => {
            shape.trim_start = value.clamp(0.0, 1.0).min(shape.trim_end);
        }
        ClipField::ShapeTrimEnd => {
            shape.trim_end = value.clamp(0.0, 1.0).max(shape.trim_start);
        }
        ClipField::ShapeShadowOn => {
            shape.shadow = (value != 0.0).then(|| {
                shape.shadow.clone().unwrap_or(model::ShapeShadow {
                    color: model::Color {
                        r: 0,
                        g: 0,
                        b: 0,
                        a: 128,
                    },
                    angle: 90.0,
                    distance: 0.012,
                    blur: 0.02,
                })
            });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn titled(words: &str, runs: &[(u32, u32)]) -> Title {
        Title {
            content: words.to_owned(),
            runs: runs
                .iter()
                .map(|&(start, end)| model::TextRun {
                    start,
                    end,
                    italic: Some(true),
                    ..model::TextRun::default()
                })
                .collect(),
            ..Title::default()
        }
    }

    fn spans(text: &Title) -> Vec<(u32, u32)> {
        text.runs.iter().map(|run| (run.start, run.end)).collect()
    }

    #[test]
    fn styled_words_ride_along_with_what_is_typed_around_them() {
        // "big" styled in "a big cat".
        let mut text = titled("a big cat", &[(2, 5)]);
        set_words(&mut text, "oh a big cat".to_owned(), Some(3));
        assert_eq!(spans(&text), vec![(5, 8)], "typed before it moves it");
        set_words(&mut text, "oh a bigger cat".to_owned(), Some(11));
        assert_eq!(
            spans(&text),
            vec![(5, 11)],
            "typed on from its end is its own"
        );
        set_words(&mut text, "oh a bigger cat!".to_owned(), Some(16));
        assert_eq!(spans(&text), vec![(5, 11)], "typed after it leaves it");
        set_words(&mut text, "oh a bier cat!".to_owned(), Some(7));
        assert_eq!(spans(&text), vec![(5, 9)], "a cut inside keeps the rest");
        set_words(&mut text, "oh cat!".to_owned(), Some(3));
        assert!(text.runs.is_empty(), "words cut away take their style");
        // Typed at the very start of a stretch: in front of it.
        let mut text = titled("a big", &[(2, 5)]);
        set_words(&mut text, "a xbig".to_owned(), Some(3));
        assert_eq!(spans(&text), vec![(3, 6)]);
    }

    #[test]
    fn a_crop_keeps_some_picture_and_none_is_no_crop() {
        let none = model::Crop {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        };
        assert_eq!(tidy_crop(none), None);
        let tight = tidy_crop(model::Crop {
            left: 0.9,
            right: 0.9,
            ..none
        })
        .expect("a crop");
        assert!(tight.left + tight.right <= 0.95 + 1e-9);
    }
}

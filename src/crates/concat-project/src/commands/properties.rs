// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! What a placed clip looks and sounds like: its patchable fields, speed, transform, keys and cutout.
//!
//! One arm per command, exactly as [`super::apply`] routes them here;
//! everything these arms share lives in the parent module.

use super::*;

/// Applies one of this module's commands. Any other is a routing error.
pub(super) fn apply(
    project: &mut Project,
    _mint: &mut IdMint,
    command: Command,
) -> Result<Outcome, CommandError> {
    match command {
        Command::UpdateClip { clip_id, patch } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            let mut applied = false;
            if let Some(name) = patch.name {
                applied |= assign(&mut clip.name, name);
            }
            if let Some(volume) = patch.volume {
                applied |= assign(&mut clip.volume, volume.max(0.0));
            }
            if let Some(fade_in) = patch.fade_in {
                applied |= assign(&mut clip.fade_in, fade_in.max(0.0));
            }
            if let Some(fade_out) = patch.fade_out {
                applied |= assign(&mut clip.fade_out, fade_out.max(0.0));
            }
            if let Some(opacity) = patch.opacity {
                applied |= assign(&mut clip.opacity, opacity.clamp(0.0, 1.0));
            }
            if let Some(preserve) = patch.preserve_pitch {
                applied |= assign(&mut clip.preserve_pitch, preserve);
            }
            if let Some(muted) = patch.muted {
                // Unmuted is the absent value, so a document never carries
                // a `muted: false` that means the same as nothing.
                applied |= assign(&mut clip.muted, muted.then_some(true));
            }
            if let Some(flip) = patch.flip_h {
                applied |= assign(&mut clip.flip_h, flip);
            }
            if let Some(flip) = patch.flip_v {
                applied |= assign(&mut clip.flip_v, flip);
            }
            if let Some(blend) = patch.blend {
                let blend = if blend == "normal" {
                    String::new()
                } else {
                    blend
                };
                applied |= assign(&mut clip.blend, blend);
            }
            if let Some(crop) = patch.crop {
                let crop = crop.map(Crop::tidy).filter(|crop| !crop.is_none());
                applied |= assign(&mut clip.crop, crop);
            }
            if let Some(masks) = patch.masks {
                applied |= assign(&mut clip.masks, Mask::tidy_all(masks));
            }
            if let Some(motion) = patch.motion {
                applied |= assign(&mut clip.motion, motion.and_then(ClipMotion::tidy));
            }
            if let Some(filters) = patch.filters {
                applied |= assign(&mut clip.filters, filters);
            }
            if let Some(effects) = patch.video_effects {
                applied |= assign(&mut clip.video_effects, effects);
            }
            if let Some(transition) = patch.transition_in {
                applied |= assign(&mut clip.transition_in, transition);
            }
            if let Some(text) = patch.text {
                // The name follows the words, like addTextClip snapshots it.
                if let Some(style) = &text {
                    applied |= assign(&mut clip.name, first_line(&style.content));
                }
                applied |= assign(&mut clip.text, text);
            }
            if let Some(shape) = patch.shape
                && clip.kind == crate::model::ClipKind::Shape
            {
                applied |= assign(&mut clip.shape, Some(shape.tidy()));
            }
            if let Some(stream) = patch.audio_stream {
                applied |= assign(&mut clip.audio_stream, stream);
            }
            if let Some(background) = patch.background {
                applied |= assign(&mut clip.background, background.map(Background::tidy));
            }
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::StyleTextRange {
            clip_id,
            start,
            end,
            patch,
        } => {
            let Some(clip) = project.active_mut().clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            if clip.kind != ClipKind::Text {
                return Ok(Outcome::default());
            }
            let Some(text) = clip.text.as_mut() else {
                return Ok(Outcome::default());
            };
            let before = text.runs.clone();
            text.style_range(start, end, &patch);
            Ok(Outcome {
                created_id: None,
                applied: text.runs != before,
            })
        }

        Command::ClearTextStyles {
            clip_id,
            start,
            end,
        } => {
            let Some(clip) = project.active_mut().clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            if clip.kind != ClipKind::Text {
                return Ok(Outcome::default());
            }
            let Some(text) = clip.text.as_mut() else {
                return Ok(Outcome::default());
            };
            let before = text.runs.clone();
            match (start, end) {
                (Some(start), Some(end)) => text.clear_range(start, end),
                _ => text.runs.clear(),
            }
            Ok(Outcome {
                created_id: None,
                applied: text.runs != before,
            })
        }

        Command::SetClipSpeed {
            clip_id,
            speed,
            ripple,
        } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            // The amount of source covered is held constant - that is what
            // makes this a speed change rather than a trim. Under a curve
            // the rate is its mean, so the curve's footage is what is kept.
            let next = speed.clamp(MIN_SPEED, MAX_SPEED);
            let source_covered = clip.duration * clip.speed;
            let old_end = clip.start + clip.duration;
            // Bitwise so no assignment is short-circuited away. A rate set
            // by hand is a constant rate: the curve goes.
            let applied = assign(&mut clip.speed, next)
                | assign(
                    &mut clip.duration,
                    (source_covered / next).max(MIN_CLIP_DURATION),
                )
                | assign(&mut clip.speed_curve, None);
            if applied {
                shift_followers(timeline, &clip_id, old_end, ripple);
            }
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SetClipCutout { clip_id, cutout } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            let applied = assign(&mut clip.cutout, cutout.map(Cutout::tidy));
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SetClipStabilize { clip_id, stabilize } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            // Only footage has a camera that moved: a still, a title, a
            // shape or a layer has nothing to steady.
            if clip.kind != ClipKind::Video && stabilize.is_some() {
                return Ok(Outcome::default());
            }
            let applied = assign(&mut clip.stabilize, stabilize.map(Stabilize::tidy));
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SetClipVoice { clip_id, voice } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            // Only what plays sound has a voice in it: footage and sound.
            if !matches!(clip.kind, ClipKind::Video | ClipKind::Audio) && voice.is_some() {
                return Ok(Outcome::default());
            }
            let applied = assign(&mut clip.voice, voice.map(VoiceMix::tidy));
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::AddCutoutStroke { clip_id, stroke } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            let Some(stroke) = stroke.tidy() else {
                return Ok(Outcome::default());
            };
            let cutout = clip.cutout.get_or_insert_with(Cutout::auto);
            cutout.mode = CutoutMode::Custom;
            cutout.strokes.push(stroke);
            Ok(Outcome {
                created_id: None,
                applied: true,
            })
        }

        Command::SetClipKey {
            clip_id,
            property,
            at,
            value,
            ease,
        } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            if !at.is_finite() || !value.is_finite() {
                return Ok(Outcome::default());
            }
            // Clamped the way the field itself is, so a key can never hold a
            // value the constant would have been refused. The clamps are the
            // ones ClipPatch applies; keeping them here as well is what stops
            // a key being the back door round them.
            let value = match property {
                KeyProperty::Scale => value.clamp(MIN_SCALE, MAX_SCALE),
                KeyProperty::Opacity => value.clamp(0.0, 1.0),
                // No ceiling, matching ClipPatch: the level fader goes to
                // +24 dB because quiet material needs it, and a key is the
                // same value at a different instant.
                KeyProperty::Volume => value.max(0.0),
                KeyProperty::OffsetX | KeyProperty::OffsetY | KeyProperty::Rotation => value,
            };
            let before = clip.keys.clone();
            clip.set_key(property, at, value, ease);
            Ok(Outcome {
                created_id: None,
                applied: clip.keys != before,
            })
        }

        Command::ClearClipKey {
            clip_id,
            property,
            at,
        } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            let applied = clip.clear_key(property, at);
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::ClearClipKeys { clip_id, property } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            let applied = clip.clear_keys(property);
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SetEffectKey {
            clip_id,
            entry,
            key,
            at,
            value,
            ease,
        } => {
            let timeline = project.active_mut();
            let Some(link) = timeline
                .clip_mut(&clip_id)
                .and_then(|clip| clip.video_effects.get_mut(entry))
            else {
                return Ok(Outcome::default());
            };
            if !at.is_finite() || !value.is_finite() {
                return Ok(Outcome::default());
            }
            let before = link.keys.clone();
            link.set_key(&key, at, value, ease);
            Ok(Outcome {
                created_id: None,
                applied: link.keys != before,
            })
        }

        Command::ClearEffectKey {
            clip_id,
            entry,
            key,
            at,
        } => {
            let timeline = project.active_mut();
            let Some(link) = timeline
                .clip_mut(&clip_id)
                .and_then(|clip| clip.video_effects.get_mut(entry))
            else {
                return Ok(Outcome::default());
            };
            let applied = link.clear_key(&key, at);
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::ClearEffectKeys {
            clip_id,
            entry,
            key,
        } => {
            let timeline = project.active_mut();
            let Some(link) = timeline
                .clip_mut(&clip_id)
                .and_then(|clip| clip.video_effects.get_mut(entry))
            else {
                return Ok(Outcome::default());
            };
            let applied = link.clear_keys(&key);
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SetMaskKey {
            clip_id,
            mask_id,
            number,
            at,
            value,
            ease,
        } => {
            let timeline = project.active_mut();
            let Some(mask) = timeline
                .clip_mut(&clip_id)
                .and_then(|clip| clip.masks.iter_mut().find(|mask| mask.id == mask_id))
            else {
                return Ok(Outcome::default());
            };
            let before = mask.keys.clone();
            mask.set_key(&number, at, value, ease);
            Ok(Outcome {
                created_id: None,
                applied: mask.keys != before,
            })
        }

        Command::ClearMaskKey {
            clip_id,
            mask_id,
            number,
            at,
        } => {
            let timeline = project.active_mut();
            let Some(mask) = timeline
                .clip_mut(&clip_id)
                .and_then(|clip| clip.masks.iter_mut().find(|mask| mask.id == mask_id))
            else {
                return Ok(Outcome::default());
            };
            let applied = mask.clear_key(&number, at);
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::ClearMaskKeys {
            clip_id,
            mask_id,
            number,
        } => {
            let timeline = project.active_mut();
            let Some(mask) = timeline
                .clip_mut(&clip_id)
                .and_then(|clip| clip.masks.iter_mut().find(|mask| mask.id == mask_id))
            else {
                return Ok(Outcome::default());
            };
            let applied = mask.clear_keys(&number);
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SetClipSpeedCurve {
            clip_id,
            curve,
            ripple,
        } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            // Stored as it will play, so a reopened project plays the same.
            // A curve with nothing usable in it is no curve: the clip keeps
            // the rate it had, its old mean, and so its length.
            let curve = curve.as_deref().and_then(crate::speed::sanitised);
            let source_covered = clip.duration * clip.speed;
            let old_end = clip.start + clip.duration;
            let mean = curve
                .as_ref()
                .map(|points| crate::speed::average_speed(points))
                .unwrap_or(clip.speed)
                .clamp(MIN_SPEED, MAX_SPEED);
            let applied = assign(&mut clip.speed_curve, curve)
                | assign(&mut clip.speed, mean)
                | assign(
                    &mut clip.duration,
                    (source_covered / mean).max(MIN_CLIP_DURATION),
                );
            if applied {
                shift_followers(timeline, &clip_id, old_end, ripple);
            }
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::MatchClip { clip_id, like } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            let mut matched = *like;
            matched.id = clip.id.clone();
            matched.track_id = clip.track_id.clone();
            matched.media_id = clip.media_id.clone();
            matched.kind = clip.kind;
            matched.start = clip.start;
            matched.duration = clip.duration;
            matched.source_start = clip.source_start;
            matched.speed = clip.speed;
            matched.speed_curve = clip.speed_curve.clone();
            matched.detached_from = clip.detached_from.clone();
            matched.transition_in = clip.transition_in.clone();
            // A title's words and a shape's figure are what it is, and only
            // a clip of the same kind can wear them.
            if matched.text.is_some() != clip.text.is_some() {
                matched.text = clip.text.clone();
            }
            if matched.shape.is_some() != clip.shape.is_some() {
                matched.shape = clip.shape.clone();
            }
            let applied = assign(clip, matched);
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }
        Command::SetClipTransform {
            clip_id,
            scale,
            offset_x,
            offset_y,
            rotation,
            stretch_x,
            stretch_y,
        } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            let mut applied = false;
            if let Some(scale) = scale {
                applied |= assign(&mut clip.scale, scale.clamp(MIN_SCALE, MAX_SCALE));
            }
            if let Some(offset) = offset_x {
                applied |= assign(&mut clip.offset_x, offset.clamp(-MAX_OFFSET, MAX_OFFSET));
            }
            if let Some(offset) = offset_y {
                applied |= assign(&mut clip.offset_y, offset.clamp(-MAX_OFFSET, MAX_OFFSET));
            }
            if let Some(rotation) = rotation {
                applied |= assign(&mut clip.rotation, wrap_rotation(rotation));
            }
            if let Some(stretch) = stretch_x {
                applied |= assign(&mut clip.stretch_x, stretch.clamp(MIN_STRETCH, MAX_STRETCH));
            }
            if let Some(stretch) = stretch_y {
                applied |= assign(&mut clip.stretch_y, stretch.clamp(MIN_STRETCH, MAX_STRETCH));
            }
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::FrameClip {
            clip_id,
            framing,
            at,
        } => {
            let timeline = project.active();
            let frame = (timeline.video.width, timeline.video.height);
            let Some(media) = timeline
                .clip(&clip_id)
                .and_then(|clip| project.media_by_id(&clip.media_id))
                .map(|item| (item.width, item.height))
            else {
                return Ok(Outcome::default());
            };
            let Some(clip) = project.active_mut().clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            let at = at.clamp(0.0, 1.0);
            let Some(scale) = clip.framing_scale(framing, media, frame, at) else {
                return Ok(Outcome::default());
            };
            let before = clip.clone();
            clip.write_at(KeyProperty::Scale, scale, at);
            clip.write_at(KeyProperty::OffsetX, 0.0, at);
            clip.write_at(KeyProperty::OffsetY, 0.0, at);
            // A stretch is a picture pulled out of its shape, and neither a
            // fit nor a fill is: both are the picture as it was shot.
            clip.stretch_x = 1.0;
            clip.stretch_y = 1.0;
            Ok(Outcome {
                created_id: None,
                applied: *clip != before,
            })
        }

        _ => unreachable!("commands::apply routes only this module's commands here"),
    }
}

/// The lane after a retime moved a clip's end from `old_end`: the clips
/// behind it move by as much as the end did when the clip has grown over
/// the next one, so nothing is covered, and always on the magnetic
/// timeline, so a shrink closes up rather than leaving a gap. A plain
/// shrink leaves the gap. The rest of the timeline follows a ripple on the
/// main lane, as every ripple does.
fn shift_followers(timeline: &mut Timeline, clip_id: &str, old_end: f64, ripple: bool) {
    let Some(clip) = timeline.clip(clip_id) else {
        return;
    };
    let track_id = clip.track_id.clone();
    let new_end = clip.start + clip.duration;
    let moved = new_end - old_end;
    if moved == 0.0 {
        return;
    }
    let behind = old_end - crate::placement::TOUCH;
    let is_behind =
        |other: &Clip| other.id != clip_id && other.track_id == track_id && other.start >= behind;
    let covers = moved > 0.0
        && timeline
            .clips
            .iter()
            .any(|other| is_behind(other) && other.start < new_end - crate::placement::TOUCH);
    if covers || ripple {
        for other in timeline.clips_where(is_behind) {
            other.start = (other.start + moved).max(0.0);
        }
        let main = timeline.background_lane().map(str::to_owned);
        // Linkage: sped up, the clip gave back the end of its old span;
        // slowed down, it put more in at its old end.
        let ripple = if moved < 0.0 {
            clips::Ripple::Removed(vec![(old_end + moved, old_end)])
        } else {
            clips::Ripple::Inserted {
                at: old_end,
                length: moved,
            }
        };
        clips::follow_main(timeline, main.as_deref(), &track_id, ripple);
    }
}

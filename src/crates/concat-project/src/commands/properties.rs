// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

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
            if let Some(reverse) = patch.reverse {
                applied |= assign(&mut clip.reverse, reverse);
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
            if let Some(stream) = patch.audio_stream {
                applied |= assign(&mut clip.audio_stream, stream);
            }
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SetClipSpeed { clip_id, speed } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            // The amount of source covered is held constant - that is what
            // makes this a speed change rather than a trim.
            let next = speed.clamp(MIN_SPEED, MAX_SPEED);
            let source_covered = clip.duration * clip.speed;
            // Bitwise so no assignment is short-circuited away. A rate set
            // by hand is a constant rate: the curve goes.
            let applied = assign(&mut clip.speed, next)
                | assign(
                    &mut clip.duration,
                    (source_covered / next).max(MIN_CLIP_DURATION),
                )
                | assign(&mut clip.speed_curve, None);
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

        Command::SetClipSpeedCurve { clip_id, curve } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            let curve = curve.filter(|points| crate::speed::curve_of(points).is_some());
            let source_covered = clip.duration * clip.speed;
            let mean = curve
                .as_ref()
                .map(|points| crate::speed::mean_of(points))
                .unwrap_or(clip.speed)
                .clamp(MIN_SPEED, MAX_SPEED);
            let applied = assign(&mut clip.speed_curve, curve)
                | assign(&mut clip.speed, mean)
                | assign(
                    &mut clip.duration,
                    (source_covered / mean).max(MIN_CLIP_DURATION),
                );
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

        _ => unreachable!("commands::apply routes only this module's commands here"),
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The bin and the fonts: what a project can place, before anything is placed.
//!
//! One arm per command, exactly as [`super::apply`] routes them here;
//! everything these arms share lives in the parent module.

use super::*;

/// Applies one of this module's commands. Any other is a routing error.
pub(super) fn apply(
    project: &mut Project,
    mint: &mut IdMint,
    command: Command,
) -> Result<Outcome, CommandError> {
    match command {
        Command::AddMedia { item } => {
            if project
                .media
                .iter()
                .any(|existing| existing.path == item.path)
            {
                return Ok(Outcome::default());
            }
            let id = mint.next("m");
            project.media.push(MediaItem {
                id: id.clone(),
                path: item.path,
                name: item.name,
                duration: item.duration,
                kind: item.kind,
                width: item.width,
                height: item.height,
                frame_rate: item.frame_rate,
                frame_rate_fraction: item.frame_rate_fraction,
                video_codec: item.video_codec,
                audio_codec: item.audio_codec,
                has_audio: item.has_audio,
                audio_tracks: item.audio_tracks,
                origin: item.origin,
                placeholder: false,
                color_range: None,
                extra: Default::default(),
            });
            Ok(Outcome {
                created_id: Some(id),
                applied: true,
            })
        }

        Command::SetMediaPlaceholder {
            media_id,
            placeholder,
        } => {
            let applied = project
                .media
                .iter_mut()
                .find(|item| item.id == media_id)
                .is_some_and(|item| assign(&mut item.placeholder, placeholder));
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::FillSlot { media_id, item } => {
            let media = project
                .media
                .iter_mut()
                .find(|existing| existing.id == media_id)
                .ok_or(CommandError::SlotGone)?;
            if !media.placeholder {
                return Err(CommandError::NotASlot);
            }

            // The slot keeps its id, so every clip that references it keeps
            // working; only the identity behind the id changes.
            media.path = item.path;
            media.name = item.name.clone();
            media.duration = item.duration;
            media.kind = item.kind;
            media.width = item.width;
            media.height = item.height;
            media.frame_rate = item.frame_rate;
            media.frame_rate_fraction = item.frame_rate_fraction;
            media.video_codec = item.video_codec;
            media.audio_codec = item.audio_codec;
            media.has_audio = item.has_audio;
            media.audio_tracks = item.audio_tracks;
            media.placeholder = false;
            let kind = match item.kind {
                MediaKind::Video => ClipKind::Video,
                MediaKind::Audio => ClipKind::Audio,
                MediaKind::Image => ClipKind::Image,
            };

            // Slot timing is the template's: start, duration and speed stay
            // put, which is what keeps cuts on the beat. The in-point resets
            // because it referred to the old footage; a clip shorter than its
            // slot freeze-frames on its last frame downstream, which is the
            // renderer's existing behaviour for a trim past the media's end.
            // All timelines, like RemoveMedia: slots are not per-timeline.
            for timeline in project.timelines.iter_mut() {
                // A timeline with no clip of this media stays the
                // snapshot's.
                if !timeline.clips.iter().any(|clip| clip.media_id == media_id) {
                    continue;
                }
                let timeline = Arc::make_mut(timeline);
                for clip in timeline.clips_where(|clip| clip.media_id == media_id) {
                    {
                        clip.source_start = 0.0;
                        clip.kind = kind;
                        clip.name = item.name.clone();
                        // A stream index named against the old file means
                        // nothing against the new one.
                        clip.audio_stream = None;
                    }
                }
            }
            // Filling always changes the project: the target was a
            // placeholder and is one no longer.
            Ok(Outcome {
                created_id: None,
                applied: true,
            })
        }

        Command::RemoveMedia { media_id } => {
            // All timelines, not just the active one: a shelved clip whose
            // media is gone would linger as a dead reference.
            let media_count = project.media.len();
            project.media.retain(|item| item.id != media_id);
            let mut applied = project.media.len() != media_count;
            for timeline in project.timelines.iter_mut().map(Arc::make_mut) {
                let clip_count = timeline.clips.len();
                timeline.clips.retain(|clip| clip.media_id != media_id);
                applied |= timeline.clips.len() != clip_count;
            }
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::AddFont { family, path } => {
            if project.fonts.iter().any(|font| font.path == path) {
                return Ok(Outcome::default());
            }
            project.fonts.push(CustomFont { family, path });
            Ok(Outcome {
                created_id: None,
                applied: true,
            })
        }

        Command::RemoveFont { family } => {
            // Clips keep the family name: the face may come back when the
            // file does.
            let font_count = project.fonts.len();
            project.fonts.retain(|font| font.family != family);
            let applied = project.fonts.len() != font_count;
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::UpdateMediaPath { media_id, new_path } => {
            let applied = project
                .media
                .iter_mut()
                .find(|item| item.id == media_id)
                .is_some_and(|item| assign(&mut item.path, new_path));
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SetMediaColorRange { media_id, range } => {
            let applied = project
                .media
                .iter_mut()
                .find(|item| item.id == media_id)
                .is_some_and(|item| assign(&mut item.color_range, range));
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        _ => unreachable!("commands::apply routes only this module's commands here"),
    }
}

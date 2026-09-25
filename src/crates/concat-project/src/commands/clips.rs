// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Placing, moving and cutting clips: the edits that change what is on the timeline and when.
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
        Command::AddClip {
            media_id,
            track_id,
            start,
            ripple,
        } => {
            let media = project
                .media_by_id(&media_id)
                .ok_or(CommandError::MediaGone)?
                .clone();
            let timeline = project.active_mut();
            if timeline.track(&track_id).is_none() {
                return Err(CommandError::TrackGone);
            }
            let start = if ripple {
                ripple_room_for(timeline, &track_id, start, &media)
            } else {
                start
            };
            let id = mint.next("c");
            timeline
                .clips
                .push(Arc::new(default_clip(id.clone(), track_id, &media, start)));
            Ok(Outcome {
                created_id: Some(id),
                applied: true,
            })
        }

        Command::AddClipAtFirstFree { media_id, start } => {
            let media = project
                .media_by_id(&media_id)
                .ok_or(CommandError::MediaGone)?
                .clone();
            let duration = match media.kind {
                MediaKind::Image => DEFAULT_IMAGE_DURATION,
                _ => media.duration.unwrap_or(UNKNOWN_DURATION),
            };
            let timeline = project.active_mut();
            let track_id =
                first_free_track(timeline, start, duration).ok_or(CommandError::NoTracks)?;
            let id = mint.next("c");
            timeline
                .clips
                .push(Arc::new(default_clip(id.clone(), track_id, &media, start)));
            Ok(Outcome {
                created_id: Some(id),
                applied: true,
            })
        }

        Command::AddTextClip {
            track_id,
            start,
            style,
            duration,
            offset_y,
            above,
        } => {
            let style = style.unwrap_or_default();
            let duration = duration
                .unwrap_or(DEFAULT_TEXT_DURATION)
                .max(MIN_CLIP_DURATION);
            let timeline = project.active_mut();
            let track_id = match track_id {
                Some(id) if timeline.track(&id).is_some() => id,
                Some(_) => return Err(CommandError::TrackGone),
                None if above => match first_free_track_above(timeline, start, duration) {
                    Some(id) => id,
                    None => {
                        // Every lane above the video is taken: mint one at
                        // the top for the words to land on.
                        let id = mint.next("t");
                        timeline.tracks.push(Track {
                            id: id.clone(),
                            visible: true,
                            muted: false,
                            extra: Default::default(),
                        });
                        id
                    }
                },
                None => {
                    first_free_track(timeline, start, duration).ok_or(CommandError::NoTracks)?
                }
            };
            let id = mint.next("c");
            let mut clip = Clip::blank(
                id.clone(),
                track_id,
                ClipKind::Text,
                first_line(&style.content),
                start,
                duration,
            );
            clip.offset_y = offset_y.unwrap_or(0.0).clamp(-MAX_OFFSET, MAX_OFFSET);
            clip.text = Some(style);
            timeline.clips.push(Arc::new(clip));
            Ok(Outcome {
                created_id: Some(id),
                applied: true,
            })
        }

        Command::AddLayerClip {
            track_id,
            start,
            duration,
            effect_id,
            name,
        } => {
            let duration = duration
                .unwrap_or(DEFAULT_LAYER_DURATION)
                .max(MIN_CLIP_DURATION);
            let timeline = project.active_mut();
            let track_id = match track_id {
                Some(id) if timeline.track(&id).is_some() => id,
                Some(_) => return Err(CommandError::TrackGone),
                None => {
                    first_free_track(timeline, start, duration).ok_or(CommandError::NoTracks)?
                }
            };
            let id = mint.next("c");
            let name = if name.trim().is_empty() {
                effect_id.clone()
            } else {
                name
            };
            let mut clip =
                Clip::blank(id.clone(), track_id, ClipKind::Layer, name, start, duration);
            clip.video_effects = vec![AppliedFilter::new(effect_id)];
            timeline.clips.push(Arc::new(clip));
            Ok(Outcome {
                created_id: Some(id),
                applied: true,
            })
        }

        Command::MoveClips { moves } => {
            let timeline = project.active_mut();
            let track_ids: HashSet<String> = timeline
                .tracks
                .iter()
                .map(|track| track.id.clone())
                .collect();
            let mut applied = false;
            for wanted in moves {
                if let Some(clip) = timeline.clip_mut(&wanted.clip_id) {
                    applied |= assign(&mut clip.start, wanted.start.max(0.0));
                    if track_ids.contains(&wanted.track_id) {
                        applied |= assign(&mut clip.track_id, wanted.track_id);
                    }
                }
            }
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::TrimClip {
            clip_id,
            edge,
            delta,
            ripple,
        } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            let track_id = clip.track_id.clone();
            let anchor = clip.start;
            let old_end = clip.start + clip.duration;
            // What the trim did, and what the lane behind it does about
            // it when it is magnetic: `by` is how far the later clips
            // move, `behind` where "later" begins.
            let (applied, by, behind) = match edge {
                TrimEdge::End => {
                    let duration = (clip.duration + delta).max(MIN_CLIP_DURATION);
                    let old = clip.duration;
                    let applied = assign(&mut clip.duration, duration);
                    if applied {
                        clip.rewindow_keys(old, 0.0, duration);
                    }
                    (applied, duration - old, old_end - JOIN_EPSILON)
                }
                TrimEdge::Start => {
                    // Dragging the head moves the in-point too, so the pixels
                    // under the remaining part of the clip do not slide. The
                    // head cannot reach before the source begins.
                    let shift = delta
                        .min(clip.duration - MIN_CLIP_DURATION)
                        .max(-clip.source_start / clip.speed);
                    // A magnetic head trim never moves the clip, so the
                    // timeline's own start is no limit to it; a plain one
                    // stops at zero.
                    let start = if ripple {
                        clip.start + shift
                    } else {
                        (clip.start + shift).max(0.0)
                    };
                    let moved = start - clip.start;
                    let duration = clip.duration - moved;
                    let source_start = (clip.source_start + moved * clip.speed).max(0.0);
                    let old = clip.duration;
                    // Bitwise so no assignment is short-circuited away.
                    let applied = assign(&mut clip.start, start)
                        | assign(&mut clip.duration, duration)
                        | assign(&mut clip.source_start, source_start);
                    if applied {
                        clip.rewindow_keys(old, moved, old);
                    }
                    if ripple {
                        // The in-point moved; the clip stays put, and the
                        // lane behind it closes by what came off the head.
                        clip.start = anchor;
                    }
                    (applied, -moved, anchor + JOIN_EPSILON)
                }
            };
            if ripple && applied && by != 0.0 {
                for other in timeline.clips_where(|other| {
                    other.id != clip_id && other.track_id == track_id && other.start >= behind
                }) {
                    other.start = (other.start + by).max(0.0);
                }
            }
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SplitClips { clip_ids, time } => {
            let timeline = project.active_mut();
            let mut created = None;
            for clip_id in clip_ids {
                let Some(index) = timeline.clips.iter().position(|clip| clip.id == clip_id) else {
                    continue;
                };
                {
                    // A curve does not survive a cut in halves: the map from
                    // here to the source is not affine, so both halves go to
                    // the constant mean, which is what they averaged.
                    let clip = timeline.clip_at_mut(index);
                    let offset = time - clip.start;
                    if offset > MIN_CLIP_DURATION
                        && offset < clip.duration - MIN_CLIP_DURATION
                        && clip.speed_curve.is_some()
                    {
                        clip.speed_curve = None;
                    }
                }
                let clip: &Clip = &timeline.clips[index];
                let offset = time - clip.start;
                if offset <= MIN_CLIP_DURATION || offset >= clip.duration - MIN_CLIP_DURATION {
                    continue;
                }
                let (head_source, tail_source) =
                    split_source(clip.source_start, clip.speed, offset);
                let whole = clip.duration;
                let mut tail = clip.clone();
                tail.id = mint.next("c");
                tail.start = clip.start + offset;
                tail.duration = clip.duration - offset;
                tail.source_start = tail_source;
                // The transition belongs to the cut at the original clip's
                // start, which the head keeps; the way in belongs to the
                // head and the way out to the tail, so neither piece plays
                // an entrance or an exit the whole did not have at the cut.
                tail.transition_in = None;
                tail.fade_in = 0.0;
                tail.rewindow_keys(whole, offset, whole);
                created = Some(tail.id.clone());
                let head = timeline.clip_at_mut(index);
                head.duration = offset;
                head.source_start = head_source;
                head.fade_out = 0.0;
                head.rewindow_keys(whole, 0.0, offset);
                timeline.clips.insert(index + 1, Arc::new(tail));
            }
            // A split always mints the tail, so "minted anything" and
            // "changed anything" are the same fact here.
            let applied = created.is_some();
            Ok(Outcome {
                created_id: created,
                applied,
            })
        }

        Command::ReplaceClipMedia {
            clip_id,
            item,
            source_start,
        } => {
            if project.active().clip(&clip_id).is_none() {
                return Ok(Outcome::default());
            }
            // The copy's bin entry, or the one already there for its path:
            // two clips of one file enhanced in turn share one copy.
            let media_id = match project.media.iter().find(|media| media.path == item.path) {
                Some(existing) => existing.id.clone(),
                None => {
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
                    id
                }
            };
            let timeline = project.active_mut();
            let Some(index) = timeline.clips.iter().position(|clip| clip.id == clip_id) else {
                return Ok(Outcome::default());
            };
            let clip = timeline.clip_at_mut(index);
            // Bitwise so no assignment is short-circuited away.
            let applied = assign(&mut clip.media_id, media_id.clone())
                | source_start.is_some_and(|start| assign(&mut clip.source_start, start.max(0.0)));
            if !applied {
                return Ok(Outcome::default());
            }
            Ok(Outcome {
                created_id: Some(media_id),
                applied: true,
            })
        }

        Command::FreezeFrame {
            clip_id,
            time,
            duration,
            still,
        } => {
            let hold = duration
                .filter(|value| *value > 0.0)
                .unwrap_or(DEFAULT_FREEZE_DURATION)
                .max(MIN_CLIP_DURATION);

            let (kind, media_id, track_id, start, clip_duration, speed, source_start, picture) = {
                let timeline = project.active();
                let Some(clip) = timeline.clip(&clip_id) else {
                    return Ok(Outcome::default());
                };
                if clip.kind != ClipKind::Video && clip.kind != ClipKind::Image {
                    return Ok(Outcome::default());
                }
                let offset = time - clip.start;
                if offset <= MIN_CLIP_DURATION || offset >= clip.duration - MIN_CLIP_DURATION {
                    return Ok(Outcome::default());
                }
                (
                    clip.kind,
                    clip.media_id.clone(),
                    clip.track_id.clone(),
                    clip.start,
                    clip.duration,
                    clip.speed,
                    clip.source_start,
                    clip.clone(),
                )
            };

            let freeze_media_id = if kind == ClipKind::Image && still.is_none() {
                media_id
            } else {
                let Some(item) = still else {
                    return Ok(Outcome::default());
                };
                if let Some(existing) = project.media.iter().find(|media| media.path == item.path) {
                    existing.id.clone()
                } else {
                    let id = mint.next("m");
                    project.media.push(MediaItem {
                        id: id.clone(),
                        path: item.path,
                        name: item.name,
                        duration: item.duration,
                        kind: MediaKind::Image,
                        width: item.width,
                        height: item.height,
                        frame_rate: item.frame_rate,
                        frame_rate_fraction: item.frame_rate_fraction,
                        video_codec: item.video_codec,
                        audio_codec: None,
                        has_audio: false,
                        audio_tracks: Vec::new(),
                        origin: item.origin,
                        placeholder: false,
                        color_range: None,
                        extra: Default::default(),
                    });
                    id
                }
            };

            let timeline = project.active_mut();
            let Some(index) = timeline.clips.iter().position(|clip| clip.id == clip_id) else {
                return Ok(Outcome::default());
            };
            // The cut is a split's, so it leaves the pieces as a split does:
            // under a curve the map is not affine, and the in-point below
            // assumes it is, so both pieces go to the constant mean they
            // averaged.
            timeline.clip_at_mut(index).speed_curve = None;
            let offset = time - start;
            let (head_source, tail_source) = split_source(source_start, speed, offset);
            let mut tail = Clip::clone(&timeline.clips[index]);
            tail.id = mint.next("c");
            tail.start = time;
            tail.duration = clip_duration - offset;
            tail.source_start = tail_source;
            tail.transition_in = None;
            tail.fade_in = 0.0;
            tail.rewindow_keys(clip_duration, offset, clip_duration);
            let head = timeline.clip_at_mut(index);
            head.duration = offset;
            head.source_start = head_source;
            head.fade_out = 0.0;
            head.rewindow_keys(clip_duration, 0.0, offset);
            timeline.clips.insert(index + 1, Arc::new(tail));

            // Ripple every later placement on this track (including the new
            // tail) so the freeze does not sit on top of the remainder.
            for clip in timeline.clips_where(|clip| clip.track_id == track_id && clip.start >= time)
            {
                clip.start += hold;
            }

            // The still is the source clip turned into a picture: cloning it
            // first carries every look field - transform, effects, crop,
            // flips, whatever the model grows - and then the hold's own
            // facts overwrite the moving ones.
            let freeze_id = mint.next("c");
            let mut frozen = picture;
            frozen.id = freeze_id.clone();
            frozen.track_id = track_id;
            frozen.kind = ClipKind::Image;
            frozen.media_id = freeze_media_id;
            frozen.start = time;
            frozen.duration = hold;
            frozen.source_start = 0.0;
            frozen.speed = 1.0;
            frozen.speed_curve = None;
            frozen.volume = 1.0;
            frozen.fade_in = 0.0;
            frozen.fade_out = 0.0;
            frozen.filters = Vec::new();
            frozen.muted = None;
            frozen.detached_from = None;
            frozen.transition_in = None;
            frozen.text = None;
            timeline.clips.push(Arc::new(frozen));

            Ok(Outcome {
                created_id: Some(freeze_id),
                applied: true,
            })
        }

        Command::MergeClips { clip_ids } => {
            let timeline = project.active_mut();
            if let Some(reason) = why_not_merge(timeline, &clip_ids) {
                return Err(CommandError::CannotMerge { reason });
            }
            let mut ordered: Vec<Clip> = clip_ids
                .iter()
                .filter_map(|id| timeline.clip(id).cloned())
                .collect();
            ordered.sort_by(|left, right| left.start.total_cmp(&right.start));
            let first = ordered.first().expect("validated above").clone();
            let last = ordered.last().expect("validated above");
            let merged_duration = last.start + last.duration - first.start;

            // A set, not a Vec: the retain below tests every clip on the
            // timeline against it.
            let doomed: HashSet<String> =
                ordered.iter().skip(1).map(|clip| clip.id.clone()).collect();
            timeline.clips.retain(|clip| !doomed.contains(&clip.id));
            let survivor = timeline
                .clip_mut(&first.id)
                .expect("the first piece survives the retain");
            survivor.duration = merged_duration;
            // Every piece's keys land where they were on the picture; the
            // way out is the last piece's, as the way in is the first's.
            survivor.rewindow_keys(first.duration, 0.0, merged_duration);
            for piece in ordered.iter().skip(1) {
                survivor.absorb_keys(piece, piece.start - first.start);
            }
            survivor.fade_out = last.fade_out;
            // A validated merge always absorbs at least one piece.
            Ok(Outcome {
                created_id: Some(first.id),
                applied: true,
            })
        }

        Command::RemoveClips { clip_ids, ripple } => {
            let timeline = project.active_mut();
            let doomed: HashSet<&str> = clip_ids.iter().map(String::as_str).collect();
            // The spans going, per track, taken before they are gone: the
            // ripple closes exactly these.
            let removed: Vec<(String, f64, f64)> = timeline
                .clips
                .iter()
                .filter(|clip| doomed.contains(clip.id.as_str()))
                .map(|clip| {
                    (
                        clip.track_id.clone(),
                        clip.start,
                        clip.start + clip.duration,
                    )
                })
                .collect();
            let clip_count = timeline.clips.len();
            timeline
                .clips
                .retain(|clip| !doomed.contains(clip.id.as_str()));
            let applied = timeline.clips.len() != clip_count;
            if ripple && applied {
                close_gaps(timeline, &removed);
            }
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        _ => unreachable!("commands::apply routes only this module's commands here"),
    }
}

/// How far apart two clips may sit and still count as touching, in seconds.
const JOIN_EPSILON: f64 = 1e-6;

/// The ripple of a delete: pulls every clip that sits after a removed
/// span left by the length of the removed spans before it, on that clip's
/// own track. Spans that overlap count once - a clip behind two doomed
/// clips that shared five seconds moves by their union, not their sum, or
/// it would land in front of what was in front of it. A span reaching past
/// the clip's start counts only up to it, and nothing is pulled before
/// zero.
/// https://github.com/jub0t/Concat/issues/106
fn close_gaps(timeline: &mut Timeline, removed: &[(String, f64, f64)]) {
    let behind_a_span = |clip: &Clip| {
        removed
            .iter()
            .any(|(track, start, _)| *track == clip.track_id && *start < clip.start)
    };
    for clip in timeline.clips_where(behind_a_span) {
        let mut spans: Vec<(f64, f64)> = removed
            .iter()
            .filter(|(track, start, _)| *track == clip.track_id && *start < clip.start)
            .map(|(_, start, end)| (*start, end.min(clip.start)))
            .collect();
        if spans.is_empty() {
            continue;
        }
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut gap = 0.0;
        let (mut from, mut to) = spans[0];
        for (start, end) in spans.into_iter().skip(1) {
            if start <= to {
                to = to.max(end);
            } else {
                gap += to - from;
                from = start;
                to = end;
            }
        }
        gap += to - from;
        clip.start = (clip.start - gap).max(0.0);
    }
}

fn default_clip(id: String, track_id: String, media: &MediaItem, start: f64) -> Clip {
    let kind = match media.kind {
        MediaKind::Video => ClipKind::Video,
        MediaKind::Audio => ClipKind::Audio,
        MediaKind::Image => ClipKind::Image,
    };
    let duration = match media.kind {
        MediaKind::Image => DEFAULT_IMAGE_DURATION,
        _ => media.duration.unwrap_or(UNKNOWN_DURATION),
    };
    let mut clip = Clip::blank(id, track_id, kind, media.name.clone(), start, duration);
    clip.media_id = media.id.clone();
    clip
}

/// Makes room on `track_id` for a new clip at `start`: a drop onto the
/// middle of a clip lands at that clip's end instead, and every clip at
/// or after the place it lands moves right by the new clip's length, so
/// the new clip slots in and nothing is covered (#129). Returns where the
/// new clip lands. A drop with room to spare changes nothing.
fn ripple_room_for(timeline: &mut Timeline, track_id: &str, start: f64, media: &MediaItem) -> f64 {
    let duration = match media.kind {
        MediaKind::Image => DEFAULT_IMAGE_DURATION,
        _ => media.duration.unwrap_or(UNKNOWN_DURATION),
    };
    // Dropped onto a clip: after it, rather than over it or through it.
    let start = timeline
        .clips
        .iter()
        .filter(|clip| {
            clip.track_id == track_id && clip.start < start && start < clip.start + clip.duration
        })
        .map(|clip| clip.start + clip.duration)
        .fold(start, f64::max);
    let end = start + duration;
    let overlaps = timeline.clips.iter().any(|clip| {
        clip.track_id == track_id && clip.start < end && start < clip.start + clip.duration
    });
    if !overlaps {
        return start;
    }
    for clip in timeline.clips_where(|clip| clip.track_id == track_id && clip.start >= start) {
        clip.start += duration;
    }
    start
}

/// The lowest track with nothing occupying `[start, start + duration)`,
/// falling back to the bottom track.
fn first_free_track(timeline: &Timeline, start: f64, duration: f64) -> Option<String> {
    let end = start + duration;
    timeline
        .tracks
        .iter()
        .find(|track| {
            !timeline.clips.iter().any(|clip| {
                clip.track_id == track.id && clip.start < end && start < clip.start + clip.duration
            })
        })
        .or(timeline.tracks.first())
        .map(|track| track.id.clone())
}

/// First free lane *above* the highest one occupied over `[start, start +
/// duration)`. `None` when every lane above is taken, which is the caller's
/// cue to mint a new one at the top.
///
/// Captions and titles go through this so they sit over the video, not
/// under it. The plain `first_free_track` still walks from the bottom,
/// which is what a sound or a picture wants.
fn first_free_track_above(timeline: &Timeline, start: f64, duration: f64) -> Option<String> {
    let end = start + duration;
    let occupied = |track_id: &str| {
        timeline.clips.iter().any(|clip| {
            clip.track_id == track_id && clip.start < end && start < clip.start + clip.duration
        })
    };
    let floor = timeline
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| occupied(&track.id))
        .map(|(row, _)| row + 1)
        .max()
        .unwrap_or(0);
    timeline
        .tracks
        .iter()
        .skip(floor)
        .find(|track| !occupied(&track.id))
        .map(|track| track.id.clone())
}

/// Where each piece of a clip cut at `offset` begins in the source: the
/// head keeps its in-point and the tail starts `offset × speed` later, so
/// the two pieces together show exactly what the whole did.
fn split_source(source_start: f64, speed: f64, offset: f64) -> (f64, f64) {
    (source_start, source_start + offset * speed)
}

/// Why these clips cannot be merged, or None if they can. A sentence, because
/// a disabled button that will not say why is worse than no button.
pub fn why_not_merge(timeline: &Timeline, clip_ids: &[String]) -> Option<String> {
    if clip_ids.len() < 2 {
        return Some("Select two or more clips to merge.".to_owned());
    }
    let clips: Vec<&Clip> = clip_ids.iter().filter_map(|id| timeline.clip(id)).collect();
    if clips.len() < 2 {
        return Some("Select two or more clips to merge.".to_owned());
    }
    if clips.iter().any(|clip| clip.track_id != clips[0].track_id) {
        return Some("Merged clips must be on the same track.".to_owned());
    }
    if clips.iter().any(|clip| clip.media_id != clips[0].media_id) {
        return Some("Merged clips must come from the same file.".to_owned());
    }
    if clips.iter().any(|clip| clip.speed != clips[0].speed) {
        return Some("Merged clips must play at the same speed.".to_owned());
    }
    if clips.iter().any(|clip| clip.kind != clips[0].kind) {
        return Some("Merged clips must be the same kind.".to_owned());
    }
    if clips.iter().any(|clip| clip.speed_curve.is_some()) {
        return Some("A clip with a speed curve cannot be merged.".to_owned());
    }
    if clips
        .iter()
        .any(|clip| clip.audio_stream != clips[0].audio_stream)
    {
        return Some("Merged clips must play the same audio track.".to_owned());
    }

    let mut ordered = clips.clone();
    ordered.sort_by(|left, right| left.start.total_cmp(&right.start));
    for pair in ordered.windows(2) {
        let (previous, current) = (pair[0], pair[1]);
        if (current.start - (previous.start + previous.duration)).abs() > JOIN_EPSILON {
            return Some("Merged clips must touch, with no gap or overlap.".to_owned());
        }
        // The next piece starts where the last one's source ended.
        let continuous =
            current.source_start - (previous.source_start + previous.duration * previous.speed);
        if continuous.abs() > JOIN_EPSILON {
            return Some("These pieces are no longer in their original order.".to_owned());
        }
    }
    None
}

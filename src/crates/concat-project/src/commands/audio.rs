// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Taking a video's sound out onto its own lane, and putting it back.
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
        Command::DetachAudio { clip_id } => {
            // What comes off, as `(stream, name)`: one sound clip per audio
            // track when the file lists several - a recording that kept the
            // desktop and the microphone apart stays apart, each on a lane
            // of its own - else the one stream the video clip was playing.
            let sounds: Vec<(Option<u32>, String)> = {
                let timeline = project.active();
                let Some(clip) = timeline.clip(&clip_id) else {
                    return Ok(Outcome::default());
                };
                let media = project.media_by_id(&clip.media_id);
                let has_audio = clip.kind == ClipKind::Video
                    && clip.muted != Some(true)
                    && media.is_some_and(|media| media.has_audio)
                    && !timeline
                        .clips
                        .iter()
                        .any(|other| other.detached_from.as_deref() == Some(clip_id.as_str()));
                if !has_audio {
                    return Ok(Outcome::default());
                }
                match media {
                    Some(media) if media.audio_tracks.len() > 1 => media
                        .audio_tracks
                        .iter()
                        .enumerate()
                        .map(|(position, track)| {
                            let label = if track.title.is_empty() {
                                format!("Track {}", position + 1)
                            } else {
                                track.title.clone()
                            };
                            (Some(track.index), format!("{} · {label}", clip.name))
                        })
                        .collect(),
                    _ => vec![(clip.audio_stream, clip.name.clone())],
                }
            };

            let timeline = project.active_mut();
            let clip = timeline.clip(&clip_id).expect("checked above").clone();
            let mut first_sound = None;
            for (stream, name) in sounds {
                // A lane free for the whole span, or a fresh one. Each sound
                // placed takes its lane, so the next looks past it.
                let track_id = {
                    let end = clip.start + clip.duration;
                    let free = timeline.tracks.iter().find(|track| {
                        !timeline.clips.iter().any(|other| {
                            other.track_id == track.id
                                && other.start < end
                                && clip.start < other.start + other.duration
                        })
                    });
                    match free {
                        Some(track) => track.id.clone(),
                        None => {
                            let id = mint.next("t");
                            timeline.tracks.push(Track {
                                id: id.clone(),
                                visible: true,
                                muted: false,
                                extra: Default::default(),
                            });
                            id
                        }
                    }
                };

                let mut sound = clip.clone();
                sound.id = mint.next("c");
                sound.track_id = track_id;
                sound.name = name;
                sound.kind = ClipKind::Audio;
                sound.video_effects = Vec::new();
                sound.transition_in = None;
                sound.detached_from = Some(clip_id.clone());
                sound.muted = None;
                sound.audio_stream = stream;
                first_sound.get_or_insert_with(|| sound.id.clone());
                timeline.clips.push(Arc::new(sound));
            }

            let video = timeline.clip_mut(&clip_id).expect("still present");
            video.muted = Some(true);
            video.filters = Vec::new();
            Ok(Outcome {
                created_id: first_sound,
                applied: true,
            })
        }

        Command::ReattachAudio { clip_id } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip(&clip_id) else {
                return Ok(Outcome::default());
            };
            let video_id = match (&clip.kind, &clip.detached_from) {
                (ClipKind::Audio, Some(source)) => source.clone(),
                _ => clip.id.clone(),
            };
            if timeline.clip(&video_id).is_none() {
                return Ok(Outcome::default());
            }
            let sounds: Vec<Clip> = timeline
                .clips
                .iter()
                .filter(|other| other.detached_from.as_deref() == Some(video_id.as_str()))
                .map(|other| Clip::clone(other))
                .collect();
            if sounds.is_empty() {
                return Ok(Outcome::default());
            }
            // A set, not a Vec: the retain below tests every clip on the
            // timeline against it.
            let doomed: HashSet<String> = sounds.iter().map(|sound| sound.id.clone()).collect();
            timeline.clips.retain(|other| !doomed.contains(&other.id));
            let video = timeline.clip_mut(&video_id).expect("checked above");
            video.muted = None;
            video.filters = sounds[0].filters.clone();
            // Reaching here means at least one sound clip was deleted.
            Ok(Outcome {
                created_id: None,
                applied: true,
            })
        }

        _ => unreachable!("commands::apply routes only this module's commands here"),
    }
}

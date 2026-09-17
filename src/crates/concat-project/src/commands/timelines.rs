// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Timelines: the tabs a project holds, their frames, names, order and which one is showing.
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
        Command::AddTimeline => {
            let id = mint.next("tl");
            let name = next_numbered(
                "Timeline",
                project
                    .timelines
                    .iter()
                    .map(|timeline| timeline.name.clone()),
            );
            let tracks = (1..=4)
                .map(|_| Track {
                    id: mint.next("t"),
                    visible: true,
                    muted: false,
                    extra: Default::default(),
                })
                .collect();
            // Born at the frame of the timeline you were looking at: the
            // likeliest second timeline is another cut of the same picture,
            // and the one that is not is a sheet away from being told so.
            let video = project.active().video;
            project.timelines.push(Arc::new(Timeline {
                id: id.clone(),
                name,
                video,
                tracks,
                clips: Vec::new(),
                extra: Default::default(),
            }));
            project.active_timeline_id = id.clone();
            Ok(Outcome {
                created_id: Some(id),
                applied: true,
            })
        }

        Command::SetTimelineVideo { timeline_id, video } => {
            if !video.is_sane() {
                return Ok(Outcome::default());
            }
            let applied = project
                .timelines
                .iter_mut()
                .find(|timeline| timeline.id == timeline_id)
                .is_some_and(|timeline| assign(&mut Arc::make_mut(timeline).video, video));
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::RemoveTimeline { timeline_id } => {
            if project.timelines.len() <= 1 {
                return Err(CommandError::LastTimeline);
            }
            let Some(index) = project
                .timelines
                .iter()
                .position(|timeline| timeline.id == timeline_id)
            else {
                return Ok(Outcome::default());
            };
            if project.active_timeline_id == timeline_id {
                let neighbour = if index + 1 < project.timelines.len() {
                    index + 1
                } else {
                    index - 1
                };
                project.active_timeline_id = project.timelines[neighbour].id.clone();
            }
            project.timelines.remove(index);
            Ok(Outcome {
                created_id: None,
                applied: true,
            })
        }

        Command::RenameTimeline { timeline_id, name } => {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Ok(Outcome::default());
            }
            let applied = project
                .timelines
                .iter_mut()
                .find(|timeline| timeline.id == timeline_id)
                .is_some_and(|timeline| {
                    assign(&mut Arc::make_mut(timeline).name, trimmed.to_owned())
                });
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SelectTimeline { timeline_id } => {
            // Short-circuiting is right here: an unknown id must not touch
            // the selection at all.
            let applied = project
                .timelines
                .iter()
                .any(|timeline| timeline.id == timeline_id)
                && assign(&mut project.active_timeline_id, timeline_id);
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::MoveTimeline { timeline_id, index } => {
            let Some(from) = project
                .timelines
                .iter()
                .position(|timeline| timeline.id == timeline_id)
            else {
                return Ok(Outcome::default());
            };
            let to = index.min(project.timelines.len() - 1);
            if to == from {
                return Ok(Outcome::default());
            }
            let timeline = project.timelines.remove(from);
            project.timelines.insert(to, timeline);
            Ok(Outcome {
                created_id: None,
                applied: true,
            })
        }

        _ => unreachable!("commands::apply routes only this module's commands here"),
    }
}

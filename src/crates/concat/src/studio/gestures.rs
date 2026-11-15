// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! Gestures in flight: drags, trims and moves, previewed on the echo.

use super::*;
use concat_project::placement::shifted_starts;

impl Studio {
    // ── gestures ──

    /// A press resolves the selection before anything moves, so a drag that
    /// starts on an already-selected clip carries the whole set.
    pub fn clip_pressed(&mut self, id: &str, additive: bool, edge: i32) {
        let Some(clip) = self.clip(id).cloned() else {
            return;
        };
        if self.locked(&clip.track_id) {
            return;
        }
        // Whatever the inspector still holds for the clip selected until
        // now lands before the selection moves: a commit flushed after the
        // change would look for it on the newly selected clip and lose it.
        self.flush_commit();
        self.end_audition();
        // A press on a transition's seam or handles selects the transition,
        // not the clip it rides into; a drag there still sets its length.
        if edge == 2
            && let Some(transition) = clip.transition_in.as_ref()
        {
            self.selection.clear();
            self.transition_selected = Some(id.to_owned());
            self.begin_echo();
            self.gesture = Gesture::TransitionResize {
                clip: id.to_owned(),
                original_duration: transition.duration as f32,
            };
            return;
        }
        self.transition_selected = None;
        let already = self.selection.iter().any(|held| held == id);
        self.selection = if additive {
            if already {
                self.selection
                    .iter()
                    .filter(|held| held.as_str() != id)
                    .cloned()
                    .collect()
            } else {
                let mut next = self.selection.clone();
                next.push(id.to_owned());
                next
            }
        } else if already {
            self.selection.clone()
        } else {
            vec![id.to_owned()]
        };

        self.begin_echo();
        if edge >= 0 && self.selection.len() <= 1 {
            self.gesture = Gesture::Trim {
                clip: id.to_owned(),
                edge: if edge == 0 { Edge::Start } else { Edge::End },
                start: clip.start as f32,
                duration: clip.duration as f32,
                source_start: clip.source_start as f32,
            };
            return;
        }
        // What moves is what is selected, and nothing else. A detached
        // sound is a clip of its own: it goes where it is dragged, and its
        // picture stays, as any two clips do. It used to travel with its
        // picture, which made Detach a split in name only (#265); the link
        // between them is kept for Reattach, not for the drag.
        let moving = if self.selection.iter().any(|held| held == id) {
            self.selection.clone()
        } else {
            vec![id.to_owned()]
        };
        let origins = moving
            .iter()
            .filter_map(|clip_id| {
                let clip = self.clip(clip_id)?;
                Some(MoveOrigin {
                    clip: clip.id.clone(),
                    start: clip.start,
                    row: self.row_of(&clip.track_id),
                })
            })
            .collect();
        self.gesture = Gesture::Move {
            primary: id.to_owned(),
            origins,
            lanes: self.lane_heights(),
        };
    }

    /// The pointer moved: the echo follows it.
    pub fn clip_dragged(&mut self, seconds: f32, pixels: f32) {
        let gesture = std::mem::replace(&mut self.gesture, Gesture::None);
        match &gesture {
            Gesture::Move {
                primary,
                origins,
                lanes,
            } => {
                let Some(anchor) = origins.iter().find(|origin| &origin.clip == primary) else {
                    self.gesture = gesture;
                    return;
                };
                let threshold = 8.0 * self.lanes.seconds_per_pixel;
                // Every moving clip's two edges, where the pointer would
                // put them, and the shift that lands the nearest on a target.
                let moving: Vec<&str> = origins.iter().map(|origin| origin.clip.as_str()).collect();
                let edges: Vec<f32> = origins
                    .iter()
                    .filter_map(|origin| {
                        let clip = self.clip(&origin.clip)?;
                        let start = origin.start as f32 + seconds;
                        Some([start, start + clip.duration as f32])
                    })
                    .flatten()
                    .collect();
                let shift = seconds + self.snap_shift(&edges, threshold, &moving);
                let rows = nearest_row(lanes, row_top(lanes, anchor.row) + pixels) - anchor.row;
                let count = self.timeline().tracks.len() as i32;
                // Every start moves by the one shift, in f64 and from the
                // start the press recorded: `self.clip` is the echo while a
                // gesture runs and already holds the last shift. Why f64 is
                // on `shifted_starts`.
                let starts: Vec<f64> = origins.iter().map(|origin| origin.start).collect();
                let landed_at = shifted_starts(&starts, f64::from(shift));
                let moves: Vec<ClipMove> = origins
                    .iter()
                    .zip(landed_at)
                    .filter_map(|(origin, start)| {
                        let clip = self.clip(&origin.clip)?;
                        let row = (origin.row + rows).clamp(0, count - 1);
                        let track_id = self
                            .row_track(row)
                            .filter(|track| !self.locked(&track.id))
                            .map(|track| track.id.clone())
                            .unwrap_or_else(|| clip.track_id.clone());
                        Some(ClipMove {
                            clip_id: origin.clip.clone(),
                            start,
                            track_id,
                        })
                    })
                    .collect();
                // The drop's own rule, asked of the timeline as it was before
                // the drag: the echo shows where the clips will land, in the
                // nearest gap rather than on top of a clip, or back where
                // they started when they would cover each other.
                let landed = self.timeline().resolve_moves(&moves).unwrap_or_else(|| {
                    origins
                        .iter()
                        .filter_map(|origin| {
                            let clip = self.clip(&origin.clip)?;
                            Some(ClipMove {
                                clip_id: clip.id.clone(),
                                start: clip.start,
                                track_id: clip.track_id.clone(),
                            })
                        })
                        .collect()
                });
                for wanted in landed {
                    if let Some(clip) = self.echo_clip_mut(&wanted.clip_id) {
                        clip.start = wanted.start;
                        clip.track_id = wanted.track_id;
                    }
                }
            }
            Gesture::Trim {
                clip,
                edge,
                start,
                duration,
                source_start,
            } => {
                let (id, edge) = (clip.clone(), *edge);
                let (start, duration, source_start) = (*start, *duration, *source_start);
                let threshold = 8.0 * self.lanes.seconds_per_pixel;
                let speed = self.clip(&id).map_or(1.0, |clip| clip.speed as f32);
                // A plain trim stops at the neighbours, as the command will;
                // a magnetic one moves them, so the lane is all its room.
                let (floor, ceiling) = if self.prefs.magnetic {
                    (0.0, f32::INFINITY)
                } else {
                    self.timeline()
                        .room_around(&id)
                        .map_or((0.0, f32::INFINITY), |(before, after)| {
                            (before as f32, after as f32)
                        })
                };
                if edge == Edge::Start {
                    // The head cannot pass the tail, and cannot pull material
                    // out of a file that has none before the in-point.
                    let wanted = self.snapped(start + seconds, threshold, &id);
                    let limit = start + duration - MIN_DURATION;
                    let at = wanted.clamp(
                        (start - source_start / speed.max(0.01))
                            .max(floor)
                            .min(limit),
                        limit,
                    );
                    let delta = at - start;
                    if let Some(clip) = self.echo_clip_mut(&id) {
                        clip.start = f64::from(at);
                        clip.duration = f64::from(duration - delta);
                        clip.source_start = f64::from(source_start + delta * speed);
                    }
                    // Trim follow: the playhead on the new first frame.
                    if self.prefs.trim_follow {
                        self.seek(at);
                    }
                } else {
                    let wanted = self.snapped(start + duration + seconds, threshold, &id);
                    let at = wanted.min(ceiling).max(start + MIN_DURATION);
                    if let Some(clip) = self.echo_clip_mut(&id) {
                        clip.duration = f64::from(at - start);
                    }
                    // Trim follow: the playhead on the new last frame, one
                    // frame inside the edge - at the edge itself the clip
                    // has already ended and the monitor would show what
                    // comes after it.
                    if self.prefs.trim_follow {
                        let frame = 1.0 / self.frame_rate().max(1.0);
                        self.seek((at - frame).max(start));
                    }
                }
            }
            Gesture::TransitionResize {
                clip,
                original_duration,
            } => {
                let id = clip.clone();
                let original_duration = *original_duration;
                if let Some(clip_ref) = self.clip(&id).cloned() {
                    let wanted = (original_duration + seconds).max(0.0) as f64;
                    if let Some(duration) = self.transition_duration(&clip_ref, wanted) {
                        if let Some(echo_clip) = self.echo_clip_mut(&id) {
                            if let Some(t) = echo_clip.transition_in.as_mut() {
                                t.duration = duration;
                            }
                        }
                    }
                }
            }
            // A stage gesture is the monitor's; the lanes have nothing to
            // add to it.
            Gesture::None
            | Gesture::StageMove { .. }
            | Gesture::StageScale { .. }
            | Gesture::StageRotate { .. }
            | Gesture::StageStretch { .. }
            | Gesture::TextWidth { .. }
            | Gesture::TextHeight { .. }
            | Gesture::StageMask { .. }
            | Gesture::Paint { .. } => {}
        }
        self.gesture = gesture;
    }

    /// The pointer let go: the whole gesture becomes one command.
    pub fn clip_released(&mut self) {
        let gesture = std::mem::replace(&mut self.gesture, Gesture::None);
        let Some(echo) = self.echo.as_ref() else {
            return;
        };
        match gesture {
            Gesture::Move { origins, .. } => {
                let moves: Vec<ClipMove> = origins
                    .iter()
                    .filter_map(|origin| {
                        let after = echo.active().clip(&origin.clip)?;
                        Some(ClipMove {
                            clip_id: origin.clip.clone(),
                            start: after.start,
                            track_id: after.track_id.clone(),
                        })
                    })
                    .collect();
                self.echo = None;
                // A press and a release with no drag between them is a
                // click: nothing moved, and a MoveClips of the clips onto
                // their own places is still an undo step and still a pass
                // through the drop's rules, which can put a clip elsewhere.
                // https://github.com/jub0t/Concat/issues/247
                if moved_any(&moves, |id| self.clip(id)) {
                    self.apply(Command::MoveClips { moves });
                }
            }
            Gesture::Trim {
                clip,
                edge,
                start,
                duration,
                ..
            } => {
                let after = echo.active().clip(&clip).cloned();
                self.echo = None;
                let Some(after) = after else { return };
                let delta = match edge {
                    Edge::Start => after.start - f64::from(start),
                    Edge::End => after.duration - f64::from(duration),
                };
                if delta.abs() > 1e-6 {
                    let id = clip.clone();
                    self.apply(Command::TrimClip {
                        clip_id: clip,
                        edge: match edge {
                            Edge::Start => TrimEdge::Start,
                            Edge::End => TrimEdge::End,
                        },
                        delta,
                        // Magnetic: the lane closes behind the edge.
                        // https://github.com/jub0t/Concat/issues/106
                        ripple: self.prefs.magnetic,
                    });
                    // Trim follow: a magnetic head trim slides the clip back
                    // into the gap, and the playhead goes with its first
                    // frame.
                    if self.prefs.trim_follow && edge == Edge::Start {
                        if let Some(start) = self.clip(&id).map(|clip| clip.start) {
                            self.seek(start as f32);
                        }
                    }
                }
            }
            Gesture::TransitionResize {
                clip,
                original_duration,
            } => {
                let after = echo.active().clip(&clip).cloned();
                self.echo = None;
                let Some(after) = after else { return };
                let new_duration = after
                    .transition_in
                    .as_ref()
                    .map(|t| t.duration)
                    .unwrap_or(0.0);
                if (new_duration - f64::from(original_duration)).abs() > 1e-4 {
                    self.set_clip_transition_duration(&clip, new_duration);
                }
            }
            Gesture::None => {
                self.echo = None;
            }
            // Not a lane gesture: hand it back untouched, echo and all.
            other @ (Gesture::StageMove { .. }
            | Gesture::StageScale { .. }
            | Gesture::StageRotate { .. }
            | Gesture::StageStretch { .. }
            | Gesture::TextWidth { .. }
            | Gesture::TextHeight { .. }
            | Gesture::StageMask { .. }
            | Gesture::Paint { .. }) => {
                self.gesture = other;
            }
        }
    }
}

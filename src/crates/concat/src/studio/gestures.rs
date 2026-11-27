// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! Gestures in flight: drags, trims and moves, previewed on the echo.
//!
//! A drag on the lanes is previewed by making the edit it would end in on
//! the document as it stands, without recording it: the echo is what the
//! release would leave, by the same rules, so where a clip is drawn is
//! where it lands.

use super::*;

/// How far a pressed clip must travel before it moves or trims.
const DRAG_DEAD_ZONE: f32 = 4.0;

fn in_dead_zone(seconds: f32, pixels: f32, seconds_per_pixel: f32) -> bool {
    seconds.abs() / seconds_per_pixel.max(f32::EPSILON) < DRAG_DEAD_ZONE
        && pixels.abs() < DRAG_DEAD_ZONE
}

/// Every start moved by one shift, held so the earliest is not before
/// zero: the group keeps its shape against the start of the timeline.
fn shifted_starts(starts: &[Tick], shift: TickDur) -> Vec<Tick> {
    let earliest = starts.iter().copied().min().unwrap_or(Tick::ZERO);
    let shift = shift.max(Tick::ZERO - earliest);
    starts.iter().map(|start| *start + shift).collect()
}

impl Studio {
    // ── gestures ──

    /// A press resolves the selection before anything moves, so a drag that
    /// starts on an already-selected clip carries the whole set.
    pub fn clip_pressed(&mut self, id: &str, additive: bool, edge: i32) {
        let Some(clip) = self.clip(id).cloned() else {
            return;
        };
        if self.lane_locked(clip.lane) {
            return;
        }
        // Whatever the inspector still holds for the clip selected until
        // now lands before the selection moves: a commit flushed after the
        // change would look for it on the newly selected clip and lose it.
        self.flush_commit();
        self.end_audition();
        self.media_focused = false;
        // A press on a transition's seam or handles selects the transition,
        // not the clip it rides into; a drag there still sets its length.
        if edge == 2
            && let Some(transition) = clip.cut_in.as_ref()
        {
            self.selection.clear();
            self.transition_selected = Some(id.to_owned());
            self.begin_echo();
            self.gesture = Gesture::TransitionResize {
                clip: id.to_owned(),
                original_duration: doc_view::len_secs(transition.len) as f32,
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
                start: doc_view::start_s(&clip) as f32,
                duration: doc_view::len_s(&clip) as f32,
                started: false,
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
                    clip: clip_id.clone(),
                    row: self.row_of_lane(clip.lane),
                })
            })
            .collect();
        self.gesture = Gesture::Move {
            primary: id.to_owned(),
            origins,
            lanes: self.lane_heights(),
            started: false,
        };
    }

    /// The echo as the document would be after `command`, by the
    /// document's own rules; the document as it is when they refuse it.
    fn preview_edit(&mut self, command: &Command) {
        let shown = doc_view::preview(&self.document, command)
            .unwrap_or_else(|| Doc::clone(&self.document));
        self.echo = Some(shown);
    }

    /// The pointer moved: the echo follows it.
    pub fn clip_dragged(&mut self, seconds: f32, pixels: f32) {
        let mut gesture = std::mem::replace(&mut self.gesture, Gesture::None);
        if let Gesture::Move { started, .. } | Gesture::Trim { started, .. } = &mut gesture
            && !*started
        {
            if in_dead_zone(seconds, pixels, self.lanes.seconds_per_pixel) {
                self.gesture = gesture;
                return;
            }
            *started = true;
        }
        match &gesture {
            Gesture::Move {
                primary,
                origins,
                lanes,
                ..
            } => {
                let Some(anchor) = origins.iter().find(|origin| &origin.clip == primary) else {
                    self.gesture = gesture;
                    return;
                };
                let threshold = 8.0 * self.lanes.seconds_per_pixel;
                // Where each clip was at the press, exactly: the echo
                // already holds the last move.
                let before: Vec<(ItemId, Item)> = origins
                    .iter()
                    .filter_map(|origin| {
                        let id = doc_view::parse::<ItemId>(&origin.clip)?;
                        Some((id, doc_view::item(self.committed(), id)?.clone()))
                    })
                    .collect();
                // Every moving clip's two edges, where the pointer would
                // put them, and the shift that lands the nearest on a target.
                let moving: Vec<&str> = origins.iter().map(|origin| origin.clip.as_str()).collect();
                let edges: Vec<f32> = before
                    .iter()
                    .flat_map(|(_, item)| {
                        let start = doc_view::start_s(item) as f32 + seconds;
                        [start, start + doc_view::len_s(item) as f32]
                    })
                    .collect();
                let shift = seconds + self.snap_shift(&edges, threshold, &moving);
                let rows = nearest_row(lanes, row_top(lanes, anchor.row) + pixels) - anchor.row;
                let count = doc_view::lanes(self.committed()).len() as i32;
                let starts: Vec<Tick> = before.iter().map(|(_, item)| item.start).collect();
                let landed_at = shifted_starts(&starts, doc_view::dur(f64::from(shift)));
                let moves: Vec<ClipMove> = before
                    .iter()
                    .zip(landed_at)
                    .map(|((id, item), start)| {
                        let row = (self.row_of_lane(item.lane) + rows).clamp(0, count - 1);
                        let lane = self
                            .row_track(row)
                            .filter(|lane| !self.lane_locked(*lane))
                            .unwrap_or(item.lane);
                        ClipMove {
                            item: *id,
                            start,
                            lane: Some(lane),
                        }
                    })
                    .collect();
                let ids: Vec<ItemId> = before.iter().map(|(id, _)| *id).collect();
                let command =
                    doc_view::sweep_strays(&self.document, Command::MoveClips { moves }, &ids);
                // The echo shows where the clips will land by the
                // document's own rule, or back where they started when it
                // would refuse the move.
                self.preview_edit(&command);
            }
            Gesture::Trim {
                clip,
                edge,
                start,
                duration,
                ..
            } => {
                let (id, edge) = (clip.clone(), *edge);
                let (start, duration) = (*start, *duration);
                let threshold = 8.0 * self.lanes.seconds_per_pixel;
                let Some(item) = doc_view::parse::<ItemId>(&id) else {
                    self.gesture = gesture;
                    return;
                };
                // The trim the release will make, previewed by the same
                // rules: it stops at the neighbours, at the ends of the
                // footage and at the shortest clip, and a magnetic one
                // moves what follows instead.
                let (trim_edge, delta) = if edge == Edge::Start {
                    let at = self.snapped(start + seconds, threshold, &id);
                    (TrimEdge::Head, at - start)
                } else {
                    let at = self.snapped(start + duration + seconds, threshold, &id);
                    (TrimEdge::Tail, at - (start + duration))
                };
                let command = doc_view::sweep_strays(
                    &self.document,
                    Command::TrimClip {
                        item,
                        edge: trim_edge,
                        delta: doc_view::dur(f64::from(delta)),
                        // Magnetic: the lane closes behind the edge.
                        ripple: self.prefs.magnetic,
                    },
                    &[item],
                );
                self.preview_edit(&command);
                // Trim follow: the playhead on the new first frame, or on
                // the new last frame - one frame inside the edge, since at
                // the edge itself the clip has already ended and the
                // monitor would show what comes after it.
                if self.prefs.trim_follow
                    && let Some(trimmed) = self.clip(&id)
                {
                    let (first, end) = (doc_view::start_s(trimmed), doc_view::end_s(trimmed));
                    if edge == Edge::Start {
                        self.seek(first as f32);
                    } else {
                        let frame = 1.0 / f64::from(self.frame_rate().max(1.0));
                        self.seek((end - frame).max(first) as f32);
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
                    let wanted = f64::from((original_duration + seconds).max(0.0));
                    if let Some(duration) = self.transition_duration(&clip_ref, wanted) {
                        self.echo_edit(&id, |echo_clip| {
                            if let Some(cut) = echo_clip.cut_in.as_mut() {
                                cut.len = doc_view::len(duration);
                            }
                        });
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
        let Some(echo) = self.echo.take() else {
            return;
        };
        match gesture {
            Gesture::Move { origins, .. } => {
                let moves: Vec<ClipMove> = origins
                    .iter()
                    .filter_map(|origin| {
                        let id = doc_view::parse::<ItemId>(&origin.clip)?;
                        let after = doc_view::item(&echo, id)?;
                        Some(ClipMove {
                            item: id,
                            start: after.start,
                            lane: Some(after.lane),
                        })
                    })
                    .collect();
                // A press and a release with no drag between them is a
                // click: nothing moved, and a MoveClips of the clips onto
                // their own places is still an undo step and still a pass
                // through the rules, which can put a clip elsewhere.
                // https://github.com/jub0t/Concat/issues/247
                if moved_any(&moves, |id| doc_view::item(self.committed(), id)) {
                    let ids: Vec<ItemId> = moves.iter().map(|wanted| wanted.item).collect();
                    let command =
                        doc_view::sweep_strays(&self.document, Command::MoveClips { moves }, &ids);
                    self.apply(command);
                }
            }
            Gesture::Trim { clip, edge, .. } => {
                let Some(id) = doc_view::parse::<ItemId>(&clip) else {
                    return;
                };
                let (Some(after), Some(before)) = (
                    doc_view::item(&echo, id),
                    doc_view::item(self.committed(), id),
                ) else {
                    return;
                };
                // The window moves with a head trim whatever closes up
                // behind it; a tail trim is the length alone.
                let delta = match edge {
                    Edge::Start => after.offset - before.offset,
                    Edge::End => after.len.dur() - before.len.dur(),
                };
                if delta != TickDur::ZERO {
                    let command = doc_view::sweep_strays(
                        &self.document,
                        Command::TrimClip {
                            item: id,
                            edge: match edge {
                                Edge::Start => TrimEdge::Head,
                                Edge::End => TrimEdge::Tail,
                            },
                            delta,
                            // Magnetic: the lane closes behind the edge.
                            // https://github.com/jub0t/Concat/issues/106
                            ripple: self.prefs.magnetic,
                        },
                        &[id],
                    );
                    self.apply(command);
                    // Trim follow: a magnetic head trim slides the clip back
                    // into the gap, and the playhead goes with its first
                    // frame.
                    if self.prefs.trim_follow
                        && edge == Edge::Start
                        && let Some(start) = self.clip(&clip).map(doc_view::start_s)
                    {
                        self.seek(start as f32);
                    }
                }
            }
            Gesture::TransitionResize {
                clip,
                original_duration,
            } => {
                let Some(after) = doc_view::item_by(&echo, &clip) else {
                    return;
                };
                let new_duration = after
                    .cut_in
                    .as_ref()
                    .map_or(0.0, |cut| doc_view::len_secs(cut.len));
                if (new_duration - f64::from(original_duration)).abs() > 1e-4 {
                    self.set_clip_transition_duration(&clip, new_duration);
                }
            }
            Gesture::None => {}
            // Not a lane gesture: hand it back untouched, echo and all.
            other @ (Gesture::StageMove { .. }
            | Gesture::StageScale { .. }
            | Gesture::StageRotate { .. }
            | Gesture::StageStretch { .. }
            | Gesture::TextWidth { .. }
            | Gesture::TextHeight { .. }
            | Gesture::StageMask { .. }
            | Gesture::Paint { .. }) => {
                self.echo = Some(echo);
                self.gesture = other;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{in_dead_zone, shifted_starts};
    use crate::doc_view::{dur, tick};

    #[test]
    fn a_wobbling_click_is_not_a_drag() {
        let spp = 0.25;
        assert!(in_dead_zone(3.0 * spp, -3.0, spp));
        assert!(!in_dead_zone(4.0 * spp, 0.0, spp));
        assert!(!in_dead_zone(0.0, 4.0, spp));
    }

    #[test]
    fn a_group_keeps_its_shape_against_the_start() {
        let starts = [tick(1.0), tick(3.0)];
        assert_eq!(
            shifted_starts(&starts, dur(2.0)),
            vec![tick(3.0), tick(5.0)]
        );
        assert_eq!(
            shifted_starts(&starts, dur(-4.0)),
            vec![tick(0.0), tick(2.0)]
        );
    }
}

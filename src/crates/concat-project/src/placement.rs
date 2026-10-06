// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Where a clip may go: no clip ever covers another on its lane.
//!
//! Lanes exist so two pieces of media never share a span of one lane. A
//! clip can sit beside another, between two, or anywhere a lane is empty;
//! it never lands on top of one. Every command that places, moves or
//! lengthens a clip asks these, and so does the window's drag, so what the
//! drag shows is what the drop does.

use std::collections::HashSet;

use crate::commands::ClipMove;
use crate::model::Timeline;

/// How far two clips may run into each other and still count as touching:
/// float seconds that should meet exactly sometimes miss by a hair.
pub const TOUCH: f64 = 1e-6;

/// Whether `[a_start, a_end)` and `[b_start, b_end)` share more than an edge.
fn covers(a_start: f64, a_end: f64, b_start: f64, b_end: f64) -> bool {
    a_start < b_end - TOUCH && b_start < a_end - TOUCH
}

impl Timeline {
    /// Whether `[start, start + duration)` on `track_id` is clear of every
    /// clip but the ones in `ignore`.
    pub fn is_free(&self, track_id: &str, start: f64, duration: f64, ignore: &[&str]) -> bool {
        let end = start + duration;
        !self.clips.iter().any(|clip| {
            clip.track_id == track_id
                && !ignore.contains(&clip.id.as_str())
                && covers(start, end, clip.start, clip.start + clip.duration)
        })
    }

    /// The start nearest `start` where `duration` fits on `track_id`
    /// without covering anything but `ignore`: `start` itself when it is
    /// clear, otherwise the closest gap before or after, never before zero.
    /// There is always one, after the lane's last clip.
    pub fn nearest_free_start(
        &self,
        track_id: &str,
        start: f64,
        duration: f64,
        ignore: &[&str],
    ) -> f64 {
        let start = start.max(0.0);
        let others = self
            .clips
            .iter()
            .filter(|clip| clip.track_id == track_id && !ignore.contains(&clip.id.as_str()));
        // Every place a gap can begin: the wanted start, the lane's start,
        // just after a clip, or just before one.
        let mut candidates = vec![start, 0.0];
        for clip in others {
            candidates.push(clip.start + clip.duration);
            candidates.push(clip.start - duration);
        }
        candidates
            .into_iter()
            .filter(|&at| at >= 0.0 && self.is_free(track_id, at, duration, ignore))
            .min_by(|a, b| (a - start).abs().total_cmp(&(b - start).abs()))
            .unwrap_or(start)
    }

    /// `moves` as they may land: every clip shifted in time by the one
    /// amount nearest zero that leaves none of them covering a clip that
    /// stays put, so a group keeps its shape and lands in the nearest gap
    /// that holds all of it. An unknown clip is dropped, and an unknown
    /// track leaves its clip on the lane it is on. None when the moved
    /// clips would cover each other, which no shift in time can fix.
    pub fn resolve_moves(&self, moves: &[ClipMove]) -> Option<Vec<ClipMove>> {
        let wanted: Vec<(ClipMove, f64)> = moves
            .iter()
            .filter_map(|wanted| {
                let clip = self.clip(&wanted.clip_id)?;
                let track_id = if self.track(&wanted.track_id).is_some() {
                    wanted.track_id.clone()
                } else {
                    clip.track_id.clone()
                };
                Some((
                    ClipMove {
                        clip_id: wanted.clip_id.clone(),
                        start: wanted.start.max(0.0),
                        track_id,
                    },
                    clip.duration,
                ))
            })
            .collect();
        if wanted.is_empty() {
            return Some(Vec::new());
        }
        for (i, (a, a_len)) in wanted.iter().enumerate() {
            for (b, b_len) in &wanted[i + 1..] {
                if a.track_id == b.track_id
                    && covers(a.start, a.start + a_len, b.start, b.start + b_len)
                {
                    return None;
                }
            }
        }

        let moving: HashSet<&str> = wanted.iter().map(|(m, _)| m.clip_id.as_str()).collect();
        let staying: Vec<_> = self
            .clips
            .iter()
            .filter(|clip| !moving.contains(clip.id.as_str()))
            .collect();
        let earliest = wanted
            .iter()
            .map(|(m, _)| m.start)
            .fold(f64::INFINITY, f64::min);
        let fits = |shift: f64| {
            earliest + shift >= -TOUCH
                && wanted.iter().all(|(m, len)| {
                    let start = m.start + shift;
                    !staying.iter().any(|clip| {
                        clip.track_id == m.track_id
                            && covers(start, start + len, clip.start, clip.start + clip.duration)
                    })
                })
        };
        // Every shift that butts one moved clip against one that stays,
        // either side, plus none and the one that lands the group at zero.
        let mut shifts = vec![0.0, -earliest];
        for (m, len) in &wanted {
            for clip in staying.iter().filter(|clip| clip.track_id == m.track_id) {
                shifts.push(clip.start + clip.duration - m.start);
                shifts.push(clip.start - len - m.start);
            }
        }
        let shift = shifts
            .into_iter()
            .filter(|&shift| fits(shift))
            .min_by(|a, b| a.abs().total_cmp(&b.abs()))?;
        Some(
            wanted
                .into_iter()
                .map(|(m, _)| ClipMove {
                    start: (m.start + shift).max(0.0),
                    ..m
                })
                .collect(),
        )
    }

    /// The gap a clip sits in on its lane: where the clip before it ends
    /// (zero when there is none) and where the clip after it starts
    /// (infinity when there is none). A trim stays inside it.
    pub fn room_around(&self, clip_id: &str) -> Option<(f64, f64)> {
        let clip = self.clip(clip_id)?;
        let end = clip.start + clip.duration;
        let neighbours = self
            .clips
            .iter()
            .filter(|other| other.track_id == clip.track_id && other.id != clip.id);
        let mut before = 0.0_f64;
        let mut after = f64::INFINITY;
        for other in neighbours {
            let other_end = other.start + other.duration;
            if other_end <= clip.start + TOUCH {
                before = before.max(other_end);
            }
            if other.start >= end - TOUCH {
                after = after.min(other.start);
            }
        }
        Some((before, after))
    }
}

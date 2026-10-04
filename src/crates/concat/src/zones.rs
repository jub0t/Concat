// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Where a clip may sit in the stack: pictures laid over the video
//! (stills, titles, shapes) and looks at the top, the video in the middle,
//! the sound at the foot.
//!
//! A lane has no type of its own; what is on it is its type. An empty lane
//! takes anything the lanes around it allow. Rows count from the top of the
//! panel, the way the lanes are drawn; `new_lane_index` alone answers in
//! the model's bottom-first order, since that is what inserting a track
//! takes.

use concat_project::model::ClipKind as ModelKind;

use crate::ui::ClipKind as UiKind;

/// What sort of thing a clip is, as far as its place in the stack goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// A still, a title or a shape: drawn over the video.
    Graphic,
    /// A look laid over a span: over the video too, on lanes of its own.
    Effect,
    Video,
    Audio,
}

impl Group {
    pub fn of_model(kind: ModelKind) -> Self {
        match kind {
            ModelKind::Video => Self::Video,
            ModelKind::Audio => Self::Audio,
            ModelKind::Image | ModelKind::Text | ModelKind::Shape => Self::Graphic,
            ModelKind::Layer => Self::Effect,
        }
    }

    pub fn of_ui(kind: UiKind) -> Self {
        match kind {
            UiKind::Video => Self::Video,
            UiKind::Audio => Self::Audio,
            UiKind::Image | UiKind::Text | UiKind::Shape => Self::Graphic,
            UiKind::Filter => Self::Effect,
        }
    }

    /// Top of the stack first. Graphics and looks share the top: they may
    /// interleave, but never share a lane.
    fn rank(self) -> u8 {
        match self {
            Self::Graphic | Self::Effect => 0,
            Self::Video => 1,
            Self::Audio => 2,
        }
    }
}

/// Whether `group` may sit on `row`, given what every row holds: the lane
/// is empty or holds the same group, nothing above it belongs lower in the
/// stack, and nothing below it higher.
pub fn valid_row(groups: &[Option<Group>], row: i32, group: Group) -> bool {
    let Ok(at) = usize::try_from(row) else {
        return false;
    };
    let Some(here) = groups.get(at) else {
        return false;
    };
    if here.is_some_and(|held| held != group) {
        return false;
    }
    let rank = group.rank();
    groups[..at].iter().flatten().all(|above| above.rank() <= rank)
        && groups[at + 1..].iter().flatten().all(|below| below.rank() >= rank)
}

/// The row a moved clip lands on: the first row allowed walking back from
/// `target` to `origin`, so a drag past the edge of the clip's zone stops
/// at that edge. `origin` when nothing on the way is allowed - a clip can
/// always stay where it is.
pub fn toward(groups: &[Option<Group>], target: i32, origin: i32, group: Group) -> i32 {
    let step = if target > origin { -1 } else { 1 };
    let mut row = target;
    while row != origin {
        if valid_row(groups, row, group) {
            return row;
        }
        row += step;
    }
    origin
}

/// The allowed row nearest `target`, either way; None when there is none.
pub fn nearest(groups: &[Option<Group>], target: i32, group: Group) -> Option<i32> {
    let count = groups.len() as i32;
    (0..count).find_map(|distance| {
        [target - distance, target + distance]
            .into_iter()
            .find(|row| valid_row(groups, *row, group))
    })
}

/// Where a new lane for `group` goes when no row allows it, as an index
/// into the model's bottom-first tracks: sound at the very bottom, graphics
/// and looks at the very top, video just over the top-most video lane - or
/// the top-most sound lane when there is no video.
pub fn new_lane_index(groups: &[Option<Group>], group: Group) -> usize {
    let count = groups.len();
    let top_most = |wanted: Group| {
        groups
            .iter()
            .position(|held| *held == Some(wanted))
            .map(|row| count - row)
    };
    match group {
        Group::Audio => 0,
        Group::Graphic | Group::Effect => count,
        Group::Video => top_most(Group::Video)
            .or_else(|| top_most(Group::Audio))
            .unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Group::*;

    /// Top to bottom: a graphic, an empty lane, video, sound, an empty lane.
    fn stack() -> Vec<Option<Group>> {
        vec![Some(Graphic), None, Some(Video), Some(Audio), None]
    }

    #[test]
    fn sound_never_rises_over_the_video() {
        let groups = stack();
        assert!(!valid_row(&groups, 0, Audio));
        assert!(!valid_row(&groups, 1, Audio));
        assert!(!valid_row(&groups, 2, Audio));
        assert!(valid_row(&groups, 3, Audio));
        assert!(valid_row(&groups, 4, Audio));
        // Dragged from its lane to the top, it stops on the top-most row
        // it may have.
        assert_eq!(toward(&groups, 0, 3, Audio), 3);
        assert_eq!(toward(&groups, 0, 4, Audio), 3);
    }

    #[test]
    fn a_graphic_never_sinks_under_the_video() {
        let groups = stack();
        assert!(valid_row(&groups, 0, Graphic));
        assert!(valid_row(&groups, 1, Graphic));
        assert!(!valid_row(&groups, 2, Graphic));
        assert!(!valid_row(&groups, 4, Graphic));
        assert_eq!(toward(&groups, 4, 0, Graphic), 1);
    }

    #[test]
    fn a_look_keeps_off_a_graphic_lane() {
        let groups = stack();
        assert!(!valid_row(&groups, 0, Effect));
        assert!(valid_row(&groups, 1, Effect));
        assert_eq!(nearest(&groups, 0, Effect), Some(1));
    }

    #[test]
    fn an_empty_lane_between_video_and_sound_takes_either_but_no_graphic() {
        let groups = vec![Some(Video), None, Some(Audio)];
        assert!(valid_row(&groups, 1, Video));
        assert!(valid_row(&groups, 1, Audio));
        assert!(!valid_row(&groups, 1, Graphic));
    }

    #[test]
    fn a_drop_with_nowhere_to_go_says_so_and_where_a_lane_belongs() {
        let groups = vec![Some(Graphic), Some(Video)];
        assert_eq!(nearest(&groups, 0, Audio), None);
        assert_eq!(new_lane_index(&groups, Audio), 0, "under everything");
        assert_eq!(nearest(&groups, 1, Effect), None);
        assert_eq!(new_lane_index(&groups, Effect), 2, "over everything");
        // Video goes just over the top-most video lane: row 1 is track
        // index 0, so the new lane is index 1, under the graphic.
        assert_eq!(new_lane_index(&groups, Video), 1);
        assert_eq!(new_lane_index(&[Some(Graphic), Some(Audio)], Video), 1);
        assert_eq!(new_lane_index(&[Some(Graphic)], Video), 0);
    }

    #[test]
    fn rows_off_the_stack_are_never_allowed() {
        let groups = stack();
        assert!(!valid_row(&groups, -1, Graphic));
        assert!(!valid_row(&groups, 5, Audio));
    }
}

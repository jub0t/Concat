// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The category bands clips may occupy in the timeline stack.

use concat_project::model::ClipKind as ModelKind;

use crate::ui::ClipKind as UiKind;

/// A clip category, ordered from the top of the stack to the bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Graphic,
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
            ModelKind::Layer | ModelKind::SoundLayer | ModelKind::Transition => Self::Effect,
        }
    }

    pub fn of_ui(kind: UiKind) -> Self {
        match kind {
            UiKind::Video => Self::Video,
            UiKind::Audio => Self::Audio,
            UiKind::Image | UiKind::Text | UiKind::Shape => Self::Graphic,
            UiKind::Filter
            | UiKind::Effect
            | UiKind::Adjustment
            | UiKind::Transition
            | UiKind::SoundLayer => Self::Effect,
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Graphic | Self::Effect => 0,
            Self::Video => 1,
            Self::Audio => 2,
        }
    }
}

/// Whether `group` may occupy `row`, given what every row already holds.
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
    groups[..at]
        .iter()
        .flatten()
        .all(|above| above.rank() <= rank)
        && groups[at + 1..]
            .iter()
            .flatten()
            .all(|below| below.rank() >= rank)
}

/// The first valid row walking back from `target` towards `origin`.
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

/// The valid row nearest `target`, either way.
pub fn nearest(groups: &[Option<Group>], target: i32, group: Group) -> Option<i32> {
    let count = groups.len() as i32;
    (0..count).find_map(|distance| {
        [target - distance, target + distance]
            .into_iter()
            .find(|row| valid_row(groups, *row, group))
    })
}

/// Where a new lane for `group` goes in the model's bottom-first list.
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

    fn stack() -> Vec<Option<Group>> {
        vec![Some(Graphic), None, Some(Video), Some(Audio), None]
    }

    #[test]
    fn clips_stay_inside_their_category_band() {
        let groups = stack();
        assert_eq!(toward(&groups, 0, 3, Audio), 3);
        assert_eq!(toward(&groups, 4, 0, Graphic), 1);
        assert_eq!(nearest(&groups, 0, Effect), Some(1));
    }

    #[test]
    fn a_new_lane_goes_at_its_band_edge() {
        let groups = [Some(Graphic), Some(Video)];
        assert_eq!(new_lane_index(&groups, Audio), 0);
        assert_eq!(new_lane_index(&groups, Effect), 2);
        assert_eq!(new_lane_index(&groups, Video), 1);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The audio-only batch exposed by the inspector, separate from title styling.
use concat_project::model::{ClipKind, Project};

/// A batch must consist entirely of sound sources. Mixed selections are not
/// silently reduced to a subset; locked lanes are left untouched.
pub fn targets(
    project: &Project,
    selection: &[String],
    locked: impl Fn(&str) -> bool,
) -> Vec<String> {
    if selection.len() < 2 {
        return Vec::new();
    }
    let all_sound = selection.iter().all(|id| {
        project
            .active()
            .clip(id)
            .is_some_and(|clip| match clip.kind {
                ClipKind::Audio => true,
                ClipKind::Video => project
                    .media_by_id(&clip.media_id)
                    .is_some_and(|m| m.has_audio),
                _ => false,
            })
    });
    if all_sound {
        selection
            .iter()
            .filter(|id| {
                project
                    .active()
                    .clip(id)
                    .is_some_and(|clip| !locked(&clip.track_id))
            })
            .cloned()
            .collect()
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_project::model::Clip;

    fn project(kinds: &[ClipKind]) -> (Project, Vec<String>) {
        let mut project = Project::new();
        let mut selection = Vec::new();
        for (i, kind) in kinds.iter().enumerate() {
            let id = format!("c{i}");
            project
                .active_mut()
                .clips
                .push(Clip::blank(&id, "T1", *kind, "clip", 0.0, 10.0).into());
            selection.push(id);
        }
        (project, selection)
    }

    #[test]
    fn audio_clips_keep_selection_order() {
        let (project, mut ids) = project(&[ClipKind::Audio, ClipKind::Audio]);
        ids.reverse();
        assert_eq!(targets(&project, &ids, |_| false), ids);
    }

    #[test]
    fn mixed_or_missing_clips_do_not_enable_batch_volume() {
        for kind in [ClipKind::Text, ClipKind::Image, ClipKind::Video] {
            let (project, ids) = project(&[ClipKind::Audio, kind]);
            assert!(targets(&project, &ids, |_| false).is_empty());
        }
        let (project, mut ids) = project(&[ClipKind::Audio, ClipKind::Audio]);
        ids.push("missing".into());
        assert!(targets(&project, &ids, |_| false).is_empty());
    }

    #[test]
    fn video_requires_an_audio_stream_and_can_mix_with_audio_clips() {
        let (mut project, ids) = project(&[ClipKind::Audio, ClipKind::Video]);
        std::sync::Arc::make_mut(&mut project.active_mut().clips[1]).media_id = "m1".into();
        project.media.push(concat_project::model::MediaItem {
            id: "m1".into(),
            has_audio: true,
            ..Default::default()
        });
        assert_eq!(targets(&project, &ids, |_| false), ids);
        project.media[0].has_audio = false;
        assert!(targets(&project, &ids, |_| false).is_empty());
    }

    #[test]
    fn locked_lanes_are_not_targets_even_when_only_one_unlocked_clip_remains() {
        let (mut project, ids) = project(&[ClipKind::Audio, ClipKind::Audio]);
        std::sync::Arc::make_mut(&mut project.active_mut().clips[1]).track_id = "T2".into();
        assert_eq!(
            targets(&project, &ids, |track| track == "T1"),
            vec![ids[1].clone()]
        );
        assert!(targets(&project, &ids, |_| true).is_empty());
    }

    #[test]
    fn single_selection_stays_with_the_existing_inspector() {
        let (project, ids) = project(&[ClipKind::Audio]);
        assert!(targets(&project, &ids, |_| false).is_empty());
        assert!(targets(&project, &[], |_| false).is_empty());
    }
}

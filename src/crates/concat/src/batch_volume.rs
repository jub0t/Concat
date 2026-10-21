// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The audio-only batch exposed by the inspector, separate from title styling.
use concat_project::model::{Clip, ClipKind, KeyProperty, Project};

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

/// Sets one clip of a batch to `level` (a gain), the way CapCut's batch
/// volume does: it is the clip's level that changes, not a key at the
/// playhead. A clip with no keys on its volume takes the level as it is.
/// A clip whose volume rides keeps its ride: every key moves by the same
/// number of decibels - the knob's unit - so the gain at `at` lands on
/// `level` and a fade keeps its shape, a key at silence staying silent.
/// Where the ride is silent at `at`, its loudest key is what lands on
/// `level`; a ride silent everywhere is lifted to it.
///
/// `original` is the clip as the document has it: each step of a drag is
/// read against where the drag began, so the shape is never worn down.
pub fn set_level(clip: &mut Clip, original: &Clip, level: f64, at: f64) {
    let level = level.max(0.0);
    if !original.is_keyed(KeyProperty::Volume) {
        clip.volume = level;
        return;
    }
    let loudest = original
        .keys_on(KeyProperty::Volume)
        .map(|key| key.value)
        .fold(0.0_f64, f64::max);
    let here = original.value_at(KeyProperty::Volume, at);
    let from = if here > 1e-6 { here } else { loudest };
    let values = original.keys_on(KeyProperty::Volume).map(|key| {
        if from > 1e-6 {
            key.value * level / from
        } else {
            level
        }
    });
    let keys = clip
        .keys
        .iter_mut()
        .filter(|key| key.property == KeyProperty::Volume);
    for (key, value) in keys.zip(values) {
        key.value = value.max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_project::model::{ClipKey, KeyEase};

    fn ride(values: &[(f64, f64)]) -> Clip {
        let mut clip = Clip::blank("c", "T1", ClipKind::Audio, "clip", 0.0, 10.0);
        clip.keys = values
            .iter()
            .map(|&(at, value)| ClipKey {
                property: KeyProperty::Volume,
                at,
                value,
                ease: KeyEase::LINEAR,
            })
            .collect();
        clip
    }

    fn volumes(clip: &Clip) -> Vec<f64> {
        clip.keys_on(KeyProperty::Volume)
            .map(|key| key.value)
            .collect()
    }

    #[test]
    fn an_unkeyed_clip_takes_the_level_and_gains_no_key() {
        let original = Clip::blank("c", "T1", ClipKind::Audio, "clip", 0.0, 10.0);
        let mut clip = original.clone();
        set_level(&mut clip, &original, 0.5, 0.3);
        assert_eq!(clip.volume, 0.5);
        assert!(clip.keys.is_empty());
    }

    #[test]
    fn a_keyed_clip_moves_its_whole_ride_and_keeps_a_fade_to_silence() {
        // Full, then a fade to silence; the playhead at the start.
        let original = ride(&[(0.0, 1.0), (0.5, 1.0), (1.0, 0.0)]);
        let mut clip = original.clone();
        set_level(&mut clip, &original, 0.5, 0.0);
        assert_eq!(volumes(&clip), vec![0.5, 0.5, 0.0]);
        assert_eq!(clip.keys.len(), 3, "no key is added at the playhead");
        // A drag's next step is read against the document, not the last
        // step: the same answer, however many steps it took.
        set_level(&mut clip, &original, 0.5, 0.0);
        assert_eq!(volumes(&clip), vec![0.5, 0.5, 0.0]);
        // At the silent end, the loudest key is what lands on the level.
        let mut clip = original.clone();
        set_level(&mut clip, &original, 2.0, 1.0);
        assert_eq!(volumes(&clip), vec![2.0, 2.0, 0.0]);
        // Silent everywhere: lifted to the level.
        let silent = ride(&[(0.0, 0.0), (1.0, 0.0)]);
        let mut clip = silent.clone();
        set_level(&mut clip, &silent, 0.8, 0.5);
        assert_eq!(volumes(&clip), vec![0.8, 0.8]);
    }

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

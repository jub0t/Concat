// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The audio-only batch exposed by the inspector, separate from title styling.
use concat_doc::Tick;
use concat_doc::model::{Item, Model};

use crate::doc_view::{self, Kind};

/// A batch must consist entirely of sound sources. Mixed selections are not
/// silently reduced to a subset; locked lanes are left untouched.
pub fn targets(doc: &Model, selection: &[String], locked: impl Fn(&str) -> bool) -> Vec<String> {
    if selection.len() < 2 {
        return Vec::new();
    }
    let all_sound = selection.iter().all(|id| {
        doc_view::item_by(doc, id).is_some_and(|item| match doc_view::kind(doc, item) {
            Kind::Audio => true,
            Kind::Video => doc_view::has_sound(doc, item),
            _ => false,
        })
    });
    if all_sound {
        selection
            .iter()
            .filter(|id| {
                doc_view::item_by(doc, id).is_some_and(|item| !locked(&item.lane.to_string()))
            })
            .cloned()
            .collect()
    } else {
        Vec::new()
    }
}

/// Sets one clip of a batch to `level` (a gain), as a batch volume
/// should: it is the clip's level that changes, not a key at the
/// playhead. A clip with no keys on its volume takes the level as it is.
/// A clip whose volume rides keeps its ride: every key moves by the same
/// number of decibels - the knob's unit - so the gain at `at` lands on
/// `level` and a fade keeps its shape, a key at silence staying silent.
/// Where the ride is silent at `at`, its loudest key is what lands on
/// `level`; a ride silent everywhere is lifted to it.
///
/// `original` is the clip as the document has it: each step of a drag is
/// read against where the drag began, so the shape is never worn down.
/// `at` is on the clip's content time.
pub fn set_level(clip: &mut Item, original: &Item, level: f64, at: Tick) {
    let level = level.max(0.0);
    let ride = &original.sound.volume;
    if !ride.is_keyed() {
        clip.sound.volume.value = level;
        return;
    }
    let loudest = ride
        .keys
        .iter()
        .map(|key| key.value)
        .fold(0.0_f64, f64::max);
    let here = doc_view::value_at(ride, at);
    let from = if here > 1e-6 { here } else { loudest };
    let values = ride.keys.iter().map(|key| {
        if from > 1e-6 {
            key.value * level / from
        } else {
            level
        }
    });
    for (key, value) in clip.sound.volume.keys.iter_mut().zip(values) {
        key.value = value.max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc_view::bench::Bench;
    use crate::doc_view::{len, tick};
    use concat_doc::commands::Command;
    use concat_doc::model::{Content, Key, MediaCut, MediaKind};

    fn ride(values: &[(f64, f64)]) -> Item {
        let mut clip = Item::new(
            doc_view::parse("t1").unwrap(),
            Tick::ZERO,
            len(10.0),
            Content::Media(MediaCut::whole(doc_view::parse("m1").unwrap())),
        );
        clip.sound.volume.keys = values
            .iter()
            .map(|&(at, value)| Key::linear(tick(at), value))
            .collect();
        clip
    }

    fn volumes(clip: &Item) -> Vec<f64> {
        clip.sound.volume.keys.iter().map(|key| key.value).collect()
    }

    #[test]
    fn an_unkeyed_clip_takes_the_level_and_gains_no_key() {
        let original = ride(&[]);
        let mut clip = original.clone();
        set_level(&mut clip, &original, 0.5, tick(3.0));
        assert_eq!(clip.sound.volume.value, 0.5);
        assert!(clip.sound.volume.keys.is_empty());
    }

    #[test]
    fn a_keyed_clip_moves_its_whole_ride_and_keeps_a_fade_to_silence() {
        // Full, then a fade to silence; the playhead at the start.
        let original = ride(&[(0.0, 1.0), (5.0, 1.0), (10.0, 0.0)]);
        let mut clip = original.clone();
        set_level(&mut clip, &original, 0.5, tick(0.0));
        assert_eq!(volumes(&clip), vec![0.5, 0.5, 0.0]);
        assert_eq!(
            clip.sound.volume.keys.len(),
            3,
            "no key is added at the playhead"
        );
        // A drag's next step is read against the document, not the last
        // step: the same answer, however many steps it took.
        set_level(&mut clip, &original, 0.5, tick(0.0));
        assert_eq!(volumes(&clip), vec![0.5, 0.5, 0.0]);
        // At the silent end, the loudest key is what lands on the level.
        let mut clip = original.clone();
        set_level(&mut clip, &original, 2.0, tick(10.0));
        assert_eq!(volumes(&clip), vec![2.0, 2.0, 0.0]);
        // Silent everywhere: lifted to the level.
        let silent = ride(&[(0.0, 0.0), (10.0, 0.0)]);
        let mut clip = silent.clone();
        set_level(&mut clip, &silent, 0.8, tick(5.0));
        assert_eq!(volumes(&clip), vec![0.8, 0.8]);
    }

    /// A bench with a clip of each kind asked for, side by side on the
    /// main lane, and their ids in order.
    fn placed(kinds: &[MediaKind], sound_alone: &[bool]) -> (Bench, Vec<String>) {
        let mut bench = Bench::new();
        let lane = bench.main_lane();
        let mut ids = Vec::new();
        for (at, kind) in kinds.iter().enumerate() {
            let media = bench.add_media(&format!("f{at}"), kind.clone(), 2.0);
            let id = bench.add_clip(media, lane, at as f64 * 3.0);
            if sound_alone.get(at).copied().unwrap_or(false) {
                bench.apply(&Command::DetachAudio { item: id });
            }
            ids.push(id.to_string());
        }
        (bench, ids)
    }

    #[test]
    fn audio_clips_keep_selection_order() {
        let (bench, mut ids) = placed(&[MediaKind::Audio, MediaKind::Audio], &[]);
        ids.reverse();
        assert_eq!(targets(bench.doc(), &ids, |_| false), ids);
    }

    #[test]
    fn mixed_or_missing_clips_do_not_enable_batch_volume() {
        let (bench, ids) = placed(&[MediaKind::Audio, MediaKind::Image], &[]);
        assert!(targets(bench.doc(), &ids, |_| false).is_empty());
        let (bench, mut ids) = placed(&[MediaKind::Audio, MediaKind::Audio], &[]);
        ids.push("c999".into());
        assert!(targets(bench.doc(), &ids, |_| false).is_empty());
        ids.pop();
        ids.push("missing".into());
        assert!(targets(bench.doc(), &ids, |_| false).is_empty());
    }

    #[test]
    fn video_with_sound_can_mix_with_audio_clips() {
        let (bench, ids) = placed(&[MediaKind::Audio, MediaKind::Video], &[]);
        assert_eq!(targets(bench.doc(), &ids, |_| false), ids);
    }

    #[test]
    fn locked_lanes_are_not_targets_even_when_only_one_unlocked_clip_remains() {
        let (bench, ids) = placed(&[MediaKind::Audio, MediaKind::Audio], &[]);
        let main = bench.main_lane().to_string();
        assert!(targets(bench.doc(), &ids, |lane| lane == main).is_empty());
        let mut bench = bench;
        let other = bench.lane(0);
        let second: concat_doc::model::ItemId = doc_view::parse(&ids[1]).unwrap();
        let start = bench.doc().items.get(second).unwrap().start;
        bench.apply(&Command::MoveClips {
            moves: vec![concat_doc::commands::ClipMove {
                item: second,
                start,
                lane: Some(other),
            }],
        });
        assert_eq!(
            targets(bench.doc(), &ids, |lane| lane == main),
            vec![ids[1].clone()]
        );
        assert!(targets(bench.doc(), &ids, |_| true).is_empty());
    }

    #[test]
    fn single_selection_stays_with_the_existing_inspector() {
        let (bench, ids) = placed(&[MediaKind::Audio], &[]);
        assert!(targets(bench.doc(), &ids, |_| false).is_empty());
        assert!(targets(bench.doc(), &[], |_| false).is_empty());
    }
}

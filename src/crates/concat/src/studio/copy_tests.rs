// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How a copy of a clip comes out, as Duplicate and Paste make it. On the
//! timeline it came from, the document copies it whole and the copy is
//! moved into place; anywhere else its file is laid past the last clip on
//! the lane it is going to, then shaped by one batch of edits into a copy
//! of the source at the asked-for start. These tests drive both on a
//! document alone, with no window.

use concat_doc::commands::{Command, Edge, speed_from_decimal, speed_graph};
use concat_doc::model::SpeedPoint;
use concat_doc::model::{
    Effect, Item, ItemId, Key, LaneId, Mask, MediaId, MediaKind, Model, TimeMap,
};
use concat_doc::{Document, Tick, TickDur};

use super::menus::{lay_copy, match_commands, move_copy, shape_copy};
use crate::doc_view::{self, bench::Bench};

/// Seconds a length may be off by and still be the same: well under a
/// microsecond.
const CLOSE: f64 = 1e-6;

struct Copier {
    bench: Bench,
    media: MediaId,
    lane: LaneId,
}

impl Copier {
    fn new(kind: MediaKind, seconds: f64) -> Self {
        let mut bench = Bench::new();
        let media = bench.add_media("take.mov", kind, seconds);
        let lane = bench.main_lane();
        Copier { bench, media, lane }
    }

    fn doc(&self) -> &Model {
        self.bench.doc()
    }

    fn run(&mut self, command: Command) -> Option<ItemId> {
        self.bench
            .apply(&command)
            .created
            .and_then(|id| id.typed::<ItemId>())
    }

    fn clip(&self, id: ItemId) -> Item {
        doc_view::item(self.doc(), id)
            .expect("the clip is on the timeline")
            .clone()
    }

    fn clips(&self) -> Vec<(ItemId, Item)> {
        doc_view::items(self.doc())
            .into_iter()
            .map(|(id, item)| (id, item.clone()))
            .collect()
    }

    /// The whole file laid at `start` on the main lane.
    fn lay(&mut self, start: f64) -> ItemId {
        self.bench.add_clip(self.media, self.lane, start)
    }

    fn trim(&mut self, id: ItemId, edge: Edge, delta: f64) {
        self.run(Command::TrimClip {
            item: id,
            edge,
            delta: doc_view::dur(delta),
            ripple: false,
        });
    }

    fn speed(&mut self, id: ItemId, speed: f64) {
        self.run(Command::SetClipSpeed {
            item: id,
            speed: speed_from_decimal(speed).expect("a speed"),
            ripple: false,
        });
    }

    /// A new, empty lane on top.
    fn other_lane(&mut self) -> LaneId {
        self.bench
            .apply(&Command::AddTrack { bottom: false })
            .created
            .and_then(|id| id.typed())
            .expect("a second lane")
    }

    /// Cuts `id` at `time` and says the two pieces, head first.
    fn split(&mut self, id: ItemId, time: f64) -> ((ItemId, Item), (ItemId, Item)) {
        let tail = self
            .run(Command::SplitClips {
                items: vec![id],
                at: doc_view::tick(time),
            })
            .expect("the cut makes a second piece");
        ((id, self.clip(id)), (tail, self.clip(tail)))
    }

    /// A copy of `source` on `lane` at `start`, made the way a paste from
    /// elsewhere makes one: the file past the lane's last clip, then one
    /// batch.
    fn copy(&mut self, source: &Item, lane: LaneId, start: f64) -> ItemId {
        let lay = lay_copy(self.doc(), source, lane).expect("a clip can be laid");
        let laid = self.run(lay).expect("the file is laid");
        let commands = shape_copy(self.doc(), source, laid, lane, doc_view::tick(start));
        self.bench.apply(&Command::Batch { commands });
        laid
    }

    /// A copy of the clip `original` on `lane` at `start`, made the way a
    /// paste on its own timeline makes one: copied whole, then moved.
    fn copy_whole(&mut self, original: ItemId, lane: LaneId, start: f64) -> ItemId {
        let source = self.clip(original);
        let copy = self
            .run(Command::DuplicateClips {
                items: vec![original],
            })
            .expect("a copy");
        let commands = move_copy(self.doc(), &source, copy, lane, doc_view::tick(start));
        self.bench.apply(&Command::Batch { commands });
        copy
    }
}

fn near(left: f64, right: f64) -> bool {
    (left - right).abs() < CLOSE
}

/// The footage a clip shows - from where, and how much - and the time it
/// takes to show it.
fn assert_same_footage(copy: &Item, source: &Item) {
    let (Some((from, span)), Some((source_from, source_span))) =
        (doc_view::covered(copy), doc_view::covered(source))
    else {
        // A still covers nothing but its length.
        assert_eq!(copy.len, source.len, "length");
        return;
    };
    assert!(
        near(doc_view::secs(from), doc_view::secs(source_from)),
        "in-point {} for {}",
        doc_view::secs(from),
        doc_view::secs(source_from)
    );
    assert!(
        near(doc_view::dur_secs(span), doc_view::dur_secs(source_span)),
        "footage {} for {}",
        doc_view::dur_secs(span),
        doc_view::dur_secs(source_span)
    );
    assert!(
        near(doc_view::len_s(copy), doc_view::len_s(source)),
        "length {} for {}",
        doc_view::len_s(copy),
        doc_view::len_s(source)
    );
    assert!(
        near(doc_view::speed(copy), doc_view::speed(source)),
        "speed {} for {}",
        doc_view::speed(copy),
        doc_view::speed(source)
    );
}

/// The second piece of a cut clip, copied right after itself, shows that
/// piece's frames at its speed, not the start of the file; both pieces
/// stay as they were.
#[test]
fn a_copy_of_the_second_piece_of_a_cut_shows_that_piece() {
    for speed in [1.0, 2.0, 0.5] {
        let mut copier = Copier::new(MediaKind::Video, 10.0);
        let whole = copier.lay(0.0);
        copier.speed(whole, speed);
        let ((head_id, head), (tail_id, tail)) = copier.split(whole, 2.0);
        let end = doc_view::end_s(&tail);

        let lane = copier.lane;
        let copy = copier.copy(&tail, lane, end);

        let copy = copier.clip(copy);
        assert!(
            near(doc_view::start_s(&copy), end),
            "at speed {speed}: starts {}",
            doc_view::start_s(&copy)
        );
        assert_eq!(copy.lane, tail.lane);
        assert_same_footage(&copy, &tail);
        assert_eq!(copier.clip(head_id), head, "at speed {speed}");
        assert_eq!(copier.clip(tail_id), tail, "at speed {speed}");
        assert_eq!(copier.clips().len(), 3, "at speed {speed}");
    }
}

/// The first piece of a cut starts at the file's first frame, and its copy
/// does too, with the piece's length.
#[test]
fn a_copy_of_the_first_piece_starts_at_the_first_frame() {
    let mut copier = Copier::new(MediaKind::Video, 10.0);
    let whole = copier.lay(0.0);
    let ((_, head), (tail_id, tail)) = copier.split(whole, 4.0);
    let end = doc_view::end_s(&tail);

    let lane = copier.lane;
    let copy = copier.copy(&head, lane, end);

    let copy = copier.clip(copy);
    assert!(near(doc_view::start_s(&copy), end));
    assert!(near(doc_view::in_point_s(&copy), 0.0));
    assert_same_footage(&copy, &head);
    assert_eq!(copier.clip(tail_id), tail);
}

/// A clip trimmed at its head and lying late on the lane, copied to the
/// start of the timeline, keeps its in-point: landing earlier on the
/// timeline than the in-point is into the file changes nothing.
#[test]
fn a_copy_laid_before_its_in_point_keeps_the_in_point() {
    let mut copier = Copier::new(MediaKind::Video, 10.0);
    let late = copier.lay(20.0);
    copier.trim(late, Edge::Head, 6.0);
    let source = copier.clip(late);
    assert!(near(doc_view::in_point_s(&source), 6.0) && near(doc_view::len_s(&source), 4.0));

    let lane = copier.lane;
    let copy = copier.copy(&source, lane, 0.0);

    let copy = copier.clip(copy);
    assert!(near(doc_view::start_s(&copy), 0.0));
    assert_same_footage(&copy, &source);
    assert_eq!(copier.clip(late), source);
}

fn curve(copier: &mut Copier, id: ItemId) {
    let point = |at, speed| SpeedPoint {
        at,
        speed,
        slope: None,
    };
    copier.run(Command::SetClipSpeedCurve {
        item: id,
        curve: Some(vec![point(0.0, 0.5), point(1.0, 2.0)]),
        ripple: false,
    });
}

/// A piece cut from inside a clip retimed by a speed curve, copied on its
/// own timeline, copies with the curve, the in-point and the length that
/// piece has: the document copies it whole.
#[test]
fn a_copy_keeps_the_speed_curve() {
    let mut copier = Copier::new(MediaKind::Video, 10.0);
    let whole = copier.lay(0.0);
    curve(&mut copier, whole);
    let curved = copier.clip(whole);
    assert!(speed_graph(&curved).is_some());
    let cut = doc_view::start_s(&curved) + doc_view::len_s(&curved) * 0.4;
    let (_, (tail_id, tail)) = copier.split(whole, cut);
    let end = doc_view::end_s(&tail);

    let lane = copier.lane;
    let copy = copier.copy_whole(tail_id, lane, end + 1.0);

    let copy = copier.clip(copy);
    assert!(near(doc_view::start_s(&copy), end + 1.0));
    assert_same_footage(&copy, &tail);
    assert_eq!(doc_view::time_map(&copy), doc_view::time_map(&tail));
    assert_eq!(copy.offset, tail.offset);
    assert_eq!(copier.clip(tail_id), tail);
}

/// Laid and shaped, as a paste from another timeline is, a curved piece
/// keeps its footage exactly and its length to within a frame, and the
/// points its curve was drawn with.
#[test]
fn a_copy_laid_elsewhere_keeps_a_curved_piece_s_footage() {
    let mut copier = Copier::new(MediaKind::Video, 10.0);
    let whole = copier.lay(0.0);
    curve(&mut copier, whole);
    let curved = copier.clip(whole);
    let cut = doc_view::start_s(&curved) + doc_view::len_s(&curved) * 0.4;
    let (_, (_, tail)) = copier.split(whole, cut);
    let other = copier.other_lane();

    let copy = copier.copy(&tail, other, 0.0);

    let copy = copier.clip(copy);
    let (from, span) = doc_view::covered(&copy).expect("footage");
    let (source_from, source_span) = doc_view::covered(&tail).expect("footage");
    assert!(near(doc_view::secs(from), doc_view::secs(source_from)));
    assert!(near(
        doc_view::dur_secs(span),
        doc_view::dur_secs(source_span)
    ));
    assert!((doc_view::len_s(&copy) - doc_view::len_s(&tail)).abs() < 1.0 / 30.0);
    // The speed pane draws the copy with the piece's own points.
    assert_eq!(speed_graph(&copy), speed_graph(&tail));
    assert!(speed_graph(&copy).is_some());
}

/// Everything else the source has comes with it - mix, fades, transform,
/// keys, looks, masks, effects - and the copy says it is one by its name.
/// Two things stay behind: the transition on the cut into the source, and
/// the tie to a video its sound was detached from.
#[test]
fn a_copy_carries_everything_but_its_place_and_ties() {
    let mut copier = Copier::new(MediaKind::Video, 10.0);
    let laid = copier.lay(0.0);
    copier.trim(laid, Edge::Head, 1.5);
    copier.trim(laid, Edge::Tail, -2.0);
    copier.speed(laid, 1.25);

    // The source dressed: everything a copy must carry, put on the way
    // the copy will be.
    let mut dressed = copier.clip(laid);
    dressed.sound.volume.value = 0.4;
    dressed.sound.fade_in = doc_view::dur(0.5);
    dressed.sound.fade_out = doc_view::dur(0.75);
    dressed.picture.scale.value = 1.3;
    dressed.picture.offset_x.value = 0.1;
    dressed.picture.offset_y.value = -0.2;
    dressed.picture.rotation.value = 15.0;
    dressed.picture.opacity.value = 0.8;
    dressed.picture.flip_h = true;
    dressed.picture.blend = concat_doc::model::Blend::Screen;
    dressed.sound.muted = true;
    let from = dressed.offset;
    dressed.picture.scale.keys = vec![
        Key::linear(from, 1.0),
        Key::linear(from + TickDur::SECOND, 2.0),
    ];
    let placeholder = concat_doc::commands::placeholder();
    let blur = Effect::new(placeholder, "concat.blur");
    let echo = Effect::new(placeholder, "concat.echo");
    let mut mask = Mask::circle(concat_doc::commands::placeholder());
    mask.x.value = 0.25;
    let keys = match_commands(
        &Item {
            effects: vec![concat_doc::model::Entry::Known(blur)],
            filters: vec![concat_doc::model::Entry::Known(echo)],
            masks: vec![concat_doc::model::Entry::Known(mask)],
            ..dressed.clone()
        },
        laid,
        TickDur::ZERO,
    );
    copier.bench.apply(&Command::Batch { commands: keys });
    // Dressed as a copy is, it is named as one: put its own name back.
    copier.run(Command::UpdateClip {
        item: laid,
        patch: concat_doc::commands::ItemPatch {
            name: Some("take.mov".to_owned()),
            ..Default::default()
        },
    });
    // Ties the copy does not take: they belong to where the source sits.
    let mut source = copier.clip(laid);
    source.cut_in = Some(concat_doc::model::CutIn {
        effect: Effect::new(concat_doc::commands::placeholder(), "concat.cross-fade"),
        len: doc_view::len(0.5),
    });

    let lane = copier.lane;
    let end = doc_view::end_s(&source);
    let copy = copier.copy(&source, lane, end);

    let copy = copier.clip(copy);
    assert_same_footage(&copy, &source);
    assert!(near(doc_view::start_s(&copy), end));
    assert_eq!(copy.name, "take.mov copy");
    assert_eq!(copy.cut_in, None);
    assert_eq!(copy.link, None);
    assert_eq!(copy.sound, source.sound);
    assert_eq!(copy.picture.blend, source.picture.blend);
    assert_eq!(copy.picture.flip_h, source.picture.flip_h);
    assert_eq!(copy.picture.opacity.value, source.picture.opacity.value);
    assert_eq!(copy.picture.scale.value, source.picture.scale.value);
    assert_eq!(copy.picture.rotation.value, source.picture.rotation.value);
    // The keys are where they were in the picture, on the copy's own
    // content time.
    let shift = copy.offset - source.offset;
    let moved: Vec<(Tick, f64)> = source
        .picture
        .scale
        .keys
        .iter()
        .map(|key| (key.at + shift, key.value))
        .collect();
    let kept: Vec<(Tick, f64)> = copy
        .picture
        .scale
        .keys
        .iter()
        .map(|key| (key.at, key.value))
        .collect();
    assert_eq!(kept, moved);
    let packages = |item: &Item| -> Vec<String> {
        doc_view::chain(&item.effects)
            .iter()
            .chain(doc_view::chain(&item.filters).iter())
            .map(|effect| effect.package.clone())
            .collect()
    };
    assert_eq!(packages(&copy), packages(&source));
    let masks = doc_view::masks(&copy.masks);
    assert_eq!(masks.len(), 1);
    assert_eq!(masks[0].x.value, 0.25);

    // A copy of a copy says so again.
    let again = copier.copy(&copy, lane, end + doc_view::len_s(&copy));
    assert_eq!(copier.clip(again).name, "take.mov copy copy");
}

/// A start that would cover a clip already there is not where the copy
/// goes: it takes the nearest gap that holds it, and the clips there are
/// neither moved nor trimmed.
#[test]
fn a_copy_onto_a_taken_spot_takes_the_nearest_gap() {
    let mut copier = Copier::new(MediaKind::Video, 10.0);
    let first = copier.lay(0.0);
    copier.trim(first, Edge::Tail, -7.0);
    let second = copier.lay(5.0);
    copier.trim(second, Edge::Tail, -7.0);
    let first_clip = copier.clip(first);
    let second_clip = copier.clip(second);
    assert!(near(doc_view::start_s(&second_clip), 5.0) && near(doc_view::len_s(&second_clip), 3.0));

    let lane = copier.lane;
    let copy = copier.copy(&first_clip, lane, 4.0);

    let copy = copier.clip(copy);
    // 3..5 is too short for three seconds; after the second clip is the
    // nearest room.
    assert!(
        near(doc_view::start_s(&copy), 8.0),
        "starts {}",
        doc_view::start_s(&copy)
    );
    assert_same_footage(&copy, &first_clip);
    assert_eq!(copier.clip(first), first_clip);
    assert_eq!(copier.clip(second), second_clip);
}

/// A copy can go to another lane, an empty one, at the asked-for start;
/// the source's lane is left alone.
#[test]
fn a_copy_lands_on_the_lane_asked_for() {
    let mut copier = Copier::new(MediaKind::Video, 10.0);
    let whole = copier.lay(0.0);
    let (_, (_, tail)) = copier.split(whole, 3.0);
    let other = copier.other_lane();

    let copy = copier.copy(&tail, other, 1.0);

    let copy = copier.clip(copy);
    assert_eq!(copy.lane, other);
    assert!(near(doc_view::start_s(&copy), 1.0));
    assert_same_footage(&copy, &tail);
    let lane = copier.lane;
    assert_eq!(doc_view::on_lane(copier.doc(), lane).count(), 2);
}

/// The same, copied whole on its own timeline.
#[test]
fn a_whole_copy_lands_on_the_lane_asked_for() {
    let mut copier = Copier::new(MediaKind::Video, 10.0);
    let whole = copier.lay(0.0);
    let (_, (tail_id, tail)) = copier.split(whole, 3.0);
    let other = copier.other_lane();

    let copy = copier.copy_whole(tail_id, other, 1.0);

    let copy = copier.clip(copy);
    assert_eq!(copy.lane, other);
    assert!(near(doc_view::start_s(&copy), 1.0));
    assert_same_footage(&copy, &tail);
    assert_eq!(copy.name, format!("{} copy", tail.name));
    let lane = copier.lane;
    assert_eq!(doc_view::on_lane(copier.doc(), lane).count(), 2);
}

/// A start before the timeline's beginning is the beginning.
#[test]
fn a_copy_never_starts_before_zero() {
    let mut copier = Copier::new(MediaKind::Video, 10.0);
    let whole = copier.lay(0.0);
    let (_, (_, tail)) = copier.split(whole, 6.0);
    let other = copier.other_lane();

    let copy = copier.copy(&tail, other, -3.0);

    let copy = copier.clip(copy);
    assert!(
        near(doc_view::start_s(&copy), 0.0),
        "starts {}",
        doc_view::start_s(&copy)
    );
    assert_same_footage(&copy, &tail);
}

/// Sound alone copies the same way: the piece's in-point and length.
#[test]
fn a_copy_of_a_sound_piece_shows_that_piece() {
    let mut copier = Copier::new(MediaKind::Audio, 30.0);
    let whole = copier.lay(0.0);
    copier.speed(whole, 1.5);
    let ((head_id, head), (_, tail)) = copier.split(whole, 7.0);
    let end = doc_view::end_s(&tail);

    let lane = copier.lane;
    let copy = copier.copy(&tail, lane, end);

    let copy = copier.clip(copy);
    assert_eq!(
        doc_view::kind(copier.doc(), &copy),
        doc_view::kind(copier.doc(), &tail)
    );
    assert!(near(doc_view::start_s(&copy), end));
    assert_same_footage(&copy, &tail);
    assert_eq!(copier.clip(head_id), head);
}

/// A still has no length of its own: a copy of one held longer or shorter
/// than a fresh still is held exactly as long as its source.
#[test]
fn a_copy_of_a_still_is_held_as_long_as_its_source() {
    for change in [7.0, -2.0] {
        let mut copier = Copier::new(MediaKind::Image, 0.0);
        let still = copier.lay(0.0);
        copier.trim(still, Edge::Tail, change);
        let source = copier.clip(still);
        let end = doc_view::end_s(&source);

        let lane = copier.lane;
        let copy = copier.copy(&source, lane, end);

        let copy = copier.clip(copy);
        assert!(near(doc_view::start_s(&copy), end));
        assert_eq!(copy.len, source.len);
        assert_eq!(copier.clip(still), source);
    }
}

/// The shaping is a single edit: the batch is one transaction, so taking
/// it back leaves the file as it was first laid, past the lane's last
/// clip, and making it again brings the copy back.
#[test]
fn the_shaping_of_a_copy_is_one_undo_step() {
    let mut copier = Copier::new(MediaKind::Video, 10.0);
    let whole = copier.lay(0.0);
    let (_, (_, tail)) = copier.split(whole, 2.0);
    let lane = copier.lane;
    let lay = lay_copy(copier.doc(), &tail, lane).expect("a clip can be laid");
    let laid = copier.run(lay).expect("the file is laid");
    let before: Document<Model> = copier.bench.document.clone();
    let commands = shape_copy(copier.doc(), &tail, laid, lane, doc_view::tick(12.0));
    copier.bench.apply(&Command::Batch {
        commands: commands.clone(),
    });
    let made = copier.clip(laid);

    // One step back is the document before the batch.
    copier.bench.document = before;
    let unmade = copier.clip(laid);
    assert!(near(doc_view::start_s(&unmade), 10.0) && near(doc_view::in_point_s(&unmade), 0.0));
    assert!(near(doc_view::len_s(&unmade), 10.0));
    copier.bench.apply(&Command::Batch { commands });
    assert_eq!(copier.clip(laid), made);
}

/// The file of a copy is first laid where the lane it is going to ends:
/// past its last clip, or at the start of an empty lane.
#[test]
fn the_file_is_first_laid_where_the_lane_ends() {
    let mut copier = Copier::new(MediaKind::Video, 10.0);
    let whole = copier.lay(0.0);
    let (_, (_, tail)) = copier.split(whole, 3.0);
    let other = copier.other_lane();
    for (lane, end) in [(copier.lane, 10.0), (other, 0.0)] {
        match lay_copy(copier.doc(), &tail, lane) {
            Some(Command::AddClip {
                media,
                lane: laid_on,
                start,
                ripple,
            }) => {
                assert_eq!(Some(media), doc_view::media_id(&tail));
                assert_eq!(laid_on, lane);
                assert!(
                    near(doc_view::secs(start), end),
                    "laid at {start} for {end}"
                );
                assert!(!ripple);
            }
            other => panic!("laid by {other:?}"),
        }
    }
}

/// Duplicate copies every clip in one edit: one step takes all of them
/// away and leaves every source as it was, and the copies land after
/// their own ends, the latest first, nothing covering anything.
#[test]
fn a_duplicate_of_several_clips_is_one_undo_step() {
    let mut copier = Copier::new(MediaKind::Video, 10.0);
    let whole = copier.lay(0.0);
    let ((head_id, head), (tail_id, tail)) = copier.split(whole, 4.0);
    let other = copier.other_lane();
    let media = copier.media;
    let third = copier
        .run(Command::AddClip {
            media,
            lane: other,
            start: doc_view::tick(2.0),
            ripple: false,
        })
        .expect("a clip id");
    copier.trim(third, Edge::Head, 1.0);
    let third_clip = copier.clip(third);
    let before = copier.clips();
    let undo: Document<Model> = copier.bench.document.clone();

    let applied = copier.bench.apply(&Command::DuplicateClips {
        items: vec![tail_id, third, head_id],
    });
    let made = applied.selection.expect("the copies are selected");
    assert_eq!(made.len(), 3);
    let after = copier.clips();
    let copy_of = |source: &Item| -> Item {
        made.iter()
            .map(|id| copier.clip(*id))
            .find(|copy| copy.lane == source.lane && copy.offset == source.offset)
            .expect("a copy of it")
    };
    for source in [&head, &tail, &third_clip] {
        let copy = copy_of(source);
        assert_eq!(copy.lane, source.lane);
        assert_same_footage(&copy, source);
    }
    assert!(near(doc_view::start_s(&copy_of(&tail)), 10.0));
    assert!(near(doc_view::start_s(&copy_of(&third_clip)), 12.0));
    // Nothing on a lane covers anything else.
    for (index, (one_id, one)) in after.iter().enumerate() {
        for (two_id, two) in &after[index + 1..] {
            let apart = one.lane != two.lane
                || doc_view::end(one) <= two.start
                || doc_view::end(two) <= one.start;
            assert!(apart, "{one_id} covers {two_id}");
        }
    }

    copier.bench.document = undo;
    assert_eq!(copier.clips(), before);
}

/// A copy has nothing of a tie or a transition in, whichever way it is made.
#[test]
fn a_whole_copy_is_untied() {
    let mut copier = Copier::new(MediaKind::Video, 10.0);
    let whole = copier.lay(0.0);
    copier.run(Command::DetachAudio { item: whole });
    let sound = doc_view::items(copier.doc())
        .into_iter()
        .find(|(_, item)| item.link.is_some())
        .map(|(id, _)| id)
        .expect("a detached sound");
    let lane = copier.clip(sound).lane;
    let copy = copier.copy_whole(sound, lane, 20.0);
    let copy = copier.clip(copy);
    assert_eq!(copy.link, None);
    assert!(matches!(
        doc_view::time_map(&copy),
        Some(TimeMap::Linear { .. })
    ));
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The model's checks and its index against slower ways of finding the
//! same.
//!
//! Long random runs of edits - items placed, moved, trimmed, retimed and
//! taken away, sounds detached, sequences nested, lanes added, reordered
//! and taken away, files re-probed and removed, several at once - are
//! committed to a project. After every one:
//!
//! - the commit was accepted exactly when the same tables, read as a
//!   file is read with everything new, are accepted: checking what
//!   changed finds what checking everything finds;
//! - a refused commit left the document as it was;
//! - the index of every accepted document is the index built afresh from
//!   its rows, and what it says is at each tick is what a scan of the
//!   lane finds.

mod fixture;

use std::sync::Arc;

use concat_doc::model::{
    Content, Effect, EffectId, Item, ItemId, Items, Lane, LaneId, Lanes, Link, LinkKind, Mask,
    MaskId, MediaCut, MediaId, Model, Nest, OrderKey, Sequence, SequenceId, Speed, Take, TimeMap,
    Title,
};
use concat_doc::{
    Document, Draft, MAX_COUNTER, Mint, Origin, Rejected, TICKS_PER_SECOND, Tick, TickLen, Writer,
};
use fixture::*;
use proptest::prelude::*;
use proptest::sample::Index;

/// A quarter of a second.
const QUARTER: i64 = TICKS_PER_SECOND / 4;

fn quarters(n: i64) -> Tick {
    Tick::new(n * QUARTER)
}

fn quarter_len(n: u8) -> TickLen {
    TickLen::new(i64::from(n.max(1)) * QUARTER).unwrap()
}

#[derive(Clone, Debug)]
enum Op {
    Cut {
        lane: Index,
        start: u8,
        len: u8,
        media: Index,
        speed: Index,
    },
    Title {
        lane: Index,
        start: u8,
        len: u8,
    },
    Transition {
        lane: Index,
        start: u8,
        len: u8,
    },
    Nest {
        lane: Index,
        start: u8,
        len: u8,
        sequence: Index,
    },
    Detach {
        item: Index,
        lane: Index,
    },
    Move {
        item: Index,
        lane: Index,
        start: u8,
    },
    Trim {
        item: Index,
        offset: u8,
        len: u8,
    },
    Remove {
        item: Index,
    },
    AddLane {
        sequence: Index,
        at: Index,
    },
    Reorder {
        lane: Index,
        to: Index,
    },
    RemoveLane {
        lane: Index,
    },
    Reprobe {
        media: Index,
        len: u8,
    },
    RemoveMedia {
        media: Index,
    },
    AddSequence,
    RemoveSequence {
        sequence: Index,
    },
    /// Several of the others in one draft.
    Together(Vec<Op>),
}

fn simple() -> impl Strategy<Value = Op> {
    let place = || (any::<Index>(), 0..40_u8, 1..12_u8);
    prop_oneof![
        8 => (place(), any::<Index>(), any::<Index>()).prop_map(|((lane, start, len), media, speed)| {
            Op::Cut { lane, start, len, media, speed }
        }),
        2 => place().prop_map(|(lane, start, len)| Op::Title { lane, start, len }),
        3 => place().prop_map(|(lane, start, len)| Op::Transition { lane, start, len }),
        2 => (place(), any::<Index>()).prop_map(|((lane, start, len), sequence)| {
            Op::Nest { lane, start, len, sequence }
        }),
        2 => (any::<Index>(), any::<Index>()).prop_map(|(item, lane)| Op::Detach { item, lane }),
        6 => (any::<Index>(), any::<Index>(), 0..40_u8)
            .prop_map(|(item, lane, start)| Op::Move { item, lane, start }),
        4 => (any::<Index>(), 0..20_u8, 1..20_u8)
            .prop_map(|(item, offset, len)| Op::Trim { item, offset, len }),
        3 => any::<Index>().prop_map(|item| Op::Remove { item }),
        2 => (any::<Index>(), any::<Index>()).prop_map(|(sequence, at)| Op::AddLane { sequence, at }),
        2 => (any::<Index>(), any::<Index>()).prop_map(|(lane, to)| Op::Reorder { lane, to }),
        1 => any::<Index>().prop_map(|lane| Op::RemoveLane { lane }),
        1 => (any::<Index>(), 1..60_u8).prop_map(|(media, len)| Op::Reprobe { media, len }),
        1 => any::<Index>().prop_map(|media| Op::RemoveMedia { media }),
        1 => Just(Op::AddSequence),
        1 => any::<Index>().prop_map(|sequence| Op::RemoveSequence { sequence }),
    ]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => simple(),
        1 => prop::collection::vec(simple(), 2..4).prop_map(Op::Together),
    ]
}

fn pick<T: Copy>(from: &[T], at: &Index) -> Option<T> {
    (!from.is_empty()).then(|| from[at.index(from.len())])
}

fn lanes_of(draft: &Draft<Model>) -> Vec<LaneId> {
    draft.lanes.ids().collect()
}

fn items_of(draft: &Draft<Model>) -> Vec<ItemId> {
    draft.items.ids().collect()
}

fn media_of(draft: &Draft<Model>) -> Vec<MediaId> {
    draft.media.ids().collect()
}

fn sequences_of(draft: &Draft<Model>) -> Vec<SequenceId> {
    draft.sequences.ids().collect()
}

/// Makes the edit in the draft. Where it names something there is none
/// of, it does nothing.
fn run(draft: &mut Draft<Model>, op: &Op) {
    let speeds = [
        Speed::NORMAL,
        Speed::new(2, 1).unwrap(),
        Speed::new(1, 2).unwrap(),
        Speed::new(-1, 1).unwrap(),
        Speed::new(32, 1).unwrap(),
    ];
    match op {
        Op::Cut {
            lane,
            start,
            len,
            media,
            speed,
        } => {
            let (Some(lane), Some(media)) =
                (pick(&lanes_of(draft), lane), pick(&media_of(draft), media))
            else {
                return;
            };
            let speed = speeds[speed.index(speeds.len())];
            let src_in = if speed.is_reverse() {
                quarters(8)
            } else {
                Tick::ZERO
            };
            let mut item = Item::new(
                lane,
                quarters(i64::from(*start)),
                quarter_len(*len),
                Content::Media(MediaCut {
                    time: TimeMap::Linear { src_in, speed },
                    ..MediaCut::whole(media)
                }),
            );
            // Now and then with an effect on a mask of its own.
            if *start % 3 == 0 {
                let mask: MaskId = draft.mint().unwrap();
                let effect: EffectId = draft.mint().unwrap();
                let mut effect = Effect::new(effect, "concat.mono");
                effect.mask = Some(mask);
                item.masks
                    .push(concat_doc::model::Entry::Known(Mask::circle(mask)));
                item.effects.push(concat_doc::model::Entry::Known(effect));
            }
            let id: ItemId = draft.mint().unwrap();
            draft.items.insert(id, item).unwrap();
        }
        Op::Title { lane, start, len } => {
            let Some(lane) = pick(&lanes_of(draft), lane) else {
                return;
            };
            let item = Item::new(
                lane,
                quarters(i64::from(*start)),
                quarter_len(*len),
                Content::Title(Title::default()),
            );
            let id: ItemId = draft.mint().unwrap();
            draft.items.insert(id, item).unwrap();
        }
        Op::Transition { lane, start, len } => {
            let Some(lane) = pick(&lanes_of(draft), lane) else {
                return;
            };
            let package: EffectId = draft.mint().unwrap();
            let item = Item::new(
                lane,
                quarters(i64::from(*start)),
                quarter_len(*len),
                Content::Transition(Effect::new(package, "concat.dissolve")),
            );
            let id: ItemId = draft.mint().unwrap();
            draft.items.insert(id, item).unwrap();
        }
        Op::Nest {
            lane,
            start,
            len,
            sequence,
        } => {
            let (Some(lane), Some(sequence)) = (
                pick(&lanes_of(draft), lane),
                pick(&sequences_of(draft), sequence),
            ) else {
                return;
            };
            let item = Item::new(
                lane,
                quarters(i64::from(*start)),
                quarter_len(*len),
                Content::Nest(Nest {
                    sequence,
                    time: TimeMap::default(),
                }),
            );
            let id: ItemId = draft.mint().unwrap();
            draft.items.insert(id, item).unwrap();
        }
        Op::Detach { item, lane } => {
            let (Some(picture), Some(lane)) =
                (pick(&items_of(draft), item), pick(&lanes_of(draft), lane))
            else {
                return;
            };
            let of = Item::clone(draft.items.get(picture).unwrap());
            let Content::Media(cut) = &of.content else {
                return;
            };
            let mut sound = of.clone();
            sound.lane = lane;
            sound.effects.clear();
            sound.masks.clear();
            sound.content = Content::Media(MediaCut {
                take: Take::Sound,
                ..cut.clone()
            });
            sound.link = Some(Link {
                to: picture,
                kind: LinkKind::SoundOf,
            });
            let id: ItemId = draft.mint().unwrap();
            draft.items.insert(id, sound).unwrap();
        }
        Op::Move { item, lane, start } => {
            let (Some(item), Some(lane)) =
                (pick(&items_of(draft), item), pick(&lanes_of(draft), lane))
            else {
                return;
            };
            draft
                .items
                .update(item, |it| {
                    it.lane = lane;
                    it.start = quarters(i64::from(*start));
                })
                .unwrap();
        }
        Op::Trim { item, offset, len } => {
            let Some(item) = pick(&items_of(draft), item) else {
                return;
            };
            draft
                .items
                .update(item, |it| {
                    it.offset = quarters(i64::from(*offset));
                    it.len = quarter_len(*len);
                })
                .unwrap();
        }
        Op::Remove { item } => {
            if let Some(item) = pick(&items_of(draft), item) {
                draft.items.remove(item).unwrap();
            }
        }
        Op::AddLane { sequence, at } => {
            let Some(sequence) = pick(&sequences_of(draft), sequence) else {
                return;
            };
            let keys: Vec<OrderKey> = draft
                .lanes
                .index()
                .of(sequence)
                .map(|(_, key)| key.clone())
                .collect();
            let at = at.index(keys.len() + 1);
            let low = at.checked_sub(1).map(|below| &keys[below]);
            // In a draft two lanes may share a key, with no room between.
            let Some(order) = OrderKey::between(low, keys.get(at)) else {
                return;
            };
            let id: LaneId = draft.mint().unwrap();
            draft.lanes.insert(id, Lane::new(sequence, order)).unwrap();
        }
        Op::Reorder { lane, to } => {
            let Some(lane) = pick(&lanes_of(draft), lane) else {
                return;
            };
            // To a few fixed places, some of them taken.
            let places = ["1", "a", "i", "m", "t", "z"];
            let order = OrderKey::parse(places[to.index(places.len())]).unwrap();
            draft.lanes.update(lane, |l| l.order = order).unwrap();
        }
        Op::RemoveLane { lane } => {
            if let Some(lane) = pick(&lanes_of(draft), lane) {
                draft.lanes.remove(lane).unwrap();
            }
        }
        Op::Reprobe { media, len } => {
            if let Some(media) = pick(&media_of(draft), media) {
                let length = quarter_len(*len);
                draft
                    .media
                    .update(media, |file| {
                        if file.length.is_some() {
                            file.length = Some(length);
                        }
                    })
                    .unwrap();
            }
        }
        Op::RemoveMedia { media } => {
            if let Some(media) = pick(&media_of(draft), media) {
                draft.media.remove(media).unwrap();
            }
        }
        Op::AddSequence => {
            let id: SequenceId = draft.mint().unwrap();
            draft
                .sequences
                .insert(id, Sequence::new("Another", concat_doc::model::Frame::hd()))
                .unwrap();
            draft.view.tabs.push(id);
            let lane: LaneId = draft.mint().unwrap();
            draft
                .lanes
                .insert(lane, Lane::new(id, OrderKey::middle()))
                .unwrap();
        }
        Op::RemoveSequence { sequence } => {
            if let Some(sequence) = pick(&sequences_of(draft), sequence) {
                draft.sequences.remove(sequence).unwrap();
                draft.view.tabs.retain(|tab| *tab != sequence);
                if draft.view.showing == Some(sequence) {
                    let next = draft.sequences.ids().next();
                    draft.view.showing = next;
                }
            }
        }
        Op::Together(ops) => {
            for op in ops {
                run(draft, op);
            }
        }
    }
}

/// The index of a document against one built afresh from its rows, and
/// against a scan.
fn index_holds(doc: &Document<Model>) -> Result<(), TestCaseError> {
    let fresh: Items = doc
        .items
        .rows()
        .iter()
        .map(|(id, item)| (id, Arc::clone(item)))
        .collect();
    let ours = doc.items.index();
    let theirs = fresh.index();
    for lane in doc.lanes.ids() {
        let on: Vec<_> = ours.on_lane(lane).collect();
        prop_assert_eq!(&on, &theirs.on_lane(lane).collect::<Vec<_>>());
        let mut scanned: Vec<_> = doc
            .items
            .iter()
            .filter(|(_, item)| item.lane == lane)
            .map(|(id, item)| (id, item.start, item.end().unwrap()))
            .collect();
        scanned.sort_by_key(|(id, start, _)| (*start, *id));
        prop_assert_eq!(&on, &scanned);
        prop_assert_eq!(
            ours.transitions_on(lane).collect::<Vec<_>>(),
            theirs.transitions_on(lane).collect::<Vec<_>>()
        );
        for q in -2..60 {
            let t = quarters(q) + concat_doc::TickDur::new(q.rem_euclid(3));
            let hit = scanned
                .iter()
                .find(|(_, start, end)| *start <= t && t < *end)
                .map(|(id, _, _)| *id);
            prop_assert_eq!(ours.at(lane, t), hit);
        }
    }
    for media in doc.media.ids() {
        prop_assert_eq!(
            ours.using_media(media).collect::<Vec<_>>(),
            theirs.using_media(media).collect::<Vec<_>>()
        );
    }
    for sequence in doc.sequences.ids() {
        prop_assert_eq!(
            ours.nesting(sequence).collect::<Vec<_>>(),
            theirs.nesting(sequence).collect::<Vec<_>>()
        );
        let fresh_lanes: Lanes = doc
            .lanes
            .rows()
            .iter()
            .map(|(id, lane)| (id, Arc::clone(lane)))
            .collect();
        prop_assert_eq!(
            doc.lanes
                .index()
                .of(sequence)
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            fresh_lanes
                .index()
                .of(sequence)
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        );
    }
    for (id, item) in doc.items.iter() {
        prop_assert_eq!(
            ours.tied_to(id).collect::<Vec<_>>(),
            theirs.tied_to(id).collect::<Vec<_>>()
        );
        for nested in item.nested_ids() {
            prop_assert_eq!(ours.holding(nested).collect::<Vec<_>>(), vec![id]);
        }
    }
    Ok(())
}

/// The fixture with a second sequence of one lane, and a clip on it.
fn start() -> Document<Model> {
    let fx = fixture();
    let mut doc = fx.doc.clone();
    doc = apply(&doc, |draft| {
        let second: SequenceId = draft.mint().unwrap();
        draft
            .sequences
            .insert(second, Sequence::new("Two", concat_doc::model::Frame::hd()))
            .unwrap();
        draft.view.tabs.push(second);
        let lane: LaneId = draft.mint().unwrap();
        draft
            .lanes
            .insert(lane, Lane::new(second, OrderKey::middle()))
            .unwrap();
        let clip: ItemId = draft.mint().unwrap();
        draft
            .items
            .insert(clip, cut(lane, 0.0, 2.0, fx.video))
            .unwrap();
    });
    doc
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn checking_what_changed_finds_what_checking_everything_finds(
        ops in prop::collection::vec(op(), 1..48),
    ) {
        let mut doc = start();
        for op in &ops {
            let mut draft = doc.begin("Random", Origin(0));
            run(&mut draft, op);
            let tables = draft.tables().clone();
            // A mint that has made every id: the rules are the question
            // here, not where the ids came from.
            let mint = Mint::resume([(Writer::LOCAL, MAX_COUNTER)]).unwrap();
            let whole = Document::load(tables, mint, doc.revision());
            match draft.commit() {
                Ok(commit) => {
                    prop_assert!(
                        whole.is_ok(),
                        "accepted, but the whole reads as {:?} after {:?}",
                        whole.err(),
                        op
                    );
                    doc = commit.into_document();
                    index_holds(&doc)?;
                }
                Err(Rejected::Invalid(broken)) => {
                    prop_assert!(
                        whole.is_err(),
                        "refused with {:?}, but the whole reads as valid after {:?}",
                        broken,
                        op
                    );
                }
                Err(Rejected::Unminted(ids)) => {
                    prop_assert!(false, "ids never made: {:?}", ids);
                }
            }
        }
    }

    #[test]
    fn the_index_is_its_rows_after_any_run_of_edits(
        ops in prop::collection::vec(op(), 1..48),
    ) {
        // The same edits, kept whether or not they keep the rules: the
        // index follows a draft's every edit, not only a document's.
        let doc = start();
        let mut draft = doc.begin("Random", Origin(0));
        for op in &ops {
            run(&mut draft, op);
        }
        let fresh: Items = draft
            .items
            .rows()
            .iter()
            .map(|(id, item)| (id, Arc::clone(item)))
            .collect();
        for lane in draft.lanes.ids() {
            prop_assert_eq!(
                draft.items.index().on_lane(lane).collect::<Vec<_>>(),
                fresh.index().on_lane(lane).collect::<Vec<_>>()
            );
        }
        for media in draft.media.ids() {
            prop_assert_eq!(
                draft.items.index().using_media(media).collect::<Vec<_>>(),
                fresh.index().using_media(media).collect::<Vec<_>>()
            );
        }
    }
}

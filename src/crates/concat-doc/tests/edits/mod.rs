// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Random runs of edits to a project, for the tests that hold the model
//! and its file to slower ways of finding the same.
//!
//! Items placed of every kind, copied, moved, trimmed, retimed, dressed
//! with keys, crops, masks, markers and the rest, and taken away; sounds
//! detached, sequences nested, lanes added, reordered, made caption lanes
//! and taken away, files re-probed and removed, fonts added, the cover
//! set, several at once. An edit that names something there is none of
//! does nothing, and one that breaks a rule is refused at its commit,
//! as it would be in the window.

#![allow(dead_code)]

use concat_doc::model::{
    Animated, Background, Blend, BrushTool, CaptionLook, Color, Content, Cover, Crop, CutIn,
    Cutout, CutoutMode, Ease, Effect, EffectId, Entry, Font, Gradient, Item, ItemId, Key, Lane,
    LaneId, Link, LinkKind, Loop, Marker, Mask, MaskId, MediaCut, MediaId, Model, Move, Nest,
    OrderKey, Residual, Sequence, SequenceId, ShapeKind, ShapeShadow, ShapeStyle, Speed,
    SpeedGraph, SpeedPoint, Stabilize, Stroke, Subject, Take, TextRun, TimeCurve, TimeMap,
    TimePoint, Title, VoiceKind, VoiceMix,
};
use concat_doc::{Document, Draft, TICKS_PER_SECOND, Tick, TickDur, TickLen};
use proptest::prelude::*;

use super::fixture::*;

/// A quarter of a second.
/// Which of the things there are: the `n`th, counting round.
#[derive(Clone, Copy, Debug)]
pub struct At(pub usize);

impl At {
    /// The place in a list of `size`, which is above zero.
    pub fn index(self, size: usize) -> usize {
        self.0 % size
    }
}

/// Any of the things there are.
pub fn any_at() -> impl Strategy<Value = At> {
    any::<usize>().prop_map(At)
}

pub const QUARTER: i64 = TICKS_PER_SECOND / 4;

pub fn quarters(n: i64) -> Tick {
    Tick::new(n * QUARTER)
}

pub fn quarter_len(n: u8) -> TickLen {
    TickLen::new(i64::from(n.max(1)) * QUARTER).unwrap()
}

#[derive(Clone, Debug)]
pub enum Op {
    Cut {
        lane: At,
        start: u8,
        len: u8,
        media: At,
        speed: At,
    },
    Title {
        lane: At,
        start: u8,
        len: u8,
    },
    Transition {
        lane: At,
        start: u8,
        len: u8,
    },
    Nest {
        lane: At,
        start: u8,
        len: u8,
        sequence: At,
    },
    Detach {
        item: At,
        lane: At,
    },
    /// A copy laid right after the item on its lane: with ids of its
    /// own, or now and then a plain clone, refused when it holds any.
    Duplicate {
        item: At,
        fresh: bool,
    },
    Move {
        item: At,
        lane: At,
        start: u8,
    },
    Trim {
        item: At,
        offset: u8,
        len: u8,
    },
    Remove {
        item: At,
    },
    AddLane {
        sequence: At,
        at: At,
    },
    Reorder {
        lane: At,
        to: At,
    },
    RemoveLane {
        lane: At,
    },
    Reprobe {
        media: At,
        len: u8,
    },
    RemoveMedia {
        media: At,
    },
    AddSequence,
    /// A lane made a caption lane, or a plain one again; now and then
    /// with a look that has words, which is refused.
    Captions {
        lane: At,
        look: u8,
    },
    RemoveSequence {
        sequence: At,
    },
    /// The cover set to a frame, now and then before the start, which is
    /// refused; or to none.
    Cover {
        into: Option<i8>,
    },
    /// The cover set to a file of the bin: a still stands, anything else
    /// is refused.
    CoverPicture {
        media: At,
    },
    /// A shape of one of a few looks, or a layer over the picture or the
    /// sound with a chain of its own.
    Drawn {
        lane: At,
        start: u8,
        len: u8,
        kind: u8,
    },
    /// An item given one more thing of its own: keys, a crop, a mix, a
    /// marker, a transition on its cut, a cutout, a move, runs of words,
    /// keys on an effect's knob, a time map of keys.
    Dress {
        item: At,
        how: u8,
    },
    /// A font added; now and then one already there, which is refused.
    Font {
        n: u8,
    },
    /// A marker on a sequence.
    SequenceMarker {
        sequence: At,
        at: u8,
    },
    /// Several of the others in one draft.
    Together(Vec<Op>),
}

pub fn simple() -> impl Strategy<Value = Op> {
    let place = || (any_at(), 0..40_u8, 1..12_u8);
    prop_oneof![
        8 => (place(), any_at(), any_at()).prop_map(|((lane, start, len), media, speed)| {
            Op::Cut { lane, start, len, media, speed }
        }),
        2 => place().prop_map(|(lane, start, len)| Op::Title { lane, start, len }),
        3 => place().prop_map(|(lane, start, len)| Op::Transition { lane, start, len }),
        2 => (place(), any_at()).prop_map(|((lane, start, len), sequence)| {
            Op::Nest { lane, start, len, sequence }
        }),
        2 => (any_at(), any_at()).prop_map(|(item, lane)| Op::Detach { item, lane }),
        2 => (any_at(), prop::bool::weighted(0.8))
            .prop_map(|(item, fresh)| Op::Duplicate { item, fresh }),
        6 => (any_at(), any_at(), 0..40_u8)
            .prop_map(|(item, lane, start)| Op::Move { item, lane, start }),
        4 => (any_at(), 0..20_u8, 1..20_u8)
            .prop_map(|(item, offset, len)| Op::Trim { item, offset, len }),
        3 => any_at().prop_map(|item| Op::Remove { item }),
        2 => (any_at(), any_at()).prop_map(|(sequence, at)| Op::AddLane { sequence, at }),
        2 => (any_at(), any_at()).prop_map(|(lane, to)| Op::Reorder { lane, to }),
        1 => any_at().prop_map(|lane| Op::RemoveLane { lane }),
        1 => (any_at(), 1..60_u8).prop_map(|(media, len)| Op::Reprobe { media, len }),
        1 => any_at().prop_map(|media| Op::RemoveMedia { media }),
        1 => Just(Op::AddSequence),
        1 => (any_at(), 0..4_u8).prop_map(|(lane, look)| Op::Captions { lane, look }),
        1 => any_at().prop_map(|sequence| Op::RemoveSequence { sequence }),
        1 => prop::option::of(-4..40_i8).prop_map(|into| Op::Cover { into }),
        1 => any_at().prop_map(|media| Op::CoverPicture { media }),
        2 => (place(), 0..4_u8).prop_map(|((lane, start, len), kind)| {
            Op::Drawn { lane, start, len, kind }
        }),
        6 => (any_at(), 0..10_u8).prop_map(|(item, how)| Op::Dress { item, how }),
        1 => (0..6_u8).prop_map(|n| Op::Font { n }),
        1 => (any_at(), 0..40_u8).prop_map(|(sequence, at)| Op::SequenceMarker { sequence, at }),
    ]
}

pub fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => simple(),
        1 => prop::collection::vec(simple(), 2..4).prop_map(Op::Together),
    ]
}

pub fn pick<T: Copy>(from: &[T], at: &At) -> Option<T> {
    (!from.is_empty()).then(|| from[at.index(from.len())])
}

pub fn lanes_of(draft: &Draft<Model>) -> Vec<LaneId> {
    draft.lanes.ids().collect()
}

pub fn items_of(draft: &Draft<Model>) -> Vec<ItemId> {
    draft.items.ids().collect()
}

pub fn media_of(draft: &Draft<Model>) -> Vec<MediaId> {
    draft.media.ids().collect()
}

pub fn sequences_of(draft: &Draft<Model>) -> Vec<SequenceId> {
    draft.sequences.ids().collect()
}

/// Makes the edit in the draft. Where it names something there is none
/// of, it does nothing.
pub fn run(draft: &mut Draft<Model>, op: &Op) {
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
        Op::Duplicate { item, fresh } => {
            let Some(item) = pick(&items_of(draft), item) else {
                return;
            };
            let of = Item::clone(draft.items.get(item).unwrap());
            let mut copy = if *fresh {
                of.with_new_ids(draft).unwrap()
            } else {
                of.clone()
            };
            let Some(end) = of.end() else {
                return;
            };
            copy.start = end;
            let id: ItemId = draft.mint().unwrap();
            draft.items.insert(id, copy).unwrap();
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
        Op::Captions { lane, look } => {
            if let Some(lane) = pick(&lanes_of(draft), lane) {
                let look = match look {
                    0 => None,
                    1 => Some(CaptionLook {
                        title: Title::default(),
                        offset_y: 0.0,
                    }),
                    _ => Some(CaptionLook::default()),
                };
                draft.lanes.update(lane, |l| l.captions = look).unwrap();
            }
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
        Op::Cover { into } => {
            draft.cover = into.map(|n| Cover::Frame {
                into: TickDur::new(i64::from(n) * QUARTER),
            });
        }
        Op::CoverPicture { media } => {
            if let Some(media) = pick(&media_of(draft), media) {
                draft.cover = Some(Cover::Picture { media });
            }
        }
        Op::Drawn {
            lane,
            start,
            len,
            kind,
        } => {
            let Some(lane) = pick(&lanes_of(draft), lane) else {
                return;
            };
            let mut item = Item::new(
                lane,
                quarters(i64::from(*start)),
                quarter_len(*len),
                Content::Layer,
            );
            match kind {
                0 => item.content = Content::Shape(ShapeStyle::default()),
                1 => {
                    item.content = Content::Shape(ShapeStyle {
                        kind: ShapeKind::Star,
                        points: 7,
                        fill: None,
                        gradient: Some(Gradient {
                            color: Color::rgb(255, 0, 80),
                            angle: 45.0,
                            radial: true,
                        }),
                        stroke: Some(Color {
                            r: 1,
                            g: 2,
                            b: 3,
                            a: 128,
                        }),
                        shadow: Some(ShapeShadow {
                            color: Color::BLACK,
                            angle: 90.0,
                            distance: 0.01,
                            blur: 0.02,
                        }),
                        ..ShapeStyle::default()
                    })
                }
                2 => {
                    let effect = Effect::new(draft.mint().unwrap(), "concat.blur");
                    item.effects.push(Entry::Known(effect));
                    item.picture.opacity = Animated::fixed(0.75);
                }
                _ => {
                    item.content = Content::SoundLayer;
                    let filter = Effect::new(draft.mint().unwrap(), "concat.echo");
                    item.filters.push(Entry::Known(filter));
                }
            }
            let id: ItemId = draft.mint().unwrap();
            draft.items.insert(id, item).unwrap();
        }
        Op::Dress { item, how } => {
            let Some(id) = pick(&items_of(draft), item) else {
                return;
            };
            let mut item = Item::clone(draft.items.get(id).unwrap());
            dress(draft, &mut item, *how);
            draft.items.replace(id, item).unwrap();
        }
        Op::Font { n } => {
            draft.fonts.push(Font {
                family: format!("Family {}", n % 4),
                path: format!("/fonts/{n}.ttf"),
            });
        }
        Op::SequenceMarker { sequence, at } => {
            let Some(sequence) = pick(&sequences_of(draft), sequence) else {
                return;
            };
            let marker = Marker {
                id: draft.mint().unwrap(),
                at: quarters(i64::from(*at)),
                len: TickDur::new(QUARTER),
                name: "Chorus".into(),
                color: Some(Color::rgb(0, 200, 100)),
                residual: Residual::default(),
            };
            draft
                .sequences
                .update(sequence, |s| s.markers.push(Entry::Known(marker)))
                .unwrap();
        }
        Op::Together(ops) => {
            for op in ops {
                run(draft, op);
            }
        }
    }
}

/// A project holding one of everything the model has: every kind of
/// item, each dressed every way it can be, a detached sound, a nest, a
/// caption lane, fonts, markers and a cover. Each edit is committed on
/// its own, and one the rules refuse leaves the project as it was.
pub fn everything() -> Document<Model> {
    let mut doc = start();
    let commit = |doc: &mut Document<Model>, op: Op| {
        let mut draft = doc.begin("Everything", concat_doc::Origin(0));
        run(&mut draft, &op);
        if let Ok(done) = draft.commit() {
            *doc = done.into_document();
        }
    };
    let at = At;
    let lane = |n: usize| at(n);
    // One of each, on lanes and places of their own.
    commit(
        &mut doc,
        Op::AddLane {
            sequence: at(0),
            at: at(9),
        },
    );
    commit(
        &mut doc,
        Op::AddLane {
            sequence: at(0),
            at: at(9),
        },
    );
    commit(
        &mut doc,
        Op::AddLane {
            sequence: at(0),
            at: at(9),
        },
    );
    for (n, how) in (0..10).enumerate() {
        commit(
            &mut doc,
            Op::Cut {
                lane: lane(0),
                start: (n * 3) as u8,
                len: 2,
                media: at(0),
                speed: at(0),
            },
        );
        let items = doc.items.ids().count();
        commit(
            &mut doc,
            Op::Dress {
                item: at(items - 1),
                how,
            },
        );
    }
    commit(
        &mut doc,
        Op::Title {
            lane: lane(1),
            start: 0,
            len: 4,
        },
    );
    let title = doc
        .items
        .iter()
        .position(|(_, item)| matches!(item.content, Content::Title(_)))
        .unwrap_or(0);
    commit(
        &mut doc,
        Op::Dress {
            item: at(title),
            how: 7,
        },
    );
    commit(
        &mut doc,
        Op::Dress {
            item: at(title),
            how: 2,
        },
    );
    for kind in 0..4 {
        commit(
            &mut doc,
            Op::Drawn {
                lane: lane(3),
                start: 6 * kind,
                len: 5,
                kind,
            },
        );
    }
    let layer = doc
        .items
        .iter()
        .position(|(_, item)| matches!(item.content, Content::Layer))
        .unwrap_or(0);
    commit(
        &mut doc,
        Op::Dress {
            item: at(layer),
            how: 8,
        },
    );
    commit(
        &mut doc,
        Op::Transition {
            lane: lane(5),
            start: 1,
            len: 1,
        },
    );
    commit(
        &mut doc,
        Op::Detach {
            item: at(1),
            lane: lane(4),
        },
    );
    commit(
        &mut doc,
        Op::Nest {
            lane: lane(1),
            start: 30,
            len: 3,
            sequence: at(1),
        },
    );
    commit(
        &mut doc,
        Op::Captions {
            lane: lane(5),
            look: 2,
        },
    );
    commit(&mut doc, Op::Font { n: 1 });
    commit(&mut doc, Op::Font { n: 2 });
    commit(
        &mut doc,
        Op::SequenceMarker {
            sequence: at(0),
            at: 3,
        },
    );
    commit(&mut doc, Op::CoverPicture { media: at(1) });
    doc
}

/// Gives an item one more thing of its own.
fn dress(draft: &mut Draft<Model>, item: &mut Item, how: u8) {
    let from = item.offset;
    let later = item.offset.checked_add(item.len).unwrap_or(from);
    match how {
        0 => {
            item.picture.opacity = Animated {
                value: 1.0,
                keys: vec![
                    Key::linear(from, 0.0),
                    Key {
                        at: later,
                        value: 1.0,
                        ease: Ease::Bezier {
                            x1: 0.42,
                            y1: 0.0,
                            x2: 0.58,
                            y2: 1.0,
                        },
                    },
                ],
            };
            item.picture.scale = Animated::fixed(1.5);
            item.picture.rotation = Animated {
                value: -12.5,
                keys: vec![Key::linear(from, 3.0)],
            };
        }
        1 => {
            item.picture.crop = Some(Crop {
                left: 0.1,
                top: 0.0,
                right: 0.125,
                bottom: 0.0,
            });
            item.picture.flip_h = true;
            item.picture.blend = Blend::Screen;
            item.picture.background = Some(Background::Blur { amount: 0.5 });
            item.picture.offset_x = Animated::fixed(-0.25);
        }
        2 => {
            item.name = "Shot \"one\"\n/é".into();
            item.sound.volume = Animated {
                value: 0.8,
                keys: vec![Key::linear(from, 0.2), Key::linear(later, 1.25)],
            };
            item.sound.fade_in = TickDur::new(QUARTER);
            item.sound.fade_out = TickDur::new(QUARTER / 2);
            item.sound.muted = true;
            item.sound.keep_pitch = false;
            item.sound.voice = Some(VoiceMix {
                kind: VoiceKind::Speech,
                voice: 1.0,
                background: 0.5,
            });
        }
        3 => item.markers.push(Entry::Known(Marker {
            id: draft.mint().unwrap(),
            at: from,
            len: TickDur::ZERO,
            name: "Beat".into(),
            color: None,
            residual: Residual::default(),
        })),
        4 => {
            let mut effect = Effect::new(draft.mint().unwrap(), "concat.dissolve");
            effect
                .params
                .insert("softness".into(), Animated::fixed(Some(0.5)));
            item.cut_in = Some(CutIn {
                effect,
                len: TickLen::new(QUARTER).unwrap(),
            });
        }
        5 => {
            item.picture.cutout = Some(Cutout {
                mode: CutoutMode::Custom,
                subject: Subject::Person,
                feather: 0.1,
                strokes: vec![Stroke {
                    tool: BrushTool::SmartEraser,
                    size: 0.05,
                    points: vec![[0.1, 0.2], [0.3, 0.4]],
                    at: Some(from),
                }],
            });
            item.picture.stabilize = Some(Stabilize {
                smoothness: 0.5,
                max_zoom: 1.2,
                keep_rotation: true,
                tripod: false,
            });
        }
        6 => {
            item.picture.motion.enter = Some(Move {
                name: "fade".into(),
                len: TickLen::new(QUARTER).unwrap(),
            });
            item.picture.motion.repeat = Some(Loop {
                name: "pulse".into(),
                lap: TickLen::new(QUARTER * 2).unwrap(),
                amount: 1.0,
            });
        }
        7 => {
            if let Content::Title(title) = &mut item.content {
                title.runs = vec![TextRun {
                    start: 0,
                    end: 4,
                    color: Some(Color::rgb(255, 0, 0)),
                    font_weight: Some(400.0),
                    ..TextRun::default()
                }];
                title.plate = Some(Color::BLACK);
                title.align = concat_doc::model::TextAlign::Left;
            }
        }
        8 => {
            let knob = Animated {
                value: None,
                keys: vec![Key::linear(from, 0.2), Key::linear(later, 0.8)],
            };
            if let Some(effect) = item.effects.iter_mut().find_map(Entry::known_mut) {
                effect.params.insert("amount".into(), knob);
                effect.enabled = false;
            }
        }
        _ => {
            // Played at its own pace through a time map of keys, slowing
            // to a stop's half at the end of the window, with the graph
            // those keys are drawn as.
            if let Content::Media(cut) = &mut item.content
                && let Some(source) = cut.time.source_at(from)
            {
                let run = later.checked_since(from).unwrap_or(TickDur::ZERO);
                cut.time = TimeMap::Keyed {
                    first: TimePoint { at: from, source },
                    rest: vec![
                        (
                            TimeCurve::Linear,
                            TimePoint {
                                at: from
                                    .checked_add(TickDur::new(run.get() / 2))
                                    .unwrap_or(from),
                                source: source
                                    .checked_add(TickDur::new(run.get() / 2))
                                    .unwrap_or(source),
                            },
                        ),
                        (
                            TimeCurve::Smooth {
                                leave: Speed::NORMAL,
                                arrive: Speed::new(1, 2).unwrap(),
                            },
                            TimePoint {
                                at: later,
                                source: source
                                    .checked_add(TickDur::new(run.get() / 2 + run.get() / 4))
                                    .unwrap_or(source),
                            },
                        ),
                    ],
                    drawn: Some(SpeedGraph {
                        from,
                        len: item.len,
                        points: [(0.0, 1.0), (0.5, 1.0), (1.0, 0.5)]
                            .into_iter()
                            .map(|(at, speed)| SpeedPoint {
                                at,
                                speed,
                                slope: None,
                            })
                            .collect(),
                    }),
                };
            }
        }
    }
}

/// The fixture with a second sequence of one lane, and a clip on it.
pub fn start() -> Document<Model> {
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

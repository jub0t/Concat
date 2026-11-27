// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Long random runs of commands, as a person makes them in the window.
//!
//! After every one the document keeps every rule, read whole as a file is
//! read; one that was refused left the document as it was. At the end,
//! undo all the way gives back the empty document, and redo all the way
//! the last one.

use concat_doc::commands::{
    ClipMove, Command, CommandError, Cue, Edge, KeyProperty, run, speed_from_decimal, speed_graph,
};
use concat_doc::model::{
    CaptionLook, Content, Ease, ItemId, LaneId, Media, MediaId, MediaKind, Model, Rule, SequenceId,
    Size, SpeedPoint, TimeMap,
};
use concat_doc::{
    Budget, Document, Entry, History, Origin, TICKS_PER_SECOND, Tick, TickDur, TickLen,
};
use proptest::prelude::*;
use proptest::sample::Index;

type Hist = History<Model, Command, ()>;

const QUARTER: i64 = TICKS_PER_SECOND / 4;

fn q(n: u8) -> Tick {
    Tick::new(i64::from(n) * QUARTER)
}

fn qlen(n: u8) -> TickLen {
    TickLen::new(i64::from(n.max(1)) * QUARTER).unwrap()
}

#[derive(Clone, Debug)]
enum Op {
    Media(u8, bool),
    Clip(Index, u8),
    Drop(Index, Index, u8, bool),
    Title(u8, u8, bool),
    Move(Index, u8, Index),
    Trim(Index, bool, i8, bool),
    Split(Index, u8),
    Merge(Index),
    Duplicate(Index),
    Remove(Index, bool),
    Speed(Index, u8, bool),
    Curve(Index, u8, u8, bool),
    Key(Index, u8),
    Detach(Index),
    Reattach(Index),
    Freeze(Index, u8),
    Captions(u8, u8),
    Transition(u8, u8),
    Lane(bool),
    RemoveLane(Index),
    Timeline,
    Select(Index),
    RemoveTimeline(Index),
    Undo,
}

fn op() -> impl Strategy<Value = Op> {
    let i = any::<Index>;
    prop_oneof![
        3 => (1..20u8, any::<bool>()).prop_map(|(n, s)| Op::Media(n, s)),
        6 => (i(), 0..40u8).prop_map(|(a, b)| Op::Clip(a, b)),
        3 => (i(), i(), 0..40u8, any::<bool>()).prop_map(|(a, b, c, d)| Op::Drop(a, b, c, d)),
        2 => (0..40u8, 1..20u8, any::<bool>()).prop_map(|(a, b, c)| Op::Title(a, b, c)),
        4 => (i(), 0..40u8, i()).prop_map(|(a, b, c)| Op::Move(a, b, c)),
        4 => (i(), any::<bool>(), any::<i8>(), any::<bool>()).prop_map(|(a, b, c, d)| Op::Trim(a, b, c, d)),
        4 => (i(), 0..60u8).prop_map(|(a, b)| Op::Split(a, b)),
        2 => i().prop_map(Op::Merge),
        2 => i().prop_map(Op::Duplicate),
        2 => (i(), any::<bool>()).prop_map(|(a, b)| Op::Remove(a, b)),
        3 => (i(), 1..64u8, any::<bool>()).prop_map(|(a, b, c)| Op::Speed(a, b, c)),
        2 => (i(), 1..64u8, 1..64u8, any::<bool>()).prop_map(|(a, b, c, d)| Op::Curve(a, b, c, d)),
        2 => (i(), 0..8u8).prop_map(|(a, b)| Op::Key(a, b)),
        2 => i().prop_map(Op::Detach),
        1 => i().prop_map(Op::Reattach),
        1 => (i(), 0..40u8).prop_map(|(a, b)| Op::Freeze(a, b)),
        1 => (0..40u8, 1..8u8).prop_map(|(a, b)| Op::Captions(a, b)),
        1 => (0..40u8, 1..8u8).prop_map(|(a, b)| Op::Transition(a, b)),
        1 => any::<bool>().prop_map(Op::Lane),
        1 => i().prop_map(Op::RemoveLane),
        1 => Just(Op::Timeline),
        1 => i().prop_map(Op::Select),
        1 => i().prop_map(Op::RemoveTimeline),
        1 => Just(Op::Undo),
    ]
}

fn pick<T: Copy>(from: &[T], at: &Index) -> Option<T> {
    (!from.is_empty()).then(|| from[at.index(from.len())])
}

fn items(m: &Model) -> Vec<ItemId> {
    m.items.ids().collect()
}

fn media(m: &Model) -> Vec<MediaId> {
    m.media.ids().collect()
}

fn lanes(m: &Model) -> Vec<LaneId> {
    m.lanes.ids().collect()
}

fn sequences(m: &Model) -> Vec<SequenceId> {
    m.sequences.ids().collect()
}

fn command(m: &Model, op: &Op) -> Option<Command> {
    Some(match op {
        Op::Media(n, sound) => Command::AddMedia {
            media: Media {
                path: format!("file{n}"),
                name: format!("file{n}"),
                kind: if n % 5 == 0 {
                    MediaKind::Image
                } else {
                    MediaKind::Video
                },
                length: Some(qlen(*n * 2)),
                size: Size::new(640, 360),
                has_sound: *sound,
                ..Media::default()
            },
        },
        Op::Clip(file, at) => Command::AddClipAtFirstFree {
            media: pick(&media(m), file)?,
            start: q(*at),
        },
        Op::Drop(file, lane, at, ripple) => Command::AddClip {
            media: pick(&media(m), file)?,
            lane: pick(&lanes(m), lane)?,
            start: q(*at),
            ripple: *ripple,
        },
        Op::Title(at, n, above) => Command::AddTextClip {
            start: q(*at),
            len: Some(qlen(*n)),
            lane: None,
            above: *above,
            offset_y: None,
            title: None,
        },
        Op::Move(item, at, lane) => Command::MoveClips {
            moves: vec![ClipMove {
                item: pick(&items(m), item)?,
                start: q(*at),
                lane: pick(&lanes(m), lane),
            }],
        },
        Op::Trim(item, head, by, ripple) => Command::TrimClip {
            item: pick(&items(m), item)?,
            edge: if *head { Edge::Head } else { Edge::Tail },
            delta: TickDur::new(i64::from(*by) * QUARTER / 8),
            ripple: *ripple,
        },
        Op::Split(item, at) => Command::SplitClips {
            items: vec![pick(&items(m), item)?],
            at: Tick::new(i64::from(*at) * QUARTER / 2),
        },
        Op::Merge(item) => {
            let id = pick(&items(m), item)?;
            let it = m.items.get(id)?;
            let end = it.start + it.len;
            let next = m.items.index().at(it.lane, end)?;
            Command::MergeClips {
                items: vec![id, next],
            }
        }
        Op::Duplicate(item) => Command::DuplicateClips {
            items: vec![pick(&items(m), item)?],
        },
        Op::Remove(item, ripple) => Command::RemoveClips {
            items: vec![pick(&items(m), item)?],
            ripple: *ripple,
        },
        Op::Speed(item, s, ripple) => Command::SetClipSpeed {
            item: pick(&items(m), item)?,
            speed: speed_from_decimal(f64::from(*s) / 8.0)?,
            ripple: *ripple,
        },
        Op::Curve(item, a, b, ripple) => {
            // Straight between its points, or eased through three.
            let slope = (a % 2 == 0).then_some(f64::from(*b) - 30.0);
            let mut curve = vec![
                SpeedPoint {
                    at: 0.0,
                    speed: f64::from(*a) / 8.0,
                    slope,
                },
                SpeedPoint {
                    at: 1.0,
                    speed: f64::from(*b) / 8.0,
                    slope,
                },
            ];
            if slope.is_some() {
                curve.insert(
                    1,
                    SpeedPoint {
                        at: f64::from(*b) / 64.0,
                        speed: f64::from(*a) / 4.0,
                        slope: Some(0.0),
                    },
                );
            }
            Command::SetClipSpeedCurve {
                item: pick(&items(m), item)?,
                curve: Some(curve),
                ripple: *ripple,
            }
        }
        Op::Key(item, at) => Command::SetClipKey {
            item: pick(&items(m), item)?,
            property: KeyProperty::Opacity,
            at: q(*at),
            value: f64::from(*at) / 8.0,
            ease: Ease::LINEAR,
        },
        Op::Detach(item) => Command::DetachAudio {
            item: pick(&items(m), item)?,
        },
        Op::Reattach(item) => Command::ReattachAudio {
            item: pick(&items(m), item)?,
        },
        Op::Freeze(item, at) => Command::FreezeFrame {
            item: pick(&items(m), item)?,
            at: q(*at),
            len: None,
            still: Some(Media {
                path: format!("still{at}"),
                kind: MediaKind::Image,
                ..Media::default()
            }),
        },
        Op::Captions(at, n) => Command::PlaceCaptions {
            cues: (0..*n)
                .map(|k| Cue {
                    start: q(at.saturating_add(k * 3)),
                    len: qlen(4),
                    words: format!("cue {k}"),
                })
                .collect(),
            look: CaptionLook::default(),
            lane: None,
        },
        Op::Transition(at, n) => Command::AddTransitionClip {
            start: q(*at),
            len: Some(qlen(*n)),
            package: "concat.push".into(),
            name: "Push".into(),
            lane: None,
        },
        Op::Lane(bottom) => Command::AddTrack { bottom: *bottom },
        Op::RemoveLane(lane) => Command::RemoveTrack {
            lane: pick(&lanes(m), lane)?,
        },
        Op::Timeline => Command::AddTimeline,
        Op::Select(s) => Command::SelectTimeline {
            sequence: pick(&sequences(m), s)?,
        },
        Op::RemoveTimeline(s) => Command::RemoveTimeline {
            sequence: pick(&sequences(m), s)?,
        },
        Op::Undo => return None,
    })
}

fn without_view(model: &Model) -> Model {
    let mut model = model.clone();
    model.view = Default::default();
    model
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    #[test]
    fn any_run_of_commands_keeps_the_rules_and_undoes_to_nothing(ops in prop::collection::vec(op(), 1..60)) {
        let empty: Document<Model> = Document::new();
        let mut h: Hist = History::new(empty.clone(), Budget::new(10_000, usize::MAX));
        let first = run(h.current(), &Command::AddTimeline, Origin(0)).unwrap();
        h.record(first.commit.unwrap(), Entry::new(vec![], (), ())).unwrap();
        for op in &ops {
            if let Op::Undo = op {
                if h.undo_len() > 1 {
                    h.undo();
                }
                continue;
            }
            let Some(command) = command(h.current().tables(), op) else { continue };
            let before = h.current().clone();
            match run(&before, &command, Origin(0)) {
                Ok(outcome) => {
                    if let Some(commit) = outcome.commit {
                        h.record(commit, Entry::new(vec![command.clone()], (), ())).unwrap();
                    }
                }
                Err(error) => {
                    prop_assert!(Document::ptr_eq(h.current(), &before));
                    // No curve makes a time map the rules refuse.
                    if let CommandError::Broken(broken) = &error {
                        let timing = broken.iter().any(|violation| matches!(
                            violation.rule,
                            Rule::SpeedNotAllowed { .. }
                                | Rule::TimeKeysTooFew
                                | Rule::TimeKeysOutOfOrder
                                | Rule::TimeNotMonotone
                                | Rule::WindowPastKeys
                                | Rule::WindowPastSource { .. }
                                | Rule::SpeedGraphEmpty
                        ) || matches!(
                            violation.rule,
                            Rule::NotFinite { what } | Rule::OutOfBounds { what } | Rule::KeysOutOfOrder { what }
                                if what == "content.time.drawn"
                        ));
                        prop_assert!(!timing, "{:?}: {}", command, error);
                    }
                }
            }
            // Read whole, as a file is read, it keeps every rule.
            let current = h.current();
            let whole = Document::load(current.tables().clone(), current.mint().clone(), current.revision());
            prop_assert!(whole.is_ok(), "after {:?}: {:?}", command, whole.err());
        }
        // What was undone last and not redone is redone first: the end
        // of the history.
        while h.redo().is_some() {}
        let last = h.current().clone();
        // Saved and opened, it is the same document, with nothing to say.
        let text = concat_doc::file::write(&last);
        let opened = concat_doc::file::read(text.as_bytes());
        prop_assert!(opened.is_ok(), "{:?}", opened.err());
        let opened = opened.unwrap();
        prop_assert_eq!(&opened.notes, &vec![]);
        prop_assert_eq!(opened.document.tables(), last.tables());
        // Every speed graph is one the speed pane can draw, and one that
        // shows the window it was drawn over changes nothing when set.
        for (id, item) in last.items.iter() {
            let Some(points) = speed_graph(item) else { continue };
            prop_assert!(points.len() >= 2);
            prop_assert_eq!((points[0].at, points[points.len() - 1].at), (0.0, 1.0));
            prop_assert!(points.windows(2).all(|pair| pair[0].at <= pair[1].at), "{:?}", points);
            prop_assert!(points.iter().all(|p| (1.0 / 16.0..=16.0).contains(&p.speed)), "{:?}", points);
            let whole = match &item.content {
                Content::Media(cut) => matches!(
                    &cut.time,
                    TimeMap::Keyed { drawn: Some(graph), .. }
                        if graph.from == item.offset && graph.len == item.len
                ),
                _ => false,
            };
            if whole {
                let again = Command::SetClipSpeedCurve { item: id, curve: Some(points), ripple: false };
                let outcome = run(&last, &again, Origin(0));
                prop_assert!(outcome.is_ok_and(|outcome| outcome.commit.is_none()), "{}", id);
            }
        }
        while h.undo().is_some() {}
        prop_assert_eq!(without_view(h.current().tables()), without_view(empty.tables()));
        prop_assert!(h.current().view.showing.is_none());
        while h.redo().is_some() {}
        prop_assert_eq!(without_view(h.current().tables()), without_view(last.tables()));
    }
}

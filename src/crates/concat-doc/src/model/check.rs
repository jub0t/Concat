// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The model's rules, checked at commit on what the change list names.
//!
//! The rules themselves are listed in [the model's documentation](super).
//! Each table has one checker, named where the tables are declared, so
//! a table cannot be left unchecked. The checkers run in the order the
//! tables are declared: the files, sequences and lanes first, which
//! name the items a change to them puts in question; then the items,
//! which checks what changed and what was put in question.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::id::{AnyId, TypedId};
use crate::time::{Tick, TickDur, TickLen};

use super::animated::{Animated, Ease, Key};
use super::effect::{Effect, Mask, MaskSource, Param};
use super::ids::{ItemId, LaneId, MaskId, MediaId, SequenceId, known};
use super::index::lowest;
use super::item::{Content, Item, LinkKind, MediaCut, Take};
use super::look::{ShapeStyle, Title};
use super::media::{Media, MediaKind};
use super::sequence::{Background, Marker};
use super::timing::{TimeCurve, TimeMap, dur_in_range, len_in_range, tick_in_range};
use super::{Model, ModelChanges};

/// A rule the change would break, and the id it is broken at.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Violation {
    /// What is at fault: the item, lane, mask or other thing whose own
    /// value breaks the rule. `None` for the document's fonts, which
    /// have no ids.
    pub id: Option<AnyId>,
    /// Which rule.
    pub rule: Rule,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.id {
            Some(id) => write!(f, "{id}: {:?}", self.rule),
            None => write!(f, "the document: {:?}", self.rule),
        }
    }
}

impl std::error::Error for Violation {}

/// The rules of the model. `what` names the field, as the model's types
/// spell it (`picture.opacity`, `content.time`).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Rule {
    /// A tick, a length, or a place worked out from them (an end, the
    /// source a window shows) is more than [`TICK_LIMIT`](super::TICK_LIMIT) from zero.
    OutOfRange {
        /// Which.
        what: &'static str,
    },
    /// An id names nothing in the document.
    Dangling {
        /// Which field names it.
        what: &'static str,
        /// What it names.
        to: AnyId,
    },
    /// An id names something of the wrong sort: a background that is not
    /// a still, a sound taken from a still.
    WrongKind {
        /// Which field names it.
        what: &'static str,
        /// What it names.
        to: AnyId,
    },
    /// A number is not a number, or is infinite.
    NotFinite {
        /// Which.
        what: &'static str,
    },
    /// A number is outside what its field can mean: an opacity past
    /// one, a scale of zero, a polygon of two sides.
    OutOfBounds {
        /// Which.
        what: &'static str,
    },
    /// A length that is kept as a distance is below zero.
    Negative {
        /// Which.
        what: &'static str,
    },
    /// A sequence's name is empty or only spaces.
    BlankName,
    /// An effect names no package.
    EmptyPackage,
    /// An effect's knob has no name, or has neither a value nor keys.
    EmptyParam {
        /// The knob's name.
        name: String,
    },
    /// Keys are not in order of their ticks, or two share a tick.
    KeysOutOfOrder {
        /// Whose keys.
        what: &'static str,
    },
    /// An easing's control points are not numbers, or one of their times
    /// is outside 0 to 1.
    BadEase {
        /// Whose key.
        what: &'static str,
    },
    /// A speed, or the pace between two position keys, is slower than a
    /// sixteenth or faster than sixteen times.
    SpeedNotAllowed {
        /// Which time map.
        what: &'static str,
    },
    /// A keyed time map has fewer than two keys.
    TimeKeysTooFew,
    /// A keyed time map's keys are not in order of their content ticks,
    /// or two share one.
    TimeKeysOutOfOrder,
    /// A keyed time map's source does not run one way all along: it
    /// stands still, turns back, or eases past a key.
    TimeNotMonotone,
    /// The window an item shows reaches past its time map's first or
    /// last key, where the map says nothing.
    WindowPastKeys,
    /// The window an item shows plays a part of its file that the file
    /// does not have: before its start, past its length, or outside its
    /// enhanced copy while the copy shows.
    WindowPastSource {
        /// The file.
        source: MediaId,
    },
    /// Two items overlap on one lane.
    Overlap {
        /// The other.
        with: ItemId,
    },
    /// A transition placed as an item has nothing beneath it: no item on
    /// a lower lane of its sequence, other than another transition, for
    /// any of its span.
    NothingBeneath,
    /// A sequence holds, at some depth, the item that nests it.
    Cycle {
        /// The sequence nested.
        through: SequenceId,
    },
    /// A lane was moved to another sequence.
    LaneMoved,
    /// Two lanes of one sequence are at one place in its stack.
    LaneOrderTaken {
        /// The other.
        with: LaneId,
    },
    /// A file lists one sound stream index twice.
    StreamTwice {
        /// The index.
        index: u32,
    },
    /// A file says it has no sound and lists sound streams.
    StreamsWithoutSound,
    /// An item plays a sound stream its file does not list.
    NoSuchStream {
        /// The index.
        index: u32,
    },
    /// An item's enhanced copy is the file it was cut from.
    EnhancedIsOriginal,
    /// An item is tied to itself.
    LinkToSelf,
    /// An item is tied to one in another sequence.
    LinkAcrossSequences {
        /// What it is tied to.
        to: ItemId,
    },
    /// An item said to be the sound detached from another is not: it is
    /// not the sound alone of the file the other plays whole.
    LinkMismatch {
        /// What it is tied to.
        to: ItemId,
    },
    /// An item is tied to one that is itself the sound of another.
    LinkChain {
        /// What it is tied to.
        to: ItemId,
    },
    /// An effect, mask or marker id is used twice in the document.
    IdTwice {
        /// What else holds it.
        with: AnyId,
    },
    /// A run of a title's words is empty, past the end of the words, or
    /// overlaps or comes before the run before it.
    BadRun {
        /// Which run, counted from zero.
        index: usize,
    },
    /// A sequence is open in two tabs.
    TabTwice,
    /// The document has sequences and none is showing.
    NothingShowing,
    /// Two fonts are the one file.
    FontTwice {
        /// The file.
        path: String,
    },
}

/// What a number may be.
#[derive(Clone, Copy)]
enum Domain {
    /// Any finite number.
    Any,
    /// From 0 to 1.
    Unit,
    /// Zero or more.
    NonNegative,
    /// More than zero.
    Positive,
    /// From the first to the second.
    Between(f64, f64),
}

impl Domain {
    fn holds(self, value: f64) -> bool {
        match self {
            Domain::Any => true,
            Domain::Unit => (0.0..=1.0).contains(&value),
            Domain::NonNegative => value >= 0.0,
            Domain::Positive => value > 0.0,
            Domain::Between(low, high) => (low..=high).contains(&value),
        }
    }
}

/// One commit's checking: the two sides, what changed, what has been
/// found wrong, and what a change elsewhere has put in question.
pub(crate) struct Check<'a> {
    before: &'a Model,
    after: &'a Model,
    changes: &'a ModelChanges,
    found: Vec<Violation>,
    /// Items to check again although they did not change: what names a
    /// file, sequence or lane that changed or went.
    items: BTreeSet<ItemId>,
    /// Lanes to check again: those of a sequence that went.
    lanes: BTreeSet<LaneId>,
    /// Sequences whose transitions are to be checked again for what is
    /// beneath them: those whose stack of lanes changed.
    stacks: BTreeSet<SequenceId>,
}

impl<'a> Check<'a> {
    pub(crate) fn new(before: &'a Model, after: &'a Model, changes: &'a ModelChanges) -> Self {
        Check {
            before,
            after,
            changes,
            found: Vec::new(),
            items: BTreeSet::new(),
            lanes: BTreeSet::new(),
            stacks: BTreeSet::new(),
        }
    }

    /// What was found, each once, in order.
    pub(crate) fn finish(mut self) -> Vec<Violation> {
        self.found.sort();
        self.found.dedup();
        self.found
    }

    fn fault(&mut self, id: impl Into<AnyId>, rule: Rule) {
        self.found.push(Violation {
            id: Some(id.into()),
            rule,
        });
    }

    fn tick(&mut self, id: AnyId, what: &'static str, t: Tick) -> bool {
        let fits = tick_in_range(t);
        if !fits {
            self.fault(id, Rule::OutOfRange { what });
        }
        fits
    }

    fn len(&mut self, id: AnyId, what: &'static str, l: TickLen) -> bool {
        let fits = len_in_range(l);
        if !fits {
            self.fault(id, Rule::OutOfRange { what });
        }
        fits
    }

    /// A distance that is a length: in range and not below zero.
    fn span(&mut self, id: AnyId, what: &'static str, d: TickDur) {
        if !dur_in_range(d) {
            self.fault(id, Rule::OutOfRange { what });
        } else if d < TickDur::ZERO {
            self.fault(id, Rule::Negative { what });
        }
    }

    /// The end of something that starts at `start` and runs `by`: in
    /// range, or the fault said.
    fn end(&mut self, id: AnyId, what: &'static str, start: Tick, by: TickDur) -> Option<Tick> {
        match start.checked_add(by) {
            Some(end) if tick_in_range(end) => Some(end),
            _ => {
                self.fault(id, Rule::OutOfRange { what });
                None
            }
        }
    }

    fn number(&mut self, id: AnyId, what: &'static str, value: f64, domain: Domain) {
        if !value.is_finite() {
            self.fault(id, Rule::NotFinite { what });
        } else if !domain.holds(value) {
            self.fault(id, Rule::OutOfBounds { what });
        }
    }

    fn count(&mut self, id: AnyId, what: &'static str, value: u32, low: u32, high: u32) {
        if !(low..=high).contains(&value) {
            self.fault(id, Rule::OutOfBounds { what });
        }
    }

    fn keys<T>(
        &mut self,
        id: AnyId,
        what: &'static str,
        keys: &[Key<T>],
        mut value: impl FnMut(&mut Self, &T),
    ) {
        for key in keys {
            self.tick(id, what, key.at);
            if let Ease::Bezier { x1, y1, x2, y2 } = key.ease {
                let numbers = [x1, y1, x2, y2].iter().all(|n| n.is_finite());
                let times = (0.0..=1.0).contains(&x1) && (0.0..=1.0).contains(&x2);
                if !numbers || !times {
                    self.fault(id, Rule::BadEase { what });
                }
            }
            value(self, &key.value);
        }
        if keys.windows(2).any(|pair| pair[0].at >= pair[1].at) {
            self.fault(id, Rule::KeysOutOfOrder { what });
        }
    }

    fn animated(&mut self, id: AnyId, what: &'static str, a: &Animated<f64>, domain: Domain) {
        self.number(id, what, a.value, domain);
        self.keys(id, what, &a.keys, |cx, v| cx.number(id, what, *v, domain));
    }

    fn param(&mut self, id: AnyId, name: &str, param: &Param) {
        if name.is_empty() || (param.value.is_none() && param.keys.is_empty()) {
            self.fault(
                id,
                Rule::EmptyParam {
                    name: name.to_owned(),
                },
            );
        }
        if let Some(value) = param.value {
            self.number(id, "params", value, Domain::Any);
        }
        self.keys(id, "params", &param.keys, |cx, v| {
            cx.number(id, "params", *v, Domain::Any)
        });
    }

    fn effect(&mut self, effect: &Effect, masks: &BTreeSet<MaskId>) {
        let id = effect.id.any();
        if effect.package.is_empty() {
            self.fault(id, Rule::EmptyPackage);
        }
        for (name, param) in &effect.params {
            self.param(id, name, param);
        }
        if let Some(mask) = effect.mask
            && !masks.contains(&mask)
        {
            self.fault(
                id,
                Rule::Dangling {
                    what: "mask",
                    to: mask.any(),
                },
            );
        }
    }

    fn mask(&mut self, mask: &Mask) {
        let id = mask.id.any();
        self.animated(id, "x", &mask.x, Domain::Any);
        self.animated(id, "y", &mask.y, Domain::Any);
        self.animated(id, "scale", &mask.scale, Domain::Positive);
        self.animated(id, "rotation", &mask.rotation, Domain::Any);
        self.animated(id, "feather", &mask.feather, Domain::Unit);
        self.animated(id, "expand", &mask.expand, Domain::Any);
        self.animated(id, "opacity", &mask.opacity, Domain::Unit);
        if let MaskSource::Shape {
            width,
            height,
            roundness,
            ..
        } = &mask.source
        {
            self.animated(id, "source.width", width, Domain::NonNegative);
            self.animated(id, "source.height", height, Domain::NonNegative);
            self.animated(id, "source.roundness", roundness, Domain::Unit);
        }
    }

    fn marker(&mut self, marker: &Marker) {
        let id = marker.id.any();
        if self.tick(id, "at", marker.at) {
            self.span(id, "len", marker.len);
            if marker.len >= TickDur::ZERO {
                self.end(id, "len", marker.at, marker.len);
            }
        }
    }

    fn background(&mut self, id: AnyId, what: &'static str, background: &Background) {
        match background {
            Background::Blur { amount } => self.number(id, what, *amount, Domain::Unit),
            Background::Image(media) => match self.after.media.get(*media) {
                None => self.fault(
                    id,
                    Rule::Dangling {
                        what,
                        to: media.any(),
                    },
                ),
                Some(file) if matches!(file.kind, MediaKind::Video | MediaKind::Audio) => self
                    .fault(
                        id,
                        Rule::WrongKind {
                            what,
                            to: media.any(),
                        },
                    ),
                Some(_) => {}
            },
            Background::None | Background::Color(_) | Background::Unknown(_) => {}
        }
    }

    /// Every effect, mask and marker id is held once in the document:
    /// once in its holder, and by no other holder.
    fn nested_once(&mut self, owner: AnyId, ids: &[AnyId]) {
        let mut seen = BTreeSet::new();
        for id in ids {
            if !seen.insert(*id) {
                self.fault(*id, Rule::IdTwice { with: owner });
            }
            let items = self.after.items.index().holding(*id).map(TypedId::any);
            let sequences = self.after.sequences.index().holding(*id).map(TypedId::any);
            let others: Vec<AnyId> = items.chain(sequences).filter(|o| *o != owner).collect();
            for other in others {
                self.fault(*id, Rule::IdTwice { with: other });
            }
        }
    }
}

// --- Files -------------------------------------------------------------

pub(crate) fn media(cx: &mut Check<'_>) {
    let changes = &cx.changes.media;
    let written = changes.added.iter().map(|(id, m)| (*id, m));
    let rewritten = changes.replaced.iter().map(|r| (r.id, &r.new));
    for (id, file) in written.chain(rewritten) {
        one_media(cx, id, file);
    }
    // What names a file that changed or went is put in question: an item
    // may take from it what it no longer has, or name it when it is gone.
    let touched = changes
        .replaced
        .iter()
        .map(|r| r.id)
        .chain(changes.removed.iter().map(|(id, _)| *id));
    let after = cx.after;
    for media in touched {
        cx.items.extend(after.items.index().using_media(media));
    }
    for (id, _) in &changes.removed {
        for (sequence, held) in after.sequences.iter() {
            if held.background.media() == Some(*id) {
                cx.fault(
                    sequence,
                    Rule::Dangling {
                        what: "background",
                        to: id.any(),
                    },
                );
            }
        }
    }
}

fn one_media(cx: &mut Check<'_>, id: MediaId, file: &Media) {
    let any = id.any();
    if let Some(length) = file.length {
        cx.len(any, "length", length);
    }
    let mut seen = BTreeSet::new();
    for stream in &file.streams {
        if !seen.insert(stream.index) {
            cx.fault(
                any,
                Rule::StreamTwice {
                    index: stream.index,
                },
            );
        }
    }
    if !file.has_sound && !file.streams.is_empty() {
        cx.fault(any, Rule::StreamsWithoutSound);
    }
}

// --- Sequences ---------------------------------------------------------

pub(crate) fn sequences(cx: &mut Check<'_>) {
    let changes = &cx.changes.sequences;
    let written = changes.added.iter().map(|(id, s)| (*id, s));
    let rewritten = changes.replaced.iter().map(|r| (r.id, &r.new));
    for (id, sequence) in written.chain(rewritten) {
        let any = id.any();
        if sequence.name.trim().is_empty() {
            cx.fault(any, Rule::BlankName);
        }
        cx.background(any, "background", &sequence.background);
        for marker in known(&sequence.markers) {
            cx.marker(marker);
        }
        let ids: Vec<AnyId> = known(&sequence.markers).map(|m| m.id.any()).collect();
        cx.nested_once(any, &ids);
    }
    // A sequence that went takes nothing with it: its lanes and what
    // nests it must have gone too.
    let after = cx.after;
    for (id, _) in &changes.removed {
        cx.lanes
            .extend(after.lanes.index().of(*id).map(|(lane, _)| lane));
        cx.items.extend(after.items.index().nesting(*id));
    }
}

// --- Lanes -------------------------------------------------------------

pub(crate) fn lanes(cx: &mut Check<'_>) {
    let (before, after, changes) = (cx.before, cx.after, &cx.changes.lanes);
    let mut check: BTreeSet<LaneId> = std::mem::take(&mut cx.lanes);
    check.extend(changes.added.iter().map(|(id, _)| *id));
    for replaced in &changes.replaced {
        check.insert(replaced.id);
        if replaced.old.sequence != replaced.new.sequence {
            cx.fault(replaced.id, Rule::LaneMoved);
        }
        if replaced.old.order != replaced.new.order {
            cx.stacks.insert(replaced.new.sequence);
        }
    }
    for (id, _) in &changes.added {
        if let Some(lane) = after.lanes.get(*id) {
            cx.stacks.insert(lane.sequence);
        }
    }
    for id in check {
        let Some(lane) = after.lanes.get(id) else {
            continue;
        };
        if !after.sequences.contains(lane.sequence) {
            cx.fault(
                id,
                Rule::Dangling {
                    what: "sequence",
                    to: lane.sequence.any(),
                },
            );
        }
        let others: Vec<LaneId> = after
            .lanes
            .index()
            .at(lane.sequence, &lane.order)
            .filter(|other| *other != id)
            .collect();
        for other in others {
            cx.fault(id, Rule::LaneOrderTaken { with: other });
        }
    }
    // A lane that went takes nothing with it, and what was beneath a
    // transition above it may have gone.
    for (id, lane) in &changes.removed {
        cx.items
            .extend(after.items.index().on_lane(*id).map(|(item, _, _)| item));
        if before.sequences.contains(lane.sequence) {
            cx.stacks.insert(lane.sequence);
        }
    }
}

// --- Items -------------------------------------------------------------

pub(crate) fn items(cx: &mut Check<'_>) {
    let (before, after, changes) = (cx.before, cx.after, &cx.changes.items);
    let mut check: BTreeSet<ItemId> = std::mem::take(&mut cx.items);
    let mut moved: Vec<(ItemId, &Item)> = Vec::new();
    check.extend(changes.added.iter().map(|(id, _)| *id));
    for replaced in &changes.replaced {
        check.insert(replaced.id);
        moved.push((replaced.id, &replaced.old));
        // What is tied to it may no longer match it.
        check.extend(after.items.index().tied_to(replaced.id));
    }
    for (id, old) in &changes.removed {
        moved.push((*id, old));
        // What was tied to it would be tied to nothing.
        check.extend(after.items.index().tied_to(*id));
    }

    let mut transitions: BTreeSet<ItemId> = BTreeSet::new();
    for id in &check {
        let Some(item) = after.items.get(*id) else {
            continue;
        };
        one_item(cx, *id, item);
        if item.is_transition() {
            transitions.insert(*id);
        }
    }
    overlaps(cx);

    // A transition stands over what is beneath it: if that moved or went,
    // or the stack under it changed, it is looked at again.
    for (_, old) in moved {
        let (Some(lane), Some(end)) = (before.lanes.get(old.lane), old.end()) else {
            continue;
        };
        for above in after.lanes.index().above(lane.sequence, &lane.order) {
            let over = after
                .items
                .index()
                .transitions_overlapping(above, old.start, end);
            transitions.extend(over.into_iter().map(|(id, _, _)| id));
        }
    }
    for sequence in std::mem::take(&mut cx.stacks) {
        for (lane, _) in after.lanes.index().of(sequence) {
            let on = after.items.index().transitions_on(lane);
            transitions.extend(on.map(|(id, _, _)| id));
        }
    }
    for id in transitions {
        if let Some(item) = after.items.get(id)
            && !over_something(after, item)
        {
            cx.fault(id, Rule::NothingBeneath);
        }
    }
}

fn one_item(cx: &mut Check<'_>, id: ItemId, item: &Item) {
    let any = id.any();
    let after = cx.after;

    // Where it is.
    let lane = after.lanes.get(item.lane);
    if lane.is_none() {
        cx.fault(
            any,
            Rule::Dangling {
                what: "lane",
                to: item.lane.any(),
            },
        );
    }
    let placed = cx.tick(any, "start", item.start)
        & cx.tick(any, "offset", item.offset)
        & cx.len(any, "len", item.len);
    let window = if placed {
        let end = cx.end(any, "len", item.start, item.len.dur());
        let window_end = cx.end(any, "offset", item.offset, item.len.dur());
        end.and(window_end).map(|w| (item.offset, w))
    } else {
        None
    };

    // What it shows.
    match &item.content {
        Content::Media(cut) => media_cut(cx, id, cut, window),
        Content::Nest(nest) => {
            if !after.sequences.contains(nest.sequence) {
                cx.fault(
                    any,
                    Rule::Dangling {
                        what: "content.sequence",
                        to: nest.sequence.any(),
                    },
                );
            }
            time_map(cx, any, &nest.time, window);
            if let Some(lane) = lane {
                cycle(cx, id, lane.sequence, nest.sequence);
            }
        }
        Content::Title(title) => self::title(cx, any, title),
        Content::Shape(shape) => self::shape(cx, any, shape),
        Content::Transition(effect) => cx.effect(effect, &BTreeSet::new()),
        Content::Layer | Content::SoundLayer | Content::Unknown { .. } => {}
    }

    // Its picture.
    let p = &item.picture;
    cx.animated(any, "picture.offset_x", &p.offset_x, Domain::Any);
    cx.animated(any, "picture.offset_y", &p.offset_y, Domain::Any);
    cx.animated(any, "picture.scale", &p.scale, Domain::Positive);
    cx.animated(any, "picture.rotation", &p.rotation, Domain::Any);
    cx.animated(any, "picture.stretch_x", &p.stretch_x, Domain::Positive);
    cx.animated(any, "picture.stretch_y", &p.stretch_y, Domain::Positive);
    cx.animated(any, "picture.opacity", &p.opacity, Domain::Unit);
    if let Some(crop) = &p.crop {
        let sides = [crop.left, crop.top, crop.right, crop.bottom];
        if sides.iter().any(|side| !side.is_finite()) {
            cx.fault(
                any,
                Rule::NotFinite {
                    what: "picture.crop",
                },
            );
        } else if sides.iter().any(|side| !(0.0..1.0).contains(side))
            || crop.left + crop.right >= 1.0
            || crop.top + crop.bottom >= 1.0
        {
            cx.fault(
                any,
                Rule::OutOfBounds {
                    what: "picture.crop",
                },
            );
        }
    }
    if let Some(cutout) = &p.cutout {
        cx.number(
            any,
            "picture.cutout.feather",
            cutout.feather,
            Domain::NonNegative,
        );
        for stroke in &cutout.strokes {
            cx.number(any, "picture.cutout.strokes", stroke.size, Domain::Positive);
            for point in stroke.points.iter().flatten() {
                cx.number(any, "picture.cutout.strokes", *point, Domain::Any);
            }
            if let Some(at) = stroke.at {
                cx.tick(any, "picture.cutout.strokes", at);
            }
        }
    }
    if let Some(stabilize) = &p.stabilize {
        cx.number(
            any,
            "picture.stabilize.smoothness",
            stabilize.smoothness,
            Domain::Unit,
        );
        cx.number(
            any,
            "picture.stabilize.max_zoom",
            stabilize.max_zoom,
            Domain::Between(1.0, f64::MAX),
        );
    }
    if let Some(background) = &p.background {
        cx.background(any, "picture.background", background);
    }
    for movement in [&p.motion.enter, &p.motion.exit].into_iter().flatten() {
        cx.len(any, "picture.motion", movement.len);
    }
    if let Some(repeat) = &p.motion.repeat {
        cx.len(any, "picture.motion", repeat.lap);
        cx.number(
            any,
            "picture.motion",
            repeat.amount,
            Domain::Between(0.0, 2.0),
        );
    }

    // Its sound.
    let s = &item.sound;
    cx.animated(any, "sound.volume", &s.volume, Domain::NonNegative);
    cx.span(any, "sound.fade_in", s.fade_in);
    cx.span(any, "sound.fade_out", s.fade_out);
    if let Some(voice) = &s.voice {
        cx.number(any, "sound.voice", voice.voice, Domain::NonNegative);
        cx.number(any, "sound.voice", voice.background, Domain::NonNegative);
    }

    // What it holds.
    let masks: BTreeSet<MaskId> = known(&item.masks).map(|mask| mask.id).collect();
    for effect in known(&item.effects).chain(known(&item.filters)) {
        cx.effect(effect, &masks);
    }
    if let Some(cut) = &item.cut_in {
        cx.effect(&cut.effect, &BTreeSet::new());
        cx.len(cut.effect.id.any(), "cut_in.len", cut.len);
    }
    for mask in known(&item.masks) {
        cx.mask(mask);
    }
    for marker in known(&item.markers) {
        cx.marker(marker);
    }
    cx.nested_once(any, &item.nested_ids());

    // What it is tied to.
    if let Some(link) = &item.link {
        self::link(cx, id, item, link.to, &link.kind);
    }
}

fn media_cut(cx: &mut Check<'_>, id: ItemId, cut: &MediaCut, window: Option<(Tick, Tick)>) {
    let any = id.any();
    let after = cx.after;
    let file = after.media.get(cut.media);
    let Some(file) = file else {
        cx.fault(
            any,
            Rule::Dangling {
                what: "content.media",
                to: cut.media.any(),
            },
        );
        time_map(cx, any, &cut.time, window);
        return;
    };
    if cut.take == Take::Sound && file.kind.can_sound() == Some(false) {
        cx.fault(
            any,
            Rule::WrongKind {
                what: "content.take",
                to: cut.media.any(),
            },
        );
    }
    if let Some(index) = cut.stream
        && !file.streams.is_empty()
        && !file.streams.iter().any(|stream| stream.index == index)
    {
        cx.fault(any, Rule::NoSuchStream { index });
    }
    let mut copy: Option<(TickDur, Option<TickLen>)> = None;
    if let Some(enhanced) = &cut.enhanced {
        cx.span(any, "content.enhanced.copy_start", enhanced.copy_start);
        if enhanced.copy == cut.media {
            cx.fault(any, Rule::EnhancedIsOriginal);
        }
        match after.media.get(enhanced.copy) {
            None => cx.fault(
                any,
                Rule::Dangling {
                    what: "content.enhanced.copy",
                    to: enhanced.copy.any(),
                },
            ),
            Some(held) if enhanced.on && held.kind != MediaKind::Image => {
                copy = Some((enhanced.copy_start, held.length));
            }
            Some(_) => {}
        }
    }
    let shown = time_map(cx, any, &cut.time, window);
    // A still is the same picture at every time, and has no time to run
    // out of.
    if let Some((low, high)) = shown
        && file.kind != MediaKind::Image
    {
        let mut inside = low >= Tick::ZERO && file.length.is_none_or(|l| high <= Tick::ZERO + l);
        if let Some((start, length)) = copy {
            let from = Tick::ZERO + start;
            inside &= low >= from
                && length.is_none_or(|l| high.checked_since(from).is_some_and(|d| d <= l.dur()));
        }
        if !inside {
            cx.fault(any, Rule::WindowPastSource { source: cut.media });
        }
    }
}

/// Checks a time map, and gives the lowest and highest source tick that
/// the window shows, where they can be known.
fn time_map(
    cx: &mut Check<'_>,
    id: AnyId,
    map: &TimeMap,
    window: Option<(Tick, Tick)>,
) -> Option<(Tick, Tick)> {
    const WHAT: &str = "content.time";
    match map {
        TimeMap::Linear { src_in, speed } => {
            if !speed.is_allowed() {
                cx.fault(id, Rule::SpeedNotAllowed { what: WHAT });
            }
            if !cx.tick(id, WHAT, *src_in) {
                return None;
            }
            let (from, to) = window?;
            let ends = [map.source_at(from), map.source_at(to)];
            match ends {
                [Some(a), Some(b)] if tick_in_range(a) && tick_in_range(b) => {
                    Some((a.min(b), a.max(b)))
                }
                _ => {
                    cx.fault(id, Rule::OutOfRange { what: WHAT });
                    None
                }
            }
        }
        TimeMap::Keyed { first, rest } => {
            let points = map.points();
            let mut fits = true;
            for point in &points {
                fits &= cx.tick(id, WHAT, point.at) & cx.tick(id, WHAT, point.source);
            }
            if rest.is_empty() {
                cx.fault(id, Rule::TimeKeysTooFew);
                return None;
            }
            if points.windows(2).any(|pair| pair[0].at >= pair[1].at) {
                cx.fault(id, Rule::TimeKeysOutOfOrder);
                return None;
            }
            if !fits {
                return None;
            }
            // Which way the source runs, from the first stretch.
            let rising = rest[0].1.source > first.source;
            let mut from = *first;
            for (curve, to) in rest {
                let run = i128::from(to.at.get()) - i128::from(from.at.get());
                let rise = i128::from(to.source.get()) - i128::from(from.source.get());
                if rise == 0 || (rise > 0) != rising {
                    cx.fault(id, Rule::TimeNotMonotone);
                } else if rise.abs() * 16 < run || rise.abs() > run * 16 {
                    cx.fault(id, Rule::SpeedNotAllowed { what: WHAT });
                }
                if let TimeCurve::Smooth { leave, arrive } = curve {
                    if !leave.is_allowed() || !arrive.is_allowed() {
                        cx.fault(id, Rule::SpeedNotAllowed { what: WHAT });
                    }
                    let secant = rise as f64 / run as f64;
                    let alpha = leave.display() / secant;
                    let beta = arrive.display() / secant;
                    // Fritsch and Carlson's condition: each tangent the
                    // same way as the stretch, and inside the circle of
                    // radius three, keeps the cubic from turning back.
                    if alpha <= 0.0 || beta <= 0.0 || alpha * alpha + beta * beta > 9.0 {
                        cx.fault(id, Rule::TimeNotMonotone);
                    }
                }
                from = *to;
            }
            let (start, end) = window?;
            let last = points[points.len() - 1];
            if start < first.at || end > last.at {
                cx.fault(id, Rule::WindowPastKeys);
                return None;
            }
            // The source the window shows: exactly on a straight stretch,
            // and up to the stretch's ends on a curved one, which runs
            // one way between them.
            let mut low = Tick::MAX;
            let mut high = Tick::MIN;
            let mut from = *first;
            for (curve, to) in rest {
                if to.at > start && from.at < end {
                    let (a, b) = match curve {
                        TimeCurve::Linear => (
                            map.source_at(start.max(from.at)),
                            map.source_at(end.min(to.at)),
                        ),
                        TimeCurve::Smooth { .. } | TimeCurve::Unknown(_) => {
                            (Some(from.source), Some(to.source))
                        }
                    };
                    let (a, b) = (a?, b?);
                    low = low.min(a.min(b));
                    high = high.max(a.max(b));
                }
                from = *to;
            }
            (low <= high).then_some((low, high))
        }
        TimeMap::Unknown(_) => None,
    }
}

fn cycle(cx: &mut Check<'_>, id: ItemId, home: SequenceId, nested: SequenceId) {
    // `nested` holds `home`, at some depth, exactly when it is among the
    // sequences that hold `home`, found by walking up through what nests
    // each.
    let after = cx.after;
    let mut seen = BTreeSet::new();
    let mut todo = vec![home];
    while let Some(sequence) = todo.pop() {
        if sequence == nested {
            cx.fault(id, Rule::Cycle { through: nested });
            return;
        }
        if !seen.insert(sequence) {
            continue;
        }
        for holder in after.items.index().nesting(sequence) {
            let holder = after.items.get(holder);
            if let Some(lane) = holder.and_then(|item| after.lanes.get(item.lane)) {
                todo.push(lane.sequence);
            }
        }
    }
}

fn link(cx: &mut Check<'_>, id: ItemId, item: &Item, to: ItemId, kind: &LinkKind) {
    let after = cx.after;
    if to == id {
        cx.fault(id, Rule::LinkToSelf);
        return;
    }
    let Some(target) = after.items.get(to) else {
        cx.fault(
            id,
            Rule::Dangling {
                what: "link",
                to: to.any(),
            },
        );
        return;
    };
    let home = |of: &Item| after.lanes.get(of.lane).map(|lane| lane.sequence);
    if let (Some(mine), Some(theirs)) = (home(item), home(target))
        && mine != theirs
    {
        cx.fault(id, Rule::LinkAcrossSequences { to });
    }
    if *kind == LinkKind::SoundOf {
        let matches = match (&item.content, &target.content) {
            (Content::Media(sound), Content::Media(picture)) => {
                sound.media == picture.media
                    && sound.take == Take::Sound
                    && picture.take == Take::Whole
            }
            _ => false,
        };
        if !matches {
            cx.fault(id, Rule::LinkMismatch { to });
        }
        if target
            .link
            .as_ref()
            .is_some_and(|theirs| theirs.kind == LinkKind::SoundOf)
        {
            cx.fault(id, Rule::LinkChain { to });
        }
    }
}

fn title(cx: &mut Check<'_>, id: AnyId, title: &Title) {
    cx.number(id, "title.font_size", title.font_size, Domain::Positive);
    cx.number(
        id,
        "title.font_weight",
        title.font_weight,
        Domain::Between(100.0, 900.0),
    );
    cx.number(id, "title.opacity", title.opacity, Domain::Unit);
    cx.number(
        id,
        "title.line_height",
        title.line_height,
        Domain::Between(0.5, f64::MAX),
    );
    cx.number(id, "title.tracking", title.tracking, Domain::Any);
    cx.number(id, "title.max_width", title.max_width, Domain::NonNegative);
    cx.number(
        id,
        "title.max_height",
        title.max_height,
        Domain::NonNegative,
    );
    cx.number(
        id,
        "title.stroke_width",
        title.stroke_width,
        Domain::NonNegative,
    );
    cx.number(
        id,
        "title.plate_padding_x",
        title.plate_padding_x,
        Domain::NonNegative,
    );
    cx.number(
        id,
        "title.plate_padding_y",
        title.plate_padding_y,
        Domain::NonNegative,
    );
    cx.number(
        id,
        "title.plate_radius",
        title.plate_radius,
        Domain::NonNegative,
    );
    let chars = title.content.chars().count();
    let mut after_last = 0_u32;
    for (index, run) in title.runs.iter().enumerate() {
        let fits = usize::try_from(run.end).is_ok_and(|end| end <= chars);
        if run.start >= run.end || run.start < after_last || !fits {
            cx.fault(id, Rule::BadRun { index });
        }
        after_last = after_last.max(run.end);
        if let Some(scale) = run.font_scale {
            cx.number(
                id,
                "title.runs.font_scale",
                scale,
                Domain::Between(0.1, 10.0),
            );
        }
        if let Some(weight) = run.font_weight {
            cx.number(
                id,
                "title.runs.font_weight",
                weight,
                Domain::Between(100.0, 900.0),
            );
        }
    }
}

fn shape(cx: &mut Check<'_>, id: AnyId, shape: &ShapeStyle) {
    cx.number(id, "shape.width", shape.width, Domain::NonNegative);
    cx.number(id, "shape.height", shape.height, Domain::NonNegative);
    cx.number(id, "shape.corner_radius", shape.corner_radius, Domain::Unit);
    cx.count(id, "shape.sides", shape.sides, 3, 12);
    cx.count(id, "shape.points", shape.points, 3, 24);
    cx.number(id, "shape.inner_radius", shape.inner_radius, Domain::Unit);
    cx.number(
        id,
        "shape.stroke_width",
        shape.stroke_width,
        Domain::NonNegative,
    );
    cx.number(id, "shape.trim_start", shape.trim_start, Domain::Unit);
    cx.number(id, "shape.trim_end", shape.trim_end, Domain::Unit);
    if let Some(gradient) = &shape.gradient {
        cx.number(id, "shape.gradient.angle", gradient.angle, Domain::Any);
    }
    if let Some(shadow) = &shape.shadow {
        cx.number(id, "shape.shadow.angle", shadow.angle, Domain::Any);
        cx.number(
            id,
            "shape.shadow.distance",
            shadow.distance,
            Domain::NonNegative,
        );
        cx.number(id, "shape.shadow.blur", shadow.blur, Domain::NonNegative);
    }
}

/// No two items overlap on a lane, checked on the stretches of the lanes
/// where items came or changed.
///
/// What was on a lane before the commit kept the rule, so the items that
/// start before the first changed one on a lane are apart, and the last
/// of them to start reaches furthest. The walk starts there and goes on
/// past the end of the last changed one, holding the furthest end seen:
/// an item that starts before it overlaps the item that reached it.
fn overlaps(cx: &mut Check<'_>) {
    let after = cx.after;
    let changes = &cx.changes.items;
    let mut changed: BTreeSet<ItemId> = BTreeSet::new();
    let mut stretches: BTreeMap<LaneId, (Tick, Tick)> = BTreeMap::new();
    let written = changes.added.iter().map(|(id, item)| (*id, item));
    let rewritten = changes.replaced.iter().map(|r| (r.id, &r.new));
    for (id, item) in written.chain(rewritten) {
        changed.insert(id);
        let end = item.end().unwrap_or(Tick::MAX);
        let stretch = stretches.entry(item.lane).or_insert((item.start, end));
        stretch.0 = stretch.0.min(item.start);
        stretch.1 = stretch.1.max(end);
    }
    let index = after.items.index();
    for (lane, (from, to)) in stretches {
        let (start, first) = match index.before(lane, from, lowest::<ItemId>()) {
            Some((id, start, _)) => (start, id),
            None => (from, lowest::<ItemId>()),
        };
        let mut reach: Option<(ItemId, Tick)> = None;
        for (id, start, end) in index.from(lane, start, first) {
            if start >= to {
                break;
            }
            if let Some((holder, furthest)) = reach
                && start < furthest
            {
                if changed.contains(&id) {
                    cx.fault(id, Rule::Overlap { with: holder });
                } else {
                    cx.fault(holder, Rule::Overlap { with: id });
                }
            }
            if reach.is_none_or(|(_, furthest)| end > furthest) {
                reach = Some((id, end));
            }
        }
    }
}

/// Whether something other than a transition is beneath an item, on a
/// lower lane of its sequence, for some of its span. An item whose lane
/// is gone is said to be over something: that it is on nothing is its
/// own fault, found where its lane is checked.
fn over_something(after: &Model, item: &Item) -> bool {
    let (Some(lane), Some(end)) = (after.lanes.get(item.lane), item.end()) else {
        return true;
    };
    let index = after.items.index();
    after
        .lanes
        .index()
        .below(lane.sequence, &lane.order)
        .any(|lower| {
            index
                .overlapping(lower, item.start, end)
                .into_iter()
                .any(|(id, _, _)| {
                    after
                        .items
                        .get(id)
                        .is_some_and(|under| !under.is_transition())
                })
        })
}

// --- What is held unread -----------------------------------------------

/// Held entries are text, and keep no rule but their own existence.
pub(crate) fn held(_: &mut Check<'_>) {}

// --- Plain values ------------------------------------------------------

pub(crate) fn fonts(cx: &mut Check<'_>) {
    if cx.changes.fonts.is_none() {
        return;
    }
    let mut seen = BTreeSet::new();
    for font in &cx.after.fonts {
        if !seen.insert(font.path.as_str()) {
            cx.found.push(Violation {
                id: None,
                rule: Rule::FontTwice {
                    path: font.path.clone(),
                },
            });
        }
    }
}

pub(crate) fn view(cx: &mut Check<'_>) {
    let sequences = &cx.changes.sequences;
    let stack_changed = !sequences.added.is_empty() || !sequences.removed.is_empty();
    if cx.changes.view.is_none() && !stack_changed {
        return;
    }
    let after = cx.after;
    let view = &after.view;
    if let Some(showing) = view.showing
        && !after.sequences.contains(showing)
    {
        cx.fault(
            showing,
            Rule::Dangling {
                what: "view.showing",
                to: showing.any(),
            },
        );
    }
    let mut seen = BTreeSet::new();
    for tab in &view.tabs {
        if !after.sequences.contains(*tab) {
            cx.fault(
                *tab,
                Rule::Dangling {
                    what: "view.tabs",
                    to: tab.any(),
                },
            );
        }
        if !seen.insert(*tab) {
            cx.fault(*tab, Rule::TabTwice);
        }
    }
    if view.showing.is_none()
        && let Some(first) = after.sequences.ids().next()
    {
        cx.fault(first, Rule::NothingShowing);
    }
}

pub(crate) fn residual(_: &mut Check<'_>) {}

/// The ids a draft added inside items and sequences: effects, masks and
/// markers, which no table holds and the mint must still have made.
pub(crate) fn nested_added(changes: &ModelChanges) -> Vec<AnyId> {
    let mut added = Vec::new();
    for (_, item) in &changes.items.added {
        added.extend(item.nested_ids());
    }
    for replaced in &changes.items.replaced {
        let old: BTreeSet<AnyId> = replaced.old.nested_ids().into_iter().collect();
        added.extend(
            replaced
                .new
                .nested_ids()
                .into_iter()
                .filter(|id| !old.contains(id)),
        );
    }
    let markers = |s: &super::sequence::Sequence| -> Vec<AnyId> {
        known(&s.markers).map(|m| m.id.any()).collect()
    };
    for (_, sequence) in &changes.sequences.added {
        added.extend(markers(sequence));
    }
    for replaced in &changes.sequences.replaced {
        let old: BTreeSet<AnyId> = markers(&replaced.old).into_iter().collect();
        added.extend(
            markers(&replaced.new)
                .into_iter()
                .filter(|id| !old.contains(id)),
        );
    }
    added
}

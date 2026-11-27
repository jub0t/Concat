// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Reading the document the window draws.
//!
//! The session hands the window a snapshot of the [`Model`]; this module
//! is the one place that knows how to read it the way the window needs:
//! the sequence showing and its lanes in stacking order, an item or a file
//! by id, a lane's items in time order, what kind of thing an item is to
//! the panels, and the number an animated value has at a place.
//!
//! It is also the one boundary between the document's ticks and the
//! seconds the panels show. Every number in seconds a panel hands the
//! window goes into ticks here, rounded the model's way
//! ([`Tick::from_display_seconds`]), and every tick a panel shows comes
//! out here.

use std::collections::BTreeMap;

use concat_doc::commands::{KeyProperty, MaskNumber};
use concat_doc::model::{
    Animated, Color, Content, Ease, Effect, Entry, Item, ItemId, Key, Lane, LaneId, Mask, Media,
    MediaId, MediaKind, Model, Param, Sequence, SequenceId, ShapeStyle, Speed, Take, TimeCurve,
    TimeMap, Title,
};
use concat_doc::model::{
    Blend, Cutout, CutoutMode, LineHead, MaskId, MaskMode, MaskShape, MaskSource, ShapeKind,
    Stabilize, Subject, VoiceKind, VoiceMix,
};
use concat_doc::{Tick, TickDur, TickLen, TypedId};

/// What an item is to the panels: which inspector it opens, how its lane
/// draws it, what it can be asked to do. Worked out from the item's
/// content and, for a cut, from its file.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    /// A cut of a video's picture and sound.
    Video,
    /// Sound alone: a recording, or a video's sound detached.
    Audio,
    /// A still.
    Image,
    /// A title.
    Text,
    /// A treatment of the picture beneath it.
    Layer,
    /// A treatment of the sound beneath it.
    SoundLayer,
    /// A figure drawn over the picture.
    Shape,
    /// A transition placed as an item.
    Transition,
    /// What the window has no panel for: a nested sequence, a cut of a
    /// file the bin no longer has, a kind a later build wrote.
    Other,
}

impl Kind {
    /// Whether it draws a picture of its own.
    pub fn is_visual(self) -> bool {
        matches!(self, Kind::Video | Kind::Image | Kind::Text | Kind::Shape)
    }

    /// Whether it plays sound of its own.
    pub fn is_audible(self) -> bool {
        matches!(self, Kind::Video | Kind::Audio)
    }
}

/// What an item is, read with the document it is in.
pub fn kind(doc: &Model, item: &Item) -> Kind {
    match &item.content {
        Content::Media(cut) => match doc.media.get(cut.media).map(|media| &media.kind) {
            Some(MediaKind::Video) if cut.take == Take::Sound => Kind::Audio,
            Some(MediaKind::Video) => Kind::Video,
            Some(MediaKind::Audio) => Kind::Audio,
            Some(MediaKind::Image) => Kind::Image,
            Some(MediaKind::Unknown(_)) | None => Kind::Other,
        },
        Content::Title(_) => Kind::Text,
        Content::Shape(_) => Kind::Shape,
        Content::Layer => Kind::Layer,
        Content::SoundLayer => Kind::SoundLayer,
        Content::Transition(_) => Kind::Transition,
        Content::Nest(_) | Content::Unknown { .. } => Kind::Other,
    }
}

// ── ids ──

/// An id of the document as the panels hold it: its text, `"c12"`.
pub fn text<I: TypedId>(id: I) -> slint::SharedString {
    id.to_string().into()
}

/// An id the panels handed back, read by the id's own parsing. `None`
/// for anything that is not an id of that kind, an empty string included.
pub fn parse<I: TypedId + std::str::FromStr>(text: &str) -> Option<I> {
    text.parse().ok()
}

// ── what is showing ──

/// The sequence showing, if the document has one.
pub fn showing(doc: &Model) -> Option<SequenceId> {
    doc.view.showing.filter(|id| doc.sequences.contains(*id))
}

/// The sequence showing, read.
pub fn sequence(doc: &Model) -> Option<&Sequence> {
    showing(doc)
        .and_then(|id| doc.sequences.get(id))
        .map(|s| &**s)
}

/// A sequence's lanes from the bottom of its stack to the top: the order
/// they are drawn in, the lowest under everything.
pub fn lanes_of(doc: &Model, sequence: SequenceId) -> Vec<LaneId> {
    doc.lanes.index().of(sequence).map(|(id, _)| id).collect()
}

/// The showing sequence's lanes, bottom first. Empty with none showing.
pub fn lanes(doc: &Model) -> Vec<LaneId> {
    showing(doc).map_or_else(Vec::new, |sequence| lanes_of(doc, sequence))
}

/// A lane by id.
pub fn lane(doc: &Model, id: LaneId) -> Option<&Lane> {
    doc.lanes.get(id).map(|lane| &**lane)
}

/// An item by id.
pub fn item(doc: &Model, id: ItemId) -> Option<&Item> {
    doc.items.get(id).map(|item| &**item)
}

/// An item by the id the panels hold for it.
pub fn item_by<'a>(doc: &'a Model, id: &str) -> Option<&'a Item> {
    item(doc, parse(id)?)
}

/// A file of the bin by id.
pub fn media(doc: &Model, id: MediaId) -> Option<&Media> {
    doc.media.get(id).map(|media| &**media)
}

/// The bin, in the order its files came in: ids are minted in order, so
/// the table's own order is the order of import.
pub fn bin(doc: &Model) -> impl Iterator<Item = (MediaId, &Media)> {
    doc.media.iter().map(|(id, media)| (id, &**media))
}

/// A lane's items in time order, from the model's index.
pub fn on_lane(doc: &Model, lane: LaneId) -> impl Iterator<Item = (ItemId, &Item)> {
    doc.items
        .index()
        .on_lane(lane)
        .filter_map(move |(id, _, _)| Some((id, item(doc, id)?)))
}

/// Every item of a sequence, lane by lane from the bottom, each lane in
/// time order.
pub fn items_of(doc: &Model, sequence: SequenceId) -> Vec<(ItemId, &Item)> {
    lanes_of(doc, sequence)
        .into_iter()
        .flat_map(|lane| on_lane(doc, lane).collect::<Vec<_>>())
        .collect()
}

/// Every item of the sequence showing, lane by lane from the bottom.
pub fn items(doc: &Model) -> Vec<(ItemId, &Item)> {
    showing(doc).map_or_else(Vec::new, |sequence| items_of(doc, sequence))
}

/// A file of the bin by its path.
pub fn media_at_path<'a>(doc: &'a Model, path: &str) -> Option<(MediaId, &'a Media)> {
    bin(doc).find(|(_, media)| media.path == path)
}

/// The sequence an item is in.
pub fn sequence_of(doc: &Model, item: &Item) -> Option<SequenceId> {
    lane(doc, item.lane).map(|lane| lane.sequence)
}

/// A lane's row on the panel: rows count from the top, lanes from the
/// bottom. `None` for a lane that is not on the sequence showing.
pub fn row_of(doc: &Model, lane: LaneId) -> Option<usize> {
    let lanes = lanes(doc);
    let at = lanes.iter().position(|id| *id == lane)?;
    Some(lanes.len() - 1 - at)
}

/// The lane on a row of the panel.
pub fn lane_at_row(doc: &Model, row: usize) -> Option<LaneId> {
    let lanes = lanes(doc);
    let at = lanes.len().checked_sub(1)?.checked_sub(row)?;
    lanes.get(at).copied()
}

/// The file an item is cut from, if it is a cut.
pub fn media_id(item: &Item) -> Option<MediaId> {
    match &item.content {
        Content::Media(cut) => Some(cut.media),
        _ => None,
    }
}

/// The file an item is cut from, read.
pub fn media_of<'a>(doc: &'a Model, item: &Item) -> Option<&'a Media> {
    media(doc, media_id(item)?)
}

/// Whether an item has sound of its own: a cut of a file with sound, or a
/// nested sequence.
pub fn has_sound(doc: &Model, item: &Item) -> bool {
    match &item.content {
        Content::Media(cut) => media(doc, cut.media).is_some_and(|media| media.has_sound),
        Content::Nest(_) => true,
        _ => false,
    }
}

// ── seconds and ticks ──

/// Seconds, for a panel to show.
pub fn secs(t: Tick) -> f64 {
    t.display_seconds()
}

/// A distance in seconds.
pub fn dur_secs(d: TickDur) -> f64 {
    d.display_seconds()
}

/// A length in seconds.
pub fn len_secs(l: TickLen) -> f64 {
    l.dur().display_seconds()
}

/// The place nearest some seconds a panel handed in; zero for what is not
/// a number.
pub fn tick(seconds: f64) -> Tick {
    Tick::from_display_seconds(seconds).unwrap_or(Tick::ZERO)
}

/// A distance of some seconds; none for what is not a number.
pub fn dur(seconds: f64) -> TickDur {
    TickDur::from_display_seconds(seconds).unwrap_or(TickDur::ZERO)
}

/// A length of some seconds, at least a tick.
pub fn len(seconds: f64) -> TickLen {
    dur(seconds).to_len().unwrap_or(TickLen::ONE)
}

/// Where an item starts, in seconds.
pub fn start_s(item: &Item) -> f64 {
    secs(item.start)
}

/// How long an item is, in seconds.
pub fn len_s(item: &Item) -> f64 {
    len_secs(item.len)
}

/// Where an item ends, in seconds.
pub fn end_s(item: &Item) -> f64 {
    secs(item.start + item.len)
}

/// Where an item ends.
pub fn end(item: &Item) -> Tick {
    item.start + item.len
}

/// The sequence showing, flattened into what the renderer draws; see
/// [`concat_export::flatten`]. Empty with none showing. With the project
/// folder, a cutout, a stabilization and a voice set apart are told where
/// their files are.
pub fn flattened(
    doc: &Model,
    project_dir: Option<&std::path::Path>,
) -> Vec<concat_export::ExportClip> {
    showing(doc).map_or_else(Vec::new, |sequence| {
        concat_export::flatten::clips(doc, sequence, project_dir)
    })
}

/// The latest end of anything on the sequence showing: how long it runs.
pub fn duration(doc: &Model) -> Tick {
    items(doc)
        .iter()
        .map(|(_, item)| end(item))
        .max()
        .unwrap_or(Tick::ZERO)
}

/// The sequence showing's frames a second; 30 with none.
pub fn rate(doc: &Model) -> f64 {
    sequence(doc).map_or(30.0, |sequence| sequence.frame.rate.display_per_second())
}

/// The sequence showing's frame size; 1920 x 1080 with none.
pub fn frame_size(doc: &Model) -> (u32, u32) {
    sequence(doc).map_or((1920, 1080), |sequence| {
        (sequence.frame.width.get(), sequence.frame.height.get())
    })
}

// ── an item's three times ──

/// The content tick under a timeline tick of an item, held to the window
/// the item shows: where a key set at the playhead goes.
pub fn content_at(item: &Item, t: Tick) -> Tick {
    let into = (t - item.start).get().clamp(0, item.len.get());
    item.offset + TickDur::new(into)
}

/// The content tick under a timeline tick, not held: a place before or
/// after the item is before or after its window.
pub fn content_free(item: &Item, t: Tick) -> Tick {
    item.offset + (t - item.start)
}

/// The timeline tick a content tick of an item shows at.
pub fn timeline_at(item: &Item, c: Tick) -> Tick {
    item.start + (c - item.offset)
}

/// How far into an item a content tick is, 0 to 1 of its length.
pub fn share_of(item: &Item, c: Tick) -> f64 {
    (c - item.offset).get() as f64 / item.len.get() as f64
}

/// The content tick a share of an item's length is at.
pub fn at_share(item: &Item, share: f64) -> Tick {
    let into = (share.clamp(0.0, 1.0) * item.len.get() as f64).round() as i64;
    item.offset + TickDur::new(into)
}

/// The time map of what an item shows, if it has one.
pub fn time_map(item: &Item) -> Option<&TimeMap> {
    match &item.content {
        Content::Media(cut) => Some(&cut.time),
        Content::Nest(nest) => Some(&nest.time),
        _ => None,
    }
}

/// The source tick at a content tick, worked out inside a smooth stretch
/// too: the cubic through the two keys that leaves the first at its
/// speed and reaches the second at its own.
pub fn source_at(map: &TimeMap, c: Tick) -> Option<Tick> {
    if let Some(exact) = map.source_at(c) {
        return Some(exact);
    }
    let TimeMap::Keyed { first, rest, .. } = map else {
        return None;
    };
    let mut from = *first;
    for (curve, to) in rest {
        if c >= from.at && c <= to.at {
            let TimeCurve::Smooth { leave, arrive } = curve else {
                return None;
            };
            let w = (to.at - from.at).get() as f64;
            let u = (c - from.at).get() as f64 / w;
            let (s0, s1) = (from.source.get() as f64, to.source.get() as f64);
            let (m0, m1) = (leave.display() * w, arrive.display() * w);
            let (u2, u3) = (u * u, u * u * u);
            let p = (2.0 * u3 - 3.0 * u2 + 1.0) * s0
                + (u3 - 2.0 * u2 + u) * m0
                + (-2.0 * u3 + 3.0 * u2) * s1
                + (u3 - u2) * m1;
            return Some(Tick::new(p.round() as i64));
        }
        from = *to;
    }
    None
}

/// Where in its source an item's window starts: its in-point. Zero for an
/// item with no source.
pub fn in_point(item: &Item) -> Tick {
    time_map(item)
        .and_then(|map| source_at(map, item.offset))
        .unwrap_or(Tick::ZERO)
}

/// The item's in-point in seconds.
pub fn in_point_s(item: &Item) -> f64 {
    secs(in_point(item))
}

/// The source an item's window covers, from its in-point, signed: below
/// zero when it plays in reverse.
pub fn covered(item: &Item) -> Option<(Tick, TickDur)> {
    let map = time_map(item)?;
    let from = source_at(map, item.offset)?;
    let to = source_at(map, item.offset + item.len)?;
    Some((from, to - from))
}

/// The pace an item plays its source at: its constant speed, or over a
/// speed curve the source it covers over its length. 1 for an item with
/// no source.
pub fn speed(item: &Item) -> f64 {
    match time_map(item) {
        Some(TimeMap::Linear { speed, .. }) => speed.display(),
        Some(TimeMap::Keyed { .. }) => {
            covered(item).map_or(1.0, |(_, span)| span.get() as f64 / item.len.get() as f64)
        }
        _ => 1.0,
    }
}

/// The constant speed of an item, when it has one.
pub fn constant_speed(item: &Item) -> Option<Speed> {
    match time_map(item) {
        Some(TimeMap::Linear { speed, .. }) => Some(*speed),
        _ => None,
    }
}

/// The source second an item shows at a timeline second; the in-point
/// for one with no source.
pub fn source_second(item: &Item, at: f64) -> f64 {
    let c = content_at(item, tick(at));
    time_map(item)
        .and_then(|map| source_at(map, c))
        .map_or_else(|| in_point_s(item), secs)
}

// ── animated numbers ──

/// Every number of an item a key may be on, in the inspector's order.
pub const KEY_PROPERTIES: [KeyProperty; 8] = [
    KeyProperty::Scale,
    KeyProperty::OffsetX,
    KeyProperty::OffsetY,
    KeyProperty::Rotation,
    KeyProperty::StretchX,
    KeyProperty::StretchY,
    KeyProperty::Opacity,
    KeyProperty::Volume,
];

/// How near two keys are before they are one key: the model's own hair.
pub const KEY_HAIR: TickDur = concat_doc::commands::HAIR;

/// One of an item's numbers that a key may be on.
pub fn animated(item: &Item, property: KeyProperty) -> &Animated<f64> {
    let picture = &item.picture;
    match property {
        KeyProperty::Scale => &picture.scale,
        KeyProperty::OffsetX => &picture.offset_x,
        KeyProperty::OffsetY => &picture.offset_y,
        KeyProperty::Rotation => &picture.rotation,
        KeyProperty::StretchX => &picture.stretch_x,
        KeyProperty::StretchY => &picture.stretch_y,
        KeyProperty::Opacity => &picture.opacity,
        KeyProperty::Volume => &item.sound.volume,
    }
}

/// The same, to change: for the echo a gesture previews on.
pub fn animated_mut(item: &mut Item, property: KeyProperty) -> &mut Animated<f64> {
    let picture = &mut item.picture;
    match property {
        KeyProperty::Scale => &mut picture.scale,
        KeyProperty::OffsetX => &mut picture.offset_x,
        KeyProperty::OffsetY => &mut picture.offset_y,
        KeyProperty::Rotation => &mut picture.rotation,
        KeyProperty::StretchX => &mut picture.stretch_x,
        KeyProperty::StretchY => &mut picture.stretch_y,
        KeyProperty::Opacity => &mut picture.opacity,
        KeyProperty::Volume => &mut item.sound.volume,
    }
}

/// Progress along a key's ease at `u` of the way in time: the CSS cubic
/// Bézier from (0, 0) to (1, 1), its time solved for by bisection, which
/// cannot fail on a curve whose times run forward.
pub fn eased(ease: &Ease, u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    let Ease::Bezier { x1, y1, x2, y2 } = *ease else {
        return u;
    };
    let bezier = |a: f64, b: f64, s: f64| {
        let r = 1.0 - s;
        3.0 * r * r * s * a + 3.0 * r * s * s * b + s * s * s
    };
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    let mut s = u;
    for _ in 0..40 {
        let x = bezier(x1, x2, s);
        if (x - u).abs() < 1e-9 {
            break;
        }
        if x < u {
            low = s;
        } else {
            high = s;
        }
        s = (low + high) / 2.0;
    }
    bezier(y1, y2, s)
}

/// What a number of keys is at a content tick: the first key's value
/// before it, the last's after it, and between two the later key's ease
/// from one to the other. `None` with no keys.
pub fn keys_at(keys: &[Key<f64>], at: Tick) -> Option<f64> {
    let first = keys.first()?;
    if at <= first.at {
        return Some(first.value);
    }
    for pair in keys.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if at <= b.at {
            let span = (b.at - a.at).get() as f64;
            let u = if span > 0.0 {
                (at - a.at).get() as f64 / span
            } else {
                1.0
            };
            return Some(a.value + (b.value - a.value) * eased(&b.ease, u));
        }
    }
    keys.last().map(|key| key.value)
}

/// What an animated number is at a content tick: its value, or once it
/// has keys, what they make there.
pub fn value_at(number: &Animated<f64>, at: Tick) -> f64 {
    keys_at(&number.keys, at).unwrap_or(number.value)
}

/// The key within a hair of `at`, if there is one.
pub fn key_near(keys: &[Key<f64>], at: Tick) -> Option<&Key<f64>> {
    keys.iter().find(|key| near(key.at, at))
}

/// The keys either side of `at`, a hair excluded: the one before and the
/// one after.
pub fn keys_around(keys: &[Key<f64>], at: Tick) -> (Option<&Key<f64>>, Option<&Key<f64>>) {
    let before = keys
        .iter()
        .rev()
        .find(|key| key.at < at && !near(key.at, at));
    let after = keys.iter().find(|key| key.at > at && !near(key.at, at));
    (before, after)
}

/// Whether two content ticks are one key's place.
pub fn near(a: Tick, b: Tick) -> bool {
    (a - b).get().abs() <= KEY_HAIR.get()
}

/// Writes a value the way the person means it on the echo: the number's
/// value while it has no keys, and once it rides, the key at `at` - put on
/// if there is none there, with the ease of the key behind it.
pub fn write_at(number: &mut Animated<f64>, value: f64, at: Tick) {
    if number.keys.is_empty() {
        number.value = value;
        return;
    }
    if let Some(key) = number.keys.iter_mut().find(|key| near(key.at, at)) {
        key.value = value;
        return;
    }
    let ease = ease_before(&number.keys, at);
    let place = number.keys.partition_point(|key| key.at < at);
    number.keys.insert(place, Key { at, value, ease });
}

/// The ease a key put on at `at` takes: that of the key it joins behind,
/// so a run of keys laid down keeps one shape. The straight line for the
/// first.
pub fn ease_before(keys: &[Key<f64>], at: Tick) -> Ease {
    keys.iter()
        .rev()
        .find(|key| key.at < at)
        .map_or(Ease::LINEAR, |key| key.ease.clone())
}

// ── titles, shapes and colours ──

/// A title's words and look, if the item is a title.
pub fn title(item: &Item) -> Option<&Title> {
    match &item.content {
        Content::Title(title) => Some(title),
        _ => None,
    }
}

/// The same, to change on the echo.
pub fn title_mut(item: &mut Item) -> Option<&mut Title> {
    match &mut item.content {
        Content::Title(title) => Some(title),
        _ => None,
    }
}

/// A shape's figure and paint, if the item is a shape.
pub fn shape(item: &Item) -> Option<&ShapeStyle> {
    match &item.content {
        Content::Shape(style) => Some(style),
        _ => None,
    }
}

/// The same, to change on the echo.
pub fn shape_mut(item: &mut Item) -> Option<&mut ShapeStyle> {
    match &mut item.content {
        Content::Shape(style) => Some(style),
        _ => None,
    }
}

/// The package a layer runs first, or a transition is: what names it.
pub fn package(item: &Item) -> Option<&str> {
    match &item.content {
        Content::Transition(effect) => Some(effect.package.as_str()),
        Content::Layer => chain(&item.effects).first().map(|e| e.package.as_str()),
        Content::SoundLayer => chain(&item.filters).first().map(|e| e.package.as_str()),
        _ => None,
    }
}

/// A document colour as the panels draw it.
pub fn to_slint(color: Color) -> slint::Color {
    slint::Color::from_argb_u8(color.a, color.r, color.g, color.b)
}

/// A colour from the panels, for the document.
pub fn from_slint(color: slint::Color) -> Color {
    Color {
        r: color.red(),
        g: color.green(),
        b: color.blue(),
        a: color.alpha(),
    }
}

/// A colour as `#rrggbb`, or `#rrggbbaa` when it lets anything through.
pub fn hex(color: Color) -> String {
    if color.a == 255 {
        format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
    } else {
        format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            color.r, color.g, color.b, color.a
        )
    }
}

/// A colour written `#rgb`, `#rrggbb` or `#rrggbbaa`, the `#` optional.
pub fn parse_hex(text: &str) -> Option<Color> {
    let digits = text.trim().trim_start_matches('#');
    let byte = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    match digits.len() {
        3 => {
            let nibble = |at: usize| {
                u8::from_str_radix(digits.get(at..at + 1)?, 16)
                    .ok()
                    .map(|n| n * 17)
            };
            Some(Color::rgb(nibble(0)?, nibble(1)?, nibble(2)?))
        }
        6 => Some(Color::rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Color {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: byte(6)?,
        }),
        _ => None,
    }
}

// ── effects and masks ──

/// The entries of a chain this build read, in order: what the inspector
/// lists and counts its rows by. An entry it could not read is kept by
/// the document and never shown.
pub fn chain(entries: &[Entry<Effect>]) -> Vec<&Effect> {
    entries.iter().filter_map(Entry::known).collect()
}

/// The same, owned: for the whole chain an edit hands back.
pub fn chain_owned(entries: &[Entry<Effect>]) -> Vec<Effect> {
    entries.iter().filter_map(Entry::known).cloned().collect()
}

/// An item's masks that were read, top first.
pub fn masks(entries: &[Entry<Mask>]) -> Vec<&Mask> {
    entries.iter().filter_map(Entry::known).collect()
}

/// An item's masks, owned.
pub fn masks_owned(entries: &[Entry<Mask>]) -> Vec<Mask> {
    entries.iter().filter_map(Entry::known).cloned().collect()
}

/// A knob's own value, without its keys; none when it is at the
/// package's default.
pub fn param(effect: &Effect, key: &str) -> Option<f64> {
    effect.params.get(key).and_then(|param| param.value)
}

/// A knob's own value, or `default`.
pub fn param_or(effect: &Effect, key: &str, default: f64) -> f64 {
    param(effect, key).unwrap_or(default)
}

/// A knob's keys, in order.
pub fn param_keys<'a>(effect: &'a Effect, key: &str) -> &'a [Key<f64>] {
    effect
        .params
        .get(key)
        .map_or(&[][..], |param| param.keys.as_slice())
}

/// Whether a knob rides keys.
pub fn param_keyed(effect: &Effect, key: &str) -> bool {
    !param_keys(effect, key).is_empty()
}

/// What a knob is at a content tick: its keys there, else its value, else
/// `default`.
pub fn param_at(effect: &Effect, key: &str, at: Tick, default: f64) -> f64 {
    keys_at(param_keys(effect, key), at).unwrap_or_else(|| param_or(effect, key, default))
}

/// Sets a knob's own value on the echo, keeping any keys it has.
pub fn set_param(effect: &mut Effect, key: &str, value: f64) {
    effect
        .params
        .entry(key.to_owned())
        .or_insert_with(|| Param::fixed(None))
        .value = Some(value);
}

/// A knob taken back to the package's default, keys and all.
pub fn clear_param(effect: &mut Effect, key: &str) {
    effect.params.remove(key);
}

/// The knobs set on an effect, by their own values: what a grading curve
/// is read from.
pub fn plain_params(effect: &Effect) -> BTreeMap<String, f64> {
    effect
        .params
        .iter()
        .filter_map(|(key, param)| Some((key.clone(), param.value?)))
        .collect()
}

/// Puts plain values back on an effect's knobs, as a grading edit leaves
/// them: a knob not in `values` goes back to its default unless it rides
/// keys.
pub fn set_plain_params(effect: &mut Effect, values: &BTreeMap<String, f64>) {
    effect
        .params
        .retain(|key, param| values.contains_key(key) || !param.keys.is_empty());
    for (key, param) in effect.params.iter_mut() {
        if !values.contains_key(key) {
            param.value = None;
        }
    }
    for (key, value) in values {
        set_param(effect, key, *value);
    }
}

/// A new effect of a package at its defaults, for the echo: the id is
/// the placeholder every command gives one of the document's own.
pub fn new_effect(package: &str) -> Effect {
    Effect::new(concat_doc::commands::placeholder(), package)
}

// ── where things go, for the window's own edits ──

/// The id of an item read out of the document: what is on its lane at
/// its start, there being nothing else there.
pub fn id_of(doc: &Model, item: &Item) -> Option<ItemId> {
    doc.items
        .index()
        .at(item.lane, item.start)
        .filter(|id| self::item(doc, *id).is_some_and(|found| found == item))
}

/// The main picture's lane of the sequence showing: the lowest lane that
/// holds a video or a still, where the main cut is made and the only one
/// whose pictures have a background drawn behind them. The bottom lane
/// when none holds a picture yet.
pub fn picture_lane(doc: &Model) -> Option<LaneId> {
    let lanes = lanes(doc);
    lanes
        .iter()
        .copied()
        .find(|lane| {
            on_lane(doc, *lane)
                .any(|(_, item)| matches!(kind(doc, item), Kind::Video | Kind::Image))
        })
        .or_else(|| lanes.first().copied())
}

/// Where a lane's last item ends; zero for an empty lane.
pub fn lane_end(doc: &Model, lane: LaneId) -> Tick {
    on_lane(doc, lane)
        .map(|(_, item)| end(item))
        .max()
        .unwrap_or(Tick::ZERO)
}

/// Whether `len` from `start` on a lane meets nothing but the items in
/// `except`.
pub fn is_free(doc: &Model, lane: LaneId, start: Tick, len: TickLen, except: &[ItemId]) -> bool {
    doc.items
        .index()
        .overlapping(lane, start, start + len)
        .iter()
        .all(|(id, _, _)| except.contains(id))
}

/// The start nearest `want`, not before zero, where `len` fits on a lane
/// among everything but `except`: `want` itself when it is free, else the
/// end of something there or the start of something less the length,
/// whichever is nearest; a tie goes later.
pub fn nearest_free_start(
    doc: &Model,
    lane: LaneId,
    want: Tick,
    len: TickLen,
    except: &[ItemId],
) -> Tick {
    let want = want.max(Tick::ZERO);
    let mut candidates = vec![want, Tick::ZERO];
    for (id, item) in on_lane(doc, lane) {
        if except.contains(&id) {
            continue;
        }
        candidates.push(end(item));
        if let Some(before) = item.start.checked_sub(len)
            && before >= Tick::ZERO
        {
            candidates.push(before);
        }
    }
    candidates
        .into_iter()
        .filter(|at| is_free(doc, lane, *at, len, except))
        .min_by_key(|at| ((*at - want).get().unsigned_abs(), std::cmp::Reverse(*at)))
        .unwrap_or_else(|| lane_end(doc, lane).max(want))
}

/// The transition items of a sequence left over nothing: no item that is
/// not a transition, on a lane beneath, overlaps any of their span. A
/// valid document has none; a draft a move or a trim made may.
pub fn stranded_transitions(doc: &Model, sequence: SequenceId) -> Vec<ItemId> {
    let all = lanes_of(doc, sequence);
    let mut gone = Vec::new();
    for (at, lane) in all.iter().enumerate() {
        for (id, start, stop) in doc.items.index().transitions_on(*lane) {
            let beneath = all[..at].iter().any(|below| {
                doc.items
                    .index()
                    .overlapping(*below, start, stop)
                    .iter()
                    .any(|(other, _, _)| item(doc, *other).is_some_and(|i| !i.is_transition()))
            });
            if !beneath {
                gone.push(id);
            }
        }
    }
    gone
}

/// A move or a trim made with the transitions it would leave over nothing
/// taken away in the same edit, so it is one step of undo and never a
/// refusal. The items the edit itself moves are never swept: a transition
/// dragged onto nothing is refused, and says so. A command that changes
/// nothing, or that is refused for another reason, is handed back as it
/// was, to say why.
pub fn sweep_strays(
    document: &concat_doc::Document<Model>,
    command: concat_doc::commands::Command,
    moved: &[ItemId],
) -> concat_doc::commands::Command {
    use concat_doc::commands::{self, Command};
    let mut draft = document.begin(command.label(), concat_doc::Origin(0));
    if commands::apply(&mut draft, &command).is_err() {
        return command;
    }
    let mut stray: Vec<ItemId> = Vec::new();
    let sequences: Vec<SequenceId> = draft.sequences.ids().collect();
    for sequence in sequences {
        stray.extend(
            stranded_transitions(&draft, sequence)
                .into_iter()
                .filter(|id| !moved.contains(id)),
        );
    }
    if stray.is_empty() {
        return command;
    }
    Command::Batch {
        commands: vec![
            command,
            Command::RemoveClips {
                items: stray,
                ripple: false,
            },
        ],
    }
}

/// What a command would make of a document, without recording it: what
/// a gesture's echo shows. `None` when the document refuses it.
pub fn preview(
    document: &concat_doc::Document<Model>,
    command: &concat_doc::commands::Command,
) -> Option<Model> {
    let outcome = concat_doc::commands::run(document, command, concat_doc::Origin(0)).ok()?;
    Some(match outcome.commit {
        Some(commit) => Model::clone(commit.document()),
        None => Model::clone(document),
    })
}

// ── figures and words ──

/// The figure a card of the Shapes shelf names, as a shape placed from it
/// starts: its kind, its proportions and its paint. `None` for a name no
/// card has.
pub fn shape_preset(name: &str) -> Option<ShapeStyle> {
    use concat_doc::model::{LineHead, ShapeKind};
    let base = ShapeStyle::default();
    Some(match name {
        "square" => base,
        "rounded" => ShapeStyle {
            corner_radius: 0.3,
            ..base
        },
        "circle" => ShapeStyle {
            kind: ShapeKind::Circle,
            ..base
        },
        "ring" => ShapeStyle {
            kind: ShapeKind::Circle,
            fill: None,
            stroke: Some(Color::WHITE),
            stroke_width: 0.03,
            ..base
        },
        "triangle" => ShapeStyle {
            kind: ShapeKind::Triangle,
            ..base
        },
        "pentagon" => ShapeStyle {
            kind: ShapeKind::Polygon,
            sides: 5,
            ..base
        },
        "polygon" => ShapeStyle {
            kind: ShapeKind::Polygon,
            sides: 6,
            ..base
        },
        "star" => ShapeStyle {
            kind: ShapeKind::Star,
            ..base
        },
        "heart" => ShapeStyle {
            kind: ShapeKind::Heart,
            ..base
        },
        "pill" => ShapeStyle {
            height: base.width / 2.0,
            corner_radius: 1.0,
            ..base
        },
        "parallelogram" => ShapeStyle {
            kind: ShapeKind::Parallelogram,
            ..base
        },
        "trapezoid" => ShapeStyle {
            kind: ShapeKind::Trapezoid,
            ..base
        },
        "line" => ShapeStyle {
            kind: ShapeKind::Line,
            ..base
        },
        "arrow" => ShapeStyle {
            kind: ShapeKind::Arrow,
            ..base
        },
        "double-arrow" => ShapeStyle {
            kind: ShapeKind::Arrow,
            start_head: Some(LineHead::Arrow),
            ..base
        },
        _ => return None,
    })
}

/// How many characters a title's words are, as its runs count them.
pub fn char_count(title: &Title) -> usize {
    title.content.chars().count()
}

/// A title's runs with `[start, end)` given `style` over what they had, or
/// with `None` given back to the title's own look: what the document's
/// own styling of some words makes, for the echo to show before the edit
/// lands. Pieces left plain are dropped and pieces that match their
/// neighbour join it.
pub fn restyle_runs(
    runs: &[concat_doc::model::TextRun],
    start: u32,
    end: u32,
    style: Option<&concat_doc::commands::RunStyle>,
) -> Vec<concat_doc::model::TextRun> {
    use concat_doc::model::TextRun;
    let mut edges: Vec<u32> = vec![start, end];
    for run in runs {
        edges.push(run.start);
        edges.push(run.end);
    }
    edges.sort_unstable();
    edges.dedup();
    let bare = |run: &TextRun| TextRun {
        start: 0,
        end: 0,
        ..run.clone()
    };
    let mut out: Vec<TextRun> = Vec::new();
    for pair in edges.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let mut piece = runs
            .iter()
            .find(|run| run.start <= a && b <= run.end)
            .cloned()
            .unwrap_or_default();
        piece.start = a;
        piece.end = b;
        if a >= start && b <= end {
            match style {
                Some(s) => {
                    piece.color = s.color.or(piece.color);
                    piece.font_scale = s.font_scale.or(piece.font_scale);
                    piece.font_weight = s.font_weight.or(piece.font_weight);
                    piece.italic = s.italic.or(piece.italic);
                    piece.underline = s.underline.or(piece.underline);
                    piece.strikethrough = s.strikethrough.or(piece.strikethrough);
                    piece.stroke_color = s.stroke_color.or(piece.stroke_color);
                }
                None => {
                    piece = TextRun {
                        start: a,
                        end: b,
                        ..TextRun::default()
                    }
                }
            }
        }
        if bare(&piece) == TextRun::default() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.end == a && bare(last) == bare(&piece) => last.end = b,
            _ => out.push(piece),
        }
    }
    out
}

/// Puts a key on a run of keys at `at`, replacing the one within a hair
/// of it, keeping them in order.
pub fn set_key(keys: &mut Vec<Key<f64>>, at: Tick, value: f64, ease: Ease) {
    if let Some(key) = keys.iter_mut().find(|key| near(key.at, at)) {
        key.value = value;
        key.ease = ease;
        return;
    }
    let place = keys.partition_point(|key| key.at < at);
    keys.insert(place, Key { at, value, ease });
}

/// Puts a key on one knob of an effect, as [`set_key`] does.
pub fn set_param_key(effect: &mut Effect, key: &str, at: Tick, value: f64, ease: Ease) {
    let param = effect
        .params
        .entry(key.to_owned())
        .or_insert_with(|| Param::fixed(None));
    set_key(&mut param.keys, at, value, ease);
}

/// Every key's place on an effect's knobs, in no order.
pub fn param_key_places(effect: &Effect) -> impl Iterator<Item = Tick> + '_ {
    effect
        .params
        .values()
        .flat_map(|param| param.keys.iter().map(|key| key.at))
}

/// An ease from its four numbers as a curve editor hands them: the
/// times held from 0 to 1, what is not a number the straight line's.
pub fn ease_of([x1, y1, x2, y2]: [f64; 4]) -> Ease {
    let time = |x: f64, fallback: f64| {
        if x.is_finite() {
            x.clamp(0.0, 1.0)
        } else {
            fallback
        }
    };
    let value = |y: f64, fallback: f64| if y.is_finite() { y } else { fallback };
    Ease::Bezier {
        x1: time(x1, 0.0),
        y1: value(y1, 0.0),
        x2: time(x2, 1.0),
        y2: value(y2, 1.0),
    }
}

/// An ease's four numbers; the straight line's for one this build does
/// not know.
pub fn ease_parts(ease: &Ease) -> [f64; 4] {
    match *ease {
        Ease::Bezier { x1, y1, x2, y2 } => [x1, y1, x2, y2],
        Ease::Unknown(_) => [0.0, 0.0, 1.0, 1.0],
    }
}

/// Every number of a mask a key may be on, in the order of the mask's
/// numbers.
pub const MASK_NUMBERS: [MaskNumber; 10] = [
    MaskNumber::X,
    MaskNumber::Y,
    MaskNumber::Scale,
    MaskNumber::Rotation,
    MaskNumber::Feather,
    MaskNumber::Expand,
    MaskNumber::Opacity,
    MaskNumber::Width,
    MaskNumber::Height,
    MaskNumber::Roundness,
];

/// One number of a mask; none for a box number of a mask that is not a
/// shape.
pub fn mask_number(mask: &Mask, number: MaskNumber) -> Option<&Animated<f64>> {
    Some(match number {
        MaskNumber::X => &mask.x,
        MaskNumber::Y => &mask.y,
        MaskNumber::Scale => &mask.scale,
        MaskNumber::Rotation => &mask.rotation,
        MaskNumber::Feather => &mask.feather,
        MaskNumber::Expand => &mask.expand,
        MaskNumber::Opacity => &mask.opacity,
        MaskNumber::Width | MaskNumber::Height | MaskNumber::Roundness => match &mask.source {
            MaskSource::Shape {
                width,
                height,
                roundness,
                ..
            } => match number {
                MaskNumber::Width => width,
                MaskNumber::Height => height,
                _ => roundness,
            },
            MaskSource::Unknown(_) => return None,
        },
    })
}

/// The same, to change on the echo.
pub fn mask_number_mut(mask: &mut Mask, number: MaskNumber) -> Option<&mut Animated<f64>> {
    Some(match number {
        MaskNumber::X => &mut mask.x,
        MaskNumber::Y => &mut mask.y,
        MaskNumber::Scale => &mut mask.scale,
        MaskNumber::Rotation => &mut mask.rotation,
        MaskNumber::Feather => &mut mask.feather,
        MaskNumber::Expand => &mut mask.expand,
        MaskNumber::Opacity => &mut mask.opacity,
        MaskNumber::Width | MaskNumber::Height | MaskNumber::Roundness => match &mut mask.source {
            MaskSource::Shape {
                width,
                height,
                roundness,
                ..
            } => match number {
                MaskNumber::Width => width,
                MaskNumber::Height => height,
                _ => roundness,
            },
            MaskSource::Unknown(_) => return None,
        },
    })
}

/// A mask's number at a content tick; zero for one it does not have.
pub fn mask_value_at(mask: &Mask, number: MaskNumber, at: Tick) -> f64 {
    mask_number(mask, number).map_or(0.0, |number| value_at(number, at))
}

/// Whether any of a mask's numbers rides keys.
pub fn mask_keyed(mask: &Mask) -> bool {
    MASK_NUMBERS
        .iter()
        .any(|number| mask_number(mask, *number).is_some_and(Animated::is_keyed))
}

/// A mask number held to what the panels let it reach.
pub fn mask_clamp(number: MaskNumber, value: f64) -> f64 {
    match number {
        MaskNumber::X | MaskNumber::Y => value.clamp(-1.0, 1.0),
        MaskNumber::Width | MaskNumber::Height => value.clamp(0.01, 4.0),
        MaskNumber::Roundness | MaskNumber::Feather | MaskNumber::Opacity => value.clamp(0.0, 1.0),
        MaskNumber::Expand => value.clamp(-0.5, 0.5),
        MaskNumber::Scale => value.max(0.01),
        MaskNumber::Rotation => value,
    }
}

/// The shape a mask is drawn as, as the compositor knows it; none for a
/// source or a shape this build does not know.
pub fn mask_shape(mask: &Mask) -> Option<concat_core::mask::MaskShape> {
    match &mask.source {
        MaskSource::Shape { shape, .. } => engine_mask_shape(shape),
        MaskSource::Unknown(_) => None,
    }
}

/// A document mask shape as the compositor knows it.
pub fn engine_mask_shape(shape: &MaskShape) -> Option<concat_core::mask::MaskShape> {
    use concat_core::mask::MaskShape as Engine;
    Some(match shape {
        MaskShape::Linear => Engine::Linear,
        MaskShape::Mirror => Engine::Mirror,
        MaskShape::Circle => Engine::Circle,
        MaskShape::Rectangle => Engine::Rectangle,
        MaskShape::Heart => Engine::Heart,
        MaskShape::Star => Engine::Star,
        MaskShape::Unknown(_) => return None,
    })
}

/// The compositor's mask shape as the document holds it.
pub fn doc_mask_shape(shape: concat_core::mask::MaskShape) -> MaskShape {
    use concat_core::mask::MaskShape as Engine;
    match shape {
        Engine::Linear => MaskShape::Linear,
        Engine::Mirror => MaskShape::Mirror,
        Engine::Circle => MaskShape::Circle,
        Engine::Rectangle => MaskShape::Rectangle,
        Engine::Heart => MaskShape::Heart,
        Engine::Star => MaskShape::Star,
    }
}

/// A document mask mode as the compositor knows it.
pub fn engine_mask_mode(mode: &MaskMode) -> Option<concat_core::mask::MaskMode> {
    use concat_core::mask::MaskMode as Engine;
    Some(match mode {
        MaskMode::Add => Engine::Add,
        MaskMode::Subtract => Engine::Subtract,
        MaskMode::Intersect => Engine::Intersect,
        MaskMode::None => Engine::None,
        MaskMode::Unknown(_) => return None,
    })
}

/// The compositor's mask mode as the document holds it.
pub fn doc_mask_mode(mode: concat_core::mask::MaskMode) -> MaskMode {
    use concat_core::mask::MaskMode as Engine;
    match mode {
        Engine::Add => MaskMode::Add,
        Engine::Subtract => MaskMode::Subtract,
        Engine::Intersect => MaskMode::Intersect,
        Engine::None => MaskMode::None,
    }
}

/// A new mask of a shape, at the shape's own size, in the middle, adding,
/// at full strength with a hard edge. A line and a band are as wide as
/// any picture; a box is wider than it is tall.
pub fn mask_of(id: MaskId, shape: concat_core::mask::MaskShape) -> Mask {
    use concat_core::mask::MaskShape as Engine;
    let (width, height) = match shape {
        Engine::Linear => (0.66, 0.0),
        Engine::Mirror => (0.66, 0.25),
        Engine::Rectangle => (0.66, 0.44),
        Engine::Circle | Engine::Heart | Engine::Star => (0.66, 0.66),
    };
    let mut mask = Mask::circle(id);
    mask.source = MaskSource::Shape {
        shape: doc_mask_shape(shape),
        width: Animated::fixed(width),
        height: Animated::fixed(height),
        roundness: Animated::fixed(0.0),
    };
    mask
}

// ── what the inspector offers ──

/// A cutout's edge softness as it starts, a share of the picture's width.
pub const CUTOUT_FEATHER: f64 = 0.01;

/// The softest edge the inspector offers a cutout.
pub const CUTOUT_FEATHER_MAX: f64 = 0.1;

/// The most a stabilized picture may be zoomed to hide its edges.
pub const STABILIZE_ZOOM_MAX: f64 = 1.5;

/// The loudest the voice, or what is set apart from it, may be made.
pub const VOICE_GAIN_MAX: f64 = 2.0;

/// The background taken away by the model alone, looking for whatever
/// the picture is of, with the edge as it starts.
pub fn cutout_auto() -> Cutout {
    Cutout {
        mode: CutoutMode::Auto,
        subject: Subject::Auto,
        feather: CUTOUT_FEATHER,
        strokes: Vec::new(),
    }
}

/// Stabilization as it is turned on: half the shake smoothed, the zoom
/// allowed to a fifth larger, the roll kept out, no tripod.
pub fn stabilize_default() -> Stabilize {
    Stabilize {
        smoothness: 0.5,
        max_zoom: 1.2,
        keep_rotation: false,
        tripod: false,
    }
}

/// The voice alone of a sound of this kind.
pub fn keep_voice(kind: VoiceKind) -> VoiceMix {
    VoiceMix {
        kind,
        voice: 1.0,
        background: 0.0,
    }
}

/// Everything but the voice.
pub fn remove_voice(kind: VoiceKind) -> VoiceMix {
    VoiceMix {
        kind,
        voice: 0.0,
        background: 1.0,
    }
}

/// The figures a shape can be, in the order the Shape section lists them.
pub const SHAPE_KINDS: [ShapeKind; 10] = [
    ShapeKind::Square,
    ShapeKind::Circle,
    ShapeKind::Triangle,
    ShapeKind::Parallelogram,
    ShapeKind::Trapezoid,
    ShapeKind::Line,
    ShapeKind::Arrow,
    ShapeKind::Polygon,
    ShapeKind::Star,
    ShapeKind::Heart,
];

/// What a line's end can wear, in the order the Shape section lists them.
pub const LINE_HEADS: [LineHead; 5] = [
    LineHead::None,
    LineHead::Arrow,
    LineHead::Triangle,
    LineHead::Circle,
    LineHead::Bar,
];

/// Whether a figure is a stroke from one end to the other.
pub fn is_line(kind: &ShapeKind) -> bool {
    matches!(kind, ShapeKind::Line | ShapeKind::Arrow)
}

/// What a line's start wears: its own, or the kind's, which is none.
pub fn start_head(style: &ShapeStyle) -> LineHead {
    style.start_head.clone().unwrap_or(LineHead::None)
}

/// What a line's end wears: its own, or the kind's - an arrow's head on
/// an arrow, none on a line.
pub fn end_head(style: &ShapeStyle) -> LineHead {
    style.end_head.clone().unwrap_or(match style.kind {
        ShapeKind::Arrow => LineHead::Arrow,
        _ => LineHead::None,
    })
}

/// A shape's box, width and height in shares of the frame's height: its
/// own height, or with none the figure's own proportion of its width - a
/// line none, the four-sided figures that lean or narrow a little under
/// their width, the rest as tall as wide.
pub fn figure_box(style: &ShapeStyle) -> (f64, f64) {
    if style.height > 0.0 {
        return (style.width, style.height);
    }
    let height = match style.kind {
        ShapeKind::Line | ShapeKind::Arrow => 0.0,
        ShapeKind::Triangle => style.width * 3f64.sqrt() / 2.0,
        ShapeKind::Parallelogram | ShapeKind::Trapezoid => style.width * 0.6,
        _ => style.width,
    };
    (style.width, height)
}

/// A document blend as the compositor knows it; normal for one this build
/// does not know.
pub fn engine_blend(blend: &Blend) -> concat_core::Blend {
    use concat_core::Blend as Engine;
    match blend {
        Blend::Normal | Blend::Unknown(_) => Engine::Normal,
        Blend::Multiply => Engine::Multiply,
        Blend::Screen => Engine::Screen,
        Blend::Add => Engine::Add,
        Blend::Lighten => Engine::Lighten,
        Blend::Darken => Engine::Darken,
    }
}

/// The compositor's blend as the document holds it.
pub fn doc_blend(blend: concat_core::Blend) -> Blend {
    use concat_core::Blend as Engine;
    match blend {
        Engine::Normal => Blend::Normal,
        Engine::Multiply => Blend::Multiply,
        Engine::Screen => Blend::Screen,
        Engine::Add => Blend::Add,
        Engine::Lighten => Blend::Lighten,
        Engine::Darken => Blend::Darken,
    }
}

/// The id a mask made on the echo holds until a command gives it one of
/// the document's own; see [`concat_doc::commands::placeholder`].
pub fn new_mask_id() -> MaskId {
    concat_doc::commands::placeholder()
}

/// A document to run the window's commands on in a test, without a
/// session or a folder: one timeline of four lanes to start.
#[cfg(test)]
pub mod bench {
    use concat_doc::commands::{self, Applied, Command};
    use concat_doc::model::{ItemId, LaneId, Media, MediaId, MediaKind, Model, Size};
    use concat_doc::{Document, Origin};

    /// The document, edited one command at a time.
    pub struct Bench {
        pub document: Document<Model>,
    }

    impl Bench {
        /// One timeline, four lanes, nothing in the bin.
        pub fn new() -> Bench {
            let mut bench = Bench {
                document: Document::new(),
            };
            bench.apply(&Command::AddTimeline);
            bench
        }

        /// The document as it stands.
        pub fn doc(&self) -> &Model {
            &self.document
        }

        /// Makes a command, which must be taken.
        pub fn apply(&mut self, command: &Command) -> Applied {
            self.try_apply(command).expect("the command is taken")
        }

        /// Makes a command, or says why it was refused.
        pub fn try_apply(&mut self, command: &Command) -> Result<Applied, String> {
            let outcome = commands::run(&self.document, command, Origin(0))
                .map_err(|error| error.to_string())?;
            if let Some(commit) = outcome.commit {
                self.document = commit.into_document();
            }
            Ok(outcome.applied)
        }

        /// The lane on a row of the panel, the top row 0.
        pub fn lane(&self, row: usize) -> LaneId {
            super::lane_at_row(self.doc(), row).expect("there is a lane on that row")
        }

        /// The main lane: the bottom one.
        pub fn main_lane(&self) -> LaneId {
            super::lanes(self.doc())[0]
        }

        /// A file of a kind and a length into the bin.
        pub fn add_media(&mut self, name: &str, kind: MediaKind, seconds: f64) -> MediaId {
            let has_sound = kind != MediaKind::Image;
            let length = (kind != MediaKind::Image).then(|| super::len(seconds));
            let media = Media {
                path: format!("/media/{name}"),
                name: name.to_owned(),
                size: (kind != MediaKind::Audio)
                    .then(|| Size::new(1920, 1080))
                    .flatten(),
                kind,
                length,
                has_sound,
                ..Media::default()
            };
            let applied = self.apply(&Command::AddMedia { media });
            applied
                .created
                .and_then(|id| id.typed())
                .expect("the file is added")
        }

        /// A clip of a file on a lane at a second.
        pub fn add_clip(&mut self, media: MediaId, lane: LaneId, start: f64) -> ItemId {
            let applied = self.apply(&Command::AddClip {
                media,
                lane,
                start: super::tick(start),
                ripple: false,
            });
            applied
                .created
                .and_then(|id| id.typed())
                .expect("the clip is placed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_come_back_as_the_ticks_they_name() {
        assert_eq!(secs(tick(1.5)), 1.5);
        assert_eq!(len_secs(len(2.25)), 2.25);
        assert_eq!(tick(f64::NAN), Tick::ZERO);
        assert_eq!(len(0.0), TickLen::ONE);
    }

    #[test]
    fn a_linear_ease_is_the_straight_line() {
        for u in [0.0, 0.25, 0.5, 0.9, 1.0] {
            assert!((eased(&Ease::LINEAR, u) - u).abs() < 1e-6);
        }
    }

    #[test]
    fn keys_hold_before_the_first_and_after_the_last() {
        let keys = vec![Key::linear(tick(1.0), 2.0), Key::linear(tick(3.0), 4.0)];
        assert_eq!(keys_at(&keys, tick(0.0)), Some(2.0));
        assert_eq!(keys_at(&keys, tick(5.0)), Some(4.0));
        assert!((keys_at(&keys, tick(2.0)).unwrap() - 3.0).abs() < 1e-9);
        assert_eq!(keys_at(&[], tick(2.0)), None);
    }

    #[test]
    fn writing_a_keyed_number_puts_a_key_where_the_playhead_is() {
        let mut number = Animated::fixed(1.0);
        write_at(&mut number, 2.0, tick(1.0));
        assert_eq!(number.value, 2.0);
        assert!(number.keys.is_empty());
        number.keys.push(Key::linear(tick(0.0), 1.0));
        write_at(&mut number, 3.0, tick(2.0));
        assert_eq!(number.keys.len(), 2);
        write_at(&mut number, 5.0, tick(2.0));
        assert_eq!(number.keys.len(), 2);
        assert_eq!(number.keys[1].value, 5.0);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How each value of the model is spelled in a file, read and written
//! side by side so the two cannot drift apart.
//!
//! The rules every value follows:
//!
//! - **A field at its default is left out**, and a field left out reads
//!   as its default. The defaults are the model's own: an item as
//!   [`Item::new`] makes it, a picture as [`Picture::default`] has it.
//!   A field with no default - an item's lane, an effect's package - is
//!   always written and must be there.
//! - **A choice is a string or an object of one key**: `"layer"`,
//!   `{"blur": 0.5}`. A name this build does not know, or a value of
//!   another shape, is kept as its text in the choice's `Unknown` arm;
//!   a name it knows with a value it cannot read is an error.
//! - **Numbers**: ticks and counts are whole numbers; everything else is
//!   written in the fewest digits that read back as the same number.
//!   A rate or a speed is `{"num": 30000, "den": 1001}`, an id is its
//!   string (`"c12"`), a colour is `"#rrggbb"` or `"#rrggbbaa"`.
//! - **A record** - a row of a table, an effect, a mask, a marker - may
//!   say what it `needs`, and may carry what is `derived` from it; see
//!   [the module](super).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::num::NonZeroU32;

use crate::id::TypedId;
use crate::model::{
    Animated, Background, BeatMarks, Blend, BrushTool, CaptionLook, Color, ColorRange, ColorSpace,
    Content, Cover, Crop, CutIn, Cutout, CutoutMode, Derived, Ease, Effect, EffectId, Enhanced,
    Entry, Font, Frame, Gradient, HeldId, Item, ItemId, Key, Lane, LaneId, LineHead, Link,
    LinkKind, Loop, Marker, MarkerId, Mask, MaskId, MaskMode, MaskShape, MaskSource, Media,
    MediaCut, MediaId, MediaKind, MediaOrigin, Motion, Move, Nest, OrderKey, Param, Picture, Raw,
    Residual, Sequence, SequenceId, ShapeKind, ShapeShadow, ShapeStyle, Size, Sound, SoundStream,
    Speed, Stabilize, Stroke, StrokeAlign, StrokeDash, Subject, Take, TextAlign, TextCase, TextRun,
    TimeCurve, TimeMap, TimePoint, Title, VoiceKind, VoiceMix,
};
use crate::time::{Rate, Tick, TickDur, TickLen};

use super::input::{Cx, R, array, choice, fields, need, record, wrong};
use super::json::{Kind, Node};
use super::out::{Obj, Out};

/// A value as a file spells it.
pub(crate) trait Form: Sized {
    /// Writes it.
    fn put<'d>(&'d self, out: &mut Out<'d>);

    /// Reads it. Fields of an object this build does not know go to the
    /// reading's `fields`, by their path from this value.
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self>;

    /// Its name as an element of a list, for the path of a field this
    /// build does not know inside it: its place, unless it has a better.
    fn step(&self, index: usize) -> String {
        index.to_string()
    }
}

/// A field written where it differs from its default, or holds fields
/// this build does not know.
pub(crate) fn field<'d, T: Form + PartialEq>(
    out: &mut Out<'d>,
    obj: &mut Obj,
    key: &str,
    value: &'d T,
    default: &T,
) {
    if value != default || out.holds_under(key) {
        out.part(obj, key, |out| value.put(out));
    }
}

/// A field always written.
pub(crate) fn always<'d, T: Form>(out: &mut Out<'d>, obj: &mut Obj, key: &str, value: &'d T) {
    out.part(obj, key, |out| value.put(out));
}

/// Works out the seal of a record just read, if it carries what is
/// derived from it, and keeps that with the seal.
pub(crate) fn seal<T: Form>(
    record: &mut T,
    derived: Option<&Node<'_>>,
    cx: &mut Cx,
    residual: impl Fn(&mut T) -> &mut Residual,
) {
    let Some(derived) = derived else {
        return;
    };
    let seal = {
        let mut out = Out::new(256);
        record.put(&mut out);
        out.seal
    };
    residual(record).derived = Some(Derived {
        raw: cx.raw(derived),
        seal,
    });
}

/// Reads one value, refused with its field's name.
fn get_at<T: Form>(node: &Node<'_>, cx: &mut Cx, name: &str) -> R<T> {
    T::get(node, cx).map_err(|bad| bad.at(name))
}

// --- Plain values ----------------------------------------------------------

fn whole(node: &Node<'_>) -> R<i64> {
    match node.kind {
        Kind::Number => match node.text.parse::<i64>() {
            Ok(value) => Ok(value),
            Err(_) => wrong("is not a whole number that fits"),
        },
        _ => wrong("is not a number"),
    }
}

impl Form for Tick {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        out.int(self.get());
    }
    fn get(node: &Node<'_>, _: &mut Cx) -> R<Self> {
        whole(node).map(Tick::new)
    }
}

impl Form for TickDur {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        out.int(self.get());
    }
    fn get(node: &Node<'_>, _: &mut Cx) -> R<Self> {
        whole(node).map(TickDur::new)
    }
}

impl Form for TickLen {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        out.int(self.get());
    }
    fn get(node: &Node<'_>, _: &mut Cx) -> R<Self> {
        match TickLen::new(whole(node)?) {
            Some(len) => Ok(len),
            None => wrong("is not a length above zero"),
        }
    }
}

impl Form for u32 {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        out.int(*self);
    }
    fn get(node: &Node<'_>, _: &mut Cx) -> R<Self> {
        match u32::try_from(whole(node)?) {
            Ok(value) => Ok(value),
            Err(_) => wrong("is not a count from 0 to 4294967295"),
        }
    }
}

impl Form for i32 {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        out.int(*self);
    }
    fn get(node: &Node<'_>, _: &mut Cx) -> R<Self> {
        match i32::try_from(whole(node)?) {
            Ok(value) => Ok(value),
            Err(_) => wrong("is too large"),
        }
    }
}

impl Form for NonZeroU32 {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        out.int(self.get());
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        match NonZeroU32::new(u32::get(node, cx)?) {
            Some(value) => Ok(value),
            None => wrong("is zero"),
        }
    }
}

impl Form for f64 {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        out.float(*self);
    }
    fn get(node: &Node<'_>, _: &mut Cx) -> R<Self> {
        let Kind::Number = node.kind else {
            return wrong("is not a number");
        };
        match node.text.parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(value),
            _ => wrong("is too large to hold"),
        }
    }
}

impl Form for bool {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        out.bool(*self);
    }
    fn get(node: &Node<'_>, _: &mut Cx) -> R<Self> {
        match node.kind {
            Kind::Bool(value) => Ok(value),
            _ => wrong("is not true or false"),
        }
    }
}

impl Form for String {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        out.string(self);
    }
    fn get(node: &Node<'_>, _: &mut Cx) -> R<Self> {
        match node.str() {
            Some(text) => Ok(text.to_owned()),
            None => wrong("is not a string"),
        }
    }
}

/// An id, written straight into the file: its spelling needs no escape.
fn put_id(out: &mut Out<'_>, id: impl std::fmt::Display) {
    out.text.push('"');
    let _ = write!(out.text, "{id}");
    out.text.push('"');
}

/// An id of one kind, read from its string.
pub(crate) fn get_id<I: TypedId + std::str::FromStr<Err = crate::id::IdError>>(
    node: &Node<'_>,
) -> R<I> {
    match node.str().map(str::parse::<I>) {
        Some(Ok(id)) => Ok(id),
        Some(Err(error)) => wrong(format!("is not an id such as \"{}12\" ({error})", I::TAG)),
        None => wrong(format!("is not an id such as \"{}12\"", I::TAG)),
    }
}

macro_rules! ids {
    ($($id:ty),*) => {$(
        impl Form for $id {
            fn put<'d>(&'d self, out: &mut Out<'d>) {
                put_id(out, self);
            }
            fn get(node: &Node<'_>, _: &mut Cx) -> R<Self> {
                get_id(node)
            }
        }
    )*};
}

ids!(
    MediaId, SequenceId, LaneId, ItemId, EffectId, MaskId, MarkerId, HeldId
);

impl Form for Rate {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        put_fraction(out, i64::from(self.num()), self.den());
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        let (num, den) = get_fraction::<u32>(node, cx)?;
        match Rate::new(num, den) {
            Ok(rate) => Ok(rate),
            Err(error) => wrong(error.to_string()),
        }
    }
}

impl Form for Speed {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        put_fraction(out, i64::from(self.num()), self.den());
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        let (num, den) = get_fraction::<i32>(node, cx)?;
        match Speed::new(num, den) {
            Some(speed) => Ok(speed),
            None => wrong("has a zero in it"),
        }
    }
}

fn put_fraction(out: &mut Out<'_>, num: i64, den: u32) {
    let mut obj = out.open();
    out.key(&mut obj, "num");
    out.int(num);
    out.key(&mut obj, "den");
    out.int(den);
    out.close(obj);
}

fn get_fraction<T: Form>(node: &Node<'_>, cx: &mut Cx) -> R<(T, u32)> {
    let (mut num, mut den) = (None, None);
    fields(node, cx, |key, node, cx| {
        match key {
            "num" => num = Some(T::get(node, cx)?),
            "den" => den = Some(u32::get(node, cx)?),
            _ => return Ok(false),
        }
        Ok(true)
    })?;
    Ok((need(num, "num")?, need(den, "den")?))
}

impl Form for Color {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        let Color { r, g, b, a } = *self;
        let _ = if a == 255 {
            write!(out.text, "\"#{r:02x}{g:02x}{b:02x}\"")
        } else {
            write!(out.text, "\"#{r:02x}{g:02x}{b:02x}{a:02x}\"")
        };
    }
    fn get(node: &Node<'_>, _: &mut Cx) -> R<Self> {
        let hex = node.str().and_then(|text| text.strip_prefix('#'));
        let Some(hex) = hex.filter(|h| (h.len() == 6 || h.len() == 8) && h.is_ascii()) else {
            return wrong("is not a colour such as \"#ff8800\"");
        };
        let byte = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
        let alpha = if hex.len() == 8 { byte(6) } else { Some(255) };
        match (byte(0), byte(2), byte(4), alpha) {
            (Some(r), Some(g), Some(b), Some(a)) => Ok(Color { r, g, b, a }),
            _ => wrong("is not a colour such as \"#ff8800\""),
        }
    }
}

impl Form for OrderKey {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        out.string(self.as_str());
    }
    fn get(node: &Node<'_>, _: &mut Cx) -> R<Self> {
        match node.str().map(OrderKey::parse) {
            Some(Ok(key)) => Ok(key),
            Some(Err(error)) => wrong(error.to_string()),
            None => wrong("is not an order key"),
        }
    }
}

impl<T: Form> Form for Option<T> {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        match self {
            Some(value) => value.put(out),
            None => out.null(),
        }
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        match node.kind {
            Kind::Null => Ok(None),
            _ => T::get(node, cx).map(Some),
        }
    }
}

/// A list: each element named by its step, then text set aside from it.
pub(crate) fn put_items<'d, T: Form>(out: &mut Out<'d>, items: &'d [T], after: &[&Raw]) {
    out.text.push('[');
    for (at, item) in items.iter().enumerate() {
        if at > 0 {
            out.text.push(',');
        }
        if out.tracking() {
            out.within(&item.step(at), |out| item.put(out));
        } else {
            item.put(out);
        }
    }
    for (at, raw) in after.iter().enumerate() {
        if at > 0 || !items.is_empty() {
            out.text.push(',');
        }
        out.raw(raw.as_str());
    }
    out.text.push(']');
}

impl<T: Form> Form for Vec<T> {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        put_items(out, self, &[]);
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        let items = array(node)?;
        let mut list = Vec::with_capacity(items.len());
        for (at, item) in items.iter().enumerate() {
            let from = cx.fields.len();
            let value = T::get(item, cx).map_err(|bad| bad.at(&(at + 1).to_string()))?;
            if cx.fields.len() > from {
                cx.under(from, &value.step(at));
            }
            list.push(value);
        }
        Ok(list)
    }
}

impl Form for [f64; 2] {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        out.list(self, |out, value| out.float(*value));
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        match array(node)? {
            [x, y] => Ok([f64::get(x, cx)?, f64::get(y, cx)?]),
            _ => wrong("is not two numbers"),
        }
    }
}

/// An effect's knobs, by name, in the order of their names.
impl Form for BTreeMap<String, Param> {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        let mut obj = out.open();
        for (name, param) in self {
            always(out, &mut obj, name, param);
        }
        out.close(obj);
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        let mut params = BTreeMap::new();
        fields(node, cx, |key, node, cx| {
            params.insert(key.to_owned(), Param::get(node, cx)?);
            Ok(true)
        })?;
        Ok(params)
    }
}

// --- Choices ----------------------------------------------------------------

/// A choice of names only: each name is a string, and any other value
/// is kept in the `Unknown` arm.
macro_rules! names {
    ($ty:ident { $($variant:ident = $name:literal),* $(,)? }) => {
        impl Form for $ty {
            fn put<'d>(&'d self, out: &mut Out<'d>) {
                match self {
                    $( $ty::$variant => out.string($name), )*
                    $ty::Unknown(raw) => out.raw(raw.as_str()),
                }
            }
            fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
                Ok(match node.str() {
                    $( Some($name) => $ty::$variant, )*
                    _ => $ty::Unknown(cx.raw(node)),
                })
            }
        }
    };
}

names!(MediaKind { Video = "video", Audio = "audio", Image = "image" });
names!(ColorSpace { Sdr = "sdr", Hlg = "hlg", Pq = "pq" });
names!(ColorRange { Limited = "limited", Full = "full" });
names!(BeatMarks { Zoom = "zoom", Every = "every", Second = "second", Bar = "bar" });
names!(MediaOrigin { Speech = "speech", Processed = "processed" });
names!(Take { Whole = "whole", Sound = "sound" });
names!(Blend {
    Normal = "normal",
    Multiply = "multiply",
    Screen = "screen",
    Add = "add",
    Lighten = "lighten",
    Darken = "darken",
});
names!(CutoutMode { Auto = "auto", Custom = "custom" });
names!(Subject { Auto = "auto", Person = "person", Object = "object" });
names!(BrushTool {
    SmartBrush = "smart_brush",
    Brush = "brush",
    SmartEraser = "smart_eraser",
    Eraser = "eraser",
});
names!(VoiceKind { Vocals = "vocals", Speech = "speech" });
names!(LinkKind { SoundOf = "sound_of" });
names!(MaskMode { Add = "add", Subtract = "subtract", Intersect = "intersect", None = "none" });
names!(MaskShape {
    Linear = "linear",
    Mirror = "mirror",
    Circle = "circle",
    Rectangle = "rectangle",
    Heart = "heart",
    Star = "star",
});
names!(TextAlign { Left = "left", Center = "center", Right = "right" });
names!(TextCase { Upper = "upper", Lower = "lower", Sentence = "sentence", Original = "original" });
names!(ShapeKind {
    Square = "square",
    Circle = "circle",
    Triangle = "triangle",
    Parallelogram = "parallelogram",
    Trapezoid = "trapezoid",
    Line = "line",
    Arrow = "arrow",
    Polygon = "polygon",
    Star = "star",
    Heart = "heart",
});
names!(StrokeAlign { Center = "center", Inside = "inside", Outside = "outside" });
names!(StrokeDash { Solid = "solid", Dashed = "dashed", Dotted = "dotted" });
names!(LineHead {
    None = "none",
    Arrow = "arrow",
    Triangle = "triangle",
    Circle = "circle",
    Bar = "bar",
});

/// Writes a choice of one key: `{"key": value}`.
fn put_choice<'d>(out: &mut Out<'d>, key: &str, write: impl FnOnce(&mut Out<'d>)) {
    out.text.push('{');
    out.string(key);
    out.text.push(':');
    out.within(key, write);
    out.text.push('}');
}

/// Reads the value of a choice of one key, its unknown fields put under
/// the key.
fn get_choice<T>(
    key: &str,
    value: &Node<'_>,
    cx: &mut Cx,
    read: impl FnOnce(&Node<'_>, &mut Cx) -> R<T>,
) -> R<T> {
    let from = cx.fields.len();
    let read = read(value, cx).map_err(|bad| bad.at(key))?;
    if cx.fields.len() > from {
        cx.under(from, key);
    }
    Ok(read)
}

impl Form for Background {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        match self {
            Background::None => out.string("none"),
            Background::Blur { amount } => put_choice(out, "blur", |out| out.float(*amount)),
            Background::Color(color) => put_choice(out, "color", |out| color.put(out)),
            Background::Image(media) => put_choice(out, "image", |out| media.put(out)),
            Background::Unknown(raw) => out.raw(raw.as_str()),
        }
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        if node.str() == Some("none") {
            return Ok(Background::None);
        }
        Ok(match choice(node) {
            Some(("blur", value)) => Background::Blur {
                amount: get_at(value, cx, "blur")?,
            },
            Some(("color", value)) => Background::Color(get_at(value, cx, "color")?),
            Some(("image", value)) => Background::Image(get_at(value, cx, "image")?),
            _ => Background::Unknown(cx.raw(node)),
        })
    }
}

impl Form for Cover {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        match self {
            Cover::Frame { into } => put_choice(out, "frame", |out| into.put(out)),
            Cover::Picture { media } => put_choice(out, "picture", |out| media.put(out)),
            Cover::Unknown(raw) => out.raw(raw.as_str()),
        }
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        Ok(match choice(node) {
            Some(("frame", value)) => Cover::Frame {
                into: get_at(value, cx, "frame")?,
            },
            Some(("picture", value)) => Cover::Picture {
                media: get_at(value, cx, "picture")?,
            },
            _ => Cover::Unknown(cx.raw(node)),
        })
    }
}

impl Form for Ease {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        match self {
            Ease::Bezier { x1, y1, x2, y2 } => out.list(&[*x1, *y1, *x2, *y2], |out, value| {
                out.float(*value);
            }),
            Ease::Unknown(raw) => out.raw(raw.as_str()),
        }
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        match node.array() {
            Some([x1, y1, x2, y2]) => Ok(Ease::Bezier {
                x1: f64::get(x1, cx)?,
                y1: f64::get(y1, cx)?,
                x2: f64::get(x2, cx)?,
                y2: f64::get(y2, cx)?,
            }),
            Some(_) => wrong("is not four numbers"),
            None => Ok(Ease::Unknown(cx.raw(node))),
        }
    }
}

impl Form for TimeCurve {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        match self {
            TimeCurve::Linear => out.string("linear"),
            TimeCurve::Smooth { leave, arrive } => put_choice(out, "smooth", |out| {
                let mut obj = out.open();
                always(out, &mut obj, "leave", leave);
                always(out, &mut obj, "arrive", arrive);
                out.close(obj);
            }),
            TimeCurve::Unknown(raw) => out.raw(raw.as_str()),
        }
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        if node.str() == Some("linear") {
            return Ok(TimeCurve::Linear);
        }
        let Some(("smooth", value)) = choice(node) else {
            return Ok(TimeCurve::Unknown(cx.raw(node)));
        };
        get_choice("smooth", value, cx, |value, cx| {
            let (mut leave, mut arrive) = (None, None);
            fields(value, cx, |key, node, cx| {
                match key {
                    "leave" => leave = Some(Speed::get(node, cx)?),
                    "arrive" => arrive = Some(Speed::get(node, cx)?),
                    _ => return Ok(false),
                }
                Ok(true)
            })?;
            Ok(TimeCurve::Smooth {
                leave: need(leave, "leave")?,
                arrive: need(arrive, "arrive")?,
            })
        })
    }
}

/// One position key of a time map; the first has no curve.
fn put_point<'d>(out: &mut Out<'d>, point: TimePoint, curve: Option<&'d TimeCurve>) {
    let mut obj = out.open();
    out.key(&mut obj, "at");
    out.int(point.at.get());
    out.key(&mut obj, "source");
    out.int(point.source.get());
    if let Some(curve) = curve
        && (*curve != TimeCurve::Linear || out.holds_under("curve"))
    {
        always(out, &mut obj, "curve", curve);
    }
    out.close(obj);
}

impl Form for TimeMap {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        match self {
            TimeMap::Linear { src_in, speed } => put_choice(out, "linear", |out| {
                let mut obj = out.open();
                field(out, &mut obj, "src_in", src_in, &Tick::ZERO);
                field(out, &mut obj, "speed", speed, &Speed::NORMAL);
                out.close(obj);
            }),
            TimeMap::Keyed { first, rest } => put_choice(out, "keyed", |out| {
                let points = std::iter::once((*first, None))
                    .chain(rest.iter().map(|(curve, point)| (*point, Some(curve))));
                out.text.push('[');
                for (at, (point, curve)) in points.enumerate() {
                    if at > 0 {
                        out.text.push(',');
                    }
                    // A key is named by its tick, as an animated number's.
                    if out.tracking() {
                        out.within(&point.at.get().to_string(), |out| {
                            put_point(out, point, curve);
                        });
                    } else {
                        put_point(out, point, curve);
                    }
                }
                out.text.push(']');
            }),
            TimeMap::Unknown(raw) => out.raw(raw.as_str()),
        }
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        match choice(node) {
            Some(("linear", value)) => get_choice("linear", value, cx, |value, cx| {
                let (mut src_in, mut speed) = (Tick::ZERO, Speed::NORMAL);
                fields(value, cx, |key, node, cx| {
                    match key {
                        "src_in" => src_in = <Tick as Form>::get(node, cx)?,
                        "speed" => speed = Speed::get(node, cx)?,
                        _ => return Ok(false),
                    }
                    Ok(true)
                })?;
                Ok(TimeMap::Linear { src_in, speed })
            }),
            Some(("keyed", value)) => get_choice("keyed", value, cx, |value, cx| {
                let mut points = Vec::new();
                for (index, item) in array(value)?.iter().enumerate() {
                    let from = cx.fields.len();
                    let (mut at, mut source, mut curve) = (None, None, None);
                    fields(item, cx, |key, node, cx| {
                        match key {
                            "at" => at = Some(<Tick as Form>::get(node, cx)?),
                            "source" => source = Some(<Tick as Form>::get(node, cx)?),
                            "curve" if index > 0 => curve = Some(TimeCurve::get(node, cx)?),
                            _ => return Ok(false),
                        }
                        Ok(true)
                    })
                    .map_err(|bad| bad.at(&(index + 1).to_string()))?;
                    let point = TimePoint {
                        at: need(at, "at")?,
                        source: need(source, "source")?,
                    };
                    if cx.fields.len() > from {
                        cx.under(from, &point.at.get().to_string());
                    }
                    points.push((curve.unwrap_or(TimeCurve::Linear), point));
                }
                let mut points = points.into_iter();
                let Some((_, first)) = points.next() else {
                    return wrong("has no keys");
                };
                Ok(TimeMap::Keyed {
                    first,
                    rest: points.collect(),
                })
            }),
            _ => Ok(TimeMap::Unknown(cx.raw(node))),
        }
    }
}

impl Form for MaskSource {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        match self {
            MaskSource::Shape {
                shape,
                width,
                height,
                roundness,
            } => put_choice(out, "shape", |out| {
                let mut obj = out.open();
                always(out, &mut obj, "shape", shape);
                always(out, &mut obj, "width", width);
                always(out, &mut obj, "height", height);
                always(out, &mut obj, "roundness", roundness);
                out.close(obj);
            }),
            MaskSource::Unknown(raw) => out.raw(raw.as_str()),
        }
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        let Some(("shape", value)) = choice(node) else {
            return Ok(MaskSource::Unknown(cx.raw(node)));
        };
        get_choice("shape", value, cx, |value, cx| {
            let (mut shape, mut width, mut height, mut roundness) = (None, None, None, None);
            fields(value, cx, |key, node, cx| {
                match key {
                    "shape" => shape = Some(MaskShape::get(node, cx)?),
                    "width" => width = Some(Animated::get(node, cx)?),
                    "height" => height = Some(Animated::get(node, cx)?),
                    "roundness" => roundness = Some(Animated::get(node, cx)?),
                    _ => return Ok(false),
                }
                Ok(true)
            })?;
            Ok(MaskSource::Shape {
                shape: need(shape, "shape")?,
                width: need(width, "width")?,
                height: need(height, "height")?,
                roundness: need(roundness, "roundness")?,
            })
        })
    }
}

impl Form for Content {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        match self {
            Content::Media(cut) => put_choice(out, "media", |out| cut.put(out)),
            Content::Nest(nest) => put_choice(out, "nest", |out| nest.put(out)),
            Content::Title(title) => put_choice(out, "title", |out| title.put(out)),
            Content::Shape(shape) => put_choice(out, "shape", |out| shape.put(out)),
            Content::Layer => out.string("layer"),
            Content::SoundLayer => out.string("sound_layer"),
            Content::Transition(effect) => put_choice(out, "transition", |out| effect.put(out)),
            Content::Unknown { kind, raw } if raw.as_str().is_empty() => out.string(kind),
            Content::Unknown { kind, raw } => put_choice(out, kind, |out| out.raw(raw.as_str())),
        }
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        const DATA: [&str; 5] = ["media", "nest", "title", "shape", "transition"];
        if let Some(kind) = node.str() {
            return match kind {
                "layer" => Ok(Content::Layer),
                "sound_layer" => Ok(Content::SoundLayer),
                kind if DATA.contains(&kind) => wrong("is a kind with no settings"),
                kind => Ok(Content::Unknown {
                    kind: kind.to_owned(),
                    raw: Raw::default(),
                }),
            };
        }
        let Some((kind, value)) = choice(node) else {
            return wrong("is not one kind of content");
        };
        let read = |cx: &mut Cx, read: fn(&Node<'_>, &mut Cx) -> R<Content>| {
            get_choice(kind, value, cx, read)
        };
        match kind {
            "media" => read(cx, |v, cx| MediaCut::get(v, cx).map(Content::Media)),
            "nest" => read(cx, |v, cx| Nest::get(v, cx).map(Content::Nest)),
            "title" => read(cx, |v, cx| Title::get(v, cx).map(Content::Title)),
            "shape" => read(cx, |v, cx| ShapeStyle::get(v, cx).map(Content::Shape)),
            "transition" => read(cx, |v, cx| Effect::get(v, cx).map(Content::Transition)),
            "layer" | "sound_layer" => wrong("is a kind with no settings"),
            kind => Ok(Content::Unknown {
                kind: kind.to_owned(),
                raw: cx.raw(value),
            }),
        }
    }
}

// --- Animated numbers -------------------------------------------------------

impl Form for Key<f64> {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        let mut obj = out.open();
        always(out, &mut obj, "at", &self.at);
        always(out, &mut obj, "value", &self.value);
        field(out, &mut obj, "ease", &self.ease, &Ease::LINEAR);
        out.close(obj);
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        let (mut at, mut value, mut ease) = (None, None, Ease::LINEAR);
        fields(node, cx, |key, node, cx| {
            match key {
                "at" => at = Some(<Tick as Form>::get(node, cx)?),
                "value" => value = Some(f64::get(node, cx)?),
                "ease" => ease = Ease::get(node, cx)?,
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(Key {
            at: need(at, "at")?,
            value: need(value, "value")?,
            ease,
        })
    }
    /// A key is named by its tick, so what this build does not know of
    /// it stays with it when keys around it come and go.
    fn step(&self, _: usize) -> String {
        self.at.get().to_string()
    }
}

/// A number, or `{"value": 1, "keys": [...]}` when it has keys.
impl Form for Animated<f64> {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        if self.keys.is_empty() && !out.holds_here() {
            out.float(self.value);
            return;
        }
        let mut obj = out.open();
        always(out, &mut obj, "value", &self.value);
        field(out, &mut obj, "keys", &self.keys, &Vec::new());
        out.close(obj);
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        if let Kind::Number = node.kind {
            return f64::get(node, cx).map(Animated::fixed);
        }
        let (mut value, mut keys) = (None, Vec::new());
        fields(node, cx, |key, node, cx| {
            match key {
                "value" => value = Some(f64::get(node, cx)?),
                "keys" => keys = Vec::get(node, cx)?,
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(Animated {
            value: need(value, "value")?,
            keys,
        })
    }
}

/// An effect's knob: a number, or an object of keys and, when it has
/// one, a value.
impl Form for Param {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        if let (Some(value), true) = (self.value, self.keys.is_empty())
            && !out.holds_here()
        {
            out.float(value);
            return;
        }
        let mut obj = out.open();
        field(out, &mut obj, "value", &self.value, &None);
        field(out, &mut obj, "keys", &self.keys, &Vec::new());
        out.close(obj);
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        if let Kind::Number = node.kind {
            return f64::get(node, cx).map(|value| Animated::fixed(Some(value)));
        }
        let (mut value, mut keys) = (None, Vec::new());
        fields(node, cx, |key, node, cx| {
            match key {
                "value" => value = Option::get(node, cx)?,
                "keys" => keys = Vec::get(node, cx)?,
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(Animated { value, keys })
    }
}

// --- Parts ------------------------------------------------------------------

/// A part whose fields are each required, or have the default given.
macro_rules! part {
    ($ty:ident { $( $field:ident $(= $default:expr)? ),* $(,)? }) => {
        impl Form for $ty {
            fn put<'d>(&'d self, out: &mut Out<'d>) {
                let mut obj = out.open();
                $( part!(@put out, obj, stringify!($field), &self.$field $(, $default)?); )*
                out.close(obj);
            }
            fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
                $( let mut $field = None; )*
                fields(node, cx, |key, node, cx| {
                    match key {
                        $( stringify!($field) => $field = Some(Form::get(node, cx)?), )*
                        _ => return Ok(false),
                    }
                    Ok(true)
                })?;
                Ok($ty {
                    $( $field: part!(@take $field, stringify!($field) $(, $default)?), )*
                })
            }
        }
    };
    (@put $out:ident, $obj:ident, $key:expr, $value:expr) => {
        always($out, &mut $obj, $key, $value)
    };
    (@put $out:ident, $obj:ident, $key:expr, $value:expr, $default:expr) => {
        field($out, &mut $obj, $key, $value, &$default)
    };
    (@take $value:ident, $key:expr) => {
        need($value, $key)?
    };
    (@take $value:ident, $key:expr, $default:expr) => {
        match $value {
            Some(value) => value,
            None => $default,
        }
    };
}

/// A part whose every field defaults to a base value's.
macro_rules! over {
    ($ty:ident = $base:expr; $( $field:ident ),* $(,)?) => {
        impl $ty {
            fn put_over<'d>(&'d self, out: &mut Out<'d>, base: &$ty) {
                let mut obj = out.open();
                $( field(out, &mut obj, stringify!($field), &self.$field, &base.$field); )*
                out.close(obj);
            }
            fn get_over(node: &Node<'_>, cx: &mut Cx, mut value: $ty) -> R<$ty> {
                fields(node, cx, |key, node, cx| {
                    match key {
                        $( stringify!($field) => value.$field = Form::get(node, cx)?, )*
                        _ => return Ok(false),
                    }
                    Ok(true)
                })?;
                Ok(value)
            }
        }
        impl Form for $ty {
            fn put<'d>(&'d self, out: &mut Out<'d>) {
                self.put_over(out, &$base)
            }
            fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
                $ty::get_over(node, cx, $base)
            }
        }
    };
}

part!(Size { width, height });
part!(Crop {
    left,
    top,
    right,
    bottom
});
part!(Stabilize {
    smoothness,
    max_zoom,
    keep_rotation,
    tripod
});
part!(Move { name, len });
part!(Loop { name, lap, amount });
part!(VoiceMix {
    kind,
    voice,
    background
});
part!(CutIn { effect, len });
part!(Link { to, kind });
part!(Enhanced {
    copy,
    copy_start,
    on
});
part!(Gradient {
    color,
    angle,
    radial
});
part!(ShapeShadow {
    color,
    angle,
    distance,
    blur
});
part!(Font { family, path });
part!(Stroke { tool, size, points, at = None });
part!(Cutout { mode, subject, feather, strokes = Vec::new() });
part!(MediaCut { media, take = Take::Whole, stream = None, time = TimeMap::default(), enhanced = None });
part!(Nest { sequence, time = TimeMap::default() });
part!(Frame { width, height, rate, canvas = Color::BLACK, color = ColorSpace::Sdr });
part!(TextRun {
    start,
    end,
    color = None,
    font_scale = None,
    font_weight = None,
    italic = None,
    underline = None,
    strikethrough = None,
    stroke_color = None,
});
part!(SoundStream {
    index,
    channels = 0,
    sample_rate = 0,
    codec = String::new(),
    language = String::new(),
    title = String::new(),
});

over!(Motion = Motion::default(); enter, exit, repeat);
over!(Sound = Sound::default(); volume, fade_in, fade_out, muted, keep_pitch, voice);
over!(Picture = Picture::default();
    offset_x, offset_y, scale, rotation, stretch_x, stretch_y, opacity, flip_h, flip_v, blend,
    crop, cutout, stabilize, background, motion);
over!(Title = Title::default();
    content, font_family, font_size, font_weight, italic, underline, strikethrough, color,
    opacity, align, case, line_height, tracking, max_width, max_height, stroke_color,
    stroke_width, shadow, plate, plate_padding_x, plate_padding_y, plate_radius, runs);
over!(ShapeStyle = ShapeStyle::default();
    kind, width, height, corner_radius, sides, points, inner_radius, fill, gradient, stroke,
    stroke_width, stroke_align, dash, trim_start, trim_end, start_head, end_head, shadow);

/// A file's sound stream is named by its index in the file.
impl SoundStream {
    fn index_step(&self) -> String {
        self.index.to_string()
    }
}

/// A caption lane's look: a title's look with no words, and where the
/// captions sit.
impl Form for CaptionLook {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        let base = CaptionLook::default();
        let mut obj = out.open();
        if self.title != base.title || out.holds_under("title") {
            out.part(&mut obj, "title", |out| {
                self.title.put_over(out, &base.title)
            });
        }
        field(out, &mut obj, "offset_y", &self.offset_y, &base.offset_y);
        out.close(obj);
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        let mut look = CaptionLook::default();
        fields(node, cx, |key, node, cx| {
            match key {
                "title" => look.title = Title::get_over(node, cx, CaptionLook::default().title)?,
                "offset_y" => look.offset_y = f64::get(node, cx)?,
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(look)
    }
}

// --- Records ----------------------------------------------------------------

/// Reads the entries of a list of records: each one this build cannot
/// read, or that needs what it does not have, is kept as its text in its
/// place, and said.
fn get_entries<T: Form>(node: &Node<'_>, cx: &mut Cx, what: &str) -> R<Vec<Entry<T>>> {
    let items = array(node)?;
    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        entries.push(match T::get(item, cx) {
            Ok(value) => Entry::Known(value),
            Err(bad) => {
                cx.note(format!(
                    "{what} is kept as it was and not opened: {}",
                    bad.words()
                ));
                Entry::Unread(cx.raw(item))
            }
        });
    }
    Ok(entries)
}

fn put_entries<'d, T: Form>(out: &mut Out<'d>, entries: &'d [Entry<T>]) {
    out.text.push('[');
    for (at, entry) in entries.iter().enumerate() {
        if at > 0 {
            out.text.push(',');
        }
        match entry {
            Entry::Known(value) => value.put(out),
            Entry::Unread(raw) => out.raw(raw.as_str()),
        }
    }
    out.text.push(']');
}

/// A list of entries, written only when it has any.
fn entries_field<'d, T: Form>(
    out: &mut Out<'d>,
    obj: &mut Obj,
    key: &str,
    entries: &'d [Entry<T>],
) {
    if !entries.is_empty() {
        out.key(obj, key);
        put_entries(out, entries);
    }
}

impl Form for Effect {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        let mut rec = out.record(&self.residual);
        always(out, &mut rec.obj, "id", &self.id);
        always(out, &mut rec.obj, "package", &self.package);
        field(out, &mut rec.obj, "enabled", &self.enabled, &true);
        field(out, &mut rec.obj, "params", &self.params, &BTreeMap::new());
        field(out, &mut rec.obj, "mask", &self.mask, &None);
        out.end(rec);
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        let (mut id, mut package) = (None, None);
        let (mut enabled, mut params, mut mask) = (true, BTreeMap::new(), None);
        let (own, derived) = record(node, cx, |key, node, cx| {
            match key {
                "id" => id = Some(EffectId::get(node, cx)?),
                "package" => package = Some(String::get(node, cx)?),
                "enabled" => enabled = bool::get(node, cx)?,
                "params" => params = <BTreeMap<String, Param> as Form>::get(node, cx)?,
                "mask" => mask = Option::get(node, cx)?,
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        let mut effect = Effect {
            id: need(id, "id")?,
            package: need(package, "package")?,
            enabled,
            params,
            mask,
            residual: Residual {
                fields: own,
                derived: None,
            },
        };
        seal(&mut effect, derived, cx, |e| &mut e.residual);
        Ok(effect)
    }
}

impl Form for Mask {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        let base = Mask::circle(self.id);
        let mut rec = out.record(&self.residual);
        let obj = &mut rec.obj;
        always(out, obj, "id", &self.id);
        field(out, obj, "name", &self.name, &base.name);
        field(out, obj, "mode", &self.mode, &base.mode);
        field(out, obj, "invert", &self.invert, &base.invert);
        field(out, obj, "source", &self.source, &base.source);
        field(out, obj, "x", &self.x, &base.x);
        field(out, obj, "y", &self.y, &base.y);
        field(out, obj, "scale", &self.scale, &base.scale);
        field(out, obj, "rotation", &self.rotation, &base.rotation);
        field(out, obj, "feather", &self.feather, &base.feather);
        field(out, obj, "expand", &self.expand, &base.expand);
        field(out, obj, "opacity", &self.opacity, &base.opacity);
        out.end(rec);
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        // The defaults are a circle's, whatever its id.
        const FIRST: crate::id::RawId = match crate::id::RawId::from_parts(crate::Writer::LOCAL, 1)
        {
            Some(raw) => raw,
            None => panic!("one is a count"),
        };
        let mut mask = Mask::circle(MaskId::from_raw(FIRST));
        let mut id = None;
        let (own, derived) = record(node, cx, |key, node, cx| {
            match key {
                "id" => id = Some(MaskId::get(node, cx)?),
                "name" => mask.name = Form::get(node, cx)?,
                "mode" => mask.mode = Form::get(node, cx)?,
                "invert" => mask.invert = Form::get(node, cx)?,
                "source" => mask.source = Form::get(node, cx)?,
                "x" => mask.x = Form::get(node, cx)?,
                "y" => mask.y = Form::get(node, cx)?,
                "scale" => mask.scale = Form::get(node, cx)?,
                "rotation" => mask.rotation = Form::get(node, cx)?,
                "feather" => mask.feather = Form::get(node, cx)?,
                "expand" => mask.expand = Form::get(node, cx)?,
                "opacity" => mask.opacity = Form::get(node, cx)?,
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        mask.id = need(id, "id")?;
        mask.residual = Residual {
            fields: own,
            derived: None,
        };
        seal(&mut mask, derived, cx, |m| &mut m.residual);
        Ok(mask)
    }
}

impl Form for Marker {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        let mut rec = out.record(&self.residual);
        always(out, &mut rec.obj, "id", &self.id);
        always(out, &mut rec.obj, "at", &self.at);
        field(out, &mut rec.obj, "len", &self.len, &TickDur::ZERO);
        field(out, &mut rec.obj, "name", &self.name, &String::new());
        field(out, &mut rec.obj, "color", &self.color, &None);
        out.end(rec);
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        let (mut id, mut at) = (None, None);
        let (mut len, mut name, mut color) = (TickDur::ZERO, String::new(), None);
        let (own, derived) = record(node, cx, |key, node, cx| {
            match key {
                "id" => id = Some(MarkerId::get(node, cx)?),
                "at" => at = Some(<Tick as Form>::get(node, cx)?),
                "len" => len = <TickDur as Form>::get(node, cx)?,
                "name" => name = String::get(node, cx)?,
                "color" => color = Option::get(node, cx)?,
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        let mut marker = Marker {
            id: need(id, "id")?,
            at: need(at, "at")?,
            len,
            name,
            color,
            residual: Residual {
                fields: own,
                derived: None,
            },
        };
        seal(&mut marker, derived, cx, |m| &mut m.residual);
        Ok(marker)
    }
}

impl Form for Media {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        let base = Media::default();
        let mut rec = out.record(&self.residual);
        let obj = &mut rec.obj;
        field(out, obj, "path", &self.path, &base.path);
        field(out, obj, "name", &self.name, &base.name);
        field(out, obj, "kind", &self.kind, &base.kind);
        field(out, obj, "length", &self.length, &base.length);
        field(out, obj, "rate", &self.rate, &base.rate);
        field(out, obj, "size", &self.size, &base.size);
        field(out, obj, "has_sound", &self.has_sound, &base.has_sound);
        if !self.streams.is_empty() || out.holds_under("streams") {
            out.part(obj, "streams", |out| put_streams(out, &self.streams));
        }
        field(
            out,
            obj,
            "video_codec",
            &self.video_codec,
            &base.video_codec,
        );
        field(
            out,
            obj,
            "audio_codec",
            &self.audio_codec,
            &base.audio_codec,
        );
        field(out, obj, "color", &self.color, &base.color);
        field(out, obj, "range", &self.range, &base.range);
        field(out, obj, "beats", &self.beats, &base.beats);
        field(out, obj, "origin", &self.origin, &base.origin);
        field(
            out,
            obj,
            "placeholder",
            &self.placeholder,
            &base.placeholder,
        );
        out.end(rec);
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        let mut media = Media::default();
        let (own, derived) = record(node, cx, |key, node, cx| {
            match key {
                "path" => media.path = Form::get(node, cx)?,
                "name" => media.name = Form::get(node, cx)?,
                "kind" => media.kind = Form::get(node, cx)?,
                "length" => media.length = Form::get(node, cx)?,
                "rate" => media.rate = Form::get(node, cx)?,
                "size" => media.size = Form::get(node, cx)?,
                "has_sound" => media.has_sound = Form::get(node, cx)?,
                "streams" => media.streams = get_streams(node, cx)?,
                "video_codec" => media.video_codec = Form::get(node, cx)?,
                "audio_codec" => media.audio_codec = Form::get(node, cx)?,
                "color" => media.color = Form::get(node, cx)?,
                "range" => media.range = Form::get(node, cx)?,
                "beats" => media.beats = Form::get(node, cx)?,
                "origin" => media.origin = Form::get(node, cx)?,
                "placeholder" => media.placeholder = Form::get(node, cx)?,
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        media.residual = Residual {
            fields: own,
            derived: None,
        };
        seal(&mut media, derived, cx, |m| &mut m.residual);
        Ok(media)
    }
}

/// A file's sound streams, each named by its index in the file.
fn put_streams<'d>(out: &mut Out<'d>, streams: &'d [SoundStream]) {
    out.text.push('[');
    for (at, stream) in streams.iter().enumerate() {
        if at > 0 {
            out.text.push(',');
        }
        out.within(&stream.index_step(), |out| stream.put(out));
    }
    out.text.push(']');
}

fn get_streams(node: &Node<'_>, cx: &mut Cx) -> R<Vec<SoundStream>> {
    let items = array(node)?;
    let mut streams = Vec::with_capacity(items.len());
    for (at, item) in items.iter().enumerate() {
        let from = cx.fields.len();
        let stream = SoundStream::get(item, cx).map_err(|bad| bad.at(&(at + 1).to_string()))?;
        if cx.fields.len() > from {
            cx.under(from, &stream.index_step());
        }
        streams.push(stream);
    }
    Ok(streams)
}

impl Form for Sequence {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        let mut rec = out.record(&self.residual);
        let obj = &mut rec.obj;
        always(out, obj, "name", &self.name);
        always(out, obj, "frame", &self.frame);
        field(out, obj, "background", &self.background, &Background::None);
        field(out, obj, "linkage", &self.linkage, &true);
        entries_field(out, obj, "markers", &self.markers);
        out.end(rec);
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        let (mut name, mut frame) = (None, None);
        let (mut background, mut linkage, mut markers) = (Background::None, true, Vec::new());
        let (own, derived) = record(node, cx, |key, node, cx| {
            match key {
                "name" => name = Some(String::get(node, cx)?),
                "frame" => frame = Some(Frame::get(node, cx)?),
                "background" => background = Background::get(node, cx)?,
                "linkage" => linkage = bool::get(node, cx)?,
                "markers" => markers = get_entries(node, cx, "a marker")?,
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        let mut sequence = Sequence {
            name: need(name, "name")?,
            frame: need(frame, "frame")?,
            background,
            linkage,
            markers,
            residual: Residual {
                fields: own,
                derived: None,
            },
        };
        seal(&mut sequence, derived, cx, |s| &mut s.residual);
        Ok(sequence)
    }
}

impl Form for Lane {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        let mut rec = out.record(&self.residual);
        let obj = &mut rec.obj;
        always(out, obj, "sequence", &self.sequence);
        always(out, obj, "order", &self.order);
        field(out, obj, "visible", &self.visible, &true);
        field(out, obj, "muted", &self.muted, &false);
        field(out, obj, "captions", &self.captions, &None);
        out.end(rec);
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        let (mut sequence, mut order) = (None, None);
        let (mut visible, mut muted, mut captions) = (true, false, None);
        let (own, derived) = record(node, cx, |key, node, cx| {
            match key {
                "sequence" => sequence = Some(SequenceId::get(node, cx)?),
                "order" => order = Some(OrderKey::get(node, cx)?),
                "visible" => visible = bool::get(node, cx)?,
                "muted" => muted = bool::get(node, cx)?,
                "captions" => captions = Option::<CaptionLook>::get(node, cx)?,
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        let mut lane = Lane {
            sequence: need(sequence, "sequence")?,
            order: need(order, "order")?,
            visible,
            muted,
            captions,
            residual: Residual {
                fields: own,
                derived: None,
            },
        };
        seal(&mut lane, derived, cx, |l| &mut l.residual);
        // The words are each caption's; a look that has some is a look
        // written wrong, and the words are no caption's.
        if let Some(look) = &mut lane.captions
            && (!look.title.content.is_empty() || !look.title.runs.is_empty())
        {
            look.title.content.clear();
            look.title.runs.clear();
            cx.note("its caption look had words of its own, which were taken out");
        }
        Ok(lane)
    }
}

impl Form for Item {
    fn put<'d>(&'d self, out: &mut Out<'d>) {
        let mut rec = out.record(&self.residual);
        let obj = &mut rec.obj;
        always(out, obj, "lane", &self.lane);
        always(out, obj, "start", &self.start);
        always(out, obj, "len", &self.len);
        field(out, obj, "offset", &self.offset, &Tick::ZERO);
        always(out, obj, "content", &self.content);
        field(out, obj, "name", &self.name, &String::new());
        if self.picture != Picture::default() || out.holds_under("picture") {
            out.part(obj, "picture", |out| self.picture.put(out));
        }
        if self.sound != Sound::default() || out.holds_under("sound") {
            out.part(obj, "sound", |out| self.sound.put(out));
        }
        entries_field(out, obj, "effects", &self.effects);
        entries_field(out, obj, "filters", &self.filters);
        entries_field(out, obj, "masks", &self.masks);
        entries_field(out, obj, "markers", &self.markers);
        field(out, obj, "cut_in", &self.cut_in, &None);
        field(out, obj, "link", &self.link, &None);
        out.end(rec);
    }
    fn get(node: &Node<'_>, cx: &mut Cx) -> R<Self> {
        let (mut lane, mut start, mut len, mut content) = (None, None, None, None);
        let mut offset = Tick::ZERO;
        let mut name = String::new();
        let mut picture = Picture::default();
        let mut sound = Sound::default();
        let (mut effects, mut filters, mut masks, mut markers) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (mut cut_in, mut link) = (None, None);
        let (own, derived) = record(node, cx, |key, node, cx| {
            match key {
                "lane" => lane = Some(LaneId::get(node, cx)?),
                "start" => start = Some(<Tick as Form>::get(node, cx)?),
                "len" => len = Some(<TickLen as Form>::get(node, cx)?),
                "offset" => offset = <Tick as Form>::get(node, cx)?,
                "content" => content = Some(Content::get(node, cx)?),
                "name" => name = String::get(node, cx)?,
                "picture" => picture = Picture::get(node, cx)?,
                "sound" => sound = Sound::get(node, cx)?,
                "effects" => effects = get_entries(node, cx, "an effect")?,
                "filters" => filters = get_entries(node, cx, "a sound filter")?,
                "masks" => masks = get_entries(node, cx, "a mask")?,
                "markers" => markers = get_entries(node, cx, "a marker")?,
                "cut_in" => cut_in = Option::get(node, cx)?,
                "link" => link = Option::get(node, cx)?,
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        let mut item = Item {
            lane: need(lane, "lane")?,
            start: need(start, "start")?,
            len: need(len, "len")?,
            offset,
            content: need(content, "content")?,
            name,
            picture,
            sound,
            effects,
            filters,
            masks,
            markers,
            cut_in,
            link,
            residual: Residual {
                fields: own,
                derived: None,
            },
        };
        seal(&mut item, derived, cx, |i| &mut i.residual);
        Ok(item)
    }
}

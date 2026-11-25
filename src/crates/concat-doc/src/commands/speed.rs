// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Speed: a constant pace, a speed curve, and what a clip's time map
//! says about the source it shows.
//!
//! A clip's map takes its content time to its source (decision 5). A
//! speed change keeps the source the window shows and its in-point, and
//! changes the window's length; the keys keep their share of the clip.
//! A speed curve becomes position keys: a straight ramp is one smooth
//! stretch exactly, and a curve that bends is fitted with eight stretches
//! to each of its own, each key exact in source.

use crate::model::{
    Animated, Content, FASTEST, Item, ItemId, MediaKind, Model, SLOWEST, Speed, TimeCurve, TimeMap,
    TimePoint,
};
use crate::time::{Tick, TickDur, TickLen, rescale};

use super::{CommandError, Ed, SpeedPoint};

/// A pace from a decimal, to a hundred-thousandth, held from a sixteenth
/// to sixteen times either way. `None` for zero or a number that is not
/// one.
pub fn speed_from_decimal(value: f64) -> Option<Speed> {
    if !value.is_finite() || value == 0.0 {
        return None;
    }
    let size = value.abs().clamp(SLOWEST.display(), FASTEST.display());
    let num = (size * 100_000.0).round() as i32;
    let speed = Speed::new(num * value.signum() as i32, 100_000)?;
    Some(speed)
}

fn clamp_speed(speed: Speed) -> Speed {
    if speed.is_allowed() {
        return speed;
    }
    speed_from_decimal(speed.display()).unwrap_or(Speed::NORMAL)
}

/// The time map of what an item shows, if it has one.
pub(crate) fn map_of(item: &Item) -> Option<&TimeMap> {
    match &item.content {
        Content::Media(cut) => Some(&cut.time),
        Content::Nest(nest) => Some(&nest.time),
        _ => None,
    }
}

fn map_mut(item: &mut Item) -> Option<&mut TimeMap> {
    match &mut item.content {
        Content::Media(cut) => Some(&mut cut.time),
        Content::Nest(nest) => Some(&mut nest.time),
        _ => None,
    }
}

/// The source at a content tick, worked out on a smooth stretch too.
/// `None` past a keyed map's keys and for a map this build cannot read.
pub(crate) fn source_at(map: &TimeMap, c: Tick) -> Option<Tick> {
    if let Some(exact) = map.source_at(c) {
        return Some(exact);
    }
    let TimeMap::Keyed { first, rest } = map else {
        return None;
    };
    let mut from = *first;
    for (curve, to) in rest {
        if c >= from.at && c <= to.at {
            let TimeCurve::Smooth { leave, arrive } = curve else {
                return None;
            };
            let w = (to.at.get() - from.at.get()) as f64;
            let u = (c.get() - from.at.get()) as f64 / w;
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

/// The source the window shows, from its start to its end, signed.
pub(crate) fn covered(item: &Item) -> Option<(Tick, TickDur)> {
    let map = map_of(item)?;
    let from = source_at(map, item.offset)?;
    let to = source_at(map, item.offset + item.len)?;
    Some((from, to - from))
}

/// The map moved so that content tick `at` shows `source`.
pub(crate) fn with_in_point(map: &TimeMap, at: Tick, source: Tick) -> Option<TimeMap> {
    let now = source_at(map, at)?;
    let by = source - now;
    Some(match map {
        TimeMap::Linear { src_in, speed } => TimeMap::Linear {
            src_in: src_in.checked_add(by)?,
            speed: *speed,
        },
        TimeMap::Keyed { first, rest } => TimeMap::Keyed {
            first: TimePoint {
                at: first.at,
                source: first.source.checked_add(by)?,
            },
            rest: rest
                .iter()
                .map(|(curve, point)| {
                    Some((
                        curve.clone(),
                        TimePoint {
                            at: point.at,
                            source: point.source.checked_add(by)?,
                        },
                    ))
                })
                .collect::<Option<Vec<_>>>()?,
        },
        TimeMap::Unknown(_) => return None,
    })
}

/// How long the file an item is cut from runs, where that limits the
/// window: video and sound with a known length.
fn source_length(m: &Model, item: &Item) -> Option<Option<TickLen>> {
    match &item.content {
        Content::Media(cut) => {
            let file = m.media.get(cut.media)?;
            if file.kind == MediaKind::Image {
                return Some(None);
            }
            Some(file.length)
        }
        Content::Nest(_) => Some(None),
        _ => None,
    }
}

/// The content ticks the window may reach to: as far back and on as the
/// source has, and inside a keyed map's keys. `None` on a side is no
/// limit.
pub(crate) fn window_limits(m: &Model, item: &Item) -> (Option<Tick>, Option<Tick>) {
    let Some(map) = map_of(item) else {
        return (None, None);
    };
    let Some(length) = source_length(m, item) else {
        return (None, None);
    };
    match map {
        TimeMap::Linear { src_in, speed } => {
            let (n, d) = (i64::from(speed.num()), i64::from(speed.den()));
            let s = src_in.get();
            if n > 0 {
                let low = rescale(s, d, n).map(|v| Tick::new(-v));
                let high = length.and_then(|l| rescale(l.get() - s, d, n).map(Tick::new));
                (low, high)
            } else {
                let size = -n;
                let high = rescale(s, d, size).map(Tick::new);
                let low = length.and_then(|l| rescale(l.get() - s, d, size).map(|v| Tick::new(-v)));
                (low, high)
            }
        }
        TimeMap::Keyed { first, rest } => {
            let last = rest.last().map_or(first.at, |(_, p)| p.at);
            (Some(first.at), Some(last))
        }
        TimeMap::Unknown(_) => (Some(item.offset), Some(item.offset + item.len)),
    }
}

/// Every key the item holds moved on content time from `[from, from +
/// old)` to `[from, from + new)`, keeping its share of the clip.
fn rescale_keys(item: &mut Item, old: TickLen, new: TickLen) {
    let from = item.offset;
    let at = |t: Tick| -> Tick {
        let into = (t - from).get();
        Tick::new(from.get() + rescale(into, new.get(), old.get()).unwrap_or(into))
    };
    let each = |a: &mut Animated<f64>| {
        for key in &mut a.keys {
            key.at = at(key.at);
        }
    };
    let p = &mut item.picture;
    for a in [
        &mut p.offset_x,
        &mut p.offset_y,
        &mut p.scale,
        &mut p.rotation,
        &mut p.stretch_x,
        &mut p.stretch_y,
        &mut p.opacity,
        &mut item.sound.volume,
    ] {
        each(a);
    }
    let chains = item.effects.iter_mut().chain(item.filters.iter_mut());
    for effect in chains.filter_map(crate::model::Entry::known_mut) {
        for param in effect.params.values_mut() {
            for key in &mut param.keys {
                key.at = at(key.at);
            }
        }
    }
    for mask in item
        .masks
        .iter_mut()
        .filter_map(crate::model::Entry::known_mut)
    {
        for a in [
            &mut mask.x,
            &mut mask.y,
            &mut mask.scale,
            &mut mask.rotation,
            &mut mask.feather,
            &mut mask.expand,
            &mut mask.opacity,
        ] {
            each(a);
        }
        if let crate::model::MaskSource::Shape {
            width,
            height,
            roundness,
            ..
        } = &mut mask.source
        {
            for a in [width, height, roundness] {
                each(a);
            }
        }
    }
    for marker in item
        .markers
        .iter_mut()
        .filter_map(crate::model::Entry::known_mut)
    {
        marker.at = at(marker.at);
    }
}

/// One stretch of a tidied curve: where it starts, its speed leaving,
/// and, for a smooth one, its slope there.
#[derive(Clone, Copy, Debug)]
struct Knot {
    x: f64,
    v: f64,
    slope: f64,
}

/// The curve as it is played: ordered, held to 0 to 1 and to the speed
/// range, reaching both ends; smooth ones held monotone between points.
fn tidy(points: &[SpeedPoint]) -> Option<(Vec<Knot>, bool)> {
    let smooth = !points.is_empty() && points.iter().all(|p| p.slope.is_some());
    let (low, high) = (SLOWEST.display(), FASTEST.display());
    let mut knots: Vec<Knot> = points
        .iter()
        .filter(|p| p.at.is_finite() && p.speed.is_finite())
        .map(|p| Knot {
            x: p.at.clamp(0.0, 1.0),
            v: p.speed.clamp(low, high),
            slope: p.slope.filter(|s| s.is_finite()).unwrap_or(0.0),
        })
        .collect();
    if knots.is_empty() {
        return None;
    }
    knots.sort_by(|a, b| a.x.total_cmp(&b.x));
    if knots[0].x > 0.0 {
        let v = knots[0].v;
        knots.insert(
            0,
            Knot {
                x: 0.0,
                v,
                slope: 0.0,
            },
        );
    }
    if let Some(last) = knots.last().copied()
        && last.x < 1.0
    {
        knots.push(Knot {
            x: 1.0,
            v: last.v,
            slope: 0.0,
        });
    }
    if smooth {
        for i in 0..knots.len() - 1 {
            let w = knots[i + 1].x - knots[i].x;
            if w <= 0.0 {
                continue;
            }
            let r = (knots[i + 1].v - knots[i].v) / w;
            if r.abs() < 1e-12 {
                knots[i].slope = 0.0;
                knots[i + 1].slope = 0.0;
                continue;
            }
            let a = (knots[i].slope / r).max(0.0);
            let b = (knots[i + 1].slope / r).max(0.0);
            let size = (a * a + b * b).sqrt();
            let tau = if size > 3.0 { 3.0 / size } else { 1.0 };
            knots[i].slope = tau * a * r;
            knots[i + 1].slope = tau * b * r;
        }
    }
    Some((knots, smooth))
}

/// The speed at `u` of the way through a stretch, and the source used up
/// over that much of it, in shares of the clip.
fn stretch(a: Knot, b: Knot, smooth: bool, u: f64) -> (f64, f64) {
    let w = b.x - a.x;
    if !smooth {
        let v = a.v + (b.v - a.v) * u;
        return (v, w * u * (a.v + v) / 2.0);
    }
    let (m0, m1) = (a.slope * w, b.slope * w);
    let (u2, u3, u4) = (u * u, u * u * u, u * u * u * u);
    let v = (2.0 * u3 - 3.0 * u2 + 1.0) * a.v
        + (u3 - 2.0 * u2 + u) * m0
        + (-2.0 * u3 + 3.0 * u2) * b.v
        + (u3 - u2) * m1;
    let c = w
        * ((u - u3 + u4 / 2.0) * a.v
            + (u4 / 4.0 - 2.0 * u3 / 3.0 + u2 / 2.0) * m0
            + (u3 - u4 / 2.0) * b.v
            + (u4 / 4.0 - u3 / 3.0) * m1);
    (v, c)
}

/// Samples of a curve: shares of the clip, the speed there, and the
/// source used up to there, in shares of the clip's length. A step (two
/// points at one share) is one sample with the speed before and after.
fn samples(knots: &[Knot], smooth: bool) -> Vec<(f64, f64, f64, f64)> {
    let pieces = if smooth { 8 } else { 1 };
    let mut out: Vec<(f64, f64, f64, f64)> = vec![(0.0, knots[0].v, knots[0].v, 0.0)];
    let mut used = 0.0;
    for pair in knots.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if b.x <= a.x {
            if let Some(last) = out.last_mut() {
                last.2 = b.v;
            }
            continue;
        }
        for k in 1..=pieces {
            let u = f64::from(k) / f64::from(pieces);
            let (v, c) = stretch(a, b, smooth, u);
            out.push((a.x + (b.x - a.x) * u, v, v, used + c));
        }
        used += stretch(a, b, smooth, 1.0).1;
    }
    out
}

impl Ed<'_> {
    /// `setClipSpeed`.
    pub(crate) fn set_speed(
        &mut self,
        id: ItemId,
        speed: Speed,
        ripple: bool,
    ) -> Result<(), CommandError> {
        let Some(item) = self.d.items.get(id).cloned() else {
            return Ok(());
        };
        let Some((from, span)) = covered(&item) else {
            return Ok(());
        };
        let backwards = span < TickDur::ZERO
            || matches!(map_of(&item), Some(TimeMap::Linear { speed, .. }) if speed.is_reverse());
        let mut speed = clamp_speed(speed);
        if speed.is_reverse() != backwards {
            speed = Speed::new(-speed.num(), speed.den()).unwrap_or(speed);
        }
        let (n, d) = (
            i64::from(speed.num().unsigned_abs()),
            i64::from(speed.den()),
        );
        let size = span.get().abs();
        let len = rescale(size, d, n)
            .and_then(TickLen::new)
            .unwrap_or(TickLen::ONE);
        let at = TickDur::new(item.offset.get());
        let Some(src_in) = speed.of(at).and_then(|moved| from.checked_sub(moved)) else {
            return Ok(());
        };
        let map = TimeMap::Linear { src_in, speed };
        self.retime(id, &item, map, len, ripple)
    }

    /// `setClipSpeedCurve`.
    pub(crate) fn set_speed_curve(
        &mut self,
        id: ItemId,
        curve: Option<&[SpeedPoint]>,
        ripple: bool,
    ) -> Result<(), CommandError> {
        let Some(item) = self.d.items.get(id).cloned() else {
            return Ok(());
        };
        let Some((from, span)) = covered(&item) else {
            return Ok(());
        };
        let tidied = curve.and_then(tidy);
        let Some((knots, smooth)) = tidied else {
            // A constant pace at the mean, the length kept.
            let Some(TimeMap::Keyed { .. }) = map_of(&item) else {
                return Ok(());
            };
            let mean = span.get() as f64 / item.len.get() as f64;
            let Some(speed) = speed_from_decimal(mean) else {
                return Ok(());
            };
            let at = TickDur::new(item.offset.get());
            let Some(src_in) = speed.of(at).and_then(|moved| from.checked_sub(moved)) else {
                return Ok(());
            };
            return self.retime(
                id,
                &item,
                TimeMap::Linear { src_in, speed },
                item.len,
                ripple,
            );
        };
        let samples = samples(&knots, smooth);
        let mean = samples.last().map_or(1.0, |s| s.3).max(SLOWEST.display());
        let size = span.get().abs() as f64;
        let sign = if span < TickDur::ZERO { -1.0 } else { 1.0 };
        let Some(len) = TickLen::new((size / mean).round() as i64) else {
            return Ok(());
        };
        let l = len.get() as f64;
        let mut points: Vec<(TimePoint, f64, f64)> = Vec::new();
        for (x, before, after, used) in samples {
            let point = TimePoint {
                at: Tick::new(item.offset.get() + (x * l).round() as i64),
                source: Tick::new(from.get() + (sign * used * l).round() as i64),
            };
            if points
                .last()
                .is_some_and(|(p, _, _)| p.at >= point.at || p.source == point.source)
            {
                continue;
            }
            points.push((point, before, after));
        }
        // The last key is where the window ends, showing where the
        // source it covered ends, exactly.
        if let Some(last) = points.last_mut() {
            last.0.at = item.offset + len;
            last.0.source = from + span;
        }
        if points.len() < 2 {
            return Ok(());
        }
        let mut rest = Vec::with_capacity(points.len() - 1);
        for pair in points.windows(2) {
            let ((a, _, leave), (b, arrive, _)) = (pair[0], pair[1]);
            let run = (b.at - a.at).get() as f64;
            let secant = ((b.source - a.source).get() as f64 / run).abs();
            let curve = match (
                speed_from_decimal(sign * leave),
                speed_from_decimal(sign * arrive),
            ) {
                (Some(l), Some(r)) => {
                    let (alpha, beta) = (leave / secant, arrive / secant);
                    if (leave - arrive).abs() < 1e-9 && (leave - secant).abs() < 1e-6 {
                        TimeCurve::Linear
                    } else if alpha > 0.0 && beta > 0.0 && alpha * alpha + beta * beta <= 9.0 {
                        TimeCurve::Smooth {
                            leave: l,
                            arrive: r,
                        }
                    } else {
                        TimeCurve::Linear
                    }
                }
                _ => TimeCurve::Linear,
            };
            rest.push((curve, b));
        }
        let map = TimeMap::Keyed {
            first: points[0].0,
            rest,
        };
        self.retime(id, &item, map, len, ripple)
    }

    /// Puts a new map and length on an item, its keys keeping their share
    /// of it, and moves what follows on its lane: with ripple, by as much
    /// as its end moved either way; without, only to make room.
    fn retime(
        &mut self,
        id: ItemId,
        item: &Item,
        map: TimeMap,
        len: TickLen,
        ripple: bool,
    ) -> Result<(), CommandError> {
        let old_end = item.start + item.len;
        let old_len = item.len;
        self.update(id, |item| {
            if let Some(time) = map_mut(item) {
                *time = map;
            }
            rescale_keys(item, old_len, len);
            item.len = len;
        })?;
        let new_end = item.start + len;
        let moved = new_end - old_end;
        let covers = !super::place::is_free(self.d, item.lane, item.start, len, &[id]);
        if ripple || (moved > TickDur::ZERO && covers) {
            self.ripple(item.lane, old_end, moved, true, &[id])?;
        }
        Ok(())
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The document as it is made: its tables, the count its ids come from,
//! and the notes for the person. And the small readings every record
//! shares: a number held to what its field can mean, an easing, keys.

use concat_doc::file::Note;
use concat_doc::id::{RawId, TypedId, Writer};
use concat_doc::model::{Animated, Ease, Key, Model, Raw};
use concat_doc::{Tick, TickLen};
use serde_json::Value;

use crate::ImportError;
use crate::read::{Fields, IdWords};

/// The document being made.
pub(crate) struct Build {
    /// Its tables and values.
    pub(crate) model: Model,
    /// The last count given out.
    count: u64,
    /// What the person is told.
    pub(crate) notes: Vec<Note>,
    /// The id-shaped words of everything kept as text.
    pub(crate) words: IdWords,
}

impl Build {
    pub(crate) fn new() -> Build {
        Build {
            model: Model::default(),
            count: 0,
            notes: Vec::new(),
            words: IdWords::default(),
        }
    }

    /// A new id of any kind, from the document's one count.
    pub(crate) fn mint<I: TypedId>(&mut self) -> Result<I, ImportError> {
        self.count += 1;
        RawId::from_parts(Writer::LOCAL, self.count)
            .map(I::from_raw)
            .ok_or(ImportError::TooLarge)
    }

    /// The last count given out.
    pub(crate) fn count(&self) -> u64 {
        self.count
    }

    /// Tells the person something about a place in the project.
    pub(crate) fn note(&mut self, place: &str, what: impl Into<String>) {
        self.notes.push(Note {
            place: place.to_owned(),
            what: what.into(),
        });
    }

    /// Text to keep as it was, its words counted for the mint.
    pub(crate) fn raw(&mut self, value: &Value) -> Raw {
        self.words.raw(value)
    }

    /// A number held to `low..=high`, said when it had to be moved.
    pub(crate) fn within(
        &mut self,
        place: &str,
        what: &str,
        value: f64,
        low: f64,
        high: f64,
    ) -> f64 {
        let held = value.clamp(low, high);
        if held != value {
            self.note(
                place,
                format!(
                    "its {what} of {} is past what it can be, and is {} now",
                    show(value),
                    show(held)
                ),
            );
        }
        held
    }
}

/// A number for a note: short, as a person writes it.
pub(crate) fn show(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    format!("{rounded}")
}

/// An easing: four numbers, or one of the names an older file wrote.
/// What reads as neither is kept as it was.
pub(crate) fn ease(build: &mut Build, place: &str, value: Option<&Value>) -> Ease {
    let bezier = |x1: f64, y1: f64, x2: f64, y2: f64| Ease::Bezier { x1, y1, x2, y2 };
    match value {
        None | Some(Value::Null) => Ease::LINEAR,
        Some(Value::String(name)) => match name.as_str() {
            "linear" => Ease::LINEAR,
            "in" | "easeIn" => bezier(0.42, 0.0, 1.0, 1.0),
            "out" | "easeOut" => bezier(0.0, 0.0, 0.58, 1.0),
            "inOut" | "easeInOut" => bezier(0.42, 0.0, 0.58, 1.0),
            _ => {
                build.note(
                    place,
                    format!("a key's easing \"{name}\" is not known; it is kept as it was"),
                );
                Ease::Unknown(build.raw(&Value::String(name.clone())))
            }
        },
        Some(Value::Array(numbers)) if numbers.len() == 4 => {
            let n: Vec<Option<f64>> = numbers.iter().map(Value::as_f64).collect();
            match n.as_slice() {
                [Some(x1), Some(y1), Some(x2), Some(y2)] => {
                    let held = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
                    if held != (*x1, *x2) {
                        build.note(
                            place,
                            "a key's easing ran back in time; its times are held to the stretch between its keys",
                        );
                    }
                    bezier(held.0, *y1, held.1, *y2)
                }
                _ => unknown_ease(build, place, value),
            }
        }
        Some(_) => unknown_ease(build, place, value),
    }
}

fn unknown_ease(build: &mut Build, place: &str, value: Option<&Value>) -> Ease {
    build.note(
        place,
        "a key's easing could not be read; it is kept as it was",
    );
    Ease::Unknown(build.raw(value.unwrap_or(&Value::Null)))
}

/// Where a share of an item's length falls on its content time: `offset`
/// and that share of `len`, to the nearest tick.
pub(crate) fn at_share(offset: Tick, len: TickLen, share: f64) -> Option<Tick> {
    let into = (share * len.get() as f64).round();
    if !into.is_finite() || into.abs() > 9.0e18 {
        return None;
    }
    offset.checked_add(concat_doc::TickDur::new(into as i64))
}

/// One key of the earlier file: `at` a share of the clip, a value, an
/// easing. `None` for one that cannot be read, which the caller keeps as
/// it was and says.
pub(crate) fn one_key(
    build: &mut Build,
    place: &str,
    fields: &mut Fields<'_>,
    offset: Tick,
    len: TickLen,
) -> Option<Key<f64>> {
    if !fields.has("at") || !fields.has("value") {
        return None;
    }
    let at = fields.f64("at")?;
    let value = fields.f64("value")?;
    let at = at_share(offset, len, at)?;
    let ease = ease(build, place, fields.value("ease"));
    Some(Key { at, value, ease })
}

/// A list of keys of one number, each `at` a share of the clip, put on
/// the item's content time; what cannot be read is kept under `path`.
pub(crate) fn key_list(
    build: &mut Build,
    place: &str,
    what: &str,
    list: &[Value],
    offset: Tick,
    len: TickLen,
    kept: &mut crate::read::Kept,
    path: &str,
) -> Vec<Key<f64>> {
    let mut keys = Vec::with_capacity(list.len());
    for (at, value) in list.iter().enumerate() {
        let mut fields = Fields::of(value);
        match one_key(build, place, &mut fields, offset, len) {
            Some(key) => {
                kept.rest(&format!("{path}/{at}"), &fields);
                keys.push(key);
            }
            None => {
                build.note(
                    place,
                    format!("a key of its {what} could not be read; it is kept as it was"),
                );
                kept.keep(path, &at.to_string(), value.clone());
            }
        }
    }
    tidy_keys(build, place, what, &mut keys);
    keys
}

/// Keys put in order of their ticks, two at one tick made one (the later
/// of them in the file), said.
pub(crate) fn tidy_keys(build: &mut Build, place: &str, what: &str, keys: &mut Vec<Key<f64>>) {
    keys.sort_by_key(|key| key.at);
    let before = keys.len();
    // The later of two at one tick is the one kept: sorting is stable, so
    // it is the last of each run.
    let mut kept: Vec<Key<f64>> = Vec::with_capacity(keys.len());
    for key in keys.drain(..) {
        match kept.last_mut() {
            Some(last) if last.at == key.at => *last = key,
            _ => kept.push(key),
        }
    }
    if kept.len() < before {
        build.note(
            place,
            format!(
                "{} of its {what} keys fell on the same tick as another and were made one",
                before - kept.len()
            ),
        );
    }
    *keys = kept;
}

/// An animated number: its value, and keys made from the file's list,
/// each held to `low..=high` as the value is, said when one had to be.
pub(crate) fn animated(
    build: &mut Build,
    place: &str,
    what: &str,
    value: f64,
    mut keys: Vec<Key<f64>>,
    low: f64,
    high: f64,
) -> Animated<f64> {
    let value = build.within(place, what, value, low, high);
    let mut moved = false;
    for key in &mut keys {
        let held = key.value.clamp(low, high);
        moved |= held != key.value;
        key.value = held;
    }
    if moved {
        build.note(
            place,
            format!("some of its {what} keys were past what it can be, and are held to it now"),
        );
    }
    Animated { value, keys }
}

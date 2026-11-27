// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Reading the earlier file's records: fields taken one by one, so that
//! what is left over is known and kept; seconds as ticks; colours; and
//! the id-shaped words a kept text holds.

use std::collections::BTreeMap;

use concat_doc::model::{Color, Raw, Residual};
use concat_doc::{Tick, TickDur, Writer};
use serde_json::{Map, Value};

/// One record of the earlier file, read field by field. Each field read
/// is marked; what is never read, or is not of the kind it should be, is
/// left for [`Fields::rest`], so nothing is dropped without being seen.
pub(crate) struct Fields<'a> {
    map: Option<&'a Map<String, Value>>,
    used: Vec<&'a str>,
}

impl<'a> Fields<'a> {
    /// The record a value is; an empty one for a value that is no object.
    pub(crate) fn of(value: &'a Value) -> Fields<'a> {
        Fields {
            map: value.as_object(),
            used: Vec::new(),
        }
    }

    /// Whether the record has the field at all.
    pub(crate) fn has(&self, key: &str) -> bool {
        self.map.is_some_and(|map| map.contains_key(key))
    }

    /// The field, marked as read, whatever it holds.
    pub(crate) fn value(&mut self, key: &str) -> Option<&'a Value> {
        let (name, value) = self.map?.get_key_value(key)?;
        self.used.push(name.as_str());
        Some(value)
    }

    /// The field if `read` makes sense of it, marked as read only then.
    fn take<T>(&mut self, key: &str, read: impl FnOnce(&'a Value) -> Option<T>) -> Option<T> {
        let (name, value) = self.map?.get_key_value(key)?;
        let read = read(value)?;
        self.used.push(name.as_str());
        Some(read)
    }

    /// A number. `null` is read as no number, and marked.
    pub(crate) fn f64(&mut self, key: &str) -> Option<f64> {
        if self.null(key) {
            return None;
        }
        self.take(key, |v| v.as_f64().filter(|n| n.is_finite()))
    }

    /// A number, or `or` when there is none.
    pub(crate) fn f64_or(&mut self, key: &str, or: f64) -> f64 {
        self.f64(key).unwrap_or(or)
    }

    /// A whole number that is not below zero and fits 32 bits.
    pub(crate) fn u32(&mut self, key: &str) -> Option<u32> {
        if self.null(key) {
            return None;
        }
        self.take(key, |v| v.as_u64().and_then(|n| u32::try_from(n).ok()))
    }

    /// A string. `null` is read as none, and marked.
    pub(crate) fn str(&mut self, key: &str) -> Option<&'a str> {
        if self.null(key) {
            return None;
        }
        self.take(key, Value::as_str)
    }

    /// A string, or `or` when there is none.
    pub(crate) fn string_or(&mut self, key: &str, or: &str) -> String {
        self.str(key).unwrap_or(or).to_owned()
    }

    /// A yes or no. `null` is read as none, and marked.
    pub(crate) fn bool(&mut self, key: &str) -> Option<bool> {
        if self.null(key) {
            return None;
        }
        self.take(key, Value::as_bool)
    }

    /// A yes or no, or `or` when there is none.
    pub(crate) fn bool_or(&mut self, key: &str, or: bool) -> bool {
        self.bool(key).unwrap_or(or)
    }

    /// A list. `null` is read as an empty one, and marked.
    pub(crate) fn list(&mut self, key: &str) -> &'a [Value] {
        if self.null(key) {
            return &[];
        }
        self.take(key, |v| v.as_array().map(Vec::as_slice))
            .unwrap_or(&[])
    }

    /// An object, as a value to read with [`Fields::of`]. `null` is read
    /// as none, and marked.
    pub(crate) fn object(&mut self, key: &str) -> Option<&'a Value> {
        if self.null(key) {
            return None;
        }
        self.take(key, |v| v.is_object().then_some(v))
    }

    /// Whether the field is `null`; marked as read if it is.
    fn null(&mut self, key: &str) -> bool {
        match self.map.and_then(|map| map.get_key_value(key)) {
            Some((name, Value::Null)) => {
                self.used.push(name.as_str());
                true
            }
            _ => false,
        }
    }

    /// Marks fields as read without reading them: fields the earlier
    /// file kept that say nothing the new document needs.
    pub(crate) fn skip(&mut self, keys: &[&str]) {
        for key in keys {
            if let Some((name, _)) = self.map.and_then(|map| map.get_key_value(*key)) {
                self.used.push(name.as_str());
            }
        }
    }

    /// The fields that were not read, in the file's order.
    pub(crate) fn rest(&self) -> Vec<(&'a str, &'a Value)> {
        let Some(map) = self.map else {
            return Vec::new();
        };
        map.iter()
            .filter(|(name, _)| !self.used.contains(&name.as_str()))
            .map(|(name, value)| (name.as_str(), value))
            .collect()
    }
}

/// What a record kept that the new document has no field for, gathered
/// under one name, with the path from the record to each field.
#[derive(Default)]
pub(crate) struct Kept {
    fields: Map<String, Value>,
}

/// The one field a record's kept part is written under in the new file:
/// a name no record of the new format uses, so nothing kept can be read
/// back as something it is not.
pub(crate) const KEPT: &str = "imported";

impl Kept {
    /// Keeps what a record's fields left over, under `path` (empty for
    /// the record's own fields, `text` for those of its title).
    pub(crate) fn rest(&mut self, path: &str, fields: &Fields<'_>) {
        for (name, value) in fields.rest() {
            self.keep(path, name, value.clone());
        }
    }

    /// Keeps one value.
    pub(crate) fn keep(&mut self, path: &str, name: &str, value: Value) {
        let key = if path.is_empty() {
            name.to_owned()
        } else {
            format!("{path}/{name}")
        };
        self.fields.insert(key, value);
    }

    /// The names kept, for the note that says so.
    pub(crate) fn names(&self) -> Vec<&str> {
        self.fields.keys().map(String::as_str).collect()
    }

    /// Whether anything is kept.
    pub(crate) fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// As a residual of the new record, and the words of it for the mint.
    pub(crate) fn residual(self, words: &mut IdWords) -> Residual {
        if self.fields.is_empty() {
            return Residual::default();
        }
        let value = Value::Object(self.fields);
        Residual {
            fields: vec![(KEPT.to_owned(), words.raw(&value))],
            derived: None,
        }
    }
}

/// The highest count of each writer named by an id-shaped word in what is
/// kept as text: a word the new file's reader takes for an id, and so
/// counts the mint past when it reads the file back. The document is made
/// with its mint past them already, so a save and an open give it back
/// as it was.
#[derive(Default)]
pub(crate) struct IdWords {
    pub(crate) highest: BTreeMap<Writer, u64>,
}

impl IdWords {
    /// A value as text to keep, its words counted.
    pub(crate) fn raw(&mut self, value: &Value) -> Raw {
        self.count(value);
        Raw(value.to_string())
    }

    fn count(&mut self, value: &Value) {
        match value {
            Value::String(text) => self.word(text),
            Value::Array(items) => items.iter().for_each(|item| self.count(item)),
            Value::Object(map) => {
                for (key, item) in map {
                    self.word(key);
                    self.count(item);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    /// One word: one to four lower-case letters and a count, with a
    /// writer before a dot when there is one (`c12`, `fx3.40`).
    fn word(&mut self, text: &str) {
        let letters = text.bytes().take_while(u8::is_ascii_lowercase).count();
        if !(1..=4).contains(&letters) {
            return;
        }
        let number = |digits: &str| -> Option<u64> {
            let valid = !digits.is_empty()
                && digits.bytes().all(|b| b.is_ascii_digit())
                && (digits == "0" || !digits.starts_with('0'));
            valid.then(|| digits.parse().ok()).flatten()
        };
        let rest = &text[letters..];
        let parsed = match rest.split_once('.') {
            None => number(rest).map(|counter| (0, counter)),
            Some((writer, counter)) => number(writer).zip(number(counter)),
        };
        let Some((writer, counter)) = parsed else {
            return;
        };
        let Ok(writer) = u16::try_from(writer) else {
            return;
        };
        let highest = self.highest.entry(Writer(writer)).or_insert(0);
        *highest = counter.max(*highest);
    }
}

/// The tick nearest a number of seconds, as the document rounds one:
/// [`Tick::from_display_seconds`].
pub(crate) fn tick(seconds: f64) -> Option<Tick> {
    let tick = Tick::from_display_seconds(seconds)?;
    concat_doc::model::tick_in_range(tick).then_some(tick)
}

/// The same for a distance.
pub(crate) fn dur(seconds: f64) -> Option<TickDur> {
    let dur = TickDur::from_display_seconds(seconds)?;
    concat_doc::model::dur_in_range(dur).then_some(dur)
}

/// A colour written `#rrggbb` or `#rrggbbaa`, or a short `#rgb`.
pub(crate) fn color(text: &str) -> Option<Color> {
    let hex = text.trim().strip_prefix('#')?;
    let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    match hex.len() {
        6 => Some(Color::rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Color {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: byte(6)?,
        }),
        3 => {
            let nibble = |at: usize| {
                u8::from_str_radix(hex.get(at..=at)?, 16)
                    .ok()
                    .map(|n| n * 17)
            };
            Some(Color::rgb(nibble(0)?, nibble(1)?, nibble(2)?))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_read_in_every_spelling_the_file_used() {
        assert_eq!(color("#ff3b30"), Some(Color::rgb(255, 59, 48)));
        assert_eq!(
            color("#00000099"),
            Some(Color {
                r: 0,
                g: 0,
                b: 0,
                a: 0x99
            })
        );
        assert_eq!(color("#fff"), Some(Color::WHITE));
        assert_eq!(color("white"), None);
        assert_eq!(color(""), None);
    }

    #[test]
    fn id_shaped_words_count_as_the_reader_counts_them() {
        let mut words = IdWords::default();
        words.raw(&serde_json::json!({"c12": ["tl4", "T1", "mask-1", "fx3.40", "c012"]}));
        assert_eq!(words.highest.get(&Writer(0)), Some(&12));
        assert_eq!(words.highest.get(&Writer(3)), Some(&40));
    }

    #[test]
    fn fields_left_unread_or_of_the_wrong_kind_are_the_rest() {
        let value = serde_json::json!({"a": 1.5, "b": "two", "c": null, "d": true});
        let mut fields = Fields::of(&value);
        assert_eq!(fields.f64("a"), Some(1.5));
        assert_eq!(fields.f64("b"), None);
        assert_eq!(fields.f64("c"), None);
        let rest: Vec<&str> = fields.rest().into_iter().map(|(name, _)| name).collect();
        assert_eq!(rest, ["b", "d"]);
    }
}

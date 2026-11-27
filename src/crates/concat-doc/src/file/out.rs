// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Writing a file: JSON's spelling of each value, objects whose fields
//! this build does not know are put back into, and the seal of a record.

use std::hash::{DefaultHasher, Hasher};

use crate::model::{Raw, Residual};

/// A file being written.
///
/// One string, grown as the document is walked: nothing is built and
/// then copied. A record's residual is followed as the record is
/// written, so a field this build does not know is put back into the
/// object it came from, after the fields it knows.
pub(crate) struct Out<'d> {
    /// The text so far.
    pub(crate) text: String,
    /// The residual of the record being written, when it has fields.
    fields: Option<&'d [(String, Raw)]>,
    /// Where in that record the writer is: escaped names joined by `/`,
    /// empty at the record itself. Kept only while `fields` is.
    path: String,
    /// The seal of the last record finished: a hash of its text up to
    /// what it marked as derived.
    pub(crate) seal: u64,
}

/// An object being written.
#[must_use]
pub(crate) struct Obj {
    first: bool,
    /// For an object whose every key starts a line of its own: what the
    /// line starts with before the key.
    lines: Option<&'static str>,
}

/// A record being written: where it began, and the record it is inside.
#[must_use]
pub(crate) struct Rec<'d> {
    /// The record's object, for its keys.
    pub(crate) obj: Obj,
    start: usize,
    outer: Option<&'d [(String, Raw)]>,
    path: String,
    residual: &'d Residual,
}

impl<'d> Out<'d> {
    /// A writer with room for about `bytes`.
    pub(crate) fn new(bytes: usize) -> Out<'d> {
        Out {
            text: String::with_capacity(bytes),
            fields: None,
            path: String::new(),
            seal: 0,
        }
    }

    /// Whether the record being written has fields this build does not
    /// know, so that the path is being followed.
    pub(crate) fn tracking(&self) -> bool {
        self.fields.is_some()
    }

    // --- Values ---------------------------------------------------------

    /// Text, as it was read.
    pub(crate) fn raw(&mut self, raw: &str) {
        self.text.push_str(raw);
    }

    /// A string, quoted and escaped.
    pub(crate) fn string(&mut self, text: &str) {
        self.text.reserve(text.len() + 2);
        self.text.push('"');
        let mut from = 0;
        for (at, byte) in text.bytes().enumerate() {
            let escape = match byte {
                b'"' => "\\\"",
                b'\\' => "\\\\",
                b'\n' => "\\n",
                b'\r' => "\\r",
                b'\t' => "\\t",
                0x08 => "\\b",
                0x0c => "\\f",
                0..0x20 => "",
                _ => continue,
            };
            self.text.push_str(&text[from..at]);
            if escape.is_empty() {
                self.text.push_str(&format!("\\u{byte:04x}"));
            } else {
                self.text.push_str(escape);
            }
            from = at + 1;
        }
        self.text.push_str(&text[from..]);
        self.text.push('"');
    }

    /// A whole number.
    pub(crate) fn int(&mut self, value: impl Into<i128>) {
        use std::fmt::Write;
        let _ = write!(self.text, "{}", value.into());
    }

    /// A number with a fraction, in the fewest digits that read back as
    /// the same number: `1`, `0.1`, `1e-7`, `-0`.
    ///
    /// A number that is not finite has no JSON spelling and is written
    /// `null`, which no field reads as a number; a document holds none
    /// (rule 33).
    pub(crate) fn float(&mut self, value: f64) {
        use std::fmt::Write;
        if !value.is_finite() {
            self.text.push_str("null");
            return;
        }
        let from = self.text.len();
        let _ = write!(self.text, "{value:?}");
        if self.text[from..].ends_with(".0") {
            self.text.truncate(self.text.len() - 2);
        }
    }

    /// `true` or `false`.
    pub(crate) fn bool(&mut self, value: bool) {
        self.text.push_str(if value { "true" } else { "false" });
    }

    /// `null`.
    pub(crate) fn null(&mut self) {
        self.text.push_str("null");
    }

    /// A list, each element written by `each`.
    pub(crate) fn list<T>(&mut self, items: &[T], mut each: impl FnMut(&mut Self, &T)) {
        self.text.push('[');
        for (at, item) in items.iter().enumerate() {
            if at > 0 {
                self.text.push(',');
            }
            each(self, item);
        }
        self.text.push(']');
    }

    // --- Objects --------------------------------------------------------

    /// Opens an object.
    pub(crate) fn open(&mut self) -> Obj {
        self.text.push('{');
        Obj {
            first: true,
            lines: None,
        }
    }

    /// Opens an object whose every key starts a line, after `indent`;
    /// with no indent the first stays beside the brace, as the file's
    /// first line does: `{"concat-project":1,`.
    pub(crate) fn open_lines(&mut self, indent: &'static str) -> Obj {
        self.text.push('{');
        Obj {
            first: true,
            lines: Some(indent),
        }
    }

    /// Writes a key of an object, for its value to follow.
    pub(crate) fn key(&mut self, obj: &mut Obj, key: &str) {
        if !obj.first {
            self.text.push(',');
        }
        if let Some(indent) = obj.lines
            && !(obj.first && indent.is_empty())
        {
            self.text.push('\n');
            self.text.push_str(indent);
        }
        obj.first = false;
        self.string(key);
        self.text.push(':');
    }

    /// A field whose value is a part of the record: its fields this
    /// build does not know are found under its name.
    pub(crate) fn part(&mut self, obj: &mut Obj, key: &str, write: impl FnOnce(&mut Self)) {
        self.key(obj, key);
        self.within(key, write);
    }

    /// Writes with the path one name deeper.
    pub(crate) fn within(&mut self, name: &str, write: impl FnOnce(&mut Self)) {
        if self.fields.is_none() {
            write(self);
            return;
        }
        let depth = self.path.len();
        if depth > 0 {
            self.path.push('/');
        }
        escape_into(&mut self.path, name);
        write(self);
        self.path.truncate(depth);
    }

    /// Whether the record has fields this build does not know under
    /// this name, so that a part at its default must still be written
    /// to hold them.
    pub(crate) fn holds_under(&self, name: &str) -> bool {
        let Some(fields) = self.fields else {
            return false;
        };
        let mut prefix = self.path.clone();
        if !prefix.is_empty() {
            prefix.push('/');
        }
        escape_into(&mut prefix, name);
        prefix.push('/');
        fields.iter().any(|(path, _)| path.starts_with(&prefix))
    }

    /// Whether the record has fields this build does not know inside the
    /// value being written, so that a number with keys or none must be
    /// written as an object to hold them.
    pub(crate) fn holds_here(&self) -> bool {
        let Some(fields) = self.fields else {
            return false;
        };
        !self.path.is_empty()
            && fields.iter().any(|(path, _)| {
                path.strip_prefix(self.path.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
            })
    }

    /// Closes an object, after the record's fields this build does not
    /// know that belong in it.
    pub(crate) fn close(&mut self, mut obj: Obj) {
        // The record's own are put in by `end`.
        if let Some(fields) = self.fields.filter(|_| !self.path.is_empty()) {
            for (path, raw) in fields {
                if let Some(name) = leaf(path, &self.path) {
                    self.key(&mut obj, &unescape(name));
                    self.raw(raw.as_str());
                }
            }
        }
        self.finish(obj);
    }

    /// The object's last brace, on a line of its own if its keys are.
    fn finish(&mut self, obj: Obj) {
        if obj.lines.is_some() && !obj.first {
            self.text.push('\n');
        }
        self.text.push('}');
    }

    /// Opens a record: an object with a residual of its own.
    pub(crate) fn record(&mut self, residual: &'d Residual) -> Rec<'d> {
        let start = self.text.len();
        let obj = self.open();
        self.enter(start, obj, residual)
    }

    /// Opens a record whose every key starts a line: the document.
    pub(crate) fn record_lines(&mut self, residual: &'d Residual) -> Rec<'d> {
        let start = self.text.len();
        let obj = self.open_lines("");
        self.enter(start, obj, residual)
    }

    fn enter(&mut self, start: usize, obj: Obj, residual: &'d Residual) -> Rec<'d> {
        let outer = self.fields.take();
        let path = std::mem::take(&mut self.path);
        if !residual.fields.is_empty() {
            self.fields = Some(&residual.fields);
        }
        Rec {
            obj,
            start,
            outer,
            path,
            residual,
        }
    }

    /// Closes a record: its fields this build does not know, then what
    /// the file marked as derived, if the record is as it was read.
    pub(crate) fn end(&mut self, rec: Rec<'d>) {
        let Rec {
            mut obj,
            start,
            outer,
            path,
            residual,
        } = rec;
        if let Some(fields) = self.fields {
            for (path, raw) in fields {
                if !path.contains('/') {
                    self.key(&mut obj, &unescape(path));
                    self.raw(raw.as_str());
                }
            }
        }
        self.fields = outer;
        self.path = path;
        let mut hasher = DefaultHasher::new();
        hasher.write(&self.text.as_bytes()[start..]);
        self.seal = hasher.finish();
        if let Some(derived) = &residual.derived
            && derived.seal == self.seal
        {
            self.key(&mut obj, "derived");
            self.raw(derived.raw.as_str());
        }
        self.finish(obj);
    }
}

/// The name of a field at `path` if it belongs directly in the object at
/// `object`, both paths from the record.
fn leaf<'p>(path: &'p str, object: &str) -> Option<&'p str> {
    let rest = if object.is_empty() {
        path
    } else {
        path.strip_prefix(object)?.strip_prefix('/')?
    };
    (!rest.contains('/')).then_some(rest)
}

/// A name as one step of a path: `~` as `~0` and `/` as `~1`.
pub(crate) fn escape_into(path: &mut String, name: &str) {
    for c in name.chars() {
        match c {
            '~' => path.push_str("~0"),
            '/' => path.push_str("~1"),
            c => path.push(c),
        }
    }
}

/// One step of a path back as the name it was.
pub(crate) fn unescape(step: &str) -> String {
    step.replace("~1", "/").replace("~0", "~")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Derived;

    #[test]
    fn strings_are_escaped_as_json_has_them() {
        let mut out = Out::new(0);
        out.string("a\"b\\c\n\r\t\u{8}\u{c}\u{1}é/");
        assert_eq!(out.text, r#""a\"b\\c\n\r\t\b\f\u0001é/""#);
    }

    #[test]
    fn a_number_is_written_in_the_fewest_digits_that_read_back_as_it() {
        let mut out = Out::new(0);
        for value in [1.0, 0.1, -0.0, 1e300, 1.5e-7, 0.09, 1e16, 123.25, f64::NAN] {
            out.float(value);
            out.text.push(' ');
        }
        assert_eq!(out.text, "1 0.1 -0 1e300 1.5e-7 0.09 1e16 123.25 null ");
        for text in out
            .text
            .split(' ')
            .filter(|t| !t.is_empty() && *t != "null")
        {
            assert!(super::super::json::parse(text).is_ok(), "{text}");
        }
    }

    #[test]
    fn a_path_has_one_reading() {
        let mut path = String::new();
        escape_into(&mut path, "a/b~c");
        assert_eq!(path, "a~1b~0c");
        assert_eq!(unescape(&path), "a/b~c");
        assert_eq!(unescape("~01"), "~1");
        assert_eq!(leaf("picture/glow", "picture"), Some("glow"));
        assert_eq!(leaf("picture/scale/tilt", "picture"), None);
        assert_eq!(leaf("glow", ""), Some("glow"));
        assert_eq!(leaf("pictures/glow", "picture"), None);
    }

    #[test]
    fn unknown_fields_go_back_into_their_objects_and_derived_only_onto_what_is_unchanged() {
        let residual = Residual {
            fields: vec![
                ("glow".into(), Raw::from("1")),
                ("picture/tint".into(), Raw::from("\"red\"")),
                ("a~1b".into(), Raw::from("[ 2 ]")),
            ],
            derived: None,
        };
        fn write(residual: &Residual) -> (String, u64) {
            let mut out = Out::new(0);
            let mut rec = out.record(residual);
            out.key(&mut rec.obj, "lane");
            out.string("t1");
            out.part(&mut rec.obj, "picture", |out| {
                let mut obj = out.open();
                out.key(&mut obj, "scale");
                out.float(2.0);
                out.close(obj);
            });
            out.end(rec);
            (out.text, out.seal)
        }
        let (text, seal) = write(&residual);
        assert_eq!(
            text,
            r#"{"lane":"t1","picture":{"scale":2,"tint":"red"},"glow":1,"a/b":[ 2 ]}"#
        );
        // A seal of the record as it is keeps what was derived from it.
        let mut sealed = residual.clone();
        sealed.derived = Some(Derived {
            raw: Raw::from("{\"thumb\":1}"),
            seal,
        });
        let (text, _) = write(&sealed);
        assert!(text.ends_with(r#""derived":{"thumb":1}}"#), "{text}");
        // Any other seal drops it.
        if let Some(derived) = sealed.derived.as_mut() {
            derived.seal ^= 1;
        }
        assert!(!write(&sealed).0.contains("derived"));
    }
}

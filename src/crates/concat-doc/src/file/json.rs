// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! JSON read into a tree that remembers where each value came from.
//!
//! Every value of the tree keeps the exact text it was read from, so
//! that what this build cannot read is kept as that text, byte for
//! byte, and written back unchanged: a number keeps its spelling, an
//! object its key order and any key it says twice. Strings without
//! escapes are borrowed from the file, and numbers are not converted
//! until a reader asks for one, so reading a file is one pass over it.
//!
//! It reads JSON as RFC 8259 has it, and refuses everything else with
//! the place it stopped: no comments, no trailing commas, no byte-order
//! mark, no lone surrogate in an escape. Nesting deeper than
//! [`MAX_DEPTH`] is refused rather than followed, so a file cannot run
//! the reader out of stack.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;

/// How deep arrays and objects may nest.
pub(crate) const MAX_DEPTH: usize = 128;

/// One value of a file, and the text it was read from.
#[derive(Debug)]
pub(crate) struct Node<'a> {
    /// The value's text, exactly as the file has it.
    pub text: &'a str,
    /// What it is.
    pub kind: Kind<'a>,
}

/// What a value is.
#[derive(Debug)]
pub(crate) enum Kind<'a> {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number. Its spelling is the node's text.
    Number,
    /// A string, its escapes undone.
    String(Cow<'a, str>),
    /// An array.
    Array(Vec<Node<'a>>),
    /// An object.
    Object(Object<'a>),
}

/// An object's members, in the order of the file.
#[derive(Debug)]
pub(crate) struct Object<'a> {
    /// Each key, its escapes undone, and its value.
    pub members: Vec<(Cow<'a, str>, Node<'a>)>,
    /// Whether some key is said twice.
    pub twice: bool,
}

/// Where and why a text is not JSON.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct NotJson {
    /// The line, from 1.
    pub line: usize,
    /// The character in the line, from 1.
    pub column: usize,
    /// What was wrong.
    pub what: &'static str,
}

impl fmt::Display for NotJson {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at line {}, character {}",
            self.what, self.line, self.column
        )
    }
}

/// Reads a whole text as one JSON value.
pub(crate) fn parse(text: &str) -> Result<Node<'_>, NotJson> {
    let mut parser = Parser {
        text,
        bytes: text.as_bytes(),
        at: 0,
        depth: 0,
    };
    parser.space();
    let read = parser.value().and_then(|node| {
        parser.space();
        if parser.at < parser.bytes.len() {
            return Err((parser.at, AFTER));
        }
        Ok(node)
    });
    read.map_err(|(at, what)| parser.error(at, what))
}

/// A failure: where it happened, and why.
type Step<T> = Result<T, (usize, &'static str)>;

const NOT_JSON: &str = "something that is not JSON";
const UNENDED: &str = "a string that does not end";
const CONTROL: &str = "a control character inside a string";
const ESCAPE: &str = "a backslash that is not an escape";
const NUMBER: &str = "a number that is not one";
const DEEP: &str = "nested too deeply";
const KEY: &str = "an object's key that is not a string";
const COLON: &str = "a key without a colon after it";
const COMMA: &str = "two values without a comma between them";
const AFTER: &str = "more after the end of the file's value";

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    at: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn error(&self, at: usize, what: &'static str) -> NotJson {
        let at = at.min(self.bytes.len());
        let before = &self.text[..floor_char(self.text, at)];
        let line = before.bytes().filter(|b| *b == b'\n').count() + 1;
        let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        let what = if at == self.bytes.len() && what != UNENDED {
            "the text ends too soon"
        } else {
            what
        };
        NotJson { line, column, what }
    }

    fn space(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.bytes.get(self.at) {
            self.at += 1;
        }
    }

    fn value(&mut self) -> Step<Node<'a>> {
        let start = self.at;
        let kind = match self.bytes.get(self.at) {
            Some(b'{') => self.object()?,
            Some(b'[') => self.array()?,
            Some(b'"') => Kind::String(self.string()?),
            Some(b't') => self.word("true", Kind::Bool(true))?,
            Some(b'f') => self.word("false", Kind::Bool(false))?,
            Some(b'n') => self.word("null", Kind::Null)?,
            Some(b'-' | b'0'..=b'9') => self.number()?,
            _ => return Err((self.at, NOT_JSON)),
        };
        Ok(Node {
            text: &self.text[start..self.at],
            kind,
        })
    }

    fn word(&mut self, word: &str, kind: Kind<'a>) -> Step<Kind<'a>> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(kind)
        } else {
            Err((self.at, NOT_JSON))
        }
    }

    fn digits(&mut self) -> usize {
        let from = self.at;
        while let Some(b'0'..=b'9') = self.bytes.get(self.at) {
            self.at += 1;
        }
        self.at - from
    }

    fn number(&mut self) -> Step<Kind<'a>> {
        let start = self.at;
        if self.bytes.get(self.at) == Some(&b'-') {
            self.at += 1;
        }
        match self.bytes.get(self.at) {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err((start, NUMBER)),
        }
        if self.bytes.get(self.at) == Some(&b'.') {
            self.at += 1;
            if self.digits() == 0 {
                return Err((start, NUMBER));
            }
        }
        if let Some(b'e' | b'E') = self.bytes.get(self.at) {
            self.at += 1;
            if let Some(b'+' | b'-') = self.bytes.get(self.at) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return Err((start, NUMBER));
            }
        }
        Ok(Kind::Number)
    }

    fn string(&mut self) -> Step<Cow<'a, str>> {
        let open = self.at;
        self.at += 1;
        let start = self.at;
        // Borrowed until the first escape.
        loop {
            match self.bytes.get(self.at) {
                None => return Err((open, UNENDED)),
                Some(b'"') => {
                    let text = &self.text[start..self.at];
                    self.at += 1;
                    return Ok(Cow::Borrowed(text));
                }
                Some(b'\\') => break,
                Some(byte) if *byte < 0x20 => return Err((self.at, CONTROL)),
                Some(_) => self.at += 1,
            }
        }
        let mut owned = String::from(&self.text[start..self.at]);
        loop {
            match self.bytes.get(self.at) {
                None => return Err((open, UNENDED)),
                Some(b'"') => {
                    self.at += 1;
                    return Ok(Cow::Owned(owned));
                }
                Some(b'\\') => owned.push(self.escape()?),
                Some(byte) if *byte < 0x20 => return Err((self.at, CONTROL)),
                Some(_) => {
                    // A run up to the next quote, backslash or control
                    // character, copied whole: the text is UTF-8 and the
                    // run ends on an ASCII byte, so it is whole characters.
                    let from = self.at;
                    while let Some(byte) = self.bytes.get(self.at) {
                        if *byte == b'"' || *byte == b'\\' || *byte < 0x20 {
                            break;
                        }
                        self.at += 1;
                    }
                    owned.push_str(&self.text[from..self.at]);
                }
            }
        }
    }

    /// One escape, from its backslash.
    fn escape(&mut self) -> Step<char> {
        let escape = self.at;
        let bad = (escape, ESCAPE);
        self.at += 1;
        let plain = match self.bytes.get(self.at) {
            Some(b'"') => '"',
            Some(b'\\') => '\\',
            Some(b'/') => '/',
            Some(b'b') => '\u{8}',
            Some(b'f') => '\u{c}',
            Some(b'n') => '\n',
            Some(b'r') => '\r',
            Some(b't') => '\t',
            Some(b'u') => {
                self.at += 1;
                let unit = self.hex4().ok_or(bad)?;
                let code = match unit {
                    0xD800..=0xDBFF => {
                        // A high surrogate must be followed by an escaped
                        // low one.
                        if !self.bytes[self.at..].starts_with(b"\\u") {
                            return Err(bad);
                        }
                        self.at += 2;
                        let low = self.hex4().ok_or(bad)?;
                        if !(0xDC00..=0xDFFF).contains(&low) {
                            return Err(bad);
                        }
                        0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00)
                    }
                    0xDC00..=0xDFFF => return Err(bad),
                    unit => unit,
                };
                return char::from_u32(code).ok_or(bad);
            }
            _ => return Err(bad),
        };
        self.at += 1;
        Ok(plain)
    }

    /// Four hex digits, as a number.
    fn hex4(&mut self) -> Option<u32> {
        let digits = self.bytes.get(self.at..self.at + 4)?;
        let mut unit = 0;
        for digit in digits {
            unit = unit * 16 + char::from(*digit).to_digit(16)?;
        }
        self.at += 4;
        Some(unit)
    }

    fn deeper(&mut self) -> Step<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err((self.at, DEEP));
        }
        self.at += 1;
        self.space();
        Ok(())
    }

    fn array(&mut self) -> Step<Kind<'a>> {
        self.deeper()?;
        let mut items = Vec::new();
        if self.bytes.get(self.at) != Some(&b']') {
            loop {
                items.push(self.value()?);
                self.space();
                match self.bytes.get(self.at) {
                    Some(b',') => {
                        self.at += 1;
                        self.space();
                    }
                    Some(b']') => break,
                    _ => return Err((self.at, COMMA)),
                }
            }
        }
        self.at += 1;
        self.depth -= 1;
        Ok(Kind::Array(items))
    }

    fn object(&mut self) -> Step<Kind<'a>> {
        self.deeper()?;
        let mut members = Vec::new();
        if self.bytes.get(self.at) != Some(&b'}') {
            loop {
                if self.bytes.get(self.at) != Some(&b'"') {
                    return Err((self.at, KEY));
                }
                let key = self.string()?;
                self.space();
                if self.bytes.get(self.at) != Some(&b':') {
                    return Err((self.at, COLON));
                }
                self.at += 1;
                self.space();
                let value = self.value()?;
                members.push((key, value));
                self.space();
                match self.bytes.get(self.at) {
                    Some(b',') => {
                        self.at += 1;
                        self.space();
                    }
                    Some(b'}') => break,
                    _ => return Err((self.at, COMMA)),
                }
            }
        }
        self.at += 1;
        self.depth -= 1;
        let twice = said_twice(&members);
        Ok(Kind::Object(Object { members, twice }))
    }
}

/// Whether some key of an object is said twice. A record's few keys are
/// compared pairwise; a table's many are hashed.
fn said_twice(members: &[(Cow<'_, str>, Node<'_>)]) -> bool {
    if members.len() <= 16 {
        return members
            .iter()
            .enumerate()
            .any(|(at, (key, _))| members[..at].iter().any(|(other, _)| other == key));
    }
    let mut seen = HashSet::with_capacity(members.len());
    !members.iter().all(|(key, _)| seen.insert(key.as_ref()))
}

/// The start of the character `at` is in, so a text can be cut there.
fn floor_char(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

impl<'a> Node<'a> {
    /// The object, if this is one.
    pub(crate) fn object(&self) -> Option<&Object<'a>> {
        match &self.kind {
            Kind::Object(object) => Some(object),
            _ => None,
        }
    }

    /// The items, if this is an array.
    pub(crate) fn array(&self) -> Option<&[Node<'a>]> {
        match &self.kind {
            Kind::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The string, if this is one.
    pub(crate) fn str(&self) -> Option<&str> {
        match &self.kind {
            Kind::String(text) => Some(text),
            _ => None,
        }
    }

    /// Calls `each` on every string of this value, keys included.
    pub(crate) fn strings(&self, each: &mut impl FnMut(&str)) {
        match &self.kind {
            Kind::String(text) => each(text),
            Kind::Array(items) => items.iter().for_each(|item| item.strings(each)),
            Kind::Object(object) => {
                for (key, value) in &object.members {
                    each(key);
                    value.strings(each);
                }
            }
            Kind::Null | Kind::Bool(_) | Kind::Number => {}
        }
    }
}

impl<'a> Object<'a> {
    /// The value of a key, if the object has it: the first, if it has it
    /// twice.
    pub(crate) fn get(&self, key: &str) -> Option<&Node<'a>> {
        self.members
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_value_keeps_the_text_it_was_read_from() {
        let text = r#" { "a" : [ 1.50 , -0 , 2E+3 ] , "b":"x\u0041" , "c" : {"d":null} } "#;
        let node = parse(text).unwrap();
        let object = node.object().unwrap();
        assert_eq!(node.text, text.trim());
        let a = object.get("a").unwrap();
        assert_eq!(a.text, "[ 1.50 , -0 , 2E+3 ]");
        let numbers: Vec<&str> = a.array().unwrap().iter().map(|n| n.text).collect();
        assert_eq!(numbers, ["1.50", "-0", "2E+3"]);
        assert_eq!(object.get("b").unwrap().text, r#""x\u0041""#);
        assert_eq!(object.get("b").unwrap().str(), Some("xA"));
        assert_eq!(object.get("c").unwrap().text, r#"{"d":null}"#);
    }

    #[test]
    fn strings_borrow_until_an_escape_and_undo_every_escape() {
        let node = parse(r#"["plain","a\"b\\c\/d\b\f\n\r\t","é\u00e9\ud83d\ude00x"]"#).unwrap();
        let items = node.array().unwrap();
        assert!(matches!(
            &items[0].kind,
            Kind::String(Cow::Borrowed("plain"))
        ));
        assert_eq!(items[1].str(), Some("a\"b\\c/d\u{8}\u{c}\n\r\t"));
        assert_eq!(items[2].str(), Some("éé😀x"));
    }

    #[test]
    fn keys_said_twice_are_kept_in_order_and_noticed() {
        let node = parse(r#"{"a":1,"b":2,"a":3}"#).unwrap();
        let object = node.object().unwrap();
        assert!(object.twice);
        let keys: Vec<(&str, &str)> = object
            .members
            .iter()
            .map(|(k, v)| (k.as_ref(), v.text))
            .collect();
        assert_eq!(keys, [("a", "1"), ("b", "2"), ("a", "3")]);
        assert_eq!(object.get("a").unwrap().text, "1");
        assert!(!parse(r#"{"a":1,"b":2}"#).unwrap().object().unwrap().twice);
        // A table's many keys are hashed, to the same answer.
        let many: Vec<String> = (0..40).map(|n| format!("\"k{n}\":{n}")).collect();
        let once = format!("{{{}}}", many.join(","));
        assert!(!parse(&once).unwrap().object().unwrap().twice);
        let again = format!("{{{},\"k7\":0}}", many.join(","));
        assert!(parse(&again).unwrap().object().unwrap().twice);
    }

    #[test]
    fn what_is_not_json_is_refused_with_its_place() {
        for (text, line, column, what) in [
            ("", 1, 1, "the text ends too soon"),
            ("{", 1, 2, "the text ends too soon"),
            ("{\"a\":1,}", 1, 8, KEY),
            ("[1,]", 1, 4, NOT_JSON),
            ("[01]", 1, 3, COMMA),
            ("[1.]", 1, 2, NUMBER),
            ("[1e]", 1, 2, NUMBER),
            ("[-]", 1, 2, NUMBER),
            ("[.5]", 1, 2, NOT_JSON),
            ("[+1]", 1, 2, NOT_JSON),
            ("\"a", 1, 1, UNENDED),
            ("\"\\x\"", 1, 2, ESCAPE),
            ("\"\\ud800\"", 1, 2, ESCAPE),
            ("\"\\udc00\"", 1, 2, ESCAPE),
            ("\"\\ud800\\u0041\"", 1, 2, ESCAPE),
            ("\"\\u12\"", 1, 2, ESCAPE),
            ("\"a\nb\"", 1, 3, CONTROL),
            ("tru", 1, 1, NOT_JSON),
            ("nul", 1, 1, NOT_JSON),
            ("{\"a\" 1}", 1, 6, COLON),
            ("{1:2}", 1, 2, KEY),
            ("[1]\n x", 2, 2, AFTER),
            ("\u{feff}[]", 1, 1, NOT_JSON),
            ("[1] [2]", 1, 5, AFTER),
            ("// no\n[]", 1, 1, NOT_JSON),
            ("NaN", 1, 1, NOT_JSON),
            ("[\"é\" x]", 1, 6, COMMA),
            ("[1", 1, 3, "the text ends too soon"),
        ] {
            let error = parse(text).expect_err(text);
            assert_eq!(
                (error.line, error.column, error.what),
                (line, column, what),
                "{text:?}: {error}"
            );
        }
    }

    #[test]
    fn nesting_past_the_limit_is_refused_and_not_followed() {
        let deep = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(parse(&deep(MAX_DEPTH)).is_ok());
        let error = parse(&deep(MAX_DEPTH + 1)).unwrap_err();
        assert_eq!(error.what, DEEP);
        // Far past it, without running out of stack.
        assert!(parse(&deep(1_000_000)).is_err());
        let objects = format!(
            "{}1{}",
            "{\"a\":".repeat(MAX_DEPTH + 1),
            "}".repeat(MAX_DEPTH + 1)
        );
        assert_eq!(parse(&objects).unwrap_err().what, DEEP);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(4000))]

        /// Short texts of the characters JSON is made of: what one parser
        /// takes, so does the other.
        #[test]
        fn it_takes_what_another_json_reader_takes(
            text in "[\\[\\]{}\":,0-9.eE+\\- \\\\ntrufalsu\u{e9}]{0,24}",
        ) {
            let ours = parse(&text).is_ok();
            let theirs = serde_json::from_str::<serde_json::Value>(&text);
            // A number past a double's range, such as 1e309, is JSON all
            // the same: the other reader refuses it because it would hold
            // it as a double, while this one keeps its text, so a value
            // written by a later build survives. That is the one
            // difference allowed.
            let out_of_range = matches!(&theirs, Err(error) if error.to_string().starts_with("number out of range"));
            if !out_of_range {
                proptest::prop_assert_eq!(ours, theirs.is_ok(), "{:?}", text);
            }
        }

        /// Any JSON another program writes reads to the same strings and
        /// keeps each value's text.
        #[test]
        fn what_another_program_writes_reads_back(
            value in proptest::arbitrary::any::<(String, Vec<(String, i64)>, Vec<f64>, bool)>(),
        ) {
            let (name, pairs, numbers, flag) = value;
            let numbers: Vec<f64> = numbers.into_iter().filter(|n| n.is_finite()).collect();
            let json = serde_json::json!({ "name": name, "pairs": pairs, "numbers": numbers, "flag": flag });
            let text = serde_json::to_string(&json).unwrap();
            let node = parse(&text).unwrap();
            let object = node.object().unwrap();
            proptest::prop_assert_eq!(object.get("name").unwrap().str(), Some(name.as_str()));
            let read: Vec<f64> = object.get("numbers").unwrap().array().unwrap().iter()
                .map(|n| n.text.parse::<f64>().unwrap()).collect();
            proptest::prop_assert_eq!(read, numbers);
            proptest::prop_assert_eq!(node.text, text.as_str());
        }
    }

    #[test]
    fn every_string_of_a_value_is_visited_keys_included() {
        let node = parse(r#"{"a":["b",{"c":"d"}],"e":1}"#).unwrap();
        let mut seen = Vec::new();
        node.strings(&mut |s| seen.push(s.to_owned()));
        assert_eq!(seen, ["a", "b", "c", "d", "e"]);
    }
}

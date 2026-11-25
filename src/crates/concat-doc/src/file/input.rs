// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Reading a file's values: what was wrong with one, and where the
//! fields this build does not know are put.

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::id::{RawId, Writer};
use crate::model::Raw;

use super::json::{Kind, Node, Object};
use super::out::escape_into;
use super::{FEATURES, Note};

/// Why a value could not be read.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Bad {
    /// It is not what its field holds.
    Wrong {
        /// The fields that lead to it, the innermost first.
        at: Vec<String>,
        /// What is wrong, in plain words.
        what: Cow<'static, str>,
    },
    /// Its record needs features this build does not have.
    Needs(Vec<String>),
}

impl Bad {
    /// The same, one field further out.
    pub(crate) fn at(mut self, name: &str) -> Bad {
        if let Bad::Wrong { at, .. } = &mut self {
            at.push(name.to_owned());
        }
        self
    }

    /// What is wrong, in plain words.
    pub(crate) fn words(&self) -> String {
        match self {
            Bad::Wrong { at, what } if at.is_empty() => what.to_string(),
            Bad::Wrong { at, what } => {
                let path: Vec<&str> = at.iter().rev().map(String::as_str).collect();
                format!("`{}` {what}", path.join("."))
            }
            Bad::Needs(features) => {
                let names: Vec<String> = features.iter().map(|f| format!("\"{f}\"")).collect();
                format!(
                    "it needs {}, which this build does not have",
                    names.join(" and ")
                )
            }
        }
    }
}

/// What reading gives back.
pub(crate) type R<T> = Result<T, Bad>;

/// A value that is wrong.
pub(crate) fn wrong<T>(what: impl Into<Cow<'static, str>>) -> R<T> {
    Err(Bad::Wrong {
        at: Vec::new(),
        what: what.into(),
    })
}

/// What a reading is in the middle of: the fields this build does not
/// know of the record being read, and what has been said so far.
pub(crate) struct Cx {
    /// The record's fields this build does not know, by their path from
    /// the part being read.
    pub(crate) fields: Vec<(String, Raw)>,
    /// What the person is told.
    pub(crate) notes: Vec<Note>,
    /// What is being read, for the notes: "item c12".
    pub(crate) here: String,
    /// For each writer, the highest count of an id seen in text kept
    /// unread: an id this build cannot see must still never be made
    /// again.
    pub(crate) unseen: BTreeMap<Writer, u64>,
}

impl Cx {
    pub(crate) fn new() -> Cx {
        Cx {
            fields: Vec::new(),
            notes: Vec::new(),
            here: String::new(),
            unseen: BTreeMap::new(),
        }
    }

    /// A value kept as its text, which is looked through for ids.
    pub(crate) fn raw(&mut self, node: &Node<'_>) -> Raw {
        let unseen = &mut self.unseen;
        node.strings(&mut |text| {
            if let Some(id) = id_shaped(text) {
                let count = unseen.entry(id.writer()).or_insert(0);
                *count = id.counter().max(*count);
            }
        });
        Raw(node.text.to_owned())
    }

    /// Something to tell the person about what is being read.
    pub(crate) fn note(&mut self, what: impl Into<String>) {
        self.notes.push(Note {
            place: self.here.clone(),
            what: what.into(),
        });
    }

    /// Puts the fields found since `from` under `name`: they were read
    /// inside the value of that field or element.
    pub(crate) fn under(&mut self, from: usize, name: &str) {
        for (path, _) in &mut self.fields[from..] {
            let mut full = String::with_capacity(name.len() + 1 + path.len());
            escape_into(&mut full, name);
            full.push('/');
            full.push_str(path);
            *path = full;
        }
    }
}

/// The id a string spells, of any kind, this build's or a later one's:
/// one to four lower-case letters, then a count, or a writer, a dot and
/// a count, each in its one spelling.
pub(crate) fn id_shaped(text: &str) -> Option<RawId> {
    let letters = text.bytes().take_while(u8::is_ascii_lowercase).count();
    if !(1..=4).contains(&letters) {
        return None;
    }
    let number = |digits: &str| -> Option<u64> {
        let valid = !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && (digits == "0" || !digits.starts_with('0'));
        valid.then(|| digits.parse().ok()).flatten()
    };
    let (writer, counter) = match text[letters..].split_once('.') {
        None => (0, number(&text[letters..])?),
        Some((writer, counter)) => (number(writer)?, number(counter)?),
    };
    RawId::from_parts(Writer(u16::try_from(writer).ok()?), counter)
}

/// The object a value is.
pub(crate) fn object<'n, 'a>(node: &'n Node<'a>) -> R<&'n Object<'a>> {
    match &node.kind {
        Kind::Object(object) => Ok(object),
        _ => wrong("is not an object"),
    }
}

/// The items of an array.
pub(crate) fn array<'n, 'a>(node: &'n Node<'a>) -> R<&'n [Node<'a>]> {
    match &node.kind {
        Kind::Array(items) => Ok(items),
        _ => wrong("is not a list"),
    }
}

/// Reads an object's fields: `known` takes each key it knows and says
/// so; the rest are kept, as their text, as fields this build does not
/// know. A key `known` takes that is said twice is refused, since which
/// of the two is meant cannot be told.
pub(crate) fn fields<'n, 'a>(
    node: &'n Node<'a>,
    cx: &mut Cx,
    mut known: impl FnMut(&str, &'n Node<'a>, &mut Cx) -> R<bool>,
) -> R<()> {
    let object = object(node)?;
    let mut taken: Vec<&str> = Vec::new();
    for (key, value) in &object.members {
        let from = cx.fields.len();
        let took = known(key, value, cx).map_err(|bad| bad.at(key))?;
        if !took {
            let mut name = String::new();
            escape_into(&mut name, key);
            let raw = cx.raw(value);
            cx.fields.push((name, raw));
            continue;
        }
        if cx.fields.len() > from {
            cx.under(from, key);
        }
        if object.twice {
            if taken.contains(&key.as_ref()) {
                return wrong("is said twice").map_err(|bad: Bad| bad.at(key));
            }
            taken.push(key);
        }
    }
    Ok(())
}

/// The one key and value of an object that holds a choice: `{"blur":
/// 0.5}`. `None` for any other value.
pub(crate) fn choice<'n, 'a>(node: &'n Node<'a>) -> Option<(&'n str, &'n Node<'a>)> {
    match node.object()?.members.as_slice() {
        [(key, value)] => Some((key, value)),
        _ => None,
    }
}

/// A value that must be there.
pub(crate) fn need<T>(value: Option<T>, name: &str) -> R<T> {
    match value {
        Some(value) => Ok(value),
        None => wrong("is missing").map_err(|bad: Bad| bad.at(name)),
    }
}

/// What a record needs that this build does not have; none when it has
/// no `needs` or this build has them all.
pub(crate) fn lacking(object: &Object<'_>) -> R<Vec<String>> {
    let Some(needs) = object.get("needs") else {
        return Ok(Vec::new());
    };
    let mut lacking = Vec::new();
    for feature in array(needs).map_err(|bad| bad.at("needs"))? {
        let Some(name) = feature.str() else {
            return wrong("is not a list of names").map_err(|bad: Bad| bad.at("needs"));
        };
        if !FEATURES.contains(&name) && !lacking.iter().any(|known| known == name) {
            lacking.push(name.to_owned());
        }
    }
    Ok(lacking)
}

/// What reading a record gives besides the record: its fields this build
/// does not know, and what it marked as derived, as the file has it.
pub(crate) type Read<'n, 'a> = (Vec<(String, Raw)>, Option<&'n Node<'a>>);

/// Reads a record: refused whole when it needs what this build does not
/// have; otherwise its fields, as [`fields`] reads them, apart from
/// `needs` and `derived`. Gives back its fields this build does not
/// know, and what it marked as derived, as text.
pub(crate) fn record<'n, 'a>(
    node: &'n Node<'a>,
    cx: &mut Cx,
    mut known: impl FnMut(&str, &'n Node<'a>, &mut Cx) -> R<bool>,
) -> R<Read<'n, 'a>> {
    let lacking = lacking(object(node)?)?;
    if !lacking.is_empty() {
        return Err(Bad::Needs(lacking));
    }
    let outer = std::mem::take(&mut cx.fields);
    let mut derived = None;
    let read = fields(node, cx, |key, value, cx| match key {
        "needs" => Ok(true),
        "derived" => {
            derived = Some(value);
            Ok(true)
        }
        _ => known(key, value, cx),
    });
    let own = std::mem::replace(&mut cx.fields, outer);
    read.map(|()| (own, derived))
}

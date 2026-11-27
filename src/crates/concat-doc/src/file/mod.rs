// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A document's file: one JSON text, written whole, read back as it was.
//!
//! # The file
//!
//! A small project - one sequence, one lane, one file in the bin and a
//! clip of it - is written so:
//!
//! ```text
//! {"concat-project":1,
//! "mint":{"0":4},
//! "view":{"showing":"tl1","tabs":["tl1"]},
//! "media":{
//!   "m3":{"path":"/footage/a.mp4","name":"a.mp4","length":7056000000,"rate":{"num":30,"den":1},"size":{"width":1920,"height":1080},"has_sound":true}
//! },
//! "sequences":{
//!   "tl1":{"name":"Timeline 1","frame":{"width":1920,"height":1080,"rate":{"num":30,"den":1}}}
//! },
//! "lanes":{
//!   "t2":{"sequence":"tl1","order":"i"}
//! },
//! "items":{
//!   "c4":{"lane":"t2","start":0,"len":1411200000,"content":{"media":{"media":"m3"}},"sound":{"volume":0.5}}
//! }
//! }
//! ```
//!
//! - **`"concat-project": 1`** comes first and says what the file is and
//!   which edition of the format; this build reads edition 1. A project
//!   of an earlier Concat, or anything else, is refused, said plainly.
//! - **`mint`** is the count ids are made from, per writer (see
//!   [`Mint`]). The document's revision is not written: it counts the
//!   changes of one session.
//! - **`view`** is which sequence is showing and the order of the tabs.
//! - **`cover`** and **`fonts`** are there when the document has them.
//! - **The tables** are objects from id to record, written in id order:
//!   two saves of one document are the same bytes, and an edit changes
//!   the lines of the records it changed.
//!
//! How each value is spelled:
//!
//! - **A field at its default is left out**, and reads as its default.
//!   The defaults are the model's: an item as [`Item::new`] makes it, a
//!   picture as [`Picture::default`](crate::model::Picture) has it. A
//!   field with no default (an item's lane, an effect's package) is
//!   always written. `null` is written only for a none whose default is
//!   something: a shape with no fill.
//! - **A choice is a string or an object of one key**: `"layer"`,
//!   `{"blur":0.5}`, `{"media":{...}}`.
//! - **Ticks and counts are whole numbers**; every other number is
//!   written in the fewest digits that read back as the same number, and
//!   read exactly. A rate or a speed is `{"num":30000,"den":1001}`, an id
//!   its string, a colour `"#rrggbb"` or `"#rrggbbaa"`, a number with keys
//!   `{"value":1,"keys":[{"at":0,"value":0}]}`.
//! - **A speed curve** is its position keys and, when a person drew it,
//!   the graph they drew: `{"keyed":{"keys":[{"at":0,"source":0},...],
//!   "drawn":{"from":0,"len":1411200000,"points":[{"at":0,"speed":1},
//!   {"at":1,"speed":3}]}}}`.
//!
//! # The layout, measured
//!
//! Each top-level field and each record of a table is a line of its own,
//! and a record is compact on its line. On a project of 10,000 items,
//! measured on an Apple M5 in a release build (`examples/file_bench.rs`):
//!
//! | | A record a line | Compact | Indented throughout |
//! |---|---|---|---|
//! | Size | 1.78 MB | 1.75 MB (1.7% less) | 3.84 MB (2.2 times) |
//! | Diff of one moved item | 1 line, 198 bytes | 1 line, 3.5 MB | 1 line, 56 bytes |
//!
//! So a record a line costs what compact JSON costs, and a diff of an
//! edit is the records it changed; indenting more halves no diff that
//! matters and doubles what every save writes, flushes and reads back.
//!
//! The same project writes in 3.8 ms and reads in 13 ms: 2.3 ms of it
//! the JSON, 1.9 ms the index of what is where on each lane, 3.1 ms the
//! check of every rule, and the rest the records. A whole save - written,
//! flushed through the drive's cache, read back and compared, the save
//! before kept, swapped in, the folder flushed - takes 12 ms, 8 of them
//! the disk's. The save writes from a snapshot that is one pointer to
//! copy (41 ns), so it never holds an edit up.
//!
//! # What this build cannot read
//!
//! A file from a later build may hold what this one does not know, and
//! must lose none of it by being opened and saved here. Three things say
//! how a later build's additions are kept, after PNG's chunks:
//!
//! - **A field this build does not know** is kept, as its text, in the
//!   record's [`Residual`](crate::model::Residual), and written back where it was however the
//!   record is edited. A field inside a part of a record (an item's
//!   `picture`, a key of its opacity) is the record's, by its path.
//! - **`derived`**: an object any record may carry, of what a later build
//!   worked out from the rest of the record. It is kept while this build
//!   leaves the record as it read it, and dropped once this build changes
//!   the record, since it would no longer hold.
//! - **`needs`**: a list of features' names any record may carry. A build
//!   that lacks one of them cannot edit the record safely, so it does not
//!   open it: a row of a table is kept in [`Model::held`], an effect, mask
//!   or marker as an [`Entry::Unread`] in its place, and either is
//!   written back as it was. At the top of the file, a feature this
//!   build lacks refuses the open, naming it.
//!
//! A choice this build does not know (a kind of content, an easing, a
//! background) is kept in its `Unknown` arm, as its text.
//!
//! # What breaks a rule
//!
//! A file that a crash, a disk or a hand edit has damaged opens as far
//! as it can, and nothing in it is repaired in silence. What cannot be
//! read, or breaks one of the model's rules, is set aside: kept as its
//! text, written back where it came from, and named in a [`Note`] of
//! [`Opened`] in plain words. What depends on something set aside is
//! set aside in turn (the items on a lane that was). Of two things that
//! clash - two items overlapping, two lanes at one place - the later
//! id is set aside. Reading ends in a [`Document`] that keeps every
//! rule, and the list of what was set aside and why.
//!
//! Four things are put right rather than set aside, each said in a
//! note: a caption lane's look that holds words of its own has them
//! taken out; a speed graph that cannot be read or breaks a rule is left
//! out, and the keys worked out from it, which are what plays, stay; a
//! count of ids below an id the file holds is raised past it, unread
//! text included, so that no id is ever made twice; and a document with
//! sequences and none showing shows its first.

mod form;
mod input;
mod json;
mod out;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::{self, Write as _};
use std::str::FromStr;

use crate::document::{Document, Rejected, Revision};
use crate::id::{AnyId, IdError, MAX_COUNTER, Mint, RawId, TypedId, Writer};
use crate::model::{
    Content, Cover, Derived, Entry, Font, Held, HeldId, Item, ItemId, Lane, LaneId, Media, MediaId,
    Model, Raw, Rule, Sequence, SequenceId, Violation,
};
use crate::table::Table;

use form::Form;
use input::{Bad, Cx, fields, record};
use json::{Kind, Node};
use out::{Obj, Out};

/// The key the file starts with, and what says it is a project's.
pub const MARKER: &str = "concat-project";

/// The edition of the format this build reads and writes.
pub const EDITION: u64 = 1;

/// The features this build has, by the names a record's `needs` gives.
/// A record that needs any other is not opened.
pub const FEATURES: &[&str] = &[];

/// Whose count the held entries are named from: one of their own, made
/// afresh at each open and never written, since what is held is written
/// back where it came from and not as itself.
pub const HELD_WRITER: Writer = Writer(u16::MAX);

/// A file that was opened.
#[derive(Debug)]
pub struct Opened {
    /// The document: it keeps every rule.
    pub document: Document<Model>,
    /// What was set aside or put right, and why, for the person to read.
    pub notes: Vec<Note>,
}

/// One thing the person is told about a file they opened.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Note {
    /// What it is about: "item c12", "the view". Empty for the document.
    pub place: String,
    /// What happened, in plain words.
    pub what: String,
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.place.is_empty() {
            f.write_str(&self.what)
        } else {
            write!(f, "{}: {}", self.place, self.what)
        }
    }
}

/// Why a file was not opened. Nothing of it was read into a document.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refused {
    /// It is not text.
    NotText,
    /// It is text, and not JSON: cut short, or damaged, here.
    NotJson(String),
    /// It is JSON, and not a project.
    NotAProject,
    /// A project of an earlier Concat, whose format this build does not
    /// read.
    Earlier,
    /// A project in a later edition of the format.
    Later(u64),
    /// A project that needs features this build does not have.
    Needs(Vec<String>),
    /// A project whose top level is damaged past setting parts aside.
    Damaged(String),
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refused::NotText => f.write_str("this is not a Concat project: it is not text"),
            Refused::NotJson(why) => write!(f, "this project's file is damaged: {why}"),
            Refused::NotAProject => f.write_str("this is not a Concat project"),
            Refused::Earlier => f.write_str(
                "this project was saved by an earlier Concat, whose projects this one does not open",
            ),
            Refused::Later(edition) => write!(
                f,
                "this project was saved by a later Concat, in edition {edition} of its format; \
                 this one reads edition {EDITION}"
            ),
            Refused::Needs(features) => {
                let names: Vec<String> = features.iter().map(|f| format!("\"{f}\"")).collect();
                write!(
                    f,
                    "this project needs a later Concat: it uses {}, which this one does not have",
                    names.join(" and ")
                )
            }
            Refused::Damaged(why) => write!(f, "this project's file is damaged: {why}"),
        }
    }
}

impl std::error::Error for Refused {}

// --- Writing ---------------------------------------------------------------------

/// The file of a document.
///
/// The document is a snapshot: whoever saves holds it for as long as the
/// writing takes, and edits go on meanwhile against the current one.
pub fn write(document: &Document<Model>) -> String {
    write_sealed(document.tables(), document.mint()).0
}

/// The file, and the seal of the document as written.
fn write_sealed(model: &Model, mint: &Mint) -> (String, u64) {
    let estimate = 256 + model.items.len() * 320 + model.media.len() * 256;
    let mut out = Out::new(estimate);
    let held = Placed::of(model);
    let mut rec = out.record_lines(&model.residual);
    let obj = &mut rec.obj;
    out.key(obj, MARKER);
    out.int(EDITION);
    out.key(obj, "mint");
    put_mint(&mut out, mint);
    out.part(obj, "view", |out| put_view(out, model, &held));
    if let Some(raw) = held.whole("cover") {
        out.key(obj, "cover");
        out.raw(raw);
    } else if let Some(cover) = &model.cover {
        form::always(&mut out, obj, "cover", cover);
    }
    put_fonts(&mut out, obj, &model.fonts, &held);
    put_table(&mut out, obj, "media", &model.media, &held);
    put_table(&mut out, obj, "sequences", model.sequences.rows(), &held);
    put_table(&mut out, obj, "lanes", model.lanes.rows(), &held);
    put_table(&mut out, obj, "items", model.items.rows(), &held);
    out.end(rec);
    out.text.push('\n');
    let seal = out.seal;
    (out.text, seal)
}

fn put_mint(out: &mut Out<'_>, mint: &Mint) {
    let mut obj = out.open();
    for (writer, count) in mint.counts().filter(|(writer, _)| *writer != HELD_WRITER) {
        out.key(&mut obj, &writer.to_string());
        out.int(count);
    }
    out.close(obj);
}

/// The held entries, by where they go back to.
struct Placed<'d> {
    /// Values held whole: a table, the view, the cover, the fonts, or a
    /// part of the view.
    whole: HashMap<&'d str, &'d Raw>,
    /// Elements held from a list, in the order they were held.
    elements: HashMap<&'d str, Vec<&'d Raw>>,
    /// Rows held from a table: the key as the file wrote it, and the row.
    rows: HashMap<&'d str, Vec<(String, &'d Raw)>>,
}

impl<'d> Placed<'d> {
    fn of(model: &'d Model) -> Placed<'d> {
        let mut placed = Placed {
            whole: HashMap::new(),
            elements: HashMap::new(),
            rows: HashMap::new(),
        };
        for (_, held) in model.held.iter() {
            let place = held.place.as_str();
            if let Some(list) = place.strip_suffix("/-") {
                placed.elements.entry(list).or_default().push(&held.raw);
            } else if let Some((table, key)) = place.split_once('/')
                && TABLES.contains(&table)
            {
                let key = out::unescape(key);
                placed.rows.entry(table).or_default().push((key, &held.raw));
            } else {
                placed.whole.insert(place, &held.raw);
            }
        }
        placed
    }

    fn whole(&self, place: &str) -> Option<&'d str> {
        self.whole.get(place).map(|raw| raw.as_str())
    }

    fn elements(&self, list: &str) -> &[&'d Raw] {
        self.elements.get(list).map_or(&[], Vec::as_slice)
    }
}

const TABLES: [&str; 4] = ["media", "sequences", "lanes", "items"];

fn put_view<'d>(out: &mut Out<'d>, model: &'d Model, held: &Placed<'d>) {
    if let Some(raw) = held.whole("view") {
        out.raw(raw);
        return;
    }
    let mut obj = out.open();
    if let Some(raw) = held.whole("view/showing") {
        out.key(&mut obj, "showing");
        out.raw(raw);
    } else if let Some(showing) = &model.view.showing {
        form::always(out, &mut obj, "showing", showing);
    }
    if let Some(raw) = held.whole("view/tabs") {
        out.key(&mut obj, "tabs");
        out.raw(raw);
    } else {
        let tabs = &model.view.tabs;
        let set_aside = held.elements("view/tabs");
        if !tabs.is_empty() || !set_aside.is_empty() || out.holds_under("tabs") {
            out.part(&mut obj, "tabs", |out| put_list(out, tabs, set_aside));
        }
    }
    out.close(obj);
}

fn put_fonts<'d>(out: &mut Out<'d>, obj: &mut Obj, fonts: &'d [Font], held: &Placed<'d>) {
    if let Some(raw) = held.whole("fonts") {
        out.key(obj, "fonts");
        out.raw(raw);
        return;
    }
    let set_aside = held.elements("fonts");
    if !fonts.is_empty() || !set_aside.is_empty() || out.holds_under("fonts") {
        out.part(obj, "fonts", |out| put_list(out, fonts, set_aside));
    }
}

/// A list, then what was set aside from it.
fn put_list<'d, T: Form>(out: &mut Out<'d>, items: &'d [T], set_aside: &[&Raw]) {
    form::put_items(out, items, set_aside);
}

/// A table: its rows in id order, the rows set aside from it among them
/// by the ids their keys spell, and those whose keys spell none after.
fn put_table<'d, I: TypedId, T: Form>(
    out: &mut Out<'d>,
    obj: &mut Obj,
    name: &str,
    rows: &'d Table<I, T>,
    held: &Placed<'d>,
) {
    out.key(obj, name);
    if let Some(raw) = held.whole(name) {
        out.raw(raw);
        return;
    }
    let mut set_aside: Vec<(Option<RawId>, &str, &Raw)> = held
        .rows
        .get(name)
        .map(|rows| {
            rows.iter()
                .map(|(key, raw)| (parse_key::<I>(key).ok(), key.as_str(), *raw))
                .collect()
        })
        .unwrap_or_default();
    // Stable, so rows set aside under one key keep the order they had.
    set_aside.sort_by_key(|(id, _, _)| (id.is_none(), *id));
    let mut set_aside = set_aside.into_iter().peekable();
    out.within(name, |out| {
        let mut table = out.open_lines("  ");
        let mut name = String::new();
        for (id, row) in rows.iter() {
            while let Some((_, key, raw)) =
                set_aside.next_if(|(held, _, _)| held.is_some_and(|h| h < id.raw()))
            {
                out.key(&mut table, key);
                out.raw(raw.as_str());
            }
            name.clear();
            let _ = write!(name, "{id}");
            out.key(&mut table, &name);
            row.put(out);
            while let Some((_, key, raw)) =
                set_aside.next_if(|(held, _, _)| *held == Some(id.raw()))
            {
                out.key(&mut table, key);
                out.raw(raw.as_str());
            }
        }
        for (_, key, raw) in set_aside {
            out.key(&mut table, key);
            out.raw(raw.as_str());
        }
        out.close(table);
    });
}

fn parse_key<I: TypedId>(key: &str) -> Result<RawId, IdError> {
    crate::id::parse_id(I::TAG, key)
}

// --- Reading ---------------------------------------------------------------------

/// Reads a document from its file.
///
/// Never panics, whatever it is given. What it cannot open at all is
/// [`Refused`]; what it opens is a document that keeps every rule, with
/// notes of what was set aside to make it so.
pub fn read(bytes: &[u8]) -> Result<Opened, Refused> {
    let text = std::str::from_utf8(bytes).map_err(|_| Refused::NotText)?;
    let top = json::parse(text).map_err(|error| Refused::NotJson(error.to_string()))?;
    let object = top.object().ok_or(Refused::NotAProject)?;
    match object.get(MARKER) {
        None if object.get("concat").and_then(Node::str).is_some()
            && object.get("version").is_some() =>
        {
            return Err(Refused::Earlier);
        }
        None => return Err(Refused::NotAProject),
        Some(edition) => match (&edition.kind, edition.text.parse::<u64>()) {
            (Kind::Number, Ok(EDITION)) => {}
            (Kind::Number, Ok(later)) if later > EDITION => return Err(Refused::Later(later)),
            _ => return Err(Refused::NotAProject),
        },
    }
    match input::lacking(object) {
        Ok(lacking) if lacking.is_empty() => {}
        Ok(lacking) => return Err(Refused::Needs(lacking)),
        Err(bad) => return Err(Refused::Damaged(bad.words())),
    }
    Reading::new().read(&top)
}

/// One reading of a file.
struct Reading<'n, 'a> {
    cx: Cx,
    model: Model,
    mint: Mint,
    /// The text of each row that was read, in the order read, and the
    /// same by id, made when a row is first set aside.
    read: Vec<(AnyId, &'n Node<'a>)>,
    rows: Option<HashMap<AnyId, &'n Node<'a>>>,
    /// The text of each font, the tabs and the showing sequence, beside
    /// the model's.
    fonts: Vec<&'n Node<'a>>,
    tabs: Vec<&'n Node<'a>>,
    showing: Option<&'n Node<'a>>,
    cover: Option<&'n Node<'a>>,
}

/// The top level's parts, as the file has them.
#[derive(Default)]
struct Parts<'n, 'a> {
    mint: Option<&'n Node<'a>>,
    view: Option<&'n Node<'a>>,
    cover: Option<&'n Node<'a>>,
    fonts: Option<&'n Node<'a>>,
    tables: [Option<&'n Node<'a>>; 4],
}

impl<'n, 'a> Reading<'n, 'a> {
    fn new() -> Self {
        Reading {
            cx: Cx::new(),
            model: Model::default(),
            mint: Mint::new(),
            read: Vec::new(),
            rows: None,
            fonts: Vec::new(),
            tabs: Vec::new(),
            showing: None,
            cover: None,
        }
    }

    fn read(mut self, top: &'n Node<'a>) -> Result<Opened, Refused> {
        let mut parts = Parts::default();
        let (residual, derived) = record(top, &mut self.cx, |key, node, _| {
            match key {
                MARKER => {}
                "mint" => parts.mint = Some(node),
                "view" => parts.view = Some(node),
                "cover" => parts.cover = Some(node),
                "fonts" => parts.fonts = Some(node),
                _ => match TABLES.iter().position(|t| *t == key) {
                    Some(at) => parts.tables[at] = Some(node),
                    None => return Ok(false),
                },
            }
            Ok(true)
        })
        .map_err(|bad| Refused::Damaged(bad.words()))?;
        self.model.residual.fields = residual;
        // The count first: what is set aside from here on is named from it.
        self.read_mint(parts.mint);
        let [media, sequences, lanes, items] = parts.tables;
        self.model.media = self
            .table::<MediaId, Media>(media, "media", "file")
            .collect();
        self.model.sequences = self
            .table::<SequenceId, Sequence>(sequences, "sequences", "sequence")
            .collect();
        self.model.lanes = self.table::<LaneId, Lane>(lanes, "lanes", "lane").collect();
        self.model.items = self.table::<ItemId, Item>(items, "items", "item").collect();
        self.read_view(parts.view);
        self.read_cover(parts.cover);
        self.read_fonts(parts.fonts);
        // The fields of those parts this build does not know are the
        // document's own, by their paths.
        let parts = std::mem::take(&mut self.cx.fields);
        self.model.residual.fields.extend(parts);
        let derived = derived.map(|node| self.cx.raw(node));
        let document = self.settle()?;
        let document = match derived {
            None => document,
            Some(raw) => {
                // Sealed as this build writes the document it opened.
                let mut model = document.tables().clone();
                let seal = write_sealed(&model, document.mint()).1;
                model.residual.derived = Some(Derived { raw, seal });
                Document::assemble(model, document.mint().clone(), Revision::default())
            }
        };
        let mut notes = self.cx.notes;
        notes.dedup();
        Ok(Opened { document, notes })
    }

    /// Sets a value aside, as its text, to be written back at `place`.
    fn hold(&mut self, place: String, raw: Raw, why: String) {
        let Ok(id) = self.mint.mint::<HeldId>(HELD_WRITER) else {
            // Sixty-five thousand billion entries set aside in one file:
            // there is no such file.
            return;
        };
        // Its own table; the id is new.
        let _ = self.model.held.insert(id, Held { place, raw, why });
    }

    /// A row set aside: kept, written back, and said.
    fn hold_row(&mut self, table: &str, key: &str, node: &Node<'_>, what: String, why: String) {
        let mut place = format!("{table}/");
        out::escape_into(&mut place, key);
        let raw = self.cx.raw(node);
        self.cx.notes.push(Note {
            place: what,
            what: format!("kept as it was and not opened: {why}"),
        });
        self.hold(place, raw, why);
    }

    /// The rows of a table this build could read; the rest set aside.
    fn table<I, T>(
        &mut self,
        node: Option<&'n Node<'a>>,
        name: &str,
        what: &str,
    ) -> std::vec::IntoIter<(I, T)>
    where
        I: TypedId + FromStr<Err = IdError>,
        T: Form,
    {
        let mut rows: Vec<(I, T)> = Vec::new();
        let Some(node) = node else {
            return rows.into_iter();
        };
        let Some(object) = node.object() else {
            let raw = self.cx.raw(node);
            self.cx.notes.push(Note {
                place: format!("the {name}"),
                what: "kept as they were and not opened: they are not a table".into(),
            });
            self.hold(name.to_owned(), raw, "not a table".into());
            return rows.into_iter();
        };
        let mut seen: BTreeSet<RawId> = BTreeSet::new();
        rows.reserve(object.members.len());
        for (key, value) in &object.members {
            let here = || format!("{what} {key}");
            let id = match key.parse::<I>() {
                Ok(id) => id,
                Err(error) => {
                    let why = format!("its key is not an id such as \"{}12\" ({error})", I::TAG);
                    self.hold_row(name, key, value, here(), why);
                    continue;
                }
            };
            if !seen.insert(id.raw()) {
                let why = "its id is in the file twice".to_owned();
                self.hold_row(name, key, value, here(), why);
                continue;
            }
            let said = self.cx.notes.len();
            // Said only when something in it is set aside.
            self.cx.here.clear();
            let _ = write!(self.cx.here, "{what} {key}");
            match T::get(value, &mut self.cx) {
                Ok(row) => {
                    self.read.push((id.any(), value));
                    rows.push((id, row));
                }
                Err(bad) => {
                    // What was said of its parts goes with it.
                    self.cx.notes.truncate(said);
                    let here = std::mem::take(&mut self.cx.here);
                    self.hold_row(name, key, value, here, bad.words());
                }
            }
        }
        rows.into_iter()
    }

    fn read_mint(&mut self, node: Option<&'n Node<'a>>) {
        let read = node.map(|node| -> Result<Mint, Bad> {
            let mut counts = Vec::new();
            fields(node, &mut Cx::new(), |key, node, _| {
                let Ok(writer) = key.parse::<u16>() else {
                    return input::wrong("is not a writer");
                };
                counts.push((Writer(writer), count(node)?));
                Ok(true)
            })?;
            Mint::resume(counts).or_else(|error| input::wrong(error.to_string()))
        });
        match read {
            Some(Ok(mint)) => self.mint = mint,
            Some(Err(bad)) => self.cx.notes.push(Note {
                place: String::new(),
                what: format!(
                    "the count new ids are made from could not be read ({}); it goes on \
                     past every id the file holds",
                    bad.words()
                ),
            }),
            // A file with no count: one with no ids, or one whose ids
            // raise it, which is said then.
            None => {}
        }
    }

    fn read_view(&mut self, node: Option<&'n Node<'a>>) {
        let Some(node) = node else {
            return;
        };
        self.cx.here = "the view".into();
        let (mut showing, mut tabs) = (None, None);
        let from = self.cx.fields.len();
        let read = fields(node, &mut self.cx, |key, node, _| {
            match key {
                "showing" => showing = Some(node),
                "tabs" => tabs = Some(node),
                _ => return Ok(false),
            }
            Ok(true)
        });
        if let Err(bad) = read {
            self.cx.fields.truncate(from);
            let raw = self.cx.raw(node);
            self.cx
                .note(format!("kept as it was and not opened: {}", bad.words()));
            self.hold("view".into(), raw, bad.words());
            return;
        }
        self.cx.under(from, "view");
        if let Some(node) = showing {
            match Option::<SequenceId>::get(node, &mut self.cx) {
                Ok(id) => {
                    self.model.view.showing = id;
                    self.showing = Some(node);
                }
                Err(bad) => self.hold_view_part("view/showing", node, "the sequence showing", bad),
            }
        }
        let Some(node) = tabs else {
            return;
        };
        let Some(items) = node.array() else {
            let bad = input::wrong::<()>("is not a list").unwrap_err();
            self.hold_view_part("view/tabs", node, "the tabs", bad);
            return;
        };
        for item in items {
            match SequenceId::get(item, &mut self.cx) {
                Ok(tab) => {
                    self.model.view.tabs.push(tab);
                    self.tabs.push(item);
                }
                Err(bad) => self.hold_view_part("view/tabs/-", item, "a tab", bad),
            }
        }
    }

    fn hold_view_part(&mut self, place: &str, node: &Node<'_>, what: &str, bad: Bad) {
        let raw = self.cx.raw(node);
        self.cx.notes.push(Note {
            place: "the view".into(),
            what: format!("{what} is kept as it was and not opened: {}", bad.words()),
        });
        self.hold(place.into(), raw, bad.words());
    }

    fn read_cover(&mut self, node: Option<&'n Node<'a>>) {
        let Some(node) = node else {
            return;
        };
        self.cx.here = "the cover".into();
        let from = self.cx.fields.len();
        match Option::<Cover>::get(node, &mut self.cx) {
            Ok(cover) => {
                self.cx.under(from, "cover");
                self.model.cover = cover;
                self.cover = Some(node);
            }
            Err(bad) => {
                self.cx.fields.truncate(from);
                let raw = self.cx.raw(node);
                self.cx
                    .note(format!("kept as it was and not opened: {}", bad.words()));
                self.hold("cover".into(), raw, bad.words());
            }
        }
    }

    fn read_fonts(&mut self, node: Option<&'n Node<'a>>) {
        let Some(node) = node else {
            return;
        };
        self.cx.here = "the fonts".into();
        let Some(items) = node.array() else {
            let raw = self.cx.raw(node);
            self.cx
                .note("kept as they were and not opened: they are not a list");
            self.hold("fonts".into(), raw, "not a list".into());
            return;
        };
        for item in items {
            let from = self.cx.fields.len();
            match Font::get(item, &mut self.cx) {
                Ok(font) => {
                    let at = self.model.fonts.len().to_string();
                    self.cx.under(from, &at);
                    self.cx.under(from, "fonts");
                    self.model.fonts.push(font);
                    self.fonts.push(item);
                }
                Err(bad) => {
                    self.cx.fields.truncate(from);
                    let raw = self.cx.raw(item);
                    self.cx.note(format!(
                        "a font is kept as it was and not opened: {}",
                        bad.words()
                    ));
                    self.hold("fonts/-".into(), raw, bad.words());
                }
            }
        }
    }

    /// Makes the document: the count raised past every id the file
    /// holds, and what breaks a rule set aside until nothing does.
    fn settle(&mut self) -> Result<Document<Model>, Refused> {
        let unseen: Vec<(Writer, u64)> = self.cx.unseen.iter().map(|(w, c)| (*w, *c)).collect();
        let mut raised = self.raise(unseen, false);
        // Each round sets aside at least one thing, or gives up.
        loop {
            let model = self.model.clone();
            match Document::load(model, self.mint.clone(), Revision::default()) {
                Ok(document) => {
                    // What was set aside on the way is unread text too.
                    let unseen = self.cx.unseen.iter().map(|(w, c)| (*w, *c)).collect();
                    if self.raise(unseen, raised) {
                        let model = document.tables().clone();
                        return Ok(Document::assemble(
                            model,
                            self.mint.clone(),
                            Revision::default(),
                        ));
                    }
                    return Ok(document);
                }
                Err(Rejected::Unminted(ids)) => {
                    let counts = ids
                        .iter()
                        .map(|id| (id.raw().writer(), id.raw().counter()))
                        .collect();
                    raised |= self.raise(counts, raised);
                }
                Err(Rejected::Invalid(broken)) => {
                    if !self.set_aside(&broken) {
                        let first = broken.first().map(|v| v.to_string()).unwrap_or_default();
                        return Err(Refused::Damaged(format!(
                            "it breaks a rule this build cannot set aside ({first})"
                        )));
                    }
                }
            }
        }
    }

    /// Raises the count past these counts, if it is below any, and says
    /// so unless it was said; whether it was raised.
    fn raise(&mut self, counts: Vec<(Writer, u64)>, said: bool) -> bool {
        let below = counts
            .iter()
            .any(|(writer, count)| *count > self.mint.issued(*writer));
        if !below {
            return false;
        }
        // Every count is one an id can have: it was read from one.
        if let Ok(mint) = Mint::resume(self.mint.counts().chain(counts)) {
            self.mint = mint;
        }
        if !said {
            self.cx.notes.push(Note {
                place: String::new(),
                what: "the count new ids are made from was below ids the file holds; it now \
                       goes on past them"
                    .into(),
            });
        }
        true
    }

    /// Sets aside what breaks each rule; whether anything was.
    fn set_aside(&mut self, broken: &[Violation]) -> bool {
        let mut rows: BTreeMap<AnyId, String> = BTreeMap::new();
        let mut nested: BTreeMap<AnyId, String> = BTreeMap::new();
        let mut twice: BTreeMap<AnyId, String> = BTreeMap::new();
        let mut fonts_twice: BTreeSet<String> = BTreeSet::new();
        let mut cover = None;
        let mut showing = None;
        let mut tabs: BTreeMap<SequenceId, (bool, String)> = BTreeMap::new();
        let mut show_first = false;
        for violation in broken {
            let why = rule_words(&violation.rule);
            let Some(id) = violation.id else {
                match &violation.rule {
                    Rule::FontTwice { path } => {
                        fonts_twice.insert(path.clone());
                    }
                    _ => cover = Some(why),
                }
                continue;
            };
            let later = |with: AnyId| if with > id { with } else { id };
            match &violation.rule {
                Rule::Dangling {
                    what: "view.showing",
                    ..
                } => showing = Some(why),
                Rule::Dangling {
                    what: "view.tabs", ..
                } => {
                    if let Some(tab) = id.typed() {
                        tabs.insert(tab, (true, why));
                    }
                }
                Rule::TabTwice => {
                    if let Some(tab) = id.typed() {
                        tabs.entry(tab).or_insert((false, why));
                    }
                }
                Rule::NothingShowing => show_first = true,
                Rule::Overlap { with } => {
                    rows.insert(later(with.any()), why);
                }
                Rule::LaneOrderTaken { with } => {
                    rows.insert(later(with.any()), why);
                }
                Rule::IdTwice { .. } => {
                    twice.insert(id, why);
                }
                _ if is_row(id) => {
                    rows.insert(id, why);
                }
                _ => {
                    nested.insert(id, why);
                }
            }
        }
        let mut changed = false;
        for (id, why) in twice {
            changed |= self.unread_twice(id, &why, &mut rows);
        }
        for (id, why) in nested {
            for holder in self.holders(id) {
                changed |= self.unread(holder, id, None, &why, &mut rows);
            }
        }
        for (id, why) in rows {
            changed |= self.hold_known_row(id, why);
        }
        for path in fonts_twice {
            let mut first = true;
            let mut at = 0;
            while at < self.model.fonts.len() {
                if self.model.fonts[at].path != path {
                    at += 1;
                } else if first {
                    first = false;
                    at += 1;
                } else {
                    self.model.fonts.remove(at);
                    renumber(&mut self.model.residual.fields, "fonts", at);
                    let node = self.fonts.remove(at);
                    let why = format!("the font file {path} is added twice");
                    self.cx.here = "the fonts".into();
                    let raw = self.cx.raw(node);
                    self.cx
                        .note(format!("a font is kept as it was and not opened: {why}"));
                    self.hold("fonts/-".into(), raw, why);
                    changed = true;
                }
            }
        }
        if let Some(why) = cover
            && let Some(node) = self.cover.take()
        {
            self.model.cover = None;
            self.cx.here = "the cover".into();
            let raw = self.cx.raw(node);
            self.cx
                .note(format!("kept as it was and not opened: {why}"));
            self.hold("cover".into(), raw, why);
            changed = true;
        }
        if let Some(why) = showing
            && let Some(node) = self.showing.take()
        {
            self.model.view.showing = None;
            self.hold_view_part(
                "view/showing",
                node,
                "the sequence showing",
                input::wrong::<()>(why).unwrap_err(),
            );
            changed = true;
        }
        for (tab, (gone, why)) in tabs {
            let mut seen = false;
            let mut at = 0;
            while at < self.model.view.tabs.len() {
                let keep = self.model.view.tabs[at] != tab || (!gone && !seen);
                seen |= self.model.view.tabs[at] == tab;
                if keep {
                    at += 1;
                    continue;
                }
                self.model.view.tabs.remove(at);
                let node = self.tabs.remove(at);
                let bad = input::wrong::<()>(why.clone()).unwrap_err();
                self.hold_view_part("view/tabs/-", node, "a tab", bad);
                changed = true;
            }
        }
        if show_first
            && self.model.view.showing.is_none()
            && let Some(first) = self.model.sequences.ids().next()
        {
            self.model.view.showing = Some(first);
            self.cx.notes.push(Note {
                place: "the view".into(),
                what: format!("no sequence was showing; {first} is shown"),
            });
            changed = true;
        }
        changed
    }

    /// The rows that hold a nested id, in id order.
    fn holders(&self, id: AnyId) -> Vec<AnyId> {
        let items = self.model.items.index().holding(id).map(TypedId::any);
        let sequences = self.model.sequences.index().holding(id).map(TypedId::any);
        let mut holders: Vec<AnyId> = items.chain(sequences).collect();
        holders.sort();
        holders.dedup();
        holders
    }

    /// A nested id held twice: kept by the first holder, and in it by its
    /// first place; set aside everywhere else.
    fn unread_twice(&mut self, id: AnyId, why: &str, rows: &mut BTreeMap<AnyId, String>) -> bool {
        let mut changed = false;
        for (at, holder) in self.holders(id).into_iter().enumerate() {
            let keep = if at == 0 { 1 } else { 0 };
            changed |= self.unread(holder, id, Some(keep), why, rows);
        }
        changed
    }

    /// Sets aside the entries of `holder` that have `id`, past the first
    /// `keep` of them (all, for none). An id that is a transition's own
    /// sets aside the item it is.
    fn unread(
        &mut self,
        holder: AnyId,
        id: AnyId,
        keep: Option<usize>,
        why: &str,
        rows: &mut BTreeMap<AnyId, String>,
    ) -> bool {
        let keep = keep.unwrap_or(0);
        let mut places: Vec<(&'static str, usize)> = Vec::new();
        if let Some(item) = holder
            .typed::<ItemId>()
            .and_then(|i| self.model.items.get(i))
        {
            let lists: [(&'static str, Vec<Option<AnyId>>); 4] = [
                ("effects", ids_of(&item.effects, |e| e.id.any())),
                ("filters", ids_of(&item.filters, |e| e.id.any())),
                ("masks", ids_of(&item.masks, |m| m.id.any())),
                ("markers", ids_of(&item.markers, |m| m.id.any())),
            ];
            // In the order the item holds them: its chains, its own
            // packages, its masks and markers.
            for (name, ids) in &lists[..2] {
                places.extend(at_id(ids, id).map(|at| (*name, at)));
            }
            if matches!(&item.content, Content::Transition(e) if e.id.any() == id) {
                places.push(("package", 0));
            }
            if item
                .cut_in
                .as_ref()
                .is_some_and(|c| c.effect.id.any() == id)
            {
                places.push(("package", 1));
            }
            for (name, ids) in &lists[2..] {
                places.extend(at_id(ids, id).map(|at| (*name, at)));
            }
        } else if let Some(sequence) = holder
            .typed::<SequenceId>()
            .and_then(|s| self.model.sequences.get(s))
        {
            let ids = ids_of(&sequence.markers, |m| m.id.any());
            places.extend(at_id(&ids, id).map(|at| ("markers", at)));
        }
        let gone: Vec<(&'static str, usize)> = places.into_iter().skip(keep).collect();
        if gone.is_empty() {
            return false;
        }
        // A package is no entry of a list: the item it is part of goes.
        if gone.iter().any(|(name, _)| *name == "package") {
            rows.entry(holder).or_insert_with(|| why.to_owned());
            return true;
        }
        // The text of each entry, as the file has it at the entry's place.
        let texts: Option<Vec<&Node<'a>>> = self.row(holder).and_then(|node| {
            let row = node.object()?;
            gone.iter()
                .map(|(name, at)| row.get(name)?.array()?.get(*at))
                .collect()
        });
        let Some(texts) = texts else {
            // Not where the file had it: the row goes whole instead.
            rows.entry(holder).or_insert_with(|| why.to_owned());
            return true;
        };
        let what = |name: &str| match name {
            "effects" => "an effect",
            "filters" => "a sound filter",
            "masks" => "a mask",
            _ => "a marker",
        };
        for ((name, at), text) in gone.into_iter().zip(texts) {
            let raw = self.cx.raw(text);
            self.cx.notes.push(Note {
                place: row_words(holder),
                what: format!("{} is kept as it was and not opened: {why}", what(name)),
            });
            // The holder is there: it was just read from the model.
            if let Some(item) = holder.typed::<ItemId>() {
                let _ = self.model.items.update(item, |item| match name {
                    "effects" => set(&mut item.effects, at, raw),
                    "filters" => set(&mut item.filters, at, raw),
                    "masks" => set(&mut item.masks, at, raw),
                    _ => set(&mut item.markers, at, raw),
                });
            } else if let Some(sequence) = holder.typed::<SequenceId>() {
                let _ = self
                    .model
                    .sequences
                    .update(sequence, |s| set(&mut s.markers, at, raw));
            }
        }
        true
    }

    /// The text of a row that was read.
    fn row(&mut self, id: AnyId) -> Option<&'n Node<'a>> {
        let read = &self.read;
        let rows = self
            .rows
            .get_or_insert_with(|| read.iter().copied().collect());
        rows.get(&id).copied()
    }

    /// A row the file read that breaks a rule: set aside, as its text.
    fn hold_known_row(&mut self, id: AnyId, why: String) -> bool {
        let removed = match id.tag() {
            MediaId::TAG => id
                .typed()
                .is_some_and(|m: MediaId| self.model.media.remove(m).is_ok()),
            SequenceId::TAG => id
                .typed()
                .is_some_and(|s: SequenceId| self.model.sequences.remove(s).is_ok()),
            LaneId::TAG => id
                .typed()
                .is_some_and(|l: LaneId| self.model.lanes.remove(l).is_ok()),
            ItemId::TAG => id
                .typed()
                .is_some_and(|i: ItemId| self.model.items.remove(i).is_ok()),
            _ => false,
        };
        let Some(node) = self.row(id).filter(|_| removed) else {
            return false;
        };
        let table = match id.tag() {
            MediaId::TAG => "media",
            SequenceId::TAG => "sequences",
            LaneId::TAG => "lanes",
            _ => "items",
        };
        self.hold_row(table, &id.to_string(), node, row_words(id), why);
        true
    }
}

/// Each entry's id, or none for one kept unread.
fn ids_of<T>(entries: &[Entry<T>], id: impl Fn(&T) -> AnyId) -> Vec<Option<AnyId>> {
    entries.iter().map(|e| e.known().map(&id)).collect()
}

/// Where in a list of ids one is.
fn at_id(ids: &[Option<AnyId>], id: AnyId) -> impl Iterator<Item = usize> + '_ {
    ids.iter()
        .enumerate()
        .filter(move |(_, known)| **known == Some(id))
        .map(|(at, _)| at)
}

/// The fields of a list's elements after one taken out of it, named
/// again by their new places; the fields of the one taken out go with
/// its text.
fn renumber(fields: &mut Vec<(String, Raw)>, list: &str, removed: usize) {
    fields.retain_mut(|(path, _)| {
        let Some(rest) = path.strip_prefix(list).and_then(|r| r.strip_prefix('/')) else {
            return true;
        };
        let (place, inner) = rest.split_once('/').unwrap_or((rest, ""));
        match place.parse::<usize>() {
            Ok(at) if at == removed => false,
            Ok(at) if at > removed => {
                *path = format!("{list}/{}/{inner}", at - 1);
                true
            }
            _ => true,
        }
    });
}

/// An entry set aside in its place.
fn set<T>(entries: &mut [Entry<T>], at: usize, raw: Raw) {
    if let Some(slot) = entries.get_mut(at) {
        *slot = Entry::Unread(raw);
    }
}

/// Whether an id is a row of a table.
fn is_row(id: AnyId) -> bool {
    [MediaId::TAG, SequenceId::TAG, LaneId::TAG, ItemId::TAG].contains(&id.tag())
}

/// A row, as a person reads it: "item c12".
fn row_words(id: AnyId) -> String {
    let kind = match id.tag() {
        MediaId::TAG => "file",
        SequenceId::TAG => "sequence",
        LaneId::TAG => "lane",
        ItemId::TAG => "item",
        _ => "entry",
    };
    format!("{kind} {id}")
}

/// A broken rule, as a person reads it.
fn rule_words(rule: &Rule) -> String {
    match rule {
        Rule::OutOfRange { what } => format!("`{what}` is more than a century from zero"),
        Rule::Dangling { what, to } => {
            format!("`{what}` names {to}, which is not in the file or was set aside")
        }
        Rule::WrongKind { what, to } => {
            format!("`{what}` names {to}, which is not a file of the kind it can be")
        }
        Rule::NotFinite { what } => format!("`{what}` is not a finite number"),
        Rule::OutOfBounds { what } => format!("`{what}` is outside what it can mean"),
        Rule::Negative { what } => format!("`{what}` is below zero"),
        Rule::BlankName => "its name is blank".into(),
        Rule::EmptyPackage => "it names no package".into(),
        Rule::EmptyParam { name } if name.is_empty() => "it has a knob with no name".into(),
        Rule::EmptyParam { name } => format!("its knob `{name}` has no value and no keys"),
        Rule::KeysOutOfOrder { what } => {
            format!("the keys of `{what}` are out of order, or two share a tick")
        }
        Rule::BadEase { what } => format!("a key of `{what}` eases back in time"),
        Rule::SpeedNotAllowed { what } => {
            format!("`{what}` plays slower than a sixteenth or faster than sixteen times")
        }
        Rule::TimeKeysTooFew => "its time map has fewer than two keys".into(),
        Rule::TimeKeysOutOfOrder => {
            "its time map's keys are out of order, or two share a tick".into()
        }
        Rule::TimeNotMonotone => "its time map stands still or turns back".into(),
        Rule::WindowPastKeys => "it shows time past its time map's keys".into(),
        Rule::SpeedGraphEmpty => "its speed graph has no points".into(),
        Rule::WindowPastSource { source } => {
            format!("it shows more of {source} than the file has")
        }
        Rule::Overlap { with } => format!("it overlaps {with} on its lane"),
        Rule::NothingBeneath => "it is a transition with nothing beneath it".into(),
        Rule::Cycle { through } => format!("it nests {through}, which holds it"),
        Rule::LaneMoved => "it moved to another sequence".into(),
        Rule::LaneOrderTaken { with } => format!("it is at the same place in the stack as {with}"),
        Rule::StreamTwice { index } => format!("it lists sound stream {index} twice"),
        Rule::StreamsWithoutSound => "it lists sound streams and has no sound".into(),
        Rule::NoSuchStream { index } => {
            format!("it plays sound stream {index}, which its file does not list")
        }
        Rule::EnhancedIsOriginal => "its enhanced copy is the file it was cut from".into(),
        Rule::LinkToSelf => "it is tied to itself".into(),
        Rule::LinkAcrossSequences { to } => format!("it is tied to {to}, in another sequence"),
        Rule::LinkMismatch { to } => format!("it is said to be the sound of {to}, and is not"),
        Rule::LinkChain { to } => {
            format!("it is tied to {to}, which is itself the sound of another")
        }
        Rule::IdTwice { with } => format!("its id is held by {with} too"),
        Rule::BadRun { index } => format!(
            "run {} of its words is empty, past the words, or out of order",
            index + 1
        ),
        Rule::CaptionWords => "its caption look has words of its own".into(),
        Rule::TabTwice => "it is open in two tabs".into(),
        Rule::NothingShowing => "no sequence is showing".into(),
        Rule::FontTwice { path } => format!("the font file {path} is added twice"),
    }
}

/// One writer's count, as the mint has it in a file.
fn count(node: &Node<'_>) -> Result<u64, Bad> {
    match (&node.kind, node.text.parse::<u64>()) {
        (Kind::Number, Ok(count)) if count <= MAX_COUNTER => Ok(count),
        _ => input::wrong("is not a count of ids"),
    }
}

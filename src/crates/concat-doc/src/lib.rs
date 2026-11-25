// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The document's core: time, names, tables that share, changes that are
//! checked, and undo.
//!
//! This crate holds no clips. It is the part of a document that is the
//! same whatever the document is of, built so that the faults a document
//! model grows cannot be written on top of it:
//!
//! - **A time that is nearly a frame edge.** Time is a whole number of
//!   [`Tick`]s, 705,600,000 to the second, which every frame and sample
//!   rate in use divides. A place and a length are different types, and
//!   every multiply-then-divide goes through one function with one
//!   rounding rule: [`rescale`]. See [`time`].
//! - **A name that comes to mean something else.** An id is a typed,
//!   non-zero number from a count that is saved with the document and
//!   never goes back, written in a file as a short string with its kind's
//!   letter. See [`id`].
//! - **A copy per undo step.** A [`Table`] is a sorted map that shares
//!   with its copies everything they have in common, so a snapshot costs
//!   what changed since the last one. See [`map`], where the choice of
//!   the map is measured.
//! - **A half-made change that someone saw.** A [`Document`] cannot be
//!   changed. A [`Draft`] is edited in private, and its
//!   [`commit`](Draft::commit) either passes the [`Schema`]'s checks and
//!   gives a new document, or gives the list of what was wrong and
//!   changes nothing. See [`document`].
//! - **A change list that forgot a side effect.** What a commit changed
//!   is worked out by comparing the two documents
//!   ([`TableChanges`]), never listed by the code that made the change.
//!   The one list is what the checks read and what everything following
//!   the document is told.
//! - **An undo that is its own program.** A step is two documents, and
//!   undo makes the earlier one current: [`History`]. See [`history`]
//!   for the rules on joining small repeats, view state, the budget, and
//!   a second writer.
//!
//! # In use
//!
//! ```
//! use concat_doc::{
//!     AnyTable, ChangePart, ChangeSet, Document, Entry, History, Origin, Schema, Table,
//!     TableChanges, Weigh,
//! };
//!
//! concat_doc::typed_id! {
//!     /// A note.
//!     pub struct NoteId = "n";
//! }
//!
//! #[derive(Clone, PartialEq, Debug)]
//! pub struct Note(String);
//!
//! impl Weigh for Note {
//!     fn weight(&self) -> usize {
//!         self.0.len()
//!     }
//! }
//!
//! /// The schema: one table.
//! #[derive(Clone, Default)]
//! pub struct Notes {
//!     notes: Table<NoteId, Note>,
//! }
//!
//! pub struct NotesChanges {
//!     notes: TableChanges<NoteId, Note>,
//! }
//!
//! impl ChangeSet for NotesChanges {
//!     fn parts(&self, each: &mut dyn FnMut(&dyn ChangePart)) {
//!         each(&self.notes);
//!     }
//! }
//!
//! impl Schema for Notes {
//!     type Changes = NotesChanges;
//!     type Violation = NoteId;
//!
//!     fn diff(before: &Self, after: &Self) -> NotesChanges {
//!         NotesChanges { notes: TableChanges::between(&before.notes, &after.notes) }
//!     }
//!
//!     // The one rule: a note says something.
//!     fn check(_: &Self, after: &Self, changes: &NotesChanges) -> Vec<NoteId> {
//!         let empty = |id: &NoteId| after.notes.get(*id).is_some_and(|note| note.0.is_empty());
//!         changes.notes.ids().filter(empty).collect()
//!     }
//!
//!     fn tables(&self, each: &mut dyn FnMut(&dyn AnyTable)) {
//!         each(&self.notes);
//!     }
//! }
//!
//! let mut history: History<Notes, &str, ()> = History::new(Document::new(), Default::default());
//!
//! // A change is a draft, committed and recorded.
//! let mut draft = history.current().begin("Add note", Origin(0));
//! let id: NoteId = draft.mint().unwrap();
//! draft.notes.insert(id, Note("milk".into())).unwrap();
//! let commit = draft.commit().unwrap();
//! assert_eq!(commit.changes().notes.added.len(), 1);
//! history.record(commit, Entry::new(vec!["add"], (), ())).unwrap();
//! assert!(history.current().notes.contains(id));
//!
//! // A change that breaks a rule changes nothing.
//! let mut draft = history.current().begin("Empty it", Origin(0));
//! draft.notes.replace(id, Note(String::new())).unwrap();
//! assert!(draft.commit().is_err());
//! assert_eq!(history.current().notes.get(id).unwrap().0, "milk");
//!
//! // Undo is the document from before.
//! let undone = history.undo().unwrap();
//! assert!(undone.document.notes.is_empty());
//! assert_eq!(undone.label, "Add note");
//! ```

#![forbid(unsafe_code)]

pub mod commands;
pub mod document;
pub mod file;
pub mod history;
pub mod id;
pub mod map;
pub mod model;
pub mod save;
pub mod table;
pub mod time;

pub use document::{
    ChangePart, ChangeSet, Changed, Commit, Document, Draft, Origin, PATH_BYTES, Rejected,
    Revision, Schema, StalePlan, Weigh,
};
pub use history::{Budget, Coalesce, Entry, History, NotCurrent, Recorded, Restored, Step};
pub use id::{AnyId, COUNTER_BITS, IdError, MAX_COUNTER, Mint, MintError, RawId, TypedId, Writer};
pub use map::{Diff, DiffIter, Iter, OrdMap};
pub use table::{AnyTable, Replaced, Table, TableChanges, TableError};
pub use time::{
    NotALength, Rate, RateError, Snap, TICKS_PER_SECOND, Tick, TickDur, TickLen, rescale,
};

/// What the crate's macros reach through. Not for use by name.
#[doc(hidden)]
pub mod __private {
    pub use serde;
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The document, and the one way to change it.
//!
//! A [`Document`] is a snapshot: the tables, the [`Mint`] and a
//! [`Revision`], behind one pointer. It cannot be changed. Whoever holds
//! one - the export, the save, the picture being drawn - holds the
//! document as it was at that revision for as long as they keep it, and
//! waits for nobody.
//!
//! A change is a [`Draft`]: a private copy that commands edit freely,
//! through states that break the document's rules if they must, since
//! nothing else can see it. [`Draft::commit`] works out what changed by
//! comparing the draft with the document it started from, has the
//! [`Schema`] check exactly that, and gives a new document or the list of
//! what was wrong. The document it started from is untouched either way,
//! and a draft that is dropped was never there. So every `Document` that
//! exists passed its checks: holding one is holding the proof.
//!
//! # What the tables are
//!
//! This crate does not know. A [`Schema`] is the caller's struct of
//! [`Table`](crate::Table)s and a few plain values, with four things said about it:
//! how two of them differ, what makes one valid, which tables it has, and
//! what in it is view state. The model of clips and lanes is one schema;
//! the tests here use a library of shelves and books.

use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::id::{AnyId, Mint, MintError, TypedId, Writer};
use crate::table::{AnyTable, TableChanges};

/// How many changes a document has been through.
///
/// It goes up by one each time the document becomes something else: a
/// commit, a step back, a step forward again. It never goes down and is
/// never used twice, so "the document at revision 40" names one state,
/// and a plan made against it can be told from one made after.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Revision(pub u64);

impl Revision {
    /// The revision after this one.
    pub const fn next(self) -> Revision {
        // Not reached in the life of the universe at a change every
        // nanosecond; and if a file claims to be there, stay there.
        Revision(self.0.saturating_add(1))
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Who made a change: the window, a script, an assistant's turn.
///
/// The number means what the caller says it means; nothing here reads
/// it. It rides on every [`Draft`], [`Commit`] and undo step, so that
/// what hears of a change can tell its own from another's.
///
/// This is not whose count an id came from; that is a
/// [`Writer`], and everyone writing to one open project shares one.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct Origin(pub u32);

/// An estimate of how many bytes a value holds on to.
///
/// What the undo history's budget adds up. It need not be exact; it
/// should grow with what the value owns, so that a step that replaced a
/// long caption costs more than one that moved a clip.
pub trait Weigh {
    /// About how many bytes this value keeps alive.
    fn weight(&self) -> usize;
}

/// About how many bytes of a table's tree one changed entry rebuilds: the
/// nodes from the root down to it. Measured between 0.8 and 1.3 KB from a
/// thousand to a hundred thousand entries; see [`map`](crate::map).
pub const PATH_BYTES: usize = 1024;

/// One part of what changed between two schemas: one table's changes, or
/// one plain value's.
pub trait ChangePart {
    /// Whether this part did not change.
    fn is_unchanged(&self) -> bool;

    /// Every id this part added.
    fn added(&self, each: &mut dyn FnMut(AnyId));

    /// About how many bytes are kept alive by keeping the older side of
    /// this change.
    fn cost(&self) -> usize;
}

impl<I: TypedId, T: Weigh> ChangePart for TableChanges<I, T> {
    fn is_unchanged(&self) -> bool {
        self.is_empty()
    }

    fn added(&self, each: &mut dyn FnMut(AnyId)) {
        for (id, _) in &self.added {
            each(id.any());
        }
    }

    /// What the newer table let go of, and the nodes rebuilt on the way
    /// to each entry. With sharing, that is all a step costs: everything
    /// else is the same memory as the step before.
    fn cost(&self) -> usize {
        let gone: usize = self.gone().map(|value| value.weight()).sum();
        gone + self.len() * PATH_BYTES
    }
}

/// A plain value that changed, before and after.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Changed<T> {
    /// What it was.
    pub old: T,
    /// What it is.
    pub new: T,
}

impl<T: Clone + PartialEq> Changed<T> {
    /// The change from `old` to `new`, if they differ.
    pub fn between(old: &T, new: &T) -> Option<Changed<T>> {
        (old != new).then(|| Changed {
            old: old.clone(),
            new: new.clone(),
        })
    }
}

impl<T> ChangePart for Option<Changed<T>> {
    fn is_unchanged(&self) -> bool {
        self.is_none()
    }

    fn added(&self, _: &mut dyn FnMut(AnyId)) {}

    /// Nothing: the plain values of a schema are small by agreement.
    fn cost(&self) -> usize {
        0
    }
}

/// What changed between two schemas: a struct with a [`TableChanges`]
/// for each table and an `Option<`[`Changed`]`>` for each plain value.
pub trait ChangeSet {
    /// Shows every part to `each`. The one thing to write; leaving a
    /// part out here hides it from the checks below.
    fn parts(&self, each: &mut dyn FnMut(&dyn ChangePart));

    /// Whether nothing changed.
    fn is_empty(&self) -> bool {
        let mut empty = true;
        self.parts(&mut |part| empty &= part.is_unchanged());
        empty
    }

    /// About how many bytes are kept alive by keeping the older side of
    /// these changes.
    fn cost(&self) -> usize {
        let mut cost = 0_usize;
        self.parts(&mut |part| cost = cost.saturating_add(part.cost()));
        cost
    }

    /// Every id that was added, of every kind.
    fn added(&self) -> Vec<AnyId> {
        let mut ids = Vec::new();
        self.parts(&mut |part| part.added(&mut |id| ids.push(id)));
        ids
    }
}

/// What a document is made of: the caller's tables and plain values, and
/// the rules they keep.
///
/// A schema is a struct whose fields are [`Table`](crate::Table)s and a
/// few small values, cheap to copy because each table is. The crate asks
/// four things of it.
pub trait Schema: Clone {
    /// What changed between two of these: see [`ChangeSet`].
    type Changes: ChangeSet;

    /// One broken rule. It should name the ids at fault, so that whoever
    /// is told can point at them.
    type Violation: fmt::Debug;

    /// What changed from `before` to `after`:
    /// [`TableChanges::between`] for each table and
    /// [`Changed::between`] for each plain value. A table or value left
    /// out of this is one whose changes are never checked or told to
    /// anyone.
    fn diff(before: &Self, after: &Self) -> Self::Changes;

    /// Every rule that `after` breaks, given what changed to make it
    /// from `before`. None means it may become a document.
    ///
    /// `before` kept the rules, so only what `changes` names needs
    /// looking at: a check of a hundred thousand clips after one of them
    /// moved is a check of one lane. When a document is being read from
    /// a file, `before` is empty and everything is in `changes`.
    fn check(before: &Self, after: &Self, changes: &Self::Changes) -> Vec<Self::Violation>;

    /// Shows every table to `each`.
    fn tables(&self, each: &mut dyn FnMut(&dyn AnyTable));

    /// `restored` with the view state of `present` put over it.
    ///
    /// View state is what is saved with a document and is no business of
    /// undo: which timeline is showing, the order of the tabs. When the
    /// history steps back or forward it restores a root, and asks this
    /// for that root as it should now be shown: everything as `restored`
    /// has it, except the view state, which is as `present` has it
    /// wherever `restored` can hold it. Where it cannot - the timeline
    /// that was showing does not exist in `restored` - keep what
    /// `restored` has.
    ///
    /// The same function is what makes a change view state: a commit
    /// whose every change this would carry back is not an undo step. See
    /// [`History`](crate::History).
    ///
    /// The default is for a schema with no view state: `restored` as it
    /// is.
    fn carry_view(present: &Self, restored: &Self) -> Self {
        let _ = present;
        restored.clone()
    }
}

/// Why a document was not made.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Rejected<V> {
    /// Ids were added that the document's mint never made. An id comes
    /// from [`Draft::mint`]; one from anywhere else may be given out
    /// again later, to something else.
    Unminted(Vec<AnyId>),
    /// The schema's rules that the change would break.
    Invalid(Vec<V>),
}

impl<V: fmt::Debug> fmt::Display for Rejected<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rejected::Unminted(ids) => write!(f, "ids the document never made: {ids:?}"),
            Rejected::Invalid(broken) => write!(f, "the change breaks the rules: {broken:?}"),
        }
    }
}

impl<V: fmt::Debug> std::error::Error for Rejected<V> {}

/// Checks a schema that is about to become a document.
fn admit<S: Schema>(
    before: &S,
    after: &S,
    mint: &Mint,
    changes: &S::Changes,
) -> Result<(), Rejected<S::Violation>> {
    let mut unminted = changes.added();
    unminted.retain(|id| !mint.covers(id.raw()));
    if !unminted.is_empty() {
        return Err(Rejected::Unminted(unminted));
    }
    let broken = S::check(before, after, changes);
    if !broken.is_empty() {
        return Err(Rejected::Invalid(broken));
    }
    Ok(())
}

/// A document at one revision: tables that keep their rules, the mint,
/// and the revision's number.
///
/// It cannot be changed, and a copy is one pointer. It reads as its
/// schema does: `document.clips.get(id)`.
pub struct Document<S> {
    inner: Arc<Inner<S>>,
}

struct Inner<S> {
    tables: S,
    mint: Mint,
    revision: Revision,
}

impl<S> Clone for Document<S> {
    /// One pointer copied.
    fn clone(&self) -> Self {
        Document {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S> Deref for Document<S> {
    type Target = S;

    fn deref(&self) -> &S {
        &self.inner.tables
    }
}

impl<S: fmt::Debug> fmt::Debug for Document<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Document")
            .field("revision", &self.inner.revision)
            .field("mint", &self.inner.mint)
            .field("tables", &self.inner.tables)
            .finish()
    }
}

impl<S> Document<S> {
    /// A document made of parts already known to belong together: the
    /// tables of one that passed its checks, or just did.
    pub(crate) fn assemble(tables: S, mint: Mint, revision: Revision) -> Self {
        Document {
            inner: Arc::new(Inner {
                tables,
                mint,
                revision,
            }),
        }
    }

    /// The tables and plain values.
    pub fn tables(&self) -> &S {
        &self.inner.tables
    }

    /// The counts ids are made from.
    pub fn mint(&self) -> &Mint {
        &self.inner.mint
    }

    /// Which revision this is.
    pub fn revision(&self) -> Revision {
        self.inner.revision
    }

    /// Whether the two are one and the same document, as a copy is.
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Arc::ptr_eq(&a.inner, &b.inner)
    }
}

impl<S: Schema> Document<S> {
    /// A new, empty document at revision zero. An empty schema is taken
    /// to keep its own rules.
    pub fn new() -> Self
    where
        S: Default,
    {
        Document::assemble(S::default(), Mint::new(), Revision::default())
    }

    /// A document from what a file held: its tables, its mint and its
    /// revision.
    ///
    /// Checked as a commit is, with everything counted as added: every
    /// id must be one the mint has made, and the schema's rules must
    /// hold. What a reader does with a file that fails - set the named
    /// items aside and try again, or give up - is the reader's to decide;
    /// nothing is repaired here.
    pub fn load(tables: S, mint: Mint, revision: Revision) -> Result<Self, Rejected<S::Violation>>
    where
        S: Default,
    {
        let empty = S::default();
        let changes = S::diff(&empty, &tables);
        admit(&empty, &tables, &mint, &changes)?;
        Ok(Document::assemble(tables, mint, revision))
    }

    /// Starts a change.
    ///
    /// `label` is what the change is called where a person will read it
    /// ("Split clip"); `origin` is who is making it. The draft is taken
    /// to be planned against this revision; a caller that decided what to
    /// do while looking at an older one says so with
    /// [`Draft::planned_at`].
    pub fn begin(&self, label: impl Into<String>, origin: Origin) -> Draft<S> {
        Draft {
            base: self.clone(),
            tables: self.tables().clone(),
            mint: self.mint().clone(),
            label: label.into(),
            origin,
            planned_at: self.revision(),
        }
    }

    /// What changed to make this document from an older one.
    pub fn changes_since(&self, older: &Self) -> S::Changes {
        S::diff(older.tables(), self.tables())
    }

    /// Whether the two hold the same tables and plain values, whatever
    /// their revisions and mints.
    pub fn same_content(&self, other: &Self) -> bool {
        Document::ptr_eq(self, other) || S::diff(self.tables(), other.tables()).is_empty()
    }

    /// Whether there is something under an id, in the table of its kind.
    pub fn contains(&self, id: impl Into<AnyId>) -> bool {
        let id = id.into();
        let mut found = false;
        self.tables().tables(&mut |table| {
            found |= table.tag() == id.tag() && table.holds(id.raw());
        });
        found
    }

    /// Whether a change that was planned against an older revision may
    /// still be made to this one.
    ///
    /// Someone who reads the document, thinks for a while and then
    /// writes may find it changed under them. The rule is the simplest
    /// that is safe: the plan stands if everything it names is still
    /// here. `names` is every id the plan read or means to write. A plan
    /// names ids and never places, so if its ids are here it still means
    /// what it meant; if one is gone, the plan is refused and the missing
    /// are named, for its maker to look again.
    pub fn check_plan(
        &self,
        planned_at: Revision,
        names: impl IntoIterator<Item = AnyId>,
    ) -> Result<(), StalePlan> {
        let now = self.revision();
        if planned_at > now {
            return Err(StalePlan::NotThisDocument { planned_at, now });
        }
        let mut missing: Vec<AnyId> = names.into_iter().filter(|id| !self.contains(*id)).collect();
        if missing.is_empty() {
            return Ok(());
        }
        missing.sort();
        missing.dedup();
        Err(StalePlan::Missing {
            planned_at,
            now,
            missing,
        })
    }
}

impl<S: Schema + Default> Default for Document<S> {
    fn default() -> Self {
        Document::new()
    }
}

/// Why a plan made against an older revision may not be carried out now.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum StalePlan {
    /// Things the plan names are no longer in the document.
    Missing {
        /// The revision the plan was made against.
        planned_at: Revision,
        /// The revision the document is at.
        now: Revision,
        /// What is gone, each id once, in order.
        missing: Vec<AnyId>,
    },
    /// The plan says it was made against a revision this document has
    /// not reached: it was made for some other document.
    NotThisDocument {
        /// The revision the plan was made against.
        planned_at: Revision,
        /// The revision the document is at.
        now: Revision,
    },
}

impl fmt::Display for StalePlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StalePlan::Missing {
                planned_at,
                now,
                missing,
            } => write!(
                f,
                "planned at revision {planned_at}, and by revision {now} these are gone: {missing:?}"
            ),
            StalePlan::NotThisDocument { planned_at, now } => write!(
                f,
                "planned at revision {planned_at}, which this document, at {now}, has not reached"
            ),
        }
    }
}

impl std::error::Error for StalePlan {}

/// A change being made: a private copy of a document's tables that may
/// be edited freely and is checked only when it is committed.
///
/// It reads and writes as its schema does: `draft.clips.insert(id, clip)`.
/// Nothing outside sees it until [`commit`](Draft::commit) returns a new
/// document. Dropping it is the cancel: the document it started from was
/// never touched, and the ids it made were never used.
///
/// # A gesture
///
/// A drag is one draft held for as long as the pointer is down. Each
/// pointer event calls [`recompute`](Draft::recompute) with the whole
/// movement so far, which starts again from the document the drag began
/// on. Nothing adds up from event to event, so nothing drifts, and the
/// result is the same as one command with the total. If someone else
/// commits in the middle, [`carry_over`](Draft::carry_over) moves the
/// drag onto their document and runs it again there.
pub struct Draft<S: Schema> {
    base: Document<S>,
    tables: S,
    mint: Mint,
    label: String,
    origin: Origin,
    planned_at: Revision,
}

impl<S: Schema> Draft<S> {
    /// Says that this change was decided on while looking at an older
    /// revision than the one it is being made to. It is carried on the
    /// commit for whoever keeps a record.
    pub fn planned_at(mut self, revision: Revision) -> Self {
        self.planned_at = revision;
        self
    }

    /// The document this change is being made to.
    pub fn base(&self) -> &Document<S> {
        &self.base
    }

    /// What the change is called.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Who is making it.
    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// The revision it was planned against.
    pub fn planned_revision(&self) -> Revision {
        self.planned_at
    }

    /// The tables as the draft has them now.
    pub fn tables(&self) -> &S {
        &self.tables
    }

    /// The tables, to edit.
    pub fn tables_mut(&mut self) -> &mut S {
        &mut self.tables
    }

    /// A new id, from the project's own count.
    ///
    /// The count goes up in the draft, and in the document only if the
    /// draft is committed. An id made in a draft that is dropped, or
    /// recomputed, was never in any document and may be made again.
    pub fn mint<I: TypedId>(&mut self) -> Result<I, MintError> {
        self.mint.mint(Writer::LOCAL)
    }

    /// A new id from another writer's count. See [`Writer`].
    pub fn mint_as<I: TypedId>(&mut self, writer: Writer) -> Result<I, MintError> {
        self.mint.mint(writer)
    }

    /// What the draft has changed so far.
    pub fn changes(&self) -> S::Changes {
        S::diff(self.base.tables(), &self.tables)
    }

    /// Throws away everything done so far and starts again from the
    /// document the draft was begun on, ids and all.
    pub fn restart(&mut self) {
        self.tables = self.base.tables().clone();
        self.mint = self.base.mint().clone();
    }

    /// Starts again from the document the draft was begun on, and makes
    /// the change afresh.
    ///
    /// What a gesture does on every input. Because it starts from the
    /// same place with the same counts each time, a change that makes
    /// new things gives them the same ids each time. If `change` fails,
    /// the draft is left as it was begun, with nothing done.
    pub fn recompute<E>(
        &mut self,
        change: impl FnOnce(&mut Self) -> Result<(), E>,
    ) -> Result<(), E> {
        self.restart();
        change(self).inspect_err(|_| self.restart())
    }

    /// Moves the draft onto a newer document and makes the change again
    /// there.
    ///
    /// For when someone else committed while this draft was open. What
    /// was done on the old document is dropped; `change` is run on the
    /// new one, and the draft is then planned against it. If `change`
    /// fails - what it was moving is gone - the draft is left on the
    /// newer document with nothing done, for the caller to drop.
    pub fn carry_over<E>(
        &mut self,
        newer: &Document<S>,
        change: impl FnOnce(&mut Self) -> Result<(), E>,
    ) -> Result<(), E> {
        self.base = newer.clone();
        self.planned_at = newer.revision();
        self.recompute(change)
    }

    /// Checks the change and makes it a document.
    ///
    /// What changed is worked out by comparing the draft with the
    /// document it began on; the ids that were added must have come from
    /// the mint; the schema checks the rest. If anything is wrong the
    /// answer is what, and no document is made: the one the draft began
    /// on is as it was, and so is everyone holding it.
    ///
    /// A draft that changed nothing gives back the document it began on,
    /// at the same revision. One that only made ids gives a new revision
    /// with the same tables, so that the ids are not made again.
    pub fn commit(self) -> Result<Commit<S>, Rejected<S::Violation>> {
        let Draft {
            base,
            tables,
            mint,
            label,
            origin,
            planned_at,
        } = self;
        let changes = S::diff(base.tables(), &tables);
        let after = if !changes.is_empty() {
            admit(base.tables(), &tables, &mint, &changes)?;
            Document::assemble(tables, mint, base.revision().next())
        } else if mint != *base.mint() {
            Document::assemble(base.tables().clone(), mint, base.revision().next())
        } else {
            base.clone()
        };
        Ok(Commit {
            before: base,
            after,
            changes,
            label,
            origin,
            planned_at,
        })
    }
}

impl<S: Schema> Deref for Draft<S> {
    type Target = S;

    fn deref(&self) -> &S {
        &self.tables
    }
}

impl<S: Schema> DerefMut for Draft<S> {
    fn deref_mut(&mut self) -> &mut S {
        &mut self.tables
    }
}

/// A change that passed its checks: the document before, the document
/// after, and what is different between them.
///
/// The changes are what everything that follows a document reads: what
/// to draw again, what to compile again, what to save. They were worked
/// out for the check, so they are handed on and not worked out twice.
pub struct Commit<S: Schema> {
    pub(crate) before: Document<S>,
    pub(crate) after: Document<S>,
    pub(crate) changes: S::Changes,
    pub(crate) label: String,
    pub(crate) origin: Origin,
    pub(crate) planned_at: Revision,
}

impl<S: Schema> Commit<S> {
    /// The document the change was made to.
    pub fn before(&self) -> &Document<S> {
        &self.before
    }

    /// The document the change made. To be made current.
    pub fn document(&self) -> &Document<S> {
        &self.after
    }

    /// The same, taking the commit apart.
    pub fn into_document(self) -> Document<S> {
        self.after
    }

    /// What is different between the two.
    pub fn changes(&self) -> &S::Changes {
        &self.changes
    }

    /// Whether the draft did nothing at all: no change and no id made.
    /// The document is then the one it began on.
    pub fn is_nothing(&self) -> bool {
        Document::ptr_eq(&self.before, &self.after)
    }

    /// What the change is called.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Who made it.
    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// The revision it was planned against.
    pub fn planned_at(&self) -> Revision {
        self.planned_at
    }
}

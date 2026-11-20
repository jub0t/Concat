// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The document and its history against a model too simple to be wrong.
//!
//! Long random lists of everything a caller can do - changes that are
//! accepted, refused, abandoned, joined, of view only; undo and redo; a
//! budget that shrinks and grows - are run on a [`History`] and on a
//! model that keeps each table as a plain sorted map and copies the whole
//! library at every step. After every one, the two must agree on
//! everything that can be seen from outside:
//!
//! - the tables, the count ids are made from, and the revision;
//! - what was recorded, and as what: a step, a joined step, view only,
//!   nothing, or refused;
//! - what the change list says changed, against a comparison of the
//!   model's whole library before and after;
//! - every step still kept, with both of its documents as they were when
//!   it was recorded, however much has been edited since;
//! - that a refused or abandoned change left the very same document;
//! - that no id was made twice.
//!
//! The model's rules are written out again here in the plainest way, not
//! shared with the code under test.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Book, BookId, Key, Lib, Library, LibraryChanges, Sel, Shelf, ShelfId, book, shelf};
use concat_doc::{
    Budget, ChangeSet, Document, Draft, Entry, History, Origin, Recorded, Replaced, Revision,
    TypedId, Writer,
};
use proptest::prelude::*;
use proptest::sample::Index;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};

/// A small repeat joins the step before it within this long.
const SOON: u64 = 500;

/// A shelf in the model: its name and its scroll.
type PlainShelf = (String, u32);
/// A book in the model: its shelf and its title.
type PlainBook = (u64, String);

/// The whole library as plain maps, by the number in each id.
#[derive(Clone, PartialEq, Debug, Default)]
struct Plain {
    shelves: BTreeMap<u64, PlainShelf>,
    books: BTreeMap<u64, PlainBook>,
    name: String,
    open: Option<u64>,
}

impl Plain {
    /// The library's rules, checked over all of it.
    fn keeps_the_rules(&self) -> bool {
        let books = self
            .books
            .values()
            .all(|(on, title)| !title.is_empty() && self.shelves.contains_key(on));
        books
            && self
                .open
                .is_none_or(|open| self.shelves.contains_key(&open))
    }

    /// `restored` shown with the view of `present`: the open shelf if it
    /// is there, and every shelf's scroll.
    fn with_view_of(restored: &Plain, present: &Plain) -> Plain {
        let mut shown = restored.clone();
        if present
            .open
            .is_none_or(|open| restored.shelves.contains_key(&open))
        {
            shown.open = present.open;
        }
        for (id, held) in &mut shown.shelves {
            if let Some((_, scroll)) = present.shelves.get(id) {
                held.1 = *scroll;
            }
        }
        shown
    }

    /// Whether the only difference from `older` is view.
    fn only_view_since(&self, older: &Plain) -> bool {
        Plain::with_view_of(older, self) == *self
    }

    fn nth_shelf(&self, pick: Index) -> Option<u64> {
        let at = pick.index(self.shelves.len().max(1));
        self.shelves.keys().nth(at).copied()
    }

    fn nth_book(&self, pick: Index) -> Option<u64> {
        let at = pick.index(self.books.len().max(1));
        self.books.keys().nth(at).copied()
    }
}

/// What a full comparison of two libraries finds, in the shape a change
/// list is put into to be compared with it.
#[derive(PartialEq, Debug, Default)]
struct Found {
    shelves_added: Vec<(u64, PlainShelf)>,
    shelves_removed: Vec<(u64, PlainShelf)>,
    shelves_replaced: Vec<(u64, PlainShelf, PlainShelf)>,
    books_added: Vec<(u64, PlainBook)>,
    books_removed: Vec<(u64, PlainBook)>,
    books_replaced: Vec<(u64, PlainBook, PlainBook)>,
    name: Option<(String, String)>,
    open: Option<(Option<u64>, Option<u64>)>,
}

/// What two plain maps differ by: the entries only the newer has, those
/// only the older has, and those in both that are not equal.
type Compared<T> = (Vec<(u64, T)>, Vec<(u64, T)>, Vec<(u64, T, T)>);

/// Compares two plain maps entry by entry.
fn compare<T: Clone + PartialEq>(old: &BTreeMap<u64, T>, new: &BTreeMap<u64, T>) -> Compared<T> {
    let only = |a: &BTreeMap<u64, T>, b: &BTreeMap<u64, T>| {
        a.iter()
            .filter(|(id, _)| !b.contains_key(id))
            .map(|(id, v)| (*id, v.clone()))
            .collect()
    };
    let replaced = old
        .iter()
        .filter_map(|(id, was)| new.get(id).map(|is| (*id, was.clone(), is.clone())))
        .filter(|(_, was, is)| was != is)
        .collect();
    (only(new, old), only(old, new), replaced)
}

fn found_by_comparing(old: &Plain, new: &Plain) -> Found {
    let (shelves_added, shelves_removed, shelves_replaced) = compare(&old.shelves, &new.shelves);
    let (books_added, books_removed, books_replaced) = compare(&old.books, &new.books);
    Found {
        shelves_added,
        shelves_removed,
        shelves_replaced,
        books_added,
        books_removed,
        books_replaced,
        name: (old.name != new.name).then(|| (old.name.clone(), new.name.clone())),
        open: (old.open != new.open).then_some((old.open, new.open)),
    }
}

fn plain_shelf(shelf: &Arc<Shelf>) -> PlainShelf {
    (shelf.name.clone(), shelf.scroll)
}

fn plain_book(book: &Arc<Book>) -> PlainBook {
    (book.shelf.counter(), book.title.clone())
}

fn found_in(changes: &LibraryChanges) -> Found {
    fn pairs<I: TypedId, T, P>(rows: &[(I, Arc<T>)], plain: fn(&Arc<T>) -> P) -> Vec<(u64, P)> {
        rows.iter()
            .map(|(id, row)| (id.counter(), plain(row)))
            .collect()
    }
    fn triples<I: TypedId, T, P>(
        rows: &[Replaced<I, T>],
        plain: fn(&Arc<T>) -> P,
    ) -> Vec<(u64, P, P)> {
        rows.iter()
            .map(|row| (row.id.counter(), plain(&row.old), plain(&row.new)))
            .collect()
    }
    let number = |id: Option<ShelfId>| id.map(TypedId::counter);
    Found {
        shelves_added: pairs(&changes.shelves.added, plain_shelf),
        shelves_removed: pairs(&changes.shelves.removed, plain_shelf),
        shelves_replaced: triples(&changes.shelves.replaced, plain_shelf),
        books_added: pairs(&changes.books.added, plain_book),
        books_removed: pairs(&changes.books.removed, plain_book),
        books_replaced: triples(&changes.books.replaced, plain_book),
        name: changes
            .name
            .as_ref()
            .map(|name| (name.old.clone(), name.new.clone())),
        open: changes
            .open
            .as_ref()
            .map(|open| (number(open.old), number(open.new))),
    }
}

fn plain_of(document: &Document<Library>) -> Plain {
    Plain {
        shelves: document
            .shelves
            .iter()
            .map(|(id, row)| (id.counter(), plain_shelf(row)))
            .collect(),
        books: document
            .books
            .iter()
            .map(|(id, row)| (id.counter(), plain_book(row)))
            .collect(),
        name: document.name.clone(),
        open: document.open.map(TypedId::counter),
    }
}

/// One thing a caller does.
#[derive(Clone, Debug)]
struct Transition {
    /// How long after the one before.
    wait: u64,
    who: u32,
    /// What is selected once it is done.
    select: Sel,
    op: Op,
}

#[derive(Clone, Debug)]
enum Op {
    AddShelf(String),
    AddBook {
        shelf: Index,
        title: String,
    },
    Retitle {
        book: Index,
        title: String,
        join: bool,
    },
    /// A small repeat: the first book retitled, always able to join.
    Nudge(String),
    MoveBook {
        book: Index,
        shelf: Index,
    },
    RemoveBook(Index),
    /// Refused while a book is on it.
    RemoveShelf(Index),
    /// A shelf and every book on it, as one change of many commands.
    ClearShelf(Index),
    Rename(String),
    Open(Option<Index>),
    Scroll {
        shelf: Index,
        to: u32,
    },
    /// Always refused: a book with no title.
    Untitle(Index),
    /// A draft that is edited and dropped.
    Abandon,
    /// A draft that makes an id and uses it for nothing.
    Reserve,
    Undo,
    Redo,
    SetBudget(usize),
}

impl Op {
    /// What a caller would call the change.
    fn label(&self) -> &'static str {
        match self {
            Op::AddShelf(_) => "Add shelf",
            Op::AddBook { .. } => "Add book",
            Op::Retitle { .. } | Op::Nudge(_) => "Retitle",
            Op::MoveBook { .. } => "Move book",
            Op::RemoveBook(_) => "Remove book",
            Op::RemoveShelf(_) => "Remove shelf",
            Op::ClearShelf(_) => "Clear shelf",
            Op::Rename(_) => "Rename",
            Op::Open(_) => "Open",
            Op::Scroll { .. } => "Scroll",
            Op::Untitle(_) => "Untitle",
            Op::Abandon => "Abandon",
            Op::Reserve => "Reserve",
            Op::Undo => "Undo",
            Op::Redo => "Redo",
            Op::SetBudget(_) => "Set budget",
        }
    }
}

/// What a transition should come to.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Outcome {
    /// There was nothing to do it to.
    Skipped,
    Refused,
    Recorded(Recorded),
    Stepped,
    NothingToStep,
    BudgetSet,
}

/// A change worked out against the model's library, not yet checked.
struct Planned {
    after: Plain,
    minted: u64,
    label: &'static str,
    commands: usize,
    key: Option<Key>,
}

/// A step as the model keeps it: two whole copies of the library.
#[derive(Clone, Debug)]
struct PlainStep {
    before: Plain,
    after: Plain,
    label: &'static str,
    who: u32,
    key: Option<Key>,
    when: u64,
    selected_before: Sel,
    selected_after: Sel,
    commands: usize,
}

#[derive(Clone, Debug)]
struct Model {
    now: Plain,
    /// The last id made that a document has seen.
    count: u64,
    revision: u64,
    steps: Vec<PlainStep>,
    /// How many of the steps can be undone; the rest can be redone.
    cursor: usize,
    /// Whether the newest step can still be joined.
    joinable: bool,
    clock: u64,
    max_steps: usize,
    selected: Sel,
    /// What the last transition came to.
    outcome: Outcome,
}

impl Model {
    /// What an op would do to the library as it is now. `None` when it
    /// has nothing to act on.
    fn plan(&self, op: &Op) -> Option<Planned> {
        let mut after = self.now.clone();
        let next = self.count + 1;
        let (mut minted, mut commands, mut key) = (0, 1, None);
        let label = match op {
            Op::AddShelf(name) => {
                after.shelves.insert(next, (name.clone(), 0));
                minted = 1;
                "Add shelf"
            }
            Op::AddBook { shelf, title } => {
                // With no shelf to put it on it goes on one that is not
                // there, and is refused.
                let on = after.nth_shelf(*shelf).unwrap_or(NO_SHELF);
                after.books.insert(next, (on, title.clone()));
                minted = 1;
                "Add book"
            }
            Op::Retitle { book, title, join } => {
                let id = after.nth_book(*book)?;
                after.books.get_mut(&id)?.1 = title.clone();
                key = join.then_some(id);
                "Retitle"
            }
            Op::Nudge(title) => {
                let id = *after.books.keys().next()?;
                after.books.get_mut(&id)?.1 = title.clone();
                key = Some(id);
                "Retitle"
            }
            Op::MoveBook { book, shelf } => {
                let (id, to) = (after.nth_book(*book)?, after.nth_shelf(*shelf)?);
                after.books.get_mut(&id)?.0 = to;
                "Move book"
            }
            Op::RemoveBook(book) => {
                let id = after.nth_book(*book)?;
                after.books.remove(&id);
                "Remove book"
            }
            Op::RemoveShelf(shelf) => {
                let id = after.nth_shelf(*shelf)?;
                after.shelves.remove(&id);
                if after.open == Some(id) {
                    after.open = None;
                }
                "Remove shelf"
            }
            Op::ClearShelf(shelf) => {
                let id = after.nth_shelf(*shelf)?;
                after.shelves.remove(&id);
                let books = after.books.len();
                after.books.retain(|_, (on, _)| *on != id);
                commands += books - after.books.len();
                if after.open == Some(id) {
                    after.open = None;
                }
                "Clear shelf"
            }
            Op::Rename(name) => {
                after.name = name.clone();
                "Rename"
            }
            Op::Open(None) => {
                after.open = None;
                "Open"
            }
            Op::Open(Some(shelf)) => {
                after.open = Some(after.nth_shelf(*shelf)?);
                "Open"
            }
            Op::Scroll { shelf, to } => {
                let id = after.nth_shelf(*shelf)?;
                after.shelves.get_mut(&id)?.1 = *to;
                "Scroll"
            }
            Op::Untitle(book) => {
                let id = after.nth_book(*book)?;
                after.books.get_mut(&id)?.1.clear();
                "Untitle"
            }
            Op::Reserve => {
                minted = 1;
                "Reserve"
            }
            Op::Abandon | Op::Undo | Op::Redo | Op::SetBudget(_) => return None,
        };
        Some(Planned {
            after,
            minted,
            label,
            commands,
            key,
        })
    }

    /// A planned change, checked, made current, and recorded.
    fn commit(&mut self, planned: Planned, who: u32, select: Sel) -> Outcome {
        let Planned {
            after,
            minted,
            label,
            commands,
            key,
        } = planned;
        if !after.keeps_the_rules() {
            return Outcome::Refused;
        }
        let before = self.now.clone();
        if after == before {
            if minted > 0 {
                self.count += minted;
                self.revision += 1;
            }
            return Outcome::Recorded(Recorded::Nothing);
        }
        self.count += minted;
        self.revision += 1;
        self.now = after.clone();
        if after.only_view_since(&before) {
            return Outcome::Recorded(Recorded::View);
        }

        self.steps.truncate(self.cursor);
        let selected_before = std::mem::replace(&mut self.selected, select);
        let joins = self.joinable
            && key.is_some()
            && self.steps.last().is_some_and(|last| {
                last.key == key && last.who == who && self.clock - last.when <= SOON
            });
        if joins {
            let mut last = self.steps.pop().expect("a step to join");
            self.cursor -= 1;
            if last.before == after || after.only_view_since(&last.before) {
                self.joinable = false;
                return Outcome::Recorded(Recorded::Cancelled);
            }
            last.after = after;
            last.selected_after = select;
            last.when = self.clock;
            last.commands += commands;
            self.push(last);
            return Outcome::Recorded(Recorded::Joined);
        }
        self.push(PlainStep {
            before,
            after,
            label,
            who,
            key,
            when: self.clock,
            selected_before,
            selected_after: select,
            commands,
        });
        Outcome::Recorded(Recorded::Step)
    }

    fn push(&mut self, step: PlainStep) {
        self.steps.push(step);
        self.cursor = self.steps.len();
        self.joinable = true;
        self.enforce();
    }

    /// Over the count: the oldest step that can be undone goes; one that
    /// can be redone goes only when no such step is left, the farthest
    /// first; the last step of all stays.
    fn enforce(&mut self) {
        while self.steps.len() > self.max_steps && self.steps.len() > 1 {
            if self.cursor > 0 {
                self.steps.remove(0);
                self.cursor -= 1;
            } else {
                self.steps.pop();
            }
        }
    }

    /// Makes one of a step's two libraries current, with the view as it
    /// is now.
    fn step_to(&mut self, root: Plain, selected: Sel) {
        let shown = Plain::with_view_of(&root, &self.now);
        self.now = if shown.keeps_the_rules() { shown } else { root };
        self.revision += 1;
        self.selected = selected;
        self.joinable = false;
    }

    fn apply(&mut self, transition: &Transition) -> Outcome {
        self.clock += transition.wait;
        match &transition.op {
            Op::Undo => {
                let Some(step) = self.cursor.checked_sub(1).map(|at| self.steps[at].clone()) else {
                    return Outcome::NothingToStep;
                };
                self.step_to(step.before, step.selected_before);
                self.cursor -= 1;
                Outcome::Stepped
            }
            Op::Redo => {
                let Some(step) = self.steps.get(self.cursor).cloned() else {
                    return Outcome::NothingToStep;
                };
                self.step_to(step.after, step.selected_after);
                self.cursor += 1;
                Outcome::Stepped
            }
            Op::SetBudget(max_steps) => {
                self.max_steps = *max_steps;
                self.enforce();
                Outcome::BudgetSet
            }
            op => match self.plan(op) {
                Some(planned) => self.commit(planned, transition.who, transition.select),
                None => Outcome::Skipped,
            },
        }
    }
}

/// A shelf no library has: its number is past any the tests reach.
const NO_SHELF: u64 = 1 << 40;

const START_BUDGET: usize = 6;

struct LibraryModel;

impl ReferenceStateMachine for LibraryModel {
    type State = Model;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Model> {
        Just(Model {
            now: Plain::default(),
            count: 0,
            revision: 0,
            steps: Vec::new(),
            cursor: 0,
            joinable: false,
            clock: 0,
            max_steps: START_BUDGET,
            selected: 0,
            outcome: Outcome::Skipped,
        })
        .boxed()
    }

    fn transitions(model: &Model) -> BoxedStrategy<Transition> {
        // Few names, so that a change to the name it already has - which
        // is no change - comes up often.
        let text = || "[ab]{1,2}";
        let pick = || any::<Index>();
        // What has nothing to act on yet is still tried, but seldom.
        let shelves = !model.now.shelves.is_empty();
        let books = !model.now.books.is_empty();
        let on = |weight: u32, there: bool| if there { weight } else { 1 };
        let op = prop_oneof![
            if shelves { 2 } else { 12 } => text().prop_map(Op::AddShelf),
            on(6, shelves) => (pick(), text()).prop_map(|(shelf, title)| Op::AddBook { shelf, title }),
            on(5, books) => (pick(), text(), prop::bool::weighted(0.7))
                .prop_map(|(book, title, join)| Op::Retitle { book, title, join }),
            on(10, books) => text().prop_map(Op::Nudge),
            on(2, books) => (pick(), pick()).prop_map(|(book, shelf)| Op::MoveBook { book, shelf }),
            on(2, books) => pick().prop_map(Op::RemoveBook),
            1 => pick().prop_map(Op::RemoveShelf),
            1 => pick().prop_map(Op::ClearShelf),
            1 => text().prop_map(Op::Rename),
            on(2, shelves) => prop::option::weighted(0.8, pick()).prop_map(Op::Open),
            on(2, shelves) => (pick(), 0..3_u32).prop_map(|(shelf, to)| Op::Scroll { shelf, to }),
            1 => pick().prop_map(Op::Untitle),
            1 => Just(Op::Abandon),
            1 => Just(Op::Reserve),
            on(7, !model.steps.is_empty()) => Just(Op::Undo),
            on(5, model.cursor < model.steps.len()) => Just(Op::Redo),
            1 => (1..8_usize).prop_map(Op::SetBudget),
        ];
        let wait = prop_oneof![
            3 => Just(0),
            3 => Just(100),
            1 => Just(SOON),
            1 => Just(SOON + 1),
            1 => Just(5000),
        ];
        let who = prop_oneof![6 => Just(1_u32), 1 => Just(2_u32)];
        (wait, who, 1..1000_u32, op)
            .prop_map(|(wait, who, select, op)| Transition {
                wait,
                who,
                select,
                op,
            })
            .boxed()
    }

    fn apply(mut model: Model, transition: &Transition) -> Model {
        model.outcome = model.apply(transition);
        model
    }
}

/// The history under test, and what the test remembers beside it.
struct Tested {
    history: Lib,
    started: Instant,
    selected: Sel,
    /// The model's library as it was before the last transition.
    was: Plain,
    /// Every id that came out of a commit.
    made: BTreeSet<u64>,
    /// Documents from along the way, with the library each one was.
    kept: Vec<(Document<Library>, Plain)>,
}

fn shelf_id(number: u64) -> ShelfId {
    format!("s{number}").parse().expect("a shelf's id")
}

impl Tested {
    fn nth_shelf(&self, pick: Index) -> Option<ShelfId> {
        let shelves = &self.history.current().shelves;
        shelves.ids().nth(pick.index(shelves.len().max(1)))
    }

    fn nth_book(&self, pick: Index) -> Option<BookId> {
        let books = &self.history.current().books;
        books.ids().nth(pick.index(books.len().max(1)))
    }

    /// Makes an op's change to a draft, as a caller's command would.
    /// Answers how many commands it was, the key it joins under, and the
    /// ids it made; `None` when it has nothing to act on.
    fn change(
        &self,
        draft: &mut Draft<Library>,
        op: &Op,
    ) -> Option<(usize, Option<Key>, Vec<u64>)> {
        let (mut commands, mut key, mut made) = (1, None, Vec::new());
        match op {
            Op::AddShelf(name) => {
                let id: ShelfId = draft.mint().unwrap();
                draft.shelves.insert(id, shelf(name)).unwrap();
                made.push(id.counter());
            }
            Op::AddBook { shelf, title } => {
                let on = self.nth_shelf(*shelf).unwrap_or_else(|| shelf_id(NO_SHELF));
                let id: BookId = draft.mint().unwrap();
                draft.books.insert(id, book(on, title)).unwrap();
                made.push(id.counter());
            }
            Op::Retitle { book, title, join } => {
                let id = self.nth_book(*book)?;
                draft
                    .books
                    .update(id, |book| book.title = title.clone())
                    .unwrap();
                key = join.then_some(id.counter());
            }
            Op::Nudge(title) => {
                let id = self.history.current().books.ids().next()?;
                draft
                    .books
                    .update(id, |book| book.title = title.clone())
                    .unwrap();
                key = Some(id.counter());
            }
            Op::MoveBook { book, shelf } => {
                let (id, to) = (self.nth_book(*book)?, self.nth_shelf(*shelf)?);
                draft.books.update(id, |book| book.shelf = to).unwrap();
            }
            Op::RemoveBook(book) => {
                draft.books.remove(self.nth_book(*book)?).unwrap();
            }
            Op::RemoveShelf(shelf) => {
                let id = self.nth_shelf(*shelf)?;
                draft.shelves.remove(id).unwrap();
                if draft.open == Some(id) {
                    draft.open = None;
                }
            }
            Op::ClearShelf(shelf) => {
                let id = self.nth_shelf(*shelf)?;
                draft.shelves.remove(id).unwrap();
                let on_it: Vec<BookId> = draft
                    .books
                    .iter()
                    .filter(|(_, book)| book.shelf == id)
                    .map(|(id, _)| id)
                    .collect();
                for book in on_it {
                    draft.books.remove(book).unwrap();
                    commands += 1;
                }
                if draft.open == Some(id) {
                    draft.open = None;
                }
            }
            Op::Rename(name) => {
                draft.name = name.clone();
            }
            Op::Open(None) => {
                draft.open = None;
            }
            Op::Open(Some(shelf)) => {
                draft.open = Some(self.nth_shelf(*shelf)?);
            }
            Op::Scroll { shelf, to } => {
                let id = self.nth_shelf(*shelf)?;
                draft
                    .shelves
                    .update(id, |shelf| shelf.scroll = *to)
                    .unwrap();
            }
            Op::Untitle(book) => {
                draft
                    .books
                    .update(self.nth_book(*book)?, |book| book.title.clear())
                    .unwrap();
            }
            Op::Reserve => {
                let id: BookId = draft.mint().unwrap();
                made.push(id.counter());
            }
            Op::Abandon | Op::Undo | Op::Redo | Op::SetBudget(_) => return None,
        }
        Some((commands, key, made))
    }

    fn apply(&mut self, model: &Model, transition: &Transition) -> Outcome {
        let before = self.history.current().clone();
        let lens = (self.history.undo_len(), self.history.redo_len());
        match &transition.op {
            Op::Undo | Op::Redo => {
                let undo = matches!(transition.op, Op::Undo);
                let step = if undo {
                    self.history.undo()
                } else {
                    self.history.redo()
                };
                let Some(restored) = step else {
                    assert!(Document::ptr_eq(self.history.current(), &before));
                    return Outcome::NothingToStep;
                };
                assert!(Document::ptr_eq(&restored.document, self.history.current()));
                assert_eq!(
                    found_in(&restored.changes),
                    found_by_comparing(&self.was, &model.now)
                );
                self.selected = restored.selection;
                Outcome::Stepped
            }
            Op::SetBudget(max_steps) => {
                self.history.set_budget(Budget::new(*max_steps, usize::MAX));
                assert!(Document::ptr_eq(self.history.current(), &before));
                // The stated rule, said directly: a step that could be
                // redone went only if no step that could be undone is left.
                if self.history.redo_len() < lens.1 {
                    assert_eq!(self.history.undo_len(), 0);
                }
                Outcome::BudgetSet
            }
            Op::Abandon => {
                let mut draft = before.begin("Abandon", Origin(transition.who));
                let id: ShelfId = draft.mint().unwrap();
                draft.shelves.insert(id, shelf("never")).unwrap();
                draft.name.push('!');
                draft.books = Default::default();
                drop(draft);
                assert!(Document::ptr_eq(self.history.current(), &before));
                Outcome::Skipped
            }
            op => {
                let label = op.label();
                let mut draft = before.begin(label, Origin(transition.who));
                let Some((commands, key, made)) = self.change(&mut draft, op) else {
                    return Outcome::Skipped;
                };
                let Ok(commit) = draft.commit() else {
                    assert!(Document::ptr_eq(self.history.current(), &before));
                    return Outcome::Refused;
                };
                assert_eq!(
                    found_in(commit.changes()),
                    found_by_comparing(&self.was, &model.now)
                );
                assert_eq!(commit.changes().is_empty(), self.was == model.now);
                for id in made {
                    assert!(self.made.insert(id), "id {id} was made twice");
                }
                let mut entry = Entry::new(
                    vec![label.to_owned(); commands],
                    self.selected,
                    transition.select,
                )
                .at(self.started + Duration::from_millis(model.clock));
                if let Some(key) = key {
                    entry = entry.coalescing(key, Duration::from_millis(SOON));
                }
                let recorded = self
                    .history
                    .record(commit, entry)
                    .expect("from the current one");
                if matches!(
                    recorded,
                    Recorded::Step | Recorded::Joined | Recorded::Cancelled
                ) {
                    self.selected = transition.select;
                }
                Outcome::Recorded(recorded)
            }
        }
    }
}

struct LibraryTest;

impl StateMachineTest for LibraryTest {
    type SystemUnderTest = Tested;
    type Reference = LibraryModel;

    fn init_test(model: &Model) -> Tested {
        let history: Lib = History::new(Document::new(), Budget::new(model.max_steps, usize::MAX));
        Tested {
            kept: vec![(history.current().clone(), Plain::default())],
            history,
            started: Instant::now(),
            selected: 0,
            was: Plain::default(),
            made: BTreeSet::new(),
        }
    }

    fn apply(mut tested: Tested, model: &Model, transition: Transition) -> Tested {
        let came_to = tested.apply(model, &transition);
        assert_eq!(came_to, model.outcome, "{transition:?}");
        tested.was = model.now.clone();
        // Keep the first document for good and a handful of late ones.
        if tested.kept.len() >= 12 {
            tested.kept.remove(1);
        }
        tested
            .kept
            .push((tested.history.current().clone(), model.now.clone()));
        tested
    }

    fn check_invariants(tested: &Tested, model: &Model) {
        let history = &tested.history;
        let now = history.current();
        assert_eq!(plain_of(now), model.now);
        assert_eq!(now.mint().issued(Writer::LOCAL), model.count);
        assert_eq!(now.revision(), Revision(model.revision));
        assert_eq!(tested.selected, model.selected);
        assert_eq!(history.undo_len(), model.cursor);
        assert_eq!(history.redo_len(), model.steps.len() - model.cursor);
        assert_eq!(history.can_undo(), model.cursor > 0);
        assert_eq!(history.can_redo(), model.cursor < model.steps.len());

        // Every step kept is the step the model kept, and both of its
        // documents are still what they were when it was recorded.
        let undoable: Vec<_> = history.undo_steps().collect();
        let steps = undoable.into_iter().rev().chain(history.redo_steps());
        for (step, plain) in steps.zip(&model.steps) {
            assert_eq!(step.label(), plain.label);
            assert_eq!(step.origin(), Origin(plain.who));
            assert_eq!(step.commands().len(), plain.commands);
            assert_eq!(*step.selection_before(), plain.selected_before);
            assert_eq!(*step.selection_after(), plain.selected_after);
            assert_eq!(plain_of(step.before()), plain.before);
            assert_eq!(plain_of(step.after()), plain.after);
            // What it costs is what its two documents differ by, however
            // many commits were joined into it.
            assert_eq!(
                step.cost(),
                step.after().changes_since(step.before()).cost()
            );
        }
        let costs: usize = history
            .undo_steps()
            .chain(history.redo_steps())
            .map(|s| s.cost())
            .sum();
        assert_eq!(history.bytes(), costs);
        for (document, was) in &tested.kept {
            assert_eq!(plain_of(document), *was);
        }
    }
}

prop_state_machine! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]

    #[test]
    fn the_history_is_a_library_copied_whole_at_every_step(sequential 1..90 => LibraryTest);
}

/// The corpus's check, on random lists: everything done is undone back
/// to the start, then done again to the end, one step an edit.
fn undo_all_then_redo_all(ops: Vec<(Index, String, u8)>) -> Result<(), TestCaseError> {
    let mut history = common::library();
    let first = common::add_shelf(&mut history, "a");
    let mut on_the_way = vec![(history.current().clone(), plain_of(history.current()))];
    for (pick, title, kind) in ops {
        let books: Vec<BookId> = history.current().books.ids().collect();
        let picked = books.get(pick.index(books.len().max(1))).copied();
        let recorded = match (kind % 4, picked) {
            (0, Some(id)) => common::retitle(&mut history, id, &title),
            (1, Some(id)) => common::edit(&mut history, "Remove", |draft| {
                draft.books.remove(id).unwrap();
            }),
            _ => {
                common::add_book(&mut history, first, &title);
                Recorded::Step
            }
        };
        if recorded == Recorded::Step {
            on_the_way.push((history.current().clone(), plain_of(history.current())));
        }
    }
    let count = history.current().mint().issued(Writer::LOCAL);
    prop_assert_eq!(history.undo_len(), on_the_way.len());

    for (document, was) in on_the_way.iter().rev().skip(1) {
        let undone = history.undo().expect("a step for every edit");
        prop_assert!(undone.document.same_content(document));
        prop_assert!(undone.document.books.ptr_eq(&document.books));
        prop_assert_eq!(&plain_of(&undone.document), was);
        prop_assert_eq!(undone.document.mint().issued(Writer::LOCAL), count);
    }
    prop_assert_eq!(
        history
            .undo()
            .expect("the first shelf")
            .document
            .shelves
            .len(),
        0
    );
    prop_assert!(history.undo().is_none());

    for (document, was) in &on_the_way {
        let redone = history.redo().expect("a step for every edit");
        prop_assert!(redone.document.same_content(document));
        prop_assert_eq!(&plain_of(&redone.document), was);
    }
    prop_assert!(history.redo().is_none());
    // The documents along the way were never touched by any of it.
    for (document, was) in &on_the_way {
        prop_assert_eq!(&plain_of(document), was);
    }
    Ok(())
}

proptest! {
    #[test]
    fn everything_done_is_undone_to_the_start_and_redone_to_the_end(
        ops in prop::collection::vec((any::<Index>(), "[a-d]{1,3}", any::<u8>()), 0..60),
    ) {
        undo_all_then_redo_all(ops)?;
    }
}

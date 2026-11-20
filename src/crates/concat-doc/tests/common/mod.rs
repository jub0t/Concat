// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What the document's tests share: a library of shelves and books.
//!
//! Small, and with one of everything a real schema has. Two tables, where
//! a row of one names a row of the other (a book is on a shelf). Rules
//! that a change can break: a book's shelf exists, a book has a title, a
//! shelf with books on it is not taken away. A plain value that is
//! content (the library's name), and view state of both kinds: a plain
//! value (which shelf is open) and a field of a row (how far a shelf is
//! scrolled).

#![allow(dead_code)]

use concat_doc::{
    AnyTable, Budget, ChangePart, ChangeSet, Changed, Document, Draft, Entry, History, Origin,
    Recorded, Rejected, Schema, Table, TableChanges, Weigh,
};

concat_doc::typed_id! {
    /// A shelf.
    pub struct ShelfId = "s";
    /// A book.
    pub struct BookId = "b";
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Shelf {
    pub name: String,
    /// View state: how far down the shelf is scrolled.
    pub scroll: u32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Book {
    pub shelf: ShelfId,
    pub title: String,
}

impl Weigh for Shelf {
    fn weight(&self) -> usize {
        64 + self.name.len()
    }
}

impl Weigh for Book {
    fn weight(&self) -> usize {
        64 + self.title.len()
    }
}

pub fn shelf(name: &str) -> Shelf {
    Shelf {
        name: name.to_owned(),
        scroll: 0,
    }
}

pub fn book(shelf: ShelfId, title: &str) -> Book {
    Book {
        shelf,
        title: title.to_owned(),
    }
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Library {
    pub shelves: Table<ShelfId, Shelf>,
    pub books: Table<BookId, Book>,
    pub name: String,
    /// View state: the shelf that is open.
    pub open: Option<ShelfId>,
}

#[derive(PartialEq, Debug)]
pub struct LibraryChanges {
    pub shelves: TableChanges<ShelfId, Shelf>,
    pub books: TableChanges<BookId, Book>,
    pub name: Option<Changed<String>>,
    pub open: Option<Changed<Option<ShelfId>>>,
}

impl ChangeSet for LibraryChanges {
    fn parts(&self, each: &mut dyn FnMut(&dyn ChangePart)) {
        each(&self.shelves);
        each(&self.books);
        each(&self.name);
        each(&self.open);
    }
}

/// A rule of the library that a change would break.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Broken {
    /// A book is on a shelf that is not there.
    NoSuchShelf { book: BookId, shelf: ShelfId },
    /// A book has no title.
    Untitled(BookId),
    /// A shelf was taken away with a book still on it.
    ShelfInUse { shelf: ShelfId, book: BookId },
    /// The shelf that is open is not there.
    OpenShelfMissing(ShelfId),
}

impl Schema for Library {
    type Changes = LibraryChanges;
    type Violation = Broken;

    fn diff(before: &Self, after: &Self) -> LibraryChanges {
        LibraryChanges {
            shelves: TableChanges::between(&before.shelves, &after.shelves),
            books: TableChanges::between(&before.books, &after.books),
            name: Changed::between(&before.name, &after.name),
            open: Changed::between(&before.open, &after.open),
        }
    }

    fn check(_: &Self, after: &Self, changes: &LibraryChanges) -> Vec<Broken> {
        let mut broken = Vec::new();
        let written = changes.books.added.iter().map(|(id, book)| (*id, book));
        let rewritten = changes
            .books
            .replaced
            .iter()
            .map(|replaced| (replaced.id, &replaced.new));
        for (id, book) in written.chain(rewritten) {
            if book.title.is_empty() {
                broken.push(Broken::Untitled(id));
            }
            if !after.shelves.contains(book.shelf) {
                broken.push(Broken::NoSuchShelf {
                    book: id,
                    shelf: book.shelf,
                });
            }
        }
        for (shelf, _) in &changes.shelves.removed {
            for (id, book) in after.books.iter() {
                // A book moved onto the shelf in this same change is
                // already named above.
                let named = changes.books.ids().any(|changed| changed == id);
                if book.shelf == *shelf && !named {
                    broken.push(Broken::ShelfInUse {
                        shelf: *shelf,
                        book: id,
                    });
                }
            }
        }
        if let Some(open) = after.open
            && !after.shelves.contains(open)
        {
            broken.push(Broken::OpenShelfMissing(open));
        }
        broken
    }

    fn tables(&self, each: &mut dyn FnMut(&dyn AnyTable)) {
        each(&self.shelves);
        each(&self.books);
    }

    /// The open shelf, if `restored` has that shelf; and for every shelf
    /// in both, how far it is scrolled.
    fn carry_view(present: &Self, restored: &Self) -> Self {
        let mut shown = restored.clone();
        if present
            .open
            .is_none_or(|open| restored.shelves.contains(open))
        {
            shown.open = present.open;
        }
        for shelf in TableChanges::between(&restored.shelves, &present.shelves).replaced {
            if shelf.old.scroll != shelf.new.scroll {
                let scrolled = shown
                    .shelves
                    .update(shelf.id, |held| held.scroll = shelf.new.scroll);
                assert_eq!(scrolled, Ok(()));
            }
        }
        shown
    }
}

/// What a test keeps of a command: its name.
pub type Cmd = String;
/// What a test selects: a number.
pub type Sel = u32;
/// What a test joins small repeats by.
pub type Key = u64;

pub type Lib = History<Library, Cmd, Sel, Key>;

pub const WINDOW: Origin = Origin(1);
pub const AGENT: Origin = Origin(2);

/// A history of an empty library, with room for anything.
pub fn library() -> Lib {
    History::new(Document::new(), Budget::new(10_000, usize::MAX))
}

pub fn entry(label: &str) -> Entry<Cmd, Sel, Key> {
    Entry::new(vec![label.to_owned()], 0, 0)
}

/// Makes a change, commits it and records it, or says why it was
/// refused.
pub fn try_edit(
    history: &mut Lib,
    label: &str,
    change: impl FnOnce(&mut Draft<Library>),
) -> Result<Recorded, Rejected<Broken>> {
    let mut draft = history.current().begin(label, WINDOW);
    change(&mut draft);
    let commit = draft.commit()?;
    Ok(history
        .record(commit, entry(label))
        .expect("made from the current document"))
}

/// The same, for a change that must be accepted.
pub fn edit(history: &mut Lib, label: &str, change: impl FnOnce(&mut Draft<Library>)) -> Recorded {
    try_edit(history, label, change).expect("a change that keeps the rules")
}

pub fn add_shelf(history: &mut Lib, name: &str) -> ShelfId {
    let mut made = None;
    edit(history, "Add shelf", |draft| {
        let id: ShelfId = draft.mint().expect("an id");
        draft.shelves.insert(id, shelf(name)).expect("a new id");
        made = Some(id);
    });
    made.expect("the change ran")
}

pub fn add_book(history: &mut Lib, on: ShelfId, title: &str) -> BookId {
    let mut made = None;
    edit(history, "Add book", |draft| {
        let id: BookId = draft.mint().expect("an id");
        draft.books.insert(id, book(on, title)).expect("a new id");
        made = Some(id);
    });
    made.expect("the change ran")
}

pub fn retitle(history: &mut Lib, id: BookId, title: &str) -> Recorded {
    edit(history, "Retitle", |draft| {
        draft
            .books
            .update(id, |book| book.title = title.to_owned())
            .expect("the book");
    })
}

pub fn titles(document: &Document<Library>) -> Vec<String> {
    document
        .books
        .values()
        .map(|book| book.title.clone())
        .collect()
}

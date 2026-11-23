// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Undo and redo: steps, joined steps, the budget, view state, and two
//! writers in one history.

mod common;

use std::time::{Duration, Instant};

use common::{
    AGENT, BookId, Key, Lib, Library, Sel, ShelfId, WINDOW, add_book, add_shelf, book, edit, entry,
    library, retitle, shelf, titles, try_edit,
};
use concat_doc::{
    AnyTable, Budget, ChangePart, ChangeSet, Changed, Document, Draft, Entry, History, NotCurrent,
    Origin, PATH_BYTES, Recorded, Revision, Schema, Table, TableChanges, TypedId, Weigh, Writer,
};

/// Half a second: how long after a step a small repeat still joins it.
const SOON: Duration = Duration::from_millis(500);

/// A change by someone, at a time, perhaps joining, with a selection.
struct Change<'a> {
    label: &'a str,
    origin: Origin,
    at: Instant,
    key: Option<Key>,
    selection: (Sel, Sel),
}

impl<'a> Change<'a> {
    fn new(label: &'a str, at: Instant) -> Self {
        Change {
            label,
            origin: WINDOW,
            at,
            key: None,
            selection: (0, 0),
        }
    }

    fn joining(mut self, key: Key) -> Self {
        self.key = Some(key);
        self
    }

    fn by(mut self, origin: Origin) -> Self {
        self.origin = origin;
        self
    }

    fn selecting(mut self, before: Sel, after: Sel) -> Self {
        self.selection = (before, after);
        self
    }

    fn make(self, history: &mut Lib, change: impl FnOnce(&mut Draft<Library>)) -> Recorded {
        let mut draft = history.current().begin(self.label, self.origin);
        change(&mut draft);
        let commit = draft.commit().expect("a change that keeps the rules");
        let mut entry = Entry::new(
            vec![self.label.to_owned()],
            self.selection.0,
            self.selection.1,
        )
        .at(self.at);
        if let Some(key) = self.key {
            entry = entry.coalescing(key, SOON);
        }
        history
            .record(commit, entry)
            .expect("made from the current document")
    }
}

fn set_title(id: BookId, title: &str) -> impl FnOnce(&mut Draft<Library>) + '_ {
    move |draft| {
        draft
            .books
            .update(id, |book| book.title = title.to_owned())
            .expect("the book")
    }
}

fn ms(start: Instant, millis: u64) -> Instant {
    start + Duration::from_millis(millis)
}

/// A shelf with one book on it, two steps in.
fn one_book() -> (Lib, ShelfId, BookId) {
    let mut history = library();
    let shelf_id = add_shelf(&mut history, "Fiction");
    let book_id = add_book(&mut history, shelf_id, "Emma");
    (history, shelf_id, book_id)
}

#[test]
fn undo_makes_the_document_before_current_and_redo_the_one_after() {
    let (mut history, _, emma) = one_book();
    let before = history.current().clone();
    let start = Instant::now();
    Change::new("Retitle", start)
        .selecting(3, 4)
        .make(&mut history, set_title(emma, "Persuasion"));
    let after = history.current().clone();
    assert!((history.can_undo(), history.can_redo()) == (true, false));
    assert_eq!(history.next_undo().unwrap().label(), "Retitle");

    let undone = history.undo().unwrap();
    assert!(undone.document.same_content(&before));
    assert!(undone.document.books.ptr_eq(&before.books));
    assert!(Document::ptr_eq(&undone.document, history.current()));
    assert_eq!(
        (undone.selection, undone.label.as_str(), undone.origin),
        (3, "Retitle", WINDOW)
    );
    assert_eq!(undone.changes.books.replaced.len(), 1);
    assert_eq!(undone.changes.books.replaced[0].new.title, "Emma");
    assert!(history.can_redo());
    assert_eq!(history.next_redo().unwrap().label(), "Retitle");

    let redone = history.redo().unwrap();
    assert!(redone.document.same_content(&after));
    assert!(redone.document.books.ptr_eq(&after.books));
    assert_eq!((redone.selection, redone.label.as_str()), (4, "Retitle"));
    assert_eq!(redone.changes.books.replaced[0].new.title, "Persuasion");
    assert!(!history.can_redo());
    assert!(history.redo().is_none());
}

#[test]
fn every_step_can_be_taken_back_to_the_start_and_made_again_to_the_end() {
    let (mut history, shelf_id, emma) = one_book();
    let other = add_book(&mut history, shelf_id, "Ulysses");
    retitle(&mut history, emma, "Persuasion");
    edit(&mut history, "Remove", |draft| {
        drop(draft.books.remove(other))
    });
    edit(&mut history, "Rename", |draft| {
        draft.name = "Home".to_owned()
    });

    let mut on_the_way = vec![history.current().clone()];
    while let Some(undone) = history.undo() {
        on_the_way.push(undone.document);
    }
    assert_eq!(on_the_way.len(), 7);
    assert!(history.current().same_content(&Document::new()));
    assert!(history.undo().is_none());

    while let Some(redone) = history.redo() {
        let was = on_the_way.pop().unwrap();
        assert!(!redone.document.same_content(&was));
        assert!(redone.document.same_content(on_the_way.last().unwrap()));
    }
    assert_eq!(on_the_way.len(), 1);
    assert_eq!(titles(history.current()), ["Persuasion"]);
    assert_eq!(history.current().name, "Home");
}

#[test]
fn a_new_change_after_an_undo_drops_what_could_have_been_redone() {
    let (mut history, shelf_id, emma) = one_book();
    retitle(&mut history, emma, "Persuasion");
    retitle(&mut history, emma, "Sanditon");
    history.undo().unwrap();
    history.undo().unwrap();
    assert_eq!((history.undo_len(), history.redo_len()), (2, 2));

    add_book(&mut history, shelf_id, "Ulysses");
    assert_eq!((history.undo_len(), history.redo_len()), (3, 0));
    assert!(history.redo().is_none());
    assert_eq!(titles(history.current()), ["Emma", "Ulysses"]);
}

#[test]
fn undoing_the_step_that_made_an_id_does_not_let_it_be_made_again() {
    let (mut history, shelf_id, emma) = one_book();
    assert_eq!(emma.to_string(), "b2");
    let count = history.current().mint().issued(Writer::LOCAL);

    let undone = history.undo().unwrap();
    assert!(!undone.document.books.contains(emma));
    // The tables went back; the count did not.
    assert_eq!(undone.document.mint().issued(Writer::LOCAL), count);

    let next = add_book(&mut history, shelf_id, "Ulysses");
    assert_eq!(next.to_string(), "b3");
    assert_ne!(next, emma);

    // Redo brings back the same thing under the same id, and what is
    // made after it is newer still.
    let mut history = one_book().0;
    history.undo().unwrap();
    let redone = history.redo().unwrap();
    assert_eq!(redone.document.books.get(emma).unwrap().title, "Emma");
    let next = add_book(&mut history, shelf_id, "Ulysses");
    assert_eq!(next.to_string(), "b3");
}

#[test]
fn a_step_back_or_forward_is_a_new_revision() {
    let (mut history, _, _) = one_book();
    assert_eq!(history.current().revision(), Revision(2));
    assert_eq!(history.undo().unwrap().document.revision(), Revision(3));
    assert_eq!(history.undo().unwrap().document.revision(), Revision(4));
    assert_eq!(history.redo().unwrap().document.revision(), Revision(5));
    // A plan made before the undo is against an older revision than now.
    assert!(history.current().revision() > Revision(2));
}

#[test]
fn small_repeats_with_one_key_soon_after_each_other_are_one_step() {
    let (mut history, _, emma) = one_book();
    let start = Instant::now();
    let type_title = |history: &mut Lib, title: &str, at: u64, sel: Sel| {
        Change::new("Type", ms(start, at))
            .joining(7)
            .selecting(sel, sel + 1)
            .make(history, set_title(emma, title))
    };
    assert_eq!(type_title(&mut history, "P", 0, 10), Recorded::Step);
    assert_eq!(type_title(&mut history, "Pe", 300, 20), Recorded::Joined);
    // Each within the time of the one before, not of the first.
    assert_eq!(type_title(&mut history, "Per", 700, 30), Recorded::Joined);
    assert_eq!(type_title(&mut history, "Pers", 1200, 40), Recorded::Joined);
    assert_eq!(history.undo_len(), 3);

    let step = history.next_undo().unwrap();
    assert_eq!(step.commands(), ["Type", "Type", "Type", "Type"]);
    assert_eq!(
        (*step.selection_before(), *step.selection_after()),
        (10, 41)
    );
    assert_eq!(step.when(), ms(start, 1200));
    assert_eq!(step.before().books.get(emma).unwrap().title, "Emma");
    assert_eq!(step.after().books.get(emma).unwrap().title, "Pers");
    // It keeps alive one old title, not four.
    assert_eq!(step.cost(), 64 + "Emma".len() + PATH_BYTES);
    assert_eq!(history.bytes(), 3 * PATH_BYTES + step.cost() - PATH_BYTES);

    // It undoes all the way in one go, and comes back in one.
    let undone = history.undo().unwrap();
    assert_eq!(titles(&undone.document), ["Emma"]);
    assert_eq!(undone.selection, 10);
    let redone = history.redo().unwrap();
    assert_eq!(titles(&redone.document), ["Pers"]);
    assert_eq!(redone.selection, 41);
}

#[test]
fn a_repeat_does_not_join_when_it_is_late_or_another_kind_or_another_writers() {
    let (mut history, shelf_id, emma) = one_book();
    let other = add_book(&mut history, shelf_id, "Ulysses");
    let start = Instant::now();
    let at = |millis| Change::new("Type", ms(start, millis));

    assert_eq!(
        at(0).joining(7).make(&mut history, set_title(emma, "a")),
        Recorded::Step
    );
    // Exactly at the limit still joins; a moment later does not.
    assert_eq!(
        at(500).joining(7).make(&mut history, set_title(emma, "b")),
        Recorded::Joined
    );
    assert_eq!(
        at(1001).joining(7).make(&mut history, set_title(emma, "c")),
        Recorded::Step
    );
    // Another key.
    assert_eq!(
        at(1002)
            .joining(8)
            .make(&mut history, set_title(other, "d")),
        Recorded::Step
    );
    // The same key from someone else.
    let theirs = at(1003).joining(8).by(AGENT);
    assert_eq!(
        theirs.make(&mut history, set_title(other, "e")),
        Recorded::Step
    );
    // No key, after a step with one; and a key, after a step with none.
    assert_eq!(
        at(1004).make(&mut history, set_title(other, "f")),
        Recorded::Step
    );
    assert_eq!(
        at(1005)
            .joining(8)
            .make(&mut history, set_title(other, "g")),
        Recorded::Step
    );
    // A clock that went backwards is no time at all, and joins.
    assert_eq!(
        at(900).joining(8).make(&mut history, set_title(other, "h")),
        Recorded::Joined
    );
    assert_eq!(history.undo_len(), 3 + 6);
}

#[test]
fn nothing_joins_a_step_across_an_undo_or_a_redo() {
    let (mut history, _, emma) = one_book();
    let start = Instant::now();
    let at = |millis| Change::new("Type", ms(start, millis)).joining(7);
    assert_eq!(
        at(0).make(&mut history, set_title(emma, "a")),
        Recorded::Step
    );
    assert_eq!(
        at(10).make(&mut history, set_title(emma, "b")),
        Recorded::Joined
    );

    history.undo().unwrap();
    history.redo().unwrap();
    assert_eq!(
        at(20).make(&mut history, set_title(emma, "c")),
        Recorded::Step
    );

    history.undo().unwrap();
    // The step now on top has the same key and is recent: still no.
    assert_eq!(
        at(30).make(&mut history, set_title(emma, "d")),
        Recorded::Step
    );
    assert_eq!(
        at(40).make(&mut history, set_title(emma, "e")),
        Recorded::Joined
    );
    assert_eq!(history.undo_len(), 4);
}

#[test]
fn repeats_that_end_where_they_began_leave_no_step() {
    let (mut history, _, emma) = one_book();
    let start = Instant::now();
    let at = |millis| Change::new("Nudge", ms(start, millis)).joining(7);
    assert_eq!(
        at(0).make(&mut history, set_title(emma, "Emma!")),
        Recorded::Step
    );
    assert_eq!(
        at(10).make(&mut history, set_title(emma, "Emma!!")),
        Recorded::Joined
    );
    assert_eq!(
        at(20).make(&mut history, set_title(emma, "Emma")),
        Recorded::Cancelled
    );
    assert_eq!((history.undo_len(), history.bytes()), (2, 2 * PATH_BYTES));
    assert_eq!(titles(history.current()), ["Emma"]);
    assert_eq!(history.next_undo().unwrap().label(), "Add book");
    // The next one starts afresh.
    assert_eq!(
        at(30).make(&mut history, set_title(emma, "Emma!")),
        Recorded::Step
    );
    assert_eq!(history.undo_len(), 3);
}

#[test]
fn a_change_of_nothing_is_no_step_and_does_not_end_redo() {
    let (mut history, _, emma) = one_book();
    history.undo().unwrap();
    let before = history.current().clone();

    // Nothing at all.
    assert_eq!(edit(&mut history, "Nothing", |_| ()), Recorded::Nothing);
    assert!(Document::ptr_eq(history.current(), &before));

    // An id made and not used: the count moves, and that is all.
    let made = edit(&mut history, "Reserve", |draft| {
        let _: BookId = draft.mint().unwrap();
    });
    assert_eq!(made, Recorded::Nothing);
    assert_eq!(
        history.current().revision(),
        Revision(before.revision().0 + 1)
    );
    assert_eq!((history.undo_len(), history.redo_len()), (1, 1));

    // Redo still brings the book back, under its own id, and the id
    // that was reserved is not given to anything.
    let redone = history.redo().unwrap();
    assert_eq!(redone.document.books.get(emma).unwrap().title, "Emma");
    assert_eq!(redone.document.mint().issued(Writer::LOCAL), 3);
}

#[test]
fn a_commit_made_from_another_document_is_refused() {
    let (mut history, shelf_id, emma) = one_book();
    let stale = history.current().clone();
    retitle(&mut history, emma, "Persuasion");
    let now = history.current().clone();

    let mut draft = stale.begin("Late", AGENT);
    let late: BookId = draft.mint().unwrap();
    draft.books.insert(late, book(shelf_id, "Late")).unwrap();
    let refused = history
        .record(draft.commit().unwrap(), entry("Late"))
        .unwrap_err();
    assert_eq!(
        refused,
        NotCurrent {
            current: Revision(3),
            commit_from: Revision(2)
        }
    );
    assert!(refused.to_string().contains("revision 2"));
    assert!(Document::ptr_eq(history.current(), &now));
    assert_eq!(history.undo_len(), 3);
}

#[test]
fn two_writers_share_one_history_and_undo_takes_back_the_latest() {
    let (mut history, shelf_id, emma) = one_book();
    let start = Instant::now();
    Change::new("Retitle", start).make(&mut history, set_title(emma, "Persuasion"));

    // An assistant's turn of several commands is one draft and one step.
    let mut turn = history.current().begin("Tidy shelf", AGENT);
    let mut commands = Vec::new();
    for title in ["Ulysses", "Dubliners", "Exiles"] {
        let id: BookId = turn.mint().unwrap();
        turn.books.insert(id, book(shelf_id, title)).unwrap();
        commands.push(format!("add {title}"));
    }
    turn.books
        .update(emma, |book| book.title.push('!'))
        .unwrap();
    commands.push("mark".to_owned());
    let recorded = history
        .record(turn.commit().unwrap(), Entry::new(commands, 0, 0))
        .unwrap();
    assert_eq!(recorded, Recorded::Step);
    Change::new("Rename", start).make(&mut history, |draft| draft.name = "Home".to_owned());

    let origins: Vec<Origin> = history.undo_steps().map(|step| step.origin()).collect();
    assert_eq!(origins, [WINDOW, AGENT, WINDOW, WINDOW, WINDOW]);
    assert_eq!(history.undo_steps().nth(1).unwrap().commands().len(), 4);

    // The person's undo takes back their own rename, then the whole of
    // the assistant's turn, then their own retitle.
    assert_eq!(history.undo().unwrap().origin, WINDOW);
    let undone = history.undo().unwrap();
    assert_eq!(
        (undone.origin, undone.label.as_str()),
        (AGENT, "Tidy shelf")
    );
    assert_eq!(titles(&undone.document), ["Persuasion"]);
    assert_eq!(undone.changes.books.removed.len(), 3);
    assert_eq!(history.undo().unwrap().label, "Retitle");
    let labels: Vec<&str> = history.redo_steps().map(|step| step.label()).collect();
    assert_eq!(labels, ["Retitle", "Tidy shelf", "Rename"]);
}

#[test]
fn over_its_count_the_history_lets_go_of_the_oldest_step() {
    let (mut history, shelf_id, _) = one_book();
    history.set_budget(Budget::new(3, usize::MAX));
    assert_eq!(history.budget().max_steps(), 3);
    for title in ["a", "b", "c", "d"] {
        add_book(&mut history, shelf_id, title);
    }
    assert_eq!(history.undo_len(), 3);
    let labels: Vec<String> = history
        .undo_steps()
        .map(|step| step.after().books.values().last().unwrap().title.clone())
        .collect();
    assert_eq!(labels, ["d", "c", "b"]);
    while history.undo().is_some() {}
    // As far back as is kept, and no further: the first three books
    // cannot be taken away any more.
    assert_eq!(titles(history.current()), ["Emma", "a"]);
    assert_eq!(history.redo_len(), 3);
}

#[test]
fn over_its_bytes_the_history_lets_go_of_the_oldest_but_never_the_last() {
    let (mut history, _, emma) = one_book();
    let long = "x".repeat(10_000);
    // Each retitle lets go of the title before it.
    retitle(&mut history, emma, &long);
    retitle(&mut history, emma, "short");
    retitle(&mut history, emma, &long);
    retitle(&mut history, emma, "short");
    let each_long = 64 + long.len() + PATH_BYTES;
    let each_short = 64 + "short".len() + PATH_BYTES;
    let first = 64 + "Emma".len() + PATH_BYTES;
    assert_eq!(
        history.bytes(),
        2 * PATH_BYTES + first + 2 * each_long + each_short
    );
    let costs: Vec<usize> = history.undo_steps().map(|step| step.cost()).collect();
    assert_eq!(
        costs,
        [
            each_long, each_short, each_long, first, PATH_BYTES, PATH_BYTES
        ]
    );

    // Room for one long title and one short: the two newest steps.
    history.set_budget(Budget::new(100, each_long + each_short));
    assert_eq!(history.undo_len(), 2);
    assert_eq!(history.bytes(), each_long + each_short);

    // Room for none: the newest step is kept all the same.
    history.set_budget(Budget::new(100, 10));
    assert_eq!(history.undo_len(), 1);
    assert_eq!(history.bytes(), each_long);
    assert_eq!(
        titles(&history.undo().unwrap().document),
        std::slice::from_ref(&long)
    );

    // And a step recorded over the budget stays until the next one.
    retitle(&mut history, emma, "next");
    assert_eq!((history.undo_len(), history.bytes()), (1, each_long));
    retitle(&mut history, emma, "last");
    assert_eq!(
        (history.undo_len(), history.bytes()),
        (1, 64 + "next".len() + PATH_BYTES)
    );
}

#[test]
fn a_step_that_can_be_redone_goes_only_after_every_older_one() {
    let (mut history, shelf_id, _) = one_book();
    for title in ["a", "b", "c", "d"] {
        add_book(&mut history, shelf_id, title);
    }
    history.undo().unwrap();
    history.undo().unwrap();
    history.undo().unwrap();
    assert_eq!((history.undo_len(), history.redo_len()), (3, 3));

    // Stepping back and forward never drops anything, even over budget.
    history.set_budget(Budget::new(4, usize::MAX));
    assert_eq!((history.undo_len(), history.redo_len()), (1, 3));
    history.set_budget(Budget::new(3, usize::MAX));
    assert_eq!((history.undo_len(), history.redo_len()), (0, 3));
    // Nothing older is left: now the farthest redo goes.
    history.set_budget(Budget::new(2, usize::MAX));
    assert_eq!((history.undo_len(), history.redo_len()), (0, 2));
    assert_eq!(
        titles(&history.redo().unwrap().document),
        ["Emma", "a", "b"]
    );
    assert_eq!(
        titles(&history.redo().unwrap().document),
        ["Emma", "a", "b", "c"]
    );
    assert!(history.redo().is_none());
    history.set_budget(Budget::new(0, 0));
    assert_eq!(history.budget().max_steps(), 1);
    assert_eq!((history.undo_len(), history.redo_len()), (1, 0));
}

#[test]
fn clearing_the_history_keeps_the_document_and_forgets_the_steps() {
    let (mut history, _, emma) = one_book();
    history.undo().unwrap();
    history.redo().unwrap();
    let now = history.current().clone();
    history.clear();
    assert!(Document::ptr_eq(history.current(), &now));
    assert_eq!(
        (history.undo_len(), history.redo_len(), history.bytes()),
        (0, 0, 0)
    );
    assert!(history.undo().is_none() && history.redo().is_none());
    assert!(history.next_undo().is_none() && history.next_redo().is_none());
    assert_eq!(retitle(&mut history, emma, "Persuasion"), Recorded::Step);
}

#[test]
fn a_change_of_view_is_saved_and_is_not_a_step() {
    let (mut history, fiction, emma) = one_book();
    let travel = add_shelf(&mut history, "Travel");
    retitle(&mut history, emma, "Persuasion");
    history.undo().unwrap();
    let before = history.current().revision();
    assert_eq!((history.undo_len(), history.redo_len()), (3, 1));

    let opened = edit(&mut history, "Open", |draft| draft.open = Some(travel));
    assert_eq!(opened, Recorded::View);
    let scrolled = edit(&mut history, "Scroll", |draft| {
        draft
            .shelves
            .update(fiction, |shelf| shelf.scroll = 40)
            .unwrap();
    });
    assert_eq!(scrolled, Recorded::View);

    // In the document, at new revisions; and nothing to undo or redo
    // was gained or lost.
    assert_eq!(history.current().open, Some(travel));
    assert_eq!(history.current().shelves.get(fiction).unwrap().scroll, 40);
    assert_eq!(history.current().revision(), Revision(before.0 + 2));
    assert_eq!((history.undo_len(), history.redo_len()), (3, 1));
    assert_eq!(history.next_undo().unwrap().label(), "Add shelf");

    // Redo was not broken by it, and does not undo it.
    let redone = history.redo().unwrap();
    assert_eq!(titles(&redone.document), ["Persuasion"]);
    assert_eq!(redone.document.open, Some(travel));
    assert_eq!(redone.document.shelves.get(fiction).unwrap().scroll, 40);
}

#[test]
fn undoing_an_earlier_step_keeps_the_view_as_it_is_now() {
    let (mut history, fiction, emma) = one_book();
    let travel = add_shelf(&mut history, "Travel");
    retitle(&mut history, emma, "Persuasion");
    edit(&mut history, "Open", |draft| draft.open = Some(travel));
    edit(&mut history, "Scroll", |draft| {
        draft
            .shelves
            .update(fiction, |shelf| shelf.scroll = 40)
            .unwrap();
        draft
            .shelves
            .update(travel, |shelf| shelf.scroll = 7)
            .unwrap();
    });

    // The retitle goes; the open shelf and both scrolls stay.
    let undone = history.undo().unwrap();
    assert_eq!(titles(&undone.document), ["Emma"]);
    assert_eq!(undone.document.open, Some(travel));
    assert_eq!(undone.document.shelves.get(fiction).unwrap().scroll, 40);
    assert_eq!(undone.document.shelves.get(travel).unwrap().scroll, 7);
    // Only the title is told as changed: the view did not move.
    assert_eq!(undone.changes.books.replaced.len(), 1);
    assert!(undone.changes.shelves.is_empty() && undone.changes.open.is_none());
}

#[test]
fn undoing_the_step_that_made_a_thing_loses_the_view_that_belonged_to_it() {
    let (mut history, fiction, _) = one_book();
    edit(&mut history, "Open", |draft| draft.open = Some(fiction));
    let travel = add_shelf(&mut history, "Travel");
    edit(&mut history, "Look at travel", |draft| {
        draft.open = Some(travel);
        draft
            .shelves
            .update(travel, |shelf| shelf.scroll = 7)
            .unwrap();
        draft
            .shelves
            .update(fiction, |shelf| shelf.scroll = 40)
            .unwrap();
    });
    assert_eq!(history.undo_len(), 3);

    // The shelf that is open goes with the step that made it: the view
    // falls back to what was open before, and keeps the rest.
    let undone = history.undo().unwrap();
    assert_eq!(undone.label, "Add shelf");
    assert!(!undone.document.shelves.contains(travel));
    assert_eq!(undone.document.open, Some(fiction));
    assert_eq!(undone.document.shelves.get(fiction).unwrap().scroll, 40);

    // Made again, the shelf is back; what is open is what is open now,
    // and its own scroll went with it and is as it was first made.
    let redone = history.redo().unwrap();
    assert!(redone.document.shelves.contains(travel));
    assert_eq!(redone.document.open, Some(fiction));
    assert_eq!(redone.document.shelves.get(travel).unwrap().scroll, 0);
    assert_eq!(redone.document.shelves.get(fiction).unwrap().scroll, 40);
}

#[test]
fn a_step_that_changed_content_and_view_takes_back_only_the_content() {
    let (mut history, fiction, emma) = one_book();
    let travel = add_shelf(&mut history, "Travel");
    let moved = edit(&mut history, "Move and show", |draft| {
        draft
            .books
            .update(emma, |book| book.shelf = travel)
            .unwrap();
        draft.open = Some(travel);
    });
    assert_eq!(moved, Recorded::Step);
    let undone = history.undo().unwrap();
    assert_eq!(undone.document.books.get(emma).unwrap().shelf, fiction);
    assert_eq!(undone.document.open, Some(travel));
}

#[test]
fn joined_steps_that_only_moved_the_view_in_the_end_leave_no_step() {
    let (mut history, fiction, emma) = one_book();
    let start = Instant::now();
    let at = |millis| Change::new("Nudge", ms(start, millis)).joining(7);
    assert_eq!(
        at(0).make(&mut history, set_title(emma, "Emma!")),
        Recorded::Step
    );
    let back = at(10).make(&mut history, |draft| {
        draft
            .books
            .update(emma, |book| book.title = "Emma".to_owned())
            .unwrap();
        draft.open = Some(fiction);
    });
    assert_eq!(back, Recorded::Cancelled);
    assert_eq!(history.undo_len(), 2);
    assert_eq!(history.current().open, Some(fiction));
}

#[test]
fn a_reader_on_another_thread_keeps_the_document_it_was_given() {
    fn shared<T: Send + Sync>() {}
    shared::<Document<Library>>();
    shared::<Draft<Library>>();
    shared::<Lib>();

    let (mut history, _, emma) = one_book();
    let held = history.current().clone();
    let (ready, go) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        go.recv().expect("the writer's word");
        titles(&held)
    });
    retitle(&mut history, emma, "Persuasion");
    history.undo().unwrap();
    retitle(&mut history, emma, "Sanditon");
    ready.send(()).expect("a reader");
    assert_eq!(reader.join().expect("the reader"), ["Emma"]);
    assert_eq!(titles(history.current()), ["Sanditon"]);
}

#[test]
fn a_step_keeps_what_it_was_told_and_the_defaults_are_as_documented() {
    let (mut history, _, emma) = one_book();
    let earlier = Instant::now();
    let mut draft = history
        .current()
        .begin("Late", AGENT)
        .planned_at(Revision(1));
    draft
        .books
        .update(emma, |book| book.title.push('?'))
        .unwrap();
    // Made with no time given: now.
    let entry: Entry<String, Sel, Key> = Entry::new(vec!["late".to_owned()], 5, 6);
    assert!(entry.coalesce.is_none() && entry.when >= earlier);
    history.record(draft.commit().unwrap(), entry).unwrap();
    let step = history.next_undo().unwrap();
    assert_eq!(
        (step.planned_at(), step.origin(), step.label()),
        (Revision(1), AGENT, "Late")
    );
    assert_eq!(step.before().revision(), Revision(2));
    assert!(step.when() >= earlier);

    let budget = Budget::default();
    assert_eq!((budget.max_steps(), budget.max_bytes()), (1000, 256 << 20));
    assert_eq!(Budget::new(0, 7), Budget::new(1, 7));
}

concat_doc::typed_id! {
    /// A pin on a board.
    struct PinId = "p";
}

#[derive(Clone, PartialEq, Debug)]
struct Pin(u32);

impl Weigh for Pin {
    fn weight(&self) -> usize {
        4
    }
}

/// A schema whose idea of carrying the view is wrong: it shows whatever
/// was showing, whether or not the pin is there.
#[derive(Clone, Default)]
struct Board {
    pins: Table<PinId, Pin>,
    showing: Option<PinId>,
}

struct BoardChanges {
    pins: TableChanges<PinId, Pin>,
    showing: Option<Changed<Option<PinId>>>,
}

impl ChangeSet for BoardChanges {
    fn parts(&self, each: &mut dyn FnMut(&dyn ChangePart)) {
        each(&self.pins);
        each(&self.showing);
    }
}

impl Schema for Board {
    type Changes = BoardChanges;
    type Violation = PinId;

    fn diff(before: &Self, after: &Self) -> BoardChanges {
        BoardChanges {
            pins: TableChanges::between(&before.pins, &after.pins),
            showing: Changed::between(&before.showing, &after.showing),
        }
    }

    fn check(_: &Self, after: &Self, _: &BoardChanges) -> Vec<PinId> {
        after
            .showing
            .filter(|pin| !after.pins.contains(*pin))
            .into_iter()
            .collect()
    }

    fn tables(&self, each: &mut dyn FnMut(&dyn AnyTable)) {
        each(&self.pins);
    }

    fn carry_view(present: &Self, restored: &Self) -> Self {
        Board {
            pins: restored.pins.clone(),
            showing: present.showing,
        }
    }
}

#[test]
fn a_view_that_the_restored_document_cannot_hold_gives_way_to_its_own() {
    let mut history: History<Board, (), ()> = History::new(Document::new(), Budget::default());
    let pin = |history: &mut History<Board, (), ()>, show: bool| {
        let mut draft = history.current().begin("Pin", WINDOW);
        let id: PinId = draft.mint().unwrap();
        draft.pins.insert(id, Pin(1)).unwrap();
        if show {
            draft.showing = Some(id);
        }
        let recorded = history.record(draft.commit().unwrap(), Entry::new(vec![()], (), ()));
        assert_eq!(recorded, Ok(Recorded::Step));
        id
    };
    let first = pin(&mut history, true);
    let second = pin(&mut history, true);
    assert_eq!(history.current().showing, Some(second));

    // Carried as the schema says, the restored document would show a pin
    // it does not have. That is not a document: its own view stands.
    let undone = history.undo().unwrap();
    assert!(!undone.document.pins.contains(second));
    assert_eq!(undone.document.showing, Some(first));
    let undone = history.undo().unwrap();
    assert_eq!(undone.document.showing, None);
    assert!(undone.document.pins.is_empty());
    // Forward again, the view that can be held is carried: none showing.
    let redone = history.redo().unwrap();
    assert_eq!(redone.document.showing, None);
    assert_eq!(redone.document.pins.len(), 1);
    assert_eq!(first.counter(), 1);
}

#[test]
fn a_refused_change_leaves_the_history_as_it_was() {
    let (mut history, _, emma) = one_book();
    history.undo().unwrap();
    let now = history.current().clone();
    let refused = try_edit(&mut history, "Add", |draft| {
        let id: BookId = draft.mint().unwrap();
        draft
            .books
            .insert(id, book("s99".parse().unwrap(), "Nowhere"))
            .unwrap();
    });
    assert!(refused.is_err());
    assert!(Document::ptr_eq(history.current(), &now));
    assert_eq!((history.undo_len(), history.redo_len()), (1, 1));
    // The id the refused change made was never used, and redo still
    // brings back the book under its own.
    assert_eq!(
        history
            .redo()
            .unwrap()
            .document
            .books
            .get(emma)
            .unwrap()
            .title,
        "Emma"
    );
    let _ = shelf("unused");
}

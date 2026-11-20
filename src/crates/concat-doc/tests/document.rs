// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A document, a draft, a commit, and what a commit says changed.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::{Book, BookId, Broken, Library, ShelfId, WINDOW, book, shelf};
use concat_doc::{
    AnyId, ChangeSet, Changed, Document, Draft, Mint, MintError, PATH_BYTES, Rejected, Replaced,
    Revision, StalePlan, Table, TableChanges, TableError, TypedId, Writer,
};
use proptest::prelude::*;

/// A library with two shelves and three books, as one commit.
fn stocked() -> (Document<Library>, [ShelfId; 2], [BookId; 3]) {
    let mut draft = Document::<Library>::new().begin("Stock", WINDOW);
    let fiction: ShelfId = draft.mint().unwrap();
    let travel: ShelfId = draft.mint().unwrap();
    draft.shelves.insert(fiction, shelf("Fiction")).unwrap();
    draft.shelves.insert(travel, shelf("Travel")).unwrap();
    let books: [BookId; 3] = [
        draft.mint().unwrap(),
        draft.mint().unwrap(),
        draft.mint().unwrap(),
    ];
    draft.books.insert(books[0], book(fiction, "Emma")).unwrap();
    draft
        .books
        .insert(books[1], book(fiction, "Ulysses"))
        .unwrap();
    draft
        .books
        .insert(books[2], book(travel, "Arabia"))
        .unwrap();
    draft.name = "Home".to_owned();
    (
        draft.commit().unwrap().into_document(),
        [fiction, travel],
        books,
    )
}

#[test]
fn a_new_document_is_empty_at_revision_zero() {
    let document = Document::<Library>::new();
    assert_eq!(document.revision(), Revision(0));
    assert_eq!(document.mint(), &Mint::new());
    assert!(document.shelves.is_empty());
    assert!(document.books.is_empty());
    assert!(Document::ptr_eq(&document, &document.clone()));
    assert!(document.same_content(&Document::default()));
    assert!(!Document::ptr_eq(&document, &Document::default()));
}

#[test]
fn a_commit_gives_a_new_document_and_leaves_the_old_one_as_it_was() {
    let (before, [fiction, _], [emma, ulysses, _]) = stocked();
    let held = Arc::clone(before.books.get(ulysses).unwrap());

    let mut draft = before.begin("Retitle", WINDOW);
    draft
        .books
        .update(emma, |book| book.title = "Persuasion".to_owned())
        .unwrap();
    let commit = draft.commit().unwrap();
    let after = commit.document().clone();

    assert_eq!(after.revision(), Revision(before.revision().0 + 1));
    assert_eq!(after.books.get(emma).unwrap().title, "Persuasion");
    assert_eq!(before.books.get(emma).unwrap().title, "Emma");
    assert!(Document::ptr_eq(commit.before(), &before));
    assert_eq!((commit.label(), commit.origin()), ("Retitle", WINDOW));
    assert_eq!(commit.planned_at(), before.revision());
    assert!(!commit.is_nothing());
    // What the change did not touch is the same memory in both.
    assert!(after.shelves.ptr_eq(&before.shelves));
    assert!(Arc::ptr_eq(after.books.get(ulysses).unwrap(), &held));
    assert!(!after.books.ptr_eq(&before.books));
    assert_eq!(after.shelves.get(fiction), before.shelves.get(fiction));
}

#[test]
fn a_commit_says_exactly_what_it_added_removed_and_replaced() {
    let (before, [fiction, travel], [emma, ulysses, arabia]) = stocked();
    let mut draft = before.begin("Tidy", WINDOW);
    let poetry: ShelfId = draft.mint().unwrap();
    draft.shelves.insert(poetry, shelf("Poetry")).unwrap();
    draft.books.remove(ulysses).unwrap();
    draft
        .books
        .replace(arabia, book(travel, "Arabia Deserta"))
        .unwrap();
    let odes: BookId = draft.mint().unwrap();
    draft.books.insert(odes, book(poetry, "Odes")).unwrap();
    draft.name = "Away".to_owned();
    draft.open = Some(fiction);

    // A draft can be asked what it has changed so far, and the commit
    // says the same.
    let so_far = draft.changes();
    let commit = draft.commit().unwrap();
    let changes = commit.changes();
    assert_eq!(&so_far, changes);

    assert_eq!(changes.shelves.added, [(poetry, Arc::new(shelf("Poetry")))]);
    assert!(changes.shelves.removed.is_empty() && changes.shelves.replaced.is_empty());
    assert_eq!(
        changes.books.added,
        [(odes, Arc::new(book(poetry, "Odes")))]
    );
    assert_eq!(
        changes.books.removed,
        [(ulysses, Arc::new(book(fiction, "Ulysses")))]
    );
    assert_eq!(
        changes.books.replaced,
        [Replaced {
            id: arabia,
            old: Arc::new(book(travel, "Arabia")),
            new: Arc::new(book(travel, "Arabia Deserta")),
        }]
    );
    assert_eq!(
        changes.name,
        Some(Changed {
            old: "Home".to_owned(),
            new: "Away".to_owned()
        })
    );
    assert_eq!(
        changes.open,
        Some(Changed {
            old: None,
            new: Some(fiction)
        })
    );
    assert_eq!(changes.books.len(), 3);
    assert_eq!(
        changes.books.ids().collect::<Vec<_>>(),
        [odes, ulysses, arabia]
    );
    assert_eq!(changes.added(), [poetry.any(), odes.any()]);
    assert!(!changes.is_empty());
    // The values are the documents' own, not copies of them.
    assert!(Arc::ptr_eq(
        &changes.books.removed[0].1,
        before.books.get(ulysses).unwrap()
    ));
    assert!(Arc::ptr_eq(
        &changes.books.replaced[0].new,
        commit.document().books.get(arabia).unwrap()
    ));
    assert_eq!(commit.document().changes_since(&before), *changes);
    assert_eq!(before.books.get(emma), commit.document().books.get(emma));
}

#[test]
fn a_step_costs_what_it_let_go_of_and_the_paths_it_rebuilt() {
    let (before, [fiction, _], [emma, ulysses, _]) = stocked();
    let mut draft = before.begin("Tidy", WINDOW);
    draft.books.remove(ulysses).unwrap();
    draft
        .books
        .update(emma, |book| book.title = "Persuasion".to_owned())
        .unwrap();
    let added: BookId = draft.mint().unwrap();
    draft
        .books
        .insert(
            added,
            book(fiction, "A very long title that is not let go of"),
        )
        .unwrap();
    let commit = draft.commit().unwrap();
    let changes = commit.changes();
    let gone = (64 + "Ulysses".len()) + (64 + "Emma".len());
    assert_eq!(changes.cost(), gone + 3 * PATH_BYTES);
    assert_eq!(changes.books.gone().count(), 2);
}

#[test]
fn a_commit_that_breaks_a_rule_names_what_is_wrong_and_changes_nothing() {
    let (before, [fiction, travel], [emma, ulysses, arabia]) = stocked();
    let nowhere: ShelfId = "s99".parse().unwrap();

    let mut draft = before.begin("Break things", WINDOW);
    draft.books.update(emma, |book| book.title.clear()).unwrap();
    draft
        .books
        .update(ulysses, |book| book.shelf = nowhere)
        .unwrap();
    draft.shelves.remove(travel).unwrap();
    draft.open = Some(travel);
    let Err(refused) = draft.commit() else {
        panic!("a change that breaks four rules was accepted");
    };
    let Rejected::Invalid(mut broken) = refused.clone() else {
        panic!("refused for the wrong reason: {refused}");
    };
    broken.sort();
    assert_eq!(
        broken,
        [
            Broken::NoSuchShelf {
                book: ulysses,
                shelf: nowhere
            },
            Broken::Untitled(emma),
            Broken::ShelfInUse {
                shelf: travel,
                book: arabia
            },
            Broken::OpenShelfMissing(travel),
        ]
    );
    assert!(refused.to_string().contains("breaks the rules"));

    // The document the draft was made from is what it was.
    assert_eq!(before.revision(), Revision(1));
    assert_eq!(before.books.get(emma).unwrap().title, "Emma");
    assert_eq!(before.books.get(ulysses).unwrap().shelf, fiction);
    assert!(before.shelves.contains(travel));
    assert_eq!(before.open, None);
}

#[test]
fn a_draft_may_break_the_rules_on_the_way_if_it_keeps_them_at_the_end() {
    let (before, [fiction, travel], [_, _, arabia]) = stocked();
    let mut draft = before.begin("Merge shelves", WINDOW);
    // For a moment a book is on a shelf that is not there.
    draft.shelves.remove(travel).unwrap();
    draft
        .books
        .update(arabia, |book| book.shelf = fiction)
        .unwrap();
    let after = draft.commit().unwrap().into_document();
    assert_eq!(after.shelves.len(), 1);
    assert_eq!(after.books.get(arabia).unwrap().shelf, fiction);
}

#[test]
fn a_dropped_draft_changes_nothing() {
    let (before, [fiction, _], [emma, ..]) = stocked();
    let untouched = before.clone();
    {
        let mut draft = before.begin("Never mind", WINDOW);
        let id: BookId = draft.mint().unwrap();
        draft.books.insert(id, book(fiction, "Unwritten")).unwrap();
        draft.books.remove(emma).unwrap();
        draft.name.clear();
        assert_eq!(draft.books.len(), 3);
        assert_eq!((draft.label(), draft.origin()), ("Never mind", WINDOW));
        assert!(Document::ptr_eq(draft.base(), &before));
    }
    assert!(Document::ptr_eq(&before, &untouched));
    assert_eq!(before.revision(), Revision(1));
    assert_eq!(before.books.len(), 3);
    assert!(before.books.contains(emma));
    assert_eq!(before.name, "Home");
    assert_eq!(before.mint().issued(Writer::LOCAL), 5);
}

#[test]
fn a_draft_that_changed_nothing_gives_back_the_document_it_began_on() {
    let (before, _, [emma, ..]) = stocked();
    let commit = before.begin("Nothing", WINDOW).commit().unwrap();
    assert!(commit.is_nothing());
    assert!(commit.changes().is_empty());
    assert!(Document::ptr_eq(commit.document(), &before));

    // Writing a value again as it was is also nothing.
    let mut draft = before.begin("Same again", WINDOW);
    draft
        .books
        .replace(emma, before.books.get(emma).unwrap().as_ref().clone())
        .unwrap();
    draft
        .books
        .update(emma, |book| book.title = "Emma".to_owned())
        .unwrap();
    draft.name = "Home".to_owned();
    let commit = draft.commit().unwrap();
    assert!(commit.changes().is_empty());
    assert!(Document::ptr_eq(commit.document(), &before));
    assert_eq!(commit.document().revision(), Revision(1));
}

#[test]
fn a_draft_that_only_made_ids_moves_the_count_and_keeps_the_tables() {
    let (before, ..) = stocked();
    let mut draft = before.begin("Reserve", WINDOW);
    let reserved: BookId = draft.mint().unwrap();
    let commit = draft.commit().unwrap();
    assert!(commit.changes().is_empty());
    assert!(!commit.is_nothing());
    let after = commit.into_document();
    assert_eq!(after.revision(), Revision(2));
    assert!(after.books.ptr_eq(&before.books));
    assert!(after.mint().covers(reserved.raw()));
    let next: BookId = after.begin("Next", WINDOW).mint().unwrap();
    assert_ne!(next, reserved);
}

#[test]
fn an_id_the_mint_never_made_is_refused() {
    let (before, [fiction, _], _) = stocked();
    let invented: BookId = "b40".parse().unwrap();
    let foreign: BookId = "b7.1".parse().unwrap();
    let mut draft = before.begin("Paste", WINDOW);
    draft
        .books
        .insert(invented, book(fiction, "From elsewhere"))
        .unwrap();
    draft
        .books
        .insert(foreign, book(fiction, "From another writer"))
        .unwrap();
    let Err(refused) = draft.commit() else {
        panic!("ids from nowhere were accepted");
    };
    assert_eq!(
        refused,
        Rejected::Unminted(vec![invented.any(), foreign.any()])
    );
    assert!(refused.to_string().contains("never made"));
    assert!(!before.books.contains(invented));
}

#[test]
fn ids_come_from_the_draft_and_are_never_given_twice_by_a_document() {
    let (before, [fiction, _], _) = stocked();
    assert_eq!(before.mint().issued(Writer::LOCAL), 5);

    // A draft that is dropped never used its ids, and the next one may.
    let mut dropped = before.begin("Dropped", WINDOW);
    let unused: BookId = dropped.mint().unwrap();
    assert_eq!(unused.to_string(), "b6");
    drop(dropped);
    let mut draft = before.begin("Kept", WINDOW);
    let used: BookId = draft.mint().unwrap();
    assert_eq!(used, unused);
    draft.books.insert(used, book(fiction, "Kept")).unwrap();
    let after = draft.commit().unwrap().into_document();

    // One count for every kind: the shelf made next is not "s6".
    let mut draft = after.begin("More", WINDOW);
    let next: ShelfId = draft.mint().unwrap();
    assert_eq!(next.to_string(), "s7");
    assert_ne!(next.raw(), used.raw());

    // Another writer has a count of its own.
    let theirs: BookId = draft.mint_as(Writer(3)).unwrap();
    assert_eq!(theirs.to_string(), "b3.1");
    draft.books.insert(theirs, book(fiction, "Theirs")).unwrap();
    let after = draft.commit().unwrap().into_document();
    assert_eq!(after.mint().issued(Writer(3)), 1);
    assert_eq!(after.mint().issued(Writer::LOCAL), 7);
    assert!(after.books.contains(theirs));
}

#[test]
fn a_draft_cannot_mint_past_the_last_id() {
    let full = Mint::resume([(Writer::LOCAL, concat_doc::MAX_COUNTER)]).unwrap();
    let document = Document::<Library>::load(Library::default(), full, Revision(3)).unwrap();
    let mut draft = document.begin("One too many", WINDOW);
    assert_eq!(
        draft.mint::<BookId>(),
        Err(MintError::Exhausted {
            writer: Writer::LOCAL
        })
    );
}

/// Moves a book along by so many letters: the whole movement, from where
/// the draft began.
fn nudge(draft: &mut Draft<Library>, id: BookId, by: usize) -> Result<(), TableError> {
    draft
        .books
        .update(id, |book| book.title.push_str(&"!".repeat(by)))
}

#[test]
fn a_gesture_recomputed_on_every_input_ends_where_one_command_would() {
    let (before, [fiction, _], [emma, ..]) = stocked();

    let mut gesture = before.begin("Drag", WINDOW);
    for total in [1, 2, 5, 9, 4, 7] {
        gesture
            .recompute(|draft| {
                // A change that makes something new makes it under the
                // same id every time.
                let copy: BookId = draft.mint().unwrap();
                assert_eq!(copy.to_string(), "b6");
                draft.books.insert(copy, book(fiction, "Copy")).unwrap();
                nudge(draft, emma, total)
            })
            .unwrap();
        assert_eq!(gesture.books.get(emma).unwrap().title.len(), 4 + total);
    }
    let dragged = gesture.commit().unwrap();

    let mut command = before.begin("Drag", WINDOW);
    let copy: BookId = command.mint().unwrap();
    command.books.insert(copy, book(fiction, "Copy")).unwrap();
    nudge(&mut command, emma, 7).unwrap();
    let commanded = command.commit().unwrap();

    assert_eq!(dragged.changes(), commanded.changes());
    assert!(dragged.document().same_content(commanded.document()));
    assert_eq!(dragged.document().mint(), commanded.document().mint());
    assert_eq!(dragged.document().revision(), Revision(2));
}

#[test]
fn a_recompute_that_fails_leaves_the_draft_as_it_began() {
    let (before, _, [emma, ..]) = stocked();
    let gone: BookId = "b77".parse().unwrap();
    let mut gesture = before.begin("Drag", WINDOW);
    gesture.recompute(|draft| nudge(draft, emma, 3)).unwrap();
    let failed = gesture.recompute(|draft| {
        nudge(draft, emma, 4)?;
        let _: ShelfId = draft.mint().unwrap();
        nudge(draft, gone, 4)
    });
    assert_eq!(failed, Err(TableError::Missing(gone.any())));
    assert!(gesture.changes().is_empty());
    assert!(gesture.commit().unwrap().is_nothing());

    // `restart` is the same thing asked for outright.
    let mut draft = before.begin("Start over", WINDOW);
    draft.books.remove(emma).unwrap();
    let _: BookId = draft.mint().unwrap();
    draft.restart();
    assert!(draft.commit().unwrap().is_nothing());
}

#[test]
fn a_draft_is_carried_over_to_a_newer_document_and_made_again_there() {
    let (before, [fiction, _], [emma, ulysses, _]) = stocked();
    let mut gesture = before.begin("Drag", WINDOW);
    gesture.recompute(|draft| nudge(draft, emma, 2)).unwrap();

    // Someone else commits while the drag is held.
    let mut other = before.begin("Add", common::AGENT);
    let theirs: BookId = other.mint().unwrap();
    other.books.insert(theirs, book(fiction, "Theirs")).unwrap();
    other
        .books
        .update(ulysses, |book| book.title = "Dubliners".to_owned())
        .unwrap();
    let newer = other.commit().unwrap().into_document();

    gesture
        .carry_over(&newer, |draft| nudge(draft, emma, 2))
        .unwrap();
    assert!(Document::ptr_eq(gesture.base(), &newer));
    assert_eq!(gesture.planned_revision(), newer.revision());
    let after = gesture.commit().unwrap().into_document();
    assert_eq!(after.revision(), Revision(3));
    assert_eq!(after.books.get(emma).unwrap().title, "Emma!!");
    assert_eq!(after.books.get(ulysses).unwrap().title, "Dubliners");
    assert!(after.books.contains(theirs));
    // The count goes on from theirs: nothing of theirs is made again.
    let next: BookId = after.begin("Next", WINDOW).mint().unwrap();
    assert_eq!(next.to_string(), "b7");
}

#[test]
fn a_draft_whose_subject_is_gone_cannot_be_carried_over() {
    let (before, _, [emma, ..]) = stocked();
    let mut gesture = before.begin("Drag", WINDOW);
    gesture.recompute(|draft| nudge(draft, emma, 2)).unwrap();

    let mut other = before.begin("Remove", common::AGENT);
    other.books.remove(emma).unwrap();
    let newer = other.commit().unwrap().into_document();

    let carried = gesture.carry_over(&newer, |draft| nudge(draft, emma, 2));
    assert_eq!(carried, Err(TableError::Missing(emma.any())));
    assert!(gesture.changes().is_empty());
    assert!(Document::ptr_eq(gesture.base(), &newer));
}

#[test]
fn a_draft_remembers_the_revision_it_was_planned_against() {
    let (before, _, [emma, ..]) = stocked();
    let mut draft = before.begin("Late", common::AGENT).planned_at(Revision(0));
    assert_eq!(draft.planned_revision(), Revision(0));
    nudge(&mut draft, emma, 1).unwrap();
    let commit = draft.commit().unwrap();
    assert_eq!(commit.planned_at(), Revision(0));
    assert_eq!(commit.before().revision(), Revision(1));
}

#[test]
fn a_plan_stands_while_everything_it_names_is_still_there() {
    let (planned, [fiction, travel], [emma, ulysses, arabia]) = stocked();
    let read = [emma.any(), fiction.any(), arabia.any()];

    // Nothing has happened: the plan stands.
    assert_eq!(planned.check_plan(planned.revision(), read), Ok(()));

    // Something else changed: it still stands.
    let mut other = planned.begin("Elsewhere", WINDOW);
    other.books.remove(ulysses).unwrap();
    nudge(&mut other, emma, 1).unwrap();
    let moved_on = other.commit().unwrap().into_document();
    assert_eq!(moved_on.check_plan(planned.revision(), read), Ok(()));
    assert_eq!(moved_on.check_plan(planned.revision(), []), Ok(()));

    // What it names is gone: refused, and the missing are named once
    // each, in order.
    let mut other = moved_on.begin("Clear out", WINDOW);
    other.books.remove(arabia).unwrap();
    other.books.remove(emma).unwrap();
    other.shelves.remove(travel).unwrap();
    let cleared = other.commit().unwrap().into_document();
    let names = [
        arabia.any(),
        fiction.any(),
        emma.any(),
        travel.any(),
        arabia.any(),
    ];
    let stale = cleared.check_plan(planned.revision(), names).unwrap_err();
    assert_eq!(
        stale,
        StalePlan::Missing {
            planned_at: Revision(1),
            now: Revision(3),
            missing: vec![emma.any(), arabia.any(), travel.any()],
        }
    );
    assert!(stale.to_string().contains("[b3, b5, s2]"), "{stale}");
}

#[test]
fn a_plan_is_refused_when_it_names_what_never_was_or_comes_from_another_document() {
    let (document, _, [emma, ..]) = stocked();
    // A book's number under a shelf's letter is not that book.
    let wrong_kind = ShelfId::from_raw(emma.raw()).any();
    let never: AnyId = "b999".parse::<BookId>().unwrap().into();
    assert_eq!(
        document.check_plan(document.revision(), [wrong_kind, never]),
        Err(StalePlan::Missing {
            planned_at: Revision(1),
            now: Revision(1),
            missing: vec![never, wrong_kind],
        })
    );
    let ahead = document.check_plan(Revision(9), [emma.any()]).unwrap_err();
    assert_eq!(
        ahead,
        StalePlan::NotThisDocument {
            planned_at: Revision(9),
            now: Revision(1)
        }
    );
    assert!(ahead.to_string().contains("has not reached"));
}

#[test]
fn a_document_knows_what_it_holds_by_any_id() {
    let (document, [fiction, _], [emma, ..]) = stocked();
    assert!(document.contains(fiction));
    assert!(document.contains(emma));
    assert!(document.contains(emma.any()));
    assert!(!document.contains("b99".parse::<BookId>().unwrap()));
    assert!(!document.contains(BookId::from_raw(fiction.raw())));
}

#[test]
fn a_file_is_loaded_through_the_same_checks_as_a_commit() {
    let (saved, [fiction, _], [emma, ..]) = stocked();
    let tables = saved.tables().clone();

    let loaded = Document::load(tables.clone(), saved.mint().clone(), Revision(41)).unwrap();
    assert_eq!(loaded.revision(), Revision(41));
    assert!(loaded.same_content(&saved));
    assert_eq!(loaded.mint(), saved.mint());
    let next: BookId = loaded.begin("Go on", WINDOW).mint().unwrap();
    assert_eq!(next.to_string(), "b6");

    // A count behind the ids in the file would give one of them out
    // again.
    let behind = Mint::resume([(Writer::LOCAL, 3)]).unwrap();
    let refused = Document::load(tables.clone(), behind, Revision(41)).unwrap_err();
    let Rejected::Unminted(ids) = refused else {
        panic!("refused for the wrong reason: {refused}");
    };
    assert_eq!(ids.len(), 2);

    // A file that breaks a rule is not repaired: it is refused, by name.
    let mut broken = tables;
    broken
        .books
        .update(emma, |book| book.title.clear())
        .unwrap();
    broken.shelves.remove(fiction).unwrap();
    let refused = Document::load(broken, saved.mint().clone(), Revision(41)).unwrap_err();
    let Rejected::Invalid(mut rules) = refused else {
        panic!("refused for the wrong reason: {refused}");
    };
    rules.sort();
    assert_eq!(
        rules[0],
        Broken::NoSuchShelf {
            book: emma,
            shelf: fiction
        }
    );
    assert!(rules.contains(&Broken::Untitled(emma)));
}

#[test]
fn a_table_is_strict_about_what_is_and_is_not_there() {
    let mut table: Table<BookId, Book> = Table::new();
    let shelf_id: ShelfId = "s1".parse().unwrap();
    let (one, two): (BookId, BookId) = ("b1".parse().unwrap(), "b2".parse().unwrap());
    assert_eq!(table.insert(one, book(shelf_id, "One")), Ok(()));
    assert_eq!(
        table.insert(one, book(shelf_id, "Again")),
        Err(TableError::Occupied(one.any()))
    );
    assert_eq!(
        table.replace(two, book(shelf_id, "Two")),
        Err(TableError::Missing(two.any()))
    );
    assert_eq!(
        table.update(two, |_| ()),
        Err(TableError::Missing(two.any()))
    );
    assert_eq!(table.remove(two), Err(TableError::Missing(two.any())));
    assert_eq!(table.get(one).unwrap().title, "One");
    assert_eq!(
        *table.replace(one, Arc::new(book(shelf_id, "Uno"))).unwrap(),
        book(shelf_id, "One")
    );
    assert_eq!(*table.remove(one).unwrap(), book(shelf_id, "Uno"));
    assert!(table.is_empty());
    let refused = TableError::Occupied(one.any());
    assert_eq!(
        (refused.id(), refused.to_string()),
        (one.any(), "b1 is already in the table".into())
    );
    assert_eq!(
        TableError::Missing(two.any()).to_string(),
        "b2 is not in the table"
    );
}

#[test]
fn a_table_is_read_in_id_order_and_a_copy_is_the_table_as_it_was() {
    let shelf_id: ShelfId = "s1".parse().unwrap();
    let id = |n: u64| -> BookId { format!("b{n}").parse().unwrap() };
    let table: Table<BookId, Book> = [5, 1, 9, 3]
        .into_iter()
        .map(|n| (id(n), book(shelf_id, &n.to_string())))
        .collect();
    assert_eq!(table.len(), 4);
    assert_eq!(
        table.ids().collect::<Vec<_>>(),
        [id(1), id(3), id(5), id(9)]
    );
    let titles: Vec<&str> = table.values().map(|book| book.title.as_str()).collect();
    assert_eq!(titles, ["1", "3", "5", "9"]);
    let some: Vec<BookId> = table.range(id(2)..=id(5)).map(|(id, _)| id).collect();
    assert_eq!(some, [id(3), id(5)]);
    assert_eq!(table.iter().count(), 4);

    let mut edited = table.clone();
    assert!(edited.ptr_eq(&table));
    assert_eq!(edited, table);
    edited.update(id(3), |book| book.title.push('!')).unwrap();
    assert!(!edited.ptr_eq(&table));
    assert_ne!(edited, table);
    assert_eq!(table.get(id(3)).unwrap().title, "3");
    assert!(Arc::ptr_eq(
        table.get(id(5)).unwrap(),
        edited.get(id(5)).unwrap()
    ));
    assert!(format!("{table:?}").starts_with("{b1: Book"));
}

/// One edit to a table of books, by the number in the id.
#[derive(Clone, Debug)]
enum Edit {
    Insert(u64, u8),
    Replace(u64, u8),
    /// Writes the value again as it is: a new allocation, no change.
    Rewrite(u64),
    Remove(u64),
}

fn edits() -> impl Strategy<Value = Vec<Edit>> {
    let edit = prop_oneof![
        3 => (1..60_u64, any::<u8>()).prop_map(|(id, title)| Edit::Insert(id, title)),
        3 => (1..60_u64, 0..4_u8).prop_map(|(id, title)| Edit::Replace(id, title)),
        1 => (1..60_u64).prop_map(Edit::Rewrite),
        2 => (1..60_u64).prop_map(Edit::Remove),
    ];
    prop::collection::vec(edit, 0..60)
}

fn apply(table: &mut Table<BookId, Book>, plain: &mut BTreeMap<u64, u8>, edits: &[Edit]) {
    let shelf_id: ShelfId = "s1".parse().unwrap();
    let id = |n: u64| -> BookId { format!("b{n}").parse().unwrap() };
    let titled = |title: u8| book(shelf_id, &title.to_string());
    for edit in edits {
        match *edit {
            Edit::Insert(n, title) => {
                if table.insert(id(n), titled(title)).is_ok() {
                    assert_eq!(plain.insert(n, title), None);
                }
            }
            Edit::Replace(n, title) => {
                if table.replace(id(n), titled(title)).is_ok() {
                    assert!(plain.insert(n, title).is_some());
                }
            }
            Edit::Rewrite(n) => {
                if let Some(title) = plain.get(&n) {
                    table.replace(id(n), titled(*title)).unwrap();
                }
            }
            Edit::Remove(n) => {
                assert_eq!(table.remove(id(n)).is_ok(), plain.remove(&n).is_some());
            }
        }
    }
}

proptest! {
    /// The change list between two tables is what comparing every entry
    /// of both would find, whatever was done to get from one to the
    /// other.
    #[test]
    fn what_changed_is_what_a_full_comparison_finds(
        start in edits(),
        left in edits(),
        right in edits(),
    ) {
        let (mut base, mut base_plain) = (Table::new(), BTreeMap::new());
        apply(&mut base, &mut base_plain, &start);
        // Two tables, each some edits away from one they both came from.
        let (mut old, mut old_plain) = (base.clone(), base_plain.clone());
        apply(&mut old, &mut old_plain, &left);
        let (mut new, mut new_plain) = (base.clone(), base_plain.clone());
        apply(&mut new, &mut new_plain, &right);

        let changes = TableChanges::between(&old, &new);
        let numbers = |ids: Vec<BookId>| ids.into_iter().map(TypedId::counter).collect::<Vec<_>>();
        let title = |book: &Arc<Book>| book.title.parse::<u8>().unwrap();

        let added: Vec<(u64, u8)> =
            changes.added.iter().map(|(id, book)| (id.counter(), title(book))).collect();
        let removed: Vec<(u64, u8)> =
            changes.removed.iter().map(|(id, book)| (id.counter(), title(book))).collect();
        let replaced: Vec<(u64, u8, u8)> = changes
            .replaced
            .iter()
            .map(|change| (change.id.counter(), title(&change.old), title(&change.new)))
            .collect();

        let want_added: Vec<(u64, u8)> = new_plain
            .iter()
            .filter(|(id, _)| !old_plain.contains_key(id))
            .map(|(id, title)| (*id, *title))
            .collect();
        let want_removed: Vec<(u64, u8)> = old_plain
            .iter()
            .filter(|(id, _)| !new_plain.contains_key(id))
            .map(|(id, title)| (*id, *title))
            .collect();
        let want_replaced: Vec<(u64, u8, u8)> = old_plain
            .iter()
            .filter_map(|(id, old)| new_plain.get(id).map(|new| (*id, *old, *new)))
            .filter(|(_, old, new)| old != new)
            .collect();

        prop_assert_eq!(added, want_added);
        prop_assert_eq!(removed, want_removed);
        prop_assert_eq!(replaced, want_replaced);
        prop_assert_eq!(changes.len(), changes.ids().count());
        prop_assert_eq!(changes.is_empty(), old_plain == new_plain);
        prop_assert_eq!(numbers(old.ids().collect()), old_plain.keys().copied().collect::<Vec<_>>());
        // And the other way round is the same list turned over.
        let back = TableChanges::between(&new, &old);
        prop_assert_eq!(back.added, changes.removed);
        prop_assert_eq!(back.removed, changes.added);
        prop_assert_eq!(back.replaced.len(), changes.replaced.len());
    }
}

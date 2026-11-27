// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The model's checks and its index against slower ways of finding the
//! same.
//!
//! Long random runs of edits (see `edits`) are committed to a project.
//! After every one:
//!
//! - the commit was accepted exactly when the same tables, read as a
//!   file is read with everything new, are accepted: checking what
//!   changed finds what checking everything finds;
//! - a refused commit left the document as it was;
//! - the index of every accepted document is the index built afresh from
//!   its rows, and what it says is at each tick is what a scan of the
//!   lane finds.

mod edits;
mod fixture;

use std::sync::Arc;

use concat_doc::model::{Items, Lanes, Model};
use concat_doc::{Document, MAX_COUNTER, Mint, Origin, Rejected, Writer};
use edits::*;
use proptest::prelude::*;
/// The index of a document against one built afresh from its rows, and
/// against a scan.
fn index_holds(doc: &Document<Model>) -> Result<(), TestCaseError> {
    let fresh: Items = doc
        .items
        .rows()
        .iter()
        .map(|(id, item)| (id, Arc::clone(item)))
        .collect();
    let ours = doc.items.index();
    let theirs = fresh.index();
    for lane in doc.lanes.ids() {
        let on: Vec<_> = ours.on_lane(lane).collect();
        prop_assert_eq!(&on, &theirs.on_lane(lane).collect::<Vec<_>>());
        let mut scanned: Vec<_> = doc
            .items
            .iter()
            .filter(|(_, item)| item.lane == lane)
            .map(|(id, item)| (id, item.start, item.end().unwrap()))
            .collect();
        scanned.sort_by_key(|(id, start, _)| (*start, *id));
        prop_assert_eq!(&on, &scanned);
        prop_assert_eq!(
            ours.transitions_on(lane).collect::<Vec<_>>(),
            theirs.transitions_on(lane).collect::<Vec<_>>()
        );
        for q in -2..60 {
            let t = quarters(q) + concat_doc::TickDur::new(q.rem_euclid(3));
            let hit = scanned
                .iter()
                .find(|(_, start, end)| *start <= t && t < *end)
                .map(|(id, _, _)| *id);
            prop_assert_eq!(ours.at(lane, t), hit);
        }
    }
    for media in doc.media.ids() {
        prop_assert_eq!(
            ours.using_media(media).collect::<Vec<_>>(),
            theirs.using_media(media).collect::<Vec<_>>()
        );
    }
    for sequence in doc.sequences.ids() {
        prop_assert_eq!(
            ours.nesting(sequence).collect::<Vec<_>>(),
            theirs.nesting(sequence).collect::<Vec<_>>()
        );
        let fresh_lanes: Lanes = doc
            .lanes
            .rows()
            .iter()
            .map(|(id, lane)| (id, Arc::clone(lane)))
            .collect();
        prop_assert_eq!(
            doc.lanes
                .index()
                .of(sequence)
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            fresh_lanes
                .index()
                .of(sequence)
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        );
    }
    for (id, item) in doc.items.iter() {
        prop_assert_eq!(
            ours.tied_to(id).collect::<Vec<_>>(),
            theirs.tied_to(id).collect::<Vec<_>>()
        );
        for nested in item.nested_ids() {
            prop_assert_eq!(ours.holding(nested).collect::<Vec<_>>(), vec![id]);
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn checking_what_changed_finds_what_checking_everything_finds(
        ops in prop::collection::vec(op(), 1..48),
    ) {
        let mut doc = start();
        for op in &ops {
            let mut draft = doc.begin("Random", Origin(0));
            run(&mut draft, op);
            let tables = draft.tables().clone();
            // A mint that has made every id: the rules are the question
            // here, not where the ids came from.
            let mint = Mint::resume([(Writer::LOCAL, MAX_COUNTER)]).unwrap();
            let whole = Document::load(tables, mint, doc.revision());
            match draft.commit() {
                Ok(commit) => {
                    prop_assert!(
                        whole.is_ok(),
                        "accepted, but the whole reads as {:?} after {:?}",
                        whole.err(),
                        op
                    );
                    doc = commit.into_document();
                    index_holds(&doc)?;
                }
                Err(Rejected::Invalid(broken)) => {
                    prop_assert!(
                        whole.is_err(),
                        "refused with {:?}, but the whole reads as valid after {:?}",
                        broken,
                        op
                    );
                }
                Err(Rejected::Unminted(ids)) => {
                    prop_assert!(false, "ids never made: {:?}", ids);
                }
            }
        }
    }

    #[test]
    fn the_index_is_its_rows_after_any_run_of_edits(
        ops in prop::collection::vec(op(), 1..48),
    ) {
        // The same edits, kept whether or not they keep the rules: the
        // index follows a draft's every edit, not only a document's.
        let doc = start();
        let mut draft = doc.begin("Random", Origin(0));
        for op in &ops {
            run(&mut draft, op);
        }
        let fresh: Items = draft
            .items
            .rows()
            .iter()
            .map(|(id, item)| (id, Arc::clone(item)))
            .collect();
        for lane in draft.lanes.ids() {
            prop_assert_eq!(
                draft.items.index().on_lane(lane).collect::<Vec<_>>(),
                fresh.index().on_lane(lane).collect::<Vec<_>>()
            );
        }
        for media in draft.media.ids() {
            prop_assert_eq!(
                draft.items.index().using_media(media).collect::<Vec<_>>(),
                fresh.index().using_media(media).collect::<Vec<_>>()
            );
        }
    }
}

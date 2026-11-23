// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Every rule of the model, broken once at commit and refused.
//!
//! The tests are named in the model's documentation, one for each rule
//! of its list, in that order.

mod fixture;

use std::collections::BTreeMap;

use concat_doc::model::{
    Animated, Background, BrushTool, Color, Content, Crop, Cutout, CutoutMode, Ease, Effect,
    EffectId, Enhanced, Entry, Font, Item, ItemId, Key, Lane, LaneId, Link, LinkKind, Loop, Marker,
    MarkerId, Mask, MaskId, MediaCut, MediaId, Model, Nest, OrderKey, Param, Raw, Residual, Rule,
    Sequence, SequenceId, ShapeStyle, SoundStream, Speed, Stabilize, Stroke, Subject, TICK_LIMIT,
    Take, TextRun, TimeCurve, TimeMap, TimePoint, Title, Violation, VoiceKind, VoiceMix,
};
use concat_doc::{
    ChangeSet, Document, Entry as HistoryEntry, History, Origin, Recorded, Rejected, Tick, TickDur,
    TypedId,
};
use fixture::*;

// --- What the rules leave alone ------------------------------------------

#[test]
fn an_empty_document_is_valid() {
    let empty = Model::default();
    let loaded = Document::load(empty, Default::default(), Default::default());
    assert!(loaded.is_ok());
    // And the fixture, read as a file is read, everything at once.
    let fx = fixture();
    let again = Document::load(
        fx.doc.tables().clone(),
        fx.doc.mint().clone(),
        fx.doc.revision(),
    );
    assert!(again.is_ok());
}

#[test]
fn a_change_to_nothing_is_no_revision_and_no_step() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    let mut history: History<Model, (), ()> = History::new(fx.doc.clone(), Default::default());
    // The item written again as it was.
    let mut draft = history.current().begin("Nothing", Origin(0));
    let same = Item::clone(draft.items.get(clip).unwrap());
    draft.items.replace(clip, same).unwrap();
    let commit = draft.commit().unwrap();
    assert!(commit.is_nothing());
    assert!(commit.changes().is_empty());
    assert_eq!(commit.document().revision(), fx.doc.revision());
    let recorded = history
        .record(commit, HistoryEntry::new(vec![], (), ()))
        .unwrap();
    assert_eq!(recorded, Recorded::Nothing);
    assert!(!history.can_undo());
}

#[test]
fn changing_the_view_is_no_undo_step() {
    let fx = fixture();
    let mut history: History<Model, (), ()> = History::new(fx.doc.clone(), Default::default());
    // A second sequence, as a step.
    let mut draft = history.current().begin("Add timeline", Origin(0));
    let second: SequenceId = draft.mint().unwrap();
    draft
        .sequences
        .insert(
            second,
            Sequence::new("Timeline 2", concat_doc::model::Frame::hd()),
        )
        .unwrap();
    draft.view.tabs.push(second);
    draft.view.showing = Some(second);
    let commit = draft.commit().unwrap();
    assert_eq!(
        history.record(commit, HistoryEntry::new(vec![], (), ())),
        Ok(Recorded::Step)
    );
    // Showing the first again and moving its tab is view: no step.
    let mut draft = history.current().begin("Select", Origin(0));
    draft.view.showing = Some(fx.seq);
    draft.view.tabs = vec![second, fx.seq];
    let commit = draft.commit().unwrap();
    assert_eq!(
        history.record(commit, HistoryEntry::new(vec![], (), ())),
        Ok(Recorded::View)
    );
    assert_eq!(history.undo_len(), 1);
    // Undoing the step that made the second sequence cannot leave it
    // showing or in a tab; the rest of the view stays.
    let undone = history.undo().unwrap();
    assert_eq!(undone.document.view.showing, Some(fx.seq));
    assert_eq!(undone.document.view.tabs, vec![fx.seq]);
    // Redone, it comes back where its tab was.
    let redone = history.redo().unwrap();
    assert_eq!(redone.document.view.tabs, vec![fx.seq, second]);
    assert_eq!(redone.document.view.showing, Some(fx.seq));
}

#[test]
fn every_table_is_compared_checked_and_shown() {
    assert_eq!(
        Model::TABLES,
        ["media", "sequences", "lanes", "items", "held"]
    );
    assert_eq!(Model::VALUES, ["fonts", "view", "residual"]);
    // Every table answers for its ids, and so do the ids held inside
    // items.
    let mut fx = fixture();
    let effect: EffectId = {
        let mut made = None;
        let mut item = cut(fx.low, 0.0, 1.0, fx.video);
        fx.doc = apply(&fx.doc, |draft| {
            let id: EffectId = draft.mint().unwrap();
            item.effects
                .push(Entry::Known(Effect::new(id, "concat.mono")));
            let clip: ItemId = draft.mint().unwrap();
            draft.items.insert(clip, item).unwrap();
            made = Some(id);
        });
        made.unwrap()
    };
    assert!(fx.doc.contains(fx.seq));
    assert!(fx.doc.contains(fx.low));
    assert!(fx.doc.contains(fx.video));
    assert!(fx.doc.contains(effect));
}

// --- 1. Time -------------------------------------------------------------

#[test]
fn a_tick_past_the_range_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    let far = Tick::new(TICK_LIMIT);
    // A start past the range.
    assert_refused(&fx.doc, clip, Rule::OutOfRange { what: "start" }, |draft| {
        draft
            .items
            .update(clip, |item| item.start = Tick::new(TICK_LIMIT + 1))
            .unwrap();
    });
    // A start inside it whose end is not.
    assert_refused(&fx.doc, clip, Rule::OutOfRange { what: "len" }, |draft| {
        draft.items.update(clip, |item| item.start = far).unwrap();
    });
    // A key far out.
    assert_refused(
        &fx.doc,
        clip,
        Rule::OutOfRange {
            what: "picture.opacity",
        },
        |draft| {
            draft
                .items
                .update(clip, |item| {
                    item.picture.opacity.keys = vec![Key::linear(Tick::MIN, 1.0)];
                })
                .unwrap();
        },
    );
    // A source tick worked out from the window that leaves the range.
    assert_refused(
        &fx.doc,
        clip,
        Rule::OutOfRange {
            what: "content.time",
        },
        |draft| {
            draft
                .items
                .update(clip, |item| {
                    item.offset = Tick::new(TICK_LIMIT - 10 * TICKS);
                    item.content = Content::Media(MediaCut {
                        time: TimeMap::Linear {
                            src_in: Tick::ZERO,
                            speed: Speed::new(16, 1).unwrap(),
                        },
                        ..MediaCut::whole(fx.video)
                    });
                })
                .unwrap();
        },
    );
    // A file's length.
    assert_refused(
        &fx.doc,
        fx.song,
        Rule::OutOfRange { what: "length" },
        |draft| {
            draft
                .media
                .update(fx.song, |file| {
                    file.length = concat_doc::TickLen::new(TICK_LIMIT + 1);
                })
                .unwrap();
        },
    );
}

const TICKS: i64 = concat_doc::TICKS_PER_SECOND;

// --- 2 to 14. What names what ---------------------------------------------

#[test]
fn a_lane_of_no_sequence_is_refused() {
    let fx = fixture();
    let mut ghost = None;
    let broken = refused(&fx.doc, |draft| {
        let nowhere: SequenceId = draft.mint().unwrap();
        let lane: LaneId = draft.mint().unwrap();
        draft
            .lanes
            .insert(lane, Lane::new(nowhere, OrderKey::middle()))
            .unwrap();
        ghost = Some((lane, nowhere));
    });
    let (lane, nowhere) = ghost.unwrap();
    assert!(broken.contains(&Violation {
        id: Some(lane.any()),
        rule: Rule::Dangling {
            what: "sequence",
            to: nowhere.any()
        }
    }));
}

#[test]
fn an_item_on_no_lane_is_refused() {
    let fx = fixture();
    let mut made = None;
    let broken = refused(&fx.doc, |draft| {
        let nowhere: LaneId = draft.mint().unwrap();
        let clip: ItemId = draft.mint().unwrap();
        draft
            .items
            .insert(clip, cut(nowhere, 0.0, 1.0, fx.video))
            .unwrap();
        made = Some((clip, nowhere));
    });
    let (clip, nowhere) = made.unwrap();
    assert!(broken.contains(&Violation {
        id: Some(clip.any()),
        rule: Rule::Dangling {
            what: "lane",
            to: nowhere.any()
        }
    }));
}

#[test]
fn a_cut_of_no_file_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 1.0, fx.video));
    let mut missing = None;
    let broken = refused(&fx.doc, |draft| {
        let gone: MediaId = draft.mint().unwrap();
        draft
            .items
            .update(clip, |item| {
                item.content = Content::Media(MediaCut::whole(gone))
            })
            .unwrap();
        missing = Some(gone);
    });
    assert!(broken.contains(&Violation {
        id: Some(clip.any()),
        rule: Rule::Dangling {
            what: "content.media",
            to: missing.unwrap().any()
        }
    }));
}

#[test]
fn a_nest_of_no_sequence_is_refused() {
    let fx = fixture();
    let mut made = None;
    let broken = refused(&fx.doc, |draft| {
        let gone: SequenceId = draft.mint().unwrap();
        let clip: ItemId = draft.mint().unwrap();
        let nest = Content::Nest(Nest {
            sequence: gone,
            time: TimeMap::default(),
        });
        draft
            .items
            .insert(clip, Item::new(fx.low, at(0.0), len(1.0), nest))
            .unwrap();
        made = Some((clip, gone));
    });
    let (clip, gone) = made.unwrap();
    assert!(broken.contains(&Violation {
        id: Some(clip.any()),
        rule: Rule::Dangling {
            what: "content.sequence",
            to: gone.any()
        }
    }));
}

fn enhanced_cut(fx: &Fixture, copy: MediaId, on: bool) -> Content {
    Content::Media(MediaCut {
        enhanced: Some(Enhanced {
            copy,
            copy_start: TickDur::ZERO,
            on,
        }),
        ..MediaCut::whole(fx.video)
    })
}

#[test]
fn an_enhanced_copy_that_is_not_there_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 1.0, fx.video));
    let mut missing = None;
    let broken = refused(&fx.doc, |draft| {
        let gone: MediaId = draft.mint().unwrap();
        let content = enhanced_cut(&fx, gone, true);
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
        missing = Some(gone);
    });
    assert!(broken.contains(&Violation {
        id: Some(clip.any()),
        rule: Rule::Dangling {
            what: "content.enhanced.copy",
            to: missing.unwrap().any()
        }
    }));
}

#[test]
fn a_background_that_is_not_a_still_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 1.0, fx.video));
    // An item's, of a video.
    assert_refused(
        &fx.doc,
        clip,
        Rule::WrongKind {
            what: "picture.background",
            to: fx.video.any(),
        },
        |draft| {
            draft
                .items
                .update(clip, |item| {
                    item.picture.background = Some(Background::Image(fx.video))
                })
                .unwrap();
        },
    );
    // A sequence's, of a song.
    assert_refused(
        &fx.doc,
        fx.seq,
        Rule::WrongKind {
            what: "background",
            to: fx.song.any(),
        },
        |draft| {
            draft
                .sequences
                .update(fx.seq, |s| s.background = Background::Image(fx.song))
                .unwrap();
        },
    );
    // A sequence's, of nothing.
    let mut missing = None;
    let broken = refused(&fx.doc, |draft| {
        let gone: MediaId = draft.mint().unwrap();
        draft
            .sequences
            .update(fx.seq, |s| s.background = Background::Image(gone))
            .unwrap();
        missing = Some(gone);
    });
    assert!(broken.contains(&Violation {
        id: Some(fx.seq.any()),
        rule: Rule::Dangling {
            what: "background",
            to: missing.unwrap().any()
        }
    }));
    // Of a still, it stands.
    apply(&fx.doc, |draft| {
        draft
            .sequences
            .update(fx.seq, |s| s.background = Background::Image(fx.still))
            .unwrap();
    });
}

/// A picture and its detached sound, on the two lanes.
fn detached(fx: &mut Fixture) -> (ItemId, ItemId) {
    let picture = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    let mut sound = cut(fx.high, 0.0, 2.0, fx.video);
    sound.content = Content::Media(MediaCut {
        take: Take::Sound,
        ..MediaCut::whole(fx.video)
    });
    sound.link = Some(Link {
        to: picture,
        kind: LinkKind::SoundOf,
    });
    let sound = place(&mut fx.doc, sound);
    (picture, sound)
}

#[test]
fn a_link_to_nothing_is_refused() {
    let mut fx = fixture();
    let (_, sound) = detached(&mut fx);
    let mut missing = None;
    let broken = refused(&fx.doc, |draft| {
        let gone: ItemId = draft.mint().unwrap();
        draft
            .items
            .update(sound, |item| item.link.as_mut().unwrap().to = gone)
            .unwrap();
        missing = Some(gone);
    });
    assert!(broken.contains(&Violation {
        id: Some(sound.any()),
        rule: Rule::Dangling {
            what: "link",
            to: missing.unwrap().any()
        }
    }));
}

#[test]
fn an_effect_on_a_mask_of_another_item_is_refused() {
    let mut fx = fixture();
    let mut ids = None;
    fx.doc = apply(&fx.doc, |draft| {
        let mask: MaskId = draft.mint().unwrap();
        let mut masked = cut(fx.high, 0.0, 1.0, fx.video);
        masked.masks.push(Entry::Known(Mask::circle(mask)));
        let a: ItemId = draft.mint().unwrap();
        draft.items.insert(a, masked).unwrap();
        let b: ItemId = draft.mint().unwrap();
        draft
            .items
            .insert(b, cut(fx.low, 0.0, 1.0, fx.video))
            .unwrap();
        ids = Some((mask, a, b));
    });
    let (mask, a, b) = ids.unwrap();
    let mut made = None;
    let broken = refused(&fx.doc, |draft| {
        let id: EffectId = draft.mint().unwrap();
        let mut effect = Effect::new(id, "concat.mono");
        effect.mask = Some(mask);
        draft
            .items
            .update(b, |item| item.effects.push(Entry::Known(effect)))
            .unwrap();
        made = Some(id);
    });
    assert!(broken.contains(&Violation {
        id: Some(made.unwrap().any()),
        rule: Rule::Dangling {
            what: "mask",
            to: mask.any()
        }
    }));
    // On its own item's mask, it stands.
    apply(&fx.doc, |draft| {
        let id: EffectId = draft.mint().unwrap();
        let mut effect = Effect::new(id, "concat.mono");
        effect.mask = Some(mask);
        draft
            .items
            .update(a, |item| item.effects.push(Entry::Known(effect)))
            .unwrap();
    });
}

#[test]
fn a_view_of_no_sequence_is_refused() {
    let fx = fixture();
    let mut missing = None;
    let broken = refused(&fx.doc, |draft| {
        let gone: SequenceId = draft.mint().unwrap();
        draft.view.showing = Some(gone);
        draft.view.tabs.push(gone);
        missing = Some(gone);
    });
    let gone = missing.unwrap();
    assert!(broken.contains(&Violation {
        id: Some(gone.any()),
        rule: Rule::Dangling {
            what: "view.showing",
            to: gone.any()
        }
    }));
    assert!(broken.contains(&Violation {
        id: Some(gone.any()),
        rule: Rule::Dangling {
            what: "view.tabs",
            to: gone.any()
        }
    }));
}

#[test]
fn removing_a_file_in_use_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 1.0, fx.video));
    assert_refused(
        &fx.doc,
        clip,
        Rule::Dangling {
            what: "content.media",
            to: fx.video.any(),
        },
        |draft| {
            draft.media.remove(fx.video).unwrap();
        },
    );
    // Named as a background, by a sequence and by an item.
    fx.doc = apply(&fx.doc, |draft| {
        draft
            .sequences
            .update(fx.seq, |s| s.background = Background::Image(fx.still))
            .unwrap();
        draft
            .items
            .update(clip, |item| {
                item.picture.background = Some(Background::Image(fx.still))
            })
            .unwrap();
    });
    let broken = refused(&fx.doc, |draft| {
        draft.media.remove(fx.still).unwrap();
    });
    for id in [fx.seq.any(), clip.any()] {
        assert!(
            broken.iter().any(|v| v.id == Some(id)
                && matches!(v.rule, Rule::Dangling { to, .. } if to == fx.still.any())),
            "{id} in {broken:?}"
        );
    }
    // With its items, the file may go.
    apply(&fx.doc, |draft| {
        draft.items.remove(clip).unwrap();
        draft.media.remove(fx.video).unwrap();
    });
}

#[test]
fn removing_a_sequence_in_use_is_refused() {
    let mut fx = fixture();
    // A second sequence, nested in the first.
    let mut inner = None;
    fx.doc = apply(&fx.doc, |draft| {
        let seq: SequenceId = draft.mint().unwrap();
        draft
            .sequences
            .insert(seq, Sequence::new("Inner", concat_doc::model::Frame::hd()))
            .unwrap();
        let clip: ItemId = draft.mint().unwrap();
        let nest = Content::Nest(Nest {
            sequence: seq,
            time: TimeMap::default(),
        });
        draft
            .items
            .insert(clip, Item::new(fx.low, at(0.0), len(1.0), nest))
            .unwrap();
        inner = Some((seq, clip));
    });
    let (seq, nesting) = inner.unwrap();
    assert_refused(
        &fx.doc,
        nesting,
        Rule::Dangling {
            what: "content.sequence",
            to: seq.any(),
        },
        |draft| {
            draft.sequences.remove(seq).unwrap();
        },
    );
    // The first, with its lanes still in it.
    assert_refused(
        &fx.doc,
        fx.low,
        Rule::Dangling {
            what: "sequence",
            to: fx.seq.any(),
        },
        |draft| {
            draft.sequences.remove(fx.seq).unwrap();
            draft.view.showing = Some(seq);
            draft.view.tabs = vec![seq];
        },
    );
}

#[test]
fn removing_a_lane_with_items_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.high, 0.0, 1.0, fx.video));
    assert_refused(
        &fx.doc,
        clip,
        Rule::Dangling {
            what: "lane",
            to: fx.high.any(),
        },
        |draft| {
            draft.lanes.remove(fx.high).unwrap();
        },
    );
    apply(&fx.doc, |draft| {
        draft.items.remove(clip).unwrap();
        draft.lanes.remove(fx.high).unwrap();
    });
}

#[test]
fn removing_what_a_sound_is_tied_to_is_refused() {
    let mut fx = fixture();
    let (picture, sound) = detached(&mut fx);
    assert_refused(
        &fx.doc,
        sound,
        Rule::Dangling {
            what: "link",
            to: picture.any(),
        },
        |draft| {
            draft.items.remove(picture).unwrap();
        },
    );
}

// --- 15, 16. Files ----------------------------------------------------------

#[test]
fn a_stream_listed_twice_is_refused() {
    let fx = fixture();
    assert_refused(&fx.doc, fx.video, Rule::StreamTwice { index: 1 }, |draft| {
        draft
            .media
            .update(fx.video, |file| {
                let again = file.streams[0].clone();
                file.streams.push(again);
            })
            .unwrap();
    });
}

#[test]
fn streams_on_a_file_without_sound_are_refused() {
    let fx = fixture();
    assert_refused(&fx.doc, fx.video, Rule::StreamsWithoutSound, |draft| {
        draft
            .media
            .update(fx.video, |file| file.has_sound = false)
            .unwrap();
    });
}

// --- 17 to 20. Sequences and lanes ------------------------------------------

#[test]
fn a_blank_sequence_name_is_refused() {
    let fx = fixture();
    assert_refused(&fx.doc, fx.seq, Rule::BlankName, |draft| {
        draft
            .sequences
            .update(fx.seq, |s| s.name = "  ".into())
            .unwrap();
    });
}

#[test]
fn a_sequence_inside_itself_is_refused() {
    let mut fx = fixture();
    let nest = |sequence| {
        Content::Nest(Nest {
            sequence,
            time: TimeMap::default(),
        })
    };
    // Directly.
    let mut made = None;
    let broken = refused(&fx.doc, |draft| {
        let clip: ItemId = draft.mint().unwrap();
        draft
            .items
            .insert(clip, Item::new(fx.low, at(0.0), len(1.0), nest(fx.seq)))
            .unwrap();
        made = Some(clip);
    });
    assert!(broken.contains(&Violation {
        id: Some(made.unwrap().any()),
        rule: Rule::Cycle { through: fx.seq }
    }));
    // Through a second sequence: the first holds the second, and an item
    // of the second's would hold the first.
    let mut inner = None;
    fx.doc = apply(&fx.doc, |draft| {
        let seq: SequenceId = draft.mint().unwrap();
        draft
            .sequences
            .insert(seq, Sequence::new("Inner", concat_doc::model::Frame::hd()))
            .unwrap();
        let lane: LaneId = draft.mint().unwrap();
        draft
            .lanes
            .insert(lane, Lane::new(seq, OrderKey::middle()))
            .unwrap();
        let clip: ItemId = draft.mint().unwrap();
        draft
            .items
            .insert(clip, Item::new(fx.low, at(0.0), len(1.0), nest(seq)))
            .unwrap();
        inner = Some(lane);
    });
    let lane = inner.unwrap();
    let mut made = None;
    let broken = refused(&fx.doc, |draft| {
        let clip: ItemId = draft.mint().unwrap();
        draft
            .items
            .insert(clip, Item::new(lane, at(0.0), len(1.0), nest(fx.seq)))
            .unwrap();
        made = Some(clip);
    });
    assert!(broken.contains(&Violation {
        id: Some(made.unwrap().any()),
        rule: Rule::Cycle { through: fx.seq }
    }));
}

#[test]
fn moving_a_lane_to_another_sequence_is_refused() {
    let fx = fixture();
    assert_refused(&fx.doc, fx.high, Rule::LaneMoved, |draft| {
        let other: SequenceId = draft.mint().unwrap();
        draft
            .sequences
            .insert(
                other,
                Sequence::new("Other", concat_doc::model::Frame::hd()),
            )
            .unwrap();
        draft
            .lanes
            .update(fx.high, |lane| lane.sequence = other)
            .unwrap();
    });
}

#[test]
fn two_lanes_at_one_place_are_refused() {
    let fx = fixture();
    assert_refused(
        &fx.doc,
        fx.high,
        Rule::LaneOrderTaken { with: fx.low },
        |draft| {
            draft
                .lanes
                .update(fx.high, |lane| lane.order = OrderKey::parse("a").unwrap())
                .unwrap();
        },
    );
}

// --- 21 to 39. Items -----------------------------------------------------------

#[test]
fn overlapping_items_on_a_lane_are_refused() {
    let mut fx = fixture();
    let a = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    let b = place(&mut fx.doc, cut(fx.low, 2.0, 2.0, fx.video));
    let c = place(&mut fx.doc, cut(fx.low, 6.0, 1.0, fx.video));
    // Touching is not overlapping: a and b are end to end. Moving b back
    // a frame is.
    assert_refused(&fx.doc, b, Rule::Overlap { with: a }, |draft| {
        draft.items.update(b, |item| item.start = at(1.9)).unwrap();
    });
    // A new item over two of them, and one inside another.
    let mut made = None;
    let broken = refused(&fx.doc, |draft| {
        let long: ItemId = draft.mint().unwrap();
        draft
            .items
            .insert(long, cut(fx.low, 1.0, 6.0, fx.video))
            .unwrap();
        made = Some(long);
    });
    let long = made.unwrap();
    for other in [a, b, c] {
        assert!(
            broken
                .iter()
                .any(|v| v.rule == Rule::Overlap { with: other }
                    || (v.id == Some(other.any()) && v.rule == Rule::Overlap { with: long })),
            "{other} in {broken:?}"
        );
    }
    // A long one earlier on the lane, reaching over a changed one.
    assert_refused(&fx.doc, c, Rule::Overlap { with: b }, |draft| {
        draft.items.update(c, |item| item.start = at(3.5)).unwrap();
    });
    // Of every kind: a title over a cut.
    let title = Item::new(fx.low, at(0.5), len(1.0), Content::Title(Title::default()));
    let mut made = None;
    let broken = refused(&fx.doc, |draft| {
        let id: ItemId = draft.mint().unwrap();
        draft.items.insert(id, title).unwrap();
        made = Some(id);
    });
    assert!(broken.contains(&Violation {
        id: Some(made.unwrap().any()),
        rule: Rule::Overlap { with: a }
    }));
    // Two that swap places in one change are fine.
    apply(&fx.doc, |draft| {
        draft.items.update(a, |item| item.start = at(2.0)).unwrap();
        draft.items.update(b, |item| item.start = at(0.0)).unwrap();
    });
}

#[test]
fn the_sound_of_a_still_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 1.0, fx.still));
    assert_refused(
        &fx.doc,
        clip,
        Rule::WrongKind {
            what: "content.take",
            to: fx.still.any(),
        },
        |draft| {
            draft
                .items
                .update(clip, |item| {
                    if let Content::Media(cut) = &mut item.content {
                        cut.take = Take::Sound;
                    }
                })
                .unwrap();
        },
    );
    // A video that a probe now calls a still puts the items that take
    // its sound in question.
    let mut sound = cut(fx.low, 2.0, 1.0, fx.video);
    sound.content = Content::Media(MediaCut {
        take: Take::Sound,
        ..MediaCut::whole(fx.video)
    });
    let sound = place(&mut fx.doc, sound);
    assert_refused(
        &fx.doc,
        sound,
        Rule::WrongKind {
            what: "content.take",
            to: fx.video.any(),
        },
        |draft| {
            draft
                .media
                .update(fx.video, |file| {
                    file.kind = concat_doc::model::MediaKind::Image;
                })
                .unwrap();
        },
    );
}

#[test]
fn a_stream_the_file_does_not_list_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 1.0, fx.video));
    assert_refused(&fx.doc, clip, Rule::NoSuchStream { index: 3 }, |draft| {
        draft
            .items
            .update(clip, |item| {
                if let Content::Media(cut) = &mut item.content {
                    cut.stream = Some(3);
                }
            })
            .unwrap();
    });
    // The file's list changing under it puts it in question too.
    fx.doc = apply(&fx.doc, |draft| {
        draft
            .items
            .update(clip, |item| {
                if let Content::Media(cut) = &mut item.content {
                    cut.stream = Some(1);
                }
            })
            .unwrap();
    });
    assert_refused(&fx.doc, clip, Rule::NoSuchStream { index: 1 }, |draft| {
        draft
            .media
            .update(fx.video, |file| {
                file.streams = vec![SoundStream {
                    index: 2,
                    ..SoundStream::default()
                }]
            })
            .unwrap();
    });
}

#[test]
fn an_enhanced_copy_that_is_the_original_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 1.0, fx.video));
    let content = enhanced_cut(&fx, fx.video, false);
    assert_refused(&fx.doc, clip, Rule::EnhancedIsOriginal, |draft| {
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
    });
}

fn with_time(fx: &Fixture, time: TimeMap) -> Content {
    Content::Media(MediaCut {
        time,
        ..MediaCut::whole(fx.video)
    })
}

fn keyed(points: &[(f64, f64)], curve: TimeCurve) -> TimeMap {
    let mut points = points.iter().map(|(c, s)| TimePoint {
        at: at(*c),
        source: at(*s),
    });
    let first = points.next().unwrap();
    TimeMap::Keyed {
        first,
        rest: points.map(|point| (curve.clone(), point)).collect(),
    }
}

#[test]
fn a_speed_past_sixteen_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 0.5, fx.video));
    let what = "content.time";
    for speed in [Speed::new(17, 1), Speed::new(-17, 1), Speed::new(1, 17)] {
        let content = with_time(
            &fx,
            TimeMap::Linear {
                src_in: Tick::ZERO,
                speed: speed.unwrap(),
            },
        );
        assert_refused(&fx.doc, clip, Rule::SpeedNotAllowed { what }, |draft| {
            draft
                .items
                .update(clip, |item| item.content = content)
                .unwrap();
        });
    }
    // Sixteen times, and a sixteenth, both ways, stand.
    for speed in [Speed::new(16, 1), Speed::new(1, 16)] {
        let content = with_time(
            &fx,
            TimeMap::Linear {
                src_in: Tick::ZERO,
                speed: speed.unwrap(),
            },
        );
        apply(&fx.doc, |draft| {
            draft
                .items
                .update(clip, |item| item.content = content)
                .unwrap();
        });
    }
    // Between keys: twenty seconds of source in one of content.
    let content = with_time(&fx, keyed(&[(0.0, 0.0), (1.0, 20.0)], TimeCurve::Linear));
    assert_refused(&fx.doc, clip, Rule::SpeedNotAllowed { what }, |draft| {
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
    });
    // At the end of a smooth stretch.
    let smooth = TimeCurve::Smooth {
        leave: Speed::new(1, 1).unwrap(),
        arrive: Speed::new(1, 32).unwrap(),
    };
    let content = with_time(&fx, keyed(&[(0.0, 0.0), (1.0, 1.0)], smooth));
    assert_refused(&fx.doc, clip, Rule::SpeedNotAllowed { what }, |draft| {
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
    });
}

#[test]
fn a_time_map_of_one_key_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 1.0, fx.video));
    let content = with_time(&fx, keyed(&[(0.0, 0.0)], TimeCurve::Linear));
    assert_refused(&fx.doc, clip, Rule::TimeKeysTooFew, |draft| {
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
    });
}

#[test]
fn time_keys_out_of_order_are_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 1.0, fx.video));
    for points in [
        [(0.0, 0.0), (2.0, 2.0), (1.0, 3.0)],
        [(0.0, 0.0), (1.0, 1.0), (1.0, 2.0)],
    ] {
        let content = with_time(&fx, keyed(&points, TimeCurve::Linear));
        assert_refused(&fx.doc, clip, Rule::TimeKeysOutOfOrder, |draft| {
            draft
                .items
                .update(clip, |item| item.content = content)
                .unwrap();
        });
    }
}

#[test]
fn a_time_map_that_turns_back_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 1.0, fx.video));
    // Forward, then back.
    let content = with_time(
        &fx,
        keyed(&[(0.0, 0.0), (1.0, 2.0), (2.0, 1.0)], TimeCurve::Linear),
    );
    assert_refused(&fx.doc, clip, Rule::TimeNotMonotone, |draft| {
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
    });
    // Standing still.
    let content = with_time(&fx, keyed(&[(0.0, 1.0), (1.0, 1.0)], TimeCurve::Linear));
    assert_refused(&fx.doc, clip, Rule::TimeNotMonotone, |draft| {
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
    });
    // Eased so hard that it would overshoot the key and come back: speeds
    // of eight into a stretch whose pace is one.
    let overshoot = TimeCurve::Smooth {
        leave: Speed::new(8, 1).unwrap(),
        arrive: Speed::new(8, 1).unwrap(),
    };
    let content = with_time(&fx, keyed(&[(0.0, 0.0), (1.0, 1.0)], overshoot));
    assert_refused(&fx.doc, clip, Rule::TimeNotMonotone, |draft| {
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
    });
    // A smooth stretch whose speed runs against it.
    let against = TimeCurve::Smooth {
        leave: Speed::new(-1, 1).unwrap(),
        arrive: Speed::new(1, 1).unwrap(),
    };
    let content = with_time(&fx, keyed(&[(0.0, 0.0), (1.0, 1.0)], against));
    assert_refused(&fx.doc, clip, Rule::TimeNotMonotone, |draft| {
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
    });
    // A ramp from one to three times, as the corpus's curve has it: the
    // source at the end is the area under the speed, two seconds.
    let ramp = TimeCurve::Smooth {
        leave: Speed::new(1, 1).unwrap(),
        arrive: Speed::new(3, 1).unwrap(),
    };
    let content = with_time(&fx, keyed(&[(0.0, 0.0), (1.0, 2.0)], ramp));
    apply(&fx.doc, |draft| {
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
    });
}

#[test]
fn a_window_past_the_time_keys_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    let content = with_time(&fx, keyed(&[(0.0, 0.0), (1.0, 1.0)], TimeCurve::Linear));
    assert_refused(&fx.doc, clip, Rule::WindowPastKeys, |draft| {
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
    });
}

#[test]
fn a_window_past_the_file_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    let past = Rule::WindowPastSource { source: fx.video };
    // Longer than the file.
    assert_refused(&fx.doc, clip, past.clone(), |draft| {
        draft
            .items
            .update(clip, |item| item.len = len(11.0))
            .unwrap();
    });
    // From before its start.
    let content = with_time(&fx, TimeMap::from_in(at(-1.0)));
    assert_refused(&fx.doc, clip, past.clone(), |draft| {
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
    });
    // Twice as fast for six seconds: twelve of a ten-second file.
    let content = with_time(
        &fx,
        TimeMap::Linear {
            src_in: Tick::ZERO,
            speed: Speed::new(2, 1).unwrap(),
        },
    );
    assert_refused(&fx.doc, clip, past.clone(), |draft| {
        draft
            .items
            .update(clip, |item| {
                item.content = content;
                item.len = len(6.0);
            })
            .unwrap();
    });
    // In reverse from its start.
    let content = with_time(
        &fx,
        TimeMap::Linear {
            src_in: at(1.0),
            speed: Speed::new(-1, 1).unwrap(),
        },
    );
    assert_refused(&fx.doc, clip, past.clone(), |draft| {
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
    });
    // A file that a probe now finds shorter puts its items in question.
    assert_refused(&fx.doc, clip, past.clone(), |draft| {
        draft
            .media
            .update(fx.video, |file| file.length = Some(len(1.0)))
            .unwrap();
    });
    // While a copy of its last four seconds shows, the first six are not
    // there to show.
    let broken = refused(&fx.doc, |draft| {
        let id: MediaId = draft.mint().unwrap();
        let mut file = fixture::video_file();
        file.path = "/footage/a-enhanced.mp4".into();
        file.length = Some(len(4.0));
        draft.media.insert(id, file).unwrap();
        let content = Content::Media(MediaCut {
            enhanced: Some(Enhanced {
                copy: id,
                copy_start: dur(6.0),
                on: true,
            }),
            ..MediaCut::whole(fx.video)
        });
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
    });
    assert!(broken.contains(&Violation {
        id: Some(clip.any()),
        rule: past
    }));
    // A still has no time to run out of: a minute of one stands.
    place(&mut fx.doc, cut(fx.high, 0.0, 60.0, fx.still));
}

fn opacity_keys(keys: Vec<Key<f64>>) -> Animated<f64> {
    Animated { value: 1.0, keys }
}

#[test]
fn keys_out_of_order_are_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    let what = "picture.opacity";
    for keys in [
        vec![Key::linear(at(1.0), 0.0), Key::linear(at(0.0), 1.0)],
        vec![Key::linear(at(1.0), 0.0), Key::linear(at(1.0), 1.0)],
    ] {
        assert_refused(&fx.doc, clip, Rule::KeysOutOfOrder { what }, |draft| {
            draft
                .items
                .update(clip, |item| item.picture.opacity = opacity_keys(keys))
                .unwrap();
        });
    }
    // An effect's knob, and a mask's number, are held to the same.
    let mut ids = None;
    let broken = refused(&fx.doc, |draft| {
        let fx_id: EffectId = draft.mint().unwrap();
        let mut effect = Effect::new(fx_id, "concat.exposure");
        effect.params.insert(
            "stops".into(),
            Param {
                value: None,
                keys: vec![Key::linear(at(1.0), 0.0), Key::linear(at(0.5), -2.0)],
            },
        );
        let mask_id: MaskId = draft.mint().unwrap();
        let mut mask = Mask::circle(mask_id);
        mask.x.keys = vec![Key::linear(at(1.0), 0.0), Key::linear(at(1.0), 0.5)];
        draft
            .items
            .update(clip, |item| {
                item.effects.push(Entry::Known(effect));
                item.masks.push(Entry::Known(mask));
            })
            .unwrap();
        ids = Some((fx_id, mask_id));
    });
    let (fx_id, mask_id) = ids.unwrap();
    assert!(broken.contains(&Violation {
        id: Some(fx_id.any()),
        rule: Rule::KeysOutOfOrder { what: "params" }
    }));
    assert!(broken.contains(&Violation {
        id: Some(mask_id.any()),
        rule: Rule::KeysOutOfOrder { what: "x" }
    }));
}

#[test]
fn an_ease_that_runs_back_in_time_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    for ease in [
        Ease::Bezier {
            x1: -0.1,
            y1: 0.0,
            x2: 1.0,
            y2: 1.0,
        },
        Ease::Bezier {
            x1: 0.0,
            y1: 0.0,
            x2: 1.5,
            y2: 1.0,
        },
        Ease::Bezier {
            x1: 0.0,
            y1: f64::NAN,
            x2: 1.0,
            y2: 1.0,
        },
    ] {
        let keys = vec![
            Key::linear(at(0.0), 0.0),
            Key {
                at: at(1.0),
                value: 1.0,
                ease,
            },
        ];
        assert_refused(
            &fx.doc,
            clip,
            Rule::BadEase {
                what: "sound.volume",
            },
            |draft| {
                draft
                    .items
                    .update(clip, |item| item.sound.volume = opacity_keys(keys))
                    .unwrap();
            },
        );
    }
    // Overshooting in progress is an ease, not a fault; and an ease this
    // build does not know is kept.
    let keys = vec![
        Key::linear(at(0.0), 0.0),
        Key {
            at: at(1.0),
            value: 1.0,
            ease: Ease::Bezier {
                x1: 0.3,
                y1: -0.5,
                x2: 0.7,
                y2: 1.5,
            },
        },
        Key {
            at: at(1.5),
            value: 1.0,
            ease: Ease::Unknown(Raw::from("\"spring\"")),
        },
    ];
    apply(&fx.doc, |draft| {
        draft
            .items
            .update(clip, |item| item.sound.volume = opacity_keys(keys))
            .unwrap();
    });
}

#[test]
fn a_number_that_is_not_a_number_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    for (what, bad) in [
        ("picture.offset_x", f64::NAN),
        ("picture.rotation", f64::INFINITY),
        ("picture.offset_y", f64::NEG_INFINITY),
    ] {
        assert_refused(&fx.doc, clip, Rule::NotFinite { what }, |draft| {
            draft
                .items
                .update(clip, |item| match what {
                    "picture.offset_x" => item.picture.offset_x.value = bad,
                    "picture.rotation" => {
                        item.picture.rotation.keys = vec![Key::linear(at(0.0), bad)]
                    }
                    _ => item.picture.offset_y.value = bad,
                })
                .unwrap();
        });
    }
    assert_refused(
        &fx.doc,
        clip,
        Rule::NotFinite {
            what: "picture.crop",
        },
        |draft| {
            draft
                .items
                .update(clip, |item| {
                    item.picture.crop = Some(Crop {
                        left: f64::NAN,
                        top: 0.0,
                        right: 0.0,
                        bottom: 0.0,
                    })
                })
                .unwrap();
        },
    );
}

/// One field set outside what it can mean, and the name the refusal
/// gives it.
type Breaks = (&'static str, Box<dyn Fn(&mut Item)>);

/// The same for a mask's numbers.
type MaskBreaks = (&'static str, Box<dyn Fn(&mut Mask)>);

fn title_of(item: &mut Item) -> &mut Title {
    if !matches!(item.content, Content::Title(_)) {
        item.content = Content::Title(Title::default());
    }
    match &mut item.content {
        Content::Title(title) => title,
        _ => unreachable!(),
    }
}

fn shape_of(item: &mut Item) -> &mut ShapeStyle {
    if !matches!(item.content, Content::Shape(_)) {
        item.content = Content::Shape(ShapeStyle::default());
    }
    match &mut item.content {
        Content::Shape(shape) => shape,
        _ => unreachable!(),
    }
}

#[test]
fn numbers_outside_their_field_are_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    let crop = |left, top, right, bottom| Crop {
        left,
        top,
        right,
        bottom,
    };
    let cases: Vec<Breaks> = vec![
        (
            "picture.opacity",
            Box::new(|i| i.picture.opacity.value = 1.5),
        ),
        (
            "picture.opacity",
            Box::new(|i| i.picture.opacity.keys = vec![Key::linear(at(0.0), -0.1)]),
        ),
        ("picture.scale", Box::new(|i| i.picture.scale.value = 0.0)),
        (
            "picture.stretch_x",
            Box::new(|i| i.picture.stretch_x.value = -1.0),
        ),
        (
            "picture.stretch_y",
            Box::new(|i| i.picture.stretch_y.value = 0.0),
        ),
        ("sound.volume", Box::new(|i| i.sound.volume.value = -0.5)),
        (
            "picture.crop",
            Box::new(move |i| i.picture.crop = Some(crop(0.6, 0.0, 0.4, 0.0))),
        ),
        (
            "picture.crop",
            Box::new(move |i| i.picture.crop = Some(crop(0.0, 1.0, 0.0, 0.0))),
        ),
        (
            "picture.crop",
            Box::new(move |i| i.picture.crop = Some(crop(-0.1, 0.0, 0.0, 0.0))),
        ),
        (
            "picture.cutout.feather",
            Box::new(|i| {
                i.picture.cutout = Some(Cutout {
                    mode: CutoutMode::Auto,
                    subject: Subject::Auto,
                    feather: -0.01,
                    strokes: vec![],
                })
            }),
        ),
        (
            "picture.cutout.strokes",
            Box::new(|i| {
                i.picture.cutout = Some(Cutout {
                    mode: CutoutMode::Custom,
                    subject: Subject::Person,
                    feather: 0.01,
                    strokes: vec![Stroke {
                        tool: BrushTool::Brush,
                        size: 0.0,
                        points: vec![[0.5, 0.5]],
                        at: None,
                    }],
                })
            }),
        ),
        (
            "picture.stabilize.smoothness",
            Box::new(|i| {
                i.picture.stabilize = Some(Stabilize {
                    smoothness: 1.5,
                    max_zoom: 1.2,
                    keep_rotation: false,
                    tripod: false,
                })
            }),
        ),
        (
            "picture.stabilize.max_zoom",
            Box::new(|i| {
                i.picture.stabilize = Some(Stabilize {
                    smoothness: 0.5,
                    max_zoom: 0.9,
                    keep_rotation: false,
                    tripod: false,
                })
            }),
        ),
        (
            "picture.background",
            Box::new(|i| i.picture.background = Some(Background::Blur { amount: 2.0 })),
        ),
        (
            "picture.motion",
            Box::new(|i| {
                i.picture.motion.repeat = Some(Loop {
                    name: "pulse".into(),
                    lap: len(1.0),
                    amount: 2.5,
                })
            }),
        ),
        (
            "sound.voice",
            Box::new(|i| {
                i.sound.voice = Some(VoiceMix {
                    kind: VoiceKind::Speech,
                    voice: -1.0,
                    background: 0.0,
                })
            }),
        ),
        ("title.font_size", Box::new(|i| title_of(i).font_size = 0.0)),
        (
            "title.font_weight",
            Box::new(|i| title_of(i).font_weight = 950.0),
        ),
        ("title.opacity", Box::new(|i| title_of(i).opacity = 1.1)),
        (
            "title.line_height",
            Box::new(|i| title_of(i).line_height = 0.4),
        ),
        (
            "title.max_width",
            Box::new(|i| title_of(i).max_width = -0.1),
        ),
        (
            "title.max_height",
            Box::new(|i| title_of(i).max_height = -0.1),
        ),
        (
            "title.stroke_width",
            Box::new(|i| title_of(i).stroke_width = -0.1),
        ),
        (
            "title.plate_padding_x",
            Box::new(|i| title_of(i).plate_padding_x = -0.1),
        ),
        (
            "title.plate_padding_y",
            Box::new(|i| title_of(i).plate_padding_y = -0.1),
        ),
        (
            "title.plate_radius",
            Box::new(|i| title_of(i).plate_radius = -0.1),
        ),
        (
            "title.runs.font_scale",
            Box::new(|i| {
                title_of(i).runs = vec![TextRun {
                    start: 0,
                    end: 1,
                    font_scale: Some(20.0),
                    ..TextRun::default()
                }]
            }),
        ),
        (
            "title.runs.font_weight",
            Box::new(|i| {
                title_of(i).runs = vec![TextRun {
                    start: 0,
                    end: 1,
                    font_weight: Some(50.0),
                    ..TextRun::default()
                }]
            }),
        ),
        ("shape.width", Box::new(|i| shape_of(i).width = -1.0)),
        ("shape.height", Box::new(|i| shape_of(i).height = -1.0)),
        (
            "shape.corner_radius",
            Box::new(|i| shape_of(i).corner_radius = 1.5),
        ),
        ("shape.sides", Box::new(|i| shape_of(i).sides = 2)),
        ("shape.sides", Box::new(|i| shape_of(i).sides = 13)),
        ("shape.points", Box::new(|i| shape_of(i).points = 25)),
        (
            "shape.inner_radius",
            Box::new(|i| shape_of(i).inner_radius = -0.1),
        ),
        (
            "shape.stroke_width",
            Box::new(|i| shape_of(i).stroke_width = -0.1),
        ),
        (
            "shape.trim_start",
            Box::new(|i| shape_of(i).trim_start = -0.1),
        ),
        ("shape.trim_end", Box::new(|i| shape_of(i).trim_end = 1.1)),
        (
            "shape.shadow.distance",
            Box::new(|i| {
                shape_of(i).shadow = Some(concat_doc::model::ShapeShadow {
                    color: Color::BLACK,
                    angle: 90.0,
                    distance: -0.1,
                    blur: 0.0,
                })
            }),
        ),
        (
            "shape.shadow.blur",
            Box::new(|i| {
                shape_of(i).shadow = Some(concat_doc::model::ShapeShadow {
                    color: Color::BLACK,
                    angle: 90.0,
                    distance: 0.0,
                    blur: -0.1,
                })
            }),
        ),
    ];
    for (what, change) in cases {
        assert_refused(&fx.doc, clip, Rule::OutOfBounds { what }, |draft| {
            draft.items.update(clip, |item| change(item)).unwrap();
        });
    }
    // A mask's numbers.
    let mask_cases: Vec<MaskBreaks> = vec![
        ("scale", Box::new(|m| m.scale.value = 0.0)),
        ("feather", Box::new(|m| m.feather.value = 1.5)),
        ("opacity", Box::new(|m| m.opacity.value = -0.5)),
        (
            "source.width",
            Box::new(|m| {
                if let concat_doc::model::MaskSource::Shape { width, .. } = &mut m.source {
                    width.value = -0.1;
                }
            }),
        ),
        (
            "source.height",
            Box::new(|m| {
                if let concat_doc::model::MaskSource::Shape { height, .. } = &mut m.source {
                    height.value = -0.1;
                }
            }),
        ),
        (
            "source.roundness",
            Box::new(|m| {
                if let concat_doc::model::MaskSource::Shape { roundness, .. } = &mut m.source {
                    roundness.value = 2.0;
                }
            }),
        ),
    ];
    for (what, change) in mask_cases {
        let mut made = None;
        let broken = refused(&fx.doc, |draft| {
            let id: MaskId = draft.mint().unwrap();
            let mut mask = Mask::circle(id);
            change(&mut mask);
            draft
                .items
                .update(clip, |item| item.masks.push(Entry::Known(mask)))
                .unwrap();
            made = Some(id);
        });
        assert!(
            broken.contains(&Violation {
                id: Some(made.unwrap().any()),
                rule: Rule::OutOfBounds { what }
            }),
            "{what}: {broken:?}"
        );
    }
    // A sequence's background.
    assert_refused(
        &fx.doc,
        fx.seq,
        Rule::OutOfBounds { what: "background" },
        |draft| {
            draft
                .sequences
                .update(fx.seq, |s| s.background = Background::Blur { amount: -0.5 })
                .unwrap();
        },
    );
}

#[test]
fn a_negative_length_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    assert_refused(
        &fx.doc,
        clip,
        Rule::Negative {
            what: "sound.fade_in",
        },
        |draft| {
            draft
                .items
                .update(clip, |item| item.sound.fade_in = dur(-0.5))
                .unwrap();
        },
    );
    assert_refused(
        &fx.doc,
        clip,
        Rule::Negative {
            what: "sound.fade_out",
        },
        |draft| {
            draft
                .items
                .update(clip, |item| item.sound.fade_out = dur(-0.5))
                .unwrap();
        },
    );
    let mut made = None;
    let broken = refused(&fx.doc, |draft| {
        let id: MarkerId = draft.mint().unwrap();
        let marker = Marker {
            id,
            at: at(1.0),
            len: dur(-1.0),
            name: "Beat".into(),
            color: None,
            residual: Residual::default(),
        };
        draft
            .sequences
            .update(fx.seq, |s| s.markers.push(Entry::Known(marker)))
            .unwrap();
        made = Some(id);
    });
    assert!(broken.contains(&Violation {
        id: Some(made.unwrap().any()),
        rule: Rule::Negative { what: "len" }
    }));
    // Where an enhanced copy starts.
    let broken = refused(&fx.doc, |draft| {
        let id: MediaId = draft.mint().unwrap();
        draft.media.insert(id, fixture::video_file()).unwrap();
        let content = Content::Media(MediaCut {
            enhanced: Some(Enhanced {
                copy: id,
                copy_start: dur(-1.0),
                on: false,
            }),
            ..MediaCut::whole(fx.video)
        });
        draft
            .items
            .update(clip, |item| item.content = content)
            .unwrap();
    });
    assert!(broken.contains(&Violation {
        id: Some(clip.any()),
        rule: Rule::Negative {
            what: "content.enhanced.copy_start"
        }
    }));
}

#[test]
fn an_effect_of_no_package_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    let mut made = None;
    let broken = refused(&fx.doc, |draft| {
        let id: EffectId = draft.mint().unwrap();
        draft
            .items
            .update(clip, |item| {
                item.filters.push(Entry::Known(Effect::new(id, "")))
            })
            .unwrap();
        made = Some(id);
    });
    assert!(broken.contains(&Violation {
        id: Some(made.unwrap().any()),
        rule: Rule::EmptyPackage
    }));
    // An entry this build could not read is kept as its text, and is no
    // fault: the corpus's "unknown" case.
    apply(&fx.doc, |draft| {
        draft
            .items
            .update(clip, |item| {
                item.effects.push(Entry::Unread(Raw::from(
                    r#"{"mystery":true,"depth":{"a":1}}"#,
                )))
            })
            .unwrap();
    });
}

#[test]
fn an_empty_knob_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    for (name, param) in [
        (
            "stops",
            Param {
                value: None,
                keys: vec![],
            },
        ),
        ("", Animated::fixed(Some(1.0))),
    ] {
        let mut made = None;
        let broken = refused(&fx.doc, |draft| {
            let id: EffectId = draft.mint().unwrap();
            let mut effect = Effect::new(id, "concat.exposure");
            effect.params = BTreeMap::from([(name.to_owned(), param)]);
            draft
                .items
                .update(clip, |item| item.effects.push(Entry::Known(effect)))
                .unwrap();
            made = Some(id);
        });
        assert!(broken.contains(&Violation {
            id: Some(made.unwrap().any()),
            rule: Rule::EmptyParam {
                name: name.to_owned()
            }
        }));
    }
    // Keys with no value of its own fall back to the package's default.
    apply(&fx.doc, |draft| {
        let id: EffectId = draft.mint().unwrap();
        let mut effect = Effect::new(id, "concat.exposure");
        effect.params.insert(
            "stops".into(),
            Param {
                value: None,
                keys: vec![Key::linear(at(0.0), 0.0), Key::linear(at(2.0), -2.0)],
            },
        );
        draft
            .items
            .update(clip, |item| item.effects.push(Entry::Known(effect)))
            .unwrap();
    });
}

fn transition_over(
    fx: &Fixture,
    draft: &mut concat_doc::Draft<Model>,
    start: f64,
    length: f64,
) -> ItemId {
    let package: EffectId = draft.mint().unwrap();
    let id: ItemId = draft.mint().unwrap();
    let item = Item::new(
        fx.high,
        at(start),
        len(length),
        Content::Transition(Effect::new(package, "concat.dissolve")),
    );
    draft.items.insert(id, item).unwrap();
    id
}

#[test]
fn a_transition_over_nothing_is_refused() {
    let mut fx = fixture();
    // Over nothing at all.
    let mut made = None;
    let broken = refused(&fx.doc, |draft| {
        made = Some(transition_over(&fx, draft, 1.5, 1.0));
    });
    assert!(broken.contains(&Violation {
        id: Some(made.unwrap().any()),
        rule: Rule::NothingBeneath
    }));
    // Over the cut between two clips, as the corpus's dissolve: it
    // stands.
    let a = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    let b = place(&mut fx.doc, cut(fx.low, 2.0, 2.0, fx.video));
    let mut made = None;
    fx.doc = apply(&fx.doc, |draft| {
        made = Some(transition_over(&fx, draft, 1.5, 1.0));
    });
    let dissolve = made.unwrap();
    // Taking one clip away leaves it over the other.
    apply(&fx.doc, |draft| {
        draft.items.remove(a).unwrap();
    });
    // Taking both away, or moving both from under it, leaves it over
    // nothing.
    assert_refused(&fx.doc, dissolve, Rule::NothingBeneath, |draft| {
        draft.items.remove(a).unwrap();
        draft.items.remove(b).unwrap();
    });
    assert_refused(&fx.doc, dissolve, Rule::NothingBeneath, |draft| {
        draft.items.update(a, |item| item.start = at(5.0)).unwrap();
        draft.items.update(b, |item| item.start = at(7.0)).unwrap();
    });
    // So does putting its lane under theirs.
    assert_refused(&fx.doc, dissolve, Rule::NothingBeneath, |draft| {
        draft
            .lanes
            .update(fx.high, |lane| lane.order = OrderKey::parse("1").unwrap())
            .unwrap();
    });
    // A transition is not something for another to be over.
    let mut stacked = None;
    let broken = refused(&fx.doc, |draft| {
        let top: LaneId = draft.mint().unwrap();
        draft
            .lanes
            .insert(top, Lane::new(fx.seq, OrderKey::parse("t").unwrap()))
            .unwrap();
        let package: EffectId = draft.mint().unwrap();
        let id: ItemId = draft.mint().unwrap();
        let mut item = Item::new(
            top,
            at(1.5),
            len(1.0),
            Content::Transition(Effect::new(package, "concat.push")),
        );
        item.name = "Push".into();
        draft.items.insert(id, item).unwrap();
        draft.items.remove(a).unwrap();
        draft.items.remove(b).unwrap();
        stacked = Some(id);
    });
    assert!(broken.contains(&Violation {
        id: Some(stacked.unwrap().any()),
        rule: Rule::NothingBeneath
    }));
}

#[test]
fn a_bad_run_of_words_is_refused() {
    let mut fx = fixture();
    let mut title = Item::new(fx.high, at(0.0), len(2.0), Content::Title(Title::default()));
    title.name = "Your text".into();
    let clip = place(&mut fx.doc, title);
    let run = |start, end| TextRun {
        start,
        end,
        color: Some(Color::rgb(255, 59, 48)),
        ..TextRun::default()
    };
    // "Your text" is nine characters.
    for (runs, bad) in [
        (vec![run(2, 2)], 0),
        (vec![run(0, 10)], 0),
        (vec![run(0, 4), run(3, 6)], 1),
        (vec![run(5, 6), run(0, 4)], 1),
    ] {
        assert_refused(&fx.doc, clip, Rule::BadRun { index: bad }, |draft| {
            draft
                .items
                .update(clip, |item| title_of(item).runs = runs)
                .unwrap();
        });
    }
    // Touching runs, and the corpus's first four letters, stand.
    apply(&fx.doc, |draft| {
        draft
            .items
            .update(clip, |item| {
                title_of(item).runs = vec![run(0, 4), run(4, 9)]
            })
            .unwrap();
    });
}

#[test]
fn a_link_to_itself_is_refused() {
    let mut fx = fixture();
    let (_, sound) = detached(&mut fx);
    assert_refused(&fx.doc, sound, Rule::LinkToSelf, |draft| {
        draft
            .items
            .update(sound, |item| item.link.as_mut().unwrap().to = sound)
            .unwrap();
    });
}

#[test]
fn a_link_across_sequences_is_refused() {
    let mut fx = fixture();
    let (picture, sound) = detached(&mut fx);
    assert_refused(
        &fx.doc,
        sound,
        Rule::LinkAcrossSequences { to: picture },
        |draft| {
            let other: SequenceId = draft.mint().unwrap();
            draft
                .sequences
                .insert(
                    other,
                    Sequence::new("Other", concat_doc::model::Frame::hd()),
                )
                .unwrap();
            let lane: LaneId = draft.mint().unwrap();
            draft
                .lanes
                .insert(lane, Lane::new(other, OrderKey::middle()))
                .unwrap();
            draft.items.update(sound, |item| item.lane = lane).unwrap();
        },
    );
}

#[test]
fn a_sound_tied_to_another_file_is_refused() {
    let mut fx = fixture();
    let (picture, sound) = detached(&mut fx);
    // The sound of the song, said to be the video's.
    assert_refused(
        &fx.doc,
        sound,
        Rule::LinkMismatch { to: picture },
        |draft| {
            draft
                .items
                .update(sound, |item| {
                    item.content = Content::Media(MediaCut {
                        take: Take::Sound,
                        ..MediaCut::whole(fx.song)
                    })
                })
                .unwrap();
        },
    );
    // The whole of the file, not its sound alone.
    assert_refused(
        &fx.doc,
        sound,
        Rule::LinkMismatch { to: picture },
        |draft| {
            draft
                .items
                .update(sound, |item| {
                    item.content = Content::Media(MediaCut::whole(fx.video))
                })
                .unwrap();
        },
    );
    // The picture re-pointed at another file puts the sound in question.
    assert_refused(
        &fx.doc,
        sound,
        Rule::LinkMismatch { to: picture },
        |draft| {
            draft
                .items
                .update(picture, |item| {
                    item.content = Content::Media(MediaCut::whole(fx.still))
                })
                .unwrap();
        },
    );
}

#[test]
fn a_sound_tied_to_a_sound_is_refused() {
    let mut fx = fixture();
    let (_, sound) = detached(&mut fx);
    let mut made = None;
    let broken = refused(&fx.doc, |draft| {
        let lane: LaneId = draft.mint().unwrap();
        draft
            .lanes
            .insert(lane, Lane::new(fx.seq, OrderKey::parse("z").unwrap()))
            .unwrap();
        let mut again = cut(lane, 0.0, 2.0, fx.video);
        again.content = Content::Media(MediaCut {
            take: Take::Sound,
            ..MediaCut::whole(fx.video)
        });
        again.link = Some(Link {
            to: sound,
            kind: LinkKind::SoundOf,
        });
        let id: ItemId = draft.mint().unwrap();
        draft.items.insert(id, again).unwrap();
        made = Some(id);
    });
    assert!(broken.contains(&Violation {
        id: Some(made.unwrap().any()),
        rule: Rule::LinkChain { to: sound }
    }));
}

#[test]
fn an_effect_id_used_twice_is_refused() {
    let mut fx = fixture();
    let mut ids = None;
    fx.doc = apply(&fx.doc, |draft| {
        let effect: EffectId = draft.mint().unwrap();
        let mut first = cut(fx.low, 0.0, 1.0, fx.video);
        first
            .effects
            .push(Entry::Known(Effect::new(effect, "concat.mono")));
        let a: ItemId = draft.mint().unwrap();
        draft.items.insert(a, first).unwrap();
        ids = Some((effect, a));
    });
    let (effect, a) = ids.unwrap();
    // Copied onto a second item without a new id.
    let mut made = None;
    let broken = refused(&fx.doc, |draft| {
        let mut second = cut(fx.low, 2.0, 1.0, fx.video);
        second
            .effects
            .push(Entry::Known(Effect::new(effect, "concat.mono")));
        let b: ItemId = draft.mint().unwrap();
        draft.items.insert(b, second).unwrap();
        made = Some(b);
    });
    let b = made.unwrap();
    assert!(broken.contains(&Violation {
        id: Some(effect.any()),
        rule: Rule::IdTwice { with: a.any() }
    }));
    // Named once, by the item that changed; the other did not.
    assert!(
        !broken
            .iter()
            .any(|v| v.rule == Rule::IdTwice { with: b.any() })
    );
    // Twice in one item: as a filter too.
    assert_refused(&fx.doc, effect, Rule::IdTwice { with: a.any() }, |draft| {
        draft
            .items
            .update(a, |item| {
                item.filters
                    .push(Entry::Known(Effect::new(effect, "concat.echo")))
            })
            .unwrap();
    });
    // A marker copied from an item onto its sequence keeps its id only
    // by mistake: the item and the sequence would both hold it.
    let mut made = None;
    let doc = apply(&fx.doc, |draft| {
        let id: MarkerId = draft.mint().unwrap();
        let marker = Marker {
            id,
            at: at(0.5),
            len: TickDur::ZERO,
            name: "Here".into(),
            color: None,
            residual: Residual::default(),
        };
        draft
            .items
            .update(a, |item| item.markers.push(Entry::Known(marker)))
            .unwrap();
        made = Some(id);
    });
    let marker = made.unwrap();
    assert_refused(&doc, marker, Rule::IdTwice { with: a.any() }, |draft| {
        let copy = draft.items.get(a).unwrap().markers[0].clone();
        draft
            .sequences
            .update(fx.seq, |s| s.markers.push(copy))
            .unwrap();
    });
}

#[test]
fn a_nested_id_the_mint_never_made_is_refused() {
    let mut fx = fixture();
    let clip = place(&mut fx.doc, cut(fx.low, 0.0, 1.0, fx.video));
    let made_up: MaskId = "mk900".parse().unwrap();
    let result = commit(&fx.doc, |draft| {
        draft
            .items
            .update(clip, |item| {
                item.masks.push(Entry::Known(Mask::circle(made_up)))
            })
            .unwrap();
    });
    match result {
        Err(Rejected::Unminted(ids)) => assert_eq!(ids, vec![made_up.any()]),
        other => panic!("{:?}", other.map(|_| ())),
    }
}

// --- 46 to 48. View and fonts ---------------------------------------------------

#[test]
fn a_sequence_in_two_tabs_is_refused() {
    let fx = fixture();
    assert_refused(&fx.doc, fx.seq, Rule::TabTwice, |draft| {
        draft.view.tabs.push(fx.seq);
    });
}

#[test]
fn a_document_with_sequences_shows_one() {
    let fx = fixture();
    assert_refused(&fx.doc, fx.seq, Rule::NothingShowing, |draft| {
        draft.view.showing = None;
    });
    // Taking the showing sequence away moves the view in the same change.
    let mut second = None;
    let doc = apply(&fx.doc, |draft| {
        let other: SequenceId = draft.mint().unwrap();
        draft
            .sequences
            .insert(other, Sequence::new("Two", concat_doc::model::Frame::hd()))
            .unwrap();
        draft.view.tabs.push(other);
        second = Some(other);
    });
    let other = second.unwrap();
    assert_refused(
        &doc,
        other,
        Rule::Dangling {
            what: "view.tabs",
            to: other.any(),
        },
        |draft| {
            draft.sequences.remove(other).unwrap();
        },
    );
}

#[test]
fn a_font_added_twice_is_refused() {
    let fx = fixture();
    let font = Font {
        family: "Inter".into(),
        path: "/fonts/Inter.ttf".into(),
    };
    let broken = refused(&fx.doc, |draft| {
        draft.fonts.push(font.clone());
        draft.fonts.push(Font {
            family: "Inter Two".into(),
            ..font.clone()
        });
    });
    assert_eq!(
        broken,
        vec![Violation {
            id: None,
            rule: Rule::FontTwice {
                path: "/fonts/Inter.ttf".into()
            }
        }]
    );
}

// --- The index -------------------------------------------------------------------

#[test]
fn what_is_at_a_tick_is_one_lookup_and_follows_every_edit() {
    let mut fx = fixture();
    let a = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    let b = place(&mut fx.doc, cut(fx.low, 3.0, 2.0, fx.video));
    let index = fx.doc.items.index();
    assert_eq!(index.at(fx.low, at(0.0)), Some(a));
    assert_eq!(index.at(fx.low, at(1.999)), Some(a));
    assert_eq!(index.at(fx.low, at(2.0)), None);
    assert_eq!(index.at(fx.low, at(2.5)), None);
    assert_eq!(index.at(fx.low, at(3.0)), Some(b));
    assert_eq!(index.at(fx.low, at(-1.0)), None);
    assert_eq!(index.at(fx.high, at(1.0)), None);
    let span: Vec<ItemId> = index
        .overlapping(fx.low, at(1.0), at(4.0))
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(span, vec![a, b]);
    // The snapshot before an edit keeps its own index.
    let before = fx.doc.clone();
    fx.doc = apply(&fx.doc, |draft| {
        draft.items.update(a, |item| item.lane = fx.high).unwrap();
    });
    assert_eq!(fx.doc.items.index().at(fx.low, at(1.0)), None);
    assert_eq!(fx.doc.items.index().at(fx.high, at(1.0)), Some(a));
    assert_eq!(before.items.index().at(fx.low, at(1.0)), Some(a));
    let used: Vec<ItemId> = fx.doc.items.index().using_media(fx.video).collect();
    assert_eq!(used, vec![a, b]);
    // And the lanes' order.
    let stack: Vec<LaneId> = fx
        .doc
        .lanes
        .index()
        .of(fx.seq)
        .map(|(lane, _)| lane)
        .collect();
    assert_eq!(stack, vec![fx.low, fx.high]);
}

#[test]
fn a_split_and_a_merge_give_back_the_item_to_the_tick() {
    // Trim and split move the window over content time; the keys and the
    // time map stay. A sped-up clip split at a third, then joined.
    let mut fx = fixture();
    let mut whole = cut(fx.low, 0.0, 3.0, fx.video);
    whole.content = with_time(
        &fx,
        TimeMap::Linear {
            src_in: at(1.0),
            speed: Speed::new(2, 1).unwrap(),
        },
    );
    whole.picture.opacity.keys = vec![Key::linear(at(0.0), 0.0), Key::linear(at(3.0), 1.0)];
    let clip = place(&mut fx.doc, whole.clone());
    let mut tail_id = None;
    let split = apply(&fx.doc, |draft| {
        let tail: ItemId = draft.mint().unwrap();
        let mut back = whole.clone();
        back.start = at(1.0);
        back.offset = at(1.0);
        back.len = len(2.0);
        draft.items.insert(tail, back).unwrap();
        draft
            .items
            .update(clip, |item| item.len = len(1.0))
            .unwrap();
        tail_id = Some(tail);
    });
    let tail = tail_id.unwrap();
    let head = split.items.get(clip).unwrap();
    let back = split.items.get(tail).unwrap();
    // The head's last content tick and the tail's first show the same
    // source: the cut is continuous.
    if let (Content::Media(h), Content::Media(t)) = (&head.content, &back.content) {
        assert_eq!(
            h.time.source_at(head.window_end().unwrap()),
            t.time.source_at(back.offset)
        );
    }
    let merged = apply(&split, |draft| {
        draft.items.remove(tail).unwrap();
        draft
            .items
            .update(clip, |item| item.len = len(3.0))
            .unwrap();
    });
    assert_eq!(**merged.items.get(clip).unwrap(), whole);
    assert!(merged.same_content(&fx.doc));
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A document's file: written as the format says, read back as it was,
//! and the same bytes when saved again; what a later build wrote kept;
//! what breaks a rule set aside and said; what is no project refused.

mod edits;
mod fixture;

use concat_doc::file::{self, Note, Refused};
use concat_doc::model::{
    Animated, CaptionLook, Content, Cover, Font, Item, ItemId, Lane, LaneId, MediaCut, MediaId,
    Model, OrderKey, Sequence, SequenceId,
};
use concat_doc::{Document, Origin};
use edits::*;
use fixture::*;
use proptest::prelude::*;

/// Opens a file that must open, with nothing to say.
fn open(text: &str) -> Document<Model> {
    let opened = file::read(text.as_bytes()).unwrap_or_else(|why| panic!("{why}\n{text}"));
    assert_eq!(opened.notes, [], "{text}");
    opened.document
}

/// Opens a file that must open, and what it said.
fn open_saying(text: &str) -> (Document<Model>, Vec<String>) {
    let opened = file::read(text.as_bytes()).unwrap_or_else(|why| panic!("{why}\n{text}"));
    let notes = opened.notes.iter().map(Note::to_string).collect();
    (opened.document, notes)
}

/// The small project of the format's documentation.
fn small() -> Document<Model> {
    let doc: Document<Model> = Document::new();
    apply(&doc, |draft| {
        let seq: SequenceId = draft.mint().unwrap();
        draft
            .sequences
            .insert(
                seq,
                Sequence::new("Timeline 1", concat_doc::model::Frame::hd()),
            )
            .unwrap();
        draft.view.showing = Some(seq);
        draft.view.tabs.push(seq);
        let lane: LaneId = draft.mint().unwrap();
        draft
            .lanes
            .insert(lane, Lane::new(seq, OrderKey::middle()))
            .unwrap();
        let media: MediaId = draft.mint().unwrap();
        let mut video = video_file();
        video.size = concat_doc::model::Size::new(1920, 1080);
        video.streams.clear();
        draft.media.insert(media, video).unwrap();
        let mut clip = Item::new(
            lane,
            at(0.0),
            len(2.0),
            Content::Media(MediaCut::whole(media)),
        );
        clip.sound.volume = Animated::fixed(0.5);
        let id: ItemId = draft.mint().unwrap();
        draft.items.insert(id, clip).unwrap();
    })
}

#[test]
fn a_small_project_is_written_as_the_format_says() {
    let text = file::write(&small());
    assert_eq!(
        text,
        r#"{"concat-project":1,
"mint":{"0":4},
"view":{"showing":"tl1","tabs":["tl1"]},
"media":{
  "m3":{"path":"/footage/a.mp4","name":"a.mp4","length":7056000000,"rate":{"num":30,"den":1},"size":{"width":1920,"height":1080},"has_sound":true}
},
"sequences":{
  "tl1":{"name":"Timeline 1","frame":{"width":1920,"height":1080,"rate":{"num":30,"den":1}}}
},
"lanes":{
  "t2":{"sequence":"tl1","order":"i"}
},
"items":{
  "c4":{"lane":"t2","start":0,"len":1411200000,"content":{"media":{"media":"m3"}},"sound":{"volume":0.5}}
}
}
"#
    );
    let back = open(&text);
    assert_eq!(back.tables(), small().tables());
    assert_eq!(back.mint(), small().mint());
}

#[test]
fn an_empty_document_is_a_few_lines() {
    let text = file::write(&Document::new());
    assert_eq!(
        text,
        "{\"concat-project\":1,\n\"mint\":{},\n\"view\":{},\n\"media\":{},\n\"sequences\":{},\n\"lanes\":{},\n\"items\":{}\n}\n"
    );
    assert_eq!(open(&text).tables(), &Model::default());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn every_document_reads_back_as_it_was_and_saves_to_the_same_bytes(
        ops in prop::collection::vec(op(), 1..40),
    ) {
        let mut doc = start();
        for op in &ops {
            let mut draft = doc.begin("Random", Origin(0));
            run(&mut draft, op);
            if let Ok(commit) = draft.commit() {
                doc = commit.into_document();
            }
        }
        let first = file::write(&doc);
        let opened = file::read(first.as_bytes());
        prop_assert!(opened.is_ok(), "{:?}\n{}", opened.err(), first);
        let opened = opened.unwrap();
        prop_assert_eq!(&opened.notes, &Vec::<Note>::new());
        prop_assert_eq!(opened.document.tables(), doc.tables(), "{}", first);
        prop_assert_eq!(opened.document.mint(), doc.mint());
        let second = file::write(&opened.document);
        prop_assert_eq!(first, second);
    }
}

#[test]
fn a_file_that_is_no_project_is_refused_and_said_so() {
    let refused = |bytes: &[u8]| file::read(bytes).expect_err("refused");
    assert_eq!(refused(b"\xff\xfe"), Refused::NotText);
    assert!(matches!(
        refused(b"{\"concat-project\":1,"),
        Refused::NotJson(_)
    ));
    assert_eq!(refused(b"[]"), Refused::NotAProject);
    assert_eq!(refused(b"{}"), Refused::NotAProject);
    assert_eq!(refused(b"{\"concat-project\":\"1\"}"), Refused::NotAProject);
    // A project of today's earlier format.
    assert_eq!(
        refused(br#"{"concat":"0.1.0","version":2,"timelines":[],"clips":[]}"#),
        Refused::Earlier
    );
    assert_eq!(refused(br#"{"concat-project":2}"#), Refused::Later(2));
    assert_eq!(
        refused(br#"{"concat-project":1,"needs":["particles","rigs"]}"#),
        Refused::Needs(vec!["particles".into(), "rigs".into()])
    );
    let said = refused(br#"{"concat-project":1,"needs":["particles"]}"#).to_string();
    assert_eq!(
        said,
        "this project needs a later Concat: it uses \"particles\", which this one does not have"
    );
    assert!(matches!(
        refused(br#"{"concat-project":1,"items":{},"items":{}}"#),
        Refused::Damaged(_)
    ));
    // What it already has is no need.
    open(r#"{"concat-project":1,"needs":[]}"#);
}

#[test]
fn a_caption_look_with_words_has_them_taken_out_and_said() {
    let fx = fixture();
    let doc = apply(&fx.doc, |draft| {
        draft
            .lanes
            .update(fx.high, |lane| lane.captions = Some(CaptionLook::default()))
            .unwrap();
    });
    let text = file::write(&doc).replace(
        "\"captions\":{}",
        "\"captions\":{\"title\":{\"content\":\"Your text\"}}",
    );
    assert!(text.contains("Your text"), "{text}");
    let (back, notes) = open_saying(&text);
    assert_eq!(back.tables(), doc.tables());
    assert_eq!(
        notes,
        [format!(
            "lane {}: its caption look had words of its own, which were taken out",
            fx.high
        )]
    );
}

#[test]
fn a_font_added_twice_and_a_tab_of_nothing_are_set_aside_and_written_back() {
    let fx = fixture();
    let doc = apply(&fx.doc, |draft| {
        draft.fonts.push(Font {
            family: "Inter".into(),
            path: "/fonts/inter.ttf".into(),
        });
    });
    let text = file::write(&doc)
        .replace(
            "\"path\":\"/fonts/inter.ttf\"}]",
            "\"path\":\"/fonts/inter.ttf\"},{\"family\":\"Inter 2\",\"path\":\"/fonts/inter.ttf\"}]",
        )
        .replace(
            &format!("\"tabs\":[\"{}\"]", fx.seq),
            &format!("\"tabs\":[\"{}\",\"tl999\"]", fx.seq),
        );
    let (back, notes) = open_saying(&text);
    assert_eq!(back.fonts, doc.fonts);
    assert_eq!(back.view, doc.view);
    assert_eq!(back.held.len(), 2);
    assert_eq!(
        notes,
        [
            "the fonts: a font is kept as it was and not opened: the font file /fonts/inter.ttf is added twice",
            "the view: a tab is kept as it was and not opened: `view.tabs` names tl999, which is not in the file or was set aside",
            // The tab set aside names an id: it is never made.
            "the count new ids are made from was below ids the file holds; it now goes on past them",
        ]
    );
    assert_eq!(back.mint().issued(concat_doc::Writer::LOCAL), 999);
    // Written back where they came from, and set aside again the same.
    let again = file::write(&back);
    assert!(
        again.contains("{\"family\":\"Inter 2\",\"path\":\"/fonts/inter.ttf\"}"),
        "{again}"
    );
    assert!(again.contains("\"tl999\""), "{again}");
    let (twice, _) = open_saying(&again);
    assert_eq!(file::write(&twice), again);
}

#[test]
fn of_two_items_that_overlap_the_later_is_set_aside_and_its_dependents_with_it() {
    let mut fx = fixture();
    let first = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    let text = file::write(&fx.doc);
    // The same clip again under a later id, and a sound tied to it.
    let row = text
        .lines()
        .find(|line| line.trim_start().starts_with(&format!("\"{first}\"")))
        .unwrap()
        .trim_end_matches(',')
        .to_owned();
    let copy = row.replacen(&format!("\"{first}\""), "\"c90\"", 1);
    let tied = format!(
        "  \"c91\":{{\"lane\":\"{}\",\"start\":0,\"len\":705600000,\"content\":{{\"media\":{{\"media\":\"{}\",\"take\":\"sound\"}}}},\"link\":{{\"to\":\"c90\",\"kind\":\"sound_of\"}}}}",
        fx.high, fx.video
    );
    let damaged = text.replace(&row, &format!("{row},\n{copy},\n{tied}"));
    let (back, notes) = open_saying(&damaged);
    assert_eq!(back.tables(), &fx.doc.tables().clone().with_held(&back));
    assert_eq!(
        notes,
        [
            "the count new ids are made from was below ids the file holds; it now goes on past them".to_owned(),
            format!("item c90: kept as it was and not opened: it overlaps {first} on its lane"),
            "item c91: kept as it was and not opened: `link` names c90, which is not in the file or was set aside".to_owned(),
        ]
    );
    // The count goes on past both.
    assert!(back.mint().issued(concat_doc::Writer::LOCAL) >= 91);
    let again = file::write(&back);
    assert!(
        again.contains("\"c90\":") && again.contains("\"c91\":"),
        "{again}"
    );
    let (twice, _) = open_saying(&again);
    assert_eq!(file::write(&twice), again);
}

#[test]
fn a_row_said_twice_is_kept_both_times_and_the_second_never_read() {
    let mut fx = fixture();
    let first = place(&mut fx.doc, cut(fx.low, 0.0, 2.0, fx.video));
    let text = file::write(&fx.doc);
    let row = text
        .lines()
        .find(|line| line.trim_start().starts_with(&format!("\"{first}\"")))
        .unwrap()
        .trim_end_matches(',')
        .to_owned();
    // Under one id, a row that does not read, then one that would: which
    // was meant cannot be told, so neither is opened.
    let twice = row.replacen(&format!("\"{first}\""), "\"c90\"", 1);
    let damaged = text.replace(&row, &format!("{row},\n  \"c90\":{{\"lane\":5}},\n{twice}"));
    let (back, notes) = open_saying(&damaged);
    assert_eq!(back.items.len(), 1);
    assert_eq!(back.held.len(), 2, "{notes:?}");
    let kept: Vec<&str> = back.held.values().map(|held| held.raw.as_str()).collect();
    assert!(kept.contains(&"{\"lane\":5}"), "{kept:?}");
    assert!(
        kept.iter().any(|raw| twice.ends_with(raw)),
        "{kept:?} has not {twice}"
    );
    let again = file::write(&back);
    let (twice_read, _) = open_saying(&again);
    assert_eq!(file::write(&twice_read), again);
}

/// What a model is with the held entries of another.
trait WithHeld {
    fn with_held(self, of: &Model) -> Model;
}

impl WithHeld for Model {
    fn with_held(mut self, of: &Model) -> Model {
        self.held = of.held.clone();
        self
    }
}

#[test]
fn ids_inside_text_this_build_cannot_read_are_never_made_again() {
    let fx = fixture();
    let text = file::write(&fx.doc).replace(
        "\"items\":{}",
        "\"items\":{},\n\"from_later\":{\"refs\":[\"c812\",\"fx3.40\",\"not an id\",\"c0\"]}",
    );
    let (back, notes) = open_saying(&text);
    assert_eq!(back.mint().issued(concat_doc::Writer::LOCAL), 812);
    assert_eq!(back.mint().issued(concat_doc::Writer(3)), 40);
    assert_eq!(
        notes,
        ["the count new ids are made from was below ids the file holds; it now goes on past them"]
    );
    let mut draft = back.begin("Add", Origin(0));
    let id: ItemId = draft.mint().unwrap();
    assert_eq!(id.to_string(), "c813");
}

/// A file as a later build writes it: fields, choices and records this
/// build does not know, records that need features it lacks, and what
/// was derived from records and from the whole.
const LATER: &str = include_str!("files/later.json");

#[test]
fn what_a_later_build_wrote_is_kept_and_written_back() {
    let (doc, notes) = open_saying(LATER);
    assert_eq!(
        notes,
        [
            "sequence tl1: a marker is kept as it was and not opened: it needs \"chapters\", which this build does not have",
            "item c6: an effect is kept as it was and not opened: it needs \"gpu_fx\", which this build does not have",
            "item c10: kept as it was and not opened: it needs \"rigs\", which this build does not have",
        ]
    );
    assert_eq!(doc.items.len(), 3);
    assert_eq!(doc.held.len(), 1);
    let text = file::write(&doc);
    // Every piece of it this build does not know, as it was.
    for kept in [
        r#""hdr_meta":{"max_nits":1000}"#,
        r#""derived":{"waveform":"cache/m2.peaks"}"#,
        r#""color":"rec2100_x""#,
        r##""background":{"gradient":["#000","#fff"]}"##,
        r#""style":"flag""#,
        r#"{"id":"mr31","at":705600000,"needs":["chapters"]}"#,
        r#""guides":[0.5]"#,
        r#""locked":true"#,
        r#""warp":"optical""#,
        r#""blend":"overlay_x""#,
        r#""tilt":0.5"#,
        r#""ease":"hold""#,
        r##""tint":"#ff000080""##,
        r#""derived":{"preview":"fx7.png"}"#,
        r#"{"id":"fx8","package":"later.glow","needs":["gpu_fx"],"params":{"radius":{"curve":"spring"}}}"#,
        r#""glow":1"#,
        r#""derived":{"thumb":"c6.jpg","refs":["c39"]}"#,
        r#""content":{"particles":{"count":300,"seed":"c38"}}"#,
        r#""c10":{"lane":"t5","start":705600000,"len":705600000,"content":{"media":{"media":"m3"}},"needs":["rigs"],"rig":{"bones":4}}"#,
        r#""content":"hologram""#,
        r#""source":{"path":[[0,0],[1,1]]}"#,
        r#""zoom":2.5"#,
        r#""zoom":{"level":3}"#,
        r#""derived":{"index":"c40"}"#,
        r#""mint":{"0":40,"3":2}"#,
    ] {
        assert!(text.contains(kept), "{kept} is not in\n{text}");
    }
    // A need this build meets is not written again.
    assert!(!text.contains("\"needs\":[]"), "{text}");
    // And it is a fixed point: read again, the same document and bytes.
    let again = open_saying(&text).0;
    assert_eq!(again.tables(), doc.tables());
    assert_eq!(file::write(&again), text);
}

#[test]
fn what_was_derived_from_a_record_goes_once_this_build_changes_it() {
    let (doc, _) = open_saying(LATER);
    let clip: ItemId = "c6".parse().unwrap();
    let moved = apply(&doc, |draft| {
        draft
            .items
            .update(clip, |item| item.name = "Renamed".into())
            .unwrap();
    });
    let text = file::write(&moved);
    // The clip's own is gone, and the whole document's.
    assert!(!text.contains("c6.jpg"), "{text}");
    assert!(!text.contains("\"index\":\"c40\""), "{text}");
    // What it does not know is kept; so is what was derived from records
    // it did not change, the effect inside the clip among them.
    assert!(text.contains("\"glow\":1"), "{text}");
    assert!(
        text.contains("\"derived\":{\"preview\":\"fx7.png\"}"),
        "{text}"
    );
    assert!(text.contains("cache/m2.peaks"), "{text}");
    // Changed back, it is as it was read, and what was derived holds.
    let back = apply(&moved, |draft| {
        draft
            .items
            .update(clip, |item| item.name = String::new())
            .unwrap();
    });
    assert_eq!(file::write(&back), file::write(&doc));
}

#[test]
fn every_kind_of_thing_a_document_holds_reads_back_as_it_was() {
    let doc = everything();
    let text = file::write(&doc);
    for kind in [
        "\"content\":{\"media\":",
        "\"content\":{\"nest\":",
        "\"content\":{\"title\":",
        "\"content\":{\"shape\":",
        "\"content\":\"layer\"",
        "\"content\":\"sound_layer\"",
        "\"content\":{\"transition\":",
        "\"keyed\":",
        "\"smooth\":",
        "\"cut_in\":",
        "\"link\":",
        "\"cutout\":",
        "\"stabilize\":",
        "\"motion\":",
        "\"voice\":",
        "\"runs\":",
        "\"gradient\":",
        "\"captions\":",
        "\"fonts\":",
        "\"cover\":",
        "\"markers\":",
        "\"ease\":",
    ] {
        assert!(text.contains(kind), "no {kind} in\n{text}");
    }
    let back = open(&text);
    assert_eq!(back.tables(), doc.tables());
    assert_eq!(file::write(&back), text);
}

#[test]
fn an_id_named_by_what_is_set_aside_is_never_made_again() {
    let fx = fixture();
    // An item on a lane the file does not have: set aside for it, and
    // the lane's id with it.
    let item =
        "\"items\":{\n  \"c20\":{\"lane\":\"t950\",\"start\":0,\"len\":1,\"content\":\"layer\"}\n}";
    let text = file::write(&fx.doc).replace("\"items\":{}", item);
    let (back, notes) = open_saying(&text);
    assert_eq!(back.held.len(), 1);
    assert!(
        back.mint().issued(concat_doc::Writer::LOCAL) >= 950,
        "{notes:?}"
    );
    assert_eq!(
        notes.first().map(String::as_str),
        Some(
            "the count new ids are made from was below ids the file holds; it now goes on past them"
        )
    );
}

#[test]
fn a_cover_of_a_still_is_written_as_its_file() {
    let fx = fixture();
    let doc = apply(&fx.doc, |draft| {
        draft.cover = Some(Cover::Picture { media: fx.still });
    });
    let text = file::write(&doc);
    assert!(
        text.contains(&format!("\"cover\":{{\"picture\":\"{}\"}}", fx.still)),
        "{text}"
    );
    assert_eq!(open(&text).cover, doc.cover);
    // Of a video, set aside.
    let damaged = text.replace(
        &format!("{{\"picture\":\"{}\"}}", fx.still),
        &format!("{{\"picture\":\"{}\"}}", fx.video),
    );
    let (back, notes) = open_saying(&damaged);
    assert_eq!(back.cover, None);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].starts_with("the cover: kept as it was"),
        "{notes:?}"
    );
}

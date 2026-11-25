// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Every command on its happy path: the document after it is what the
//! command says, it is one step, and undo gives back the document before
//! it exactly.

use concat_doc::commands::{
    Applied, ClipMove, Command, CommandError, Cue, Edge, Framing, ItemPatch, KeyProperty, LaneFlag,
    MaskNumber, RunStyle, SpeedPoint, placeholder, run, speed_from_decimal,
};
use concat_doc::model::{
    Background, BeatMarks, CaptionLook, Color, ColorRange, Content, Cover, Crop, Cutout,
    CutoutMode, Ease, Effect, EffectId, Frame, Item, ItemId, LaneId, LinkKind, Mask, MaskId, Media,
    MediaId, MediaKind, Model, Motion, Move, SequenceId, ShapeStyle, Size, Stabilize, Stroke,
    Subject, Take, TimeCurve, TimeMap, Title, VoiceKind, VoiceMix,
};
use concat_doc::{
    Budget, Document, Entry as Step, History, Origin, TICKS_PER_SECOND, Tick, TickDur, TickLen,
    TypedId,
};

type Hist = History<Model, Command, ()>;

fn secs(s: f64) -> Tick {
    Tick::new((s * TICKS_PER_SECOND as f64).round() as i64)
}

fn dur(s: f64) -> TickDur {
    TickDur::new((s * TICKS_PER_SECOND as f64).round() as i64)
}

fn len(s: f64) -> TickLen {
    TickLen::new((s * TICKS_PER_SECOND as f64).round() as i64).unwrap()
}

fn video(path: &str, seconds: f64) -> Media {
    Media {
        path: path.to_owned(),
        name: path.to_owned(),
        kind: MediaKind::Video,
        length: Some(len(seconds)),
        size: Size::new(1920, 1080),
        has_sound: true,
        ..Media::default()
    }
}

fn still(path: &str) -> Media {
    Media {
        path: path.to_owned(),
        name: path.to_owned(),
        kind: MediaKind::Image,
        size: Size::new(1000, 1000),
        ..Media::default()
    }
}

/// Runs a command, records it, and checks that undo gives back the
/// document before it exactly and redo the one after.
fn exec(h: &mut Hist, command: Command) -> Applied {
    try_exec(h, command).unwrap_or_else(|e| panic!("refused: {e}"))
}

fn try_exec(h: &mut Hist, command: Command) -> Result<Applied, CommandError> {
    let before = h.current().clone();
    let outcome = run(&before, &command, Origin(0))?;
    let Some(commit) = outcome.commit else {
        return Ok(outcome.applied);
    };
    let after = commit.document().clone();
    let recorded = h.record(commit, Step::new(vec![command], (), ())).unwrap();
    if recorded == concat_doc::Recorded::Step {
        let undone = h.undo().unwrap();
        assert_eq!(
            without_view(undone.document.tables()),
            without_view(before.tables()),
            "undo gives back the earlier root"
        );
        let redone = h.redo().unwrap();
        assert_eq!(
            without_view(redone.document.tables()),
            without_view(after.tables()),
            "redo gives back the later root"
        );
        // Undo keeps the view as it is; put the command's own back.
        if redone.document.view != after.view {
            let mut draft = h.current().begin("View", Origin(0));
            draft.view = after.view.clone();
            let commit = draft.commit().unwrap();
            h.record(commit, Step::new(vec![], (), ())).unwrap();
        }
    }
    Ok(outcome.applied)
}

/// The tables without the view, which no step of undo changes.
fn without_view(model: &Model) -> Model {
    let mut model = model.clone();
    model.view = Default::default();
    model
}

fn project() -> Hist {
    let mut h = History::new(Document::new(), Budget::default());
    exec(&mut h, Command::AddTimeline);
    h.clear();
    h
}

fn m(h: &Hist) -> &Model {
    h.current().tables()
}

fn showing(h: &Hist) -> SequenceId {
    m(h).view.showing.unwrap()
}

fn lanes(h: &Hist) -> Vec<LaneId> {
    m(h).lanes
        .index()
        .of(showing(h))
        .map(|(id, _)| id)
        .collect()
}

fn item(h: &Hist, id: ItemId) -> Item {
    (**m(h).items.get(id).unwrap()).clone()
}

fn created<I: TypedId>(applied: &Applied) -> I {
    applied.created.unwrap().typed().unwrap()
}

fn add_media(h: &mut Hist, media: Media) -> MediaId {
    created(&exec(h, Command::AddMedia { media }))
}

fn clip_at(h: &mut Hist, media: MediaId, start: f64) -> ItemId {
    created(&exec(
        h,
        Command::AddClipAtFirstFree {
            media,
            start: secs(start),
        },
    ))
}

fn end(i: &Item) -> Tick {
    i.start + i.len
}

// ----- The project -----

#[test]
fn a_new_timeline_has_four_lanes_and_shows() {
    let h = project();
    assert_eq!(lanes(&h).len(), 4);
    let s = m(&h).sequences.get(showing(&h)).unwrap();
    assert_eq!(s.name, "Timeline 1");
    assert_eq!(m(&h).view.tabs, vec![showing(&h)]);
}

#[test]
fn add_timeline_names_the_next_and_shows_it() {
    let mut h = project();
    let first = showing(&h);
    let second: SequenceId = created(&exec(&mut h, Command::AddTimeline));
    assert_eq!(showing(&h), second);
    assert_eq!(m(&h).sequences.get(second).unwrap().name, "Timeline 2");
    assert_eq!(m(&h).view.tabs, vec![first, second]);
}

#[test]
fn remove_timeline_shows_a_neighbour_and_keeps_one() {
    let mut h = project();
    let first = showing(&h);
    let a = add_media(&mut h, video("a", 2.0));
    let second: SequenceId = created(&exec(&mut h, Command::AddTimeline));
    clip_at(&mut h, a, 0.0);
    exec(&mut h, Command::RemoveTimeline { sequence: second });
    assert_eq!(showing(&h), first);
    assert!(!m(&h).sequences.contains(second));
    assert!(m(&h).items.is_empty());
    let refused = try_exec(&mut h, Command::RemoveTimeline { sequence: first });
    assert!(matches!(refused, Err(CommandError::Refused(_))));
}

#[test]
fn rename_trims_and_ignores_blank() {
    let mut h = project();
    let s = showing(&h);
    exec(
        &mut h,
        Command::RenameTimeline {
            sequence: s,
            name: "  Cutdown ".into(),
        },
    );
    assert_eq!(m(&h).sequences.get(s).unwrap().name, "Cutdown");
    let before = h.current().revision();
    exec(
        &mut h,
        Command::RenameTimeline {
            sequence: s,
            name: "   ".into(),
        },
    );
    assert_eq!(
        h.current().revision(),
        before,
        "nothing changed, no revision"
    );
}

#[test]
fn select_and_move_timeline_are_view_state_and_no_step() {
    let mut h = project();
    let first = showing(&h);
    let second: SequenceId = created(&exec(&mut h, Command::AddTimeline));
    let steps = h.undo_len();
    exec(&mut h, Command::SelectTimeline { sequence: first });
    assert_eq!(showing(&h), first);
    exec(
        &mut h,
        Command::MoveTimeline {
            sequence: second,
            index: 0,
        },
    );
    assert_eq!(m(&h).view.tabs, vec![second, first]);
    assert_eq!(h.undo_len(), steps);
}

#[test]
fn timeline_frame_background_and_linkage() {
    let mut h = project();
    let s = showing(&h);
    let mut frame = Frame::hd();
    frame.rate = concat_doc::Rate::FPS_60;
    exec(
        &mut h,
        Command::SetTimelineVideo {
            sequence: s,
            frame: frame.clone(),
        },
    );
    exec(
        &mut h,
        Command::SetTimelineBackground {
            sequence: s,
            background: Background::Blur { amount: 3.0 },
        },
    );
    exec(
        &mut h,
        Command::SetTimelineLinkage {
            sequence: s,
            on: false,
        },
    );
    let seq = m(&h).sequences.get(s).unwrap();
    assert_eq!(seq.frame, frame);
    assert_eq!(seq.background, Background::Blur { amount: 1.0 });
    assert!(!seq.linkage);
}

#[test]
fn cover_and_fonts() {
    let mut h = project();
    exec(
        &mut h,
        Command::SetCover {
            cover: Some(Cover::Frame { into: dur(-1.0) }),
        },
    );
    assert_eq!(
        m(&h).cover,
        Some(Cover::Frame {
            into: TickDur::ZERO
        })
    );
    exec(
        &mut h,
        Command::AddFont {
            family: "Inter".into(),
            path: "/f/inter.ttf".into(),
        },
    );
    let rev = h.current().revision();
    exec(
        &mut h,
        Command::AddFont {
            family: "Inter".into(),
            path: "/f/inter.ttf".into(),
        },
    );
    assert_eq!(h.current().revision(), rev);
    assert_eq!(m(&h).fonts.len(), 1);
    exec(
        &mut h,
        Command::RemoveFont {
            family: "Inter".into(),
        },
    );
    assert!(m(&h).fonts.is_empty());
}

// ----- The bin -----

#[test]
fn add_media_twice_is_one_file_and_answers_its_id() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let rev = h.current().revision();
    let again: MediaId = created(&exec(
        &mut h,
        Command::AddMedia {
            media: video("a", 2.0),
        },
    ));
    assert_eq!(a, again);
    assert_eq!(h.current().revision(), rev, "no new revision");
    assert_eq!(m(&h).media.len(), 1);
}

#[test]
fn remove_media_takes_its_clips_and_backgrounds() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let b = add_media(&mut h, still("b"));
    clip_at(&mut h, a, 0.0);
    let keep = clip_at(&mut h, a, 3.0);
    exec(
        &mut h,
        Command::UpdateClip {
            item: keep,
            patch: ItemPatch {
                background: Some(Some(Background::Image(b))),
                ..ItemPatch::default()
            },
        },
    );
    let s = showing(&h);
    exec(
        &mut h,
        Command::SetTimelineBackground {
            sequence: s,
            background: Background::Image(b),
        },
    );
    exec(
        &mut h,
        Command::SetCover {
            cover: Some(Cover::Picture { media: b }),
        },
    );
    exec(&mut h, Command::RemoveMedia { media: b });
    assert_eq!(m(&h).cover, None);
    assert_eq!(item(&h, keep).picture.background, None);
    assert_eq!(
        m(&h).sequences.get(showing(&h)).unwrap().background,
        Background::None
    );
    exec(&mut h, Command::RemoveMedia { media: a });
    assert!(m(&h).items.is_empty());
    assert!(m(&h).media.is_empty());
}

#[test]
fn media_settings() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    exec(
        &mut h,
        Command::UpdateMediaPath {
            media: a,
            path: "/new/a".into(),
        },
    );
    exec(
        &mut h,
        Command::SetMediaColorRange {
            media: a,
            range: Some(ColorRange::Full),
        },
    );
    exec(
        &mut h,
        Command::SetMediaBeats {
            media: a,
            marks: Some(BeatMarks::Bar),
        },
    );
    exec(
        &mut h,
        Command::SetMediaPlaceholder {
            media: a,
            placeholder: true,
        },
    );
    let file = m(&h).media.get(a).unwrap();
    assert_eq!(file.path, "/new/a");
    assert_eq!(file.range, Some(ColorRange::Full));
    assert_eq!(file.beats, Some(BeatMarks::Bar));
    assert!(file.placeholder);
}

#[test]
fn fill_slot_keeps_the_id_and_resets_the_in_point() {
    let mut h = project();
    let slot = add_media(&mut h, video("", 10.0));
    let clip = clip_at(&mut h, slot, 0.0);
    exec(
        &mut h,
        Command::TrimClip {
            item: clip,
            edge: Edge::Head,
            delta: dur(1.0),
            ripple: false,
        },
    );
    assert!(
        try_exec(
            &mut h,
            Command::FillSlot {
                media: slot,
                file: video("mine", 10.0)
            }
        )
        .is_err()
    );
    exec(
        &mut h,
        Command::SetMediaPlaceholder {
            media: slot,
            placeholder: true,
        },
    );
    exec(
        &mut h,
        Command::FillSlot {
            media: slot,
            file: video("mine", 10.0),
        },
    );
    let i = item(&h, clip);
    assert_eq!(i.name, "mine");
    let Content::Media(cut) = &i.content else {
        panic!()
    };
    assert_eq!(cut.media, slot);
    assert_eq!(cut.time.source_at(i.offset), Some(Tick::ZERO));
    assert!(!m(&h).media.get(slot).unwrap().placeholder);
}

// ----- Placing -----

#[test]
fn add_clip_at_first_free_takes_the_lowest_free_lane() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let two = clip_at(&mut h, a, 1.0);
    let three = clip_at(&mut h, a, 2.0);
    let l = lanes(&h);
    assert_eq!(item(&h, one).lane, l[0]);
    assert_eq!(item(&h, two).lane, l[1]);
    assert_eq!(item(&h, three).lane, l[0], "touching is not overlapping");
    assert_eq!(item(&h, one).len, len(2.0));
}

#[test]
fn add_clip_at_first_free_makes_a_lane_when_none_is_free() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    for _ in 0..4 {
        clip_at(&mut h, a, 0.0);
    }
    let fifth = clip_at(&mut h, a, 0.0);
    assert_eq!(lanes(&h).len(), 5);
    assert_eq!(item(&h, fifth).lane, lanes(&h)[4]);
}

#[test]
fn add_clip_on_a_taken_lane_goes_to_a_new_lane_above_it() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let first = clip_at(&mut h, a, 0.0);
    let lane = item(&h, first).lane;
    let second: ItemId = created(&exec(
        &mut h,
        Command::AddClip {
            media: a,
            lane,
            start: secs(1.0),
            ripple: false,
        },
    ));
    let l = lanes(&h);
    assert_eq!(l.len(), 5);
    assert_eq!(item(&h, second).lane, l[1], "just above the lane named");
}

#[test]
fn add_clip_with_ripple_makes_room() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let first = clip_at(&mut h, a, 0.0);
    let after = clip_at(&mut h, a, 2.0);
    let lane = item(&h, first).lane;
    let new: ItemId = created(&exec(
        &mut h,
        Command::AddClip {
            media: a,
            lane,
            start: secs(2.0),
            ripple: true,
        },
    ));
    assert_eq!(item(&h, new).start, secs(2.0));
    assert_eq!(item(&h, new).lane, lane);
    assert_eq!(item(&h, after).start, secs(4.0));
}

#[test]
fn a_still_lasts_five_seconds() {
    let mut h = project();
    let s = add_media(&mut h, still("s"));
    let c = clip_at(&mut h, s, -3.0);
    assert_eq!(item(&h, c).start, Tick::ZERO, "floored at zero");
    assert_eq!(item(&h, c).len, len(5.0));
}

#[test]
fn clips_in_sequence_are_end_to_end() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let b = add_media(&mut h, video("b", 3.0));
    let out = exec(
        &mut h,
        Command::AddClipsInSequence {
            media: vec![a, b],
            start: secs(1.0),
            lane: None,
            ripple: false,
        },
    );
    let made = out.selection.unwrap();
    assert_eq!(made.len(), 2);
    assert_eq!(item(&h, made[0]).start, secs(1.0));
    assert_eq!(item(&h, made[1]).start, secs(3.0));
    assert_eq!(item(&h, made[0]).lane, item(&h, made[1]).lane);
    let lane = lanes(&h)[2];
    let out = exec(
        &mut h,
        Command::AddClipsInSequence {
            media: vec![b, a],
            start: Tick::ZERO,
            lane: Some(lane),
            ripple: false,
        },
    );
    let made = out.selection.unwrap();
    assert_eq!(item(&h, made[1]).start, secs(3.0));
}

#[test]
fn a_title_lands_above_the_picture() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    clip_at(&mut h, a, 0.0);
    let title = Title {
        content: "Hello\nworld".into(),
        ..Title::default()
    };
    let t: ItemId = created(&exec(
        &mut h,
        Command::AddTextClip {
            start: Tick::ZERO,
            len: Some(len(2.0)),
            lane: None,
            above: true,
            offset_y: Some(9.0),
            title: Some(title),
        },
    ));
    let t = item(&h, t);
    assert_eq!(t.lane, lanes(&h)[1]);
    assert_eq!(t.name, "Hello");
    assert_eq!(t.picture.offset_y.value, 3.0);
    assert_eq!(t.len, len(2.0));
}

#[test]
fn shapes_layers_and_transitions() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 4.0));
    clip_at(&mut h, a, 0.0);
    let s: ItemId = created(&exec(
        &mut h,
        Command::AddShapeClip {
            start: Tick::ZERO,
            len: None,
            name: "Square".into(),
            style: Some(ShapeStyle::default()),
            lane: None,
            above: true,
        },
    ));
    assert!(matches!(item(&h, s).content, Content::Shape(_)));
    assert_eq!(item(&h, s).len, len(3.0));
    let layer: ItemId = created(&exec(
        &mut h,
        Command::AddLayerClip {
            start: Tick::ZERO,
            len: Some(len(1.0)),
            package: "concat.warm".into(),
            name: "Warm".into(),
            sound: false,
            lane: None,
        },
    ));
    let layer = item(&h, layer);
    assert!(matches!(layer.content, Content::Layer));
    assert_eq!(layer.effects.len(), 1);
    let sound: ItemId = created(&exec(
        &mut h,
        Command::AddLayerClip {
            start: Tick::ZERO,
            len: None,
            package: "concat.telephone".into(),
            name: "Phone".into(),
            sound: true,
            lane: None,
        },
    ));
    assert_eq!(item(&h, sound).filters.len(), 1);
    let t: ItemId = created(&exec(
        &mut h,
        Command::AddTransitionClip {
            start: secs(1.0),
            len: None,
            package: "concat.push".into(),
            name: "Push".into(),
            lane: None,
        },
    ));
    assert!(item(&h, t).is_transition());
    assert_eq!(item(&h, t).len, len(1.0));
    // Over nothing is refused, and changes nothing.
    let rev = h.current().revision();
    let refused = try_exec(
        &mut h,
        Command::AddTransitionClip {
            start: secs(30.0),
            len: None,
            package: "concat.push".into(),
            name: "Push".into(),
            lane: None,
        },
    );
    assert!(matches!(refused, Err(CommandError::Broken(_))));
    assert_eq!(h.current().revision(), rev);
}

#[test]
fn captions_go_on_a_caption_lane_in_its_look() {
    let mut h = project();
    let mut look = CaptionLook::default();
    look.title.content = "Your text".into();
    look.offset_y = 0.3;
    let cues = vec![
        Cue {
            start: secs(2.0),
            len: len(1.0),
            words: "second".into(),
        },
        Cue {
            start: secs(0.0),
            len: len(5.0),
            words: "first".into(),
        },
        Cue {
            start: secs(4.0),
            len: len(1.0),
            words: "  ".into(),
        },
    ];
    let lane: LaneId = created(&exec(
        &mut h,
        Command::PlaceCaptions {
            cues,
            look: look.clone(),
            lane: None,
        },
    ));
    assert_eq!(lanes(&h).last(), Some(&lane));
    let held = m(&h).lanes.get(lane).unwrap().captions.clone().unwrap();
    assert!(held.title.content.is_empty(), "a lane's look has no words");
    let on: Vec<_> = m(&h).items.index().on_lane(lane).collect();
    assert_eq!(on.len(), 2);
    let first = item(&h, on[0].0);
    assert_eq!(
        first.len,
        len(2.0),
        "runs into the next and ends where it begins"
    );
    assert_eq!(first.picture.offset_y.value, 0.3);
    let Content::Title(t) = &first.content else {
        panic!()
    };
    assert_eq!(t.content, "first");

    let mut restyled = CaptionLook::default();
    restyled.title.color = Color::rgb(255, 0, 0);
    restyled.offset_y = -0.2;
    exec(
        &mut h,
        Command::SetCaptionLook {
            lane,
            look: Some(restyled),
        },
    );
    let first = item(&h, on[0].0);
    let Content::Title(t) = &first.content else {
        panic!()
    };
    assert_eq!(t.content, "first");
    assert_eq!(t.color, Color::rgb(255, 0, 0));
    assert_eq!(first.picture.offset_y.value, -0.2);
    exec(&mut h, Command::SetCaptionLook { lane, look: None });
    assert!(m(&h).lanes.get(lane).unwrap().captions.is_none());
}

// ----- Moving and cutting -----

#[test]
fn moves_land_together_and_one_on_another_goes_up() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let two = clip_at(&mut h, a, 4.0);
    let l = lanes(&h);
    exec(
        &mut h,
        Command::MoveClips {
            moves: vec![
                ClipMove {
                    item: one,
                    start: secs(10.0),
                    lane: None,
                },
                ClipMove {
                    item: two,
                    start: secs(1.0),
                    lane: Some(l[2]),
                },
            ],
        },
    );
    assert_eq!(item(&h, one).start, secs(10.0));
    assert_eq!(item(&h, two).lane, l[2]);
    exec(
        &mut h,
        Command::MoveClips {
            moves: vec![ClipMove {
                item: one,
                start: secs(2.0),
                lane: Some(l[2]),
            }],
        },
    );
    assert_eq!(
        item(&h, one).lane,
        l[3],
        "the first lane above that is free"
    );
}

#[test]
fn moves_onto_a_new_lane() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let lane: LaneId = created(&exec(
        &mut h,
        Command::MoveClipsToNewTrack {
            moves: vec![ClipMove {
                item: one,
                start: secs(1.0),
                lane: None,
            }],
            bottom: true,
        },
    ));
    assert_eq!(lanes(&h)[0], lane);
    assert_eq!(item(&h, one).lane, lane);
    let rev = h.current().revision();
    exec(
        &mut h,
        Command::MoveClipsToNewTrack {
            moves: vec![ClipMove {
                item: placeholder(),
                start: Tick::ZERO,
                lane: None,
            }],
            bottom: false,
        },
    );
    assert_eq!(h.current().revision(), rev, "nothing lands: no lane");
}

#[test]
fn a_tail_trim_with_ripple_closes_up() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let two = clip_at(&mut h, a, 2.0);
    exec(
        &mut h,
        Command::TrimClip {
            item: one,
            edge: Edge::Tail,
            delta: dur(-0.5),
            ripple: true,
        },
    );
    assert_eq!(item(&h, one).len, len(1.5));
    assert_eq!(item(&h, two).start, secs(1.5));
}

#[test]
fn a_tail_trim_stops_at_the_file_and_the_next_clip() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    exec(
        &mut h,
        Command::TrimClip {
            item: one,
            edge: Edge::Tail,
            delta: dur(-1.0),
            ripple: false,
        },
    );
    let two = clip_at(&mut h, a, 1.5);
    exec(
        &mut h,
        Command::TrimClip {
            item: one,
            edge: Edge::Tail,
            delta: dur(5.0),
            ripple: false,
        },
    );
    assert_eq!(item(&h, one).len, len(1.5), "stops at the next clip");
    exec(
        &mut h,
        Command::RemoveClips {
            items: vec![two],
            ripple: false,
        },
    );
    exec(
        &mut h,
        Command::TrimClip {
            item: one,
            edge: Edge::Tail,
            delta: dur(5.0),
            ripple: false,
        },
    );
    assert_eq!(item(&h, one).len, len(2.0), "stops at the file's end");
}

#[test]
fn a_head_trim_moves_the_window_and_the_start() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 4.0));
    let one = clip_at(&mut h, a, 1.0);
    exec(
        &mut h,
        Command::TrimClip {
            item: one,
            edge: Edge::Head,
            delta: dur(1.0),
            ripple: false,
        },
    );
    let i = item(&h, one);
    assert_eq!(i.start, secs(2.0));
    assert_eq!(i.offset, secs(1.0));
    assert_eq!(i.len, len(3.0));
    exec(
        &mut h,
        Command::TrimClip {
            item: one,
            edge: Edge::Head,
            delta: dur(-5.0),
            ripple: false,
        },
    );
    let i = item(&h, one);
    assert_eq!(i.offset, Tick::ZERO, "not before the file's start");
    assert_eq!(i.start, secs(1.0));
}

#[test]
fn a_head_trim_with_ripple_keeps_the_start() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let two = clip_at(&mut h, a, 2.0);
    exec(
        &mut h,
        Command::TrimClip {
            item: one,
            edge: Edge::Head,
            delta: dur(0.5),
            ripple: true,
        },
    );
    assert_eq!(item(&h, one).start, Tick::ZERO);
    assert_eq!(item(&h, one).offset, secs(0.5));
    assert_eq!(item(&h, two).start, secs(1.5));
}

#[test]
fn split_keeps_keys_in_content_time_and_merge_gives_back_the_clip() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    exec(
        &mut h,
        Command::SetClipSpeed {
            item: one,
            speed: speed_from_decimal(2.0).unwrap(),
            ripple: false,
        },
    );
    exec(
        &mut h,
        Command::SetClipKey {
            item: one,
            property: KeyProperty::Opacity,
            at: secs(0.8),
            value: 0.5,
            ease: Ease::LINEAR,
        },
    );
    exec(
        &mut h,
        Command::UpdateClip {
            item: one,
            patch: ItemPatch {
                fade_in: Some(dur(0.1)),
                fade_out: Some(dur(0.2)),
                ..ItemPatch::default()
            },
        },
    );
    let whole = item(&h, one);
    let tail: ItemId = created(&exec(
        &mut h,
        Command::SplitClips {
            items: vec![one],
            at: secs(0.5),
        },
    ));
    let (head, t) = (item(&h, one), item(&h, tail));
    assert_eq!(head.len, len(0.5));
    assert_eq!(t.start, secs(0.5));
    assert_eq!(t.offset, secs(0.5));
    assert_eq!(t.content, head.content, "the same map");
    assert_eq!(
        t.picture.opacity.keys, head.picture.opacity.keys,
        "keys where they were"
    );
    assert_eq!(head.sound.fade_out, TickDur::ZERO);
    assert_eq!(t.sound.fade_in, TickDur::ZERO);
    exec(
        &mut h,
        Command::MergeClips {
            items: vec![tail, one],
        },
    );
    assert!(!m(&h).items.contains(tail));
    assert_eq!(item(&h, one), whole);
}

#[test]
fn a_split_that_misses_or_grazes_is_nothing() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let rev = h.current().revision();
    exec(
        &mut h,
        Command::SplitClips {
            items: vec![one],
            at: secs(5.0),
        },
    );
    exec(
        &mut h,
        Command::SplitClips {
            items: vec![one],
            at: Tick::new(1),
        },
    );
    assert_eq!(h.current().revision(), rev);
}

#[test]
fn merge_refuses_pieces_of_two_clips() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let two = clip_at(&mut h, a, 2.0);
    assert!(matches!(
        try_exec(
            &mut h,
            Command::MergeClips {
                items: vec![one, two]
            }
        ),
        Err(CommandError::Refused(_))
    ));
}

#[test]
fn duplicate_lays_a_copy_after_with_new_ids() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let blocker = clip_at(&mut h, a, 2.0);
    let effect = Effect::new(placeholder(), "concat.mono");
    exec(
        &mut h,
        Command::UpdateClip {
            item: one,
            patch: ItemPatch {
                effects: Some(vec![effect]),
                ..ItemPatch::default()
            },
        },
    );
    let copy: ItemId = created(&exec(&mut h, Command::DuplicateClips { items: vec![one] }));
    let (o, c) = (item(&h, one), item(&h, copy));
    assert_eq!(
        c.start,
        end(&item(&h, blocker)),
        "the first gap after its end"
    );
    assert_eq!(c.lane, o.lane);
    assert_ne!(c.effects, o.effects, "its own effect ids");
    assert_eq!(c.content, o.content);
}

#[test]
fn remove_with_ripple_closes_only_its_lane() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let two = clip_at(&mut h, a, 2.0);
    let other = clip_at(&mut h, a, 3.0);
    assert_ne!(item(&h, other).lane, item(&h, one).lane);
    exec(
        &mut h,
        Command::RemoveClips {
            items: vec![one],
            ripple: true,
        },
    );
    assert_eq!(item(&h, two).start, Tick::ZERO);
    assert_eq!(item(&h, other).start, secs(3.0));
}

#[test]
fn removing_what_a_transition_is_over_takes_the_transition() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let t: ItemId = created(&exec(
        &mut h,
        Command::AddTransitionClip {
            start: secs(0.5),
            len: None,
            package: "concat.push".into(),
            name: "Push".into(),
            lane: None,
        },
    ));
    exec(
        &mut h,
        Command::RemoveClips {
            items: vec![one],
            ripple: false,
        },
    );
    assert!(!m(&h).items.contains(t));
}

#[test]
fn freeze_holds_a_still_and_moves_what_follows() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 4.0));
    let one = clip_at(&mut h, a, 0.0);
    let after = clip_at(&mut h, a, 4.0);
    assert!(
        try_exec(
            &mut h,
            Command::FreezeFrame {
                item: one,
                at: secs(1.0),
                len: None,
                still: None
            }
        )
        .is_err()
    );
    let frozen: ItemId = created(&exec(
        &mut h,
        Command::FreezeFrame {
            item: one,
            at: secs(1.0),
            len: Some(len(2.0)),
            still: Some(still("frame.jpg")),
        },
    ));
    let f = item(&h, frozen);
    assert_eq!(f.start, secs(1.0));
    assert_eq!(f.len, len(2.0));
    assert_eq!(item(&h, one).len, len(1.0));
    assert_eq!(item(&h, after).start, secs(6.0));
    let on: Vec<_> = m(&h).items.index().on_lane(f.lane).collect();
    assert_eq!(on.len(), 4, "head, freeze, tail, the next");
    assert_eq!(on[2].1, secs(3.0));
}

#[test]
fn consolidate_makes_one_clip_over_the_span() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let two = clip_at(&mut h, a, 3.0);
    exec(
        &mut h,
        Command::ConsolidateClips {
            items: vec![two, one],
            file: video("render", 5.0),
        },
    );
    assert!(!m(&h).items.contains(two));
    let i = item(&h, one);
    assert_eq!(i.len, len(5.0));
    assert_eq!(i.name, "render");
}

#[test]
fn replace_media_and_enhance() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 4.0));
    let one = clip_at(&mut h, a, 0.0);
    exec(
        &mut h,
        Command::TrimClip {
            item: one,
            edge: Edge::Tail,
            delta: dur(-2.0),
            ripple: false,
        },
    );
    exec(
        &mut h,
        Command::ReplaceClipMedia {
            item: one,
            file: video("a-enhanced", 4.0),
            enhanced: true,
            copy_start: TickDur::ZERO,
            source_start: None,
        },
    );
    let Content::Media(cut) = &item(&h, one).content else {
        panic!()
    };
    assert_eq!(cut.media, a);
    let e = cut.enhanced.clone().unwrap();
    assert!(e.on);
    exec(
        &mut h,
        Command::SetClipEnhanced {
            item: one,
            on: false,
        },
    );
    let Content::Media(cut) = &item(&h, one).content else {
        panic!()
    };
    assert!(!cut.enhanced.as_ref().unwrap().on);
    exec(
        &mut h,
        Command::ReplaceClipMedia {
            item: one,
            file: video("a-reversed", 2.0),
            enhanced: false,
            copy_start: TickDur::ZERO,
            source_start: Some(Tick::ZERO),
        },
    );
    let i = item(&h, one);
    let Content::Media(cut) = &i.content else {
        panic!()
    };
    assert_ne!(cut.media, a);
    assert!(cut.enhanced.is_none());
    assert_eq!(cut.time.source_at(i.offset), Some(Tick::ZERO));
    assert_eq!(i.len, len(2.0));
}

#[test]
fn detach_and_reattach() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let filter = Effect::new(placeholder(), "concat.echo");
    exec(
        &mut h,
        Command::UpdateClip {
            item: one,
            patch: ItemPatch {
                filters: Some(vec![filter]),
                ..ItemPatch::default()
            },
        },
    );
    let sound: ItemId = created(&exec(&mut h, Command::DetachAudio { item: one }));
    let (v, s) = (item(&h, one), item(&h, sound));
    assert!(v.sound.muted);
    assert!(v.filters.is_empty());
    assert_eq!(s.filters.len(), 1);
    assert_eq!(
        s.link.as_ref().map(|l| (l.to, l.kind.clone())),
        Some((one, LinkKind::SoundOf))
    );
    let Content::Media(cut) = &s.content else {
        panic!()
    };
    assert_eq!(cut.take, Take::Sound);
    assert_eq!((s.start, s.len), (v.start, v.len));
    let rev = h.current().revision();
    exec(&mut h, Command::DetachAudio { item: one });
    assert_eq!(h.current().revision(), rev, "already detached");
    exec(&mut h, Command::ReattachAudio { item: sound });
    assert!(!m(&h).items.contains(sound));
    let v = item(&h, one);
    assert!(!v.sound.muted);
    assert_eq!(v.filters.len(), 1);
}

// ----- Speed -----

#[test]
fn speed_with_ripple_closes_up() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let two = clip_at(&mut h, a, 2.0);
    exec(
        &mut h,
        Command::SetClipKey {
            item: one,
            property: KeyProperty::Scale,
            at: secs(1.0),
            value: 2.0,
            ease: Ease::LINEAR,
        },
    );
    exec(
        &mut h,
        Command::SetClipSpeed {
            item: one,
            speed: speed_from_decimal(2.0).unwrap(),
            ripple: true,
        },
    );
    let i = item(&h, one);
    assert_eq!(i.len, len(1.0));
    assert_eq!(item(&h, two).start, secs(1.0));
    assert_eq!(
        i.picture.scale.keys[0].at,
        secs(0.5),
        "keys keep their share"
    );
    let Content::Media(cut) = &i.content else {
        panic!()
    };
    assert_eq!(cut.time.source_at(i.offset + i.len), Some(secs(2.0)));
}

#[test]
fn slowing_down_without_ripple_makes_room() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let two = clip_at(&mut h, a, 2.0);
    exec(
        &mut h,
        Command::SetClipSpeed {
            item: one,
            speed: speed_from_decimal(0.5).unwrap(),
            ripple: false,
        },
    );
    assert_eq!(item(&h, one).len, len(4.0));
    assert_eq!(item(&h, two).start, secs(4.0));
}

#[test]
fn a_straight_ramp_is_exact() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 4.0));
    let one = clip_at(&mut h, a, 0.0);
    exec(
        &mut h,
        Command::SetClipSpeedCurve {
            item: one,
            curve: Some(vec![
                SpeedPoint {
                    at: 0.0,
                    speed: 1.0,
                    slope: None,
                },
                SpeedPoint {
                    at: 1.0,
                    speed: 3.0,
                    slope: None,
                },
            ]),
            ripple: false,
        },
    );
    let i = item(&h, one);
    assert_eq!(i.len, len(2.0), "a mean of two");
    let Content::Media(cut) = &i.content else {
        panic!()
    };
    let TimeMap::Keyed { first, rest } = &cut.time else {
        panic!("keyed")
    };
    assert_eq!(first.source, Tick::ZERO);
    assert_eq!(rest.len(), 1);
    assert!(matches!(rest[0].0, TimeCurve::Smooth { .. }));
    assert_eq!(rest[0].1.source, secs(4.0));
    // Back to a constant pace at the mean.
    exec(
        &mut h,
        Command::SetClipSpeedCurve {
            item: one,
            curve: None,
            ripple: false,
        },
    );
    let i = item(&h, one);
    let Content::Media(cut) = &i.content else {
        panic!()
    };
    assert!(matches!(cut.time, TimeMap::Linear { .. }));
    assert_eq!(i.len, len(2.0));
}

#[test]
fn a_smooth_curve_is_fitted_and_valid() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 10.0));
    let one = clip_at(&mut h, a, 0.0);
    exec(
        &mut h,
        Command::SetClipSpeedCurve {
            item: one,
            curve: Some(vec![
                SpeedPoint {
                    at: 0.0,
                    speed: 0.5,
                    slope: Some(0.0),
                },
                SpeedPoint {
                    at: 0.5,
                    speed: 4.0,
                    slope: Some(0.0),
                },
                SpeedPoint {
                    at: 1.0,
                    speed: 1.0,
                    slope: Some(0.0),
                },
            ]),
            ripple: false,
        },
    );
    let i = item(&h, one);
    let Content::Media(cut) = &i.content else {
        panic!()
    };
    assert!(cut.time.points().len() > 3);
    // A split of a curved clip shows what the whole did.
    let tail: ItemId = created(&exec(
        &mut h,
        Command::SplitClips {
            items: vec![one],
            at: secs(1.0),
        },
    ));
    exec(
        &mut h,
        Command::MergeClips {
            items: vec![one, tail],
        },
    );
    assert_eq!(item(&h, one), i);
}

// ----- Keys -----

#[test]
fn clip_keys_replace_within_a_hair_and_clear() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    exec(
        &mut h,
        Command::SetClipKey {
            item: one,
            property: KeyProperty::Opacity,
            at: secs(1.0),
            value: 2.0,
            ease: Ease::LINEAR,
        },
    );
    exec(
        &mut h,
        Command::SetClipKey {
            item: one,
            property: KeyProperty::Opacity,
            at: secs(1.0) + TickDur::new(5),
            value: 0.3,
            ease: Ease::LINEAR,
        },
    );
    exec(
        &mut h,
        Command::SetClipKey {
            item: one,
            property: KeyProperty::Opacity,
            at: secs(0.5),
            value: 0.9,
            ease: Ease::LINEAR,
        },
    );
    let keys = item(&h, one).picture.opacity.keys;
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].at, secs(0.5));
    assert_eq!(keys[1].value, 0.3);
    exec(
        &mut h,
        Command::ClearClipKey {
            item: one,
            property: KeyProperty::Opacity,
            at: secs(0.5),
        },
    );
    assert_eq!(item(&h, one).picture.opacity.keys.len(), 1);
    exec(
        &mut h,
        Command::ClearClipKeys {
            item: one,
            property: KeyProperty::Opacity,
        },
    );
    assert!(item(&h, one).picture.opacity.keys.is_empty());
}

#[test]
fn effect_keys() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let effect = Effect::new(placeholder(), "concat.blur");
    exec(
        &mut h,
        Command::UpdateClip {
            item: one,
            patch: ItemPatch {
                effects: Some(vec![effect]),
                ..ItemPatch::default()
            },
        },
    );
    let id: EffectId = item(&h, one).effects[0].known().unwrap().id;
    exec(
        &mut h,
        Command::SetEffectKey {
            item: one,
            effect: id,
            param: "radius".into(),
            at: secs(0.5),
            value: 4.0,
            ease: Ease::LINEAR,
        },
    );
    exec(
        &mut h,
        Command::SetEffectKey {
            item: one,
            effect: id,
            param: "radius".into(),
            at: secs(1.5),
            value: 8.0,
            ease: Ease::LINEAR,
        },
    );
    let get = |h: &Hist| item(h, one).effects[0].known().unwrap().params.clone();
    assert_eq!(get(&h)["radius"].keys.len(), 2);
    exec(
        &mut h,
        Command::ClearEffectKey {
            item: one,
            effect: id,
            param: "radius".into(),
            at: secs(0.5),
        },
    );
    assert_eq!(get(&h)["radius"].keys.len(), 1);
    exec(
        &mut h,
        Command::ClearEffectKeys {
            item: one,
            effect: id,
            param: "radius".into(),
        },
    );
    assert!(
        !get(&h).contains_key("radius"),
        "a knob with no value and no keys goes"
    );
}

#[test]
fn mask_keys() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    exec(
        &mut h,
        Command::UpdateClip {
            item: one,
            patch: ItemPatch {
                masks: Some(vec![Mask::circle(placeholder())]),
                ..ItemPatch::default()
            },
        },
    );
    let mask: MaskId = item(&h, one).masks[0].known().unwrap().id;
    exec(
        &mut h,
        Command::SetMaskKey {
            item: one,
            mask,
            number: MaskNumber::Feather,
            at: secs(0.5),
            value: 5.0,
            ease: Ease::LINEAR,
        },
    );
    exec(
        &mut h,
        Command::SetMaskKey {
            item: one,
            mask,
            number: MaskNumber::Width,
            at: secs(0.5),
            value: 0.3,
            ease: Ease::LINEAR,
        },
    );
    let got = item(&h, one).masks[0].known().unwrap().clone();
    assert_eq!(got.feather.keys[0].value, 1.0, "held to its range");
    exec(
        &mut h,
        Command::ClearMaskKey {
            item: one,
            mask,
            number: MaskNumber::Feather,
            at: secs(0.5),
        },
    );
    exec(
        &mut h,
        Command::ClearMaskKeys {
            item: one,
            mask,
            number: MaskNumber::Width,
        },
    );
    let got = item(&h, one).masks[0].known().unwrap().clone();
    assert!(got.feather.keys.is_empty());
    // A second patch keeps the mask's id.
    exec(
        &mut h,
        Command::UpdateClip {
            item: one,
            patch: ItemPatch {
                masks: Some(vec![got.clone()]),
                ..ItemPatch::default()
            },
        },
    );
    assert_eq!(item(&h, one).masks[0].known().unwrap().id, mask);
}

// ----- Look -----

#[test]
fn update_clip_sets_what_it_names() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let patch = ItemPatch {
        crop: Some(Some(Crop {
            left: 0.1,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        })),
        flip_h: Some(true),
        opacity: Some(4.0),
        volume: Some(-1.0),
        name: Some("named".into()),
        motion: Some(Motion {
            enter: Some(Move {
                name: "fade".into(),
                len: len(0.5),
            }),
            ..Motion::default()
        }),
        muted: Some(true),
        keep_pitch: Some(false),
        ..ItemPatch::default()
    };
    exec(&mut h, Command::UpdateClip { item: one, patch });
    let i = item(&h, one);
    assert!(i.picture.flip_h);
    assert_eq!(i.picture.opacity.value, 1.0);
    assert_eq!(i.sound.volume.value, 0.0);
    assert_eq!(i.name, "named");
    assert!(i.sound.muted);
    assert!(i.picture.crop.is_some());
}

#[test]
fn transform_holds_its_numbers() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    exec(
        &mut h,
        Command::SetClipTransform {
            item: one,
            offset_x: Some(5.0),
            offset_y: None,
            rotation: Some(270.0),
            scale: Some(20.0),
            stretch_x: None,
            stretch_y: Some(0.0),
        },
    );
    let p = item(&h, one).picture;
    assert_eq!(p.offset_x.value, 3.0);
    assert_eq!(p.rotation.value, -90.0);
    assert_eq!(p.scale.value, 8.0);
    assert_eq!(p.stretch_y.value, 0.1);
}

#[test]
fn frame_fill_covers_the_frame() {
    let mut h = project();
    let s = add_media(&mut h, still("square"));
    let one = clip_at(&mut h, s, 0.0);
    exec(
        &mut h,
        Command::FrameClip {
            item: one,
            framing: Framing::Fill,
            at: Tick::ZERO,
        },
    );
    let scale = item(&h, one).picture.scale.value;
    assert!((scale - 1920.0 / 1080.0).abs() < 1e-9);
    exec(
        &mut h,
        Command::FrameClip {
            item: one,
            framing: Framing::Fit,
            at: Tick::ZERO,
        },
    );
    assert!((item(&h, one).picture.scale.value - 1.0).abs() < 1e-9);
}

#[test]
fn cutout_stabilize_voice_and_strokes() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let stroke = Stroke {
        tool: concat_doc::model::BrushTool::Brush,
        size: 0.1,
        points: vec![[0.5, 0.5]],
        at: None,
    };
    exec(&mut h, Command::AddCutoutStroke { item: one, stroke });
    assert_eq!(
        item(&h, one).picture.cutout.as_ref().unwrap().mode,
        CutoutMode::Custom
    );
    exec(
        &mut h,
        Command::SetClipCutout {
            item: one,
            cutout: Some(Cutout {
                mode: CutoutMode::Auto,
                subject: Subject::Person,
                feather: -1.0,
                strokes: vec![],
            }),
        },
    );
    assert_eq!(item(&h, one).picture.cutout.as_ref().unwrap().feather, 0.0);
    exec(
        &mut h,
        Command::SetClipStabilize {
            item: one,
            stabilize: Some(Stabilize {
                smoothness: 3.0,
                max_zoom: 0.5,
                keep_rotation: false,
                tripod: true,
            }),
        },
    );
    let st = item(&h, one).picture.stabilize.unwrap();
    assert_eq!((st.smoothness, st.max_zoom), (1.0, 1.0));
    exec(
        &mut h,
        Command::SetClipVoice {
            item: one,
            voice: Some(VoiceMix {
                kind: VoiceKind::Speech,
                voice: 1.0,
                background: 0.0,
            }),
        },
    );
    assert!(item(&h, one).sound.voice.is_some());
}

#[test]
fn words_styled_apart_and_given_back() {
    let mut h = project();
    let t: ItemId = created(&exec(
        &mut h,
        Command::AddTextClip {
            start: Tick::ZERO,
            len: None,
            lane: None,
            above: false,
            offset_y: None,
            title: Some(Title {
                content: "héllo world".into(),
                ..Title::default()
            }),
        },
    ));
    let bold = RunStyle {
        font_weight: Some(900.0),
        ..RunStyle::default()
    };
    let red = RunStyle {
        color: Some(Color::rgb(255, 0, 0)),
        ..RunStyle::default()
    };
    exec(
        &mut h,
        Command::StyleTextRange {
            item: t,
            start: 0,
            end: 5,
            style: bold,
        },
    );
    exec(
        &mut h,
        Command::StyleTextRange {
            item: t,
            start: 3,
            end: 100,
            style: red,
        },
    );
    let runs = |h: &Hist| match &item(h, t).content {
        Content::Title(title) => title.runs.clone(),
        _ => panic!(),
    };
    let r = runs(&h);
    assert_eq!(
        r.iter().map(|r| (r.start, r.end)).collect::<Vec<_>>(),
        vec![(0, 3), (3, 5), (5, 11)]
    );
    assert_eq!(r[1].font_weight, Some(900.0));
    assert_eq!(r[1].color, Some(Color::rgb(255, 0, 0)));
    exec(
        &mut h,
        Command::ClearTextStyles {
            item: t,
            start: Some(0),
            end: Some(4),
        },
    );
    assert_eq!(runs(&h)[0].start, 4);
    exec(
        &mut h,
        Command::ClearTextStyles {
            item: t,
            start: None,
            end: None,
        },
    );
    assert!(runs(&h).is_empty());
}

// ----- Lanes -----

#[test]
fn lanes_added_flagged_and_removed() {
    let mut h = project();
    let top: LaneId = created(&exec(&mut h, Command::AddTrack { bottom: false }));
    let bottom: LaneId = created(&exec(&mut h, Command::AddTrack { bottom: true }));
    let l = lanes(&h);
    assert_eq!((l[0], l[5]), (bottom, top));
    exec(
        &mut h,
        Command::SetTrackFlag {
            lane: top,
            flag: LaneFlag::Visible,
            value: false,
        },
    );
    exec(
        &mut h,
        Command::SetTrackFlag {
            lane: top,
            flag: LaneFlag::Muted,
            value: true,
        },
    );
    let lane = m(&h).lanes.get(top).unwrap();
    assert!(!lane.visible && lane.muted);
    let a = add_media(&mut h, video("a", 2.0));
    let c: ItemId = created(&exec(
        &mut h,
        Command::AddClip {
            media: a,
            lane: top,
            start: Tick::ZERO,
            ripple: false,
        },
    ));
    exec(&mut h, Command::RemoveTrack { lane: top });
    assert!(!m(&h).items.contains(c));
    for lane in lanes(&h).into_iter().skip(1) {
        exec(&mut h, Command::RemoveTrack { lane });
    }
    let last = lanes(&h)[0];
    assert!(matches!(
        try_exec(&mut h, Command::RemoveTrack { lane: last }),
        Err(CommandError::Refused(_))
    ));
}

#[test]
fn the_main_lane_ripples_the_others_when_linked() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let one = clip_at(&mut h, a, 0.0);
    let next = clip_at(&mut h, a, 2.0);
    let title: ItemId = created(&exec(
        &mut h,
        Command::AddTextClip {
            start: secs(2.5),
            len: Some(len(1.0)),
            lane: None,
            above: true,
            offset_y: None,
            title: None,
        },
    ));
    exec(
        &mut h,
        Command::TrimClip {
            item: one,
            edge: Edge::Tail,
            delta: dur(-1.0),
            ripple: true,
        },
    );
    assert_eq!(item(&h, next).start, secs(1.0));
    assert_eq!(item(&h, title).start, secs(1.5), "it follows the main lane");
}

#[test]
fn a_batch_is_one_step() {
    let mut h = project();
    let a = add_media(&mut h, video("a", 2.0));
    let steps = h.undo_len();
    let out = exec(
        &mut h,
        Command::Batch {
            commands: vec![
                Command::AddClipAtFirstFree {
                    media: a,
                    start: Tick::ZERO,
                },
                Command::AddClipAtFirstFree {
                    media: a,
                    start: secs(2.0),
                },
            ],
        },
    );
    assert_eq!(h.undo_len(), steps + 1);
    assert_eq!(m(&h).items.len(), 2);
    let last: ItemId = created(&out);
    assert_eq!(item(&h, last).start, secs(2.0), "the last id made");
    // One refused, none made.
    let rev = h.current().revision();
    let refused = try_exec(
        &mut h,
        Command::Batch {
            commands: vec![
                Command::AddClipAtFirstFree {
                    media: a,
                    start: secs(9.0),
                },
                Command::RemoveTrack {
                    lane: placeholder(),
                },
            ],
        },
    );
    assert!(refused.is_err());
    assert_eq!(h.current().revision(), rev);
    assert_eq!(m(&h).items.len(), 2);
}

#[test]
fn labels_are_plain_words() {
    assert_eq!(
        Command::SplitClips {
            items: vec![],
            at: Tick::ZERO
        }
        .label(),
        "Split clip"
    );
}

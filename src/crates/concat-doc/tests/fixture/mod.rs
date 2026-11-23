// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A small project the model's tests start from: one sequence showing
//! in a tab, two lanes, and three files - a ten-second video with
//! sound, a still and a five-second song.

#![allow(dead_code)]

use concat_doc::model::{
    Content, Frame, Item, ItemId, Lane, LaneId, Media, MediaCut, MediaId, MediaKind, Model,
    OrderKey, Rule, Sequence, SequenceId, Size, SoundStream, Violation,
};
use concat_doc::{
    AnyId, Commit, Document, Draft, Origin, Rate, Rejected, TICKS_PER_SECOND, Tick, TickDur,
    TickLen,
};

/// A place this many seconds from zero.
pub fn at(seconds: f64) -> Tick {
    Tick::new((seconds * TICKS_PER_SECOND as f64).round() as i64)
}

/// A distance of this many seconds.
pub fn dur(seconds: f64) -> TickDur {
    TickDur::new((seconds * TICKS_PER_SECOND as f64).round() as i64)
}

/// A length of this many seconds.
pub fn len(seconds: f64) -> TickLen {
    dur(seconds).to_len().expect("a length above zero")
}

pub struct Fixture {
    pub doc: Document<Model>,
    pub seq: SequenceId,
    /// The lower lane.
    pub low: LaneId,
    /// The lane above it.
    pub high: LaneId,
    /// Ten seconds of 30 fps video with one sound stream, index 1.
    pub video: MediaId,
    /// A still.
    pub still: MediaId,
    /// Five seconds of sound.
    pub song: MediaId,
}

pub fn video_file() -> Media {
    Media {
        path: "/footage/a.mp4".into(),
        name: "a.mp4".into(),
        kind: MediaKind::Video,
        length: Some(len(10.0)),
        rate: Some(Rate::FPS_30),
        size: Size::new(320, 180),
        has_sound: true,
        streams: vec![SoundStream {
            index: 1,
            channels: 2,
            sample_rate: 48_000,
            ..SoundStream::default()
        }],
        ..Media::default()
    }
}

pub fn still_file() -> Media {
    Media {
        path: "/footage/b.png".into(),
        name: "b.png".into(),
        kind: MediaKind::Image,
        size: Size::new(240, 180),
        ..Media::default()
    }
}

pub fn song_file() -> Media {
    Media {
        path: "/footage/song.wav".into(),
        name: "song.wav".into(),
        kind: MediaKind::Audio,
        length: Some(len(5.0)),
        has_sound: true,
        ..Media::default()
    }
}

/// The project: everything above and nothing placed.
pub fn fixture() -> Fixture {
    let doc: Document<Model> = Document::new();
    let mut draft = doc.begin("Start", Origin(0));
    let seq: SequenceId = draft.mint().unwrap();
    let low: LaneId = draft.mint().unwrap();
    let high: LaneId = draft.mint().unwrap();
    let video: MediaId = draft.mint().unwrap();
    let still: MediaId = draft.mint().unwrap();
    let song: MediaId = draft.mint().unwrap();
    let mut frame = Frame::hd();
    frame.width = std::num::NonZeroU32::new(320).unwrap();
    frame.height = std::num::NonZeroU32::new(180).unwrap();
    draft
        .sequences
        .insert(seq, Sequence::new("Timeline 1", frame))
        .unwrap();
    draft.view.showing = Some(seq);
    draft.view.tabs = vec![seq];
    let a = OrderKey::parse("a").unwrap();
    let m = OrderKey::parse("m").unwrap();
    draft.lanes.insert(low, Lane::new(seq, a)).unwrap();
    draft.lanes.insert(high, Lane::new(seq, m)).unwrap();
    draft.media.insert(video, video_file()).unwrap();
    draft.media.insert(still, still_file()).unwrap();
    draft.media.insert(song, song_file()).unwrap();
    let doc = draft
        .commit()
        .expect("the fixture keeps the rules")
        .into_document();
    Fixture {
        doc,
        seq,
        low,
        high,
        video,
        still,
        song,
    }
}

/// A cut of a file from its start, at a place on a lane.
pub fn cut(lane: LaneId, start: f64, length: f64, media: MediaId) -> Item {
    Item::new(
        lane,
        at(start),
        len(length),
        Content::Media(MediaCut::whole(media)),
    )
}

/// Makes a change and commits it.
pub fn commit(
    doc: &Document<Model>,
    change: impl FnOnce(&mut Draft<Model>),
) -> Result<Commit<Model>, Rejected<Violation>> {
    let mut draft = doc.begin("Test", Origin(0));
    change(&mut draft);
    draft.commit()
}

/// Makes a change that must be accepted, and gives the new document.
pub fn apply(doc: &Document<Model>, change: impl FnOnce(&mut Draft<Model>)) -> Document<Model> {
    match commit(doc, change) {
        Ok(commit) => commit.into_document(),
        Err(why) => panic!("refused: {why}"),
    }
}

/// Places an item, which must be accepted, and gives its id.
pub fn place(doc: &mut Document<Model>, item: Item) -> ItemId {
    let mut made = None;
    *doc = apply(doc, |draft| {
        let id: ItemId = draft.mint().unwrap();
        draft.items.insert(id, item).unwrap();
        made = Some(id);
    });
    made.unwrap()
}

/// The rules a change breaks; it must break some.
pub fn refused(doc: &Document<Model>, change: impl FnOnce(&mut Draft<Model>)) -> Vec<Violation> {
    match commit(doc, change) {
        Ok(_) => panic!("the change was accepted"),
        Err(Rejected::Invalid(broken)) => broken,
        Err(Rejected::Unminted(ids)) => panic!("refused for ids never made: {ids:?}"),
    }
}

/// Asserts that a change is refused, naming this id and this rule.
pub fn assert_refused(
    doc: &Document<Model>,
    id: impl Into<AnyId>,
    rule: Rule,
    change: impl FnOnce(&mut Draft<Model>),
) {
    let id = id.into();
    let broken = refused(doc, change);
    let wanted = Violation { id: Some(id), rule };
    assert!(
        broken.contains(&wanted),
        "wanted {wanted}, the commit was refused with {broken:?}"
    );
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The measurements behind a document's file: how long a project of ten
//! thousand items takes to write, to read and to save, how large it is,
//! and what a diff of one edit costs in each layout the file could have.
//!
//! ```text
//! cargo run --release -p concat-doc --example file_bench
//! ```
//!
//! The project is one sequence of ten lanes and two hundred files, with
//! 10,000 items: seven in ten clips (one in three of those with keys on
//! its opacity, an effect with a keyed knob working inside a mask of its
//! own), and titles, shapes and transitions. Every time is the median of
//! nine runs. The results are written up in the documentation of
//! `concat_doc::file`.

use std::time::{Duration, Instant};

use concat_doc::model::{
    Animated, Content, Effect, Entry, Frame, Item, ItemId, Key, Lane, LaneId, Mask, Media,
    MediaCut, MediaId, MediaKind, Model, OrderKey, Sequence, SequenceId, ShapeStyle, Size, Title,
};
use concat_doc::{Document, Origin, Rate, TICKS_PER_SECOND, Tick, TickLen, file, save};

const ITEMS: usize = 10_000;
const LANES: usize = 10;
const FILES: usize = 200;

fn project() -> Document<Model> {
    let doc: Document<Model> = Document::new();
    let mut draft = doc.begin("Build", Origin(0));
    let seq: SequenceId = draft.mint().unwrap();
    draft
        .sequences
        .insert(seq, Sequence::new("Timeline 1", Frame::hd()))
        .unwrap();
    draft.view.showing = Some(seq);
    draft.view.tabs.push(seq);
    let mut lanes: Vec<LaneId> = Vec::new();
    let mut below = None;
    for _ in 0..LANES {
        let id: LaneId = draft.mint().unwrap();
        let order = OrderKey::between(below.as_ref(), None).unwrap();
        draft
            .lanes
            .insert(id, Lane::new(seq, order.clone()))
            .unwrap();
        below = Some(order);
        lanes.push(id);
    }
    let mut files: Vec<MediaId> = Vec::new();
    for n in 0..FILES {
        let id: MediaId = draft.mint().unwrap();
        let file = Media {
            path: format!("/Users/someone/Movies/Holiday 2026/clip {n:04}.mp4"),
            name: format!("clip {n:04}.mp4"),
            kind: MediaKind::Video,
            length: TickLen::new(60 * TICKS_PER_SECOND),
            rate: Some(Rate::FPS_29_97),
            size: Size::new(3840, 2160),
            has_sound: true,
            video_codec: Some("hevc".into()),
            audio_codec: Some("aac".into()),
            ..Media::default()
        };
        draft.media.insert(id, file).unwrap();
        files.push(id);
    }
    let second = TICKS_PER_SECOND;
    for n in 0..ITEMS {
        let lane = lanes[n % LANES];
        let start = Tick::new((n / LANES) as i64 * 3 * second);
        let len = TickLen::new(2 * second + (n as i64 % 7) * second / 10).unwrap();
        let content = match n % 20 {
            0..=13 => Content::Media(MediaCut::whole(files[n % FILES])),
            14..=16 => Content::Title(Title {
                content: format!("Caption number {n}, a line long enough to wrap"),
                ..Title::default()
            }),
            17 | 18 => Content::Shape(ShapeStyle::default()),
            _ => Content::Transition(Effect::new(draft.mint().unwrap(), "concat.dissolve")),
        };
        let mut item = Item::new(lane, start, len, content);
        if n % 20 < 14 && n % 3 == 0 {
            item.picture.opacity = Animated {
                value: 1.0,
                keys: vec![
                    Key::linear(Tick::ZERO, 0.0),
                    Key::linear(Tick::new(second / 2), 1.0),
                ],
            };
            let mask = Mask::circle(draft.mint().unwrap());
            let mut effect = Effect::new(draft.mint().unwrap(), "concat.color");
            effect.mask = Some(mask.id);
            effect.params.insert(
                "saturation".into(),
                Animated {
                    value: Some(1.2),
                    keys: vec![
                        Key::linear(Tick::ZERO, 0.8),
                        Key::linear(Tick::new(second), 1.4),
                    ],
                },
            );
            item.masks.push(Entry::Known(mask));
            item.effects.push(Entry::Known(effect));
        }
        let id: ItemId = draft.mint().unwrap();
        draft.items.insert(id, item).unwrap();
    }
    draft.commit().expect("a valid project").into_document()
}

/// The median of nine runs of `run`.
fn median<T>(mut run: impl FnMut() -> T) -> (Duration, T) {
    let mut times = Vec::new();
    let mut last = None;
    for _ in 0..9 {
        let began = Instant::now();
        let out = run();
        times.push(began.elapsed());
        last = Some(out);
    }
    times.sort();
    (times[4], last.expect("nine runs"))
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1000.0)
}

/// How many lines differ between two texts of the same number of lines,
/// and how many bytes those lines hold on both sides.
fn diff(a: &str, b: &str) -> (usize, usize) {
    let (mut lines, mut bytes) = (0, 0);
    for (x, y) in a.lines().zip(b.lines()) {
        if x != y {
            lines += 1;
            bytes += x.len() + y.len();
        }
    }
    (lines, bytes)
}

fn main() {
    let doc = project();
    println!(
        "{} items, {} lanes, {} files",
        doc.items.len(),
        doc.lanes.len(),
        doc.media.len()
    );

    let (write, text) = median(|| file::write(&doc));
    let (read, opened) = median(|| file::read(text.as_bytes()).expect("opens"));
    assert!(opened.notes.is_empty());
    assert_eq!(opened.document.tables(), doc.tables());
    let (index, _) = median(|| {
        doc.items
            .rows()
            .iter()
            .map(|(id, item)| (id, std::sync::Arc::clone(item)))
            .collect::<concat_doc::model::Items>()
    });
    println!("index build: {}", ms(index));
    let (check, _) = median(|| {
        Document::load(doc.tables().clone(), doc.mint().clone(), Default::default()).is_ok()
    });
    println!("load check: {}", ms(check));
    let (snapshot, _) = median(|| doc.clone());
    println!("snapshot (the save's copy): {snapshot:?}");
    println!("write:  {} for {} bytes", ms(write), text.len());
    println!("read:   {}", ms(read));

    // The layouts: compact, a record a line (the file's), indented
    // throughout.
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    let compact = serde_json::to_string(&value).expect("writes");
    let (pretty_time, pretty) = median(|| serde_json::to_string_pretty(&value).expect("writes"));
    println!(
        "size:   a record a line {} bytes, compact {} bytes (+{:.2}%), indented {} bytes (x{:.2})",
        text.len(),
        compact.len(),
        100.0 * (text.len() as f64 / compact.len() as f64 - 1.0),
        pretty.len(),
        pretty.len() as f64 / text.len() as f64
    );
    println!(
        "        (an indenting writer, for its time: {})",
        ms(pretty_time)
    );

    // One edit: an item moved by a frame.
    let moved = {
        let id = doc.items.ids().nth(ITEMS / 2).unwrap();
        let mut draft = doc.begin("Nudge", Origin(0));
        draft
            .items
            .update(id, |item| {
                item.start = Tick::new(item.start.get() + 23_543_520)
            })
            .unwrap();
        draft.commit().unwrap().into_document()
    };
    let after = file::write(&moved);
    let after_value: serde_json::Value = serde_json::from_str(&after).unwrap();
    let (lines, bytes) = diff(&text, &after);
    println!("diff of one edit: a record a line {lines} line(s), {bytes} bytes");
    let after_compact = serde_json::to_string(&after_value).unwrap();
    let (lines, bytes) = diff(&compact, &after_compact);
    println!("                  compact {lines} line(s), {bytes} bytes");
    let after_pretty = serde_json::to_string_pretty(&after_value).unwrap();
    let (lines, bytes) = diff(&pretty, &after_pretty);
    println!("                  indented {lines} line(s), {bytes} bytes");

    // The whole save: written, flushed through the drive's cache, read back
    // and compared, the previous kept, swapped in, the folder flushed.
    let folder = tempfile::tempdir().expect("a folder");
    let path = folder.path().join("bench.concat");
    save::save_document(&path, &doc).expect("first save");
    let (whole, _) = median(|| save::save_document(&path, &doc).expect("saves"));
    let (bytes_only, _) = median(|| save::save(&path, text.as_bytes()).expect("saves"));
    let (open, _) = median(|| save::open(&path).expect("opens"));
    println!(
        "save:   {} in all; {} of it the disk's",
        ms(whole),
        ms(bytes_only)
    );
    println!("open:   {} from the disk", ms(open));
}

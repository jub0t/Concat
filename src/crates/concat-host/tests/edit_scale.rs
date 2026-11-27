// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How the work an edit and a drag do grows with the document: a video
//! under `n` captions, the shape a transcribed interview takes, timed step
//! by step - the edit applied with its undo, the document flattened, the
//! titles painted (kept, and live as a drag paints them), the preview's
//! plan built, and the autosave's document written.
//!
//! Ignored: a measurement, not a check.
//! `cargo test -p concat-host --release --test edit_scale -- --ignored --nocapture`;
//! `CONCAT_SCALE_COUNTS` lists the caption counts ("100,500,2000").

use std::time::Instant;

use concat_doc::commands::{self, Command};
use concat_doc::model::{ItemId, Media, MediaId, MediaKind, Model, SequenceId, Size, Title};
use concat_doc::{Document, Tick, TickLen};
use concat_host::{AppDirs, Titles};

/// The flattened clips of the timeline showing.
fn flatten_timeline_in(
    project: &Model,
    _sequence: Option<SequenceId>,
    dir: Option<&std::path::Path>,
) -> Vec<concat_export::ExportClip> {
    let sequence = project.view.showing.expect("a timeline shows");
    concat_export::flatten::clips(project, sequence, dir)
}

fn tick(seconds: f64) -> Tick {
    Tick::from_display_seconds(seconds).expect("a place")
}

fn made(editor: &mut Document<Model>, command: Command) -> commands::Applied {
    let (after, applied) = commands::made(editor, &command).expect("applies");
    *editor = after;
    applied
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

/// Milliseconds `work` takes, median of `runs`.
fn timed(runs: usize, mut work: impl FnMut()) -> f64 {
    median(
        (0..runs)
            .map(|_| {
                let begun = Instant::now();
                work();
                begun.elapsed().as_secs_f64() * 1000.0
            })
            .collect(),
    )
}

fn project(captions: usize) -> (Document<Model>, Vec<ItemId>) {
    let mut editor = Document::new();
    made(&mut editor, Command::AddTimeline);
    let length = captions as f64 * 1.5 + 1.0;
    let media: MediaId = made(
        &mut editor,
        Command::AddMedia {
            media: Media {
                path: "/footage/interview.mp4".into(),
                name: "interview.mp4".into(),
                kind: MediaKind::Video,
                length: TickLen::try_from(tick(length).get()).ok(),
                size: Size::new(1920, 1080),
                has_sound: true,
                ..Media::default()
            },
        },
    )
    .created
    .and_then(|id| id.typed())
    .expect("an id");
    let sequence = editor.view.showing.expect("a timeline");
    let lane = editor.lanes.index().of(sequence).next().expect("a lane").0;
    made(
        &mut editor,
        Command::AddClip {
            media,
            lane,
            start: Tick::ZERO,
            ripple: false,
        },
    );
    let mut ids = Vec::new();
    for index in 0..captions {
        let title = Title {
            content: format!("caption number {index}, a line of what was said"),
            ..Title::default()
        };
        let id: ItemId = made(
            &mut editor,
            Command::AddTextClip {
                lane: None,
                above: true,
                start: tick(index as f64 * 1.5),
                title: Some(title),
                len: TickLen::try_from(tick(1.4).get()).ok(),
                offset_y: Some(0.35),
            },
        )
        .created
        .and_then(|id| id.typed())
        .expect("an id");
        ids.push(id);
    }
    (editor, ids)
}

#[test]
#[ignore = "a measurement, not a check"]
fn edit_scale() {
    let counts: Vec<usize> = std::env::var("CONCAT_SCALE_COUNTS")
        .unwrap_or_else(|_| "100,500,2000".to_owned())
        .split(',')
        .filter_map(|count| count.parse().ok())
        .collect();
    let dir = std::env::temp_dir().join("concat-edit-scale");
    let dirs = AppDirs::under(&dir);
    eprintln!(
        "{:>6} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "titles", "apply", "flatten", "paint", "live", "plan", "document", "drag step"
    );
    for count in counts {
        let (mut editor, ids) = project(count);
        let titles = Titles::new(&dirs);
        let target = ids[ids.len() / 2];
        let mut nudge = 0.0;
        let apply = timed(20, || {
            nudge += 0.001;
            made(
                &mut editor,
                Command::SetClipTransform {
                    item: target,
                    scale: None,
                    offset_x: Some(nudge),
                    offset_y: None,
                    rotation: None,
                    stretch_x: None,
                    stretch_y: None,
                },
            );
        });
        let project = editor.clone();
        let flatten = timed(20, || {
            std::hint::black_box(flatten_timeline_in(&project, None, Some(&dir)));
        });
        // Cold: every title painted, as on opening the project or after a
        // restyle of every caption - per title, each in a fresh cache.
        let cold = {
            let fresh = AppDirs::under(&dir.join(format!("cold-{count}")));
            let _ = std::fs::remove_dir_all(dir.join(format!("cold-{count}")));
            let begun = Instant::now();
            Titles::new(&fresh).clips(&project, 1920, 1080);
            begun.elapsed().as_secs_f64() * 1000.0 / count as f64
        };
        let cold_live = {
            let begun = Instant::now();
            Titles::new(&dirs).clips_live(&project, 960, 540);
            begun.elapsed().as_secs_f64() * 1000.0 / count as f64
        };
        // What the window's thread pays on that open now: a look at what
        // is painted, with nothing painted yet.
        let fresh = Titles::new(&AppDirs::under(&dir.join(format!("ready-{count}"))));
        let _ = std::fs::remove_dir_all(dir.join(format!("ready-{count}")));
        let begun = Instant::now();
        let _ = fresh.ready(&project, 1920, 1080);
        let looked = begun.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "{count:>6} cold paint {cold:.2} ms a title (1080p, kept on disk), \
             {cold_live:.2} ms live (540p); the window's look with none painted {looked:.2} ms"
        );
        // Painted once into the cache, then timed warm: what an edit that
        // changed no title pays.
        titles.clips(&project, 1920, 1080);
        let paint = timed(10, || {
            std::hint::black_box(titles.clips(&project, 1920, 1080));
        });
        titles.clips_live(&project, 960, 540);
        let live = timed(10, || {
            std::hint::black_box(titles.clips_live(&project, 960, 540));
        });
        let mut clips = flatten_timeline_in(&project, None, Some(&dir));
        clips.extend(
            titles
                .clips(&project, 1920, 1080)
                .into_iter()
                .map(|title| title.clip),
        );
        let plan = timed(10, || {
            std::hint::black_box(concat_export::preview_plan(
                &clips,
                1920,
                1080,
                30,
                1,
                Default::default(),
                [0, 0, 0, 255],
            ));
        });
        let document = timed(10, || {
            std::hint::black_box(concat_doc::file::write(&editor));
        });
        // One pointer step of a drag on the stage, as the monitor does it:
        // the echo flattened fresh, every title painted live, a new plan.
        let drag = timed(10, || {
            let mut clips = flatten_timeline_in(&project, None, Some(&dir));
            clips.extend(
                titles
                    .clips_live(&project, 960, 540)
                    .into_iter()
                    .map(|title| title.clip),
            );
            std::hint::black_box(concat_export::preview_plan(
                &clips,
                1920,
                1080,
                30,
                1,
                Default::default(),
                [0, 0, 0, 255],
            ));
        });
        eprintln!(
            "{count:>6} {apply:>7.2}ms {flatten:>7.2}ms {paint:>7.2}ms {live:>7.2}ms \
             {plan:>7.2}ms {document:>7.2}ms {drag:>7.2}ms"
        );
    }
}

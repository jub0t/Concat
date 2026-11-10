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

use concat_export::flatten::flatten_timeline_in;
use concat_host::{AppDirs, Titles};
use concat_project::commands::NewMedia;
use concat_project::model::{MediaKind, TextStyle};
use concat_project::{Command, DocumentSettings, Editor};

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

fn project(captions: usize) -> (Editor, Vec<String>) {
    let mut editor = Editor::new();
    let length = captions as f64 * 1.5 + 1.0;
    let media = editor
        .apply(Command::AddMedia {
            item: NewMedia {
                path: "/footage/interview.mp4".into(),
                name: "interview.mp4".into(),
                duration: Some(length),
                kind: MediaKind::Video,
                width: Some(1920),
                height: Some(1080),
                frame_rate: None,
                frame_rate_fraction: None,
                video_codec: None,
                audio_codec: None,
                has_audio: true,
                audio_tracks: Vec::new(),
                origin: None,
                color_space: Default::default(),
            },
        })
        .expect("adds media")
        .created_id
        .expect("an id");
    let track = editor.project().timelines[0].tracks[0].id.clone();
    editor
        .apply(Command::AddClip {
            media_id: media,
            track_id: track,
            start: 0.0,
            ripple: false,
        })
        .expect("adds the video");
    let mut ids = Vec::new();
    for index in 0..captions {
        let style = TextStyle {
            content: format!("caption number {index}, a line of what was said"),
            ..TextStyle::default()
        };
        let id = editor
            .apply(Command::AddTextClip {
                track_id: None,
                above: true,
                start: index as f64 * 1.5,
                style: Some(style),
                duration: Some(1.4),
                offset_y: Some(0.35),
            })
            .expect("adds a caption")
            .created_id
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
    let settings = DocumentSettings {
        name: "scale".into(),
        width: 1920,
        height: 1080,
        rate_num: 30,
        rate_den: 1,
        canvas: [0, 0, 0, 255],
    };
    eprintln!(
        "{:>6} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "titles", "apply", "flatten", "paint", "live", "plan", "document", "drag step"
    );
    for count in counts {
        let (mut editor, ids) = project(count);
        let titles = Titles::new(&dirs);
        let target = ids[ids.len() / 2].clone();
        let mut nudge = 0.0;
        let apply = timed(20, || {
            nudge += 0.001;
            editor
                .apply(Command::SetClipTransform {
                    clip_id: target.clone(),
                    scale: None,
                    offset_x: Some(nudge),
                    offset_y: None,
                    rotation: None,
                    stretch_x: None,
                    stretch_y: None,
                })
                .expect("moves");
        });
        let project = editor.project().clone();
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
        eprintln!(
            "{count:>6} cold paint {cold:.2} ms a title (1080p, kept on disk), {cold_live:.2} ms live (540p)"
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
            let value = editor.to_document(&settings);
            std::hint::black_box(serde_json::to_string_pretty(&value).expect("writes"));
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

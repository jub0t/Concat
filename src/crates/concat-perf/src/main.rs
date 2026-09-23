// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! How fast the parts that matter are, and where they fall short.
//!
//! Every scenario here is one thing a person waits on - a frame to plan,
//! an undo, a file to open, a scrub, a frame to draw, an export - measured
//! on synthetic media so the numbers are the machine's and the code's
//! and nothing else's. Each has a budget. `cargo run -p concat-perf
//! --release` prints the table; with `--check` a scenario over its budget
//! fails the run, which is what CI wants: a regression should stop the
//! build, not wait for a person to notice.
//!
//! The budgets are set with room to spare on a 2023 laptop, so a slower
//! machine still passes and a regression of two or three times does not.
//! What is not here: anything that needs a window. The timeline's repaint
//! is measured in the app with `SLINT_DEBUG_PERFORMANCE`; the rows the
//! window publishes for it are pure and tested in the window crate.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use concat_core::frame::Frame;
use concat_core::time::{FrameRate, Rational};
use concat_core::timeline::{Clip, MediaRef, Timeline, Track, TrackKind, Transform};
use concat_media::decode::{DecodeOptions, Decoder, FrameSource};
use concat_media::{EncodeOptions, Encoder, FrameRequest, FrameSink, ReaderPool};
use concat_project::model::AppliedFilter;
use concat_project::{Command, Editor};
use concat_render::{Compositor, CpuCompositor, FramePlan, PlannedLayer, plan_frame};

/// Whether a number is good when it is under the budget or over it.
#[derive(Clone, Copy)]
enum Budget {
    /// A time or a count that must stay at or under this.
    AtMost(f64),
    /// A rate or a ratio that must reach this.
    AtLeast(f64),
}

/// One scenario's result.
struct Measure {
    name: &'static str,
    value: f64,
    unit: &'static str,
    budget: Budget,
    /// A line on what the number means or lacks.
    note: String,
}

impl Measure {
    fn holds(&self) -> bool {
        match self.budget {
            Budget::AtMost(limit) => self.value <= limit,
            Budget::AtLeast(floor) => self.value >= floor,
        }
    }
}

fn main() {
    let check = std::env::args().any(|arg| arg == "--check");
    let quick = std::env::args().any(|arg| arg == "--quick");
    concat_media::init();
    let mut results = vec![
        plan_200_clips(),
        undo_200_edits(),
        ripple_delete_of_200(),
        document_round_trip(),
        publish_window_of_200(),
        waveform_of_an_hour(),
    ];
    if !quick {
        let media = Media::synthesise();
        results.push(decode_software(&media));
        if let Some(measure) = decode_hardware(&media) {
            results.push(measure);
        }
        results.extend(scrub(&media));
        results.push(compose_cpu());
        if let Some(measure) = compose_gpu() {
            results.push(measure);
        }
        results.push(export(&media));
    }
    println!();
    println!(
        "{:<44} {:>10} {:<8} {:>12}  note",
        "scenario", "measured", "unit", "budget"
    );
    println!("{}", "-".repeat(110));
    let mut failed = 0;
    for result in &results {
        let (mark, budget) = match result.budget {
            Budget::AtMost(limit) => (
                if result.holds() { "ok " } else { "OVER" },
                format!("<= {limit}"),
            ),
            Budget::AtLeast(floor) => (
                if result.holds() { "ok " } else { "UNDER" },
                format!(">= {floor}"),
            ),
        };
        if !result.holds() {
            failed += 1;
        }
        println!(
            "{:<44} {:>10.2} {:<8} {:>12}  {mark} {}",
            result.name, result.value, result.unit, budget, result.note
        );
    }
    println!();
    if failed > 0 {
        println!("{failed} scenario(s) outside their budget");
        if check {
            std::process::exit(1);
        }
    } else {
        println!("every scenario within its budget");
    }
}

/// A timeline with `clips` one-second clips across `tracks` video tracks.
fn timeline_of(clips: usize, tracks: usize) -> Timeline {
    let mut timeline = Timeline::new(1920, 1080, FrameRate::THIRTY);
    let lanes: Vec<_> = (0..tracks)
        .map(|index| timeline.add_track(Track::new(format!("V{index}"), TrackKind::Video)))
        .collect();
    for index in 0..clips {
        let track = lanes[index % tracks];
        let start = Rational::new((index / tracks) as i64, 1);
        timeline
            .add_clip(
                track,
                Clip::new(
                    MediaRef::new(format!("clip-{index}.mp4")),
                    start,
                    Rational::new(1, 1),
                ),
            )
            .expect("the track exists");
    }
    timeline
}

/// Ripple-deleting every other clip of a four-hundred-clip cut, on four
/// lanes: the gap-closing walks every survivor against every removed span
/// on its lane, and a rough cut does this after every single deletion.
/// https://github.com/jub0t/Concat/issues/106
fn ripple_delete_of_200() -> Measure {
    let mut editor = Editor::new();
    let media_id = editor
        .apply(Command::AddMedia {
            item: concat_project::commands::NewMedia {
                path: "/perf/ripple.mp4".to_owned(),
                name: "ripple.mp4".to_owned(),
                duration: Some(1.0),
                kind: concat_project::model::MediaKind::Video,
                width: Some(1920),
                height: Some(1080),
                frame_rate: Some(30.0),
                frame_rate_fraction: Some("30/1".to_owned()),
                video_codec: Some("h264".to_owned()),
                audio_codec: None,
                has_audio: false,
                audio_tracks: Vec::new(),
                origin: None,
            },
        })
        .expect("adds media")
        .created_id
        .expect("a media id");
    let tracks: Vec<String> = editor
        .project()
        .active()
        .tracks
        .iter()
        .map(|track| track.id.clone())
        .collect();
    let mut ids = Vec::with_capacity(400);
    for index in 0..400usize {
        let id = editor
            .apply(Command::AddClip {
                media_id: media_id.clone(),
                track_id: tracks[index % tracks.len()].clone(),
                start: (index / tracks.len()) as f64,
                ripple: false,
            })
            .expect("adds a clip")
            .created_id
            .expect("a clip id");
        ids.push(id);
    }
    let doomed: Vec<String> = ids.iter().step_by(2).cloned().collect();
    let started = Instant::now();
    editor
        .apply(Command::RemoveClips {
            clip_ids: doomed,
            ripple: true,
        })
        .expect("removes");
    let millis = started.elapsed().as_secs_f64() * 1e3;
    let last = editor
        .project()
        .active()
        .clips
        .iter()
        .map(|clip| clip.start + clip.duration)
        .fold(0.0_f64, f64::max);
    Measure {
        name: "ripple delete of 200 clips out of 400",
        value: millis,
        unit: "ms",
        budget: Budget::AtMost(20.0),
        note: format!("the cut ends at {last}s, was 100s"),
    }
}

/// Planning one frame of a two-hundred-clip cut: microseconds, and the
/// thing every scrub and every export frame starts with.
fn plan_200_clips() -> Measure {
    let timeline = timeline_of(200, 4);
    let rounds = 2000;
    let started = Instant::now();
    let mut layers = 0;
    for round in 0..rounds {
        let time = Rational::new(round % 50, 1) + Rational::new(1, 2);
        layers += plan_frame(&timeline, time).layers.len();
    }
    let micros = started.elapsed().as_secs_f64() * 1e6 / f64::from(rounds as u32);
    Measure {
        name: "plan one frame of 200 clips on 4 tracks",
        value: micros,
        unit: "us",
        budget: Budget::AtMost(200.0),
        note: format!("{} layers a frame on average", layers / rounds as usize),
    }
}

/// Two hundred edits and two hundred undos on the document: the snapshot
/// per command is what makes undo instant or not.
fn undo_200_edits() -> Measure {
    let mut editor = Editor::new();
    let media_id = editor
        .apply(Command::AddMedia {
            item: concat_project::commands::NewMedia {
                path: "/perf/clip.mp4".to_owned(),
                name: "clip.mp4".to_owned(),
                duration: Some(10.0),
                kind: concat_project::model::MediaKind::Video,
                width: Some(1920),
                height: Some(1080),
                frame_rate: Some(30.0),
                frame_rate_fraction: Some("30/1".to_owned()),
                video_codec: Some("h264".to_owned()),
                audio_codec: None,
                has_audio: false,
                audio_tracks: Vec::new(),
                origin: None,
            },
        })
        .expect("adds media")
        .created_id
        .expect("a media id");
    let started = Instant::now();
    for index in 0..200 {
        editor
            .apply(Command::AddClipAtFirstFree {
                media_id: media_id.clone(),
                start: f64::from(index) * 0.5,
            })
            .expect("adds a clip");
    }
    for _ in 0..200 {
        assert!(editor.undo(), "two hundred steps to undo");
    }
    let millis = started.elapsed().as_secs_f64() * 1e3;
    Measure {
        name: "200 clip additions and 200 undos",
        value: millis,
        unit: "ms",
        budget: Budget::AtMost(150.0),
        note: format!(
            "{} clips left after the undos",
            editor.project().active().clips.len()
        ),
    }
}

/// Writing a two-hundred-clip document and reading it back: what a save
/// and an open cost, before any media is touched.
fn document_round_trip() -> Measure {
    let mut editor = Editor::new();
    let media_id = editor
        .apply(Command::AddMedia {
            item: concat_project::commands::NewMedia {
                path: "/perf/clip.mp4".to_owned(),
                name: "clip.mp4".to_owned(),
                duration: Some(10.0),
                kind: concat_project::model::MediaKind::Video,
                width: Some(1920),
                height: Some(1080),
                frame_rate: Some(30.0),
                frame_rate_fraction: Some("30/1".to_owned()),
                video_codec: Some("h264".to_owned()),
                audio_codec: None,
                has_audio: false,
                audio_tracks: Vec::new(),
                origin: None,
            },
        })
        .expect("adds media")
        .created_id
        .expect("a media id");
    for index in 0..200 {
        editor
            .apply(Command::AddClipAtFirstFree {
                media_id: media_id.clone(),
                start: f64::from(index) * 0.5,
            })
            .expect("adds a clip");
    }
    let settings = concat_project::DocumentSettings {
        name: "perf".to_owned(),
        width: 1920,
        height: 1080,
        rate_num: 30,
        rate_den: 1,
    };
    let rounds = 20;
    let started = Instant::now();
    let mut bytes = 0;
    for _ in 0..rounds {
        let document = editor.to_document(&settings);
        bytes = document.to_string().len();
        let back = Editor::from_document(&document).expect("reads back");
        assert_eq!(back.project().active().clips.len(), 200);
    }
    let millis = started.elapsed().as_secs_f64() * 1e3 / f64::from(rounds);
    Measure {
        name: "save and open a 200-clip document",
        value: millis,
        unit: "ms",
        budget: Budget::AtMost(60.0),
        note: format!("{} KB of JSON", bytes / 1024),
    }
}

/// What the window publishes for a 200-clip cut: only the clips near the
/// view, decided by a pure test over the plan's own numbers, so a long
/// cut costs the window what is on screen.
fn publish_window_of_200() -> Measure {
    let timeline = timeline_of(200, 4);
    let rounds = 2000;
    // A thousand pixels at a twentieth of a second each, looking at the
    // middle of the cut, a screen either side: sixty seconds of it.
    let (from, to) = (25.0 - 50.0, 25.0 + 100.0);
    let started = Instant::now();
    let mut shown = 0;
    for _ in 0..rounds {
        shown = timeline
            .tracks()
            .flat_map(|(_, track)| track.clips().iter().filter_map(|id| timeline.clip(*id)))
            .filter(|clip| {
                let start = clip.start.as_f64();
                let end = start + clip.duration.as_f64();
                end >= from && start <= to
            })
            .count();
    }
    let micros = started.elapsed().as_secs_f64() * 1e6 / f64::from(rounds as u32);
    Measure {
        name: "choose the visible clips of 200 for one publish",
        value: micros,
        unit: "us",
        budget: Budget::AtMost(50.0),
        note: format!("{shown} of 200 published"),
    }
}

/// Drawing an hour-long file's waveform across a screen: the pyramid hands
/// the level that fits the column, so the path reads thousands of buckets
/// and not millions, at every zoom.
fn waveform_of_an_hour() -> Measure {
    let buckets = 3600 * 1000;
    let mut min = vec![-0.2f32; buckets];
    let mut max = vec![0.2f32; buckets];
    for index in (0..buckets).step_by(997) {
        min[index] = -0.9;
        max[index] = 0.9;
    }
    let started = Instant::now();
    let pyramid = concat_media::Pyramid::of(concat_media::Peaks {
        min,
        max,
        buckets_per_second: 1000.0,
    });
    let built = started.elapsed();
    let rounds = 50;
    let started = Instant::now();
    let mut columns_read = 0;
    for round in 0..rounds {
        // Every zoom from the whole hour across the screen down to a
        // second across it, each a column a pixel.
        let seconds = 3600.0 / f64::from(1 << (round % 12)) as f32;
        let level = pyramid.level_for(seconds / 2048.0);
        for column in 0..2048 {
            let from = column as f32 * seconds / 2048.0;
            let (low, high) = level.extremes(from, from + seconds / 2048.0);
            assert!(high >= 0.2 && low <= -0.2, "every column has sound in it");
            columns_read += 1;
        }
    }
    let micros = started.elapsed().as_secs_f64() * 1e6 / f64::from(rounds);
    Measure {
        name: "read an hour's waveform across 2048 columns",
        value: micros,
        unit: "us",
        budget: Budget::AtMost(2000.0),
        note: format!(
            "{} levels built in {:.0} ms; {} columns read",
            pyramid.depth(),
            built.as_secs_f64() * 1e3,
            columns_read / rounds
        ),
    }
}

/// The synthetic media the heavier scenarios read: ninety frames of
/// 1080p H.264, three seconds at thirty.
struct Media {
    path: PathBuf,
    frames: u32,
}

impl Media {
    const WIDTH: u32 = 1920;
    const HEIGHT: u32 = 1080;

    fn synthesise() -> Media {
        let dir = std::env::temp_dir().join(format!("concat-perf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let path = dir.join("perf-1080p.mp4");
        let frames = 90;
        let mut encoder = Encoder::create(
            &path,
            Self::WIDTH,
            Self::HEIGHT,
            FrameRate::THIRTY,
            &EncodeOptions {
                preset: "veryfast".to_owned(),
                hardware: false,
                ..EncodeOptions::default()
            },
        )
        .expect("the linked FFmpeg encodes h264");
        for index in 0..frames {
            encoder
                .write_frame(&gradient(Self::WIDTH, Self::HEIGHT, index as u8))
                .expect("writes");
        }
        encoder.finish().expect("finishes");
        Media { path, frames }
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// A picture with something in it everywhere, different per `seed`.
fn gradient(width: u32, height: u32, seed: u8) -> Frame {
    let mut frame = Frame::black(width, height);
    let stride = width as usize * 4;
    let pixels = frame.pixels_mut();
    for y in 0..height as usize {
        for x in 0..width as usize {
            let at = y * stride + x * 4;
            pixels[at] = ((x * 255) / width as usize) as u8;
            pixels[at + 1] = ((y * 255) / height as usize) as u8;
            pixels[at + 2] = seed.wrapping_mul(3);
            pixels[at + 3] = 255;
        }
    }
    frame
}

fn decode_with(
    path: &Path,
    options: &DecodeOptions,
    expected: u32,
) -> (f64, Option<concat_media::HwDevice>) {
    let started = Instant::now();
    let mut decoder = Decoder::open(path, options).expect("opens");
    let mut count = 0;
    while let Some(_frame) = decoder.next_frame().expect("decodes") {
        count += 1;
    }
    // The encoder's last frame can stay in its delay; one short is the
    // codec's rounding, two is a bug.
    assert!(
        count + 1 >= expected,
        "{count} of {expected} frames came out"
    );
    (
        f64::from(count) / started.elapsed().as_secs_f64(),
        decoder.hardware(),
    )
}

/// Decoding 1080p H.264 in software at the file's own size: frames a
/// second, on every core.
fn decode_software(media: &Media) -> Measure {
    let (fps, _) = decode_with(
        &media.path,
        &DecodeOptions::default().in_software(),
        media.frames,
    );
    Measure {
        name: "decode 1080p h264 in software",
        value: fps,
        unit: "fps",
        budget: Budget::AtLeast(120.0),
        note: "the walk from a keyframe a scrub pays".to_owned(),
    }
}

/// The same on the platform's hardware, where this build has some.
fn decode_hardware(media: &Media) -> Option<Measure> {
    let device = concat_media::HwDevice::platform_default().filter(|device| device.linked())?;
    let (fps, used) = decode_with(
        &media.path,
        &DecodeOptions::default().accelerated(device),
        media.frames,
    );
    Some(Measure {
        name: "decode 1080p h264 on the hardware",
        value: fps,
        unit: "fps",
        budget: Budget::AtLeast(120.0),
        note: match used {
            Some(device) => format!("on {}", device.label()),
            None => "fell back to software".to_owned(),
        },
    })
}

/// A scrub through the reader pool: forward over the file, back over it,
/// then again with an effect knob turned. The second pass must decode
/// nothing, and the knob must cost a filter and not a decode.
fn scrub(media: &Media) -> Vec<Measure> {
    let pool = ReaderPool::new(512 * 1024 * 1024, 2);
    let rate = FrameRate::THIRTY;
    let request = |index: u32, chain: Option<&str>| {
        FrameRequest::new(&media.path, rate.time_of_frame(i64::from(index)), 960, 540)
            .covering(960, 540)
            .filtered(chain)
    };
    let started = Instant::now();
    for index in 0..media.frames {
        pool.frame(&request(index, None)).expect("decodes");
    }
    let first = started.elapsed();
    let warmed = pool.stats();
    let started = Instant::now();
    for index in (0..media.frames).rev() {
        pool.frame(&request(index, None)).expect("cached");
    }
    let back = started.elapsed();
    let revisit = pool.stats().since(warmed);
    let before_knob = pool.stats();
    for index in 0..media.frames {
        pool.frame(&request(index, Some("eq=brightness=0.1")))
            .expect("treated");
    }
    let knob = pool.stats().since(before_knob);
    vec![
        Measure {
            name: "scrub forward over 90 frames, cold",
            value: first.as_secs_f64() * 1e3 / f64::from(media.frames),
            unit: "ms/frame",
            budget: Budget::AtMost(25.0),
            note: format!("{} decoded", warmed.decoded),
        },
        Measure {
            name: "scrub back over covered ground",
            value: back.as_secs_f64() * 1e6 / f64::from(media.frames),
            unit: "us/frame",
            budget: Budget::AtMost(200.0),
            note: format!("{} decoded, {} hits", revisit.decoded, revisit.source_hits),
        },
        Measure {
            name: "decodes when an effect knob is turned",
            value: knob.decoded as f64,
            unit: "frames",
            budget: Budget::AtMost(0.0),
            note: format!("{} treated from cached sources", knob.treated),
        },
    ]
}

/// The plan the compositors are timed on: a 1080p ground, a placed and
/// turned picture over it through the sepia kernel, and a small one on top.
fn compose_plan() -> FramePlan {
    let mut plan = FramePlan::empty(1920, 1080);
    plan.layers.push(PlannedLayer::picture(
        concat_render::detached_clip(),
        Arc::new(gradient(1920, 1080, 0)),
    ));
    let mut placed = PlannedLayer::picture(
        concat_render::detached_clip(),
        Arc::new(gradient(1280, 720, 1)),
    );
    placed.track = 1;
    placed.transform = Transform {
        scale: 0.6,
        rotation: 15.0,
        offset_x: 0.1,
        ..Transform::default()
    };
    placed.effects = concat_effects::Catalogue::builtin().shader_passes_at(
        &[AppliedFilter::new("concat.sepia")],
        0.0,
        None,
    );
    plan.layers.push(placed);
    let mut small = PlannedLayer::picture(
        concat_render::detached_clip(),
        Arc::new(gradient(320, 180, 2)),
    );
    small.track = 2;
    small.opacity = 0.7;
    small.transform = Transform {
        offset_x: -0.3,
        offset_y: 0.3,
        ..Transform::default()
    };
    plan.layers.push(small);
    plan
}

fn time_render(compositor: &mut dyn Compositor, plan: &FramePlan, rounds: u32) -> f64 {
    compositor.render(plan);
    let started = Instant::now();
    for _ in 0..rounds {
        compositor.render(plan);
    }
    started.elapsed().as_secs_f64() * 1e3 / f64::from(rounds)
}

/// The CPU reference drawing the plan: the fallback's speed, and what a
/// machine with no GPU gets.
fn compose_cpu() -> Measure {
    let plan = compose_plan();
    let millis = time_render(&mut CpuCompositor, &plan, 3);
    Measure {
        name: "compose 1080p, 3 layers, sepia, on the CPU",
        value: millis,
        unit: "ms",
        budget: Budget::AtMost(400.0),
        note: "the reference; a machine without a GPU sees this".to_owned(),
    }
}

/// The GPU drawing the same plan and reading it back, where there is one.
fn compose_gpu() -> Option<Measure> {
    let mut gpu = concat_render::WgpuCompositor::new()?;
    let plan = compose_plan();
    let millis = time_render(&mut gpu, &plan, 20);
    Some(Measure {
        name: "compose 1080p, 3 layers, sepia, on the GPU",
        value: millis,
        unit: "ms",
        budget: Budget::AtMost(25.0),
        note: "with the readback an export pays".to_owned(),
    })
}

/// A real export of a three-second cut through the session, as the
/// window runs it: frames a second at 720p.
fn export(media: &Media) -> Measure {
    use concat_host::{Session, projects};
    let dir = media.path.parent().expect("a dir").join("project");
    std::fs::create_dir_all(&dir).expect("a dir");
    let info = projects::create(&dir.to_string_lossy(), "perf", 1280, 720, 30, 1).expect("creates");
    let mut session = Session::open_info(&info).expect("opens");
    let summary = concat_host::media::probe(&media.path.to_string_lossy()).expect("probes");
    let media_id = session
        .apply(Command::AddMedia {
            item: summary.to_new_media(),
        })
        .expect("adds")
        .created_id
        .expect("a media id");
    session
        .apply(Command::AddClipAtFirstFree {
            media_id: media_id.clone(),
            start: 0.0,
        })
        .expect("places");
    session
        .apply(Command::AddClipAtFirstFree {
            media_id,
            start: 1.0,
        })
        .expect("places a second, overlapping");
    let output = dir.join("out.mp4");
    let spec = concat_host::export::ExportSpec {
        output: output.to_string_lossy().into_owned(),
        crf: 23,
        preset: "veryfast".to_owned(),
        codec: concat_media::VideoCodec::H264,
        ten_bit: false,
        rate_mode: concat_media::RateMode::Vbr,
        bitrate_kbps: 0,
        color_range: concat_media::ColorRange::Limited,
    };
    let request = concat_host::export::request(&session, &spec, Vec::new());
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let mut frames = 0;
    concat_host::export::run(&request, &cancel, |progress| {
        if progress.stage == "rendering" {
            frames = progress.total;
        }
    })
    .expect("exports");
    let elapsed = started.elapsed();
    let written = std::fs::metadata(&output)
        .map(|meta| meta.len())
        .unwrap_or(0);
    Measure {
        name: "export a 4 s 720p cut of two overlapping clips",
        value: frames as f64 / elapsed.as_secs_f64().max(1e-9),
        unit: "fps",
        budget: Budget::AtLeast(30.0),
        note: format!("{frames} frames, {} KB, in {:.1?}", written / 1024, elapsed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The quick scenarios hold their budgets under the test runner: a
    /// regression in planning, undo, the document or the publish window
    /// fails the build here, without waiting for anyone to run the table.
    #[test]
    fn the_quick_budgets_hold() {
        for measure in [
            plan_200_clips(),
            undo_200_edits(),
            document_round_trip(),
            publish_window_of_200(),
        ] {
            assert!(
                measure.holds(),
                "{}: {:.2} {} is outside its budget",
                measure.name,
                measure.value,
                measure.unit
            );
        }
    }

    #[test]
    fn a_budget_reads_the_right_way_round() {
        let under = Measure {
            name: "",
            value: 1.0,
            unit: "",
            budget: Budget::AtMost(2.0),
            note: String::new(),
        };
        let over = Measure {
            name: "",
            value: 3.0,
            unit: "",
            budget: Budget::AtMost(2.0),
            note: String::new(),
        };
        let enough = Measure {
            name: "",
            value: 3.0,
            unit: "",
            budget: Budget::AtLeast(2.0),
            note: String::new(),
        };
        let short = Measure {
            name: "",
            value: 1.0,
            unit: "",
            budget: Budget::AtLeast(2.0),
            note: String::new(),
        };
        assert!(under.holds() && enough.holds());
        assert!(!over.holds() && !short.holds());
    }
}

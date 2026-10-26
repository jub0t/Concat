// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Stabilization end to end: a shaky shot made here, its clip stabilized
//! by a command, the camera's movement measured by the host's job, and the
//! export and the monitor both checked against what the shake was.
//!
//! The shot is a still, textured scene filmed by a camera that wanders a
//! few pixels about one spot, by a known amount each frame, encoded with
//! H.264 as a phone would. So the measurement can be held to the numbers
//! the shot was made with, and a stabilized export to the scene keeping
//! still.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use concat_core::frame::Frame;
use concat_core::time::FrameRate;
use concat_host::export::{self, ExportSpec};
use concat_host::preview::{FrameSpec, Monitor};
use concat_host::session::Session;
use concat_host::stabilize::Stabilizers;
use concat_host::{media, projects};
use concat_media::{
    DecodeOptions, Decoder, EncodeOptions, Encoder, FrameSink, FrameSource, RateMode, VideoCodec,
};
use concat_project::commands::Command;
use concat_project::model::{ColorSpace, Stabilize};
use concat_vision::stabilize::{Motion, Step};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FRAMES: usize = 60;

/// Where the camera sits at frame `k`, in pixels: a wander of a few
/// pixels about one spot, never the same twice running.
fn camera(k: usize) -> (f64, f64) {
    let k = k as f64;
    (
        4.0 * (1.9 * k).sin() + 2.0 * (0.7 * k).sin(),
        3.0 * (1.3 * k).cos() + 1.0 * (0.4 * k).sin(),
    )
}

/// The scene, a smooth texture with corners all over it.
fn scene(x: f64, y: f64) -> f64 {
    let mut v = 120.0;
    for (k, (fx, fy, phase)) in [
        (0.071, 0.023, 0.3),
        (-0.031, 0.083, 1.1),
        (0.11, 0.097, 2.0),
        (0.017, -0.13, 0.7),
        (0.21, 0.05, 1.7),
    ]
    .iter()
    .enumerate()
    {
        v += (28.0 / (k as f64 + 1.0).sqrt()) * (x * fx + y * fy + phase).sin();
    }
    v + 30.0
        * (3.0 * (x * std::f64::consts::PI / 17.0).sin() * (y * std::f64::consts::PI / 13.0).sin())
            .tanh()
}

/// The shaky shot: frame `k` shows the scene moved by `camera(k)`.
fn shoot(path: &Path) {
    let options = EncodeOptions {
        codec: VideoCodec::H264,
        preset: "ultrafast".to_owned(),
        crf: 14,
        rate_mode: RateMode::Vbr,
        bitrate_kbps: 0,
        ten_bit: false,
        color_range: concat_media::ColorRange::Limited,
        hardware: false,
        threads: 0,
        software_only: false,
    };
    let mut encoder = Encoder::create(path, WIDTH, HEIGHT, FrameRate::THIRTY, &options)
        .expect("the linked FFmpeg encodes");
    for k in 0..FRAMES {
        let (dx, dy) = camera(k);
        let mut frame = Frame::black(WIDTH, HEIGHT);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let v = scene(f64::from(x) - dx, f64::from(y) - dy).clamp(0.0, 255.0) as u8;
                frame.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        encoder.write_frame(&frame).expect("writes a frame");
    }
    encoder.finish().expect("finishes the shot");
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("concat-stabilize-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("makes the scratch folder");
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every frame of a file.
fn frames_of(path: &str) -> Vec<Frame> {
    let mut decoder = Decoder::open(path, &DecodeOptions::default()).expect("opens");
    let mut frames = Vec::new();
    while let Some(frame) = decoder.next_frame().expect("decodes") {
        frames.push(frame);
    }
    frames
}

/// The mean difference in level between two frames over their middle
/// half, where no edge a correction moves in can reach.
fn difference(a: &[u8], b: &[u8]) -> f64 {
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let mut sum = 0.0;
    let mut count = 0.0;
    for y in h / 4..3 * h / 4 {
        for x in w / 4..3 * w / 4 {
            let at = (y * w + x) * 4;
            sum += (f64::from(a[at]) - f64::from(b[at])).abs();
            count += 1.0;
        }
    }
    sum / count
}

/// How much the picture moves frame to frame over a run of frames.
fn wobble(frames: &[Vec<u8>]) -> f64 {
    let pairs: Vec<f64> = frames
        .windows(2)
        .map(|pair| difference(&pair[0], &pair[1]))
        .collect();
    pairs.iter().sum::<f64>() / pairs.len() as f64
}

fn export_to(session: &Session, output: &Path) -> String {
    let spec = ExportSpec {
        output: output.to_string_lossy().into_owned(),
        crf: 12,
        preset: "ultrafast".to_owned(),
        codec: VideoCodec::H264,
        ten_bit: false,
        rate_mode: RateMode::Vbr,
        bitrate_kbps: 0,
        color_range: concat_media::ColorRange::Limited,
        hdr: false,
    };
    let request = export::request(session, &spec, Vec::new());
    export::run(&request, &AtomicBool::new(false), |_| {}).expect("exports")
}

#[test]
fn a_shaky_shot_is_measured_and_held_still() {
    let scratch = Scratch::new("shot");
    let shot = scratch.0.join("shaky.mp4");
    shoot(&shot);

    let info = projects::create(
        scratch.0.to_str().expect("utf-8"),
        "Steady",
        WIDTH,
        HEIGHT,
        30,
        1,
        [0, 0, 0, 255],
    )
    .expect("creates the project");
    let mut session = Session::open_info(&info).expect("opens the project");
    let summary = media::probe(shot.to_str().expect("utf-8")).expect("probes");
    let media_id = session
        .apply(Command::AddMedia {
            item: summary.to_new_media(),
        })
        .expect("imports")
        .created_id
        .expect("an id");
    let clip_id = session
        .apply(Command::AddClipAtFirstFree {
            media_id,
            start: 0.0,
        })
        .expect("adds")
        .created_id
        .expect("an id");

    // Shown as shot first, for the comparison.
    let plain = export_to(&session, &scratch.0.join("plain.mp4"));

    session
        .apply(Command::SetClipStabilize {
            clip_id: clip_id.clone(),
            stabilize: Some(Stabilize {
                smoothness: 0.6,
                max_zoom: 1.3,
                ..Stabilize::default()
            }),
        })
        .expect("stabilizes");
    let project_dir = PathBuf::from(session.path());

    // Not measured yet: the clip shows as shot.
    let requests = Stabilizers::requests(session.project(), &project_dir);
    assert_eq!(requests.len(), 1);
    let (_, request) = &requests[0];
    assert!(!Stabilizers::outstanding(request).is_empty());

    let stabilizers = Stabilizers::new();
    let started = std::time::Instant::now();
    let mut last = 0.0f32;
    let measured = stabilizers
        .analyse(request, &mut |fraction| last = fraction)
        .expect("measures");
    let took = started.elapsed();
    eprintln!(
        "measured {measured} frames in {:.0} ms ({:.2} ms a frame, decode included)",
        took.as_secs_f64() * 1000.0,
        took.as_secs_f64() * 1000.0 / measured as f64
    );
    assert_eq!(measured, FRAMES);
    assert!((last - 1.0).abs() < 1e-6);
    assert!(Stabilizers::outstanding(request).is_empty(), "all measured");
    assert!(!Stabilizers::tripped(request), "the tripwire is cleared");

    // The measurement against the shot: each frame's movement is the
    // camera's step, to within a fifth of a pixel through H.264.
    let motion = Motion::read(&Stabilizers::dir(request)).expect("cached");
    assert_eq!(motion.frames.len(), FRAMES);
    assert_eq!(motion.frames[0].step, Step::Start);
    let h = f64::from(HEIGHT);
    let mut worst: f64 = 0.0;
    for k in 1..FRAMES {
        let frame = motion.frames[k];
        assert_eq!(frame.step, Step::Tracked, "frame {k}");
        let (x0, y0) = camera(k - 1);
        let (x1, y1) = camera(k);
        let (dx, dy) = (frame.motion.tx * h, frame.motion.ty * h);
        worst = worst
            .max((dx - (x1 - x0)).abs())
            .max((dy - (y1 - y0)).abs());
        assert!(frame.motion.angle().abs() < 0.002, "frame {k} turned");
        assert!(
            (frame.motion.scale() - 1.0).abs() < 0.002,
            "frame {k} zoomed"
        );
    }
    eprintln!("worst step error: {worst:.3} px");
    assert!(worst < 0.2, "a step measured {worst:.3} px off");

    // A second run has nothing to do.
    assert_eq!(stabilizers.analyse(request, &mut |_| {}), Ok(0));

    // Exported steadied: the scene holds still where the shot wobbled.
    let steady = export_to(&session, &scratch.0.join("steady.mp4"));
    let plain: Vec<Vec<u8>> = frames_of(&plain)
        .into_iter()
        .map(Frame::into_pixels)
        .collect();
    let steady: Vec<Vec<u8>> = frames_of(&steady)
        .into_iter()
        .map(Frame::into_pixels)
        .collect();
    assert_eq!(steady.len(), FRAMES);
    // Away from the ends, where the smoothing's reflection still lets a
    // little of the path through.
    let (shaky, held) = (wobble(&plain[10..50]), wobble(&steady[10..50]));
    eprintln!("frame-to-frame difference: as shot {shaky:.2}, stabilized {held:.2}");
    assert!(
        held < shaky * 0.35,
        "as shot {shaky:.2}, stabilized {held:.2}"
    );

    // The monitor draws what the export wrote: the same frame, to within
    // what the encoder changed, and nowhere near the frame as shot.
    let monitor = Monitor::new();
    let clips = Arc::new(session.flattened_clips());
    for k in [12usize, 31, 47] {
        let pixels = monitor
            .frame(
                Arc::clone(&clips),
                &session.settings(),
                FrameSpec {
                    canvas: [0, 0, 0, 255],
                    time: k as f64 / 30.0,
                    width: WIDTH,
                    height: HEIGHT,
                    moving: false,
                    proxy: false,
                    color_space: ColorSpace::default(),
                },
            )
            .expect("the monitor draws");
        let against_export = difference(&pixels, &steady[k]);
        let against_shot = difference(&pixels, &plain[k]);
        eprintln!("frame {k}: monitor vs export {against_export:.2}, vs as shot {against_shot:.2}");
        assert!(against_export < 2.0, "frame {k}: {against_export:.2}");
        assert!(against_shot > against_export * 2.0, "frame {k}");
    }
}

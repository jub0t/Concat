// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How fast an ordinary project exports: 1080p at 30 fps, a cut every few
//! seconds through footage in H.264, HEVC, AV1 and VP9 with alpha, and a
//! title every six seconds, some of them moving. Issue #754 was such a
//! project exporting at a frame and a half a second on a fast machine.
//!
//! Ignored: it is a measurement, not a check. Run it optimised, with the
//! log at info so the export's own timing line shows:
//!
//! ```text
//! CONCAT_LOG=info cargo test --profile quick -p concat-host --test export_speed -- --ignored --nocapture
//! ```
//!
//! `CONCAT_SPEED_SOURCES` names a folder holding `h264.mp4`, `hevc.mp4`,
//! `av1.mp4` and `vp9a.webm` to use instead of making them (making them
//! takes a while), `CONCAT_SPEED_SECONDS` how long the timeline is (60 by
//! default), `CONCAT_SPEED_LEAVE` what to leave out, any of `titles`,
//! `motion`, `vp9`, `av1`, `cuts` (one long clip of each codec rather
//! than a cut every few seconds), and `CONCAT_SPEED_ADD` what to put in,
//! any of `transitions`, `effects`, `blur` (pictures in the frame over
//! their own blurred copy) and `curves` (speed ramps). The file is HEVC
//! unless `CONCAT_SPEED_CODEC` says `h264` or `av1`; the footage is read on
//! the hardware unless `CONCAT_SPEED_SOFTWARE` is set; and
//! `CONCAT_SPEED_CANCEL_AT` cancels the export at that frame.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use concat_core::frame::Frame;
use concat_core::time::FrameRate;
use concat_host::dirs::AppDirs;
use concat_host::export::{self, ExportSpec};
use concat_host::session::Session;
use concat_host::titles::Titles;
use concat_host::{media, projects};
use concat_media::{EncodeOptions, Encoder, FrameSink, RateMode, VideoCodec};
use concat_project::commands::{ClipPatch, Command, TrimEdge};
use concat_project::model::{ClipLoop, ClipMotion, ClipMove, TextStyle};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
/// How long each source is.
const SOURCE_SECONDS: u32 = 12;

/// A source `seconds` long in `codec`: bars that move and a square that
/// travels, so every frame differs from the last as footage does. The
/// alpha codec's is a square on nothing.
fn source(path: &Path, codec: VideoCodec) {
    let options = EncodeOptions {
        codec,
        preset: "ultrafast".to_owned(),
        crf: 23,
        rate_mode: RateMode::Vbr,
        bitrate_kbps: 0,
        ten_bit: false,
        color_range: concat_media::ColorRange::Limited,
        hardware: true,
        threads: 0,
        software_only: false,
    };
    let rate = FrameRate::THIRTY;
    let mut encoder =
        Encoder::create(path, WIDTH, HEIGHT, rate, &options).expect("the linked FFmpeg encodes");
    let alpha = codec.keeps_alpha();
    let mut pixels = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    for index in 0..SOURCE_SECONDS * 30 {
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let at = ((y * WIDTH + x) * 4) as usize;
                let square = (x + 1920 - index * 9 % 1920) % 1920 < 400 && (300..700).contains(&y);
                let pixel = if alpha {
                    if square {
                        [240, 80, 40, 255]
                    } else {
                        [0, 0, 0, 0]
                    }
                } else if square {
                    [255, 255, 255, 255]
                } else {
                    let band = ((x + index * 4) / 120) as u8;
                    [
                        band.wrapping_mul(37),
                        (y / 4) as u8,
                        band.wrapping_mul(91),
                        255,
                    ]
                };
                pixels[at..at + 4].copy_from_slice(&pixel);
            }
        }
        let frame = Frame::from_rgba(WIDTH, HEIGHT, pixels.clone()).expect("a frame");
        encoder.write_frame(&frame).expect("writes a frame");
    }
    encoder.finish().expect("finishes the source");
}

fn sources(dir: &Path) -> [PathBuf; 4] {
    if let Ok(given) = std::env::var("CONCAT_SPEED_SOURCES") {
        let given = PathBuf::from(given);
        return ["h264.mp4", "hevc.mp4", "av1.mp4", "vp9a.webm"].map(|name| given.join(name));
    }
    [
        ("h264.mp4", VideoCodec::H264),
        ("hevc.mp4", VideoCodec::Hevc),
        ("av1.mp4", VideoCodec::Av1),
        ("vp9a.webm", VideoCodec::Vp9Alpha),
    ]
    .map(|(name, codec)| {
        let path = dir.join(name);
        source(&path, codec);
        path
    })
}

struct Studio {
    session: Session,
}

impl Studio {
    fn apply(&mut self, command: Command) -> Option<String> {
        let what = format!("{command:?}");
        self.session
            .apply(command)
            .unwrap_or_else(|error| panic!("{what}: {error}"))
            .created_id
    }

    fn import(&mut self, path: &Path) -> String {
        let summary = media::probe(path.to_str().expect("utf-8"))
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        self.apply(Command::AddMedia {
            item: summary.to_new_media(),
        })
        .expect("an imported file has an id")
    }

    /// `media` at `start`, `length` long, on the first free lane.
    fn clip(&mut self, media: &str, start: f64, length: f64) -> String {
        let before: Vec<String> = self.ids();
        self.apply(Command::AddClipAtFirstFree {
            media_id: media.to_owned(),
            start,
        });
        let id = self
            .ids()
            .into_iter()
            .find(|id| !before.contains(id))
            .expect("a clip was added");
        let clip = self
            .session
            .project()
            .active()
            .clips
            .iter()
            .find(|c| c.id == id)
            .unwrap();
        let trim = clip.duration - length;
        if trim > 0.0 {
            self.apply(Command::TrimClip {
                clip_id: id.clone(),
                edge: TrimEdge::End,
                delta: -trim,
                ripple: false,
            });
        }
        id
    }

    fn ids(&self) -> Vec<String> {
        self.session
            .project()
            .active()
            .clips
            .iter()
            .map(|clip| clip.id.clone())
            .collect()
    }
}

/// Each source read as the export reads it - paced at 30 fps, at the
/// frame's size - on its own: frames a second.
#[test]
#[ignore = "a measurement: run by hand, optimised"]
fn each_source_decodes_at_speed() {
    concat_media::set_hardware_decode(std::env::var("CONCAT_SPEED_SOFTWARE").is_err());
    let root = std::env::temp_dir().join(format!("concat-decode-speed-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("a scratch folder");
    for path in sources(&root) {
        for deep in [false, true] {
            let options = concat_media::DecodeOptions::default()
                .starting_at(concat_core::time::Rational::new(1, 2))
                .scaled_to(WIDTH, HEIGHT)
                .at_rate(FrameRate::THIRTY)
                .deep(deep);
            let begun = std::time::Instant::now();
            let mut decoder = concat_media::Decoder::open(&path, &options).expect("opens");
            let opened = begun.elapsed().as_secs_f64();
            let mut frames = 0;
            while frames < 300 {
                use concat_media::FrameSource;
                if decoder.next_frame().expect("decodes").is_none() {
                    break;
                }
                frames += 1;
            }
            let took = begun.elapsed().as_secs_f64();
            eprintln!(
                "{} deep {deep}: opened in {:.0} ms, {frames} frames at {:.0} a second",
                path.file_name().unwrap().to_string_lossy(),
                opened * 1000.0,
                f64::from(frames) / (took - opened)
            );
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "a measurement: run by hand, optimised"]
fn an_ordinary_project_exports_at_speed() {
    let filter = std::env::var(concat_log::LEVEL_VARIABLE).unwrap_or_else(|_| "info".into());
    let _ = concat_log::open(concat_log::Config {
        folder: None,
        filter: concat_log::Filter::parse(&filter).0,
        platform: None,
        home: None,
    });
    // As the window has it: on unless asked otherwise.
    concat_media::set_hardware_decode(std::env::var("CONCAT_SPEED_SOFTWARE").is_err());
    let leave = std::env::var("CONCAT_SPEED_LEAVE").unwrap_or_default();
    let leaves = |what: &str| leave.split(',').any(|word| word.trim() == what);
    let seconds: f64 = std::env::var("CONCAT_SPEED_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(60.0);

    let root = std::env::temp_dir().join(format!("concat-export-speed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a scratch folder");
    let made = std::time::Instant::now();
    let [h264, hevc, av1, vp9a] = sources(&root);
    eprintln!("sources ready in {:.1} s", made.elapsed().as_secs_f64());

    let info = projects::create(
        root.to_str().expect("utf-8"),
        "Speed",
        WIDTH,
        HEIGHT,
        30,
        1,
        [0, 0, 0, 255],
    )
    .expect("creates the project");
    let mut studio = Studio {
        session: Session::open_info(&info).expect("opens the project"),
    };
    let mut footage = vec![studio.import(&h264), studio.import(&hevc)];
    if !leaves("av1") {
        footage.push(studio.import(&av1));
    }
    let overlay = (!leaves("vp9")).then(|| studio.import(&vp9a));

    // The issue's density: 94 clips and 76 titles over eight minutes, a
    // clip every five seconds and a title every six. Three in four clips
    // are the picture, cut end to end; every fourth is an overlay.
    let mut clips = 0;
    let mut at = 0.0;
    let cut = if leaves("cuts") {
        f64::from(SOURCE_SECONDS)
    } else {
        20.0 / 3.0
    };
    let mut which = 0;
    let add = std::env::var("CONCAT_SPEED_ADD").unwrap_or_default();
    let adds = |what: &str| add.split(',').any(|word| word.trim() == what);
    while at < seconds {
        let length = cut.min(seconds - at);
        let id = studio.clip(&footage[which % footage.len()], at, length);
        clips += 1;
        let mut patch = ClipPatch::default();
        if adds("transitions") && which > 0 {
            patch.transition_in = Some(Some(concat_project::model::Transition {
                id: "cross-fade".into(),
                duration: 0.5,
                params: Default::default(),
            }));
        }
        if adds("effects") && which % 2 == 0 {
            patch.video_effects = Some(vec![concat_project::model::AppliedFilter {
                id: "concat.glow".into(),
                params: Default::default(),
                enabled: true,
                keys: Default::default(),
            }]);
        }
        if adds("blur") && which % 2 == 1 {
            patch.background = Some(Some(concat_project::model::Background::Blur {
                amount: 0.5,
            }));
            studio.apply(Command::SetClipTransform {
                clip_id: id.clone(),
                scale: Some(0.7),
                offset_x: None,
                offset_y: None,
                rotation: None,
                stretch_x: None,
                stretch_y: None,
            });
        }
        if patch != ClipPatch::default() {
            studio.apply(Command::UpdateClip {
                clip_id: id.clone(),
                patch,
            });
        }
        if adds("curves") && which % 3 == 2 {
            studio.apply(Command::SetClipSpeedCurve {
                clip_id: id.clone(),
                curve: Some(vec![
                    concat_project::model::SpeedPoint {
                        at: 0.0,
                        speed: 1.0,
                        slope: None,
                    },
                    concat_project::model::SpeedPoint {
                        at: 0.5,
                        speed: 0.5,
                        slope: None,
                    },
                    concat_project::model::SpeedPoint {
                        at: 1.0,
                        speed: 1.0,
                        slope: None,
                    },
                ]),
                ripple: false,
            });
        }
        if let Some(overlay) = &overlay
            && which % 2 == 1
        {
            studio.clip(overlay, at + 1.0, 4.0_f64.min(seconds - at - 1.0).max(0.5));
            clips += 1;
        }
        which += 1;
        at += length;
    }

    let mut titles = 0;
    if !leaves("titles") {
        let mut at = 0.5;
        let mut index = 0;
        while at + 3.5 <= seconds {
            let id = studio
                .apply(Command::AddTextClip {
                    track_id: None,
                    above: true,
                    start: at,
                    style: Some(TextStyle {
                        content: format!("Title number {index}\nwith a second line"),
                        ..TextStyle::default()
                    }),
                    duration: Some(3.5),
                    offset_y: Some(if index % 2 == 0 { 0.3 } else { -0.3 }),
                })
                .expect("a title has an id");
            titles += 1;
            // Half of them move: in, out, and every fourth a loop.
            if !leaves("motion") && index % 2 == 0 {
                studio.apply(Command::UpdateClip {
                    clip_id: id,
                    patch: ClipPatch {
                        motion: Some(Some(ClipMotion {
                            enter: Some(ClipMove {
                                kind: "pop".into(),
                                seconds: 0.5,
                            }),
                            exit: Some(ClipMove {
                                kind: "fade".into(),
                                seconds: 0.5,
                            }),
                            cycle: (index % 4 == 0).then(|| ClipLoop {
                                kind: "pulse".into(),
                                seconds: 1.0,
                                amount: 1.0,
                            }),
                        })),
                        ..ClipPatch::default()
                    },
                });
            }
            // Every fifth overlaps the next.
            at += if index % 5 == 4 { 2.0 } else { 6.0 };
            index += 1;
        }
    }

    let painter = Titles::new(&AppDirs::under(&root.join("app")));
    let painting = std::time::Instant::now();
    let title_clips = painter
        .clips(studio.session.project(), WIDTH, HEIGHT)
        .into_iter()
        .map(|title| title.clip)
        .collect();
    eprintln!(
        "{clips} clips, {titles} titles over {seconds} s; titles painted in {:.2} s",
        painting.elapsed().as_secs_f64()
    );

    let codec = match std::env::var("CONCAT_SPEED_CODEC").as_deref() {
        Ok("h264") => VideoCodec::H264,
        Ok("av1") => VideoCodec::Av1,
        _ => VideoCodec::Hevc,
    };
    let spec = ExportSpec {
        output: root.join("out.mp4").to_string_lossy().into_owned(),
        crf: 23,
        preset: "medium".to_owned(),
        codec,
        ten_bit: false,
        rate_mode: RateMode::Vbr,
        bitrate_kbps: 0,
        color_range: concat_media::ColorRange::Limited,
        hdr: false,
    };
    let request = export::request(&studio.session, &spec, title_clips);
    let begun = std::time::Instant::now();
    let mut stages: Vec<(&'static str, std::time::Instant)> = Vec::new();
    // `CONCAT_SPEED_CANCEL_AT`: a frame to press Cancel at, to see the
    // line a cancelled export leaves.
    let cancel_at: Option<i64> = std::env::var("CONCAT_SPEED_CANCEL_AT")
        .ok()
        .and_then(|value| value.parse().ok());
    let cancel = AtomicBool::new(false);
    let exported = export::run(&request, &cancel, |progress| {
        if stages
            .last()
            .is_none_or(|(stage, _)| *stage != progress.stage)
        {
            stages.push((progress.stage, std::time::Instant::now()));
        }
        if cancel_at.is_some_and(|at| progress.frame >= at) {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    if cancel_at.is_some() {
        eprintln!("cancelled: {exported:?}");
        return;
    }
    exported.unwrap_or_else(|error| panic!("the export failed: {error}"));
    let took = begun.elapsed().as_secs_f64();
    let frames = seconds * 30.0;
    for pair in stages.windows(2) {
        eprintln!(
            "  {}: {:.2} s",
            pair[0].0,
            (pair[1].1 - pair[0].1).as_secs_f64()
        );
    }
    eprintln!(
        "exported {frames} frames in {took:.2} s: {:.1} frames a second (leaving out: {leave:?})",
        frames / took
    );
    if std::env::var("CONCAT_SPEED_KEEP").is_err() {
        let _ = std::fs::remove_dir_all(&root);
    }
}
